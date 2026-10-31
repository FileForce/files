use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    #[default]
    Expressive,
    Pro,
    Dual,
}

impl ViewMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Expressive => "Expressive",
            Self::Pro => "Pro",
            Self::Dual => "Dual",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorScheme {
    Dark,
    Light,
    /// Follows the OS light/dark appearance.
    System,
}

/// The OS appearance, kept current by `set_system_dark` (see `app.rs`).
static SYSTEM_DARK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_system_dark(dark: bool) {
    SYSTEM_DARK.store(dark, std::sync::atomic::Ordering::Relaxed);
}

impl ColorScheme {
    /// ⌘⇧T: flips the theme that is showing (System becomes the opposite).
    pub fn toggled(self) -> Self {
        if self.is_dark() { Self::Light } else { Self::Dark }
    }

    pub fn is_dark(self) -> bool {
        match self {
            Self::Dark => true,
            Self::Light => false,
            Self::System => SYSTEM_DARK.load(std::sync::atomic::Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    Iris,
    Azure,
    Amber,
}

impl Accent {
    pub fn label(self) -> &'static str {
        match self {
            Self::Iris => "Iris",
            Self::Azure => "Azure",
            Self::Amber => "Amber",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pane {
    Left,
    Right,
}

/// A browsable place. Real folders are interned into `Dir` ids so `Location`
/// stays `Copy` and cheap to keep in history, tabs and panes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Location {
    Recents,
    /// A folder on the connected SSH server (interned remote path).
    Remote(u32),
    /// The OS trash: `~/.Trash`, the freedesktop trash, or the Recycle Bin.
    Trash,
    /// Every file carrying one macOS Finder tag (interned name).
    Tag(u32),
    Dir(u32),
}

#[allow(non_upper_case_globals)]
impl Location {
    pub const Home: Self = Self::Dir(0);
    pub const Documents: Self = Self::Dir(1);
    pub const Pictures: Self = Self::Dir(2);
    pub const Downloads: Self = Self::Dir(3);
}

impl Location {
    pub fn from_path(path: &Path) -> Self {
        Self::Dir(fs_index::intern_dir(path))
    }

    pub fn path(self) -> Option<PathBuf> {
        match self {
            Self::Dir(id) => Some(fs_index::dir_path(id)),
            Self::Recents | Self::Remote(_) | Self::Trash | Self::Tag(_) => None,
        }
    }

    /// A folder on the connected SSH server.
    pub fn remote(path: &str) -> Self {
        Self::Remote(remote::intern(path))
    }

    pub fn remote_path(self) -> Option<&'static str> {
        match self {
            Self::Remote(id) => Some(remote::path(id)),
            _ => None,
        }
    }

    pub fn is_remote(self) -> bool {
        matches!(self, Self::Remote(_))
    }

    /// The location listing every file with this Finder tag.
    pub fn tag(name: &str) -> Self {
        Self::Tag(tag_names::intern(name))
    }

    pub fn tag_name(self) -> Option<&'static str> {
        match self {
            Self::Tag(id) => Some(tag_names::name(id)),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Recents => "Recents",
            Self::Remote(id) => remote::label(id),
            Self::Trash => {
                if cfg!(windows) {
                    "Recycle Bin"
                } else {
                    "Trash"
                }
            }
            Self::Tag(id) => tag_names::name(id),
            // A fork's top folder is labelled with the fork's name.
            Self::Dir(id) => match crate::forks::root_of(&fs_index::dir_path(id)).and_then(crate::forks::name) {
                Some(name) => Box::leak(name.into_boxed_str()),
                None => fs_index::dir_label(id),
            },
        }
    }

    /// Every folder from the top down to this one (for clickable crumbs).
    pub fn ancestry(self) -> Vec<Self> {
        let mut chain = Vec::new();
        let mut current = Some(self);
        while let Some(location) = current {
            chain.push(location);
            current = location.parent();
        }
        chain.reverse();
        chain
    }

    pub fn breadcrumbs(self) -> Vec<&'static str> {
        let mut crumbs = Vec::new();
        let mut current = Some(self);
        while let Some(location) = current {
            crumbs.push(location.label());
            current = location.parent();
        }
        crumbs.reverse();
        crumbs
    }

    pub fn parent(self) -> Option<Self> {
        if let Some(path) = self.remote_path() {
            return crate::ssh::parent(path).map(|parent| Self::remote(&parent));
        }
        let path = self.path()?;
        // Going up stops at a fork's top folder: its parent is app storage.
        if crate::forks::root_of(&path).is_some() {
            return None;
        }
        path.parent().map(Self::from_path)
    }

}

pub use files_sdk::Endpoint as SshConnection;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileKind {
    Folder,
    Image,
    Video,
    Document,
    Design,
    Code,
    Archive,
    Audio,
    Text,
}

impl FileKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Folder => "Folder",
            Self::Image => "Image",
            Self::Video => "Video",
            Self::Document => "Document",
            Self::Design => "Design",
            Self::Code => "Code",
            Self::Archive => "Archive",
            Self::Audio => "Audio",
            Self::Text => "Text",
        }
    }

    pub(crate) fn from_extension(extension: &str) -> Self {
        match extension.to_ascii_lowercase().as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "heic" | "heif" | "hif" | "avif"
            | "svg" | "ico" => {
                Self::Image
            }
            "mov" | "mp4" | "mkv" | "avi" | "webm" | "m4v" => Self::Video,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "numbers" | "pages"
            | "odt" | "ods" | "rtf" | "epub" => Self::Document,
            "key" | "fig" | "sketch" | "psd" | "ai" | "xd" => Self::Design,
            "rs" | "js" | "ts" | "tsx" | "jsx" | "py" | "go" | "c" | "h" | "cpp" | "hpp"
            | "java" | "kt" | "swift" | "rb" | "php" | "sh" | "zsh" | "json" | "toml" | "yaml"
            | "yml" | "xml" | "html" | "css" | "scss" | "sql" | "lua" | "cs" => Self::Code,
            "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "crate" | "dmg"
            | "iso" | "jar" => Self::Archive,
            "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" | "aiff" => Self::Audio,
            _ => Self::Text,
        }
    }
}

/// Where a file's data lives relative to a cloud drive.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CloudState {
    /// Not in a synced cloud folder.
    #[default]
    Local,
    /// In iCloud Drive / OneDrive / Dropbox… and present on this disk.
    Downloaded,
    /// A placeholder: the data is only in the cloud.
    OnlineOnly,
}

#[derive(Debug)]
pub struct FileEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: FileKind,
    pub parent: Location,
    pub target: Option<Location>,
    pub modified: &'static str,
    pub modified_at: Option<std::time::SystemTime>,
    pub bytes: u64,
    pub metadata: &'static str,
    pub tags: &'static [&'static str],
    pub preview: Option<&'static str>,
    pub path: Option<&'static Path>,
    pub cloud: CloudState,
}

impl FileEntry {
    pub fn is_folder(&self) -> bool {
        self.kind == FileKind::Folder
    }
}

static RECENTS_FOLDER: FileEntry = FileEntry {
    id: "folder-recents",
    name: "Recents",
    kind: FileKind::Folder,
    parent: Location::Recents,
    target: Some(Location::Recents),
    modified: "—",
    modified_at: None,
    bytes: 0,
    metadata: "Recently modified files",
    tags: &[],
    preview: None,
    path: None,
    cloud: CloudState::Local,
};

/// Mounted volumes other than the startup disk: USB drives, network shares,
/// disk images. On macOS these live in `/Volumes`; on Linux in `/media/$USER`
/// and `/run/media/$USER`.
pub fn mounted_volumes() -> Vec<PathBuf> {
    let user = std::env::var("USER").unwrap_or_default();
    let roots = if cfg!(target_os = "macos") {
        vec![PathBuf::from("/Volumes")]
    } else {
        vec![PathBuf::from("/media").join(&user), PathBuf::from("/run/media").join(&user)]
    };
    let mut volumes: Vec<PathBuf> = roots
        .iter()
        .filter_map(|root| std::fs::read_dir(root).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        // The startup disk shows up as a link to `/`.
        .filter(|path| std::fs::canonicalize(path).is_ok_and(|real| real != Path::new("/")))
        .filter(|path| path.is_dir())
        .collect();
    volumes.sort();
    volumes
}

/// The account name, as `whoami` prints it.
pub fn user_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        let from_env = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).ok().filter(|name| !name.trim().is_empty());
        #[cfg(unix)]
        let from_system = || {
            // SAFETY: getpwuid returns static storage or null; copied out at once.
            unsafe {
                let entry = libc::getpwuid(libc::getuid());
                (!entry.is_null()).then(|| std::ffi::CStr::from_ptr((*entry).pw_name).to_string_lossy().into_owned())
            }
        };
        #[cfg(not(unix))]
        let from_system = || None::<String>;
        from_env.or_else(from_system).unwrap_or_else(|| "Local".to_string())
    })
}

/// The first letter of [`user_name`], for the avatar.
pub fn user_initial() -> String {
    user_name().chars().next().map_or_else(|| "?".to_string(), |letter| letter.to_uppercase().collect())
}

pub fn folder_entry(location: Location) -> Option<&'static FileEntry> {
    match location {
        Location::Recents | Location::Trash | Location::Tag(_) => Some(&RECENTS_FOLDER),
        Location::Remote(id) => remote::folder(id),
        Location::Dir(_) => fs_index::entry_for_path(&location.path()?),
    }
}

pub fn entries(location: Location, query: &str) -> Vec<&'static FileEntry> {
    let candidates = match location {
        Location::Remote(id) => remote::listing(id).unwrap_or_default(),
        Location::Recents => fs_index::recents(),
        Location::Trash => fs_index::trash(),
        Location::Tag(_) => fs_index::tagged(location),
        Location::Dir(_) => fs_index::list_dir(location),
    };
    rank_entries(candidates, query)
}

/// Shortens `name` to at most `max` characters with "…" in the middle,
/// keeping the extension so the file type stays visible:
/// `quarterly-financial-report-2026.pdf` → `quarterly-fina…2026.pdf`.
pub fn trim_middle(name: &str, max: usize) -> std::borrow::Cow<'_, str> {
    let length = name.chars().count();
    if length <= max || max < 5 {
        return std::borrow::Cow::Borrowed(name);
    }
    // Keep a short extension whole (".pdf", ".tar.gz" counts only ".gz").
    let extension = name
        .rfind('.')
        .filter(|&dot| dot > 0 && name.len() - dot <= 6)
        .map_or(0, |dot| name[dot..].chars().count());
    let budget = max - 1; // room for "…"
    let tail = (extension + (budget - extension) / 3).min(budget);
    let head = budget - tail;
    let start: String = name.chars().take(head).collect();
    let end: String = name.chars().skip(length - tail).collect();
    std::borrow::Cow::Owned(format!("{}…{}", start.trim_end(), end.trim_start()))
}

/// Cloud status of a file: placeholder, downloaded copy, or plain local file.
pub fn cloud_state(path: &Path) -> CloudState {
    if is_online_only(path) {
        CloudState::OnlineOnly
    } else if in_cloud_drive(path) {
        CloudState::Downloaded
    } else {
        CloudState::Local
    }
}

/// Synced folders: iCloud Drive and File Provider drives on macOS, OneDrive on
/// Windows. Symlinks such as `~/OneDrive` are resolved (once per folder).
fn in_cloud_drive(path: &Path) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static ROOTS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    static FOLDERS: OnceLock<Mutex<HashMap<PathBuf, bool>>> = OnceLock::new();
    let roots = ROOTS.get_or_init(|| {
        let mut roots = Vec::new();
        if let Some(home) = Location::Home.path() {
            if cfg!(target_os = "macos") {
                roots.push(home.join("Library/CloudStorage"));
                roots.push(home.join("Library/Mobile Documents"));
            }
            if cfg!(windows) {
                roots.push(home.join("OneDrive"));
            }
        }
        if let Some(onedrive) = std::env::var_os("OneDrive") {
            roots.push(PathBuf::from(onedrive));
        }
        roots
    });
    let Some(folder) = path.parent() else {
        return false;
    };
    let mut folders = FOLDERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *folders.entry(folder.to_path_buf()).or_insert_with(|| {
        let real = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
        roots.iter().any(|root| real.starts_with(root) || folder.starts_with(root))
    })
}

/// True for cloud placeholders (iCloud, OneDrive, Dropbox via File Provider)
/// whose data is not on disk; reading them blocks or times out.
pub use files_sdk::fs::is_online_only;

#[derive(Clone, Debug, PartialEq)]
pub struct DiskUsage {
    pub name: String,
    pub total: u64,
    pub available: u64,
}

impl DiskUsage {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }
}

/// The volume holding `path` (longest matching mount point), refreshed every 30 s.
pub fn disk_usage(path: &Path) -> Option<DiskUsage> {
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    type Cache = Option<(Instant, Vec<(PathBuf, DiskUsage)>)>;
    static DISKS: OnceLock<Mutex<Cache>> = OnceLock::new();
    let mut cache = DISKS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache
        .as_ref()
        .is_none_or(|(loaded, _)| loaded.elapsed() >= Duration::from_secs(30))
    {
        let disks = sysinfo::Disks::new_with_refreshed_list()
            .list()
            .iter()
            .map(|disk| {
                let mount = disk.mount_point().to_path_buf();
                let name = disk.name().to_string_lossy().into_owned();
                let usage = DiskUsage {
                    name: if name.is_empty() { mount.display().to_string() } else { name },
                    total: disk.total_space(),
                    available: disk.available_space(),
                };
                (mount, usage)
            })
            .collect();
        *cache = Some((Instant::now(), disks));
    }
    let (_, disks) = cache.as_ref()?;
    disks
        .iter()
        .filter(|(mount, _)| path.starts_with(mount))
        .max_by_key(|(mount, _)| mount.components().count())
        .map(|(_, usage)| usage.clone())
}

/// Recursive folder size in bytes, computed on background threads.
/// `None` while the first calculation is still running.
pub fn folder_size(entry: &FileEntry) -> Option<u64> {
    folder_sizes::get(entry.path?)
}

/// Bumped whenever a folder size finishes, so views know to repaint.
pub fn folder_sizes_generation() -> u64 {
    folder_sizes::generation()
}

/// The folder backing the trash on macOS and Linux (None on Windows).
pub fn trash_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(Location::Home.path()?.join(".Trash"))
    } else if cfg!(windows) {
        None
    } else {
        let data = std::env::var_os("XDG_DATA_HOME")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| Some(Location::Home.path()?.join(".local/share")))?;
        Some(data.join("Trash/files"))
    }
}

/// Why the trash cannot be listed, e.g. missing Full Disk Access on macOS.
pub fn trash_access_error() -> Option<String> {
    let dir = trash_dir()?;
    match std::fs::read_dir(&dir) {
        Ok(_) => None,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) if cfg!(target_os = "macos") && error.kind() == std::io::ErrorKind::PermissionDenied => Some(
            "macOS blocks reading the Trash · allow this app in System Settings → Privacy & Security → Full Disk Access"
                .to_string(),
        ),
        Err(error) => Some(format!("Cannot read {}: {error}", dir.display())),
    }
}

/// Finder tag names behind `Location::Tag` ids (a tag name never changes id).
/// Folders of the connected SSH server: interned paths and the listings
/// fetched for them. Everything is dropped when the connection changes.
pub mod remote {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    use super::{CloudState, FileEntry, FileKind, Location};
    use crate::ssh::RemoteItem;

    #[derive(Default)]
    struct State {
        paths: Vec<&'static str>,
        ids: HashMap<&'static str, u32>,
        /// Label of the top folder: the server's address.
        host: Option<&'static str>,
        listings: HashMap<u32, Vec<&'static FileEntry>>,
        entries: HashMap<&'static str, &'static FileEntry>,
    }

    fn lock() -> std::sync::MutexGuard<'static, State> {
        static STATE: OnceLock<Mutex<State>> = OnceLock::new();
        STATE.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn intern(path: &str) -> u32 {
        let mut state = lock();
        if let Some(id) = state.ids.get(path) {
            return *id;
        }
        let path: &'static str = Box::leak(path.to_string().into_boxed_str());
        let id = state.paths.len() as u32;
        state.paths.push(path);
        state.ids.insert(path, id);
        id
    }

    pub fn path(id: u32) -> &'static str {
        lock().paths[id as usize]
    }

    pub fn label(id: u32) -> &'static str {
        let path = path(id);
        if path == "/" {
            return lock().host.unwrap_or("/");
        }
        crate::ssh::file_name(path)
    }

    /// A new connection: forget the previous server's listings.
    pub fn reset(host: Option<String>) {
        let mut state = lock();
        state.listings.clear();
        state.host = host.map(|host| &*Box::leak(host.into_boxed_str()));
    }

    pub fn listing(id: u32) -> Option<Vec<&'static FileEntry>> {
        lock().listings.get(&id).cloned()
    }

    pub fn is_loaded(location: Location) -> bool {
        matches!(location, Location::Remote(id) if lock().listings.contains_key(&id))
    }


    pub fn entry(id: &str) -> Option<&'static FileEntry> {
        lock().entries.get(id).copied()
    }

    /// Every entry of every folder fetched so far.
    pub fn loaded_entries() -> Vec<&'static FileEntry> {
        lock().listings.values().flatten().copied().collect()
    }

    pub fn folder(id: u32) -> Option<&'static FileEntry> {
        let path = path(id);
        let key = format!("ssh:{path}");
        if let Some(entry) = entry(&key) {
            return Some(entry);
        }
        let entry = leak_entry(Location::Remote(id), &RemoteItem {
            name: label(id).to_string(),
            is_dir: true,
            bytes: 0,
            mode: 0,
            modified: "—".to_string(),
        }, path.to_string());
        Some(entry)
    }

    pub fn store(id: u32, items: Vec<RemoteItem>) {
        let folder = path(id);
        let mut entries: Vec<&'static FileEntry> = items
            .iter()
            .map(|item| {
                let child = crate::ssh::join(folder, &item.name);
                leak_entry(Location::Remote(id), item, child)
            })
            .collect();
        entries.sort_by_key(|entry| (!entry.is_folder(), entry.name.to_lowercase()));
        lock().listings.insert(id, entries);
    }

    /// The remote path behind an entry.
    pub fn entry_path(entry: &FileEntry) -> Option<&str> {
        entry.id.strip_prefix("ssh:")
    }

    fn leak_entry(parent: Location, item: &RemoteItem, path: String) -> &'static FileEntry {
        let key = format!("ssh:{path}");
        let kind = if item.is_dir {
            FileKind::Folder
        } else {
            std::path::Path::new(&item.name)
                .extension()
                .map_or(FileKind::Text, |extension| FileKind::from_extension(&extension.to_string_lossy()))
        };
        let leak = |value: String| -> &'static str { Box::leak(value.into_boxed_str()) };
        let mode: String = (0..9)
            .map(|index| if item.mode & (0o400 >> index) == 0 { '-' } else { ['r', 'w', 'x'][index % 3] })
            .collect();
        let entry: &'static FileEntry = Box::leak(Box::new(FileEntry {
            id: leak(key.clone()),
            name: leak(item.name.clone()),
            kind,
            parent,
            target: item.is_dir.then(|| Location::remote(&path)),
            modified: leak(item.modified.clone()),
            modified_at: None,
            bytes: item.bytes,
            metadata: leak(format!("{} · {mode} · on server", kind.label())),
            tags: &[],
            preview: None,
            path: None,
            cloud: CloudState::Local,
        }));
        lock().entries.insert(entry.id, entry);
        entry
    }
}

mod tag_names {
    use std::sync::{Mutex, OnceLock};

    fn names() -> &'static Mutex<Vec<&'static str>> {
        static NAMES: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
        NAMES.get_or_init(|| Mutex::new(Vec::new()))
    }

    pub fn intern(name: &str) -> u32 {
        let mut names = names().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(index) = names.iter().position(|known| *known == name) {
            return index as u32;
        }
        names.push(Box::leak(name.to_string().into_boxed_str()));
        (names.len() - 1) as u32
    }

    pub fn name(id: u32) -> &'static str {
        names().lock().unwrap_or_else(|poisoned| poisoned.into_inner())[id as usize]
    }
}

/// Drops cached listings after the filesystem was changed by the app.
pub fn invalidate_listings() {
    fs_index::invalidate();
}

/// Every indexed entry for global search: a bounded walk of the home folder.
pub fn searchable_entries() -> Vec<&'static FileEntry> {
    fs_index::search_index()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortKey {
    Name,
    Kind,
    Modified,
    Size,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SortOrder {
    pub key: SortKey,
    pub ascending: bool,
}

impl Default for SortOrder {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            ascending: true,
        }
    }
}

impl SortOrder {
    /// The direction a column starts with: A→Z for text, newest/largest first.
    fn initial_ascending(key: SortKey) -> bool {
        matches!(key, SortKey::Name | SortKey::Kind)
    }

    /// Clicks on one column cycle: initial direction → reversed → reset to default.
    pub fn toggled(self, key: SortKey) -> Self {
        let initial = Self::initial_ascending(key);
        if self.key != key {
            Self { key, ascending: initial }
        } else if self.ascending == initial {
            Self { key, ascending: !initial }
        } else {
            Self::default()
        }
    }
}

fn entry_bytes(entry: &FileEntry) -> u64 {
    if entry.is_folder() {
        folder_size(entry).unwrap_or(0)
    } else {
        entry.bytes
    }
}

/// Folders always stay before files; ties fall back to the name.
pub fn sort_entries(entries: &mut [&'static FileEntry], order: SortOrder) {
    entries.sort_by(|a, b| {
        let by_name = || a.name.to_lowercase().cmp(&b.name.to_lowercase());
        let by_key = match order.key {
            SortKey::Name => by_name(),
            SortKey::Kind => a.kind.label().cmp(b.kind.label()),
            SortKey::Modified => a.modified_at.cmp(&b.modified_at),
            SortKey::Size => entry_bytes(a).cmp(&entry_bytes(b)),
        };
        let by_key = if order.ascending { by_key } else { by_key.reverse() };
        b.is_folder()
            .cmp(&a.is_folder())
            .then(by_key)
            .then_with(by_name)
    });
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Compare {
    Less,
    LessEq,
    Greater,
    GreaterEq,
}

impl Compare {
    fn split(value: &str) -> (Option<Self>, &str) {
        for (prefix, compare) in [
            ("<=", Self::LessEq),
            (">=", Self::GreaterEq),
            ("<", Self::Less),
            (">", Self::Greater),
        ] {
            if let Some(rest) = value.strip_prefix(prefix) {
                return (Some(compare), rest);
            }
        }
        (None, value)
    }

    fn test<T: PartialOrd>(self, left: T, right: T) -> bool {
        match self {
            Self::Less => left < right,
            Self::LessEq => left <= right,
            Self::Greater => left > right,
            Self::GreaterEq => left >= right,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Filter {
    Kind(Vec<FileKind>),
    Extension(Vec<String>),
    Size(Compare, u64),
    /// Age of the last modification, e.g. `modified:<7d`.
    Age(Compare, std::time::Duration),
    /// Absolute modification date, e.g. `modified:>2026-10-01`.
    Date(Compare, std::time::SystemTime),
    Tag(String),
    /// Exact `rwxrwxrwx` permission bits, e.g. `mode:644` or `mode:rx-rx-rx`.
    Mode(u32),
}

/// A search box query: free text for fuzzy matching plus `key:value` filters.
///
/// Filters: `kind:image,video`, `ext:pdf,png`, `size:>10mb`, `size:<=500kb`,
/// `modified:<7d` (newer than 7 days), `modified:>1w` (older than a week),
/// `modified:today`, `modified:>2026-10-01`, `tag:work`, `mode:0o644`,
/// `mode:rw-r--r--`, `mode:rx-rx-rx`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchQuery {
    pub text: String,
    filters: Vec<Filter>,
}

impl SearchQuery {
    pub fn parse(query: &str) -> Self {
        let mut text = Vec::new();
        let mut filters = Vec::new();
        for token in query.split_whitespace() {
            match token.split_once(':').and_then(|(key, value)| parse_filter(key, value)) {
                Some(filter) => filters.push(filter),
                None => text.push(token),
            }
        }
        Self {
            text: text.join(" "),
            filters,
        }
    }

    pub fn matches(&self, entry: &FileEntry) -> bool {
        self.filters.iter().all(|filter| match filter {
            Filter::Kind(kinds) => kinds.contains(&entry.kind),
            Filter::Extension(extensions) => {
                let extension = Path::new(entry.name)
                    .extension()
                    .map(|extension| extension.to_string_lossy().to_lowercase());
                extension.is_some_and(|extension| extensions.contains(&extension))
            }
            Filter::Size(compare, bytes) => {
                let size = if entry.is_folder() {
                    folder_size(entry)
                } else {
                    Some(entry.bytes)
                };
                size.is_some_and(|size| compare.test(size, *bytes))
            }
            Filter::Age(compare, age) => entry
                .modified_at
                .and_then(|time| time.elapsed().ok())
                .is_some_and(|elapsed| compare.test(elapsed, *age)),
            Filter::Date(compare, date) => entry
                .modified_at
                .is_some_and(|time| compare.test(time, *date)),
            // Case-insensitive for any script (Зеленый = зеленый) and partial: tag:зел finds it.
            Filter::Tag(tag) => entry.tags.iter().any(|candidate| candidate.to_lowercase().contains(tag.as_str())),
            Filter::Mode(mode) => entry_mode(entry) == Some(*mode),
        })
    }
}

fn parse_filter(key: &str, value: &str) -> Option<Filter> {
    if value.is_empty() {
        return None;
    }
    let list = |value: &str| -> Vec<String> {
        value
            .split(',')
            .filter(|item| !item.is_empty())
            .map(|item| item.trim_start_matches('.').to_lowercase())
            .collect()
    };
    match key.to_lowercase().as_str() {
        "kind" | "type" => {
            let kinds: Vec<_> = list(value)
                .iter()
                .filter_map(|kind| parse_kind(kind))
                .collect();
            (!kinds.is_empty()).then_some(Filter::Kind(kinds))
        }
        "ext" | "extension" => Some(Filter::Extension(list(value))),
        "size" => {
            let (compare, rest) = Compare::split(value);
            Some(Filter::Size(compare.unwrap_or(Compare::GreaterEq), parse_bytes(rest)?))
        }
        "modified" | "date" => {
            if value.eq_ignore_ascii_case("today") {
                let midnight = chrono::Local::now()
                    .date_naive()
                    .and_hms_opt(0, 0, 0)?
                    .and_local_timezone(chrono::Local)
                    .earliest()?;
                return Some(Filter::Date(Compare::GreaterEq, midnight.into()));
            }
            let (compare, rest) = Compare::split(value);
            if let Some(age) = parse_age(rest) {
                return Some(Filter::Age(compare.unwrap_or(Compare::LessEq), age));
            }
            let date = chrono::NaiveDate::parse_from_str(rest, "%Y-%m-%d")
                .ok()?
                .and_hms_opt(0, 0, 0)?
                .and_local_timezone(chrono::Local)
                .earliest()?;
            Some(Filter::Date(compare.unwrap_or(Compare::GreaterEq), date.into()))
        }
        "tag" => Some(Filter::Tag(value.to_lowercase())),
        "mode" | "perm" | "permissions" => parse_mode(value).map(Filter::Mode),
        _ => None,
    }
}

/// Permission bits of the item itself (a symlink is not followed).
fn entry_mode(entry: &FileEntry) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        Some(std::fs::symlink_metadata(entry.path?).ok()?.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = entry;
        None
    }
}

/// Accepts octal (`644`, `0644`, `0o644`), `ls` style (`rw-r--r--`, with an
/// optional leading type letter like `drwxr-xr-x`) and the short form with
/// one group per class separated by dashes (`rw-r-r`, `rx-rx-rx`, `rwx--`).
fn parse_mode(value: &str) -> Option<u32> {
    let value = value.to_lowercase();
    let octal = value.strip_prefix("0o").unwrap_or(&value);
    if !octal.is_empty() && octal.len() <= 4 && octal.bytes().all(|byte| (b'0'..=b'7').contains(&byte)) {
        return u32::from_str_radix(octal, 8).ok().map(|mode| mode & 0o777);
    }
    let ls = match value.len() {
        10 if matches!(value.as_bytes()[0], b'-' | b'd' | b'l') => &value[1..],
        _ => value.as_str(),
    };
    if ls.len() == 9 {
        let mut mode = 0;
        let parsed = ls.bytes().enumerate().all(|(index, byte)| {
            let letter = b"rwx"[index % 3];
            if byte == letter {
                mode |= 0o400 >> index;
                true
            } else {
                byte == b'-'
            }
        });
        if parsed {
            return Some(mode);
        }
    }
    let groups: Vec<&str> = value.split('-').collect();
    if groups.len() != 3 {
        return None;
    }
    let mut mode = 0;
    for (class, group) in groups.iter().enumerate() {
        for letter in group.chars() {
            let bit = match letter {
                'r' => 0o4,
                'w' => 0o2,
                'x' => 0o1,
                _ => return None,
            };
            mode |= bit << (3 * (2 - class));
        }
    }
    Some(mode)
}

fn parse_kind(kind: &str) -> Option<FileKind> {
    Some(match kind.trim_end_matches('s') {
        "folder" | "dir" => FileKind::Folder,
        "image" | "picture" | "photo" => FileKind::Image,
        "video" | "movie" => FileKind::Video,
        "document" | "doc" => FileKind::Document,
        "design" => FileKind::Design,
        "code" | "source" => FileKind::Code,
        "archive" => FileKind::Archive,
        "audio" | "music" => FileKind::Audio,
        "text" => FileKind::Text,
        _ => return None,
    })
}

fn split_number(value: &str) -> Option<(f64, String)> {
    let index = value
        .find(|character: char| !(character.is_ascii_digit() || character == '.'))
        .unwrap_or(value.len());
    let number = value[..index].parse::<f64>().ok()?;
    Some((number, value[index..].to_lowercase()))
}

fn parse_bytes(value: &str) -> Option<u64> {
    let (number, unit) = split_number(value)?;
    let multiplier: f64 = match unit.as_str() {
        "" | "b" => 1.,
        "k" | "kb" => 1024.,
        "m" | "mb" => 1024f64.powi(2),
        "g" | "gb" => 1024f64.powi(3),
        "t" | "tb" => 1024f64.powi(4),
        _ => return None,
    };
    Some((number * multiplier) as u64)
}

fn parse_age(value: &str) -> Option<std::time::Duration> {
    let (number, unit) = split_number(value)?;
    let seconds: f64 = match unit.as_str() {
        "m" | "min" => 60.,
        "h" => 3600.,
        "d" => 86_400.,
        "w" => 7. * 86_400.,
        "mo" => 30. * 86_400.,
        "y" => 365. * 86_400.,
        _ => return None,
    };
    Some(std::time::Duration::from_secs_f64(number * seconds))
}

pub fn rank_entries(
    candidates: impl IntoIterator<Item = &'static FileEntry>,
    query: &str,
) -> Vec<&'static FileEntry> {
    use nucleo::pattern::{CaseMatching, Normalization, Pattern};
    use nucleo::{Config, Matcher, Utf32Str};

    let query = SearchQuery::parse(query);
    let candidates = candidates.into_iter().filter(|entry| query.matches(entry));
    if query.text.is_empty() {
        return candidates.collect();
    }

    let pattern = Pattern::parse(&query.text, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut matched: Vec<_> = candidates
        .filter_map(|entry| {
            let haystack = format!("{} {}", entry.name, entry.tags.join(" "));
            let mut buffer = Vec::new();
            let score = pattern.score(Utf32Str::new(&haystack, &mut buffer), &mut matcher)?;
            Some((entry, score))
        })
        .collect();
    matched.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    matched.into_iter().map(|(entry, _)| entry).collect()
}

pub fn entry(id: &str) -> Option<&'static FileEntry> {
    remote::entry(id).or_else(|| fs_index::entry_for_path(Path::new(id)))
}

/// Real filesystem access. Entries are leaked into a path-keyed registry so the
/// rest of the UI can keep holding `&'static FileEntry`; an entry is re-created
/// only when its size or modification time changes, which bounds the growth.
mod fs_index {
    use std::collections::{HashMap, VecDeque};
    use std::fs::{self, Metadata};
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant, SystemTime};

    use super::{FileEntry, FileKind, Location};

    const LISTING_TTL: Duration = Duration::from_secs(2);
    const SEARCH_TTL: Duration = Duration::from_secs(30);
    const RECENTS_LIMIT: usize = 60;
    const SEARCH_LIMIT: usize = 20_000;
    const SEARCH_DEPTH: usize = 4;

    struct Registry {
        dirs: Vec<(PathBuf, &'static str)>,
        dir_ids: HashMap<PathBuf, u32>,
        entries: HashMap<PathBuf, (Option<SystemTime>, u64, &'static FileEntry)>,
        listings: HashMap<Location, (Instant, Vec<&'static FileEntry>)>,
        search: Option<(Instant, Vec<&'static FileEntry>)>,
    }

    fn registry() -> &'static Mutex<Registry> {
        static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            let mut registry = Registry {
                dirs: Vec::new(),
                dir_ids: HashMap::new(),
                entries: HashMap::new(),
                listings: HashMap::new(),
                search: None,
            };
            // Order must match the `Location::Home`.. constants.
            for path in [
                Some(home.clone()),
                dirs::document_dir(),
                dirs::picture_dir(),
                dirs::download_dir(),
            ] {
                let path = path.unwrap_or_else(|| home.clone());
                let id = registry.dirs.len() as u32;
                registry.dirs.push((path.clone(), leak(dir_name(&path))));
                registry.dir_ids.entry(path).or_insert(id);
            }
            Mutex::new(registry)
        })
    }

    fn lock() -> std::sync::MutexGuard<'static, Registry> {
        registry().lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn leak(value: String) -> &'static str {
        Box::leak(value.into_boxed_str())
    }

    fn dir_name(path: &Path) -> String {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
    }

    pub fn intern_dir(path: &Path) -> u32 {
        let mut registry = lock();
        if let Some(id) = registry.dir_ids.get(path) {
            return *id;
        }
        let id = registry.dirs.len() as u32;
        registry
            .dirs
            .push((path.to_path_buf(), leak(dir_name(path))));
        registry.dir_ids.insert(path.to_path_buf(), id);
        id
    }

    pub fn dir_path(id: u32) -> PathBuf {
        lock().dirs[id as usize].0.clone()
    }

    pub fn dir_label(id: u32) -> &'static str {
        lock().dirs[id as usize].1
    }

    pub fn entry_for_path(path: &Path) -> Option<&'static FileEntry> {
        let metadata = fs::metadata(path).ok()?;
        Some(register(path, &metadata))
    }

    fn register(path: &Path, metadata: &Metadata) -> &'static FileEntry {
        let modified = metadata.modified().ok();
        let bytes = if metadata.is_dir() { 0 } else { metadata.len() };
        if let Some((cached_modified, cached_bytes, entry)) = lock().entries.get(path)
            && *cached_modified == modified
            && *cached_bytes == bytes
        {
            return entry;
        }
        let entry: &'static FileEntry = Box::leak(Box::new(build_entry(path, metadata)));
        lock()
            .entries
            .insert(path.to_path_buf(), (modified, bytes, entry));
        entry
    }

    fn build_entry(path: &Path, metadata: &Metadata) -> FileEntry {
        let is_dir = metadata.is_dir();
        let extension = path
            .extension()
            .map(|extension| extension.to_string_lossy().into_owned())
            .unwrap_or_default();
        let kind = if is_dir {
            FileKind::Folder
        } else {
            FileKind::from_extension(&extension)
        };
        let bytes = if is_dir { 0 } else { metadata.len() };
        let metadata_label = if is_dir {
            match fs::read_dir(path) {
                Ok(children) => format!(
                    "{} items",
                    children.filter_map(Result::ok).filter(is_visible).count()
                ),
                Err(_) => "Folder · no access".to_string(),
            }
        } else if super::is_online_only(path) {
            format!("{} · Online-only", kind.label())
        } else if extension.is_empty() {
            kind.label().to_string()
        } else {
            format!("{} · {}", extension.to_uppercase(), kind.label())
        };
        let path: &'static Path = Box::leak(path.to_path_buf().into_boxed_path());
        let parent = path
            .parent()
            .map(Location::from_path)
            .unwrap_or(Location::Home);
        FileEntry {
            id: leak(path.to_string_lossy().into_owned()),
            name: leak(dir_name(path)),
            kind,
            parent,
            target: is_dir.then(|| Location::from_path(path)),
            modified: leak(
                metadata
                    .modified()
                    .map(format_time)
                    .unwrap_or_else(|_| "—".to_string()),
            ),
            modified_at: metadata.modified().ok(),
            bytes,
            metadata: leak(metadata_label),
            tags: finder_tags_of(path),
            preview: None,
            path: Some(path),
            cloud: super::cloud_state(path),
        }
    }

    /// Finder tags as `'static` names (macOS only; empty elsewhere).
    fn finder_tags_of(path: &Path) -> &'static [&'static str] {
        let tags = crate::finder_tags::read(path);
        if tags.is_empty() {
            return &[];
        }
        let names: Vec<&'static str> = tags.into_iter().map(|tag| leak(tag.name)).collect();
        Box::leak(names.into_boxed_slice())
    }

    /// Matches the `Mon DD, YYYY, HH:MM` form `settings::format_demo_date` parses.
    fn format_time(time: SystemTime) -> String {
        chrono::DateTime::<chrono::Local>::from(time)
            .format("%b %d, %Y, %H:%M")
            .to_string()
    }

    fn is_visible(entry: &fs::DirEntry) -> bool {
        !entry.file_name().to_string_lossy().starts_with('.')
    }

    fn read_children(path: &Path) -> Vec<&'static FileEntry> {
        let Ok(children) = fs::read_dir(path) else {
            return Vec::new();
        };
        let mut entries: Vec<_> = children
            .filter_map(Result::ok)
            .filter(is_visible)
            .filter_map(|child| {
                let path = child.path();
                // Follow symlinks so linked folders stay navigable.
                let metadata = fs::metadata(&path).ok()?;
                Some(register(&path, &metadata))
            })
            .collect();
        entries.sort_by(|a, b| {
            b.is_folder()
                .cmp(&a.is_folder())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        entries
    }

    fn cached_listing(
        location: Location,
        load: impl FnOnce() -> Vec<&'static FileEntry>,
    ) -> Vec<&'static FileEntry> {
        if let Some((loaded, entries)) = lock().listings.get(&location)
            && loaded.elapsed() < LISTING_TTL
        {
            return entries.clone();
        }
        let entries = load();
        lock()
            .listings
            .insert(location, (Instant::now(), entries.clone()));
        entries
    }

    pub fn list_dir(location: Location) -> Vec<&'static FileEntry> {
        let Some(path) = location.path() else {
            return Vec::new();
        };
        cached_listing(location, || read_children(&path))
    }

    pub fn recents() -> Vec<&'static FileEntry> {
        cached_listing(Location::Recents, || {
            let home = Location::Home.path();
            let desktop = dirs::desktop_dir();
            let roots = [
                Location::Documents.path(),
                Location::Downloads.path(),
                Location::Pictures.path(),
                desktop,
                home,
            ];
            let mut files: Vec<_> = roots
                .into_iter()
                .flatten()
                .flat_map(|root| read_children(&root))
                .filter(|entry| !entry.is_folder())
                .collect();
            files.sort_by_key(|entry| entry.id);
            files.dedup_by_key(|entry| entry.id);
            files.sort_by_key(|entry| {
                std::cmp::Reverse(
                    entry
                        .path
                        .and_then(|path| fs::metadata(path).ok()?.modified().ok()),
                )
            });
            files.truncate(RECENTS_LIMIT);
            files
        })
    }

    pub fn invalidate() {
        let mut registry = lock();
        registry.listings.clear();
        registry.search = None;
    }

    pub fn tagged(location: Location) -> Vec<&'static FileEntry> {
        let Some(name) = location.tag_name() else {
            return Vec::new();
        };
        cached_listing(location, || {
            let mut entries: Vec<_> = crate::finder_tags::files_with(name)
                .iter()
                .filter_map(|path| entry_for_path(path))
                .collect();
            entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            entries
        })
    }

    pub fn trash() -> Vec<&'static FileEntry> {
        cached_listing(Location::Trash, || {
            #[cfg(windows)]
            {
                // The Recycle Bin is not a plain folder; list it through the shell API.
                trash::os_limited::list()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|item| {
                        let name = item.name.to_string_lossy().into_owned();
                        let entry: &'static FileEntry = Box::leak(Box::new(FileEntry {
                            id: leak(format!("trash:{}", item.id.to_string_lossy())),
                            kind: FileKind::from_extension(
                                Path::new(&name).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default().as_str(),
                            ),
                            name: leak(name),
                            parent: Location::Trash,
                            target: None,
                            modified: "—",
                            modified_at: None,
                            bytes: 0,
                            metadata: leak(format!("From {}", item.original_parent.display())),
                            tags: &[],
                            preview: None,
                            path: None,
                            cloud: super::CloudState::Local,
                        }));
                        entry
                    })
                    .collect()
            }
            #[cfg(not(windows))]
            {
                super::trash_dir().map(|dir| read_children(&dir)).unwrap_or_default()
            }
        })
    }

    pub fn search_index() -> Vec<&'static FileEntry> {
        if let Some((loaded, entries)) = &lock().search
            && loaded.elapsed() < SEARCH_TTL
        {
            return entries.clone();
        }
        let mut entries = Vec::new();
        let mut queue: VecDeque<(PathBuf, usize)> = Location::Home
            .path()
            .into_iter()
            .map(|path| (path, 0))
            .collect();
        while let Some((dir, depth)) = queue.pop_front() {
            for entry in read_children(&dir) {
                if entries.len() >= SEARCH_LIMIT {
                    break;
                }
                if entry.is_folder()
                    && depth + 1 < SEARCH_DEPTH
                    && let Some(path) = entry.path
                {
                    queue.push_back((path.to_path_buf(), depth + 1));
                }
                entries.push(entry);
            }
        }
        lock().search = Some((Instant::now(), entries.clone()));
        entries
    }
}

mod folder_sizes {
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    const WORKERS: usize = 4;
    const TTL: Duration = Duration::from_secs(60);

    #[derive(Clone, Copy)]
    struct Slot {
        bytes: Option<u64>,
        computed: Option<Instant>,
        queued: bool,
    }

    struct State {
        slots: Mutex<HashMap<PathBuf, Slot>>,
        queue: Mutex<Sender<PathBuf>>,
        generation: AtomicU64,
    }

    fn state() -> &'static State {
        static STATE: OnceLock<State> = OnceLock::new();
        STATE.get_or_init(|| {
            let (sender, receiver) = channel::<PathBuf>();
            let receiver = Arc::new(Mutex::new(receiver));
            for index in 0..WORKERS {
                let receiver = receiver.clone();
                std::thread::Builder::new()
                    .name(format!("folder-size-{index}"))
                    .spawn(move || worker(&receiver))
                    .expect("failed to spawn folder size worker");
            }
            State {
                slots: Mutex::new(HashMap::new()),
                queue: Mutex::new(sender),
                generation: AtomicU64::new(0),
            }
        })
    }

    fn worker(receiver: &Mutex<Receiver<PathBuf>>) {
        loop {
            let next = receiver
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .recv();
            let Ok(path) = next else { return };
            let bytes = dir_size(&path);
            let state = state();
            state
                .slots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(
                    path,
                    Slot {
                        bytes: Some(bytes),
                        computed: Some(Instant::now()),
                        queued: false,
                    },
                );
            state.generation.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Sums file sizes below `root` without following symlinks.
    fn dir_size(root: &Path) -> u64 {
        let mut total = 0;
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(children) = fs::read_dir(&dir) else {
                continue;
            };
            for child in children.filter_map(Result::ok) {
                let Ok(file_type) = child.file_type() else {
                    continue;
                };
                if file_type.is_dir() {
                    stack.push(child.path());
                } else if file_type.is_file()
                    && let Ok(metadata) = child.metadata()
                {
                    total += metadata.len();
                }
            }
        }
        total
    }

    pub fn get(path: &Path) -> Option<u64> {
        let state = state();
        let mut slots = state
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let slot = slots.entry(path.to_path_buf()).or_insert(Slot {
            bytes: None,
            computed: None,
            queued: false,
        });
        let stale = slot.computed.is_none_or(|computed| computed.elapsed() >= TTL);
        if stale && !slot.queued {
            slot.queued = true;
            let _ = state
                .queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .send(path.to_path_buf());
        }
        // Keep showing the previous value while a refresh runs.
        slot.bytes
    }

    pub fn generation() -> u64 {
        state().generation.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn file(name: &'static str, kind: FileKind, bytes: u64, age_days: u64) -> &'static FileEntry {
        Box::leak(Box::new(FileEntry {
            id: name,
            name,
            kind,
            parent: Location::Home,
            target: None,
            modified: "—",
            modified_at: Some(SystemTime::now() - Duration::from_secs(age_days * 86_400)),
            bytes,
            metadata: "",
            tags: &["Work"],
            preview: None,
            path: None,
            cloud: CloudState::Local,
        }))
    }

    fn names(entries: &[&'static FileEntry]) -> Vec<&'static str> {
        entries.iter().map(|entry| entry.name).collect()
    }

    fn fixtures() -> Vec<&'static FileEntry> {
        vec![
            file("big.mov", FileKind::Video, 500 * 1024 * 1024, 40),
            file("photo.png", FileKind::Image, 3 * 1024 * 1024, 1),
            file("notes.md", FileKind::Text, 2 * 1024, 3),
            file("report.pdf", FileKind::Document, 12 * 1024 * 1024, 10),
        ]
    }

    #[test]
    fn search_filters_by_kind_extension_size_and_age() {
        let run = |query| names(&rank_entries(fixtures(), query));
        assert_eq!(run("kind:image"), ["photo.png"]);
        assert_eq!(run("kind:video,document"), ["big.mov", "report.pdf"]);
        assert_eq!(run("ext:md"), ["notes.md"]);
        assert_eq!(run("size:>10mb"), ["big.mov", "report.pdf"]);
        assert_eq!(run("size:<=2kb"), ["notes.md"]);
        assert_eq!(run("modified:<7d"), ["photo.png", "notes.md"]);
        assert_eq!(run("modified:>30d"), ["big.mov"]);
        assert_eq!(run("size:>1mb modified:<7d"), ["photo.png"]);
        assert_eq!(run("tag:work kind:text"), ["notes.md"]);
        assert_eq!(run("tag:WOR kind:text"), ["notes.md"], "tag: is partial and case-insensitive");
        assert_eq!(run("size:>1mb rep"), ["report.pdf"]);
    }

    #[test]
    fn unknown_filters_fall_back_to_text() {
        let query = SearchQuery::parse("foo:bar size:big kind:image");
        assert_eq!(query.text, "foo:bar size:big");
    }

    #[test]
    fn sorts_by_each_column_with_toggle() {
        let mut entries = fixtures();
        let order = SortOrder::default().toggled(SortKey::Size);
        assert!(!order.ascending, "size starts largest first");
        sort_entries(&mut entries, order);
        assert_eq!(names(&entries), ["big.mov", "report.pdf", "photo.png", "notes.md"]);

        let reversed = order.toggled(SortKey::Size);
        sort_entries(&mut entries, reversed);
        assert_eq!(names(&entries), ["notes.md", "photo.png", "report.pdf", "big.mov"]);
        assert_eq!(reversed.toggled(SortKey::Size), SortOrder::default(), "third click resets");

        sort_entries(&mut entries, SortOrder::default().toggled(SortKey::Modified));
        assert_eq!(names(&entries), ["photo.png", "notes.md", "report.pdf", "big.mov"]);

        sort_entries(&mut entries, SortOrder::default().toggled(SortKey::Kind));
        assert_eq!(names(&entries), ["report.pdf", "photo.png", "notes.md", "big.mov"]);
    }

    #[test]
    fn folder_size_sums_nested_files() {
        let root = std::env::temp_dir().join(format!("file-force-size-{}", std::process::id()));
        let nested = root.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("one.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(nested.join("two.bin"), vec![0u8; 2500]).unwrap();

        let entry = fs_index::entry_for_path(&root).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let size = loop {
            if let Some(size) = folder_size(entry) {
                break size;
            }
            assert!(std::time::Instant::now() < deadline, "folder size never finished");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(size, 3500);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn disk_usage_finds_the_home_volume() {
        let home = Location::Home.path().unwrap();
        let disk = disk_usage(&home).expect("home is on a mounted volume");
        assert!(!disk.name.is_empty());
        assert!(disk.total > 0);
        assert!(disk.available <= disk.total);
        assert_eq!(disk.used(), disk.total - disk.available);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn finder_tags_reach_entries_and_tag_search() {
        let dir = std::env::temp_dir().join(format!("file-force-entry-tags-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tagged = dir.join("tagged.md");
        std::fs::write(&tagged, "x").unwrap();
        std::fs::write(dir.join("plain.md"), "x").unwrap();
        let mut bytes = Vec::new();
        plist::to_writer_binary(&mut bytes, &vec!["Client\n3".to_string(), "Зеленый\n2".to_string()]).unwrap();
        xattr::set(&tagged, "com.apple.metadata:_kMDItemUserTags", &bytes).unwrap();

        let listed = entries(Location::from_path(&dir), "");
        let entry = listed.iter().find(|entry| entry.name == "tagged.md").unwrap();
        assert_eq!(entry.tags, ["Client", "Зеленый"]);
        for query in ["tag:client", "tag:зеленый", "tag:ЗЕЛ", "tag:лен"] {
            let found = entries(Location::from_path(&dir), query);
            assert_eq!(found.iter().map(|entry| entry.name).collect::<Vec<_>>(), ["tagged.md"], "{query}");
        }
        assert_eq!(Location::tag("Client").label(), "Client");
        assert_eq!(Location::tag("Client"), Location::tag("Client"), "tags intern to one id");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn cloud_state_tells_placeholders_downloads_and_local_files_apart() {
        let local = std::env::temp_dir().join(format!("file-force-cloud-{}", std::process::id()));
        std::fs::write(&local, "x").unwrap();
        assert_eq!(cloud_state(&local), CloudState::Local);
        let _ = std::fs::remove_file(&local);

        // Real cloud files on this machine, when there are any to compare.
        let home = Location::Home.path().unwrap();
        for root in [home.join("Library/Mobile Documents"), home.join("Library/CloudStorage")] {
            let Ok(drives) = std::fs::read_dir(&root) else { continue };
            for drive in drives.filter_map(Result::ok).take(3) {
                let Ok(files) = std::fs::read_dir(drive.path()) else { continue };
                for file in files.filter_map(Result::ok).take(20) {
                    let state = cloud_state(&file.path());
                    assert_ne!(state, CloudState::Local, "{} is in a cloud drive", file.path().display());
                    assert_eq!(state == CloudState::OnlineOnly, is_online_only(&file.path()));
                }
            }
        }
    }

    #[test]
    fn trim_middle_keeps_the_extension_and_the_length() {
        assert_eq!(trim_middle("short.txt", 20), "short.txt");
        let trimmed = trim_middle("quarterly-financial-report-2026-final.pdf", 24);
        assert_eq!(trimmed.chars().count(), 24, "{trimmed}");
        assert!(trimmed.ends_with(".pdf") && trimmed.contains('…'), "{trimmed}");
        assert_eq!(trim_middle("AndroidStudioProjects", 16).chars().count(), 16);
        // Multi-byte names are cut on character boundaries.
        let cyrillic = trim_middle("Лукина - Бизнес и_или любовь.zip.zip", 20);
        assert!(cyrillic.ends_with(".zip") && cyrillic.chars().count() == 20, "{cyrillic}");
        assert_eq!(trim_middle("x".repeat(30).as_str(), 4).len(), 30, "tiny budgets leave names alone");
    }

    #[test]
    fn the_workspace_shows_the_account_name() {
        let whoami = std::process::Command::new("whoami").output().map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string());
        if let Some(whoami) = whoami.ok().filter(|name| !name.is_empty()) {
            assert_eq!(user_name(), whoami);
        }
        assert_eq!(user_initial().chars().count(), 1);
        assert!(user_initial().chars().all(|letter| !letter.is_lowercase()));
    }

    #[test]
    fn mode_filter_accepts_octal_ls_and_short_forms() {
        for (text, mode) in [
            ("666", 0o666),
            ("0644", 0o644),
            ("0o755", 0o755),
            ("rw-r--r--", 0o644),
            ("drwxr-xr-x", 0o755),
            ("rx-rx-rx", 0o555),
            ("rw-r-r", 0o644),
            ("rwx--", 0o700),
            ("RW-RW-RW", 0o666),
        ] {
            assert_eq!(parse_mode(text), Some(mode), "{text}");
        }
        for text in ["9", "rwz-r-r", "rw-r", "abc"] {
            assert_eq!(parse_mode(text), None, "{text}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn search_filters_by_mode() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("file-force-mode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, mode) in [("shared.txt", 0o666), ("script.sh", 0o555), ("private.txt", 0o600)] {
            let path = dir.join(name);
            std::fs::write(&path, "x").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let names = |query: &str| {
            entries(Location::from_path(&dir), query).iter().map(|entry| entry.name).collect::<Vec<_>>()
        };
        assert_eq!(names("mode:0o666"), ["shared.txt"]);
        assert_eq!(names("mode:rx-rx-rx"), ["script.sh"]);
        assert_eq!(names("private mode:rw-------"), ["private.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
