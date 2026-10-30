//! Passwords and keys in the system keychain: the login keychain on macOS
//! (shown in the Passwords app), the Secret Service (`secret-tool`) on Linux.
//! Each extension only sees its own entries, stored under the service
//! `files.<extension id>`, and needs [`Permission::Secrets`].

use std::io;
#[cfg(not(target_os = "macos"))]
use std::io::Write as _;
use std::process::{Command, Stdio};

use crate::permissions::{Permission, check};

fn service(extension: &str) -> String {
    format!("files.{extension}")
}

/// The keychain service name an extension's secrets live under.
pub fn service_name(extension: &str) -> String {
    service(extension)
}

/// Saves (or replaces) the secret `account` of `extension`.
pub fn set(extension: &str, account: &str, secret: &str) -> io::Result<()> {
    check(extension, Permission::Secrets)?;
    #[cfg(target_os = "macos")]
    {
        // Commands go through stdin (`security -i`), so the secret never
        // shows up in the process list.
        let quote = |value: &str| format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""));
        let mut child = Command::new("/usr/bin/security")
            .arg("-i")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        {
            use std::io::Write as _;
            let mut stdin = child.stdin.take().expect("piped");
            writeln!(
                stdin,
                "add-generic-password -U -s {} -a {} -w {}",
                quote(&service(extension)),
                quote(account),
                quote(secret)
            )?;
        }
        return status(child.wait_with_output()?);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut child = Command::new("secret-tool")
            .args(["store", "--label", &format!("Files · {extension} · {account}"), "service", &service(extension), "account", account])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        child.stdin.take().expect("piped").write_all(secret.as_bytes())?;
        status(child.wait_with_output()?)
    }
}

/// The secret `account` of `extension`, or `None` when there is none.
pub fn get(extension: &str, account: &str) -> io::Result<Option<String>> {
    check(extension, Permission::Secrets)?;
    #[cfg(target_os = "macos")]
    let output = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", &service(extension), "-a", account, "-w"])
        .stdin(Stdio::null())
        .output()?;
    #[cfg(not(target_os = "macos"))]
    let output = Command::new("secret-tool")
        .args(["lookup", "service", &service(extension), "account", account])
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let secret = String::from_utf8_lossy(&output.stdout).trim_end_matches('\n').to_string();
    Ok((!secret.is_empty()).then_some(secret))
}

pub fn delete(extension: &str, account: &str) -> io::Result<()> {
    check(extension, Permission::Secrets)?;
    #[cfg(target_os = "macos")]
    let output = Command::new("/usr/bin/security")
        .args(["delete-generic-password", "-s", &service(extension), "-a", account])
        .stdin(Stdio::null())
        .output()?;
    #[cfg(not(target_os = "macos"))]
    let output = Command::new("secret-tool")
        .args(["clear", "service", &service(extension), "account", account])
        .stdin(Stdio::null())
        .output()?;
    // Deleting what is not there is fine.
    let _ = output;
    Ok(())
}

fn status(output: std::process::Output) -> io::Result<()> {
    if output.status.success() {
        return Ok(());
    }
    Err(io::Error::other(String::from_utf8_lossy(&output.stderr).trim().to_string()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn secrets_need_the_permission() {
        let error = super::get("no-such-extension", "me").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(super::service_name("ssh"), "files.ssh");
    }
}
