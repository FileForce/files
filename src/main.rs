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
mod power;
mod resources;
mod sound;
mod storage;
mod session;
mod quick_actions;
mod bug_report;
mod uninstall;
mod alloc_stats;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: alloc_stats::Counting = alloc_stats::Counting;
mod desktop;
mod instance;
mod media_info;
mod pe_info;
mod tasks;
mod pdf_pages;
mod svg_stress;
mod office_text;
mod ext_preview;
mod system_notify;
mod media_keys;
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
mod video;
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
    desktop::hand_shell_folders_to_explorer();
    desktop::set_app_id();
    // DevTools → Tasks: how each kind of job runs again.
    for kind in ["thumbnail", "quick look", "program icon", "image conversion"] {
        tasks::on_retry(kind, thumbnails::retry);
    }
    tasks::on_retry("file details", media_info::retry);
    host::install();
    let launch = desktop::launch_target(std::env::args().collect());
    // Settings → General → One window: a running Files takes the folder.
    let single = settings::load().single_instance;
    if single && instance::hand_over(launch.as_deref()) {
        return;
    }
    let app = application().with_assets(AppAssets);
    // macOS: folders other apps ask Files to open (it is the folder viewer).
    app.on_open_urls(desktop::opened_from_system);
    // macOS: a click on the Dock icon with every window closed (or
    // minimized) opens one again; otherwise the windows come to the front.
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            let _ = open_main_window(cx);
        } else {
            for window in cx.windows() {
                let _ = window.update(cx, |_, window, _| window.activate_window());
            }
        }
    });
    app.run(move |cx| {
        init(cx);
        model::set_system_dark(matches!(
            cx.window_appearance(),
            gpui_kit::WindowAppearance::Dark | gpui_kit::WindowAppearance::VibrantDark
        ));
        let loaded = settings::load();
        loaded.apply_permissions();
        // Signed Pro plugins the license covers (the rest are never loaded).
        plugins::load_installed(&loaded);
        files_ext_music::spotify::set_client_id(env!("FILES_SPOTIFY_CLIENT_ID"));
        power::set_mode(loaded.power_saving, loaded.power_saving_below);
        power::refresh();
        cx.set_reduce_motion(loaded.accessibility.reduce_motion() || power::power_saving());
        palette::apply_motion(cx, loaded.look.motion);
        palette::set_look(loaded.theme, loaded.icon_theme);
        palette::set_font(loaded.font.clone());
        palette::set_corners(loaded.corners);
        apply_theme(cx, loaded.scheme, loaded.accent);
        app::register_keys(cx);
        keymap::apply(cx, &Default::default(), &loaded.keymap);
        menus::register(cx);
        let loaded_search_root = loaded.search_filters.global_root().unwrap_or_default();
        let appearance = cx.new(|_| loaded);
        power::watch(appearance.clone(), cx);
        // Linux: the menu, notifications and "Open with" know Files.
        std::thread::spawn(desktop::install_desktop_entry);
        std::thread::spawn(desktop::refresh_folder_verb);
        // The ⌘K index, built quietly a moment after start so the first
        // search finds it ready.
        let search_root = loaded_search_root.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(3));
            if !power::power_saving() {
                let _ = model::searchable_entries(&search_root);
            }
        });

        MAIN.with(|main| main.borrow_mut().appearance = Some(appearance));
        let window = open_main_window(cx).expect("Failed to open File Explorer");
        // The tabs of last time (unless started with a folder: that one shows).
        {
            let (handle, explorer) = window.clone();
            let _ = handle.update(cx, |_, window, cx| explorer.update(cx, |explorer, cx| explorer.restore_session(window, cx)));
        }
        if single {
            instance::listen();
            cx.on_app_quit(|_| async { instance::release() }).detach();
        }
        // Folders from the system (or a second start) while Files runs: each
        // in a new tab; a start without one brings the window up. With every
        // window closed, a new one opens for them.
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(300)).await;
                // Scripts waiting for a dialog (libfiles `ask`).
                for ask in quick_actions::take_asks() {
                    let _ = cx.update(|cx| {
                        let Some((handle, _)) = main_window(cx) else {
                            let _ = ask.reply.send(None);
                            return;
                        };
                        let appearance = MAIN.with(|main| main.borrow().appearance.clone());
                        let _ = handle.update(cx, |_, window, cx| match appearance {
                            Some(appearance) => views::quick_actions::ask_for_script(ask, appearance, window, cx),
                            None => {
                                let _ = ask.reply.send(None);
                            }
                        });
                    });
                }
                let opened = desktop::take_opened();
                let activate = instance::take_activate();
                if opened.is_empty() && !activate {
                    continue;
                }
                let _ = cx.update(|cx| {
                    let Some((handle, explorer)) = main_window(cx).or_else(|| open_main_window(cx)) else { return };
                    cx.activate(true);
                    let _ = handle.update(cx, |_, window, cx| {
                        window.activate_window();
                        explorer.update(cx, |explorer, cx| {
                            for path in opened {
                                if path.is_dir() {
                                    explorer.open_in_new_tab(crate::model::Location::from_path(&path), window, cx);
                                } else {
                                    explorer.reveal_path(&path, window, cx);
                                }
                            }
                        })
                    });
                });
            }
        })
        .detach();
        // Started with a folder (the default file manager, `files ~/Music`):
        // open it; a file: its folder with it selected.
        if let Some(path) = launch {
            let (handle, explorer) = window;
            let _ = handle.update(cx, |_, window, cx| {
                explorer.update(cx, |explorer, cx| {
                    // A new tab beside the restored ones.
                    if path.is_dir() {
                        explorer.open_in_new_tab(crate::model::Location::from_path(&path), window, cx);
                    } else {
                        explorer.reveal_path(&path, window, cx);
                    }
                })
            });
        }
    });
}

/// The main windows (the first one, and those reopened from the Dock), and
/// the settings they share.
#[derive(Default)]
struct Main {
    appearance: Option<gpui_kit::Entity<app::AppearanceSettings>>,
    windows: Vec<(gpui_kit::AnyWindowHandle, gpui_kit::WeakEntity<FileExplorer>)>,
}

thread_local! {
    static MAIN: std::cell::RefCell<Main> = std::cell::RefCell::new(Main::default());
}

/// Opens a main window.
fn open_main_window(cx: &mut gpui_kit::App) -> Option<(gpui_kit::AnyWindowHandle, gpui_kit::Entity<FileExplorer>)> {
    let appearance = MAIN.with(|main| main.borrow().appearance.clone())?;
    let (handle, explorer) = open_window(explorer_window_options(0), cx, move |window, cx| cx.new(|cx| FileExplorer::new(window, cx, appearance.clone()))).ok()?;
    MAIN.with(|main| main.borrow_mut().windows.push((handle, explorer.downgrade())));
    Some((handle, explorer))
}

/// A main window still open.
fn main_window(cx: &mut gpui_kit::App) -> Option<(gpui_kit::AnyWindowHandle, gpui_kit::Entity<FileExplorer>)> {
    let open: Vec<gpui_kit::AnyWindowHandle> = cx.windows();
    MAIN.with(|main| {
        let mut main = main.borrow_mut();
        main.windows.retain(|(handle, explorer)| open.contains(handle) && explorer.upgrade().is_some());
        main.windows.first().and_then(|(handle, explorer)| Some((*handle, explorer.upgrade()?)))
    })
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
