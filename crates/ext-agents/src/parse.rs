//! Reading the session logs of Claude Code and Codex (JSON lines).

use std::collections::HashSet;
use std::io::BufRead as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{Account, Agent, ChatMessage, Limit, Role, Session, Usage};

fn lines(path: &Path) -> impl Iterator<Item = Value> {
    std::fs::File::open(path)
        .ok()
        .map(std::io::BufReader::new)
        .into_iter()
        .flat_map(|reader| reader.lines().map_while(Result::ok))
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// `2026-10-29T07:47:27.473Z` → seconds since the Unix epoch (UTC).
pub(crate) fn iso_seconds(value: &str) -> Option<i64> {
    let (date, time) = value.split_once('T')?;
    let mut date = date.split('-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    let time = time.trim_end_matches('Z');
    let time = time.split(['+', '.']).next()?;
    let mut time = time.split(':').map(|part| part.parse::<i64>().ok());
    let (hour, minute, second) = (time.next()??, time.next()??, time.next().flatten().unwrap_or(0));
    // Days from civil (Howard Hinnant).
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second)
}

/// The first line of a prompt, short enough for the sidebar.
fn title_from(prompt: &str) -> String {
    let line = prompt.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("");
    let mut title: String = line.chars().take(80).collect();
    if line.chars().count() > 80 {
        title.push('…');
    }
    title
}

/// Prompts that are app plumbing, not something the user typed.
fn is_plumbing(prompt: &str) -> bool {
    let prompt = prompt.trim_start();
    prompt.starts_with('<') || prompt.starts_with("Caveat:") || prompt.is_empty()
}

// ------------------------------------------------------------- Claude

pub fn claude_session(path: &Path) -> Option<Session> {
    let mut session = Session {
        agent: Agent::Claude,
        id: path.file_stem()?.to_string_lossy().into_owned(),
        title: String::new(),
        cwd: None,
        model: None,
        updated: 0,
        usage: Usage::default(),
        origin: None,
        running: false,
        external: true,
        source: Some(path.to_path_buf()),
    };
    let mut ai_title = None;
    let mut first_prompt = None;
    // One assistant message is logged once per content block, usage repeated.
    let mut counted = HashSet::new();
    for line in lines(path) {
        if let Some(time) = text(&line, "timestamp").and_then(iso_seconds) {
            session.updated = session.updated.max(time);
        }
        if session.cwd.is_none() {
            session.cwd = text(&line, "cwd").map(PathBuf::from);
        }
        if session.origin.is_none() {
            session.origin = text(&line, "entrypoint").map(str::to_string);
        }
        match text(&line, "type") {
            Some("ai-title") => ai_title = text(&line, "aiTitle").map(str::to_string),
            Some("summary") if ai_title.is_none() => ai_title = text(&line, "summary").map(str::to_string),
            Some("user") if first_prompt.is_none() && line.get("isMeta").and_then(Value::as_bool) != Some(true) => {
                if let Some(prompt) = line.pointer("/message/content").and_then(Value::as_str).filter(|prompt| !is_plumbing(prompt)) {
                    first_prompt = Some(title_from(prompt));
                }
            }
            Some("assistant") => {
                let Some(message) = line.get("message") else { continue };
                if let Some(model) = text(message, "model").filter(|model| !model.starts_with('<')) {
                    session.model = Some(model.to_string());
                }
                let id = text(message, "id").unwrap_or_default().to_string();
                if let Some(usage) = message.get("usage").filter(|_| counted.insert(id)) {
                    session.usage.add(&Usage {
                        input: number(usage, "input_tokens"),
                        output: number(usage, "output_tokens"),
                        cache_read: number(usage, "cache_read_input_tokens"),
                        cache_write: number(usage, "cache_creation_input_tokens"),
                        reasoning: usage.pointer("/output_tokens_details/thinking_tokens").and_then(Value::as_u64).unwrap_or(0),
                        cost: None,
                    });
                }
            }
            _ => {}
        }
    }
    // A log without a single prompt (only hooks or snapshots) is not a session.
    let title = ai_title.or(first_prompt)?;
    session.title = title;
    Some(session)
}

fn short(text: &str, limit: usize) -> String {
    let text = text.trim();
    let mut out: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        out.push('…');
    }
    out
}

pub fn claude_messages(path: &Path) -> Vec<ChatMessage> {
    let mut messages: Vec<ChatMessage> = Vec::new();
    let mut last_assistant: Option<String> = None;
    for line in lines(path) {
        if line.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        match text(&line, "type") {
            Some("user") => {
                last_assistant = None;
                if line.get("isMeta").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                match line.pointer("/message/content") {
                    Some(Value::String(prompt)) if !is_plumbing(prompt) => {
                        messages.push(ChatMessage { role: Role::User, text: prompt.clone() });
                    }
                    Some(Value::Array(blocks)) => {
                        for block in blocks {
                            match text(block, "type") {
                                Some("text") => {
                                    if let Some(prompt) = text(block, "text").filter(|prompt| !is_plumbing(prompt)) {
                                        messages.push(ChatMessage { role: Role::User, text: prompt.to_string() });
                                    }
                                }
                                Some("tool_result") => {
                                    let output = match block.get("content") {
                                        Some(Value::String(output)) => output.clone(),
                                        Some(Value::Array(parts)) => parts.iter().filter_map(|part| text(part, "text")).collect::<Vec<_>>().join("\n"),
                                        _ => String::new(),
                                    };
                                    messages.push(ChatMessage { role: Role::Tool, text: format!("↳ {}", short(&output, 240)) });
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            Some("assistant") => {
                let id = line.pointer("/message/id").and_then(Value::as_str).map(str::to_string);
                let blocks = line.pointer("/message/content").and_then(Value::as_array).cloned().unwrap_or_default();
                for block in blocks {
                    match text(&block, "type") {
                        Some("text") => {
                            let piece = text(&block, "text").unwrap_or_default();
                            let same = id.is_some() && last_assistant == id;
                            match messages.last_mut() {
                                Some(last) if same && last.role == Role::Assistant => {
                                    last.text.push_str(piece);
                                }
                                _ => messages.push(ChatMessage { role: Role::Assistant, text: piece.to_string() }),
                            }
                        }
                        Some("tool_use") => {
                            let name = text(&block, "name").unwrap_or("tool");
                            let detail = block
                                .get("input")
                                .and_then(|input| text(input, "command").or(text(input, "file_path")).or(text(input, "description")))
                                .map(|detail| format!(" · {}", short(detail, 120)))
                                .unwrap_or_default();
                            messages.push(ChatMessage { role: Role::Tool, text: format!("⚙ {name}{detail}") });
                        }
                        _ => {}
                    }
                }
                last_assistant = id;
            }
            _ => {}
        }
    }
    messages.retain(|message| !message.text.trim().is_empty());
    messages
}

// -------------------------------------------------------------- Codex

pub fn codex_session(path: &Path) -> Option<Session> {
    let mut session = Session {
        agent: Agent::Codex,
        id: String::new(),
        title: String::new(),
        cwd: None,
        model: None,
        updated: 0,
        usage: Usage::default(),
        origin: None,
        running: false,
        external: true,
        source: Some(path.to_path_buf()),
    };
    let mut title = None;
    for line in lines(path) {
        if let Some(time) = text(&line, "timestamp").and_then(iso_seconds) {
            session.updated = session.updated.max(time);
        }
        let payload = line.get("payload").cloned().unwrap_or(Value::Null);
        match (text(&line, "type"), text(&payload, "type")) {
            (Some("session_meta"), _) => {
                session.id = text(&payload, "id").or(text(&payload, "session_id")).unwrap_or_default().to_string();
                session.cwd = text(&payload, "cwd").map(PathBuf::from);
                session.origin = text(&payload, "source").or(text(&payload, "originator")).map(str::to_string);
            }
            (Some("turn_context"), _) => {
                if let Some(model) = text(&payload, "model") {
                    session.model = Some(model.to_string());
                }
            }
            (Some("event_msg"), Some("user_message")) if title.is_none() => {
                title = text(&payload, "message").filter(|prompt| !is_plumbing(prompt)).map(title_from);
            }
            (Some("event_msg"), Some("token_count")) => {
                if let Some(total) = payload.pointer("/info/total_token_usage") {
                    let cached = number(total, "cached_input_tokens");
                    session.usage = Usage {
                        // Codex counts cached input inside input_tokens.
                        input: number(total, "input_tokens").saturating_sub(cached),
                        output: number(total, "output_tokens"),
                        cache_read: cached,
                        cache_write: number(total, "cache_write_input_tokens"),
                        reasoning: number(total, "reasoning_output_tokens"),
                        cost: None,
                    };
                }
            }
            _ => {}
        }
    }
    if session.id.is_empty() {
        return None;
    }
    session.title = title?;
    Some(session)
}

pub fn codex_messages(path: &Path) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    for line in lines(path) {
        let Some(payload) = line.get("payload") else { continue };
        match (text(&line, "type"), text(payload, "type")) {
            (Some("event_msg"), Some("user_message")) => {
                if let Some(prompt) = text(payload, "message").filter(|prompt| !is_plumbing(prompt)) {
                    messages.push(ChatMessage { role: Role::User, text: prompt.to_string() });
                }
            }
            (Some("event_msg"), Some("agent_message")) => {
                if let Some(answer) = text(payload, "message") {
                    messages.push(ChatMessage { role: Role::Assistant, text: answer.to_string() });
                }
            }
            (Some("response_item"), Some("function_call")) => {
                let name = text(payload, "name").unwrap_or("tool");
                messages.push(ChatMessage { role: Role::Tool, text: format!("⚙ {name}") });
            }
            (Some("event_msg"), Some("task_complete")) => {
                if let Some(error) = payload.pointer("/error/message").and_then(Value::as_str) {
                    messages.push(ChatMessage { role: Role::Tool, text: format!("⚠ {error}") });
                }
            }
            _ => {}
        }
    }
    messages
}

// --------------------------------------------------------------- Kimi

pub fn kimi_session(wire: &Path) -> Option<Session> {
    let state_path = wire.parent()?.parent()?.parent()?.join("state.json");
    let state: Value = serde_json::from_str(&std::fs::read_to_string(state_path).ok()?).ok()?;
    if state.get("archived").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let mut session = Session {
        agent: Agent::Kimi,
        id: text(&state, "id")?.to_string(),
        title: text(&state, "title").or(text(&state, "lastPrompt")).filter(|title| !title.is_empty())?.to_string(),
        cwd: text(&state, "cwd").map(PathBuf::from),
        model: None,
        updated: number(&state, "updatedAt") as i64 / 1000,
        usage: Usage::default(),
        origin: Some("kimi".to_string()),
        running: false,
        external: true,
        source: Some(wire.to_path_buf()),
    };
    for line in lines(wire) {
        session.updated = session.updated.max(number(&line, "time") as i64 / 1000);
        if text(&line, "type") == Some("usage.record") {
            session.model = text(&line, "model").map(str::to_string).or(session.model);
            let usage = line.get("usage").unwrap_or(&Value::Null);
            session.usage.add(&Usage {
                input: number(usage, "inputOther"),
                output: number(usage, "output"),
                cache_read: number(usage, "inputCacheRead"),
                cache_write: number(usage, "inputCacheCreation"),
                reasoning: 0,
                cost: None,
            });
        }
    }
    Some(session)
}

pub fn kimi_messages(path: &Path) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    for line in lines(path) {
        if text(&line, "type") != Some("agent.message.appended") {
            continue;
        }
        let Some(message) = line.pointer("/message/message") else { continue };
        let role = match text(message, "role") {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            Some("tool") => Role::Tool,
            _ => continue,
        };
        if role != Role::Tool {
            let content = message.get("content").unwrap_or(&Value::Null);
            let text = match content {
                Value::String(text) => text.clone(),
                Value::Array(blocks) => blocks.iter().filter(|block| text(block, "type") == Some("text")).filter_map(|block| text(block, "text")).collect::<Vec<_>>().join("\n"),
                _ => String::new(),
            };
            if !text.trim().is_empty() {
                messages.push(ChatMessage { role, text });
            }
        }
        if role == Role::Tool {
            let output = match message.get("content") {
                Some(Value::String(output)) => output.clone(),
                Some(Value::Array(blocks)) => blocks.iter().filter_map(|block| text(block, "text")).collect::<Vec<_>>().join("\n"),
                _ => String::new(),
            };
            if !output.trim().is_empty() {
                messages.push(ChatMessage { role, text: format!("↳ {}", short(&output, 240)) });
            }
        }
        for call in message.get("toolCalls").or(message.get("tool_calls")).and_then(Value::as_array).into_iter().flatten() {
            let name = text(call, "name").or_else(|| call.pointer("/function/name").and_then(Value::as_str)).unwrap_or("tool");
            let arguments = call.get("arguments").cloned().or_else(|| call.pointer("/function/arguments").cloned());
            let detail = arguments
                .and_then(|arguments| match arguments {
                    Value::String(arguments) => serde_json::from_str::<Value>(&arguments).ok(),
                    value => Some(value.clone()),
                })
                .and_then(|arguments| text(&arguments, "command").or(text(&arguments, "path")).or(text(&arguments, "file_path")).map(str::to_string))
                .map(|detail| format!(" · {}", short(&detail, 120)))
                .unwrap_or_default();
            messages.push(ChatMessage { role: Role::Tool, text: format!("⚙ {name}{detail}") });
        }
    }
    messages
}

fn window_name(minutes: u64) -> String {
    match minutes {
        0 => "Window".to_string(),
        minutes if minutes % 1440 == 0 => format!("{} days", minutes / 1440),
        minutes if minutes % 60 == 0 => format!("{} hours", minutes / 60),
        minutes => format!("{minutes} min"),
    }
}

/// The newest rate-limit report in a Codex log.
pub fn codex_limits(path: &Path) -> Option<Account> {
    let mut report = None;
    for line in lines(path) {
        if line.pointer("/payload/type").and_then(Value::as_str) == Some("token_count") {
            // Reports without a window or a plan (e.g. after an error) say nothing.
            let useful = |limits: &&Value| {
                !limits.is_null()
                    && (limits.get("primary").is_some_and(|window| !window.is_null())
                        || limits.get("plan_type").is_some_and(|plan| !plan.is_null()))
            };
            if let Some(limits) = line.pointer("/payload/rate_limits").filter(useful) {
                report = Some(limits.clone());
            }
        }
    }
    let report = report?;
    let mut limits = Vec::new();
    for key in ["primary", "secondary"] {
        if let Some(window) = report.get(key).filter(|window| !window.is_null()) {
            limits.push(Limit {
                name: window_name(number(window, "window_minutes")),
                used_percent: window.get("used_percent").and_then(Value::as_f64).unwrap_or(0.),
                resets_at: window.get("resets_at").and_then(Value::as_i64),
            });
        }
    }
    let mut details = Vec::new();
    if let Some(credits) = report.get("credits").filter(|credits| !credits.is_null()) {
        let unlimited = credits.get("unlimited").and_then(Value::as_bool) == Some(true);
        let balance = credits.get("balance").and_then(|balance| balance.as_f64().or_else(|| balance.as_str()?.parse().ok()));
        details.push((
            "Credits".to_string(),
            if unlimited { "unlimited".to_string() } else { balance.map_or_else(|| "none".to_string(), |balance| format!("{balance:.2}")) },
        ));
    }
    if let Some(reached) = text(&report, "rate_limit_reached_type") {
        details.push(("Limit reached".to_string(), reached.replace('_', " ")));
    }
    Some(Account {
        plan: text(&report, "plan_type").map(str::to_string),
        organization: None,
        details,
        limits,
    })
}
