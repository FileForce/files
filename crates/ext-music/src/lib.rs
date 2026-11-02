//! Music: a library of the music folders on this computer, plus Spotify
//! and Yandex Music accounts. Local files and Yandex tracks play in the app
//! (through VLC, which the app ships); Spotify plays in the Spotify app and
//! is remote-controlled through its Web API (Premium), as Spotify requires.

use std::io;
use std::path::PathBuf;

use files_sdk::{Icon, Manifest, Permission, host};

pub mod local;
pub mod spotify;
pub mod yandex;

pub static MANIFEST: Manifest = Manifest {
    id: "music",
    name: "Music",
    version: env!("CARGO_PKG_VERSION"),
    source: "Built-in (experimental)",
    summary: "Your music folders as a library with covers and a queue, plus Spotify and Yandex Music: liked tracks, playlists, search.",
    icon: Icon::Builtin("music"),
    adds: &[
        "Sidebar → Music: the library, Spotify and Yandex Music",
        "Context menu on audio files → Play in Music, Add to Queue",
        "Music tab: tracks, playlists, search and the player",
    ],
    permissions: &[Permission::Network, Permission::Secrets, Permission::ReadFiles],
    commands: &[],
    menu: &[],
    sidebar: &[],
    menu_color: Some("pink"),
    tier: files_sdk::Tier::Free,
};

pub struct Music;

pub static EXTENSION: Music = Music;

impl files_sdk::Extension for Music {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn search_keys(&self) -> &'static [files_sdk::SearchKey] {
        &[files_sdk::SearchKey {
            key: "music",
            summary: "tracks in the library, Spotify and Yandex Music (title, artist, album)",
            example: "music:beatles",
        }]
    }
}

/// `music:` in global search: the library first, then the services the
/// user is signed in to (each failing quietly).
pub fn search_everywhere(query: &str, limit: usize) -> Vec<Track> {
    let mut found: Vec<Track> = local::filter(&local::library(), query).into_iter().take(limit).cloned().collect();
    if found.len() < limit && spotify::connected() {
        found.extend(spotify::search(query).unwrap_or_default().into_iter().take(limit - found.len()));
    }
    if found.len() < limit && yandex::connected() {
        found.extend(yandex::search(query).unwrap_or_default().into_iter().take(limit - found.len()));
    }
    found
}

/// Where a track comes from, and how to play it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Source {
    /// A file on this computer.
    Local(PathBuf),
    /// `spotify:track:…`, played in the Spotify app.
    Spotify(String),
    /// A Yandex Music track id, downloaded then played here.
    Yandex(String),
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Track {
    pub source: Source,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Milliseconds, 0 when unknown.
    pub duration_ms: u64,
    /// A cover picture: a local file's own, or a web address.
    pub cover_url: Option<String>,
    /// The service's id of the (first) artist, for "Go to Artist" and radio.
    #[serde(default)]
    pub artist_id: Option<String>,
    /// The service's album id (Yandex needs it to add to a playlist).
    #[serde(default)]
    pub album_id: Option<String>,
}

impl Track {
    pub fn duration(&self) -> String {
        let seconds = self.duration_ms / 1000;
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

/// Where a track's cover is kept once fetched (by its address or file).
pub fn cover_cache_path(track: &Track) -> Option<PathBuf> {
    use sha2::Digest as _;
    let key = match (&track.source, &track.cover_url) {
        (Source::Local(path), _) => format!("file:{}", path.display()),
        (_, Some(url)) => url.clone(),
        _ => return None,
    };
    let hash: String = sha2::Sha256::digest(key.as_bytes()).iter().take(12).map(|byte| format!("{byte:02x}")).collect();
    Some(std::env::temp_dir().join("file-force-music").join("covers").join(format!("{hash}.img")))
}

/// Fetches a cover into the cache: the web address, or a file's own picture.
/// `None` when the track has none.
pub fn fetch_cover(track: &Track) -> Option<PathBuf> {
    let target = cover_cache_path(track)?;
    if target.is_file() {
        return Some(target);
    }
    let bytes = match (&track.source, &track.cover_url) {
        (Source::Local(path), _) => local::cover(path)?,
        (_, Some(url)) => {
            let response = agent().get(url).call().ok()?;
            check(response, "cover").ok()?.body_mut().with_config().limit(8 * 1024 * 1024).read_to_vec().ok()?
        }
        _ => return None,
    };
    std::fs::create_dir_all(target.parent()?).ok()?;
    let partial = target.with_extension("partial");
    std::fs::write(&partial, bytes).ok()?;
    std::fs::rename(&partial, &target).ok()?;
    Some(target)
}

/// A list of tracks the sidebar shows: liked, a playlist…
#[derive(Clone, Debug, PartialEq)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub count: usize,
}

/// `music.toml` beside `settings.toml`: library folders and the sound.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Config {
    pub folders: Vec<PathBuf>,
    /// Yandex account id, read once from the token.
    pub yandex_uid: Option<u64>,
    /// 0…100.
    pub volume: u8,
    /// The equalizer when it is on.
    pub equalizer: Option<EqualizerConfig>,
    /// Groups folded in the Music tab's left column (`local`, `spotify`, `yandex`).
    pub collapsed: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self { folders: Vec::new(), yandex_uid: None, volume: 80, equalizer: None, collapsed: Vec::new() }
    }
}

/// Gains in dB per band (60 Hz … 16 kHz) and the preamp; the preset it
/// started from, if any.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EqualizerConfig {
    pub preset: Option<String>,
    pub preamp: f32,
    pub bands: Vec<f32>,
}

fn config_path() -> Option<PathBuf> {
    Some(host().config_dir()?.join("music.toml"))
}

/// Read once, then kept: the Music tab draws often and asks every time.
static CONFIG: std::sync::Mutex<Option<Config>> = std::sync::Mutex::new(None);

pub fn config() -> Config {
    let mut cached = CONFIG.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    cached
        .get_or_insert_with(|| {
            let mut config: Config = config_path()
                .and_then(|path| std::fs::read_to_string(path).ok())
                .and_then(|text| toml::from_str(&text).ok())
                .unwrap_or_default();
            if config.folders.is_empty()
                && let Some(music) = dirs_music()
            {
                config.folders.push(music);
            }
            config
        })
        .clone()
}

pub fn save_config(config: &Config) -> io::Result<()> {
    *CONFIG.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(config.clone());
    let path = config_path().ok_or_else(|| io::Error::other("no settings folder"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, toml::to_string_pretty(config).map_err(io::Error::other)?)
}

/// Which services have a token in the keychain: asked once per run (the
/// keychain is another process), then updated on sign-in and sign-out.
static SIGNED: std::sync::Mutex<Vec<(&'static str, bool)>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn signed_in(secret: &'static str) -> bool {
    let mut known = SIGNED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((_, signed_in)) = known.iter().find(|(name, _)| *name == secret) {
        return *signed_in;
    }
    let signed_in = files_sdk::secrets::get(MANIFEST.id, secret).ok().flatten().is_some();
    known.push((secret, signed_in));
    signed_in
}

pub(crate) fn set_signed_in(secret: &'static str, value: bool) {
    let mut known = SIGNED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    known.retain(|(name, _)| *name != secret);
    known.push((secret, value));
}

fn dirs_music() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let music = PathBuf::from(home).join("Music");
    music.is_dir().then_some(music)
}

/// A JSON client shared by Spotify and Yandex: rustls, sane timeouts.
pub(crate) fn agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into()
    })
}

pub(crate) fn http_error(error: ureq::Error) -> io::Error {
    match error {
        ureq::Error::Io(error) => error,
        other => io::Error::other(other.to_string()),
    }
}

/// Fails with the server's message for a non-2xx answer.
pub(crate) fn check(mut response: ureq::http::Response<ureq::Body>, service: &str) -> io::Result<ureq::http::Response<ureq::Body>> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.body_mut().read_to_string().unwrap_or_default();
    let kind = if status == 401 || status == 403 { io::ErrorKind::PermissionDenied } else { io::ErrorKind::Other };
    let detail: String = body.chars().take(200).collect();
    Err(io::Error::new(kind, format!("{service} answered {status}: {detail}")))
}

pub(crate) fn log(service: &str, message: String) {
    files_sdk::log::record(MANIFEST.id, files_sdk::log::Kind::Network, format!("{service} · {message}"), None);
}
