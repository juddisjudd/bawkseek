use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable, VirtualListScrollHandle, WindowExt, v_virtual_list};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::kit;
use crate::format;
use crate::net::{BrowseState, Command, DlState, FileHit, Listing, Node, Session, Wanted};
use crate::theme::{Palette, palette};

const TREE_ROW: f32 = 28.;
const FILE_ROW: f32 = 30.;
const CONFIRM_ABOVE: usize = 200;

#[derive(Default)]
struct TabUi {
    expanded: HashSet<usize>,
    selected: Option<usize>,
    expanded_rev: u64,
}

#[derive(PartialEq)]
struct TreeKey {
    listing: usize,
    filter: String,
    expanded: u64,
}

struct Tree {
    key: TreeKey,
    rows: Vec<usize>,
    sizes: Rc<Vec<Size<Pixels>>>,
}

pub struct BrowseView {
    session: Entity<Session>,
    user: Entity<InputState>,
    filter: Entity<InputState>,
    active: usize,
    tabs: HashMap<SharedString, TabUi>,
    tree: Option<Tree>,
    tree_scroll: VirtualListScrollHandle,
    file_scroll: VirtualListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl BrowseView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let user = cx.new(|cx| InputState::new(window, cx).placeholder("username"));
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("filter folders"));
        let subscriptions = vec![
            cx.subscribe_in(&user, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.open_typed(cx);
                }
            }),
            cx.subscribe_in(&filter, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.observe(&session, |_, _, cx| cx.notify()),
        ];
        Self {
            session,
            user,
            filter,
            active: 0,
            tabs: HashMap::new(),
            tree: None,
            tree_scroll: VirtualListScrollHandle::new(),
            file_scroll: VirtualListScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.user.update(cx, |state, cx| state.focus(window, cx));
    }

    pub fn open(&mut self, username: &str, cx: &mut Context<Self>) {
        let ix = self
            .session
            .update(cx, |session, cx| session.browse(username, cx));
        self.select_tab(ix, cx);
    }

    fn open_typed(&mut self, cx: &mut Context<Self>) {
        let username = self.user.read(cx).value().trim().to_string();
        if !username.is_empty() {
            self.open(&username, cx);
        }
    }

    fn select_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.active = ix;
        self.tree_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.file_scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn close(&mut self, ix: usize, cx: &mut Context<Self>) {
        let username = self
            .session
            .read(cx)
            .browses
            .get(ix)
            .map(|tab| tab.username.clone());
        if let Some(username) = username {
            self.tabs.remove(&username);
        }
        self.session
            .update(cx, |session, cx| session.close_browse(ix, cx));
        if self.active >= ix && self.active > 0 {
            self.active -= 1;
        }
        cx.notify();
    }

    fn current(&self, cx: &App) -> Option<(SharedString, Arc<Listing>)> {
        let tab = self.session.read(cx).browses.get(self.active)?;
        match &tab.state {
            BrowseState::Ready(listing) => Some((tab.username.clone(), listing.clone())),
            _ => None,
        }
    }

    fn ui(&mut self, username: &SharedString, listing: &Listing) -> &mut TabUi {
        self.tabs.entry(username.clone()).or_insert_with(|| TabUi {
            expanded: listing.roots.iter().copied().collect(),
            selected: listing.roots.first().copied(),
            expanded_rev: 0,
        })
    }

    fn toggle(&mut self, node: usize, cx: &mut Context<Self>) {
        let Some((username, listing)) = self.current(cx) else {
            return;
        };
        let ui = self.ui(&username, &listing);
        if !ui.expanded.remove(&node) {
            ui.expanded.insert(node);
        }
        ui.expanded_rev += 1;
        cx.notify();
    }

    fn select_node(&mut self, node: usize, cx: &mut Context<Self>) {
        let Some((username, listing)) = self.current(cx) else {
            return;
        };
        self.ui(&username, &listing).selected = Some(node);
        self.file_scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn download_file(&mut self, node: usize, file: usize, cx: &mut Context<Self>) {
        let Some((username, listing)) = self.current(cx) else {
            return;
        };
        let wanted = Wanted::from_hit(&username, &listing.nodes[node].files[file]);
        self.session.read(cx).send(Command::Download(vec![wanted]));
    }

    fn download_folder(&mut self, node: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((username, listing)) = self.current(cx) else {
            return;
        };
        let entry = &listing.nodes[node];
        if entry.total_files <= CONFIRM_ABOVE {
            send_tree(&self.session, &username, &listing, node, cx);
            return;
        }
        let title: SharedString = format!("download all of {}?", entry.name).into();
        let message: SharedString = format!(
            "this queues {} ({}) from {username}.",
            format::plural(entry.total_files, "file", "files"),
            format::bytes(entry.total_size)
        )
        .into();
        let session = self.session.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let (session, username, listing) = (session.clone(), username.clone(), listing.clone());
            dialog.title(title.clone()).child(message.clone()).footer(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        kit::button("cancel-tree", cx)
                            .label("cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("confirm-tree")
                            .primary()
                            .label("download")
                            .on_click(move |_, window, cx| {
                                send_tree(&session, &username, &listing, node, cx);
                                window.close_dialog(cx);
                            }),
                    ),
            )
        });
    }

    fn refresh_tree(&mut self, cx: &App) {
        let Some((username, listing)) = self.current(cx) else {
            self.tree = None;
            return;
        };
        let filter = self.filter.read(cx).value().trim().to_lowercase();
        let expanded_rev = self.tabs.get(&username).map_or(0, |ui| ui.expanded_rev);
        let key = TreeKey {
            listing: Arc::as_ptr(&listing) as usize,
            filter: filter.clone(),
            expanded: expanded_rev,
        };
        if self.tree.as_ref().is_some_and(|tree| tree.key == key) {
            return;
        }
        let ui = self.ui(&username, &listing);
        let rows = if filter.is_empty() {
            visible_rows(&listing, &ui.expanded)
        } else {
            filtered_rows(&listing, &filter)
        };
        let sizes = Rc::new(vec![size(px(1.), px(TREE_ROW)); rows.len()]);
        self.tree = Some(Tree { key, rows, sizes });
    }

    fn render_tree_rows(
        &mut self,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = palette(cx);
        let (Some((username, listing)), Some(tree)) = (self.current(cx), &self.tree) else {
            return Vec::new();
        };
        let filtering = !tree.key.filter.is_empty();
        let ui = self.tabs.get(&username);
        range
            .filter_map(|ix| tree.rows.get(ix).copied())
            .map(|node| {
                let entry = &listing.nodes[node];
                let open = filtering || ui.is_some_and(|ui| ui.expanded.contains(&node));
                let selected = ui.is_some_and(|ui| ui.selected == Some(node));
                let has_children = !entry.children.is_empty();
                div()
                    .id(("node", node))
                    .h(px(TREE_ROW))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl(px(8. + entry.depth as f32 * 16.))
                    .pr(px(10.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .when(selected, |this| {
                        this.bg(p.bg_hover).text_color(p.text_strong)
                    })
                    .when(!selected, |this| this.hover(|style| style.bg(p.bg_weak)))
                    .on_click(cx.listener(move |this, _, _, cx| this.select_node(node, cx)))
                    .child(
                        div()
                            .id(("twisty", node))
                            .size(px(16.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(has_children && !filtering, |this| {
                                this.on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.toggle(node, cx);
                                }))
                            })
                            .when(has_children, |this| {
                                this.child(
                                    Icon::new(if open {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    })
                                    .size(px(13.))
                                    .text_color(p.text_weaker),
                                )
                            }),
                    )
                    .child(
                        Icon::new(if selected {
                            IconName::FolderOpen
                        } else {
                            IconName::Folder
                        })
                        .size(px(14.))
                        .text_color(if selected {
                            p.yolk
                        } else {
                            p.text_weaker
                        }),
                    )
                    .child(kit::truncate(entry.name.clone()).flex_1())
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(p.text_weaker)
                            .child(format::count(entry.total_files)),
                    )
                    .into_any_element()
            })
            .collect()
    }

    fn render_file_rows(
        &mut self,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = palette(cx);
        let Some((username, listing)) = self.current(cx) else {
            return Vec::new();
        };
        let Some(node) = self.tabs.get(&username).and_then(|ui| ui.selected) else {
            return Vec::new();
        };
        let downloads = self.session.read(cx).downloads.clone();
        let files = &listing.nodes[node].files;
        range
            .filter_map(|ix| files.get(ix).map(|file| (ix, file)))
            .map(|(ix, file)| {
                let state = downloads
                    .iter()
                    .find(|row| row.username == username.as_ref() && row.filename == file.filename)
                    .map(|row| row.state.clone());
                file_row(file, node, ix, state, &p, cx).into_any_element()
            })
            .collect()
    }

    fn render_tabs(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        div()
            .flex()
            .items_end()
            .gap_5()
            .border_b_1()
            .border_color(p.border_weak)
            .children(session.browses.iter().enumerate().map(|(ix, tab)| {
                let active = ix == self.active;
                div()
                    .id(("browse-tab", ix))
                    .flex()
                    .items_center()
                    .gap_2()
                    .pb(px(8.))
                    .border_b_2()
                    .border_color(if active {
                        p.text_strong
                    } else {
                        transparent_black()
                    })
                    .text_color(if active { p.text_strong } else { p.text_weak })
                    .cursor_pointer()
                    .hover(|style| style.text_color(p.text_strong))
                    .on_click(cx.listener(move |this, _, _, cx| this.select_tab(ix, cx)))
                    .child(Icon::new(IconName::User).size(px(13.)))
                    .child(kit::truncate(tab.username.clone()).max_w(px(200.)))
                    .when(matches!(tab.state, BrowseState::Loading), |this| {
                        this.child(
                            div()
                                .text_size(px(12.))
                                .text_color(p.text_weaker)
                                .child("…"),
                        )
                    })
                    .child(
                        div()
                            .id(("browse-close", ix))
                            .rounded(px(3.))
                            .p(px(2.))
                            .text_color(p.text_weaker)
                            .hover(|style| style.bg(p.bg_hover).text_color(p.text_strong))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close(ix, cx);
                            }))
                            .child(Icon::new(IconName::X).size(px(12.))),
                    )
            }))
    }

    fn render_listing(
        &mut self,
        username: SharedString,
        listing: Arc<Listing>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        self.refresh_tree(cx);
        let selected = self.tabs.get(&username).and_then(|ui| ui.selected);
        let tree_sizes = self
            .tree
            .as_ref()
            .map(|tree| tree.sizes.clone())
            .unwrap_or_default();

        let tree_empty = tree_sizes.is_empty();
        let tree_pane = div()
            .w(relative(0.38))
            .flex_none()
            .h_full()
            .relative()
            .pr_2()
            .border_r_1()
            .border_color(p.border_weak)
            .child(
                v_virtual_list(cx.entity(), "tree", tree_sizes, |this, range, _, cx| {
                    this.render_tree_rows(range, cx)
                })
                .track_scroll(&self.tree_scroll)
                .pb_4(),
            )
            .vertical_scrollbar(&self.tree_scroll)
            .when(tree_empty, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(p.text_weak)
                        .child("no folder or file matches"),
                )
            });

        let files_pane = match selected {
            None => div().flex_1(),
            Some(node) => {
                let entry = &listing.nodes[node];
                let count = entry.files.len();
                let file_sizes = Rc::new(vec![size(px(1.), px(FILE_ROW)); count]);
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .pl_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .pb_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        kit::truncate(entry.name.clone()).text_color(p.text_strong),
                                    )
                                    .child(
                                        kit::truncate(format!(
                                            "{} here · {} in all · {}",
                                            format::plural(count, "file", "files"),
                                            format::count(entry.total_files),
                                            format::bytes(entry.total_size)
                                        ))
                                        .text_size(px(12.))
                                        .text_color(p.text_weak),
                                    ),
                            )
                            .child(
                                kit::button("get-tree", cx)
                                    .small()
                                    .icon(Icon::new(IconName::FolderDown))
                                    .label("download folder")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.download_folder(node, window, cx)
                                    })),
                            ),
                    )
                    .child(if count == 0 {
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(p.text_weak)
                            .child("no files directly in this folder")
                            .into_any_element()
                    } else {
                        div()
                            .flex_1()
                            .min_h_0()
                            .relative()
                            .child(
                                v_virtual_list(
                                    cx.entity(),
                                    "files",
                                    file_sizes,
                                    |this, range, _, cx| this.render_file_rows(range, cx),
                                )
                                .track_scroll(&self.file_scroll)
                                .pb_4(),
                            )
                            .vertical_scrollbar(&self.file_scroll)
                            .into_any_element()
                    })
            }
        };

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .pb(px(8.))
                    .border_b_1()
                    .border_color(p.border_weak)
                    .child(
                        div().flex_1().child(
                            kit::input(&self.filter)
                                .appearance(false)
                                .cleanable(true)
                                .prefix(
                                    Icon::new(IconName::Search).small().text_color(p.text_weak),
                                ),
                        ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(p.text_weak)
                            .child(format!(
                                "{} · {} · {}",
                                format::plural(listing.nodes.len(), "folder", "folders"),
                                format::plural(listing.files, "file", "files"),
                                format::bytes(listing.size)
                            )),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(tree_pane)
                    .child(files_pane),
            )
    }
}

fn visible_rows(listing: &Listing, expanded: &HashSet<usize>) -> Vec<usize> {
    let mut rows = Vec::new();
    let mut stack: Vec<usize> = listing.roots.iter().rev().copied().collect();
    while let Some(node) = stack.pop() {
        rows.push(node);
        if expanded.contains(&node) {
            stack.extend(listing.nodes[node].children.iter().rev().copied());
        }
    }
    rows
}

/// Shows every folder whose path matches all terms, with its parents so the tree still reads.
fn send_tree(session: &Entity<Session>, username: &str, listing: &Listing, node: usize, cx: &App) {
    let files = listing
        .files_under(node)
        .into_iter()
        .map(|(file, relative)| (Wanted::from_hit(username, file), relative))
        .collect();
    session.read(cx).send(Command::DownloadTree {
        root: listing.nodes[node].path.clone(),
        files,
    });
}

/// A folder matches when every term is in its path or, together with its path, in one file name.
fn node_matches(node: &Node, terms: &[&str]) -> bool {
    let path = node.path.to_lowercase();
    terms.iter().all(|term| path.contains(term))
        || node.files.iter().any(|file| {
            let name = file.name.to_lowercase();
            terms
                .iter()
                .all(|term| path.contains(term) || name.contains(term))
        })
}

fn filtered_rows(listing: &Listing, filter: &str) -> Vec<usize> {
    let terms: Vec<&str> = filter.split_whitespace().collect();
    let mut keep = vec![false; listing.nodes.len()];
    for (ix, node) in listing.nodes.iter().enumerate() {
        if node_matches(node, &terms) {
            let mut cursor = Some(ix);
            while let Some(current) = cursor {
                if keep[current] {
                    break;
                }
                keep[current] = true;
                cursor = listing.nodes[current].parent;
            }
        }
    }
    let mut rows = Vec::new();
    let mut stack: Vec<usize> = listing.roots.iter().rev().copied().collect();
    while let Some(node) = stack.pop() {
        if !keep[node] {
            continue;
        }
        rows.push(node);
        stack.extend(listing.nodes[node].children.iter().rev().copied());
    }
    rows
}

fn file_row(
    file: &FileHit,
    node: usize,
    ix: usize,
    state: Option<DlState>,
    p: &Palette,
    cx: &mut Context<BrowseView>,
) -> impl IntoElement {
    let status = state.map(|state| match state {
        DlState::Queued { .. } => ("queued", p.text_weak),
        DlState::Active { .. } => ("downloading", p.yolk),
        DlState::Paused { .. } => ("paused", p.text_weak),
        DlState::Completed => ("done", p.success),
        DlState::Cancelled => ("cancelled", p.text_weaker),
        DlState::Failed(_) => ("failed", p.danger),
    });
    div()
        .id(("browse-file", ix))
        .h(px(FILE_ROW))
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .px(px(10.))
        .rounded(px(4.))
        .hover(|style| style.bg(p.bg_weak))
        .child(kit::file_icon(&file.ext, p))
        .child(kit::truncate(file.name.clone()).flex_1().text_color(p.text))
        .child(
            div()
                .flex_none()
                .flex()
                .gap_3()
                .text_color(p.text_weak)
                .child(div().text_color(p.format(&file.ext)).child(file.quality()))
                .child(file.duration.map(format::duration).unwrap_or_default()),
        )
        .child(
            div()
                .w(px(80.))
                .flex_none()
                .flex()
                .justify_end()
                .text_color(p.text_weak)
                .child(format::bytes(file.size)),
        )
        .child(
            div()
                .w(px(90.))
                .flex_none()
                .flex()
                .justify_end()
                .when_some(status, |this, (label, color)| {
                    this.text_color(color).child(label)
                }),
        )
        .child(
            kit::icon_button(("browse-get", ix), IconName::Download, "download file")
                .on_click(cx.listener(move |this, _, _, cx| this.download_file(node, ix, cx))),
        )
}

impl Render for BrowseView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let tab_count = self.session.read(cx).browses.len();
        if self.active >= tab_count {
            self.active = tab_count.saturating_sub(1);
        }

        let command = div()
            .h(px(46.))
            .flex()
            .items_center()
            .gap_2()
            .pl(px(14.))
            .pr(px(6.))
            .rounded(px(6.))
            .border_1()
            .border_color(p.border_weak)
            .bg(p.bg_weak)
            .child(
                Icon::new(IconName::FolderSearch)
                    .size(px(15.))
                    .text_color(p.text_weaker),
            )
            .child(
                div()
                    .flex_1()
                    .child(kit::input(&self.user).appearance(false)),
            )
            .child(
                kit::button("open-browse", cx)
                    .small()
                    .label("open")
                    .on_click(cx.listener(|this, _, _, cx| this.open_typed(cx))),
            );

        let tab = self
            .session
            .read(cx)
            .browses
            .get(self.active)
            .map(|tab| match &tab.state {
                BrowseState::Loading => (tab.username.clone(), None, None),
                BrowseState::Failed(reason) => (tab.username.clone(), None, Some(reason.clone())),
                BrowseState::Ready(listing) => (tab.username.clone(), Some(listing.clone()), None),
            });

        let subtitle = match &tab {
            Some((username, Some(listing), _)) => format!(
                "{username} shares {} in {}",
                format::plural(listing.files, "file", "files"),
                format::plural(listing.nodes.len(), "folder", "folders")
            ),
            _ => "open anyone's shared folders".to_string(),
        };

        let body = match tab {
            None => kit::empty_state(
                IconName::FolderSearch,
                "browse a user's shares",
                "type a username and press enter. you can also click a username in search, transfers or uploads.",
                &p,
            )
            .into_any_element(),
            Some((username, listing, failure)) => {
                let content = match (listing, failure) {
                    (Some(listing), _) => self.render_listing(username, listing, &p, cx).into_any_element(),
                    (None, Some(reason)) => {
                        let ix = self.active;
                        kit::empty_state(IconName::CircleAlert, &format!("could not browse {username}"), &reason, &p)
                            .child(
                                div().mt_3().child(
                                    kit::button("retry-browse", cx)
                                        .small()
                                        .icon(Icon::new(IconName::RotateCw))
                                        .label("try again")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.session
                                                .update(cx, |session, cx| session.refresh_browse(ix, cx));
                                        })),
                                ),
                            )
                            .into_any_element()
                    }
                    (None, None) => kit::empty_state(
                        IconName::Hourglass,
                        &format!("asking {username} for their shares…"),
                        "big collections can take up to a minute to arrive.",
                        &p,
                    )
                    .into_any_element(),
                };
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.render_tabs(&p, cx))
                    .child(content)
                    .into_any_element()
            }
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .px(px(40.))
            .pt(px(32.))
            .pb_4()
            .child(kit::page_header("browse", subtitle, &p))
            .child(command)
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use slsk::proto::types::{Directory, FileEntry};

    use super::{filtered_rows, visible_rows};
    use crate::net::Listing;

    fn listing() -> Listing {
        let dir = |name: &str| Directory {
            name: name.into(),
            files: vec![FileEntry {
                name: "a.mp3".into(),
                size: 1,
                ext: String::new(),
                attrs: Vec::new(),
            }],
        };
        Listing::build(vec![
            dir("Music\\Rock\\Album One"),
            dir("Music\\Jazz\\Blue Train"),
            dir("Books"),
        ])
    }

    fn names(listing: &Listing, rows: &[usize]) -> Vec<String> {
        rows.iter()
            .map(|ix| listing.nodes[*ix].name.clone())
            .collect()
    }

    #[test]
    fn shows_only_expanded_branches() {
        let listing = listing();
        let music = listing.roots[1];
        let rows = visible_rows(&listing, &HashSet::from([music]));
        assert_eq!(
            names(&listing, &rows),
            vec!["Books", "Music", "Jazz", "Rock"]
        );
    }

    #[test]
    fn filter_matches_file_names_with_the_folder_path() {
        let listing = listing();
        assert_eq!(
            names(&listing, &filtered_rows(&listing, "a.mp3 jazz")),
            vec!["Music", "Jazz", "Blue Train"]
        );
        assert!(filtered_rows(&listing, "b.mp3").is_empty());
    }

    #[test]
    fn filter_keeps_matching_folders_and_their_parents() {
        let listing = listing();
        let rows = filtered_rows(&listing, "blue");
        assert_eq!(names(&listing, &rows), vec!["Music", "Jazz", "Blue Train"]);
    }
}
