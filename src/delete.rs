//! Deleting files: protected-path rules and friendly error reasons.

use std::io;
use std::path::{Component, Path, PathBuf};

/// Built-in blacklist for the current OS. Pattern syntax:
/// - `~` expands to the home folder;
/// - a plain path protects that exact item (its contents stay deletable);
/// - a trailing `/**` also protects everything inside;
/// - `*` matches any run of characters within one path component.
///
/// Any folder that *contains* a protected item is protected as well, so
/// deleting `/` or `/Users` can never take the home folder with it.
pub fn default_protected() -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    {
        &[
            "/",
            "/System/**",
            "/bin/**",
            "/sbin/**",
            "/usr/bin/**",
            "/usr/lib/**",
            "/usr/libexec/**",
            "/usr/sbin/**",
            "/private/**",
            "/etc",
            "/var",
            "/tmp",
            "/Library",
            "/Applications",
            "/Users",
            "/Volumes",
            "~",
            "~/Library",
            "~/Library/Keychains/**",
            "~/Desktop",
            "~/Documents",
            "~/Downloads",
            "~/.ssh/**",
            "~/.gnupg/**",
        ]
    }
    #[cfg(target_os = "windows")]
    {
        &[
            "C:/",
            "C:/Windows/**",
            "C:/Program Files",
            "C:/Program Files (x86)",
            "C:/ProgramData",
            "C:/Users",
            "C:/pagefile.sys",
            "C:/hiberfil.sys",
            "C:/swapfile.sys",
            "~",
            "~/AppData",
            "~/Desktop",
            "~/Documents",
            "~/Downloads",
            "~/.ssh/**",
        ]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        &[
            "/",
            "/bin/**",
            "/boot/**",
            "/dev/**",
            "/etc/**",
            "/lib/**",
            "/lib32/**",
            "/lib64/**",
            "/proc/**",
            "/run/**",
            "/sbin/**",
            "/sys/**",
            "/usr/**",
            "/var/lib/**",
            "/home",
            "/root",
            "~",
            "~/.ssh/**",
            "~/.gnupg/**",
        ]
    }
    #[cfg(not(any(unix, windows)))]
    {
        &["/", "~"]
    }
}

struct Pattern {
    parts: Vec<String>,
    recursive: bool,
}

fn expand(pattern: &str, home: &Path) -> Option<Pattern> {
    let pattern = pattern.trim();
    if pattern.is_empty() || pattern.starts_with('#') {
        return None;
    }
    let (pattern, recursive) = match pattern.strip_suffix("/**") {
        Some(rest) => (if rest.is_empty() { "/" } else { rest }, true),
        None => (pattern, false),
    };
    let path = match pattern.strip_prefix('~') {
        Some(rest) => home.join(rest.trim_start_matches(['/', '\\'])),
        None => PathBuf::from(pattern),
    };
    Some(Pattern {
        parts: parts(&path),
        recursive,
    })
}

/// Normalized components; case-insensitive on Windows and macOS (default APFS).
fn parts(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::CurDir => None,
            Component::RootDir => Some("/".to_string()),
            other => {
                let text = other.as_os_str().to_string_lossy().into_owned();
                Some(if cfg!(any(windows, target_os = "macos")) {
                    text.to_lowercase()
                } else {
                    text
                })
            }
        })
        .collect()
}

fn glob(pattern: &str, text: &str) -> bool {
    let Some((first, rest)) = pattern.split_once('*') else {
        return pattern == text;
    };
    let Some(mut remaining) = text.strip_prefix(first) else {
        return false;
    };
    let pieces: Vec<_> = rest.split('*').collect();
    for (index, piece) in pieces.iter().enumerate() {
        if index + 1 == pieces.len() {
            return remaining.ends_with(piece);
        }
        match remaining.find(piece) {
            Some(position) => remaining = &remaining[position + piece.len()..],
            None => return false,
        }
    }
    true
}

fn prefix_matches(pattern: &[String], path: &[String]) -> bool {
    pattern.len() <= path.len()
        && pattern
            .iter()
            .zip(path)
            .all(|(pattern, part)| glob(pattern, part))
}

/// The pattern that protects `path`, if any.
pub fn protection<'a>(path: &Path, patterns: &'a [String], home: &Path) -> Option<&'a str> {
    let path = parts(path);
    patterns.iter().map(String::as_str).find(|raw| {
        let Some(pattern) = expand(raw, home) else {
            return false;
        };
        let exact = pattern.parts.len() == path.len() && prefix_matches(&pattern.parts, &path);
        let inside = pattern.recursive && prefix_matches(&pattern.parts, &path);
        // Deleting a folder would also delete the protected item within it.
        let contains = path.len() < pattern.parts.len() && prefix_matches(&path, &pattern.parts);
        exact || inside || contains
    })
}

/// Why a delete failed, phrased for the user; `busy` errors are retryable.
pub struct DeleteFailure {
    pub message: String,
    pub busy: bool,
}

pub fn describe(error: &io::Error) -> DeleteFailure {
    let raw = error.raw_os_error();
    // Windows: ERROR_SHARING_VIOLATION (32), ERROR_LOCK_VIOLATION (33).
    // Unix: EBUSY (16), ETXTBSY (26).
    let busy = matches!(error.kind(), io::ErrorKind::ResourceBusy | io::ErrorKind::ExecutableFileBusy)
        || if cfg!(windows) {
            matches!(raw, Some(32 | 33))
        } else {
            matches!(raw, Some(16 | 26))
        };
    let message = if busy {
        "The file is in use by another program. Close it there and try again.".to_string()
    } else if error.kind() == io::ErrorKind::PermissionDenied {
        "Permission denied. The file or its folder is read-only or owned by another user.".to_string()
    } else if error.kind() == io::ErrorKind::NotFound {
        "The file no longer exists.".to_string()
    } else {
        error.to_string()
    };
    DeleteFailure { message, busy }
}

/// Deletes a batch. With the trash this is a single OS operation, so Finder
/// plays its sound once instead of once per item. On failure returns how many
/// of `paths` (in order) are already gone, so a retry can continue from there.
pub fn delete_many(paths: &[&Path], use_trash: bool) -> Result<(), (usize, io::Error)> {
    if use_trash {
        return trash::delete_all(paths).map_err(|error| {
            // The OS reports one error for the whole batch; count what it removed.
            let gone = paths
                .iter()
                .take_while(|path| std::fs::symlink_metadata(path).is_err())
                .count();
            (gone, trash_error(error))
        });
    }
    for (index, path) in paths.iter().enumerate() {
        delete(path, false).map_err(|error| (index, error))?;
    }
    Ok(())
}

fn trash_error(error: trash::Error) -> io::Error {
    match error {
        #[cfg(windows)]
        trash::Error::Os { code, description } => {
            io::Error::new(io::Error::from_raw_os_error(code).kind(), description)
        }
        other => io::Error::other(other.to_string()),
    }
}

/// Moves to Trash or deletes permanently. Folders are removed recursively.
pub fn delete(path: &Path, use_trash: bool) -> io::Result<()> {
    if use_trash {
        return trash::delete(path).map_err(trash_error);
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// Permanently deletes everything in the trash.
pub fn empty_trash() -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        // Finder empties the trash of every volume and needs no Full Disk Access
        // (macOS asks once for permission to control Finder).
        let output = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", "tell application \"Finder\" to empty trash"])
            .output()?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(if stderr.contains("-1743") {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "allow controlling Finder in System Settings → Privacy & Security → Automation",
            )
        } else {
            io::Error::other(stderr.trim().to_string())
        });
    }
    #[cfg(windows)]
    {
        let items = trash::os_limited::list().map_err(|error| io::Error::other(error.to_string()))?;
        return trash::os_limited::purge_all(items).map_err(|error| io::Error::other(error.to_string()));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let Some(files) = crate::model::trash_dir() else {
            return Ok(());
        };
        // freedesktop trash: `files/` holds items, `info/` their metadata.
        for dir in [files.clone(), files.with_file_name("info")] {
            let Ok(children) = std::fs::read_dir(&dir) else {
                continue;
            };
            for child in children.filter_map(Result::ok) {
                delete(&child.path(), false)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Vec<String> {
        default_protected().iter().map(|pattern| pattern.to_string()).collect()
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn defaults_protect_system_home_and_their_ancestors() {
        let home = Path::new("/Users/me");
        let patterns = defaults();
        let protected = |path: &str| protection(Path::new(path), &patterns, home).is_some();
        assert!(protected("/"));
        assert!(protected("/System/Library/CoreServices"));
        assert!(protected("/usr/bin/ls"));
        assert!(protected("/Users"), "contains the home folder");
        assert!(protected("/Users/me"));
        assert!(protected("/users/ME/documents"), "APFS is case-insensitive");
        assert!(protected("/Users/me/.ssh/id_ed25519"));
        assert!(!protected("/Users/me/Documents/report.pdf"), "user files stay deletable");
        assert!(!protected("/Users/me/Downloads/old.zip"));
        assert!(!protected("/usr/local/bin/tool"));
    }

    #[test]
    fn custom_patterns_support_home_and_wildcards() {
        let home = Path::new("/home/me");
        let patterns = vec!["~/keep-*.txt".to_string(), "/data/vault/**".to_string()];
        let protected = |path: &str| protection(Path::new(path), &patterns, home).is_some();
        assert!(protected("/home/me/keep-this.txt"));
        assert!(!protected("/home/me/drop-this.txt"));
        assert!(protected("/data/vault/a/b"));
        assert!(protected("/data"), "ancestor of a protected folder");
        assert!(!protected("/data/other"));
        assert_eq!(protection(Path::new("/data/vault/x"), &patterns, home), Some("/data/vault/**"));
    }

    #[test]
    fn glob_matches_within_a_component() {
        assert!(glob("id_*", "id_rsa"));
        assert!(glob("*.sys", "pagefile.sys"));
        assert!(glob("a*b*c", "axxbyyc"));
        assert!(!glob("a*b*c", "axxbyy"));
    }

    #[test]
    fn busy_errors_are_retryable() {
        let busy = io::Error::from_raw_os_error(if cfg!(windows) { 32 } else { 16 });
        assert!(describe(&busy).busy);
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert!(!describe(&denied).busy);
        assert!(describe(&denied).message.contains("Permission"));
    }

    #[test]
    fn permanent_batch_reports_progress_on_failure() {
        let root = std::env::temp_dir().join(format!("file-force-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a"), "a").unwrap();
        std::fs::write(root.join("c"), "c").unwrap();
        let (a, missing, c) = (root.join("a"), root.join("missing"), root.join("c"));
        let (gone, error) = delete_many(&[&a, &missing, &c], false).unwrap_err();
        assert_eq!(gone, 1, "a was deleted before the failure");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(c.exists(), "stops at the first failure");
        delete_many(&[&c], false).unwrap();
        assert!(!c.exists());
    }

    #[test]
    fn permanent_delete_removes_files_and_folders() {
        let root = std::env::temp_dir().join(format!("file-force-delete-{}", std::process::id()));
        std::fs::create_dir_all(root.join("dir/inner")).unwrap();
        std::fs::write(root.join("dir/inner/a.txt"), "a").unwrap();
        std::fs::write(root.join("file.txt"), "b").unwrap();
        delete(&root.join("file.txt"), false).unwrap();
        delete(&root.join("dir"), false).unwrap();
        assert!(!root.join("file.txt").exists());
        assert!(!root.join("dir").exists());
        assert_eq!(delete(&root.join("file.txt"), false).unwrap_err().kind(), io::ErrorKind::NotFound);
    }
}
