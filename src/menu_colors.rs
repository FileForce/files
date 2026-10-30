//! Context-menu tints: each extension may color its items (Forks purple,
//! Agents amber), Delete is red; Settings → Appearance changes any of them
//! (`[menu_colors]` in settings.toml).

use gpui_kit::{Hsla, rgb};

use crate::settings::AppSettings;

/// The swatches offered in Settings, in order.
pub const PRESETS: [&str; 10] = ["red", "orange", "amber", "green", "teal", "blue", "indigo", "purple", "pink", "gray"];

/// `delete` is not an extension but is tinted the same way.
pub const DELETE: &str = "delete";

/// The app's own menu items, set in Settings → Appearance (extensions set
/// theirs on their own page): key, label, icon name.
pub const BASE: [(&str, &str, &str); 8] = [
    ("open", "Open", "folder-open"),
    ("quick-look", "Quick Look", "eye"),
    ("favorites", "Add to Favorites", "star"),
    ("new-folder", "New Folder…", "folder-plus"),
    ("new-file", "New File…", "file-plus"),
    ("terminal", "Open in Terminal", "square-terminal"),
    (DELETE, "Delete…", "trash"),
    ("properties", "Properties", "info"),
];

/// A color name or `#rrggbb`; `None` for "none" or anything unknown.
pub fn parse(value: &str) -> Option<Hsla> {
    let value = value.trim().to_lowercase();
    let hex = match value.as_str() {
        "red" => 0xe5484d,
        "orange" => 0xf76b15,
        "amber" => 0xd9940f,
        "green" => 0x30a46c,
        "teal" => 0x12a594,
        "blue" => 0x3e63dd,
        "indigo" => 0x5b5bd6,
        "purple" => 0x8e4ec6,
        "pink" => 0xd6409f,
        "gray" => 0x8b8d98,
        other => {
            let digits = other.strip_prefix('#')?;
            if digits.len() != 6 {
                return None;
            }
            u32::from_str_radix(digits, 16).ok()?
        }
    };
    Some(rgb(hex).into())
}

/// What `key` (an extension id, or [`DELETE`]) is tinted by default.
pub fn default_for(key: &str) -> Option<&'static str> {
    if key == DELETE {
        return Some("red");
    }
    crate::extensions::CATALOG.iter().find(|manifest| manifest.id == key).and_then(|manifest| manifest.menu_color)
}

/// The setting as stored, or the default (`"none"` when there is none).
pub fn value_for(settings: &AppSettings, key: &str) -> String {
    settings
        .menu_colors
        .get(key)
        .cloned()
        .unwrap_or_else(|| default_for(key).unwrap_or("none").to_string())
}

pub fn color_for(settings: &AppSettings, key: &str) -> Option<Hsla> {
    parse(&value_for(settings, key))
}

/// Stores a choice; choosing the default removes the override.
pub fn set(settings: &mut AppSettings, key: &str, value: &str) {
    if default_for(key).unwrap_or("none") == value {
        settings.menu_colors.remove(key);
    } else {
        settings.menu_colors.insert(key.to_string(), value.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_overrides_and_parsing() {
        let mut settings = AppSettings::default();
        assert_eq!(value_for(&settings, "forks"), "purple");
        assert_eq!(value_for(&settings, "agents"), "amber");
        assert_eq!(value_for(&settings, DELETE), "red");
        assert_eq!(value_for(&settings, "ssh"), "none");
        assert!(color_for(&settings, "ssh").is_none());

        set(&mut settings, "forks", "#112233");
        set(&mut settings, DELETE, "none");
        assert_eq!(color_for(&settings, "forks"), Some(rgb(0x112233).into()));
        assert!(color_for(&settings, DELETE).is_none(), "red can be turned off");
        set(&mut settings, "forks", "purple");
        assert!(!settings.menu_colors.contains_key("forks"), "back to default: no override kept");

        let source = toml::to_string(&settings).unwrap();
        assert!(source.contains("[menu_colors]") && source.contains("delete = \"none\""), "{source}");
        for name in PRESETS {
            assert!(parse(name).is_some(), "{name}");
        }
        assert!(parse("#12").is_none() && parse("chartreuse").is_none());
    }
}
