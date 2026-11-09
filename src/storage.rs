//! Settings → Storage: what Files keeps on disk, by kind (thumbnails,
//! extensions' caches such as Music's tracks and covers, sounds), cleared per kind and held under a limit. Past
//! the limit the files used least recently go first. Extensions get their
//! own folders through `files_sdk::storage`, listed one by one.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// One kind of cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Thumbnails,
    /// Fork folders: the user's own work in progress, never cleared here.
    Forks,
    /// Global search's list of files, in memory.
    SearchIndex,
    /// Every extension's `files_sdk::storage` folder together.
    Extensions,
    Sounds,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Thumbnails, Kind::Extensions, Kind::Forks, Kind::SearchIndex, Kind::Sounds];

    pub fn label(self, language: crate::settings::Language) -> &'static str {
        use crate::settings::Language::Russian;
        match (self, language) {
            (Kind::Thumbnails, Russian) => "Миниатюры",
            (Kind::Thumbnails, _) => "Thumbnails",
            (Kind::Forks, Russian) => "Форки",
            (Kind::Forks, _) => "Forks",
            (Kind::SearchIndex, Russian) => "Индекс поиска",
            (Kind::SearchIndex, _) => "Search index",
            (Kind::Extensions, Russian) => "Расширения",
            (Kind::Extensions, _) => "Extensions",
            (Kind::Sounds, Russian) => "Звуки",
            (Kind::Sounds, _) => "Sounds",
        }
    }

    pub fn note(self, language: crate::settings::Language) -> &'static str {
        use crate::settings::Language::Russian;
        match (self, language) {
            (Kind::Thumbnails, Russian) => "Превью HEIC, PDF, видео и документов",
            (Kind::Thumbnails, _) => "Previews of HEIC, PDF, video and documents",
            (Kind::Forks, Russian) => "Копии папок с изменениями; удаляются в панели Forks",
            (Kind::Forks, _) => "Folder copies with changes; discard them in Forks",
            (Kind::SearchIndex, Russian) => "Список файлов для ⌘K, в памяти; соберётся заново",
            (Kind::SearchIndex, _) => "Files listed for ⌘K, in memory; rebuilt when needed",
            (Kind::Extensions, Russian) => "Музыка, SSH и другие — по каждому ниже",
            (Kind::Extensions, _) => "Music, SSH and others: each one below",
            (Kind::Sounds, Russian) => "Звуки уведомлений и теста колонок",
            (Kind::Sounds, _) => "Notification and speaker test sounds",
        }
    }

    /// Where it lives.
    fn dir(self) -> Option<PathBuf> {
        match self {
            Kind::Thumbnails => crate::thumbnails::cache_dir(),
            Kind::Sounds => Some(crate::sound::cache_dir()),
            Kind::Extensions => Some(extensions_root()),
            Kind::Forks | Kind::SearchIndex => None,
        }
    }

    /// Forks hold the user's changes: shown, never deleted from here.
    pub fn clearable(self) -> bool {
        self != Kind::Forks
    }

    /// Sounds are made again on demand and are tiny: never counted against
    /// the limit.
    fn limited(self) -> bool {
        matches!(self, Kind::Thumbnails | Kind::Extensions)
    }
}

/// One cached file.
#[derive(Clone, Debug)]
struct Entry {
    path: PathBuf,
    bytes: u64,
    used: SystemTime,
}

fn entry(path: PathBuf, metadata: &std::fs::Metadata) -> Entry {
    Entry { path, bytes: metadata.len(), used: metadata.accessed().ok().max(metadata.modified().ok()).unwrap_or(SystemTime::UNIX_EPOCH) }
}

/// Files of a kind, directly in its folder (extensions: all the way down).
fn entries(kind: Kind) -> Vec<Entry> {
    let Some(dir) = kind.dir() else { return Vec::new() };
    if kind == Kind::Extensions {
        return files_sdk::storage::walk(&dir).into_iter().map(|(path, metadata)| entry(path, &metadata)).collect();
    }
    let Ok(read) = std::fs::read_dir(&dir) else { return Vec::new() };
    read.flatten()
        .filter_map(|item| {
            let metadata = item.metadata().ok()?;
            metadata.is_file().then(|| entry(item.path(), &metadata))
        })
        .collect()
}

/// How much one kind takes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub files: usize,
    pub bytes: u64,
}

/// Every kind, measured (walks folders: off the UI thread).
pub fn usage() -> Vec<(Kind, Usage)> {
    Kind::ALL
        .into_iter()
        .map(|kind| {
            let usage = match kind {
                Kind::Forks => files_ext_forks::list().iter().fold(Usage::default(), |sum, fork| {
                    let (files, bytes) = crate::resources::folder_size(&fork.dir);
                    Usage { files: sum.files + files, bytes: sum.bytes + bytes }
                }),
                Kind::SearchIndex => {
                    let (_, _, _, search) = crate::model::index_stats();
                    Usage { files: search, bytes: search as u64 * crate::resources::ENTRY_BYTES }
                }
                _ => {
                    let entries = entries(kind);
                    Usage { files: entries.len(), bytes: entries.iter().map(|entry| entry.bytes).sum() }
                }
            };
            (kind, usage)
        })
        .collect()
}

/// Where extensions' caches live: the app's cache folder (a scratch folder
/// in tests, so they never touch the user's).
pub fn extensions_root() -> PathBuf {
    if cfg!(test) {
        return std::env::temp_dir().join(format!("file-force-ext-cache-{}", std::process::id()));
    }
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("file-force").join("extensions")
}

/// One extension's cache, for its own row.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtensionUsage {
    /// The id it asked `files_sdk::storage` with (its folder name).
    pub id: String,
    pub name: String,
    pub usage: Usage,
}

/// Each extension with something cached, biggest first (walks folders).
pub fn extensions() -> Vec<ExtensionUsage> {
    let Ok(read) = std::fs::read_dir(extensions_root()) else { return Vec::new() };
    let mut list: Vec<ExtensionUsage> = read
        .flatten()
        .filter(|item| item.path().is_dir())
        .map(|item| {
            let id = item.file_name().to_string_lossy().into_owned();
            let name = crate::extensions::CATALOG
                .iter()
                .find(|manifest| files_sdk::storage::folder_name(manifest.id) == id)
                .map_or_else(|| id.clone(), |manifest| manifest.name.to_string());
            let usage = files_sdk::storage::usage(&id);
            ExtensionUsage { id, name, usage: Usage { files: usage.files, bytes: usage.bytes } }
        })
        .filter(|extension| extension.usage.files > 0)
        .collect();
    list.sort_by_key(|extension| std::cmp::Reverse(extension.usage.bytes));
    list
}

/// Empties one extension's cache.
pub fn clear_extension(id: &str) -> u64 {
    let freed = files_sdk::storage::clear(id).unwrap_or(0);
    files_sdk::log::record("storage", files_sdk::log::Kind::Info, format!("Cleared the cache of {id}: {}", crate::resources::size(freed)), None);
    freed
}

/// A file being written now: left alone.
fn in_progress(entry: &Entry) -> bool {
    entry.path.extension().is_some_and(|extension| extension == "partial")
        && entry.used.elapsed().is_ok_and(|age| age < Duration::from_secs(10 * 60))
}

/// Deletes a kind's files; returns how many bytes went.
pub fn clear(kind: Kind) -> u64 {
    match kind {
        Kind::Forks => return 0,
        Kind::SearchIndex => {
            let (_, _, _, search) = crate::model::index_stats();
            crate::model::clear_search_index();
            return search as u64 * crate::resources::ENTRY_BYTES;
        }
        _ => {}
    }
    let freed = entries(kind).into_iter().filter(|entry| !in_progress(entry)).filter(|entry| std::fs::remove_file(&entry.path).is_ok()).map(|entry| entry.bytes).sum();
    files_sdk::log::record("storage", files_sdk::log::Kind::Info, format!("Cleared {kind:?}: {}", crate::resources::size(freed)), None);
    freed
}

/// The limit in bytes, 0 for none. Set from Settings.
static LIMIT: AtomicU64 = AtomicU64::new(0);

pub fn set_limit_mb(megabytes: u32) {
    LIMIT.store(megabytes as u64 * 1024 * 1024, Ordering::Relaxed);
}

/// Least recently used first, until what is left fits `limit`. Returns the
/// bytes freed.
fn trim(mut all: Vec<Entry>, limit: u64) -> u64 {
    let mut total: u64 = all.iter().map(|entry| entry.bytes).sum();
    if limit == 0 || total <= limit {
        return 0;
    }
    all.sort_by_key(|entry| entry.used);
    let mut freed = 0;
    for entry in all {
        if total <= limit {
            break;
        }
        if !in_progress(&entry) && std::fs::remove_file(&entry.path).is_ok() {
            total -= entry.bytes;
            freed += entry.bytes;
        }
    }
    freed
}

/// Keeps the caches under the limit (off the UI thread).
pub fn enforce() -> u64 {
    crate::tasks::run("cache limit", "all caches", || Ok::<_, String>(enforce_now())).unwrap_or(0)
}

fn enforce_now() -> u64 {
    let all: Vec<Entry> = Kind::ALL.into_iter().filter(|kind| kind.limited()).flat_map(entries).collect();
    let freed = trim(all, LIMIT.load(Ordering::Relaxed));
    if freed > 0 {
        files_sdk::log::record("storage", files_sdk::log::Kind::Info, format!("Cache over the limit: freed {}", crate::resources::size(freed)), None);
    }
    freed
}

/// Runs `enforce` in the background now and every few minutes after.
pub fn watch() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            loop {
                enforce();
                std::thread::sleep(Duration::from_secs(5 * 60));
            }
        });
    });
}

/// Marks a cached file as just used, so the limit keeps it longest.
pub fn touch(path: &Path) {
    if let Ok(file) = std::fs::File::options().append(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oldest_files_go_first_until_under_the_limit() {
        let dir = std::env::temp_dir().join(format!("file-force-storage-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let now = SystemTime::now();
        let all: Vec<Entry> = (0..4)
            .map(|index| {
                let path = dir.join(format!("{index}.bin"));
                std::fs::write(&path, vec![0u8; 100]).unwrap();
                Entry { path, bytes: 100, used: now - Duration::from_secs(1000 - index * 100) }
            })
            .collect();
        assert_eq!(trim(all.clone(), 0), 0, "no limit, nothing goes");
        assert_eq!(trim(all.clone(), 250), 200);
        assert!(!dir.join("0.bin").exists() && !dir.join("1.bin").exists(), "the two oldest went");
        assert!(dir.join("2.bin").exists() && dir.join("3.bin").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
