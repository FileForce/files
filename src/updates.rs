//! "Check for Updates…": the newest version comes from `latest.json` on the
//! website (`[updates] url` in Bundle.toml), or else from the GitHub releases
//! of `repository` in Cargo.toml. A downloaded update is opened only when its
//! Ed25519 signature is the publisher's (the same key as licenses), so a
//! swapped file on the server or in transit is refused.

use std::path::PathBuf;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// From Bundle.toml (see build.rs); empty when not set.
pub const UPDATE_URL: &str = env!("FILES_UPDATE_URL");
pub const WEBSITE: &str = env!("FILES_WEBSITE");
pub const COPYRIGHT: &str = env!("FILES_COPYRIGHT");

/// The release notes of a version, on GitHub.
pub fn release_notes_url(version: &str) -> String {
    let repository = github_repository().unwrap_or_else(|| "FileForce/files".to_string());
    format!("https://github.com/{repository}/releases/tag/v{}", version.trim_start_matches('v'))
}

/// After an update (a newer version than last time): what to announce.
/// `None` on the first run ever and when nothing changed.
/// `ran_before`: settings existed — versions before 0.1.2 did not record
/// their number, so an empty one then means "updated from an older one".
pub fn just_updated(last_version: &str, ran_before: bool) -> Option<String> {
    let updated = if last_version.is_empty() { ran_before } else { is_newer(CURRENT_VERSION, last_version) };
    updated.then(|| CURRENT_VERSION.to_string())
}

/// Checked automatically at most this often.
pub const AUTO_CHECK_EVERY: i64 = 24 * 3600;

/// `owner/repo` from `package.repository` when it is a GitHub URL.
pub fn github_repository() -> Option<String> {
    parse_github(env!("CARGO_PKG_REPOSITORY"))
}

fn parse_github(url: &str) -> Option<String> {
    let rest = url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .split_once("github.com")?
        .1
        .trim_start_matches(['/', ':']);
    let mut parts = rest.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    (!owner.is_empty() && !repo.is_empty()).then(|| format!("{owner}/{repo}"))
}

/// A file to download for this platform.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Download {
    pub url: String,
    /// The publisher's signature (see `files_sdk::package`); updates without
    /// one are shown as a link only, never opened automatically.
    #[serde(default)]
    pub signature: Option<String>,
}

/// `latest.json` on the website.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LatestJson {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub published: String,
    #[serde(default)]
    pub macos: Option<Download>,
    #[serde(default)]
    pub linux: Option<Download>,
    #[serde(default)]
    pub windows: Option<Download>,
    /// A page about the release.
    #[serde(default)]
    pub page: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateStatus {
    /// Neither a website nor a GitHub repository is configured.
    NotConfigured,
    UpToDate { latest: String },
    Available { latest: String, notes: String, page: Option<String>, download: Option<Download> },
    /// The repository has no published release yet.
    NoReleases,
}

/// Numeric `major.minor.patch` comparison; a leading `v` is ignored.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |version: &str| -> Vec<u64> {
        version
            .trim()
            .trim_start_matches(['v', 'V'])
            .split(['.', '-', '+'])
            .take(3)
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    };
    parse(candidate) > parse(current)
}

fn curl(url: &str, extra: &[&str]) -> Result<(String, String), String> {
    let mut command = files_sdk::log::command("curl");
    command
        .args(["--silent", "--show-error", "--location", "--max-time", "15"])
        .args(["--header", &format!("User-Agent: files/{CURRENT_VERSION}")])
        .args(extra)
        .args(["--write-out", "\n%{http_code}"])
        .arg(url);
    let output = files_sdk::log::run_with_timeout("files", &mut command, std::time::Duration::from_secs(20))
        .map_err(|error| format!("Could not check: {error}"))?;
    if !output.status.success() {
        return Err(format!("Network error: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let (body, code) = text.rsplit_once('\n').unwrap_or((&text, ""));
    Ok((body.to_string(), code.trim().to_string()))
}

/// Asks the website (or GitHub) for the newest version. Blocking: call off
/// the UI thread.
pub fn check() -> Result<UpdateStatus, String> {
    if !UPDATE_URL.is_empty() {
        let (body, code) = curl(UPDATE_URL, &[])?;
        if code != "200" {
            return Err(format!("{UPDATE_URL} answered HTTP {code}"));
        }
        return parse_latest_json(&body);
    }
    let Some(repository) = github_repository() else {
        return Ok(UpdateStatus::NotConfigured);
    };
    let url = format!("https://api.github.com/repos/{repository}/releases/latest");
    let (body, code) = curl(&url, &["--header", "Accept: application/vnd.github+json"])?;
    match code.as_str() {
        "404" => Ok(UpdateStatus::NoReleases),
        "200" => parse_release(&body),
        other => Err(format!("GitHub answered HTTP {other}")),
    }
}

pub fn parse_latest_json(body: &str) -> Result<UpdateStatus, String> {
    let latest: LatestJson = serde_json::from_str(body).map_err(|error| format!("Unexpected latest.json: {error}"))?;
    let download = if cfg!(target_os = "macos") {
        latest.macos
    } else if cfg!(windows) {
        latest.windows
    } else {
        latest.linux
    };
    Ok(if is_newer(&latest.version, CURRENT_VERSION) {
        UpdateStatus::Available { latest: latest.version, notes: latest.notes, page: latest.page, download }
    } else {
        UpdateStatus::UpToDate { latest: latest.version }
    })
}

fn parse_release(body: &str) -> Result<UpdateStatus, String> {
    let release: serde_json::Value =
        serde_json::from_str(body).map_err(|error| format!("Unexpected answer from GitHub: {error}"))?;
    let latest = release["tag_name"].as_str().ok_or("The release has no tag")?.to_string();
    let url = release["html_url"].as_str().unwrap_or_default().to_string();
    Ok(if is_newer(&latest, CURRENT_VERSION) {
        UpdateStatus::Available {
            latest,
            notes: release["body"].as_str().unwrap_or_default().to_string(),
            page: (!url.is_empty()).then_some(url),
            download: None,
        }
    } else {
        UpdateStatus::UpToDate { latest }
    })
}

/// Downloads an update into Downloads and checks the publisher's signature;
/// a file that does not match is deleted. Blocking.
pub fn download(download: &Download, keys: &[&str]) -> Result<PathBuf, String> {
    let signature = download.signature.as_deref().ok_or("This update is not signed: open its page instead")?;
    let name = download.url.rsplit('/').next().filter(|name| !name.is_empty()).unwrap_or("Files-update.dmg");
    let folder = dirs::download_dir().ok_or("No Downloads folder")?;
    let target = files_sdk::fs::unique_path(&folder.join(name));
    let target = if folder.join(name).exists() { target } else { folder.join(name) };
    let mut command = files_sdk::log::command("curl");
    command.args(["--fail", "--silent", "--show-error", "--location", "--output"]).arg(&target).arg(&download.url);
    let output = files_sdk::log::run_with_timeout("files", &mut command, std::time::Duration::from_secs(900)).map_err(|error| error.to_string())?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&target);
        return Err(format!("Download failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    let bytes = std::fs::read(&target).map_err(|error| error.to_string())?;
    if !files_sdk::package::verify(&bytes, signature, keys) {
        let _ = std::fs::remove_file(&target);
        return Err("The downloaded file is not signed by the Files publisher: it was deleted".to_string());
    }
    Ok(target)
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_secs() as i64)
}

/// Whether the daily automatic check is due.
pub fn auto_check_due(settings: &crate::settings::UpdateSettings) -> bool {
    settings.automatic && now() - settings.last_check >= AUTO_CHECK_EVERY && (!UPDATE_URL.is_empty() || github_repository().is_some())
}

/// The update the last "is available" notification offers to install.
pub fn offered() -> std::sync::MutexGuard<'static, Option<(String, Download)>> {
    static OFFERED: std::sync::Mutex<Option<(String, Download)>> = std::sync::Mutex::new(None);
    OFFERED.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Installs a verified update and starts the new version once this one
/// has quit (the caller quits right after `Ok`). Windows runs the MSI
/// (its progress bar and the admin prompt show), macOS copies the app from
/// the disk image over this one, Linux replaces the AppImage. `Err` means
/// nothing was changed: the installer is then opened for the user instead.
pub fn install_and_relaunch(installer: &std::path::Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let pid = std::process::id();
    #[cfg(windows)]
    {
        let quote = |path: &std::path::Path| path.display().to_string().replace('\'', "''");
        let script = format!(
            "Wait-Process -Id {pid} -ErrorAction SilentlyContinue; \
             $run = Start-Process msiexec.exe -ArgumentList '/i', '\"{msi}\"', '/passive', '/norestart' -Wait -PassThru; \
             Start-Process '{exe}'",
            msi = quote(installer),
            exe = quote(&exe),
        );
        files_sdk::log::command("powershell")
            .args(["-NoProfile", "-WindowStyle", "Hidden", "-ExecutionPolicy", "Bypass", "-Command", &script])
            .spawn()
            .map_err(|error| format!("Could not start the installer: {error}"))?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        // …/Files.app/Contents/MacOS/file-force → …/Files.app
        let bundle = exe.ancestors().nth(3).filter(|bundle| bundle.extension().is_some_and(|extension| extension == "app")).ok_or("Not running from an app bundle")?;
        let quote = |path: &std::path::Path| format!("'{}'", path.display().to_string().replace('\'', r"'\''"));
        let mount = std::env::temp_dir().join(format!("file-force-update-{pid}"));
        let script = format!(
            "while kill -0 {pid} 2>/dev/null; do sleep 0.2; done; \
             mkdir -p {mount} && /usr/bin/hdiutil attach -nobrowse -readonly -mountpoint {mount} {dmg} >/dev/null && \
             app=$(ls -d {mount}/*.app | head -n1) && /usr/bin/ditto \"$app\" {bundle}.new && \
             rm -rf {bundle} && mv {bundle}.new {bundle}; \
             /usr/bin/hdiutil detach {mount} >/dev/null 2>&1; /usr/bin/open {bundle}",
            mount = quote(&mount),
            dmg = quote(installer),
            bundle = quote(bundle),
        );
        std::process::Command::new("/bin/sh").args(["-c", &script]).spawn().map_err(|error| error.to_string())?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Only an AppImage can replace itself; a package manager owns the rest.
        let image = std::env::var_os("APPIMAGE").map(std::path::PathBuf::from).ok_or("Not running from an AppImage")?;
        let quote = |path: &std::path::Path| format!("'{}'", path.display().to_string().replace('\'', r"'\''"));
        let script = format!(
            "while kill -0 {pid} 2>/dev/null; do sleep 0.2; done; cp {new} {image}.new && chmod 755 {image}.new && mv {image}.new {image}; {image} &",
            new = quote(installer),
            image = quote(&image),
        );
        let _ = exe;
        std::process::Command::new("/bin/sh").args(["-c", &script]).spawn().map_err(|error| error.to_string())?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err("Installing is not supported here".to_string())
}

pub fn now_seconds() -> i64 {
    now()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_urls_are_recognised() {
        assert_eq!(parse_github("https://github.com/owner/files"), Some("owner/files".into()));
        assert_eq!(parse_github("https://github.com/owner/files.git/"), Some("owner/files".into()));
        assert_eq!(parse_github("git@github.com:owner/files.git"), Some("owner/files".into()));
        assert_eq!(parse_github("https://gitlab.com/owner/files"), None);
        assert_eq!(parse_github(""), None);
    }

    #[test]
    fn an_update_is_announced_once() {
        assert_eq!(just_updated("", false), None, "first run: nothing to announce");
        assert_eq!(just_updated("", true).as_deref(), Some(CURRENT_VERSION), "from a version that did not record itself");
        assert_eq!(just_updated(CURRENT_VERSION, true), None, "same version");
        assert_eq!(just_updated("0.0.1", true).as_deref(), Some(CURRENT_VERSION));
        assert_eq!(just_updated("999.0.0", true), None, "a downgrade is not an update");
        assert!(release_notes_url("0.2.0").ends_with("/releases/tag/v0.2.0"));
        assert!(release_notes_url("v0.2.0").ends_with("/releases/tag/v0.2.0"));
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("0.10.0", "0.9.5"), "not string order");
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
    }

    #[test]
    fn release_answers_are_parsed() {
        let newer = r#"{"tag_name":"v99.0.0","html_url":"https://github.com/o/r/releases/tag/v99.0.0","body":"Fixes"}"#;
        match parse_release(newer).unwrap() {
            UpdateStatus::Available { latest, notes, page, download } => {
                assert_eq!((latest.as_str(), notes.as_str(), download), ("v99.0.0", "Fixes", None));
                assert_eq!(page.as_deref(), Some("https://github.com/o/r/releases/tag/v99.0.0"));
            }
            other => panic!("{other:?}"),
        }
        let same = format!(r#"{{"tag_name":"v{CURRENT_VERSION}","html_url":""}}"#);
        assert!(matches!(parse_release(&same).unwrap(), UpdateStatus::UpToDate { .. }));
        assert!(parse_release("not json").is_err());
    }

    #[test]
    fn latest_json_from_the_website() {
        let body = r#"{"version":"99.1.0","notes":"New: Kubernetes","macos":{"url":"https://files.dev/Files-99.1.0.dmg","signature":"abc"},"linux":{"url":"https://files.dev/Files.AppImage"}}"#;
        match parse_latest_json(body).unwrap() {
            UpdateStatus::Available { latest, notes, download, .. } => {
                assert_eq!((latest.as_str(), notes.as_str()), ("99.1.0", "New: Kubernetes"));
                let download = download.unwrap();
                if cfg!(target_os = "macos") {
                    assert_eq!(download.url, "https://files.dev/Files-99.1.0.dmg");
                    assert_eq!(download.signature.as_deref(), Some("abc"));
                }
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(parse_latest_json(&format!(r#"{{"version":"{CURRENT_VERSION}"}}"#)).unwrap(), UpdateStatus::UpToDate { .. }));
    }

    #[test]
    fn only_publisher_signed_downloads_are_kept() {
        let (private, public) = files_sdk::license::generate_keys().unwrap();
        let dir = std::env::temp_dir().join(format!("file-force-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Files-9.9.9.dmg");
        std::fs::write(&file, b"disk image").unwrap();
        let url = format!("file://{}", file.display());
        let good = Download { url: url.clone(), signature: Some(files_sdk::package::sign(b"disk image", &private).unwrap()) };
        let downloaded = download(&good, &[&public]).unwrap();
        assert_eq!(std::fs::read(&downloaded).unwrap(), b"disk image");
        let _ = std::fs::remove_file(&downloaded);
        let (stranger, _) = files_sdk::license::generate_keys().unwrap();
        let bad = Download { url, signature: Some(files_sdk::package::sign(b"disk image", &stranger).unwrap()) };
        let error = download(&bad, &[&public]).unwrap_err();
        assert!(error.contains("not signed"), "{error}");
        assert!(download(&Download { url: "file:///x".into(), signature: None }, &[&public]).unwrap_err().contains("not signed"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
