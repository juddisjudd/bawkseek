use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use chrono::Local;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Icon, Sizable, VirtualListScrollHandle, v_virtual_list};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::covers::CoverCache;
use super::kit;
use super::player::Playback;
use crate::format;
use crate::library::{self, Album, Library, Section, Sort};
use crate::theme::{Palette, palette};

const CARD: f32 = 160.;
const GAP: f32 = 20.;
const ROW: f32 = CARD + 64.;
const HEADING: f32 = 44.;
/// Roughly how wide one character of a heading draws, to give a group room for its name.
const HEADING_CHAR: f32 = 8.;
/// Covers kept decoded at once; a screenful is a few dozen.
const COVER_CACHE: usize = 160;
const RESCAN_DELAY: std::time::Duration = std::time::Duration::from_secs(5);
/// The sidebar and page padding, which the album grid cannot use.
const CHROME: f32 = 208. + 80.;

pub struct SortChanged(pub Sort);

/// Albums under one heading, `slots` grid columns wide.
struct Group {
    heading: SharedString,
    count: usize,
    albums: Vec<usize>,
    slots: usize,
}

/// One row of the grid: headed groups side by side, or a group's albums that did not fit its first row.
enum Line {
    Groups(Vec<Group>),
    Cards(Vec<usize>),
}

impl Line {
    fn height(&self) -> f32 {
        match self {
            Line::Groups(_) => HEADING + ROW,
            Line::Cards(_) => ROW,
        }
    }
}

fn slots_for(heading: &str, albums: usize, cols: usize) -> usize {
    let count = if albums > 1 { 80. } else { 0. };
    let text = heading.chars().count() as f32 * HEADING_CHAR + count;
    let slots = ((text + GAP) / (CARD + GAP)).ceil() as usize;
    slots.max(albums).clamp(1, cols)
}

/// Small groups share a row so a library of one-album artists does not spend a row on each.
fn lines(sections: Vec<Section>, cols: usize) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut row: Vec<Group> = Vec::new();
    let mut used = 0;
    for section in sections {
        if section.heading.is_empty() {
            lines.extend(section.albums.chunks(cols).map(|r| Line::Cards(r.to_vec())));
            continue;
        }
        let count = section.albums.len();
        let slots = slots_for(&section.heading, count, cols);
        if used + slots > cols {
            lines.push(Line::Groups(std::mem::take(&mut row)));
            used = 0;
        }
        let mut albums = section.albums;
        let rest = albums.split_off(albums.len().min(cols));
        row.push(Group {
            heading: section.heading.into(),
            count,
            albums,
            slots,
        });
        used += slots;
        if !rest.is_empty() {
            lines.push(Line::Groups(std::mem::take(&mut row)));
            used = 0;
            lines.extend(rest.chunks(cols).map(|r| Line::Cards(r.to_vec())));
        }
    }
    if !row.is_empty() {
        lines.push(Line::Groups(row));
    }
    lines
}

pub struct LibraryView {
    playback: Entity<Playback>,
    library: Option<Arc<Library>>,
    scanning: bool,
    roots: Vec<PathBuf>,
    open: Option<usize>,
    sort: Sort,
    filter: Entity<InputState>,
    covers: Entity<CoverCache>,
    scroll: VirtualListScrollHandle,
    rescan: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SortChanged> for LibraryView {}

impl LibraryView {
    pub fn new(
        playback: Entity<Playback>,
        sort: Sort,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx
            .new(|cx| InputState::new(window, cx).placeholder("filter by artist, album or track"));
        let subscriptions = vec![
            cx.subscribe_in(&filter, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.observe(&playback, |_, _, cx| cx.notify()),
        ];
        Self {
            playback,
            library: None,
            scanning: false,
            roots: Vec::new(),
            open: None,
            sort,
            filter,
            covers: CoverCache::new(COVER_CACHE, cx),
            scroll: VirtualListScrollHandle::new(),
            rescan: None,
            _subscriptions: subscriptions,
        }
    }

    /// The folders the library is built from; a change takes effect at the next scan.
    pub fn set_roots(&mut self, roots: Vec<PathBuf>) {
        self.roots = roots;
    }

    /// Scans the first time the page is shown, so the library costs nothing until it is used.
    pub fn ensure_scanned(&mut self, cx: &mut Context<Self>) {
        if self.library.is_none() {
            self.scan(cx);
        }
    }

    /// Rescans a few seconds after downloads finish, once per burst, and only if the library was ever opened.
    pub fn rescan_soon(&mut self, cx: &mut Context<Self>) {
        if self.library.is_none() || self.rescan.is_some() {
            return;
        }
        self.rescan = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RESCAN_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                this.rescan = None;
                this.scan(cx);
            });
        }));
    }

    fn scan(&mut self, cx: &mut Context<Self>) {
        if self.scanning {
            return;
        }
        let Some(data) = crate::config::data_dir() else {
            return;
        };
        self.scanning = true;
        cx.notify();
        let roots = self.roots.clone();
        cx.spawn(async move |this, cx| {
            let scanned = cx
                .background_executor()
                .spawn(async move { library::scan(&roots, &data) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.scanning = false;
                let open = this.open.and_then(|ix| {
                    let album = this.library.as_ref()?.albums.get(ix)?;
                    Some((album.artist.clone(), album.title.clone()))
                });
                this.open = open.and_then(|(artist, title)| {
                    scanned
                        .albums
                        .iter()
                        .position(|album| album.artist == artist && album.title == title)
                });
                this.library = Some(Arc::new(scanned));
                cx.notify();
            });
        })
        .detach();
    }

    fn visible(&self, cx: &App) -> Vec<usize> {
        let Some(library) = &self.library else {
            return Vec::new();
        };
        let filter = self.filter.read(cx).value().trim().to_lowercase();
        let words: Vec<&str> = filter.split_whitespace().collect();
        (0..library.albums.len())
            .filter(|ix| {
                let album = &library.albums[*ix];
                let text = format!("{} {}", album.artist, album.title).to_lowercase();
                words.iter().all(|word| {
                    text.contains(word)
                        || album
                            .tracks
                            .iter()
                            .any(|track| track.title.to_lowercase().contains(word))
                })
            })
            .collect()
    }

    fn set_sort(&mut self, sort: Sort, cx: &mut Context<Self>) {
        if sort == self.sort {
            return;
        }
        self.sort = sort;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.emit(SortChanged(sort));
        cx.notify();
    }

    fn play(&mut self, album: usize, start: usize, cx: &mut Context<Self>) {
        let Some(album) = self
            .library
            .as_ref()
            .and_then(|library| library.albums.get(album))
        else {
            return;
        };
        let (tracks, cover) = (album.tracks.clone(), album.cover.clone());
        self.playback
            .update(cx, |playback, cx| playback.play(tracks, start, cover, cx));
    }

    fn render_rows(
        &mut self,
        range: std::ops::Range<usize>,
        lines: Rc<Vec<Line>>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = palette(cx);
        let Some(library) = self.library.clone() else {
            return Vec::new();
        };
        let cards = |albums: &[usize], cx: &mut Context<Self>| {
            div()
                .h(px(ROW))
                .flex()
                .gap(px(GAP))
                .children(albums.iter().map(|&ix| {
                    album_card(&library.albums[ix], ix, &p).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.open = Some(ix);
                            cx.notify();
                        },
                    ))
                }))
        };
        range
            .map(|row| match &lines[row] {
                Line::Groups(groups) => div()
                    .h(px(HEADING + ROW))
                    .flex()
                    .gap(px(GAP))
                    .children(groups.iter().map(|group| {
                        div()
                            .w(px(group.slots as f32 * (CARD + GAP) - GAP))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .child(heading(group.heading.clone(), group.count, &p))
                            .child(cards(&group.albums, cx))
                    }))
                    .into_any_element(),
                Line::Cards(albums) => cards(albums, cx).into_any_element(),
            })
            .collect()
    }

    fn render_album(&self, ix: usize, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(album) = self
            .library
            .as_ref()
            .and_then(|library| library.albums.get(ix))
        else {
            return div().into_any_element();
        };
        let playing = self
            .playback
            .read(cx)
            .current()
            .map(|track| track.path.clone());
        let details = [
            album.year.map(|year| year.to_string()),
            Some(format::plural(album.tracks.len(), "track", "tracks")),
            Some(format::duration(album.duration)),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        div()
            .id("album")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_5()
            .pb_6()
            .child(
                div().flex().child(
                    kit::button("back", cx)
                        .small()
                        .icon(Icon::new(IconName::ArrowLeft))
                        .label("all albums")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open = None;
                            cx.notify();
                        })),
                ),
            )
            .child(
                div()
                    .flex()
                    .gap_5()
                    .items_end()
                    .child(cover(album, 180., p))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p.text_strong)
                                    .child(album.title.clone()),
                            )
                            .child(div().text_color(p.text).child(album.artist.clone()))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(p.text_weak)
                                    .child(details),
                            )
                            .child(
                                div().pt_2().flex().child(
                                    kit::button("play-album", cx)
                                        .small()
                                        .icon(Icon::new(IconName::Play))
                                        .label("play")
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| this.play(ix, 0, cx)),
                                        ),
                                ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .children(album.tracks.iter().enumerate().map(|(n, track)| {
                        let current = playing.as_ref() == Some(&track.path);
                        let number = track
                            .track
                            .map_or_else(|| (n + 1).to_string(), |t| t.to_string());
                        let show_artist = !track.artist.is_empty() && track.artist != album.artist;
                        div()
                            .id(("track", n))
                            .h(px(34.))
                            .flex()
                            .items_center()
                            .gap_3()
                            .px_3()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .when(current, |this| this.bg(p.yolk_dim))
                            .hover(|style| style.bg(p.bg_hover))
                            .child(
                                div()
                                    .w(px(28.))
                                    .flex_none()
                                    .text_color(if current { p.yolk } else { p.text_weaker })
                                    .child(number),
                            )
                            .child(
                                kit::truncate(track.title.clone())
                                    .flex_1()
                                    .text_color(if current { p.text_strong } else { p.text }),
                            )
                            .when(show_artist, |this| {
                                this.child(
                                    kit::truncate(track.artist.clone())
                                        .w(px(200.))
                                        .flex_none()
                                        .text_color(p.text_weak),
                                )
                            })
                            .child(
                                div()
                                    .w(px(56.))
                                    .flex_none()
                                    .flex()
                                    .justify_end()
                                    .text_color(p.text_weak)
                                    .child(format::duration(track.duration)),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| this.play(ix, n, cx)))
                    })),
            )
            .into_any_element()
    }
}

fn cover(album: &Album, size: f32, p: &Palette) -> AnyElement {
    match &album.cover {
        Some(path) => img(path.clone())
            .size(px(size))
            .flex_none()
            .rounded(px(4.))
            .object_fit(ObjectFit::Cover)
            .into_any_element(),
        None => div()
            .size(px(size))
            .flex_none()
            .rounded(px(4.))
            .bg(p.bg_hover)
            .flex()
            .items_center()
            .justify_center()
            .child(
                Icon::new(IconName::Music)
                    .size(px(size / 4.))
                    .text_color(p.text_weaker),
            )
            .into_any_element(),
    }
}

fn heading(name: SharedString, count: usize, p: &Palette) -> Div {
    div()
        .h(px(HEADING - 12.))
        .mb(px(12.))
        .flex()
        .items_end()
        .gap_3()
        .pb(px(8.))
        .border_b_1()
        .border_color(p.border_weak)
        .child(
            kit::truncate(name)
                .text_color(p.text_strong)
                .font_weight(FontWeight::SEMIBOLD),
        )
        .when(count > 1, |this| {
            this.child(
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(p.text_weaker)
                    .child(format::plural(count, "album", "albums")),
            )
        })
}

fn album_card(album: &Album, ix: usize, p: &Palette) -> Stateful<Div> {
    div()
        .id(("album", ix))
        .w(px(CARD))
        .flex_none()
        .flex()
        .flex_col()
        .gap_2()
        .cursor_pointer()
        .child(cover(album, CARD, p))
        .child(
            div()
                .flex()
                .flex_col()
                .child(kit::truncate(album.title.clone()).text_color(p.text_strong))
                .child(
                    kit::truncate(album.artist.clone())
                        .text_size(px(12.))
                        .text_color(p.text_weak),
                ),
        )
}

impl Render for LibraryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let subtitle = match (&self.library, self.scanning) {
            (_, true) => "reading your music…".to_string(),
            (Some(library), false) => format!(
                "{} · {}",
                format::plural(library.albums.len(), "album", "albums"),
                format::plural(library.tracks, "track", "tracks")
            ),
            (None, false) => "your downloads and shared folders".to_string(),
        };
        let header = div()
            .flex()
            .items_start()
            .justify_between()
            .child(kit::page_header("library", subtitle, &p))
            .child(
                kit::button("rescan", cx)
                    .small()
                    .icon(Icon::new(IconName::RefreshCw))
                    .label("rescan")
                    .disabled(self.scanning)
                    .on_click(cx.listener(|this, _, _, cx| this.scan(cx))),
            );

        let body = if let Some(ix) = self.open {
            self.render_album(ix, &p, cx)
        } else {
            let visible = self.visible(cx);
            let available = (window.viewport_size().width.as_f32() - CHROME).max(CARD);
            let cols = (((available + GAP) / (CARD + GAP)) as usize).max(1);
            let sections = match &self.library {
                Some(library) => library::arrange(
                    &library.albums,
                    &visible,
                    self.sort,
                    &self.roots,
                    Local::now(),
                ),
                None => Vec::new(),
            };
            let lines = Rc::new(lines(sections, cols));
            let sizes = Rc::new(
                lines
                    .iter()
                    .map(|line| size(px(1.), px(line.height())))
                    .collect::<Vec<_>>(),
            );
            let list = if visible.is_empty() {
                let (title, body) = if self.scanning || self.library.is_none() {
                    (
                        "reading your music",
                        "albums show up here once the scan is done.",
                    )
                } else if self.library.as_ref().is_some_and(|l| l.albums.is_empty()) {
                    (
                        "no music found",
                        "download some albums, or add shared folders under uploads.",
                    )
                } else {
                    ("nothing matches", "try other words.")
                };
                kit::empty_state(IconName::Library, title, body, &p).into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .image_cache(self.covers.clone())
                    .child(
                        v_virtual_list(cx.entity(), "albums", sizes, move |this, range, _, cx| {
                            this.render_rows(range, lines.clone(), cx)
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
                .gap_4()
                .child(
                    div()
                        .pb(px(8.))
                        .border_b_1()
                        .border_color(p.border_weak)
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div().flex_1().min_w_0().child(
                                kit::input(&self.filter)
                                    .appearance(false)
                                    .cleanable(true)
                                    .prefix(
                                        Icon::new(IconName::Search).small().text_color(p.text_weak),
                                    ),
                            ),
                        )
                        .child(div().flex_none().text_color(p.text_weaker).child("sort by"))
                        .child(kit::segmented(&p).children(Sort::ALL.map(|sort| {
                            kit::segment(sort.label(), sort.label(), sort == self.sort, &p)
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.set_sort(sort, cx)),
                                )
                        }))),
                )
                .child(list)
                .into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .px(px(40.))
            .pt(px(32.))
            .image_cache(self.covers.clone())
            .child(header)
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::{Line, lines};
    use crate::library::Section;

    fn section(heading: &str, albums: std::ops::Range<usize>) -> Section {
        Section {
            heading: heading.into(),
            albums: albums.collect(),
        }
    }

    fn shape(lines: &[Line]) -> Vec<Vec<usize>> {
        lines
            .iter()
            .map(|line| match line {
                Line::Groups(groups) => groups.iter().map(|group| group.albums.len()).collect(),
                Line::Cards(albums) => vec![albums.len()],
            })
            .collect()
    }

    #[test]
    fn packs_small_groups_into_one_row_and_wraps_big_ones() {
        let packed = lines(
            vec![
                section("Abe", 0..1),
                section("Bo", 1..3),
                section("Cy", 3..4),
                section("Di", 4..11),
            ],
            5,
        );
        assert_eq!(shape(&packed), [vec![1, 2, 1], vec![5], vec![2]]);
        assert!(matches!(packed[2], Line::Cards(_)));
    }

    #[test]
    fn gives_long_headings_room() {
        let packed = lines(
            vec![
                section(r"Music\Rock\Some Long Folder", 0..1),
                section("Ed", 1..2),
            ],
            3,
        );
        let Line::Groups(groups) = &packed[0] else {
            panic!("expected a headed row");
        };
        assert_eq!(groups[0].slots, 2);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn leaves_ungrouped_albums_as_plain_rows() {
        let packed = lines(vec![section("", 0..7)], 3);
        assert_eq!(shape(&packed), [vec![3], vec![3], vec![1]]);
    }
}
