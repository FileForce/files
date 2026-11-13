//! Enhance and upmix audio files into FLAC, all in-process (no ffmpeg):
//! Symphonia decodes, a harmonic exciter brings back the top end lossy
//! encoders cut, the loudness is lifted towards -14 LUFS without clipping,
//! stereo can be spread over 5.1, and flacenc writes 24-bit FLAC.
//!
//! An enhanced MP3 is not the lost original: the exciter makes new highs
//! from what is left, it cannot bring back what the encoder threw away.

use std::io;
use std::path::{Path, PathBuf};

use files_sdk::quick_actions::{Applies, Field, Outcome, QuickAction, QuickActionProvider, answer};

/// What Symphonia here can read.
const READS: [&str; 6] = ["mp3", "m4a", "aac", "ogg", "wav", "flac"];

/// Loudness the quiet tracks are lifted to (streaming services use it too).
const TARGET_LUFS: f64 = -14.0;
/// Highest true peak after the lift, in dBTP.
const CEILING_DB: f64 = -1.0;
/// Never lift by more than this, so a quiet intro or a field recording stays quiet.
const MAX_LIFT_DB: f64 = 12.0;

/// Decoded audio: one Vec per channel.
pub struct Audio {
    pub rate: u32,
    pub channels: Vec<Vec<f32>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Strength {
    Low,
    Medium,
    High,
}

impl Strength {
    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    fn parse(choice: Option<&str>) -> Self {
        match choice {
            Some("Low") => Self::Low,
            Some("High") => Self::High,
            _ => Self::Medium,
        }
    }

    /// How much of the generated harmonics goes into the mix.
    fn amount(self) -> f32 {
        match self {
            Self::Low => 0.12,
            Self::Medium => 0.25,
            Self::High => 0.45,
        }
    }
}

pub fn decode(path: &Path) -> io::Result<Audio> {
    let mut audio = Audio { rate: 0, channels: Vec::new() };
    let extension = path.extension().and_then(|extension| extension.to_str());
    decode_stream(Box::new(std::fs::File::open(path)?), extension, |rate, count, interleaved| {
        if audio.channels.is_empty() {
            audio.rate = rate;
            audio.channels = vec![Vec::new(); count];
        }
        if count == audio.channels.len() {
            for frame in interleaved.chunks_exact(count) {
                for (channel, sample) in audio.channels.iter_mut().zip(frame) {
                    channel.push(*sample);
                }
            }
        }
        Ok(())
    })?;
    if audio.channels.is_empty() || audio.channels[0].is_empty() {
        return Err(io::Error::other("The file has no audio"));
    }
    Ok(audio)
}

/// Decodes `source` packet by packet, handing each one's interleaved
/// samples to `each` with the rate and channel count: a source still
/// arriving (a stream) is decoded as it comes.
pub fn decode_stream(
    source: Box<dyn symphonia::core::io::MediaSource>,
    extension: Option<&str>,
    mut each: impl FnMut(u32, usize, &[f32]) -> io::Result<()>,
) -> io::Result<()> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::errors::Error;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let failed = |what: &str, error: Error| io::Error::other(format!("{what}: {error}"));
    let source = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = extension {
        hint.with_extension(extension);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, source, FormatOptions::default(), MetadataOptions::default())
        .map_err(|error| failed("Not a readable audio file", error))?;
    let track = format.default_track(TrackType::Audio).ok_or_else(|| io::Error::other("No audio in the file"))?;
    let track_id = track.id;
    let params = track.codec_params.as_ref().and_then(|params| params.audio()).ok_or_else(|| io::Error::other("No audio in the file"))?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|error| failed("Unsupported audio", error))?;

    let mut interleaved: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            // A truncated file ends where it breaks off.
            Err(Error::IoError(_)) => break,
            Err(error) => return Err(failed("Reading failed", error)),
        };
        if packet.track_id != track_id {
            continue;
        }
        let buffer = match decoder.decode(&packet) {
            Ok(buffer) => buffer,
            Err(Error::DecodeError(_)) => continue,
            Err(error) => return Err(failed("Decoding failed", error)),
        };
        interleaved.resize(buffer.samples_interleaved(), 0.0);
        buffer.copy_to_slice_interleaved(&mut interleaved);
        each(buffer.spec().rate(), buffer.spec().channels().count(), &interleaved)?;
    }
    Ok(())
}

/// RBJ biquad, one per channel and stage.
#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn new(rate: u32, cutoff: f32, high: bool) -> Self {
        let w = std::f32::consts::TAU * cutoff.min(rate as f32 * 0.45) / rate as f32;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / std::f32::consts::SQRT_2; // Q = 0.707: Butterworth
        let a0 = 1.0 + alpha;
        let (b0, b1) = if high { ((1.0 + cos) / 2.0, -(1.0 + cos)) } else { ((1.0 - cos) / 2.0, 1.0 - cos) };
        Self { b0: b0 / a0, b1: b1 / a0, b2: b0 / a0, a1: -2.0 * cos / a0, a2: (1.0 - alpha) / a0, x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }

    fn highpass(rate: u32, cutoff: f32) -> Self {
        Self::new(rate, cutoff, true)
    }

    fn lowpass(rate: u32, cutoff: f32) -> Self {
        Self::new(rate, cutoff, false)
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        (self.x2, self.x1, self.y2, self.y1) = (self.x1, x, self.y1, y);
        y
    }
}

/// Harmonic exciter for one channel: the band over 3 kHz is saturated,
/// which makes overtones above it; only those new highs (over 7 kHz) are
/// mixed back in. Keeps its filters' state, so a stream goes through in pieces.
pub struct Exciter {
    band: [Biquad; 2],
    top: [Biquad; 2],
    amount: f32,
}

impl Exciter {
    const DRIVE: f32 = 6.0;

    pub fn new(rate: u32, strength: Strength) -> Self {
        Self {
            band: [Biquad::highpass(rate, 3000.0), Biquad::highpass(rate, 3000.0)],
            top: [Biquad::highpass(rate, 7000.0), Biquad::highpass(rate, 7000.0)],
            amount: strength.amount(),
        }
    }

    pub fn run(&mut self, sample: f32) -> f32 {
        let mid = self.band.iter_mut().fold(sample, |x, filter| filter.run(x));
        let harmonics = (mid * Self::DRIVE).tanh() / Self::DRIVE;
        let new = self.top.iter_mut().fold(harmonics, |x, filter| filter.run(x));
        sample + self.amount * new
    }
}

fn excite(channel: &mut [f32], rate: u32, strength: Strength) {
    let mut exciter = Exciter::new(rate, strength);
    for sample in channel.iter_mut() {
        *sample = exciter.run(*sample);
    }
}

/// The gain in dB that lifts the track towards [`TARGET_LUFS`] with its
/// true peak at most [`CEILING_DB`]; loud tracks are only turned down if
/// they would clip.
fn lift_db(audio: &Audio) -> f64 {
    use ebur128::{EbuR128, Mode};
    let count = audio.channels.len() as u32;
    let Ok(mut meter) = EbuR128::new(count, audio.rate, Mode::I | Mode::TRUE_PEAK) else { return 0.0 };
    let mut frames = Vec::with_capacity(audio.rate as usize * count as usize);
    for start in (0..audio.channels[0].len()).step_by(audio.rate as usize) {
        let end = (start + audio.rate as usize).min(audio.channels[0].len());
        frames.clear();
        for index in start..end {
            frames.extend(audio.channels.iter().map(|channel| channel[index]));
        }
        let _ = meter.add_frames_f32(&frames);
    }
    let loudness = meter.loudness_global().unwrap_or(f64::NEG_INFINITY);
    let peak = (0..count).filter_map(|channel| meter.true_peak(channel).ok()).fold(0.0, f64::max);
    let peak_db = if peak > 0.0 { 20.0 * peak.log10() } else { f64::NEG_INFINITY };
    let wanted = if loudness.is_finite() { (TARGET_LUFS - loudness).clamp(0.0, MAX_LIFT_DB) } else { 0.0 };
    wanted.min(CEILING_DB - peak_db)
}

fn apply_gain(audio: &mut Audio, db: f64) {
    let gain = 10f64.powf(db / 20.0) as f32;
    for sample in audio.channels.iter_mut().flatten() {
        *sample *= gain;
    }
}

/// Exciter on every channel, then the loudness lift.
pub fn enhance(audio: &mut Audio, strength: Strength) {
    for channel in &mut audio.channels {
        excite(channel, audio.rate, strength);
    }
    let lift = lift_db(audio);
    apply_gain(audio, lift);
}

/// Stereo (or mono) to 5.1 in FLAC's order: FL FR FC LFE BL BR.
/// Fronts keep the mix, the centre takes the shared voice band, the LFE
/// the shared lows, and the rears the delayed, darker difference (ambience),
/// as passive surround decoders do.
pub fn upmix(audio: &Audio) -> io::Result<Audio> {
    let (left, right) = match audio.channels.as_slice() {
        [mono] => (mono, mono),
        [left, right] => (left, right),
        _ => return Err(io::Error::other("Already more than stereo")),
    };
    let rate = audio.rate;
    let length = left.len();
    let delay = (rate as usize * 12) / 1000;
    let mut centre = [Biquad::highpass(rate, 120.0), Biquad::lowpass(rate, 8000.0)];
    let mut lfe = [Biquad::lowpass(rate, 120.0), Biquad::lowpass(rate, 120.0)];
    let mut rear = [Biquad::lowpass(rate, 7000.0), Biquad::highpass(rate, 100.0)];
    let mut out: Vec<Vec<f32>> = (0..6).map(|_| Vec::with_capacity(length)).collect();
    let mut ambience = vec![0.0f32; length];
    for index in 0..length {
        let (l, r) = (left[index], right[index]);
        let mid = (l + r) * 0.5;
        let side = (l - r) * 0.5;
        let voice = centre.iter_mut().fold(mid, |x, filter| filter.run(x));
        out[0].push(l - 0.5 * voice);
        out[1].push(r - 0.5 * voice);
        out[2].push(0.7 * voice);
        out[3].push(lfe.iter_mut().fold(mid, |x, filter| filter.run(x)));
        ambience[index] = rear.iter_mut().fold(side, |x, filter| filter.run(x));
    }
    for index in 0..length {
        let back = if index >= delay { ambience[index - delay] } else { 0.0 };
        out[4].push(0.7 * back);
        out[5].push(-0.7 * back);
    }
    let mut upmixed = Audio { rate, channels: out };
    // Keep the loudest channel under full scale.
    let peak = upmixed.channels.iter().flatten().fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    if peak > 0.97 {
        apply_gain(&mut upmixed, 20.0 * (0.97 / peak as f64).log10());
    }
    Ok(upmixed)
}

/// 24-bit FLAC.
pub fn encode_flac(audio: &Audio, to: &Path) -> io::Result<()> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    const BITS: usize = 24;
    let scale = ((1 << (BITS - 1)) - 1) as f32;
    let count = audio.channels.len();
    let mut samples = Vec::with_capacity(count * audio.channels[0].len());
    for index in 0..audio.channels[0].len() {
        samples.extend(audio.channels.iter().map(|channel| (channel[index].clamp(-1.0, 1.0) * scale).round() as i32));
    }
    let config = flacenc::config::Encoder::default().into_verified().map_err(|(_, error)| io::Error::other(format!("FLAC settings: {error:?}")))?;
    let source = flacenc::source::MemSource::from_samples(&samples, count, BITS, audio.rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size).map_err(|error| io::Error::other(format!("FLAC encoding failed: {error:?}")))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|error| io::Error::other(format!("FLAC encoding failed: {error:?}")))?;
    let mut bytes = sink.as_slice().to_vec();
    // "fLaC", STREAMINFO's 4-byte header (last-block flag, type, length 34),
    // then min (u16) and max (u16) block size.
    const STREAMINFO_END: usize = 4 + 4 + 34;
    if bytes.len() > STREAMINFO_END && bytes.starts_with(b"fLaC") {
        // flacenc gives the short last block's size as the minimum; the spec
        // leaves the last block out, and Symphonia (our player) refuses it.
        bytes.copy_within(10..12, 8);
        // STREAMINFO is no longer last: padding is, room for the tags (as
        // the reference encoder leaves; lofty writes the tags before it).
        const PADDING: usize = 8192;
        bytes[4] &= 0x7f;
        let mut padding = vec![0x80 | 1, (PADDING >> 16) as u8, (PADDING >> 8) as u8, PADDING as u8];
        padding.resize(4 + PADDING, 0);
        bytes.splice(STREAMINFO_END..STREAMINFO_END, padding);
    }
    std::fs::write(to, bytes)
}

/// Title, artist, album and cover go along to the new file.
fn copy_tags(from: &Path, to: &Path) {
    use lofty::config::WriteOptions;
    use lofty::file::TaggedFileExt as _;
    use lofty::tag::{TagExt as _, TagType};
    let Ok(file) = lofty::read_from_path(from) else { return };
    let Some(mut tag) = file.primary_tag().or_else(|| file.first_tag()).cloned() else { return };
    tag.re_map(TagType::VorbisComments);
    let _ = tag.save_to_path(to, WriteOptions::default());
}

/// `song.mp3` → `song.enhanced.flac` beside it, never over an existing file.
fn target(path: &Path, suffix: &str) -> PathBuf {
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let target = path.with_file_name(format!("{stem}.{suffix}.flac"));
    if target.exists() { files_sdk::fs::unique_path(&target) } else { target }
}

/// Decodes `path`, enhances and/or upmixes it, writes FLAC beside it.
pub fn process(path: &Path, enhance_with: Option<Strength>, surround: bool) -> io::Result<PathBuf> {
    let mut audio = decode(path)?;
    if let Some(strength) = enhance_with {
        enhance(&mut audio, strength);
    }
    if surround {
        audio = upmix(&audio)?;
    }
    let suffix = match (enhance_with.is_some(), surround) {
        (true, true) => "enhanced.5.1",
        (true, false) => "enhanced",
        _ => "5.1",
    };
    let to = target(path, suffix);
    encode_flac(&audio, &to)?;
    copy_tags(path, &to);
    Ok(to)
}

pub struct Actions;

pub static ACTIONS: Actions = Actions;

impl QuickActionProvider for Actions {
    fn actions(&self) -> Vec<QuickAction> {
        vec![
            QuickAction {
                id: "enhance-flac".into(),
                title: "Enhance to FLAC…".into(),
                applies: Applies::to(&["mp3", "m4a", "aac", "ogg"]),
                order: 40,
                ask: vec![
                    Field::Choice { key: "strength".into(), label: "Enhance".into(), options: ["Medium", "Low", "High"].map(String::from).to_vec() },
                    Field::Choice { key: "channels".into(), label: "Channels".into(), options: ["Stereo", "5.1 surround"].map(String::from).to_vec() },
                ],
            },
            QuickAction { id: "upmix-surround".into(), title: "Upmix to 5.1 Surround (FLAC)".into(), applies: Applies::to(&READS), order: 41, ask: Vec::new() },
        ]
    }

    fn run(&self, id: &str, selection: &[PathBuf], answers: &[(String, String)]) -> io::Result<Outcome> {
        let (enhance_with, surround) = match id {
            "enhance-flac" => (Some(Strength::parse(answer(answers, "strength"))), answer(answers, "channels") == Some("5.1 surround")),
            "upmix-surround" => (None, true),
            _ => return Err(io::Error::new(io::ErrorKind::NotFound, format!("No action {id}"))),
        };
        // A few tracks at a time: each is a whole song in memory.
        let workers = std::thread::available_parallelism().map_or(2, |count| count.get()).min(4);
        let results: Vec<io::Result<PathBuf>> = std::thread::scope(|scope| {
            selection
                .chunks(selection.len().div_ceil(workers).max(1))
                .map(|chunk| scope.spawn(move || chunk.iter().map(|path| process(path, enhance_with, surround)).collect::<Vec<_>>()))
                .collect::<Vec<_>>()
                .into_iter()
                .flat_map(|worker| worker.join().unwrap_or_else(|_| vec![Err(io::Error::other("Processing crashed"))]))
                .collect()
        });
        let mut made = Vec::new();
        let mut failed = Vec::new();
        for (path, result) in selection.iter().zip(results) {
            match result {
                Ok(to) => made.push(to),
                Err(error) => failed.push(format!("{}: {error}", path.file_name().unwrap_or_default().to_string_lossy())),
            }
        }
        if made.is_empty() {
            return Err(io::Error::other(failed.join("; ")));
        }
        let mut summary = format!("{} FLAC file{} made", made.len(), if made.len() == 1 { "" } else { "s" });
        if !failed.is_empty() {
            summary.push_str(&format!(", {} failed ({})", failed.len(), failed.join("; ")));
        }
        Ok(Outcome { summary, made })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, seconds: f32, frequency: f32, level: f32) -> Vec<f32> {
        (0..(rate as f32 * seconds) as usize).map(|index| level * (std::f32::consts::TAU * frequency * index as f32 / rate as f32).sin()).collect()
    }

    /// Energy above `cutoff`, through a steep high-pass.
    fn highs(samples: &[f32], rate: u32, cutoff: f32) -> f32 {
        let mut filters = [Biquad::highpass(rate, cutoff); 4];
        samples.iter().map(|sample| filters.iter_mut().fold(*sample, |x, filter| filter.run(x)).powi(2)).sum()
    }

    #[test]
    fn the_exciter_adds_highs_a_lossy_file_lost() {
        let rate = 44_100;
        let original = tone(rate, 1.0, 4000.0, 0.5);
        let mut excited = original.clone();
        excite(&mut excited, rate, Strength::Medium);
        assert!(highs(&excited, rate, 10_000.0) > 10.0 * highs(&original, rate, 10_000.0), "overtones of 4 kHz appear above 10 kHz");
    }

    #[test]
    fn quiet_tracks_are_lifted_but_never_clip() {
        let rate = 44_100;
        let mut quiet = Audio { rate, channels: vec![tone(rate, 3.0, 440.0, 0.05); 2] };
        let lift = lift_db(&quiet);
        assert!(lift > 6.0 && lift <= MAX_LIFT_DB, "lifted {lift} dB");
        apply_gain(&mut quiet, lift);
        let peak = quiet.channels.iter().flatten().fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        assert!(peak < 0.9, "peak {peak}");
        let loud = Audio { rate, channels: vec![tone(rate, 3.0, 440.0, 0.99); 2] };
        assert!(lift_db(&loud) <= 0.0, "a loud track is not made louder");
    }

    #[test]
    fn upmix_puts_shared_sound_in_the_centre_and_differences_behind() {
        let rate = 44_100;
        let voice = tone(rate, 1.0, 1000.0, 0.5);
        let centred = upmix(&Audio { rate, channels: vec![voice.clone(), voice.clone()] }).unwrap();
        assert_eq!(centred.channels.len(), 6);
        let energy = |channel: &[f32]| channel.iter().map(|sample| sample * sample).sum::<f32>();
        assert!(energy(&centred.channels[2]) > 10.0 * energy(&centred.channels[4]), "the same sound in both is the centre's");
        let wide = upmix(&Audio { rate, channels: vec![voice.clone(), voice.iter().map(|sample| -sample).collect()] }).unwrap();
        assert!(energy(&wide.channels[4]) > 10.0 * energy(&wide.channels[2]), "opposite sound is ambience, behind");
    }

    #[test]
    fn a_track_round_trips_through_flac() {
        let dir = std::env::temp_dir().join(format!("files-music-enhance-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let to = dir.join("tone.flac");
        let rate = 48_000;
        // 24,000 frames: the last FLAC block is a short one.
        let audio = Audio { rate, channels: vec![tone(rate, 0.5, 440.0, 0.5), tone(rate, 0.5, 660.0, 0.5)] };
        let tagged = Audio { rate, channels: audio.channels.clone() };
        encode_flac(&tagged, &to).unwrap();
        let mut tag = lofty::tag::Tag::new(lofty::tag::TagType::VorbisComments);
        lofty::tag::Accessor::set_title(&mut tag, "Tone".into());
        lofty::tag::TagExt::save_to_path(&tag, &to, lofty::config::WriteOptions::default()).unwrap();
        let back = decode(&to).unwrap();
        let read = lofty::read_from_path(&to).unwrap();
        assert_eq!(lofty::file::TaggedFileExt::primary_tag(&read).and_then(|tag| lofty::tag::Accessor::title(tag)).as_deref(), Some("Tone"), "tags written after the encoder are found");
        assert_eq!((back.rate, back.channels.len(), back.channels[0].len()), (rate, 2, audio.channels[0].len()));
        assert!((back.channels[1][100] - audio.channels[1][100]).abs() < 1e-4, "24-bit keeps the samples");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
