//! Spotify through its Web API: sign-in in the browser (PKCE, no secret),
//! liked tracks, playlists, search, and remote control of the Spotify app
//! (playback needs Premium; Spotify does not let other apps play the audio).

use std::io::{self, BufRead as _, Write as _};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::Value;

use crate::{Playlist, Source, Track, agent, check, http_error, log};

pub const REDIRECT_PORT: u16 = 43821;
pub const REDIRECT_URI: &str = "http://127.0.0.1:43821/callback";
const SCOPES: &str = "user-read-private user-library-read playlist-read-private playlist-read-collaborative \
                      user-read-playback-state user-modify-playback-state user-read-currently-playing";
const API: &str = "https://api.spotify.com/v1";
/// Keychain entry of the refresh token.
const SECRET: &str = "spotify-refresh-token";

struct Access {
    token: String,
    until: Instant,
}

static ACCESS: Mutex<Option<Access>> = Mutex::new(None);
/// The client id the app was built with (see Bundle.toml [music]).
static CLIENT_ID: Mutex<String> = Mutex::new(String::new());

pub fn set_client_id(client_id: &str) {
    *CLIENT_ID.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = client_id.trim().to_string();
}

fn client_id() -> io::Result<String> {
    let id = CLIENT_ID.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
    if id.is_empty() {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "This build has no Spotify client id (Bundle.toml [music] spotify_client_id)"));
    }
    Ok(id)
}

pub fn configured() -> bool {
    client_id().is_ok()
}

/// Signed in: a refresh token is in the keychain.
pub fn connected() -> bool {
    crate::signed_in(SECRET)
}

pub fn disconnect() -> io::Result<()> {
    *ACCESS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    crate::set_signed_in(SECRET, false);
    files_sdk::secrets::delete(crate::MANIFEST.id, SECRET)
}

fn random_url_safe(bytes: usize) -> io::Result<String> {
    let mut buffer = vec![0u8; bytes];
    getrandom::getrandom(&mut buffer).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buffer))
}

/// The PKCE challenge of a verifier: base64url(sha256(verifier)).
pub fn challenge(verifier: &str) -> String {
    use sha2::Digest as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

/// The address to send the user to.
pub fn authorize_url(client_id: &str, challenge: &str, state: &str) -> String {
    let mut url = url::Url::parse("https://accounts.spotify.com/authorize").expect("valid url");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", challenge)
        .append_pair("scope", SCOPES)
        .append_pair("state", state);
    url.into()
}

/// `code` out of the browser's `GET /callback?code=…&state=…`, checking the state.
pub fn parse_callback(request_line: &str, state: &str) -> io::Result<String> {
    let target = request_line.split_whitespace().nth(1).unwrap_or_default();
    let url = url::Url::parse(&format!("http://127.0.0.1{target}")).map_err(io::Error::other)?;
    let get = |key: &str| url.query_pairs().find(|(name, _)| name == key).map(|(_, value)| value.into_owned());
    if let Some(error) = get("error") {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("Spotify sign-in was cancelled ({error})")));
    }
    if get("state").as_deref() != Some(state) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Spotify sign-in answer did not match this request"));
    }
    get("code").ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Spotify sent no code"))
}

/// Signs in: opens the browser with `open` and waits (up to 5 minutes) for
/// Spotify to send the browser back to this computer.
pub fn login(open: impl FnOnce(&str)) -> io::Result<()> {
    let client_id = client_id()?;
    let listener = TcpListener::bind(("127.0.0.1", REDIRECT_PORT))
        .map_err(|error| io::Error::new(error.kind(), format!("Port {REDIRECT_PORT} is busy: {error}")))?;
    listener.set_nonblocking(true)?;
    let verifier = random_url_safe(64)?;
    let state = random_url_safe(16)?;
    open(&authorize_url(&client_id, &challenge(&verifier), &state));
    let started = Instant::now();
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if started.elapsed() > Duration::from_secs(300) {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "Spotify sign-in took too long"));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(error) => return Err(error),
        }
    };
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut request_line = String::new();
    io::BufReader::new(stream.try_clone()?).read_line(&mut request_line)?;
    let code = parse_callback(&request_line, &state);
    let page = match &code {
        Ok(_) => "<h2>Signed in to Spotify</h2><p>You can close this tab and go back to Files.</p>",
        Err(_) => "<h2>Spotify sign-in did not finish</h2><p>Go back to Files and try again.</p>",
    };
    let body = format!("<!doctype html><meta charset=utf-8><body style=\"font-family:sans-serif;text-align:center;margin-top:20vh\">{page}</body>");
    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let code = code?;
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", REDIRECT_URI),
        ("client_id", client_id.as_str()),
        ("code_verifier", verifier.as_str()),
    ];
    store_tokens(&token_request(&form)?)?;
    log("Spotify", "signed in".to_string());
    Ok(())
}

fn token_request(form: &[(&str, &str)]) -> io::Result<Value> {
    let response = agent().post("https://accounts.spotify.com/api/token").send_form(form.iter().copied()).map_err(http_error)?;
    check(response, "Spotify")?.body_mut().read_json::<Value>().map_err(http_error)
}

fn store_tokens(answer: &Value) -> io::Result<String> {
    let token = answer["access_token"].as_str().ok_or_else(|| io::Error::other("Spotify sent no token"))?.to_string();
    let seconds = answer["expires_in"].as_u64().unwrap_or(3600);
    if let Some(refresh) = answer["refresh_token"].as_str() {
        files_sdk::secrets::set(crate::MANIFEST.id, SECRET, refresh)?;
        crate::set_signed_in(SECRET, true);
    }
    *ACCESS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(Access { token: token.clone(), until: Instant::now() + Duration::from_secs(seconds.saturating_sub(60)) });
    Ok(token)
}

fn access_token(force_refresh: bool) -> io::Result<String> {
    if !force_refresh
        && let Some(access) = ACCESS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref()
        && Instant::now() < access.until
    {
        return Ok(access.token.clone());
    }
    let refresh = files_sdk::secrets::get(crate::MANIFEST.id, SECRET)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "Not signed in to Spotify"))?;
    let client_id = client_id()?;
    store_tokens(&token_request(&[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", &client_id)])?)
}

/// One API call, signing in again once when the token expired.
fn call(method: &str, path: &str, body: Option<Value>) -> io::Result<Option<Value>> {
    let url = if path.starts_with("https://") { path.to_string() } else { format!("{API}{path}") };
    for attempt in 0..2 {
        let token = access_token(attempt > 0)?;
        let authorization = format!("Bearer {token}");
        let response = match (method, &body) {
            ("GET", _) => agent().get(&url).header("Authorization", &authorization).call(),
            ("PUT", Some(body)) => agent().put(&url).header("Authorization", &authorization).send_json(body),
            ("PUT", None) => agent().put(&url).header("Authorization", &authorization).send_empty(),
            (_, Some(body)) => agent().post(&url).header("Authorization", &authorization).send_json(body),
            (_, None) => agent().post(&url).header("Authorization", &authorization).send_empty(),
        }
        .map_err(http_error)?;
        if response.status() == 401 && attempt == 0 {
            continue;
        }
        if response.status() == 204 {
            return Ok(None);
        }
        let mut response = check(response, "Spotify")?;
        let text = response.body_mut().read_to_string().map_err(http_error)?;
        return Ok(if text.trim().is_empty() { None } else { Some(serde_json::from_str(&text).map_err(io::Error::other)?) });
    }
    Err(io::Error::new(io::ErrorKind::PermissionDenied, "Spotify sign-in expired: connect again in Music"))
}

fn get(path: &str) -> io::Result<Value> {
    call("GET", path, None)?.ok_or_else(|| io::Error::other("Spotify sent nothing"))
}

/// A Web API track object as a Track.
pub fn track_of(item: &Value) -> Option<Track> {
    let uri = item["uri"].as_str()?;
    if !uri.starts_with("spotify:track:") {
        return None;
    }
    let artists: Vec<&str> = item["artists"].as_array().map(|artists| artists.iter().filter_map(|artist| artist["name"].as_str()).collect()).unwrap_or_default();
    Some(Track {
        source: Source::Spotify(uri.to_string()),
        title: item["name"].as_str().unwrap_or_default().to_string(),
        artist: artists.join(", "),
        album: item["album"]["name"].as_str().unwrap_or_default().to_string(),
        duration_ms: item["duration_ms"].as_u64().unwrap_or(0),
        cover_url: item["album"]["images"].as_array().and_then(|images| images.last().or(images.first())).and_then(|image| image["url"].as_str()).map(str::to_string),
        artist_id: item["artists"][0]["id"].as_str().map(str::to_string),
        album_id: item["album"]["id"].as_str().map(str::to_string),
    })
}

pub fn profile_name() -> io::Result<String> {
    let me = get("/me")?;
    Ok(me["display_name"].as_str().or(me["id"].as_str()).unwrap_or("Spotify").to_string())
}

/// Liked songs, newest first (the first 200).
pub fn liked() -> io::Result<Vec<Track>> {
    let mut tracks = Vec::new();
    let mut next = Some(format!("{API}/me/tracks?limit=50"));
    while let Some(url) = next.take() {
        let page = get(&url)?;
        tracks.extend(page["items"].as_array().into_iter().flatten().filter_map(|item| track_of(&item["track"])));
        next = page["next"].as_str().map(str::to_string).filter(|_| tracks.len() < 200);
    }
    Ok(tracks)
}

pub fn playlists() -> io::Result<Vec<Playlist>> {
    let page = get("/me/playlists?limit=50")?;
    Ok(page["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some(Playlist {
                id: item["id"].as_str()?.to_string(),
                name: item["name"].as_str().unwrap_or_default().to_string(),
                count: item["tracks"]["total"].as_u64().unwrap_or(0) as usize,
            })
        })
        .collect())
}

pub fn playlist_tracks(id: &str) -> io::Result<Vec<Track>> {
    let page = get(&format!("/playlists/{id}/tracks?limit=100"))?;
    Ok(page["items"].as_array().into_iter().flatten().filter_map(|item| track_of(&item["track"])).collect())
}

pub fn search(query: &str) -> io::Result<Vec<Track>> {
    let mut url = url::Url::parse(&format!("{API}/search")).expect("valid url");
    url.query_pairs_mut().append_pair("q", query).append_pair("type", "track").append_pair("limit", "50");
    let page = get(url.as_str())?;
    Ok(page["tracks"]["items"].as_array().into_iter().flatten().filter_map(track_of).collect())
}

fn track_id(track: &Track) -> io::Result<&str> {
    match &track.source {
        Source::Spotify(uri) => Ok(uri.trim_start_matches("spotify:track:")),
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Not a Spotify track")),
    }
}

/// Adds to (or removes from) Liked Songs.
pub fn set_liked(track: &Track, liked: bool) -> io::Result<()> {
    let id = track_id(track)?;
    let url = format!("/me/tracks?ids={id}");
    if liked { call("PUT", &url, None) } else { call_delete(&url) }.map(drop)
}

fn call_delete(path: &str) -> io::Result<Option<Value>> {
    let token = access_token(false)?;
    let response = agent().delete(&format!("{API}{path}")).header("Authorization", &format!("Bearer {token}")).call().map_err(http_error)?;
    check(response, "Spotify").map(|_| None)
}

/// Playlists this account can add to (its own and collaborative ones).
pub fn editable_playlists() -> io::Result<Vec<Playlist>> {
    let me = get("/me")?;
    let owner = me["id"].as_str().unwrap_or_default().to_string();
    let page = get("/me/playlists?limit=50")?;
    Ok(page["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["owner"]["id"].as_str() == Some(owner.as_str()) || item["collaborative"].as_bool() == Some(true))
        .filter_map(|item| {
            Some(Playlist {
                id: item["id"].as_str()?.to_string(),
                name: item["name"].as_str().unwrap_or_default().to_string(),
                count: item["tracks"]["total"].as_u64().unwrap_or(0) as usize,
            })
        })
        .collect())
}

pub fn add_to_playlist(playlist: &str, track: &Track) -> io::Result<()> {
    let Source::Spotify(uri) = &track.source else { return Err(io::Error::new(io::ErrorKind::InvalidInput, "Not a Spotify track")) };
    call("POST", &format!("/playlists/{playlist}/tracks"), Some(serde_json::json!({ "uris": [uri] }))).map(drop)
}

/// An artist's name and most played tracks (Spotify's "radio" for apps is gone:
/// its recommendations API is closed to new apps since 2024).
pub fn artist(artist_id: &str) -> io::Result<(String, Vec<Track>)> {
    let name = get(&format!("/artists/{artist_id}"))?["name"].as_str().unwrap_or("Artist").to_string();
    let top = get(&format!("/artists/{artist_id}/top-tracks?market=from_token"))?;
    Ok((name, top["tracks"].as_array().into_iter().flatten().filter_map(track_of).collect()))
}

/// What the Spotify app is playing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playback {
    pub track: Option<Track>,
    pub playing: bool,
    pub progress_ms: u64,
    pub device: Option<String>,
}

pub fn playback() -> io::Result<Playback> {
    let Some(state) = call("GET", "/me/player", None)? else { return Ok(Playback::default()) };
    Ok(Playback {
        track: track_of(&state["item"]),
        playing: state["is_playing"].as_bool().unwrap_or(false),
        progress_ms: state["progress_ms"].as_u64().unwrap_or(0),
        device: state["device"]["name"].as_str().map(str::to_string),
    })
}

/// Plays `uris` (starting at `index`) on the active Spotify device, or on
/// the first one when none is active.
pub fn play(uris: &[String], index: usize) -> io::Result<()> {
    let body = serde_json::json!({ "uris": uris, "offset": { "position": index } });
    match call("PUT", "/me/player/play", Some(body.clone())) {
        Err(error) if error.to_string().contains("NO_ACTIVE_DEVICE") || error.to_string().contains("404") => {
            let devices = get("/me/player/devices")?;
            let id = devices["devices"].as_array().and_then(|devices| devices.first()).and_then(|device| device["id"].as_str()).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "Open Spotify on this computer or your phone first: there is no device to play on")
            })?;
            call("PUT", &format!("/me/player/play?device_id={id}"), Some(body)).map(drop)
        }
        Err(error) if error.to_string().contains("PREMIUM_REQUIRED") => {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "Spotify lets other apps start music only with Premium"))
        }
        other => other.map(drop),
    }
}

pub fn pause() -> io::Result<()> {
    call("PUT", "/me/player/pause", None).map(drop)
}

pub fn resume() -> io::Result<()> {
    call("PUT", "/me/player/play", None).map(drop)
}

pub fn next() -> io::Result<()> {
    call("POST", "/me/player/next", None).map(drop)
}

pub fn previous() -> io::Result<()> {
    call("POST", "/me/player/previous", None).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_the_rfc_example() {
        // RFC 7636, appendix B.
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn the_callback_gives_the_code_only_for_this_request() {
        assert_eq!(parse_callback("GET /callback?code=abc&state=s1 HTTP/1.1", "s1").unwrap(), "abc");
        assert!(parse_callback("GET /callback?code=abc&state=other HTTP/1.1", "s1").is_err(), "a foreign state");
        let denied = parse_callback("GET /callback?error=access_denied&state=s1 HTTP/1.1", "s1").unwrap_err();
        assert_eq!(denied.kind(), io::ErrorKind::PermissionDenied);
        let url = authorize_url("id1", "ch", "s1");
        assert!(url.contains("client_id=id1") && url.contains("code_challenge=ch") && url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A43821%2Fcallback"), "{url}");
    }

    #[test]
    fn web_api_tracks_become_tracks() {
        let item = serde_json::json!({
            "uri": "spotify:track:1", "name": "Song", "duration_ms": 61000,
            "artists": [{"name": "A"}, {"name": "B"}],
            "album": {"name": "LP", "images": [{"url": "big"}, {"url": "small"}]}
        });
        let track = track_of(&item).unwrap();
        assert_eq!((track.title.as_str(), track.artist.as_str(), track.album.as_str()), ("Song", "A, B", "LP"));
        assert_eq!(track.duration(), "1:01");
        assert_eq!(track.cover_url.as_deref(), Some("small"));
        assert!(track_of(&serde_json::json!({"uri": "spotify:episode:1"})).is_none(), "podcasts are not tracks");
    }
}
