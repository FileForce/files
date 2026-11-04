//! Cache space for extensions: each gets its own folder, which the app lists
//! in Settings → Storage (by extension, with a Clear button) and keeps under
//! the user's cache limit, least recently used files first. Put only what can
//! be fetched or made again here; settings and the user's data belong in
//! [`crate::Host::config_dir`].

use std::io;
use std::path::{Path, PathBuf};

/// How much an extension's cache takes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    pub files: usize,
    pub bytes: u64,
}

/// An id as a folder name: letters, digits, `-`, `_` and `.` stay.
pub fn folder_name(extension: &str) -> String {
    extension.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect()
}

/// Where every extension's cache folder lives.
pub fn root() -> io::Result<PathBuf> {
    crate::host().cache_dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "This host keeps no cache"))
}

/// The extension's own cache folder, made if missing.
pub fn cache_dir(extension: &str) -> io::Result<PathBuf> {
    let dir = root()?.join(folder_name(extension));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// A file in the cache folder (its parent folders made).
pub fn cache_file(extension: &str, name: &str) -> io::Result<PathBuf> {
    let path = cache_dir(extension)?.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(path)
}

/// Marks a cached file as just used, so the limit keeps it longest.
pub fn touch(path: &Path) {
    if let Ok(file) = std::fs::File::options().append(true).open(path) {
        let _ = file.set_modified(std::time::SystemTime::now());
    }
}

/// Every file under `dir`, with its size.
pub fn walk(dir: &Path) -> Vec<(PathBuf, std::fs::Metadata)> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            match entry.metadata() {
                Ok(metadata) if metadata.is_dir() => stack.push(entry.path()),
                Ok(metadata) => files.push((entry.path(), metadata)),
                Err(_) => {}
            }
        }
    }
    files
}

/// What the extension's cache takes now.
pub fn usage(extension: &str) -> Usage {
    let Ok(root) = root() else { return Usage::default() };
    let files = walk(&root.join(folder_name(extension)));
    Usage { files: files.len(), bytes: files.iter().map(|(_, metadata)| metadata.len()).sum() }
}

/// Empties the extension's cache folder; returns the bytes freed.
pub fn clear(extension: &str) -> io::Result<u64> {
    let dir = root()?.join(folder_name(extension));
    let freed = usage(extension).bytes;
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_extension_gets_its_own_folder_measured_and_cleared() {
        assert_eq!(folder_name("acme/../x y"), "acme_.._x_y");
        let file = cache_file("storage-test", "covers/a.bin").unwrap();
        std::fs::write(&file, [0u8; 300]).unwrap();
        assert_eq!(usage("storage-test"), Usage { files: 1, bytes: 300 });
        assert_eq!(clear("storage-test").unwrap(), 300);
        assert_eq!(usage("storage-test"), Usage::default());
    }
}
