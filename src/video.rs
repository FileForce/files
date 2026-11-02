//! Video in Quick Look through libvlc, loaded at run time from an installed
//! VLC (macOS, Windows, Linux). VLC decodes picture and sound; the picture
//! is handed over frame by frame as BGRA and drawn as an image, so nothing
//! native is embedded. Without VLC, the preview says how to get it.

use std::ffi::{CString, c_char, c_int, c_uint, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// Frames are scaled down to at most this width: enough for a preview.
const MAX_WIDTH: u32 = 1280;

type Instance = c_void;
type Media = c_void;
type MediaPlayer = c_void;
type LockCallback = unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> *mut c_void;
type UnlockCallback = unsafe extern "C" fn(*mut c_void, *mut c_void, *const *mut c_void);
type DisplayCallback = unsafe extern "C" fn(*mut c_void, *mut c_void);
type SetupCallback = unsafe extern "C" fn(*mut *mut c_void, *mut c_char, *mut c_uint, *mut c_uint, *mut c_uint, *mut c_uint) -> c_uint;
type CleanupCallback = unsafe extern "C" fn(*mut c_void);

/// The libvlc 3 functions the player uses.
struct Vlc {
    _library: libloading::Library,
    /// libvlccore, loaded first on macOS so libvlc's `@rpath` finds it.
    _core: Option<libloading::Library>,
    instance: *mut Instance,
    media_new_path: unsafe extern "C" fn(*mut Instance, *const c_char) -> *mut Media,
    media_release: unsafe extern "C" fn(*mut Media),
    player_new: unsafe extern "C" fn(*mut Media) -> *mut MediaPlayer,
    player_release: unsafe extern "C" fn(*mut MediaPlayer),
    set_callbacks: unsafe extern "C" fn(*mut MediaPlayer, Option<LockCallback>, Option<UnlockCallback>, Option<DisplayCallback>, *mut c_void),
    set_format_callbacks: unsafe extern "C" fn(*mut MediaPlayer, Option<SetupCallback>, Option<CleanupCallback>),
    play: unsafe extern "C" fn(*mut MediaPlayer) -> c_int,
    set_pause: unsafe extern "C" fn(*mut MediaPlayer, c_int),
    stop: unsafe extern "C" fn(*mut MediaPlayer),
    is_playing: unsafe extern "C" fn(*mut MediaPlayer) -> c_int,
    get_time: unsafe extern "C" fn(*mut MediaPlayer) -> i64,
    get_length: unsafe extern "C" fn(*mut MediaPlayer) -> i64,
    set_position: unsafe extern "C" fn(*mut MediaPlayer, f32),
    get_state: unsafe extern "C" fn(*mut MediaPlayer) -> c_int,
    audio_set_mute: unsafe extern "C" fn(*mut MediaPlayer, c_int),
    audio_set_volume: unsafe extern "C" fn(*mut MediaPlayer, c_int) -> c_int,
    equalizer_new: unsafe extern "C" fn() -> *mut c_void,
    equalizer_new_from_preset: unsafe extern "C" fn(c_uint) -> *mut c_void,
    equalizer_release: unsafe extern "C" fn(*mut c_void),
    equalizer_set_preamp: unsafe extern "C" fn(*mut c_void, f32) -> c_int,
    equalizer_get_preamp: unsafe extern "C" fn(*mut c_void) -> f32,
    equalizer_set_amp_at_index: unsafe extern "C" fn(*mut c_void, f32, c_uint) -> c_int,
    equalizer_get_amp_at_index: unsafe extern "C" fn(*mut c_void, c_uint) -> f32,
    equalizer_preset_count: unsafe extern "C" fn() -> c_uint,
    equalizer_preset_name: unsafe extern "C" fn(c_uint) -> *const c_char,
    equalizer_band_count: unsafe extern "C" fn() -> c_uint,
    equalizer_band_frequency: unsafe extern "C" fn(c_uint) -> f32,
    set_equalizer: unsafe extern "C" fn(*mut MediaPlayer, *mut c_void) -> c_int,
}

// SAFETY: libvlc's instance and functions may be used from any thread.
unsafe impl Send for Vlc {}
unsafe impl Sync for Vlc {}

/// libvlc_Ended in libvlc 3's state enum.
const STATE_ENDED: c_int = 6;

/// Where an installed VLC keeps libvlc on this system.
fn candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(custom) = std::env::var_os("FILES_LIBVLC") {
        paths.push(PathBuf::from(custom));
    }
    // The copy that ships with the app (cargo xtask bundle), then an installed VLC.
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) {
        if cfg!(target_os = "macos") {
            paths.push(exe_dir.join("../Frameworks/vlc/lib/libvlc.dylib"));
        } else if cfg!(windows) {
            paths.push(exe_dir.join("vlc").join("libvlc.dll"));
        } else {
            // AppImage: usr/bin/file-force and usr/lib/vlc/.
            paths.push(exe_dir.join("../lib/vlc/lib/libvlc.so.5"));
        }
    }
    if cfg!(target_os = "macos") {
        paths.push(PathBuf::from("/Applications/VLC.app/Contents/MacOS/lib/libvlc.dylib"));
        if let Some(home) = dirs::home_dir() {
            paths.push(home.join("Applications/VLC.app/Contents/MacOS/lib/libvlc.dylib"));
        }
    } else if cfg!(windows) {
        for base in [std::env::var_os("ProgramFiles"), std::env::var_os("ProgramFiles(x86)")].into_iter().flatten() {
            paths.push(PathBuf::from(base).join("VideoLAN").join("VLC").join("libvlc.dll"));
        }
    } else {
        paths.push(PathBuf::from("libvlc.so.5"));
        paths.push(PathBuf::from("libvlc.so"));
    }
    paths
}

#[cfg(windows)]
fn open_library(path: &Path) -> Result<libloading::Library, libloading::Error> {
    // libvlc.dll needs libvlccore.dll from its own folder.
    const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;
    // SAFETY: loading VLC's own library; its initializers are trusted like VLC itself.
    unsafe { libloading::os::windows::Library::load_with_flags(path, LOAD_WITH_ALTERED_SEARCH_PATH).map(Into::into) }
}

#[cfg(not(windows))]
fn open_library(path: &Path) -> Result<libloading::Library, libloading::Error> {
    // SAFETY: as above.
    unsafe { libloading::Library::new(path) }
}

fn load() -> Result<Vlc, String> {
    let mut last = "VLC is not installed".to_string();
    for path in candidates() {
        if path.is_absolute() && !path.is_file() {
            continue;
        }
        // `lib/` beside `plugins/` (VLC.app, the bundled copies): say where.
        // On Windows libvlc finds `plugins\` next to itself.
        if let Some(plugins) = path.parent().and_then(Path::parent).map(|dir| dir.join("plugins")).filter(|plugins| plugins.is_dir()) {
            // SAFETY: set once, before libvlc starts any thread, from the UI thread.
            unsafe { std::env::set_var("VLC_PLUGIN_PATH", plugins) };
        }
        // libvlc asks for libvlccore by name (`@rpath/…` on macOS): loaded
        // first from the same folder, it is found.
        let core_name = if cfg!(target_os = "macos") { "libvlccore.dylib" } else { "libvlccore.so.9" };
        let core = path.parent().map(|lib| lib.join(core_name)).filter(|core| !cfg!(windows) && core.is_file());
        let core = match core.map(|core| open_library(&core)) {
            Some(Ok(core)) => Some(core),
            Some(Err(error)) => {
                last = format!("{}: {error}", path.display());
                continue;
            }
            None => None,
        };
        let library = match open_library(&path) {
            Ok(library) => library,
            Err(error) => {
                last = format!("{}: {error}", path.display());
                continue;
            }
        };
        // SAFETY: the symbols are libvlc 3's public C API with these signatures.
        unsafe {
            macro_rules! symbol {
                ($name:literal) => {
                    *library.get(concat!($name, "\0").as_bytes()).map_err(|error| format!("{}: {error}", $name))?
                };
            }
            let new: unsafe extern "C" fn(c_int, *const *const c_char) -> *mut Instance = symbol!("libvlc_new");
            let arguments: Vec<CString> = ["--quiet", "--no-video-title-show", "--no-osd", "--no-snapshot-preview"]
                .iter()
                .map(|argument| CString::new(*argument).expect("no NUL"))
                .collect();
            let pointers: Vec<*const c_char> = arguments.iter().map(|argument| argument.as_ptr()).collect();
            let instance = new(pointers.len() as c_int, pointers.as_ptr());
            if instance.is_null() {
                last = format!("{}: VLC did not start", path.display());
                continue;
            }
            let vlc = Vlc {
                instance,
                media_new_path: symbol!("libvlc_media_new_path"),
                media_release: symbol!("libvlc_media_release"),
                player_new: symbol!("libvlc_media_player_new_from_media"),
                player_release: symbol!("libvlc_media_player_release"),
                set_callbacks: symbol!("libvlc_video_set_callbacks"),
                set_format_callbacks: symbol!("libvlc_video_set_format_callbacks"),
                play: symbol!("libvlc_media_player_play"),
                set_pause: symbol!("libvlc_media_player_set_pause"),
                stop: symbol!("libvlc_media_player_stop"),
                is_playing: symbol!("libvlc_media_player_is_playing"),
                get_time: symbol!("libvlc_media_player_get_time"),
                get_length: symbol!("libvlc_media_player_get_length"),
                set_position: symbol!("libvlc_media_player_set_position"),
                get_state: symbol!("libvlc_media_player_get_state"),
                audio_set_mute: symbol!("libvlc_audio_set_mute"),
                audio_set_volume: symbol!("libvlc_audio_set_volume"),
                equalizer_new: symbol!("libvlc_audio_equalizer_new"),
                equalizer_new_from_preset: symbol!("libvlc_audio_equalizer_new_from_preset"),
                equalizer_release: symbol!("libvlc_audio_equalizer_release"),
                equalizer_set_preamp: symbol!("libvlc_audio_equalizer_set_preamp"),
                equalizer_get_preamp: symbol!("libvlc_audio_equalizer_get_preamp"),
                equalizer_set_amp_at_index: symbol!("libvlc_audio_equalizer_set_amp_at_index"),
                equalizer_get_amp_at_index: symbol!("libvlc_audio_equalizer_get_amp_at_index"),
                equalizer_preset_count: symbol!("libvlc_audio_equalizer_get_preset_count"),
                equalizer_preset_name: symbol!("libvlc_audio_equalizer_get_preset_name"),
                equalizer_band_count: symbol!("libvlc_audio_equalizer_get_band_count"),
                equalizer_band_frequency: symbol!("libvlc_audio_equalizer_get_band_frequency"),
                set_equalizer: symbol!("libvlc_media_player_set_equalizer"),
                _library: library,
                _core: core,
            };
            files_sdk::log::record("files", files_sdk::log::Kind::Process, format!("Video: libvlc loaded from {}", path.display()), None);
            return Ok(vlc);
        }
    }
    Err(last)
}

fn vlc() -> Result<&'static Vlc, &'static str> {
    static VLC: OnceLock<Result<Vlc, String>> = OnceLock::new();
    VLC.get_or_init(load).as_ref().map_err(String::as_str)
}

/// The equalizer's settings: a gain per band, in dB (−20…20), and the
/// preamp. `None` anywhere it is used means "off".
#[derive(Clone, Debug, PartialEq)]
pub struct Equalizer {
    pub preamp: f32,
    pub bands: Vec<f32>,
}

/// The band centres in Hz (VLC's 10 bands: 60 Hz … 16 kHz).
pub fn equalizer_bands() -> Vec<f32> {
    let Ok(vlc) = vlc() else { return Vec::new() };
    // SAFETY: plain queries of libvlc's static tables.
    unsafe { (0..(vlc.equalizer_band_count)()).map(|band| (vlc.equalizer_band_frequency)(band)).collect() }
}

/// VLC's presets by name ("Flat", "Rock", "Classical"…).
pub fn equalizer_presets() -> Vec<String> {
    let Ok(vlc) = vlc() else { return Vec::new() };
    // SAFETY: names are static NUL-terminated strings owned by libvlc.
    unsafe {
        (0..(vlc.equalizer_preset_count)())
            .map(|index| {
                let name = (vlc.equalizer_preset_name)(index);
                if name.is_null() { String::new() } else { std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned() }
            })
            .collect()
    }
}

/// A preset's values, to start from and adjust.
pub fn equalizer_preset(index: usize) -> Option<Equalizer> {
    let vlc = vlc().ok()?;
    // SAFETY: the equalizer is made, read and released here.
    unsafe {
        let equalizer = (vlc.equalizer_new_from_preset)(index as c_uint);
        if equalizer.is_null() {
            return None;
        }
        let values = Equalizer {
            preamp: (vlc.equalizer_get_preamp)(equalizer),
            bands: (0..(vlc.equalizer_band_count)()).map(|band| (vlc.equalizer_get_amp_at_index)(equalizer, band)).collect(),
        };
        (vlc.equalizer_release)(equalizer);
        Some(values)
    }
}

pub const VLC_DOWNLOAD: &str = "https://www.videolan.org/vlc/";

/// The latest decoded picture, shared with VLC's threads.
#[derive(Default)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// BGRA (VLC's RV32), `width * height * 4` bytes.
    pub bgra: Vec<u8>,
    /// Bumped for every new picture.
    pub sequence: u64,
}

/// What VLC's callbacks write into: a decode buffer VLC owns while it
/// draws, copied into `front` when a picture is complete.
struct Target {
    decode: Vec<u8>,
    width: u32,
    height: u32,
    front: Arc<Mutex<Frame>>,
}

unsafe extern "C" fn setup(
    opaque: *mut *mut c_void,
    chroma: *mut c_char,
    width: *mut c_uint,
    height: *mut c_uint,
    pitches: *mut c_uint,
    lines: *mut c_uint,
) -> c_uint {
    // SAFETY: VLC passes the opaque set in set_callbacks (a live Target) and
    // valid out-pointers for the format.
    unsafe {
        let target = &mut *(*opaque as *mut Target);
        let (mut w, mut h) = (*width, *height);
        if w == 0 || h == 0 {
            return 0;
        }
        if w > MAX_WIDTH {
            h = ((h as u64 * MAX_WIDTH as u64 / w as u64) as u32).max(2) & !1;
            w = MAX_WIDTH;
        }
        std::ptr::copy_nonoverlapping(b"RV32".as_ptr() as *const c_char, chroma, 4);
        *width = w;
        *height = h;
        *pitches = w * 4;
        *lines = h;
        target.width = w;
        target.height = h;
        target.decode = vec![0; (w * h * 4) as usize];
        1
    }
}

unsafe extern "C" fn cleanup(_: *mut c_void) {}

unsafe extern "C" fn lock(opaque: *mut c_void, planes: *mut *mut c_void) -> *mut c_void {
    // SAFETY: opaque is the player's Target; planes has room for one plane.
    unsafe {
        let target = &mut *(opaque as *mut Target);
        *planes = target.decode.as_mut_ptr() as *mut c_void;
    }
    std::ptr::null_mut()
}

unsafe extern "C" fn unlock(_: *mut c_void, _: *mut c_void, _: *const *mut c_void) {}

unsafe extern "C" fn display(opaque: *mut c_void, _: *mut c_void) {
    // SAFETY: opaque is the player's Target, alive until the player stops.
    let target = unsafe { &mut *(opaque as *mut Target) };
    let mut front = target.front.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    front.width = target.width;
    front.height = target.height;
    front.bgra.clear();
    front.bgra.extend_from_slice(&target.decode);
    // RV32's fourth byte is padding: make the picture opaque.
    for alpha in front.bgra.iter_mut().skip(3).step_by(4) {
        *alpha = 255;
    }
    front.sequence += 1;
}

/// One file playing (or paused).
pub struct Player {
    vlc: &'static Vlc,
    player: *mut MediaPlayer,
    target: *mut Target,
    pub frame: Arc<Mutex<Frame>>,
}

// SAFETY: libvlc's media player may be driven from any thread.
unsafe impl Send for Player {}

impl Player {
    pub fn open(path: &Path) -> Result<Self, String> {
        let vlc = vlc().map_err(str::to_string)?;
        let location = CString::new(path.to_string_lossy().as_bytes()).map_err(|_| "Unusual file name".to_string())?;
        let frame = Arc::new(Mutex::new(Frame::default()));
        let target = Box::into_raw(Box::new(Target { decode: Vec::new(), width: 0, height: 0, front: frame.clone() }));
        // SAFETY: libvlc calls with a valid instance; the target outlives the
        // player (released in Drop after stop, which joins VLC's threads).
        unsafe {
            let media = (vlc.media_new_path)(vlc.instance, location.as_ptr());
            if media.is_null() {
                drop(Box::from_raw(target));
                return Err("VLC cannot open this file".to_string());
            }
            let player = (vlc.player_new)(media);
            (vlc.media_release)(media);
            if player.is_null() {
                drop(Box::from_raw(target));
                return Err("VLC cannot play this file".to_string());
            }
            (vlc.set_callbacks)(player, Some(lock), Some(unlock), Some(display), target as *mut c_void);
            (vlc.set_format_callbacks)(player, Some(setup), Some(cleanup));
            Ok(Self { vlc, player, target, frame })
        }
    }

    pub fn play(&self) {
        // SAFETY: a live player.
        unsafe {
            if (self.vlc.get_state)(self.player) == STATE_ENDED {
                (self.vlc.stop)(self.player);
            }
            (self.vlc.play)(self.player);
        }
    }

    pub fn pause(&self) {
        // SAFETY: a live player.
        unsafe { (self.vlc.set_pause)(self.player, 1) }
    }

    pub fn is_playing(&self) -> bool {
        // SAFETY: a live player.
        unsafe { (self.vlc.is_playing)(self.player) != 0 }
    }

    /// Milliseconds played and the whole length (0 until known).
    pub fn progress(&self) -> (i64, i64) {
        // SAFETY: a live player.
        unsafe { ((self.vlc.get_time)(self.player).max(0), (self.vlc.get_length)(self.player).max(0)) }
    }

    /// Jumps to a fraction (0…1) of the video.
    pub fn seek(&self, fraction: f32) {
        // SAFETY: a live player.
        unsafe { (self.vlc.set_position)(self.player, fraction.clamp(0., 1.)) }
    }

    /// 0…100 (VLC's 100 is the file's own level).
    pub fn set_volume(&self, volume: u8) {
        // SAFETY: a live player.
        unsafe { (self.vlc.audio_set_volume)(self.player, volume.min(100) as c_int) };
    }

    /// The equalizer (`None` turns it off). VLC copies the values.
    pub fn set_equalizer(&self, equalizer: Option<&Equalizer>) {
        // SAFETY: a live player; the equalizer is released after VLC copied it.
        unsafe {
            match equalizer {
                None => {
                    (self.vlc.set_equalizer)(self.player, std::ptr::null_mut());
                }
                Some(values) => {
                    let handle = (self.vlc.equalizer_new)();
                    if handle.is_null() {
                        return;
                    }
                    (self.vlc.equalizer_set_preamp)(handle, values.preamp.clamp(-20., 20.));
                    for (band, gain) in values.bands.iter().enumerate() {
                        (self.vlc.equalizer_set_amp_at_index)(handle, gain.clamp(-20., 20.), band as c_uint);
                    }
                    (self.vlc.set_equalizer)(self.player, handle);
                    (self.vlc.equalizer_release)(handle);
                }
            }
        }
    }

    /// The file played to its end.
    pub fn ended(&self) -> bool {
        // SAFETY: a live player.
        unsafe { (self.vlc.get_state)(self.player) == STATE_ENDED }
    }

    pub fn set_muted(&self, muted: bool) {
        // SAFETY: a live player.
        unsafe { (self.vlc.audio_set_mute)(self.player, muted as c_int) }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // SAFETY: stop waits for VLC's threads, so no callback runs after it;
        // then the player and its target are freed exactly once.
        unsafe {
            (self.vlc.stop)(self.player);
            (self.vlc.player_release)(self.player);
            drop(Box::from_raw(self.target));
        }
    }
}

/// `1:05` / `1:02:03`.
pub fn clock(milliseconds: i64) -> String {
    let seconds = milliseconds / 1000;
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes}:{seconds:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_reads_like_a_player() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(65_000), "1:05");
        assert_eq!(clock(3_723_000), "1:02:03");
    }

    /// Needs VLC and a video: `FILES_TEST_VIDEO=/path/to.mov cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn vlc_plays_frames_of_a_real_video() {
        let Some(video) = std::env::var_os("FILES_TEST_VIDEO") else { return };
        let player = Player::open(Path::new(&video)).expect("VLC");
        player.set_muted(true);
        player.play();
        let started = std::time::Instant::now();
        while player.frame.lock().unwrap().sequence < 5 {
            assert!(started.elapsed().as_secs() < 10, "no frames from VLC");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let frame = player.frame.lock().unwrap();
        assert!(frame.width > 0 && frame.width <= MAX_WIDTH);
        assert_eq!(frame.bgra.len(), (frame.width * frame.height * 4) as usize);
        drop(frame);
        assert!(player.progress().1 > 0, "length known while playing");
        // Sound controls: volume and an equalizer preset.
        player.set_volume(40);
        let presets = equalizer_presets();
        assert!(presets.iter().any(|name| name == "Rock"), "{presets:?}");
        assert_eq!(equalizer_bands().len(), 10);
        let rock = equalizer_preset(presets.iter().position(|name| name == "Rock").unwrap()).unwrap();
        assert_eq!(rock.bands.len(), 10);
        player.set_equalizer(Some(&rock));
        player.set_equalizer(None);
    }
}
