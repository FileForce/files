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
    /// Programs: .exe, .msi, .app, .AppImage…
    Application,
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
            Self::Application => "Application",
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
            "exe" | "msi" | "appimage" | "app" | "bat" | "cmd" | "com" | "scr" | "pkg" | "mpkg" | "prefpane" | "appex" | "bundle" | "plugin" | "kext" | "saver" => Self::Application,
            "xcodeproj" | "xcworkspace" | "playground" | "swiftpm" => Self::Code,
            "photoslibrary" => Self::Image,
            "rtfd" | "scptd" => Self::Document,
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

/// A shared file entry: views and caches hold clones; the last one to go
/// frees it (and its text).
pub type Entry = std::sync::Arc<FileEntry>;

#[derive(Debug)]
pub struct FileEntry {
    pub id: gpui_kit::SharedString,
    pub name: gpui_kit::SharedString,
    pub kind: FileKind,
    pub parent: Location,
    pub target: Option<Location>,
    pub modified: gpui_kit::SharedString,
    pub modified_at: Option<std::time::SystemTime>,
    pub bytes: u64,
    pub metadata: gpui_kit::SharedString,
    pub tags: &'static [&'static str],
    pub preview: Option<&'static str>,
    pub path: Option<std::sync::Arc<Path>>,
    pub cloud: CloudState,
}

/// Entries alive right now (DevTools): a number that only grows is a leak.
pub static LIVE_ENTRIES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl FileEntry {
    /// Counts the entry as alive; every entry is made through here.
    pub fn shared(self) -> Entry {
        LIVE_ENTRIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::sync::Arc::new(self)
    }
}

impl Drop for FileEntry {
    fn drop(&mut self) {
        LIVE_ENTRIES.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl FileEntry {
    pub fn is_folder(&self) -> bool {
        self.kind == FileKind::Folder
    }
}

static RECENTS_FOLDER: std::sync::LazyLock<Entry> = std::sync::LazyLock::new(|| FileEntry {
    id: "folder-recents".into(),
    name: "Recents".into(),
    kind: FileKind::Folder,
    parent: Location::Recents,
    target: Some(Location::Recents),
    modified: "—".into(),
    modified_at: None,
    bytes: 0,
    metadata: "Recently modified files".into(),
    tags: &[],
    preview: None,
    path: None,
    cloud: CloudState::Local,
}
.shared());

/// Mounted volumes other than the startup disk: USB drives, network shares,
/// disk images. On macOS these live in `/Volumes`; on Linux in `/media/$USER`
/// and `/run/media/$USER`.
pub fn mounted_volumes() -> Vec<PathBuf> {
    // Windows: every drive letter but the system one (USB sticks, DVDs,
    // mounted images, network drives).
    if cfg!(windows) {
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()).to_ascii_uppercase();
        return ('A'..='Z')
            .map(|letter| format!("{letter}:"))
            .filter(|drive| *drive != system && drive != "A:" && drive != "B:")
            .map(|drive| PathBuf::from(format!("{drive}\\")))
            .filter(|root| root.is_dir())
            .collect();
    }
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

/// Folders macOS shows as one file (Finder's packages): apps, installers,
/// Xcode projects, the Photos library, Keynote/Pages/Numbers documents…
/// Double-click opens them; Quick Actions → Show Package Contents goes in.
pub const PACKAGES: [&str; 21] = [
    "app", "pkg", "mpkg", "bundle", "plugin", "kext", "prefpane", "appex", "saver", "xcodeproj", "xcworkspace", "playground", "swiftpm",
    "photoslibrary", "musiclibrary", "rtfd", "scptd", "key", "pages", "numbers", "wdgt",
];

pub fn is_package(path: &Path) -> bool {
    cfg!(target_os = "macos")
        && path.extension().is_some_and(|extension| PACKAGES.iter().any(|package| package.eq_ignore_ascii_case(&extension.to_string_lossy())))
        && path.is_dir()
}

/// Disk images the system can mount: ISO and IMG everywhere, DMG on macOS,
/// VHD/VHDX on Windows.
pub fn mountable(path: &Path) -> bool {
    let extension = path.extension().map(|extension| extension.to_string_lossy().to_lowercase()).unwrap_or_default();
    match extension.as_str() {
        "iso" | "img" => true,
        "dmg" | "sparseimage" | "sparsebundle" => cfg!(target_os = "macos"),
        "vhd" | "vhdx" => cfg!(windows),
        _ => false,
    }
}

/// The mount point from hdiutil's or udisksctl's output.
fn mount_point(output: &str) -> Option<PathBuf> {
    output.lines().rev().find_map(|line| {
        // hdiutil: "/dev/disk4s1 \t Apple_HFS \t /Volumes/Name"; udisksctl:
        // "Mounted /dev/loop0 at /media/me/LABEL" (sometimes with a dot).
        if let Some(at) = line.find("/Volumes/") {
            return Some(PathBuf::from(line[at..].trim()));
        }
        line.split_once(" at ").map(|(_, path)| PathBuf::from(path.trim().trim_end_matches('.')))
    })
}

/// Mounts a disk image read-only and returns where its files are: macOS
/// hdiutil, Windows Mount-DiskImage (a drive letter), Linux udisksctl (no
/// administrator rights).
pub fn mount_image(path: &Path) -> std::io::Result<PathBuf> {
    let run = |command: &mut std::process::Command| -> std::io::Result<String> {
        let output = files_sdk::log::run("files", command)?;
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        if output.status.success() {
            Ok(text)
        } else {
            let reason = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(std::io::Error::other(if reason.is_empty() { text.trim().to_string() } else { reason }))
        }
    };
    if cfg!(target_os = "macos") {
        let output = run(files_sdk::log::command("/usr/bin/hdiutil").args(["attach", "-readonly"]).arg(path))?;
        return mount_point(&output).ok_or_else(|| std::io::Error::other("The image has no volume to open"));
    }
    if cfg!(windows) {
        let quoted = path.display().to_string().replace('\'', "''");
        let script = format!("$v = Mount-DiskImage -ImagePath '{quoted}' -Access ReadOnly -PassThru | Get-Volume; ($v | Where-Object DriveLetter | Select-Object -First 1).DriveLetter");
        let letter = run(files_sdk::log::command("powershell").args(["-NoProfile", "-Command", &script]))?;
        let letter = letter.trim().chars().next().filter(char::is_ascii_alphabetic).ok_or_else(|| std::io::Error::other("Windows mounted the image without a drive letter"))?;
        return Ok(PathBuf::from(format!("{letter}:\\")));
    }
    let setup = run(files_sdk::log::command("udisksctl").args(["loop-setup", "-r", "-f"]).arg(path))?;
    // "Mapped file X as /dev/loop0."
    let device = setup.split_whitespace().last().map(|device| device.trim_end_matches('.').to_string()).filter(|device| device.starts_with("/dev/")).ok_or_else(|| std::io::Error::other("udisksctl gave no loop device"))?;
    // The desktop may mount it by itself; otherwise mount it, or its first partition.
    for _ in 0..10 {
        for candidate in [device.clone(), format!("{device}p1")] {
            if let Ok(target) = run(files_sdk::log::command("findmnt").args(["-no", "TARGET", &candidate]))
                && !target.trim().is_empty()
            {
                return Ok(PathBuf::from(target.trim()));
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let mounted = run(files_sdk::log::command("udisksctl").args(["mount", "-b", &device])).or_else(|_| run(files_sdk::log::command("udisksctl").args(["mount", "-b", &format!("{device}p1")])))?;
    mount_point(&mounted).ok_or_else(|| std::io::Error::other("udisksctl did not say where it mounted the image"))
}

/// Ejects a disk, a USB drive or a mounted disk image (unmounts a share).
pub fn eject(path: &Path) -> std::io::Result<()> {
    let run = |program: &str, args: &[&std::ffi::OsStr]| -> std::io::Result<()> {
        let output = files_sdk::log::run("files", files_sdk::log::command(program).args(args))?;
        if output.status.success() {
            Ok(())
        } else {
            let reason = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(std::io::Error::other(if reason.is_empty() { format!("{program} could not eject it") } else { reason }))
        }
    };
    let target = path.as_os_str();
    if cfg!(target_os = "macos") {
        // diskutil handles drives and images; hdiutil is the fallback for images.
        run("/usr/sbin/diskutil", &["eject".as_ref(), target]).or_else(|_| run("/usr/bin/hdiutil", &["detach".as_ref(), target]))
    } else if cfg!(windows) {
        // Explorer's own "Eject" verb on the drive.
        let drive = path.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
        let script = format!("(New-Object -ComObject Shell.Application).Namespace(17).ParseName('{drive}').InvokeVerb('Eject')");
        run("powershell", &["-NoProfile".as_ref(), "-Command".as_ref(), script.as_ref()])
    } else {
        // A mounted image: unmount, then free its loop device.
        let source = files_sdk::log::run("files", files_sdk::log::command("findmnt").args(["-no", "SOURCE"]).arg(path))
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .filter(|source| source.starts_with("/dev/loop"));
        if let Some(source) = source {
            run("udisksctl", &["unmount".as_ref(), "-b".as_ref(), source.as_ref()])?;
            // /dev/loop0p1 → /dev/loop0.
            let device = match source.rfind('p') {
                Some(at) if at > "/dev/loo".len() => &source[..at],
                _ => source.as_str(),
            };
            let _ = run("udisksctl", &["loop-delete".as_ref(), "-b".as_ref(), device.as_ref()]);
            return Ok(());
        }
        run("gio", &["mount".as_ref(), "-u".as_ref(), target]).or_else(|_| run("umount", &[target]))
    }
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

pub fn folder_entry(location: Location) -> Option<Entry> {
    match location {
        Location::Recents | Location::Trash | Location::Tag(_) => Some(RECENTS_FOLDER.clone()),
        Location::Remote(id) => remote::folder(id),
        Location::Dir(_) => fs_index::entry_for_path(&location.path()?),
    }
}

pub fn entries(location: Location, query: &str) -> Vec<Entry> {
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
    mounted_disks()
        .into_iter()
        .filter(|(mount, _)| path.starts_with(mount))
        .max_by_key(|(mount, _)| mount.components().count())
        .map(|(_, usage)| usage)
}

/// System volumes macOS mounts for itself (APFS Data, VM, Preboot…) and
/// Linux pseudo mounts: not disks a user thinks of.
fn is_system_mount(mount: &Path) -> bool {
    let text = mount.to_string_lossy();
    ["/System/Volumes/", "/private/var/vm", "/boot", "/snap/", "/run/", "/dev", "/proc", "/sys", "/var/lib/"]
        .iter()
        .any(|prefix| text.starts_with(prefix))
}

/// The disks shown in the sidebar: the startup disk, other internal disks,
/// USB drives and network shares (C:, D:, … on Windows), largest first after
/// the startup disk; duplicates of one volume are shown once.
pub fn user_disks() -> Vec<(PathBuf, DiskUsage)> {
    #[cfg(test)]
    if let Some(disks) = TEST_DISKS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone() {
        return disks;
    }
    let mut disks: Vec<(PathBuf, DiskUsage)> = mounted_disks().into_iter().filter(|(mount, usage)| usage.total > 0 && !is_system_mount(mount)).collect();
    let mut seen = std::collections::HashSet::new();
    disks.retain(|(_, usage)| seen.insert((usage.name.clone(), usage.total)));
    // The startup disk ("/" or C:\) first, then the rest by path.
    disks.sort_by_key(|(mount, _)| (mount.components().count() > 1, mount.clone()));
    disks
}

/// Tests show these disks instead of this computer's.
#[cfg(test)]
pub(crate) static TEST_DISKS: std::sync::Mutex<Option<Vec<(PathBuf, DiskUsage)>>> = std::sync::Mutex::new(None);

fn mounted_disks() -> Vec<(PathBuf, DiskUsage)> {
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
    cache.as_ref().map(|(_, disks)| disks.clone()).unwrap_or_default()
}

/// Recursive folder size in bytes, computed on background threads.
/// `None` while the first calculation is still running.
pub fn folder_size(entry: &FileEntry) -> Option<u64> {
    folder_sizes::get(entry.path.as_deref()?)
}

/// Bumped whenever a folder size finishes, so views know to repaint.
pub fn folder_sizes_generation() -> u64 {
    folder_sizes::generation()
}

/// DevTools → Indexes: forget measured folder sizes / how many are known.
pub fn clear_folder_sizes() {
    folder_sizes::clear();
}

pub fn folder_sizes_count() -> usize {
    folder_sizes::count()
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
        listings: HashMap<u32, Vec<crate::model::Entry>>,
        entries: HashMap<String, crate::model::Entry>,
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
        super::listings_changed();
    }

    pub fn listing(id: u32) -> Option<Vec<crate::model::Entry>> {
        lock().listings.get(&id).cloned()
    }

    pub fn is_loaded(location: Location) -> bool {
        matches!(location, Location::Remote(id) if lock().listings.contains_key(&id))
    }


    pub fn entry(id: &str) -> Option<crate::model::Entry> {
        lock().entries.get(id).cloned()
    }

    /// Every entry of every folder fetched so far.
    pub fn loaded_entries() -> Vec<crate::model::Entry> {
        lock().listings.values().flatten().cloned().collect()
    }

    pub fn folder(id: u32) -> Option<crate::model::Entry> {
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
        let mut entries: Vec<crate::model::Entry> = items
            .iter()
            .map(|item| {
                let child = crate::ssh::join(folder, &item.name);
                leak_entry(Location::Remote(id), item, child)
            })
            .collect();
        entries.sort_by_key(|entry| (!entry.is_folder(), entry.name.to_lowercase()));
        lock().listings.insert(id, entries);
        super::listings_changed();
    }

    /// The remote path behind an entry.
    pub fn entry_path(entry: &FileEntry) -> Option<&str> {
        entry.id.strip_prefix("ssh:")
    }

    fn leak_entry(parent: Location, item: &RemoteItem, path: String) -> crate::model::Entry {
        let key = format!("ssh:{path}");
        let kind = if item.is_dir {
            FileKind::Folder
        } else {
            std::path::Path::new(&item.name)
                .extension()
                .map_or(FileKind::Text, |extension| FileKind::from_extension(&extension.to_string_lossy()))
        };
        let mode: String = (0..9)
            .map(|index| if item.mode & (0o400 >> index) == 0 { '-' } else { ['r', 'w', 'x'][index % 3] })
            .collect();
        let entry = FileEntry {
            id: key.clone().into(),
            name: item.name.clone().into(),
            kind,
            parent,
            target: item.is_dir.then(|| Location::remote(&path)),
            modified: item.modified.clone().into(),
            modified_at: None,
            bytes: item.bytes,
            metadata: format!("{} · {mode} · on server", kind.label()).into(),
            tags: &[],
            preview: None,
            path: None,
            cloud: CloudState::Local,
        }
        .shared();
        lock().entries.insert(key, entry.clone());
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

/// Bumped whenever a folder listing is read again or forgotten (local or
/// remote): views reuse what they built from listings until it changes.
static LISTINGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn listings_generation() -> u64 {
    LISTINGS.load(std::sync::atomic::Ordering::Relaxed)
}

fn listings_changed() {
    LISTINGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Every indexed entry for global search: a bounded walk of the home folder.
/// Memory bookkeeping for the developer tools: (entries known, entries made
/// since launch, cached folder listings, global search index size).
pub fn index_stats() -> (usize, u64, usize, usize) {
    fs_index::stats()
}

/// Forgets global search's file list (built again on the next search).
pub fn clear_search_index() {
    fs_index::clear_search();
}

/// The entry for a file on disk (to reveal it).
pub fn entry_at(path: &Path) -> Option<Entry> {
    fs_index::entry_for_path(path)
}

/// Everything under `root` (a few levels deep) for global search.
pub fn searchable_entries(root: &Path) -> Vec<Entry> {
    fs_index::search_index(root)
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
    if entry.is_folder() || entry.path.as_deref().is_some_and(is_package) {
        folder_size(entry).unwrap_or(0)
    } else {
        entry.bytes
    }
}

/// Folders always stay before files; ties fall back to the name.
/// Compares names ignoring case without making lowercase copies: sorting a
/// big folder compares names tens of thousands of times.
pub fn cmp_ignore_case(a: &str, b: &str) -> std::cmp::Ordering {
    if a.is_ascii() && b.is_ascii() {
        return a.bytes().map(|byte| byte.to_ascii_lowercase()).cmp(b.bytes().map(|byte| byte.to_ascii_lowercase()));
    }
    a.chars().flat_map(char::to_lowercase).cmp(b.chars().flat_map(char::to_lowercase))
}

pub fn sort_entries(entries: &mut [Entry], order: SortOrder) {
    entries.sort_by(|a, b| {
        let by_name = || cmp_ignore_case(&a.name, &b.name);
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
    /// A filter an extension adds, e.g. `git:main` (value lowercase).
    Contributed(Contributor, String, String),
}

/// An extension answering a search filter.
#[derive(Clone, Copy)]
pub struct Contributor(&'static dyn files_sdk::Extension);

impl std::fmt::Debug for Contributor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0.manifest().id)
    }
}

impl PartialEq for Contributor {
    fn eq(&self, other: &Self) -> bool {
        self.0.manifest().id == other.0.manifest().id
    }
}

/// Where a search runs: the folder's search box, or Cmd-K over the home folder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchScope {
    Folder,
    Global,
}

/// Filters that stay plain text, per scope: extensions turned off, or a
/// filter switched off for that box in Settings.
#[derive(Default)]
struct SearchOff {
    extensions: Vec<&'static str>,
    folder_keys: Vec<String>,
    global_keys: Vec<String>,
}

static SEARCH_OFF: std::sync::RwLock<SearchOff> =
    std::sync::RwLock::new(SearchOff { extensions: Vec::new(), folder_keys: Vec::new(), global_keys: Vec::new() });

pub fn set_search_disabled(ids: Vec<&'static str>) {
    SEARCH_OFF.write().unwrap_or_else(|poisoned| poisoned.into_inner()).extensions = ids;
}

/// Filter keys (`git`…) switched off in the folder search / in Cmd-K.
pub fn set_search_keys_off(folder: Vec<String>, global: Vec<String>) {
    let mut off = SEARCH_OFF.write().unwrap_or_else(|poisoned| poisoned.into_inner());
    off.folder_keys = folder;
    off.global_keys = global;
}

/// The enabled extension that answers `key:` in `scope`.
fn contributor(key: &str, scope: SearchScope) -> Option<Contributor> {
    let off = SEARCH_OFF.read().unwrap_or_else(|poisoned| poisoned.into_inner());
    let keys = match scope {
        SearchScope::Folder => &off.folder_keys,
        SearchScope::Global => &off.global_keys,
    };
    if keys.iter().any(|off| off.eq_ignore_ascii_case(key)) {
        return None;
    }
    crate::views::extension_ui::registry()
        .into_iter()
        .filter(|extension| !off.extensions.contains(&extension.manifest().id))
        .find(|extension| extension.search_keys().iter().any(|search| search.key.eq_ignore_ascii_case(key)))
        .map(Contributor)
}

/// `git: …` hints for the search placeholder, from enabled extensions.
pub fn contributed_search_keys(scope: SearchScope) -> Vec<files_sdk::SearchKey> {
    crate::views::extension_ui::registry()
        .into_iter()
        .flat_map(|extension| extension.search_keys().iter().copied())
        .filter(|search| contributor(search.key, scope).is_some())
        .collect()
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
        Self::parse_in(query, SearchScope::Folder)
    }

    pub fn parse_in(query: &str, scope: SearchScope) -> Self {
        let mut text = Vec::new();
        let mut filters = Vec::new();
        for token in query.split_whitespace() {
            match token.split_once(':').and_then(|(key, value)| parse_filter(key, value, scope)) {
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
                let extension = Path::new(&*entry.name)
                    .extension()
                    .map(|extension| extension.to_string_lossy().to_lowercase());
                // Partial, like names: ext:web finds .webp and .webm, ext:j .jpg.
                extension.is_some_and(|extension| extensions.iter().any(|wanted| extension.starts_with(wanted.as_str())))
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
            Filter::Contributed(extension, key, value) => {
                entry.path.as_deref().is_some_and(|path| extension.0.search_matches(key, value, path))
            }
        })
    }
}

fn parse_filter(key: &str, value: &str, scope: SearchScope) -> Option<Filter> {
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
        other => contributor(other, scope).map(|extension| Filter::Contributed(extension, other.to_string(), value.to_lowercase())),
    }
}

/// Permission bits of the item itself (a symlink is not followed).
fn entry_mode(entry: &FileEntry) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        Some(std::fs::symlink_metadata(entry.path.as_deref()?).ok()?.permissions().mode() & 0o777)
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

/// A kind by name, or by the start of one (`kind:vid`, `kind:img`).
fn parse_kind(kind: &str) -> Option<FileKind> {
    const NAMES: [(&str, FileKind); 15] = [
        ("folder", FileKind::Folder),
        ("dir", FileKind::Folder),
        ("image", FileKind::Image),
        ("picture", FileKind::Image),
        ("photo", FileKind::Image),
        ("video", FileKind::Video),
        ("movie", FileKind::Video),
        ("document", FileKind::Document),
        ("design", FileKind::Design),
        ("code", FileKind::Code),
        ("source", FileKind::Code),
        ("archive", FileKind::Archive),
        ("audio", FileKind::Audio),
        ("music", FileKind::Audio),
        ("text", FileKind::Text),
    ];
    let kind = kind.trim_end_matches('s');
    if kind == "img" {
        return Some(FileKind::Image);
    }
    if let Some(exact) = parse_kind_exact(kind) {
        return Some(exact);
    }
    (kind.len() >= 2).then(|| NAMES.iter().find(|(name, _)| name.starts_with(kind)).map(|(_, kind)| *kind)).flatten()
}

fn parse_kind_exact(kind: &str) -> Option<FileKind> {
    Some(match kind {
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
    candidates: impl IntoIterator<Item = Entry>,
    query: &str,
) -> Vec<Entry> {
    rank_entries_in(candidates, query, SearchScope::Folder)
}

pub fn rank_entries_in(
    candidates: impl IntoIterator<Item = Entry>,
    query: &str,
    scope: SearchScope,
) -> Vec<Entry> {
    use nucleo::pattern::{CaseMatching, Normalization, Pattern};
    use nucleo::{Config, Matcher, Utf32Str};

    let query = SearchQuery::parse_in(query, scope);
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

pub fn entry(id: &str) -> Option<Entry> {
    remote::entry(id).or_else(|| fs_index::entry_for_path(Path::new(id)))
}

/// Real filesystem access. Entries are leaked into a path-keyed registry so the
/// rest of the UI can keep holding `Entry`; an entry is re-created
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
        entries: HashMap<PathBuf, (Option<SystemTime>, u64, crate::model::Entry)>,
        listings: HashMap<Location, (Instant, Vec<crate::model::Entry>)>,
        search: Option<(Instant, PathBuf, Vec<crate::model::Entry>)>,
    }

    /// Entries made since launch: they live until quit, so a number that
    /// keeps growing is memory that keeps growing.
    static MADE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// (entries known now, entries made since launch, cached listings, search index size)
    pub fn stats() -> (usize, u64, usize, usize) {
        let registry = lock();
        (
            registry.entries.len(),
            MADE.load(std::sync::atomic::Ordering::Relaxed),
            registry.listings.len(),
            registry.search.as_ref().map_or(0, |(_, _, entries)| entries.len()),
        )
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

    pub fn entry_for_path(path: &Path) -> Option<crate::model::Entry> {
        let metadata = fs::metadata(path).ok()?;
        Some(register(path, &metadata))
    }

    fn register(path: &Path, metadata: &Metadata) -> crate::model::Entry {
        let modified = metadata.modified().ok();
        let bytes = if metadata.is_dir() { 0 } else { metadata.len() };
        if let Some((cached_modified, cached_bytes, entry)) = lock().entries.get(path)
            && *cached_modified == modified
            && *cached_bytes == bytes
        {
            return entry.clone();
        }
        // The replaced entry is freed once no view holds it any more.
        let entry = build_entry(path, metadata).shared();
        MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        lock()
            .entries
            .insert(path.to_path_buf(), (modified, bytes, entry.clone()));
        entry
    }

    fn build_entry(path: &Path, metadata: &Metadata) -> FileEntry {
        // A macOS package (an app, a Keynote document…) is one file here.
        let is_dir = metadata.is_dir() && !super::is_package(path);
        let extension = path
            .extension()
            .map(|extension| extension.to_string_lossy().into_owned())
            .unwrap_or_default();
        let kind = if is_dir {
            FileKind::Folder
        } else {
            FileKind::from_extension(&extension)
        };
        // A package's size is its folder's, measured in the background.
        let bytes = if is_dir || metadata.is_dir() { 0 } else { metadata.len() };
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
        let parent = path
            .parent()
            .map(Location::from_path)
            .unwrap_or(Location::Home);
        FileEntry {
            id: path.to_string_lossy().into_owned().into(),
            name: dir_name(path).into(),
            kind,
            parent,
            target: is_dir.then(|| Location::from_path(path)),
            modified: metadata
                .modified()
                .map(format_time)
                .unwrap_or_else(|_| "—".to_string())
                .into(),
            modified_at: metadata.modified().ok(),
            bytes,
            metadata: metadata_label.into(),
            tags: finder_tags_of(path),
            preview: None,
            path: Some(path.into()),
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

    fn read_children(path: &Path) -> Vec<crate::model::Entry> {
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
        load: impl FnOnce() -> Vec<crate::model::Entry>,
    ) -> Vec<crate::model::Entry> {
        if let Some((loaded, entries)) = lock().listings.get(&location)
            && loaded.elapsed() < LISTING_TTL
        {
            return entries.clone();
        }
        let entries = load();
        lock()
            .listings
            .insert(location, (Instant::now(), entries.clone()));
        super::listings_changed();
        entries
    }

    pub fn list_dir(location: Location) -> Vec<crate::model::Entry> {
        let Some(path) = location.path() else {
            return Vec::new();
        };
        cached_listing(location, || read_children(&path))
    }

    pub fn recents() -> Vec<crate::model::Entry> {
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
            files.sort_by(|a, b| a.id.cmp(&b.id));
            files.dedup_by(|a, b| a.id == b.id);
            files.sort_by_key(|entry| {
                std::cmp::Reverse(
                    entry
                        .path
                        .as_deref()
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
        super::listings_changed();
    }

    pub fn tagged(location: Location) -> Vec<crate::model::Entry> {
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

    pub fn trash() -> Vec<crate::model::Entry> {
        cached_listing(Location::Trash, || {
            #[cfg(windows)]
            {
                // The Recycle Bin is not a plain folder; list it through the shell API.
                trash::os_limited::list()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|item| {
                        let name = item.name.to_string_lossy().into_owned();
                        FileEntry {
                            id: format!("trash:{}", item.id.to_string_lossy()).into(),
                            kind: FileKind::from_extension(
                                Path::new(&name).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default().as_str(),
                            ),
                            name: name.into(),
                            parent: Location::Trash,
                            target: None,
                            modified: "—".into(),
                            modified_at: None,
                            bytes: 0,
                            metadata: format!("From {}", item.original_parent.display()).into(),
                            tags: &[],
                            preview: None,
                            path: None,
                            cloud: super::CloudState::Local,
                        }
                        .shared()
                    })
                    .collect()
            }
            #[cfg(not(windows))]
            {
                super::trash_dir().map(|dir| read_children(&dir)).unwrap_or_default()
            }
        })
    }

    pub fn clear_search() {
        lock().search = None;
    }

    pub fn search_index(root: &Path) -> Vec<crate::model::Entry> {
        if let Some((loaded, indexed, entries)) = &lock().search
            && loaded.elapsed() < SEARCH_TTL
            && indexed == root
        {
            return entries.clone();
        }
        let job = crate::tasks::queue("search index", root.display().to_string());
        crate::tasks::start(job);
        let mut entries = Vec::new();
        let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::from([(root.to_path_buf(), 0)]);
        while let Some((dir, depth)) = queue.pop_front() {
            for entry in read_children(&dir) {
                if entries.len() >= SEARCH_LIMIT {
                    break;
                }
                if entry.is_folder()
                    && depth + 1 < SEARCH_DEPTH
                    && let Some(path) = entry.path.as_deref()
                {
                    queue.push_back((path.to_path_buf(), depth + 1));
                }
                entries.push(entry);
            }
        }
        lock().search = Some((Instant::now(), root.to_path_buf(), entries.clone()));
        crate::tasks::finish(job, Ok(()));
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
        queue: Mutex<Sender<(PathBuf, u64)>>,
        generation: AtomicU64,
    }

    fn state() -> &'static State {
        static STATE: OnceLock<State> = OnceLock::new();
        STATE.get_or_init(|| {
            let (sender, receiver) = channel::<(PathBuf, u64)>();
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

    fn worker(receiver: &Mutex<Receiver<(PathBuf, u64)>>) {
        loop {
            let next = receiver
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .recv();
            let Ok((path, job)) = next else { return };
            if !crate::tasks::start(job) {
                if let Some(slot) = state().slots.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get_mut(&path) {
                    slot.queued = false;
                }
                continue;
            }
            let bytes = dir_size(&path);
            crate::tasks::finish(job, Ok(()));
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
                .send((path.to_path_buf(), crate::tasks::queue("folder size", path.display().to_string())));
        }
        // Keep showing the previous value while a refresh runs.
        slot.bytes
    }

    /// DevTools → Indexes: sizes measured again as folders show.
    pub fn clear() {
        state().slots.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
        state().generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn count() -> usize {
        state().slots.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).values().filter(|slot| slot.bytes.is_some()).count()
    }

    pub fn generation() -> u64 {
        state().generation.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    /// A watched file changing over and over frees each entry it replaces.
    #[test]
    #[ignore]
    fn leak_per_changed_file() {
        let dir = std::env::temp_dir().join(format!("file-force-leak-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("growing-build-output-with-a-long-name.rlib");
        std::fs::write(&file, b"x").unwrap();
        let _ = fs_index::entry_for_path(&file);
        let before = crate::alloc_stats::counts();
        let live = LIVE_ENTRIES.load(std::sync::atomic::Ordering::Relaxed);
        for size in 2..10_002u64 {
            std::fs::File::options().write(true).open(&file).unwrap().set_len(size).unwrap();
            let _ = fs_index::entry_for_path(&file);
        }
        let after = crate::alloc_stats::counts() - before;
        println!("10,000 changes: {} allocations never freed, {} bytes allocated in all ({} per change)", after.allocs.saturating_sub(after.frees), after.bytes, after.bytes / 10_000);
        // Each replaced entry is freed: the one file is still one entry.
        assert_eq!(LIVE_ENTRIES.load(std::sync::atomic::Ordering::Relaxed), live, "entries alive after 10,000 changes");
    }

    use super::*;
    use std::time::{Duration, SystemTime};

    fn file(name: &'static str, kind: FileKind, bytes: u64, age_days: u64) -> Entry {
        FileEntry {
            id: name.into(),
            name: name.into(),
            kind,
            parent: Location::Home,
            target: None,
            modified: "—".into(),
            modified_at: Some(SystemTime::now() - Duration::from_secs(age_days * 86_400)),
            bytes,
            metadata: "".into(),
            tags: &["Work"],
            preview: None,
            path: None,
            cloud: CloudState::Local,
        }
        .shared()
    }

    fn names(entries: &[Entry]) -> Vec<gpui_kit::SharedString> {
        entries.iter().map(|entry| entry.name.clone()).collect()
    }

    fn fixtures() -> Vec<Entry> {
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
        assert_eq!(run("ext:pn"), ["photo.png"], "ext: is partial, from the start (ext:web finds .webp)");
        assert_eq!(run("ext:ng"), Vec::<&str>::new(), "…not from the middle");
        assert_eq!(run("kind:vid"), ["big.mov"], "kind: takes the start of a name");
        assert_eq!(run("kind:img"), ["photo.png"]);
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
    fn images_mount_where_the_system_says() {
        assert!(mountable(Path::new("ubuntu.ISO")) && mountable(Path::new("disk.img")));
        assert_eq!(mountable(Path::new("Files.dmg")), cfg!(target_os = "macos"));
        assert!(!mountable(Path::new("notes.txt")));
        let hdiutil = "/dev/disk4          \tGUID_partition_scheme\t\n/dev/disk4s1        \tApple_HFS                      \t/Volumes/Files 0.1.14\n";
        assert_eq!(mount_point(hdiutil), Some(PathBuf::from("/Volumes/Files 0.1.14")));
        assert_eq!(mount_point("Mounted /dev/loop0 at /media/me/Ubuntu 24.04."), Some(PathBuf::from("/media/me/Ubuntu 24.04")));
    }

    #[test]
    fn ejecting_something_that_is_not_mounted_says_why() {
        let error = eject(Path::new("/Volumes/file-force-not-a-disk-xyz")).unwrap_err();
        assert!(!error.to_string().is_empty(), "the reason is shown to the user");
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
            if let Some(size) = folder_size(&entry) {
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
            assert_eq!(found.iter().map(|entry| entry.name.clone()).collect::<Vec<_>>(), ["tagged.md"], "{query}");
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
    fn only_disks_people_know_are_listed() {
        assert!(is_system_mount(Path::new("/System/Volumes/Data")));
        assert!(is_system_mount(Path::new("/private/var/vm")));
        assert!(!is_system_mount(Path::new("/")));
        assert!(!is_system_mount(Path::new("/Volumes/USB")));
        let disks = user_disks();
        assert!(!disks.is_empty(), "the startup disk is always there");
        assert_eq!(disks[0].0.components().count(), 1, "startup disk first: {:?}", disks[0].0);
        assert!(disks.iter().all(|(mount, usage)| !is_system_mount(mount) && usage.total > 0));
    }

    #[test]
    fn the_workspace_shows_the_account_name() {
        let whoami = files_sdk::log::command("whoami").output().map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string());
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
            entries(Location::from_path(&dir), query).iter().map(|entry| entry.name.clone()).collect::<Vec<_>>()
        };
        assert_eq!(names("mode:0o666"), ["shared.txt"]);
        assert_eq!(names("mode:rx-rx-rx"), ["script.sh"]);
        assert_eq!(names("private mode:rw-------"), ["private.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod mount_live {
    #[test]
    #[ignore]
    fn mount_and_eject_an_image() {
        let image = std::path::PathBuf::from(std::env::var("IMAGE").unwrap());
        let target = super::mount_image(&image).unwrap();
        println!("mounted at {}", target.display());
        assert!(std::fs::read_dir(&target).unwrap().count() > 0);
        super::eject(&target).unwrap();
        assert!(!target.exists());
    }
}
