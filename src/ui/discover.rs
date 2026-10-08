use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use soulseek_rs::Recommendation;

use super::{UserAction, kit};
use crate::format;
use crate::net::{Command, Session};
use crate::theme::{Palette, palette};

pub enum DiscoverEvent {
    Search(String),
    SetInterest { item: String, like: bool, add: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    ForYou,
    Popular,
    People,
}

pub struct DiscoverView {
    session: Entity<Session>,
    like: Entity<InputState>,
    dislike: Entity<InputState>,
    tab: Tab,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DiscoverEvent> for DiscoverView {}
impl EventEmitter<UserAction> for DiscoverView {}

impl DiscoverView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let like = cx.new(|cx| InputState::new(window, cx).placeholder("add an artist or genre"));
        let dislike =
            cx.new(|cx| InputState::new(window, cx).placeholder("add something to avoid"));
        let subscriptions = vec![
            cx.subscribe_in(&like, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.add_typed(true, window, cx);
                }
            }),
            cx.subscribe_in(
                &dislike,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.add_typed(false, window, cx);
                    }
                },
            ),
            cx.observe(&session, |_, _, cx| cx.notify()),
        ];
        Self {
            session,
            like,
            dislike,
            tab: Tab::ForYou,
            _subscriptions: subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.like.update(cx, |state, cx| state.focus(window, cx));
    }

    pub fn refresh(&self, cx: &App) {
        self.session.read(cx).send(Command::Discover);
    }

    fn add_typed(&mut self, like: bool, window: &mut Window, cx: &mut Context<Self>) {
        let input = if like { &self.like } else { &self.dislike };
        let item = input.read(cx).value().trim().to_lowercase();
        if item.is_empty() {
            return;
        }
        input.update(cx, |state, cx| state.set_value("", window, cx));
        cx.emit(DiscoverEvent::SetInterest {
            item,
            like,
            add: true,
        });
    }

    fn open_item(&mut self, item: String, cx: &mut Context<Self>) {
        self.session.read(cx).send(Command::DiscoverItem(item));
    }

    fn interest_list(
        &self,
        title: &'static str,
        like: bool,
        items: &[String],
        input: &Entity<InputState>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut list = div().flex().flex_col().gap(px(2.)).child(
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
                .child(
                    div()
                        .text_color(p.text_weaker)
                        .child(format::count(items.len())),
                ),
        );
        for (ix, item) in items.iter().enumerate() {
            let remove = item.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("{title}-{ix}")))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px(px(8.))
                    .rounded(px(4.))
                    .hover(|style| style.bg(p.bg_weak))
                    .child(
                        Icon::new(if like {
                            IconName::ThumbsUp
                        } else {
                            IconName::ThumbsDown
                        })
                        .size(px(12.))
                        .text_color(p.text_weaker),
                    )
                    .child(kit::truncate(item.clone()).flex_1().text_color(p.text))
                    .child(
                        kit::icon_button(
                            SharedString::from(format!("{title}-x-{ix}")),
                            IconName::X,
                            "remove",
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(DiscoverEvent::SetInterest {
                                item: remove.clone(),
                                like,
                                add: false,
                            })
                        })),
                    ),
            );
        }
        list.child(div().pt_1().child(kit::input(input).small()))
    }

    fn recommendation_rows(
        &self,
        prefix: &'static str,
        items: &[Recommendation],
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        items
            .iter()
            .enumerate()
            .map(|(ix, recommendation)| {
                let (search, like, open) = (
                    recommendation.item.clone(),
                    recommendation.item.clone(),
                    recommendation.item.clone(),
                );
                div()
                    .id(SharedString::from(format!("{prefix}-{ix}")))
                    .h(px(34.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px(px(10.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|style| style.bg(p.bg_weak))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_item(open.clone(), cx)))
                    .child(
                        kit::truncate(recommendation.item.clone())
                            .flex_1()
                            .text_color(p.text_strong),
                    )
                    .child(
                        div()
                            .w(px(70.))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .text_color(p.text_weak)
                            .child(format!("{:+}", recommendation.rating)),
                    )
                    .child(
                        kit::icon_button(
                            SharedString::from(format!("{prefix}-s-{ix}")),
                            IconName::Search,
                            "search for it",
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            cx.emit(DiscoverEvent::Search(search.clone()));
                        })),
                    )
                    .child(
                        kit::icon_button(
                            SharedString::from(format!("{prefix}-l-{ix}")),
                            IconName::ThumbsUp,
                            "i like this",
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            cx.emit(DiscoverEvent::SetInterest {
                                item: like.clone(),
                                like: true,
                                add: true,
                            });
                        })),
                    )
                    .into_any_element()
            })
            .collect()
    }

    fn render_results(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let discovery = self.session.read(cx).discovery.clone();
        let tab = |id: &'static str, label: &'static str, value: Tab, count: usize, this: &Self| {
            let active = this.tab == value;
            div()
                .id(id)
                .flex()
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
                .child(label)
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.text_weaker)
                        .child(format::count(count)),
                )
        };

        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(item) = &discovery.item {
            let people: Vec<AnyElement> = item
                .users
                .iter()
                .take(20)
                .enumerate()
                .map(|(ix, user)| {
                    kit::user_cell(("item-user", ix), user, p.text_strong, p, cx).into_any_element()
                })
                .collect();
            rows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .mb_3()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(p.border_weak)
                    .bg(p.bg_weak)
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(div().text_color(p.text_weak).child("related to"))
                            .child(div().text_color(p.text_strong).child(item.item.clone())),
                    )
                    .child(
                        div()
                            .text_color(p.text)
                            .child(if item.recommendations.is_empty() {
                                "asking the server…".to_string()
                            } else {
                                item.recommendations
                                    .iter()
                                    .take(12)
                                    .map(|recommendation| recommendation.item.clone())
                                    .collect::<Vec<_>>()
                                    .join(" · ")
                            }),
                    )
                    .when(!people.is_empty(), |this| {
                        this.child(div().text_color(p.text_weak).child("people who like it"))
                            .child(div().flex().flex_wrap().gap_3().children(people))
                    })
                    .into_any_element(),
            );
        }
        match self.tab {
            Tab::ForYou => {
                rows.extend(self.recommendation_rows("for-you", &discovery.recommended, p, cx));
                if !discovery.unrecommended.is_empty() {
                    rows.push(
                        div()
                            .pt_4()
                            .pb_1()
                            .text_color(p.text_weak)
                            .child("people with your taste tend to skip")
                            .into_any_element(),
                    );
                    rows.extend(self.recommendation_rows("skip", &discovery.unrecommended, p, cx));
                }
            }
            Tab::Popular => {
                rows.extend(self.recommendation_rows("popular", &discovery.global, p, cx))
            }
            Tab::People => {
                for (ix, user) in discovery.similar.iter().enumerate() {
                    rows.push(
                        div()
                            .h(px(32.))
                            .flex()
                            .items_center()
                            .gap_3()
                            .px(px(10.))
                            .child(div().flex_1().flex().child(kit::user_cell(
                                ("similar", ix),
                                &user.username,
                                p.text_strong,
                                p,
                                cx,
                            )))
                            .child(
                                div()
                                    .text_color(p.text_weak)
                                    .child(format!("{} in common", user.weight)),
                            )
                            .into_any_element(),
                    );
                }
            }
        }
        let empty = rows.is_empty();

        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .gap_3()
            .pl_5()
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_5()
                    .border_b_1()
                    .border_color(p.border_weak)
                    .child(
                        tab("tab-you", "for you", Tab::ForYou, discovery.recommended.len(), self)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tab = Tab::ForYou;
                                cx.notify();
                            })),
                    )
                    .child(
                        tab("tab-popular", "popular", Tab::Popular, discovery.global.len(), self)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tab = Tab::Popular;
                                cx.notify();
                            })),
                    )
                    .child(
                        tab("tab-people", "similar users", Tab::People, discovery.similar.len(), self)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tab = Tab::People;
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(div().pb_1().child(
                        kit::icon_button("refresh-discover", IconName::RotateCw, "ask the server again")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    )),
            )
            .child(if empty {
                kit::empty_state(
                    IconName::ThumbsUp,
                    "nothing here yet",
                    "add a few artists or genres you like. the server recommends more and finds people with similar taste.",
                    p,
                )
                .into_any_element()
            } else {
                div()
                    .id("discover-results")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_y_scrollbar()
                    .children(rows)
                    .into_any_element()
            })
    }
}

impl Render for DiscoverView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let session = self.session.read(cx);
        let (likes, dislikes) = (session.likes.clone(), session.dislikes.clone());

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .px(px(40.))
            .pt(px(32.))
            .pb_4()
            .child(kit::page_header(
                "discover",
                "find music and people through shared interests",
                &p,
            ))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .id("interests")
                            .w(px(300.))
                            .flex_none()
                            .h_full()
                            .flex()
                            .flex_col()
                            .gap_6()
                            .pr_4()
                            .border_r_1()
                            .border_color(p.border_weak)
                            .overflow_y_scrollbar()
                            .child(self.interest_list(
                                "you like",
                                true,
                                &likes,
                                &self.like.clone(),
                                &p,
                                cx,
                            ))
                            .child(self.interest_list(
                                "you dislike",
                                false,
                                &dislikes,
                                &self.dislike.clone(),
                                &p,
                                cx,
                            )),
                    )
                    .child(self.render_results(&p, cx)),
            )
    }
}
