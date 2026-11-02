//! Every keyboard shortcut in one table, so Settings → Hotkeys can list,
//! search and rebind them (the Zed / VS Code approach). User overrides live in
//! `settings.toml` under `[keymap]` as `command id = "keystroke"`; an empty
//! string unbinds a command.

use std::collections::BTreeMap;
use std::rc::Rc;

use gpui_kit::{Action, App, KeyBinding, KeyBindingContextPredicate, NoAction};

/// One rebindable command.
pub struct Command {
    /// Stable id used in settings.toml, e.g. `"explorer.new_tab"`.
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    /// Where the shortcut works (a GPUI key context).
    pub context: &'static str,
    pub mac: &'static [&'static str],
    pub other: &'static [&'static str],
    pub action: fn() -> Box<dyn Action>,
    /// The extension providing this command; it only works while enabled.
    pub extension: Option<&'static str>,
}

impl Command {
    pub fn defaults(&self, mac: bool) -> &'static [&'static str] {
        if mac { self.mac } else { self.other }
    }
}

macro_rules! command {
    ($id:literal, $label:literal, $group:literal, $context:literal, $mac:expr, $other:expr, $action:expr) => {
        Command {
            id: $id,
            label: $label,
            group: $group,
            context: $context,
            mac: $mac,
            other: $other,
            action: || Box::new($action),
            extension: None,
        }
    };
    (extension $ext:literal, $id:literal, $label:literal, $context:literal, $mac:expr, $other:expr, $action:expr) => {
        Command {
            id: $id,
            label: $label,
            group: concat!("Extension · ", $ext),
            context: $context,
            mac: $mac,
            other: $other,
            action: || Box::new($action),
            extension: Some($ext),
        }
    };
}

/// Sequences are bound twice: with the modifier released after the first key
/// (`cmd-n f f`) and held all the way (`cmd-n cmd-f cmd-f`), since people
/// type them both ways.
pub fn commands() -> Vec<Command> {
    use crate::app::*;
    vec![
        command!("app.quit", "Quit", "App", "", &["cmd-q"], &["ctrl-q"], crate::menus::Quit),
        command!("app.hide", "Hide app", "App", "", &["cmd-h"], &[], crate::menus::Hide),
        command!("app.hide_others", "Hide others", "App", "", &["cmd-alt-h"], &[], crate::menus::HideOthers),
        command!("explorer.view_expressive", "Expressive view", "View", "FileExplorer", &["cmd-1"], &["ctrl-1"], ShowExpressive),
        command!("explorer.view_pro", "Pro view", "View", "FileExplorer", &["cmd-2"], &["ctrl-2"], ShowPro),
        command!("explorer.view_dual", "Dual view", "View", "FileExplorer", &["cmd-3"], &["ctrl-3"], ShowDual),
        command!("explorer.toggle_theme", "Toggle theme", "View", "FileExplorer", &["cmd-shift-t"], &["ctrl-shift-t"], ToggleTheme),
        command!("explorer.toggle_inspector", "Toggle preview panel", "View", "FileExplorer", &["cmd-alt-p"], &["ctrl-alt-p"], ToggleInspector),
        command!("explorer.quick_look", "Quick Look", "Files", "FileExplorer", &["space"], &["space"], QuickLook),
        command!("explorer.open", "Open selected", "Files", "FileExplorer", &["enter"], &["enter"], OpenSelected),
        command!("explorer.new_file", "New file", "Files", "FileExplorer", &["cmd-n"], &["ctrl-n"], NewFile),
        command!("explorer.new_folder", "New folder", "Files", "FileExplorer", &["cmd-n f", "cmd-n cmd-f"], &["ctrl-n f", "ctrl-n ctrl-f"], NewFolder),
        command!(extension "Forks", "forks.fork", "Fork selected folder", "FileExplorer", &["cmd-n f f", "cmd-n cmd-f cmd-f"], &["ctrl-n f f", "ctrl-n ctrl-f ctrl-f"], ForkHere),
        command!("explorer.delete", "Delete selected", "Files", "FileExplorer", &["cmd-backspace", "delete"], &["delete"], DeleteSelected),
        command!("edit.copy", "Copy files", "Files", "FileExplorer", &["cmd-c"], &["ctrl-c"], CopyFiles),
        command!("edit.cut", "Cut files", "Files", "FileExplorer", &["cmd-x"], &["ctrl-x"], CutFiles),
        command!("edit.paste", "Paste files", "Files", "FileExplorer", &["cmd-v"], &["ctrl-v"], PasteFiles),
        command!("explorer.select_all", "Select all", "Selection", "FileExplorer", &["cmd-a"], &["ctrl-a"], SelectAll),
        command!("explorer.select_previous", "Select previous", "Selection", "FileExplorer", &["up"], &["up"], SelectPrevious),
        command!("explorer.select_next", "Select next", "Selection", "FileExplorer", &["down"], &["down"], SelectNext),
        command!("explorer.extend_up", "Extend selection up", "Selection", "FileExplorer", &["shift-up"], &["shift-up"], ExtendSelectionUp),
        command!("explorer.extend_down", "Extend selection down", "Selection", "FileExplorer", &["shift-down"], &["shift-down"], ExtendSelectionDown),
        command!("explorer.focus_search", "Search in folder", "Search", "FileExplorer", &["cmd-f"], &["ctrl-f"], FocusSearch),
        command!("explorer.global_search", "Global search", "Search", "FileExplorer", &["cmd-k"], &["ctrl-k"], ShowGlobalSearch),
        command!("explorer.new_tab", "New tab", "Tabs & windows", "FileExplorer", &["cmd-t"], &["ctrl-t"], NewTab),
        command!("explorer.close_tab", "Close tab", "Tabs & windows", "FileExplorer", &["cmd-w"], &["ctrl-w"], CloseTab),
        command!("explorer.close_other_tabs", "Close other tabs", "Tabs & windows", "FileExplorer", &["cmd-alt-w"], &["ctrl-alt-w"], CloseOtherTabs),
        command!("explorer.close_tabs_left", "Close tabs to the left", "Tabs & windows", "FileExplorer", &[], &[], CloseTabsLeft),
        command!("explorer.close_tabs_right", "Close tabs to the right", "Tabs & windows", "FileExplorer", &[], &[], CloseTabsRight),
        command!("explorer.toggle_pin_tab", "Pin or unpin tab", "Tabs & windows", "FileExplorer", &[], &[], TogglePinTab),
        command!("explorer.new_window", "New window", "Tabs & windows", "FileExplorer", &["cmd-shift-n"], &["ctrl-shift-n"], NewWindow),
        command!("explorer.settings", "Open settings", "Tabs & windows", "FileExplorer", &["cmd-,"], &["ctrl-,"], ShowSettings),
        command!("explorer.focus_left", "Focus left pane", "Dual panes", "FileExplorer", &["left"], &["left"], FocusLeftPane),
        command!("explorer.focus_right", "Focus right pane", "Dual panes", "FileExplorer", &["right"], &["right"], FocusRightPane),
        command!("explorer.focus_next", "Focus next control", "Focus", "FileExplorer", &["tab"], &["tab"], FocusNext),
        command!("explorer.focus_previous", "Focus previous control", "Focus", "FileExplorer", &["shift-tab"], &["shift-tab"], FocusPrevious),
        command!(extension "Open With", "open_with.choose", "Open with…", "FileExplorer", &["cmd-alt-o"], &["ctrl-alt-o"], OpenWithApp),
        command!(extension "Archives", "archives.compress", "Compress selected", "FileExplorer", &["cmd-alt-z"], &["ctrl-alt-z"], CompressSelected),
        command!(extension "Archives", "archives.extract", "Extract selected archive", "FileExplorer", &["cmd-alt-x"], &["ctrl-alt-x"], ExtractSelected),
        command!(extension "Copy", "copy.path", "Copy path", "FileExplorer", &["cmd-alt-c"], &["ctrl-alt-c"], CopySelectedPath),
        command!(extension "Terminal", "terminal.open", "Open folder in terminal", "FileExplorer", &["cmd-alt-t"], &["ctrl-alt-t"], OpenTerminalHere),
        command!("quick_look.close", "Close Quick Look", "Quick Look", "QuickLook", &["space", "escape"], &["space", "escape"], crate::views::quick_look::CloseQuickLook),
        command!("quick_look.previous", "Previous item", "Quick Look", "QuickLook", &["up", "left"], &["up", "left"], crate::views::quick_look::QuickLookPrevious),
        command!("quick_look.next", "Next item", "Quick Look", "QuickLook", &["down", "right"], &["down", "right"], crate::views::quick_look::QuickLookNext),
        command!("quick_look.open", "Open in app", "Quick Look", "QuickLook", &["enter"], &["enter"], crate::views::quick_look::QuickLookOpen),
        command!("settings.close", "Close settings", "Settings", "Settings", &["escape", "cmd-w"], &["escape", "ctrl-w"], crate::settings_window::CloseSettings),
        command!("settings.previous_section", "Previous section", "Settings", "Settings", &["up"], &["up"], crate::settings_window::SettingsPrevious),
        command!("settings.next_section", "Next section", "Settings", "Settings", &["down"], &["down"], crate::settings_window::SettingsNext),
        command!("settings.focus_next", "Focus next control", "Settings", "Settings", &["tab"], &["tab"], crate::settings_window::SettingsFocusNext),
        command!("settings.focus_previous", "Focus previous control", "Settings", "Settings", &["shift-tab"], &["shift-tab"], crate::settings_window::SettingsFocusPrevious),
        command!("archive.close", "Close archive window", "Windows", "Archive", &["escape", "cmd-w"], &["escape", "ctrl-w"], crate::archive::CloseArchive),
        command!("properties.close", "Close properties", "Windows", "Properties", &["escape", "cmd-w"], &["escape", "ctrl-w"], crate::properties::CloseProperties),
        command!("git.close", "Close Git window", "Windows", "Git", &["escape", "cmd-w"], &["escape", "ctrl-w"], crate::views::git::CloseGit),
        command!("mail.close", "Close Mail window", "Windows", "Mail", &["cmd-w"], &["ctrl-w"], crate::views::mail::CloseMail),
        command!("agents.close", "Close agent window", "Windows", "Agents", &["escape", "cmd-w"], &["escape", "ctrl-w"], crate::views::agents::CloseAgents),
        command!("compress.close", "Close compress window", "Windows", "Compress", &["escape", "cmd-w"], &["escape", "ctrl-w"], crate::compress::CloseCompress),
    ]
    .into_iter()
    .collect()
}

/// The keystrokes a command answers to with `overrides` applied.
pub fn effective(command: &Command, overrides: &BTreeMap<String, String>, mac: bool) -> Vec<String> {
    match overrides.get(command.id) {
        Some(keys) if keys.trim().is_empty() => Vec::new(),
        Some(keys) => vec![keys.trim().to_string()],
        None => command.defaults(mac).iter().map(|keys| keys.to_string()).collect(),
    }
}

/// An empty context means "anywhere in the app" (no predicate).
fn binding(keys: &str, action: Box<dyn Action>, context: &str, cx: &App) -> Option<KeyBinding> {
    let predicate = if context.is_empty() {
        None
    } else {
        KeyBindingContextPredicate::parse(context).ok().map(Rc::new)
    };
    KeyBinding::load(keys, action, predicate, false, None, cx).ok()
}

/// Binds every command, honouring `overrides`. Safe to call again after
/// overrides change: later bindings win, and keys a command no longer uses
/// are bound to `NoAction` in its context, so the old shortcut stops working.
pub fn apply(cx: &mut App, previous: &BTreeMap<String, String>, overrides: &BTreeMap<String, String>) {
    let mac = cx.platform_info().is_mac();
    let mut bindings = Vec::new();
    for command in commands() {
        let now = effective(&command, overrides, mac);
        for old in effective(&command, previous, mac) {
            if !now.contains(&old) && !old.is_empty() {
                bindings.extend(binding(&old, Box::new(NoAction), command.context, cx));
            }
        }
        for keys in &now {
            bindings.extend(binding(keys, (command.action)(), command.context, cx));
        }
    }
    cx.bind_keys(bindings);
}

/// Commands provided by one extension (by its catalog name).
pub fn for_extension(name: &str) -> Vec<Command> {
    commands().into_iter().filter(|command| command.extension == Some(name)).collect()
}

/// Commands other than `id` that already use `keys` in `id`'s context.
pub fn clashes(id: &str, keys: &str, overrides: &BTreeMap<String, String>, mac: bool) -> Vec<&'static str> {
    let all = commands();
    let Some(command) = all.iter().find(|command| command.id == id) else {
        return Vec::new();
    };
    all.iter()
        .filter(|other| other.id != id && other.context == command.context)
        .filter(|other| effective(other, overrides, mac).iter().any(|other_keys| other_keys == keys))
        .map(|other| other.label)
        .collect()
}

/// Commands that share a keystroke in the same context.
pub fn conflicts(overrides: &BTreeMap<String, String>, mac: bool) -> BTreeMap<String, Vec<&'static str>> {
    let mut by_key: BTreeMap<String, Vec<&'static str>> = BTreeMap::new();
    for command in commands() {
        for keys in effective(&command, overrides, mac) {
            by_key.entry(format!("{}|{keys}", command.context)).or_default().push(command.id);
        }
    }
    by_key.retain(|_, ids| ids.len() > 1);
    by_key
}

/// Formats a pressed keystroke the way bindings are written: `cmd-shift-k`.
pub fn keystroke_text(keystroke: &gpui_kit::Keystroke) -> String {
    let modifiers = &keystroke.modifiers;
    let mut parts = Vec::new();
    if modifiers.control {
        parts.push("ctrl");
    }
    if modifiers.alt {
        parts.push("alt");
    }
    if modifiers.shift {
        parts.push("shift");
    }
    if modifiers.platform {
        parts.push(if cfg!(target_os = "macos") { "cmd" } else { "super" });
    }
    if modifiers.function {
        parts.push("fn");
    }
    parts.push(keystroke.key.as_str());
    parts.join("-")
}

/// Pretty label for a keystroke, e.g. `cmd-shift-k` → `⌘⇧K` on macOS.
pub fn display(keys: &str, mac: bool) -> String {
    if keys.is_empty() {
        return "—".to_string();
    }
    keys.split(' ')
        .map(|stroke| {
            let parts: Vec<&str> = stroke.split('-').collect();
            let (key, modifiers) = match parts.split_last() {
                // "cmd--" style (minus key) leaves an empty last part.
                Some((key, modifiers)) => (*key, modifiers),
                None => ("", &[][..]),
            };
            let key = match key {
                "space" => if mac { "Space" } else { "Space" },
                "enter" => if mac { "↩" } else { "Enter" },
                "escape" => "Esc",
                "backspace" => if mac { "⌫" } else { "Backspace" },
                "delete" => if mac { "⌦" } else { "Del" },
                "tab" => "Tab",
                "up" => "↑",
                "down" => "↓",
                "left" => "←",
                "right" => "→",
                other => other,
            }
            .to_string();
            let key = if key.chars().count() == 1 { key.to_uppercase() } else { key };
            let symbols: Vec<&str> = modifiers
                .iter()
                .map(|modifier| match (*modifier, mac) {
                    ("cmd", true) => "⌘",
                    ("shift", true) => "⇧",
                    ("alt", true) => "⌥",
                    ("ctrl", true) => "⌃",
                    ("cmd", false) | ("super", false) => "Super+",
                    ("shift", false) => "Shift+",
                    ("alt", false) => "Alt+",
                    ("ctrl", false) => "Ctrl+",
                    (other, _) => other,
                })
                .collect();
            format!("{}{key}", symbols.concat())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_defaults_parse() {
        let commands = commands();
        let mut ids: Vec<_> = commands.iter().map(|command| command.id).collect();
        ids.sort();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate command ids");
        for command in &commands {
            for keys in command.mac.iter().chain(command.other) {
                // Sequences are space-separated keystrokes; each must parse.
                for stroke in keys.split(' ') {
                    let stroke: &'static str = Box::leak(stroke.to_string().into_boxed_str());
                    assert!(gpui_kit::Keystroke::parse(stroke).is_ok(), "{}: {keys}", command.id);
                }
            }
        }
    }

    #[test]
    fn overrides_replace_or_unbind_defaults() {
        let new_tab = commands().into_iter().find(|command| command.id == "explorer.new_tab").unwrap();
        let none = BTreeMap::new();
        assert_eq!(effective(&new_tab, &none, true), ["cmd-t"]);
        let rebound = BTreeMap::from([("explorer.new_tab".to_string(), "cmd-y".to_string())]);
        assert_eq!(effective(&new_tab, &rebound, true), ["cmd-y"]);
        let unbound = BTreeMap::from([("explorer.new_tab".to_string(), String::new())]);
        assert!(effective(&new_tab, &unbound, true).is_empty());
    }

    #[test]
    fn conflicts_are_per_context() {
        assert!(conflicts(&BTreeMap::new(), true).is_empty(), "defaults must not clash");
        let clash = BTreeMap::from([("explorer.new_tab".to_string(), "cmd-w".to_string())]);
        let found = conflicts(&clash, true);
        assert_eq!(found.get("FileExplorer|cmd-w").map(Vec::len), Some(2), "{found:?}");
    }

    #[test]
    fn extension_commands_are_tagged_and_clashes_are_found() {
        let archives = for_extension("Archives");
        assert_eq!(archives.len(), 2);
        assert!(archives.iter().all(|command| command.group == "Extension · Archives"));
        let none = BTreeMap::new();
        assert_eq!(clashes("explorer.new_tab", "cmd-w", &none, true), ["Close tab"]);
        assert!(clashes("explorer.new_tab", "cmd-y", &none, true).is_empty());
        // Same keys in another context are fine (Quick Look's Space vs the list's).
        assert!(clashes("quick_look.close", "cmd-w", &none, true).is_empty());
    }

    #[test]
    fn displays_mac_symbols() {
        assert_eq!(display("cmd-shift-k", true), "⌘⇧K");
        assert_eq!(display("cmd-n f f", true), "⌘N F F", "sequences show every step");
        assert_eq!(display("ctrl-alt-p", false), "Ctrl+Alt+P");
        assert_eq!(display("space", true), "Space");
        assert_eq!(display("", true), "—");
    }
}
