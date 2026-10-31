use std::path::Path;
use std::process::Command;

use gpui_kit::{ClipboardItem, Context, Window};

use crate::app::FileExplorer;
use crate::model::{FileKind, Location, entry, folder_entry};

#[derive(Clone, Copy)]
pub enum DemoCommand {
    Open,
    QuickLook,
    OpenWithDefault,
    OpenWithChooser,
    Compress,
    Extract,
    CopyName,
    CopyPath,
    CopyReference,
    BrowseArchive,
    Delete,
    Properties,
}

#[derive(Clone, Copy)]
pub enum FolderCommand {
    OpenTerminal,
    Properties,
}

impl FileExplorer {
    pub fn run_demo_command(
        &mut self,
        command: DemoCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(file) = self.active_selection().and_then(entry) else {
            return;
        };
        match command {
            DemoCommand::Open => {
                if file.kind == FileKind::Archive && self.appearance.read(cx).extensions.archives {
                    crate::archive::open_archive(file, self.appearance.clone(), cx);
                } else if file.is_folder() {
                    if let Some(target) = file.target {
                        self.navigate(target, window, cx);
                    }
                } else {
                    // Same as Enter / double-click: the file's default app.
                    self.open_file(file, cx);
                }
            }
            DemoCommand::QuickLook => {
                self.toggle_quick_look(window, cx);
                return;
            }
            DemoCommand::OpenWithDefault => {
                self.open_file(file, cx);
            }
            DemoCommand::OpenWithChooser => match file.path {
                Some(path) => self.open_with_chooser(path, file.name, cx),
                None => {
                    self.notice =
                        Some(format!("Open With · Choose Application · {} (demo)", file.name));
                }
            },
            DemoCommand::Compress => {
                if file.path.is_some() {
                    crate::compress::open_compress(file, self.appearance.clone(), cx);
                } else {
                    self.notice = Some(format!("{} is not on the local disk", file.name));
                }
            }
            DemoCommand::Extract => {
                if crate::archive::is_supported(file) {
                    crate::archive::open_archive_page(
                        file,
                        crate::archive::ArchivePage::Extract,
                        self.appearance.clone(),
                        cx,
                    );
                } else {
                    self.notice = Some(format!("Not a supported archive (.zip, .tar, .tar.gz, .tgz, .gz) · {}", file.name));
                }
            }
            DemoCommand::CopyName => {
                cx.write_to_clipboard(ClipboardItem::new_string(file.name.to_string()));
                self.notice = Some("Name copied".to_string());
            }
            DemoCommand::CopyPath => {
                self.copy_demo_path(cx);
            }
            DemoCommand::CopyReference => {
                let reference = file.path.map_or_else(
                    || format!("{}/{}", self.active_location().label(), file.name),
                    |path| format!("file://{}", path.display()),
                );
                cx.write_to_clipboard(ClipboardItem::new_string(reference));
                self.notice = Some("File reference copied".to_string());
            }
            DemoCommand::BrowseArchive => {
                crate::archive::open_archive(file, self.appearance.clone(), cx);
                self.notice = None;
            }
            DemoCommand::Delete => {
                self.open_delete_dialog(window, cx);
                return;
            }
            DemoCommand::Properties => {
                crate::properties::open_properties(
                    file,
                    self.active_location(),
                    self.ssh_connection.clone(),
                    self.appearance.clone(),
                    cx,
                );
                self.notice = None;
            }
        }
        cx.notify();
    }

    pub fn run_folder_command(
        &mut self,
        command: FolderCommand,
        location: Location,
        cx: &mut Context<Self>,
    ) {
        match command {
            FolderCommand::OpenTerminal => self.open_terminal(location, cx),
            FolderCommand::Properties => {
                if let Some(folder) = folder_entry(location) {
                    crate::properties::open_properties(
                        folder,
                        location,
                        self.ssh_connection.clone(),
                        self.appearance.clone(),
                        cx,
                    );
                    self.notice = None;
                    cx.notify();
                }
            }
        }
    }

    /// Lets the user pick an application, then opens `path` with it.
    fn open_with_chooser(&mut self, path: &'static Path, name: &'static str, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            self.notice = Some(format!("Choose an application for {name}…"));
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { choose_and_open(path) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.notice = open_with_notice(name, result);
                    cx.notify();
                });
            })
            .detach();
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Open".into()),
            });
            cx.spawn(async move |this, cx| {
                let application = match picked.await {
                    Ok(Ok(Some(mut paths))) if !paths.is_empty() => paths.remove(0),
                    Ok(Err(error)) => {
                        let _ = this.update(cx, |this, cx| {
                            this.notice = Some(format!("Open failed · {name}: {error}"));
                            cx.notify();
                        });
                        return;
                    }
                    _ => return,
                };
                let result = files_sdk::log::command(&application)
                    .arg(path)
                    .spawn()
                    .map(|_| Some(application.display().to_string()));
                let _ = this.update(cx, |this, cx| {
                    this.notice = open_with_notice(name, result);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn open_terminal(&mut self, location: Location, cx: &mut Context<Self>) {
        if !self.appearance.read(cx).extensions.terminal {
            self.notice = Some("Terminal extension is disabled".to_string());
            cx.notify();
            return;
        }
        if let Some(folder) = location.remote_path() {
            let app = self.appearance.read(cx).extensions.terminal_app;
            self.notice = Some(match (&self.ssh_connection, crate::views::ssh::open_remote_terminal(self.ssh_connection.as_ref(), folder, app)) {
                (Some(connection), Ok(())) => format!("Terminal · ssh {} · {folder}", connection.address()),
                (_, Err(error)) => format!("Terminal failed: {error}"),
                (None, Ok(())) => "Not connected".to_string(),
            });
            cx.notify();
            return;
        }

        let Some(current_dir) = location.path() else {
            self.notice = Some(format!("{} has no folder on disk", location.label()));
            cx.notify();
            return;
        };
        let app = self.appearance.read(cx).extensions.terminal_app.effective();
        match spawn_terminal(&current_dir, app) {
            Ok(mut child) => {
                self.notice = Some(format!("Terminal launched · {}", current_dir.display()));
                cx.notify();
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { child.wait() })
                        .await;
                    let error = match result {
                        Ok(status) if !status.success() => {
                            Some(format!("Terminal exited with status {status}"))
                        }
                        Err(error) => Some(format!("Terminal failed: {error}")),
                        _ => None,
                    };
                    if let Some(error) = error {
                        let _ = this.update(cx, |this, cx| {
                            this.notice = Some(error);
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
            Err(error) => {
                self.notice = Some(format!("Terminal failed to start: {error}"));
                cx.notify();
            }
        }
    }
}

/// `Ok(None)` means the user cancelled the chooser.
fn open_with_notice(name: &str, result: std::io::Result<Option<String>>) -> Option<String> {
    match result {
        Ok(Some(application)) => Some(format!("Opened · {name} · {application}")),
        Ok(None) => None,
        Err(error) => Some(format!("Open failed · {name}: {error}")),
    }
}

/// Shows the native "Choose Application" dialog and opens `path` with the pick.
#[cfg(target_os = "macos")]
fn choose_and_open(path: &Path) -> std::io::Result<Option<String>> {
    let output = Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "POSIX path of (path to (choose application with prompt \"Open with:\"))",
        ])
        .output()?;
    if !output.status.success() {
        // Exit code 1 with error -128 is "User canceled".
        let stderr = String::from_utf8_lossy(&output.stderr);
        return if stderr.contains("-128") {
            Ok(None)
        } else {
            Err(std::io::Error::other(stderr.trim().to_string()))
        };
    }
    let application = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let status = Command::new("/usr/bin/open")
        .arg("-a")
        .arg(&application)
        .arg(path)
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other(format!("open exited with {status}")));
    }
    let label = Path::new(application.trim_end_matches('/'))
        .file_stem()
        .map_or(application.clone(), |stem| stem.to_string_lossy().into_owned());
    Ok(Some(label))
}

/// Shows the Windows "Open with" dialog, which opens the file itself.
#[cfg(target_os = "windows")]
fn choose_and_open(path: &Path) -> std::io::Result<Option<String>> {
    files_sdk::log::command("rundll32.exe")
        .arg("shell32.dll,OpenAs_RunDLL")
        .arg(path)
        .spawn()?;
    Ok(Some("chosen application".to_string()))
}


#[cfg(target_os = "macos")]
fn spawn_terminal(
    current_dir: &Path,
    app: crate::settings::TerminalApp,
) -> std::io::Result<std::process::Child> {
    Command::new("/usr/bin/open")
        .arg("-a")
        .arg(app.app_name())
        .arg(current_dir)
        .current_dir(current_dir)
        .spawn()
}

#[cfg(target_os = "windows")]
fn spawn_terminal(current_dir: &Path, app: crate::settings::TerminalApp) -> std::io::Result<std::process::Child> {
    use crate::settings::TerminalApp;
    let mut command = if app == TerminalApp::WindowsTerminal {
        let mut command = Command::new("wt");
        command.arg("-d").arg(current_dir);
        command
    } else {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", "/D"]).arg(current_dir);
        if app == TerminalApp::PowerShell {
            command.args(["powershell", "-NoExit"]);
        } else {
            command.arg("cmd");
        }
        command
    };
    files_sdk::log::hide_window(&mut command).current_dir(current_dir).spawn()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn spawn_terminal(current_dir: &Path, app: crate::settings::TerminalApp) -> std::io::Result<std::process::Child> {
    let program = if app == crate::settings::TerminalApp::Ghostty { "ghostty" } else { "x-terminal-emulator" };
    Command::new(program).current_dir(current_dir).spawn()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", unix)))]
fn spawn_terminal(_: &Path, _: crate::settings::TerminalApp) -> std::io::Result<std::process::Child> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Terminal is not supported on this platform",
    ))
}
