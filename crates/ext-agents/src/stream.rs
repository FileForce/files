//! The streaming output of each agent's CLI, as one event type.

use serde_json::Value;

use crate::{Agent, Usage};

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The agent's id for this conversation (to reply in it later).
    Session(String),
    /// More of the answer, as it is generated.
    Text(String),
    /// A finished block of answer text (when the CLI does not stream deltas).
    Message(String),
    /// The agent used a tool.
    Tool(String),
    /// The turn finished.
    Done { usage: Usage },
    Error(String),
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// Events in one line of output (most lines give zero or one).
pub fn parse_line(agent: Agent, line: &str) -> Vec<Event> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    match agent {
        Agent::Claude => claude(&value),
        Agent::Codex => codex(&value),
        Agent::OpenCode => opencode(&value),
    }
}

/// `claude -p --output-format stream-json --include-partial-messages`.
fn claude(value: &Value) -> Vec<Event> {
    let mut events = Vec::new();
    match text(value, "type") {
        Some("system") if text(value, "subtype") == Some("init") => {
            if let Some(id) = text(value, "session_id") {
                events.push(Event::Session(id.to_string()));
            }
        }
        Some("stream_event") => {
            let event = value.get("event").unwrap_or(&Value::Null);
            if text(event, "type") == Some("content_block_delta") {
                if let Some(delta) = event.pointer("/delta/text").and_then(Value::as_str) {
                    events.push(Event::Text(delta.to_string()));
                }
            }
        }
        Some("assistant") => {
            for block in value.pointer("/message/content").and_then(Value::as_array).into_iter().flatten() {
                if text(block, "type") == Some("tool_use") {
                    events.push(Event::Tool(text(block, "name").unwrap_or("tool").to_string()));
                }
            }
        }
        Some("result") => {
            let usage = value.get("usage").unwrap_or(&Value::Null);
            if value.get("is_error").and_then(Value::as_bool) == Some(true) {
                events.push(Event::Error(text(value, "result").unwrap_or("failed").to_string()));
            }
            events.push(Event::Done {
                usage: Usage {
                    input: number(usage, "input_tokens"),
                    output: number(usage, "output_tokens"),
                    cache_read: number(usage, "cache_read_input_tokens"),
                    cache_write: number(usage, "cache_creation_input_tokens"),
                    reasoning: 0,
                    cost: value.get("total_cost_usd").and_then(Value::as_f64),
                },
            });
        }
        _ => {}
    }
    events
}

/// `codex exec --json`.
fn codex(value: &Value) -> Vec<Event> {
    let mut events = Vec::new();
    match text(value, "type") {
        Some("thread.started") => {
            if let Some(id) = text(value, "thread_id") {
                events.push(Event::Session(id.to_string()));
            }
        }
        Some("item.completed") => {
            let item = value.get("item").unwrap_or(&Value::Null);
            match text(item, "type").or(text(item, "item_type")) {
                Some("agent_message" | "assistant_message") => {
                    if let Some(answer) = text(item, "text") {
                        events.push(Event::Message(answer.to_string()));
                    }
                }
                Some("command_execution") => {
                    events.push(Event::Tool(text(item, "command").unwrap_or("command").to_string()));
                }
                Some(other) if other != "reasoning" => events.push(Event::Tool(other.replace('_', " "))),
                _ => {}
            }
        }
        Some("turn.completed") => {
            let usage = value.get("usage").unwrap_or(&Value::Null);
            let cached = number(usage, "cached_input_tokens");
            events.push(Event::Done {
                usage: Usage {
                    input: number(usage, "input_tokens").saturating_sub(cached),
                    output: number(usage, "output_tokens"),
                    cache_read: cached,
                    cache_write: 0,
                    reasoning: number(usage, "reasoning_output_tokens"),
                    cost: None,
                },
            });
        }
        Some("turn.failed" | "error") => {
            let message = value.pointer("/error/message").and_then(Value::as_str).or(text(value, "message")).unwrap_or("failed");
            events.push(Event::Error(message.to_string()));
        }
        _ => {}
    }
    events
}

/// `opencode run --format json`.
fn opencode(value: &Value) -> Vec<Event> {
    let mut events = Vec::new();
    if let Some(id) = text(value, "sessionID") {
        events.push(Event::Session(id.to_string()));
    }
    let part = value.get("part").unwrap_or(value);
    match (text(value, "type"), text(part, "type")) {
        (_, Some("text")) => {
            if let Some(answer) = text(part, "text") {
                events.push(Event::Message(answer.to_string()));
            }
        }
        (_, Some("tool")) => events.push(Event::Tool(text(part, "tool").unwrap_or("tool").to_string())),
        (_, Some("step-finish")) => {
            let tokens = part.get("tokens").unwrap_or(&Value::Null);
            events.push(Event::Done {
                usage: Usage {
                    input: number(tokens, "input"),
                    output: number(tokens, "output"),
                    cache_read: tokens.pointer("/cache/read").and_then(Value::as_u64).unwrap_or(0),
                    cache_write: tokens.pointer("/cache/write").and_then(Value::as_u64).unwrap_or(0),
                    reasoning: number(tokens, "reasoning"),
                    cost: part.get("cost").and_then(Value::as_f64),
                },
            });
        }
        (Some("error"), _) => {
            let message = value.pointer("/error/data/message").and_then(Value::as_str).or(text(value, "message")).unwrap_or("failed");
            events.push(Event::Error(message.to_string()));
        }
        _ => {}
    }
    events
}
