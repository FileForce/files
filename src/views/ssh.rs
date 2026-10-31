use std::cell::Cell;
use std::path::PathBuf;

use files_sdk::RemoteProvider as _;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Enter, Input, InputState};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    WeakEntity, Window, div, px,
};

use crate::app::{AppearanceSettings, FileExplorer};
use crate::model::{FileEntry, Location, Pane, SshConnection};
use crate::palette::Palette;

pub struct SshDialog {
    explorer: WeakEntity<FileExplorer>,
    previous_focus: Option<FocusHandle>,
    appearance: Entity<AppearanceSettings>,
    host: Entity<InputState>,
    user: Entity<InputState>,
    port: Entity<InputState>,
    /// Optional; saved in the keychain, never in settings.
    password: Entity<InputState>,
    error: Option<String>,
    /// The server is an interactive app (no shell): offer a terminal instead.
    interactive_only: bool,
    /// The server waits for a sign-in in the browser (Tailscale SSH…).
    auth: Option<crate::ssh::AuthPrompt>,
    connecting: bool,
    _appearance_subscription: Subscription,
}

impl SshDialog {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        explorer: WeakEntity<FileExplorer>,
        previous_focus: Option<FocusHandle>,
        appearance: Entity<AppearanceSettings>,
    ) -> Self {
        // Start from the last server used; a Host alias from ~/.ssh/config works too.
        let last = appearance
            .read(cx)
            .sidebar
            .ssh
            .first()
            .and_then(|address| SshConnection::parse(address));
        let host = input_with_value(window, cx, last.as_ref().map_or("", |last| &last.host));
        let user = input_with_value(window, cx, last.as_ref().map_or("", |last| &last.user));
        let port = input_with_value(window, cx, &last.as_ref().map_or(22, |last| last.port).to_string());
        let password = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("Only if the server has no key for you"));
        let appearance_subscription = cx.observe(&appearance, |_, _, cx| cx.notify());
        Self {
            explorer,
            previous_focus,
            appearance,
            host,
            user,
            port,
            password,
            error: None,
            interactive_only: false,
            auth: None,
            connecting: false,
            _appearance_subscription: appearance_subscription,
        }
    }

    fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connecting {
            return;
        }
        let host = self.host.read(cx).value().trim().to_string();
        let user = self.user.read(cx).value().trim().to_string();
        if host.is_empty() {
            self.error = Some("Host must not be empty".to_string());
            cx.notify();
            return;
        }
        let port = self.port.read(cx).value().trim().parse::<u16>().ok();
        let Some(port) = port.filter(|port| *port > 0) else {
            self.error = Some("Port must be between 1 and 65535".to_string());
            cx.notify();
            return;
        };
        let connection = SshConnection { host, user, port };
        let password = self.password.read(cx).value().to_string();
        if !password.is_empty() {
            if let Err(error) = crate::ssh::save_password(&connection, &password) {
                self.error = Some(format!("Password not saved: {error}"));
                cx.notify();
                return;
            }
        }
        self.error = None;
        self.auth = None;
        self.interactive_only = false;
        self.connecting = true;
        cx.notify();
        // The connection runs on its own thread; a sign-in link it meets is
        // handed over here and shown while the connection keeps waiting.
        let prompt = std::sync::Arc::new(std::sync::Mutex::new(None::<crate::ssh::AuthPrompt>));
        let done = std::sync::Arc::new(std::sync::Mutex::new(None::<std::io::Result<String>>));
        {
            let (prompt, done, attempt) = (prompt.clone(), done.clone(), connection.clone());
            std::thread::spawn(move || {
                let result = crate::ssh::connect_with_prompts(&attempt, |asked| {
                    *prompt.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(asked);
                });
                *done.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result);
            });
        }
        cx.spawn_in(window, async move |this, cx| {
            let result = loop {
                cx.background_executor().timer(std::time::Duration::from_millis(150)).await;
                let asked = prompt.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
                if let Some(asked) = asked {
                    if this.update(cx, |this, cx| {
                        this.auth = Some(asked);
                        cx.notify();
                    }).is_err() {
                        return;
                    }
                }
                if let Some(result) = done.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take() {
                    break result;
                }
            };
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(home) => {
                    let _ = this.explorer.update(cx, |explorer, cx| {
                        explorer.start_ssh_session(connection, &home, window, cx)
                    });
                    this.close(window, cx);
                }
                Err(error) => {
                    this.connecting = false;
                    this.auth = None;
                    this.interactive_only = error.kind() == std::io::ErrorKind::Unsupported;
                    this.error = Some(error.to_string());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Fills the form with `connection` and connects.
    pub(crate) fn start(&mut self, connection: &SshConnection, window: &mut Window, cx: &mut Context<Self>) {
        self.host.update(cx, |input, cx| input.set_value(connection.host.clone(), window, cx));
        self.user.update(cx, |input, cx| input.set_value(connection.user.clone(), window, cx));
        self.port.update(cx, |input, cx| input.set_value(connection.port.to_string(), window, cx));
        self.connect(window, cx);
    }

    /// `ssh host` in the user's terminal, for servers that are an app.
    fn open_in_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let host = self.host.read(cx).value().trim().to_string();
        let user = self.user.read(cx).value().trim().to_string();
        let port = self.port.read(cx).value().trim().parse::<u16>().unwrap_or(22);
        let endpoint = SshConnection { host, user, port };
        let app = self.appearance.read(cx).extensions.terminal_app.effective();
        let command = format!(
            "exec ssh -t -p {} {}",
            endpoint.port,
            files_sdk::log::shell_quote(&endpoint.destination())
        );
        match run_in_terminal(&command, "ssh", app) {
            Ok(()) => {
                files_sdk::log::record(crate::ssh::MANIFEST.id, files_sdk::log::Kind::Process, command, Some("opened in the terminal".to_string()));
                self.close(window, cx);
            }
            Err(error) => {
                self.error = Some(format!("Terminal failed: {error}"));
                cx.notify();
            }
        }
    }

    fn close(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        close_ssh_dialog(&self.explorer, &self.previous_focus, window, cx);
    }
}

#[cfg(test)]
impl SshDialog {
    pub(crate) fn auth_for_test(&self) -> Option<crate::ssh::AuthPrompt> {
        self.auth.clone()
    }
}

impl Focusable for SshDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.host.read(cx).focus_handle(cx)
    }
}

impl Render for SshDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&settings);
        let focus_handle = self.focus_handle(cx);

        v_flex()
            .track_focus(&focus_handle)
            .key_context("SshDialog")
            .on_action(cx.listener(|this, _: &Enter, window, cx| {
                this.connect(window, cx);
            }))
            .w_full()
            .gap_4()
            .px_2()
            .pb_2()
            .text_color(palette.text)
            .child(
                div()
                    .text_xs()
                    .text_color(palette.muted)
                    .child("Uses your ssh: ~/.ssh/config hosts, keys and the agent. User may stay empty."),
            )
            .child(input_row("Host", &self.host, palette))
            .child(input_row("User", &self.user, palette))
            .child(input_row("Port", &self.port, palette))
            .child(input_row("Password (saved in Passwords / Keychain)", &self.password, palette))
            .children(self.auth.clone().map(|auth| {
                let url = auth.url.clone();
                let copy = auth.url.clone();
                v_flex()
                    .gap_2()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(palette.accent)
                    .bg(palette.accent_soft)
                    .child(div().text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Waiting for you to sign in…"))
                    .child(div().text_xs().text_color(palette.muted).whitespace_normal().child(auth.message.clone()))
                    .child(div().text_xs().font_family("Menlo").text_color(palette.accent).truncate().child(auth.url.clone()))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("ssh-auth-open")
                                    .primary()
                                    .small()
                                    .icon(gpui_kit::assets::IconName::ExternalLink)
                                    .label("Open in Browser")
                                    .on_click(move |_, _, cx| cx.open_url(&url)),
                            )
                            .child(
                                Button::new("ssh-auth-copy")
                                    .ghost()
                                    .small()
                                    .icon(gpui_kit::assets::IconName::Copy)
                                    .label("Copy link")
                                    .on_click(move |_, _, cx| cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(copy.clone()))),
                            ),
                    )
                    .child(div().text_xs().text_color(palette.faint).child("The connection continues by itself once you approve it (up to 5 minutes)."))
                    .into_any_element()
            }))
            .children(self.error.as_ref().map(|error| {
                div()
                    .text_xs()
                    .text_color(palette.video)
                    .child(error.clone())
                    .into_any_element()
            }))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .when(self.interactive_only, |row| {
                        row.child(
                            Button::new("ssh-open-terminal")
                                .secondary()
                                .icon(gpui_kit::assets::IconName::SquareTerminal)
                                .label("Open in Terminal")
                                .on_click(cx.listener(|this, _, window, cx| this.open_in_terminal(window, cx))),
                        )
                    })
                    .child(
                        Button::new("cancel-ssh")
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close(window, cx);
                            })),
                    )
                    .child(
                        Button::new("connect-ssh")
                            .primary()
                            .label(if self.connecting { "Connecting…" } else { "Connect" })
                            .loading(self.connecting)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.connect(window, cx);
                            })),
                    ),
            )
    }
}

impl FileExplorer {
    pub fn open_ssh_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.appearance.read(cx).extensions.ssh
            || self.ssh_dialog.is_some()
            || window.has_active_dialog(cx)
        {
            return;
        }
        let previous_focus = window.focused(cx);
        let explorer = cx.entity().downgrade();
        let appearance = self.appearance.clone();
        let form = cx.new(|cx| {
            SshDialog::new(
                window,
                cx,
                explorer.clone(),
                previous_focus.clone(),
                appearance,
            )
        });
        self.ssh_dialog = Some(form.clone());
        let initial_focus = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                let focus = form.read(cx).focus_handle(cx);
                focus.focus(window, cx);
            }
            dialog
                .title("SSH connection")
                .centered()
                .w(px(460.))
                .close_button(false)
                .overlay(true)
                .overlay_closable(true)
                .child(form.clone())
                .on_close({
                    let explorer = explorer.clone();
                    let previous_focus = previous_focus.clone();
                    move |_, window, cx| {
                        close_ssh_dialog(&explorer, &previous_focus, window, cx);
                    }
                })
        });
    }

    pub fn disconnect_ssh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = self.ssh_connection.take() else {
            return;
        };
        crate::model::remote::reset(None);
        self.remote_loading.clear();
        for pane in [Pane::Left, Pane::Right] {
            let location = match pane {
                Pane::Left => self.left.location,
                Pane::Right => self.right.location,
            };
            if location.is_remote() {
                self.active_pane = pane;
                self.navigate(Location::Home, window, cx);
            }
        }
        if self.current.is_remote() {
            self.navigate(Location::Home, window, cx);
        }
        self.notice = Some(format!("Disconnected · {}", connection.address()));
        cx.notify();
    }

    /// Connects in the background to a server from the sidebar list.
    /// Reconnects to a server from the sidebar list: through the dialog, so
    /// a sign-in link (Tailscale SSH…) or an error has somewhere to show.
    pub fn connect_saved_ssh(&mut self, address: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = SshConnection::parse(address) else {
            return;
        };
        self.open_ssh_dialog(window, cx);
        if let Some(form) = self.ssh_dialog.clone() {
            form.update(cx, |form, cx| form.start(&connection, window, cx));
        }
    }

    pub(crate) fn start_ssh_session(
        &mut self,
        connection: SshConnection,
        home: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let address = connection.address();
        crate::model::remote::reset(Some(address.clone()));
        self.remote_loading.clear();
        self.ssh_home = Location::remote(home);
        self.ssh_connection = Some(connection);
        self.update_sidebar(cx, |sidebar| {
            sidebar.ssh.retain(|known| *known != address);
            sidebar.ssh.insert(0, address.clone());
            sidebar.ssh.truncate(8);
        });
        let home = self.ssh_home;
        self.navigate(home, window, cx);
        self.notice = Some(format!("Connected · {address}"));
        cx.notify();
    }

    pub(crate) fn forget_saved_ssh(&mut self, address: &str, cx: &mut Context<Self>) {
        // Forgetting a server also removes its saved password.
        if let Some(connection) = SshConnection::parse(address) {
            let _ = crate::ssh::forget_password(&connection);
        }
        self.update_sidebar(cx, |sidebar| sidebar.ssh.retain(|known| known != address));
    }

    /// Fetches a remote folder in the background (unless cached and `force`
    /// is off) and redraws when it arrives.
    pub(crate) fn load_remote(&mut self, location: Location, force: bool, cx: &mut Context<Self>) {
        let (Some(connection), Some(path)) = (self.ssh_connection.clone(), location.remote_path()) else {
            return;
        };
        if (!force && crate::model::remote::is_loaded(location)) || !self.remote_loading.insert(location) {
            return;
        }
        let loading = format!("Loading {path}…");
        self.notice = Some(loading.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { crate::ssh::EXTENSION.list(&connection, path) }).await;
            let _ = this.update(cx, |this, cx| {
                this.remote_loading.remove(&location);
                match result {
                    Ok(items) => {
                        let Location::Remote(id) = location else { return };
                        crate::model::remote::store(id, items);
                        if this.notice.as_deref() == Some(loading.as_str()) {
                            this.notice = None;
                        }
                    }
                    Err(error) => this.notice = Some(format!("SSH · {path}: {error}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Downloads a remote file into the local cache and opens it.
    pub(crate) fn open_remote_file(&mut self, file: &'static FileEntry, cx: &mut Context<Self>) {
        let (Some(connection), Some(remote)) = (self.ssh_connection.clone(), crate::model::remote::entry_path(file)) else {
            return;
        };
        let local = crate::ssh::cache_path(&connection, remote);
        let remote = remote.to_string();
        self.notice = Some(format!("Downloading {}…", file.name));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let target = local.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    crate::ssh::EXTENSION.download(&connection, &remote, &target)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        let overrides = this.appearance.read(cx).default_apps.clone();
                        this.notice = Some(match crate::default_apps::open(&local, &overrides) {
                            Ok(_) => format!("Opened {} · a local copy, edits stay on this Mac", file.name),
                            Err(error) => format!("Open failed · {}: {error}", file.name),
                        });
                    }
                    Err(error) => this.notice = Some(format!("Download failed · {}: {error}", file.name)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The dragged remote item, or the whole selection when it is part of one.
    pub(crate) fn remote_drag_entries(&self, entry: &'static FileEntry) -> Vec<&'static FileEntry> {
        let selection = self.selected_entries();
        if selection.iter().any(|selected| selected.id == entry.id) { selection } else { vec![entry] }
    }

    /// Remote files dropped onto a local folder are downloaded into it.
    pub(crate) fn download_remote_into_active(&mut self, files: Vec<&'static FileEntry>, cx: &mut Context<Self>) {
        match self.active_location().path() {
            Some(target) => self.download_remote(files, target, cx),
            None => {
                self.notice = Some(format!("Cannot download into {}", self.active_location().label()));
                cx.notify();
            }
        }
    }

    /// Copies the selected remote files into a local folder.
    pub(crate) fn download_remote(&mut self, files: Vec<&'static FileEntry>, target: PathBuf, cx: &mut Context<Self>) {
        let Some(connection) = self.ssh_connection.clone() else {
            return;
        };
        let jobs: Vec<(String, String)> = files
            .iter()
            .filter(|file| !file.is_folder())
            .filter_map(|file| Some((crate::model::remote::entry_path(file)?.to_string(), file.name.to_string())))
            .collect();
        let skipped = files.len() - jobs.len();
        if jobs.is_empty() {
            self.notice = Some("Only files can be downloaded".to_string());
            cx.notify();
            return;
        }
        self.notice = Some(format!("Downloading {} item(s)…", jobs.len()));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let folder = target.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut done = 0;
                    for (remote, name) in jobs {
                        let local = crate::archive::unique_path(&folder.join(&name));
                        let local = if folder.join(&name).exists() { local } else { folder.join(&name) };
                        crate::ssh::EXTENSION.download(&connection, &remote, &local)?;
                        done += 1;
                    }
                    std::io::Result::Ok(done)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let message = match result {
                    Ok(done) if skipped > 0 => format!("Downloaded {done} file(s) to {} · folders skipped", target.display()),
                    Ok(done) => format!("Downloaded {done} file(s) to {}", target.display()),
                    Err(error) => format!("Download failed: {error}"),
                };
                let level = if message.starts_with("Download failed") { files_sdk::log::Level::Error } else { files_sdk::log::Level::Success };
                files_sdk::log::notify(crate::ssh::MANIFEST.id, level, message.clone(), None);
                this.notice = Some(message);
                cx.notify();
            });
        })
        .detach();
    }

    /// Uploads local files into the remote folder `location`.
    pub(crate) fn upload_remote(&mut self, paths: Vec<PathBuf>, location: Location, cx: &mut Context<Self>) {
        let (Some(connection), Some(folder)) = (self.ssh_connection.clone(), location.remote_path()) else {
            return;
        };
        let files: Vec<PathBuf> = paths.into_iter().filter(|path| path.is_file()).collect();
        if files.is_empty() {
            self.notice = Some("Only files can be uploaded over SSH".to_string());
            cx.notify();
            return;
        }
        self.notice = Some(format!("Uploading {} file(s)…", files.len()));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    for file in &files {
                        crate::ssh::EXTENSION.upload(&connection, file, folder)?;
                    }
                    std::io::Result::Ok(files.len())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let message = match result {
                    Ok(count) => format!("Uploaded {count} file(s) to {folder}"),
                    Err(error) => format!("Upload failed: {error}"),
                };
                let level = if message.starts_with("Upload failed") { files_sdk::log::Level::Error } else { files_sdk::log::Level::Success };
                files_sdk::log::notify(crate::ssh::MANIFEST.id, level, message.clone(), None);
                this.notice = Some(message);
                this.load_remote(location, true, cx);
                cx.notify();
            });
        })
        .detach();
    }
}

/// Opens the terminal app on an `ssh` session already inside `folder`.
pub(crate) fn open_remote_terminal(
    connection: Option<&SshConnection>,
    folder: &str,
    app: crate::settings::TerminalApp,
) -> std::io::Result<()> {
    let Some(connection) = connection else {
        return Ok(());
    };
    let quoted = format!("'{}'", folder.replace('\'', r"'\''"));
    let remote = format!("cd {quoted} && exec \"$SHELL\" -l");
    let command = format!(
        "exec ssh -t -p {} {} {}",
        connection.port,
        shell_quote(&connection.destination()),
        shell_quote(&remote)
    );
    run_in_terminal(&command, "ssh", app)
}

/// Runs a shell command in a new window of the user's terminal app.
pub(crate) fn run_in_terminal(command: &str, name: &str, app: crate::settings::TerminalApp) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        // Terminal and iTerm run a `.command` file in a new window.
        let script = std::env::temp_dir().join(format!("file-force-{name}-{}.command", std::process::id()));
        std::fs::write(&script, format!("#!/bin/sh\n{command}\n"))?;
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
        std::process::Command::new("/usr/bin/open").arg("-a").arg(app.app_name()).arg(&script).spawn()?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, name);
        std::process::Command::new("x-terminal-emulator").args(["-e", "sh", "-c", command]).spawn()?;
        Ok(())
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn input_with_value(
    window: &mut Window,
    cx: &mut Context<SshDialog>,
    value: &str,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut input = InputState::new(window, cx);
        input.set_value(value.to_string(), window, cx);
        input
    })
}

fn input_row(label: &'static str, input: &Entity<InputState>, palette: Palette) -> AnyElement {
    v_flex()
        .gap_1()
        .child(div().text_xs().text_color(palette.muted).child(label))
        .child(Input::new(input).w_full())
        .into_any_element()
}

fn close_ssh_dialog(
    explorer: &WeakEntity<FileExplorer>,
    previous_focus: &Option<FocusHandle>,
    window: &mut Window,
    cx: &mut App,
) {
    let _ = explorer.update(cx, |explorer, cx| {
        explorer.ssh_dialog = None;
        cx.notify();
    });
    if let Some(focus) = previous_focus {
        focus.focus(window, cx);
    }
}
