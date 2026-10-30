//! The Agents extension in the UI: a sidebar list of Claude Code, Codex and
//! OpenCode sessions, a session window (the chat, with live answers when you
//! reply), and a usage window (tokens, cost, plans and rate limits).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use files_ext_agents::stream::Event as StreamEvent;
use files_ext_agents::{Account, Agent, ChatMessage, Role, Roots, Session, Usage};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, ClipboardItem, Context, Entity, FocusHandle, Focusable, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle, StatefulInteractiveElement as _,
    Styled as _, Subscription, WeakEntity, Window, WindowBounds, WindowOptions, actions, div, point, px, size,
};

use crate::app::{AppearanceSettings, FileExplorer};
use crate::palette::Palette;

actions!(agents, [CloseAgents]);

/// Sessions shown in the sidebar before "All sessions…".
const SIDEBAR_SESSIONS: usize = 8;
/// A long chat opens on its latest messages.
const SHOWN_MESSAGES: usize = 200;

/// Where the agents keep their data. Tests use an empty folder instead of
/// this computer's real sessions.
fn roots() -> Roots {
    #[cfg(test)]
    {
        if let Some(roots) = TEST_ROOTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone() {
            return roots;
        }
        let empty = std::env::temp_dir().join("file-force-no-agents");
        return Roots {
            claude: empty.join("claude"),
            claude_account: empty.join("claude.json"),
            codex: empty.join("codex"),
            opencode_db: empty.join("opencode.db"),
        };
    }
    #[cfg(not(test))]
    Roots::detect()
}

#[cfg(test)]
pub(crate) static TEST_ROOTS: Mutex<Option<Roots>> = Mutex::new(None);

// ------------------------------------------------------------ formatting

pub(crate) fn tokens(count: u64) -> String {
    match count {
        0..=999 => count.to_string(),
        1_000..=999_999 => format!("{:.1}K", count as f64 / 1e3),
        1_000_000..=999_999_999 => format!("{:.1}M", count as f64 / 1e6),
        _ => format!("{:.2}B", count as f64 / 1e9),
    }
}

pub(crate) fn ago(seconds: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_secs() as i64);
    match (now - seconds).max(0) {
        0..=59 => "now".to_string(),
        delta @ 60..=3599 => format!("{}m", delta / 60),
        delta @ 3600..=86_399 => format!("{}h", delta / 3600),
        delta => format!("{}d", delta / 86_400),
    }
}

fn until(seconds: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_secs() as i64);
    match (seconds - now).max(0) {
        0..=59 => "now".to_string(),
        delta @ 60..=3599 => format!("in {} min", delta / 60),
        delta @ 3600..=86_399 => format!("in {} h", delta / 3600),
        delta => format!("in {} days", delta / 86_400),
    }
}

fn agent_color(agent: Agent, palette: Palette) -> Hsla {
    match agent {
        Agent::Claude => palette.archive,
        Agent::Codex => palette.code,
        Agent::OpenCode => palette.media,
    }
}

fn agent_glyph(agent: Agent, palette: Palette, size_px: f32) -> AnyElement {
    let letter = match agent {
        Agent::Claude => "C",
        Agent::Codex => "X",
        Agent::OpenCode => "O",
    };
    div()
        .size(px(size_px))
        .flex_shrink_0()
        .rounded_md()
        .bg(agent_color(agent, palette))
        .text_color(palette.accent_contrast)
        .text_size(px(size_px * 0.55))
        .font_weight(FontWeight::BOLD)
        .flex()
        .items_center()
        .justify_center()
        .child(letter)
        .into_any_element()
}

fn tag(text: &'static str, color: Hsla, palette: Palette) -> AnyElement {
    div()
        .px_1()
        .rounded_sm()
        .border_1()
        .border_color(color)
        .text_size(px(8.))
        .text_color(color)
        .bg(palette.sidebar)
        .child(text)
        .into_any_element()
}

fn folder_name(session: &Session) -> String {
    session
        .cwd
        .as_ref()
        .and_then(|cwd| cwd.file_name())
        .map_or_else(|| "—".to_string(), |name| name.to_string_lossy().into_owned())
}

// ------------------------------------------------------------ sidebar

impl FileExplorer {
    /// Rescans agent sessions in the background (cheap when nothing changed).
    pub(crate) fn refresh_agents(&mut self, cx: &mut Context<Self>) {
        if !self.appearance.read(cx).extensions.agents || self.agents_scanning {
            return;
        }
        self.agents_scanning = true;
        let ours = self.appearance.read(cx).sidebar.agents.clone();
        cx.spawn(async move |this, cx| {
            let sessions = cx
                .background_executor()
                .spawn(async move { files_ext_agents::scan(&roots(), &ours) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.agents_scanning = false;
                if this.agent_sessions != sessions {
                    this.agent_sessions = sessions;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Keeps the list fresh: every 15 s while the window exists.
    pub(crate) fn watch_agents(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                if this.update(cx, |this, cx| this.refresh_agents(cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(std::time::Duration::from_secs(15)).await;
            }
        })
        .detach();
    }

    pub(crate) fn render_agents_section(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.appearance.read(cx).extensions.agents {
            return None;
        }
        let language = self.appearance.read(cx).language;
        let header = h_flex()
            .items_center()
            .child(div().flex_1().child(self.section_header(
                "agents",
                crate::settings::localized(language, "AGENTS", "АГЕНТЫ"),
                palette,
                cx,
            )))
            .child(
                Button::new("agents-usage")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChartPie)
                    .tooltip("Usage, limits and plans")
                    .on_click(cx.listener(|this, _, _, cx| this.open_agents_usage(cx))),
            )
            .child(
                Button::new("agents-new")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .tooltip("New agent session in this folder")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let cwd = this.active_location().path().unwrap_or_else(|| dirs::home_dir().unwrap_or_default());
                        open_agent_window(None, cwd, this.appearance.clone(), cx.entity().downgrade(), this.agent_sessions.clone(), cx);
                    })),
            );
        let mut section = v_flex().gap_0p5().child(header);
        if self.section_collapsed("agents", cx) {
            return Some(section.into_any_element());
        }
        if self.agent_sessions.is_empty() {
            let row = div().px_2().py_1().text_xs();
            section = section.child(if self.agents_scanning {
                row.child(crate::views::text_fx::shimmer("agents-scanning", "Looking for sessions…", palette.faint, palette))
            } else {
                row.text_color(palette.faint).child("No Claude Code, Codex or OpenCode sessions yet")
            });
            return Some(section.into_any_element());
        }
        for (index, session) in self.agent_sessions.iter().take(SIDEBAR_SESSIONS).enumerate() {
            section = section.child(self.agent_row(index, session.clone(), palette, cx));
        }
        let total = self.agent_sessions.len();
        if total > SIDEBAR_SESSIONS {
            section = section.child(
                div()
                    .id("agents-all")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_xs()
                    .text_color(palette.muted)
                    .cursor_pointer()
                    .hover(|style| style.bg(palette.hover).text_color(palette.text))
                    .on_click(cx.listener(|this, _, _, cx| this.open_agents_usage(cx)))
                    .child(format!("All {total} sessions & usage…")),
            );
        }
        Some(section.into_any_element())
    }

    fn agent_row(&self, index: usize, session: Session, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let label = format!(
            "{} session {}{}{}",
            session.agent.name(),
            session.title,
            if session.running { ", running" } else { "" },
            if session.external { ", started outside Files" } else { "" }
        );
        let subtitle = format!("{} · {} · {} tok", folder_name(&session), ago(session.updated), tokens(session.usage.total()));
        let open = session.clone();
        let row = h_flex()
            .id(("agent-session", index))
            .min_h(px(38.))
            .px_2()
            .gap_2()
            .items_center()
            .rounded_md()
            .cursor_pointer()
            .hover(|style| style.bg(palette.hover))
            .aria_label(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                let cwd = open.cwd.clone().unwrap_or_default();
                open_agent_window(Some(open.clone()), cwd, this.appearance.clone(), cx.entity().downgrade(), this.agent_sessions.clone(), cx);
            }))
            .child(agent_glyph(session.agent, palette, 18.))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_xs().text_color(palette.text).truncate().child(session.title.clone()))
                    .child(div().text_size(px(9.)).text_color(palette.faint).truncate().child(subtitle)),
            )
            .when(session.running, |this| {
                this.child(div().size(px(6.)).flex_shrink_0().rounded_full().bg(palette.code))
            })
            .when(session.external, |this| this.child(tag("external", palette.faint, palette)));
        #[cfg(test)]
        let row = row.debug_selector(move || format!("agent-session-{index}"));
        row.into_any_element()
    }

    fn open_agents_usage(&self, cx: &mut Context<Self>) {
        let _ = open_usage_window(self.agent_sessions.clone(), self.appearance.clone(), cx);
    }

    /// "Open in Agents": a new session in Files, the agent's CLI in the
    /// terminal, or the agent's desktop app (its CLI when there is none).
    pub(crate) fn open_in_agents(&mut self, target: AgentTarget, folder: PathBuf, cx: &mut Context<Self>) {
        let app = self.appearance.read(cx).extensions.terminal_app;
        let terminal = |agent: Agent| {
            files_ext_agents::terminal_command(agent, &folder)
                .and_then(|command| crate::views::ssh::run_in_terminal(&command, agent.short(), app))
                .map(|()| format!("{} · terminal · {}", agent.name(), folder.display()))
        };
        let result = match target {
            AgentTarget::Files => {
                open_agent_window(None, folder.clone(), self.appearance.clone(), cx.entity().downgrade(), self.agent_sessions.clone(), cx);
                Ok(format!("New agent session · {}", folder.display()))
            }
            AgentTarget::Terminal(agent) => terminal(agent),
            AgentTarget::App(agent) if agent.has_desktop_app() => files_ext_agents::open_in_desktop(agent, &folder)
                .map(|()| format!("{} · {}", agent.name(), folder.display())),
            AgentTarget::App(agent) => terminal(agent),
        };
        self.notice = Some(result.unwrap_or_else(|error| format!("Open in Agents failed: {error}")));
        cx.notify();
    }

    /// Remembers a session started from Files, so it is not tagged external.
    pub(crate) fn remember_agent_session(&mut self, id: String, cx: &mut Context<Self>) {
        self.update_sidebar(cx, |sidebar| {
            if !sidebar.agents.contains(&id) {
                sidebar.agents.push(id);
            }
        });
        self.refresh_agents(cx);
    }
}

/// Where "Open in Agents" opens a folder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AgentTarget {
    Files,
    Terminal(Agent),
    /// The desktop app, or the CLI in the terminal when it is not installed.
    App(Agent),
}

/// The "Open in Agents" submenu for `folder`.
pub(crate) fn agents_submenu(
    menu: gpui_kit::component::menu::PopupMenu,
    view: Entity<FileExplorer>,
    folder: PathBuf,
    color: Option<Hsla>,
    window: &mut Window,
    cx: &mut Context<gpui_kit::component::menu::PopupMenu>,
) -> gpui_kit::component::menu::PopupMenu {
    let icon = crate::views::context_menu::tinted_icon(crate::extensions::icon("Agents"), color);
    menu.submenu_with_icon(Some(icon), "Open in Agents", window, cx, move |submenu, window, cx| {
        let item = |label: String, target: AgentTarget, enabled: bool, window: &mut Window| {
            let folder = folder.clone();
            let tinted = if enabled { color } else { None };
            crate::views::context_menu::tinted_item(label, crate::extensions::icon("Agents"), tinted)
                .disabled(!enabled)
                .on_click(window.listener_for(&view, move |this, _, _, cx| {
                this.open_in_agents(target, folder.clone(), cx);
            }))
        };
        let terminal_view = view.clone();
        let terminal_folder = folder.clone();
        let submenu = submenu
            .item(item("Open in Files".into(), AgentTarget::Files, true, window))
            .submenu_with_icon(Some(gpui_kit::component::Icon::new(IconName::SquareTerminal)), "Open in Terminal", window, cx, move |terminal, window, _| {
                Agent::ALL.into_iter().fold(terminal, |terminal, agent| {
                    let folder = terminal_folder.clone();
                    terminal.item(
                        crate::views::context_menu::tinted_item(
                            agent.name(),
                            gpui_kit::component::Icon::new(IconName::SquareTerminal),
                            color.filter(|_| agent.installed()),
                        )
                        .disabled(!agent.installed())
                            .on_click(window.listener_for(&terminal_view, move |this, _, _, cx| {
                                this.open_in_agents(AgentTarget::Terminal(agent), folder.clone(), cx);
                            })),
                    )
                })
            })
            .separator();
        Agent::ALL.into_iter().fold(submenu, |submenu, agent| {
            let label = match (agent, agent.has_desktop_app()) {
                (Agent::Claude, true) => "Open in Claude (desktop)".to_string(),
                (_, true) => format!("Open in {} (desktop)", agent.name()),
                (_, false) => format!("Open in {} (terminal)", agent.name()),
            };
            let enabled = agent.has_desktop_app() || agent.installed();
            submenu.item(item(label, AgentTarget::App(agent), enabled, window))
        })
    })
}

// ------------------------------------------------------------ session window

/// What a running turn has produced so far (shared with the reader thread).
#[derive(Default)]
struct Live {
    events: Vec<StreamEvent>,
    done: bool,
}

pub struct AgentWindow {
    appearance: Entity<AppearanceSettings>,
    explorer: WeakEntity<FileExplorer>,
    session: Option<Session>,
    /// For a new session.
    agent: Agent,
    cwd: PathBuf,
    messages: Vec<ChatMessage>,
    show_all: bool,
    loading: bool,
    /// The answer being streamed.
    streaming: String,
    running: bool,
    turn_usage: Usage,
    notice: Option<String>,
    input: Entity<InputState>,
    scroll: ScrollHandle,
    /// The session log as last read; a change means new messages.
    fingerprint: Option<(u64, i64)>,
    reloading: bool,
    /// New messages scroll into view only while the reader stays at the end;
    /// scrolling up turns this off, scrolling back down turns it on.
    follow: bool,
    /// Models offered for `agent`, and the pick (`None`: the agent's default).
    models: Vec<files_ext_agents::ModelChoice>,
    /// Sessions known when the window opened (for models used before).
    known_sessions: Vec<Session>,
    model: Option<String>,
    effort: Option<String>,
    /// When the log last changed while this window watched it.
    last_change: Option<std::time::Instant>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl AgentWindow {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        session: Option<Session>,
        cwd: PathBuf,
        appearance: Entity<AppearanceSettings>,
        explorer: WeakEntity<FileExplorer>,
        known_sessions: Vec<Session>,
    ) -> Self {
        let placeholder = if session.is_some() { "Reply… (Enter to send)" } else { "What should the agent do in this folder?" };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subscriptions = vec![
            cx.observe(&appearance, |_, _, cx| cx.notify()),
            cx.subscribe_in(&input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.send(window, cx);
                }
            }),
        ];
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        let agent = session.as_ref().map_or_else(
            || Agent::ALL.into_iter().find(|agent| agent.installed()).unwrap_or(Agent::Claude),
            |session| session.agent,
        );
        let mut this = Self {
            appearance,
            explorer,
            loading: session.is_some(),
            session,
            agent,
            cwd,
            messages: Vec::new(),
            show_all: false,
            streaming: String::new(),
            running: false,
            turn_usage: Usage::default(),
            notice: None,
            input,
            scroll: ScrollHandle::new(),
            fingerprint: None,
            reloading: false,
            follow: true,
            models: Vec::new(),
            known_sessions,
            model: None,
            effort: None,
            last_change: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        this.reload(cx);
        this.load_models(cx);
        Self::watch(cx);
        this
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|session| session.source.is_some() || session.agent == Agent::OpenCode) else {
            return;
        };
        if self.reloading {
            return;
        }
        self.reloading = true;
        cx.spawn(async move |this, cx| {
            let (messages, fingerprint) = cx
                .background_executor()
                .spawn(async move {
                    let roots = roots();
                    // Fingerprint first: a write landing mid-read is picked up next time.
                    let fingerprint = files_ext_agents::fingerprint(&roots, &session);
                    (files_ext_agents::messages(&roots, &session), fingerprint)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.reloading = false;
                this.loading = false;
                let follow = this.follow || this.messages.is_empty();
                this.fingerprint = fingerprint;
                if this.messages != messages && !this.running {
                    this.messages = messages;
                    if follow {
                        this.scroll.scroll_to_bottom();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Loads the model list of the current agent in the background.
    fn load_models(&mut self, cx: &mut Context<Self>) {
        let agent = self.agent;
        // Passed in, not read from the explorer: it may be the one opening this window.
        let sessions = self.known_sessions.clone();
        cx.spawn(async move |this, cx| {
            let models = cx
                .background_executor()
                .spawn(async move { files_ext_agents::models(&roots(), agent, &sessions) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.agent == agent {
                    this.models = models;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn set_agent(&mut self, agent: Agent, cx: &mut Context<Self>) {
        if self.agent == agent {
            return;
        }
        self.agent = agent;
        self.model = None;
        self.effort = None;
        self.models.clear();
        self.load_models(cx);
        cx.notify();
    }

    /// The efforts the picked model accepts (the default model's when none is picked).
    fn efforts(&self) -> Vec<String> {
        let picked = self.model.as_ref().and_then(|model| self.models.iter().find(|choice| &choice.id == model));
        picked
            .or_else(|| self.models.iter().find(|choice| choice.is_default))
            .or(self.models.first())
            .map(|choice| choice.efforts.clone())
            .unwrap_or_default()
    }

    fn model_label(&self) -> String {
        match &self.model {
            Some(model) => self.models.iter().find(|choice| &choice.id == model).map_or(model.clone(), |choice| choice.label.clone()),
            None => match self.models.iter().find(|choice| choice.is_default) {
                Some(choice) => format!("Default · {}", choice.label),
                None => "Default model".to_string(),
            },
        }
    }

    fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.follow = true;
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    fn at_bottom(&self) -> bool {
        let offset = self.scroll.offset().y;
        let max = self.scroll.max_offset().y;
        -offset >= max - px(48.)
    }

    /// Polls the session log once a second and shows what was added, so a
    /// session running elsewhere (a terminal, an IDE, the agent's app)
    /// streams into this window too.
    fn watch(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                let Ok(Some(session)) = this.update(cx, |this, _| {
                    // Our own turn streams directly; the log is re-read after it.
                    (!this.running && !this.reloading).then(|| this.session.clone()).flatten()
                }) else {
                    if this.upgrade().is_none() {
                        break;
                    }
                    continue;
                };
                let fingerprint =
                    cx.background_executor().spawn(async move { files_ext_agents::fingerprint(&roots(), &session) }).await;
                let alive = this.update(cx, |this, cx| {
                    if fingerprint.is_some() && fingerprint != this.fingerprint {
                        if this.fingerprint.is_some() {
                            this.last_change = Some(std::time::Instant::now());
                        }
                        this.reload(cx);
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// The log changed within the last few seconds: someone is working in it.
    fn is_live(&self) -> bool {
        self.last_change.is_some_and(|time| time.elapsed() < std::time::Duration::from_secs(10))
    }

    /// After a turn: find the log of a session Files just started, then show
    /// the conversation as the agent saved it.
    fn after_turn(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else { return };
        if session.source.is_some() || session.agent == Agent::OpenCode {
            self.reload(cx);
            return;
        }
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move { files_ext_agents::find(&roots(), session.agent, &session.id) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let (Some(found), Some(session)) = (found, this.session.as_mut()) {
                    session.source = found.source;
                    session.model = found.model.or(session.model.take());
                    session.usage = found.usage;
                    this.turn_usage = Usage::default();
                }
                this.reload(cx);
            });
        })
        .detach();
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = self.input.read(cx).value().trim().to_string();
        if prompt.is_empty() || self.running {
            return;
        }
        self.input.update(cx, |input, cx| input.set_value("", window, cx));
        self.messages.push(ChatMessage { role: Role::User, text: prompt.clone() });
        // Sending a message jumps back to the conversation's end.
        self.follow = true;
        self.running = true;
        self.notice = None;
        self.streaming.clear();
        self.scroll.scroll_to_bottom();
        cx.notify();

        let live = Arc::new(Mutex::new(Live::default()));
        let agent = self.agent;
        let cwd = self.cwd.clone();
        let resume = self.session.as_ref().map(|session| session.id.clone());
        let options = files_ext_agents::RunOptions { model: self.model.clone(), effort: self.effort.clone() };
        let writer = live.clone();
        std::thread::spawn(move || {
            let push = |event| writer.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).events.push(event);
            match files_ext_agents::start(agent, &cwd, &prompt, resume.as_deref(), &options) {
                Ok(mut run) => {
                    if let Err(error) = files_ext_agents::follow(&mut run, push) {
                        writer.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).events.push(StreamEvent::Error(error.to_string()));
                    }
                }
                Err(error) => push(StreamEvent::Error(error.to_string())),
            }
            writer.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).done = true;
        });

        let title = self.messages.last().map(|message| message.text.clone()).unwrap_or_default();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(60)).await;
                let (events, done) = {
                    let mut live = live.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    (std::mem::take(&mut live.events), live.done)
                };
                let title = title.clone();
                let alive = this.update(cx, |this, cx| {
                    for event in events {
                        this.apply(event, &title, cx);
                    }
                    if done {
                        this.finish_turn();
                        this.after_turn(cx);
                    }
                    cx.notify();
                });
                if done || alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn apply(&mut self, event: StreamEvent, prompt: &str, cx: &mut Context<Self>) {
        match event {
            StreamEvent::Session(id) => {
                if self.session.is_none() {
                    self.session = Some(Session {
                        agent: self.agent,
                        id: id.clone(),
                        title: prompt.chars().take(80).collect(),
                        cwd: Some(self.cwd.clone()),
                        model: None,
                        updated: 0,
                        usage: Usage::default(),
                        origin: Some("files".to_string()),
                        running: true,
                        external: false,
                        source: None,
                    });
                }
                let _ = self.explorer.update(cx, |explorer, cx| explorer.remember_agent_session(id, cx));
            }
            StreamEvent::Text(delta) => self.streaming.push_str(&delta),
            StreamEvent::Message(text) => {
                self.streaming.clear();
                self.messages.push(ChatMessage { role: Role::Assistant, text });
            }
            StreamEvent::Tool(name) => {
                self.flush_streaming();
                self.messages.push(ChatMessage { role: Role::Tool, text: format!("⚙ {name}") });
            }
            StreamEvent::Done { usage } => {
                self.flush_streaming();
                self.turn_usage.add(&usage);
            }
            StreamEvent::Error(message) => {
                self.flush_streaming();
                self.notice = Some(message);
            }
        }
        if self.follow {
            self.scroll.scroll_to_bottom();
        }
    }

    fn flush_streaming(&mut self) {
        if !self.streaming.trim().is_empty() {
            let text = std::mem::take(&mut self.streaming);
            self.messages.push(ChatMessage { role: Role::Assistant, text });
        }
        self.streaming.clear();
    }

    fn finish_turn(&mut self) {
        self.flush_streaming();
        self.running = false;
        if let Some(session) = &mut self.session {
            session.running = false;
        }
    }

    fn close(&mut self, _: &CloseAgents, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    fn render_message(&self, index: usize, message: &ChatMessage, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        if message.role == Role::Tool {
            // Tool lines stay one compact line, but can still be selected.
            return div()
                .px_3()
                .text_xs()
                .font_family("Menlo")
                .text_color(palette.faint)
                .child(TextView::markdown(("agent-tool", index), escape_markdown(&message.text)).selectable(true))
                .into_any_element();
        }
        let user = message.role == Role::User;
        let group: gpui_kit::SharedString = format!("agent-message-{index}").into();
        let text = message.text.clone();
        let copy = Button::new(("agent-copy", index))
            .ghost()
            .xsmall()
            .icon(IconName::Copy)
            .tooltip("Copy message")
            .on_click(cx.listener(move |_, _, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))));
        let bubble = div()
            .id(("agent-message", index))
            .max_w(gpui_kit::relative(0.86))
            .min_w_0()
            .px_3()
            .py_2()
            .rounded_lg()
            .text_sm()
            .text_color(palette.text)
            .when(user, |bubble| bubble.bg(palette.accent_soft))
            .when(!user, |bubble| bubble.bg(palette.surface).border_1().border_color(palette.border))
            // Markdown (bold, code, lists, tables…) that can be selected and copied.
            .child(TextView::markdown(("agent-markdown", index), message.text.clone()).selectable(true));
        // The copy button shows on hover, on the inner side of the bubble.
        let copy = div().invisible().group_hover(group.clone(), |style| style.visible()).child(copy);
        #[cfg(test)]
        let copy = copy.debug_selector(move || format!("agent-copy-{index}"));
        #[cfg(test)]
        let bubble = bubble.debug_selector(move || format!("agent-bubble-{index}"));
        let row = h_flex().group(group).w_full().gap_1().items_start();
        if user {
            row.justify_end().child(copy).child(bubble).into_any_element()
        } else {
            row.child(bubble).child(copy).into_any_element()
        }
    }
}

#[cfg(test)]
pub(crate) fn escape_markdown_for_test(text: &str) -> String {
    escape_markdown(text)
}

/// Shows text literally inside a markdown view (tool lines are not markdown).
fn escape_markdown(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for char in text.chars() {
        if "\\`*_{}[]<>()#+-.!|~".contains(char) {
            escaped.push('\\');
        }
        escaped.push(char);
    }
    escaped
}

#[cfg(test)]
impl AgentWindow {
    pub(crate) fn set_prompt_for_test(&mut self, prompt: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.set_value(prompt.to_string(), window, cx));
    }

    pub(crate) fn send_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.send(window, cx);
    }

    pub(crate) fn set_messages_for_test(&mut self, messages: Vec<ChatMessage>) {
        self.loading = false;
        self.messages = messages;
    }

    pub(crate) fn set_agent_for_test(&mut self, agent: Agent, cx: &mut Context<Self>) {
        self.set_agent(agent, cx);
    }

    pub(crate) fn pick_for_test(&mut self, model: Option<&str>, effort: Option<&str>) {
        self.model = model.map(str::to_string);
        self.effort = effort.map(str::to_string);
    }


    pub(crate) fn follows_for_test(&self) -> bool {
        self.follow
    }

    pub(crate) fn scroll_offset_for_test(&self) -> gpui_kit::Pixels {
        self.scroll.offset().y
    }

    pub(crate) fn is_live_for_test(&self) -> bool {
        self.is_live()
    }

    pub(crate) fn notice_for_test(&self) -> Option<String> {
        self.notice.clone()
    }

    pub(crate) fn is_running(&self) -> bool {
        self.running
    }

    pub(crate) fn messages_for_test(&self) -> Vec<ChatMessage> {
        self.messages.clone()
    }

    pub(crate) fn session_id_for_test(&self) -> Option<String> {
        self.session.as_ref().map(|session| session.id.clone())
    }
}

impl Focusable for AgentWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AgentWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&settings);
        let new_session = self.session.is_none();

        let header = h_flex()
            .px_4()
            .py_3()
            .gap_3()
            .items_center()
            .border_b_1()
            .border_color(palette.border)
            .child(agent_glyph(self.agent, palette, 28.))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(self.session.as_ref().map_or_else(|| "New session".to_string(), |session| session.title.clone())),
                    )
                    .child(
                        div().text_xs().text_color(palette.muted).truncate().child(format!(
                            "{} · {}{}",
                            self.agent.name(),
                            self.cwd.display(),
                            self.session
                                .as_ref()
                                .and_then(|session| session.model.clone())
                                .map(|model| format!(" · {model}"))
                                .unwrap_or_default()
                        )),
                    ),
            )
            .children(self.session.as_ref().map(|session| {
                h_flex()
                    .gap_1()
                    .when(session.running || self.running || self.is_live(), |this| this.child(tag("live", palette.code, palette)))
                    .when(session.external, |this| this.child(tag("external", palette.faint, palette)))
                    .children(session.origin.clone().map(|origin| {
                        div().text_size(px(9.)).text_color(palette.faint).child(origin)
                    }))
            }));

        // Tokens of the session so far, plus what this window's turns used.
        let mut usage = self.session.as_ref().map(|session| session.usage).unwrap_or_default();
        usage.add(&self.turn_usage);
        let usage_bar = h_flex()
            .px_4()
            .py_1p5()
            .gap_4()
            .text_xs()
            .text_color(palette.muted)
            .border_b_1()
            .border_color(palette.border)
            .child(format!("in {}", tokens(usage.input)))
            .child(format!("out {}", tokens(usage.output)))
            .child(format!("cache read {}", tokens(usage.cache_read)))
            .child(format!("cache write {}", tokens(usage.cache_write)))
            .when(usage.reasoning > 0, |this| this.child(format!("reasoning {}", tokens(usage.reasoning))))
            .children(usage.cost.map(|cost| format!("${cost:.2}")));

        let hidden = if self.show_all { 0 } else { self.messages.len().saturating_sub(SHOWN_MESSAGES) };
        let mut chat = v_flex().p_4().gap_2();
        if hidden > 0 {
            chat = chat.child(
                Button::new("agent-show-all")
                    .ghost()
                    .small()
                    .label(format!("Show {hidden} earlier messages"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_all = true;
                        cx.notify();
                    })),
            );
        }
        for (index, message) in self.messages.iter().enumerate().skip(hidden) {
            chat = chat.child(self.render_message(index, message, palette, cx));
        }
        if !self.streaming.is_empty() {
            let live = ChatMessage { role: Role::Assistant, text: format!("{}▍", self.streaming) };
            chat = chat.child(self.render_message(usize::MAX, &live, palette, cx));
        } else if self.running {
            chat = chat.child(div().px_3().text_xs().child(crate::views::text_fx::shimmer("agent-thinking", "Thinking…", palette.faint, palette)));
        }
        if self.loading {
            chat = chat.child(div().text_xs().child(crate::views::text_fx::shimmer("agent-loading", "Loading the conversation…", palette.faint, palette)));
        } else if self.messages.is_empty() && !self.running {
            chat = chat.child(div().text_xs().text_color(palette.faint).child(if new_session {
                "Pick an agent and describe the task. It runs in the folder above with the agent's own settings."
            } else {
                "No messages in this session"
            }));
        }

        let picker = new_session.then(|| {
            h_flex().gap_1().children(Agent::ALL.into_iter().map(|agent| {
                let installed = agent.installed();
                Button::new(gpui_kit::ElementId::Name(format!("pick-{}", agent.short()).into()))
                    .small()
                    .when(agent == self.agent, |button| button.primary())
                    .when(agent != self.agent, |button| button.ghost())
                    .label(agent.name())
                    .disabled(!installed || self.running)
                    .tooltip(if installed { "Use this agent" } else { "Not installed" })
                    .on_click(cx.listener(move |this, _, _, cx| this.set_agent(agent, cx)))
            }))
        });

        // Model and effort for the next turn; "Default" keeps the agent's own settings.
        let entity = cx.entity();
        let models = self.models.clone();
        let picked_model = self.model.clone();
        let model_menu = Button::new("agent-model")
            .ghost()
            .small()
            .icon(IconName::Bot)
            .label(self.model_label())
            .disabled(self.running)
            .tooltip("Model for the next message")
            .dropdown_menu({
                let entity = entity.clone();
                move |menu, window, _| {
                    let pick = |menu: gpui_kit::component::menu::PopupMenu, id: Option<String>, label: String, window: &mut Window| {
                        let checked = id == picked_model;
                        menu.item(
                            gpui_kit::component::menu::PopupMenuItem::new(label)
                                .checked(checked)
                                .on_click(window.listener_for(&entity, move |this, _, _, cx| {
                                    this.model = id.clone();
                                    // An effort the new model does not accept falls back to its default.
                                    if this.effort.as_ref().is_some_and(|effort| !this.efforts().contains(effort)) {
                                        this.effort = None;
                                    }
                                    cx.notify();
                                })),
                        )
                    };
                    let menu = pick(menu.scrollable(true).max_h(px(360.)), None, "Default (agent settings)".to_string(), window).separator();
                    models.iter().fold(menu, |menu, choice| {
                        let label = if choice.is_default { format!("{} · default", choice.label) } else { choice.label.clone() };
                        pick(menu, Some(choice.id.clone()), label, window)
                    })
                }
            });
        let efforts = self.efforts();
        let picked_effort = self.effort.clone();
        let default_effort = self
            .model
            .as_ref()
            .and_then(|model| self.models.iter().find(|choice| &choice.id == model))
            .or_else(|| self.models.iter().find(|choice| choice.is_default))
            .and_then(|choice| choice.default_effort.clone());
        let effort_label = match (&self.effort, &default_effort) {
            (Some(effort), _) => format!("Effort · {effort}"),
            (None, Some(effort)) => format!("Effort · default ({effort})"),
            (None, None) => "Effort · default".to_string(),
        };
        let effort_menu = Button::new("agent-effort")
            .ghost()
            .small()
            .icon(IconName::Gauge)
            .label(effort_label)
            .disabled(self.running || efforts.is_empty())
            .tooltip("Reasoning effort for the next message")
            .dropdown_menu(move |menu, window, _| {
                let pick = |menu: gpui_kit::component::menu::PopupMenu, effort: Option<String>, window: &mut Window| {
                    let label = effort.clone().unwrap_or_else(|| "Default".to_string());
                    let checked = effort == picked_effort;
                    menu.item(
                        gpui_kit::component::menu::PopupMenuItem::new(label)
                            .checked(checked)
                            .on_click(window.listener_for(&entity, move |this, _, _, cx| {
                                this.effort = effort.clone();
                                cx.notify();
                            })),
                    )
                };
                let menu = pick(menu, None, window).separator();
                efforts.iter().fold(menu, |menu, effort| pick(menu, Some(effort.clone()), window))
            });
        let options_row = h_flex().gap_1().items_center().child(model_menu).child(effort_menu);

        let can_reply = self.session.as_ref().is_none_or(|session| !session.running || self.running) && self.agent.installed();
        let composer = v_flex()
            .p_3()
            .gap_2()
            .border_t_1()
            .border_color(palette.border)
            .children(picker)
            .child(options_row)
            .children(self.notice.clone().map(|notice| div().text_xs().text_color(palette.video).child(notice)))
            .when(!self.agent.installed(), |this| {
                this.child(div().text_xs().text_color(palette.faint).child(format!("{} is not installed: replying is off.", self.agent.name())))
            })
            .when(self.session.as_ref().is_some_and(|session| session.running && session.external), |this| {
                this.child(div().text_xs().text_color(palette.faint).child(
                    "Running elsewhere right now: a reply here would start a parallel turn. Wait for it to finish.",
                ))
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.input).w_full()))
                    .child(
                        Button::new("agent-send")
                            .primary()
                            .icon(IconName::ArrowUp)
                            .loading(self.running)
                            .disabled(self.running || !can_reply)
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            );

        v_flex()
            .key_context("Agents")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            .size_full()
            .bg(palette.window)
            .text_color(palette.text)
            .child(header)
            .child(usage_bar)
            .child(
                div()
                    .id("agent-chat")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .on_scroll(cx.listener(|this, event: &gpui_kit::ScrollEvent, _, cx| {
                        let delta = event.delta.pixel_delta(px(16.)).y;
                        let follow = if delta > px(0.) { false } else { this.follow || this.at_bottom() };
                        if follow != this.follow {
                            this.follow = follow;
                            cx.notify();
                        }
                    }))
                    .child(chat)
                    .vertical_scrollbar(&self.scroll),
            )
            // Floating "jump to the end", while the reader is up in the history.
            .when(!self.follow, |this| {
                this.child(
                    div().relative().h_0().child(
                        div().absolute().bottom(px(12.)).right(px(20.)).debug_selector(|| "agent-scroll-bottom".into()).child(
                            Button::new("agent-scroll-bottom")
                                .icon(IconName::ArrowDown)
                                .rounded_full()
                                .with_size(gpui_kit::component::Size::Medium)
                                .tooltip("Scroll to the latest message")
                                .on_click(cx.listener(|this, _, _, cx| this.scroll_to_end(cx))),
                        ),
                    ),
                )
            })
            .child(composer)
    }
}

pub(crate) fn open_agent_window(
    session: Option<Session>,
    cwd: PathBuf,
    appearance: Entity<AppearanceSettings>,
    explorer: WeakEntity<FileExplorer>,
    known_sessions: Vec<Session>,
    cx: &mut App,
) -> Option<Entity<AgentWindow>> {
    let title = session.as_ref().map_or_else(|| "New agent session".to_string(), |session| format!("{} — {}", session.title, session.agent.name()));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds { origin: point(px(220.), px(90.)), size: size(px(760.), px(720.)) })),
        window_min_size: Some(size(px(480.), px(420.))),
        titlebar: Some(gpui_kit::TitlebarOptions { title: Some(title.into()), ..Default::default() }),
        ..Default::default()
    };
    let created = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = created.clone();
    gpui_kit::open_window(options, cx, move |window, cx| {
        let view = cx.new(|cx| AgentWindow::new(window, cx, session, cwd, appearance, explorer, known_sessions));
        *slot.borrow_mut() = Some(view.clone());
        view
    })
    .ok()?;
    created.borrow_mut().take()
}

// ------------------------------------------------------------ usage window

pub struct UsageWindow {
    appearance: Entity<AppearanceSettings>,
    sessions: Vec<Session>,
    accounts: Vec<(Agent, Account)>,
    loading: bool,
    /// Unfolded agent cards.
    expanded: std::collections::HashSet<Agent>,
    focus_handle: FocusHandle,
    _subscription: Subscription,
}

impl UsageWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>, sessions: Vec<Session>, appearance: Entity<AppearanceSettings>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let known = sessions.clone();
        // The agent used most recently starts unfolded.
        let expanded = sessions.first().map(|session| session.agent).into_iter().collect();
        cx.spawn(async move |this, cx| {
            let accounts = cx
                .background_executor()
                .spawn(async move { files_ext_agents::accounts(&roots(), &known) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.accounts = accounts;
                this.loading = false;
                cx.notify();
            });
        })
        .detach();
        Self {
            _subscription: cx.observe(&appearance, |_, _, cx| cx.notify()),
            appearance,
            sessions,
            accounts: Vec::new(),
            loading: true,
            expanded,
            focus_handle,
        }
    }

    fn close(&mut self, _: &CloseAgents, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    fn agent_card(&self, agent: Agent, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let sessions: Vec<&Session> = self.sessions.iter().filter(|session| session.agent == agent).collect();
        let week_ago = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_secs() as i64) - 7 * 86_400;
        let mut all = Usage::default();
        let mut week = Usage::default();
        for session in &sessions {
            all.add(&session.usage);
            if session.updated >= week_ago {
                week.add(&session.usage);
            }
        }
        let mut models: Vec<String> = sessions.iter().filter_map(|session| session.model.clone()).collect();
        models.sort();
        models.dedup();
        let running = sessions.iter().filter(|session| session.running).count();
        let external = sessions.iter().filter(|session| session.external).count();
        let account = self.accounts.iter().find(|(known, _)| *known == agent).map(|(_, account)| account);

        let row = |label: &str, value: String| {
            h_flex()
                .gap_3()
                .text_xs()
                .child(div().w(px(120.)).flex_shrink_0().text_color(palette.muted).child(label.to_string()))
                // Long values (model lists, tiers) wrap instead of widening the window.
                .child(div().flex_1().min_w_0().text_color(palette.text).child(value))
        };
        let usage_rows = |usage: &Usage| {
            v_flex()
                .gap_1()
                .child(row("Input", tokens(usage.input)))
                .child(row("Output", tokens(usage.output)))
                .child(row("Cache read", tokens(usage.cache_read)))
                .child(row("Cache write", tokens(usage.cache_write)))
                .when(usage.reasoning > 0, |this| this.child(row("Reasoning", tokens(usage.reasoning))))
                .children(usage.cost.map(|cost| row("Cost", format!("${cost:.2}"))))
        };

        let expanded = self.expanded.contains(&agent);
        // The folded card still says the essentials: sessions, tokens, the tightest limit.
        let tightest = account
            .and_then(|account| account.limits.iter().max_by(|a, b| a.used_percent.total_cmp(&b.used_percent)))
            .map(|limit| format!(" · {:.0}% of {}", limit.used_percent, limit.name))
            .unwrap_or_default();
        let summary = format!("{} sessions · {} tok{tightest}", sessions.len(), tokens(all.total()));
        let header = h_flex()
            .id(gpui_kit::ElementId::Name(format!("usage-card-{}", agent.short()).into()))
            .px_4()
            .py_3()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .hover(|style| style.bg(palette.hover))
            .aria_label(format!("{}, {summary}, {}", agent.name(), if expanded { "expanded" } else { "collapsed" }))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.expanded.remove(&agent) {
                    this.expanded.insert(agent);
                }
                cx.notify();
            }))
            .child(
                gpui_kit::component::Icon::new(if expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                    .xsmall()
                    .text_color(palette.faint),
            )
            .child(agent_glyph(agent, palette, 22.))
            .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(agent.name()))
            .child(div().flex_1().min_w_0().truncate().text_xs().text_color(palette.muted).child(summary))
            .child(div().flex_shrink_0().text_xs().text_color(palette.faint).child(if agent.installed() { "installed" } else { "not installed" }));
        #[cfg(test)]
        let header = header.debug_selector(move || format!("usage-card-{}", agent.short()));
        let wrapper = v_flex()
            .w_full()
            .rounded_lg()
            .overflow_hidden()
            .border_1()
            .border_color(palette.border)
            .bg(palette.surface)
            .child(header);
        if !expanded {
            return wrapper.into_any_element();
        }
        let mut card = v_flex()
            .px_4()
            .pb_4()
            .pt_1()
            .gap_3()
            .child(row("Sessions", format!("{} · {running} running · {external} external", sessions.len())))
            .child(row("Models", if models.is_empty() { "—".to_string() } else { models.join(", ") }));

        if let Some(account) = account {
            card = card
                .children(account.plan.clone().map(|plan| row("Plan", plan)))
                .children(account.organization.clone().map(|organization| row("Organization", organization)))
                .children(account.details.iter().map(|(label, value)| row(label, value.clone())));
            for limit in &account.limits {
                let used = limit.used_percent.clamp(0., 100.) as f32;
                let color = if used >= 90. { palette.video } else if used >= 70. { palette.archive } else { palette.code };
                card = card.child(
                    v_flex()
                        .gap_1()
                        .child(row(
                            &format!("Limit · {}", limit.name),
                            format!(
                                "{used:.0}% used{}",
                                limit.resets_at.map(|reset| format!(" · resets {}", until(reset))).unwrap_or_default()
                            ),
                        ))
                        .child(
                            div()
                                .h(px(6.))
                                .w_full()
                                .rounded_full()
                                .bg(palette.hover)
                                .child(div().h_full().w(gpui_kit::relative(used / 100.)).rounded_full().bg(color)),
                        ),
                );
            }
        } else if !self.loading {
            card = card.child(row("Plan", "not reported locally".to_string()));
        }
        if agent == Agent::Claude && account.is_some_and(|account| account.limits.is_empty()) {
            card = card.child(div().text_xs().text_color(palette.faint).child(
                "Claude Code does not store its rate-limit windows on disk; run /usage in Claude Code to see them.",
            ));
        }
        let card = card.child(div().text_xs().font_weight(FontWeight::MEDIUM).text_color(palette.muted).child("LAST 7 DAYS"))
            .child(usage_rows(&week))
            .child(div().text_xs().font_weight(FontWeight::MEDIUM).text_color(palette.muted).child("ALL TIME"))
            .child(usage_rows(&all));
        wrapper.child(card).into_any_element()
    }
}

#[cfg(test)]
impl UsageWindow {
    pub(crate) fn expanded_for_test(&self) -> Vec<Agent> {
        let mut agents: Vec<_> = self.expanded.iter().copied().collect();
        agents.sort();
        agents
    }
}

impl Focusable for UsageWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for UsageWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&settings);
        let sessions = self.sessions.iter().enumerate().map(|(index, session)| {
            h_flex()
                .id(("usage-session", index))
                .px_2()
                .py_1()
                .gap_2()
                .items_center()
                .rounded_md()
                .text_xs()
                .child(agent_glyph(session.agent, palette, 14.))
                .child(div().flex_1().min_w_0().truncate().child(session.title.clone()))
                .child(div().w(px(110.)).text_color(palette.faint).truncate().child(folder_name(session)))
                .child(div().w(px(70.)).text_color(palette.muted).child(format!("{} tok", tokens(session.usage.total()))))
                .child(div().w(px(36.)).text_color(palette.faint).child(ago(session.updated)))
                .when(session.running, |this| this.child(tag("running", palette.code, palette)))
                .when(session.external, |this| this.child(tag("external", palette.faint, palette)))
        });
        v_flex()
            .key_context("Agents")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            .size_full()
            .overflow_hidden()
            .bg(palette.window)
            .text_color(palette.text)
            .child(
                div()
                    .id("usage-scroll")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .w_full()
                            .p_4()
                            .gap_4()
                            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Agents usage"))
                            .child(div().text_xs().text_color(palette.faint).child(
                                "From the agents' own files on this computer. Passwords, tokens and API keys are never read.",
                            ))
                            .children(Agent::ALL.map(|agent| self.agent_card(agent, palette, cx)))
                            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(format!("Sessions ({})", self.sessions.len())))
                            .child(v_flex().children(sessions)),
                    ),
            )
            .child(
                h_flex().p_2().justify_end().border_t_1().border_color(palette.border).child(
                    Button::new("usage-close").ghost().small().label("Close").on_click(cx.listener(|_, _, window, _| window.remove_window())),
                ),
            )
    }
}

pub(crate) fn open_usage_window(sessions: Vec<Session>, appearance: Entity<AppearanceSettings>, cx: &mut App) -> Option<Entity<UsageWindow>> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds { origin: point(px(160.), px(80.)), size: size(px(1080.), px(760.)) })),
        window_min_size: Some(size(px(760.), px(480.))),
        titlebar: Some(gpui_kit::TitlebarOptions { title: Some("Agents usage".into()), ..Default::default() }),
        ..Default::default()
    };
    let created = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = created.clone();
    gpui_kit::open_window(options, cx, move |window, cx| {
        let view = cx.new(|cx| UsageWindow::new(window, cx, sessions, appearance));
        *slot.borrow_mut() = Some(view.clone());
        view
    })
    .ok()?;
    created.borrow_mut().take()
}
