use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Social, UserAction, kit};
use crate::format;
use crate::net::{Command, Presence, Session, UserCard};
use crate::theme::{Palette, palette};

pub struct UsersView {
    session: Entity<Session>,
    target: Entity<InputState>,
    days: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<UserAction> for UsersView {}

impl UsersView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let target = cx.new(|cx| InputState::new(window, cx).placeholder("username"));
        let subscriptions = vec![
            cx.subscribe_in(
                &target,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.look_up_typed(window, cx);
                    }
                },
            ),
            cx.observe(&session, |_, _, cx| cx.notify()),
        ];
        let days = cx.new(|cx| InputState::new(window, cx).placeholder("days"));
        Self {
            session,
            target,
            days,
            _subscriptions: subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.target.update(cx, |state, cx| state.focus(window, cx));
    }

    fn look_up(&mut self, username: &str, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, cx| session.look_up(username, cx));
    }

    fn look_up_typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let username = self.target.read(cx).value().trim().to_string();
        if username.is_empty() {
            return;
        }
        self.target
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.look_up(&username, cx);
    }

    fn render_buddies(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let cards = session.buddies.clone();
        let ignored: Vec<String> = cx
            .try_global::<Social>()
            .map(|social| {
                let mut names: Vec<String> = social.ignored.iter().cloned().collect();
                names.sort_by_cached_key(|name| name.to_lowercase());
                names
            })
            .unwrap_or_default();

        let mut list = div()
            .id("buddies")
            .w(px(340.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .gap(px(2.))
            .pr_3()
            .border_r_1()
            .border_color(p.border_weak)
            .overflow_y_scrollbar()
            .child(section_title("buddies", cards.len(), p));
        if cards.is_empty() {
            list = list.child(
                div()
                    .py_2()
                    .text_color(p.text_weak)
                    .child("right-click any username and pick add to buddies."),
            );
        }
        for (ix, card) in cards.iter().enumerate() {
            let username = card.username.clone();
            let remove = card.username.clone();
            list = list.child(
                div()
                    .id(("buddy", ix))
                    .h(px(34.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px(px(8.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|style| style.bg(p.bg_weak))
                    .on_click(cx.listener(move |this, _, _, cx| this.look_up(&username, cx)))
                    .child(kit::dot(presence_color(card.presence, p)))
                    .child(div().flex_1().min_w_0().flex().child(kit::user_cell(
                        ("buddy-name", ix),
                        &card.username,
                        p.text_strong,
                        p,
                        cx,
                    )))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(p.text_weak)
                            .child(presence_label(card.presence)),
                    )
                    .child(
                        kit::icon_button(("unbuddy", ix), IconName::X, "remove from buddies")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.stop_propagation();
                                cx.emit(UserAction::SetBuddy(remove.clone(), false));
                            })),
                    ),
            );
        }

        list = list
            .child(div().h_4())
            .child(section_title("ignored", ignored.len(), p));
        if ignored.is_empty() {
            list = list.child(
                div()
                    .py_2()
                    .text_color(p.text_weak)
                    .child("ignored users' messages and room lines stay hidden."),
            );
        }
        for (ix, name) in ignored.into_iter().enumerate() {
            let unignore = name.clone();
            list = list.child(
                div()
                    .id(("ignored", ix))
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px(px(8.))
                    .child(
                        Icon::new(IconName::EyeOff)
                            .size(px(13.))
                            .text_color(p.text_weaker),
                    )
                    .child(kit::truncate(name).flex_1().text_color(p.text))
                    .child(
                        kit::icon_button(("unignore", ix), IconName::X, "stop ignoring").on_click(
                            cx.listener(move |_, _, _, cx| {
                                cx.emit(UserAction::SetIgnored(unignore.clone(), false))
                            }),
                        ),
                    ),
            );
        }
        list
    }

    fn give_privileges(&self, username: String, window: &mut Window, cx: &mut Context<Self>) {
        self.days
            .update(cx, |state, cx| state.set_value("1", window, cx));
        let (days, session) = (self.days.clone(), self.session.clone());
        let title: SharedString = format!("give {username} privileges").into();
        window.open_dialog(cx, move |dialog, _, cx| {
            let (days, session, username) = (days.clone(), session.clone(), username.clone());
            dialog
                .title(title.clone())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child("the days come out of your own privileges.")
                        .child(kit::input(&days)),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            kit::button("cancel-give", cx)
                                .label("cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("confirm-give")
                                .primary()
                                .label("give")
                                .on_click(move |_, window, cx| {
                                    let Ok(days) = days.read(cx).value().trim().parse::<u32>()
                                    else {
                                        return;
                                    };
                                    if days == 0 {
                                        return;
                                    }
                                    session.read(cx).send(Command::GivePrivileges {
                                        username: username.clone(),
                                        days,
                                    });
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
    }

    fn render_card(
        &self,
        card: &UserCard,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (buddy, ignored) = cx.try_global::<Social>().map_or((false, false), |social| {
            (
                social.buddies.contains(&card.username),
                social.ignored.contains(&card.username),
            )
        });
        let username = card.username.clone();
        let can_give = self
            .session
            .read(cx)
            .privileges
            .is_some_and(|seconds| seconds > 0);
        let row = |label: &'static str, value: String| {
            div()
                .flex()
                .gap_4()
                .py(px(6.))
                .border_b_1()
                .border_color(p.border_weak)
                .child(
                    div()
                        .w(px(160.))
                        .flex_none()
                        .text_color(p.text_weak)
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(p.text_strong)
                        .child(value),
                )
        };
        let unknown = || {
            if card.loading {
                "asking…".to_string()
            } else {
                "no answer".to_string()
            }
        };
        let shares = card
            .stats
            .map(|stats| {
                format!(
                    "{} in {}",
                    format::plural(stats.shared_files as usize, "file", "files"),
                    format::plural(stats.shared_folders as usize, "folder", "folders")
                )
            })
            .unwrap_or_else(unknown);
        let speed = card
            .stats
            .map(|stats| format::speed(u64::from(stats.average_speed)))
            .unwrap_or_else(unknown);
        let peer = card.peer.clone();
        let slots = peer
            .as_ref()
            .map(|peer| peer.total_uploads.to_string())
            .unwrap_or_else(unknown);
        let queue = peer
            .as_ref()
            .map(|peer| format::plural(peer.queue_size as usize, "file", "files"))
            .unwrap_or_else(unknown);
        let free = peer
            .as_ref()
            .map(|peer| if peer.slots_free { "yes" } else { "no" }.to_string())
            .unwrap_or_else(unknown);
        let description = peer
            .as_ref()
            .map(|peer| peer.description.trim().to_string())
            .filter(|text| !text.is_empty());

        let action = |id: &'static str, label: &'static str, icon: IconName, event: UserAction| {
            Button::new(id)
                .outline()
                .small()
                .icon(Icon::new(icon))
                .label(label)
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(event.clone())))
        };

        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_4()
            .pl_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.text_strong)
                            .child(username.clone()),
                    )
                    .child(kit::dot(presence_color(card.presence, p)))
                    .child(div().text_color(p.text_weak).child(if card.loading {
                        "asking…"
                    } else {
                        presence_label(card.presence)
                    }))
                    .when(card.privileged, |this| this.child(kit::tag("privileged", p))),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(action("card-browse", "browse shares", IconName::FolderSearch, UserAction::Browse(username.clone())))
                    .child(action("card-message", "send message", IconName::MessagesSquare, UserAction::Message(username.clone())))
                    .child(if buddy {
                        action("card-buddy", "remove buddy", IconName::UserMinus, UserAction::SetBuddy(username.clone(), false))
                    } else {
                        action("card-buddy", "add buddy", IconName::UserPlus, UserAction::SetBuddy(username.clone(), true))
                    })
                    .child(if ignored {
                        action("card-ignore", "stop ignoring", IconName::Eye, UserAction::SetIgnored(username.clone(), false))
                    } else {
                        action("card-ignore", "ignore", IconName::EyeOff, UserAction::SetIgnored(username.clone(), true))
                    })
                    .when(can_give, |this| {
                        let username = username.clone();
                        this.child(
                            kit::button("card-give", cx)
                                .small()
                                .icon(Icon::new(IconName::StarFill))
                                .label("give privileges…")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.give_privileges(username.clone(), window, cx)
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(row("shares", shares))
                    .child(row("average speed", speed))
                    .child(row("upload slots", slots))
                    .child(row("queue", queue))
                    .child(row("free slot now", free)),
            )
            .when_some(description, |this, text| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().text_color(p.text_weak).child("about"))
                        .child(
                            div()
                                .p_3()
                                .rounded(px(4.))
                                .border_1()
                                .border_color(p.border_weak)
                                .bg(p.bg_weak)
                                .text_color(p.text)
                                .child(text),
                        ),
                )
            })
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(p.text_weaker)
                    .child("ignoring hides someone's messages and room lines. soulseek gives bawkseek no way to refuse their downloads."),
            )
    }
}

fn section_title(title: &'static str, count: usize, p: &Palette) -> Div {
    div()
        .flex()
        .gap_2()
        .pb_1()
        .child(
            div()
                .text_color(p.text_strong)
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(div().text_color(p.text_weaker).child(format::count(count)))
}

fn presence_color(presence: Presence, p: &Palette) -> Hsla {
    match presence {
        Presence::Online => p.success,
        Presence::Away => p.warning,
        Presence::Offline => p.text_weaker,
        Presence::Unknown => p.border_weak,
    }
}

fn presence_label(presence: Presence) -> &'static str {
    match presence {
        Presence::Online => "online",
        Presence::Away => "away",
        Presence::Offline => "offline",
        Presence::Unknown => "unknown",
    }
}

impl Render for UsersView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let session = self.session.read(cx);
        let online = session
            .buddies
            .iter()
            .filter(|card| card.presence == Presence::Online)
            .count();
        let subtitle = format!(
            "{} · {online} online",
            format::plural(session.buddies.len(), "buddy", "buddies")
        );
        let card = session.card.clone();

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
                Icon::new(IconName::User)
                    .size(px(15.))
                    .text_color(p.text_weaker),
            )
            .child(
                div()
                    .flex_1()
                    .child(kit::input(&self.target).appearance(false)),
            )
            .child(
                kit::button("look-up", cx)
                    .small()
                    .label("look up")
                    .on_click(cx.listener(|this, _, window, cx| this.look_up_typed(window, cx))),
            );

        let detail = match &card {
            Some(card) => self.render_card(card, &p, cx).into_any_element(),
            None => kit::empty_state(
                IconName::User,
                "look up a user",
                "see someone's status, shares, speed and queue. click a buddy or type a name above.",
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
            .pb_4()
            .child(kit::page_header("users", subtitle, &p))
            .child(command)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.render_buddies(&p, cx))
                    .child(detail),
            )
    }
}
