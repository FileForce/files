//! Sounds: a chime for new notifications, and the speaker test of
//! Settings → Sound (a tone in each speaker of a 2.1…7.1 or headphone set).
//! Every sound is made here as a WAV and played through VLC, which the app
//! ships, so it sounds the same on macOS, Windows and Linux.

use std::path::PathBuf;
use std::time::Duration;

use crate::settings::{Chime, SpeakerLayout};

const RATE: u32 = 44_100;

/// A speaker: its name and its bit in WAVE_FORMAT_EXTENSIBLE's channel mask.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Speaker {
    pub name: &'static str,
    pub short: &'static str,
    mask: u32,
    /// The subwoofer gets a low tone.
    low: bool,
}

const FL: Speaker = Speaker { name: "Front left", short: "L", mask: 0x1, low: false };
const FR: Speaker = Speaker { name: "Front right", short: "R", mask: 0x2, low: false };
const FC: Speaker = Speaker { name: "Center", short: "C", mask: 0x4, low: false };
const LFE: Speaker = Speaker { name: "Subwoofer", short: "LFE", mask: 0x8, low: true };
const BL: Speaker = Speaker { name: "Rear left", short: "RL", mask: 0x10, low: false };
const BR: Speaker = Speaker { name: "Rear right", short: "RR", mask: 0x20, low: false };
const SL: Speaker = Speaker { name: "Side left", short: "SL", mask: 0x200, low: false };
const SR: Speaker = Speaker { name: "Side right", short: "SR", mask: 0x400, low: false };
const HL: Speaker = Speaker { name: "Left ear", short: "L", mask: 0x1, low: false };
const HR: Speaker = Speaker { name: "Right ear", short: "R", mask: 0x2, low: false };

/// The speakers of a layout, in the WAV channel order.
pub fn speakers(layout: SpeakerLayout) -> &'static [Speaker] {
    match layout {
        SpeakerLayout::Headphones => &[HL, HR],
        SpeakerLayout::Stereo21 => &[FL, FR, LFE],
        SpeakerLayout::Surround31 => &[FL, FR, FC, LFE],
        SpeakerLayout::Surround51 => &[FL, FR, FC, LFE, BL, BR],
        SpeakerLayout::Surround71 => &[FL, FR, FC, LFE, BL, BR, SL, SR],
    }
}

/// 16-bit PCM WAV (WAVE_FORMAT_EXTENSIBLE when more than two channels).
pub fn wav(channels: u16, mask: u32, frames: &[Vec<f32>]) -> Vec<u8> {
    let samples = frames.first().map_or(0, Vec::len);
    let data_len = samples as u32 * channels as u32 * 2;
    let extensible = channels > 2;
    let fmt_len: u32 = if extensible { 40 } else { 16 };
    let mut out = Vec::with_capacity(44 + data_len as usize + 24);
    out.extend(b"RIFF");
    out.extend((4 + 8 + fmt_len + 8 + data_len).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(fmt_len.to_le_bytes());
    out.extend((if extensible { 0xFFFEu16 } else { 1u16 }).to_le_bytes());
    out.extend(channels.to_le_bytes());
    out.extend(RATE.to_le_bytes());
    out.extend((RATE * channels as u32 * 2).to_le_bytes());
    out.extend((channels * 2).to_le_bytes());
    out.extend(16u16.to_le_bytes());
    if extensible {
        out.extend(22u16.to_le_bytes());
        out.extend(16u16.to_le_bytes());
        out.extend(mask.to_le_bytes());
        // KSDATAFORMAT_SUBTYPE_PCM
        out.extend([0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71]);
    }
    out.extend(b"data");
    out.extend(data_len.to_le_bytes());
    for index in 0..samples {
        for channel in frames {
            let value = (channel[index].clamp(-1., 1.) * i16::MAX as f32) as i16;
            out.extend(value.to_le_bytes());
        }
    }
    out
}

/// A tone that fades in and out (no clicks).
fn tone(frequency: f32, seconds: f32, gain: f32) -> Vec<f32> {
    let count = (seconds * RATE as f32) as usize;
    let fade = (RATE as f32 * 0.02) as usize;
    (0..count)
        .map(|index| {
            let envelope = (index.min(count - index) as f32 / fade as f32).min(1.);
            (index as f32 * frequency * std::f32::consts::TAU / RATE as f32).sin() * envelope * gain
        })
        .collect()
}

/// Notes one after the other, each decaying like a bell.
fn bell(notes: &[(f32, f32)]) -> Vec<f32> {
    notes
        .iter()
        .flat_map(|(frequency, seconds)| {
            let count = (seconds * RATE as f32) as usize;
            (0..count).map(move |index| {
                let at = index as f32 / RATE as f32;
                let attack = (index as f32 / (RATE as f32 * 0.005)).min(1.);
                let decay = (-at * 6.).exp();
                let wave = (at * frequency * std::f32::consts::TAU).sin() + 0.3 * (at * frequency * 2. * std::f32::consts::TAU).sin();
                wave * 0.35 * attack * decay
            })
        })
        .collect()
}

/// How a notification sounds, by its kind.
pub fn chime_samples(chime: Chime, level: files_sdk::log::Level) -> Vec<f32> {
    use files_sdk::log::Level;
    let base = match chime {
        Chime::Soft => 660.,
        Chime::Bright => 880.,
        Chime::Low => 440.,
    };
    match level {
        Level::Success => bell(&[(base, 0.12), (base * 1.5, 0.35)]),
        Level::Error => bell(&[(base * 0.75, 0.15), (base * 0.6, 0.4)]),
        Level::Warning => bell(&[(base, 0.15), (base, 0.35)]),
        Level::Info => bell(&[(base * 1.25, 0.4)]),
    }
}

pub(crate) fn cache_dir() -> PathBuf {
    std::env::temp_dir().join("file-force-sounds")
}

/// Writes a sound once and returns its file.
fn cached(name: &str, make: impl FnOnce() -> Vec<u8>) -> std::io::Result<PathBuf> {
    let path = cache_dir().join(name);
    if !path.is_file() {
        std::fs::create_dir_all(cache_dir())?;
        std::fs::write(&path, make())?;
    }
    Ok(path)
}

/// Sounds started (tests count them instead of making noise).
pub static PLAYED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Plays a file at `volume` (0…100) and lets it finish in the background.
fn play(path: PathBuf, volume: u8, length: Duration) {
    PLAYED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if cfg!(test) {
        return;
    }
    std::thread::spawn(move || {
        let Ok(player) = crate::video::Player::open(&path) else { return };
        player.set_volume(volume);
        player.play();
        // The player lives until the sound has played.
        let started = std::time::Instant::now();
        while started.elapsed() < length + Duration::from_millis(400) && !player.ended() {
            std::thread::sleep(Duration::from_millis(50));
        }
    });
}

/// The chime for a new notification of `level`.
pub fn notify(chime: Chime, level: files_sdk::log::Level, volume: u8) {
    let name = format!("chime-{chime:?}-{level:?}.wav").to_lowercase();
    if let Ok(path) = cached(&name, || {
        let samples = chime_samples(chime, level);
        wav(1, 0x4, &[samples])
    }) {
        play(path, volume, Duration::from_millis(700));
    }
}

/// A tone in one speaker of `layout` (silence in the others).
pub fn test_speaker(layout: SpeakerLayout, index: usize, volume: u8) {
    let list = speakers(layout);
    let Some(speaker) = list.get(index) else { return };
    let name = format!("test-{layout:?}-{index}.wav").to_lowercase();
    if let Ok(path) = cached(&name, || {
        let length = 1.2;
        let wanted = if speaker.low { tone(55., length, 0.8) } else { tone(523.25, length, 0.5) };
        let silence = vec![0.; wanted.len()];
        let frames: Vec<Vec<f32>> = (0..list.len()).map(|channel| if channel == index { wanted.clone() } else { silence.clone() }).collect();
        let mask = list.iter().fold(0, |mask, speaker| mask | speaker.mask);
        wav(list.len() as u16, mask, &frames)
    }) {
        files_sdk::log::record("files", files_sdk::log::Kind::Setting, format!("speaker test: {} ({layout:?})", speaker.name), None);
        play(path, volume, Duration::from_millis(1200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_files_have_the_right_shape() {
        let mono = wav(1, 0x4, &[vec![0.; 100]]);
        assert_eq!(&mono[..4], b"RIFF");
        assert_eq!(&mono[8..16], b"WAVEfmt ");
        assert_eq!(mono.len(), 44 + 200, "16-bit mono: 2 bytes a sample");
        // 5.1: extensible format with its channel mask.
        let layout = speakers(SpeakerLayout::Surround51);
        let mask = layout.iter().fold(0, |mask, speaker| mask | speaker.mask);
        assert_eq!(mask, 0x3F);
        let surround = wav(6, mask, &vec![vec![0.; 10]; 6]);
        assert_eq!(u16::from_le_bytes([surround[20], surround[21]]), 0xFFFE);
        assert_eq!(u16::from_le_bytes([surround[22], surround[23]]), 6);
        assert_eq!(u32::from_le_bytes([surround[40], surround[41], surround[42], surround[43]]), 0x3F);
        assert_eq!(speakers(SpeakerLayout::Surround71).len(), 8);
        assert_eq!(speakers(SpeakerLayout::Headphones).len(), 2);
    }

    /// VLC plays the generated files (muted): `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn vlc_plays_chimes_and_surround_tests() {
        let chime = cached("check-chime.wav", || wav(1, 0x4, &[chime_samples(Chime::Soft, files_sdk::log::Level::Success)])).unwrap();
        let layout = speakers(SpeakerLayout::Surround51);
        let frames: Vec<Vec<f32>> = (0..layout.len()).map(|channel| if channel == 3 { tone(55., 0.5, 0.8) } else { vec![0.; (0.5 * RATE as f32) as usize] }).collect();
        let surround = cached("check-51.wav", || wav(6, 0x3F, &frames)).unwrap();
        for path in [chime, surround] {
            let player = crate::video::Player::open(&path).unwrap();
            player.set_muted(true);
            player.play();
            let started = std::time::Instant::now();
            while player.progress().1 == 0 {
                assert!(started.elapsed().as_secs() < 5, "VLC did not open {}", path.display());
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }

    #[test]
    fn chimes_differ_by_kind_and_stay_in_range() {
        use files_sdk::log::Level;
        let success = chime_samples(Chime::Soft, Level::Success);
        let error = chime_samples(Chime::Soft, Level::Error);
        assert_ne!(success, error);
        assert!(success.iter().all(|sample| sample.abs() <= 1.));
    }
}

// ------------------------------------------------------------ for extensions

/// The user's sound settings, for chimes extensions ask for off the UI thread.
static SETTINGS: std::sync::Mutex<Option<crate::settings::SoundSettings>> = std::sync::Mutex::new(None);

pub fn set_settings(settings: &crate::settings::SoundSettings) {
    *SETTINGS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(settings.clone());
}

fn settings() -> crate::settings::SoundSettings {
    SETTINGS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone().unwrap_or_default()
}

/// Player commands wait here for the UI thread (the music player lives there).
static PENDING: std::sync::Mutex<Vec<files_sdk::audio::Command>> = std::sync::Mutex::new(Vec::new());

pub fn take_pending() -> Vec<files_sdk::audio::Command> {
    std::mem::take(&mut *PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
}

/// What an extension asked for: chimes and one-off sounds play at once,
/// the rest goes to the music player.
pub fn extension_audio(extension: &str, command: files_sdk::audio::Command) -> std::io::Result<()> {
    use files_sdk::audio::{Command, Source};
    files_sdk::log::record(extension, files_sdk::log::Kind::Info, format!("sound: {command:?}"), None);
    match command {
        Command::Chime(level) => {
            let settings = settings();
            notify(settings.chime, level, settings.volume);
        }
        Command::Play { source, volume } => {
            let volume = volume.unwrap_or_else(|| settings().volume).min(100);
            match source {
                Source::File(path) if path.is_file() => play(path, volume, Duration::from_secs(30)),
                Source::File(path) => return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("{} not found", path.display()))),
                Source::Url(url) => play_url(url, volume),
            }
        }
        other => PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(other),
    }
    Ok(())
}

/// A one-off sound from the web.
fn play_url(url: String, volume: u8) {
    PLAYED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if cfg!(test) {
        return;
    }
    std::thread::spawn(move || {
        let Ok(player) = crate::video::Player::open_url(&url) else { return };
        player.set_volume(volume);
        player.play();
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(60) && !player.ended() {
            std::thread::sleep(Duration::from_millis(100));
        }
    });
}
