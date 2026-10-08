use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{
    Disableable, Icon, Sizable, VirtualListScrollHandle, WindowExt, v_virtual_list,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{UserAction, kit};
use crate::format;
use crate::net::{Command, Session, UlState, UploadRow, overlaps, virtual_roots};
use crate::theme::{Palette, palette};

const GROUP_ROW: f32 = 44.;
const FILE_ROW: f32 = 42.;
const GAP_ROW: f32 = 8.;
const MAX_SLOTS: usize = 50;

pub enum UploadsEvent {
    Shares(Vec<PathBuf>),
    Slots(usize),
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    File { row: usize, last: bool },
    Gap,
}

struct Group {
    username: String,
    rows: Vec<usize>,
}

struct Layout {
    source: usize,
    uploads: Arc<Vec<UploadRow>>,
    groups: Vec<Group>,
    rows: Vec<Row>,
    sizes: Rc<Vec<Size<Pixels>>>,
}

pub struct UploadsView {
    session: Entity<Session>,
    shares: Vec<PathBuf>,
    names: Vec<Option<String>>,
    slots: Entity<InputState>,
    slots_error: bool,
    scroll: VirtualListScrollHandle,
    layout: Option<Layout>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<UploadsEvent> for UploadsView {}

impl UploadsView {
    pub fn new(
        session: Entity<Session>,
        shares: Vec<PathBuf>,
        slots: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let slots = cx.new(|cx| InputState::new(window, cx).default_value(slots.to_string()));
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe_in(&slots, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.commit_slots(cx);
                }
            }),
        ];
        Self {
            session,
            names: virtual_roots(&shares),
            shares,
            slots,
            slots_error: false,
            scroll: VirtualListScrollHandle::new(),
            layout: None,
            _subscriptions: subscriptions,
        }
    }

    fn send(&self, command: Command, cx: &App) {
        self.session.read(cx).send(command);
    }

    fn set_shares(&mut self, shares: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.names = virtual_roots(&shares);
        self.shares = shares.clone();
        cx.emit(UploadsEvent::Shares(shares));
        cx.notify();
    }

    fn add_share(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("share".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let mut shares = this.shares.clone();
                for path in paths {
                    if overlaps(&shares, &path) {
                        window.push_notification(
                            Notification::warning(format!(
                                "{} overlaps a folder you already share",
                                path.display()
                            )),
                            cx,
                        );
                    } else {
                        shares.push(path);
                    }
                }
                if shares.len() != this.shares.len() {
                    this.set_shares(shares, cx);
                }
            });
        })
        .detach();
    }

    fn remove_share(&mut self, ix: usize, cx: &mut Context<Self>) {
        let mut shares = self.shares.clone();
        if ix < shares.len() {
            shares.remove(ix);
            self.set_shares(shares, cx);
        }
    }

    fn commit_slots(&mut self, cx: &mut Context<Self>) {
        match self.slots.read(cx).value().trim().parse::<usize>() {
            Ok(slots) if (1..=MAX_SLOTS).contains(&slots) => {
                self.slots_error = false;
                cx.emit(UploadsEvent::Slots(slots));
            }
            _ => self.slots_error = true,
        }
        cx.notify();
    }

    fn refresh_layout(&mut self, cx: &App) {
        let uploads = self.session.read(cx).uploads.clone();
        let source = Arc::as_ptr(&uploads) as usize;
        if self
            .layout
            .as_ref()
            .is_some_and(|layout| layout.source == source)
        {
            return;
        }

        let mut groups: Vec<Group> = Vec::new();
        for (ix, row) in uploads.iter().enumerate() {
            match groups
                .iter_mut()
                .find(|group| group.username == row.username)
            {
                Some(group) => group.rows.push(ix),
                None => groups.push(Group {
                    username: row.username.clone(),
                    rows: vec![ix],
                }),
            }
        }
        let rank = |group: &Group| {
            let states = group.rows.iter().map(|ix| &uploads[*ix].state);
            let mut rank = 2;
            for state in states {
                match state {
                    UlState::Active { .. } => return 0,
                    UlState::Queued { .. } => rank = 1,
                    _ => {}
                }
            }
            rank
        };
        groups.sort_by_key(rank);

        let mut rows = Vec::new();
        for (gx, group) in groups.iter().enumerate() {
            if gx > 0 {
                rows.push(Row::Gap);
            }
            rows.push(Row::Group(gx));
            let count = group.rows.len();
            rows.extend(group.rows.iter().enumerate().map(|(n, row)| Row::File {
                row: *row,
                last: n + 1 == count,
            }));
        }
        let sizes = rows
            .iter()
            .map(|row| {
                let height = match row {
                    Row::Group(_) => GROUP_ROW,
                    Row::File { .. } => FILE_ROW,
                    Row::Gap => GAP_ROW,
                };
                size(px(1.), px(height))
            })
            .collect();
        self.layout = Some(Layout {
            source,
            uploads,
            groups,
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
        range
            .filter_map(|ix| layout.rows.get(ix).copied())
            .map(|row| match row {
                Row::Gap => div().h(px(GAP_ROW)).w_full().into_any_element(),
                Row::Group(gx) => {
                    group_row(&layout.groups[gx], gx, &layout.uploads, &p, cx).into_any_element()
                }
                Row::File { row, last } => {
                    file_row(&layout.uploads[row], last, &p, cx).into_any_element()
                }
            })
            .collect()
    }

    fn render_shares(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        if self.shares.is_empty() {
            return div()
                .flex()
                .items_center()
                .gap_4()
                .p_5()
                .rounded(px(6.))
                .border_1()
                .border_color(p.border_weak)
                .bg(p.bg_weak)
                .child(
                    Icon::new(IconName::Share2)
                        .size(px(20.))
                        .text_color(p.text_weak),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(kit::strong("share a folder", p))
                        .child(div().text_color(p.text_weak).child(
                            "soulseek works because people share back. other users can search, browse and download what you share.",
                        )),
                )
                .child(
                    Button::new("share-first")
                        .primary()
                        .small()
                        .label("share a folder…")
                        .on_click(cx.listener(|this, _, window, cx| this.add_share(window, cx))),
                );
        }

        let count = self.shares.len();
        let mut rows = div().flex().flex_col();
        for (ix, (dir, name)) in self.shares.iter().zip(&self.names).enumerate() {
            let row = div()
                .id(("share", ix))
                .h(px(40.))
                .flex()
                .items_center()
                .gap_3()
                .px(px(14.))
                .when(ix > 0, |this| this.border_t_1());
            rows = rows.child(
                kit::box_edges(row, ix == 0, false, p)
                    .child(
                        Icon::new(IconName::Folder)
                            .size(px(14.))
                            .text_color(p.text_weaker),
                    )
                    .child(
                        kit::truncate(dir.display().to_string())
                            .flex_1()
                            .text_color(p.text_strong),
                    )
                    .child(match name {
                        Some(name) => div()
                            .flex_none()
                            .text_color(p.text_weak)
                            .child(format!("as {name}")),
                        None => div()
                            .flex_none()
                            .text_color(p.danger)
                            .child("folder not found"),
                    })
                    .child(
                        kit::icon_button(("unshare", ix), IconName::X, "stop sharing")
                            .on_click(cx.listener(move |this, _, _, cx| this.remove_share(ix, cx))),
                    ),
            );
        }

        let footer = div()
            .id("share-footer")
            .h(px(44.))
            .flex()
            .items_center()
            .gap_3()
            .px(px(14.))
            .border_t_1()
            .bg(p.bg_weak);
        rows.child(
            kit::box_edges(footer, count == 0, true, p)
                .child(div().text_color(p.text).child("upload slots"))
                .child(div().w(px(64.)).child(kit::input(&self.slots).small()))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .text_color(if self.slots_error {
                            p.danger
                        } else {
                            p.text_weak
                        })
                        .child(if self.slots_error {
                            "use a number from 1 to 50"
                        } else {
                            "how many people can download from you at once"
                        }),
                ),
        )
    }
}

fn group_row(
    group: &Group,
    gx: usize,
    uploads: &[UploadRow],
    p: &Palette,
    cx: &mut Context<UploadsView>,
) -> impl IntoElement {
    let rows: Vec<&UploadRow> = group.rows.iter().map(|ix| &uploads[*ix]).collect();
    let active = rows
        .iter()
        .filter(|row| matches!(row.state, UlState::Active { .. }))
        .count();
    let queued = rows
        .iter()
        .filter(|row| matches!(row.state, UlState::Queued { .. }))
        .count();
    let mut summary = format::plural(rows.len(), "file", "files");
    if active > 0 {
        summary.push_str(&format!(" · {active} uploading"));
    }
    if queued > 0 {
        summary.push_str(&format!(" · {queued} queued"));
    }

    let row = div()
        .id(("upload-group", gx))
        .h(px(GROUP_ROW))
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .px(px(14.))
        .bg(p.bg_weak);
    kit::box_edges(row, true, false, p)
        .child(
            Icon::new(IconName::User)
                .size(px(14.))
                .text_color(p.text_weaker),
        )
        .child(kit::user_cell(
            ("upload-user", gx),
            &group.username,
            p.text_strong,
            p,
            cx,
        ))
        .child(div().flex_1())
        .child(
            div()
                .flex_none()
                .text_size(px(12.))
                .text_color(p.text_weak)
                .child(summary),
        )
}

fn file_row(
    row: &UploadRow,
    last: bool,
    p: &Palette,
    cx: &mut Context<UploadsView>,
) -> impl IntoElement {
    let (status, color): (String, Hsla) = match &row.state {
        UlState::Queued { place } => (format!("queued #{place}"), p.text_weak),
        UlState::Active { speed } => (format::speed(*speed), p.yolk),
        UlState::Completed => ("done".into(), p.success),
        UlState::Cancelled => ("cancelled".into(), p.text_weaker),
        UlState::Failed(reason) => (reason.clone(), p.danger),
    };
    let active = matches!(row.state, UlState::Active { .. });
    let amount = if active {
        format!("{} / {}", format::bytes(row.sent), format::bytes(row.size))
    } else {
        format::bytes(row.size)
    };
    let (username, filename) = (row.username.clone(), row.filename.clone());

    let el = div()
        .id(("upload", row.id as usize))
        .h(px(FILE_ROW))
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .pl(px(40.))
        .pr(px(14.))
        .border_t_1()
        .hover(|style| style.bg(p.bg_weak));
    kit::box_edges(el, false, last, p)
        .child(
            Icon::new(match row.state {
                UlState::Completed => IconName::CircleCheck,
                UlState::Failed(_) => IconName::CircleAlert,
                _ => IconName::FileMusic,
            })
            .size(px(13.))
            .text_color(match row.state {
                UlState::Completed => p.success,
                UlState::Failed(_) => p.danger,
                _ => p.format(&format::extension(&row.name)),
            }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(kit::truncate(row.name.clone()).text_color(p.text))
                .when(active, |this| {
                    this.child(kit::progress_bar(row.progress(), p.yolk, p))
                }),
        )
        .child(
            div()
                .w(px(170.))
                .flex_none()
                .flex()
                .justify_end()
                .text_color(p.text_weak)
                .child(amount),
        )
        .child(
            kit::truncate(status)
                .w(px(170.))
                .flex_none()
                .flex()
                .justify_end()
                .text_color(color),
        )
        .child(
            div()
                .w(px(32.))
                .flex_none()
                .flex()
                .justify_end()
                .when(active, |this| {
                    this.child(
                        kit::icon_button(("cancel-upload", row.id as usize), IconName::X, "cancel")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.send(
                                    Command::CancelUpload {
                                        username: username.clone(),
                                        filename: filename.clone(),
                                    },
                                    cx,
                                )
                            })),
                    )
                }),
        )
}

impl EventEmitter<UserAction> for UploadsView {}

impl Render for UploadsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        self.refresh_layout(cx);
        let session = self.session.read(cx);
        let shares = session.shares;
        let uploads = session.uploads.clone();
        let active = uploads
            .iter()
            .filter(|row| matches!(row.state, UlState::Active { .. }))
            .count();
        let queued = uploads
            .iter()
            .filter(|row| matches!(row.state, UlState::Queued { .. }))
            .count();
        let finished = uploads
            .iter()
            .any(|row| !matches!(row.state, UlState::Active { .. } | UlState::Queued { .. }));

        let subtitle = if shares.scanning {
            "scanning your shared folders…".to_string()
        } else if self.shares.is_empty() {
            "you share nothing yet".to_string()
        } else {
            format!(
                "sharing {} in {}",
                format::plural(shares.files as usize, "file", "files"),
                format::plural(shares.folders as usize, "folder", "folders"),
            )
        };

        let actions = div()
            .flex()
            .gap_2()
            .child(
                Button::new("rescan")
                    .outline()
                    .small()
                    .icon(Icon::new(IconName::RotateCw))
                    .label("rescan")
                    .disabled(shares.scanning || self.shares.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| this.send(Command::Rescan, cx))),
            )
            .when(!self.shares.is_empty(), |this| {
                this.child(
                    Button::new("share")
                        .outline()
                        .small()
                        .icon(Icon::new(IconName::Plus))
                        .label("share a folder…")
                        .on_click(cx.listener(|this, _, window, cx| this.add_share(window, cx))),
                )
            });

        let list_header = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .text_color(p.text_strong)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("transfers"),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.))
                    .text_color(p.text_weak)
                    .child(format!("{active} uploading · {queued} queued")),
            )
            .child(
                Button::new("clear-uploads")
                    .outline()
                    .small()
                    .label("clear finished")
                    .disabled(!finished)
                    .on_click(cx.listener(|this, _, _, cx| this.send(Command::ClearUploads, cx))),
            );

        let list = match &self.layout {
            Some(layout) if !layout.rows.is_empty() => div()
                .flex_1()
                .min_h_0()
                .relative()
                .child(
                    v_virtual_list(
                        cx.entity(),
                        "uploads",
                        layout.sizes.clone(),
                        |this, range, _, cx| this.render_rows(range, cx),
                    )
                    .track_scroll(&self.scroll)
                    .pb_4(),
                )
                .vertical_scrollbar(&self.scroll)
                .into_any_element(),
            _ => kit::empty_state(
                IconName::Upload,
                "nobody is downloading from you yet",
                "when someone downloads from your shared folders, the files show up here.",
                &p,
            )
            .into_any_element(),
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .px(px(40.))
            .pt(px(32.))
            .child(kit::page_header("uploads", subtitle, &p).child(actions))
            .child(self.render_shares(&p, cx))
            .child(list_header)
            .child(list)
    }
}
