//! Mail in the UI: the sidebar section with every account, the Mail view
//! (folders, messages, the open message with its attachments) in a tab or
//! its own window, and the window to write a message, which "Send as
//! Attachment…" opens with files.

use std::path::PathBuf;
use std::sync::Mutex;

use files_ext_mail::{Account, Folder, Message, Provider, Scheduled, Search, Special, Summary};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _, DropdownButton};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::text::TextView;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window, WindowBounds,
    WindowOptions, actions, div, point, px, size,
};

use crate::app::{AppearanceSettings, FileExplorer};
use crate::palette::Palette;

actions!(mail, [CloseMail]);

/// Messages read ahead after a folder loads, so opening them is instant.
const PREFETCH: usize = 15;

// ---------------------------------------------------------------- accounts

/// `mail.toml`, read once and again after every change (the sidebar draws often).
static ACCOUNTS: Mutex<Option<Vec<Account>>> = Mutex::new(None);

pub(crate) fn accounts() -> Vec<Account> {
    let mut cached = ACCOUNTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    cached.get_or_insert_with(files_ext_mail::accounts).clone()
}

fn accounts_changed() {
    *ACCOUNTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

fn time(seconds: Option<i64>) -> String {
    seconds
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .map(|time| {
            let local = time.with_timezone(&chrono::Local);
            if local.date_naive() == chrono::Local::now().date_naive() {
                local.format("%H:%M").to_string()
            } else {
                local.format("%b %d, %Y").to_string()
            }
        })
        .unwrap_or_default()
}

fn long_time(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|time| time.with_timezone(&chrono::Local).format("%a %b %d, %H:%M").to_string())
        .unwrap_or_default()
}

fn notice_row(text: &str, palette: Palette) -> AnyElement {
    div().px_4().py_2().border_t_1().border_color(palette.border).text_xs().text_color(palette.muted).child(text.to_string()).into_any_element()
}

fn special_icon(special: Option<Special>) -> IconName {
    match special {
        Some(Special::Inbox) => IconName::Inbox,
        Some(Special::Flagged) => IconName::Star,
        Some(Special::Drafts) => IconName::FilePen,
        Some(Special::Sent) => IconName::Send,
        Some(Special::Scheduled) => IconName::Clock3,
        Some(Special::Archive) => IconName::Archive,
        Some(Special::All) => IconName::Mails,
        Some(Special::Junk) => IconName::ShieldAlert,
        Some(Special::Trash) => IconName::Trash,
        None => IconName::Folder,
    }
}

pub(crate) fn special_icon_for(special: Special) -> IconName {
    special_icon(Some(special))
}

/// The color the user gave a special folder in Settings → Extensions → Mail.
pub(crate) fn special_color(settings: &crate::settings::AppSettings, special: Option<Special>) -> Option<Hsla> {
    let key = special?.key();
    settings.mail.folder_colors.get(key).and_then(|name| crate::menu_colors::parse(name))
}

/// `email:` in a search box: it picks the account, the rest are filters.
fn account_for(search: &Search, current: Option<&Account>) -> Option<Account> {
    match &search.account {
        Some(email) => accounts().into_iter().find(|account| account.email.eq_ignore_ascii_case(email)),
        None => current.cloned(),
    }
}

/// Splits text into plain runs and web links.
fn linkify(text: &str) -> Vec<(String, bool)> {
    let mut parts = Vec::new();
    let mut rest = text;
    while let Some(start) = ["https://", "http://", "mailto:"].iter().filter_map(|scheme| rest.find(scheme)).min() {
        if start > 0 {
            parts.push((rest[..start].to_string(), false));
        }
        let tail = &rest[start..];
        let end = tail.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | ')' | ']')).unwrap_or(tail.len());
        let link = tail[..end].trim_end_matches(['.', ',', ';', ':', '!', '?']);
        parts.push((link.to_string(), true));
        rest = &tail[link.len()..];
    }
    if !rest.is_empty() {
        parts.push((rest.to_string(), false));
    }
    parts
}

/// Shows text literally inside a markdown view.
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

/// Plain mail as markdown: every line kept (hard breaks), links clickable.
pub(crate) fn plain_to_markdown(text: &str) -> String {
    text.lines()
        .map(|line| {
            linkify(line)
                .into_iter()
                .map(|(part, link)| if link { format!("<{part}>") } else { escape_markdown(&part) })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("  \n")
}

/// HTML mail without what a reader must not run or show as text: scripts,
/// styles, the head and comments.
pub(crate) fn clean_html(html: &str) -> String {
    let mut out = html.to_string();
    for (open, close) in [("<!--", "-->"), ("<script", "</script>"), ("<style", "</style>"), ("<head", "</head>")] {
        loop {
            let lower = out.to_lowercase();
            let Some(start) = lower.find(open) else { break };
            let end = lower[start..].find(close).map_or(out.len(), |end| start + end + close.len());
            out.replace_range(start..end, "");
        }
    }
    out
}

/// Selectable text (subject, sender…): select and copy with the mouse.
fn selectable(id: impl Into<gpui_kit::ElementId>, text: &str) -> TextView {
    TextView::markdown(id, escape_markdown(text)).selectable(true)
}

/// The message itself: HTML with its styles when there is one, else the
/// text with every line kept. Both select and copy; links open.
fn render_body(message: &Message) -> AnyElement {
    let view = match &message.html {
        Some(html) => TextView::html(("mail-html", message.uid as usize), clean_html(html)),
        None => TextView::markdown(("mail-text", message.uid as usize), plain_to_markdown(&message.body)),
    };
    div().w_full().min_w_0().text_sm().child(view.selectable(true)).into_any_element()
}

/// A reply to `message`: to its sender, "Re:" subject, the text quoted.
fn open_reply(message: &Message, appearance: Entity<AppearanceSettings>, cx: &mut App) {
    let subject = if message.subject.to_lowercase().starts_with("re:") { message.subject.clone() } else { format!("Re: {}", message.subject) };
    let quoted: String = message.body.lines().map(|line| format!("> {line}\n")).collect();
    let body = format!("\n\nOn {}, {} wrote:\n{quoted}", time(message.date), message.from);
    open_compose(Vec::new(), Some((message.reply_to.clone(), subject, body)), appearance, cx);
}

/// Sender, trail, checks, structure and every header of a message.
pub(crate) fn render_details(message: &Message, palette: Palette) -> AnyElement {
    let details = &message.details;
    let section = |title: &'static str| div().pt_3().pb_1().text_size(px(10.)).font_weight(FontWeight::SEMIBOLD).text_color(palette.faint).child(title);
    let row = |label: &'static str, value: String| {
        h_flex()
            .gap_2()
            .items_start()
            .text_xs()
            .child(div().w(px(84.)).flex_shrink_0().text_color(palette.faint).child(label))
            .child(div().flex_1().min_w_0().child(TextView::markdown(gpui_kit::ElementId::Name(format!("detail-{label}").into()), escape_markdown(&value)).selectable(true)))
    };
    let optional = |label: &'static str, value: &Option<String>| value.clone().map(|value| row(label, value));
    let attachments = message.attachments.len();
    v_flex()
        .p_3()
        .gap_1()
        .child(section("SENDER"))
        .child(row("From", message.from.clone()))
        .children(optional("Reply-To", &details.reply_to))
        .children(optional("Return-Path", &details.return_path))
        .child(row("IP", details.sender_ip.clone().unwrap_or_else(|| "not in the headers".into())))
        .children(optional("Client", &details.client))
        .child(section("DELIVERY"))
        .child(row("To", message.to.clone()))
        .when(!message.cc.is_empty(), |this| this.child(row("Cc", message.cc.clone())))
        .child(row("Date", time(message.date)))
        .children(optional("Message-ID", &details.message_id))
        .when(!details.hops.is_empty(), |this| this.child(row("Route", details.hops.join(" → "))))
        .children(optional("Checks", &details.authentication))
        .child(section("CONTENT"))
        .child(row("MIME", details.mime.clone()))
        .child(row("Size", crate::settings::format_size(details.size as u64, Default::default())))
        .child(row("Attachments", if attachments == 0 { "none".into() } else { format!("{attachments}: {}", message.attachments.iter().map(|attachment| attachment.name.as_str()).collect::<Vec<_>>().join(", ")) }))
        .children(details.parts.iter().map(|(kind, name, bytes)| {
            row("Part", format!("{kind}{} · {}", name.as_ref().map(|name| format!(" · {name}")).unwrap_or_default(), crate::settings::format_size(*bytes as u64, Default::default())))
        }))
        .child(section("ALL HEADERS"))
        .children(details.headers.iter().enumerate().map(|(index, (name, value))| {
            v_flex()
                .py_0p5()
                .border_b_1()
                .border_color(palette.border)
                .child(div().text_size(px(10.)).text_color(palette.muted).child(name.clone()))
                .child(div().text_xs().font_family("Menlo").child(TextView::markdown(("mail-header", index), escape_markdown(value)).selectable(true)))
        }))
        .into_any_element()
}

// ------------------------------------------------------------------ sidebar

impl FileExplorer {
    /// MAIL: every account (opens Mail on it) and "Add account".
    pub(crate) fn render_mail_section(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !crate::licensing::active(self.appearance.read(cx), mail_index()) {
            return None;
        }
        let language = self.appearance.read(cx).language;
        let mut section = v_flex().gap_1().child(self.section_header("mail", crate::settings::localized(language, "MAIL", "ПОЧТА"), palette, cx));
        if self.section_collapsed("mail", cx) {
            return Some(section.into_any_element());
        }
        let row = |id: String, icon: IconName, label: String| {
            div()
                .id(gpui_kit::ElementId::Name(id.into()))
                .h(px(30.))
                .px_2()
                .rounded_md()
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .text_color(palette.muted)
                .hover(|style| style.bg(palette.hover).text_color(palette.text))
                .child(Icon::new(icon).small())
                .child(div().flex_1().min_w_0().truncate().child(label))
        };
        for account in accounts() {
            let email = account.email.clone();
            section = section.child(row(format!("mail-account-{email}"), IconName::Mail, account.email.clone()).on_click(cx.listener(
                move |this, _, window, cx| this.open_mail(Some(email.clone()), false, window, cx),
            )));
        }
        let scheduled = files_ext_mail::scheduled().len();
        if scheduled > 0 {
            section = section.child(
                row("mail-scheduled".into(), IconName::Clock3, crate::settings::localized(language, "Scheduled", "Запланированные").into())
                    .child(div().text_xs().text_color(palette.faint).child(scheduled.to_string()))
                    .on_click(cx.listener(|this, _, window, cx| this.open_mail(None, false, window, cx))),
            );
        }
        section = section.child(
            row("mail-add-account".into(), IconName::MailPlus, crate::settings::localized(language, "Add account…", "Добавить аккаунт…").into())
                .on_click(cx.listener(|this, _, window, cx| this.open_mail(None, true, window, cx))),
        );
        Some(section.into_any_element())
    }

    /// Mail on `email` (or the first account), in a tab or a window as set
    /// in Settings → Extensions → Mail.
    pub(crate) fn open_mail(&mut self, email: Option<String>, adding: bool, window: &mut Window, cx: &mut Context<Self>) {
        let appearance = self.appearance.clone();
        if self.appearance.read(cx).opens_in("mail") == crate::settings::OpenIn::Window {
            open_mail_window(email, adding, appearance, cx);
            return;
        }
        // One Mail tab: switch to it rather than opening another.
        let existing = self
            .tabs
            .iter()
            .enumerate()
            .find_map(|(index, tab)| tab.panel.as_ref().and_then(|panel| panel.clone().downcast::<MailWindow>().ok()).map(|view| (index, view)));
        if let Some((index, view)) = existing {
            view.update(cx, |mail, cx| mail.show(email, adding, cx));
            self.select_tab(index, window, cx);
            return;
        }
        let files = cx.entity().downgrade();
        let view = cx.new(|cx| MailWindow::new(window, cx, email, adding, true, appearance, Some(files)));
        self.open_panel_tab("Mail", IconName::Mail, view.into(), window, cx);
    }

    pub(crate) fn open_mail_result(&mut self, result: files_ext_mail::MailResult, window: &mut Window, cx: &mut Context<Self>) {
        let appearance = self.appearance.clone();
        let email = result.account.email.clone();
        if self.appearance.read(cx).opens_in("mail") == crate::settings::OpenIn::Window {
            if let Some(view) = open_mail_window(Some(email), false, appearance, cx) {
                view.update(cx, |mail, cx| mail.show_result(result, cx));
            }
            return;
        }
        let existing = self
            .tabs
            .iter()
            .enumerate()
            .find_map(|(index, tab)| tab.panel.as_ref().and_then(|panel| panel.clone().downcast::<MailWindow>().ok()).map(|view| (index, view)));
        if let Some((index, view)) = existing {
            view.update(cx, |mail, cx| mail.show_result(result, cx));
            self.select_tab(index, window, cx);
            return;
        }
        let files = cx.entity().downgrade();
        let view = cx.new(|cx| MailWindow::new(window, cx, Some(email), false, true, appearance, Some(files)));
        view.update(cx, |mail, cx| mail.show_result(result, cx));
        self.open_panel_tab("Mail", IconName::Mail, view.into(), window, cx);
    }

    /// Context menu → Send as Attachment…: the selected files, or this one.
    pub(crate) fn send_as_attachment(&mut self, entry: crate::model::Entry, window: &mut Window, cx: &mut Context<Self>) {
        let files = self.attachment_files(entry);
        if accounts().is_empty() {
            self.notice = Some("Add a mail account first: sidebar → Mail → Add account…".to_string());
            self.open_mail(None, true, window, cx);
        } else {
            open_compose(files, None, self.appearance.clone(), cx);
        }
        cx.notify();
    }

    /// The right-clicked file, or every selected file when it is one of them.
    pub(crate) fn attachment_files(&self, entry: crate::model::Entry) -> Vec<PathBuf> {
        let selected: Vec<PathBuf> = self.selected_paths().into_iter().filter(|path| path.is_file()).collect();
        match entry.path.as_deref() {
            Some(path) if !selected.iter().any(|file| file == path) => vec![path.to_path_buf()],
            _ => selected,
        }
    }
}

/// Sends scheduled mail that is due (checked about once a minute).
pub(crate) fn send_due_mail(cx: &mut App) {
    static SENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if SENDING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let now = crate::updates::now_seconds();
    cx.background_executor()
        .spawn(async move {
            for (message, result) in files_ext_mail::send_due(now) {
                let (level, title, body) = match result {
                    Ok(()) => (files_sdk::log::Level::Success, format!("Sent \"{}\"", message.draft.subject), format!("Scheduled, to {}", message.draft.to)),
                    Err(error) => (files_sdk::log::Level::Error, format!("Not sent: \"{}\"", message.draft.subject), format!("{error} · retrying in a minute")),
                };
                files_sdk::log::notify(files_ext_mail::MANIFEST.id, level, title, Some(body));
            }
            SENDING.store(false, std::sync::atomic::Ordering::SeqCst);
        })
        .detach();
}

/// Mail's place in the extension catalog (for "is it turned on").
pub(crate) fn mail_index() -> usize {
    crate::extensions::CATALOG.iter().position(|manifest| manifest.id == files_ext_mail::MANIFEST.id).unwrap_or(usize::MAX)
}

// ------------------------------------------------------------- Mail view

/// What the message list's context menu does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum MailAction {
    Read(bool),
    Star(bool),
    Spam,
    Delete,
}

pub(crate) struct MailWindow {
    appearance: Entity<AppearanceSettings>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
    /// Shown as a tab of the main window (Cmd-W closes the tab, not this).
    in_tab: bool,
    /// The details panel (sender, IP, MIME…): its own button, or the
    /// inspector button when Mail is a tab.
    show_details: bool,
    explorer: Option<gpui_kit::WeakEntity<FileExplorer>>,
    account: Option<Account>,
    folders: Vec<Folder>,
    folder: String,
    special: Option<Special>,
    messages: Vec<Summary>,
    open: Option<Message>,
    selected: Option<u32>,
    busy: bool,
    notice: Option<String>,
    adding: bool,
    provider: Provider,
    search: Entity<InputState>,
    email: Entity<InputState>,
    name: Entity<InputState>,
    password: Entity<InputState>,
    imap: Entity<InputState>,
    smtp: Entity<InputState>,
    /// Bumped on every load, so a slow answer for an old folder is dropped.
    generation: u64,
}

impl Focusable for MailWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl MailWindow {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        email: Option<String>,
        adding: bool,
        in_tab: bool,
        appearance: Entity<AppearanceSettings>,
        explorer: Option<gpui_kit::WeakEntity<FileExplorer>>,
    ) -> Self {
        let input = |placeholder: &str, window: &mut Window, cx: &mut Context<Self>| {
            let placeholder = placeholder.to_string();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let focus_handle = cx.focus_handle();
        if !in_tab {
            window.focus(&focus_handle, cx);
        }
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search mail · from: to: subject: is:unread email:you@…:to:…"));
        let subscriptions = vec![
            cx.observe(&appearance, |_, _, cx| cx.notify()),
            cx.subscribe(&search, |this: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.run_search(cx);
                }
            }),
        ];
        let mut subscriptions = subscriptions;
        if let Some(files) = explorer.as_ref().and_then(gpui_kit::WeakEntity::upgrade) {
            subscriptions.push(cx.observe(&files, |_, _, cx| cx.notify()));
        }
        let all = accounts();
        let account = email.and_then(|email| all.iter().find(|account| account.email == email).cloned()).or_else(|| all.first().cloned());
        let mut this = Self {
            _subscriptions: subscriptions,
            appearance,
            focus_handle,
            in_tab,
            show_details: false,
            explorer: explorer.clone(),
            adding: adding || account.is_none(),
            account,
            folders: Vec::new(),
            folder: "INBOX".to_string(),
            special: Some(Special::Inbox),
            messages: Vec::new(),
            open: None,
            selected: None,
            busy: false,
            notice: None,
            provider: Provider::Gmail,
            search,
            email: input("you@gmail.com", window, cx),
            name: input("Your name (optional)", window, cx),
            password: cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("App password")),
            imap: input("imap.example.com:993", window, cx),
            smtp: input("smtp.example.com:465", window, cx),
            generation: 0,
        };
        this.load(cx);
        this
    }

    /// Brings an existing Mail view to `email`, or to the sign-in form.
    fn show(&mut self, email: Option<String>, adding: bool, cx: &mut Context<Self>) {
        if adding {
            self.adding = true;
        } else if let Some(account) = email.and_then(|email| accounts().into_iter().find(|account| account.email == email)) {
            self.select_account(account, cx);
        }
        cx.notify();
    }

    fn show_result(&mut self, result: files_ext_mail::MailResult, cx: &mut Context<Self>) {
        self.generation += 1;
        self.account = Some(result.account);
        self.adding = false;
        self.busy = false;
        self.notice = None;
        if !self.folders.iter().any(|folder| folder.name == result.folder.name) {
            self.folders.push(result.folder.clone());
        }
        self.folder = result.folder.name;
        self.special = result.folder.special;
        self.messages = vec![result.summary.clone()];
        self.open = None;
        self.selected = None;
        self.open_message(result.summary.uid, cx);
        cx.notify();
    }

    fn details_open(&self, cx: &App) -> bool {
        self.show_details || self.explorer.as_ref().and_then(gpui_kit::WeakEntity::upgrade).is_some_and(|files| files.read(cx).show_inspector)
    }

    fn close(&mut self, _: &CloseMail, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_tab {
            cx.propagate();
        } else {
            window.remove_window();
        }
    }

    /// Folders and the current folder's messages: what is in memory at once,
    /// then fresh from the server.
    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self.account.clone().filter(|_| !self.adding) else { return };
        if self.special == Some(Special::Scheduled) {
            cx.notify();
            return;
        }
        let folder = self.folder.clone();
        if let Some(cached) = files_ext_mail::cached_messages(&account, &folder) {
            self.messages = cached;
        }
        self.generation += 1;
        let generation = self.generation;
        self.busy = true;
        self.notice = self.messages.is_empty().then(|| format!("Loading {}…", account.email));
        cx.notify();
        let need_folders = self.folders.is_empty();
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn({
                    let account = account.clone();
                    let folder = folder.clone();
                    async move {
                        let folders = if need_folders { Some(files_ext_mail::folders(&account)?) } else { None };
                        Ok::<_, std::io::Error>((folders, files_ext_mail::messages(&account, &folder)?))
                    }
                })
                .await;
            let ahead: Vec<u32> = match &loaded {
                Ok((_, messages)) => messages.iter().take(PREFETCH).map(|summary| summary.uid).collect(),
                Err(_) => Vec::new(),
            };
            let current = this
                .update(cx, |this, cx| {
                    if this.generation != generation {
                        return false;
                    }
                    this.busy = false;
                    match loaded {
                        Ok((folders, messages)) => {
                            this.notice = None;
                            if let Some(folders) = folders {
                                this.folders = folders;
                            }
                            this.messages = messages;
                        }
                        Err(error) => this.notice = Some(error.to_string()),
                    }
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if current && !ahead.is_empty() && !crate::power::power_saving() {
                cx.background_executor().spawn(async move { files_ext_mail::prefetch(&account, &folder, &ahead) }).await;
            }
        })
        .detach();
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().to_string();
        let search = Search::parse(&query);
        let Some(account) = account_for(&search, self.account.as_ref()) else {
            self.notice = Some(format!("No account {}", search.account.unwrap_or_default()));
            cx.notify();
            return;
        };
        if self.account.as_ref() != Some(&account) {
            self.account = Some(account.clone());
            self.folders.clear();
        }
        if search.criteria.is_empty() {
            self.load(cx);
            return;
        }
        // Scheduled mail is local; search the Inbox instead.
        if self.special == Some(Special::Scheduled) {
            self.folder = "INBOX".to_string();
            self.special = Some(Special::Inbox);
        }
        let folder = self.folder.clone();
        self.generation += 1;
        let generation = self.generation;
        self.busy = true;
        self.notice = Some(format!("Searching {}…", account.email));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let found = cx.background_executor().spawn(async move { files_ext_mail::search(&account, &folder, &search) }).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match found {
                    Ok(messages) => {
                        this.notice = Some(format!("{} found", messages.len()));
                        this.messages = messages;
                    }
                    Err(error) => this.notice = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_folder(&mut self, folder: String, special: Option<Special>, cx: &mut Context<Self>) {
        self.folder = folder;
        self.special = special;
        self.open = None;
        self.selected = None;
        self.messages.clear();
        self.load(cx);
    }

    fn select_account(&mut self, account: Account, cx: &mut Context<Self>) {
        self.account = Some(account);
        self.adding = false;
        self.folders.clear();
        self.select_folder("INBOX".to_string(), Some(Special::Inbox), cx);
    }

    fn open_message(&mut self, uid: u32, cx: &mut Context<Self>) {
        let Some(account) = self.account.clone() else { return };
        let folder = self.folder.clone();
        self.selected = Some(uid);
        // Read before (or read ahead): no round trip.
        if let Some(message) = files_ext_mail::cached_message(&account, &folder, uid) {
            self.mark_seen(uid);
            self.open = Some(message);
            cx.notify();
            return;
        }
        self.open = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let message = cx.background_executor().spawn(async move { files_ext_mail::message(&account, &folder, uid) }).await;
            let _ = this.update(cx, |this, cx| {
                match message {
                    Ok(message) if this.selected == Some(uid) => {
                        this.mark_seen(uid);
                        this.open = Some(message);
                    }
                    Ok(_) => {}
                    Err(error) => this.notice = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn folder_with(&self, special: Special) -> Option<String> {
        self.folders.iter().find(|folder| folder.special == Some(special)).map(|folder| folder.name.clone())
    }

    /// Read / unread, star, Spam, Delete (to Trash; for good inside Trash).
    fn act(&mut self, uid: u32, action: MailAction, cx: &mut Context<Self>) {
        let Some(account) = self.account.clone() else { return };
        let folder = self.folder.clone();
        let trash = self.folder_with(Special::Trash).filter(|trash| *trash != folder);
        let junk = self.folder_with(Special::Junk);
        let removes = matches!(action, MailAction::Spam | MailAction::Delete);
        // Shown at once; put back if the server says no.
        let before = self.messages.clone();
        match action {
            MailAction::Read(seen) => {
                if let Some(summary) = self.messages.iter_mut().find(|summary| summary.uid == uid) {
                    summary.seen = seen;
                }
            }
            MailAction::Star(_) => {}
            MailAction::Spam | MailAction::Delete => {
                self.messages.retain(|summary| summary.uid != uid);
                if self.selected == Some(uid) {
                    self.selected = None;
                    self.open = None;
                }
            }
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_executor()
                .spawn(async move {
                    match action {
                        MailAction::Read(seen) => files_ext_mail::set_seen(&account, &folder, uid, seen),
                        MailAction::Star(on) => files_ext_mail::set_flagged(&account, &folder, uid, on),
                        MailAction::Spam => match junk {
                            Some(junk) => files_ext_mail::move_to(&account, &folder, uid, &junk),
                            None => Err(std::io::Error::other("This account has no Spam folder")),
                        },
                        MailAction::Delete => match trash {
                            Some(trash) => files_ext_mail::move_to(&account, &folder, uid, &trash),
                            None => files_ext_mail::delete_forever(&account, &folder, uid),
                        },
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.notice = Some(match done {
                    Ok(()) => match action {
                        MailAction::Read(true) => "Marked as read".to_string(),
                        MailAction::Read(false) => "Marked as unread".to_string(),
                        MailAction::Star(true) => "Starred".to_string(),
                        MailAction::Star(false) => "Star removed".to_string(),
                        MailAction::Spam => "Moved to Spam".to_string(),
                        MailAction::Delete => "Deleted".to_string(),
                    },
                    Err(error) => {
                        if removes || matches!(action, MailAction::Read(_)) {
                            this.messages = before;
                        }
                        error.to_string()
                    }
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// Reply from the list: the message is read first when it is not in memory.
    fn reply(&mut self, uid: u32, cx: &mut Context<Self>) {
        let Some(account) = self.account.clone() else { return };
        let folder = self.folder.clone();
        let appearance = self.appearance.clone();
        cx.spawn(async move |this, cx| {
            let message = cx.background_executor().spawn(async move { files_ext_mail::message(&account, &folder, uid) }).await;
            let _ = this.update(cx, |this, cx| match message {
                Ok(message) => open_reply(&message, appearance, cx),
                Err(error) => {
                    this.notice = Some(error.to_string());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn mark_seen(&mut self, uid: u32) {
        if let Some(summary) = self.messages.iter_mut().find(|summary| summary.uid == uid) {
            summary.seen = true;
        }
    }

    fn save_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(attachment) = self.open.as_ref().and_then(|message| message.attachments.get(index)).cloned() else { return };
        let Some(downloads) = dirs::download_dir() else { return };
        self.notice = Some(match files_ext_mail::save_attachment(&attachment, &downloads) {
            Ok(path) => {
                // What to do with it next, right on the notification.
                let actions = crate::views::console::file_actions(&path);
                files_sdk::log::notify_actions(
                    files_ext_mail::MANIFEST.id,
                    files_sdk::log::Level::Success,
                    format!("Saved {}", path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()),
                    Some(path.display().to_string()),
                    actions,
                );
                format!("Saved to {}", path.display())
            }
            Err(error) => format!("Could not save {}: {error}", attachment.name),
        });
        cx.notify();
    }

    fn add_account(&mut self, cx: &mut Context<Self>) {
        let email = self.email.read(cx).value().trim().to_string();
        let name = self.name.read(cx).value().to_string();
        let password = self.password.read(cx).value().to_string();
        let account = match Account::for_provider(&email, &name, self.provider) {
            Some(account) => account,
            None => {
                let server = |text: &str, port: u16| {
                    let text = text.trim();
                    let (host, port) = text.rsplit_once(':').and_then(|(host, value)| Some((host, value.parse().ok()?))).unwrap_or((text, port));
                    files_ext_mail::Server::new(host, port)
                };
                let imap = server(&self.imap.read(cx).value(), 993);
                let smtp = server(&self.smtp.read(cx).value(), 465);
                if imap.host.is_empty() || smtp.host.is_empty() {
                    self.notice = Some("Enter the IMAP and SMTP servers".to_string());
                    cx.notify();
                    return;
                }
                Account { email: email.clone(), name: name.trim().to_string(), provider: Provider::Other, imap, smtp }
            }
        };
        self.busy = true;
        self.notice = Some(format!("Signing in as {email}…"));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let added = cx
                .background_executor()
                .spawn({
                    let account = account.clone();
                    async move { files_ext_mail::add_account(account, &password) }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match added {
                    Ok(()) => {
                        accounts_changed();
                        this.notice = None;
                        this.select_account(account, cx);
                    }
                    Err(error) => this.notice = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn remove_account(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self.account.take() else { return };
        if let Err(error) = files_ext_mail::remove_account(&account.email) {
            self.notice = Some(error.to_string());
        }
        accounts_changed();
        self.messages.clear();
        self.folders.clear();
        self.open = None;
        match accounts().first().cloned() {
            Some(next) => self.select_account(next, cx),
            None => self.adding = true,
        }
        cx.notify();
    }

    fn render_add(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let help = self.provider.app_password_url().map(|url| {
            let url = url.to_string();
            h_flex()
                .gap_2()
                .items_center()
                .child(div().text_xs().text_color(palette.muted).child(match self.provider {
                    Provider::Gmail => "Google asks for an app password (needs 2-Step Verification):",
                    _ => "Yandex asks for an app password for mail:",
                }))
                .child(Button::new("mail-app-password").link().xsmall().label("Create one").on_click(move |_, _, cx| cx.open_url(&url)))
        });
        let field = |label: &'static str, input: &Entity<InputState>| {
            v_flex().gap_1().child(div().text_xs().text_color(palette.muted).child(label)).child(Input::new(input).w_full())
        };
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(px(440.))
                    .gap_3()
                    .p_5()
                    .rounded_lg()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border)
                    .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Add a mail account"))
                    .child(h_flex().gap_1().children(Provider::ALL.map(|provider| {
                        let button = Button::new(gpui_kit::ElementId::Name(format!("mail-provider-{}", provider.name()).into()))
                            .small()
                            .label(provider.name());
                        let button = if provider == self.provider { button.primary() } else { button.secondary() };
                        button.on_click(cx.listener(move |this, _, _, cx| {
                            this.provider = provider;
                            cx.notify();
                        }))
                    })))
                    .child(field("Address", &self.email))
                    .child(field("Name shown to people you write to", &self.name))
                    .child(field("App password", &self.password))
                    .children(help)
                    .when(self.provider == Provider::Other, |this| {
                        this.child(field("IMAP server (TLS)", &self.imap)).child(field("SMTP server (TLS)", &self.smtp))
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_end()
                            .when(self.account.is_some(), |this| {
                                this.child(Button::new("mail-add-cancel").ghost().small().label("Cancel").on_click(cx.listener(|this, _, _, cx| {
                                    this.adding = false;
                                    this.load(cx);
                                })))
                            })
                            .child(
                                Button::new("mail-add-sign-in")
                                    .primary()
                                    .small()
                                    .label("Sign in")
                                    .loading(self.busy)
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, _, cx| this.add_account(cx))),
                            ),
                    )
                    .child(div().text_xs().text_color(palette.faint).child(
                        "The password stays in your system keychain. \"Sign in with Google / Yandex\" comes in a later version.",
                    )),
            )
            .into_any_element()
    }

    /// Folders like the sidebar: special ones first, with their icon and color.
    fn render_folders(&self, settings: &crate::settings::AppSettings, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let hidden = &settings.mail.hidden;
        let mut folders: Vec<Folder> =
            self.folders.iter().filter(|folder| folder.special.is_none_or(|special| !hidden.iter().any(|key| key == special.key()))).cloned().collect();
        // Scheduled mail lives on this computer, after Sent.
        if !hidden.iter().any(|key| key == Special::Scheduled.key()) {
            let at = folders.iter().position(|folder| folder.special.is_none_or(|special| special > Special::Sent)).unwrap_or(folders.len());
            folders.insert(at, Folder { name: String::new(), label: "Scheduled".into(), special: Some(Special::Scheduled) });
        }
        let scheduled = files_ext_mail::scheduled().len();
        let rows = folders.into_iter().enumerate().map(|(index, folder)| {
            let selected = if folder.special == Some(Special::Scheduled) {
                self.special == Some(Special::Scheduled)
            } else {
                folder.name == self.folder && self.special != Some(Special::Scheduled)
            };
            let color = special_color(settings, folder.special);
            let (name, special) = (folder.name.clone(), folder.special);
            h_flex()
                .id(("mail-folder", index))
                .px_2()
                .h(px(30.))
                .gap_2()
                .items_center()
                .rounded_md()
                .text_sm()
                .text_color(if selected { palette.text } else { palette.muted })
                .bg(if selected { palette.selection } else { palette.sidebar })
                .hover(|style| style.bg(palette.hover).text_color(palette.text))
                .child(Icon::new(special_icon(folder.special)).small().text_color(color.unwrap_or(if selected { palette.accent } else { palette.muted })))
                .child(div().flex_1().min_w_0().truncate().child(folder.special.map_or(folder.label.clone(), |special| special.label().to_string())))
                .when(special == Some(Special::Scheduled) && scheduled > 0, |this| {
                    this.child(div().text_xs().text_color(palette.faint).child(scheduled.to_string()))
                })
                .on_click(cx.listener(move |this, _, _, cx| this.select_folder(name.clone(), special, cx)))
        });
        v_flex()
            .id("mail-folders")
            .w(px(200.))
            .flex_shrink_0()
            .h_full()
            .p_2()
            .gap_0p5()
            .bg(palette.sidebar)
            .border_r_1()
            .border_color(palette.border)
            .overflow_y_scrollbar()
            .children(rows)
            .into_any_element()
    }

    fn render_list(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        if self.special == Some(Special::Scheduled) {
            return self.render_scheduled(palette, cx);
        }
        let view = cx.entity();
        let has_junk = self.folder_with(Special::Junk).is_some();
        let in_junk = self.special == Some(Special::Junk);
        let in_trash = self.special == Some(Special::Trash);
        let rows = self.messages.iter().map(|summary| {
            let uid = summary.uid;
            let selected = self.selected == Some(uid);
            h_flex()
                .id(("mail-message", uid as usize))
                .px_3()
                .py_2()
                .gap_2()
                .items_start()
                .border_b_1()
                .border_color(palette.border)
                .bg(if selected { palette.selection } else { palette.window })
                .hover(|style| style.bg(palette.hover))
                .child(div().mt(px(6.)).size(px(7.)).flex_shrink_0().rounded_full().when(!summary.seen, |this| this.bg(palette.accent)))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .when(!summary.seen, |this| this.font_weight(FontWeight::SEMIBOLD))
                                        .child(summary.from.clone()),
                                )
                                .child(div().text_xs().text_color(palette.faint).child(time(summary.date))),
                        )
                        .child(div().truncate().text_xs().text_color(if summary.seen { palette.muted } else { palette.text }).child(summary.subject.clone())),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.open_message(uid, cx)))
                .context_menu({
                    let view = view.clone();
                    let (seen, address) = (summary.seen, summary.address.clone());
                    move |menu, window, _| {
                        let on = |action: MailAction| window.listener_for(&view, move |this: &mut MailWindow, _, _, cx| this.act(uid, action, cx));
                        let address = address.clone();
                        menu.item(PopupMenuItem::new("Reply").icon(IconName::Reply).on_click(window.listener_for(&view, move |this: &mut MailWindow, _, _, cx| this.reply(uid, cx))))
                            .separator()
                            .item(PopupMenuItem::new(if seen { "Mark as Unread" } else { "Mark as Read" }).icon(if seen { IconName::Mail } else { IconName::MailOpen }).on_click(on(MailAction::Read(!seen))))
                            .item(PopupMenuItem::new("Star").icon(IconName::Star).on_click(on(MailAction::Star(true))))
                            .item(PopupMenuItem::new("Remove Star").icon(IconName::StarOff).on_click(on(MailAction::Star(false))))
                            .item(PopupMenuItem::new("Copy Sender Address").icon(IconName::Copy).disabled(address.is_empty()).on_click(move |_, _, cx| {
                                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(address.clone()));
                            }))
                            .separator()
                            .item(PopupMenuItem::new("Move to Spam").icon(IconName::ShieldAlert).disabled(!has_junk || in_junk).on_click(on(MailAction::Spam)))
                            .item(PopupMenuItem::new(if in_trash { "Delete Forever" } else { "Delete" }).icon(IconName::Trash).on_click(on(MailAction::Delete)))
                    }
                })
        });
        let empty = self.messages.is_empty() && !self.busy;
        v_flex()
            .id("mail-list")
            .w(px(340.))
            .flex_shrink_0()
            .h_full()
            .border_r_1()
            .border_color(palette.border)
            .overflow_y_scrollbar()
            .children(rows)
            .when(empty, |this| this.child(div().p_4().text_sm().text_color(palette.faint).child("No messages here")))
            .into_any_element()
    }

    fn render_scheduled(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let queued = files_ext_mail::scheduled();
        let empty = queued.is_empty();
        let rows = queued.into_iter().map(|message: Scheduled| {
            let id = message.id;
            v_flex()
                .px_3()
                .py_2()
                .gap_0p5()
                .border_b_1()
                .border_color(palette.border)
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().min_w_0().truncate().text_sm().child(format!("To {}", message.draft.to)))
                        .child(
                            Button::new(("mail-cancel-scheduled", id as usize))
                                .ghost()
                                .xsmall()
                                .icon(IconName::X)
                                .tooltip("Cancel: do not send")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Err(error) = files_ext_mail::cancel(id) {
                                        this.notice = Some(error.to_string());
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .child(div().truncate().text_xs().text_color(palette.muted).child(message.draft.subject.clone()))
                .child(div().text_xs().text_color(palette.accent).child(format!("{} · from {}", long_time(message.send_at), message.account)))
                .children(message.error.map(|error| div().text_xs().text_color(palette.muted).child(format!("Last try: {error}"))))
        });
        v_flex()
            .id("mail-scheduled-list")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scrollbar()
            .children(rows)
            .when(empty, |this| {
                this.child(div().p_4().text_sm().text_color(palette.faint).child("Nothing scheduled. In a new message: Send ▾ → Send later."))
            })
            .into_any_element()
    }

    fn render_message(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(message) = &self.open else {
            let text = if self.selected.is_some() { "Loading the message…" } else { "Pick a message" };
            return div().flex_1().flex().items_center().justify_center().text_sm().text_color(palette.faint).child(text).into_any_element();
        };
        let reply = message.clone();
        let attachments = message.attachments.iter().enumerate().map(|(index, attachment)| {
            let extension = std::path::Path::new(&attachment.name).extension().map(|extension| extension.to_string_lossy().into_owned()).unwrap_or_default();
            h_flex()
                .gap_2()
                .px_2()
                .py_1()
                .items_center()
                .rounded_md()
                .bg(palette.surface)
                .child(crate::views::files::kind_icon(crate::model::FileKind::from_extension(&extension), palette, 16.))
                .child(div().flex_1().min_w_0().truncate().text_sm().child(attachment.name.clone()))
                .child(div().text_xs().text_color(palette.faint).child(crate::settings::format_size(attachment.bytes.len() as u64, Default::default())))
                .child(
                    Button::new(("mail-attachment-save", index))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Download)
                        .tooltip("Save to Downloads")
                        .on_click(cx.listener(move |this, _, _, cx| this.save_attachment(index, cx))),
                )
        });
        let recipients = if message.cc.is_empty() { message.to.clone() } else { format!("{} · cc {}", message.to, message.cc) };
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(
                v_flex()
                    .px_4()
                    .py_3()
                    .gap_1()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_start()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(selectable(("mail-subject", message.uid as usize), &message.subject)),
                            )
                            .child(Button::new("mail-reply").small().ghost().icon(IconName::Reply).label("Reply").on_click(cx.listener(
                                move |this, _, _, cx| open_reply(&reply, this.appearance.clone(), cx),
                            ))),
                    )
                    .child(div().text_xs().text_color(palette.muted).child(selectable(("mail-from", message.uid as usize), &message.from)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.faint)
                            .child(selectable(("mail-to", message.uid as usize), &format!("to {recipients} · {}", time(message.date)))),
                    ),
            )
            .when(!message.attachments.is_empty(), |this| {
                this.child(v_flex().px_4().py_2().gap_1().border_b_1().border_color(palette.border).children(attachments))
            })
            .child(
                div()
                    .id("mail-body")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .p_4()
                    .overflow_y_scrollbar()
                    .child(if message.body.is_empty() && message.html.is_none() {
                        div().text_sm().text_color(palette.faint).child("(no text)").into_any_element()
                    } else {
                        render_body(message)
                    }),
            )
            .into_any_element()
    }
}

impl Render for MailWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&settings);
        let all = accounts();
        let current = self.account.as_ref().map(|account| account.email.clone());
        let tabs = all.into_iter().map(|account| {
            let selected = Some(&account.email) == current.as_ref() && !self.adding;
            let button = Button::new(gpui_kit::ElementId::Name(format!("mail-tab-{}", account.email).into())).small().icon(IconName::Mail).label(account.email.clone());
            let button = if selected { button.primary() } else { button.ghost() };
            button.on_click(cx.listener(move |this, _, _, cx| this.select_account(account.clone(), cx)))
        });
        let header = h_flex()
            .px_3()
            .py_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(palette.border)
            .child(h_flex().flex_shrink_0().max_w(px(420.)).gap_1().overflow_hidden().children(tabs))
            .child(
                div()
                    .flex_1()
                    .min_w(px(180.))
                    .child(Input::new(&self.search).small().cleanable(true).prefix(Icon::new(IconName::Search).small().text_color(palette.muted))),
            )
            .child(Button::new("mail-add").ghost().small().icon(IconName::MailPlus).tooltip("Add account").on_click(cx.listener(|this, _, _, cx| {
                this.adding = true;
                cx.notify();
            })))
            .when(self.account.is_some() && !self.adding, |this| {
                this.child(
                    Button::new("mail-refresh")
                        .ghost()
                        .small()
                        .icon(IconName::RefreshCw)
                        .tooltip("Check for new mail")
                        .loading(self.busy)
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                )
                .child(
                    Button::new("mail-details")
                        .ghost()
                        .small()
                        .icon(IconName::Info)
                        .tooltip("Details: sender, IP, MIME, headers")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_details = !this.show_details;
                            cx.notify();
                        })),
                )
                .child(Button::new("mail-remove").ghost().small().icon(IconName::Trash).tooltip("Remove this account").on_click(cx.listener(
                    |this, _, _, cx| this.remove_account(cx),
                )))
                .child(Button::new("mail-compose").primary().small().icon(IconName::MailPen).label("New message").on_click(cx.listener(
                    |this, _, _, cx| {
                        open_compose(Vec::new(), None, this.appearance.clone(), cx);
                    },
                )))
            });
        let body = if self.adding {
            self.render_add(palette, cx)
        } else {
            h_flex()
                .size_full()
                .child(self.render_folders(&settings, palette, cx))
                .child(self.render_list(palette, cx))
                .when(self.special != Some(Special::Scheduled), |this| this.child(self.render_message(palette, cx)))
                .children(self.details_open(cx).then(|| self.open.as_ref().map(|message| {
                    div()
                        .id("mail-details")
                        .w(px(320.))
                        .flex_shrink_0()
                        .h_full()
                        .border_l_1()
                        .border_color(palette.border)
                        .bg(palette.sidebar)
                        .overflow_y_scrollbar()
                        .child(render_details(message, palette))
                })).flatten())
                .into_any_element()
        };
        v_flex()
            .key_context("Mail")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            .size_full()
            .bg(palette.window)
            .text_color(palette.text)
            .child(header)
            .child(div().flex_1().min_h_0().child(body))
            .children(self.notice.as_deref().map(|notice| notice_row(notice, palette)))
    }
}

pub(crate) fn open_mail_window(email: Option<String>, adding: bool, appearance: Entity<AppearanceSettings>, cx: &mut App) -> Option<Entity<MailWindow>> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds { origin: point(px(160.), px(80.)), size: size(px(1180.), px(740.)) })),
        window_min_size: Some(size(px(760.), px(460.))),
        titlebar: Some(gpui_kit::TitlebarOptions { title: Some("Mail".into()), ..Default::default() }),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, move |window, cx| cx.new(|cx| MailWindow::new(window, cx, email, adding, false, appearance, None)))
        .ok()
        .map(|(_, view)| view)
}

// ---------------------------------------------------------------- compose

/// When "Send later" sends: in an hour, this evening, tomorrow morning…
pub(crate) fn later_choices(now: chrono::DateTime<chrono::Local>) -> Vec<(String, i64)> {
    use chrono::{Datelike as _, Duration, TimeZone as _};
    let at = |date: chrono::NaiveDate, hour: u32| chrono::Local.from_local_datetime(&date.and_hms_opt(hour, 0, 0).expect("valid hour")).earliest();
    let today = now.date_naive();
    let mut choices = vec![("In 1 hour".to_string(), (now + Duration::hours(1)).timestamp())];
    if let Some(evening) = at(today, 18).filter(|evening| *evening > now + Duration::minutes(30)) {
        choices.push(("This evening, 18:00".to_string(), evening.timestamp()));
    }
    if let Some(morning) = at(today + Duration::days(1), 9) {
        choices.push(("Tomorrow, 09:00".to_string(), morning.timestamp()));
    }
    let to_monday = (7 - today.weekday().num_days_from_monday()) % 7;
    let monday = today + Duration::days(if to_monday == 0 { 7 } else { to_monday.into() });
    if let Some(monday) = at(monday, 9) {
        choices.push((format!("Monday {}, 09:00", monday.format("%b %d")), monday.timestamp()));
    }
    choices
}

pub(crate) struct ComposeWindow {
    appearance: Entity<AppearanceSettings>,
    focus_handle: FocusHandle,
    _subscription: Subscription,
    from: usize,
    to: Entity<InputState>,
    cc: Entity<InputState>,
    bcc: Entity<InputState>,
    show_copies: bool,
    subject: Entity<InputState>,
    body: Entity<TextareaState>,
    /// "Send at": `YYYY-MM-DD HH:MM`.
    later: Entity<InputState>,
    attachments: Vec<PathBuf>,
    sending: bool,
    notice: Option<String>,
}

impl Focusable for ComposeWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ComposeWindow {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        attachments: Vec<PathBuf>,
        reply: Option<(String, String, String)>,
        appearance: Entity<AppearanceSettings>,
    ) -> Self {
        let (to, subject, body) = reply.unwrap_or_default();
        let subject = if subject.is_empty() && !attachments.is_empty() {
            attachments.iter().filter_map(|path| path.file_name()).map(|name| name.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ")
        } else {
            subject
        };
        let field = |placeholder: &str, value: String, window: &mut Window, cx: &mut Context<Self>| {
            let placeholder = placeholder.to_string();
            cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder(placeholder);
                input.set_value(value, window, cx);
                input
            })
        };
        let to = field("name@example.com, …", to, window, cx);
        let cc = field("Copy to…", String::new(), window, cx);
        let bcc = field("Hidden copy to… (nobody sees these)", String::new(), window, cx);
        let subject = field("Subject", subject, window, cx);
        let later = field("2026-11-02 09:00", String::new(), window, cx);
        let body = cx.new(|cx| {
            let mut input = TextareaState::new(window, cx).rows(12).placeholder("Message");
            input.set_value(body, window, cx);
            input
        });
        let focus_handle = cx.focus_handle();
        window.focus(&to.focus_handle(cx), cx);
        Self {
            _subscription: cx.observe(&appearance, |_, _, cx| cx.notify()),
            appearance,
            focus_handle,
            from: 0,
            to,
            cc,
            bcc,
            show_copies: false,
            subject,
            body,
            later,
            attachments,
            sending: false,
            notice: None,
        }
    }

    fn close(&mut self, _: &CloseMail, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    pub(crate) fn draft(&self, cx: &App) -> files_ext_mail::Draft {
        files_ext_mail::Draft {
            to: self.to.read(cx).value().to_string(),
            cc: self.cc.read(cx).value().to_string(),
            bcc: self.bcc.read(cx).value().to_string(),
            subject: self.subject.read(cx).value().to_string(),
            body: self.body.read(cx).value().to_string(),
            attachments: self.attachments.clone(),
        }
    }

    fn attach(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Attach".into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                let _ = this.update(cx, |this, cx| {
                    for path in paths {
                        if !this.attachments.contains(&path) {
                            this.attachments.push(path);
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn account(&mut self, cx: &mut Context<Self>) -> Option<Account> {
        let account = accounts().get(self.from).cloned();
        if account.is_none() {
            self.notice = Some("Add a mail account first".to_string());
            cx.notify();
        }
        account
    }

    /// Queues the message for `send_at` and closes the window.
    fn send_later(&mut self, send_at: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(account) = self.account(cx) else { return };
        let draft = self.draft(cx);
        match files_ext_mail::schedule(&account, &draft, send_at) {
            Ok(_) => {
                files_sdk::log::notify(
                    files_ext_mail::MANIFEST.id,
                    files_sdk::log::Level::Info,
                    format!("Scheduled \"{}\"", draft.subject),
                    Some(format!("Goes out {} (while Files is open). Mail → Scheduled to cancel.", long_time(send_at))),
                );
                window.remove_window();
            }
            Err(error) => {
                self.notice = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn send_at_typed_time(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use chrono::TimeZone as _;
        let text = self.later.read(cx).value().trim().to_string();
        let parsed = chrono::NaiveDateTime::parse_from_str(&text, "%Y-%m-%d %H:%M")
            .ok()
            .and_then(|time| chrono::Local.from_local_datetime(&time).earliest())
            .filter(|time| *time > chrono::Local::now());
        match parsed {
            Some(time) => self.send_later(time.timestamp(), window, cx),
            None => {
                self.notice = Some("Type a future time as YYYY-MM-DD HH:MM".to_string());
                cx.notify();
            }
        }
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(account) = self.account(cx) else { return };
        let draft = self.draft(cx);
        if let Err(error) = files_ext_mail::build(&account, &draft) {
            self.notice = Some(error.to_string());
            cx.notify();
            return;
        }
        self.sending = true;
        self.notice = Some(format!("Sending from {}…", account.email));
        cx.notify();
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let sent = cx
                .background_executor()
                .spawn({
                    let draft = draft.clone();
                    async move { files_ext_mail::send(&account, &draft) }
                })
                .await;
            match sent {
                Ok(()) => {
                    files_sdk::log::notify(
                        files_ext_mail::MANIFEST.id,
                        files_sdk::log::Level::Success,
                        format!("Sent \"{}\"", draft.subject),
                        Some(format!("To {}", draft.to)),
                    );
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.sending = false;
                        this.notice = Some(error.to_string());
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }
}

impl Render for ComposeWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&settings);
        let all = accounts();
        let label = |text: &'static str| div().w(px(70.)).flex_shrink_0().text_xs().text_color(palette.muted).child(text);
        let senders = all.iter().enumerate().map(|(index, account)| {
            let button = Button::new(("compose-from", index)).xsmall().label(account.email.clone());
            let button = if index == self.from { button.primary() } else { button.secondary() };
            button.on_click(cx.listener(move |this, _, _, cx| {
                this.from = index;
                cx.notify();
            }))
        });
        let files = self.attachments.iter().enumerate().map(|(index, path)| {
            let size = std::fs::metadata(path).map(|metadata| crate::settings::format_size(metadata.len(), settings.human_sizes)).unwrap_or_default();
            let extension = path.extension().map(|extension| extension.to_string_lossy().into_owned()).unwrap_or_default();
            h_flex()
                .gap_1()
                .pl_2()
                .items_center()
                .rounded_md()
                .bg(palette.surface)
                .child(crate::views::files::kind_icon(crate::model::FileKind::from_extension(&extension), palette, 14.))
                .child(div().max_w(px(220.)).truncate().text_xs().child(path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()))
                .child(div().text_xs().text_color(palette.faint).child(size))
                .child(Button::new(("compose-detach", index)).ghost().xsmall().icon(IconName::X).on_click(cx.listener(move |this, _, _, cx| {
                    this.attachments.remove(index);
                    cx.notify();
                })))
        });
        let view = cx.entity();
        let send = DropdownButton::new("compose-send")
            .primary()
            .small()
            .button(
                Button::new("compose-send-now")
                    .icon(IconName::Send)
                    .label("Send")
                    .loading(self.sending)
                    .disabled(self.sending || all.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
            )
            .dropdown_menu(move |menu, window, _| {
                later_choices(chrono::Local::now()).into_iter().fold(menu.label("Send later"), |menu, (label, at)| {
                    menu.item(
                        PopupMenuItem::new(label)
                            .icon(IconName::Clock3)
                            .on_click(window.listener_for(&view, move |this: &mut ComposeWindow, _, window, cx| this.send_later(at, window, cx))),
                    )
                })
            });
        v_flex()
            .key_context("Mail")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            .size_full()
            .p_4()
            .gap_3()
            .bg(palette.window)
            .text_color(palette.text)
            .child(h_flex().gap_2().items_center().child(label("From")).child(h_flex().gap_1().flex_wrap().children(senders)))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(label("To"))
                    .child(div().flex_1().child(Input::new(&self.to).w_full()))
                    .when(!self.show_copies, |this| {
                        this.child(Button::new("compose-show-copies").ghost().xsmall().label("Cc / Bcc").on_click(cx.listener(|this, _, _, cx| {
                            this.show_copies = true;
                            cx.notify();
                        })))
                    }),
            )
            .when(self.show_copies, |this| {
                this.child(h_flex().gap_2().items_center().child(label("Cc")).child(div().flex_1().child(Input::new(&self.cc).w_full())))
                    .child(h_flex().gap_2().items_center().child(label("Bcc")).child(div().flex_1().child(Input::new(&self.bcc).w_full())))
            })
            .child(h_flex().gap_2().items_center().child(label("Subject")).child(div().flex_1().child(Input::new(&self.subject).w_full())))
            .child(div().flex_1().min_h_0().child(Textarea::new(&self.body).h_full()))
            .child(h_flex().gap_1().flex_wrap().children(files))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(Button::new("compose-attach").ghost().small().icon(IconName::Paperclip).label("Attach…").on_click(cx.listener(|this, _, _, cx| this.attach(cx))))
                    .child(div().flex_1())
                    .child(div().w(px(150.)).child(Input::new(&self.later).small()))
                    .child(
                        Button::new("compose-send-at")
                            .ghost()
                            .small()
                            .icon(IconName::Clock3)
                            .label("Send at")
                            .tooltip("Send at the typed time (YYYY-MM-DD HH:MM)")
                            .disabled(self.sending || all.is_empty())
                            .on_click(cx.listener(|this, _, window, cx| this.send_at_typed_time(window, cx))),
                    )
                    .child(send),
            )
            .children(self.notice.as_deref().map(|notice| div().text_xs().text_color(palette.muted).child(notice.to_string())))
    }
}

/// A new message, with `attachments` already attached, or a reply
/// (`to`, `subject`, quoted `body`).
pub(crate) fn open_compose(
    attachments: Vec<PathBuf>,
    reply: Option<(String, String, String)>,
    appearance: Entity<AppearanceSettings>,
    cx: &mut App,
) -> Option<Entity<ComposeWindow>> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds { origin: point(px(240.), px(120.)), size: size(px(760.), px(600.)) })),
        window_min_size: Some(size(px(520.), px(380.))),
        titlebar: Some(gpui_kit::TitlebarOptions { title: Some("New message".into()), ..Default::default() }),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, move |window, cx| cx.new(|cx| ComposeWindow::new(window, cx, attachments, reply, appearance)))
        .ok()
        .map(|(_, view)| view)
}

#[cfg(test)]
pub(crate) fn set_accounts_for_test(accounts: Vec<Account>) {
    *ACCOUNTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(accounts);
}

#[cfg(test)]
impl MailWindow {
    pub(crate) fn is_adding_for_test(&self) -> bool {
        self.adding
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_found_in_text() {
        let parts = linkify("See https://example.com/a?b=1, or mail mailto:x@y.z.");
        assert_eq!(
            parts,
            [
                ("See ".to_string(), false),
                ("https://example.com/a?b=1".to_string(), true),
                (", or mail ".to_string(), false),
                ("mailto:x@y.z".to_string(), true),
                (".".to_string(), false),
            ]
        );
        assert_eq!(linkify("no links"), [("no links".to_string(), false)]);
    }

    #[test]
    fn plain_mail_keeps_lines_and_html_loses_scripts() {
        assert_eq!(plain_to_markdown("Hi *you*\nsee https://x.com/a_b"), "Hi \\*you\\*  \nsee <https://x.com/a_b>");
        let html = "<html><head><title>t</title></head><style>p{}</style><!-- note --><p>Hi</p><SCRIPT>alert(1)</SCRIPT></html>";
        assert_eq!(clean_html(html), "<html><p>Hi</p></html>");
    }

    #[test]
    fn send_later_offers_future_times_only() {
        use chrono::TimeZone as _;
        let friday_evening = chrono::Local.with_ymd_and_hms(2026, 10, 30, 20, 0, 0).unwrap();
        let choices = later_choices(friday_evening);
        let labels: Vec<_> = choices.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(labels, ["In 1 hour", "Tomorrow, 09:00", "Monday Nov 02, 09:00"], "18:00 has passed");
        assert!(choices.iter().all(|(_, at)| *at > friday_evening.timestamp()));
    }
}
