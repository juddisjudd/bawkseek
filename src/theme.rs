use std::borrow::Cow;

use gpui_kit::component::{ActiveTheme, Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Hsla, rgb, rgba};

use crate::assets::FONTS;

const THEMES: &str = include_str!("../assets/themes/bawk.json");

pub fn init(cx: &mut App) {
    cx.text_system()
        .add_fonts(FONTS.iter().map(|bytes| Cow::Borrowed(*bytes)).collect())
        .expect("bundled fonts are valid");

    ThemeRegistry::global_mut(cx)
        .load_themes_from_str(THEMES)
        .expect("bundled theme is valid");
    let themes = ThemeRegistry::global(cx).themes().clone();
    Theme::update(cx, |theme| {
        if let Some(light) = themes.get("Bawk Light") {
            theme.light_theme = light.clone();
        }
        if let Some(dark) = themes.get("Bawk Dark") {
            theme.dark_theme = dark.clone();
        }
    });
    Theme::change(ThemeMode::Dark, None, cx);
}

/// bawkterm's color tokens, for the places the component theme has no slot for.
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Hsla,
    pub bg_weak: Hsla,
    pub bg_hover: Hsla,
    pub text: Hsla,
    pub text_weak: Hsla,
    pub text_weaker: Hsla,
    pub text_strong: Hsla,
    pub border_weak: Hsla,
    pub icon: Hsla,
    pub yolk: Hsla,
    pub yolk_dim: Hsla,
    pub danger: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
}

impl Palette {
    fn dark() -> Self {
        Self {
            bg: rgb(0x13100f).into(),
            bg_weak: rgb(0x1a1616).into(),
            bg_hover: rgb(0x241f1f).into(),
            text: rgb(0xc3bcbb).into(),
            text_weak: rgb(0x9d9696).into(),
            text_weaker: rgb(0x797372).into(),
            text_strong: rgb(0xf3efee).into(),
            border_weak: rgb(0x2c2827).into(),
            icon: rgb(0x928a89).into(),
            yolk: rgb(0xf6d56b).into(),
            yolk_dim: rgba(0xf6d56b24).into(),
            danger: rgb(0xf8767a).into(),
            success: rgb(0x6ed889).into(),
            warning: rgb(0xf7ac4d).into(),
        }
    }

    fn light() -> Self {
        Self {
            bg: rgb(0xfdfbfb).into(),
            bg_weak: rgb(0xf8f5f5).into(),
            bg_hover: rgb(0xefeceb).into(),
            text: rgb(0x403c3b).into(),
            text_weak: rgb(0x676261).into(),
            text_weaker: rgb(0x8a8584).into(),
            text_strong: rgb(0x1b1717).into(),
            border_weak: rgb(0xe1dddd).into(),
            icon: rgb(0x76706f).into(),
            yolk: rgb(0xa28200).into(),
            yolk_dim: rgba(0x84720024).into(),
            danger: rgb(0xc2272d).into(),
            success: rgb(0x137738).into(),
            warning: rgb(0xa16100).into(),
        }
    }
}

pub fn palette(cx: &App) -> Palette {
    if cx.theme().is_dark() {
        Palette::dark()
    } else {
        Palette::light()
    }
}
