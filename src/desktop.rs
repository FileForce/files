//! How the system knows Files: the launch argument (a folder or file to
//! open, `file://` too), the Windows AppUserModelID that toasts and the taskbar
//! group by, and on Linux the desktop entry (menu, notifications' name and
//! icon) and being the default file manager (`inode/directory`).

use std::path::PathBuf;

/// Bundle.toml's identifier: the MSI shortcut, the desktop entry and the
/// macOS bundle all use it.
pub const APP_ID: &str = "dev.fileforce.files";

/// The folder or file Files was started with (`files ~/Music`,
/// `file:///home/me/Music` from a desktop entry's `%U`).
pub fn launch_path(args: impl IntoIterator<Item = String>) -> Option<PathBuf> {
    let arg = args.into_iter().skip(1).find(|arg| !arg.starts_with('-'))?;
    let path = match arg.strip_prefix("file://") {
        Some(rest) => PathBuf::from(url::Url::parse(&format!("file://{rest}")).ok()?.to_file_path().ok()?),
        None => PathBuf::from(arg),
    };
    path.exists().then(|| path.canonicalize().unwrap_or(path))
}

/// Windows groups the taskbar button and shows toasts under this id; the
/// MSI's Start menu shortcut carries the same one.
pub fn set_app_id() {
    #[cfg(windows)]
    {
        let wide: Vec<u16> = APP_ID.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: a NUL-terminated UTF-16 string that outlives the call.
        unsafe {
            windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(wide.as_ptr());
        }
    }
}

// ------------------------------------------------------------------ Linux

/// The program a desktop entry starts: the AppImage itself when Files runs
/// from one (the binary inside is in a temporary mount).
#[cfg(all(unix, not(target_os = "macos")))]
fn program() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE").map(PathBuf::from).or_else(|| std::env::current_exe().ok())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn applications_dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("applications"))
}

/// The desktop entry's text for `program`.
pub fn desktop_entry(program: &std::path::Path) -> String {
    let quoted = program.display().to_string().replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "[Desktop Entry]\nType=Application\nName=file FORCE\nGenericName=File Manager\nComment=Files, Git, SSH, mail and music in one window\nExec=\"{quoted}\" %U\nIcon={APP_ID}\nTerminal=false\nCategories=Utility;FileManager;\nMimeType=inode/directory;\nStartupWMClass={APP_ID}\nX-GNOME-UsesNotifications=true\n"
    )
}

/// Writes the desktop entry and icon into the user's folders (once per
/// program path), so the menu, notifications and "Open with" know Files.
pub fn install_desktop_entry() {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let (Some(program), Some(dir)) = (program(), applications_dir()) else { return };
        let entry = dir.join(format!("{APP_ID}.desktop"));
        let text = desktop_entry(&program);
        if std::fs::read_to_string(&entry).ok().as_deref() == Some(text.as_str()) {
            return;
        }
        if std::fs::create_dir_all(&dir).is_err() || std::fs::write(&entry, text).is_err() {
            return;
        }
        if let Some(icons) = dirs::data_dir().map(|data| data.join("icons/hicolor/scalable/apps")) {
            let _ = std::fs::create_dir_all(&icons);
            let _ = std::fs::write(icons.join(format!("{APP_ID}.svg")), include_bytes!("../packaging/icon/icon.svg"));
        }
        let _ = files_sdk::log::command("update-desktop-database").arg(&dir).status();
        files_sdk::log::record("desktop", files_sdk::log::Kind::Setting, format!("Desktop entry written: {}", entry.display()), None);
    }
}

/// Whether the system opens folders with Files.
pub fn is_default_file_manager() -> bool {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return files_sdk::log::command("xdg-mime")
            .args(["query", "default", "inode/directory"])
            .output()
            .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).trim() == format!("{APP_ID}.desktop"));
    }
    #[allow(unreachable_code)]
    false
}

/// Makes Files open folders everywhere (the desktop, "Show in folder" of
/// browsers and other apps). Linux only.
pub fn make_default_file_manager() -> std::io::Result<()> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        install_desktop_entry();
        let status = files_sdk::log::command("xdg-mime").args(["default", &format!("{APP_ID}.desktop"), "inode/directory"]).status()?;
        if !status.success() {
            return Err(std::io::Error::other("xdg-mime could not set the default file manager"));
        }
        files_sdk::log::record("desktop", files_sdk::log::Kind::Setting, "Files is now the default file manager".to_string(), None);
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Only on Linux"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launch_argument_becomes_a_path() {
        let dir = std::env::temp_dir();
        let plain = launch_path(["files".to_string(), dir.display().to_string()]).unwrap();
        assert_eq!(plain, dir.canonicalize().unwrap());
        let url = url::Url::from_file_path(dir.canonicalize().unwrap()).unwrap().to_string();
        assert_eq!(launch_path(["files".to_string(), "--flag".to_string(), url]).unwrap(), dir.canonicalize().unwrap());
        assert_eq!(launch_path(["files".to_string()]), None);
        assert_eq!(launch_path(["files".to_string(), "/no/such/folder".to_string()]), None);
    }

    #[test]
    fn the_desktop_entry_opens_folders() {
        let entry = desktop_entry(std::path::Path::new("/opt/Files \"x\".AppImage"));
        assert!(entry.contains("MimeType=inode/directory;"));
        assert!(entry.contains("Exec=\"/opt/Files \\\"x\\\".AppImage\" %U"));
        assert!(entry.contains(&format!("Icon={APP_ID}")));
    }
}
