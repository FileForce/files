//! Everything the preview can say about a photo, a video, a song or an
//! archive: EXIF for pictures (camera, lens, exposure, place), streams and
//! tags for video and audio (through the bundled VLC), and an archive's
//! format and compression. Read once per file version, off the UI thread.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// A titled group of rows ("Camera", "Video", "Archive"…).
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub title: &'static str,
    pub rows: Vec<(String, String)>,
}

type Key = (PathBuf, Option<SystemTime>, u64);

#[derive(Default)]
struct State {
    done: HashMap<Key, Vec<Section>>,
    asked: HashSet<Key>,
}

fn state() -> std::sync::MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Bumped when details arrive: the preview redraws.
static READY: AtomicU64 = AtomicU64::new(0);

pub fn generation() -> u64 {
    READY.load(Ordering::Relaxed)
}

fn key(path: &Path) -> Option<Key> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((path.to_path_buf(), metadata.modified().ok(), metadata.len()))
}

/// One worker reads in turn, so browsing a folder never floods the disk.
fn worker() -> &'static std::sync::mpsc::Sender<Key> {
    static WORKER: OnceLock<std::sync::mpsc::Sender<Key>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<Key>();
        std::thread::spawn(move || {
            for key in receiver {
                let sections = read(&key.0);
                let mut state = state();
                // Small cache: the last couple hundred files looked at.
                if state.done.len() > 256 {
                    state.done.clear();
                    state.asked.clear();
                }
                state.done.insert(key, sections);
                drop(state);
                READY.fetch_add(1, Ordering::Relaxed);
            }
        });
        sender
    })
}

/// The details of `path`, or `None` while they are read (asked once).
pub fn sections(path: &Path) -> Option<Vec<Section>> {
    let key = key(path)?;
    let mut state = state();
    if let Some(done) = state.done.get(&key) {
        return Some(done.clone());
    }
    if state.asked.insert(key.clone()) {
        let _ = worker().send(key);
    }
    None
}

/// Whether the preview has details to look for in this kind of file.
pub fn applies(path: &Path) -> bool {
    use crate::model::FileKind;
    let extension = path.extension().map(|extension| extension.to_string_lossy().to_lowercase()).unwrap_or_default();
    matches!(FileKind::from_extension(&extension), FileKind::Image | FileKind::Video | FileKind::Audio)
        || files_ext_archive::ArchiveKind::of(path).is_some()
}

fn read(path: &Path) -> Vec<Section> {
    use crate::model::FileKind;
    let extension = path.extension().map(|extension| extension.to_string_lossy().to_lowercase()).unwrap_or_default();
    if files_ext_archive::ArchiveKind::of(path).is_some() {
        return archive(path);
    }
    match FileKind::from_extension(&extension) {
        FileKind::Image => image(path),
        FileKind::Video | FileKind::Audio => media(path),
        _ => Vec::new(),
    }
}

// ----------------------------------------------------------------- archives

fn archive(path: &Path) -> Vec<Section> {
    match files_ext_archive::info::info(path) {
        Ok(info) => {
            let mut rows = vec![("Format".to_string(), info.format)];
            rows.extend(info.details);
            vec![Section { title: "ARCHIVE", rows }]
        }
        Err(error) => vec![Section { title: "ARCHIVE", rows: vec![("Error".into(), error.to_string())] }],
    }
}

// ------------------------------------------------------------------- images

fn image(path: &Path) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut picture = Vec::new();
    if let Ok((width, height)) = image::image_dimensions(path) {
        picture.push(("Dimensions".to_string(), format!("{width} × {height}")));
        let megapixels = f64::from(width) * f64::from(height) / 1_000_000.;
        if megapixels >= 0.1 {
            picture.push(("Resolution".to_string(), format!("{megapixels:.1} MP · {}", aspect(width, height))));
        }
    }
    if let Ok(reader) = image::ImageReader::open(path).and_then(|reader| reader.with_guessed_format())
        && let Some(format) = reader.format()
    {
        picture.push(("Format".to_string(), format!("{format:?}").to_uppercase()));
    }
    let exif = std::fs::File::open(path).ok().and_then(|file| exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file)).ok());
    let Some(exif) = exif else {
        if !picture.is_empty() {
            sections.push(Section { title: "IMAGE", rows: picture });
        }
        return sections;
    };
    let field = |tag: exif::Tag| exif.get_field(tag, exif::In::PRIMARY).map(|field| field.display_value().with_unit(&exif).to_string().trim_matches('"').trim().to_string()).filter(|value| !value.is_empty());
    let push = |rows: &mut Vec<(String, String)>, label: &str, tag: exif::Tag| {
        if let Some(value) = field(tag) {
            rows.push((label.to_string(), value));
        }
    };
    for (label, tag) in [("Color space", exif::Tag::ColorSpace), ("Bit depth", exif::Tag::BitsPerSample)] {
        push(&mut picture, label, tag);
    }
    if let Some(orientation) = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY).and_then(|field| field.value.get_uint(0)) {
        let text = match orientation {
            1 => "Normal",
            2 => "Mirrored",
            3 => "Rotated 180°",
            4 => "Mirrored, rotated 180°",
            5 => "Mirrored, rotated 90° left",
            6 => "Rotated 90° right",
            7 => "Mirrored, rotated 90° right",
            8 => "Rotated 90° left",
            _ => "",
        };
        if !text.is_empty() {
            picture.push(("Orientation".to_string(), text.to_string()));
        }
    }
    if !picture.is_empty() {
        sections.push(Section { title: "IMAGE", rows: picture });
    }
    let mut camera = Vec::new();
    for (label, tag) in [
        ("Make", exif::Tag::Make),
        ("Model", exif::Tag::Model),
        ("Lens", exif::Tag::LensModel),
        ("Lens make", exif::Tag::LensMake),
        ("Serial number", exif::Tag::BodySerialNumber),
        ("Software", exif::Tag::Software),
        ("Taken", exif::Tag::DateTimeOriginal),
        ("Artist", exif::Tag::Artist),
        ("Copyright", exif::Tag::Copyright),
    ] {
        push(&mut camera, label, tag);
    }
    if !camera.is_empty() {
        sections.push(Section { title: "CAMERA", rows: camera });
    }
    let mut exposure = Vec::new();
    for (label, tag) in [
        ("Aperture", exif::Tag::FNumber),
        ("Shutter", exif::Tag::ExposureTime),
        ("ISO", exif::Tag::PhotographicSensitivity),
        ("Focal length", exif::Tag::FocalLength),
        ("35 mm equivalent", exif::Tag::FocalLengthIn35mmFilm),
        ("Exposure bias", exif::Tag::ExposureBiasValue),
        ("Exposure program", exif::Tag::ExposureProgram),
        ("Metering", exif::Tag::MeteringMode),
        ("Flash", exif::Tag::Flash),
        ("White balance", exif::Tag::WhiteBalance),
        ("Scene", exif::Tag::SceneCaptureType),
    ] {
        push(&mut exposure, label, tag);
    }
    if !exposure.is_empty() {
        sections.push(Section { title: "EXPOSURE", rows: exposure });
    }
    let mut place = Vec::new();
    if let Some((latitude, longitude)) = gps(&exif) {
        place.push(("Coordinates".to_string(), format!("{latitude:.5}, {longitude:.5}")));
    }
    for (label, tag) in [("Altitude", exif::Tag::GPSAltitude), ("Direction", exif::Tag::GPSImgDirection)] {
        push(&mut place, label, tag);
    }
    if !place.is_empty() {
        sections.push(Section { title: "LOCATION", rows: place });
    }
    sections
}

/// Decimal degrees from EXIF's degrees/minutes/seconds and N/S, E/W.
fn gps(exif: &exif::Exif) -> Option<(f64, f64)> {
    let degrees = |tag: exif::Tag, reference: exif::Tag, negative: &str| -> Option<f64> {
        let exif::Value::Rational(parts) = &exif.get_field(tag, exif::In::PRIMARY)?.value else { return None };
        let value = parts.iter().zip([1., 60., 3600.]).map(|(part, divisor)| part.to_f64() / divisor).sum::<f64>();
        let sign = exif.get_field(reference, exif::In::PRIMARY).map(|field| field.display_value().to_string()).is_some_and(|text| text.contains(negative));
        Some(if sign { -value } else { value })
    };
    Some((degrees(exif::Tag::GPSLatitude, exif::Tag::GPSLatitudeRef, "S")?, degrees(exif::Tag::GPSLongitude, exif::Tag::GPSLongitudeRef, "W")?))
}

/// "16:9", "3:2", else "1.85:1".
fn aspect(width: u32, height: u32) -> String {
    fn gcd(a: u32, b: u32) -> u32 {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    let divisor = gcd(width, height).max(1);
    let (w, h) = (width / divisor, height / divisor);
    if w <= 32 && h <= 32 { format!("{w}:{h}") } else { format!("{:.2}:1", f64::from(width) / f64::from(height.max(1))) }
}

// -------------------------------------------------------------- video, audio

fn duration(ms: i64) -> String {
    let seconds = ms / 1000;
    if seconds >= 3600 { format!("{}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60) } else { format!("{}:{:02}", seconds / 60, seconds % 60) }
}

fn channels(count: u32) -> String {
    match count {
        1 => "Mono".into(),
        2 => "Stereo".into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        count => format!("{count} channels"),
    }
}

fn media(path: &Path) -> Vec<Section> {
    let Some(probe) = crate::video::probe(path) else {
        return vec![Section { title: "MEDIA", rows: vec![("Details".into(), "VLC could not read this file".into())] }];
    };
    let mut sections = Vec::new();
    let mut general = Vec::new();
    if probe.duration_ms > 0 {
        general.push(("Duration".to_string(), duration(probe.duration_ms)));
        if let Ok(metadata) = std::fs::metadata(path) {
            let kbps = metadata.len() as f64 * 8. / (probe.duration_ms as f64 / 1000.) / 1000.;
            general.push(("Overall bitrate".to_string(), format!("{kbps:.0} kb/s")));
        }
    }
    if let Some(video) = probe.streams.iter().find(|stream| stream.kind == "Video") {
        general.push(("Resolution".to_string(), format!("{} × {} · {}", video.width, video.height, aspect(video.width, video.height))));
    }
    if !general.is_empty() {
        sections.push(Section { title: "MEDIA", rows: general });
    }
    for (index, stream) in probe.streams.iter().enumerate() {
        let mut rows = vec![("Codec".to_string(), stream.codec.clone())];
        if stream.kind == "Video" {
            if stream.width > 0 {
                rows.push(("Size".to_string(), format!("{} × {}", stream.width, stream.height)));
            }
            if stream.frame_rate > 0. {
                rows.push(("Frame rate".to_string(), format!("{:.3} fps", stream.frame_rate).replace(".000", "")));
            }
        }
        if stream.kind == "Audio" {
            if stream.channels > 0 {
                rows.push(("Channels".to_string(), channels(stream.channels)));
            }
            if stream.sample_rate > 0 {
                rows.push(("Sample rate".to_string(), format!("{:.1} kHz", f64::from(stream.sample_rate) / 1000.).replace(".0 ", " ")));
            }
        }
        if stream.bitrate > 0 {
            rows.push(("Bitrate".to_string(), format!("{} kb/s", stream.bitrate / 1000)));
        }
        if !stream.language.is_empty() {
            rows.push(("Language".to_string(), stream.language.clone()));
        }
        if !stream.description.is_empty() {
            rows.push(("Name".to_string(), stream.description.clone()));
        }
        let title: &'static str = match (stream.kind, probe.streams.iter().filter(|other| other.kind == stream.kind).count() > 1) {
            ("Video", _) => "VIDEO",
            ("Audio", false) => "AUDIO",
            ("Audio", true) => Box::leak(format!("AUDIO {}", index + 1).into_boxed_str()),
            ("Subtitles", _) => "SUBTITLES",
            _ => "STREAM",
        };
        sections.push(Section { title, rows });
    }
    if !probe.tags.is_empty() {
        sections.push(Section { title: "TAGS", rows: probe.tags.into_iter().map(|(label, value)| (label.to_string(), value)).collect() });
    }
    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_and_lengths_read_well() {
        assert_eq!(aspect(1920, 1080), "16:9");
        assert_eq!(aspect(6000, 4000), "3:2");
        assert_eq!(aspect(1998, 1080), "1.85:1");
        assert_eq!(duration(3_725_000), "1:02:05");
        assert_eq!(duration(65_000), "1:05");
        assert_eq!(channels(6), "5.1");
    }

    #[test]
    fn a_picture_says_its_size_and_format() {
        let path = std::env::temp_dir().join(format!("file-force-media-info-{}.png", std::process::id()));
        image::RgbImage::new(64, 36).save(&path).unwrap();
        let sections = read(&path);
        let rows = &sections.iter().find(|section| section.title == "IMAGE").unwrap().rows;
        assert!(rows.contains(&("Dimensions".to_string(), "64 × 36".to_string())));
        assert!(rows.contains(&("Format".to_string(), "PNG".to_string())));
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod probe_files {
    #[test]
    #[ignore]
    fn print_details_of_files() {
        for path in std::env::var("PROBE").unwrap_or_default().split('|').filter(|path| !path.is_empty()) {
            println!("== {path}");
            for section in super::read(std::path::Path::new(path)) {
                println!("  [{}]", section.title);
                for (label, value) in section.rows {
                    println!("    {label}: {value}");
                }
            }
        }
    }
}
