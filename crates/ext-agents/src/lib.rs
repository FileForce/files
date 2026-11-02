//! The Agents extension: sessions of Claude Code, Codex, OpenCode, Kimi and GLM.
//!
//! Everything is read from what the agents already keep on disk, so sessions
//! started anywhere (a terminal, an IDE, the agent's own app) show up too and
//! are tagged *external*; nothing is uploaded anywhere.
//!
//! - Claude Code: `~/.claude/projects/<project>/<session>.jsonl`, account and
//!   plan from `~/.claude.json`.
//! - Codex: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`, including the
//!   rate-limit windows the server reports with every turn.
//! - OpenCode: the SQLite database `~/.local/share/opencode/opencode.db`
//!   (read-only, through the system `sqlite3`).
//! - Kimi: `~/.kimi-code/sessions/<project>/<session>/state.json` and the
//!   main agent's `wire.jsonl`.
//! - GLM / ZCode: the SQLite database `~/.zcode/cli/db/db.sqlite`, read-only.
//!
//! New sessions run the agent's own CLI in non-interactive streaming mode
//! ([`start`]), so answers appear as they are generated.

use std::collections::HashMap;
use std::io::{self, BufRead as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use files_sdk::{Icon, Manifest, Permission};
use serde_json::Value;

mod parse;
pub mod stream;

pub use parse::{claude_session, codex_session};

pub static MANIFEST: Manifest = Manifest {
    id: "agents",
    name: "Agents",
    version: env!("CARGO_PKG_VERSION"),
    source: "Built-in (experimental)",
    summary: "Sessions of Claude Code, Codex, OpenCode, Kimi and GLM: chats, live answers, tokens, limits and plans.",
    icon: Icon::Svg(ICON),
    adds: &[
        "Sidebar → Agents: recent sessions, running ones marked, external ones tagged",
        "Session window: the chat, a reply box with streaming answers",
        "Usage: tokens in/out/cache, cost, rate limits and plan per agent",
    ],
    permissions: &[Permission::ReadFiles, Permission::LaunchApps, Permission::Network],
    commands: &[],
    menu: &[],
    sidebar: &[],
    menu_color: Some("amber"),
    tier: files_sdk::Tier::Free,
};

/// A small robot head.
pub const ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="8" width="16" height="12" rx="3"/><path d="M12 8V4"/><circle cx="12" cy="3" r="1"/><path d="M9 13v1"/><path d="M15 13v1"/><path d="M2 13v3"/><path d="M22 13v3"/></svg>"#;

pub struct Agents;

pub static EXTENSION: Agents = Agents;

impl files_sdk::Extension for Agents {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Agent {
    Claude,
    Codex,
    OpenCode,
    Kimi,
    Glm,
}

impl Agent {
    pub const ALL: [Self; 5] = [Self::Claude, Self::Codex, Self::OpenCode, Self::Kimi, Self::Glm];
    pub const RUNNABLE: [Self; 4] = [Self::Claude, Self::Codex, Self::OpenCode, Self::Kimi];

    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
            Self::Kimi => "Kimi Code",
            Self::Glm => "GLM / ZCode",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Kimi => "kimi",
            Self::Glm => "zcode",
        }
    }

    /// The CLI program.
    pub fn program(self) -> &'static str {
        self.short()
    }

    pub fn supports_run(self) -> bool {
        Self::RUNNABLE.contains(&self)
    }

    /// The macOS bundle id of the agent's desktop app.
    pub fn desktop_bundle(self) -> &'static str {
        match self {
            Self::Claude => "com.anthropic.claudefordesktop",
            Self::Codex => "com.openai.codex",
            Self::OpenCode => "ai.opencode.desktop",
            Self::Kimi | Self::Glm => "",
        }
    }

    /// Whether the desktop app is installed (looked up once).
    pub fn has_desktop_app(self) -> bool {
        static FOUND: OnceLock<Vec<Agent>> = OnceLock::new();
        FOUND
            .get_or_init(|| {
                Agent::ALL
                    .into_iter()
                    .filter(|agent| desktop_app_path(agent.desktop_bundle()).is_some())
                    .collect()
            })
            .contains(&self)
    }

    pub fn installed(self) -> bool {
        override_for(self).is_some() || which(self.program()).is_some()
    }
}

/// Tokens used, as each agent reports them. Missing kinds stay 0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    /// Input served from the prompt cache.
    pub cache_read: u64,
    /// Input written to the prompt cache.
    pub cache_write: u64,
    /// Output spent on hidden reasoning (included in `output` by some agents).
    pub reasoning: u64,
    /// In US dollars, when the agent computes it (OpenCode).
    pub cost: Option<f64>,
}

impl Usage {
    pub fn add(&mut self, other: &Usage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.reasoning += other.reasoning;
        self.cost = match (self.cost, other.cost) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0.) + b.unwrap_or(0.)),
        };
    }

    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub agent: Agent,
    pub id: String,
    pub title: String,
    pub cwd: Option<PathBuf>,
    pub model: Option<String>,
    /// Seconds since the Unix epoch.
    pub updated: i64,
    pub usage: Usage,
    /// Where it was started: "cli", "vscode", "desktop"…
    pub origin: Option<String>,
    /// The agent is working in it right now.
    pub running: bool,
    /// Not started from Files.
    pub external: bool,
    /// The file it was read from (Claude, Codex).
    pub source: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    User,
    Assistant,
    /// Tool calls and their results, shortened.
    Tool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    pub role: Role,
    pub text: String,
}

/// One rate-limit window, e.g. "5 hours" or "7 days".
#[derive(Clone, Debug, PartialEq)]
pub struct Limit {
    pub name: String,
    pub used_percent: f64,
    /// Seconds since the Unix epoch.
    pub resets_at: Option<i64>,
}

/// What an agent's account says about plan and limits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Account {
    pub plan: Option<String>,
    pub organization: Option<String>,
    pub details: Vec<(String, String)>,
    pub limits: Vec<Limit>,
}

/// Where each agent keeps its data. [`Roots::detect`] follows the agents'
/// own environment variables; tests point it at fixtures.
#[derive(Clone, Debug, PartialEq)]
pub struct Roots {
    pub claude: PathBuf,
    /// `~/.claude.json`.
    pub claude_account: PathBuf,
    pub codex: PathBuf,
    pub opencode_db: PathBuf,
    pub kimi: PathBuf,
    pub zcode_db: PathBuf,
}

impl Roots {
    pub fn detect() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let claude = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home.join(".claude"));
        let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".codex"));
        let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
        let kimi = std::env::var_os("KIMI_CODE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".kimi-code"));
        let zcode = std::env::var_os("ZCODE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".zcode"));
        Self {
            claude_account: home.join(".claude.json"),
            claude,
            codex,
            opencode_db: data.join("opencode/opencode.db"),
            kimi,
            zcode_db: zcode.join("cli/db/db.sqlite"),
        }
    }
}

// ------------------------------------------------------------ scanning

/// Parsed sessions by file, reused while the file is unchanged.
fn cache() -> &'static Mutex<HashMap<PathBuf, (SystemTime, Session)>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (SystemTime, Session)>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn cached(path: &Path, parse: impl FnOnce(&Path) -> Option<Session>) -> Option<Session> {
    let modified = std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok()?;
    if let Some((time, session)) = cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(path) {
        if *time == modified {
            return Some(session.clone());
        }
    }
    let session = parse(path)?;
    cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(path.to_path_buf(), (modified, session.clone()));
    Some(session)
}

fn files_with(root: &Path, depth: usize, keep: &dyn Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                files_with(&path, depth - 1, keep, out);
            }
        } else if keep(&path) {
            out.push(path);
        }
    }
}

/// Every session of every agent, newest first. `ours` are the ids Files
/// started itself (everything else is external).
pub fn scan(roots: &Roots, ours: &[String]) -> Vec<Session> {
    if files_sdk::permissions::check(MANIFEST.id, Permission::ReadFiles).is_err() {
        return Vec::new();
    }
    let mut sessions = Vec::new();

    let mut files = Vec::new();
    files_with(&roots.claude.join("projects"), 1, &|path| path.extension().is_some_and(|ext| ext == "jsonl"), &mut files);
    sessions.extend(files.iter().filter_map(|path| cached(path, claude_session)));

    let mut files = Vec::new();
    files_with(
        &roots.codex.join("sessions"),
        3,
        &|path| path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("rollout-")),
        &mut files,
    );
    sessions.extend(files.iter().filter_map(|path| cached(path, codex_session)));

    sessions.extend(sqlite_sessions(&roots.opencode_db, Agent::OpenCode));

    let mut files = Vec::new();
    files_with(&roots.kimi.join("sessions"), 4, &|path| path.ends_with("agents/main/wire.jsonl"), &mut files);
    sessions.extend(files.iter().filter_map(|path| cached(path, parse::kimi_session)));

    sessions.extend(sqlite_sessions(&roots.zcode_db, Agent::Glm));

    let running = running_agents();
    let now = now();
    // The newest session of an agent in a folder where that agent runs now.
    let mut newest: HashMap<(Agent, Option<PathBuf>), usize> = HashMap::new();
    for (index, session) in sessions.iter().enumerate() {
        let key = (session.agent, session.cwd.clone());
        match newest.get(&key) {
            Some(&best) if sessions[best].updated >= session.updated => {}
            _ => {
                newest.insert(key, index);
            }
        }
    }
    for ((agent, cwd), index) in newest {
        let alive = running.iter().any(|(running_agent, running_cwd)| *running_agent == agent && Some(running_cwd) == cwd.as_ref());
        // An idle agent left open for hours is not "running".
        if alive && now - sessions[index].updated < 6 * 3600 {
            sessions[index].running = true;
        }
    }
    for session in &mut sessions {
        session.external = !ours.contains(&session.id);
    }
    sessions.sort_by_key(|session| std::cmp::Reverse(session.updated));
    sessions
}

pub(crate) fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_secs() as i64)
}

/// Agent processes running now, with their working folder.
fn running_agents() -> Vec<(Agent, PathBuf)> {
    let Ok(output) = Command::new("ps").args(["-axo", "pid=,command="]).output() else {
        return Vec::new();
    };
    let mut pids: Vec<(u32, Agent)> = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let line = line.trim_start();
        let Some((pid, command)) = line.split_once(' ') else { continue };
        let Ok(pid) = pid.parse::<u32>() else { continue };
        if let Some(agent) = agent_of_command(command) {
            pids.push((pid, agent));
        }
    }
    if pids.is_empty() {
        return Vec::new();
    }
    let list = pids.iter().map(|(pid, _)| pid.to_string()).collect::<Vec<_>>().join(",");
    let Ok(output) = Command::new("lsof").args(["-a", "-d", "cwd", "-Fpn", "-p", &list]).output() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut current: Option<Agent> = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(pid) = line.strip_prefix('p') {
            current = pid.parse::<u32>().ok().and_then(|pid| pids.iter().find(|(known, _)| *known == pid).map(|(_, agent)| *agent));
        } else if let (Some(path), Some(agent)) = (line.strip_prefix('n'), current) {
            result.push((agent, PathBuf::from(path)));
        }
    }
    result
}

/// Which agent a process command line belongs to.
pub fn agent_of_command(command: &str) -> Option<Agent> {
    let mut parts = command.split_whitespace();
    let program = parts.next()?;
    let name = |part: &str| Path::new(part).file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default();
    let first = name(program);
    // Node-based CLIs run as `node …/bin/codex`.
    let script = if first == "node" || first.starts_with("bun") { parts.next().map(name).unwrap_or_default() } else { first };
    match script.as_str() {
        "claude" => Some(Agent::Claude),
        "codex" => Some(Agent::Codex),
        "opencode" => Some(Agent::OpenCode),
        "kimi" => Some(Agent::Kimi),
        "zcode" | "zcode-cli" => Some(Agent::Glm),
        _ => None,
    }
}

/// Where an app with this bundle id is installed (Spotlight).
fn desktop_app_path(bundle: &str) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let output = Command::new("/usr/bin/mdfind").arg(format!("kMDItemCFBundleIdentifier == '{bundle}'")).output().ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(PathBuf::from)
        .find(|path| path.extension().is_some_and(|ext| ext == "app") && path.exists())
}

/// Opens `folder` in the agent's desktop app.
pub fn open_in_desktop(agent: Agent, folder: &Path) -> io::Result<()> {
    files_sdk::permissions::check(MANIFEST.id, Permission::LaunchApps)?;
    let output = files_sdk::log::run(MANIFEST.id, Command::new("/usr/bin/open").args(["-b", agent.desktop_bundle()]).arg(folder))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{} desktop app is not installed", agent.name())))
    }
}

/// The shell command that starts the agent's CLI interactively in `folder`.
pub fn terminal_command(agent: Agent, folder: &Path) -> io::Result<String> {
    files_sdk::permissions::check(MANIFEST.id, Permission::LaunchApps)?;
    let program = override_for(agent)
        .or_else(|| which(agent.program()))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{} is not installed", agent.name())))?;
    let quote = |value: &str| format!("'{}'", value.replace('\'', r"'\''"));
    Ok(format!("cd {} && exec {}", quote(&folder.to_string_lossy()), quote(&program.to_string_lossy())))
}

fn overrides() -> &'static Mutex<HashMap<Agent, PathBuf>> {
    static OVERRIDES: OnceLock<Mutex<HashMap<Agent, PathBuf>>> = OnceLock::new();
    OVERRIDES.get_or_init(Default::default)
}

fn override_for(agent: Agent) -> Option<PathBuf> {
    overrides().lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(&agent).cloned()
}

/// For tests: run `program` instead of the agent's real CLI.
#[doc(hidden)]
pub fn use_program(agent: Agent, program: PathBuf) {
    overrides().lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(agent, program);
}

fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    // GUI apps get a short PATH: also look where installers put these CLIs.
    let extra = [
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".local/bin"),
        home.join(".opencode/bin"),
        home.join("Library/pnpm"),
        home.join(".npm-global/bin"),
        home.join(".bun/bin"),
    ];
    std::env::split_paths(&path)
        .chain(extra)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

// ------------------------------------------------------------ OpenCode

fn sqlite_json(db: &Path, query: &str) -> Vec<Value> {
    if !db.is_file() {
        return Vec::new();
    }
    let Ok(output) = Command::new("sqlite3").arg("-readonly").arg("-json").arg(db).arg(query).output() else {
        return Vec::new();
    };
    serde_json::from_slice::<Vec<Value>>(&output.stdout).unwrap_or_default()
}

fn sqlite_sessions(db: &Path, agent: Agent) -> Vec<Session> {
    sqlite_json(
        db,
        "select id, title, directory, model, cost, tokens_input, tokens_output, tokens_reasoning, \
         tokens_cache_read, tokens_cache_write, time_updated from session \
         where time_archived is null and parent_id is null",
    )
    .into_iter()
    .filter_map(|row| {
        let number = |key: &str| row.get(key).and_then(Value::as_u64).unwrap_or(0);
        let model = row.get("model").and_then(Value::as_str).map(|model| {
            // Stored as JSON {"providerID":…,"modelID":…} or a plain string.
            serde_json::from_str::<Value>(model)
                .ok()
                .and_then(|value| value.get("modelID").or(value.get("id")).and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| model.to_string())
        });
        Some(Session {
            agent,
            id: row.get("id")?.as_str()?.to_string(),
            title: row.get("title").and_then(Value::as_str).unwrap_or("Untitled").to_string(),
            cwd: row.get("directory").and_then(Value::as_str).map(PathBuf::from),
            model,
            updated: number("time_updated") as i64 / 1000,
            usage: Usage {
                input: number("tokens_input"),
                output: number("tokens_output"),
                cache_read: number("tokens_cache_read"),
                cache_write: number("tokens_cache_write"),
                reasoning: number("tokens_reasoning"),
                cost: row.get("cost").and_then(Value::as_f64),
            },
            origin: Some(agent.short().to_string()),
            running: false,
            external: true,
            source: Some(db.to_path_buf()),
        })
    })
    .collect()
}

fn sqlite_messages(db: &Path, session: &str) -> Vec<ChatMessage> {
    let id = session.replace('\'', "''");
    sqlite_json(
        db,
        &format!(
            "select m.data as message, p.data as part from part p join message m on m.id = p.message_id \
             where p.session_id = '{id}' order by p.time_created, p.id"
        ),
    )
    .into_iter()
    .filter_map(|row| {
        let message: Value = serde_json::from_str(row.get("message")?.as_str()?).ok()?;
        let part: Value = serde_json::from_str(row.get("part")?.as_str()?).ok()?;
        let role = match message.get("role")?.as_str()? {
            "user" => Role::User,
            _ => Role::Assistant,
        };
        match part.get("type")?.as_str()? {
            "text" => Some(ChatMessage { role, text: part.get("text")?.as_str()?.to_string() }),
            "tool" => Some(ChatMessage {
                role: Role::Tool,
                text: format!("⚙ {}", part.get("tool").and_then(Value::as_str).unwrap_or("tool")),
            }),
            _ => None,
        }
    })
    .filter(|message| !message.text.trim().is_empty())
    .collect()
}

// ------------------------------------------------------------ chats

/// The conversation of a session, oldest first.
pub fn messages(roots: &Roots, session: &Session) -> Vec<ChatMessage> {
    match (session.agent, &session.source) {
        (Agent::Claude, Some(path)) => parse::claude_messages(path),
        (Agent::Codex, Some(path)) => parse::codex_messages(path),
        (Agent::Kimi, Some(path)) => parse::kimi_messages(path),
        (Agent::OpenCode, _) => sqlite_messages(&roots.opencode_db, &session.id),
        (Agent::Glm, _) => sqlite_messages(&roots.zcode_db, &session.id),
        _ => Vec::new(),
    }
}

/// Changes whenever the session gets new messages: the log's size and
/// modification time, or the database's newest part. Cheap enough to poll.
pub fn fingerprint(roots: &Roots, session: &Session) -> Option<(u64, i64)> {
    match (session.agent, &session.source) {
        (Agent::OpenCode | Agent::Glm, _) => {
            let db = if session.agent == Agent::OpenCode { &roots.opencode_db } else { &roots.zcode_db };
            let id = session.id.replace('\'', "''");
            let rows = sqlite_json(
                db,
                &format!("select count(*) as n, max(time_updated) as t from part where session_id = '{id}'"),
            );
            let row = rows.first()?;
            Some((row.get("n")?.as_u64()?, row.get("t").and_then(Value::as_i64).unwrap_or(0)))
        }
        (_, Some(path)) => {
            let metadata = std::fs::metadata(path).ok()?;
            let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as i64;
            Some((metadata.len(), modified))
        }
        _ => None,
    }
}

/// Finds a session by id (e.g. one Files just started, to follow its log).
pub fn find(roots: &Roots, agent: Agent, id: &str) -> Option<Session> {
    scan(roots, &[]).into_iter().find(|session| session.agent == agent && session.id == id)
}

// ------------------------------------------------------------ accounts

/// Plan, organization and rate limits per agent, from local files only.
/// Secrets (tokens, API keys) are never read.
pub fn accounts(roots: &Roots, sessions: &[Session]) -> Vec<(Agent, Account)> {
    let mut accounts = Vec::new();
    if let Some(account) = claude_account(&roots.claude_account) {
        accounts.push((Agent::Claude, account));
    }
    // Codex reports its limits with every turn: the newest report wins.
    let newest_codex = sessions.iter().filter(|session| session.agent == Agent::Codex).filter_map(|session| session.source.as_ref());
    // (sessions are newest first; one that hit an error may carry no report).
    if let Some(account) = newest_codex.filter_map(|path| parse::codex_limits(path)).next() {
        accounts.push((Agent::Codex, account));
    }
    accounts
}

fn claude_account(path: &Path) -> Option<Account> {
    let value: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let account = value.get("oauthAccount")?;
    let text = |key: &str| account.get(key).and_then(Value::as_str).map(str::to_string);
    let mut details = Vec::new();
    for (key, label) in [
        ("seatTier", "Seat"),
        ("billingType", "Billing"),
        ("organizationType", "Organization type"),
        ("userRateLimitTier", "Rate limit tier"),
        ("organizationRateLimitTier", "Organization limit tier"),
    ] {
        if let Some(value) = text(key) {
            details.push((label.to_string(), value.replace('_', " ")));
        }
    }
    if account.get("hasExtraUsageEnabled").and_then(Value::as_bool) == Some(true) {
        details.push(("Extra usage".to_string(), "enabled".to_string()));
    }
    Some(Account {
        plan: text("organizationType").or_else(|| text("seatTier")).map(|plan| plan.replace('_', " ")),
        organization: text("organizationName"),
        details,
        limits: Vec::new(),
    })
}

// ------------------------------------------------------------ models

/// A model the user can pick for a turn.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelChoice {
    /// What the CLI takes (`opus`, `gpt-5.5`, `kimi-for-coding/k3`).
    pub id: String,
    pub label: String,
    /// Reasoning efforts this model accepts, weakest first.
    pub efforts: Vec<String>,
    pub default_effort: Option<String>,
    /// The agent's own default (from its config).
    pub is_default: bool,
}

/// Model and effort for a turn; `None` keeps the agent's own settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunOptions {
    pub model: Option<String>,
    pub effort: Option<String>,
}

const CLAUDE_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
const OPENCODE_VARIANTS: [&str; 5] = ["minimal", "low", "medium", "high", "max"];

/// The models offered for `agent`: its aliases and its own model list,
/// plus models seen in past sessions.
pub fn models(roots: &Roots, agent: Agent, sessions: &[Session]) -> Vec<ModelChoice> {
    let efforts = |list: &[&str]| list.iter().map(|effort| effort.to_string()).collect::<Vec<_>>();
    let mut choices: Vec<ModelChoice> = match agent {
        Agent::Claude => ["fable", "opus", "sonnet", "haiku"]
            .into_iter()
            .map(|alias| ModelChoice {
                id: alias.to_string(),
                label: format!("{}{} (latest)", alias[..1].to_uppercase(), &alias[1..]),
                efforts: efforts(&CLAUDE_EFFORTS),
                default_effort: None,
                is_default: false,
            })
            .collect(),
        Agent::Codex => codex_models(&roots.codex),
        Agent::Kimi => kimi_models(&roots.kimi),
        Agent::Glm => Vec::new(),
        Agent::OpenCode => opencode_models()
            .into_iter()
            .map(|id| ModelChoice {
                label: id.clone(),
                id,
                efforts: efforts(&OPENCODE_VARIANTS),
                default_effort: None,
                is_default: false,
            })
            .collect(),
    };
    // Full model names used before (Claude logs them; a pinned id stays pickable).
    let mut seen: Vec<String> = sessions
        .iter()
        .filter(|session| session.agent == agent)
        .filter_map(|session| session.model.clone())
        .collect();
    seen.sort();
    seen.dedup();
    for model in seen {
        if !choices.iter().any(|choice| choice.id == model) {
            let efforts = choices.first().map(|choice| choice.efforts.clone()).unwrap_or_default();
            choices.push(ModelChoice { label: model.clone(), id: model, efforts, default_effort: None, is_default: false });
        }
    }
    choices
}

fn kimi_models(kimi: &Path) -> Vec<ModelChoice> {
    let config = std::fs::read_to_string(kimi.join("config.toml")).unwrap_or_default();
    config
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == "default_model").then(|| value.trim().trim_matches('"').to_string())
        })
        .map(|id| vec![ModelChoice { label: id.clone(), id, efforts: Vec::new(), default_effort: None, is_default: true }])
        .unwrap_or_default()
}

/// `~/.codex/models_cache.json`: what the Codex server offers, with each
/// model's efforts; the default comes from `config.toml`.
fn codex_models(codex: &Path) -> Vec<ModelChoice> {
    let config = std::fs::read_to_string(codex.join("config.toml")).unwrap_or_default();
    let configured = |key: &str| {
        config.lines().find_map(|line| {
            let (name, value) = line.split_once('=')?;
            (name.trim() == key).then(|| value.trim().trim_matches('"').to_string())
        })
    };
    let default_model = configured("model");
    let Ok(source) = std::fs::read_to_string(codex.join("models_cache.json")) else {
        return default_model
            .map(|id| vec![ModelChoice { label: id.clone(), id, efforts: Vec::new(), default_effort: configured("model_reasoning_effort"), is_default: true }])
            .unwrap_or_default();
    };
    let value: Value = serde_json::from_str(&source).unwrap_or(Value::Null);
    value
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|model| model.get("visibility").and_then(Value::as_str) != Some("hide"))
        .filter_map(|model| {
            let id = model.get("slug")?.as_str()?.to_string();
            let efforts = model
                .get("supported_reasoning_levels")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|level| level.get("effort").and_then(Value::as_str).map(str::to_string))
                .collect();
            let is_default = default_model.as_deref() == Some(id.as_str());
            Some(ModelChoice {
                label: model.get("display_name").and_then(Value::as_str).unwrap_or(&id).to_string(),
                default_effort: if is_default { configured("model_reasoning_effort") } else { None }
                    .or_else(|| model.get("default_reasoning_level").and_then(Value::as_str).map(str::to_string)),
                is_default,
                efforts,
                id,
            })
        })
        .collect()
}

/// `opencode models`, asked once per run of the app.
fn opencode_models() -> Vec<String> {
    static MODELS: OnceLock<Vec<String>> = OnceLock::new();
    MODELS
        .get_or_init(|| {
            let Some(program) = override_for(Agent::OpenCode).or_else(|| which("opencode")) else {
                return Vec::new();
            };
            files_sdk::log::command(program)
                .arg("models")
                .stdin(Stdio::null())
                .output()
                .map(|output| {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .map(str::trim)
                        .filter(|line| line.contains('/') && !line.contains(' '))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        })
        .clone()
}

/// The CLI flags that choose `options` for `agent`.
pub fn option_args(agent: Agent, options: &RunOptions) -> Vec<String> {
    let mut args = Vec::new();
    match agent {
        Agent::Claude => {
            if let Some(model) = &options.model {
                args.extend(["--model".to_string(), model.clone()]);
            }
            if let Some(effort) = &options.effort {
                args.extend(["--effort".to_string(), effort.clone()]);
            }
        }
        Agent::Codex => {
            if let Some(model) = &options.model {
                args.extend(["-m".to_string(), model.clone()]);
            }
            if let Some(effort) = &options.effort {
                args.extend(["-c".to_string(), format!("model_reasoning_effort=\"{effort}\"")]);
            }
        }
        Agent::OpenCode => {
            if let Some(model) = &options.model {
                args.extend(["--model".to_string(), model.clone()]);
            }
            if let Some(effort) = &options.effort {
                args.extend(["--variant".to_string(), effort.clone()]);
            }
        }
        Agent::Kimi => {
            if let Some(model) = &options.model {
                args.extend(["--model".to_string(), model.clone()]);
            }
        }
        Agent::Glm => {}
    }
    args
}

// ------------------------------------------------------------ running

/// A session started from Files: the agent's CLI in streaming mode.
pub struct Run {
    pub child: Child,
    pub agent: Agent,
}

/// Starts `prompt` in `cwd`, continuing `resume` when given. Read the
/// output with [`stream::read`].
pub fn start(agent: Agent, cwd: &Path, prompt: &str, resume: Option<&str>, options: &RunOptions) -> io::Result<Run> {
    files_sdk::permissions::check(MANIFEST.id, Permission::LaunchApps)?;
    files_sdk::permissions::check(MANIFEST.id, Permission::Network)?;
    let program = override_for(agent)
        .or_else(|| which(agent.program()))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{} is not installed", agent.name())))?;
    let mut command = files_sdk::log::command(program);
    command.current_dir(cwd);
    match agent {
        Agent::Claude => {
            command.args(["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages"]);
            command.args(option_args(agent, options));
            if let Some(id) = resume {
                command.args(["--resume", id]);
            }
            command.arg(prompt);
        }
        Agent::Codex => {
            command.arg("exec").arg("--json").arg("--skip-git-repo-check");
            command.args(option_args(agent, options));
            if let Some(id) = resume {
                command.args(["resume", id]);
            }
            command.arg(prompt);
        }
        Agent::OpenCode => {
            command.args(["run", "--format", "json"]);
            command.args(option_args(agent, options));
            if let Some(id) = resume {
                command.args(["--session", id]);
            }
            command.arg(prompt);
        }
        Agent::Kimi => {
            command.args(["--prompt", prompt, "--output-format", "stream-json"]);
            command.args(option_args(agent, options));
            if let Some(id) = resume {
                command.args(["--session", id]);
            }
        }
        Agent::Glm => return Err(io::Error::new(io::ErrorKind::Unsupported, "GLM / ZCode sessions are read-only in Files")),
    }
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    files_sdk::log::spawned(MANIFEST.id, &command);
    let child = command.spawn()?;
    Ok(Run { child, agent })
}

/// Reads a run line by line, calling `event` for each parsed event.
pub fn follow(run: &mut Run, mut event: impl FnMut(stream::Event)) -> io::Result<()> {
    let stdout = run.child.stdout.take().ok_or_else(|| io::Error::other("no output"))?;
    for line in io::BufReader::new(stdout).lines() {
        for parsed in stream::parse_line(run.agent, &line?) {
            event(parsed);
        }
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = run.child.stderr.take() {
        let _ = io::Read::read_to_string(&mut pipe, &mut stderr);
    }
    let status = run.child.wait()?;
    if !status.success() {
        let message = stderr.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("failed").trim().to_string();
        event(stream::Event::Error(message));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
