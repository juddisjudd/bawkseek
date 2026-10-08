use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, Sizable, TitleBar};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::assets::MARK;
use crate::theme::palette;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Search,
    Transfers,
    Uploads,
    Browse,
    Rooms,
    Messages,
    Users,
    Settings,
}

impl Page {
    pub const MAIN: [Page; 7] = [
        Page::Search,
        Page::Transfers,
        Page::Uploads,
        Page::Browse,
        Page::Rooms,
        Page::Messages,
        Page::Users,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Page::Search => "search",
            Page::Transfers => "transfers",
            Page::Uploads => "uploads",
            Page::Browse => "browse",
            Page::Rooms => "rooms",
            Page::Messages => "messages",
            Page::Users => "users",
            Page::Settings => "settings",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Page::Search => IconName::Search,
            Page::Transfers => IconName::Download,
            Page::Uploads => IconName::Upload,
            Page::Browse => IconName::FolderSearch,
            Page::Rooms => IconName::Hash,
            Page::Messages => IconName::MessagesSquare,
            Page::Users => IconName::Users,
            Page::Settings => IconName::Settings,
        }
    }
}

pub struct Presence {
    pub username: SharedString,
    pub online: bool,
    pub away: bool,
}

pub fn title_bar(presence: Option<Presence>, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    let wordmark = div()
        .flex()
        .items_center()
        .gap(px(7.))
        .child(svg().path(MARK).size(px(17.)).text_color(p.text_strong))
        .child(
            div()
                .flex()
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(div().text_color(p.text_strong).child("bawk"))
                .child(div().text_color(p.text_weak).child("seek")),
        );

    let status = presence.map(|presence| {
        let color = if presence.online && !presence.away {
            p.success
        } else {
            p.warning
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .pr_3()
            .text_size(px(12.))
            .text_color(p.text_weak)
            .child(div().size(px(7.)).rounded_full().bg(color))
            .child(presence.username)
            .when(presence.away, |this| this.child("· away"))
    });

    TitleBar::new()
        .h(px(40.))
        .bg(p.bg)
        .border_color(p.border_weak)
        .child(wordmark)
        .children(status)
}

pub fn sidebar(
    current: Page,
    counts: &[(Page, usize)],
    on_select: impl Fn(Page, &mut Window, &mut App) + Clone + 'static,
    cx: &App,
) -> impl IntoElement {
    let p = palette(cx);
    let item = |page: Page| {
        let active = page == current;
        let count = counts
            .iter()
            .find(|(candidate, _)| *candidate == page)
            .map(|(_, count)| *count)
            .filter(|count| *count > 0);
        let on_select = on_select.clone();
        div()
            .id(page.label())
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(4.))
            .cursor_pointer()
            .text_color(if active { p.text_strong } else { p.text })
            .when(active, |this| this.bg(p.bg_hover))
            .when(!active, |this| this.hover(|style| style.bg(p.bg_weak)))
            .on_click(move |_, window, cx| on_select(page, window, cx))
            .child(Icon::new(page.icon()).small().text_color(if active {
                p.text_strong
            } else {
                p.icon
            }))
            .child(div().flex_1().child(page.label()))
            .children(count.map(|count| {
                div()
                    .text_size(px(12.))
                    .text_color(p.text_weak)
                    .child(count.to_string())
            }))
    };

    div()
        .w(px(200.))
        .flex_shrink_0()
        .h_full()
        .flex()
        .flex_col()
        .justify_between()
        .p(px(8.))
        .border_r_1()
        .border_color(p.border_weak)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .children(Page::MAIN.map(item)),
        )
        .child(item(Page::Settings))
}
