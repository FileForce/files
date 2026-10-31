// A GUI app on Windows: no console window next to it (debug builds keep
// one for logs).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod archive;
mod assets;
mod commands;
mod compress;
mod create;
mod default_apps;
mod dev_shell;
mod delete;
mod extensions;
mod finder_tags;
mod forks;
mod host;
mod keymap;
mod licensing;
mod plugins;
mod menu_colors;
mod menus;
mod model;
mod palette;
mod properties;
mod settings;
mod settings_window;
mod ssh;
mod thumbnails;
mod transfer;
mod updates;
mod views;

use gpui_kit::component::TitleBar;
use gpui_kit::{
    AppContext as _, Bounds, WindowBounds, WindowOptions, application, init, open_window, point,
    px, size,
};

use crate::app::FileExplorer;
use crate::assets::AppAssets;
use crate::palette::apply_theme;

fn main() {
    host::install();
    application().with_assets(AppAssets).run(|cx| {
        init(cx);
        model::set_system_dark(matches!(
            cx.window_appearance(),
            gpui_kit::WindowAppearance::Dark | gpui_kit::WindowAppearance::VibrantDark
        ));
        let loaded = settings::load();
        loaded.apply_permissions();
        // Signed Pro plugins the license covers (the rest are never loaded).
        plugins::load_installed(&loaded);
        cx.set_reduce_motion(loaded.accessibility.reduce_motion());
        palette::apply_motion(cx, loaded.look.motion);
        apply_theme(cx, loaded.scheme, loaded.accent);
        app::register_keys(cx);
        keymap::apply(cx, &Default::default(), &loaded.keymap);
        menus::register(cx);
        let appearance = cx.new(|_| loaded);

        open_window(explorer_window_options(0), cx, move |window, cx| {
            let appearance = appearance.clone();
            cx.new(|cx| FileExplorer::new(window, cx, appearance))
        })
        .expect("Failed to open File Explorer");
    });
}

pub(crate) fn explorer_window_options(index: usize) -> WindowOptions {
    let origin = match index {
        0 => point(px(24.), px(80.)),
        1 => point(px(1200.), px(80.)),
        _ => {
            let offset = 24. + index as f32 * 32.;
            point(px(offset), px(80. + index as f32 * 32.))
        }
    };
    let mut options = TitleBar::window_options();
    options.window_bounds = Some(WindowBounds::Windowed(Bounds {
        origin,
        size: size(px(1360.), px(820.)),
    }));
    options.window_min_size = Some(size(px(1080.), px(680.)));
    if let Some(titlebar) = options.titlebar.as_mut() {
        titlebar.title = Some(if index == 0 {
            "Files — Design Concept".into()
        } else {
            format!("Files · Window {}", index + 1).into()
        });
        titlebar.traffic_light_position = Some(point(px(10.), px(16.)));
    }
    options
}
