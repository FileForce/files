//! What the app offers to extensions (`files_sdk::Host`).

use std::io;
use std::path::{Path, PathBuf};

struct AppHost;

impl files_sdk::Host for AppHost {
    /// Extensions keep their files beside `settings.toml`.
    fn config_dir(&self) -> Option<PathBuf> {
        Some(crate::settings::settings_path()?.parent()?.to_path_buf())
    }

    /// Extensions' caches, shown and limited in Settings → Storage.
    fn cache_dir(&self) -> Option<PathBuf> {
        Some(crate::storage::extensions_root())
    }

    fn protection(&self, path: &Path, patterns: &[String], home: &Path) -> Option<String> {
        crate::delete::protection(path, patterns, home).map(str::to_string)
    }

    fn audio(&self, extension: &str, command: files_sdk::audio::Command) -> io::Result<()> {
        crate::sound::extension_audio(extension, command)
    }

    fn delete(&self, paths: &[&Path], use_trash: bool) -> Result<(), (usize, io::Error)> {
        crate::delete::delete_many(paths, use_trash)
    }

    fn describe_error(&self, error: &io::Error) -> String {
        crate::delete::describe(error).message
    }

    fn files_changed(&self) {
        crate::model::invalidate_listings();
    }
}

/// Installs the app as the extensions' host and registers what every
/// manifest asks for. Safe to call more than once.
pub fn install() {
    files_sdk::set_host(Box::new(AppHost));
    for manifest in crate::extensions::CATALOG {
        files_sdk::permissions::declare(manifest);
    }
}
