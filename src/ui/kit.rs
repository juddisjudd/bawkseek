use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::Palette;

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

pub fn strong(text: impl Into<SharedString>, p: &Palette) -> Div {
    div().text_color(p.text_strong).child(text.into())
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
