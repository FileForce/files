//! Forks: throwaway copies of a folder you can change freely, in the spirit
//! of `git worktree`, Btrfs snapshots and Nix. A fork is a real folder, so a
//! terminal, an editor or a build works inside it. It is made with a
//! copy-on-write clone where the file system supports one (APFS `clonefile`
//! on macOS, reflinks on Btrfs/XFS), so it is instant and takes no extra
//! space until files change; elsewhere it falls back to a plain copy.
//!
//! The original is never touched until the fork is applied: Apply copies
//! added and modified files back and removes what the fork removed (to the
//! Trash by default). Discard deletes the copy. Forks are listed in
//! `forks.toml` next to `settings.toml`, so they survive restarts.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use files_sdk::{Permission, Icon, Manifest, SnapshotProvider, host};

pub static MANIFEST: Manifest = Manifest {
    id: "forks",
    name: "Forks",
    version: env!("CARGO_PKG_VERSION"),
    source: "Built-in (experimental)",
    summary: "Throwaway forks of a folder: change anything, then apply or discard. Inspired by git worktree, Btrfs and Nix.",
    icon: Icon::Svg(ICON),
    adds: &[
        "Context menu → Fork folder / Fork this folder",
        "Sidebar → Forks (kept across restarts in forks.toml)",
        "Fork header → Changes, Apply, Discard",
    ],
    permissions: &[Permission::ReadFiles, Permission::WriteFiles, Permission::LaunchApps],
    menu: &[],
    sidebar: &[],
    menu_color: Some("purple"),
    tier: files_sdk::Tier::Free,
    commands: &[files_sdk::Command {
        id: "forks.fork",
        title: "Fork selected folder",
        mac_keys: &["cmd-n f f", "cmd-n cmd-f cmd-f"],
        keys: &["ctrl-n f f", "ctrl-n ctrl-f ctrl-f"],
    }],
};

/// Forks ships its own icon to prove custom SVGs work end to end.
pub const ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="7" height="6" rx="1.5"/><rect x="14" y="15" width="7" height="6" rx="1.5"/><rect x="3" y="15" width="7" height="6" rx="1.5"/><path d="M6.5 9v6"/><path d="M6.5 12h8a3 3 0 0 1 3 3"/></svg>"#;

pub struct Forks;

pub static EXTENSION: Forks = Forks;

impl files_sdk::Extension for Forks {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn snapshots(&self) -> Option<&dyn SnapshotProvider> {
        Some(self)
    }
}

impl SnapshotProvider for Forks {
    fn create(&self, source: &Path) -> io::Result<PathBuf> {
        create(source).map(|(_, dir, _)| dir)
    }

    fn snapshot_of(&self, path: &Path) -> Option<u32> {
        fork_of(path)
    }

    fn discard(&self, id: u32) -> Option<PathBuf> {
        discard(id)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Added { rel: String, folder: bool },
    Modified { rel: String },
    Removed { rel: String },
}

impl Change {
    pub fn rel(&self) -> &str {
        match self {
            Self::Added { rel, .. } | Self::Modified { rel } | Self::Removed { rel } => rel,
        }
    }
}

/// How a fork's files were made.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloneKind {
    /// Copy-on-write clone: instant, shares data with the original.
    CopyOnWrite,
    /// Plain copy: the file system has no copy-on-write support.
    Copy,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Fork {
    name: String,
    base: PathBuf,
    dir: PathBuf,
    /// When the fork was made; originals changed later are conflicts.
    created: SystemTime,
    kind: CloneKind,
}

#[derive(Clone, Debug)]
pub struct ForkSummary {
    pub id: u32,
    pub name: String,
    pub base: PathBuf,
    pub dir: PathBuf,
    pub changes: usize,
}

#[derive(Default)]
struct State {
    /// `None` once discarded or applied; ids are indexes and never reused.
    forks: Vec<Option<Fork>>,
    /// Last computed change list per fork, refreshed in the background.
    changes: HashMap<u32, (Instant, Vec<Change>)>,
    computing: std::collections::HashSet<u32>,
}

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Bumped when a background change scan finishes, so views repaint.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

fn state() -> MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE
        .get_or_init(|| {
            #[cfg_attr(test, allow(unused_mut))]
            let mut state = State::default();
            // Tests start empty; the app picks up the forks it saved last time.
            #[cfg(not(test))]
            if let Some(path) = store_path() {
                state.forks = load(&path)
                    .into_iter()
                    .filter(|fork| fork.dir.is_dir())
                    .map(Some)
                    .collect();
            }
            Mutex::new(state)
        })
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Store {
    #[serde(default)]
    forks: Vec<Fork>,
}

fn config_dir() -> Option<PathBuf> {
    host().config_dir()
}

/// `forks.toml` lives beside `settings.toml`.
fn store_path() -> Option<PathBuf> {
    Some(config_dir()?.join("forks.toml"))
}

/// Where fork folders are created.
pub fn storage_dir() -> Option<PathBuf> {
    Some(config_dir()?.join("forks"))
}

fn load(path: &Path) -> Vec<Fork> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|source| toml::from_str::<Store>(&source).ok())
        .unwrap_or_default()
        .forks
}

fn persist(state: &State) {
    let Some(path) = store_path() else {
        return;
    };
    let store = Store {
        forks: state.forks.iter().flatten().cloned().collect(),
    };
    let result = toml::to_string_pretty(&store)
        .map_err(io::Error::other)
        .and_then(|source| {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let temporary = path.with_extension("toml.tmp");
            std::fs::write(&temporary, source)?;
            std::fs::rename(&temporary, &path)
        });
    if let Err(error) = result {
        eprintln!("Could not save forks to {}: {error}", path.display());
    }
}

/// Clones `source` to `target` (which must not exist yet).
fn clone_tree(source: &Path, target: &Path) -> io::Result<CloneKind> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt as _;
        unsafe extern "C" {
            fn clonefile(src: *const std::ffi::c_char, dst: *const std::ffi::c_char, flags: u32) -> std::ffi::c_int;
        }
        let source_c = std::ffi::CString::new(source.as_os_str().as_bytes()).map_err(io::Error::other)?;
        let target_c = std::ffi::CString::new(target.as_os_str().as_bytes()).map_err(io::Error::other)?;
        // SAFETY: both are valid NUL-terminated paths; clonefile copies a whole
        // directory tree as copy-on-write clones on APFS.
        if unsafe { clonefile(source_c.as_ptr(), target_c.as_ptr(), 0) } == 0 {
            return Ok(CloneKind::CopyOnWrite);
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let status = std::process::Command::new("cp")
            .args(["-a", "--reflink=always"])
            .arg(source)
            .arg(target)
            .stderr(std::process::Stdio::null())
            .status();
        if status.is_ok_and(|status| status.success()) {
            return Ok(CloneKind::CopyOnWrite);
        }
        let _ = std::fs::remove_dir_all(target);
    }
    copy_tree(source, target)?;
    Ok(CloneKind::Copy)
}

/// A plain recursive copy that keeps modification times (so unchanged files
/// do not look modified) and recreates symlinks instead of following them.
pub fn copy_tree(source: &Path, target: &Path) -> io::Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&from, &to)?;
        } else if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(&from)?, &to)?;
            #[cfg(not(unix))]
            std::fs::copy(&from, &to).map(drop)?;
        } else {
            std::fs::copy(&from, &to)?;
            if let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) {
                let _ = std::fs::File::options().write(true).open(&to).and_then(|file| file.set_modified(modified));
            }
        }
    }
    Ok(())
}

/// Makes a fork of `base` and returns its id and folder.
pub fn create(base: &Path) -> io::Result<(u32, PathBuf, CloneKind)> {
    let storage = storage_dir().ok_or_else(|| io::Error::other("no settings folder"))?;
    let folder = base
        .file_name()
        .map_or_else(|| "root".to_string(), |name| name.to_string_lossy().into_owned());
    let (id, number) = {
        let state = state();
        let number = state.forks.iter().flatten().filter(|fork| fork.base == base).count() + 1;
        (state.forks.len() as u32, number)
    };
    let container = storage.join(format!(
        "{}-{}",
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |time| time.as_millis()),
        id
    ));
    std::fs::create_dir_all(&container)?;
    let dir = container.join(&folder);
    let kind = match clone_tree(base, &dir) {
        Ok(kind) => kind,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&container);
            return Err(error);
        }
    };
    let mut state = state();
    // Another fork may have been made meanwhile; take the next free id.
    let id = state.forks.len() as u32;
    state.forks.push(Some(Fork {
        name: format!("{folder} · fork {number}"),
        base: base.to_path_buf(),
        dir: dir.clone(),
        created: SystemTime::now(),
        kind,
    }));
    persist(&state);
    files_sdk::log::record(
        MANIFEST.id,
        files_sdk::log::Kind::Fs,
        format!("Forked {} → {}", base.display(), dir.display()),
        Some(format!("{kind:?}")),
    );
    Ok((id, dir, kind))
}

pub fn list() -> Vec<ForkSummary> {
    let ids: Vec<u32> = state()
        .forks
        .iter()
        .enumerate()
        .filter_map(|(id, fork)| fork.as_ref().map(|_| id as u32))
        .collect();
    ids.into_iter()
        .filter_map(|id| {
            let changes = changes(id).len();
            let state = state();
            let fork = state.forks.get(id as usize)?.as_ref()?;
            Some(ForkSummary {
                id,
                name: fork.name.clone(),
                base: fork.base.clone(),
                dir: fork.dir.clone(),
                changes,
            })
        })
        .collect()
}

fn get(id: u32) -> Option<Fork> {
    state().forks.get(id as usize)?.clone()
}

pub fn base(id: u32) -> Option<PathBuf> {
    get(id).map(|fork| fork.base)
}

pub fn name(id: u32) -> Option<String> {
    get(id).map(|fork| fork.name)
}

/// The fork's own top folder.
pub fn root(id: u32) -> Option<PathBuf> {
    get(id).map(|fork| fork.dir)
}

/// The fork a path belongs to, if it lies inside a fork's folder.
pub fn fork_of(path: &Path) -> Option<u32> {
    state()
        .forks
        .iter()
        .enumerate()
        .find(|(_, fork)| fork.as_ref().is_some_and(|fork| path.starts_with(&fork.dir)))
        .map(|(id, _)| id as u32)
}

/// The fork whose top folder is exactly `path` (for labels and "up" limits).
pub fn root_of(path: &Path) -> Option<u32> {
    state()
        .forks
        .iter()
        .enumerate()
        .find(|(_, fork)| fork.as_ref().is_some_and(|fork| fork.dir == path))
        .map(|(id, _)| id as u32)
}

/// Whether `path` is the original of at least one live fork.
pub fn has_forks(path: &Path) -> bool {
    state().forks.iter().flatten().any(|fork| fork.base == path)
}

#[derive(Clone, Copy, PartialEq)]
struct Node {
    dir: bool,
    len: u64,
    modified: Option<SystemTime>,
}

/// Every item under `root`, keyed by `/`-separated relative path.
fn snapshot(root: &Path) -> BTreeMap<String, Node> {
    let mut nodes = BTreeMap::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(root.join(&rel)) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let child = rel.join(entry.file_name());
            let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            let key = child.to_string_lossy().replace('\\', "/");
            if metadata.is_dir() {
                stack.push(child);
            }
            nodes.insert(
                key,
                Node {
                    dir: metadata.is_dir(),
                    len: metadata.len(),
                    modified: metadata.modified().ok(),
                },
            );
        }
    }
    nodes
}

/// What differs between a fork and its original; parents of added or removed
/// folders are reported once, not file by file.
fn diff(base: &Path, fork: &Path) -> Vec<Change> {
    let original = snapshot(base);
    let copy = snapshot(fork);
    let mut changes = Vec::new();
    let covered = |changes: &[Change], rel: &str| {
        changes.iter().any(|change| {
            matches!(change, Change::Added { folder: true, .. } | Change::Removed { .. })
                && rel.starts_with(&format!("{}/", change.rel()))
        })
    };
    for (rel, node) in &copy {
        match original.get(rel) {
            None if !covered(&changes, rel) => changes.push(Change::Added { rel: rel.clone(), folder: node.dir }),
            Some(before) if !node.dir && (before.dir || before.len != node.len || before.modified != node.modified) => {
                changes.push(Change::Modified { rel: rel.clone() });
            }
            _ => {}
        }
    }
    for rel in original.keys() {
        if !copy.contains_key(rel) && !covered(&changes, rel) {
            changes.push(Change::Removed { rel: rel.clone() });
        }
    }
    changes
}

const CHANGES_TTL: Duration = Duration::from_secs(3);

/// The fork's changes, from a cache refreshed in the background.
pub fn changes(id: u32) -> Vec<Change> {
    let mut state = state();
    let cached = state.changes.get(&id).cloned();
    let stale = cached.as_ref().is_none_or(|(at, _)| at.elapsed() >= CHANGES_TTL);
    let paths = state
        .forks
        .get(id as usize)
        .and_then(Option::as_ref)
        .map(|fork| (fork.base.clone(), fork.dir.clone()));
    if stale && !state.computing.contains(&id)
        && let Some((base, dir)) = paths
    {
        state.computing.insert(id);
        std::thread::spawn(move || {
            let changes = diff(&base, &dir);
            let mut state = self::state();
            state.computing.remove(&id);
            state.changes.insert(id, (Instant::now(), changes));
            GENERATION.fetch_add(1, Ordering::Relaxed);
        });
    }
    cached.map(|(_, changes)| changes).unwrap_or_default()
}

/// Up-to-date changes, computed now (used before applying).
pub fn changes_now(id: u32) -> Vec<Change> {
    let Some(fork) = get(id) else {
        return Vec::new();
    };
    let changes = diff(&fork.base, &fork.dir);
    state().changes.insert(id, (Instant::now(), changes.clone()));
    changes
}

/// Deletes the fork's copy; the original was never touched.
pub fn discard(id: u32) -> Option<PathBuf> {
    let fork = {
        let mut state = state();
        let fork = state.forks.get_mut(id as usize)?.take()?;
        state.changes.remove(&id);
        persist(&state);
        fork
    };
    if let Some(container) = fork.dir.parent()
        && storage_dir().is_some_and(|storage| container.starts_with(storage))
    {
        let _ = std::fs::remove_dir_all(container);
    }
    Some(fork.base)
}

#[derive(Debug, Default, PartialEq)]
pub struct ApplyReport {
    pub created: usize,
    pub updated: usize,
    pub removed: usize,
    /// (item, why it was left alone)
    pub skipped: Vec<(String, String)>,
}

/// Copies the fork's changes into the original folder, then deletes the fork.
/// Never overwrites an original that changed after the fork was made, never
/// removes protected paths (Settings → Deletion).
pub fn apply(
    id: u32,
    protected: &[String],
    home: &Path,
    use_trash: bool,
) -> Result<(PathBuf, ApplyReport), String> {
    let fork = get(id).ok_or("This fork was discarded")?;
    let changes = changes_now(id);
    let changed_since = |path: &Path| {
        std::fs::symlink_metadata(path)
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified > fork.created)
    };
    let mut report = ApplyReport::default();

    let mut removals = Vec::new();
    for change in &changes {
        if let Change::Removed { rel } = change {
            let path = fork.base.join(rel);
            if let Some(rule) = host().protection(&path, protected, home) {
                report.skipped.push((rel.clone(), format!("protected by \"{rule}\"")));
            } else if changed_since(&path) {
                report.skipped.push((rel.clone(), "changed in the original after forking".to_string()));
            } else {
                removals.push((rel.clone(), path));
            }
        }
    }
    let paths: Vec<&Path> = removals.iter().map(|(_, path)| path.as_path()).collect();
    match host().delete(&paths, use_trash) {
        Ok(()) => report.removed = removals.len(),
        Err((done, error)) => {
            report.removed = done;
            for (rel, _) in &removals[done..] {
                report.skipped.push((rel.clone(), host().describe_error(&error)));
            }
        }
    }

    for change in &changes {
        let (rel, kind) = match change {
            Change::Added { rel, .. } => (rel, "created"),
            Change::Modified { rel } => (rel, "updated"),
            Change::Removed { .. } => continue,
        };
        let source = fork.dir.join(rel);
        let target = fork.base.join(rel);
        if kind == "created" && std::fs::symlink_metadata(&target).is_ok() {
            report.skipped.push((rel.clone(), "already exists in the original".to_string()));
            continue;
        }
        if kind == "updated" && changed_since(&target) {
            report.skipped.push((rel.clone(), "changed in the original after forking".to_string()));
            continue;
        }
        let result = if source.is_dir() {
            copy_tree(&source, &target)
        } else {
            target
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::copy(&source, &target).map(drop))
        };
        match result {
            Ok(()) if kind == "created" => report.created += 1,
            Ok(()) => report.updated += 1,
            Err(error) => report.skipped.push((rel.clone(), host().describe_error(&error))),
        }
    }
    discard(id);
    host().files_changed();
    files_sdk::log::record(
        MANIFEST.id,
        files_sdk::log::Kind::Fs,
        format!("Applied fork to {}", fork.base.display()),
        Some(format!("created {} · updated {} · removed {} · skipped {}", report.created, report.updated, report.removed, report.skipped.len())),
    );
    Ok((fork.base, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Like the app: an exact path in the protected list is left alone, and
    /// each test thread keeps its own `forks.toml`.
    struct TestHost;

    impl files_sdk::Host for TestHost {
        fn config_dir(&self) -> Option<PathBuf> {
            let thread = format!("{:?}", std::thread::current().id()).replace(|c: char| !c.is_alphanumeric(), "");
            Some(std::env::temp_dir().join(format!("files-ext-forks-{}-{thread}", std::process::id())))
        }

        fn protection(&self, path: &Path, patterns: &[String], _: &Path) -> Option<String> {
            patterns.iter().find(|pattern| Path::new(pattern) == path).cloned()
        }

        fn delete(&self, paths: &[&Path], _: bool) -> Result<(), (usize, io::Error)> {
            for (done, path) in paths.iter().enumerate() {
                let result = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
                result.map_err(|error| (done, error))?;
            }
            Ok(())
        }
    }

    fn base(name: &str) -> PathBuf {
        files_sdk::set_host(Box::new(TestHost));
        let dir = std::env::temp_dir().join(format!("file-force-forkcow-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("notes.txt"), "keep me").unwrap();
        std::fs::write(dir.join("docs/plan.md"), "plan").unwrap();
        dir
    }

    fn rels(changes: &[Change]) -> Vec<String> {
        let mut out: Vec<String> = changes
            .iter()
            .map(|change| match change {
                Change::Added { rel, .. } => format!("+{rel}"),
                Change::Modified { rel } => format!("~{rel}"),
                Change::Removed { rel } => format!("-{rel}"),
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn a_fork_is_a_real_folder_and_the_original_stays_untouched() {
        let dir = base("real");
        let (id, fork, kind) = create(&dir).unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(kind, CloneKind::CopyOnWrite, "APFS should clone");
        }
        assert_eq!(std::fs::read_to_string(fork.join("notes.txt")).unwrap(), "keep me");
        assert!(changes_now(id).is_empty(), "a fresh fork has no changes");

        // Anything works inside: plain file system operations.
        std::fs::write(fork.join("notes.txt"), "edited in the fork").unwrap();
        std::fs::write(fork.join("new.md"), "new").unwrap();
        std::fs::create_dir_all(fork.join("assets/icons")).unwrap();
        std::fs::write(fork.join("assets/icons/logo.svg"), "<svg/>").unwrap();
        std::fs::remove_dir_all(fork.join("docs")).unwrap();

        assert_eq!(rels(&changes_now(id)), ["+assets", "+new.md", "-docs", "~notes.txt"]);
        assert_eq!(std::fs::read_to_string(dir.join("notes.txt")).unwrap(), "keep me");
        assert!(dir.join("docs/plan.md").exists());
        assert_eq!(fork_of(&fork.join("assets")), Some(id));
        assert_eq!(root_of(&fork), Some(id));

        assert_eq!(discard(id), Some(dir.clone()));
        assert!(!fork.exists(), "discard deletes the copy");
        assert_eq!(std::fs::read_to_string(dir.join("notes.txt")).unwrap(), "keep me");
    }

    #[test]
    fn applying_copies_changes_back_and_skips_conflicts() {
        let dir = base("apply");
        std::fs::write(dir.join("guarded.txt"), "x").unwrap();
        std::fs::write(dir.join("both.txt"), "v1").unwrap();
        let (id, fork, _) = create(&dir).unwrap();
        std::fs::write(fork.join("notes.txt"), "edited").unwrap();
        std::fs::write(fork.join("fresh.md"), "fresh").unwrap();
        std::fs::remove_file(fork.join("guarded.txt")).unwrap();
        std::fs::remove_dir_all(fork.join("docs")).unwrap();
        // Edited in both places: the original's newer edit wins, the fork's is skipped.
        std::fs::write(fork.join("both.txt"), "fork").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("both.txt"), "original edit").unwrap();

        let protected = vec![dir.join("guarded.txt").display().to_string()];
        let (applied, report) = apply(id, &protected, Path::new("/nonexistent-home"), false).unwrap();
        assert_eq!(applied, dir);
        assert_eq!(std::fs::read_to_string(dir.join("notes.txt")).unwrap(), "edited");
        assert_eq!(std::fs::read_to_string(dir.join("fresh.md")).unwrap(), "fresh");
        assert!(!dir.join("docs").exists());
        assert!(dir.join("guarded.txt").exists(), "protected item was removed");
        assert_eq!(std::fs::read_to_string(dir.join("both.txt")).unwrap(), "original edit");
        assert_eq!((report.created, report.updated, report.removed), (1, 1, 1), "{report:?}");
        assert_eq!(report.skipped.len(), 2, "{:?}", report.skipped);
        assert!(!fork.exists(), "applying deletes the fork");
    }

    #[test]
    fn forks_survive_a_restart() {
        let dir = base("restart");
        let (id, fork, _) = create(&dir).unwrap();
        let saved = load(&store_path().unwrap());
        assert!(saved.iter().any(|stored| stored.dir == fork && stored.base == dir), "fork was not saved");
        discard(id);
        assert!(load(&store_path().unwrap()).iter().all(|stored| stored.dir != fork));
    }

    #[test]
    fn plain_copies_keep_dates_so_nothing_looks_changed() {
        let dir = base("copy");
        let target = std::env::temp_dir().join(format!("file-force-forkcopy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&target);
        copy_tree(&dir, &target).unwrap();
        assert!(diff(&dir, &target).is_empty(), "{:?}", diff(&dir, &target));
        let _ = std::fs::remove_dir_all(&target);
    }
}
