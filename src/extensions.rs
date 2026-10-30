//! The extension catalog. Today every extension is built in; the shape (id,
//! version, source, permissions) is what future WASM modules installed from
//! GitHub, GitLab or a marketplace will describe in their manifest.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;

use crate::settings::ExtensionSettings;

use files_sdk::{Icon as SdkIcon, Manifest, Permission};

/// The icon an extension's manifest asks for: one from the app's icon set
/// or its own SVG. Used in Settings and for every menu item it adds.
pub fn icon_of(manifest: &Manifest) -> Icon {
    match manifest.icon {
        SdkIcon::Svg(bytes) => Icon::empty().data(bytes),
        SdkIcon::Builtin(name) => Icon::new(builtin_icon(name).unwrap_or(IconName::Puzzle)),
    }
}

/// App icons a manifest may name.
fn builtin_icon(name: &str) -> Option<IconName> {
    Some(match name {
        "square-arrow-out-up-right" => IconName::SquareArrowOutUpRight,
        "file-archive" => IconName::FileArchive,
        "git-branch" => IconName::GitBranch,
        "copy" => IconName::Copy,
        "cloud" => IconName::Cloud,
        "square-terminal" => IconName::SquareTerminal,
        _ => return None,
    })
}

/// The icon of the extension called `name`, for its menu items.
pub fn icon(name: &str) -> Icon {
    CATALOG
        .iter()
        .find(|extension| extension.name == name)
        .map_or_else(|| Icon::new(IconName::Puzzle), |extension| icon_of(extension))
}

static OPEN_WITH: Manifest = Manifest {
    id: "open-with",
    name: "Open With",
    version: "1.0.0",
    source: "Built-in",
    summary: "Open files in a chosen app from the context menu.",
    icon: SdkIcon::Builtin("square-arrow-out-up-right"),
    adds: &[
        "Context menu → Open With → Default Application",
        "Context menu → Open With → Choose Application…",
        "Uses Settings → Default apps overrides",
    ],
    permissions: &[Permission::LaunchApps],
    commands: &[],
    menu: &[],
    sidebar: &[],
    menu_color: None,
    tier: files_sdk::Tier::Free,
};


static COPY: Manifest = Manifest {
    id: "copy",
    name: "Copy",
    version: "1.0.0",
    source: "Built-in",
    summary: "Copy names, paths and file references to the clipboard.",
    icon: SdkIcon::Builtin("copy"),
    adds: &["Context menu → Copy → Name / Path / Reference", "Preview panel → Copy path"],
    permissions: &[Permission::Clipboard],
    commands: &[],
    menu: &[],
    sidebar: &[],
    menu_color: None,
    tier: files_sdk::Tier::Free,
};

static TERMINAL: Manifest = Manifest {
    id: "terminal",
    name: "Terminal",
    version: "1.0.0",
    source: "Built-in",
    summary: "Open the current folder in your terminal app.",
    icon: SdkIcon::Builtin("square-terminal"),
    adds: &["Context menu on a folder → Open Current Folder in Terminal"],
    permissions: &[Permission::LaunchApps],
    commands: &[],
    menu: &[],
    sidebar: &[],
    menu_color: None,
    tier: files_sdk::Tier::Free,
};

/// Every extension, in Settings order. Archives, SSH and Forks are separate
/// crates built on `files-sdk`; the rest are small enough to live in the app.
pub static CATALOG: [&Manifest; 8] = [
    &OPEN_WITH,
    &files_ext_archive::MANIFEST,
    &files_ext_git::MANIFEST,
    &COPY,
    &files_ext_ssh::MANIFEST,
    &TERMINAL,
    &files_ext_forks::MANIFEST,
    &files_ext_agents::MANIFEST,
];

pub fn enabled(settings: &ExtensionSettings, index: usize) -> bool {
    match index {
        0 => settings.open_with,
        1 => settings.archives,
        2 => settings.git,
        3 => settings.copy,
        4 => settings.ssh,
        5 => settings.terminal,
        6 => settings.forks,
        _ => settings.agents,
    }
}

pub fn toggle(settings: &mut ExtensionSettings, index: usize) {
    let flag = match index {
        0 => &mut settings.open_with,
        1 => &mut settings.archives,
        2 => &mut settings.git,
        3 => &mut settings.copy,
        4 => &mut settings.ssh,
        5 => &mut settings.terminal,
        6 => &mut settings.forks,
        _ => &mut settings.agents,
    };
    *flag = !*flag;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_svg_icons_are_valid_svg() {
        let svg = std::str::from_utf8(files_ext_forks::ICON).unwrap();
        assert!(svg.starts_with("<svg") && svg.trim_end().ends_with("</svg>"));
        assert!(CATALOG.iter().any(|extension| matches!(extension.icon, SdkIcon::Svg(_))));
        // Every built-in icon name resolves to a real icon.
        for extension in CATALOG {
            if let SdkIcon::Builtin(name) = extension.icon {
                assert!(builtin_icon(name).is_some(), "unknown icon {name}");
            }
        }
    }

    #[test]
    fn catalog_ids_are_unique_and_toggles_match() {
        let mut ids: Vec<_> = CATALOG.iter().map(|extension| extension.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), CATALOG.len());
        let mut settings = ExtensionSettings::default();
        for index in 0..CATALOG.len() {
            let before = enabled(&settings, index);
            toggle(&mut settings, index);
            assert_ne!(enabled(&settings, index), before, "{}", CATALOG[index].id);
        }
    }
}
