//! Yandex Music through the API its own apps use (there is no public one,
//! so it may change). Sign-in is an OAuth token; tracks are downloaded to a
//! cache and played in the app.

use std::io;
use std::path::PathBuf;

use serde_json::Value;

use crate::{Playlist, Source, Track, agent, check, http_error, log};

const API: &str = "https://api.music.yandex.net";
/// The client id of Yandex Music's own app: its token page.
pub const TOKEN_PAGE: &str = "https://oauth.yandex.ru/authorize?response_type=token&client_id=23cabbbdc6cd418abb4b39c32c41195d";
const SECRET: &str = "yandex-token";
/// Mixed into the download signature (public, from Yandex's apps).
const SIGN_SALT: &str = "XGRlBW9FXlekgbPrRHuSiA";

/// A pasted token, or the whole address the token page ends on
/// (`…#access_token=…&token_type=bearer…`).
pub fn token_from(text: &str) -> Option<String> {
    let text = text.trim();
    let token = match text.find("access_token=") {
        Some(start) => text[start + "access_token=".len()..].split(['&', '#', ' ']).next().unwrap_or_default(),
        None => text,
    };
    (token.len() >= 20 && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')).then(|| token.to_string())
}

pub fn connected() -> bool {
    crate::signed_in(SECRET)
}

fn token() -> io::Result<String> {
    files_sdk::secrets::get(crate::MANIFEST.id, SECRET)?.ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "Not signed in to Yandex Music"))
}

fn get(path: &str) -> io::Result<Value> {
    let url = if path.starts_with("https://") { path.to_string() } else { format!("{API}{path}") };
    let response = agent().get(&url).header("Authorization", &format!("OAuth {}", token()?)).call().map_err(http_error)?;
    let mut answer: Value = check(response, "Yandex Music")?.body_mut().read_json().map_err(http_error)?;
    Ok(answer["result"].take())
}

/// Checks the token, remembers it (keychain) and the account id.
pub fn connect(text: &str) -> io::Result<String> {
    let token = token_from(text).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Paste the token or the whole address the Yandex page ended on"))?;
    let response = agent().get(&format!("{API}/account/status")).header("Authorization", &format!("OAuth {token}")).call().map_err(http_error)?;
    let answer: Value = check(response, "Yandex Music")?.body_mut().read_json().map_err(http_error)?;
    let account = &answer["result"]["account"];
    let uid = account["uid"].as_u64().ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "Yandex did not accept this token"))?;
    files_sdk::secrets::set(crate::MANIFEST.id, SECRET, &token)?;
    crate::set_signed_in(SECRET, true);
    let mut config = crate::config();
    config.yandex_uid = Some(uid);
    crate::save_config(&config)?;
    let name = account["displayName"].as_str().or(account["login"].as_str()).unwrap_or("Yandex").to_string();
    log("Yandex Music", format!("signed in as {name}"));
    Ok(name)
}

pub fn disconnect() -> io::Result<()> {
    let mut config = crate::config();
    config.yandex_uid = None;
    crate::save_config(&config)?;
    crate::set_signed_in(SECRET, false);
    files_sdk::secrets::delete(crate::MANIFEST.id, SECRET)
}

fn uid() -> io::Result<u64> {
    crate::config().yandex_uid.ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "Not signed in to Yandex Music"))
}

/// Ids come as numbers or strings.
fn id_of(value: &Value) -> Option<String> {
    match value {
        Value::String(id) => Some(id.clone()),
        Value::Number(id) => Some(id.to_string()),
        _ => None,
    }
}

/// A Yandex track object as a Track.
pub fn track_of(item: &Value) -> Option<Track> {
    let id = match &item["id"] {
        Value::String(id) => id.clone(),
        Value::Number(id) => id.to_string(),
        _ => return None,
    };
    let artists: Vec<&str> = item["artists"].as_array().map(|artists| artists.iter().filter_map(|artist| artist["name"].as_str()).collect()).unwrap_or_default();
    Some(Track {
        source: Source::Yandex(id),
        title: item["title"].as_str().unwrap_or_default().to_string(),
        artist: artists.join(", "),
        album: item["albums"][0]["title"].as_str().unwrap_or_default().to_string(),
        duration_ms: item["durationMs"].as_u64().unwrap_or(0),
        cover_url: item["coverUri"].as_str().map(|uri| format!("https://{}", uri.replace("%%", "200x200"))),
        artist_id: id_of(&item["artists"][0]["id"]),
        album_id: id_of(&item["albums"][0]["id"]),
    })
}

/// Full track objects for ids (the likes list only has ids).
fn tracks(ids: &[String]) -> io::Result<Vec<Track>> {
    let mut found = Vec::new();
    for chunk in ids.chunks(100) {
        let answer = get(&format!("/tracks?track-ids={}", chunk.join(",")))?;
        found.extend(answer.as_array().into_iter().flatten().filter_map(track_of));
    }
    Ok(found)
}

/// Liked tracks, newest first (the first 300).
pub fn liked() -> io::Result<Vec<Track>> {
    let answer = get(&format!("/users/{}/likes/tracks", uid()?))?;
    let ids: Vec<String> = answer["library"]["tracks"]
        .as_array()
        .into_iter()
        .flatten()
        .take(300)
        .filter_map(|item| item["id"].as_str().map(str::to_string).or_else(|| item["id"].as_u64().map(|id| id.to_string())))
        .collect();
    tracks(&ids)
}

pub fn playlists() -> io::Result<Vec<Playlist>> {
    let answer = get(&format!("/users/{}/playlists/list", uid()?))?;
    Ok(answer
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some(Playlist {
                id: item["kind"].as_u64()?.to_string(),
                name: item["title"].as_str().unwrap_or_default().to_string(),
                count: item["trackCount"].as_u64().unwrap_or(0) as usize,
            })
        })
        .collect())
}

pub fn playlist_tracks(kind: &str) -> io::Result<Vec<Track>> {
    let answer = get(&format!("/users/{}/playlists/{kind}", uid()?))?;
    let items = answer["tracks"].as_array().cloned().unwrap_or_default();
    let complete: Vec<Track> = items.iter().filter_map(|item| track_of(&item["track"])).collect();
    if complete.len() == items.len() {
        return Ok(complete);
    }
    // Some answers carry only ids.
    let ids: Vec<String> = items.iter().filter_map(|item| item["id"].as_u64().map(|id| id.to_string()).or_else(|| item["id"].as_str().map(str::to_string))).collect();
    tracks(&ids)
}

pub fn search(query: &str) -> io::Result<Vec<Track>> {
    let mut url = url::Url::parse(&format!("{API}/search")).expect("valid url");
    url.query_pairs_mut().append_pair("text", query).append_pair("type", "track").append_pair("page", "0");
    let answer = get(url.as_str())?;
    Ok(answer["tracks"]["results"].as_array().into_iter().flatten().filter_map(track_of).collect())
}

// ------------------------------------------------------------ actions

fn post_form(path: &str, form: &[(&str, &str)]) -> io::Result<Value> {
    let response = agent()
        .post(&format!("{API}{path}"))
        .header("Authorization", &format!("OAuth {}", token()?))
        .send_form(form.iter().copied())
        .map_err(http_error)?;
    let mut answer: Value = check(response, "Yandex Music")?.body_mut().read_json().map_err(http_error)?;
    Ok(answer["result"].take())
}

fn track_id(track: &Track) -> io::Result<&str> {
    match &track.source {
        Source::Yandex(id) => Ok(id),
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Not a Yandex Music track")),
    }
}

/// «Мне нравится», or takes it back.
pub fn set_liked(track: &Track, liked: bool) -> io::Result<()> {
    let action = if liked { "add-multiple" } else { "remove" };
    post_form(&format!("/users/{}/likes/tracks/{action}", uid()?), &[("track-ids", track_id(track)?)]).map(drop)
}

/// «Не нравится»: Yandex stops suggesting it (and My Wave learns).
pub fn dislike(track: &Track) -> io::Result<()> {
    post_form(&format!("/users/{}/dislikes/tracks/add-multiple", uid()?), &[("track-ids", track_id(track)?)]).map(drop)
}

/// Inserts the track at the top of the playlist `kind`.
pub fn add_to_playlist(kind: &str, track: &Track) -> io::Result<()> {
    let id = track_id(track)?;
    let album = track.album_id.clone().ok_or_else(|| io::Error::other("Yandex did not say which album this track is from"))?;
    let uid = uid()?;
    // Changes are made against the playlist's current revision.
    let playlist = get(&format!("/users/{uid}/playlists/{kind}"))?;
    let revision = playlist["revision"].as_u64().unwrap_or(1).to_string();
    let diff = serde_json::json!([{ "op": "insert", "at": 0, "tracks": [{ "id": id, "albumId": album }] }]).to_string();
    post_form(&format!("/users/{uid}/playlists/{kind}/change-relative"), &[("diff", diff.as_str()), ("revision", revision.as_str())]).map(drop)
}

/// An artist's name and tracks (most popular first).
pub fn artist(artist_id: &str) -> io::Result<(String, Vec<Track>)> {
    let brief = get(&format!("/artists/{artist_id}/brief-info"))?;
    let name = brief["artist"]["name"].as_str().unwrap_or("Artist").to_string();
    let tracks = get(&format!("/artists/{artist_id}/tracks?page-size=50"))?;
    Ok((name, tracks["tracks"].as_array().into_iter().flatten().filter_map(track_of).collect()))
}

/// The station built around an artist.
pub fn artist_station(artist_id: &str) -> String {
    format!("artist:{artist_id}")
}

/// Saves a track as `Artist — Title.mp3` in `folder`, with its tags written.
pub fn save_to(track: &Track, folder: &std::path::Path) -> io::Result<PathBuf> {
    use lofty::config::WriteOptions;
    use lofty::file::TaggedFileExt as _;
    use lofty::tag::{Accessor as _, TagExt as _};
    let cached = download(track_id(track)?)?;
    let clean = |text: &str| text.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect::<String>();
    let name = if track.artist.is_empty() { clean(&track.title) } else { format!("{} — {}", clean(&track.artist), clean(&track.title)) };
    let target = files_sdk::fs::unique_path(&folder.join(format!("{name}.mp3")));
    std::fs::copy(&cached, &target)?;
    // Tags, so the file is named the same in any player.
    if let Ok(mut file) = lofty::read_from_path(&target) {
        if file.primary_tag().is_none() {
            let kind = file.primary_tag_type();
            file.insert_tag(lofty::tag::Tag::new(kind));
        }
        if let Some(tag) = file.primary_tag_mut() {
            tag.set_title(track.title.clone());
            tag.set_artist(track.artist.clone());
            tag.set_album(track.album.clone());
            let _ = tag.save_to_path(&target, WriteOptions::default());
        }
    }
    log("Yandex Music", format!("saved {}", target.display()));
    Ok(target)
}

// ------------------------------------------------------------ My Wave

/// "Моя волна", the personal radio.
pub const MY_WAVE: &str = "user:onyourwave";

/// The next tracks of a station: the first batch, or the ones after `after`
/// (a track id). Returns them with the batch id feedback goes to.
pub fn wave(station: &str, after: Option<&str>) -> io::Result<(Vec<Track>, String)> {
    let mut path = format!("/rotor/station/{station}/tracks?settings2=true");
    if let Some(after) = after {
        path.push_str(&format!("&queue={after}"));
    }
    let answer = get(&path)?;
    let tracks = answer["sequence"].as_array().into_iter().flatten().filter_map(|item| track_of(&item["track"])).collect();
    Ok((tracks, answer["batchId"].as_str().unwrap_or_default().to_string()))
}

/// What happened, for the station to learn from: `radioStarted`,
/// `trackStarted`, `trackFinished`, `skip`. Best effort: a failure is only
/// logged, the music goes on.
pub fn wave_feedback(station: &str, kind: &str, track: Option<&str>, batch: &str, played_seconds: u64) {
    let mut body = serde_json::json!({
        "type": kind,
        "timestamp": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_secs()),
        "from": "files-desktop",
    });
    if let Some(track) = track {
        body["trackId"] = Value::String(track.to_string());
    }
    if kind == "trackFinished" || kind == "skip" {
        body["totalPlayedSeconds"] = Value::from(played_seconds);
    }
    let url = format!("{API}/rotor/station/{station}/feedback?batch-id={batch}");
    let sent = token().and_then(|token| agent().post(&url).header("Authorization", &format!("OAuth {token}")).send_json(&body).map_err(http_error));
    if let Err(error) = sent {
        log("Yandex Music", format!("wave feedback {kind} not sent: {error}"));
    }
}

/// `<tag>value</tag>` out of the small download-info document.
fn xml_value<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let start = xml.find(&format!("<{tag}>"))? + tag.len() + 2;
    let end = start + xml[start..].find(&format!("</{tag}>"))?;
    Some(&xml[start..end])
}

/// The stream address from download-info: md5(salt + path without "/" + s).
pub fn stream_url(info_xml: &str) -> Option<String> {
    use md5::Digest as _;
    let (host, path, ts, s) = (xml_value(info_xml, "host")?, xml_value(info_xml, "path")?, xml_value(info_xml, "ts")?, xml_value(info_xml, "s")?);
    let sign = md5::Md5::digest(format!("{SIGN_SALT}{}{s}", path.trim_start_matches('/')).as_bytes());
    let sign: String = sign.iter().map(|byte| format!("{byte:02x}")).collect();
    Some(format!("https://{host}/get-mp3/{sign}/{ts}{path}"))
}

fn cache_dir() -> PathBuf {
    std::env::temp_dir().join("file-force-music")
}

/// Where a track is (or will be) cached.
pub fn cached_file(id: &str) -> PathBuf {
    cache_dir().join(format!("yandex-{}.mp3", id.replace(|c: char| !c.is_ascii_alphanumeric(), "_")))
}

/// The address the best MP3 of a track streams from (valid for a while).
pub fn stream(id: &str) -> io::Result<String> {
    let options = get(&format!("/tracks/{id}/download-info"))?;
    let best = options
        .as_array()
        .into_iter()
        .flatten()
        .filter(|option| option["codec"].as_str() == Some("mp3"))
        .max_by_key(|option| option["bitrateInKbps"].as_u64().unwrap_or(0))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Yandex offers no MP3 for this track (a subscription may be needed)"))?;
    let info_url = best["downloadInfoUrl"].as_str().ok_or_else(|| io::Error::other("Yandex sent no download address"))?;
    let info = agent().get(info_url).call().map_err(http_error)?;
    let xml = check(info, "Yandex Music")?.body_mut().read_to_string().map_err(http_error)?;
    stream_url(&xml).ok_or_else(|| io::Error::other("Yandex download info could not be read"))
}

/// How much of a download has arrived, shared with the player's buffer bar.
#[derive(Debug, Default)]
pub struct Progress {
    pub received: std::sync::atomic::AtomicU64,
    /// 0 until the server says how big it is.
    pub total: std::sync::atomic::AtomicU64,
}

impl Progress {
    /// 0…1 of the track downloaded.
    pub fn fraction(&self) -> f32 {
        let total = self.total.load(std::sync::atomic::Ordering::Relaxed);
        if total == 0 { 0. } else { (self.received.load(std::sync::atomic::Ordering::Relaxed) as f32 / total as f32).min(1.) }
    }
}

/// Downloads `url` into the cache of track `id`, reporting as it goes.
pub fn fetch(id: &str, url: &str, progress: &Progress) -> io::Result<PathBuf> {
    use std::io::{Read as _, Write as _};
    use std::sync::atomic::Ordering;
    let file = cached_file(id);
    if file.is_file() {
        let size = std::fs::metadata(&file)?.len();
        progress.total.store(size, Ordering::Relaxed);
        progress.received.store(size, Ordering::Relaxed);
        return Ok(file);
    }
    let response = check(agent().get(url).call().map_err(http_error)?, "Yandex Music")?;
    let total = response.headers().get("content-length").and_then(|value| value.to_str().ok()?.parse().ok()).unwrap_or(0);
    progress.total.store(total, Ordering::Relaxed);
    std::fs::create_dir_all(cache_dir())?;
    let partial = file.with_extension(format!("partial-{}", std::process::id()));
    let mut output = std::fs::File::create(&partial)?;
    let mut body = response.into_body();
    let mut reader = body.as_reader();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        progress.received.fetch_add(read as u64, Ordering::Relaxed);
    }
    drop(output);
    std::fs::rename(&partial, &file)?;
    log("Yandex Music", format!("cached track {id}"));
    Ok(file)
}

/// Downloads the best MP3 of a track (once; later plays use the cache).
pub fn download(id: &str) -> io::Result<PathBuf> {
    let file = cached_file(id);
    if file.is_file() {
        return Ok(file);
    }
    fetch(id, &stream(id)?, &Progress::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_is_cached_with_its_progress() {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = vec![7u8; 100 * 1024];
        let served = body.clone();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", served.len());
            let _ = stream.write_all(&served);
        });
        let id = format!("test-{}", std::process::id());
        let _ = std::fs::remove_file(cached_file(&id));
        let progress = Progress::default();
        assert_eq!(progress.fraction(), 0., "nothing yet");
        let file = fetch(&id, &format!("http://127.0.0.1:{port}/track.mp3"), &progress).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), body);
        assert_eq!(progress.fraction(), 1., "fully downloaded");
        // Cached: a second fetch does not download again.
        let again = Progress::default();
        assert_eq!(fetch(&id, "http://127.0.0.1:1/never", &again).unwrap(), file);
        assert_eq!(again.fraction(), 1.);
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn tokens_are_read_from_a_paste_or_the_address() {
        let token = "y0_AgAAAAAABCDEFGHIJKLMNOPQRSTUVWXYZ";
        assert_eq!(token_from(token).as_deref(), Some(token));
        let address = format!("https://music.yandex.ru/#access_token={token}&token_type=bearer&expires_in=31535645");
        assert_eq!(token_from(&address).as_deref(), Some(token));
        assert_eq!(token_from("short"), None);
        assert_eq!(token_from("has spaces in it but is long enough"), None);
    }

    #[test]
    fn download_info_becomes_a_signed_address() {
        let xml = "<?xml version=\"1.0\"?><download-info><host>s1.storage.yandex.net</host><path>/rmusic/abc/def</path><ts>0005f1</ts><region>-1</region><s>9e0</s></download-info>";
        let url = stream_url(xml).unwrap();
        assert!(url.starts_with("https://s1.storage.yandex.net/get-mp3/") && url.ends_with("/0005f1/rmusic/abc/def"), "{url}");
        let sign = url.split('/').nth(4).unwrap();
        assert_eq!(sign.len(), 32, "an md5 in hex");
        assert!(stream_url("<download-info></download-info>").is_none());
    }

    #[test]
    fn yandex_tracks_become_tracks() {
        let item = serde_json::json!({
            "id": "123", "title": "Песня", "durationMs": 125000,
            "artists": [{"name": "Исполнитель"}], "albums": [{"title": "Альбом"}],
            "coverUri": "avatars.yandex.net/get-music-content/1/2/%%"
        });
        let track = track_of(&item).unwrap();
        assert_eq!(track.source, Source::Yandex("123".into()));
        assert_eq!(track.cover_url.as_deref(), Some("https://avatars.yandex.net/get-music-content/1/2/200x200"));
        assert_eq!(track.duration(), "2:05");
        assert_eq!(track_of(&serde_json::json!({"id": 7, "title": "x"})).unwrap().source, Source::Yandex("7".into()));
    }
}
