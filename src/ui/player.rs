use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::kit;
use crate::audio::{Audio, AudioCommand, AudioEvent};
use crate::format;
use crate::library::Track;
use crate::theme::palette;

const POLL: Duration = Duration::from_millis(200);
/// Pressing previous this far into a track starts it over instead of going back.
const RESTART_AFTER: Duration = Duration::from_secs(3);

/// What is playing and what comes next; drawn as the bar along the bottom of the window.
pub struct Playback {
    audio: Audio,
    queue: Vec<Track>,
    cover: Option<std::path::PathBuf>,
    index: Option<usize>,
    playing: bool,
    position: Duration,
    seeking: bool,
    error: Option<SharedString>,
    seek: Entity<SliderState>,
    volume: Entity<SliderState>,
    _subscriptions: Vec<Subscription>,
    _poll: Task<()>,
}

impl Playback {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let seek = cx.new(|_| SliderState::new().min(0.0).max(1.0).step(0.001));
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(1.0)
                .step(0.01)
                .default_value(0.8)
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &seek,
                window,
                |this, _, event: &SliderEvent, _, _| match event {
                    SliderEvent::Change(_) => this.seeking = true,
                    SliderEvent::Release(value) => {
                        this.seeking = false;
                        this.seek_to(value.start());
                    }
                },
            ),
            cx.subscribe_in(&volume, window, |this, _, event: &SliderEvent, _, _| {
                let (SliderEvent::Change(value) | SliderEvent::Release(value)) = event;
                this.audio.send(AudioCommand::Volume(value.start()));
            }),
        ];
        let poll = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                if this
                    .update_in(cx, |this, window, cx| this.drain(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let audio = Audio::spawn();
        audio.send(AudioCommand::Volume(0.8));
        Self {
            audio,
            queue: Vec::new(),
            cover: None,
            index: None,
            playing: false,
            position: Duration::ZERO,
            seeking: false,
            error: None,
            seek,
            volume,
            _subscriptions: subscriptions,
            _poll: poll,
        }
    }

    pub fn current(&self) -> Option<&Track> {
        self.queue.get(self.index?)
    }

    pub fn is_active(&self) -> bool {
        self.index.is_some()
    }

    /// Replaces the queue with an album's tracks and starts at `start`.
    pub fn play(
        &mut self,
        tracks: Vec<Track>,
        start: usize,
        cover: Option<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.queue = tracks;
        self.cover = cover;
        self.load(start, cx);
    }

    fn load(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(track) = self.queue.get(index) else {
            self.stop(cx);
            return;
        };
        self.audio.send(AudioCommand::Load(track.path.clone()));
        self.index = Some(index);
        self.playing = true;
        self.position = Duration::ZERO;
        self.error = None;
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.audio.send(AudioCommand::Stop);
        self.playing = false;
        self.position = Duration::ZERO;
        cx.notify();
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.index.is_none() {
            return;
        }
        self.playing = !self.playing;
        self.audio.send(if self.playing {
            AudioCommand::Play
        } else {
            AudioCommand::Pause
        });
        cx.notify();
    }

    pub fn next(&mut self, cx: &mut Context<Self>) {
        match self.index {
            Some(index) if index + 1 < self.queue.len() => self.load(index + 1, cx),
            _ => self.stop(cx),
        }
    }

    pub fn previous(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.index else {
            return;
        };
        if self.position > RESTART_AFTER || index == 0 {
            self.load(index, cx);
        } else {
            self.load(index - 1, cx);
        }
    }

    fn seek_to(&mut self, fraction: f32) {
        let Some(track) = self.current() else {
            return;
        };
        let target = Duration::from_secs_f32(track.duration as f32 * fraction.clamp(0.0, 1.0));
        self.audio.send(AudioCommand::Seek(target));
        self.position = target;
    }

    fn drain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut changed = false;
        while let Some(event) = self.audio.try_event() {
            changed = true;
            match event {
                AudioEvent::Position(position) => self.position = position,
                AudioEvent::Ended => self.next(cx),
                AudioEvent::Failed(error) => {
                    let name = self
                        .current()
                        .map(|track| track.title.clone())
                        .unwrap_or_default();
                    self.error = Some(format!("{name}: {error}").into());
                    self.playing = false;
                }
            }
        }
        if changed && !self.seeking {
            let duration = self.current().map_or(0, |track| track.duration).max(1);
            let fraction = self.position.as_secs_f32() / duration as f32;
            self.seek
                .update(cx, |seek, cx| seek.set_value(fraction.min(1.0), window, cx));
            cx.notify();
        }
    }
}

fn clock(seconds: u64) -> String {
    format::duration(seconds as u32)
}

impl Render for Playback {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let Some(track) = self.current().cloned() else {
            return div().into_any_element();
        };
        let artist = if track.artist.is_empty() {
            track.album.clone()
        } else {
            format!("{} · {}", track.artist, track.album)
        };
        let cover = match &self.cover {
            Some(path) => img(path.clone())
                .size(px(44.))
                .flex_none()
                .rounded(px(4.))
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
            None => div()
                .size(px(44.))
                .flex_none()
                .rounded(px(4.))
                .bg(p.bg_hover)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(IconName::Music)
                        .size(px(18.))
                        .text_color(p.text_weaker),
                )
                .into_any_element(),
        };
        div()
            .h(px(64.))
            .flex_none()
            .flex()
            .items_center()
            .gap_4()
            .px_4()
            .border_t_1()
            .border_color(p.border_weak)
            .bg(p.bg_weak)
            .child(
                div()
                    .w(px(280.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(cover)
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(kit::truncate(track.title.clone()).text_color(p.text_strong))
                            .child(match &self.error {
                                Some(error) => kit::truncate(error.clone())
                                    .text_size(px(12.))
                                    .text_color(p.danger),
                                None => kit::truncate(artist)
                                    .text_size(px(12.))
                                    .text_color(p.text_weak),
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        kit::icon_button("previous", IconName::SkipBack, "previous")
                            .on_click(cx.listener(|this, _, _, cx| this.previous(cx))),
                    )
                    .child(
                        kit::icon_button(
                            "play-pause",
                            if self.playing {
                                IconName::Pause
                            } else {
                                IconName::Play
                            },
                            "play or pause",
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle(cx))),
                    )
                    .child(
                        kit::icon_button("next", IconName::SkipForward, "next")
                            .on_click(cx.listener(|this, _, _, cx| this.next(cx))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_3()
                    .text_size(px(12.))
                    .text_color(p.text_weak)
                    .child(clock(self.position.as_secs()))
                    .child(div().flex_1().child(Slider::new(&self.seek)))
                    .child(clock(u64::from(track.duration))),
            )
            .child(
                div()
                    .w(px(140.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::new(IconName::Volume2)
                            .size(px(15.))
                            .text_color(p.icon),
                    )
                    .child(div().flex_1().child(Slider::new(&self.volume))),
            )
            .when(!self.playing && self.error.is_none(), |this| {
                this.opacity(0.9)
            })
            .into_any_element()
    }
}
