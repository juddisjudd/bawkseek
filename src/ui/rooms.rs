use std::rc::Rc;

use chrono::{DateTime, Local};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable, VirtualListScrollHandle, v_virtual_list};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{UserAction, kit};
use crate::format;
use crate::net::{Command, RoomLine, Session};
use crate::theme::{Palette, palette};

const LIST_ROW: f32 = 36.;
const MEMBER_ROW: f32 = 26.;

pub struct RoomsView {
    session: Entity<Session>,
    target: Entity<InputState>,
    filter: Entity<InputState>,
    composer: Entity<InputState>,
    ticker: Entity<InputState>,
    private: bool,
    active: Option<String>,
    feed: bool,
    feed_log: ScrollHandle,
    feed_shown: usize,
    visible: bool,
    log: ScrollHandle,
    shown: (Option<String>, usize),
    list_scroll: VirtualListScrollHandle,
    member_scroll: VirtualListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<UserAction> for RoomsView {}

impl RoomsView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let target = cx.new(|cx| InputState::new(window, cx).placeholder("room name"));
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("filter rooms"));
        let composer = cx.new(|cx| InputState::new(window, cx).placeholder("say something"));
        let ticker = cx.new(|cx| InputState::new(window, cx).placeholder("your ticker"));
        let subscriptions = vec![
            cx.subscribe_in(
                &target,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.join_typed(window, cx);
                    }
                },
            ),
            cx.subscribe_in(&filter, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &composer,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.say(window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &ticker,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.set_ticker(window, cx);
                    }
                },
            ),
            cx.observe(&session, |_, _, cx| cx.notify()),
        ];
        Self {
            session,
            target,
            filter,
            composer,
            ticker,
            private: false,
            active: None,
            feed: false,
            feed_log: ScrollHandle::new(),
            feed_shown: 0,
            visible: false,
            log: ScrollHandle::new(),
            shown: (None, 0),
            list_scroll: VirtualListScrollHandle::new(),
            member_scroll: VirtualListScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        let input = if self.active.is_some() {
            &self.composer
        } else {
            &self.filter
        };
        input.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Only the room on screen counts as read, so the workspace reports page changes here.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.visible = visible;
        self.report_viewing(cx);
    }

    fn report_viewing(&self, cx: &mut Context<Self>) {
        let viewing = self.visible.then(|| self.active.clone()).flatten();
        self.session
            .update(cx, |session, cx| session.view_room(viewing, cx));
    }

    fn select(&mut self, room: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(room) = &room {
            self.composer.update(cx, |state, cx| {
                state.set_placeholder(format!("say something in {room}"), window, cx)
            });
        }
        self.active = room;
        self.feed = false;
        self.shown = (None, 0);
        self.member_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.report_viewing(cx);
        self.focus(window, cx);
        cx.notify();
    }

    fn join(&mut self, room: String, window: &mut Window, cx: &mut Context<Self>) {
        let already = self.session.read(cx).rooms.get(&room).is_some();
        if !already {
            self.session.read(cx).send(Command::JoinRoom {
                room: room.clone(),
                private: self.private,
            });
        }
        self.select(Some(room), window, cx);
    }

    fn join_typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let room = self.target.read(cx).value().trim().to_string();
        if room.is_empty() {
            return;
        }
        self.target
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.join(room, window, cx);
    }

    fn leave(&mut self, room: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.session
            .read(cx)
            .send(Command::LeaveRoom(room.to_string()));
        if self.active.as_deref() == Some(room) {
            self.select(None, window, cx);
        }
    }

    fn say(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(room) = self.active.clone() else {
            return;
        };
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.session.read(cx).send(Command::Say { room, text });
    }

    fn set_ticker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(room) = self.active.clone() else {
            return;
        };
        let ticker = self.ticker.read(cx).value().trim().to_string();
        self.ticker
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.session
            .read(cx)
            .send(Command::SetTicker { room, ticker });
    }

    fn visible_list(&self, cx: &App) -> Vec<usize> {
        let filter = self.filter.read(cx).value().trim().to_lowercase();
        self.session
            .read(cx)
            .rooms
            .list
            .iter()
            .enumerate()
            .filter(|(_, room)| filter.is_empty() || room.name.to_lowercase().contains(&filter))
            .map(|(ix, _)| ix)
            .collect()
    }

    fn render_list_rows(
        &mut self,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = palette(cx);
        let visible = self.visible_list(cx);
        let session = self.session.read(cx);
        let rows: Vec<(String, u32, bool)> = range
            .filter_map(|ix| visible.get(ix))
            .map(|ix| {
                let room = &session.rooms.list[*ix];
                (
                    room.name.clone(),
                    room.user_count,
                    session.rooms.get(&room.name).is_some(),
                )
            })
            .collect();
        rows.into_iter()
            .map(|(name, users, joined)| {
                div()
                    .id(SharedString::from(format!("room-{name}")))
                    .h(px(LIST_ROW))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px(px(12.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|style| style.bg(p.bg_weak))
                    .on_click(cx.listener({
                        let name = name.clone();
                        move |this, _, window, cx| this.join(name.clone(), window, cx)
                    }))
                    .child(
                        Icon::new(IconName::Hash)
                            .size(px(14.))
                            .text_color(p.text_weaker),
                    )
                    .child(kit::truncate(name).flex_1().text_color(p.text_strong))
                    .when(joined, |this| {
                        this.child(
                            div()
                                .text_size(px(12.))
                                .text_color(p.success)
                                .child("joined"),
                        )
                    })
                    .child(
                        div()
                            .w(px(110.))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .text_color(p.text_weak)
                            .child(format::plural(users as usize, "user", "users")),
                    )
                    .into_any_element()
            })
            .collect()
    }

    fn render_member_rows(
        &mut self,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = palette(cx);
        let Some(room) = self.active.clone() else {
            return Vec::new();
        };
        let users: Vec<String> = self
            .session
            .read(cx)
            .rooms
            .get(&room)
            .map(|room| range.filter_map(|ix| room.users.get(ix).cloned()).collect())
            .unwrap_or_default();
        users
            .into_iter()
            .map(|user| {
                div()
                    .h(px(MEMBER_ROW))
                    .w_full()
                    .flex()
                    .items_center()
                    .px(px(8.))
                    .child(kit::user_cell(
                        SharedString::from(format!("member-{user}")),
                        &user,
                        p.text,
                        &p,
                        cx,
                    ))
                    .into_any_element()
            })
            .collect()
    }

    fn render_tabs(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let rooms: Vec<(String, usize)> = self
            .session
            .read(cx)
            .rooms
            .joined
            .iter()
            .map(|room| (room.name.clone(), room.unread))
            .collect();
        let mut pending = self
            .active
            .clone()
            .filter(|active| !rooms.iter().any(|(name, _)| name == active));
        let tab = |id: SharedString, label: SharedString, active: bool, unread: usize| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap_2()
                .pb(px(8.))
                .border_b_2()
                .border_color(if active {
                    p.text_strong
                } else {
                    transparent_black()
                })
                .text_color(if active { p.text_strong } else { p.text_weak })
                .cursor_pointer()
                .hover(|style| style.text_color(p.text_strong))
                .child(kit::truncate(label).max_w(px(200.)))
                .when(unread > 0, |this| {
                    this.child(
                        div()
                            .px(px(6.))
                            .rounded_full()
                            .bg(p.yolk)
                            .text_size(px(11.))
                            .text_color(p.bg)
                            .child(format::count(unread)),
                    )
                })
        };
        div()
            .flex()
            .items_end()
            .gap_5()
            .border_b_1()
            .border_color(p.border_weak)
            .child(
                tab(
                    "all-rooms".into(),
                    "all rooms".into(),
                    self.active.is_none() && !self.feed,
                    0,
                )
                .on_click(cx.listener(|this, _, window, cx| this.select(None, window, cx))),
            )
            .child(
                tab("public-feed".into(), "public feed".into(), self.feed, 0).on_click(
                    cx.listener(|this, _, window, cx| {
                        this.select(None, window, cx);
                        this.feed = true;
                        this.feed_shown = 0;
                    }),
                ),
            )
            .children(
                pending
                    .take()
                    .map(|name| tab("pending".into(), format!("{name} …").into(), true, 0)),
            )
            .children(rooms.into_iter().map(|(name, unread)| {
                let active = self.active.as_deref() == Some(name.as_str());
                let (select, leave) = (name.clone(), name.clone());
                tab(
                    format!("tab-{name}").into(),
                    format!("#{name}").into(),
                    active,
                    unread,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select(Some(select.clone()), window, cx)
                }))
                .child(
                    div()
                        .id(SharedString::from(format!("leave-{name}")))
                        .rounded(px(3.))
                        .p(px(2.))
                        .text_color(p.text_weaker)
                        .hover(|style| style.bg(p.bg_hover).text_color(p.text_strong))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.leave(&leave, window, cx);
                        }))
                        .child(Icon::new(IconName::X).size(px(12.))),
                )
            }))
    }

    fn render_room_list(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let visible = self.visible_list(cx);
        let total = self.session.read(cx).rooms.list.len();
        let sizes = Rc::new(vec![size(px(1.), px(LIST_ROW)); visible.len()]);
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .pb(px(8.))
                    .border_b_1()
                    .border_color(p.border_weak)
                    .child(
                        div().flex_1().child(
                            kit::input(&self.filter)
                                .appearance(false)
                                .cleanable(true)
                                .prefix(
                                    Icon::new(IconName::Search).small().text_color(p.text_weak),
                                ),
                        ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(p.text_weak)
                            .child(format::plural(total, "public room", "public rooms")),
                    )
                    .child(
                        kit::icon_button("refresh-rooms", IconName::RotateCw, "refresh the list")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.session.read(cx).send(Command::RoomList)
                            })),
                    ),
            )
            .child(if visible.is_empty() {
                kit::empty_state(
                    IconName::Hash,
                    if total == 0 {
                        "loading the room list…"
                    } else {
                        "no room matches"
                    },
                    "type a name above to join or create any room.",
                    p,
                )
                .into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(
                        v_virtual_list(cx.entity(), "room-list", sizes, |this, range, _, cx| {
                            this.render_list_rows(range, cx)
                        })
                        .track_scroll(&self.list_scroll)
                        .pb_4(),
                    )
                    .vertical_scrollbar(&self.list_scroll)
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_room(&mut self, name: String, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let Some(room) = session.rooms.get(&name) else {
            return kit::empty_state(
                IconName::Hourglass,
                &format!("joining {name}…"),
                "if nothing happens, the name may belong to a private room you are not in.",
                p,
            )
            .into_any_element();
        };
        let me = session.username.to_string();
        let lines: Vec<RoomLine> = room.lines.clone();
        let tickers = room.tickers.clone();
        let members = room.users.len();
        let private = room.private;

        let shown = (Some(name.clone()), lines.len());
        if self.shown != shown {
            self.shown = shown;
            self.log.scroll_to_bottom();
        }

        let mut log = div()
            .id("room-log")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(4.))
            .py_3()
            .track_scroll(&self.log)
            .overflow_y_scroll();
        for (ix, line) in lines.iter().enumerate() {
            let color = if line.username == me {
                p.yolk
            } else {
                p.text_strong
            };
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
                    .child(div().w(px(150.)).flex_none().flex().child(kit::user_cell(
                        ("line", ix),
                        &line.username,
                        color,
                        p,
                        cx,
                    )))
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
                    .child("quiet so far. new lines show up here."),
            );
        }

        let member_sizes = Rc::new(vec![size(px(1.), px(MEMBER_ROW)); members]);
        let side = div()
            .w(px(240.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .gap_3()
            .pl_4()
            .border_l_1()
            .border_color(p.border_weak)
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(p.text_weak)
                    .child(format!(
                        "{}{}",
                        format::plural(members, "member", "members"),
                        if private { " · private" } else { "" }
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(
                        v_virtual_list(
                            cx.entity(),
                            "members",
                            member_sizes,
                            |this, range, _, cx| this.render_member_rows(range, cx),
                        )
                        .track_scroll(&self.member_scroll),
                    )
                    .vertical_scrollbar(&self.member_scroll),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .pt_2()
                    .border_t_1()
                    .border_color(p.border_weak)
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(p.text_weak)
                            .child(format::plural(tickers.len(), "ticker", "tickers")),
                    )
                    .children(tickers.iter().take(5).map(|ticker| {
                        div()
                            .text_size(px(12.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_color(p.text_strong)
                                            .child(ticker.username.clone()),
                                    )
                                    .child(
                                        div().text_color(p.text_weak).child(ticker.ticker.clone()),
                                    ),
                            )
                    }))
                    .child(kit::input(&self.ticker).small()),
            );

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .pr_4()
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
                                Button::new("say").primary().small().label("send").on_click(
                                    cx.listener(|this, _, window, cx| this.say(window, cx)),
                                ),
                            ),
                    ),
            )
            .child(side)
            .into_any_element()
    }
}

impl RoomsView {
    fn render_feed(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let rooms = &self.session.read(cx).rooms;
        let (on, lines) = (rooms.feed_on, rooms.feed.clone());
        if self.feed_shown != lines.len() {
            self.feed_shown = lines.len();
            self.feed_log.scroll_to_bottom();
        }
        let mut log = div()
            .id("feed-log")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(4.))
            .py_3()
            .track_scroll(&self.feed_log)
            .overflow_y_scroll();
        for (ix, line) in lines.iter().enumerate() {
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
                        kit::truncate(format!("#{}", line.room))
                            .w(px(140.))
                            .flex_none()
                            .text_color(p.text_weak),
                    )
                    .child(div().w(px(150.)).flex_none().flex().child(kit::user_cell(
                        ("feed-user", ix),
                        &line.username,
                        p.text_strong,
                        p,
                        cx,
                    )))
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
                    .child(if on {
                        "waiting for the first line…"
                    } else {
                        "turn the feed on to see lines from every public room as they are said."
                    }),
            );
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        kit::chip(
                            "feed-on",
                            if on {
                                "following"
                            } else {
                                "follow the public feed"
                            },
                            on,
                            p,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.session
                                .update(cx, |session, cx| session.set_public_feed(!on, cx));
                        })),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(p.text_weak)
                            .child("every line said in any public room. it moves fast."),
                    ),
            )
            .child(log)
            .into_any_element()
    }
}

fn local(at: i64) -> DateTime<Local> {
    DateTime::from_timestamp(at, 0)
        .unwrap_or_default()
        .with_timezone(&Local)
}

impl Render for RoomsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let session = self.session.read(cx);
        let joined = session.rooms.joined.len();
        let subtitle = if joined == 0 {
            "public chat rooms, and private ones you create".to_string()
        } else {
            format!("{} joined", format::plural(joined, "room", "rooms"))
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
                Icon::new(IconName::Hash)
                    .size(px(15.))
                    .text_color(p.text_weaker),
            )
            .child(
                div()
                    .flex_1()
                    .child(kit::input(&self.target).appearance(false)),
            )
            .child(
                kit::chip("private-room", "private", self.private, &p).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.private = !this.private;
                        cx.notify();
                    },
                )),
            )
            .child(
                Button::new("join-room")
                    .primary()
                    .small()
                    .label("join")
                    .on_click(cx.listener(|this, _, window, cx| this.join_typed(window, cx))),
            );

        let body = match self.active.clone() {
            None if self.feed => self.render_feed(&p, cx),
            None => self.render_room_list(&p, cx),
            Some(room) => self.render_room(room, &p, cx),
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .px(px(40.))
            .pt(px(32.))
            .pb_4()
            .child(kit::page_header("rooms", subtitle, &p))
            .child(command)
            .child(self.render_tabs(&p, cx))
            .child(body)
    }
}
