//! One Files at a time (Settings → General): a second start hands its folder
//! to the running one, which opens it in a new tab and comes to the front,
//! and quits. The running one listens on 127.0.0.1 with a random port and
//! token, written to a file only the user can read; nothing else can reach it.

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Set when another start asked to be shown (no folder given).
static ACTIVATE: AtomicBool = AtomicBool::new(false);

pub fn take_activate() -> bool {
    ACTIVATE.swap(false, Ordering::Relaxed)
}

fn lock_file() -> Option<PathBuf> {
    Some(crate::settings::settings_path()?.parent()?.join("instance"))
}

/// "port token".
fn parse(text: &str) -> Option<(u16, String)> {
    let mut parts = text.split_whitespace();
    let port = parts.next()?.parse().ok()?;
    let token = parts.next()?.to_string();
    (token.len() >= 16).then_some((port, token))
}

/// One request: the token, then the path (or an empty line); "ok" back.
fn request(port: u16, token: &str, path: Option<&Path>) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(500)) else { return false };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let path = path.map(|path| path.display().to_string()).unwrap_or_default();
    if writeln!(stream, "{token}\n{path}").is_err() {
        return false;
    }
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer).is_ok() && answer.trim() == "ok"
}

/// Hands `path` to a running Files; `true` when it took it (this start then
/// quits).
pub fn hand_over(path: Option<&Path>) -> bool {
    let Some(text) = lock_file().and_then(|file| std::fs::read_to_string(file).ok()) else { return false };
    let Some((port, token)) = parse(&text) else { return false };
    // Windows lets the running one come to the front only if we allow it.
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(windows_sys::Win32::UI::WindowsAndMessaging::ASFW_ANY);
    }
    request(port, &token, path)
}

fn token() -> String {
    let mut bytes = [0u8; 16];
    let _ = getrandom::fill(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Becomes the running Files: listens, and records where.
pub fn listen() {
    let Ok(listener) = TcpListener::bind(("127.0.0.1", 0)) else { return };
    let Ok(port) = listener.local_addr().map(|address| address.port()) else { return };
    let token = token();
    let Some(file) = lock_file() else { return };
    if std::fs::write(&file, format!("{port} {token}")).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
    }
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            serve(stream, &token);
        }
    });
}

fn serve(stream: TcpStream, token: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut reader = BufReader::new(&stream);
    let (mut given, mut path) = (String::new(), String::new());
    if reader.read_line(&mut given).is_err() || given.trim() != token || reader.read_line(&mut path).is_err() {
        return;
    }
    let path = path.trim();
    if path.is_empty() {
        ACTIVATE.store(true, Ordering::Relaxed);
    } else {
        crate::desktop::opened_from_system(vec![path.to_string()]);
    }
    let _ = (&stream).write_all(b"ok\n");
}

/// At quit: the next start becomes the running one.
pub fn release() {
    if let Some(file) = lock_file() {
        let _ = std::fs::remove_file(file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_start_hands_its_folder_over() {
        assert_eq!(parse("4242 0123456789abcdef0123"), Some((4242, "0123456789abcdef0123".to_string())));
        assert_eq!(parse("4242 short"), None, "a weak token is refused");
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let token = token();
        let served = token.clone();
        let server = std::thread::spawn(move || {
            for stream in listener.incoming().take(2).flatten() {
                serve(stream, &served);
            }
        });
        let dir = std::env::temp_dir();
        assert!(!request(port, "wrong-token-0000000", Some(&dir)), "a wrong token gets nothing");
        assert!(request(port, &token, Some(&dir)));
        server.join().unwrap();
        assert!(crate::desktop::take_opened().iter().any(|path| path == &dir.canonicalize().unwrap()));
    }
}
