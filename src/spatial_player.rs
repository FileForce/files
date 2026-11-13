//! Music's player when spatial audio is on (Settings → Music): the track is
//! decoded here, goes through `files_ext_music::spatial::Chain` (upmix to 5.1,
//! a virtual speaker room for the ears) and out of the system's audio device
//! directly — not through VLC, which reads ~100 s ahead, far too late for
//! the head turning. Plays, pauses, seeks and reports like `video::Player`.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use files_ext_music::enhance::Strength;
use files_ext_music::live::Growing;
use files_ext_music::spatial::{Chain, Equalizer, Target};

use crate::settings::SpeakerLayout;

/// Settings → Music → Spatial audio.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpatialMode {
    #[default]
    Off,
    /// When the sound goes to headphones.
    Auto,
    On,
}

impl SpatialMode {
    pub const ALL: [SpatialMode; 3] = [SpatialMode::Off, SpatialMode::Auto, SpatialMode::On];
}

static MODE: AtomicU32 = AtomicU32::new(0);
static HEAD_TRACKING: AtomicBool = AtomicBool::new(false);
/// Settings → Sound / Music → Your speakers, as its index in `SpeakerLayout::ALL`.
static LAYOUT: AtomicU32 = AtomicU32::new(1);

pub fn set_mode(mode: SpatialMode, head_tracking: bool, layout: SpeakerLayout) {
    MODE.store(mode as u32, Ordering::Relaxed);
    HEAD_TRACKING.store(head_tracking, Ordering::Relaxed);
    LAYOUT.store(SpeakerLayout::ALL.iter().position(|known| *known == layout).unwrap_or(1) as u32, Ordering::Relaxed);
}

fn layout() -> SpeakerLayout {
    SpeakerLayout::ALL.get(LAYOUT.load(Ordering::Relaxed) as usize).copied().unwrap_or_default()
}

fn mode() -> SpatialMode {
    match MODE.load(Ordering::Relaxed) {
        1 => SpatialMode::Auto,
        2 => SpatialMode::On,
        _ => SpatialMode::Off,
    }
}

/// Where Music's next track goes when it plays itself, or `None` for VLC:
/// headphones (spatial audio) when they are on and it is wanted, else the
/// speakers of the layout when the sound device has that many channels
/// (stereo through a stereo device stays with VLC).
pub fn target() -> Option<Target> {
    if mode() == SpatialMode::Auto && headphones() {
        return Some(Target::Headphones);
    }
    match layout() {
        SpeakerLayout::Headphones => (mode() != SpatialMode::Off).then_some(Target::Headphones),
        speakers => {
            let count = crate::sound::speakers(speakers).len();
            (device_channels() >= count).then_some(Target::Speakers(count))
        }
    }
}

/// The most channels the system's sound device takes.
fn device_channels() -> usize {
    let Some(device) = cpal::default_host().default_output_device() else { return 0 };
    device.supported_output_configs().map_or(0, |configs| configs.map(|config| config.channels() as usize).max().unwrap_or(0))
}

/// The default device's stream settings, with at least `channels` channels
/// when it has them (surround devices often default to stereo).
fn device_config(channels: usize) -> Result<(cpal::Device, cpal::SupportedStreamConfig), String> {
    let device = cpal::default_host().default_output_device().ok_or("No sound output")?;
    let default = device.default_output_config().map_err(|error| error.to_string())?;
    if default.channels() as usize >= channels {
        return Ok((device, default));
    }
    let wider = device
        .supported_output_configs()
        .map_err(|error| error.to_string())?
        .filter(|config| config.channels() as usize >= channels && config.sample_format() == default.sample_format())
        .min_by_key(|config| config.channels());
    let config = match wider {
        Some(range) => {
            let rate = default.sample_rate().clamp(range.min_sample_rate(), range.max_sample_rate());
            range.with_sample_rate(rate)
        }
        None => default,
    };
    Ok((device, config))
}

/// Where WAV-ordered channel `index` (of `count`) goes on the device. ALSA
/// orders 5.1 and 7.1 L R RL RR C LFE (SL SR); the others, as WAV does.
fn device_slot(index: usize, count: usize) -> usize {
    if cfg!(target_os = "linux") && count >= 6 {
        const ALSA: [usize; 8] = [0, 1, 4, 5, 2, 3, 6, 7];
        return ALSA[index];
    }
    index
}

/// Settings → Sound / Music: a tone in one speaker of `layout`, through the
/// same output and channel order Music plays through.
pub fn test_speaker(layout: SpeakerLayout, index: usize, volume: u8) {
    let count = crate::sound::speakers(layout).len();
    let Some(speaker) = crate::sound::speakers(layout).get(index).copied() else { return };
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let (device, config) = device_config(count)?;
            let rate = config.sample_rate() as f32;
            let channels = config.channels() as usize;
            let slot = if channels >= count { device_slot(index, count) } else { index.min(channels.saturating_sub(1)) };
            let frequency = if speaker.short == "LFE" { 55.0 } else { 523.25 };
            let level = if speaker.short == "LFE" { 0.8 } else { 0.5 } * volume.min(100) as f32 / 100.0;
            let length = (rate * 1.2) as usize;
            let mut at = 0usize;
            let mut tone = move || {
                let fade = (at.min(length.saturating_sub(at)) as f32 / (rate * 0.05)).min(1.0);
                let sample = if at < length { (std::f32::consts::TAU * frequency * at as f32 / rate).sin() * level * fade } else { 0.0 };
                at += 1;
                sample
            };
            let stream = device
                .build_output_stream(
                    config.config(),
                    move |data: &mut [f32], _| {
                        for frame in data.chunks_mut(channels) {
                            let sample = tone();
                            frame.fill(0.0);
                            if let Some(target) = frame.get_mut(slot) {
                                *target = sample;
                            }
                        }
                    },
                    |error| {
                        files_sdk::log::record("music", files_sdk::log::Kind::Error, format!("Speaker test: {error}"), None);
                    },
                    None,
                )
                .map_err(|error| error.to_string())?;
            stream.play().map_err(|error| error.to_string())?;
            std::thread::sleep(std::time::Duration::from_millis(1400));
            Ok(())
        })();
        let detail = result.err();
        files_sdk::log::record("files", files_sdk::log::Kind::Setting, format!("speaker test: {} ({layout:?})", speaker.name), detail);
    });
}

/// The system's sound goes to headphones (or a headset, AirPods…).
pub fn headphones() -> bool {
    let Some(device) = cpal::default_host().default_output_device() else { return false };
    let Ok(description) = device.description() else { return false };
    matches!(description.device_type(), cpal::DeviceType::Headphones | cpal::DeviceType::Headset | cpal::DeviceType::Earpiece)
        || is_headphone_name(description.name())
}

/// Names that mean headphones when the system does not say (most Bluetooth ones).
fn is_headphone_name(name: &str) -> bool {
    const HINTS: [&str; 14] =
        ["headphone", "headset", "airpods", "earpods", "buds", "beats", "earphone", "wh-1000", "wf-1000", "freebuds", "jabra", "qc35", "qc45", "наушник"];
    let name = name.to_lowercase();
    HINTS.iter().any(|hint| name.contains(hint))
}

/// The decoded track, as 16-bit samples (half the memory of floats).
#[derive(Default)]
struct Decoded {
    rate: u32,
    channels: usize,
    samples: Vec<i16>,
    done: bool,
}

impl Decoded {
    fn frames(&self) -> u64 {
        self.samples.len().checked_div(self.channels).unwrap_or(0) as u64
    }
}

struct Shared {
    decoded: Mutex<Decoded>,
    /// Source frame playing now.
    position: AtomicU64,
    /// A seek waiting for the audio thread (`u64::MAX`: none).
    seek: AtomicU64,
    playing: AtomicBool,
    ended: AtomicBool,
    /// 0…1 as f32 bits.
    volume: AtomicU32,
    equalizer: Mutex<Option<(f32, Vec<f32>)>>,
    equalizer_changes: AtomicU64,
    /// The track's length from its listing (ms), until the decoder knows.
    expected_ms: i64,
    /// Whatever stopped the decoder.
    failed: Mutex<Option<String>>,
}

pub enum Source {
    File(std::path::PathBuf),
    /// An MP3 still downloading (Yandex Music).
    Arriving(Arc<Growing>),
}

pub struct SpatialPlayer {
    shared: Arc<Shared>,
    _stream: cpal::Stream,
}

impl SpatialPlayer {
    /// Starts decoding `source` and opens the audio device (paused).
    /// `enhance`: the exciter of Enhance streaming, for MP3 streams.
    pub fn open(source: Source, enhance: Option<Strength>, expected_ms: i64, target: Target) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            decoded: Mutex::new(Decoded::default()),
            position: AtomicU64::new(0),
            seek: AtomicU64::new(u64::MAX),
            playing: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            volume: AtomicU32::new(0.8f32.to_bits()),
            equalizer: Mutex::new(None),
            equalizer_changes: AtomicU64::new(0),
            expected_ms,
            failed: Mutex::new(None),
        });
        let decoding = shared.clone();
        std::thread::spawn(move || {
            let (input, extension): (Box<dyn files_ext_music::live::MediaSource>, Option<String>) = match source {
                Source::File(path) => match std::fs::File::open(&path) {
                    Ok(file) => (Box::new(file), path.extension().map(|extension| extension.to_string_lossy().into_owned())),
                    Err(error) => {
                        *lock(&decoding.failed) = Some(error.to_string());
                        lock(&decoding.decoded).done = true;
                        return;
                    }
                },
                Source::Arriving(mp3) => (files_ext_music::live::reader(mp3), Some("mp3".into())),
            };
            let result = files_ext_music::enhance::decode_stream(input, extension.as_deref(), |rate, channels, interleaved| {
                let mut decoded = lock(&decoding.decoded);
                if decoded.channels == 0 {
                    (decoded.rate, decoded.channels) = (rate, channels);
                }
                if channels == decoded.channels {
                    decoded.samples.extend(interleaved.iter().map(|sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16));
                }
                Ok(())
            });
            if let Err(error) = result {
                *lock(&decoding.failed) = Some(error.to_string());
            }
            lock(&decoding.decoded).done = true;
        });

        let (device, config) = device_config(target.channels())?;
        let format = config.sample_format();
        let stream_config = config.config();
        let mut render = Renderer::new(shared.clone(), stream_config.sample_rate, stream_config.channels as usize, enhance, target);
        let failed = |error: cpal::Error| {
            files_sdk::log::record("music", files_sdk::log::Kind::Error, format!("Spatial audio output: {error}"), None);
        };
        let stream = match format {
            cpal::SampleFormat::F32 => device.build_output_stream(stream_config, move |data: &mut [f32], _| render.fill(data, |sample| sample), failed, None),
            cpal::SampleFormat::I16 => device.build_output_stream(stream_config, move |data: &mut [i16], _| render.fill(data, |sample| (sample * i16::MAX as f32) as i16), failed, None),
            cpal::SampleFormat::I32 => device.build_output_stream(stream_config, move |data: &mut [i32], _| render.fill(data, |sample| (sample as f64 * i32::MAX as f64) as i32), failed, None),
            cpal::SampleFormat::U16 => device.build_output_stream(stream_config, move |data: &mut [u16], _| render.fill(data, |sample| ((sample + 1.0) * 0.5 * u16::MAX as f32) as u16), failed, None),
            other => return Err(format!("Unsupported sound output format {other:?}")),
        }
        .map_err(|error| error.to_string())?;
        stream.play().map_err(|error| error.to_string())?;
        Ok(Self { shared, _stream: stream })
    }

    pub fn play(&self) {
        if self.shared.ended.swap(false, Ordering::SeqCst) {
            self.shared.seek.store(0, Ordering::SeqCst);
        }
        self.shared.playing.store(true, Ordering::SeqCst);
    }

    pub fn pause(&self) {
        self.shared.playing.store(false, Ordering::SeqCst);
    }

    /// Milliseconds played and the whole length (the listing's until decoded).
    pub fn progress(&self) -> (i64, i64) {
        let decoded = lock(&self.shared.decoded);
        if decoded.rate == 0 {
            return (0, self.shared.expected_ms.max(0));
        }
        let ms = |frames: u64| (frames * 1000 / decoded.rate as u64) as i64;
        let pending = self.shared.seek.load(Ordering::SeqCst);
        let at = if pending != u64::MAX { pending } else { self.shared.position.load(Ordering::SeqCst) };
        let length = if decoded.done { ms(decoded.frames()) } else { self.shared.expected_ms.max(ms(decoded.frames())) };
        (ms(at), length)
    }

    /// Jumps to a fraction (0…1); past what has arrived, it waits for it.
    pub fn seek(&self, fraction: f32) {
        let (_, length) = self.progress();
        let rate = lock(&self.shared.decoded).rate as f64;
        let frame = (fraction.clamp(0.0, 1.0) as f64 * length as f64 / 1000.0 * rate) as u64;
        self.shared.ended.store(false, Ordering::SeqCst);
        self.shared.seek.store(frame, Ordering::SeqCst);
    }

    /// 0…100, as VLC's.
    pub fn set_volume(&self, volume: u8) {
        self.shared.volume.store((volume.min(100) as f32 / 100.0).to_bits(), Ordering::Relaxed);
    }

    pub fn set_equalizer(&self, equalizer: Option<&crate::video::Equalizer>) {
        *lock(&self.shared.equalizer) = equalizer.map(|equalizer| (equalizer.preamp, equalizer.bands.clone()));
        self.shared.equalizer_changes.fetch_add(1, Ordering::SeqCst);
    }

    pub fn ended(&self) -> bool {
        self.shared.ended.load(Ordering::SeqCst)
    }

    /// Why decoding stopped early, if it did.
    pub fn error(&self) -> Option<String> {
        lock(&self.shared.failed).clone()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The audio thread's side: source frames through the chain, resampled to
/// the device's rate, then the equalizer and the volume.
struct Renderer {
    shared: Arc<Shared>,
    out_rate: u32,
    out_channels: usize,
    enhance: Option<Strength>,
    target: Target,
    chain: Option<Chain>,
    equalizer: Option<Equalizer>,
    equalizer_seen: u64,
    /// Where between `previous` and `current` (rendered frames) the output is.
    phase: f64,
    previous: [f32; 8],
    current: [f32; 8],
    frame: Vec<f32>,
}

impl Renderer {
    fn new(shared: Arc<Shared>, out_rate: u32, out_channels: usize, enhance: Option<Strength>, target: Target) -> Self {
        Self {
            shared,
            out_rate,
            out_channels,
            enhance,
            target,
            chain: None,
            equalizer: None,
            equalizer_seen: u64::MAX,
            phase: 0.0,
            previous: [0.0; 8],
            current: [0.0; 8],
            frame: Vec::new(),
        }
    }

    fn fill<T: Copy + Default>(&mut self, data: &mut [T], convert: impl Fn(f32) -> T) {
        let channels = self.out_channels.max(1);
        let playing = self.shared.playing.load(Ordering::SeqCst);
        let decoded = lock(&self.shared.decoded);
        if !playing || decoded.channels == 0 {
            data.fill(convert(0.0));
            return;
        }
        let source_channels = decoded.channels;
        if self.chain.is_none() {
            self.chain = Some(Chain::new(decoded.rate, source_channels, self.enhance, self.target));
        }
        let seek = self.shared.seek.swap(u64::MAX, Ordering::SeqCst);
        if seek != u64::MAX {
            self.shared.position.store(seek, Ordering::SeqCst);
            // Fresh filters: no tail of where it was.
            self.chain = Some(Chain::new(decoded.rate, source_channels, self.enhance, self.target));
            (self.previous, self.current, self.phase) = ([0.0; 8], [0.0; 8], 0.0);
        }
        let changes = self.shared.equalizer_changes.load(Ordering::SeqCst);
        if changes != self.equalizer_seen {
            self.equalizer_seen = changes;
            self.equalizer = lock(&self.shared.equalizer).as_ref().map(|(preamp, bands)| Equalizer::new(self.out_rate, *preamp, bands, self.target.channels()));
        }
        let chain = self.chain.as_mut().expect("made above");
        chain.set_yaw(if HEAD_TRACKING.load(Ordering::Relaxed) { crate::head_tracking::yaw() } else { 0.0 });
        let step = decoded.rate as f64 / self.out_rate as f64;
        let volume = f32::from_bits(self.shared.volume.load(Ordering::Relaxed));
        let total = decoded.frames();
        let mut position = self.shared.position.load(Ordering::SeqCst);
        for out in data.chunks_mut(channels) {
            self.phase += step;
            let mut starved = false;
            while self.phase >= 1.0 {
                if position >= total {
                    starved = true;
                    break;
                }
                let start = position as usize * source_channels;
                self.frame.clear();
                self.frame.extend(decoded.samples[start..start + source_channels].iter().map(|sample| *sample as f32 / i16::MAX as f32));
                self.previous = self.current;
                chain.run(&self.frame, &mut self.current);
                position += 1;
                self.phase -= 1.0;
            }
            if starved {
                // The end, or a stream that has not caught up: silence, and wait.
                self.phase = self.phase.min(1.0);
                if decoded.done {
                    self.shared.ended.store(true, Ordering::SeqCst);
                    self.shared.playing.store(false, Ordering::SeqCst);
                }
                out.fill(convert(0.0));
                continue;
            }
            let part = self.phase as f32;
            let count = self.target.channels();
            let mut sample = [0.0f32; 8];
            for ((value, previous), current) in sample.iter_mut().zip(&self.previous).zip(&self.current).take(count) {
                *value = previous + (current - previous) * part;
            }
            if let Some(equalizer) = &mut self.equalizer {
                equalizer.run(&mut sample[..count]);
            }
            for value in &mut sample[..count] {
                *value = (*value * volume).clamp(-1.0, 1.0);
            }
            out.fill(convert(0.0));
            if out.len() >= count {
                for (index, value) in sample[..count].iter().enumerate() {
                    out[device_slot(index, count)] = convert(*value);
                }
            } else {
                // Fewer channels than the layout: everything to the first one or two.
                let (left, right) = (sample[0] + sample.get(2).copied().unwrap_or(0.0) * 0.7, sample[1] + sample.get(2).copied().unwrap_or(0.0) * 0.7);
                match out {
                    [mono] => *mono = convert(((left + right) * 0.5).clamp(-1.0, 1.0)),
                    [first, second, ..] => {
                        *first = convert(left.clamp(-1.0, 1.0));
                        *second = convert(right.clamp(-1.0, 1.0));
                    }
                    [] => {}
                }
            }
        }
        self.shared.position.store(position, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headphones_are_told_by_name_when_the_system_does_not_say() {
        assert!(is_headphone_name("AirPods Pro"));
        assert!(is_headphone_name("Headphones (Realtek(R) Audio)"));
        assert!(is_headphone_name("WH-1000XM5"));
        assert!(!is_headphone_name("MacBook Pro Speakers"));
        assert!(!is_headphone_name("LG HDR 4K"));
    }

    /// Renders `frames` output frames from `decoded` without a sound device.
    fn render(decoded: Decoded, out_rate: u32, frames: usize, seek: Option<u64>) -> (Vec<f32>, Arc<Shared>) {
        let shared = Arc::new(Shared {
            decoded: Mutex::new(decoded),
            position: AtomicU64::new(0),
            seek: AtomicU64::new(seek.unwrap_or(u64::MAX)),
            playing: AtomicBool::new(true),
            ended: AtomicBool::new(false),
            volume: AtomicU32::new(1.0f32.to_bits()),
            equalizer: Mutex::new(None),
            equalizer_changes: AtomicU64::new(0),
            expected_ms: 0,
            failed: Mutex::new(None),
        });
        let mut renderer = Renderer::new(shared.clone(), out_rate, 2, None, Target::Headphones);
        let mut out = vec![0.0f32; frames * 2];
        renderer.fill(&mut out, |sample| sample);
        (out, shared)
    }

    fn tone(rate: u32, seconds: f32) -> Decoded {
        let samples = (0..(rate as f32 * seconds) as usize).flat_map(|index| {
            let value = ((std::f32::consts::TAU * 440.0 * index as f32 / rate as f32).sin() * 0.5 * i16::MAX as f32) as i16;
            [value, value]
        });
        Decoded { rate, channels: 2, samples: samples.collect(), done: true }
    }

    #[test]
    fn a_44k_track_plays_on_a_48k_device_at_its_own_speed() {
        let (out, shared) = render(tone(44_100, 1.0), 48_000, 24_000, None);
        assert!(out.iter().any(|sample| sample.abs() > 0.1), "sound comes out");
        let played = shared.position.load(Ordering::SeqCst);
        assert!((played as i64 - 22_050).abs() <= 2, "half a second of 44.1 kHz in half a second of 48 kHz: {played}");
    }

    #[test]
    fn the_end_stops_and_says_so() {
        let (out, shared) = render(tone(48_000, 0.1), 48_000, 9_600, None);
        assert!(shared.ended.load(Ordering::SeqCst) && !shared.playing.load(Ordering::SeqCst));
        assert!(out[out.len() - 100..].iter().all(|sample| *sample == 0.0), "silence after the end");
    }

    #[test]
    fn a_seek_jumps_there() {
        let (_, shared) = render(tone(48_000, 2.0), 48_000, 480, Some(48_000));
        assert_eq!(shared.position.load(Ordering::SeqCst), 48_480);
    }

    #[test]
    fn a_stream_that_has_not_arrived_waits_in_silence() {
        let mut decoded = tone(48_000, 0.01);
        decoded.done = false;
        let (out, shared) = render(decoded, 48_000, 4_800, None);
        assert!(!shared.ended.load(Ordering::SeqCst) && shared.playing.load(Ordering::SeqCst), "still playing, waiting");
        assert!(out[out.len() - 100..].iter().all(|sample| *sample == 0.0));
    }
}

#[cfg(test)]
mod device {
    use super::*;

    /// Plays a file on the real sound device, silently (`cargo test -- --ignored`).
    #[test]
    #[ignore]
    fn plays_on_the_sound_device() {
        let path = std::env::var("FILES_SPATIAL_TEST").unwrap_or_else(|_| "/private/tmp/claude-501/sp.mp3".into());
        let player = SpatialPlayer::open(Source::File(path.into()), Some(Strength::Medium), 10_000, Target::Headphones).unwrap();
        player.set_volume(0);
        player.play();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let (at, length) = player.progress();
        println!("after 1.5 s: at {at} ms of {length} ms; headphones: {}", headphones());
        assert!((1200..=1700).contains(&at), "plays in real time: {at}");
        player.seek(0.8);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let (at, _) = player.progress();
        println!("after seeking to 80%: at {at} ms");
        assert!((8000..=8500).contains(&at));
        std::thread::sleep(std::time::Duration::from_millis(2200));
        assert!(player.ended(), "ended at the end");
    }
}
