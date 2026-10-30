//! The macOS menu bar (on other platforms GPUI shows no global menu).
//! Items dispatch the same actions as the keyboard shortcuts, so the menu
//! shows the user's current shortcuts automatically.

use gpui_kit::{App, Menu, MenuItem, OsAction, SystemMenuType, actions};

use crate::app::*;

actions!(app_menu, [Quit, Hide, HideOthers, ShowAll, About, CheckForUpdates]);

pub fn register(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.set_menus(menus());
}

fn menus() -> Vec<Menu> {
    vec![
        Menu::new("Files").items([
            MenuItem::action("About Files", About),
            MenuItem::action("Check for Updates…", CheckForUpdates),
            MenuItem::separator(),
            MenuItem::action("Settings…", ShowSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Files", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Files", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New File…", NewFile),
            MenuItem::action("New Folder…", NewFolder),
            MenuItem::action("Fork Selected Folder", ForkHere),
            MenuItem::separator(),
            MenuItem::action("New Tab", NewTab),
            MenuItem::action("New Window", NewWindow),
            MenuItem::action("Close Tab", CloseTab),
            MenuItem::separator(),
            MenuItem::action("Open", OpenSelected),
            MenuItem::action("Quick Look", QuickLook),
            MenuItem::separator(),
            MenuItem::action("Search in Folder", FocusSearch),
            MenuItem::action("Global Search…", ShowGlobalSearch),
        ]),
        Menu::new("Edit").items([
            // Text fields handle these themselves; in lists they act on files.
            MenuItem::os_action("Cut", CutFiles, OsAction::Cut),
            MenuItem::os_action("Copy", CopyFiles, OsAction::Copy),
            MenuItem::os_action("Paste", PasteFiles, OsAction::Paste),
            MenuItem::separator(),
            MenuItem::action("Select All", SelectAll),
            MenuItem::action("Copy Path", CopySelectedPath),
            MenuItem::separator(),
            MenuItem::action("Move to Trash", DeleteSelected),
        ]),
        Menu::new("View").items([
            MenuItem::action("Expressive", ShowExpressive),
            MenuItem::action("Pro", ShowPro),
            MenuItem::action("Dual", ShowDual),
            MenuItem::separator(),
            MenuItem::action("Toggle Preview Panel", ToggleInspector),
            MenuItem::action("Toggle Theme", ToggleTheme),
        ]),
        Menu::new("Window").items([MenuItem::action("New Window", NewWindow)]),
    ]
}
