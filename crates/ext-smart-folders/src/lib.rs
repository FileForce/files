//! Smart folders: every video, photo, script, application or duplicate on
//! this computer, gathered in the background (Spotlight on macOS, a bounded
//! walk of the home folder elsewhere or when Spotlight finds nothing).

use std::collections::HashMap;
use std::hash::Hasher as _;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;

pub static MANIFEST: files_sdk::Manifest = files_sdk::Manifest {
    id: "smart-folders",
    name: "Smart Folders",
    version: env!("CARGO_PKG_VERSION"),
    source: "Built-in",
    summary: "Videos, photos, scripts, applications and duplicate files gathered in the background.",
    icon: files_sdk::Icon::Builtin("copy"),
    adds: &["Sidebar → Smart Folders: Videos, Photos, Executables, Applications and Duplicates"],
    permissions: &[files_sdk::Permission::ReadFiles],
    commands: &[],
    menu: &[],
    sidebar: &[],
    menu_color: None,
    tier: files_sdk::Tier::Free,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SmartFolder {
    Videos,
    Photos,
    Executables,
    Applications,
    Duplicates,
}

impl SmartFolder {
    pub const ALL: [Self; 5] = [
        Self::Videos,
        Self::Photos,
        Self::Executables,
        Self::Applications,
        Self::Duplicates,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Videos => "Smart Videos",
            Self::Photos => "Smart Photos",
            Self::Executables => "Smart Executables",
            Self::Applications => "Smart Applications",
            Self::Duplicates => "Smart Duplicates",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::Videos => IconName::Film,
            Self::Photos => IconName::Images,
            Self::Executables => IconName::SquareTerminal,
            Self::Applications => IconName::LayoutGrid,
            Self::Duplicates => IconName::Copy,
        }
    }

    /// Its name in a saved session (`smart:videos`).
    pub fn key(self) -> &'static str {
        match self {
            Self::Videos => "videos",
            Self::Photos => "photos",
            Self::Executables => "executables",
            Self::Applications => "applications",
            Self::Duplicates => "duplicates",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|folder| folder.key() == key)
    }

    /// Duplicates hash file contents: gather them less often.
    fn refresh(self) -> Duration {
        match self {
            Self::Duplicates => Duration::from_secs(30 * 60),
            _ => Duration::from_secs(5 * 60),
        }
    }
}

/// Most files one smart folder lists.
const MAX_FILES: usize = 20_000;
/// Most entries a walk of the home folder looks at.
const WALK_BUDGET: usize = 250_000;
/// Smaller files are not worth reporting as duplicates.
const MIN_DUPLICATE_BYTES: u64 = 1024;

#[derive(Default)]
struct Slot {
    loaded: Option<Instant>,
    loading: bool,
    files: Vec<PathBuf>,
}

fn slots() -> &'static Mutex<HashMap<SmartFolder, Slot>> {
    static SLOTS: OnceLock<Mutex<HashMap<SmartFolder, Slot>>> = OnceLock::new();
    SLOTS.get_or_init(Default::default)
}

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Bumped each time a smart folder finishes gathering.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Whether `folder` is being gathered or refreshed in the background.
pub fn is_loading(folder: SmartFolder) -> bool {
    slots()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&folder)
        .is_some_and(|slot| slot.loading)
}

/// What `folder` holds so far; starts (or refreshes) the gathering.
pub fn files(folder: SmartFolder) -> Vec<PathBuf> {
    let mut all = slots()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let slot = all.entry(folder).or_default();
    let fresh = slot
        .loaded
        .is_some_and(|loaded| loaded.elapsed() < folder.refresh());
    if !fresh && !slot.loading {
        slot.loading = true;
        std::thread::Builder::new()
            .name(format!("smart-{}", folder.key()))
            .spawn(move || {
                let files = gather(folder);
                let mut slots = slots()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let slot = slots.entry(folder).or_default();
                slot.files = files;
                slot.loaded = Some(Instant::now());
                slot.loading = false;
                drop(slots);
                files_sdk::host().files_changed();
                GENERATION.fetch_add(1, Ordering::Relaxed);
            })
            .expect("failed to spawn smart folder gathering");
    }
    slot.files.clone()
}

fn gather(folder: SmartFolder) -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    if folder == SmartFolder::Duplicates {
        return duplicates(walk(&home, |_| true));
    }
    let mut found = spotlight(folder, &home);
    if found.is_empty() {
        found = match folder {
            SmartFolder::Applications => applications(),
            _ => walk(&home, |path| matches_kind(folder, path)),
        };
    }
    found.truncate(MAX_FILES);
    found
}

/// Folders that hold no user files but can hold millions of entries.
fn skipped(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "Library"
                | "AppData"
                | "node_modules"
                | "target"
                | "Pods"
                | "DerivedData"
                | "build"
                | "dist"
                | "vendor"
        )
}

/// Not hidden, not in Library, not inside an app or a package.
fn user_file(path: &Path, home: &Path) -> bool {
    let Ok(inside) = path.strip_prefix(home) else {
        return true;
    };
    let mut parts = inside
        .components()
        .map(|part| part.as_os_str().to_string_lossy());
    let file = parts.next_back();
    parts
        .all(|part| !skipped(&part) && !part.ends_with(".app") && !part.ends_with(".photoslibrary"))
        && file.is_some()
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|extension| extension.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn matches_kind(folder: SmartFolder, path: &Path) -> bool {
    let extension = extension(path);
    match folder {
        SmartFolder::Videos => matches!(
            extension.as_str(),
            "mp4" | "mov" | "m4v" | "webm" | "mkv" | "avi" | "wmv" | "flv" | "mpg" | "mpeg"
        ),
        SmartFolder::Photos => matches!(
            extension.as_str(),
            "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "bmp"
                | "tif"
                | "tiff"
                | "webp"
                | "heic"
                | "heif"
                | "avif"
                | "jxl"
                | "ico"
        ),
        SmartFolder::Executables => is_script(&extension),
        SmartFolder::Applications => matches!(
            extension.as_str(),
            "app" | "exe" | "msi" | "appimage" | "deb" | "rpm"
        ),
        SmartFolder::Duplicates => true,
    }
}

fn is_script(extension: &str) -> bool {
    matches!(
        extension,
        "sh" | "bash" | "zsh" | "fish" | "command" | "tool" | "ps1" | "bat" | "cmd"
    )
}

/// Spotlight's answer, or nothing when it is off (or not macOS).
fn spotlight(folder: SmartFolder, home: &Path) -> Vec<PathBuf> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let query = match folder {
        SmartFolder::Videos => "kMDItemContentTypeTree == 'public.movie'",
        SmartFolder::Photos => "kMDItemContentTypeTree == 'public.image'",
        SmartFolder::Executables => "kMDItemContentTypeTree == 'public.shell-script'",
        SmartFolder::Applications => "kMDItemContentType == 'com.apple.application-bundle'",
        SmartFolder::Duplicates => return Vec::new(),
    };
    let mut command = std::process::Command::new("/usr/bin/mdfind");
    // Applications live outside the home folder too.
    if folder != SmartFolder::Applications {
        command.arg("-onlyin").arg(home);
    }
    let Ok(output) = command
        .arg(query)
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(PathBuf::from)
        .filter(|path| match folder {
            // Not the helper apps inside other apps, not system internals.
            SmartFolder::Applications => {
                let text = path.to_string_lossy();
                text.matches(".app/").count() == 0
                    && !text.starts_with("/System/Library")
                    && !text.starts_with("/Library")
            }
            // Spotlight calls TypeScript `.ts` an MPEG stream and `.py` a
            // shell script: the extension must agree too.
            _ => user_file(path, home) && matches_kind(folder, path),
        })
        .take(MAX_FILES)
        .collect();
    found.sort_by_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
    });
    found
}

/// Installed applications without Spotlight.
fn applications() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        roots.extend(["/Applications", "/System/Applications"].map(PathBuf::from));
        roots.extend(dirs::home_dir().map(|home| home.join("Applications")));
    } else if cfg!(target_os = "linux") {
        roots.push(PathBuf::from("/usr/share/applications"));
        roots.extend(dirs::data_dir().map(|data| data.join("applications")));
    } else if cfg!(windows) {
        roots.extend(
            std::env::var_os("ProgramData")
                .map(|data| PathBuf::from(data).join(r"Microsoft\Windows\Start Menu\Programs")),
        );
        roots.extend(
            dirs::data_dir().map(|data| data.join(r"Microsoft\Windows\Start Menu\Programs")),
        );
    }
    let mut found = Vec::new();
    let mut stack = roots;
    while let Some(folder) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let extension = extension(&path);
            if matches!(extension.as_str(), "app" | "desktop" | "lnk" | "exe") {
                found.push(path);
            } else if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                stack.push(path);
            }
        }
    }
    found.sort_by_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
    });
    found
}

/// Files under `root` that `keep` accepts; looks at no more than
/// [`WALK_BUDGET`] entries and never follows links.
fn walk(root: &Path, keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0;
    while let Some(folder) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > WALK_BUDGET {
                return found;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if skipped(&name) || name.ends_with(".app") || name.ends_with(".photoslibrary") {
                continue;
            }
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(path),
                Ok(kind) if kind.is_file() && keep(&path) => found.push(path),
                _ => {}
            }
        }
    }
    found
}

/// The files with a twin of the same contents, twins next to each other,
/// biggest first: same size, then the same first 16 KB, then the same bytes.
fn duplicates(files: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for path in files {
        if let Ok(metadata) = std::fs::metadata(&path)
            && metadata.len() >= MIN_DUPLICATE_BYTES
        {
            by_size.entry(metadata.len()).or_default().push(path);
        }
    }
    let mut sizes: Vec<u64> = by_size
        .iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(size, _)| *size)
        .collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    let mut found = Vec::new();
    for size in sizes {
        let same_size = by_size.remove(&size).unwrap_or_default();
        for same_start in twins(same_size, |path| hash(path, Some(16 * 1024))) {
            let full = if size <= 16 * 1024 {
                vec![same_start]
            } else {
                twins(same_start, |path| hash(path, None))
            };
            for group in full {
                found.extend(group);
                if found.len() >= MAX_FILES {
                    return found;
                }
            }
        }
    }
    found
}

/// `paths` grouped by `key`, only the groups of two or more.
fn twins(paths: Vec<PathBuf>, key: impl Fn(&Path) -> Option<u64>) -> Vec<Vec<PathBuf>> {
    let mut groups: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for path in paths {
        if let Some(key) = key(&path) {
            groups.entry(key).or_default().push(path);
        }
    }
    let mut groups: Vec<Vec<PathBuf>> = groups
        .into_values()
        .filter(|group| group.len() > 1)
        .collect();
    for group in &mut groups {
        group.sort();
    }
    groups.sort();
    groups
}

/// A hash of the file's first `limit` bytes (all of them for `None`).
fn hash(path: &Path, limit: Option<u64>) -> Option<u64> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader: Box<dyn std::io::Read> = match limit {
        Some(limit) => Box::new(file.take(limit)),
        None => Box::new(file),
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).ok()?;
        if read == 0 {
            return Some(hasher.finish());
        }
        hasher.write(&buffer[..read]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_twins_by_contents_not_names() {
        let dir = std::env::temp_dir().join(format!("file-force-smart-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        let body = vec![7u8; 40 * 1024];
        let mut other = body.clone();
        *other.last_mut().unwrap() = 8;
        std::fs::write(dir.join("a.bin"), &body).unwrap();
        std::fs::write(dir.join("nested/copy.bin"), &body).unwrap();
        // Same size, same first 16 KB, different last byte: not a twin.
        std::fs::write(dir.join("almost.bin"), &other).unwrap();
        std::fs::write(dir.join("tiny1.txt"), b"same").unwrap();
        std::fs::write(dir.join("tiny2.txt"), b"same").unwrap();
        let found = duplicates(walk(&dir, |_| true));
        let names: Vec<String> = found
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.bin", "copy.bin"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scripts_and_media_by_extension() {
        assert!(matches_kind(
            SmartFolder::Executables,
            Path::new("/x/deploy.sh")
        ));
        assert!(!matches_kind(
            SmartFolder::Executables,
            Path::new("/x/readme.md")
        ));
        assert!(matches_kind(SmartFolder::Videos, Path::new("/x/clip.MP4")));
        assert!(matches_kind(SmartFolder::Photos, Path::new("/x/photo.jpg")));
    }

    #[test]
    fn library_and_hidden_folders_are_not_user_files() {
        let home = Path::new("/Users/me");
        assert!(user_file(Path::new("/Users/me/Movies/clip.mov"), home));
        assert!(!user_file(
            Path::new("/Users/me/Library/Caches/x.png"),
            home
        ));
        assert!(!user_file(Path::new("/Users/me/.cache/x.png"), home));
        assert!(!user_file(
            Path::new("/Users/me/Apps/Tool.app/Contents/icon.png"),
            home
        ));
    }
}
