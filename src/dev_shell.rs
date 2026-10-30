//! The developer shell in Settings → Developer: inspect the event journal,
//! extensions and permissions, post test notifications, run a program the
//! way extensions do (so it shows in the console too).

use files_sdk::log::{self, Kind, Level};

pub const HELP: &str = "\
help                         this list
log [N] [kind:K] [source:S]  last N events (default 20); K = process, fs, network, notice, notification, error, info
stats                        events by source and kind
clear                        clear the journal
ext                          extensions: enabled, version, permissions
perm <extension>             what an extension may do
notify <text>                post a test notification
run <program> [args…]        run a program through the journal (dev mode only)";

/// Runs one shell line; returns what to print.
pub fn execute(line: &str, settings: &crate::settings::AppSettings) -> String {
    let mut words = line.split_whitespace();
    let Some(command) = words.next() else {
        return String::new();
    };
    let rest: Vec<&str> = words.collect();
    match command {
        "help" | "?" => HELP.to_string(),
        "clear" => {
            log::clear();
            "Journal cleared".to_string()
        }
        "log" => {
            let mut count = 20;
            let mut kind = None;
            let mut source = None;
            for word in &rest {
                if let Some(value) = word.strip_prefix("kind:") {
                    match Kind::from_label(value) {
                        Some(parsed) => kind = Some(parsed),
                        None => return format!("Unknown kind {value}"),
                    }
                } else if let Some(value) = word.strip_prefix("source:") {
                    source = Some(value.to_string());
                } else if let Ok(number) = word.parse() {
                    count = number;
                }
            }
            let events: Vec<_> = log::events()
                .into_iter()
                .filter(|event| kind.is_none_or(|kind| event.kind == kind))
                .filter(|event| source.as_ref().is_none_or(|source| &event.source == source))
                .collect();
            let start = events.len().saturating_sub(count);
            if events.is_empty() {
                return "No events".to_string();
            }
            events[start..]
                .iter()
                .map(|event| {
                    format!(
                        "{} {:<12} {:<12} {}{}{}",
                        clock(event.time),
                        event.source,
                        event.kind.label(),
                        if event.ok { "" } else { "✗ " },
                        event.message,
                        event.duration_ms.map(|ms| format!("  [{ms} ms]")).unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        "stats" => {
            let mut counts = std::collections::BTreeMap::<(String, &str), usize>::new();
            for event in log::events() {
                *counts.entry((event.source, event.kind.label())).or_default() += 1;
            }
            if counts.is_empty() {
                return "No events".to_string();
            }
            counts.into_iter().map(|((source, kind), count)| format!("{source:<14} {kind:<13} {count}")).collect::<Vec<_>>().join("\n")
        }
        "ext" => crate::extensions::CATALOG
            .iter()
            .enumerate()
            .map(|(index, manifest)| {
                let enabled = crate::extensions::enabled(&settings.extensions, index);
                let permissions = manifest
                    .permissions
                    .iter()
                    .map(|permission| {
                        let mark = if files_sdk::permissions::granted(manifest.id, *permission) { "" } else { "✗" };
                        format!("{mark}{}", permission.key())
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                format!("{:<13} {:<4} {:<8} {}", manifest.id, if enabled { "on" } else { "off" }, manifest.version, permissions)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        "perm" => {
            let Some(id) = rest.first() else {
                return "Usage: perm <extension id>".to_string();
            };
            let Some(manifest) = crate::extensions::CATALOG.iter().find(|manifest| manifest.id == *id) else {
                return format!("No extension {id} (see: ext)");
            };
            if manifest.permissions.is_empty() {
                return format!("{id} needs no permissions");
            }
            manifest
                .permissions
                .iter()
                .map(|permission| {
                    let state = if files_sdk::permissions::granted(manifest.id, *permission) { "granted" } else { "revoked" };
                    format!("{:<15} {state:<8} {}", permission.key(), permission.description())
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        "notify" => {
            let text = rest.join(" ");
            if text.is_empty() {
                return "Usage: notify <text>".to_string();
            }
            log::notify("files", Level::Info, text, Some("Posted from the developer shell".to_string()));
            "Notification posted".to_string()
        }
        "run" => {
            if !settings.developer.enabled {
                return "run needs Developer mode (the switch above)".to_string();
            }
            let Some(program) = rest.first() else {
                return "Usage: run <program> [args…]".to_string();
            };
            let mut command = std::process::Command::new(program);
            command.args(&rest[1..]).stdin(std::process::Stdio::null());
            if let Some(home) = dirs::home_dir() {
                command.current_dir(home);
            }
            match log::run("dev-shell", &mut command) {
                Ok(output) => {
                    let mut text = String::from_utf8_lossy(&output.stdout).trim_end().to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if !stderr.trim().is_empty() {
                        text.push_str(&format!("\n{}", stderr.trim_end()));
                    }
                    format!("{text}\n(exit {})", output.status.code().map_or("signal".to_string(), |code| code.to_string()))
                }
                Err(error) => format!("Cannot run {program}: {error}"),
            }
        }
        other => format!("Unknown command {other}. Type help."),
    }
}

/// `HH:MM:SS.mmm` in local time.
pub fn clock(millis: u64) -> String {
    let time = chrono::DateTime::from_timestamp_millis(millis as i64).map(|time| time.with_timezone(&chrono::Local));
    time.map_or_else(|| "--:--:--".to_string(), |time| time.format("%H:%M:%S%.3f").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_commands() {
        let mut settings = crate::settings::AppSettings::default();
        crate::host::install();
        assert!(execute("help", &settings).contains("notify <text>"));
        assert!(execute("ext", &settings).contains("git "));
        assert!(execute("perm ssh", &settings).contains("network"));
        assert!(execute("perm nope", &settings).starts_with("No extension"));
        assert_eq!(execute("notify Hello there", &settings), "Notification posted");
        assert!(files_sdk::log::notifications().iter().any(|notification| notification.title == "Hello there"));
        assert!(execute("log 50 kind:notification", &settings).contains("Hello there"));
        assert!(execute("log kind:bogus", &settings).starts_with("Unknown kind"));
        assert!(execute("run echo hi", &settings).contains("Developer mode"), "run is dev-mode only");
        settings.developer.enabled = true;
        let output = execute("run echo hi", &settings);
        assert!(output.starts_with("hi") && output.ends_with("(exit 0)"), "{output}");
        assert!(execute("log 5 source:dev-shell", &settings).contains("echo hi"), "runs land in the journal");
        assert!(execute("stats", &settings).contains("dev-shell"));
        assert!(execute("frobnicate", &settings).starts_with("Unknown command"));
    }
}
