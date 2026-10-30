//! Copying and moving files: the app-wide clipboard (⌘C / ⌘X / ⌘V across
//! windows) and drag-and-drop between windows.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Copy,
    Move,
}

#[derive(Clone, Debug, Default)]
struct Clipboard {
    paths: Vec<PathBuf>,
    /// Cut items move on paste (and look dimmed until then).
    cut: bool,
}

static CLIPBOARD: Mutex<Clipboard> = Mutex::new(Clipboard { paths: Vec::new(), cut: false });
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Bumped when the clipboard or the disk changes here, so every window repaints.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

fn touch() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

fn clipboard() -> std::sync::MutexGuard<'static, Clipboard> {
    CLIPBOARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn set_clipboard(paths: Vec<PathBuf>, cut: bool) {
    *clipboard() = Clipboard { paths, cut };
    touch();
}

/// (paths, cut)
pub fn clipboard_contents() -> (Vec<PathBuf>, bool) {
    let clipboard = clipboard();
    (clipboard.paths.clone(), clipboard.cut)
}

pub fn clear_clipboard() {
    *clipboard() = Clipboard::default();
    touch();
}

/// Whether `path` is waiting to be moved by a paste (shown dimmed).
pub fn is_cut(path: &Path) -> bool {
    let clipboard = clipboard();
    clipboard.cut && clipboard.paths.iter().any(|cut| cut == path)
}

/// Finder's naming for a copy in the same folder: `a copy.txt`, `a copy 2.txt`.
fn copy_name(target_dir: &Path, name: &str) -> PathBuf {
    let path = Path::new(name);
    let (stem, extension) = match (path.file_stem(), path.extension()) {
        (Some(stem), Some(extension)) if !name.starts_with('.') => (
            stem.to_string_lossy().into_owned(),
            format!(".{}", extension.to_string_lossy()),
        ),
        _ => (name.to_string(), String::new()),
    };
    (1..)
        .map(|index| {
            let suffix = if index == 1 { " copy".to_string() } else { format!(" copy {index}") };
            target_dir.join(format!("{stem}{suffix}{extension}"))
        })
        .find(|candidate| std::fs::symlink_metadata(candidate).is_err())
        .expect("unbounded range always yields a free name")
}

/// Same file system: a move is a cheap rename (Finder moves on drag then).
pub fn same_volume(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        a.components().next() == b.components().next()
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Report {
    /// Where each item ended up.
    pub done: Vec<PathBuf>,
    /// (item, why it was left alone)
    pub skipped: Vec<(String, String)>,
}

fn copy_item(source: &Path, target: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(source)?.is_dir() {
        crate::forks::copy_tree(source, target)
    } else {
        std::fs::copy(source, target)?;
        if let Ok(modified) = std::fs::metadata(source).and_then(|metadata| metadata.modified()) {
            let _ = std::fs::File::options().write(true).open(target).and_then(|file| file.set_modified(modified));
        }
        Ok(())
    }
}

/// Copies or moves `sources` into `target_dir`. Nothing is ever overwritten:
/// a clash gets a free name. Moving a protected path is refused.
pub fn transfer(
    sources: &[PathBuf],
    target_dir: &Path,
    operation: Operation,
    protected: &[String],
    home: &Path,
) -> Report {
    let mut report = Report::default();
    for source in sources {
        let name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if std::fs::symlink_metadata(source).is_err() {
            report.skipped.push((name, "no longer exists".into()));
            continue;
        }
        if target_dir.starts_with(source) {
            report.skipped.push((name, "a folder cannot go inside itself".into()));
            continue;
        }
        let same_folder = source.parent() == Some(target_dir);
        if operation == Operation::Move {
            if same_folder {
                report.skipped.push((name, "already in this folder".into()));
                continue;
            }
            if let Some(rule) = crate::delete::protection(source, protected, home) {
                report.skipped.push((name, format!("protected by \"{rule}\"")));
                continue;
            }
        }
        let target = if same_folder {
            copy_name(target_dir, &name)
        } else {
            let direct = target_dir.join(&name);
            if std::fs::symlink_metadata(&direct).is_ok() {
                crate::archive::unique_path(&direct)
            } else {
                direct
            }
        };
        let result = match operation {
            Operation::Copy => copy_item(source, &target),
            Operation::Move => std::fs::rename(source, &target).or_else(|error| {
                // Across volumes a rename fails: copy, then remove the original.
                if error.raw_os_error() == Some(18) || error.kind() == io::ErrorKind::CrossesDevices {
                    copy_item(source, &target).and_then(|()| crate::delete::delete(source, false))
                } else {
                    Err(error)
                }
            }),
        };
        match result {
            Ok(()) => report.done.push(target),
            Err(error) => report.skipped.push((name, crate::delete::describe(&error).message)),
        }
    }
    crate::model::invalidate_listings();
    touch();
    report
}

/// Tests that touch the shared clipboard take this so they never interleave.
#[cfg(test)]
pub(crate) fn clipboard_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("file-force-transfer-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(a.join("folder/inner")).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("note.txt"), "note").unwrap();
        std::fs::write(a.join("folder/inner/deep.md"), "deep").unwrap();
        (a, b)
    }

    #[test]
    fn copies_files_and_folders_and_never_overwrites() {
        let (a, b) = dirs("copy");
        std::fs::write(b.join("note.txt"), "theirs").unwrap();
        let report = transfer(&[a.join("note.txt"), a.join("folder")], &b, Operation::Copy, &[], Path::new("/x"));
        assert_eq!(report.skipped, []);
        assert_eq!(std::fs::read_to_string(b.join("note.txt")).unwrap(), "theirs", "overwrote an existing file");
        assert_eq!(std::fs::read_to_string(b.join("note (1).txt")).unwrap(), "note");
        assert_eq!(std::fs::read_to_string(b.join("folder/inner/deep.md")).unwrap(), "deep");
        assert!(a.join("note.txt").exists(), "copy removed the original");
    }

    #[test]
    fn copying_into_the_same_folder_names_it_like_finder() {
        let (a, _) = dirs("dup");
        transfer(&[a.join("note.txt")], &a, Operation::Copy, &[], Path::new("/x"));
        transfer(&[a.join("note.txt")], &a, Operation::Copy, &[], Path::new("/x"));
        assert!(a.join("note copy.txt").exists());
        assert!(a.join("note copy 2.txt").exists());
    }

    #[test]
    fn moving_relocates_and_refuses_bad_targets() {
        let (a, b) = dirs("move");
        let report = transfer(&[a.join("note.txt"), a.join("folder")], &b, Operation::Move, &[], Path::new("/x"));
        assert_eq!(report.done.len(), 2);
        assert!(!a.join("note.txt").exists() && b.join("note.txt").exists());
        assert!(b.join("folder/inner/deep.md").exists());

        let inside = transfer(&[b.join("folder")], &b.join("folder/inner"), Operation::Move, &[], Path::new("/x"));
        assert_eq!(inside.skipped[0].1, "a folder cannot go inside itself");
        let protected = vec![b.join("note.txt").display().to_string()];
        let refused = transfer(&[b.join("note.txt")], &a, Operation::Move, &protected, Path::new("/x"));
        assert!(refused.skipped[0].1.starts_with("protected"), "{:?}", refused.skipped);
        assert!(b.join("note.txt").exists());
    }

    #[test]
    fn cut_items_are_tracked_until_cleared() {
        let _clipboard = clipboard_test_lock();
        let path = PathBuf::from("/tmp/some/cut/file");
        set_clipboard(vec![path.clone()], true);
        assert!(is_cut(&path));
        set_clipboard(vec![path.clone()], false);
        assert!(!is_cut(&path), "copied items are not dimmed");
        clear_clipboard();
        assert_eq!(clipboard_contents().0, Vec::<PathBuf>::new());
    }
}
