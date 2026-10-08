use chrono::{DateTime, Local};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{UserAction, kit};
use crate::chats::{Conversation, Line};
use crate::format;
use crate::net::Session;
use crate::theme::{Palette, palette};

pub struct MessagesView {
    session: Entity<Session>,
    target: Entity<InputState>,
    composer: Entity<InputState>,
    selected: Option<String>,
    visible: bool,
    log: ScrollHandle,
    shown: (Option<String>, usize),
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<UserAction> for MessagesView {}

impl MessagesView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let target = cx.new(|cx| InputState::new(window, cx).placeholder("username"));
        let composer = cx.new(|cx| InputState::new(window, cx).placeholder("write a message"));
        let subscriptions = vec![
            cx.subscribe_in(
                &target,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.open_typed(window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &composer,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.send(window, cx);
                    }
                },
            ),
            cx.observe(&session, |_, _, cx| cx.notify()),
        ];
        Self {
            session,
            target,
            composer,
            selected: None,
            visible: false,
            log: ScrollHandle::new(),
            shown: (None, 0),
            _subscriptions: subscriptions,
        }
    }

    pub fn open(&mut self, username: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |session, _| {
            session.chats.open(username);
        });
        self.select(Some(username.to_string()), window, cx);
    }

    /// Only a conversation on screen counts as read, so the workspace reports page changes here.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.visible = visible;
        let viewing = visible.then(|| self.selected.clone()).flatten();
        self.session
            .update(cx, |session, cx| session.view_chat(viewing, cx));
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        let input = if self.selected.is_some() {
            &self.composer
        } else {
            &self.target
        };
        input.update(cx, |state, cx| state.focus(window, cx));
    }

    fn select(&mut self, username: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = match &username {
            Some(username) => format!("message {username}"),
            None => "write a message".to_string(),
        };
        self.composer.update(cx, |state, cx| {
            state.set_placeholder(placeholder, window, cx)
        });
        self.selected = username;
        self.shown = (None, 0);
        if self.visible {
            let viewing = self.selected.clone();
            self.session
                .update(cx, |session, cx| session.view_chat(viewing, cx));
        }
        self.focus(window, cx);
        cx.notify();
    }

    fn open_typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let username = self.target.read(cx).value().trim().to_string();
        if username.is_empty() {
            return;
        }
        self.target
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.open(&username, window, cx);
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(username) = self.selected.clone() else {
            return;
        };
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.session
            .update(cx, |session, cx| session.send_message(&username, text, cx));
    }

    fn delete(&mut self, username: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            session.chats.remove(username);
            session.chats.save();
            cx.notify();
        });
        if self.selected.as_deref() == Some(username) {
            self.select(None, window, cx);
        }
    }

    fn render_list(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let summaries: Vec<Summary> = self
            .session
            .read(cx)
            .chats
            .conversations
            .iter()
            .map(Summary::of)
            .collect();
        let rows: Vec<AnyElement> = summaries
            .into_iter()
            .enumerate()
            .map(|(ix, summary)| self.conversation_row(ix, summary, p, cx))
            .collect();
        div()
            .id("conversations")
            .w(px(280.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .gap(px(2.))
            .pr_2()
            .border_r_1()
            .border_color(p.border_weak)
            .overflow_y_scrollbar()
            .children(rows)
    }

    fn conversation_row(
        &self,
        ix: usize,
        summary: Summary,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Summary {
            username,
            preview,
            when,
            unread,
        } = summary;
        let selected = self.selected.as_deref() == Some(username.as_str());
        let view = cx.entity().downgrade();
        let menu_user = username.clone();

        div()
            .id(("conversation", ix))
            .h(px(52.))
            .flex()
            .flex_col()
            .justify_center()
            .gap(px(2.))
            .px(px(10.))
            .rounded(px(4.))
            .cursor_pointer()
            .when(selected, |this| this.bg(p.bg_hover))
            .when(!selected, |this| this.hover(|style| style.bg(p.bg_weak)))
            .on_click(cx.listener({
                let username = username.clone();
                move |this, _, window, cx| this.select(Some(username.clone()), window, cx)
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        kit::truncate(username.clone())
                            .flex_1()
                            .text_color(if unread > 0 || selected {
                                p.text_strong
                            } else {
                                p.text
                            })
                            .when(unread > 0, |this| this.font_weight(FontWeight::SEMIBOLD)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(p.text_weaker)
                            .child(when),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        kit::truncate(preview)
                            .flex_1()
                            .text_size(px(12.))
                            .text_color(p.text_weak),
                    )
                    .when(unread > 0, |this| {
                        this.child(
                            div()
                                .flex_none()
                                .px(px(6.))
                                .rounded_full()
                                .bg(p.yolk)
                                .text_size(px(11.))
                                .text_color(p.bg)
                                .child(format::count(unread)),
                        )
                    }),
            )
            .context_menu(move |menu, _, cx| {
                let (view, username) = (view.clone(), menu_user.clone());
                kit::user_menu(menu, &username, view.clone(), cx)
                    .separator()
                    .item(
                        PopupMenuItem::new("delete conversation")
                            .icon(Icon::new(IconName::Trash))
                            .on_click(move |_, window, cx| {
                                let username = username.clone();
                                let _ =
                                    view.update(cx, |this, cx| this.delete(&username, window, cx));
                            }),
                    )
            })
            .into_any_element()
    }

    fn render_chat(
        &mut self,
        username: String,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let lines: Vec<Line> = self
            .session
            .read(cx)
            .chats
            .get(&username)
            .map(|conversation| conversation.lines.clone())
            .unwrap_or_default();
        let shown = (Some(username.clone()), lines.len());
        if self.shown != shown {
            self.shown = shown;
            self.log.scroll_to_bottom();
        }

        let me: SharedString = self.session.read(cx).username.clone();
        let mut log = div()
            .id("chat-log")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(6.))
            .py_3()
            .track_scroll(&self.log)
            .overflow_y_scroll();
        let mut day = None;
        for line in &lines {
            let this_day = local(line.at).date_naive();
            if day != Some(this_day) {
                day = Some(this_day);
                log = log.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .py_1()
                        .text_size(px(11.))
                        .text_color(p.text_weaker)
                        .child(div().flex_1().h(px(1.)).bg(p.border_weak))
                        .child(
                            local(line.at)
                                .format("%A %-d %B")
                                .to_string()
                                .to_lowercase(),
                        )
                        .child(div().flex_1().h(px(1.)).bg(p.border_weak)),
                );
            }
            log = log.child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        div()
                            .w(px(44.))
                            .flex_none()
                            .text_color(p.text_weaker)
                            .child(local(line.at).format("%H:%M").to_string()),
                    )
                    .child(
                        div()
                            .w(px(150.))
                            .flex_none()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(if line.mine { p.yolk } else { p.text_strong })
                            .child(if line.mine {
                                me.to_string()
                            } else {
                                username.clone()
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(p.text)
                            .child(line.text.clone()),
                    ),
            );
        }
        if lines.is_empty() {
            log = log.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(p.text_weak)
                    .child(format!("no messages with {username} yet")),
            );
        }

        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .pl_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .pb_2()
                    .border_b_1()
                    .border_color(p.border_weak)
                    .child(
                        Icon::new(IconName::User)
                            .size(px(14.))
                            .text_color(p.text_weaker),
                    )
                    .child(kit::user_cell("chat-user", &username, p.text_strong, p, cx))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(p.text_weaker)
                            .child("saved on this computer"),
                    ),
            )
            .child(log)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pt_2()
                    .border_t_1()
                    .border_color(p.border_weak)
                    .child(div().flex_1().child(kit::input(&self.composer)))
                    .child(
                        kit::button("send", cx)
                            .small()
                            .label("send")
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            )
    }
}

struct Summary {
    username: String,
    preview: String,
    when: String,
    unread: usize,
}

impl Summary {
    fn of(conversation: &Conversation) -> Self {
        let last = conversation.lines.last();
        Self {
            username: conversation.username.clone(),
            preview: last
                .map(|line| {
                    if line.mine {
                        format!("you: {}", line.text)
                    } else {
                        line.text.clone()
                    }
                })
                .unwrap_or_default(),
            when: last.map(|line| short_time(line.at)).unwrap_or_default(),
            unread: conversation.unread,
        }
    }
}

fn local(at: i64) -> DateTime<Local> {
    DateTime::from_timestamp(at, 0)
        .unwrap_or_default()
        .with_timezone(&Local)
}

fn short_time(at: i64) -> String {
    let time = local(at);
    if time.date_naive() == Local::now().date_naive() {
        time.format("%H:%M").to_string()
    } else {
        time.format("%-d %b").to_string().to_lowercase()
    }
}

impl Render for MessagesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let session = self.session.read(cx);
        let count = session.chats.conversations.len();
        let unread = session.chats.unread();
        if let Some(selected) = &self.selected
            && session.chats.get(selected).is_none()
        {
            self.selected = None;
        }
        let subtitle = if count == 0 {
            "talk to any user, even when they are offline".to_string()
        } else {
            format!(
                "{} · {unread} unread",
                format::plural(count, "conversation", "conversations")
            )
        };

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
                Icon::new(IconName::MessagesSquare)
                    .size(px(15.))
                    .text_color(p.text_weaker),
            )
            .child(
                div()
                    .flex_1()
                    .child(kit::input(&self.target).appearance(false)),
            )
            .child(
                kit::button("open-chat", cx)
                    .small()
                    .label("open")
                    .on_click(cx.listener(|this, _, window, cx| this.open_typed(window, cx))),
            );

        let body = if count == 0 {
            kit::empty_state(
                IconName::MessagesSquare,
                "no conversations yet",
                "type a username above, or right-click any username and pick send message.",
                &p,
            )
            .into_any_element()
        } else {
            let chat = match self.selected.clone() {
                Some(username) => self.render_chat(username, &p, cx).into_any_element(),
                None => div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(p.text_weak)
                    .child("pick a conversation")
                    .into_any_element(),
            };
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .child(self.render_list(&p, cx))
                .child(chat)
                .into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .px(px(40.))
            .pt(px(32.))
            .pb_4()
            .child(kit::page_header("messages", subtitle, &p))
            .child(command)
            .child(body)
    }
}
