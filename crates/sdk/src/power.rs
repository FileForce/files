//! Keeping the computer awake while a long task runs (a big copy, an
//! upload). Needs [`Permission::PreventSleep`]. Sleep is allowed again when
//! the returned guard is dropped.

use std::io;
use std::process::Child;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::process::{Command, Stdio};

use crate::permissions::{Permission, check};

/// While alive, idle sleep is blocked.
pub struct Awake {
    child: Option<Child>,
}

impl Drop for Awake {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Blocks idle sleep for `extension` until the guard is dropped (or the app
/// quits: `caffeinate -w` follows this process).
pub fn keep_awake(extension: &str) -> io::Result<Awake> {
    check(extension, Permission::PreventSleep)?;
    #[cfg(target_os = "macos")]
    let child = Command::new("/usr/bin/caffeinate")
        .args(["-i", "-w", &std::process::id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();
    #[cfg(target_os = "linux")]
    let child = Command::new("systemd-inhibit")
        .args(["--what=idle:sleep", "--who=Files", "--why=Extension task", "sleep", "infinity"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let child: Option<Child> = None;
    Ok(Awake { child })
}
