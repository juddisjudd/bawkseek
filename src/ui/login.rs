use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::kit;
use crate::assets::MARK;
use crate::net::Status;
use crate::theme::palette;

pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub remember: bool,
}

pub struct LoginView {
    username: Entity<InputState>,
    password: Entity<InputState>,
    remember: bool,
    status: Status,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LoginRequest> for LoginView {}

impl LoginView {
    pub fn new(
        username: &str,
        remember: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let username_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("pick any free name")
                .default_value(username.to_string())
        });
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("password")
                .masked(true)
        });
        let submit_on_enter = |this: &mut Self,
                               _: &Entity<InputState>,
                               event: &InputEvent,
                               window: &mut Window,
                               cx: &mut Context<Self>| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit(window, cx);
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(&username_state, window, submit_on_enter),
            cx.subscribe_in(&password, window, submit_on_enter),
        ];
        let focus = if username.is_empty() {
            &username_state
        } else {
            &password
        };
        focus.update(cx, |state, cx| state.focus(window, cx));
        Self {
            username: username_state,
            password,
            remember,
            status: Status::Offline,
            _subscriptions: subscriptions,
        }
    }

    pub fn clear_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.password.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
    }

    pub fn set_status(&mut self, status: Status, cx: &mut Context<Self>) {
        if self.status != status {
            self.status = status;
            cx.notify();
        }
    }

    fn submit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.status == Status::Connecting {
            return;
        }
        let username = self.username.read(cx).value().trim().to_string();
        let password = self.password.read(cx).value().to_string();
        if username.is_empty() || password.is_empty() {
            self.status = Status::Failed("enter a name and a password".into());
            cx.notify();
            return;
        }
        cx.emit(LoginRequest {
            username,
            password,
            remember: self.remember,
        });
    }
}

impl Render for LoginView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let connecting = self.status == Status::Connecting;
        let error = match &self.status {
            Status::Failed(reason) => Some(reason.clone()),
            _ => None,
        };
        let field = |label: &'static str, input: &Entity<InputState>| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.text_weak)
                        .child(label),
                )
                .child(kit::input(input).disabled(connecting))
        };

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(380.))
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_3()
                            .child(svg().path(MARK).size(px(56.)).text_color(p.text_strong))
                            .child(
                                div()
                                    .flex()
                                    .text_size(px(22.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(div().text_color(p.text_strong).child("bawk"))
                                    .child(div().text_color(p.text_weak).child("seek")),
                            )
                            .child(div().text_color(p.text_weak).child("soulseek client & audio player")),
                    )
                    .child(
                        div()
                            .p_5()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(p.border_weak)
                            .bg(p.bg_weak)
                            .child(field("username", &self.username))
                            .child(field("password", &self.password))
                            .child(
                                Checkbox::new("remember")
                                    .label("remember me on this computer")
                                    .checked(self.remember)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.remember = *checked;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("login")
                                    .primary()
                                    .w_full()
                                    .label(if connecting { "connecting…" } else { "log in" })
                                    .loading(connecting)
                                    .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                            )
                            .when_some(error, |this, error| {
                                this.child(div().text_color(p.danger).child(error))
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(p.text_weaker)
                            .text_center()
                            .child("new to soulseek? pick any free name and a password. the server makes the account on first login."),
                    ),
            )
    }
}
