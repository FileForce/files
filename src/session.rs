//! Tabs come back after Files is quit and started again: the main window's
//! tabs — folders, Recents, Trash, tags, and Music, Mail and Settings tabs —
//! with which is pinned and active, kept in session.toml beside the
//! settings (Settings → General → Windows → Restore tabs).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::app::FileExplorer;
use crate::model::{Location, ViewMode};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub tabs: Vec<SavedTab>,
    pub active: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    /// `path:/Users/me/Music`, `recents`, `trash`, `tag:Red`, `panel:Music`.
    pub place: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default = "default_mode")]
    pub mode: ViewMode,
}

fn default_mode() -> ViewMode {
    ViewMode::Expressive
}

fn file() -> Option<PathBuf> {
    Some(crate::settings::settings_path()?.parent()?.join("session.toml"))
}

/// How a place is written down; `None` for what cannot come back (a
/// server's folder).
pub fn place_of(location: Location) -> Option<String> {
    match location {
        Location::Recents => Some("recents".into()),
        Location::Trash => Some("trash".into()),
        Location::Tag(_) => Some(format!("tag:{}", location.tag_name()?)),
        Location::Dir(_) => Some(format!("path:{}", location.path()?.display())),
        Location::Remote(_) => None,
    }
}

/// The place again, if it still exists.
pub fn location_of(place: &str) -> Option<Location> {
    match place {
        "recents" => Some(Location::Recents),
        "trash" => Some(Location::Trash),
        _ => {
            if let Some(name) = place.strip_prefix("tag:") {
                return Some(Location::tag(name));
            }
            let path = PathBuf::from(place.strip_prefix("path:")?);
            path.is_dir().then(|| Location::from_path(&path))
        }
    }
}

/// The window's tabs, written down.
pub fn capture(explorer: &FileExplorer) -> Session {
    let tabs = explorer
        .tabs
        .iter()
        .enumerate()
        .filter_map(|(index, tab)| {
            // The active tab's place lives in the window until it is left.
            let current = if index == explorer.active_tab { explorer.current } else { tab.current };
            let place = match tab.panel {
                Some(_) => format!("panel:{}", tab.panel_title),
                None => place_of(current)?,
            };
            let mode = if index == explorer.active_tab { explorer.mode } else { tab.mode };
            Some(SavedTab { place, pinned: tab.pinned, mode })
        })
        .collect();
    Session { tabs, active: explorer.active_tab }
}

/// Saves when it changed since the last time (called every frame).
pub fn save_if_changed(session: Session) {
    static LAST: std::sync::Mutex<Option<Session>> = std::sync::Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if last.as_ref() == Some(&session) {
        return;
    }
    *last = Some(session.clone());
    drop(last);
    let Some(file) = file() else { return };
    std::thread::spawn(move || {
        if let Ok(text) = toml::to_string(&session) {
            let partial = file.with_extension("partial");
            if std::fs::write(&partial, text).is_ok() {
                let _ = std::fs::rename(&partial, &file);
            }
        }
    });
}

pub fn load() -> Option<Session> {
    toml::from_str(&std::fs::read_to_string(file()?).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_are_written_and_read_back() {
        let dir = std::env::temp_dir();
        let place = place_of(Location::from_path(&dir)).unwrap();
        assert!(place.starts_with("path:"));
        assert_eq!(location_of(&place), Some(Location::from_path(&dir)));
        assert_eq!(location_of("recents"), Some(Location::Recents));
        assert_eq!(location_of("path:/no/such/folder/anywhere"), None, "a folder that is gone does not come back");
        let session = Session { tabs: vec![SavedTab { place, pinned: true, mode: ViewMode::Pro }], active: 0 };
        assert_eq!(toml::from_str::<Session>(&toml::to_string(&session).unwrap()).unwrap(), session);
    }
}
