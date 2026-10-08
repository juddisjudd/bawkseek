use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Icon, Sizable, VirtualListScrollHandle, v_virtual_list};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::kit;
use crate::format;
use crate::net::{Command, DlState, DownloadRow, Session};
use crate::theme::{Palette, palette};

const GROUP_ROW: f32 = 44.;
const FILE_ROW: f32 = 42.;
const GAP_ROW: f32 = 8.;

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    File { row: usize, last: bool },
    Gap,
}

struct Group {
    dir: PathBuf,
    folder: String,
    username: String,
    rows: Vec<usize>,
}

struct Layout {
    source: usize,
    downloads: Arc<Vec<DownloadRow>>,
    groups: Vec<Group>,
    rows: Vec<Row>,
    sizes: Rc<Vec<Size<Pixels>>>,
}

pub struct TransfersView {
    session: Entity<Session>,
    download_dir: PathBuf,
    scroll: VirtualListScrollHandle,
    layout: Option<Layout>,
    _subscriptions: Vec<Subscription>,
}

impl TransfersView {
    pub fn new(session: Entity<Session>, download_dir: PathBuf, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&session, |_, _, cx| cx.notify())];
        Self {
            session,
            download_dir,
            scroll: VirtualListScrollHandle::new(),
            layout: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_download_dir(&mut self, dir: PathBuf) {
        self.download_dir = dir;
    }

    fn send(&self, command: Command, cx: &App) {
        self.session.read(cx).send(command);
    }

    fn refresh_layout(&mut self, cx: &App) {
        let downloads = self.session.read(cx).downloads.clone();
        let source = Arc::as_ptr(&downloads) as usize;
        if self
            .layout
            .as_ref()
            .is_some_and(|layout| layout.source == source)
        {
            return;
        }

        let mut groups: Vec<Group> = Vec::new();
        for (ix, row) in downloads.iter().enumerate() {
            match groups
                .iter_mut()
                .find(|group| group.dir == row.local_dir && group.username == row.username)
            {
                Some(group) => group.rows.push(ix),
                None => groups.push(Group {
                    dir: row.local_dir.clone(),
                    folder: row.folder.clone(),
                    username: row.username.clone(),
                    rows: vec![ix],
                }),
            }
        }
        groups.reverse();

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
            downloads,
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
                    group_row(&layout.groups[gx], gx, &layout.downloads, &p, cx).into_any_element()
                }
                Row::File { row, last, .. } => {
                    file_row(&layout.downloads[row], last, &p, cx).into_any_element()
                }
            })
            .collect()
    }
}

fn open_dir(dir: &Path, fallback: &Path, cx: &App) {
    let target = if dir.exists() { dir } else { fallback };
    let _ = std::fs::create_dir_all(target);
    cx.open_with_system(target);
}

fn edges(this: Stateful<Div>, top: bool, bottom: bool, p: &Palette) -> Stateful<Div> {
    this.border_color(p.border_weak)
        .border_l_1()
        .border_r_1()
        .when(top, |this| this.border_t_1().rounded_t(px(6.)))
        .when(bottom, |this| this.border_b_1().rounded_b(px(6.)))
}

fn group_row(
    group: &Group,
    gx: usize,
    downloads: &[DownloadRow],
    p: &Palette,
    cx: &mut Context<TransfersView>,
) -> impl IntoElement {
    let rows: Vec<&DownloadRow> = group.rows.iter().map(|ix| &downloads[*ix]).collect();
    let done = rows
        .iter()
        .filter(|row| row.state == DlState::Completed)
        .count();
    let failed = rows
        .iter()
        .filter(|row| matches!(row.state, DlState::Failed(_)))
        .count();
    let total: u64 = rows.iter().map(|row| row.size).sum();
    let dir = group.dir.clone();
    let mut summary = format!("{done} of {} done · {}", rows.len(), format::bytes(total));
    if failed > 0 {
        summary.push_str(&format!(" · {failed} failed"));
    }

    let row = div()
        .id(("group", gx))
        .h(px(GROUP_ROW))
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .px(px(14.))
        .bg(p.bg_weak);
    edges(row, true, false, p)
        .child(
            Icon::new(IconName::Folder)
                .size(px(14.))
                .text_color(p.text_weaker),
        )
        .child(
            kit::truncate(if group.folder.is_empty() {
                "downloads".to_string()
            } else {
                group.folder.clone()
            })
            .text_color(p.text_strong),
        )
        .child(kit::truncate(format!("from {}", group.username)).text_color(p.text_weak))
        .child(div().flex_1())
        .child(
            div()
                .flex_none()
                .text_size(px(12.))
                .text_color(p.text_weak)
                .child(summary),
        )
        .when(failed > 0, |this| {
            let failed_ids: Vec<u64> = rows
                .iter()
                .filter(|row| matches!(row.state, DlState::Failed(_)))
                .map(|row| row.id)
                .collect();
            this.child(
                kit::icon_button(
                    ("retry-group", gx),
                    IconName::RotateCw,
                    "retry failed files",
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    for id in &failed_ids {
                        this.send(Command::Retry(*id), cx);
                    }
                })),
            )
        })
        .child(
            kit::icon_button(("open-group", gx), IconName::ExternalLink, "open folder").on_click(
                cx.listener(move |this, _, _, cx| open_dir(&dir, &this.download_dir, cx)),
            ),
        )
}

fn file_row(
    row: &DownloadRow,
    last: bool,
    p: &Palette,
    cx: &mut Context<TransfersView>,
) -> impl IntoElement {
    let id = row.id;
    let fraction = row.state.progress(row.size);
    let (status, color): (String, Hsla) = match &row.state {
        DlState::Queued {
            position: Some(position),
        } => (format!("queued #{position}"), p.text_weak),
        DlState::Queued { position: None } => ("queued".into(), p.text_weak),
        DlState::Active { speed, .. } => (format::speed(*speed), p.yolk),
        DlState::Paused { .. } => ("paused".into(), p.text_weak),
        DlState::Completed => ("done".into(), p.success),
        DlState::Cancelled => ("cancelled".into(), p.text_weaker),
        DlState::Failed(reason) => (reason.clone(), p.danger),
    };
    let amount = match &row.state {
        DlState::Active { done, .. } | DlState::Paused { done, .. } => {
            format!("{} / {}", format::bytes(*done), format::bytes(row.size))
        }
        _ => format::bytes(row.size),
    };
    let show_bar = matches!(row.state, DlState::Active { .. } | DlState::Paused { .. });

    let primary: Option<Button> = match &row.state {
        DlState::Active { .. } => Some(
            kit::icon_button(("pause", id), IconName::Pause, "pause")
                .on_click(cx.listener(move |this, _, _, cx| this.send(Command::Pause(id), cx))),
        ),
        DlState::Paused { .. } => Some(
            kit::icon_button(("resume", id), IconName::Play, "resume")
                .on_click(cx.listener(move |this, _, _, cx| this.send(Command::Resume(id), cx))),
        ),
        DlState::Failed(_) | DlState::Cancelled => Some(
            kit::icon_button(("retry", id), IconName::RotateCw, "retry")
                .on_click(cx.listener(move |this, _, _, cx| this.send(Command::Retry(id), cx))),
        ),
        _ => None,
    };
    let secondary = if row.state.is_live() {
        kit::icon_button(("cancel", id), IconName::X, "cancel")
            .on_click(cx.listener(move |this, _, _, cx| this.send(Command::Cancel(id), cx)))
    } else {
        kit::icon_button(("remove", id), IconName::Trash, "remove from list")
            .on_click(cx.listener(move |this, _, _, cx| this.send(Command::Remove(id), cx)))
    };

    let el = div()
        .id(("download", id as usize))
        .h(px(FILE_ROW))
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .pl(px(40.))
        .pr(px(14.))
        .border_t_1()
        .hover(|style| style.bg(p.bg_weak));
    edges(el, false, last, p)
        .child(
            Icon::new(match row.state {
                DlState::Completed => IconName::CircleCheck,
                DlState::Failed(_) => IconName::CircleAlert,
                _ => IconName::FileMusic,
            })
            .size(px(13.))
            .text_color(match row.state {
                DlState::Completed => p.success,
                DlState::Failed(_) => p.danger,
                _ => p.text_weaker,
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
                .when(show_bar, |this| {
                    let bar_color = if matches!(row.state, DlState::Paused { .. }) {
                        p.text_weaker
                    } else {
                        p.yolk
                    };
                    this.child(kit::progress_bar(fraction, bar_color, p))
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
                .w(px(60.))
                .flex_none()
                .flex()
                .justify_end()
                .children(primary)
                .child(secondary),
        )
}

impl Render for TransfersView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        self.refresh_layout(cx);
        let downloads = self.session.read(cx).downloads.clone();
        let active = downloads
            .iter()
            .filter(|row| matches!(row.state, DlState::Active { .. }))
            .count();
        let queued = downloads
            .iter()
            .filter(|row| matches!(row.state, DlState::Queued { .. }))
            .count();
        let done = downloads
            .iter()
            .filter(|row| row.state == DlState::Completed)
            .count();
        let failed = downloads
            .iter()
            .filter(|row| matches!(row.state, DlState::Failed(_)))
            .count();
        let mut subtitle = format!("{active} downloading · {queued} queued · {done} done");
        if failed > 0 {
            subtitle.push_str(&format!(" · {failed} failed"));
        }
        let finished = downloads
            .iter()
            .any(|row| matches!(row.state, DlState::Completed | DlState::Cancelled));

        let actions = div()
            .flex()
            .gap_2()
            .child(
                Button::new("open-downloads")
                    .outline()
                    .small()
                    .icon(Icon::new(IconName::ExternalLink))
                    .label("open downloads")
                    .on_click(cx.listener(|this, _, _, cx| {
                        open_dir(&this.download_dir, &this.download_dir, cx)
                    })),
            )
            .child(
                Button::new("clear-finished")
                    .outline()
                    .small()
                    .label("clear finished")
                    .disabled(!finished)
                    .on_click(cx.listener(|this, _, _, cx| this.send(Command::ClearFinished, cx))),
            );

        let body = match &self.layout {
            Some(layout) if !layout.rows.is_empty() => div()
                .flex_1()
                .min_h_0()
                .relative()
                .child(
                    v_virtual_list(cx.entity(), "downloads", layout.sizes.clone(), |this, range, _, cx| {
                        this.render_rows(range, cx)
                    })
                    .track_scroll(&self.scroll)
                    .pb_4(),
                )
                .vertical_scrollbar(&self.scroll)
                .into_any_element(),
            _ => kit::empty_state(
                IconName::Download,
                "nothing downloading yet",
                "find something under search, then press the download button next to a file or a folder.",
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
            .child(kit::page_header("transfers", subtitle, &p).child(actions))
            .child(body)
    }
}
