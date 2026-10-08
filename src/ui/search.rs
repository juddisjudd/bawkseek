use std::cmp::Reverse;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable, VirtualListScrollHandle, v_virtual_list};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{UserAction, kit};
use crate::format;
use crate::net::{Command, DlState, FolderHit, Scope, SearchHits, Session, Wanted, parse_scope};
use crate::theme::{Palette, palette};

const FOLDER_ROW: f32 = 52.;
const FILE_ROW: f32 = 30.;
const GAP_ROW: f32 = 8.;

const USER_W: f32 = 150.;
const FILES_W: f32 = 76.;
const SIZE_W: f32 = 80.;
const SPEED_W: f32 = 96.;
const SLOT_W: f32 = 64.;
const ACTION_W: f32 = 32.;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sort {
    Speed,
    Name,
    User,
    Files,
    Size,
}

#[derive(Clone, Copy)]
enum Row {
    Folder {
        folder: usize,
        open: bool,
    },
    File {
        folder: usize,
        file: usize,
        last: bool,
    },
    Gap,
}

#[derive(PartialEq)]
struct LayoutKey {
    query: SharedString,
    hits: usize,
    filter: String,
    free_only: bool,
    sort: Sort,
    expanded: u64,
}

struct Layout {
    key: LayoutKey,
    hits: Arc<SearchHits>,
    shown: usize,
    rows: Vec<Row>,
    sizes: Rc<Vec<Size<Pixels>>>,
}

pub struct SearchView {
    session: Entity<Session>,
    query: Entity<InputState>,
    filter: Entity<InputState>,
    active: usize,
    free_only: bool,
    sort: Sort,
    expanded: HashSet<(String, String)>,
    expanded_rev: u64,
    scroll: VirtualListScrollHandle,
    layout: Option<Layout>,
    _subscriptions: Vec<Subscription>,
}

impl SearchView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("artist, album or track · @user or #room to narrow")
        });
        let filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder("filter by user, folder, file or format")
        });
        let subscriptions = vec![
            cx.subscribe_in(&query, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.run(window, cx);
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
            query,
            filter,
            active: 0,
            free_only: false,
            sort: Sort::Speed,
            expanded: HashSet::new(),
            expanded_rev: 0,
            scroll: VirtualListScrollHandle::new(),
            layout: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Starts a search limited to one user, leaving the cursor after the name.
    pub fn prefill_user(&self, username: &str, window: &mut Window, cx: &mut Context<Self>) {
        let name = if username.contains(char::is_whitespace) {
            format!("\"{username}\"")
        } else {
            username.to_string()
        };
        self.query.update(cx, |state, cx| {
            state.set_value(format!("@{name} "), window, cx);
            state.focus(window, cx);
        });
    }

    fn run(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let text = self.query.read(cx).value().trim().to_string();
        if !text.is_empty() {
            self.open(&text, cx);
        }
    }

    pub fn open(&mut self, query: &str, cx: &mut Context<Self>) {
        let ix = self
            .session
            .update(cx, |session, cx| session.search(query, cx));
        self.select(ix, cx);
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.active = ix;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn close(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, cx| session.close_search(ix, cx));
        if self.active >= ix && self.active > 0 {
            self.active -= 1;
        }
        cx.notify();
    }

    fn toggle(&mut self, key: (String, String), cx: &mut Context<Self>) {
        if !self.expanded.remove(&key) {
            self.expanded.insert(key);
        }
        self.expanded_rev += 1;
        cx.notify();
    }

    fn set_sort(&mut self, sort: Sort, cx: &mut Context<Self>) {
        self.sort = sort;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn download_file(&mut self, folder: usize, file: usize, cx: &mut Context<Self>) {
        let Some(layout) = &self.layout else {
            return;
        };
        let hit = &layout.hits.folders[folder];
        let wanted = Wanted::from_hit(&hit.username, &hit.files[file]);
        self.session.read(cx).send(Command::Download(vec![wanted]));
    }

    fn download_folder(&mut self, folder: usize, cx: &mut Context<Self>) {
        let Some(layout) = &self.layout else {
            return;
        };
        let hit = &layout.hits.folders[folder];
        let fallback = hit
            .files
            .iter()
            .map(|file| Wanted::from_hit(&hit.username, file))
            .collect();
        self.session.read(cx).send(Command::DownloadFolder {
            username: hit.username.clone(),
            folder: hit.folder.clone(),
            fallback,
        });
    }

    /// Rebuilds the flattened rows only when results, filter, sort or expansion changed.
    fn refresh_layout(&mut self, cx: &App) {
        let session = self.session.read(cx);
        let Some(tab) = session.searches.get(self.active) else {
            self.layout = None;
            return;
        };
        let filter = self.filter.read(cx).value().trim().to_lowercase();
        let key = LayoutKey {
            query: tab.query.clone(),
            hits: Arc::as_ptr(&tab.hits) as usize,
            filter: filter.clone(),
            free_only: self.free_only,
            sort: self.sort,
            expanded: self.expanded_rev,
        };
        if self.layout.as_ref().is_some_and(|layout| layout.key == key) {
            return;
        }

        let hits = tab.hits.clone();
        let terms: Vec<&str> = filter.split_whitespace().collect();
        let mut visible: Vec<usize> = (0..hits.folders.len())
            .filter(|ix| {
                let folder = &hits.folders[*ix];
                (!self.free_only || folder.free) && matches(folder, &terms)
            })
            .collect();
        sort(&mut visible, &hits.folders, self.sort);

        let mut rows = Vec::with_capacity(visible.len() * 2);
        for (n, ix) in visible.iter().copied().enumerate() {
            if n > 0 {
                rows.push(Row::Gap);
            }
            let folder = &hits.folders[ix];
            let open = self
                .expanded
                .contains(&(folder.username.clone(), folder.folder.clone()));
            rows.push(Row::Folder { folder: ix, open });
            if open {
                let count = folder.files.len();
                rows.extend((0..count).map(|file| Row::File {
                    folder: ix,
                    file,
                    last: file + 1 == count,
                }));
            }
        }
        let sizes = rows
            .iter()
            .map(|row| {
                let height = match row {
                    Row::Folder { .. } => FOLDER_ROW,
                    Row::File { .. } => FILE_ROW,
                    Row::Gap => GAP_ROW,
                };
                size(px(1.), px(height))
            })
            .collect();
        self.layout = Some(Layout {
            key,
            hits,
            shown: visible.len(),
            rows,
            sizes: Rc::new(sizes),
        });
    }

    fn render_rows(
        &mut self,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = palette(cx);
        let Some(layout) = &self.layout else {
            return Vec::new();
        };
        let downloads = self.session.read(cx).downloads.clone();
        range
            .filter_map(|ix| layout.rows.get(ix).copied())
            .map(|row| match row {
                Row::Gap => div().h(px(GAP_ROW)).w_full().into_any_element(),
                Row::Folder { folder, open } => {
                    folder_row(&layout.hits.folders[folder], folder, open, &p, cx)
                        .into_any_element()
                }
                Row::File { folder, file, last } => {
                    let hit = &layout.hits.folders[folder];
                    let state = downloads
                        .iter()
                        .find(|row| {
                            row.username == hit.username && row.filename == hit.files[file].filename
                        })
                        .map(|row| row.state.clone());
                    file_row(hit, folder, file, last, state, &p, cx).into_any_element()
                }
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
            .children(session.searches.iter().enumerate().map(|(ix, tab)| {
                let active = ix == self.active;
                div()
                    .id(("tab", ix))
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
                    .on_click(cx.listener(move |this, _, _, cx| this.select(ix, cx)))
                    .when(tab.wish, |this| {
                        this.child(Icon::new(IconName::Star).size(px(12.)).text_color(p.yolk))
                    })
                    .child(kit::truncate(tab.query.clone()).max_w(px(220.)))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(p.text_weaker)
                            .child(format::count(tab.hits.files)),
                    )
                    .child(
                        div()
                            .id(("close", ix))
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

    fn render_columns(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let column = |sort: Sort, label: &'static str, width: Option<f32>, right: bool| {
            let active = self.sort == sort;
            div()
                .id(label)
                .flex()
                .gap_1()
                .cursor_pointer()
                .text_color(if active { p.text_strong } else { p.text_weak })
                .hover(|style| style.text_color(p.text_strong))
                .map(|this| match width {
                    Some(width) => this.w(px(width)).flex_none(),
                    None => this.flex_1(),
                })
                .when(right, |this| this.justify_end())
                .on_click(cx.listener(move |this, _, _, cx| this.set_sort(sort, cx)))
                .child(label)
                .when(active, |this| {
                    this.child(div().text_color(p.yolk).child("↓"))
                })
        };
        div()
            .flex()
            .items_center()
            .h(px(28.))
            .pl(px(42.))
            .pr(px(14. + ACTION_W))
            .gap_3()
            .text_size(px(12.))
            .child(column(Sort::Name, "folder", None, false))
            .child(column(Sort::User, "user", Some(USER_W), false))
            .child(column(Sort::Files, "files", Some(FILES_W), true))
            .child(column(Sort::Size, "size", Some(SIZE_W), true))
            .child(column(Sort::Speed, "speed", Some(SPEED_W), true))
            .child(
                div()
                    .w(px(SLOT_W))
                    .flex_none()
                    .text_color(p.text_weak)
                    .child("slot"),
            )
    }
}

fn matches(folder: &FolderHit, terms: &[&str]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let haystack =
        format!("{} {} {}", folder.username, folder.folder, folder.summary()).to_lowercase();
    terms.iter().all(|term| {
        if let Some(excluded) = term.strip_prefix('-').filter(|rest| !rest.is_empty()) {
            !haystack.contains(excluded)
                && !folder
                    .files
                    .iter()
                    .any(|file| file.name.to_lowercase().contains(excluded))
        } else {
            haystack.contains(term)
                || folder
                    .files
                    .iter()
                    .any(|file| file.name.to_lowercase().contains(term))
        }
    })
}

fn sort(visible: &mut [usize], folders: &[FolderHit], sort: Sort) {
    match sort {
        Sort::Speed => {
            visible.sort_by_key(|ix| (Reverse(folders[*ix].free), Reverse(folders[*ix].speed)))
        }
        Sort::Name => visible.sort_by_cached_key(|ix| folders[*ix].name.to_lowercase()),
        Sort::User => visible.sort_by_cached_key(|ix| folders[*ix].username.to_lowercase()),
        Sort::Files => visible.sort_by_key(|ix| Reverse(folders[*ix].files.len())),
        Sort::Size => visible.sort_by_key(|ix| Reverse(folders[*ix].size)),
    }
}

fn folder_row(
    hit: &FolderHit,
    ix: usize,
    open: bool,
    p: &Palette,
    cx: &mut Context<SearchView>,
) -> impl IntoElement {
    let key = (hit.username.clone(), hit.folder.clone());
    let row = div()
        .id(("folder", ix))
        .h(px(FOLDER_ROW))
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .px(px(14.))
        .cursor_pointer()
        .bg(if open { p.bg_weak } else { p.bg })
        .hover(|style| style.bg(p.bg_weak))
        .on_click(cx.listener(move |this, _, _, cx| this.toggle(key.clone(), cx)));
    kit::box_edges(row, true, !open, p)
        .child(
            Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .size(px(14.))
            .text_color(p.text_weaker),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(kit::truncate(display_name(hit)).text_color(p.text_strong))
                        .when(!hit.format.is_empty(), |this| {
                            this.child(kit::format_tag(&hit.format, &hit.quality, p))
                        }),
                )
                .child(
                    kit::truncate(hit.folder.clone())
                        .text_size(px(12.))
                        .text_color(p.text_weaker),
                ),
        )
        .child(div().w(px(USER_W)).flex_none().flex().child(kit::user_cell(
            ("user", ix),
            &hit.username,
            p.text,
            p,
            cx,
        )))
        .child(
            div()
                .w(px(FILES_W))
                .flex_none()
                .flex()
                .justify_end()
                .text_color(p.text_weak)
                .child(format::count(hit.files.len())),
        )
        .child(
            div()
                .w(px(SIZE_W))
                .flex_none()
                .flex()
                .justify_end()
                .text_color(p.text_weak)
                .child(format::bytes(hit.size)),
        )
        .child(
            div()
                .w(px(SPEED_W))
                .flex_none()
                .flex()
                .justify_end()
                .text_color(p.text_weak)
                .child(format::speed(hit.speed as u64)),
        )
        .child(
            div()
                .w(px(SLOT_W))
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .text_color(p.text_weak)
                .child(kit::dot(if hit.free { p.success } else { p.warning }))
                .child(if hit.free { "free" } else { "busy" }),
        )
        .child(div().w(px(ACTION_W)).flex_none().child(
            kit::icon_button(("get-folder", ix), IconName::FolderDown, "download folder").on_click(
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.download_folder(ix, cx);
                }),
            ),
        ))
}

fn display_name(hit: &FolderHit) -> String {
    if hit.name.is_empty() {
        "(shared root)".into()
    } else {
        hit.name.clone()
    }
}

fn file_row(
    hit: &FolderHit,
    folder: usize,
    file: usize,
    last: bool,
    state: Option<DlState>,
    p: &Palette,
    cx: &mut Context<SearchView>,
) -> impl IntoElement {
    let entry = &hit.files[file];
    let status = state.map(|state| match state {
        DlState::Queued { .. } => ("queued", p.text_weak),
        DlState::Active { .. } => ("downloading", p.yolk),
        DlState::Paused { .. } => ("paused", p.text_weak),
        DlState::Completed => ("done", p.success),
        DlState::Cancelled => ("cancelled", p.text_weaker),
        DlState::Failed(_) => ("failed", p.danger),
    });
    let row = div()
        .id(("file", folder * 100_000 + file))
        .h(px(FILE_ROW))
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .pl(px(42.))
        .pr(px(14.))
        .bg(p.bg_weak)
        .hover(|style| style.bg(p.bg_hover));
    kit::box_edges(row, false, last, p)
        .child(kit::file_icon(&entry.ext, p))
        .child(
            kit::truncate(entry.name.clone())
                .flex_1()
                .text_color(p.text),
        )
        .child(
            div()
                .w(px(USER_W))
                .flex_none()
                .flex()
                .justify_end()
                .gap_3()
                .text_color(p.text_weak)
                .child(
                    div()
                        .text_color(p.format(&entry.ext))
                        .child(entry.quality()),
                )
                .child(entry.duration.map(format::duration).unwrap_or_default()),
        )
        .child(div().w(px(FILES_W)).flex_none())
        .child(
            div()
                .w(px(SIZE_W))
                .flex_none()
                .flex()
                .justify_end()
                .text_color(p.text_weak)
                .child(format::bytes(entry.size)),
        )
        .child(
            div()
                .w(px(SPEED_W + SLOT_W + 12.))
                .flex_none()
                .flex()
                .justify_end()
                .when_some(status, |this, (label, color)| {
                    this.text_color(color).child(label)
                }),
        )
        .child(
            div().w(px(ACTION_W)).flex_none().child(
                kit::icon_button(
                    ("get-file", folder * 100_000 + file),
                    IconName::Download,
                    "download file",
                )
                .on_click(cx.listener(move |this, _, _, cx| this.download_file(folder, file, cx))),
            ),
        )
}

impl EventEmitter<UserAction> for SearchView {}

impl Render for SearchView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let tab_count = self.session.read(cx).searches.len();
        if self.active >= tab_count {
            self.active = tab_count.saturating_sub(1);
        }
        self.refresh_layout(cx);

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
                Icon::new(IconName::Search)
                    .size(px(15.))
                    .text_color(p.text_weaker),
            )
            .child(
                div()
                    .flex_1()
                    .child(kit::input(&self.query).appearance(false)),
            )
            .child(
                Button::new("run-search")
                    .primary()
                    .small()
                    .label("search")
                    .on_click(cx.listener(|this, _, window, cx| this.run(window, cx))),
            );

        let subtitle = match tab_count {
            0 => "results come straight from other users".to_string(),
            n => format::plural(n, "open search", "open searches"),
        };

        let wish = self
            .session
            .read(cx)
            .searches
            .get(self.active)
            .map(|tab| (tab.wish, parse_scope(&tab.query).0 == Scope::Everyone));
        let (wish, can_wish) = wish.unwrap_or((false, false));
        let body = match &self.layout {
            None => kit::empty_state(
                IconName::Search,
                "search the soulseek network",
                "type an artist, album or track and press enter. other users answer over the next half minute, so results keep arriving.",
                &p,
            )
            .into_any_element(),
            Some(layout) => {
                let stats = format!(
                    "{} · {} · {}",
                    format::plural(layout.shown, "folder", "folders"),
                    format::plural(layout.hits.files, "file", "files"),
                    format::plural(layout.hits.users, "user", "users"),
                );
                let list = if layout.rows.is_empty() {
                    let (title, body) = if layout.hits.files == 0 {
                        ("searching…", "peers answer over the next half minute.")
                    } else {
                        ("nothing matches the filter", "clear the filter or turn off free slots.")
                    };
                    kit::empty_state(IconName::Search, title, body, &p).into_any_element()
                } else {
                    div()
                        .flex_1()
                        .min_h_0()
                        .relative()
                        .child(
                            v_virtual_list(cx.entity(), "results", layout.sizes.clone(), |this, range, _, cx| {
                                this.render_rows(range, cx)
                            })
                            .track_scroll(&self.scroll)
                            .pb_4(),
                        )
                        .vertical_scrollbar(&self.scroll)
                        .into_any_element()
                };
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.render_tabs(&p, cx))
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
                                        .prefix(Icon::new(IconName::Search).small().text_color(p.text_weak)),
                                ),
                            )
                            .child(div().text_size(px(12.)).text_color(p.text_weak).child(stats))
                            .child(
                                kit::chip("free-only", "free slots", self.free_only, &p).on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.free_only = !this.free_only;
                                        cx.notify();
                                    },
                                )),
                            )
                            .when(can_wish, |this| {
                                this.child(kit::chip("wishlist", "keep searching", wish, &p).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        let ix = this.active;
                                        this.session.update(cx, |session, cx| session.toggle_wish(ix, cx));
                                    }),
                                ))
                            }),
                    )
                    .child(self.render_columns(&p, cx))
                    .child(list)
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
            .child(kit::page_header("search", subtitle, &p))
            .child(command)
            .child(body)
    }
}
