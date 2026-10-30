//! File system helpers every extension needs.

use std::path::{Path, PathBuf};

/// First free `name (n).ext` next to `path`.
pub fn unique_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = path
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    (1..)
        .map(|index| path.with_file_name(format!("{stem} ({index}){extension}")))
        .find(|candidate| !candidate.exists())
        .expect("unbounded range always yields a free name")
}

/// A cloud placeholder whose content is not on this disk (iCloud, OneDrive…).
pub fn is_online_only(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt as _;
        const SF_DATALESS: u32 = 0x4000_0000;
        std::fs::metadata(path).is_ok_and(|metadata| metadata.st_flags() & SF_DATALESS != 0)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        // OneDrive "online-only": RECALL_ON_DATA_ACCESS or OFFLINE.
        const RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
        const OFFLINE: u32 = 0x0000_1000;
        std::fs::symlink_metadata(path)
            .is_ok_and(|metadata| metadata.file_attributes() & (RECALL_ON_DATA_ACCESS | OFFLINE) != 0)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = path;
        false
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unique_path_counts_up() {
        let dir = std::env::temp_dir().join(format!("files-sdk-unique-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "").unwrap();
        std::fs::write(dir.join("a (1).txt"), "").unwrap();
        assert_eq!(super::unique_path(&dir.join("a.txt")), dir.join("a (2).txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
