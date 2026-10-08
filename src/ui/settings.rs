use std::path::PathBuf;

use gpui_kit::component::Sizable;
use gpui_kit::component::WindowExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::kit;
use crate::config::Config;
use crate::net::{PortMap, Session};
use crate::theme::{self, DEFAULT_THEME, Look, Mode, Palette, palette};

pub enum SettingsEvent {
    DownloadDir(PathBuf),
    ListenPort(u16),
    Theme(SharedString, Mode),
    Away(bool),
    Upnp(bool),
    DownloadLimit(u64),
    ChangePassword(String),
    Logout,
}

pub struct SettingsView {
    session: Entity<Session>,
    username: SharedString,
    theme: SharedString,
    mode: Mode,
    away: bool,
    upnp: bool,
    download_dir: Entity<InputState>,
    listen_port: Entity<InputState>,
    download_limit: Entity<InputState>,
    new_password: Entity<InputState>,
    port_error: bool,
    limit_error: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    pub fn new(
        config: &Config,
        session: Entity<Session>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let download_dir = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(config.download_dir.to_string_lossy().into_owned())
        });
        let listen_port =
            cx.new(|cx| InputState::new(window, cx).default_value(config.listen_port.to_string()));
        let download_limit = cx
            .new(|cx| InputState::new(window, cx).default_value(config.download_limit.to_string()));
        let new_password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("new password")
                .masked(true)
        });
        let commit = |this: &mut Self,
                      input: &Entity<InputState>,
                      event: &InputEvent,
                      _: &mut Window,
                      cx: &mut Context<Self>| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.commit(input, cx);
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(&download_dir, window, commit),
            cx.subscribe_in(&listen_port, window, commit),
            cx.subscribe_in(&download_limit, window, commit),
            cx.observe(&session, |_, _, cx| cx.notify()),
        ];
        Self {
            session,
            username: config.username.clone().into(),
            theme: config.theme.clone().into(),
            mode: config.mode,
            away: false,
            upnp: config.upnp,
            download_dir,
            listen_port,
            download_limit,
            new_password,
            port_error: false,
            limit_error: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_username(&mut self, username: SharedString) {
        self.username = username;
        self.away = false;
    }

    fn commit(&mut self, input: &Entity<InputState>, cx: &mut Context<Self>) {
        let value = input.read(cx).value().trim().to_string();
        if *input == self.download_dir {
            if !value.is_empty() {
                cx.emit(SettingsEvent::DownloadDir(PathBuf::from(value)));
            }
        } else if *input == self.listen_port {
            match value.parse::<u16>() {
                Ok(port) if port >= 1024 => {
                    self.port_error = false;
                    cx.emit(SettingsEvent::ListenPort(port));
                }
                _ => self.port_error = true,
            }
        } else if *input == self.download_limit {
            match value.parse::<u64>() {
                Ok(limit) => {
                    self.limit_error = false;
                    cx.emit(SettingsEvent::DownloadLimit(limit));
                }
                Err(_) => self.limit_error = true,
            }
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
                this.commit(&input, cx);
            });
        })
        .detach();
    }

    fn pick_theme(&mut self, id: SharedString, mode: Mode, cx: &mut Context<Self>) {
        self.theme = id.clone();
        self.mode = mode;
        cx.emit(SettingsEvent::Theme(id, mode));
        cx.notify();
    }

    fn render_modes(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let own = self.theme == DEFAULT_THEME;
        div()
            .flex()
            .flex_none()
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(4.))
            .border_1()
            .border_color(p.border_weak)
            .children(
                [
                    (Mode::System, "system"),
                    (Mode::Dark, "dark"),
                    (Mode::Light, "light"),
                ]
                .map(|(mode, label)| {
                    let active = own && self.mode == mode;
                    div()
                        .id(label)
                        .px(px(12.))
                        .py(px(3.))
                        .rounded(px(3.))
                        .cursor_pointer()
                        .text_color(if active { p.text_strong } else { p.text_weak })
                        .when(active, |this| this.bg(p.bg_hover))
                        .hover(|style| style.text_color(p.text_strong))
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.pick_theme(DEFAULT_THEME.into(), mode, cx)
                        }))
                }),
            )
    }

    fn render_looks(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let looks = theme::looks(cx).to_vec();
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .children(looks.into_iter().map(|look| {
                let active = look.id == self.theme;
                let (bg, fg, cursor) = if look.id == DEFAULT_THEME && active {
                    (p.bg, p.text_strong, p.yolk)
                } else {
                    (look.bg, look.fg, look.cursor)
                };
                look_card(&look, bg, fg, cursor, active, p).on_click(cx.listener({
                    let id = look.id.clone();
                    move |this, _, _, cx| {
                        let mode = this.mode;
                        this.pick_theme(id.clone(), mode, cx)
                    }
                }))
            }))
    }

    fn change_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_password
            .update(cx, |state, cx| state.set_value("", window, cx));
        let input = self.new_password.clone();
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let (input, view) = (input.clone(), view.clone());
            dialog
                .title("change password")
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child("the server changes it right away. remembered passwords are updated too.")
                        .child(kit::input(&input).mask_toggle()),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            kit::button("cancel-password", cx)
                                .label("cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("confirm-password")
                                .primary()
                                .label("change")
                                .on_click(move |_, window, cx| {
                                    let password = input.read(cx).value().to_string();
                                    if password.is_empty() {
                                        return;
                                    }
                                    let _ = view.update(cx, |_, cx| {
                                        cx.emit(SettingsEvent::ChangePassword(password))
                                    });
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
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

fn field(
    label: &'static str,
    hint: impl Into<SharedString>,
    control: impl IntoElement,
    p: &Palette,
) -> Div {
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
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.text_weak)
                        .child(hint.into()),
                ),
        )
        .child(div().flex_1().min_w_0().max_w(px(560.)).child(control))
}

fn look_card(
    look: &Look,
    bg: Hsla,
    fg: Hsla,
    cursor: Hsla,
    active: bool,
    p: &Palette,
) -> Stateful<Div> {
    let mut words: Vec<&str> = look.label.split(' ').collect();
    let last = words.pop().unwrap_or_default().to_string();
    div()
        .id(SharedString::from(format!("look-{}", look.id)))
        .w(px(150.))
        .flex()
        .flex_col()
        .gap_2()
        .p(px(10.))
        .rounded(px(4.))
        .border_2()
        .border_color(if active { p.yolk } else { p.border_weak })
        .cursor_pointer()
        .bg(bg)
        .text_color(fg)
        .text_size(px(12.))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(px(7.))
                .children(words.into_iter().map(|word| div().child(word.to_string())))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(last)
                        .child(div().w(px(7.)).h(px(13.)).bg(cursor)),
                ),
        )
        .child(
            div().flex().gap(px(3.)).children(
                look.swatches
                    .map(|color| div().size(px(12.)).rounded(px(2.)).bg(color)),
            ),
        )
}

fn portmap_status(state: &PortMap) -> String {
    match state {
        PortMap::Off => "off. forward the port on your router yourself.".into(),
        PortMap::Searching => "looking for a router that supports upnp…".into(),
        PortMap::Mapped {
            port,
            external: Some(ip),
        } => format!("open: other users reach you at {ip}:{port}"),
        PortMap::Mapped {
            port,
            external: None,
        } => format!("open on port {port}"),
        PortMap::NoRouter => {
            "no upnp router answered. forward the port on your router yourself.".into()
        }
        PortMap::Failed(reason) => format!("the router refused: {reason}"),
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let session = self.session.read(cx);
        let portmap = portmap_status(&session.portmap);
        let privileges = match session.privileges {
            None => "not known yet".to_string(),
            Some(0) => "none. privileged users go first in upload queues.".to_string(),
            Some(seconds) => format!(
                "privileged for {}",
                crate::format::plural((seconds / 86_400).max(1) as usize, "more day", "more days")
            ),
        };
        let port_hint = if self.port_error {
            "use a number from 1024 to 65535"
        } else {
            "peers connect to you on this port. takes effect at next login."
        };
        let theme_hint: SharedString = if self.theme == DEFAULT_THEME {
            "system follows windows".into()
        } else {
            let label = theme::looks(cx)
                .iter()
                .find(|look| look.id == self.theme)
                .map(|look| look.label.clone())
                .unwrap_or_default();
            format!("set by {label}. pick one to go back to bawk.").into()
        };
        let limit_hint = if self.limit_error {
            "use a whole number, or 0 for no limit"
        } else {
            "kilobytes per second for all downloads together. 0 means no limit."
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
                section("account", &p)
                    .child(field(
                        "logged in as",
                        "logging out stops every transfer",
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(kit::strong(self.username.clone(), &p))
                            .child(
                                kit::button("logout", cx)
                                    .small()
                                    .label("log out")
                                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Logout))),
                            )
                            .child(
                                kit::button("password", cx)
                                    .small()
                                    .label("change password…")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.change_password(window, cx)
                                    })),
                            ),
                        &p,
                    ))
                    .child(field(
                        "away",
                        "tells other users you are not at the keyboard. resets when you log in again.",
                        Switch::new("away")
                            .checked(self.away)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.away = *checked;
                                cx.emit(SettingsEvent::Away(*checked));
                                cx.notify();
                            })),
                        &p,
                    ))
                    .child(field(
                        "privileges",
                        "bought on slsknet.org; the server reports them at login",
                        div().text_color(p.text).child(privileges),
                        &p,
                    )),
            )
            .child(
                section("network", &p)
                    .child(field(
                        "listening port",
                        port_hint,
                        div().w(px(120.)).child(kit::input(&self.listen_port)),
                        &p,
                    ))
                    .child(field(
                        "open the port automatically",
                        "asks your router over upnp. most home routers allow it.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                Switch::new("upnp")
                                    .checked(self.upnp)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.upnp = *checked;
                                        cx.emit(SettingsEvent::Upnp(*checked));
                                        cx.notify();
                                    })),
                            )
                            .child(div().text_size(px(12.)).text_color(p.text_weak).child(portmap)),
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
                            .child(div().flex_1().child(kit::input(&self.download_dir)))
                            .child(kit::button("browse", cx).label("browse…").on_click(
                                cx.listener(|this, _, window, cx| this.browse(window, cx)),
                            )),
                        &p,
                    ))
                    .child(field(
                        "speed limit",
                        limit_hint,
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().w(px(120.)).child(kit::input(&self.download_limit)))
                            .child(div().text_color(p.text_weak).child("kb/s")),
                        &p,
                    )),
            )
            .child(
                section("appearance", &p)
                    .child(field(
                        "app theme",
                        theme_hint,
                        div().flex().child(self.render_modes(&p, cx)),
                        &p,
                    ))
                    .child(self.render_looks(&p, cx)),
            )
    }
}
