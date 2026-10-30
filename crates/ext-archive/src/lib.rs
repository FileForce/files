//! The Archives extension.

use std::io;
use std::path::Path;

use files_sdk::{Permission, ArchiveItem, ArchiveProvider, Command, ConflictPolicy, ExtractReport, Icon, Manifest};

pub mod compress;
pub mod extract;

pub use extract::{ArchiveKind, describe_error, extract_archive, find_conflicts, list_items, parent_path};

pub static MANIFEST: Manifest = Manifest {
    id: "archives",
    name: "Archives",
    version: env!("CARGO_PKG_VERSION"),
    source: "Built-in",
    summary: "Browse and extract .zip, .tar, .tar.gz, .tgz and .gz; create ZIP, 7z, TAR.GZ and RAR.",
    icon: Icon::Builtin("file-archive"),
    adds: &[
        "Double-click an archive to browse it",
        "Extract with conflict handling (keep both, overwrite, skip)",
        "Context menu → Compress…",
    ],
    permissions: &[Permission::ReadFiles, Permission::WriteFiles],
    menu: &[],
    sidebar: &[],
    menu_color: None,
    tier: files_sdk::Tier::Free,
    commands: &[
        Command { id: "archive.close", title: "Close archive window", mac_keys: &["escape", "cmd-w"], keys: &["escape", "ctrl-w"] },
        Command { id: "compress.close", title: "Close compress window", mac_keys: &["escape", "cmd-w"], keys: &["escape", "ctrl-w"] },
    ],
};

pub struct Archives;

pub static EXTENSION: Archives = Archives;

impl files_sdk::Extension for Archives {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn archive(&self) -> Option<&dyn ArchiveProvider> {
        Some(self)
    }
}

impl ArchiveProvider for Archives {
    fn handles(&self, path: &Path) -> bool {
        ArchiveKind::of(path).is_some()
    }

    fn list(&self, archive: &Path) -> io::Result<Vec<ArchiveItem>> {
        Ok(list_items(archive)?
            .into_iter()
            .map(|item| ArchiveItem {
                name: item.name,
                dir: item.dir,
                size: item.size,
                modified: item.modified.map(|time| time.and_utc().timestamp()),
            })
            .collect())
    }

    fn conflicts(&self, archive: &Path, target: &Path) -> io::Result<Vec<String>> {
        find_conflicts(archive, target)
    }

    fn extract(&self, archive: &Path, target: &Path, policy: ConflictPolicy) -> io::Result<ExtractReport> {
        extract_archive(archive, target, policy)
    }
}
