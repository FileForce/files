//! The Files extension SDK.
//!
//! An extension is a [`Manifest`] (what it is, what it adds, which
//! permissions it needs) plus one or more capabilities, each a plain trait:
//!
//! - [`ArchiveProvider`]: browse and extract an archive format;
//! - [`RemoteProvider`]: browse a remote server like a folder;
//! - [`SnapshotProvider`]: throwaway copies of a folder (forks).
//!
//! Besides capabilities, a manifest declares *contributions*: sidebar
//! sections ([`SidebarSection`]) and context-menu items ([`MenuItem`], each
//! for files, folders and/or the empty area), plus the [`Permission`]s it
//! needs. The app grants or revokes each permission in Settings; every
//! guarded host API checks it ([`permissions::check`]).
//!
//! Extensions never talk to the UI. What they need from the app (where to
//! keep their files, which paths are protected, how to delete, when to
//! refresh) goes through the [`Host`] the app installs with [`set_host`].
//!
//! The SDK has no UI or async dependencies on purpose: the same traits are
//! the boundary future WASM extensions will be compiled against.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub mod fs;
pub mod license;
pub mod audio;
pub mod log;
pub mod package;
pub mod plugin;
pub mod permissions;
pub mod power;
pub mod secrets;
pub mod storage;

pub use permissions::Permission;

/// An extension's icon: a name from the app's icon set, or its own SVG.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Icon {
    /// A Lucide icon name, e.g. `"file-archive"`.
    Builtin(&'static str),
    Svg(&'static [u8]),
}

/// A command an extension adds, bindable in Settings → Hotkeys.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Command {
    /// Stable id, e.g. `"forks.fork-here"`.
    pub id: &'static str,
    pub title: &'static str,
    /// Default keys on macOS and elsewhere, e.g. `&["cmd-n f f"]`.
    pub mac_keys: &'static [&'static str],
    pub keys: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Manifest {
    /// Stable id, also the key in `settings.toml`.
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    /// Where it comes from: "Built-in", a GitHub repo, a marketplace…
    pub source: &'static str,
    pub summary: &'static str,
    pub icon: Icon,
    /// What it adds to the app, one line per place.
    pub adds: &'static [&'static str],
    /// What it may do; each one can be revoked in Settings.
    pub permissions: &'static [Permission],
    pub commands: &'static [Command],
    /// Items it adds to context menus.
    pub menu: &'static [MenuItem],
    /// Sections it adds to the sidebar.
    pub sidebar: &'static [SidebarSection],
    /// Tint of its context-menu items: a color name (`purple`, `amber`…) or
    /// `#rrggbb`. The user can change it in Settings → Appearance.
    pub menu_color: Option<&'static str>,
    /// Free, or unlocked by a license for `product`.
    pub tier: Tier,
}

/// Whether an extension is free or needs a license.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tier {
    Free,
    /// Needs a license covering `product` (see [`license`]).
    Pro { product: &'static str },
}

/// What a context-menu item applies to. Combine with `|`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Targets(u8);

impl Targets {
    /// Right-click on a file.
    pub const FILE: Self = Self(1);
    /// Right-click on a folder.
    pub const FOLDER: Self = Self(2);
    /// Right-click on the empty area of a folder.
    pub const BACKGROUND: Self = Self(4);
    pub const ITEMS: Self = Self(1 | 2);
    pub const ALL: Self = Self(1 | 2 | 4);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Targets {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Where in the file system a menu item may appear.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    Local,
    Remote,
    Anywhere,
}

/// Text for screen readers (VoiceOver): what an item is and what it does.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Accessibility {
    /// Spoken instead of the title when set, e.g. "Copy SSH address".
    pub label: Option<&'static str>,
    /// Spoken after the label, e.g. "Copies ssh://host/path to the clipboard".
    pub hint: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuItem {
    /// The command it runs: [`Extension::run`] receives this id.
    pub command: &'static str,
    pub title: &'static str,
    pub icon: Option<Icon>,
    pub targets: Targets,
    pub scope: Scope,
    pub accessibility: Accessibility,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SidebarSection {
    /// Stable id; also the key that remembers whether it is folded.
    pub id: &'static str,
    pub title: &'static str,
    pub accessibility: Accessibility,
}

/// One row of an extension's sidebar section.
#[derive(Clone, Debug, PartialEq)]
pub struct SidebarItem {
    pub title: String,
    pub icon: Option<Icon>,
    /// A small text on the right, e.g. "LAN".
    pub badge: Option<String>,
    /// Run on click with [`Context::sidebar_item`] set to `id`.
    pub command: &'static str,
    pub id: String,
    pub accessibility_label: Option<String>,
    /// Right-click menu of the row; each item runs its command with
    /// [`Context::sidebar_item`] set to `id` (Eject, Disconnect, Forget…).
    pub menu: Vec<SidebarMenuItem>,
}

/// One item of a sidebar row's right-click menu.
#[derive(Clone, Debug, PartialEq)]
pub struct SidebarMenuItem {
    pub title: String,
    pub icon: Option<Icon>,
    pub command: &'static str,
    /// Shown dimmed and not clickable (e.g. "Disconnect" while offline).
    pub disabled: bool,
}

impl SidebarMenuItem {
    pub fn new(title: impl Into<String>, command: &'static str) -> Self {
        Self { title: title.into(), icon: None, command, disabled: false }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// What a command runs on.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Context {
    /// The right-clicked or selected items (local paths, or remote paths).
    pub selection: Vec<String>,
    /// The folder being shown.
    pub folder: Option<String>,
    /// Whether `selection` and `folder` are remote paths.
    pub remote: bool,
    /// The clicked sidebar row.
    pub sidebar_item: Option<String>,
    /// The connected server (`user@host:port`) when `remote`.
    pub server: Option<String>,
}

/// What the app should do after a command.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Outcome {
    /// Shown in the status bar.
    pub notice: Option<String>,
    /// Put on the clipboard (needs [`Permission::Clipboard`]).
    pub clipboard: Option<String>,
    /// Open this local folder.
    pub navigate: Option<PathBuf>,
}

/// Something that happened to the app, for [`Extension::on_event`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    /// The window became active / inactive.
    WindowActive(bool),
    /// The computer slept and woke up again.
    Woke,
}

/// What every extension implements. Capabilities default to "not offered".
pub trait Extension: Send + Sync {
    fn manifest(&self) -> &'static Manifest;

    fn archive(&self) -> Option<&dyn ArchiveProvider> {
        None
    }

    fn remote(&self) -> Option<&dyn RemoteProvider> {
        None
    }

    fn snapshots(&self) -> Option<&dyn SnapshotProvider> {
        None
    }

    /// Runs one of the manifest's menu or sidebar commands.
    fn run(&self, command: &str, context: &Context) -> io::Result<Outcome> {
        let _ = context;
        Err(io::Error::new(io::ErrorKind::Unsupported, format!("unknown command {command}")))
    }

    /// Rows of the sidebar section `section`.
    fn sidebar_items(&self, section: &str) -> Vec<SidebarItem> {
        let _ = section;
        Vec::new()
    }

    /// Only delivered for [`Event::WindowActive`]`(false)` when the
    /// extension holds [`Permission::Background`].
    fn on_event(&self, event: Event) {
        let _ = event;
    }

    /// `key:value` filters this extension adds to search, e.g. `git:main`.
    fn search_keys(&self) -> &'static [SearchKey] {
        &[]
    }

    /// Whether `path` passes the filter `key:value` (`key` is one of
    /// [`search_keys`](Self::search_keys), `value` is lowercase). Called for
    /// every candidate, so it must be fast: cache anything slow.
    fn search_matches(&self, key: &str, value: &str, path: &Path) -> bool {
        let _ = (key, value, path);
        false
    }
}

/// A search filter an extension contributes, listed in the search hints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SearchKey {
    pub key: &'static str,
    /// What it matches, e.g. "branch or status (modified, untracked…)".
    pub summary: &'static str,
    pub example: &'static str,
}

// ---------------------------------------------------------------- archives

/// One archive member: relative, `/`-separated, never escaping the target.
#[derive(Clone, Debug, PartialEq)]
pub struct ArchiveItem {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    /// Seconds since the Unix epoch, when the format stores it.
    pub modified: Option<i64>,
}

/// What to do with a file that already exists at its extract destination.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConflictPolicy {
    /// Write next to it as `name (1).ext`.
    Rename,
    Overwrite,
    Skip,
}

#[derive(Debug, Default, PartialEq)]
pub struct ExtractReport {
    /// Every file written, including renamed and overwritten ones.
    pub files: usize,
    pub renamed: usize,
    pub overwritten: usize,
    pub skipped: usize,
}

pub trait ArchiveProvider: Send + Sync {
    /// Whether this provider reads the file at `path`.
    fn handles(&self, path: &Path) -> bool;
    fn list(&self, archive: &Path) -> io::Result<Vec<ArchiveItem>>;
    /// Members whose destination under `target` already exists.
    fn conflicts(&self, archive: &Path, target: &Path) -> io::Result<Vec<String>>;
    fn extract(&self, archive: &Path, target: &Path, policy: ConflictPolicy) -> io::Result<ExtractReport>;
}

// ------------------------------------------------------------------ remote

/// A server to connect to: `[user@]host[:port]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    pub host: String,
    /// Empty: whatever the provider's own configuration says.
    pub user: String,
    pub port: u16,
}

impl Endpoint {
    /// `user@host`, or just `host` when the user comes from the provider's config.
    pub fn destination(&self) -> String {
        if self.user.is_empty() {
            self.host.clone()
        } else {
            format!("{}@{}", self.user, self.host)
        }
    }

    /// `user@host:port`, the port left out when it is 22.
    pub fn address(&self) -> String {
        if self.port == 22 {
            self.destination()
        } else {
            format!("{}:{}", self.destination(), self.port)
        }
    }

    /// Reads back what [`Self::address`] wrote: `[ssh://][user@]host[:port]`.
    pub fn parse(address: &str) -> Option<Self> {
        let address = address.trim().trim_start_matches("ssh://");
        let (user, rest) = match address.rsplit_once('@') {
            Some((user, rest)) => (user.to_string(), rest),
            None => (String::new(), address),
        };
        let (host, port) = match rest.rsplit_once(':') {
            Some((host, port)) => (host, port.parse().ok()?),
            None => (rest, 22),
        };
        (!host.is_empty() && port > 0).then(|| Self { host: host.to_string(), user, port })
    }
}

/// One row of a remote folder listing.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteItem {
    pub name: String,
    pub is_dir: bool,
    pub bytes: u64,
    /// `rwxrwxrwx` bits.
    pub mode: u32,
    /// As the server reports it, e.g. `Oct 24 16:08`.
    pub modified: String,
}

pub trait RemoteProvider: Send + Sync {
    /// Checks the server is reachable; returns the folder to start in.
    fn connect(&self, endpoint: &Endpoint) -> io::Result<String>;
    fn list(&self, endpoint: &Endpoint, folder: &str) -> io::Result<Vec<RemoteItem>>;
    fn download(&self, endpoint: &Endpoint, remote: &str, local: &Path) -> io::Result<()>;
    /// Copies `local` into `folder`, never replacing a file; returns the new path.
    fn upload(&self, endpoint: &Endpoint, local: &Path, folder: &str) -> io::Result<String>;
}

// --------------------------------------------------------------- snapshots

pub trait SnapshotProvider: Send + Sync {
    /// Makes a throwaway copy of `source`; returns its folder.
    fn create(&self, source: &Path) -> io::Result<PathBuf>;
    /// The copy `path` lies in, if any.
    fn snapshot_of(&self, path: &Path) -> Option<u32>;
    fn discard(&self, id: u32) -> Option<PathBuf>;
}

// -------------------------------------------------------------------- host

/// What the app provides to extensions.
pub trait Host: Send + Sync {
    /// A folder the extension may keep its own files in.
    fn config_dir(&self) -> Option<PathBuf>;
    /// Where extensions' caches live, one folder each ([`storage`]).
    fn cache_dir(&self) -> Option<PathBuf> {
        Some(std::env::temp_dir().join("files-sdk-cache"))
    }
    /// The user's rule that protects `path` from changes, if any.
    fn protection(&self, path: &Path, patterns: &[String], home: &Path) -> Option<String>;
    /// Deletes `paths` (to the Trash when `use_trash`); on failure reports
    /// how many were removed before the error.
    fn delete(&self, paths: &[&Path], use_trash: bool) -> Result<(), (usize, io::Error)>;
    /// A short, human explanation of an I/O error.
    fn describe_error(&self, error: &io::Error) -> String {
        error.to_string()
    }
    /// Files changed on disk behind the app's back: refresh listings.
    fn files_changed(&self) {}
    /// Plays sound for an extension (see [`audio`]); hosts without a
    /// player say so.
    fn audio(&self, extension: &str, command: audio::Command) -> io::Result<()> {
        let _ = (extension, command);
        Err(io::Error::new(io::ErrorKind::Unsupported, "This host plays no sound"))
    }
}

static HOST: OnceLock<Box<dyn Host>> = OnceLock::new();

/// Installs the app's host. Later calls are ignored.
pub fn set_host(host: Box<dyn Host>) {
    let _ = HOST.set(host);
}

/// The installed host, or a minimal stand-in (tests, tools).
pub fn host() -> &'static dyn Host {
    HOST.get_or_init(|| Box::new(DefaultHost)).as_ref()
}

struct DefaultHost;

impl Host for DefaultHost {
    fn config_dir(&self) -> Option<PathBuf> {
        Some(std::env::temp_dir().join("files-sdk"))
    }

    fn protection(&self, _: &Path, _: &[String], _: &Path) -> Option<String> {
        None
    }

    fn delete(&self, paths: &[&Path], _: bool) -> Result<(), (usize, io::Error)> {
        for (done, path) in paths.iter().enumerate() {
            let result = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
            result.map_err(|error| (done, error))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod sidebar_menu_tests {
    use super::*;

    #[test]
    fn sidebar_rows_carry_their_own_menu() {
        let item = SidebarMenuItem::new("Eject", "disks.eject").icon(Icon::Builtin("eject")).disabled(true);
        assert_eq!((item.title.as_str(), item.command, item.disabled), ("Eject", "disks.eject", true));
        assert_eq!(item.icon, Some(Icon::Builtin("eject")));
    }
}
