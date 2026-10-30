//! Creating new files and folders with names valid on every platform.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// Names Windows reserves for devices, with or without an extension.
const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Why `name` cannot be used, checked against the rules of all three OSes so
/// a name created here also survives copying to another system.
pub fn invalid_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Some("Enter a name".to_string());
    }
    if trimmed == "." || trimmed == ".." {
        return Some(format!("\"{trimmed}\" is reserved"));
    }
    if let Some(character) = name
        .chars()
        .find(|character| matches!(character, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || character.is_control())
    {
        return Some(if character.is_control() {
            "Names cannot contain control characters".to_string()
        } else {
            format!("Names cannot contain \"{character}\"")
        });
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Some("Names cannot end with a dot or a space".to_string());
    }
    let stem = trimmed.split('.').next().unwrap_or(trimmed).to_ascii_uppercase();
    if WINDOWS_RESERVED.contains(&stem.as_str()) {
        return Some(format!("\"{trimmed}\" is reserved on Windows"));
    }
    if name.len() > 255 {
        return Some("Name is longer than 255 bytes".to_string());
    }
    None
}

/// Creates `name` inside `dir`. Never overwrites: an existing item is an error.
pub fn create(dir: &Path, name: &str, folder: bool) -> io::Result<PathBuf> {
    if let Some(reason) = invalid_name(name) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, reason));
    }
    let path = dir.join(name.trim());
    if folder {
        fs::create_dir(&path)?;
    } else {
        // `create_new` fails atomically if something appeared meanwhile.
        OpenOptions::new().write(true).create_new(true).open(&path)?;
    }
    Ok(path)
}

/// What to pre-select when renaming or naming a new item (UTF-8 byte range):
/// a folder's whole name, but only a file's stem so the extension survives
/// typing. Dot-files like `.gitignore` have no extension to keep.
pub fn name_selection(name: &str, folder: bool) -> std::ops::Range<usize> {
    let end = if folder {
        name.len()
    } else {
        match name.rfind('.') {
            Some(dot) if dot > 0 => dot,
            _ => name.len(),
        }
    };
    0..end
}

/// A default name that is still free, e.g. `untitled folder 2`.
pub fn free_name(dir: &Path, folder: bool) -> String {
    let (stem, extension) = if folder { ("untitled folder", "") } else { ("untitled", ".txt") };
    (1..)
        .map(|index| {
            if index == 1 {
                format!("{stem}{extension}")
            } else {
                format!("{stem} {index}{extension}")
            }
        })
        .find(|name| !dir.join(name).exists())
        .expect("unbounded range always yields a free name")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_names_invalid_on_any_platform() {
        for name in ["", "  ", ".", "..", "a/b", "a\\b", "a:b", "what?", "con", "LPT1.txt", "dot.", "space "] {
            assert!(invalid_name(name).is_some(), "{name:?} should be rejected");
        }
        for name in ["notes.md", "Report 2026", ".gitignore", "console.log", "файл.txt"] {
            assert_eq!(invalid_name(name), None, "{name:?} should be accepted");
        }
    }

    #[test]
    fn selection_covers_folder_name_or_file_stem() {
        assert_eq!(name_selection("untitled folder", true), 0..15);
        assert_eq!(name_selection("my.photos", true), 0..9, "folders keep dots");
        assert_eq!(name_selection("untitled.txt", false), 0..8);
        assert_eq!(name_selection("archive.tar.gz", false), 0..11);
        assert_eq!(name_selection(".gitignore", false), 0..10);
        assert_eq!(name_selection("README", false), 0..6);
        assert_eq!(name_selection("файл.txt", false), 0.."файл".len(), "byte offsets");
    }

    #[test]
    fn creates_files_and_folders_without_overwriting() {
        let dir = std::env::temp_dir().join(format!("file-force-create-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        assert_eq!(free_name(&dir, true), "untitled folder");
        let folder = create(&dir, "untitled folder", true).unwrap();
        assert!(folder.is_dir());
        assert_eq!(free_name(&dir, true), "untitled folder 2");

        let file = create(&dir, "notes.txt", false).unwrap();
        fs::write(&file, "keep").unwrap();
        let error = create(&dir, "notes.txt", false).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&file).unwrap(), "keep");
        assert_eq!(create(&dir, "a/b", false).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
}
