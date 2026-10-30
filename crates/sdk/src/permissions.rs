//! What an extension may do. Built-in extensions get what their manifest
//! asks for; the user can revoke any permission in Settings → Extensions.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Permission {
    ReadFiles,
    WriteFiles,
    /// Connections to other computers (SSH, FTP, HTTP…).
    Network,
    LaunchApps,
    Clipboard,
    /// Passwords and keys in the system keychain (macOS Passwords).
    Secrets,
    /// Keep working while the window is in the background.
    Background,
    /// Keep the computer awake while a task runs.
    PreventSleep,
    /// Read what is on screen through the accessibility API.
    Accessibility,
}

impl Permission {
    pub const ALL: [Self; 9] = [
        Self::ReadFiles,
        Self::WriteFiles,
        Self::Network,
        Self::LaunchApps,
        Self::Clipboard,
        Self::Secrets,
        Self::Background,
        Self::PreventSleep,
        Self::Accessibility,
    ];

    /// The key in `settings.toml`.
    pub fn key(self) -> &'static str {
        match self {
            Self::ReadFiles => "read_files",
            Self::WriteFiles => "write_files",
            Self::Network => "network",
            Self::LaunchApps => "launch_apps",
            Self::Clipboard => "clipboard",
            Self::Secrets => "secrets",
            Self::Background => "background",
            Self::PreventSleep => "prevent_sleep",
            Self::Accessibility => "accessibility",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|permission| permission.key() == key)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::ReadFiles => "Read files",
            Self::WriteFiles => "Write files",
            Self::Network => "Network",
            Self::LaunchApps => "Launch applications",
            Self::Clipboard => "Clipboard",
            Self::Secrets => "Passwords & keys",
            Self::Background => "Run in the background",
            Self::PreventSleep => "Keep the computer awake",
            Self::Accessibility => "Accessibility",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::ReadFiles => "Read files and folders you browse.",
            Self::WriteFiles => "Create, change and delete files.",
            Self::Network => "Connect to servers on your network or the internet.",
            Self::LaunchApps => "Open other applications.",
            Self::Clipboard => "Put text and files on the clipboard.",
            Self::Secrets => "Save and read its own passwords in the system keychain (Passwords on macOS).",
            Self::Background => "Keep running tasks and receive events while the window is inactive.",
            Self::PreventSleep => "Stop the computer from sleeping while a task runs.",
            Self::Accessibility => "Read what is on screen through the accessibility API.",
        }
    }
}

fn revoked() -> &'static Mutex<BTreeMap<String, BTreeSet<Permission>>> {
    static REVOKED: OnceLock<Mutex<BTreeMap<String, BTreeSet<Permission>>>> = OnceLock::new();
    REVOKED.get_or_init(Default::default)
}

fn declared() -> &'static Mutex<BTreeMap<String, Vec<Permission>>> {
    static DECLARED: OnceLock<Mutex<BTreeMap<String, Vec<Permission>>>> = OnceLock::new();
    DECLARED.get_or_init(Default::default)
}

/// Registers what an extension's manifest asks for.
pub fn declare(manifest: &crate::Manifest) {
    declared()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(manifest.id.to_string(), manifest.permissions.to_vec());
}

/// Replaces the user's revocations (from settings).
pub fn set_revoked(map: BTreeMap<String, BTreeSet<Permission>>) {
    *revoked().lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = map;
}

/// Declared by the manifest and not revoked by the user.
pub fn granted(extension: &str, permission: Permission) -> bool {
    let declared = declared()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(extension)
        .is_some_and(|permissions| permissions.contains(&permission));
    declared
        && !revoked()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(extension)
            .is_some_and(|denied| denied.contains(&permission))
}

/// `Ok` when granted; a readable `PermissionDenied` error otherwise.
pub fn check(extension: &str, permission: Permission) -> io::Result<()> {
    if granted(extension, permission) {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{extension} needs “{}” · allow it in Settings → Extensions", permission.title()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST: crate::Manifest = crate::Manifest {
        id: "perm-test",
        name: "Test",
        version: "0",
        source: "",
        summary: "",
        icon: crate::Icon::Builtin("cloud"),
        adds: &[],
        permissions: &[Permission::Network],
        commands: &[],
        menu: &[],
        sidebar: &[],
        menu_color: None,
        tier: crate::Tier::Free,
    };

    #[test]
    fn declared_permissions_are_granted_until_revoked() {
        declare(&TEST);
        assert!(check("perm-test", Permission::Network).is_ok());
        assert!(check("perm-test", Permission::Secrets).is_err(), "never declared");
        assert!(check("unknown", Permission::Network).is_err());
        set_revoked(BTreeMap::from([("perm-test".to_string(), BTreeSet::from([Permission::Network]))]));
        let error = check("perm-test", Permission::Network).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("Settings → Extensions"));
        set_revoked(BTreeMap::new());
        for permission in Permission::ALL {
            assert_eq!(Permission::from_key(permission.key()), Some(permission));
        }
    }
}
