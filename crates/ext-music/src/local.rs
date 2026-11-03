//! The local library: audio files under the music folders, with their tags.

use std::path::{Path, PathBuf};

use lofty::file::{AudioFile as _, TaggedFileExt as _};
use lofty::tag::Accessor as _;

use crate::{Source, Track};

/// Files the library reads (and that "Play in Music" offers).
pub const EXTENSIONS: [&str; 9] = ["mp3", "m4a", "aac", "flac", "ogg", "opus", "wav", "aiff", "alac"];

/// Folders this deep are still read; deeper ones are left alone.
const MAX_DEPTH: usize = 8;
/// At most this many tracks, so a huge disk cannot stall the app.
const MAX_TRACKS: usize = 20_000;

pub fn is_audio(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| EXTENSIONS.contains(&extension.to_lowercase().as_str()))
}

/// One file's track: its tags, or its name when it has none.
pub fn track(path: &Path) -> Track {
    let name = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let mut track = Track {
        source: Source::Local(path.to_path_buf()),
        title: name,
        artist: String::new(),
        album: String::new(),
        duration_ms: 0,
        cover_url: None,
        artist_id: None,
        album_id: None,
    };
    if let Ok(file) = lofty::read_from_path(path) {
        track.duration_ms = file.properties().duration().as_millis() as u64;
        if let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) {
            if let Some(title) = tag.title().filter(|title| !title.trim().is_empty()) {
                track.title = title.into_owned();
            }
            track.artist = tag.artist().map(|artist| artist.into_owned()).unwrap_or_default();
            track.album = tag.album().map(|album| album.into_owned()).unwrap_or_default();
        }
    }
    track
}

/// Every audio file under `folders`, sorted by artist, album, title.
pub fn scan(folders: &[PathBuf]) -> Vec<Track> {
    let mut found = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = folders.iter().map(|folder| (folder.clone(), 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let hidden = entry.file_name().to_string_lossy().starts_with('.');
            match entry.file_type() {
                Ok(kind) if kind.is_dir() && !hidden && depth + 1 < MAX_DEPTH => stack.push((path, depth + 1)),
                Ok(kind) if kind.is_file() && !hidden && is_audio(&path) => found.push(path),
                _ => {}
            }
            if found.len() >= MAX_TRACKS {
                break;
            }
        }
    }
    let mut tracks: Vec<Track> = found.iter().map(|path| track(path)).collect();
    tracks.sort_by(|a, b| {
        (a.artist.to_lowercase(), a.album.to_lowercase(), a.title.to_lowercase()).cmp(&(b.artist.to_lowercase(), b.album.to_lowercase(), b.title.to_lowercase()))
    });
    tracks
}

/// The library of the configured folders, read once and kept (rescan to
/// pick up new files).
pub fn library() -> Vec<Track> {
    let mut cached = LIBRARY.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    cached.get_or_insert_with(|| scan(&crate::config().folders)).clone()
}

/// Reads the folders again (after adding one, or the Rescan button).
pub fn rescan() -> Vec<Track> {
    let tracks = scan(&crate::config().folders);
    *LIBRARY.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(tracks.clone());
    tracks
}

static LIBRARY: std::sync::Mutex<Option<Vec<Track>>> = std::sync::Mutex::new(None);

/// Tracks read into memory so far (0 before the library is first opened).
pub fn library_len() -> usize {
    LIBRARY.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref().map_or(0, Vec::len)
}

/// The picture embedded in a file (front cover first), as encoded bytes.
pub fn cover(path: &Path) -> Option<Vec<u8>> {
    let file = lofty::read_from_path(path).ok()?;
    let tag = file.primary_tag().or_else(|| file.first_tag())?;
    let pictures = tag.pictures();
    pictures
        .iter()
        .find(|picture| picture.pic_type() == lofty::picture::PictureType::CoverFront)
        .or_else(|| pictures.first())
        .map(|picture| picture.data().to_vec())
}

/// Tracks whose title, artist or album contain `query` (any case).
pub fn filter<'a>(tracks: &'a [Track], query: &str) -> Vec<&'a Track> {
    let query = query.trim().to_lowercase();
    tracks
        .iter()
        .filter(|track| query.is_empty() || [&track.title, &track.artist, &track.album].iter().any(|field| field.to_lowercase().contains(&query)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untagged_files_are_named_after_the_file_and_found_in_folders() {
        let dir = std::env::temp_dir().join(format!("files-music-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Album").join(".hidden")).unwrap();
        std::fs::write(dir.join("Album").join("01 Intro.mp3"), b"not audio").unwrap();
        std::fs::write(dir.join("Album").join("cover.jpg"), b"x").unwrap();
        std::fs::write(dir.join("Album").join(".hidden").join("x.mp3"), b"x").unwrap();
        let tracks = scan(&[dir.clone()]);
        assert_eq!(tracks.len(), 1, "audio only, hidden folders skipped");
        assert_eq!(tracks[0].title, "01 Intro");
        assert_eq!(filter(&tracks, "intro").len(), 1);
        assert!(filter(&tracks, "nothing").is_empty());
        assert!(is_audio(Path::new("a.FLAC")) && !is_audio(Path::new("a.txt")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
