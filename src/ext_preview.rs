//! The Preview extension's switch, and previews other extensions make
//! through `files_sdk::preview` — asked once per file version, off the UI
//! thread, and only while the extension may read files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use files_sdk::preview::PreviewContent;

/// Index of Preview in the catalog.
fn index() -> usize {
    crate::extensions::CATALOG.iter().position(|manifest| manifest.id == "preview").unwrap_or(usize::MAX)
}

/// Quick Look and the preview panel look inside files: Preview is on and
/// keeps its permission to read files.
pub fn enabled(settings: &crate::settings::AppSettings) -> bool {
    crate::licensing::active(settings, index()) && files_sdk::permissions::check("preview", files_sdk::Permission::ReadFiles).is_ok()
}

static READY: AtomicU64 = AtomicU64::new(0);

pub fn generation() -> u64 {
    READY.load(Ordering::Relaxed)
}

type Key = (PathBuf, Option<std::time::SystemTime>);

fn cache() -> &'static Mutex<HashMap<Key, Option<(String, PreviewContent)>>> {
    static CACHE: OnceLock<Mutex<HashMap<Key, Option<(String, PreviewContent)>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// An extension's preview of `path` and the extension's name: `None` while
/// it is made, or when no extension (allowed to read files) knows the file.
pub fn preview(path: &Path) -> Option<(String, PreviewContent)> {
    let extensions: Vec<_> = crate::views::extension_ui::registry()
        .into_iter()
        .filter(|extension| extension.preview().is_some_and(|provider| provider.handles(path)))
        .filter(|extension| files_sdk::permissions::check(extension.manifest().id, files_sdk::Permission::ReadFiles).is_ok())
        .collect();
    let extension = *extensions.first()?;
    let key = (path.to_path_buf(), std::fs::metadata(path).ok()?.modified().ok());
    let mut known = cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(done) = known.get(&key) {
        return done.clone();
    }
    known.insert(key.clone(), None);
    drop(known);
    let name = extension.manifest().name.to_string();
    let job = crate::tasks::queue("extension preview", format!("{name}: {}", path.display()));
    std::thread::spawn(move || {
        crate::tasks::start(job);
        let made = extension.preview().map(|provider| provider.preview(&key.0));
        let result = match made {
            Some(Ok(content)) => {
                cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(key, Some((name, content)));
                Ok(())
            }
            Some(Err(error)) => Err(error.to_string()),
            None => Ok(()),
        };
        crate::tasks::finish(job, result);
        READY.fetch_add(1, Ordering::Relaxed);
    });
    None
}
