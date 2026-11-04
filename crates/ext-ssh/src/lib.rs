//! SSH through the system `ssh` client, so `~/.ssh/config`, keys, the agent,
//! ProxyJump and known_hosts all work exactly as in a terminal. One master
//! connection per server is shared by every command (`ControlMaster`).

use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub use files_sdk::{Endpoint as SshConnection, RemoteItem};
use files_sdk::{
    Accessibility, Context, Icon, Manifest, MenuItem, Outcome, Permission, RemoteProvider, Scope, Targets, secrets,
};

static MENU: [MenuItem; 1] = [MenuItem {
    command: "ssh.copy-address",
    title: "Copy SSH Address",
    icon: Some(Icon::Builtin("copy")),
    targets: Targets::ALL,
    scope: Scope::Remote,
    accessibility: Accessibility {
        label: Some("Copy SSH address"),
        hint: Some("Copies ssh://server/path to the clipboard"),
    },
}];

pub static MANIFEST: Manifest = Manifest {
    id: "ssh",
    name: "SSH",
    version: "1.0.0",
    source: "Built-in",
    summary: "Browse servers over SSH with your own ssh setup: ~/.ssh/config, keys, the agent, ProxyJump.",
    icon: Icon::Builtin("cloud"),
    adds: &[
        "Sidebar → Connected → SSH and recent servers",
        "Open, download, drop or paste to upload",
        "Open in Terminal starts ssh in that folder",
    ],
    permissions: &[Permission::Network, Permission::Secrets, Permission::WriteFiles, Permission::LaunchApps, Permission::Clipboard, Permission::PreventSleep],
    menu: &MENU,
    sidebar: &[],
    menu_color: None,
    tier: files_sdk::Tier::Free,
    commands: &[],
};

/// The SSH extension.
pub struct Ssh;

pub static EXTENSION: Ssh = Ssh;

impl files_sdk::Extension for Ssh {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn remote(&self) -> Option<&dyn RemoteProvider> {
        Some(self)
    }

    fn run(&self, command: &str, context: &Context) -> io::Result<Outcome> {
        match command {
            "ssh.copy-address" => {
                let server = context.server.as_deref().ok_or_else(|| io::Error::other("not connected"))?;
                let paths = if context.selection.is_empty() {
                    context.folder.iter().cloned().collect()
                } else {
                    context.selection.clone()
                };
                let text = paths.iter().map(|path| format!("ssh://{server}{path}")).collect::<Vec<_>>().join("\n");
                Ok(Outcome { notice: Some(format!("Copied {text}")), clipboard: Some(text), navigate: None })
            }
            _ => Err(io::Error::new(io::ErrorKind::Unsupported, format!("unknown command {command}"))),
        }
    }
}

/// Saves the server's password in the keychain (Passwords on macOS); `ssh`
/// reads it back through `SSH_ASKPASS` and it is never written to a file.
pub fn save_password(endpoint: &SshConnection, password: &str) -> io::Result<()> {
    secrets::set(MANIFEST.id, &endpoint.address(), password)
}

pub fn forget_password(endpoint: &SshConnection) -> io::Result<()> {
    secrets::delete(MANIFEST.id, &endpoint.address())
}

fn has_password(endpoint: &SshConnection) -> bool {
    secrets::get(MANIFEST.id, &endpoint.address()).ok().flatten().is_some()
}

/// A tiny helper `ssh` runs to ask for the password: it reads the keychain.
fn askpass_script(endpoint: &SshConnection) -> io::Result<PathBuf> {
    let service = secrets::service_name(MANIFEST.id);
    let account = endpoint.address();
    let quote = |value: &str| format!("'{}'", value.replace('\'', r"'\''"));
    #[cfg(target_os = "macos")]
    let body = format!("exec /usr/bin/security find-generic-password -s {} -a {} -w", quote(&service), quote(&account));
    #[cfg(not(target_os = "macos"))]
    let body = format!("exec secret-tool lookup service {} account {}", quote(&service), quote(&account));
    let path = std::env::temp_dir().join(format!("file-force-askpass-{}", std::process::id()));
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}

impl RemoteProvider for Ssh {
    fn connect(&self, endpoint: &SshConnection) -> io::Result<String> {
        files_sdk::permissions::check(MANIFEST.id, Permission::Network)?;
        let reply = String::from_utf8_lossy(&run(endpoint, "pwd")?).trim().to_string();
        home_from_pwd(&endpoint.host, &reply)
    }

    fn list(&self, endpoint: &SshConnection, path: &str) -> io::Result<Vec<RemoteItem>> {
        // `-L` shows what symlinks point to, so linked folders open as folders.
        let output = files_sdk::log::run(MANIFEST.id, &mut command(endpoint, &format!("cd -- {} && LC_ALL=C ls -lanL", quote(path))))?;
        // Broken links make `ls` exit non-zero but the rest is still listed.
        if !output.status.success() && output.stdout.is_empty() {
            return Err(ssh_error(&output.stderr));
        }
        Ok(parse_listing(&String::from_utf8_lossy(&output.stdout)))
    }

    fn download(&self, endpoint: &SshConnection, remote: &str, local: &Path) -> io::Result<()> {
        let mut download = command(endpoint, &format!("cat -- {}", quote(remote)));
        files_sdk::log::spawned(MANIFEST.id, &download);
        let mut child = download.spawn()?;
        let mut stdout = child.stdout.take().expect("piped");
        let mut file = std::fs::File::create(local)?;
        io::copy(&mut stdout, &mut file)?;
        let mut stderr = Vec::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_end(&mut stderr);
        }
        if child.wait()?.success() {
            return Ok(());
        }
        let _ = std::fs::remove_file(local);
        Err(ssh_error(&stderr))
    }

    fn upload(&self, endpoint: &SshConnection, local: &Path, folder: &str) -> io::Result<String> {
        let name = local
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?
            .to_string_lossy()
            .into_owned();
        let taken: Vec<String> = self.list(endpoint, folder)?.into_iter().map(|item| item.name).collect();
        let name = free_name(&name, &taken);
        let target = join(folder, &name);
        let mut command = command(endpoint, &format!("cat > {}", quote(&target)));
        command.stdin(Stdio::piped());
        files_sdk::log::spawned(MANIFEST.id, &command);
        let mut child = command.spawn()?;
        {
            let mut stdin = child.stdin.take().expect("piped");
            let mut file = std::fs::File::open(local)?;
            io::copy(&mut file, &mut stdin)?;
            stdin.flush()?;
        }
        let output = child.wait_with_output()?;
        if output.status.success() {
            return Ok(target);
        }
        Err(ssh_error(&output.stderr))
    }
}

/// Where the shared connection's socket lives. Unix sockets allow ~104
/// bytes, and macOS's temp folder (`/var/folders/…/T/`) plus the
/// 40-character `%C` hash is longer than that ("Control path too long"), so
/// it is a short folder in `/tmp`, readable only by this user.
fn control_dir() -> PathBuf {
    #[cfg(unix)]
    let dir = {
        use std::os::unix::fs::MetadataExt as _;
        // The owner of the home folder is the user running the app.
        let uid = std::env::var_os("HOME").and_then(|home| std::fs::metadata(home).ok()).map_or(0, |metadata| metadata.uid());
        PathBuf::from(format!("/tmp/fe-ssh-{uid}"))
    };
    #[cfg(not(unix))]
    let dir = std::env::temp_dir().join("fe-ssh");
    if std::fs::create_dir_all(&dir).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    dir
}

fn command(endpoint: &SshConnection, remote: &str) -> Command {
    let mut command = files_sdk::log::command(program());
    let control = control_dir().join("%C");
    command.args(["-p", &endpoint.port.to_string()]);
    // A saved password is answered by the keychain helper; otherwise never
    // hang on a prompt: there is no terminal.
    match has_password(endpoint).then(|| askpass_script(endpoint)).transpose() {
        Ok(Some(askpass)) => {
            command
                .env("SSH_ASKPASS", askpass)
                .env("SSH_ASKPASS_REQUIRE", "force")
                .env("DISPLAY", std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_string()))
                .args(["-o", "NumberOfPasswordPrompts=1"]);
        }
        _ => {
            command.args(["-o", "BatchMode=yes"]);
        }
    }
    command
        .args(["-o", "ConnectTimeout=10"])
        .args(["-o", "ControlMaster=auto"])
        .arg("-o")
        .arg(format!("ControlPath={}", control.display()))
        // An hour: servers that ask for a browser sign-in (Tailscale SSH
        // "check" mode) do it once per connection, not per command.
        .args(["-o", "ControlPersist=3600"])
        .arg(endpoint.destination())
        .arg("--")
        .arg(remote)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn run(endpoint: &SshConnection, remote: &str) -> io::Result<Vec<u8>> {
    let output = files_sdk::log::run(MANIFEST.id, &mut command(endpoint, remote))?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    Err(ssh_error(&output.stderr))
}


/// Tests swap `ssh` for a script (connections run on their own threads, so
/// this is process-wide, not per thread).
static PROGRAM: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

fn program() -> PathBuf {
    PROGRAM.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone().unwrap_or_else(|| PathBuf::from("ssh"))
}

/// For tests (here and in the app): `ssh` becomes a script that runs the
/// remote command with a local shell. The host `auth-host` first asks for a
/// browser sign-in, like Tailscale SSH in check mode.
#[cfg(unix)]
#[doc(hidden)]
pub fn use_fake_ssh(dir: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::create_dir_all(dir).unwrap();
    let fake = dir.join("ssh");
    std::fs::write(
        &fake,
        "#!/bin/sh\n\
         for arg in \"$@\"; do case \"$arg\" in *auth-host*) \
           echo '# Tailscale SSH requires an additional check.' >&2; \
           echo '# To authenticate, visit: https://login.tailscale.com/a/test123' >&2; \
           sleep 1;; esac; done\n\
         while [ \"$1\" != \"--\" ]; do shift; done\nshift\nexec sh -c \"$1\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    *PROGRAM.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(fake);
}

/// A server asks the user to approve the connection somewhere else, e.g.
/// Tailscale SSH: "To authenticate, visit: https://login.tailscale.com/a/…".
#[derive(Clone, Debug, PartialEq)]
pub struct AuthPrompt {
    /// What the server said, without the `#` comment marks.
    pub message: String,
    pub url: String,
}

/// How long a sign-in in the browser may take before giving up.
pub const AUTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// The sign-in link in a line the server printed, if any.
pub fn auth_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let url: String = line[start..].chars().take_while(|char| !char.is_whitespace() && !"\"'<>)]".contains(*char)).collect();
    let url = url.trim_end_matches(['.', ',', ';']).to_string();
    (url.len() > "https://".len()).then_some(url)
}

/// Connects like [`RemoteProvider::connect`], but reads what the server says
/// while it waits: when it asks for a sign-in in the browser, `prompt` gets
/// the link at once and the connection keeps waiting (up to
/// [`AUTH_TIMEOUT`]) until the user approves it.
pub fn connect_with_prompts(endpoint: &SshConnection, mut prompt: impl FnMut(AuthPrompt)) -> io::Result<String> {
    use std::io::BufRead as _;
    files_sdk::permissions::check(MANIFEST.id, Permission::Network)?;
    let mut ssh = command(endpoint, "pwd");
    // The server's banner and prompts arrive on stderr; a login can take a
    // while, so the connect timeout only covers reaching the server.
    files_sdk::log::spawned(MANIFEST.id, &ssh);
    let started = std::time::Instant::now();
    let mut child = ssh.spawn()?;
    let stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::BufReader::new(stdout).read_to_string(&mut text);
        text
    });
    let (lines, receiver) = std::sync::mpsc::channel::<String>();
    let stderr = child.stderr.take().expect("piped");
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let mut said = Vec::new();
    let mut prompted = false;
    let status = loop {
        while let Ok(line) = receiver.try_recv() {
            let clean = line.trim_start_matches('#').trim().to_string();
            if !clean.is_empty() {
                said.push(clean.clone());
            }
            if !prompted {
                if let Some(url) = auth_url(&clean) {
                    prompted = true;
                    files_sdk::log::record(MANIFEST.id, files_sdk::log::Kind::Network, format!("{} asks to sign in", endpoint.address()), Some(said.join("\n")));
                    prompt(AuthPrompt { message: said.join("\n"), url });
                }
            }
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > AUTH_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::TimedOut, "The sign-in was not completed in time. Try connecting again."));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    // Lines that came in right before ssh exited.
    while let Ok(line) = receiver.recv_timeout(std::time::Duration::from_millis(100)) {
        let clean = line.trim_start_matches('#').trim().to_string();
        if !clean.is_empty() {
            said.push(clean);
        }
    }
    let reply = reader.join().unwrap_or_default();
    if !status.success() {
        let stderr = said.join("\n");
        return Err(ssh_error(stderr.as_bytes()));
    }
    home_from_pwd(&endpoint.host, reply.trim())
}

/// The remote home from `pwd`'s reply. Some servers are an interactive app
/// rather than a shell (`ssh terminal.shop` answers "Requires an active
/// PTY"): their files cannot be browsed, but they can be opened in a terminal.
pub fn home_from_pwd(host: &str, reply: &str) -> io::Result<String> {
    if reply.is_empty() {
        return Ok("/".to_string());
    }
    if reply.starts_with('/') && !reply.contains('\n') {
        return Ok(reply.to_string());
    }
    let first = reply.lines().next().unwrap_or(reply);
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{host} runs an interactive app, not a shell (it said: \"{first}\"). Open it in Terminal instead."),
    ))
}

/// Where opened remote files are cached locally.
pub fn cache_path(connection: &SshConnection, remote: &str) -> PathBuf {
    let host: String = connection
        .address()
        .chars()
        .map(|char| if char.is_alphanumeric() || char == '.' || char == '@' { char } else { '_' })
        .collect();
    // The extension's cache folder (Settings → Storage lists it).
    files_sdk::storage::cache_dir(MANIFEST.id)
        .unwrap_or_else(|_| std::env::temp_dir().join("file-force-remote"))
        .join(host)
        .join(remote.trim_start_matches('/'))
}

pub fn join(folder: &str, name: &str) -> String {
    if folder.ends_with('/') { format!("{folder}{name}") } else { format!("{folder}/{name}") }
}

/// The folder above `path`, or `None` at `/`.
pub fn parent(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.rfind('/') {
        Some(0) => Some("/".to_string()),
        Some(index) => Some(trimmed[..index].to_string()),
        None => None,
    }
}

pub fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() { "/" } else { trimmed.rsplit('/').next().unwrap_or(trimmed) }
}

/// Single-quotes `value` for the remote POSIX shell.
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn free_name(name: &str, taken: &[String]) -> String {
    if !taken.iter().any(|known| known == name) {
        return name.to_string();
    }
    let (stem, extension) = match name.rfind('.') {
        Some(index) if index > 0 => (&name[..index], &name[index..]),
        _ => (name, ""),
    };
    (1..)
        .map(|index| format!("{stem} ({index}){extension}"))
        .find(|candidate| !taken.contains(candidate))
        .expect("unbounded range always yields a free name")
}

fn ssh_error(stderr: &[u8]) -> io::Error {
    let message = String::from_utf8_lossy(stderr);
    let message = message.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("ssh failed").trim();
    let hint = if message.contains("Permission denied") {
        " · save the password in the connection dialog, or add your key with ssh-add / ssh-copy-id"
    } else if message.contains("Host key verification failed") {
        " · connect once with ssh in a terminal to trust this server"
    } else {
        ""
    };
    io::Error::other(format!("{message}{hint}"))
}

/// Parses `ls -lan` output. Devices, `.` and `..` are skipped.
pub fn parse_listing(output: &str) -> Vec<RemoteItem> {
    output.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<RemoteItem> {
    let mut rest = line.trim_start();
    let mut fields = Vec::with_capacity(8);
    for _ in 0..8 {
        let end = rest.find(char::is_whitespace)?;
        fields.push(&rest[..end]);
        rest = rest[end..].trim_start();
    }
    let name = rest;
    let perms = fields[0].as_bytes();
    if perms.len() < 10 || name.is_empty() || name == "." || name == ".." {
        return None;
    }
    let is_dir = match perms[0] {
        b'd' => true,
        b'-' => false,
        _ => return None,
    };
    let mode = (0..9).fold(0, |mode, index| if perms[index + 1] == b'-' { mode } else { mode | (0o400 >> index) });
    Some(RemoteItem {
        name: name.to_string(),
        is_dir,
        bytes: fields[4].parse().ok()?,
        mode,
        modified: format!("{} {} {}", fields[5], fields[6], fields[7]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Calls the provider with one endpoint, to keep the test readable.
    struct Bound<'a>(&'a SshConnection);

    impl Bound<'_> {
        fn list(&self, folder: &str) -> io::Result<Vec<RemoteItem>> {
            Ssh.list(self.0, folder)
        }
        fn download(&self, remote: &str, local: &Path) -> io::Result<()> {
            Ssh.download(self.0, remote, local)
        }
        fn upload(&self, local: &Path, folder: &str) -> io::Result<String> {
            Ssh.upload(self.0, local, folder)
        }
    }

    #[test]
    fn parses_gnu_and_bsd_listings() {
        let output = "total 24\n\
            drwxr-xr-x  5 1000 1000 4096 Oct 24 16:08 .\n\
            drwxr-xr-x  3 0    0    4096 Jan  1  2025 ..\n\
            -rw-r--r--  1 1000 1000  812 Oct 24 16:08 notes with spaces.md\n\
            drwx------  2 1000 1000 4096 Mar  3  2024 .ssh\n\
            -rwxr-xr-x@ 1 501  20   9001 Oct  2 09:15 run.sh\n\
            crw-rw-rw-  1 0    0    1,   3 Oct 24 16:08 null\n";
        let items = parse_listing(output);
        assert_eq!(items.len(), 3, "{items:?}");
        assert_eq!(items[0].name, "notes with spaces.md");
        assert_eq!((items[0].bytes, items[0].mode, items[0].is_dir), (812, 0o644, false));
        assert_eq!(items[0].modified, "Oct 24 16:08");
        assert_eq!((items[1].name.as_str(), items[1].is_dir, items[1].mode), (".ssh", true, 0o700));
        assert_eq!((items[2].name.as_str(), items[2].mode), ("run.sh", 0o755));
    }

    #[test]
    fn remote_paths() {
        assert_eq!(parent("/home/me/src"), Some("/home/me".to_string()));
        assert_eq!(parent("/home"), Some("/".to_string()));
        assert_eq!(parent("/"), None);
        assert_eq!(file_name("/home/me/"), "me");
        assert_eq!(file_name("/"), "/");
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("/home", "a"), "/home/a");
        assert_eq!(quote("it's"), r"'it'\''s'");
        let taken = vec!["a.txt".to_string(), "a (1).txt".to_string()];
        assert_eq!(free_name("a.txt", &taken), "a (2).txt");
        assert_eq!(free_name("b.txt", &taken), "b.txt");
    }

    #[test]
    #[cfg(unix)]
    fn lists_downloads_and_uploads_through_ssh() {
        let dir = std::env::temp_dir().join(format!("file-force-fake-ssh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let server = dir.join("server");
        std::fs::create_dir_all(server.join("src")).unwrap();
        std::fs::write(server.join("it's here.txt"), "remote body").unwrap();
        use_fake_ssh(&dir);

        let connection = SshConnection { host: "test".into(), user: String::new(), port: 22 };
        let connection = Bound(&connection);
        let folder = server.to_string_lossy().into_owned();
        let items = connection.list(&folder).unwrap();
        let names: Vec<_> = items.iter().map(|item| (item.name.as_str(), item.is_dir)).collect();
        assert!(names.contains(&("it's here.txt", false)) && names.contains(&("src", true)), "{names:?}");

        let local = dir.join("downloaded.txt");
        connection.download(&join(&folder, "it's here.txt"), &local).unwrap();
        assert_eq!(std::fs::read_to_string(&local).unwrap(), "remote body");
        assert!(connection.download(&join(&folder, "missing"), &dir.join("x")).is_err());

        let upload = dir.join("it's here.txt");
        std::fs::write(&upload, "local body").unwrap();
        let target = connection.upload(&upload, &folder).unwrap();
        assert!(target.ends_with("it's here (1).txt"), "never overwrites: {target}");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "local body");
        assert_eq!(std::fs::read_to_string(server.join("it's here.txt")).unwrap(), "remote body");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_address_builds_ssh_urls() {
        let context = Context {
            selection: vec!["/srv/a.txt".into(), "/srv/b".into()],
            remote: true,
            server: Some("me@box:2222".into()),
            ..Default::default()
        };
        let outcome = files_sdk::Extension::run(&Ssh, "ssh.copy-address", &context).unwrap();
        assert_eq!(outcome.clipboard.as_deref(), Some("ssh://me@box:2222/srv/a.txt\nssh://me@box:2222/srv/b"));
        let empty = Context { folder: Some("/srv".into()), server: Some("box".into()), ..Default::default() };
        let outcome = files_sdk::Extension::run(&Ssh, "ssh.copy-address", &empty).unwrap();
        assert_eq!(outcome.clipboard.as_deref(), Some("ssh://box/srv"));
    }

    #[test]
    fn the_control_socket_path_fits_the_unix_limit() {
        // %C expands to a 40-character hash; sockets allow 104 bytes.
        let path = control_dir().join("0".repeat(40));
        assert!(path.as_os_str().len() < 100, "{} is too long", path.display());
        let endpoint = SshConnection { host: "terminal.shop".into(), user: String::new(), port: 22 };
        let line = files_sdk::log::shell_line(&command(&endpoint, "pwd"));
        assert!(line.contains("ControlPath=/tmp/fe-ssh-"), "{line}");
    }

    #[test]
    fn browser_sign_in_links_are_found() {
        assert_eq!(
            auth_url("# To authenticate, visit: https://login.tailscale.com/a/l5ec5023386c26").as_deref(),
            Some("https://login.tailscale.com/a/l5ec5023386c26")
        );
        assert_eq!(auth_url("Open (https://sso.example.com/x?y=1).").as_deref(), Some("https://sso.example.com/x?y=1"));
        assert_eq!(auth_url("Permission denied (publickey)."), None);
    }

    #[test]
    #[cfg(unix)]
    fn a_sign_in_prompt_reaches_the_ui_while_the_connection_waits() {
        let dir = std::env::temp_dir().join(format!("files-ext-ssh-auth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Like Tailscale SSH in check mode: a banner with a link on stderr,
        // a pause while the user signs in, then the command's output.
        let fake = dir.join("ssh");
        std::fs::write(
            &fake,
            "#!/bin/sh\necho '# Tailscale SSH requires an additional check.' >&2\necho '# To authenticate, visit: https://login.tailscale.com/a/abc123' >&2\nsleep 1\necho /home/me\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = fake;
        use_fake_ssh(&dir);
        files_sdk::permissions::declare(&MANIFEST);

        let endpoint = SshConnection { host: "auth-host".into(), user: "me".into(), port: 22 };
        let mut prompts = Vec::new();
        let started = std::time::Instant::now();
        let home = connect_with_prompts(&endpoint, |prompt| prompts.push((prompt, started.elapsed()))).unwrap();
        assert!(home.starts_with('/'), "{home}");
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].0.url, "https://login.tailscale.com/a/test123");
        assert!(prompts[0].0.message.contains("requires an additional check"));
        assert!(prompts[0].1 < std::time::Duration::from_millis(900), "the link must arrive before the server finishes waiting");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn servers_without_a_shell_are_reported() {
        assert_eq!(home_from_pwd("box", "/home/me").unwrap(), "/home/me");
        assert_eq!(home_from_pwd("box", "").unwrap(), "/");
        let error = home_from_pwd("terminal.shop", "Requires an active PTY").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("interactive app") && error.to_string().contains("Requires an active PTY"));
    }

    #[test]
    fn destination_omits_user_from_ssh_config() {
        let connection = SshConnection { host: "prod".into(), user: String::new(), port: 22 };
        assert_eq!(connection.address(), "prod");
        let connection = SshConnection { host: "h".into(), user: "me".into(), port: 2222 };
        assert_eq!(connection.address(), "me@h:2222");
        assert_eq!(SshConnection::parse("me@h:2222"), Some(connection));
        assert_eq!(SshConnection::parse("prod").map(|it| it.port), Some(22));
        assert_eq!(SshConnection::parse("me@h:0"), None);
    }
}
