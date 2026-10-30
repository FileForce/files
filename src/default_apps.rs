//! Which application opens a file type: the system's choice (read from
//! LaunchServices on macOS) and this app's own per-extension overrides, kept
//! in `settings.toml` under `[default_apps]`. Overrides never touch the OS.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Extensions listed in Settings even without an override.
pub const COMMON: &[&str] = &[
    "docx", "xlsx", "pptx", "pdf", "txt", "md", "json", "html", "csv", "png", "jpg", "heic",
    "mp4", "mov", "mp3", "zip",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppChoice {
    pub name: String,
    pub path: PathBuf,
}

impl AppChoice {
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        Self { name, path }
    }
}

/// `".DOCX"` and `"docx"` are the same key.
pub fn normalize_extension(extension: &str) -> String {
    extension.trim().trim_start_matches('.').to_lowercase()
}

pub fn extension_of(path: &Path) -> Option<String> {
    path.extension().map(|extension| normalize_extension(&extension.to_string_lossy()))
}

type Cache = HashMap<String, (Option<AppChoice>, Vec<AppChoice>)>;

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// System default and every app able to open `extension` (cached).
fn lookup(extension: &str) -> (Option<AppChoice>, Vec<AppChoice>) {
    let extension = normalize_extension(extension);
    if let Some(found) = cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(&extension) {
        return found.clone();
    }
    let found = platform::lookup(&extension);
    cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(extension, found.clone());
    found
}

/// The app the OS opens `extension` files with, if it can tell.
pub fn system_default(extension: &str) -> Option<AppChoice> {
    lookup(extension).0
}

/// Apps the OS considers able to open `extension`, by name.
pub fn candidates(extension: &str) -> Vec<AppChoice> {
    lookup(extension).1
}

/// The override for `path`'s type, if one is set and the app still exists.
pub fn override_for<'a>(path: &Path, overrides: &'a BTreeMap<String, String>) -> Option<&'a str> {
    let app = overrides.get(&extension_of(path)?)?;
    Path::new(app).exists().then_some(app.as_str())
}

/// Opens `path` with its override, or with the system default otherwise.
/// Returns the name of the app used when an override applied.
pub fn open(path: &Path, overrides: &BTreeMap<String, String>) -> io::Result<Option<String>> {
    // Tests record what would be opened instead of launching real apps.
    #[cfg(test)]
    {
        let app = override_for(path, overrides).map(|app| AppChoice::from_path(app).name);
        opened().lock().unwrap().push((path.to_path_buf(), app.clone()));
        return Ok(app);
    }
    #[cfg(not(test))]
    if let Some(app) = override_for(path, overrides) {
        platform::open_with(Path::new(app), path)?;
        return Ok(Some(AppChoice::from_path(app).name));
    }
    #[cfg(not(test))]
    {
        platform::open_default(path)?;
        Ok(None)
    }
}

/// Files "opened" during tests, with the override app name if one applied.
#[cfg(test)]
pub fn opened() -> &'static Mutex<Vec<(PathBuf, Option<String>)>> {
    static OPENED: OnceLock<Mutex<Vec<(PathBuf, Option<String>)>>> = OnceLock::new();
    OPENED.get_or_init(|| Mutex::new(Vec::new()))
}

/// Asks the user to pick any application (macOS "Choose Application" dialog).
/// `Ok(None)` when cancelled or unsupported on this platform.
pub fn choose_application() -> io::Result<Option<PathBuf>> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", "POSIX path of (path to (choose application with prompt \"Default app:\"))"])
            .output()?;
        if !output.status.success() {
            return Ok(None);
        }
        let path = String::from_utf8_lossy(&output.stdout).trim().trim_end_matches('/').to_string();
        Ok((!path.is_empty()).then(|| PathBuf::from(path)))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(None)
    }
}

/// Looks up the common extensions in the background so Settings opens instantly.
pub fn prefetch() {
    std::thread::spawn(|| {
        for extension in COMMON {
            lookup(extension);
        }
    });
}

#[cfg(target_os = "macos")]
mod platform {
    use super::AppChoice;
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::TCFType as _;
    use core_foundation::url::{CFURL, CFURLRef};
    use std::path::Path;
    use std::process::Command;

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSCopyDefaultApplicationURLForURL(
            url: CFURLRef,
            roles: u32,
            error: *mut *const std::ffi::c_void,
        ) -> CFURLRef;
        fn LSCopyApplicationURLsForURL(url: CFURLRef, roles: u32) -> CFArrayRef;
    }

    /// kLSRolesAll
    const ROLES_ALL: u32 = u32::MAX;

    /// LaunchServices answers per file, so ask about an empty sample file.
    fn sample(extension: &str) -> Option<std::path::PathBuf> {
        let dir = std::env::temp_dir().join("file-force-default-apps");
        std::fs::create_dir_all(&dir).ok()?;
        let file = dir.join(format!("sample.{extension}"));
        if !file.exists() {
            std::fs::write(&file, b"").ok()?;
        }
        Some(file)
    }

    pub fn lookup(extension: &str) -> (Option<AppChoice>, Vec<AppChoice>) {
        let Some(file) = sample(extension) else {
            return (None, Vec::new());
        };
        let Some(url) = CFURL::from_path(&file, false) else {
            return (None, Vec::new());
        };
        // SAFETY: `url` is a valid CFURL; both calls follow the Copy rule, so
        // the returned objects are owned and wrapped to be released on drop.
        let default = unsafe {
            let app = LSCopyDefaultApplicationURLForURL(
                url.as_concrete_TypeRef(),
                ROLES_ALL,
                std::ptr::null_mut(),
            );
            (!app.is_null()).then(|| CFURL::wrap_under_create_rule(app))
        }
        .and_then(|url| url.to_path())
        .map(AppChoice::from_path);
        let mut all: Vec<AppChoice> = unsafe {
            let apps = LSCopyApplicationURLsForURL(url.as_concrete_TypeRef(), ROLES_ALL);
            if apps.is_null() {
                Vec::new()
            } else {
                CFArray::<CFURL>::wrap_under_create_rule(apps)
                    .iter()
                    .filter_map(|url| url.to_path())
                    .map(AppChoice::from_path)
                    .collect()
            }
        };
        all.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        all.dedup_by(|a, b| a.path == b.path);
        (default, all)
    }

    #[cfg_attr(test, allow(dead_code))]
    pub fn open_with(app: &Path, file: &Path) -> std::io::Result<()> {
        status(Command::new("/usr/bin/open").arg("-a").arg(app).arg(file))
    }

    #[cfg_attr(test, allow(dead_code))]
    pub fn open_default(file: &Path) -> std::io::Result<()> {
        status(Command::new("/usr/bin/open").arg(file))
    }

    fn status(command: &mut Command) -> std::io::Result<()> {
        let status = command.status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!("open exited with {status}")))
        }
    }
}

/// Without LaunchServices the system default is unknown here; overrides still work.
#[cfg(not(target_os = "macos"))]
mod platform {
    use super::AppChoice;
    use std::path::Path;
    use std::process::Command;

    pub fn lookup(_: &str) -> (Option<AppChoice>, Vec<AppChoice>) {
        (None, Vec::new())
    }

    #[cfg_attr(test, allow(dead_code))]
    pub fn open_with(app: &Path, file: &Path) -> std::io::Result<()> {
        Command::new(app).arg(file).spawn().map(drop)
    }

    #[cfg_attr(test, allow(dead_code))]
    pub fn open_default(file: &Path) -> std::io::Result<()> {
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("cmd");
            command.args(["/C", "start", ""]);
            command
        };
        #[cfg(not(windows))]
        let mut command = Command::new("xdg-open");
        command.arg(file).spawn().map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_are_normalized() {
        assert_eq!(normalize_extension(".DOCX "), "docx");
        assert_eq!(extension_of(Path::new("/a/Report.PDF")).as_deref(), Some("pdf"));
        assert_eq!(AppChoice::from_path("/Applications/LibreOffice.app").name, "LibreOffice");
    }

    #[test]
    fn overrides_apply_only_to_existing_apps() {
        let existing = std::env::temp_dir();
        let overrides = BTreeMap::from([
            ("docx".to_string(), existing.display().to_string()),
            ("xlsx".to_string(), "/Applications/Missing Thing.app".to_string()),
        ]);
        assert!(override_for(Path::new("a.DOCX"), &overrides).is_some());
        assert_eq!(override_for(Path::new("a.xlsx"), &overrides), None, "missing app falls back");
        assert_eq!(override_for(Path::new("a.pdf"), &overrides), None);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn reads_system_defaults_from_launch_services() {
        // Every Mac opens .txt with something, usually TextEdit.
        let default = system_default("txt").expect("txt has a default app");
        assert!(default.path.extension().is_some_and(|extension| extension == "app"), "{default:?}");
        let all = candidates("txt");
        assert!(all.iter().any(|app| app.path == default.path), "default is among candidates");
        println!("txt → {} ({} candidates)", default.name, all.len());
    }
}
