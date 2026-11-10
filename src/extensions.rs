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
        "mail" => IconName::Mail,
        "music" => IconName::Music,
        "eye" => IconName::Eye,
        "zap" => IconName::Zap,
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

static PREVIEW: Manifest = Manifest {
    id: "preview",
    name: "Preview",
    version: "1.0.0",
    source: "Built-in",
    summary: "Look inside files without opening them: Quick Look and the preview panel.",
    icon: SdkIcon::Builtin("eye"),
    adds: &[
        "Space or context menu → Quick Look",
        "Preview panel: text and code, PDF pages, photo, video, archive and program details",
        "Other extensions add formats through files_sdk::preview",
    ],
    permissions: &[Permission::ReadFiles],
    commands: &[],
    menu: &[],
    sidebar: &[],
    menu_color: None,
    tier: files_sdk::Tier::Free,
};

static QUICK_ACTIONS: Manifest = Manifest {
    id: "quick-actions",
    name: "Quick Actions",
    version: "1.0.0",
    source: "Built-in",
    summary: "Actions for the selected files, after Copy in the context menu: Convert Image, Rotate and your own scripts.",
    icon: SdkIcon::Builtin("zap"),
    adds: &[
        "Context menu → Quick Actions → Convert Image… (PNG, JPEG, WebP, HEIC), Rotate Image…",
        "Scripts in Python, Lua, JavaScript, Rust, Go or shell, for the files they fit",
        "Quick Actions → Customize…: write them, or let an agent write them",
        "Other extensions add actions through files_sdk::quick_actions",
    ],
    permissions: &[Permission::ReadFiles, Permission::WriteFiles, Permission::LaunchApps],
    commands: &[],
    menu: &[],
    sidebar: &[],
    menu_color: None,
    tier: files_sdk::Tier::Free,
};

/// Every extension, in Settings order. Archives, SSH and Forks are separate
/// crates built on `files-sdk`; the rest are small enough to live in the app.
pub static CATALOG: [&Manifest; 12] = [
    &OPEN_WITH,
    &files_ext_archive::MANIFEST,
    &files_ext_git::MANIFEST,
    &COPY,
    &files_ext_ssh::MANIFEST,
    &TERMINAL,
    &files_ext_forks::MANIFEST,
    &files_ext_agents::MANIFEST,
    &files_ext_mail::MANIFEST,
    &files_ext_music::MANIFEST,
    &PREVIEW,
    &QUICK_ACTIONS,
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
        7 => settings.agents,
        8 => settings.mail,
        9 => settings.music,
        10 => settings.preview,
        _ => settings.quick_actions,
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
        7 => &mut settings.agents,
        8 => &mut settings.mail,
        9 => &mut settings.music,
        10 => &mut settings.preview,
        _ => &mut settings.quick_actions,
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
