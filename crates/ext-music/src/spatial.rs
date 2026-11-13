//! Spatial audio for headphones, made here so it works the same on macOS,
//! Windows and Linux: stereo is spread over 5.1 (see [`Upmixer`]), and each
//! of the six virtual speakers is rendered for the two ears through a
//! structural head model (Brown & Duda, 1998): the delay between the ears and
//! the shadow of the head, both computed for any angle — so the whole scene
//! can turn against the head when the listener's head turns (head tracking).
//! A faint room (a few early reflections) helps the sound leave the head.

use crate::enhance::Biquad;

/// Head radius (m) and the speed of sound (m/s) of the model.
const HEAD_RADIUS: f32 = 0.0875;
const SOUND_SPEED: f32 = 343.0;

/// Streaming stereo (or mono) → 5.1, FLAC's order FL FR FC LFE BL BR — the
/// same split as [`crate::enhance::upmix`], a frame at a time.
pub struct Upmixer {
    centre: [Biquad; 2],
    lfe: [Biquad; 2],
    rear: [Biquad; 2],
    delay: Vec<f32>,
    at: usize,
}

impl Upmixer {
    pub fn new(rate: u32) -> Self {
        Self {
            centre: [Biquad::highpass(rate, 120.0), Biquad::lowpass(rate, 8000.0)],
            lfe: [Biquad::lowpass(rate, 120.0), Biquad::lowpass(rate, 120.0)],
            rear: [Biquad::lowpass(rate, 7000.0), Biquad::highpass(rate, 100.0)],
            // The rears come 12 ms late, as in passive surround decoders.
            delay: vec![0.0; (rate as usize * 12 / 1000).max(1)],
            at: 0,
        }
    }

    pub fn run(&mut self, left: f32, right: f32) -> [f32; 6] {
        let mid = (left + right) * 0.5;
        let side = (left - right) * 0.5;
        let voice = self.centre.iter_mut().fold(mid, |x, filter| filter.run(x));
        let lfe = self.lfe.iter_mut().fold(mid, |x, filter| filter.run(x));
        let ambience = self.rear.iter_mut().fold(side, |x, filter| filter.run(x));
        let back = std::mem::replace(&mut self.delay[self.at], ambience);
        self.at = (self.at + 1) % self.delay.len();
        [left - 0.5 * voice, right - 0.5 * voice, 0.7 * voice, lfe, 0.7 * back, -0.7 * back]
    }
}

/// One-pole one-zero head-shadow filter of the model: a boost of the highs
/// facing the source (up to +6 dB), a cut behind the head (to −20 dB).
#[derive(Clone, Copy, Default)]
struct Shadow {
    b0: f32,
    b1: f32,
    a1: f32,
    x1: f32,
    y1: f32,
}

impl Shadow {
    /// `alpha` from [`shadow_alpha`]; `k` is rate / ω0.
    fn set(&mut self, alpha: f32, k: f32) {
        let a0 = 1.0 + k;
        self.b0 = (1.0 + alpha * k) / a0;
        self.b1 = (1.0 - alpha * k) / a0;
        self.a1 = (1.0 - k) / a0;
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 - self.a1 * self.y1;
        (self.x1, self.y1) = (x, y);
        y
    }
}

/// The head shadow's high-frequency gain for a source `theta` radians off
/// the ear's axis (0: straight at the ear, π: opposite it).
fn shadow_alpha(theta: f32) -> f32 {
    const ALPHA_MIN: f32 = 0.1;
    const THETA_MIN: f32 = 150.0 * std::f32::consts::PI / 180.0;
    (1.0 + ALPHA_MIN / 2.0) + (1.0 - ALPHA_MIN / 2.0) * (theta / THETA_MIN * std::f32::consts::PI).cos()
}

/// How late the sound reaches an ear `theta` radians off its axis, in
/// seconds, the earliest ear at 0 (Woodworth's formula as the model uses it).
fn ear_delay(theta: f32) -> f32 {
    let half = std::f32::consts::FRAC_PI_2;
    let path = if theta < half { 1.0 - theta.cos() } else { 1.0 + theta - half };
    HEAD_RADIUS / SOUND_SPEED * path
}

/// The angle (0…π) between a source at `azimuth` and an ear at `ear`
/// (radians, positive to the left).
fn off_axis(azimuth: f32, ear: f32) -> f32 {
    let difference = (azimuth - ear).rem_euclid(std::f32::consts::TAU);
    if difference > std::f32::consts::PI { std::f32::consts::TAU - difference } else { difference }
}

/// A virtual speaker: where it stands, and its recent samples for the delays.
struct Speaker {
    azimuth: f32,
    history: [f32; 64],
    at: usize,
    /// Per ear (left, right): the shadow filter, and the delay in samples
    /// now and where it is going (glided over a block, so a turning head
    /// does not click).
    shadow: [Shadow; 2],
    delay: [f32; 2],
    target: [f32; 2],
}

/// Renders 5.1 for headphones; [`Binaural::set_yaw`] turns the scene.
pub struct Binaural {
    rate: f32,
    speakers: Vec<Speaker>,
    yaw: f32,
    /// Frames since the angles were last worked out.
    since: usize,
    room: Room,
}

/// The virtual speakers' azimuths in degrees (ITU 5.1: fronts ±30°, rears ±110°).
const LAYOUT: [(usize, f32); 5] = [(0, 30.0), (1, -30.0), (2, 0.0), (4, 110.0), (5, -110.0)];
/// Angles are worked out again every this many frames (~1.3 ms at 48 kHz).
const BLOCK: usize = 64;
/// The LFE reaches both ears alike (it has no direction).
const LFE_GAIN: f32 = 0.5;
/// Keeps the sum of five speakers near the level of the stereo it came from.
const OUTPUT_GAIN: f32 = 0.6;

impl Binaural {
    pub fn new(rate: u32) -> Self {
        let speakers = LAYOUT
            .iter()
            .map(|&(_, degrees)| Speaker {
                azimuth: degrees.to_radians(),
                history: [0.0; 64],
                at: 0,
                shadow: [Shadow::default(); 2],
                delay: [0.0; 2],
                target: [0.0; 2],
            })
            .collect();
        let mut binaural = Self { rate: rate as f32, speakers, yaw: 0.0, since: 0, room: Room::new(rate) };
        binaural.aim(true);
        binaural
    }

    /// The listener's head turned `yaw` radians to the left of where it
    /// faced at the start: the speakers stay put in the room.
    pub fn set_yaw(&mut self, yaw: f32) {
        self.yaw = yaw;
    }

    fn aim(&mut self, at_once: bool) {
        let k = self.rate / (SOUND_SPEED / HEAD_RADIUS);
        let ears = [std::f32::consts::FRAC_PI_2, -std::f32::consts::FRAC_PI_2];
        for speaker in &mut self.speakers {
            let azimuth = speaker.azimuth - self.yaw;
            for (ear, &axis) in ears.iter().enumerate() {
                let theta = off_axis(azimuth, axis);
                speaker.shadow[ear].set(shadow_alpha(theta), k);
                speaker.target[ear] = ear_delay(theta) * self.rate;
                if at_once {
                    speaker.delay[ear] = speaker.target[ear];
                }
            }
        }
    }

    /// One 5.1 frame (FL FR FC LFE BL BR) in, (left ear, right ear) out.
    pub fn run(&mut self, frame: [f32; 6]) -> [f32; 2] {
        if self.since == 0 {
            self.aim(false);
        }
        self.since = (self.since + 1) % BLOCK;
        let glide = 1.0 / BLOCK as f32;
        let mut ears = [0.0f32; 2];
        for (speaker, &(channel, _)) in self.speakers.iter_mut().zip(LAYOUT.iter()) {
            speaker.history[speaker.at] = frame[channel];
            #[allow(clippy::needless_range_loop)] // four arrays share the ear index
            for ear in 0..2 {
                speaker.delay[ear] += (speaker.target[ear] - speaker.delay[ear]) * glide;
                // Fractional delay: between the two nearest samples.
                let delay = speaker.delay[ear].clamp(0.0, 62.0);
                let whole = delay as usize;
                let part = delay - whole as f32;
                let newer = speaker.history[(speaker.at + 64 - whole) % 64];
                let older = speaker.history[(speaker.at + 64 - whole - 1) % 64];
                let delayed = newer + (older - newer) * part;
                ears[ear] += speaker.shadow[ear].run(delayed);
            }
            speaker.at = (speaker.at + 1) % 64;
        }
        let lfe = frame[3] * LFE_GAIN;
        let dry = [(ears[0] + lfe) * OUTPUT_GAIN, (ears[1] + lfe) * OUTPUT_GAIN];
        let wet = self.room.run(dry);
        [dry[0] + wet[0], dry[1] + wet[1]]
    }
}

/// A few early reflections, each ear hearing the other side a little late
/// and darker: the cue that puts the sound outside the head.
struct Room {
    line: Vec<[f32; 2]>,
    at: usize,
    taps: [(usize, f32, bool); 4],
    tone: [Biquad; 2],
}

impl Room {
    fn new(rate: u32) -> Self {
        let samples = |ms: f32| (rate as f32 * ms / 1000.0) as usize;
        // (delay, gain, crossed to the other ear)
        let taps = [(samples(7.0), 0.18, true), (samples(11.0), 0.14, false), (samples(17.0), 0.11, true), (samples(23.0), 0.08, false)];
        let length = taps.iter().map(|tap| tap.0).max().unwrap_or(1) + 1;
        Self { line: vec![[0.0; 2]; length], at: 0, taps, tone: [Biquad::lowpass(rate, 6000.0), Biquad::lowpass(rate, 6000.0)] }
    }

    fn run(&mut self, dry: [f32; 2]) -> [f32; 2] {
        let length = self.line.len();
        self.line[self.at] = dry;
        let mut wet = [0.0f32; 2];
        for &(delay, gain, crossed) in &self.taps {
            let past = self.line[(self.at + length - delay) % length];
            let (left, right) = if crossed { (past[1], past[0]) } else { (past[0], past[1]) };
            wet[0] += gain * left;
            wet[1] += gain * right;
        }
        self.at = (self.at + 1) % length;
        [self.tone[0].run(wet[0]), self.tone[1].run(wet[1])]
    }
}

/// The equalizer's band centres in Hz, as VLC's (so the presets match).
pub const EQUALIZER_BANDS: [f32; 10] = [60.0, 170.0, 310.0, 600.0, 1000.0, 3000.0, 6000.0, 12000.0, 14000.0, 16000.0];

/// A 10-band equalizer (peaking filters) with a preamp, in dB like VLC's,
/// for `channels` channels.
pub struct Equalizer {
    /// Per channel, the bands that are not flat.
    bands: Vec<Vec<Biquad>>,
    preamp: f32,
}

impl Equalizer {
    pub fn new(rate: u32, preamp_db: f32, gains_db: &[f32], channels: usize) -> Self {
        let filters: Vec<Biquad> = EQUALIZER_BANDS
            .iter()
            .zip(gains_db)
            .filter(|(_, gain)| gain.abs() > 0.05)
            .map(|(&frequency, &gain)| Biquad::peaking(rate, frequency, 1.4, gain.clamp(-20.0, 20.0)))
            .collect();
        Self { bands: vec![filters; channels], preamp: 10f32.powf(preamp_db.clamp(-20.0, 20.0) / 20.0) }
    }

    pub fn run(&mut self, frame: &mut [f32]) {
        for (sample, bands) in frame.iter_mut().zip(&mut self.bands) {
            *sample = bands.iter_mut().fold(*sample * self.preamp, |x, band| band.run(x));
        }
    }
}

/// Where the sound goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Two ears: the binaural room.
    Headphones,
    /// Real speakers, in WAV channel order: 2.1 (L R LFE), 3.1 (L R C LFE),
    /// 5.1 (L R C LFE RL RR) or 7.1 (… SL SR).
    Speakers(usize),
}

impl Target {
    pub fn channels(self) -> usize {
        match self {
            Target::Headphones => 2,
            Target::Speakers(count) => count,
        }
    }
}

/// Everything a frame goes through when Music plays itself (spatial audio,
/// or spread over real speakers): any channel count in, `target`'s out.
pub struct Chain {
    channels: usize,
    target: Target,
    upmixer: Upmixer,
    binaural: Binaural,
    exciters: Option<Vec<crate::enhance::Exciter>>,
}

impl Chain {
    /// `enhance`: the exciter first (for MP3 streams), as Enhance streaming does.
    pub fn new(rate: u32, channels: usize, enhance: Option<crate::enhance::Strength>, target: Target) -> Self {
        Self {
            channels,
            target,
            upmixer: Upmixer::new(rate),
            binaural: Binaural::new(rate),
            exciters: enhance.map(|strength| (0..channels).map(|_| crate::enhance::Exciter::new(rate, strength)).collect()),
        }
    }

    pub fn set_yaw(&mut self, yaw: f32) {
        self.binaural.set_yaw(yaw);
    }

    /// One interleaved frame of `channels` samples in; `target.channels()`
    /// samples written to `out`.
    pub fn run(&mut self, frame: &[f32], out: &mut [f32; 8]) {
        let mut input = [0.0f32; 8];
        for (index, sample) in frame.iter().take(8).enumerate() {
            input[index] = match &mut self.exciters {
                Some(exciters) => exciters[index].run(*sample),
                None => *sample,
            };
        }
        let surround = match self.channels {
            1 => self.upmixer.run(input[0], input[0]),
            2 => self.upmixer.run(input[0], input[1]),
            // 5.1 (and 7.1, its sides folded into the rears) as it is.
            6 => [input[0], input[1], input[2], input[3], input[4], input[5]],
            8 => [input[0], input[1], input[2], input[3], input[4] + 0.7 * input[6], input[5] + 0.7 * input[7]],
            // Anything else: its first two channels.
            _ => self.upmixer.run(input[0], input[1]),
        };
        let [fl, fr, fc, lfe, bl, br] = surround;
        let written: &[f32] = &match self.target {
            Target::Headphones => {
                let [left, right] = self.binaural.run(surround);
                [left, right, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
            }
            // No centre: it goes to both fronts; no rears: they too.
            Target::Speakers(3) => [fl + 0.7 * fc + 0.7 * bl, fr + 0.7 * fc + 0.7 * br, lfe, 0.0, 0.0, 0.0, 0.0, 0.0],
            Target::Speakers(4) => [fl + 0.7 * bl, fr + 0.7 * br, fc, lfe, 0.0, 0.0, 0.0, 0.0],
            Target::Speakers(6) => [fl, fr, fc, lfe, bl, br, 0.0, 0.0],
            // 7.1: the ambience shared by the rears and the sides.
            Target::Speakers(8) => [fl, fr, fc, lfe, 0.8 * bl, 0.8 * br, 0.7 * bl, 0.7 * br],
            Target::Speakers(_) => [fl + 0.7 * fc + 0.7 * bl, fr + 0.7 * fc + 0.7 * br, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        }[..8];
        for (slot, sample) in out.iter_mut().zip(written) {
            *slot = crate::live::soft_limit(*sample);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn energy(samples: &[f32]) -> f32 {
        samples.iter().map(|sample| sample * sample).sum()
    }

    /// Plays `frames` from one speaker channel; (left ear, right ear).
    fn ears(binaural: &mut Binaural, channel: usize, frames: usize) -> (Vec<f32>, Vec<f32>) {
        let mut left = Vec::new();
        let mut right = Vec::new();
        for index in 0..frames {
            let mut frame = [0.0; 6];
            frame[channel] = (index as f32 * 0.07).sin() * 0.5 + if index % 9 == 0 { 0.3 } else { 0.0 };
            let [l, r] = binaural.run(frame);
            left.push(l);
            right.push(r);
        }
        (left, right)
    }

    #[test]
    fn a_speaker_on_the_left_is_louder_and_earlier_in_the_left_ear() {
        let mut binaural = Binaural::new(48_000);
        let (left, right) = ears(&mut binaural, 4, 48_000); // rear left, 110°
        assert!(energy(&left) > 1.5 * energy(&right), "the head shadows the far ear");
        let first = |ear: &[f32]| ear.iter().position(|sample| sample.abs() > 1e-4).unwrap();
        assert!(first(&left) < first(&right), "the near ear hears it first");
    }

    #[test]
    fn the_centre_reaches_both_ears_alike() {
        let mut binaural = Binaural::new(48_000);
        let (left, right) = ears(&mut binaural, 2, 48_000);
        assert!((energy(&left) / energy(&right) - 1.0).abs() < 0.05);
    }

    #[test]
    fn turning_the_head_left_moves_the_centre_to_the_right_ear() {
        let mut binaural = Binaural::new(48_000);
        binaural.set_yaw(60f32.to_radians());
        let (left, right) = ears(&mut binaural, 2, 48_000);
        assert!(energy(&right) > 1.3 * energy(&left), "facing left, what was ahead is now on the right");
    }

    #[test]
    fn the_equalizer_boosts_its_band_only() {
        let rate = 48_000;
        let tone = |frequency: f32| -> f32 {
            let mut equalizer = Equalizer::new(rate, 0.0, &[0.0, 0.0, 0.0, 0.0, 12.0, 0.0, 0.0, 0.0, 0.0, 0.0], 2);
            let samples: Vec<f32> = (0..rate as usize)
                .map(|index| {
                    let mut frame = [(std::f32::consts::TAU * frequency * index as f32 / rate as f32).sin() * 0.1; 2];
                    equalizer.run(&mut frame);
                    frame[0]
                })
                .collect();
            energy(&samples[rate as usize / 2..]).sqrt()
        };
        let reference = 0.1 * (rate as f32 / 4.0).sqrt();
        assert!(tone(1000.0) > 3.0 * reference, "+12 dB at 1 kHz");
        assert!((tone(60.0) / reference - 1.0).abs() < 0.1, "60 Hz untouched");
    }

    #[test]
    fn the_chain_takes_any_channel_count_and_stays_under_full_scale() {
        for target in [Target::Headphones, Target::Speakers(3), Target::Speakers(4), Target::Speakers(6), Target::Speakers(8)] {
            for channels in [1, 2, 6, 8, 3] {
                let mut chain = Chain::new(44_100, channels, Some(crate::enhance::Strength::High), target);
                let loud = vec![0.99f32; channels];
                let mut out = [0.0; 8];
                let mut peak = 0.0f32;
                for _ in 0..10_000 {
                    chain.run(&loud, &mut out);
                    peak = out.iter().fold(peak, |peak, sample| peak.max(sample.abs()));
                }
                assert!(peak <= 1.0, "{target:?} from {channels} channels: peak {peak}");
            }
        }
    }

    /// Feeds 1 s of `frame` (repeated) and returns each output channel's energy.
    fn spread(target: Target, frame: &[f32]) -> Vec<f32> {
        let mut chain = Chain::new(48_000, frame.len(), None, target);
        let mut out = [0.0; 8];
        let mut energy = vec![0.0f32; target.channels()];
        for index in 0..48_000 {
            let wave = (index as f32 * 0.13).sin();
            let input: Vec<f32> = frame.iter().map(|level| level * wave).collect();
            chain.run(&input, &mut out);
            for (sum, sample) in energy.iter_mut().zip(out) {
                *sum += sample * sample;
            }
        }
        energy
    }

    #[test]
    fn stereo_is_spread_over_real_speakers() {
        // The same in both: centre (and a little in the fronts).
        let centred = spread(Target::Speakers(6), &[0.5, 0.5]);
        assert!(centred[2] > 10.0 * centred[4], "5.1: shared sound in the centre, not behind");
        // Opposite in each: ambience, behind.
        let wide = spread(Target::Speakers(8), &[0.5, -0.5]);
        assert!(wide[4] > 10.0 * wide[2] && wide[6] > 10.0 * wide[2], "7.1: rears and sides carry the ambience");
        let small = spread(Target::Speakers(3), &[0.5, 0.5]);
        assert!(small[0] > 0.0 && small[1] > 0.0, "2.1 has no centre: it goes to the fronts");
    }
}
