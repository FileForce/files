//! How the system knows Files: the launch argument (a folder or file to
//! open, `file://` too), the Windows AppUserModelID that toasts and the taskbar
//! group by, and on Linux the desktop entry (menu, notifications' name and
//! icon) and being the default file manager: `inode/directory` on Linux,
//! `public.folder` plus `NSFileViewer` on macOS (Finder keeps the desktop,
//! the Dock and Spotlight).

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

/// Folders the system asked Files to open while it runs (macOS: "Show in
/// Finder" of other apps, `open ~/Music`), waiting for the window.
static OPENED: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// The system's open-URL event: `file://` addresses become paths.
pub fn opened_from_system(urls: Vec<String>) {
    let paths = urls.into_iter().filter_map(|url| launch_path(["files".to_string(), url]));
    OPENED.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).extend(paths);
}

pub fn take_opened() -> Vec<PathBuf> {
    std::mem::take(&mut *OPENED.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
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
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
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

#[cfg(target_os = "macos")]
mod mac {
    use core_foundation::base::TCFType as _;
    use core_foundation::string::{CFString, CFStringRef};

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSSetDefaultRoleHandlerForContentType(content_type: CFStringRef, role: u32, handler: CFStringRef) -> i32;
        fn LSCopyDefaultRoleHandlerForContentType(content_type: CFStringRef, role: u32) -> CFStringRef;
    }

    /// kLSRolesViewer | kLSRolesEditor | …: every role.
    const ROLES_ALL: u32 = u32::MAX;

    pub fn set_folder_handler(bundle: &str) -> std::io::Result<()> {
        let (kind, handler) = (CFString::new("public.folder"), CFString::new(bundle));
        // SAFETY: both strings live for the call; LaunchServices copies them.
        let status = unsafe { LSSetDefaultRoleHandlerForContentType(kind.as_concrete_TypeRef(), ROLES_ALL, handler.as_concrete_TypeRef()) };
        if status == 0 { Ok(()) } else { Err(std::io::Error::other(format!("LaunchServices refused ({status}): Files must run from its installed app"))) }
    }

    pub fn folder_handler() -> Option<String> {
        let kind = CFString::new("public.folder");
        // SAFETY: a "copy" function: we own the returned string (or null).
        let handler = unsafe { LSCopyDefaultRoleHandlerForContentType(kind.as_concrete_TypeRef(), ROLES_ALL) };
        (!handler.is_null()).then(|| unsafe { CFString::wrap_under_create_rule(handler) }.to_string())
    }
}

/// Whether the system opens folders with Files.
pub fn is_default_file_manager() -> bool {
    #[cfg(windows)]
    {
        return files_sdk::log::command("reg")
            .args(["query", WINDOWS_CLASSES[0], "/ve"])
            .output()
            .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).contains(WINDOWS_VERB));
    }
    #[cfg(target_os = "macos")]
    {
        return mac::folder_handler().is_some_and(|handler| handler.eq_ignore_ascii_case(APP_ID));
    }
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

/// Makes Files open folders: on Linux everywhere (the desktop, "Show in
/// folder" of browsers and other apps); on macOS what other apps reveal and
/// `open` (the desktop and the Dock stay Finder's).
pub fn make_default_file_manager() -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe()?;
        let command = windows_command(&exe);
        for classes in WINDOWS_CLASSES {
            let verb = format!(r"{classes}\{WINDOWS_VERB}");
            reg(&["add", &verb, "/ve", "/d", "Open in file FORCE", "/f"])?;
            reg(&["add", &format!(r"{verb}\command"), "/ve", "/d", &command, "/f"])?;
            // The default verb: what a double-click runs.
            reg(&["add", classes, "/ve", "/d", WINDOWS_VERB, "/f"])?;
        }
        files_sdk::log::record("desktop", files_sdk::log::Kind::Setting, "Files now opens folders instead of Explorer".to_string(), None);
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        mac::set_folder_handler(APP_ID)?;
        // Cocoa apps' "Show in Finder" (NSWorkspace) asks this one.
        let _ = files_sdk::log::command("defaults").args(["write", "-g", "NSFileViewer", "-string", APP_ID]).status();
        // The terminal's `open <folder>` too (new terminal windows).
        set_open_script(true)?;
        files_sdk::log::record("desktop", files_sdk::log::Kind::Setting, "Files now opens folders instead of Finder".to_string(), None);
        return Ok(());
    }
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

// ------------------------------------------------------------ Windows

/// The verb Explorer runs on a double-clicked folder or drive, under the
/// user's own classes (no administrator needed; Restore Explorer and the
/// uninstaller take it out).
#[cfg_attr(not(windows), allow(dead_code))]
const WINDOWS_VERB: &str = "openinfiles";
#[cfg_attr(not(windows), allow(dead_code))]
const WINDOWS_CLASSES: [&str; 2] = [r"HKCU\Software\Classes\Directory\shell", r"HKCU\Software\Classes\Drive\shell"];

/// The command a folder verb runs: `"C:\…\file-force.exe" --from-shell "%V"`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn windows_command(exe: &std::path::Path) -> String {
    format!("\"{}\" {FROM_SHELL} \"%V\"", exe.display())
}

/// Marks a start from Explorer's folder verb. A browser's "Show in folder"
/// goes through it with the folder alone (Windows never passes the file to
/// another file manager), so the file that just arrived there is selected.
pub const FROM_SHELL: &str = "--from-shell";

/// The newest file in `dir` changed in the last two minutes: the download a
/// "Show in folder" meant.
pub fn just_arrived(dir: &std::path::Path) -> Option<PathBuf> {
    let now = std::time::SystemTime::now();
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|item| {
            let metadata = item.metadata().ok()?;
            let changed = metadata.created().ok().max(metadata.modified().ok())?;
            (metadata.is_file() && now.duration_since(changed).is_ok_and(|age| age.as_secs() < 120)).then(|| (changed, item.path()))
        })
        .max_by_key(|(changed, _)| *changed)
        .map(|(_, path)| path)
}

/// What to open for the start's arguments: from Explorer's folder verb, the
/// file that just arrived in the folder (selected), else the path itself.
pub fn launch_target(args: Vec<String>) -> Option<PathBuf> {
    let from_shell = args.iter().any(|arg| arg == FROM_SHELL);
    let path = launch_path(args)?;
    if from_shell && path.is_dir() {
        return Some(just_arrived(&path).unwrap_or(path));
    }
    Some(path)
}

/// Windows: once Files is the folder manager, the verb is written again at
/// each start (the exe may have moved with an update; older versions wrote
/// it without `--from-shell`).
pub fn refresh_folder_verb() {
    #[cfg(windows)]
    if is_default_file_manager() {
        let _ = make_default_file_manager();
    }
}

/// Folders without a path ("This PC", Control Panel: `::{GUID}`,
/// `shell:` names) are Explorer's: Files hands them back.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn is_shell_folder(arg: &str) -> bool {
    arg.starts_with("::") || arg.to_ascii_lowercase().starts_with("shell:")
}

/// Windows: started for a shell folder, Explorer opens it and Files quits.
pub fn hand_shell_folders_to_explorer() {
    #[cfg(windows)]
    if let Some(arg) = std::env::args().nth(1)
        && is_shell_folder(&arg)
    {
        let _ = files_sdk::log::command("explorer.exe").arg(arg).spawn();
        std::process::exit(0);
    }
}

#[cfg(windows)]
fn reg(args: &[&str]) -> std::io::Result<()> {
    let status = files_sdk::log::command("reg").args(args).status()?;
    if status.success() { Ok(()) } else { Err(std::io::Error::other(format!("reg {} failed", args.first().copied().unwrap_or_default()))) }
}

// ------------------------------------------------------- macOS `open`

/// `open <folder>` in a terminal goes to Finder whatever LaunchServices says,
/// so a small `open` placed before /usr/bin in PATH sends folders to Files.
/// It checks that Files is still installed every time: once the app is
/// deleted, folders open in Finder again, with nothing to undo.
pub const OPEN_SCRIPT: &str = r#"#!/bin/sh
# Written by file FORCE (Settings → Default apps). Folders open in Files;
# everything else, and every folder once Files is deleted, goes to
# /usr/bin/open as usual. Remove with "Restore Finder" in Settings.
APP_ID="dev.fileforce.files"
folders=1
[ "$#" -eq 0 ] && folders=0
for arg in "$@"; do
  case "$arg" in
    -*) folders=0 ;;
    # Packages are folders that open as apps or documents.
    *.app|*.app/|*.bundle|*.bundle/|*.framework|*.framework/|*.pkg|*.pkg/|*.xcodeproj|*.xcodeproj/|*.xcworkspace|*.xcworkspace/|*.playground|*.playground/|*.photoslibrary|*.photoslibrary/|*.rtfd|*.rtfd/|*.key|*.pages|*.numbers|*.plugin|*.kext) folders=0 ;;
    *) [ -d "$arg" ] || folders=0 ;;
  esac
done
if [ "$folders" -eq 1 ] && /usr/bin/open -b "$APP_ID" "$@" 2>/dev/null; then
  exit 0
fi
exec /usr/bin/open "$@"
"#;

/// Lines added to shell profiles, between these markers (so they can be
/// taken out again exactly).
const PROFILE_BEGIN: &str = "# >>> file FORCE open >>>";
const PROFILE_END: &str = "# <<< file FORCE open <<<";

/// Where the `open` script lives.
pub fn open_script_dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("file-force").join("bin"))
}

/// The profile text with the PATH block added (once) or removed.
pub fn with_profile_block(profile: &str, dir: Option<&std::path::Path>) -> String {
    let mut kept = String::new();
    let mut inside = false;
    for line in profile.lines() {
        match line.trim() {
            PROFILE_BEGIN => inside = true,
            PROFILE_END => inside = false,
            _ if !inside => {
                kept.push_str(line);
                kept.push('\n');
            }
            _ => {}
        }
    }
    if let Some(dir) = dir {
        if !kept.is_empty() && !kept.ends_with("\n\n") {
            kept.push('\n');
        }
        kept.push_str(&format!("{PROFILE_BEGIN}\n# `open <folder>` opens it in file FORCE (Settings → Default apps).\nexport PATH=\"{}:$PATH\"\n{PROFILE_END}\n", dir.display()));
    }
    kept
}

/// zsh (the default) and bash profiles in the home folder.
fn profiles() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else { return Vec::new() };
    let mut list = vec![home.join(".zshrc")];
    let bash = home.join(".bash_profile");
    if bash.exists() {
        list.push(bash);
    }
    list
}

/// Writes the `open` script and puts its folder first in PATH (new
/// terminals; `install`: true), or takes both out again.
pub fn set_open_script(install: bool) -> std::io::Result<()> {
    let dir = open_script_dir().ok_or_else(|| std::io::Error::other("No data folder"))?;
    if install {
        std::fs::create_dir_all(&dir)?;
        let script = dir.join("open");
        std::fs::write(&script, OPEN_SCRIPT)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
        }
    } else {
        let _ = std::fs::remove_file(dir.join("open"));
    }
    sync_profiles(&dir)
}

/// The PATH lines stay while either command (`open`, `files`) is there.
fn sync_profiles(dir: &std::path::Path) -> std::io::Result<()> {
    let keep = dir.join("open").is_file() || dir.join("files").is_file();
    for profile in profiles() {
        let before = std::fs::read_to_string(&profile).unwrap_or_default();
        let after = with_profile_block(&before, keep.then_some(dir));
        if after != before {
            std::fs::write(&profile, after)?;
        }
    }
    Ok(())
}

// --------------------------------------------------- the `files` command

/// macOS and Linux: `files [path…]` opens them in Files; no path, the
/// current folder (like `code .`).
#[cfg_attr(windows, allow(dead_code))]
pub fn files_script(program: &str) -> String {
    format!(
        "#!/bin/sh\n# file FORCE on the command line: files [folder or file…]; no argument opens the current folder.\n# Written by Settings → General → Command line; Uninstall there removes it.\n[ \"$#\" -eq 0 ] && set -- \"$PWD\"\n{program}\n"
    )
}

/// Windows: `files [path…]` as a .cmd next to nothing else on PATH.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn files_cmd(exe: &std::path::Path) -> String {
    format!(
        "@echo off\r\nrem file FORCE on the command line: files [folder or file]; no argument opens the current folder.\r\nif \"%~1\"==\"\" (start \"\" \"{exe}\" \"%CD%\") else (start \"\" \"{exe}\" %*)\r\n",
        exe = exe.display()
    )
}

/// Where `files` goes.
fn cli_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return Some(dirs::data_local_dir()?.join("file-force").join("bin"));
    }
    if cfg!(target_os = "macos") {
        return open_script_dir();
    }
    // Linux: ~/.local/bin, on PATH on most desktops already.
    Some(dirs::home_dir()?.join(".local").join("bin"))
}

fn cli_file() -> Option<PathBuf> {
    Some(cli_dir()?.join(if cfg!(windows) { "files.cmd" } else { "files" }))
}

pub fn cli_installed() -> bool {
    cli_file().is_some_and(|file| file.is_file())
}

/// Installs `files`; says what to do next.
pub fn install_cli() -> std::io::Result<String> {
    let dir = cli_dir().ok_or_else(|| std::io::Error::other("No folder for the command"))?;
    std::fs::create_dir_all(&dir)?;
    let file = cli_file().ok_or_else(|| std::io::Error::other("No folder for the command"))?;
    #[cfg(windows)]
    {
        std::fs::write(&file, files_cmd(&std::env::current_exe()?))?;
        windows_user_path(&dir, true)?;
        return Ok("Installed: open a new terminal and type files .".to_string());
    }
    #[cfg(not(windows))]
    {
        // macOS: through Launch Services (the installed app, single instance);
        // Linux: the AppImage (or binary) itself.
        let program = if cfg!(target_os = "macos") {
            format!("exec /usr/bin/open -b {APP_ID} \"$@\"")
        } else {
            let exe = std::env::var_os("APPIMAGE").map(PathBuf::from).or_else(|| std::env::current_exe().ok()).unwrap_or_default();
            format!("exec \"{}\" \"$@\"", exe.display().to_string().replace('"', "\\\""))
        };
        std::fs::write(&file, files_script(&program))?;
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))?;
        if cfg!(target_os = "macos") {
            sync_profiles(&dir)?;
        }
        Ok("Installed: open a new terminal and type files .".to_string())
    }
}

pub fn uninstall_cli() -> std::io::Result<()> {
    let Some(dir) = cli_dir() else { return Ok(()) };
    if let Some(file) = cli_file() {
        let _ = std::fs::remove_file(file);
    }
    #[cfg(windows)]
    windows_user_path(&dir, false)?;
    if cfg!(target_os = "macos") {
        sync_profiles(&dir)?;
    }
    Ok(())
}

/// Windows: adds or removes `dir` in the user's PATH (no administrator);
/// new terminals see it.
#[cfg(windows)]
fn windows_user_path(dir: &std::path::Path, add: bool) -> std::io::Result<()> {
    let dir = dir.display().to_string().replace('\'', "''");
    let script = if add {
        format!("$p=[Environment]::GetEnvironmentVariable('Path','User'); if (($p -split ';') -notcontains '{dir}') {{ [Environment]::SetEnvironmentVariable('Path', (($p.TrimEnd(';')) + ';{dir}').TrimStart(';'), 'User') }}")
    } else {
        format!("$p=[Environment]::GetEnvironmentVariable('Path','User'); [Environment]::SetEnvironmentVariable('Path', (($p -split ';') | Where-Object {{ $_ -and $_ -ne '{dir}' }}) -join ';', 'User')")
    };
    let status = files_sdk::log::command("powershell").args(["-NoProfile", "-Command", &script]).status()?;
    if status.success() { Ok(()) } else { Err(std::io::Error::other("Could not change PATH")) }
}

/// Windows: Explorer opens folders again.
pub fn restore_explorer() -> std::io::Result<()> {
    #[cfg(windows)]
    {
        for classes in WINDOWS_CLASSES {
            let _ = reg(&["delete", classes, "/ve", "/f"]);
            let _ = reg(&["delete", &format!(r"{classes}\{WINDOWS_VERB}"), "/f"]);
        }
        files_sdk::log::record("desktop", files_sdk::log::Kind::Setting, "Explorer opens folders again".to_string(), None);
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Only on Windows"))
}

/// Gives folders back to the system's own manager (Finder, Explorer).
pub fn restore_system_file_manager() -> std::io::Result<()> {
    if cfg!(windows) { restore_explorer() } else { restore_finder() }
}

/// macOS: Finder opens folders again.
pub fn restore_finder() -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let _ = files_sdk::log::command("defaults").args(["delete", "-g", "NSFileViewer"]).status();
        set_open_script(false)?;
        mac::set_folder_handler("com.apple.finder")?;
        files_sdk::log::record("desktop", files_sdk::log::Kind::Setting, "Finder opens folders again".to_string(), None);
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Only on macOS"))
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
    fn the_files_command_opens_its_arguments_or_the_current_folder() {
        let script = files_script("exec /usr/bin/open -b dev.fileforce.files \"$@\"");
        assert!(script.starts_with("#!/bin/sh") && script.contains("set -- \"$PWD\"") && script.contains("open -b dev.fileforce.files"));
        let path = std::env::temp_dir().join(format!("file-force-cli-{}.sh", std::process::id()));
        std::fs::write(&path, &script).unwrap();
        assert!(std::process::Command::new("sh").arg("-n").arg(&path).status().unwrap().success(), "valid sh");
        let _ = std::fs::remove_file(&path);
        let cmd = files_cmd(std::path::Path::new(r"C:\Program Files\Files\file-force.exe"));
        assert!(cmd.contains(r#"start "" "C:\Program Files\Files\file-force.exe" "%CD%""#) && cmd.contains("%*"));
    }

    #[test]
    fn the_profile_block_goes_in_once_and_out_exactly() {
        let dir = std::path::Path::new("/Users/me/Library/Application Support/file-force/bin");
        let profile = "export EDITOR=vim\n";
        let added = with_profile_block(profile, Some(dir));
        assert!(added.starts_with("export EDITOR=vim\n") && added.contains("export PATH=\"/Users/me/Library/Application Support/file-force/bin:$PATH\""));
        assert_eq!(with_profile_block(&added, Some(dir)), added, "added once");
        assert_eq!(with_profile_block(&added, None).trim_end(), "export EDITOR=vim", "removed exactly");
    }

    #[cfg(unix)]
    #[test]
    fn the_open_script_passes_everything_but_folders_on() {
        // A stand-in /usr/bin/open is not possible, so check the script with sh -n
        // and that packages and flags never count as folders.
        let script = std::env::temp_dir().join(format!("file-force-open-script-{}.sh", std::process::id()));
        std::fs::write(&script, OPEN_SCRIPT).unwrap();
        assert!(std::process::Command::new("sh").arg("-n").arg(&script).status().unwrap().success(), "valid sh");
        assert!(OPEN_SCRIPT.contains("*.app|") && OPEN_SCRIPT.contains("-*) folders=0") && OPEN_SCRIPT.contains("exec /usr/bin/open \"$@\""));
        let _ = std::fs::remove_file(&script);
    }

    #[test]
    fn show_in_folder_selects_the_file_that_just_arrived() {
        let dir = std::env::temp_dir().join(format!("file-force-arrived-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("report.pdf"), b"pdf").unwrap();
        let dir = dir.canonicalize().unwrap();
        let folder = dir.display().to_string();
        let args = |extra: &[&str]| std::iter::once("files").chain(extra.iter().copied()).map(String::from).collect::<Vec<_>>();
        assert_eq!(launch_target(args(&[FROM_SHELL, &folder])).and_then(|path| path.file_name().map(|name| name.to_owned())), Some("report.pdf".into()), "the download is selected");
        assert_eq!(launch_target(args(&[&folder])), Some(dir.clone()), "a plain start opens the folder");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn windows_folders_open_by_path_shell_folders_stay_explorer_s() {
        assert_eq!(windows_command(std::path::Path::new(r"C:\Program Files\Files\file-force.exe")), r#""C:\Program Files\Files\file-force.exe" --from-shell "%V""#);
        assert!(is_shell_folder("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}") && is_shell_folder("shell:Downloads"));
        assert!(!is_shell_folder(r"C:\Users\me"));
    }

    #[test]
    fn the_desktop_entry_opens_folders() {
        let entry = desktop_entry(std::path::Path::new("/opt/Files \"x\".AppImage"));
        assert!(entry.contains("MimeType=inode/directory;"));
        assert!(entry.contains("Exec=\"/opt/Files \\\"x\\\".AppImage\" %U"));
        assert!(entry.contains(&format!("Icon={APP_ID}")));
    }
}
