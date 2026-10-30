//! macOS Finder tags: read from each file's extended attributes, and found
//! system-wide through Spotlight. Other platforms have no Finder tags, so the
//! feature (and its sidebar section) is macOS-only.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use unicode_normalization::UnicodeNormalization as _;

/// Finder's eight label colors, as stored after the `\n` in a tag.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum TagColor {
    #[default]
    None,
    Gray,
    Green,
    Purple,
    Blue,
    Yellow,
    Red,
    Orange,
}

impl TagColor {
    fn from_index(index: u8) -> Self {
        match index {
            1 => Self::Gray,
            2 => Self::Green,
            3 => Self::Purple,
            4 => Self::Blue,
            5 => Self::Yellow,
            6 => Self::Red,
            7 => Self::Orange,
            _ => Self::None,
        }
    }

    /// Finder's own swatches, so tags look the same as in Finder.
    pub fn hsla(self) -> Option<gpui_kit::Hsla> {
        let rgb = match self {
            Self::None => return None,
            Self::Gray => 0x8e8e93,
            Self::Green => 0x34c759,
            Self::Purple => 0xaf52de,
            Self::Blue => 0x007aff,
            Self::Yellow => 0xffcc00,
            Self::Red => 0xff3b30,
            Self::Orange => 0xff9500,
        };
        Some(gpui_kit::rgb(rgb).into())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinderTag {
    pub name: String,
    pub color: TagColor,
}

/// Parses one stored tag, `"Name\n2"`; names are NFC so typed text matches.
fn parse(raw: &str) -> FinderTag {
    let (name, color) = match raw.rsplit_once('\n') {
        Some((name, color)) => (name, color.trim().parse().unwrap_or(0)),
        None => (raw, 0),
    };
    #[cfg(target_os = "macos")]
    let name: String = name.nfc().collect();
    #[cfg(not(target_os = "macos"))]
    let name = name.to_string();
    FinderTag {
        name,
        color: TagColor::from_index(color),
    }
}

/// The Finder tags on `path`; empty when it has none or off macOS.
pub fn read(path: &Path) -> Vec<FinderTag> {
    #[cfg(target_os = "macos")]
    {
        const ATTRIBUTE: &str = "com.apple.metadata:_kMDItemUserTags";
        let Ok(Some(bytes)) = xattr::get(path, ATTRIBUTE) else {
            return Vec::new();
        };
        let tags: Vec<FinderTag> = plist::from_bytes::<Vec<String>>(&bytes)
            .map(|raw| raw.iter().map(|tag| parse(tag)).collect())
            .unwrap_or_default();
        remember_colors(&tags);
        tags
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Vec::new()
    }
}

/// A tag with how many indexed files carry it.
#[derive(Clone, Debug)]
pub struct TagSummary {
    pub name: String,
    pub color: TagColor,
    pub count: usize,
}

struct Index {
    loaded: Option<Instant>,
    loading: bool,
    /// Loaded by walking the home folder (Spotlight gave nothing).
    walked: bool,
    files: BTreeMap<PathBuf, Vec<FinderTag>>,
}

fn index() -> &'static Mutex<Index> {
    static INDEX: OnceLock<Mutex<Index>> = OnceLock::new();
    INDEX.get_or_init(|| {
        Mutex::new(Index {
            loaded: None,
            loading: false,
            walked: false,
            files: BTreeMap::new(),
        })
    })
}

fn colors() -> &'static Mutex<HashMap<String, TagColor>> {
    static COLORS: OnceLock<Mutex<HashMap<String, TagColor>>> = OnceLock::new();
    COLORS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn remember_colors(tags: &[FinderTag]) {
    let mut colors = colors().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    for tag in tags {
        colors.insert(tag.name.clone(), tag.color);
    }
}

/// The Finder color of a tag name seen anywhere so far.
pub fn color_of(name: &str) -> TagColor {
    colors()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(name)
        .copied()
        .unwrap_or_default()
}

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Bumped when the Spotlight import finishes, so the sidebar repaints.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

const REFRESH: Duration = Duration::from_secs(60);
const MAX_FILES: usize = 20_000;
const WALK_REFRESH: Duration = Duration::from_secs(600);
/// Files and folders looked at by one walk.
const WALK_BUDGET: usize = 250_000;

/// Folders that never hold tagged user files but can hold millions of entries.
fn skipped(name: &str) -> bool {
    name.starts_with('.')
        || matches!(name, "Library" | "node_modules" | "target" | "Pods" | "DerivedData" | "build" | "dist" | "vendor")
}

/// Tagged files under `root`, read from their extended attributes; looks at
/// no more than `budget` entries.
fn walk(root: &Path, budget: usize) -> BTreeMap<PathBuf, Vec<FinderTag>> {
    let mut found = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0;
    while let Some(folder) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > budget || found.len() >= MAX_FILES {
                return found;
            }
            let name = entry.file_name();
            if skipped(&name.to_string_lossy()) {
                continue;
            }
            let path = entry.path();
            let tags = read(&path);
            if !tags.is_empty() {
                found.insert(path.clone(), tags);
            }
            // Links are not followed, so a walk never loops.
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                stack.push(path);
            }
        }
    }
    found
}

/// Starts (or refreshes) the Spotlight import in the background.
fn ensure_loaded() {
    if !cfg!(target_os = "macos") {
        return;
    }
    {
        let mut index = index().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // A walk of the home folder is costly: repeat it less often.
        let refresh = if index.walked { WALK_REFRESH } else { REFRESH };
        let fresh = index.loaded.is_some_and(|loaded| loaded.elapsed() < refresh);
        if fresh || index.loading {
            return;
        }
        index.loading = true;
    }
    std::thread::Builder::new()
        .name("finder-tags".into())
        .spawn(|| {
            let mut files = import();
            // Spotlight can be off or broken (its index turns read-only on a
            // full disk); then read the tags straight from the files.
            let walked = files.is_empty();
            if walked {
                if let Some(home) = dirs::home_dir() {
                    files = walk(&home, WALK_BUDGET);
                }
            }
            let mut index = index().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            index.walked = walked;
            index.files = files;
            index.loaded = Some(Instant::now());
            index.loading = false;
            GENERATION.fetch_add(1, Ordering::Relaxed);
        })
        .expect("failed to spawn Finder tag import");
}

/// Every tagged file Spotlight knows about, with its tags.
fn import() -> BTreeMap<PathBuf, Vec<FinderTag>> {
    let Ok(output) = std::process::Command::new("/usr/bin/mdfind")
        .arg("kMDItemUserTags == '*'")
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return BTreeMap::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .take(MAX_FILES)
        .map(PathBuf::from)
        .filter_map(|path| {
            let tags = read(&path);
            (!tags.is_empty()).then_some((path, tags))
        })
        .collect()
}

/// False until the first Spotlight import has finished.
pub fn is_loaded() -> bool {
    ensure_loaded();
    index().lock().unwrap_or_else(|poisoned| poisoned.into_inner()).loaded.is_some()
}

/// All Finder tags in use, most used first. Empty until the import finishes.
pub fn all() -> Vec<TagSummary> {
    ensure_loaded();
    let index = index().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut summary: BTreeMap<String, TagSummary> = BTreeMap::new();
    for tag in index.files.values().flatten() {
        summary
            .entry(tag.name.clone())
            .or_insert_with(|| TagSummary {
                name: tag.name.clone(),
                color: tag.color,
                count: 0,
            })
            .count += 1;
    }
    let mut tags: Vec<_> = summary.into_values().collect();
    tags.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    tags
}

/// Files carrying `name`, re-checked on disk so removed tags drop out.
pub fn files_with(name: &str) -> Vec<PathBuf> {
    ensure_loaded();
    let candidates: Vec<PathBuf> = index()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .files
        .iter()
        .filter(|(_, tags)| tags.iter().any(|tag| tag.name == name))
        .map(|(path, _)| path.clone())
        .collect();
    candidates
        .into_iter()
        .filter(|path| read(path).iter().any(|tag| tag.name == name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_name_and_finder_color() {
        assert_eq!(parse("Work\n4"), FinderTag { name: "Work".into(), color: TagColor::Blue });
        assert_eq!(parse("Plain"), FinderTag { name: "Plain".into(), color: TagColor::None });
        assert_eq!(parse("Odd\nx").color, TagColor::None);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn decomposed_names_are_normalized_like_typed_text() {
        // Finder stores "й" decomposed (и + combining breve).
        let stored = "Зелены\u{0438}\u{0306}\n2";
        let tag = parse(stored);
        assert_eq!(tag.name, "Зеленый");
        assert_eq!(tag.color, TagColor::Green);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn reads_tags_written_by_finder() {
        let dir = std::env::temp_dir().join(format!("file-force-tags-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tagged.txt");
        std::fs::write(&file, "x").unwrap();
        // The same binary plist Finder writes for two tags.
        let mut bytes = Vec::new();
        plist::to_writer_binary(&mut bytes, &vec!["Work\n4".to_string(), "Urgent\n6".to_string()]).unwrap();
        xattr::set(&file, "com.apple.metadata:_kMDItemUserTags", &bytes).unwrap();

        let tags = read(&file);
        assert_eq!(tags.iter().map(|tag| tag.name.as_str()).collect::<Vec<_>>(), ["Work", "Urgent"]);

        // Without Spotlight the tags are found by walking the folders,
        // skipping hidden and build folders.
        std::fs::create_dir_all(dir.join("deep/er")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        for hidden in [dir.join("deep/er/photo.png"), dir.join("node_modules/skip.js")] {
            std::fs::write(&hidden, "x").unwrap();
            xattr::set(&hidden, "com.apple.metadata:_kMDItemUserTags", &bytes).unwrap();
        }
        let walked = walk(&dir, 1000);
        let paths: Vec<_> = walked.keys().map(|path| path.strip_prefix(&dir).unwrap().to_path_buf()).collect();
        assert_eq!(paths, [PathBuf::from("deep/er/photo.png"), PathBuf::from("tagged.txt")]);
        assert!(walk(&dir, 1).len() <= 1, "the budget stops the walk");
        assert_eq!(tags[1].color, TagColor::Red);
        assert_eq!(color_of("Urgent"), TagColor::Red);
        assert!(read(&dir.join("missing")).is_empty());
    }

    #[test]
    #[ignore = "walks this computer's home folder"]
    fn walk_real_home() {
        let started = Instant::now();
        let found = walk(&dirs::home_dir().unwrap(), WALK_BUDGET);
        let mut names: Vec<_> = found.values().flatten().map(|tag| tag.name.clone()).collect();
        names.sort();
        names.dedup();
        println!("{} tagged files, tags {names:?}, in {:?}", found.len(), started.elapsed());
    }
}
