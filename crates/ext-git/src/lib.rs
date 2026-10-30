//! Git for Files: the everyday Git commands from the context menu,
//! run through the system `git` (so your config, credentials helper, hooks
//! and SSH keys all apply). Modeled on TortoiseGit's menu:
//! <https://github.com/tortoisegit/tortoisegit>.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use files_sdk::{Icon, Manifest, Permission};

pub static MANIFEST: Manifest = Manifest {
    id: "git",
    name: "Git",
    version: env!("CARGO_PKG_VERSION"),
    source: "Built-in",
    summary: "Git from the context menu: commit, log, diff, pull, push, branches, stash, clone.",
    icon: Icon::Builtin("git-branch"),
    adds: &[
        "Context menu → Git (in a repository: Commit, Log, Diff, Sync, Branches, Stash, Revert…)",
        "Context menu → Git Clone… / Git Create repository here… (outside one)",
        "Windows: Commit, Log, Diff, Switch/Create branch, Clone",
    ],
    permissions: &[Permission::ReadFiles, Permission::WriteFiles, Permission::LaunchApps, Permission::Network],
    commands: &[],
    menu: &[],
    sidebar: &[],
    menu_color: Some("orange"),
    tier: files_sdk::Tier::Free,
};

pub struct Git;

pub static EXTENSION: Git = Git;

impl files_sdk::Extension for Git {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }
}

// ------------------------------------------------------------ running git

fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(dir)
        // Never wait for a terminal prompt: credentials come from the helper.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Runs git and returns stdout; a failure carries git's own message.
fn run(dir: &Path, args: &[&str]) -> io::Result<String> {
    let output = files_sdk::log::run(MANIFEST.id, git(dir).args(args))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let message = stderr.lines().filter(|line| !line.trim().is_empty()).collect::<Vec<_>>().join("\n");
    Err(io::Error::other(if message.is_empty() { format!("git {} failed", args.join(" ")) } else { message }))
}

fn write_allowed() -> io::Result<()> {
    files_sdk::permissions::check(MANIFEST.id, Permission::WriteFiles)
}

fn network_allowed() -> io::Result<()> {
    files_sdk::permissions::check(MANIFEST.id, Permission::Network)
}

pub fn installed() -> bool {
    Command::new("git").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|status| status.success())
}

/// The top folder of the repository `path` is in.
pub fn repo_root(path: &Path) -> Option<PathBuf> {
    let dir = if path.is_dir() { path } else { path.parent()? };
    let root = run(dir, &["rev-parse", "--show-toplevel"]).ok()?;
    let root = root.trim();
    (!root.is_empty()).then(|| PathBuf::from(root))
}

/// `path` relative to `root`, as git wants it.
fn relative(root: &Path, path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    path.strip_prefix(&root).map_or_else(|_| path.display().to_string(), |rel| {
        let rel = rel.display().to_string();
        if rel.is_empty() { ".".to_string() } else { rel }
    })
}

// ------------------------------------------------------------ status

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

impl ChangeKind {
    pub fn letter(self) -> &'static str {
        match self {
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Untracked => "?",
            Self::Conflicted => "!",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
            Self::Untracked => "not versioned",
            Self::Conflicted => "conflicted",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    /// Relative to the repository root, `/`-separated.
    pub path: String,
    pub kind: ChangeKind,
    /// Already in the index.
    pub staged: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub changes: Vec<Change>,
}

pub fn status(root: &Path) -> io::Result<Status> {
    Ok(parse_status(&run(root, &["status", "--porcelain=v2", "--branch", "--untracked-files=all"])?))
}

/// `git status --porcelain=v2 --branch`.
pub fn parse_status(output: &str) -> Status {
    let mut status = Status::default();
    for line in output.lines() {
        if let Some(head) = line.strip_prefix("# branch.head ") {
            status.branch = (head != "(detached)").then(|| head.to_string());
        } else if let Some(upstream) = line.strip_prefix("# branch.upstream ") {
            status.upstream = Some(upstream.to_string());
        } else if let Some(counts) = line.strip_prefix("# branch.ab ") {
            for part in counts.split_whitespace() {
                if let Some(ahead) = part.strip_prefix('+') {
                    status.ahead = ahead.parse().unwrap_or(0);
                } else if let Some(behind) = part.strip_prefix('-') {
                    status.behind = behind.parse().unwrap_or(0);
                }
            }
        } else if let Some(path) = line.strip_prefix("? ") {
            status.changes.push(Change { path: path.to_string(), kind: ChangeKind::Untracked, staged: false });
        } else if let Some(rest) = line.strip_prefix("u ") {
            let path = rest.splitn(10, ' ').nth(9).unwrap_or_default();
            status.changes.push(Change { path: path.to_string(), kind: ChangeKind::Conflicted, staged: false });
        } else if line.starts_with("1 ") || line.starts_with("2 ") {
            let renamed = line.starts_with("2 ");
            let fields: Vec<&str> = line.splitn(if renamed { 10 } else { 9 }, ' ').collect();
            let xy = fields.get(1).copied().unwrap_or("..");
            let mut path = fields.last().copied().unwrap_or_default();
            if renamed {
                // `new\told`
                path = path.split('\t').next().unwrap_or(path);
            }
            let (index, tree) = (xy.chars().next().unwrap_or('.'), xy.chars().nth(1).unwrap_or('.'));
            let code = if tree != '.' { tree } else { index };
            let kind = match code {
                'A' => ChangeKind::Added,
                'D' => ChangeKind::Deleted,
                'R' | 'C' => ChangeKind::Renamed,
                _ => ChangeKind::Modified,
            };
            status.changes.push(Change { path: path.to_string(), kind, staged: index != '.' });
        }
    }
    status
}

// ------------------------------------------------------------ commit, add, revert

/// Commits exactly `paths` (relative to `root`) with `message`.
pub fn commit(root: &Path, paths: &[String], message: &str) -> io::Result<String> {
    write_allowed()?;
    if message.trim().is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "The commit message is empty"));
    }
    if paths.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Nothing selected to commit"));
    }
    let mut add = vec!["add", "--all", "--"];
    add.extend(paths.iter().map(String::as_str));
    run(root, &add)?;
    let mut commit = vec!["commit", "-m", message, "--"];
    commit.extend(paths.iter().map(String::as_str));
    run(root, &commit)?;
    Ok(run(root, &["log", "-1", "--format=%h %s"])?.trim().to_string())
}

pub fn add(root: &Path, paths: &[PathBuf]) -> io::Result<()> {
    write_allowed()?;
    let rel: Vec<String> = paths.iter().map(|path| relative(root, path)).collect();
    let mut args = vec!["add", "--"];
    args.extend(rel.iter().map(String::as_str));
    run(root, &args).map(drop)
}

/// Throws away local changes of `paths` (TortoiseGit's Revert). Untracked
/// files are left alone.
pub fn revert(root: &Path, paths: &[PathBuf]) -> io::Result<()> {
    write_allowed()?;
    let rel: Vec<String> = paths.iter().map(|path| relative(root, path)).collect();
    let mut args = vec!["restore", "--staged", "--worktree", "--source=HEAD", "--"];
    args.extend(rel.iter().map(String::as_str));
    run(root, &args).map(drop)
}

// ------------------------------------------------------------ sync

pub fn pull(root: &Path) -> io::Result<String> {
    network_allowed()?;
    write_allowed()?;
    run(root, &["pull", "--ff-only"]).map(|out| summary(&out, "Already up to date"))
}

pub fn fetch(root: &Path) -> io::Result<String> {
    network_allowed()?;
    run(root, &["fetch", "--all", "--prune"]).map(|out| summary(&out, "Fetched"))
}

/// Pushes the current branch, setting its upstream the first time.
pub fn push(root: &Path) -> io::Result<String> {
    network_allowed()?;
    let status = status(root)?;
    let branch = status.branch.ok_or_else(|| io::Error::other("Detached HEAD: switch to a branch first"))?;
    let output = if status.upstream.is_some() {
        files_sdk::log::run(MANIFEST.id, git(root).args(["push"]))?
    } else {
        files_sdk::log::run(MANIFEST.id, git(root).args(["push", "--set-upstream", "origin", &branch]))?
    };
    // git push reports progress on stderr even when it works.
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    if output.status.success() {
        Ok(summary(&text, &format!("Pushed {branch}")))
    } else {
        Err(io::Error::other(text.trim().to_string()))
    }
}

fn summary(output: &str, fallback: &str) -> String {
    output.lines().rev().map(str::trim).find(|line| !line.is_empty()).unwrap_or(fallback).to_string()
}

// ------------------------------------------------------------ log

#[derive(Clone, Debug, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub author: String,
    /// Seconds since the Unix epoch.
    pub time: i64,
    pub subject: String,
    /// Branch and tag names pointing here.
    pub refs: Vec<String>,
}

/// Newest first; only commits touching `path` when given.
pub fn log(root: &Path, path: Option<&Path>, limit: usize) -> io::Result<Vec<Commit>> {
    let limit = format!("-{limit}");
    let mut args = vec!["log", &limit, "--format=%H%x1f%h%x1f%an%x1f%at%x1f%s%x1f%D%x1e"];
    let rel = path.map(|path| relative(root, path));
    if let Some(rel) = &rel {
        args.extend(["--", rel.as_str()]);
    }
    // A repository without commits has an empty log, not an error.
    let output = match run(root, &args) {
        Ok(output) => output,
        Err(error) if error.to_string().contains("does not have any commits") => String::new(),
        Err(error) => return Err(error),
    };
    Ok(parse_log(&output))
}

pub fn parse_log(output: &str) -> Vec<Commit> {
    output
        .split('\x1e')
        .filter_map(|record| {
            let fields: Vec<&str> = record.trim_start_matches('\n').split('\x1f').collect();
            if fields.len() < 6 {
                return None;
            }
            Some(Commit {
                hash: fields[0].to_string(),
                short: fields[1].to_string(),
                author: fields[2].to_string(),
                time: fields[3].parse().unwrap_or(0),
                subject: fields[4].to_string(),
                refs: fields[5].split(", ").filter(|name| !name.is_empty()).map(str::to_string).collect(),
            })
        })
        .collect()
}

/// Files a commit changed, with their kind.
pub fn commit_files(root: &Path, hash: &str) -> io::Result<Vec<Change>> {
    let output = run(root, &["show", "--format=", "--name-status", "--no-renames", hash])?;
    Ok(output
        .lines()
        .filter_map(|line| {
            let (code, path) = line.split_once('\t')?;
            let kind = match code.chars().next()? {
                'A' => ChangeKind::Added,
                'D' => ChangeKind::Deleted,
                _ => ChangeKind::Modified,
            };
            Some(Change { path: path.to_string(), kind, staged: true })
        })
        .collect())
}

// ------------------------------------------------------------ diff

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineKind {
    Header,
    Hunk,
    Added,
    Removed,
    Context,
}

/// A diff split into lines, each tagged for coloring.
pub fn diff_lines(diff: &str) -> Vec<(LineKind, String)> {
    diff.lines()
        .map(|line| {
            let kind = if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") || line.starts_with("index ") {
                LineKind::Header
            } else if line.starts_with("@@") {
                LineKind::Hunk
            } else if line.starts_with('+') {
                LineKind::Added
            } else if line.starts_with('-') {
                LineKind::Removed
            } else {
                LineKind::Context
            };
            (kind, line.to_string())
        })
        .collect()
}

/// Working tree against HEAD for `path` (a new file shows whole).
pub fn diff_worktree(root: &Path, path: &str) -> io::Result<String> {
    let diff = run(root, &["diff", "HEAD", "--", path]).or_else(|_| run(root, &["diff", "--", path]))?;
    if !diff.trim().is_empty() {
        return Ok(diff);
    }
    // Untracked: show it as all added.
    let output = files_sdk::log::run(MANIFEST.id, git(root).args(["diff", "--no-index", "--", "/dev/null", path]))?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// What `hash` changed in `path`.
pub fn diff_commit(root: &Path, hash: &str, path: &str) -> io::Result<String> {
    run(root, &["show", "--format=", hash, "--", path])
}

// ------------------------------------------------------------ branches

#[derive(Clone, Debug, PartialEq)]
pub struct Branch {
    pub name: String,
    pub current: bool,
    pub remote: bool,
}

pub fn branches(root: &Path) -> io::Result<Vec<Branch>> {
    let output = run(root, &["branch", "--all", "--format=%(HEAD)%(refname)"])?;
    Ok(output
        .lines()
        .filter_map(|line| {
            let current = line.starts_with('*');
            let name = line.trim_start_matches(['*', ' ']);
            if let Some(local) = name.strip_prefix("refs/heads/") {
                Some(Branch { name: local.to_string(), current, remote: false })
            } else {
                let remote = name.strip_prefix("refs/remotes/")?;
                (!remote.ends_with("/HEAD")).then(|| Branch { name: remote.to_string(), current: false, remote: true })
            }
        })
        .collect())
}

/// Switches to `name`; a remote branch gets a local tracking branch.
pub fn switch(root: &Path, name: &str, remote: bool) -> io::Result<()> {
    write_allowed()?;
    if remote {
        let local = name.split_once('/').map_or(name, |(_, branch)| branch);
        return run(root, &["switch", "--track", "-c", local, name]).map(drop);
    }
    run(root, &["switch", name]).map(drop)
}

pub fn create_branch(root: &Path, name: &str, switch_to: bool) -> io::Result<()> {
    write_allowed()?;
    if name.trim().is_empty() || name.contains(' ') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "A branch name cannot be empty or contain spaces"));
    }
    if switch_to {
        run(root, &["switch", "-c", name]).map(drop)
    } else {
        run(root, &["branch", name]).map(drop)
    }
}

// ------------------------------------------------------------ stash, init, clone

pub fn stash_save(root: &Path, message: Option<&str>) -> io::Result<String> {
    write_allowed()?;
    let mut args = vec!["stash", "push", "--include-untracked"];
    if let Some(message) = message.filter(|message| !message.trim().is_empty()) {
        args.extend(["-m", message]);
    }
    run(root, &args).map(|out| summary(&out, "Stashed"))
}

pub fn stash_pop(root: &Path) -> io::Result<String> {
    write_allowed()?;
    run(root, &["stash", "pop"]).map(|out| summary(&out, "Applied the stash"))
}

pub fn stash_count(root: &Path) -> usize {
    run(root, &["stash", "list"]).map_or(0, |out| out.lines().count())
}

pub fn init(dir: &Path) -> io::Result<()> {
    write_allowed()?;
    run(dir, &["init"]).map(drop)
}

/// Clones `url` into `parent/<name>`; returns the new folder.
pub fn clone(url: &str, parent: &Path) -> io::Result<PathBuf> {
    network_allowed()?;
    write_allowed()?;
    let name = url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .map(|name| name.trim_end_matches(".git"))
        .filter(|name| !name.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Not a repository URL"))?;
    let target = parent.join(name);
    if target.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", target.display())));
    }
    run(parent, &["clone", "--", url, name])?;
    Ok(target)
}

#[cfg(test)]
mod tests;
