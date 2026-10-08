use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Social, UserAction};
use crate::theme::{Palette, palette};

/// A username that opens its shares on click and offers every user action on right-click.
pub fn user_cell<V: EventEmitter<UserAction>>(
    id: impl Into<ElementId>,
    username: &str,
    color: Hsla,
    p: &Palette,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let browse = username.to_string();
    let menu_user = username.to_string();
    let view = cx.entity().downgrade();
    // The menu renders inside this wrapper, so it must not inherit the link's hover underline.
    div()
        .id(id)
        .flex()
        .min_w_0()
        .child(
            user_link("link", username.to_string(), color, p).on_click(cx.listener(
                move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.emit(UserAction::Browse(browse.clone()));
                },
            )),
        )
        .context_menu(move |menu, _, cx| user_menu(menu, &menu_user, view.clone(), cx))
}

/// Every text box in the app, without the border that lights up on focus.
pub fn input(state: &Entity<InputState>) -> Input {
    Input::new(state).focus_bordered(false)
}

pub fn page_header(
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    p: &Palette,
) -> Div {
    div().flex().items_end().justify_between().gap_4().child(
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .text_size(px(20.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.text_strong)
                    .child(title.into()),
            )
            .child(div().text_color(p.text_weak).child(subtitle.into())),
    )
}

/// bawkterm's plain button: no fill and a faint border until hovered.
pub fn button(id: impl Into<ElementId>, cx: &App) -> Button {
    let p = palette(cx);
    Button::new(id)
        .custom(
            ButtonCustomVariant::new(cx)
                .foreground(p.text_strong)
                .hover(p.bg_hover)
                .active(p.border_weak),
        )
        .border_1()
        .border_color(p.border_weak)
}

pub fn icon_button(id: impl Into<ElementId>, icon: IconName, tooltip: &'static str) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .icon(Icon::new(icon))
        .tooltip(tooltip)
}

pub fn progress_bar(fraction: f32, color: Hsla, p: &Palette) -> Div {
    div()
        .h(px(3.))
        .w_full()
        .rounded_full()
        .bg(p.border_weak)
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(color)
                .w(relative(fraction.clamp(0.0, 1.0))),
        )
}

pub fn chip(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    p: &Palette,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(26.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .border_1()
        .text_size(px(12.))
        .cursor_pointer()
        .map(|this| {
            if active {
                this.border_color(p.yolk)
                    .bg(p.yolk_dim)
                    .text_color(p.text_strong)
            } else {
                this.border_color(p.border_weak)
                    .text_color(p.text_weak)
                    .hover(|style| style.text_color(p.text_strong).bg(p.bg_weak))
            }
        })
        .child(label.into())
}

pub fn empty_state(icon: IconName, title: &str, body: &str, p: &Palette) -> Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .text_color(p.text_weak)
        .child(Icon::new(icon).size(px(28.)).text_color(p.text_weaker))
        .child(
            div()
                .mt_2()
                .text_color(p.text_strong)
                .child(title.to_string()),
        )
        .child(div().max_w(px(440.)).text_center().child(body.to_string()))
}

pub fn dot(color: Hsla) -> Div {
    div().size(px(7.)).flex_none().rounded_full().bg(color)
}

pub fn tag(label: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .flex_none()
        .px(px(5.))
        .rounded(px(3.))
        .border_1()
        .border_color(p.border_weak)
        .text_size(px(11.))
        .text_color(p.text_weak)
        .child(label.into())
}

/// A format name in its own color, so flac and mp3 tell apart at a glance.
pub fn format_tag(ext: &str, quality: &str, p: &Palette) -> Div {
    let color = p.format(ext);
    div()
        .flex_none()
        .flex()
        .gap(px(5.))
        .px(px(5.))
        .rounded(px(3.))
        .bg(color.opacity(0.14))
        .text_size(px(11.))
        .child(div().text_color(color).child(ext.to_string()))
        .when(!quality.is_empty(), |this| {
            this.child(div().text_color(p.text_weak).child(quality.to_string()))
        })
}

pub fn file_icon(ext: &str, p: &Palette) -> Icon {
    let audio = p.format(ext) != p.text_weaker;
    Icon::new(if audio {
        IconName::FileMusic
    } else {
        IconName::File
    })
    .size(px(13.))
    .text_color(p.format(ext))
}

pub fn strong(text: impl Into<SharedString>, p: &Palette) -> Div {
    div().text_color(p.text_strong).child(text.into())
}

/// The actions offered for any username, as items of a right-click menu.
pub fn user_menu<V: EventEmitter<UserAction>>(
    menu: PopupMenu,
    username: &str,
    view: WeakEntity<V>,
    cx: &App,
) -> PopupMenu {
    let (buddy, ignored) = cx.try_global::<Social>().map_or((false, false), |social| {
        (
            social.buddies.contains(username),
            social.ignored.contains(username),
        )
    });
    let name = username.to_string();
    let entries = [
        (
            "browse shares",
            IconName::FolderSearch,
            UserAction::Browse(name.clone()),
        ),
        (
            "send message",
            IconName::MessagesSquare,
            UserAction::Message(name.clone()),
        ),
        (
            "search their files",
            IconName::Search,
            UserAction::SearchUser(name.clone()),
        ),
        ("user info", IconName::Info, UserAction::Info(name.clone())),
        if buddy {
            (
                "remove from buddies",
                IconName::UserMinus,
                UserAction::SetBuddy(name.clone(), false),
            )
        } else {
            (
                "add to buddies",
                IconName::UserPlus,
                UserAction::SetBuddy(name.clone(), true),
            )
        },
        if ignored {
            (
                "stop ignoring",
                IconName::Eye,
                UserAction::SetIgnored(name.clone(), false),
            )
        } else {
            (
                "ignore",
                IconName::EyeOff,
                UserAction::SetIgnored(name, true),
            )
        },
    ];
    entries
        .into_iter()
        .fold(menu, |menu, (label, icon, action)| {
            let view = view.clone();
            menu.item(
                PopupMenuItem::new(label)
                    .icon(Icon::new(icon))
                    .on_click(move |_, _, cx| {
                        let _ = view.update(cx, |_, cx| cx.emit(action.clone()));
                    }),
            )
        })
}

pub fn user_link(
    id: impl Into<ElementId>,
    username: impl Into<SharedString>,
    color: Hsla,
    p: &Palette,
) -> Stateful<Div> {
    let hover = p.text_strong;
    div()
        .id(id)
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .cursor_pointer()
        .text_color(color)
        .hover(move |style| style.text_color(hover).underline())
        .child(username.into())
}

pub fn truncate(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(text.into())
}

/// One row of a bordered group: the first row draws the top edge, the last the bottom.
pub fn box_edges(this: Stateful<Div>, top: bool, bottom: bool, p: &Palette) -> Stateful<Div> {
    this.border_color(p.border_weak)
        .border_l_1()
        .border_r_1()
        .when(top, |this| this.border_t_1().rounded_t(px(6.)))
        .when(bottom, |this| this.border_b_1().rounded_b(px(6.)))
}
