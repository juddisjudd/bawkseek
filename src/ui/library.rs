use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

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
use crate::library::{self, Album, Library};
use crate::theme::{Palette, palette};

const CARD: f32 = 160.;
const GAP: f32 = 20.;
const ROW: f32 = CARD + 64.;
/// Covers kept decoded at once; a screenful is a few dozen.
const COVER_CACHE: usize = 160;
/// The sidebar and page padding, which the album grid cannot use.
const CHROME: f32 = 208. + 80.;

pub struct LibraryView {
    playback: Entity<Playback>,
    library: Option<Arc<Library>>,
    scanning: bool,
    roots: Vec<PathBuf>,
    open: Option<usize>,
    filter: Entity<InputState>,
    covers: Entity<CoverCache>,
    scroll: VirtualListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl LibraryView {
    pub fn new(playback: Entity<Playback>, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            filter,
            covers: CoverCache::new(COVER_CACHE, cx),
            scroll: VirtualListScrollHandle::new(),
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
                this.open = None;
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
        cols: usize,
        visible: Rc<Vec<usize>>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = palette(cx);
        let Some(library) = self.library.clone() else {
            return Vec::new();
        };
        range
            .map(|row| {
                let start = row * cols;
                let end = (start + cols).min(visible.len());
                div()
                    .h(px(ROW))
                    .flex()
                    .gap(px(GAP))
                    .children(visible[start..end].iter().map(|ix| {
                        let ix = *ix;
                        album_card(&library.albums[ix], ix, &p).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.open = Some(ix);
                                cx.notify();
                            },
                        ))
                    }))
                    .into_any_element()
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
            let visible = Rc::new(self.visible(cx));
            let available = (window.viewport_size().width.as_f32() - CHROME).max(CARD);
            let cols = (((available + GAP) / (CARD + GAP)) as usize).max(1);
            let rows = visible.len().div_ceil(cols);
            let sizes = Rc::new(vec![size(px(1.), px(ROW)); rows]);
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
                            this.render_rows(range, cols, visible.clone(), cx)
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
                        .child(
                            kit::input(&self.filter)
                                .appearance(false)
                                .cleanable(true)
                                .prefix(
                                    Icon::new(IconName::Search).small().text_color(p.text_weak),
                                ),
                        ),
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
