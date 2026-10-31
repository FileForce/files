//! The event journal and notifications, shared by Files and extensions.
//!
//! Everything goes into one ring buffer: commands extensions run (with
//! arguments, exit code, duration and output), file operations, network
//! calls, status messages and errors. The footer console shows it, Settings →
//! Developer adds a shell on top. Notifications are a separate list the user
//! reads and dismisses (the bell in the footer).

use std::collections::VecDeque;
use std::io;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Events kept; older ones drop out.
const CAPACITY: usize = 2000;
const NOTIFICATIONS: usize = 200;
/// Captured output kept per command.
const OUTPUT_LIMIT: usize = 4000;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Kind {
    /// A program was run (git, ssh, an agent's CLI…).
    Process,
    /// Files were created, moved, deleted.
    Fs,
    Network,
    /// A status-bar message.
    Notice,
    /// A user notification was posted.
    Notification,
    Error,
    /// Anything else an extension reports.
    Info,
}

impl Kind {
    pub const ALL: [Self; 7] = [Self::Process, Self::Fs, Self::Network, Self::Notice, Self::Notification, Self::Error, Self::Info];

    pub fn label(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Fs => "fs",
            Self::Network => "network",
            Self::Notice => "notice",
            Self::Notification => "notification",
            Self::Error => "error",
            Self::Info => "info",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.label() == label)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub id: u64,
    /// Milliseconds since the Unix epoch.
    pub time: u64,
    /// `files` or an extension id.
    pub source: String,
    pub kind: Kind,
    pub message: String,
    /// Arguments, output, paths… shown when the row is expanded.
    pub detail: Option<String>,
    pub duration_ms: Option<u64>,
    pub ok: bool,
    /// For programs: the exact line to paste in a terminal to run it again
    /// (`cd 'folder' && program 'arg'…`).
    pub command: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Level {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub id: u64,
    pub time: u64,
    pub source: String,
    pub level: Level,
    pub title: String,
    pub body: Option<String>,
    pub read: bool,
    /// Buttons on the toast and in the list, e.g. "Open release notes".
    pub actions: Vec<Action>,
}

/// What a notification's button does.
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub label: String,
    /// Opened in the browser; a `files:` link is a command the app runs
    /// (`files:update/install`…).
    pub url: String,
}

#[derive(Default)]
struct Journal {
    events: VecDeque<Event>,
    notifications: VecDeque<Notification>,
}

fn journal() -> &'static Mutex<Journal> {
    static JOURNAL: OnceLock<Mutex<Journal>> = OnceLock::new();
    JOURNAL.get_or_init(Default::default)
}

fn lock() -> std::sync::MutexGuard<'static, Journal> {
    journal().lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Bumps whenever something is recorded: the UI redraws when it changes.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_millis() as u64)
}

fn push(mut event: Event) -> u64 {
    event.id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let id = event.id;
    let mut journal = lock();
    if journal.events.len() >= CAPACITY {
        journal.events.pop_front();
    }
    journal.events.push_back(event);
    drop(journal);
    GENERATION.fetch_add(1, Ordering::Relaxed);
    id
}

/// Windows: no console window flashing up for every program the app runs
/// (the app itself has none, so each child would get its own).
pub fn hide_window(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Records an event.
pub fn record(source: &str, kind: Kind, message: impl Into<String>, detail: Option<String>) -> u64 {
    push(Event {
        id: 0,
        time: now_ms(),
        source: source.to_string(),
        kind,
        message: message.into(),
        detail,
        duration_ms: None,
        ok: kind != Kind::Error,
        command: None,
    })
}

/// Every event, oldest first.
pub fn events() -> Vec<Event> {
    lock().events.iter().cloned().collect()
}

pub fn clear() {
    lock().events.clear();
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Runs `command`, recording the program, its arguments, folder, exit code,
/// duration and (shortened) output. Extensions run every program this way,
/// so the developer console sees all of them.
pub fn run(source: &str, command: &mut Command) -> io::Result<Output> {
    let line = describe(command);
    let shell = shell_line(command);
    let started = Instant::now();
    let result = hide_window(command).output();
    let duration_ms = started.elapsed().as_millis() as u64;
    let (ok, detail) = match &result {
        Ok(output) => {
            let mut detail = format!("{line}\nexit: {}", output.status.code().map_or("signal".to_string(), |code| code.to_string()));
            for (name, bytes) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
                let text = String::from_utf8_lossy(bytes);
                if !text.trim().is_empty() {
                    let mut text: String = text.chars().take(OUTPUT_LIMIT).collect();
                    if bytes.len() > OUTPUT_LIMIT {
                        text.push_str("\n…");
                    }
                    detail.push_str(&format!("\n--- {name}\n{}", text.trim_end()));
                }
            }
            (output.status.success(), detail)
        }
        Err(error) => (false, format!("{line}\nfailed to start: {error}")),
    };
    push(Event {
        id: 0,
        time: now_ms(),
        source: source.to_string(),
        kind: Kind::Process,
        message: line,
        detail: Some(detail),
        duration_ms: Some(duration_ms),
        ok,
        command: Some(shell),
    });
    result
}

/// Like [`run`], but kills the program if it takes longer than `timeout`
/// (the error then has kind `TimedOut`). Use it for anything that can hang:
/// renderers, converters, network calls.
pub fn run_with_timeout(source: &str, command: &mut Command, timeout: std::time::Duration) -> io::Result<Output> {
    use std::io::Read as _;
    let line = describe(command);
    let shell = shell_line(command);
    let started = Instant::now();
    let child = hide_window(command).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            record_run(source, line, shell.clone(), format!("failed to start: {error}"), started, false);
            return Err(error);
        }
    };
    // Drain the pipes on their own threads so a chatty program cannot block.
    let drain = |pipe: Option<Box<dyn io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = drain(child.stdout.take().map(|pipe| Box::new(pipe) as Box<dyn io::Read + Send>));
    let stderr = drain(child.stderr.take().map(|pipe| Box::new(pipe) as Box<dyn io::Read + Send>));
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let message = format!("timed out after {} s and was stopped", timeout.as_secs_f32());
            record_run(source, line, shell.clone(), message.clone(), started, false);
            return Err(io::Error::new(io::ErrorKind::TimedOut, message));
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    };
    let output = Output { status, stdout: stdout.join().unwrap_or_default(), stderr: stderr.join().unwrap_or_default() };
    let mut detail = format!("exit: {}", status.code().map_or("signal".to_string(), |code| code.to_string()));
    for (name, bytes) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
        let text = String::from_utf8_lossy(bytes);
        if !text.trim().is_empty() {
            detail.push_str(&format!("\n--- {name}\n{}", text.chars().take(OUTPUT_LIMIT).collect::<String>().trim_end()));
        }
    }
    record_run(source, line, shell.clone(), detail, started, status.success());
    Ok(output)
}

fn record_run(source: &str, line: String, shell: String, detail: String, started: Instant, ok: bool) {
    push(Event {
        id: 0,
        time: now_ms(),
        source: source.to_string(),
        kind: Kind::Process,
        detail: Some(format!("{line}\n{detail}")),
        message: line,
        duration_ms: Some(started.elapsed().as_millis() as u64),
        ok,
        command: Some(shell),
    });
}

/// Records a program started in the background (its output is streamed).
pub fn spawned(source: &str, command: &Command) {
    let shell = shell_line(command);
    push(Event {
        id: 0,
        time: now_ms(),
        source: source.to_string(),
        kind: Kind::Process,
        message: describe(command),
        detail: Some(format!("{shell}\nstarted in the background; its output is streamed")),
        duration_ms: None,
        ok: true,
        command: Some(shell),
    });
}

/// Quotes one argument for a POSIX shell (only when needed).
pub fn shell_quote(arg: &str) -> String {
    let safe = !arg.is_empty() && arg.chars().all(|char| char.is_ascii_alphanumeric() || "-_./=:,@%+".contains(char));
    if safe { arg.to_string() } else { format!("'{}'", arg.replace('\'', "'\\''")) }
}

/// The command as one line a terminal runs as is: its folder, the
/// environment it was given, the program and its arguments.
pub fn shell_line(command: &Command) -> String {
    let mut line = String::new();
    if let Some(dir) = command.get_current_dir() {
        line.push_str(&format!("cd {} && ", shell_quote(&dir.to_string_lossy())));
    }
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            line.push_str(&format!("{}={} ", key.to_string_lossy(), shell_quote(&value.to_string_lossy())));
        }
    }
    line.push_str(&shell_quote(&command.get_program().to_string_lossy()));
    for arg in command.get_args() {
        line.push(' ');
        line.push_str(&shell_quote(&arg.to_string_lossy()));
    }
    line
}

/// `program arg1 'arg two'  (in /folder)`.
pub fn describe(command: &Command) -> String {
    let program = std::path::Path::new(command.get_program())
        .file_name()
        .map_or_else(|| command.get_program().to_string_lossy().into_owned(), |name| name.to_string_lossy().into_owned());
    let mut line = program;
    for arg in command.get_args() {
        line.push(' ');
        line.push_str(&shell_quote(&arg.to_string_lossy()));
    }
    if let Some(dir) = command.get_current_dir() {
        line.push_str(&format!("  (in {})", dir.display()));
    }
    line
}

// ------------------------------------------------------------ notifications

/// Posts a notification (and records it in the journal).
pub fn notify(source: &str, level: Level, title: impl Into<String>, body: Option<String>) -> u64 {
    notify_with(source, level, title, body, None)
}

/// [`notify`] with a button.
pub fn notify_with(source: &str, level: Level, title: impl Into<String>, body: Option<String>, action: Option<Action>) -> u64 {
    notify_actions(source, level, title, body, action.into_iter().collect())
}

/// [`notify`] with several buttons.
pub fn notify_actions(source: &str, level: Level, title: impl Into<String>, body: Option<String>, actions: Vec<Action>) -> u64 {
    let title = title.into();
    record(source, Kind::Notification, title.clone(), body.clone());
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let mut journal = lock();
    if journal.notifications.len() >= NOTIFICATIONS {
        journal.notifications.pop_front();
    }
    journal.notifications.push_back(Notification { id, time: now_ms(), source: source.to_string(), level, title, body, read: false, actions });
    drop(journal);
    GENERATION.fetch_add(1, Ordering::Relaxed);
    id
}

/// Newest first.
pub fn notifications() -> Vec<Notification> {
    lock().notifications.iter().rev().cloned().collect()
}

pub fn unread() -> usize {
    lock().notifications.iter().filter(|notification| !notification.read).count()
}

pub fn mark_all_read() {
    lock().notifications.iter_mut().for_each(|notification| notification.read = true);
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

pub fn dismiss(id: u64) {
    lock().notifications.retain(|notification| notification.id != id);
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

pub fn clear_notifications() {
    lock().notifications.clear();
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_are_recorded_with_arguments_output_and_exit_code() {
        let before = generation();
        let mut command = Command::new("sh");
        command.args(["-c", "echo hello; echo oops >&2; exit 3"]);
        let output = run("test-ext", &mut command).unwrap();
        assert_eq!(output.status.code(), Some(3));
        assert!(generation() > before);
        let event = events().into_iter().rev().find(|event| event.source == "test-ext").unwrap();
        assert_eq!(event.kind, Kind::Process);
        assert!(!event.ok);
        assert_eq!(event.message, "sh -c 'echo hello; echo oops >&2; exit 3'");
        let detail = event.detail.unwrap();
        assert!(detail.contains("exit: 3") && detail.contains("--- stdout\nhello") && detail.contains("--- stderr\noops"), "{detail}");
        assert!(event.duration_ms.is_some());
        assert_eq!(event.command.as_deref(), Some("sh -c 'echo hello; echo oops >&2; exit 3'"));
    }

    #[test]
    fn command_lines_paste_into_a_shell() {
        let tricky = "cd -- '/it''s here' && ls";
        let mut command = Command::new("printf");
        command.current_dir("/tmp").env("LC_ALL", "C").args(["[%s]", "-p", "22", "a b", tricky]);
        let line = shell_line(&command);
        assert!(line.starts_with("cd /tmp && LC_ALL=C printf "), "{line}");
        // Pasted into a shell, it gives the program exactly the same arguments.
        let pasted = Command::new("sh").args(["-c", &line]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&pasted.stdout), format!("[-p][22][a b][{tricky}]"));
        spawned("test-spawn", &command);
        let event = events().into_iter().rev().find(|event| event.source == "test-spawn").unwrap();
        assert_eq!(event.command.as_deref(), Some(line.as_str()), "background programs keep their full command");
        assert!(event.detail.unwrap().starts_with(&line));
    }

    #[test]
    fn a_hanging_program_is_stopped_on_time() {
        let started = Instant::now();
        let mut command = Command::new("sleep");
        command.arg("30");
        let error = run_with_timeout("test-timeout", &mut command, std::time::Duration::from_millis(300)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "not killed");
        let event = events().into_iter().rev().find(|event| event.source == "test-timeout").unwrap();
        assert!(!event.ok && event.detail.unwrap().contains("timed out"));

        let mut quick = Command::new("sh");
        quick.args(["-c", "echo fine"]);
        let output = run_with_timeout("test-timeout", &mut quick, std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "fine");
    }

    #[test]
    fn notifications_are_read_and_dismissed() {
        let id = notify("test-ext", Level::Success, "Fetched", Some("origin".into()));
        assert!(notifications().iter().any(|notification| notification.id == id && !notification.read));
        assert!(unread() >= 1);
        assert!(events().iter().any(|event| event.kind == Kind::Notification && event.message == "Fetched"));
        mark_all_read();
        assert!(notifications().iter().all(|notification| notification.read));
        dismiss(id);
        assert!(!notifications().iter().any(|notification| notification.id == id));
        assert_eq!(Kind::from_label("process"), Some(Kind::Process));
    }
}
