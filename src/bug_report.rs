//! Help → Report a Bug…: a short form that becomes an issue on GitHub. The
//! report goes to the website, which files it with its own token, so no
//! GitHub account is needed; when that fails, the issue opens filled in, in
//! the browser.

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, FontWeight, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px,
};

use crate::app::AppearanceSettings;
use crate::palette::Palette;

const ENDPOINT: &str = "https://fileforce.vercel.app/api/report-bug";
const REPOSITORY: &str = "FileForce/files";

/// What the form sends.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Report {
    pub title: String,
    pub description: String,
    /// An e-mail or GitHub name to ask back; may be empty.
    pub contact: String,
    pub version: String,
    pub system: String,
    /// The last Console events, when the user agreed.
    pub log: Option<String>,
}

/// "macOS 26.1 · aarch64".
pub fn system() -> String {
    let os = sysinfo::System::long_os_version().unwrap_or_else(|| std::env::consts::OS.to_string());
    format!("{os} · {}", std::env::consts::ARCH)
}

/// The last `count` journal events, one per line (no file contents).
pub fn recent_log(count: usize) -> String {
    let events = files_sdk::log::events();
    events
        .iter()
        .rev()
        .take(count)
        .rev()
        .map(|event| format!("{} [{}] {:?} {}", crate::dev_shell::clock(event.time), event.source, event.kind, event.message))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The issue's body, as Markdown.
pub fn body(report: &Report) -> String {
    let mut body = format!("{}\n\n---\n**Version:** {}  \n**System:** {}", report.description.trim(), report.version, report.system);
    if !report.contact.trim().is_empty() {
        body.push_str(&format!("  \n**Contact:** {}", report.contact.trim()));
    }
    if let Some(log) = &report.log {
        body.push_str(&format!("\n\n<details><summary>Recent events</summary>\n\n```\n{log}\n```\n</details>"));
    }
    body.push_str("\n\n_Sent from Files → Help → Report a Bug_");
    body
}

/// The issue page opened with the report filled in (a GitHub account needed).
pub fn prefilled_url(report: &Report) -> String {
    let mut url = url::Url::parse(&format!("https://github.com/{REPOSITORY}/issues/new")).expect("valid");
    // Long logs do not fit in an address: the browser version leaves them out.
    let short = Report { log: None, ..report.clone() };
    url.query_pairs_mut().append_pair("title", &report.title).append_pair("body", &body(&short)).append_pair("labels", "bug");
    url.into()
}

/// Files the issue through the website; its address on success.
pub fn send(report: &Report) -> Result<String, String> {
    let payload = serde_json::json!({ "title": report.title, "body": body(report) });
    let config = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(20))).http_status_as_error(false).build();
    let agent: ureq::Agent = config.into();
    let mut response = agent.post(ENDPOINT).send_json(&payload).map_err(|error| error.to_string())?;
    let status = response.status();
    let answer: serde_json::Value = response.body_mut().read_json().unwrap_or_default();
    match answer["url"].as_str() {
        Some(url) if status.is_success() => Ok(url.to_string()),
        _ => Err(answer["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("The website answered {status}"))),
    }
}

fn open_in_browser(url: &str) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = files_sdk::log::command(program).arg(url).spawn();
}

enum Status {
    Editing,
    Sending,
    Sent(String),
    Failed(String),
}

pub struct BugReportDialog {
    appearance: Entity<AppearanceSettings>,
    title: Entity<InputState>,
    description: Entity<TextareaState>,
    contact: Entity<InputState>,
    include_log: bool,
    status: Status,
    focus: FocusHandle,
}

impl BugReportDialog {
    fn new(appearance: Entity<AppearanceSettings>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("What went wrong, in a few words"));
        let description = cx.new(|cx| TextareaState::new(window, cx).rows(7).placeholder("What did you do, what did you expect, what happened instead?"));
        let contact = cx.new(|cx| InputState::new(window, cx).placeholder("E-mail or GitHub name, to ask you back (optional)"));
        Self { appearance, title, description, contact, include_log: false, status: Status::Editing, focus: cx.focus_handle() }
    }

    fn report(&self, cx: &App) -> Report {
        Report {
            title: self.title.read(cx).value().trim().to_string(),
            description: self.description.read(cx).value().to_string(),
            contact: self.contact.read(cx).value().to_string(),
            version: crate::updates::CURRENT_VERSION.to_string(),
            system: system(),
            log: self.include_log.then(|| recent_log(60)),
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let report = self.report(cx);
        if report.title.is_empty() || report.description.trim().is_empty() {
            self.status = Status::Failed("Add a title and a few words about the problem.".into());
            cx.notify();
            return;
        }
        self.status = Status::Sending;
        cx.notify();
        let task = cx.background_executor().spawn(async move { (send(&report), report) });
        cx.spawn(async move |this, cx| {
            let (sent, report) = task.await;
            let _ = this.update(cx, |this, cx| {
                this.status = match sent {
                    Ok(url) => {
                        files_sdk::log::notify("files", files_sdk::log::Level::Success, "Bug reported — thank you", Some(url.clone()));
                        Status::Sent(url)
                    }
                    Err(error) => {
                        // No account needed to try the website; the browser way needs one.
                        open_in_browser(&prefilled_url(&report));
                        Status::Failed(format!("Could not send it ({error}). The issue opened in your browser, filled in — press Submit there."))
                    }
                };
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for BugReportDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::from_settings(self.appearance.read(cx));
        let sending = matches!(self.status, Status::Sending);
        let label = |text: &'static str| div().text_xs().font_weight(FontWeight::MEDIUM).text_color(palette.muted).child(text);
        let status = match &self.status {
            Status::Editing => None,
            Status::Sending => Some(div().text_xs().text_color(palette.muted).child("Sending…")),
            Status::Sent(url) => Some(div().text_xs().text_color(palette.code).child(format!("Thank you! The report is at {url}"))),
            Status::Failed(error) => Some(div().text_xs().text_color(palette.video).child(error.clone())),
        };
        let sent = matches!(self.status, Status::Sent(_));
        v_flex()
            .track_focus(&self.focus)
            .gap_3()
            .p_5()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(gpui_kit::component::Icon::new(IconName::Bug).text_color(palette.accent))
                    .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child("Report a bug")),
            )
            .child(div().text_xs().text_color(palette.faint).child(format!(
                "It becomes an issue on github.com/{REPOSITORY}, visible to everyone. Version {} and your system ({}) are added.",
                crate::updates::CURRENT_VERSION,
                system()
            )))
            .child(v_flex().gap_1().child(label("Title")).child(Input::new(&self.title)))
            .child(v_flex().gap_1().child(label("What happened")).child(Textarea::new(&self.description)))
            .child(v_flex().gap_1().child(label("Contact")).child(Input::new(&self.contact)))
            .child(
                Checkbox::new("bug-include-log")
                    .label("Attach the last 60 Console events (names of commands and files, no contents)")
                    .checked(self.include_log)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.include_log = *checked;
                        cx.notify();
                    })),
            )
            .children(status)
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(Button::new("bug-cancel").ghost().small().label(if sent { "Close" } else { "Cancel" }).on_click(|_, window, cx| window.close_dialog(cx)))
                    .when(!sent, |this| {
                        this.child(
                            Button::new("bug-send")
                                .primary()
                                .small()
                                .icon(IconName::Send)
                                .label(if sending { "Sending…" } else { "Send report" })
                                .disabled(sending)
                                .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                        )
                    }),
            )
    }
}

use gpui_kit::{Focusable as _, InteractiveElement as _};
use gpui_kit::prelude::FluentBuilder as _;

/// Opens the form over `window`.
pub fn open(appearance: Entity<AppearanceSettings>, window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    let view = cx.new(|cx| BugReportDialog::new(appearance, window, cx));
    let focus = view.read(cx).title.focus_handle(cx);
    window.focus(&focus, cx);
    window.open_dialog(cx, move |dialog, _, _| dialog.w(px(560.)).p_0().centered().close_button(false).overlay(true).overlay_closable(false).on_ok(|_, _, _| false).child(view.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_reads_well_as_an_issue() {
        let report = Report {
            title: "Preview stays empty".into(),
            description: "Select a PNG, nothing shows.".into(),
            contact: "me@example.com".into(),
            version: "0.1.17".into(),
            system: "macOS · aarch64".into(),
            log: Some("12:00 [files] Fs listed".into()),
        };
        let text = body(&report);
        assert!(text.starts_with("Select a PNG, nothing shows."));
        assert!(text.contains("**Version:** 0.1.17") && text.contains("**Contact:** me@example.com") && text.contains("<details>"));
        let url = prefilled_url(&report);
        assert!(url.starts_with("https://github.com/FileForce/files/issues/new?title=Preview+stays+empty"));
        assert!(url.contains("labels=bug") && !url.contains("details"), "the browser version leaves the log out");
    }
}
