#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod audio;
mod chats;
mod config;
mod format;
mod library;
mod logging;
mod net;
mod theme;
mod ui;
mod updater;

use gpui_kit::component::TitleBar;
use gpui_kit::*;

fn main() {
    logging::init();
    updater::clean_up();
    application().with_assets(assets::AppAssets).run(|cx| {
        init(cx);
        cx.set_app_identity("com.bawkseek.app", "bawkseek");
        theme::init(cx);

        let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(860.), px(560.))),
            ..TitleBar::window_options()
        };
        open_window(options, cx, |window, cx| {
            window.set_window_title("bawkseek");
            cx.new(|cx| ui::Workspace::new(window, cx))
        })
        .expect("failed to open window");
        cx.activate(true);
    });
}
