//! Quick Actions on screen: the questions an action asks before it runs,
//! and Customize… — the user's scripts, written by hand or by an agent.

use std::path::PathBuf;

use files_sdk::quick_actions::{Field, QuickAction};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _,
    Styled as _, Window, div, px,
};

use crate::app::AppearanceSettings;
use crate::palette::Palette;
use crate::quick_actions::{Language, Script, Source};

/// Runs an action on the selection, asking first when it has questions.
pub(crate) fn start(source: Source, action: QuickAction, selection: Vec<PathBuf>, appearance: Entity<AppearanceSettings>, window: &mut Window, cx: &mut App) {
    if action.ask.is_empty() {
        run_in_background(source, action, selection, Vec::new(), cx);
        return;
    }
    let view = cx.new(|cx| AskDialog::new(source, action, selection, appearance, window, cx));
    window.open_dialog(cx, move |dialog, _, _| dialog.w(px(460.)).p_0().centered().close_button(false).overlay(true).overlay_closable(true).on_ok(|_, _, _| false).child(view.clone()));
}

fn run_in_background(source: Source, action: QuickAction, selection: Vec<PathBuf>, answers: Vec<(String, String)>, cx: &mut App) {
    let job = crate::tasks::queue("quick action", format!("{} · {} item(s)", action.title, selection.len()));
    cx.background_executor()
        .spawn(async move {
            crate::tasks::start(job);
            let result = crate::quick_actions::run(&source, &action.id, &selection, &answers);
            crate::tasks::finish(job, result.as_ref().map(|_| ()).map_err(ToString::to_string));
            let title = action.title.trim_end_matches('…').to_string();
            match result {
                Ok(outcome) => {
                    let actions = outcome.made.first().map(|path| crate::views::console::file_actions(path)).unwrap_or_default();
                    files_sdk::log::notify_actions("quick-actions", files_sdk::log::Level::Success, format!("{title}: {}", outcome.summary), outcome.made.first().map(|path| path.display().to_string()), actions);
                    files_sdk::host().files_changed();
                }
                Err(error) => {
                    files_sdk::log::notify("quick-actions", files_sdk::log::Level::Error, format!("{title} failed"), Some(error.to_string()));
                }
            }
        })
        .detach();
}

enum Answer {
    Choice(usize, Vec<String>),
    Typed(Entity<InputState>),
}

pub struct AskDialog {
    /// A running script waits for these answers (libfiles `ask`).
    reply: Option<std::sync::mpsc::Sender<Option<Vec<(String, String)>>>>,
    source: Source,
    action: QuickAction,
    selection: Vec<PathBuf>,
    appearance: Entity<AppearanceSettings>,
    answers: Vec<(String, String, Answer)>,
}

impl AskDialog {
    fn new(source: Source, action: QuickAction, selection: Vec<PathBuf>, appearance: Entity<AppearanceSettings>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let answers = action
            .ask
            .iter()
            .map(|field| match field {
                Field::Choice { key, label, options } => (key.clone(), label.clone(), Answer::Choice(0, options.clone())),
                Field::Text { key, label, default } => {
                    let default = default.clone();
                    (key.clone(), label.clone(), Answer::Typed(cx.new(|cx| InputState::new(window, cx).default_value(default))))
                }
                Field::Number { key, label, default } => {
                    let default = default.to_string();
                    (key.clone(), label.clone(), Answer::Typed(cx.new(|cx| InputState::new(window, cx).default_value(default))))
                }
            })
            .collect();
        Self { reply: None, source, action, selection, appearance, answers }
    }

    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let answers = self
            .answers
            .iter()
            .map(|(key, _, answer)| {
                let value = match answer {
                    Answer::Choice(index, options) => options.get(*index).cloned().unwrap_or_default(),
                    Answer::Typed(input) => input.read(cx).value().to_string(),
                };
                (key.clone(), value)
            })
            .collect();
        window.close_dialog(cx);
        match self.reply.take() {
            Some(reply) => {
                let _ = reply.send(Some(answers));
            }
            None => run_in_background(self.source.clone(), self.action.clone(), self.selection.clone(), answers, cx),
        }
    }
}

impl Drop for AskDialog {
    /// Closed without Run: the waiting script hears it was cancelled.
    fn drop(&mut self) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(None);
        }
    }
}

/// A running script asked something (libfiles `ask`): the dialog, whose
/// answers go back to it.
pub(crate) fn ask_for_script(ask: crate::quick_actions::ScriptAsk, appearance: Entity<AppearanceSettings>, window: &mut Window, cx: &mut App) {
    let action = QuickAction { id: String::new(), title: ask.title, applies: Default::default(), order: 0, ask: ask.fields };
    let reply = ask.reply;
    let view = cx.new(|cx| {
        let mut dialog = AskDialog::new(Source::Script, action, Vec::new(), appearance, window, cx);
        dialog.reply = Some(reply);
        dialog
    });
    window.open_dialog(cx, move |dialog, _, _| dialog.w(px(460.)).p_0().centered().close_button(false).overlay(true).overlay_closable(false).on_ok(|_, _, _| false).child(view.clone()));
}

impl Render for AskDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::from_settings(self.appearance.read(cx));
        let count = self.selection.len();
        let fields = self.answers.iter().enumerate().map(|(field, (_, label, answer))| {
            let control = match answer {
                Answer::Choice(chosen, options) => h_flex()
                    .gap_1()
                    .flex_wrap()
                    .children(options.iter().enumerate().map(|(index, option)| {
                        let button = Button::new(("qa-choice", field * 32 + index)).small().label(option.clone());
                        let button = if index == *chosen { button.primary() } else { button.secondary() };
                        button.on_click(cx.listener(move |this, _, _, cx| {
                            if let Some((_, _, Answer::Choice(chosen, _))) = this.answers.get_mut(field) {
                                *chosen = index;
                            }
                            cx.notify();
                        }))
                    }))
                    .into_any_element(),
                Answer::Typed(input) => Input::new(input).into_any_element(),
            };
            v_flex().gap_1().child(div().text_xs().font_weight(FontWeight::MEDIUM).text_color(palette.muted).child(label.clone())).child(control)
        });
        v_flex()
            .gap_3()
            .p_5()
            .child(h_flex().gap_2().items_center().child(Icon::new(IconName::Zap).text_color(palette.accent)).child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(self.action.title.trim_end_matches('…').to_string())))
            .when(self.reply.is_none(), |this| this.child(div().text_xs().text_color(palette.faint).child(format!("{count} item{} selected", if count == 1 { "" } else { "s" }))))
            .children(fields)
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(Button::new("qa-cancel").ghost().small().label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(Button::new("qa-run").primary().small().icon(IconName::Play).label("Run").on_click(cx.listener(|this, _, window, cx| this.run(window, cx)))),
            )
    }
}

// ------------------------------------------------------------ customize

pub struct CustomizeDialog {
    appearance: Entity<AppearanceSettings>,
    scripts: Vec<(PathBuf, Script)>,
    editing: Option<PathBuf>,
    name: Entity<InputState>,
    extensions: Entity<InputState>,
    order: Entity<InputState>,
    code: Entity<TextareaState>,
    language: Language,
    prompt: Entity<InputState>,
    status: Option<String>,
    writing: bool,
    /// An agent's code, put in the editor on the next draw.
    pending_code: Option<String>,
    /// Who writes it, with which model and effort.
    agent: Option<files_ext_agents::Agent>,
    models: Vec<files_ext_agents::ModelChoice>,
    model: Option<String>,
    effort: Option<String>,
}

impl CustomizeDialog {
    fn new(appearance: Entity<AppearanceSettings>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let language = Language::ALL.into_iter().find(|language| language.available()).unwrap_or_default();
        Self {
            appearance,
            scripts: crate::quick_actions::scripts(),
            editing: None,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Make thumbnails")),
            extensions: cx.new(|cx| InputState::new(window, cx).placeholder("png, jpg — empty for any file")),
            order: cx.new(|cx| InputState::new(window, cx).default_value("100")),
            code: cx.new(|cx| {
                let mut code = TextareaState::new(window, cx).rows(14);
                code.set_value(language.template(), window, cx);
                code
            }),
            language,
            prompt: cx.new(|cx| InputState::new(window, cx).placeholder("Describe the script, e.g. resize pictures to 1080 px wide")),
            status: None,
            writing: false,
            pending_code: None,
            agent: None,
            models: Vec::new(),
            model: None,
            effort: None,
        }
        .with_first_agent()
    }

    fn with_first_agent(mut self) -> Self {
        if let Some(agent) = files_ext_agents::Agent::RUNNABLE.into_iter().find(|agent| agent.installed()) {
            self.choose_agent(agent);
        }
        self
    }

    /// An agent, its models, its default model and effort.
    fn choose_agent(&mut self, agent: files_ext_agents::Agent) {
        self.agent = Some(agent);
        self.models = files_ext_agents::models(&files_ext_agents::Roots::detect(), agent, &[]);
        let default = self.models.iter().find(|model| model.is_default).or_else(|| self.models.first());
        self.model = default.map(|model| model.id.clone());
        self.effort = default.and_then(|model| model.default_effort.clone());
    }

    fn edit(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let (path, script) = match index.and_then(|index| self.scripts.get(index).cloned()) {
            Some((path, script)) => (Some(path), script),
            None => (None, Script { name: String::new(), language: self.language, order: 100, code: self.language.template().to_string(), ..Default::default() }),
        };
        self.editing = path;
        self.language = script.language;
        self.name.update(cx, |input, cx| input.set_value(script.name.clone(), window, cx));
        self.extensions.update(cx, |input, cx| input.set_value(script.extensions.join(", "), window, cx));
        self.order.update(cx, |input, cx| input.set_value(script.order.to_string(), window, cx));
        self.code.update(cx, |input, cx| input.set_value(script.code.clone(), window, cx));
        self.status = None;
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.status = Some("Give the action a name".into());
            cx.notify();
            return;
        }
        let previous = self.editing.as_ref().and_then(|path| self.scripts.iter().find(|(known, _)| known == path).map(|(_, script)| script.clone()));
        let script = Script {
            name,
            language: self.language,
            extensions: self.extensions.read(cx).value().split([',', ' ']).map(|extension| extension.trim().trim_start_matches('.').to_lowercase()).filter(|extension| !extension.is_empty()).collect(),
            order: self.order.read(cx).value().trim().parse().unwrap_or(100),
            code: self.code.read(cx).value().to_string(),
            ..previous.unwrap_or_default()
        };
        self.status = Some(match crate::quick_actions::save_script(&script, self.editing.as_deref()) {
            Ok(path) => {
                self.editing = Some(path);
                format!("Saved · {} is in the Quick Actions menu", script.name)
            }
            Err(error) => error.to_string(),
        });
        self.scripts = crate::quick_actions::scripts();
        cx.notify();
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.editing.take() {
            let _ = std::fs::remove_file(path);
        }
        self.scripts = crate::quick_actions::scripts();
        self.edit(None, window, cx);
    }

    /// Asks an installed agent to write the script; its code lands in the editor.
    fn write_with_agent(&mut self, cx: &mut Context<Self>) {
        let Some(agent) = self.agent.filter(|agent| agent.installed()) else {
            self.status = Some("No agent is installed (Claude Code, Codex, OpenCode or Kimi)".into());
            cx.notify();
            return;
        };
        let want = self.prompt.read(cx).value().trim().to_string();
        if want.is_empty() {
            self.status = Some("Describe what the script should do".into());
            cx.notify();
            return;
        }
        let language = self.language;
        let options = files_ext_agents::RunOptions { model: self.model.clone(), effort: self.effort.clone() };
        let prompt = format!(
            "Write a {} script for a file manager's Quick Action: {want}. The selected files are the command-line arguments. {} Report every file you create or change, and print a short summary as the last line. Reply with only the code in one fenced block.",
            language.label(),
            match language {
                Language::Python => "Use the bundled module `libfiles` (import libfiles): libfiles.selection(), libfiles.ask(title, [libfiles.choice(key, label, options), libfiles.text(key, label), libfiles.number(key, label)]) returns a dict of answers, libfiles.made(path), libfiles.notify(title, body), libfiles.progress(text).",
                Language::JavaScript => "Use the bundled module `libfiles` (const files = require('libfiles')): files.selection(), files.ask(title, [files.choice(key, label, options), files.text(key, label)]) returns an object of answers, files.made(path), files.notify(title, body).",
                Language::Lua => "Use the bundled module `libfiles` (local files = require('libfiles')): files.selection(), files.ask(title, { files.choice(key, label, options) }) returns a table or nil, files.made(path), files.notify(title, body).",
                _ => "Print `made: <path>` for each file.",
            }
        );
        self.writing = true;
        self.status = Some(format!("{} is writing the script…", agent.name()));
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            let cwd = std::env::temp_dir();
            let mut run = files_ext_agents::start(agent, &cwd, &prompt, None, &options)?;
            let mut answer = String::new();
            files_ext_agents::follow(&mut run, |event| match event {
                files_ext_agents::stream::Event::Text(text) | files_ext_agents::stream::Event::Message(text) => answer.push_str(&text),
                _ => {}
            })?;
            Ok::<String, std::io::Error>(answer)
        });
        cx.spawn(async move |this, cx| {
            let answer = task.await;
            let _ = this.update(cx, |this, cx| {
                this.writing = false;
                match answer {
                    Ok(answer) => {
                        this.pending_code = Some(crate::quick_actions::code_from_answer(&answer));
                        this.status = Some("Written · check it, then Save".into());
                    }
                    Err(error) => this.status = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for CustomizeDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(code) = self.pending_code.take() {
            self.code.update(cx, |input, cx| input.set_value(code, window, cx));
        }
        let palette = Palette::from_settings(self.appearance.read(cx));
        let agents = self.appearance.read(cx).extensions.agents && files_ext_agents::Agent::RUNNABLE.into_iter().any(|agent| agent.installed());
        let list = self.scripts.iter().enumerate().map(|(index, (path, script))| {
            let selected = self.editing.as_ref() == Some(path);
            div()
                .id(("qa-script", index))
                .px_2()
                .py_1p5()
                .rounded_md()
                .text_sm()
                .cursor_pointer()
                .bg(if selected { palette.selection } else { gpui_kit::transparent_black() })
                .hover(|style| style.bg(palette.hover))
                .child(script.name.clone())
                .child(div().text_xs().text_color(palette.faint).child(script.language.label()))
                .on_click(cx.listener(move |this, _, window, cx| this.edit(Some(index), window, cx)))
        });
        let languages = Language::ALL.map(|language| {
            let available = language.available();
            let button = Button::new(gpui_kit::ElementId::Name(format!("qa-language-{}", language.label()).into())).xsmall().label(language.label()).disabled(!available);
            let button = if self.language == language { button.primary() } else { button.ghost() };
            button.tooltip(if available { "" } else { "Not installed" }).on_click(cx.listener(move |this, _, window, cx| {
                // A fresh template when the code is still the old language's.
                let untouched = this.code.read(cx).value() == this.language.template();
                this.language = language;
                if untouched {
                    this.code.update(cx, |input, cx| input.set_value(language.template(), window, cx));
                }
                cx.notify();
            }))
        });
        let label = |text: &'static str| div().text_xs().font_weight(FontWeight::MEDIUM).text_color(palette.muted).child(text);
        h_flex()
            .h(px(620.))
            .child(
                v_flex()
                    .w(px(200.))
                    .h_full()
                    .p_3()
                    .gap_1()
                    .border_r_1()
                    .border_color(palette.border)
                    .bg(palette.sidebar)
                    .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(palette.accent).child("YOUR ACTIONS"))
                    .children(list)
                    .child(Button::new("qa-new").ghost().small().icon(IconName::Plus).label("New action").on_click(cx.listener(|this, _, window, cx| this.edit(None, window, cx)))),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .p_4()
                    .gap_2()
                    .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child("Customize Quick Actions"))
                    .child(h_flex().gap_3().child(v_flex().flex_1().gap_1().child(label("Name")).child(Input::new(&self.name))).child(v_flex().w(px(80.)).gap_1().child(label("Order")).child(Input::new(&self.order))))
                    .child(v_flex().gap_1().child(label("For files ending in")).child(Input::new(&self.extensions)))
                    .child(v_flex().gap_1().child(label("Language")).child(h_flex().gap_1().flex_wrap().children(languages)))
                    .when(agents, |this| {
                        let installed: Vec<_> = files_ext_agents::Agent::RUNNABLE.into_iter().filter(|agent| agent.installed()).collect();
                        let agent_buttons = installed.into_iter().map(|agent| {
                            let button = Button::new(gpui_kit::ElementId::Name(format!("qa-agent-{}", agent.short()).into())).xsmall().label(agent.name());
                            let button = if self.agent == Some(agent) { button.primary() } else { button.ghost() };
                            button.on_click(cx.listener(move |this, _, _, cx| {
                                this.choose_agent(agent);
                                cx.notify();
                            }))
                        });
                        let model_buttons = self.models.iter().take(10).enumerate().map(|(index, model)| {
                            let id = model.id.clone();
                            let button = Button::new(("qa-model", index)).xsmall().label(model.label.clone());
                            let button = if self.model.as_deref() == Some(model.id.as_str()) { button.primary() } else { button.ghost() };
                            button.on_click(cx.listener(move |this, _, _, cx| {
                                // An effort this model does not take is dropped.
                                let efforts = this.models.iter().find(|model| model.id == id).map(|model| model.efforts.clone()).unwrap_or_default();
                                if this.effort.as_ref().is_some_and(|effort| !efforts.contains(effort)) {
                                    this.effort = None;
                                }
                                this.model = Some(id.clone());
                                cx.notify();
                            }))
                        });
                        let efforts = self.models.iter().find(|model| Some(&model.id) == self.model.as_ref()).map(|model| model.efforts.clone()).unwrap_or_default();
                        let effort_buttons = std::iter::once(None).chain(efforts.into_iter().map(Some)).enumerate().map(|(index, effort)| {
                            let button = Button::new(("qa-effort", index)).xsmall().label(effort.clone().unwrap_or_else(|| "default".into()));
                            let button = if self.effort == effort { button.primary() } else { button.ghost() };
                            button.on_click(cx.listener(move |this, _, _, cx| {
                                this.effort = effort.clone();
                                cx.notify();
                            }))
                        });
                        this.child(h_flex().gap_2().items_center().child(label("Agent")).child(h_flex().gap_1().flex_wrap().children(agent_buttons)))
                            .when(!self.models.is_empty(), |this| this.child(h_flex().gap_2().items_center().child(label("Model")).child(h_flex().gap_1().flex_wrap().children(model_buttons))))
                            .child(h_flex().gap_2().items_center().child(label("Effort")).child(h_flex().gap_1().flex_wrap().children(effort_buttons)))
                            .child(
                            h_flex()
                                .gap_2()
                                .child(div().flex_1().child(Input::new(&self.prompt)))
                                .child(Button::new("qa-agent").small().icon(IconName::Sparkles).label(if self.writing { "Writing…" } else { "Write with agent" }).disabled(self.writing).on_click(cx.listener(|this, _, _, cx| this.write_with_agent(cx)))),
                        )
                    })
                    .child(div().flex_1().min_h_0().child(Textarea::new(&self.code).h_full()))
                    .children(self.status.clone().map(|status| div().text_xs().text_color(palette.muted).child(status)))
                    .child(
                        h_flex()
                            .gap_2()
                            .when(self.editing.is_some(), |this| {
                                this.child(Button::new("qa-delete").ghost().small().icon(IconName::Trash).label("Delete").on_click(cx.listener(|this, _, window, cx| this.delete(window, cx))))
                            })
                            .child(div().flex_1())
                            .child(Button::new("qa-close").ghost().small().label("Close").on_click(|_, window, cx| window.close_dialog(cx)))
                            .child(Button::new("qa-save").primary().small().label("Save").on_click(cx.listener(|this, _, _, cx| this.save(cx)))),
                    ),
            )
    }
}

pub(crate) fn open_customize(appearance: Entity<AppearanceSettings>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| CustomizeDialog::new(appearance, window, cx));
    window.open_dialog(cx, move |dialog, _, _| dialog.w(px(860.)).p_0().centered().close_button(false).overlay(true).overlay_closable(false).on_ok(|_, _, _| false).child(view.clone()));
}
