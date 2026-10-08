use std::borrow::Cow;
use std::collections::HashMap;

use gpui_kit::component::{Colorize, Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Global, Hsla, SharedString, Window, rgb, rgba};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::assets::FONTS;

const BAWK: &str = include_str!("../assets/themes/bawk.json");
const PRESETS: &str = include_str!("../assets/themes/presets.json");

pub const DEFAULT_THEME: &str = "bawk";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    System,
    Dark,
    Light,
}

/// A theme as the appearance settings show it: a name on its own colors.
#[derive(Clone)]
pub struct Look {
    pub id: SharedString,
    pub label: SharedString,
    pub dark: bool,
    pub bg: Hsla,
    pub fg: Hsla,
    pub cursor: Hsla,
    pub swatches: [Hsla; 6],
    palette: Palette,
    config: SharedString,
}

struct Looks {
    list: Vec<Look>,
    active: Palette,
}

impl Global for Looks {}

#[derive(Deserialize)]
struct Preset {
    id: String,
    label: String,
    dark: bool,
    card: Card,
    colors: HashMap<String, String>,
}

#[derive(Deserialize)]
struct Card {
    bg: String,
    fg: String,
    cursor: String,
    swatches: [String; 6],
}

pub fn init(cx: &mut App) {
    cx.text_system()
        .add_fonts(FONTS.iter().map(|bytes| Cow::Borrowed(*bytes)).collect())
        .expect("bundled fonts are valid");

    let bawk: Value = serde_json::from_str(BAWK).expect("bundled theme is valid");
    let presets: Vec<Preset> = serde_json::from_str(PRESETS).expect("bundled presets are valid");

    let mut themes = Vec::new();
    let mut list = vec![bawk_look()];
    for preset in &presets {
        let tokens: HashMap<&str, Hsla> = preset
            .colors
            .iter()
            .map(|(key, value)| (key.as_str(), hex(value)))
            .collect();
        let config = format!("bawkseek {}", preset.id);
        themes.push(theme_config(&bawk, &config, preset.dark, &tokens));
        list.push(Look {
            id: preset.id.clone().into(),
            label: preset.label.clone().into(),
            dark: preset.dark,
            bg: hex(&preset.card.bg),
            fg: hex(&preset.card.fg),
            cursor: hex(&preset.card.cursor),
            swatches: preset.card.swatches.each_ref().map(|color| hex(color)),
            palette: Palette::from_tokens(&tokens),
            config: config.into(),
        });
    }

    let registry = ThemeRegistry::global_mut(cx);
    registry
        .load_themes_from_str(BAWK)
        .expect("bundled theme is valid");
    registry
        .load_themes_from_str(&json!({ "name": "bawkseek presets", "themes": themes }).to_string())
        .expect("generated themes are valid");

    cx.set_global(Looks {
        list,
        active: Palette::dark(),
    });
    apply(DEFAULT_THEME, Mode::Dark, None, cx);
}

pub fn looks(cx: &App) -> &[Look] {
    &cx.global::<Looks>().list
}

/// Switches the whole app to a theme. `mode` only matters for bawk's own theme, the others are dark or light by design.
pub fn apply(id: &str, mode: Mode, window: Option<&mut Window>, cx: &mut App) {
    let appearance = window
        .as_ref()
        .map(|window| window.appearance())
        .unwrap_or_else(|| cx.window_appearance());
    let looks = cx.global::<Looks>();
    let look = looks
        .list
        .iter()
        .find(|look| look.id == id)
        .unwrap_or(&looks.list[0])
        .clone();

    let (dark, palette, light_name, dark_name) = if look.id == DEFAULT_THEME {
        let dark = match mode {
            Mode::System => ThemeMode::from(appearance).is_dark(),
            Mode::Dark => true,
            Mode::Light => false,
        };
        let palette = if dark {
            Palette::dark()
        } else {
            Palette::light()
        };
        (dark, palette, "Bawk Light".into(), "Bawk Dark".into())
    } else {
        (look.dark, look.palette, look.config.clone(), look.config)
    };

    let themes = ThemeRegistry::global(cx).themes().clone();
    cx.global_mut::<Looks>().active = palette;
    Theme::update(cx, |theme| {
        if let Some(config) = themes.get(&light_name) {
            theme.light_theme = config.clone();
        }
        if let Some(config) = themes.get(&dark_name) {
            theme.dark_theme = config.clone();
        }
    });
    let mode = if dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    Theme::change(mode, window, cx);
}

fn hex(value: &str) -> Hsla {
    Hsla::parse_hex(value).unwrap_or_default()
}

fn bawk_look() -> Look {
    let p = Palette::dark();
    Look {
        id: DEFAULT_THEME.into(),
        label: "bawk".into(),
        dark: true,
        bg: p.bg,
        fg: p.text_strong,
        cursor: p.yolk,
        swatches: [p.red, p.green, p.yellow, p.blue, p.purple, p.orange],
        palette: p,
        config: "Bawk Dark".into(),
    }
}

/// The component colors for a named theme, laid over bawk's own theme of the same mode.
fn theme_config(bawk: &Value, name: &str, dark: bool, t: &HashMap<&str, Hsla>) -> Value {
    let base = if dark { "Bawk Dark" } else { "Bawk Light" };
    let mut config = bawk["themes"]
        .as_array()
        .and_then(|themes| themes.iter().find(|theme| theme["name"] == base))
        .cloned()
        .expect("bawk.json has both modes");
    config["name"] = name.into();

    let c = |key: &str| t.get(key).copied().unwrap_or_default();
    let h = |color: Hsla| Value::from(color.to_hex());
    let shade = |color: Hsla, amount: f32| {
        if dark {
            color.darken(amount)
        } else {
            color.lighten(amount)
        }
    };
    let raise = |color: Hsla, amount: f32| {
        if dark {
            color.lighten(amount)
        } else {
            color.darken(amount)
        }
    };
    let (bg, accent) = (c("bg"), c("accent"));
    let mut colors = Map::new();
    let mut set = |key: &str, color: Hsla| {
        colors.insert(key.into(), h(color));
    };

    set("background", bg);
    set("foreground", c("text-strong"));
    set("border", c("border-weak"));
    set("input.border", c("border"));
    set("ring", c("focus"));
    set("caret", c("text-strong"));
    set("selection.background", accent.opacity(0.25));
    set("window.border", c("border-weak"));
    set("muted.background", c("bg-weak"));
    set("muted.foreground", c("text-weak"));
    set("accent.background", c("bg-weak-hover"));
    set("accent.foreground", c("text-strong"));
    set("primary.background", c("bg-strong"));
    set("primary.hover.background", c("bg-strong-hover"));
    set("primary.active.background", shade(c("bg-strong"), 0.06));
    set("primary.foreground", bg);
    set("secondary.background", c("bg-weak"));
    set("secondary.hover.background", c("bg-weak-hover"));
    set("secondary.active.background", c("border-weak"));
    set("secondary.foreground", c("text-strong"));
    for (name, token) in [
        ("danger", "danger"),
        ("success", "success"),
        ("warning", "warning"),
        ("info", "folder-blue"),
    ] {
        set(&format!("{name}.background"), c(token));
        set(&format!("{name}.hover.background"), raise(c(token), 0.08));
        set(&format!("{name}.active.background"), shade(c(token), 0.08));
        set(&format!("{name}.foreground"), bg);
    }
    set("link", c("text-strong"));
    set("link.hover", raise(c("text-strong"), 0.1));
    set("link.active", c("text"));
    set("drag.border", accent);
    set("drop_target.background", accent.opacity(0.1));
    for list in ["list", "table"] {
        set(&format!("{list}.background"), bg);
        set(&format!("{list}.even.background"), bg);
        set(&format!("{list}.hover.background"), c("bg-weak"));
        set(&format!("{list}.active.background"), c("bg-selected"));
        set(&format!("{list}.active.border"), accent);
    }
    set("list.head.background", c("bg-weak"));
    set("table.head.background", bg);
    set("table.head.foreground", c("text-weak"));
    set("table.row.border", c("border-weak"));
    set("popover.background", if dark { c("bg-weak") } else { bg });
    set("popover.foreground", c("text-strong"));
    set("group_box.background", c("bg-weak"));
    set("group_box.foreground", c("text-strong"));
    set("accordion.background", bg);
    set("skeleton.background", c("bg-weak"));
    set("scrollbar.thumb.background", c("border-weak"));
    set("scrollbar.thumb.hover.background", c("border"));
    set("sidebar.background", bg);
    set("sidebar.foreground", c("text"));
    set("sidebar.border", c("border-weak"));
    set("sidebar.accent.background", c("bg-weak-hover"));
    set("sidebar.accent.foreground", c("text-strong"));
    set("sidebar.primary.background", c("bg-strong"));
    set("sidebar.primary.foreground", bg);
    set("tab.foreground", c("text-weak"));
    set("tab.active.foreground", c("text-strong"));
    set("tab_bar.background", bg);
    set("tab_bar.segmented.background", c("bg-weak"));
    set("title_bar.background", bg);
    set("title_bar.border", c("border-weak"));
    set("status_bar.background", bg);
    set("status_bar.border", c("border-weak"));
    set("progress.bar.background", accent);
    set("slider.background", accent);
    set("slider.thumb.background", bg);
    set("switch.background", c("border-weak").mix(c("border"), 0.6));
    set(
        "switch.thumb.background",
        if dark { c("text-weak") } else { bg },
    );
    set("chart.1", accent);
    set("chart.2", c("folder-green"));
    set("chart.3", c("folder-blue"));
    set("chart.4", c("folder-orange"));
    set("chart.5", c("folder-purple"));
    set("chart.bullish", c("folder-green"));
    set("chart.bearish", c("folder-red"));
    set("chart.grid", c("border-weak").opacity(0.6));
    for (name, token) in [
        ("red", "folder-red"),
        ("green", "folder-green"),
        ("blue", "folder-blue"),
        ("yellow", "folder-yellow"),
        ("magenta", "folder-purple"),
        ("cyan", "folder-blue"),
    ] {
        set(&format!("base.{name}"), c(token));
        set(&format!("base.{name}.light"), c(token).mix(bg, 0.6));
    }

    if let Some(target) = config["colors"].as_object_mut() {
        target.extend(colors);
    }
    config
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
    pub red: Hsla,
    pub orange: Hsla,
    pub yellow: Hsla,
    pub green: Hsla,
    pub blue: Hsla,
    pub purple: Hsla,
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
            red: rgb(0xf17070).into(),
            orange: rgb(0xf49752).into(),
            yellow: rgb(0xeed059).into(),
            green: rgb(0x61cb7c).into(),
            blue: rgb(0x60aaf3).into(),
            purple: rgb(0xb98cea).into(),
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
            red: rgb(0xcc3336).into(),
            orange: rgb(0xc9690c).into(),
            yellow: rgb(0x9f7a00).into(),
            green: rgb(0x218a45).into(),
            blue: rgb(0x2971c6).into(),
            purple: rgb(0x8048b6).into(),
        }
    }

    fn from_tokens(t: &HashMap<&str, Hsla>) -> Self {
        let c = |key: &str| t.get(key).copied().unwrap_or_default();
        Self {
            bg: c("bg"),
            bg_weak: c("bg-weak"),
            bg_hover: c("bg-weak-hover"),
            text: c("text"),
            text_weak: c("text-weak"),
            text_weaker: c("text-weaker"),
            text_strong: c("text-strong"),
            border_weak: c("border-weak"),
            icon: c("icon"),
            yolk: c("accent"),
            yolk_dim: c("accent").opacity(0.14),
            danger: c("danger"),
            success: c("success"),
            warning: c("warning"),
            red: c("folder-red"),
            orange: c("folder-orange"),
            yellow: c("folder-yellow"),
            green: c("folder-green"),
            blue: c("folder-blue"),
            purple: c("folder-purple"),
        }
    }

    /// Cool colors for lossless formats and warm ones for lossy, so a glance tells them apart.
    pub fn format(&self, ext: &str) -> Hsla {
        match ext {
            "flac" => self.green,
            "wav" | "aiff" | "aif" => self.blue,
            "alac" | "ape" | "wv" => self.purple,
            "mp3" => self.yellow,
            "ogg" | "opus" => self.orange,
            "m4a" | "aac" | "wma" => self.red,
            _ => self.text_weaker,
        }
    }
}

pub fn palette(cx: &App) -> Palette {
    cx.global::<Looks>().active
}

#[cfg(test)]
mod tests {
    use super::{PRESETS, Preset};

    #[test]
    fn every_preset_has_every_color() {
        let presets: Vec<Preset> = serde_json::from_str(PRESETS).unwrap();
        assert_eq!(presets.len(), 30);
        for preset in &presets {
            assert_eq!(preset.colors.len(), 26, "{}", preset.id);
            assert!(
                preset
                    .colors
                    .values()
                    .all(|value| super::hex(value) != Default::default()),
                "{}",
                preset.id
            );
        }
    }
}
