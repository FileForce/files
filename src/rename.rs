use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Renames an item in place without replacing a different existing item.
pub fn rename(path: &Path, name: &str) -> io::Result<PathBuf> {
    if let Some(reason) = crate::create::invalid_name(name) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, reason));
    }
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "This location cannot be renamed",
        )
    })?;
    let target = parent.join(name.trim());
    if target == path {
        return Ok(target);
    }
    if target.exists() {
        let same_item = fs::canonicalize(path)
            .ok()
            .zip(fs::canonicalize(&target).ok())
            .is_some_and(|(source, target)| source == target);
        if !same_item {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("\"{}\" already exists here", name.trim()),
            ));
        }
    }
    fs::rename(path, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_files_and_folders_without_overwriting() {
        let dir = std::env::temp_dir().join(format!("file-force-rename-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("old folder")).unwrap();
        fs::write(dir.join("old.txt"), "keep").unwrap();
        fs::write(dir.join("existing.txt"), "other").unwrap();

        let file = rename(&dir.join("old.txt"), "new.txt").unwrap();
        assert_eq!(fs::read_to_string(file).unwrap(), "keep");
        assert!(!dir.join("old.txt").exists());
        let folder = rename(&dir.join("old folder"), "new folder").unwrap();
        assert!(folder.is_dir());
        assert!(rename(&folder, "new folder").is_ok());
        assert!(rename(&dir.join("existing.txt"), "bad/name").is_err());
        let error = rename(&dir.join("new.txt"), "existing.txt").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            fs::read_to_string(dir.join("existing.txt")).unwrap(),
            "other"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
