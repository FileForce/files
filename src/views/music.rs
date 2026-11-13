//! Music in the UI: one player for the whole app (a queue of local files,
//! Yandex tracks played through VLC, or Spotify remote-controlled), the
//! Music tab (sources, search, tracks, now playing) and the sidebar entry.

use std::path::{Path, PathBuf};
use std::time::Duration;

use files_ext_music::{Playlist, Source, Track, local, spotify, yandex};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable, FontWeight, Global, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription, Window, div, px,
};

use crate::app::{AppearanceSettings, FileExplorer};
use crate::palette::Palette;
use crate::video::{Player, clock};

/// How often the player checks the time and the end of a track.
const TICK: Duration = Duration::from_millis(250);
/// Spotify is asked what it plays this often (it is another app).
const SPOTIFY_POLL: u32 = 12;

/// The web page to open, outside the app (sign-in pages).
fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = files_sdk::log::command("/usr/bin/open").arg(url).spawn();
    // `start` in cmd would cut the address at its first `&`.
    #[cfg(windows)]
    let _ = files_sdk::log::command("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = files_sdk::log::command("xdg-open").arg(url).spawn();
}

// ------------------------------------------------------------------ covers

/// Bumped when a cover arrives: visible rows pick it up.
static COVERS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Covers asked for (so each is fetched once).
static ASKED: std::sync::Mutex<Option<std::collections::HashSet<PathBuf>>> = std::sync::Mutex::new(None);

/// One worker fetches covers in turn, so a long list never floods the network.
fn cover_worker() -> &'static std::sync::mpsc::Sender<Track> {
    static WORKER: std::sync::OnceLock<std::sync::mpsc::Sender<Track>> = std::sync::OnceLock::new();
    WORKER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<Track>();
        std::thread::spawn(move || {
            for track in receiver {
                if files_ext_music::fetch_cover(&track).is_some() {
                    COVERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        });
        sender
    })
}

/// The cached cover of a track, or `None` while it is fetched (asked once).
pub(crate) fn cover_path(track: &Track, small: bool) -> Option<PathBuf> {
    let path = files_ext_music::cover_cache_path(track, small)?;
    if path.is_file() {
        return Some(path);
    }
    let mut asked = ASKED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if asked.get_or_insert_with(Default::default).insert(path) {
        let _ = cover_worker().send(track.clone());
    }
    None
}

// ------------------------------------------------------------------ player

/// What extensions asked for (`files_sdk::audio`), on the UI thread.
pub(crate) fn run_extension_commands(commands: Vec<files_sdk::audio::Command>, cx: &mut App) {
    use files_sdk::audio::{Command, Source as From};
    let jukebox = jukebox(cx);
    jukebox.update(cx, |jukebox, cx| {
        for command in commands {
            match command {
                Command::Queue { tracks, replace } => {
                    let tracks: Vec<Track> = tracks
                        .into_iter()
                        .map(|track| Track {
                            source: match track.source {
                                From::File(path) => Source::Local(path),
                                From::Url(url) => Source::Url(url),
                            },
                            title: track.title,
                            artist: track.artist,
                            album: track.album,
                            duration_ms: 0,
                            cover_url: None,
                            artist_id: None,
                            album_id: None,
                        })
                        .collect();
                    if replace {
                        jukebox.play_queue(tracks, 0, cx);
                    } else {
                        jukebox.enqueue(tracks, cx);
                    }
                }
                Command::Pause if jukebox.playing => jukebox.toggle(cx),
                Command::Resume if !jukebox.playing => jukebox.toggle(cx),
                Command::Next => jukebox.next(cx),
                Command::Previous => jukebox.previous(cx),
                Command::Stop => jukebox.stop(cx),
                _ => {}
            }
        }
    });
}

/// Volume or equalizer changed (from any panel): what plays hears it now.
pub(crate) fn sound_changed(cx: &mut App) {
    if let Some(global) = cx.try_global::<JukeboxGlobal>() {
        let jukebox = global.0.clone();
        jukebox.read(cx).sound_changed();
    }
}

/// The saved sound (volume, equalizer), applied to every player.
pub(crate) fn apply_sound(player: &Player) {
    let config = files_ext_music::config();
    player.set_volume(config.volume);
    let equalizer = config.equalizer.map(|equalizer| crate::video::Equalizer { preamp: equalizer.preamp, bands: equalizer.bands });
    player.set_equalizer(equalizer.as_ref());
}

/// What plays now, for the whole app (tabs and windows share it).
pub(crate) struct Jukebox {
    pub(crate) queue: Vec<Track>,
    pub(crate) index: Option<usize>,
    player: Option<Player>,
    pub(crate) playing: bool,
    pub(crate) position: i64,
    pub(crate) length: i64,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    ticks: u32,
    /// Playing a Yandex station ("Моя волна"): more tracks come as it plays.
    radio: Option<Radio>,
    /// The OS media keys and "Now Playing", made on the first track.
    system: Option<crate::media_keys::SystemControls>,
    /// How much of a streamed track is downloaded (the grey bar).
    pub(crate) buffer: Option<std::sync::Arc<yandex::Progress>>,
    system_failed: bool,
}

#[derive(Clone)]
struct Radio {
    station: String,
    batch: String,
    fetching: bool,
}

struct JukeboxGlobal(Entity<Jukebox>);

impl Global for JukeboxGlobal {}

/// The app's player, made on first use.
pub(crate) fn jukebox(cx: &mut App) -> Entity<Jukebox> {
    if let Some(global) = cx.try_global::<JukeboxGlobal>() {
        return global.0.clone();
    }
    let entity = cx.new(|cx| {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |this: &mut Jukebox, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Jukebox { queue: Vec::new(), index: None, player: None, playing: false, position: 0, length: 0, loading: false, error: None, ticks: 0, radio: None, system: None, system_failed: false, buffer: None }
    });
    cx.set_global(JukeboxGlobal(entity.clone()));
    entity
}

impl Jukebox {
    pub(crate) fn current(&self) -> Option<&Track> {
        self.index.and_then(|index| self.queue.get(index))
    }

    fn is_spotify(&self) -> bool {
        matches!(self.current().map(|track| &track.source), Some(Source::Spotify(_)))
    }

    /// Plays `tracks` from `index` on.
    pub(crate) fn play_queue(&mut self, tracks: Vec<Track>, index: usize, cx: &mut Context<Self>) {
        self.radio = None;
        self.queue = tracks;
        self.start(index, cx);
    }

    pub(crate) fn radio_station(&self) -> Option<&str> {
        self.radio.as_ref().map(|radio| radio.station.as_str())
    }

    /// Starts a Yandex station: its first tracks, more as they play.
    pub(crate) fn play_wave(&mut self, station: String, cx: &mut Context<Self>) {
        self.player = None;
        self.queue.clear();
        self.index = None;
        self.loading = true;
        self.error = None;
        self.radio = Some(Radio { station: station.clone(), batch: String::new(), fetching: true });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let asked = station.clone();
            let first = cx.background_executor().spawn(async move { yandex::wave(&asked, None) }).await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match first {
                    Ok((tracks, batch)) if !tracks.is_empty() => {
                        let (started, name) = (batch.clone(), station.clone());
                        cx.background_executor().spawn(async move { yandex::wave_feedback(&name, "radioStarted", None, &started, 0) }).detach();
                        this.radio = Some(Radio { station, batch, fetching: false });
                        this.queue = tracks;
                        this.start(0, cx);
                    }
                    Ok(_) => this.fail("The station sent no tracks".to_string(), cx),
                    Err(error) => this.fail(error.to_string(), cx),
                }
            });
        })
        .detach();
    }

    /// A station near the end of what it sent: ask for the next tracks.
    fn top_up_radio(&mut self, cx: &mut Context<Self>) {
        let Some(radio) = self.radio.as_mut() else { return };
        let near_end = self.index.is_some_and(|index| index + 2 >= self.queue.len());
        if !near_end || radio.fetching {
            return;
        }
        radio.fetching = true;
        let station = radio.station.clone();
        let after = self.queue.last().and_then(|track| match &track.source {
            Source::Yandex(id) => Some(id.clone()),
            _ => None,
        });
        cx.spawn(async move |this, cx| {
            let more = cx.background_executor().spawn(async move { yandex::wave(&station, after.as_deref()) }).await;
            let _ = this.update(cx, |this, cx| {
                let Some(radio) = this.radio.as_mut() else { return };
                radio.fetching = false;
                if let Ok((tracks, batch)) = more {
                    radio.batch = batch;
                    // Never the same track twice in a row of batches.
                    let known: std::collections::HashSet<Source> = this.queue.iter().map(|track| track.source.clone()).collect();
                    this.queue.extend(tracks.into_iter().filter(|track| !known.contains(&track.source)));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Tells the station what is played or skipped.
    fn radio_feedback(&self, kind: &'static str, cx: &mut Context<Self>) {
        let Some(radio) = &self.radio else { return };
        let Some(Source::Yandex(id)) = self.current().map(|track| track.source.clone()) else { return };
        let (station, batch, seconds) = (radio.station.clone(), radio.batch.clone(), (self.position / 1000).max(0) as u64);
        cx.background_executor().spawn(async move { yandex::wave_feedback(&station, kind, Some(&id), &batch, seconds) }).detach();
    }

    /// Plays right after the current track (or now, when nothing plays).
    pub(crate) fn play_next(&mut self, track: Track, cx: &mut Context<Self>) {
        match self.index {
            Some(index) => {
                self.queue.insert(index + 1, track);
                cx.notify();
            }
            None => self.play_queue(vec![track], 0, cx),
        }
    }

    /// A click in the queue: play that track (a station keeps going).
    pub(crate) fn jump(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.queue.len() {
            self.radio_feedback("skip", cx);
            self.start(index, cx);
        }
    }

    pub(crate) fn enqueue(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        let start = self.index.is_none();
        self.queue.extend(tracks);
        if start && !self.queue.is_empty() {
            self.start(0, cx);
        }
        cx.notify();
    }

    fn start(&mut self, index: usize, cx: &mut Context<Self>) {
        self.player = None;
        self.buffer = None;
        self.index = Some(index);
        self.position = 0;
        self.error = None;
        let Some(track) = self.queue.get(index).cloned() else {
            self.index = None;
            self.playing = false;
            cx.notify();
            return;
        };
        files_sdk::log::record("music", files_sdk::log::Kind::Info, format!("Playing {} — {}", track.artist, track.title), None);
        self.announce(&track, cx);
        self.radio_feedback("trackStarted", cx);
        self.top_up_radio(cx);
        self.length = track.duration_ms as i64;
        match track.source.clone() {
            Source::Local(path) => self.open_file(&path, cx),
            // From an extension: streamed as it comes.
            Source::Url(url) => self.open_stream(&url, cx),
            Source::Yandex(id) if let Some(strength) = files_ext_music::streaming_enhance() => {
                // Settings → Music → Enhance streaming: enhanced while it
                // downloads (served locally as WAV), cached as FLAC after.
                let ready = yandex::enhanced_file(&id, strength);
                if ready.is_file() {
                    self.buffer = None;
                    crate::storage::touch(&ready);
                    self.open_file(&ready, cx);
                } else {
                    self.loading = true;
                    cx.spawn(async move |this, cx| {
                        let asked = id.clone();
                        let url = cx.background_executor().spawn(async move { yandex::stream(&asked) }).await;
                        let _ = this.update(cx, |this, cx| {
                            if this.index != Some(index) {
                                return;
                            }
                            this.loading = false;
                            let progress = std::sync::Arc::new(yandex::Progress::default());
                            match url.and_then(|url| files_ext_music::live::start(&id, &url, strength, progress.clone())) {
                                Ok(local) => {
                                    this.buffer = Some(progress);
                                    this.open_stream(&local, cx);
                                }
                                Err(error) => this.fail(error.to_string(), cx),
                            }
                        });
                    })
                    .detach();
                }
                self.prefetch_next(index);
            }
            Source::Yandex(id) => {
                let cached = yandex::cached_file(&id);
                if cached.is_file() {
                    self.buffer = None;
                    // Played again: kept longest under the cache limit.
                    crate::storage::touch(&cached);
                    self.open_file(&cached, cx);
                } else {
                    // Plays while it downloads: the stream starts at once,
                    // the cache fills behind it (the grey bar).
                    self.loading = true;
                    cx.spawn(async move |this, cx| {
                        let asked = id.clone();
                        let url = cx.background_executor().spawn(async move { yandex::stream(&asked) }).await;
                        let _ = this.update(cx, |this, cx| {
                            if this.index != Some(index) {
                                return;
                            }
                            this.loading = false;
                            match url {
                                Ok(url) => {
                                    let progress = std::sync::Arc::new(yandex::Progress::default());
                                    this.buffer = Some(progress.clone());
                                    let (id, address) = (id.clone(), url.clone());
                                    std::thread::spawn(move || {
                                        let _ = yandex::fetch(&id, &address, &progress);
                                    });
                                    this.open_stream(&url, cx);
                                }
                                Err(error) => this.fail(error.to_string(), cx),
                            }
                        });
                    })
                    .detach();
                }
                self.prefetch_next(index);
            }
            Source::Spotify(_) => {
                // Spotify plays the whole list in its app, from this track.
                let uris: Vec<String> = self
                    .queue
                    .iter()
                    .filter_map(|track| match &track.source {
                        Source::Spotify(uri) => Some(uri.clone()),
                        _ => None,
                    })
                    .collect();
                let at = match &track.source {
                    Source::Spotify(uri) => uris.iter().position(|queued| queued == uri).unwrap_or(0),
                    _ => 0,
                };
                self.loading = true;
                cx.spawn(async move |this, cx| {
                    let played = cx.background_executor().spawn(async move { spotify::play(&uris, at) }).await;
                    let _ = this.update(cx, |this, cx| {
                        this.loading = false;
                        match played {
                            Ok(()) => this.playing = true,
                            Err(error) => this.fail(error.to_string(), cx),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
        }
        cx.notify();
    }

    /// Tells the OS what plays, taking the media keys on the first track.
    fn announce(&mut self, track: &Track, cx: &mut Context<Self>) {
        if self.system.is_none() && !self.system_failed {
            let hwnd = crate::media_keys::hwnd(cx);
            match crate::media_keys::SystemControls::new(hwnd) {
                Ok(controls) => self.system = Some(controls),
                Err(error) => {
                    self.system_failed = true;
                    files_sdk::log::record("music", files_sdk::log::Kind::Error, format!("System media keys unavailable: {error}"), None);
                }
            }
        }
        let cover = cover_path(track, false);
        if let Some(system) = self.system.as_mut() {
            system.show(&track.title, &track.artist, &track.album, cover.as_deref(), track.duration_ms);
        }
    }

    /// A key pressed on the keyboard, headphones or in Control Center.
    fn obey(&mut self, command: crate::media_keys::Command, cx: &mut Context<Self>) {
        use crate::media_keys::Command;
        files_sdk::log::record("music", files_sdk::log::Kind::Setting, format!("media key: {command:?}"), None);
        match command {
            Command::Toggle => self.toggle(cx),
            Command::Play if !self.playing => self.toggle(cx),
            Command::Pause | Command::Stop if self.playing => self.toggle(cx),
            Command::Next => self.next(cx),
            Command::Previous => self.previous(cx),
            Command::SeekBy(seconds) if self.length > 0 => {
                let at = (self.position + seconds * 1000).clamp(0, self.length);
                self.seek(at as f32 / self.length as f32);
            }
            Command::SeekTo(at) if self.length > 0 => self.seek(at as f32 / self.length as f32),
            _ => {}
        }
    }

    fn open_stream(&mut self, url: &str, cx: &mut Context<Self>) {
        match Player::open_url(url) {
            Ok(player) => {
                apply_sound(&player);
                player.play();
                self.player = Some(player);
                self.playing = true;
            }
            Err(error) => self.fail(error, cx),
        }
        cx.notify();
    }

    /// Downloads the next Yandex track of the queue ahead (one at a time).
    fn prefetch_next(&self, index: usize) {
        static FETCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let Some(Source::Yandex(id)) = self.queue.get(index + 1).map(|track| track.source.clone()) else { return };
        let enhance = files_ext_music::streaming_enhance();
        let ready = match enhance {
            Some(strength) => yandex::enhanced_file(&id, strength),
            None => yandex::cached_file(&id),
        };
        if ready.is_file() || FETCHING.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(move || {
            // The next one ahead is made whole: it then plays from the cache.
            let _ = match enhance {
                Some(strength) => yandex::enhanced(&id, strength),
                None => yandex::stream(&id).and_then(|url| yandex::fetch(&id, &url, &yandex::Progress::default())),
            };
            FETCHING.store(false, std::sync::atomic::Ordering::SeqCst);
        });
    }

    /// 0…1 downloaded of what plays: 1 for files on disk, the stream's
    /// progress for a track still arriving.
    pub(crate) fn buffered(&self) -> f32 {
        match (&self.buffer, self.current().map(|track| &track.source)) {
            (_, Some(Source::Local(_))) => 1.,
            (Some(progress), Some(Source::Yandex(_))) => progress.fraction(),
            (None, Some(Source::Yandex(_))) => 1.,
            _ => 0.,
        }
    }

    fn open_file(&mut self, path: &Path, cx: &mut Context<Self>) {
        match Player::open(path) {
            Ok(player) => {
                apply_sound(&player);
                player.play();
                self.player = Some(player);
                self.playing = true;
            }
            Err(error) => self.fail(error, cx),
        }
        cx.notify();
    }

    fn fail(&mut self, error: String, cx: &mut Context<Self>) {
        files_sdk::log::record("music", files_sdk::log::Kind::Error, format!("Not played: {error}"), None);
        self.error = Some(error);
        self.playing = false;
        cx.notify();
    }

    pub(crate) fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.is_spotify() {
            let pause = self.playing;
            self.playing = !pause;
            cx.background_executor().spawn(async move { if pause { spotify::pause() } else { spotify::resume() } }).detach();
        } else if let Some(player) = &self.player {
            if self.playing {
                player.pause();
            } else {
                player.play();
            }
            self.playing = !self.playing;
        } else if let Some(index) = self.index {
            self.start(index, cx);
        }
        cx.notify();
    }

    /// Next: a skip, for a station.
    pub(crate) fn next(&mut self, cx: &mut Context<Self>) {
        if self.is_spotify() {
            cx.background_executor().spawn(async { spotify::next() }).detach();
            self.ticks = SPOTIFY_POLL - 2;
            return;
        }
        self.radio_feedback("skip", cx);
        self.advance(cx);
    }

    fn advance(&mut self, cx: &mut Context<Self>) {
        match self.index {
            Some(index) if index + 1 < self.queue.len() => self.start(index + 1, cx),
            _ => {
                self.player = None;
                self.playing = false;
                cx.notify();
            }
        }
    }

    pub(crate) fn previous(&mut self, cx: &mut Context<Self>) {
        if self.is_spotify() {
            cx.background_executor().spawn(async { spotify::previous() }).detach();
            self.ticks = SPOTIFY_POLL - 2;
            return;
        }
        // A few seconds in: back to the start, like every player.
        match (self.index, &self.player) {
            (Some(_), Some(player)) if self.position > 3000 => player.seek(0.),
            (Some(index), _) if index > 0 => self.start(index - 1, cx),
            _ => {}
        }
    }

    /// Volume or equalizer changed: the current track hears it at once.
    pub(crate) fn sound_changed(&self) {
        if let Some(player) = &self.player {
            apply_sound(player);
        }
    }

    /// Stops and empties the queue.
    pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
        self.player = None;
        self.queue.clear();
        self.index = None;
        self.playing = false;
        self.radio = None;
        cx.notify();
    }

    pub(crate) fn seek(&mut self, fraction: f32) {
        if let Some(player) = &self.player {
            player.seek(fraction);
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        self.ticks += 1;
        let commands: Vec<_> = self.system.as_ref().map(|system| system.commands.try_iter().collect()).unwrap_or_default();
        for command in commands {
            self.obey(command, cx);
        }
        // The OS shows the time and the play/pause state (every second is enough).
        if self.ticks % 4 == 0
            && let Some(system) = self.system.as_mut()
        {
            let state = self.index.map(|_| self.playing);
            system.state(state, self.position.max(0) as u64);
        }
        if let Some(player) = &self.player {
            let (position, length) = player.progress();
            if length > 0 {
                self.length = length;
            }
            // Redraw while the grey buffer bar is still growing.
            let filling = self.buffer.as_ref().is_some_and(|progress| progress.fraction() < 1.);
            if position != self.position || filling {
                self.position = position;
                cx.notify();
            }
            if player.ended() {
                self.radio_feedback("trackFinished", cx);
                self.advance(cx);
            }
        } else if self.is_spotify() && self.ticks % SPOTIFY_POLL == 0 {
            cx.spawn(async move |this, cx| {
                let state = cx.background_executor().spawn(async { spotify::playback() }).await;
                let _ = this.update(cx, |this, cx| {
                    if let Ok(state) = state {
                        this.playing = state.playing;
                        this.position = state.progress_ms as i64;
                        // Spotify moved on by itself: show what it plays.
                        if let Some(track) = state.track
                            && this.current() != Some(&track)
                        {
                            this.length = track.duration_ms as i64;
                            match this.queue.iter().position(|queued| queued.source == track.source) {
                                Some(index) => this.index = Some(index),
                                None => {
                                    this.queue.push(track);
                                    this.index = Some(this.queue.len() - 1);
                                }
                            }
                        }
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }
}

/// Audio files in `path`'s folder, in name order, and where `path` is.
pub(crate) fn folder_queue(path: &Path) -> (Vec<Track>, usize) {
    let folder = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|file| local::is_audio(file))
        .collect();
    files.sort_by_key(|file| file.file_name().map(|name| name.to_string_lossy().to_lowercase()));
    let index = files.iter().position(|file| file == path).unwrap_or(0);
    (files.iter().map(|file| local::track(file)).collect(), index)
}

// ------------------------------------------------------------- the Music tab

#[derive(Clone, Debug, PartialEq)]
enum Shelf {
    Library,
    SpotifyLiked,
    SpotifyPlaylist(String),
    SpotifySearch,
    YandexLiked,
    YandexWave,
    YandexPlaylist(String),
    YandexSearch,
    /// An artist's tracks: service, id, name.
    Artist(Service, String, String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Service {
    Spotify,
    Yandex,
}

/// What a track's context menu does.
#[derive(Clone, Debug, PartialEq)]
enum TrackAction {
    PlayNext,
    AddToQueue,
    ShowInFiles,
    Like(bool),
    Dislike,
    AddToPlaylist(String),
    GoToArtist,
    ArtistRadio,
    Download,
}

gpui_kit::actions!(music, [MusicUp, MusicDown, MusicHome, MusicEnd, MusicPageUp, MusicPageDown, MusicPlaySelected, MusicPreview]);

/// Rows a Page Up / Page Down moves.
const PAGE: usize = 12;

use gpui_kit::component::WindowExt as _;

pub(crate) struct MusicView {
    /// The track the keyboard is on (↑↓, Home/End, PgUp/PgDn; Enter plays).
    cursor: Option<usize>,
    list_scroll: gpui_kit::UniformListScrollHandle,
    /// The track preview's own focus: Space there closes it again.
    preview_focus: FocusHandle,
    appearance: Entity<AppearanceSettings>,
    /// The file browser, for Show in Files.
    explorer: gpui_kit::WeakEntity<FileExplorer>,
    /// Spotify playlists this account can add to (loaded with the others).
    spotify_editable: Vec<Playlist>,
    jukebox: Entity<Jukebox>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
    shelf: Shelf,
    library: Option<Vec<Track>>,
    tracks: Vec<Track>,
    search: Entity<InputState>,
    yandex_token: Entity<InputState>,
    seek: Entity<SliderState>,
    seeking: bool,
    volume: Entity<SliderState>,
    show_equalizer: bool,
    /// The shared equalizer (also in Settings → Sound).
    equalizer: Entity<crate::views::equalizer::EqualizerPanel>,
    spotify_playlists: Vec<Playlist>,
    yandex_playlists: Vec<Playlist>,
    busy: bool,
    notice: Option<String>,
    generation: u64,
}

impl Focusable for MusicView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl MusicView {
    #[cfg(test)]
    pub(crate) fn set_tracks_for_test(&mut self, tracks: Vec<Track>) {
        self.tracks = tracks;
        self.shelf = Shelf::Library;
    }

    #[cfg(test)]
    pub(crate) fn cursor_for_test(&self) -> Option<usize> {
        self.cursor
    }

    /// Moves the keyboard's track by `delta` (or to an end) and keeps it on screen.
    fn move_cursor(&mut self, to: impl FnOnce(Option<usize>, usize) -> usize, cx: &mut Context<Self>) {
        let count = self.shown_tracks(cx).len();
        if count == 0 {
            return;
        }
        let next = to(self.cursor, count).min(count - 1);
        self.cursor = Some(next);
        self.list_scroll.scroll_to_item(next, gpui_kit::ScrollStrategy::Nearest);
        cx.notify();
    }

    fn on_up(&mut self, _: &MusicUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(|cursor, _| cursor.map_or(0, |at| at.saturating_sub(1)), cx);
    }

    fn on_down(&mut self, _: &MusicDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(|cursor, _| cursor.map_or(0, |at| at + 1), cx);
    }

    fn on_home(&mut self, _: &MusicHome, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(|_, _| 0, cx);
    }

    fn on_end(&mut self, _: &MusicEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(|_, count| count - 1, cx);
    }

    fn on_page_up(&mut self, _: &MusicPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(|cursor, _| cursor.unwrap_or(0).saturating_sub(PAGE), cx);
    }

    fn on_page_down(&mut self, _: &MusicPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(|cursor, _| cursor.map_or(0, |at| at + PAGE), cx);
    }

    /// Enter: the keyboard's track plays.
    fn on_play_selected(&mut self, _: &MusicPlaySelected, _: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.cursor else { return };
        if self.shelf == Shelf::YandexWave {
            self.jukebox.update(cx, |jukebox, cx| jukebox.jump(index, cx));
        } else {
            self.play(index, cx);
        }
    }

    /// Space: the track big — cover, title, artist, album, length.
    fn on_preview(&mut self, _: &MusicPreview, window: &mut Window, cx: &mut Context<Self>) {
        let Some(track) = self.cursor.and_then(|index| self.shown_tracks(cx).get(index).cloned()) else { return };
        let palette = Palette::from_settings(self.appearance.read(cx));
        let index = self.cursor.unwrap_or(0);
        let view = cx.entity().downgrade();
        let jukebox = self.jukebox.clone();
        let focus = self.preview_focus.clone();
        let dialog_focus = focus.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let view = view.clone();
            let jukebox = jukebox.clone();
            // The shown track already playing: Pause (and Play again), not a restart.
            let state = jukebox.read(cx);
            let current = state.current().is_some_and(|playing| playing.source == track.source);
            let playing = current && state.playing;
            dialog.w(px(520.)).centered().overlay(true).overlay_closable(true).close_button(false).child(
                v_flex()
                    .track_focus(&dialog_focus)
                    .on_key_down(|event, window, cx| {
                        if event.keystroke.key == "space" {
                            window.close_dialog(cx);
                            cx.stop_propagation();
                        }
                    })
                    .gap_3()
                    .items_center()
                    .p_4()
                    .child(cover_thumb(&track, 320., palette))
                    .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).text_center().child(track.title.clone()))
                    .child(div().text_sm().text_color(palette.muted).text_center().child(format!("{} · {}", track.artist, track.album)))
                    .child(div().text_xs().text_color(palette.faint).child(format!("{} · Space or Esc closes", track.duration())))
                    .child(
                        Button::new("music-preview-play")
                            .primary()
                            .small()
                            .icon(if playing { IconName::Pause } else { IconName::Play })
                            .label(if playing { "Pause" } else { "Play" })
                            .on_click(move |_, window, cx| {
                        if current {
                            jukebox.update(cx, |jukebox, cx| jukebox.toggle(cx));
                            return;
                        }
                        window.close_dialog(cx);
                        let _ = view.update(cx, |this, cx| {
                            this.cursor = Some(index);
                            this.on_play_selected(&MusicPlaySelected, window, cx);
                        });
                    })),
            )
        });
        focus.focus(window, cx);
    }

    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>, appearance: Entity<AppearanceSettings>, explorer: gpui_kit::WeakEntity<FileExplorer>) -> Self {
        let jukebox = jukebox(cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search the library, Spotify or Yandex Music · Enter"));
        let yandex_token = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("Token, or the whole address the Yandex page ended on"));
        let seek = cx.new(|_| SliderState::new().min(0.).max(1.).step(0.001));
        let config = files_ext_music::config();
        let volume = cx.new(|_| SliderState::new().min(0.).max(100.).step(1.).default_value(config.volume as f32));
        let equalizer = {
            let appearance = appearance.clone();
            cx.new(|cx| crate::views::equalizer::EqualizerPanel::new(appearance, cx))
        };
        let mut subscriptions = vec![
            cx.observe(&appearance, |_, _, cx| cx.notify()),
            cx.observe(&jukebox, |_, _, cx| cx.notify()),
            cx.subscribe(&search, |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::PressEnter { .. } => this.run_search(cx),
                // The library filters as you type; services search on Enter.
                InputEvent::Change if this.shelf == Shelf::Library => this.show_library(cx),
                _ => {}
            }),
            cx.subscribe(&seek, |this: &mut Self, _, event: &SliderEvent, cx| match event {
                SliderEvent::Change(_) => this.seeking = true,
                SliderEvent::Release(value) => {
                    this.seeking = false;
                    let fraction = value.start();
                    this.jukebox.update(cx, |jukebox, _| jukebox.seek(fraction));
                }
            }),
        ];
        subscriptions.push(cx.subscribe(&volume, |this: &mut Self, _, event: &SliderEvent, cx| {
            if let SliderEvent::Release(value) = event {
                let mut config = files_ext_music::config();
                config.volume = value.end().round() as u8;
                let _ = files_ext_music::save_config(&config);
                this.jukebox.read(cx).sound_changed();
                cx.notify();
            }
        }));

        if let Some(files) = explorer.upgrade() {
            // The title bar's inspector button opens this tab's track panel too.
            subscriptions.push(cx.observe(&files, |_, _, cx| cx.notify()));
        }
        let mut this = Self {
            appearance,
            explorer,
            spotify_editable: Vec::new(),
            jukebox,
            focus_handle: cx.focus_handle(),
            cursor: None,
            list_scroll: gpui_kit::UniformListScrollHandle::new(),
            preview_focus: cx.focus_handle(),
            _subscriptions: subscriptions,
            shelf: Shelf::Library,
            library: None,
            tracks: Vec::new(),
            search,
            yandex_token,
            seek,
            seeking: false,
            volume,
            show_equalizer: false,
            equalizer,
            spotify_playlists: Vec::new(),
            yandex_playlists: Vec::new(),
            busy: false,
            notice: None,
            generation: 0,
        };
        this.load_library(cx);
        this.load_playlists(cx);
        // New covers in: draw them (checked a few times a second, cheaply).
        cx.spawn(async move |this, cx| {
            let mut seen = 0;
            loop {
                cx.background_executor().timer(Duration::from_millis(400)).await;
                let now = COVERS.load(std::sync::atomic::Ordering::Relaxed);
                if now != seen {
                    seen = now;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                } else if this.update(cx, |_, _| ()).is_err() {
                    break;
                }
            }
        })
        .detach();
        this
    }

    fn load_library(&mut self, cx: &mut Context<Self>) {
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let tracks = cx.background_executor().spawn(async { local::rescan() }).await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.library = Some(tracks);
                if this.shelf == Shelf::Library {
                    this.show_library(cx);
                }
            });
        })
        .detach();
    }

    fn load_playlists(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let lists = cx
                .background_executor()
                .spawn(async {
                    let spotify = if spotify::connected() { spotify::playlists().unwrap_or_default() } else { Vec::new() };
                    let editable = if spotify::connected() { spotify::editable_playlists().unwrap_or_default() } else { Vec::new() };
                    let yandex = if yandex::connected() { yandex::playlists().unwrap_or_default() } else { Vec::new() };
                    (spotify, editable, yandex)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                (this.spotify_playlists, this.spotify_editable, this.yandex_playlists) = lists;
                cx.notify();
            });
        })
        .detach();
    }

    fn show_library(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().to_string();
        self.tracks = self.library.as_deref().map(|library| local::filter(library, &query).into_iter().cloned().collect()).unwrap_or_default();
        cx.notify();
    }

    /// Loads a shelf from its service in the background.
    fn open_shelf(&mut self, shelf: Shelf, cx: &mut Context<Self>) {
        self.shelf = shelf.clone();
        self.notice = None;
        if shelf == Shelf::Library {
            self.show_library(cx);
            return;
        }
        if shelf == Shelf::YandexWave {
            // A station is not a list: it plays, and the queue shows what comes.
            self.jukebox.update(cx, |jukebox, cx| jukebox.play_wave(yandex::MY_WAVE.to_string(), cx));
            cx.notify();
            return;
        }
        let query = self.search.read(cx).value().to_string();
        self.generation += 1;
        let generation = self.generation;
        self.busy = true;
        self.tracks.clear();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    match shelf {
                        Shelf::Library => Ok(Vec::new()),
                        Shelf::SpotifyLiked => spotify::liked(),
                        Shelf::SpotifyPlaylist(id) => spotify::playlist_tracks(&id),
                        Shelf::SpotifySearch => spotify::search(&query),
                        Shelf::YandexLiked => yandex::liked(),
                        Shelf::YandexWave => Ok(Vec::new()),
                        Shelf::YandexPlaylist(kind) => yandex::playlist_tracks(&kind),
                        Shelf::YandexSearch => yandex::search(&query),
                        Shelf::Artist(Service::Spotify, id, _) => spotify::artist(&id).map(|(_, tracks)| tracks),
                        Shelf::Artist(Service::Yandex, id, _) => yandex::artist(&id).map(|(_, tracks)| tracks),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match loaded {
                    Ok(tracks) => {
                        this.notice = tracks.is_empty().then(|| "Nothing here".to_string());
                        this.tracks = tracks;
                    }
                    Err(error) => this.notice = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Enter: the library filters; Spotify and Yandex shelves search there.
    fn run_search(&mut self, cx: &mut Context<Self>) {
        match self.shelf {
            Shelf::Library => self.show_library(cx),
            Shelf::SpotifyLiked | Shelf::SpotifyPlaylist(_) | Shelf::SpotifySearch => self.open_shelf(Shelf::SpotifySearch, cx),
            Shelf::YandexLiked | Shelf::YandexWave | Shelf::YandexPlaylist(_) | Shelf::YandexSearch => self.open_shelf(Shelf::YandexSearch, cx),
            Shelf::Artist(Service::Spotify, ..) => self.open_shelf(Shelf::SpotifySearch, cx),
            Shelf::Artist(Service::Yandex, ..) => self.open_shelf(Shelf::YandexSearch, cx),
        }
    }

    fn play(&mut self, index: usize, cx: &mut Context<Self>) {
        let tracks = self.tracks.clone();
        self.jukebox.update(cx, |jukebox, cx| jukebox.play_queue(tracks, index, cx));
    }

    /// The right panel: the track that plays (or the first one listed).
    fn render_track_panel(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let open = self.explorer.upgrade().is_some_and(|files| files.read(cx).show_inspector);
        if !open {
            return None;
        }
        let track = self.jukebox.read(cx).current().cloned().or_else(|| self.shown_tracks(cx).first().cloned())?;
        let service = match &track.source {
            Source::Local(path) => path.display().to_string(),
            Source::Spotify(_) => "Spotify · plays in the Spotify app".to_string(),
            Source::Yandex(id) if yandex::cached_file(id).is_file() => "Yandex Music · downloaded".to_string(),
            Source::Yandex(_) => "Yandex Music · streamed".to_string(),
            Source::Url(url) => url.clone(),
        };
        let row = |label: &'static str, value: String| {
            h_flex()
                .gap_2()
                .text_xs()
                .child(div().w(px(56.)).flex_shrink_0().text_color(palette.faint).child(label))
                .child(div().flex_1().min_w_0().text_color(palette.text).whitespace_normal().child(value))
        };
        let action = |id: &'static str, icon: IconName, label: &'static str, what: TrackAction, cx: &mut Context<Self>| {
            let track = track.clone();
            Button::new(id).ghost().xsmall().icon(icon).label(label).on_click(cx.listener(move |this, _, window, cx| this.act(track.clone(), what.clone(), window, cx)))
        };
        let streaming = !matches!(track.source, Source::Local(_));
        let yandex_track = matches!(track.source, Source::Yandex(_));
        Some(
            v_flex()
                .id("music-track-panel")
                .w(px(280.))
                .flex_shrink_0()
                .h_full()
                .p_4()
                .gap_3()
                .border_l_1()
                .border_color(palette.border)
                .bg(palette.sidebar)
                .overflow_y_scrollbar()
                .child(div().flex().justify_center().child(cover_thumb(&track, 220., palette)))
                .child(div().text_base().font_weight(FontWeight::SEMIBOLD).whitespace_normal().child(track.title.clone()))
                .child(row("Artist", track.artist.clone()))
                .child(row("Album", track.album.clone()))
                .child(row("Length", track.duration()))
                .child(row("From", service))
                .child(
                    v_flex()
                        .gap_0p5()
                        .when(!streaming, |this| this.child(action("music-panel-show", IconName::FolderOpen, "Show in Files", TrackAction::ShowInFiles, cx)))
                        .when(streaming, |this| {
                            this.child(action("music-panel-like", IconName::Heart, "Like", TrackAction::Like(true), cx))
                                .child(action("music-panel-artist", IconName::User, "Go to Artist", TrackAction::GoToArtist, cx))
                        })
                        .when(yandex_track, |this| this.child(action("music-panel-download", IconName::Download, "Download", TrackAction::Download, cx))),
                )
                .into_any_element(),
        )
    }

    fn act(&mut self, track: Track, action: TrackAction, window: &mut Window, cx: &mut Context<Self>) {
        let service = match &track.source {
            Source::Spotify(_) => Some(Service::Spotify),
            Source::Yandex(_) => Some(Service::Yandex),
            Source::Local(_) | Source::Url(_) => None,
        };
        match action {
            TrackAction::PlayNext => self.jukebox.update(cx, |jukebox, cx| jukebox.play_next(track, cx)),
            TrackAction::AddToQueue => self.jukebox.update(cx, |jukebox, cx| jukebox.enqueue(vec![track], cx)),
            TrackAction::ShowInFiles => {
                if let Source::Local(path) = &track.source {
                    let path = path.clone();
                    let _ = self.explorer.update(cx, |explorer, cx| explorer.reveal_path(&path, window, cx));
                }
            }
            TrackAction::GoToArtist => match (service, track.artist_id.clone()) {
                (Some(service), Some(id)) => {
                    let name = track.artist.split(", ").next().unwrap_or_default().to_string();
                    self.open_shelf(Shelf::Artist(service, id, name), cx);
                }
                _ => self.notice = Some("The service did not say who the artist is".to_string()),
            },
            TrackAction::ArtistRadio => match (service, track.artist_id.clone()) {
                (Some(Service::Yandex), Some(id)) => self.jukebox.update(cx, |jukebox, cx| jukebox.play_wave(yandex::artist_station(&id), cx)),
                // Spotify closed its radio API to new apps: the artist's top tracks instead.
                (Some(Service::Spotify), Some(id)) => {
                    let name = track.artist.split(", ").next().unwrap_or_default().to_string();
                    self.open_shelf(Shelf::Artist(Service::Spotify, id, name), cx);
                    self.notice = Some("Spotify has no artist radio for apps: its top tracks are listed, play the first".to_string());
                }
                _ => {}
            },
            remote => self.remote_action(track, remote, cx),
        }
        cx.notify();
    }

    /// The actions that ask the service (in the background).
    fn remote_action(&mut self, track: Track, action: TrackAction, cx: &mut Context<Self>) {
        let title = track.title.clone();
        let (disliked, current_title) = (action == TrackAction::Dislike, title.clone());
        self.notice = Some(format!("{title}…"));
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_executor()
                .spawn({
                    let action = action.clone();
                    async move {
                        let spotify = matches!(track.source, Source::Spotify(_));
                        match action {
                            TrackAction::Like(liked) if spotify => spotify::set_liked(&track, liked).map(|()| String::new()),
                            TrackAction::Like(liked) => yandex::set_liked(&track, liked).map(|()| String::new()),
                            TrackAction::Dislike => yandex::dislike(&track).map(|()| String::new()),
                            TrackAction::AddToPlaylist(id) if spotify => spotify::add_to_playlist(&id, &track).map(|()| String::new()),
                            TrackAction::AddToPlaylist(kind) => yandex::add_to_playlist(&kind, &track).map(|()| String::new()),
                            TrackAction::Download => {
                                let folder = dirs::download_dir().ok_or_else(|| std::io::Error::other("No Downloads folder"))?;
                                yandex::save_to(&track, &folder).map(|path| path.display().to_string())
                            }
                            _ => Ok(String::new()),
                        }
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.notice = Some(match (&done, &action) {
                    (Ok(_), TrackAction::Like(true)) => format!("Liked: {title}"),
                    (Ok(_), TrackAction::Like(false)) => format!("Removed from liked: {title}"),
                    (Ok(_), TrackAction::Dislike) => format!("Disliked: {title} (Yandex will not suggest it)"),
                    (Ok(_), TrackAction::AddToPlaylist(_)) => format!("Added to the playlist: {title}"),
                    (Ok(path), TrackAction::Download) => {
                        files_sdk::log::notify("music", files_sdk::log::Level::Success, format!("Downloaded {title}"), Some(path.clone()));
                        format!("Saved to {path}")
                    }
                    (Ok(_), _) => String::new(),
                    (Err(error), _) => error.to_string(),
                });
                cx.notify();
            });
        })
        .detach();
        // Disliking what plays: on to the next one.
        if disliked {
            self.jukebox.update(cx, |jukebox, cx| {
                if jukebox.current().is_some_and(|current| current.title == current_title) {
                    jukebox.next(cx);
                }
            });
        }
    }

    fn connect_spotify(&mut self, cx: &mut Context<Self>) {
        self.busy = true;
        self.notice = Some("Sign in to Spotify in the browser…".to_string());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let signed_in = cx.background_executor().spawn(async { spotify::login(open_in_browser).and_then(|()| spotify::profile_name()) }).await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match signed_in {
                    Ok(name) => {
                        this.notice = Some(format!("Spotify: signed in as {name}"));
                        this.load_playlists(cx);
                        this.open_shelf(Shelf::SpotifyLiked, cx);
                    }
                    Err(error) => this.notice = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn connect_yandex(&mut self, cx: &mut Context<Self>) {
        let pasted = self.yandex_token.read(cx).value().to_string();
        self.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let connected = cx.background_executor().spawn(async move { yandex::connect(&pasted) }).await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match connected {
                    Ok(name) => {
                        this.notice = Some(format!("Yandex Music: signed in as {name}"));
                        this.load_playlists(cx);
                        this.open_shelf(Shelf::YandexLiked, cx);
                    }
                    Err(error) => this.notice = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn shelf_row(&self, id: String, icon: IconName, label: String, shelf: Shelf, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.shelf == shelf;
        let selector = id.clone();
        h_flex()
            .id(gpui_kit::ElementId::Name(id.into()))
            .debug_selector(move || selector.clone())
            .px_2()
            .h(px(30.))
            .gap_2()
            .items_center()
            .rounded_md()
            .text_sm()
            .text_color(if selected { palette.text } else { palette.muted })
            .bg(if selected { palette.selection } else { palette.sidebar })
            .hover(|style| style.bg(palette.hover).text_color(palette.text))
            .child(Icon::new(icon).small().text_color(if selected { palette.accent } else { palette.muted }))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .on_click(cx.listener(move |this, _, _, cx| this.open_shelf(shelf.clone(), cx)))
            .into_any_element()
    }

    fn render_sources(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let collapsed = files_ext_music::config().collapsed;
        let open = |group: &str| !collapsed.iter().any(|folded| folded == group);
        // A group title folds and unfolds what is under it.
        let heading = |group: &'static str, text: &'static str, cx: &mut Context<Self>| {
            let folded = !open(group);
            h_flex()
                .id(gpui_kit::ElementId::Name(format!("music-group-{group}").into()))
                .px_2()
                .pt_3()
                .pb_1()
                .gap_1()
                .items_center()
                .cursor_pointer()
                .text_size(px(10.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette.faint)
                .hover(|style| style.text_color(palette.muted))
                .debug_selector(move || format!("music-group-{group}"))
                .child(Icon::new(if folded { IconName::ChevronRight } else { IconName::ChevronDown }).xsmall())
                .child(text)
                .on_click(cx.listener(move |_, _, _, cx| {
                    let mut config = files_ext_music::config();
                    if config.collapsed.iter().any(|known| known == group) {
                        config.collapsed.retain(|known| known != group);
                    } else {
                        config.collapsed.push(group.to_string());
                    }
                    let _ = files_ext_music::save_config(&config);
                    cx.notify();
                }))
        };
        let library_count = self.library.as_ref().map_or_else(|| "…".to_string(), |library| library.len().to_string());
        let mut column = v_flex()
            .id("music-sources")
            .w(px(220.))
            .flex_shrink_0()
            .h_full()
            .p_2()
            .gap_0p5()
            .bg(palette.sidebar)
            .border_r_1()
            .border_color(palette.border)
            .overflow_y_scrollbar()
            .child(heading("local", "THIS COMPUTER", cx))
            .when(open("local"), |this| {
                this.child(self.shelf_row("music-library".into(), IconName::Music, format!("Library · {library_count}"), Shelf::Library, palette, cx))
            })
            .child(heading("spotify", "SPOTIFY", cx));
        if !open("spotify") {
        } else if spotify::connected() {
            column = column.child(self.shelf_row("music-spotify-liked".into(), IconName::Heart, "Liked Songs".into(), Shelf::SpotifyLiked, palette, cx));
            for playlist in &self.spotify_playlists {
                column = column.child(self.shelf_row(
                    format!("music-spotify-{}", playlist.id),
                    IconName::ListMusic,
                    playlist.name.clone(),
                    Shelf::SpotifyPlaylist(playlist.id.clone()),
                    palette,
                    cx,
                ));
            }
        } else if spotify::configured() {
            column = column.child(
                div().px_2().child(
                    Button::new("music-spotify-connect")
                        .small()
                        .icon(IconName::LogIn)
                        .label("Connect Spotify")
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.connect_spotify(cx))),
                ),
            );
        } else {
            column = column.child(div().px_2().text_xs().text_color(palette.faint).child("This build has no Spotify client id yet."));
        }
        column = column.child(heading("yandex", "YANDEX MUSIC", cx));
        if !open("yandex") {
        } else if yandex::connected() {
            column = column
                .child(self.shelf_row("music-yandex-wave".into(), IconName::Radio, "Моя волна".into(), Shelf::YandexWave, palette, cx))
                .child(self.shelf_row("music-yandex-liked".into(), IconName::Heart, "Мне нравится".into(), Shelf::YandexLiked, palette, cx));
            for playlist in &self.yandex_playlists {
                column = column.child(self.shelf_row(
                    format!("music-yandex-{}", playlist.id),
                    IconName::ListMusic,
                    playlist.name.clone(),
                    Shelf::YandexPlaylist(playlist.id.clone()),
                    palette,
                    cx,
                ));
            }
        } else {
            column = column.child(
                v_flex()
                    .px_2()
                    .gap_1()
                    .child(Button::new("music-yandex-token").link().xsmall().label("1. Get a token in the browser").on_click(|_, _, _| open_in_browser(yandex::TOKEN_PAGE)))
                    .child(Input::new(&self.yandex_token).small())
                    .child(
                        Button::new("music-yandex-connect")
                            .small()
                            .icon(IconName::LogIn)
                            .label("2. Connect")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.connect_yandex(cx))),
                    )
                    .child(div().text_xs().text_color(palette.faint).child("Unofficial API: it may stop working.")),
            );
        }
        column.into_any_element()
    }

    /// The list shown: the shelf's tracks, or the station's queue for My Wave.
    fn shown_tracks<'a>(&'a self, cx: &'a App) -> &'a [Track] {
        if self.shelf == Shelf::YandexWave { &self.jukebox.read(cx).queue } else { &self.tracks }
    }

    fn track_row(&self, index: usize, track: &Track, current: bool, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let wave = self.shelf == Shelf::YandexWave;
        h_flex()
            .id(("music-track", index))
            // A virtual list sizes rows by their content: span the whole list.
            .w_full()
            .debug_selector(move || format!("music-track-{index}"))
            .px_3()
            .h(px(44.))
            .gap_3()
            .items_center()
            .border_b_1()
            .border_color(palette.border)
            .bg(if current { palette.selection } else if self.cursor == Some(index) { palette.hover } else { palette.window })
            .when(self.cursor == Some(index), |row| row.border_l_2().border_color(palette.accent))
            .hover(|style| style.bg(palette.hover))
            .child(div().w(px(24.)).text_xs().text_color(palette.faint).child(if current { "▶".to_string() } else { (index + 1).to_string() }))
            .child(cover_thumb(track, 32., palette))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().text_sm().text_color(if current { palette.accent } else { palette.text }).child(track.title.clone()))
                    .child(div().truncate().text_xs().text_color(palette.muted).child(track.artist.clone())),
            )
            .child(div().w(px(200.)).truncate().text_xs().text_color(palette.muted).child(track.album.clone()))
            .child(div().w(px(48.)).text_right().text_xs().text_color(palette.faint).child(track.duration()))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.cursor = Some(index);
                window.focus(&this.focus_handle, cx);
                if wave {
                    this.jukebox.update(cx, |jukebox, cx| jukebox.jump(index, cx));
                } else {
                    this.play(index, cx);
                }
            }))
            .context_menu({
                let view = cx.entity();
                let track = track.clone();
                let playlists = match &track.source {
                    Source::Spotify(_) => self.spotify_editable.clone(),
                    Source::Yandex(_) => self.yandex_playlists.clone(),
                    Source::Local(_) | Source::Url(_) => Vec::new(),
                };
                move |menu, window, cx| {
                    let on = |action: TrackAction| {
                        let (track, view) = (track.clone(), view.clone());
                        move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                            view.update(cx, |this, cx| this.act(track.clone(), action.clone(), window, cx));
                        }
                    };
                    let streaming = !matches!(track.source, Source::Local(_));
                    let yandex = matches!(track.source, Source::Yandex(_));
                    let mut menu = menu
                        .item(PopupMenuItem::new("Play Next").icon(IconName::ListStart).on_click(on(TrackAction::PlayNext)))
                        .item(PopupMenuItem::new("Add to Queue").icon(IconName::ListMusic).on_click(on(TrackAction::AddToQueue)));
                    if !streaming {
                        return menu.separator().item(PopupMenuItem::new("Show in Files").icon(IconName::FolderOpen).on_click(on(TrackAction::ShowInFiles)));
                    }
                    menu = menu
                        .separator()
                        .item(PopupMenuItem::new("Like").icon(IconName::Heart).on_click(on(TrackAction::Like(true))))
                        .item(PopupMenuItem::new("Remove from Liked").icon(IconName::HeartOff).on_click(on(TrackAction::Like(false))));
                    if yandex {
                        menu = menu.item(PopupMenuItem::new("Dislike").icon(IconName::ThumbsDown).on_click(on(TrackAction::Dislike)));
                    }
                    let (lists, sub_view, sub_track) = (playlists.clone(), view.clone(), track.clone());
                    menu = menu.submenu_with_icon(Some(Icon::new(IconName::ListPlus)), "Add to Playlist", window, cx, move |submenu, window, _| {
                        if lists.is_empty() {
                            return submenu.item(PopupMenuItem::new("No playlists you can edit").disabled(true));
                        }
                        lists.iter().fold(submenu, |submenu, playlist| {
                            let (track, id) = (sub_track.clone(), playlist.id.clone());
                            submenu.item(PopupMenuItem::new(playlist.name.clone()).on_click(window.listener_for(
                                &sub_view,
                                move |this: &mut MusicView, _, window, cx| this.act(track.clone(), TrackAction::AddToPlaylist(id.clone()), window, cx),
                            )))
                        })
                    });
                    menu = menu
                        .separator()
                        .item(PopupMenuItem::new("Go to Artist").icon(IconName::User).disabled(track.artist_id.is_none()).on_click(on(TrackAction::GoToArtist)))
                        .item(
                            PopupMenuItem::new(if yandex { "Artist Radio" } else { "Artist's Top Tracks" })
                                .icon(IconName::Radio)
                                .disabled(track.artist_id.is_none())
                                .on_click(on(TrackAction::ArtistRadio)),
                        );
                    if yandex {
                        menu = menu.separator().item(PopupMenuItem::new("Download").icon(IconName::Download).on_click(on(TrackAction::Download)));
                    }
                    menu
                }
            })
            .into_any_element()
    }

    /// Only the rows on screen are built: libraries have thousands.
    fn render_tracks(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let count = self.shown_tracks(cx).len();
        if count == 0 {
            let wave_loading = self.shelf == Shelf::YandexWave && self.jukebox.read(cx).loading;
            let text = if self.busy || wave_loading {
                "Loading…".to_string()
            } else {
                self.notice.clone().unwrap_or_else(|| match self.shelf {
                    Shelf::Library => "No music found in the library folders (Settings → Extensions → Music).".to_string(),
                    _ => "Nothing here".to_string(),
                })
            };
            return div().flex_1().p_6().text_sm().text_color(palette.faint).child(text).into_any_element();
        }
        gpui_kit::uniform_list(
            "music-tracks",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                let palette = Palette::from_settings(this.appearance.read(cx));
                let playing = this.jukebox.read(cx).current().map(|track| track.source.clone());
                let tracks: Vec<(usize, Track)> = this.shown_tracks(cx).iter().enumerate().skip(range.start).take(range.len()).map(|(index, track)| (index, track.clone())).collect();
                tracks
                    .into_iter()
                    .map(|(index, track)| {
                        let current = playing.as_ref() == Some(&track.source);
                        this.track_row(index, &track, current, palette, cx)
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.list_scroll)
        .flex_1()
        .w_full()
        .min_w_0()
        .h_full()
        .into_any_element()
    }

    fn render_now_playing(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let jukebox = self.jukebox.read(cx);
        let title = jukebox.current().map_or_else(|| "Nothing playing".to_string(), |track| track.title.clone());
        let artist = jukebox.current().map(|track| track.artist.clone()).unwrap_or_default();
        let remote = jukebox.current().is_some_and(|track| matches!(track.source, Source::Spotify(_)));
        let wave = jukebox.radio_station().is_some();
        let status = match (&jukebox.error, jukebox.loading, remote) {
            (Some(error), _, _) => error.clone(),
            (None, true, _) => "Loading…".to_string(),
            (None, false, true) => "Playing in Spotify".to_string(),
            _ if wave => {
                let station = if jukebox.radio_station() == Some(yandex::MY_WAVE) { "Моя волна" } else { "Радио" };
                format!("{station} · {} / {}", clock(jukebox.position), clock(jukebox.length))
            }
            _ => format!("{} / {}", clock(jukebox.position), clock(jukebox.length)),
        };
        let has_track = jukebox.current().is_some();
        let playing = jukebox.playing;
        h_flex()
            .flex_shrink_0()
            .h(px(64.))
            .px_4()
            .gap_4()
            .items_center()
            .border_t_1()
            .border_color(palette.border)
            .bg(palette.surface)
            .child(match jukebox.current() {
                Some(track) => cover_thumb(track, 44., palette),
                None => Icon::new(IconName::Music).large().text_color(palette.accent).into_any_element(),
            })
            .child(
                v_flex()
                    .w(px(220.))
                    .min_w_0()
                    .child(div().truncate().text_sm().font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(div().truncate().text_xs().text_color(palette.muted).child(artist)),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(Button::new("music-previous").ghost().small().icon(IconName::SkipBack).disabled(!has_track).on_click(cx.listener(|this, _, _, cx| {
                        this.jukebox.update(cx, |jukebox, cx| jukebox.previous(cx));
                    })))
                    .child(
                        Button::new("music-play")
                            .primary()
                            .small()
                            .icon(if playing { IconName::Pause } else { IconName::Play })
                            .disabled(!has_track)
                            .on_click(cx.listener(|this, _, _, cx| this.jukebox.update(cx, |jukebox, cx| jukebox.toggle(cx)))),
                    )
                    .child(Button::new("music-next").ghost().small().icon(IconName::SkipForward).disabled(!has_track).on_click(cx.listener(|this, _, _, cx| {
                        this.jukebox.update(cx, |jukebox, cx| jukebox.next(cx));
                    }))),
            )
            .child(div().flex_1().when(!remote, |this| {
                // Downloaded so far, in grey over the track (like a video site).
                let buffered = jukebox.buffered();
                this.relative()
                    .child(Slider::new(&self.seek).horizontal())
                    .when(buffered > 0. && buffered < 1., |this| {
                        this.child(
                            div()
                                .absolute()
                                .left_0()
                                .top(px(0.))
                                .bottom(px(0.))
                                .w(gpui_kit::relative(buffered))
                                .flex()
                                .items_center()
                                .child(div().w_full().h(px(4.)).rounded_full().bg(gpui_kit::hsla(0., 0., 0.6, 0.35))),
                        )
                    })
            }))
            .child(div().w(px(160.)).truncate().text_xs().text_color(palette.muted).child(status))
            .when(!remote, |this| {
                this.child(Icon::new(IconName::Volume2).small().text_color(palette.muted))
                    .child(div().w(px(90.)).child(Slider::new(&self.volume).horizontal()))
                    .child(
                        Button::new("music-eq")
                            .ghost()
                            .small()
                            .icon(IconName::SlidersVertical)
                            .tooltip("Equalizer")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_equalizer = !this.show_equalizer;
                                cx.notify();
                            })),
                    )
            })
            .into_any_element()
    }
}

impl Render for MusicView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        // The knob follows the music unless it is being dragged.
        if !self.seeking {
            let jukebox = self.jukebox.read(cx);
            let fraction = if jukebox.length > 0 { (jukebox.position as f32 / jukebox.length as f32).clamp(0., 1.) } else { 0. };
            self.seek.update(cx, |seek, cx| seek.set_value(fraction, window, cx));
        }
        let palette = Palette::from_settings(&settings);
        // Covers in a cache of the shelf being shown, freed on switching shelves.
        gpui_kit::image_cache(gpui_kit::retain_all(gpui_kit::ElementId::Name(format!("music-{:?}", self.shelf).into())))
            .size_full()
            .child(self.render_body(palette, cx))
    }
}

impl MusicView {
    fn render_body(&mut self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .key_context("Music")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_up))
            .on_action(cx.listener(Self::on_down))
            .on_action(cx.listener(Self::on_home))
            .on_action(cx.listener(Self::on_end))
            .on_action(cx.listener(Self::on_page_up))
            .on_action(cx.listener(Self::on_page_down))
            .on_action(cx.listener(Self::on_play_selected))
            .on_action(cx.listener(Self::on_preview))
            .size_full()
            .bg(palette.window)
            .text_color(palette.text)
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(div().flex_1().child(Input::new(&self.search).small().cleanable(true).prefix(Icon::new(IconName::Search).small().text_color(palette.muted))))
                    .child(Button::new("music-rescan").ghost().small().icon(IconName::RefreshCw).tooltip("Read the library folders again").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.load_library(cx);
                            this.load_playlists(cx);
                        },
                    ))),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sources(palette, cx))
                    .child(div().flex_1().min_w_0().h_full().debug_selector(|| "music-track-list".into()).child(self.render_tracks(palette, cx)))
                    .children(self.render_track_panel(palette, cx)),
            )
            .when(self.show_equalizer, |this| {
                this.child(div().w_full().flex_shrink_0().px_4().py_3().border_t_1().border_color(palette.border).bg(palette.surface).child(self.equalizer.clone()))
            })
            .child(self.render_now_playing(palette, cx))
    }
}

/// A track's cover, small: the picture once it is downloaded, else a note.
fn cover_thumb(track: &Track, size: f32, palette: Palette) -> AnyElement {
    match crate::views::music::cover_path(track, size <= files_ext_music::COVER_SMALL as f32) {
        Some(path) => gpui_kit::img(path).size(px(size)).rounded_sm().object_fit(gpui_kit::ObjectFit::Cover).into_any_element(),
        None => div()
            .size(px(size))
            .flex_shrink_0()
            .rounded_sm()
            .bg(palette.surface)
            .flex()
            .items_center()
            .justify_center()
            .child(Icon::new(IconName::Music).xsmall().text_color(palette.faint))
            .into_any_element(),
    }
}

// ------------------------------------------------------- sidebar and menus

impl FileExplorer {
    /// MUSIC: open the tab; what plays now, with play/pause.
    pub(crate) fn render_music_section(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !crate::licensing::active(self.appearance.read(cx), music_index()) {
            return None;
        }
        let language = self.appearance.read(cx).language;
        let mut section = v_flex().gap_1().child(self.section_header("music", crate::settings::localized(language, "MUSIC", "МУЗЫКА"), palette, cx));
        if self.section_collapsed("music", cx) {
            return Some(section.into_any_element());
        }
        let now = cx.try_global::<JukeboxGlobal>().map(|global| global.0.clone());
        let (title, playing) = now.as_ref().map(|jukebox| jukebox.read(cx)).map_or((None, false), |jukebox| (jukebox.current().map(|track| track.title.clone()), jukebox.playing));
        section = section.child(
            h_flex()
                .id("music-open")
                .h(px(30.))
                .px_2()
                .gap_2()
                .items_center()
                .rounded_md()
                .text_sm()
                .text_color(palette.muted)
                .hover(|style| style.bg(palette.hover).text_color(palette.text))
                .child(Icon::new(IconName::Music).small())
                .child(div().flex_1().min_w_0().truncate().child(title.unwrap_or_else(|| crate::settings::localized(language, "Music", "Музыка").to_string())))
                .when_some(now, |this, jukebox| {
                    this.child(Button::new("music-sidebar-play").ghost().xsmall().icon(if playing { IconName::Pause } else { IconName::Play }).on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        jukebox.update(cx, |jukebox, cx| jukebox.toggle(cx));
                    }))
                })
                .on_click(cx.listener(|this, _, window, cx| this.open_music(window, cx))),
        );
        Some(section.into_any_element())
    }

    /// The Music tab (one), switched to when it is open.
    pub(crate) fn open_music(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.tabs.iter().position(|tab| tab.panel.as_ref().is_some_and(|panel| panel.clone().downcast::<MusicView>().is_ok())) {
            self.select_tab(index, window, cx);
            return;
        }
        let appearance = self.appearance.clone();
        let explorer = cx.entity().downgrade();
        // Settings → Extensions → Music → Opens in.
        if self.appearance.read(cx).opens_in("music") == crate::settings::OpenIn::Window {
            let options = gpui_kit::WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds { origin: gpui_kit::point(px(180.), px(90.)), size: gpui_kit::size(px(1120.), px(720.)) })),
                window_min_size: Some(gpui_kit::size(px(720.), px(440.))),
                titlebar: Some(gpui_kit::TitlebarOptions { title: Some("Music".into()), ..Default::default() }),
                ..Default::default()
            };
            let _ = gpui_kit::open_window(options, cx, move |window, cx| cx.new(|cx| MusicView::new(window, cx, appearance, explorer)));
            return;
        }
        let view = cx.new(|cx| MusicView::new(window, cx, appearance, explorer));
        self.open_panel_tab("Music", IconName::Music, view.into(), window, cx);
    }

    /// Context menu → Play in Music: this file, then the folder's others.
    pub(crate) fn play_in_music(&mut self, path: &Path, cx: &mut Context<Self>) {
        let (tracks, index) = folder_queue(path);
        jukebox(cx).update(cx, |jukebox, cx| jukebox.play_queue(tracks, index, cx));
        cx.notify();
    }

    /// Cmd-K `music:` result: play it now.
    pub(crate) fn play_track(&mut self, track: Track, cx: &mut Context<Self>) {
        jukebox(cx).update(cx, |jukebox, cx| jukebox.play_queue(vec![track], 0, cx));
        cx.notify();
    }

    pub(crate) fn add_to_queue(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let tracks: Vec<Track> = paths.iter().filter(|path| local::is_audio(path)).map(|path| local::track(path)).collect();
        let count = tracks.len();
        jukebox(cx).update(cx, |jukebox, cx| jukebox.enqueue(tracks, cx));
        self.notice = Some(format!("{count} added to the Music queue"));
        cx.notify();
    }
}

pub(crate) fn music_index() -> usize {
    crate::extensions::CATALOG.iter().position(|manifest| manifest.id == files_ext_music::MANIFEST.id).unwrap_or(usize::MAX)
}

#[cfg(test)]
impl MusicView {
    pub(crate) fn show_tracks_for_test(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        self.shelf = Shelf::SpotifyLiked;
        self.busy = false;
        self.tracks = tracks;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_becomes_a_queue_starting_at_the_file() {
        let dir = std::env::temp_dir().join(format!("file-force-music-queue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["b.mp3", "a.flac", "notes.txt", "c.m4a"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let (tracks, index) = folder_queue(&dir.join("b.mp3"));
        let titles: Vec<_> = tracks.iter().map(|track| track.title.as_str()).collect();
        assert_eq!(titles, ["a", "b", "c"], "audio only, in name order");
        assert_eq!(index, 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
