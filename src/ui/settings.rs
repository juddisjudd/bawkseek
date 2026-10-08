use std::path::PathBuf;

use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::*;

use super::kit;
use crate::config::Config;
use crate::theme::{Palette, palette};

pub enum SettingsEvent {
    DownloadDir(PathBuf),
    ListenPort(u16),
    LightTheme(bool),
    Logout,
}

pub struct SettingsView {
    username: SharedString,
    light_theme: bool,
    download_dir: Entity<InputState>,
    listen_port: Entity<InputState>,
    port_error: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    pub fn new(config: &Config, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let download_dir = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(config.download_dir.to_string_lossy().into_owned())
        });
        let listen_port =
            cx.new(|cx| InputState::new(window, cx).default_value(config.listen_port.to_string()));
        let subscriptions = vec![
            cx.subscribe_in(
                &download_dir,
                window,
                |this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        this.commit_dir(cx);
                    }
                },
            ),
            cx.subscribe_in(
                &listen_port,
                window,
                |this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        this.commit_port(cx);
                    }
                },
            ),
        ];
        Self {
            username: config.username.clone().into(),
            light_theme: config.light_theme,
            download_dir,
            listen_port,
            port_error: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_username(&mut self, username: SharedString) {
        self.username = username;
    }

    fn commit_dir(&mut self, cx: &mut Context<Self>) {
        let value = self.download_dir.read(cx).value().trim().to_string();
        if !value.is_empty() {
            cx.emit(SettingsEvent::DownloadDir(PathBuf::from(value)));
        }
    }

    fn commit_port(&mut self, cx: &mut Context<Self>) {
        let value = self.listen_port.read(cx).value();
        match value.trim().parse::<u16>() {
            Ok(port) if port >= 1024 => {
                self.port_error = false;
                cx.emit(SettingsEvent::ListenPort(port));
            }
            _ => self.port_error = true,
        }
        cx.notify();
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("use this folder".into()),
        });
        let input = self.download_dir.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                input.update(cx, |state, cx| {
                    state.set_value(path.to_string_lossy().into_owned(), window, cx)
                });
                this.commit_dir(cx);
            });
        })
        .detach();
    }
}

fn section(title: &'static str, p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .pb_6()
        .border_b_1()
        .border_color(p.border_weak)
        .child(
            div()
                .text_color(p.text_strong)
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
}

fn field(label: &'static str, hint: &'static str, control: impl IntoElement, p: &Palette) -> Div {
    div()
        .flex()
        .gap_6()
        .child(
            div()
                .w(px(220.))
                .flex_none()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_color(p.text).child(label))
                .child(div().text_size(px(12.)).text_color(p.text_weak).child(hint)),
        )
        .child(div().flex_1().min_w_0().max_w(px(560.)).child(control))
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let port_hint = if self.port_error {
            "use a number from 1024 to 65535"
        } else {
            "peers connect to you on this port. takes effect at next login."
        };

        div()
            .id("settings")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_6()
            .px(px(40.))
            .pt(px(32.))
            .pb_8()
            .child(kit::page_header("settings", "saved as you change them", &p))
            .child(
                section("account", &p).child(field(
                    "logged in as",
                    "logging out stops every transfer",
                    div()
                        .flex()
                        .items_center()
                        .gap_4()
                        .child(kit::strong(self.username.clone(), &p))
                        .child(
                            Button::new("logout")
                                .outline()
                                .small()
                                .label("log out")
                                .on_click(
                                    cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Logout)),
                                ),
                        ),
                    &p,
                )),
            )
            .child(
                section("downloads", &p)
                    .child(field(
                        "download folder",
                        "each album gets its own folder inside",
                        div()
                            .flex()
                            .gap_2()
                            .child(div().flex_1().child(Input::new(&self.download_dir)))
                            .child(Button::new("browse").outline().label("browse…").on_click(
                                cx.listener(|this, _, window, cx| this.browse(window, cx)),
                            )),
                        &p,
                    ))
                    .child(field(
                        "listening port",
                        port_hint,
                        div().w(px(120.)).child(Input::new(&self.listen_port)),
                        &p,
                    )),
            )
            .child(
                section("appearance", &p).child(field(
                    "light theme",
                    "dark is the default",
                    Switch::new("light-theme")
                        .checked(self.light_theme)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.light_theme = *checked;
                            cx.emit(SettingsEvent::LightTheme(*checked));
                            cx.notify();
                        })),
                    &p,
                )),
            )
    }
}
