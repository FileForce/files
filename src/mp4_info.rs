//! The frame rate of an MP4 / MOV video read from its boxes, for when VLC's
//! quick parse gives none: fragmented files (OBS, ffmpeg's `frag_keyframe`,
//! screen recorders) keep an empty sample table in `moov` and their frame
//! durations in each fragment, which VLC only reads while playing.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// A box's type, where its content starts and ends in the file.
struct Box4 {
    kind: [u8; 4],
    start: u64,
    end: u64,
}

/// The boxes from `start` to `end`, read header by header (content skipped).
fn boxes(file: &mut File, start: u64, end: u64) -> Vec<Box4> {
    let mut found = Vec::new();
    let mut at = start;
    while at + 8 <= end {
        let mut header = [0u8; 16];
        if file.seek(SeekFrom::Start(at)).is_err() || file.read_exact(&mut header[..8]).is_err() {
            break;
        }
        let mut size = u64::from(u32::from_be_bytes([
            header[0], header[1], header[2], header[3],
        ]));
        let kind = [header[4], header[5], header[6], header[7]];
        let mut content = at + 8;
        if size == 1 {
            if file.read_exact(&mut header[8..16]).is_err() {
                break;
            }
            size = u64::from_be_bytes(header[8..16].try_into().unwrap_or_default());
            content = at + 16;
        } else if size == 0 {
            size = end - at;
        }
        if size < content - at || at + size > end {
            break;
        }
        found.push(Box4 {
            kind,
            start: content,
            end: at + size,
        });
        at += size;
        // A file is scanned for its first fragment only, not every one.
        if found.len() > 4096 {
            break;
        }
    }
    found
}

fn read(file: &mut File, item: &Box4, limit: u64) -> Option<Vec<u8>> {
    let length = (item.end - item.start).min(limit);
    let mut bytes = vec![0u8; length as usize];
    file.seek(SeekFrom::Start(item.start)).ok()?;
    file.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

fn child<'a>(list: &'a [Box4], kind: &[u8; 4]) -> Option<&'a Box4> {
    list.iter().find(|item| &item.kind == kind)
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// The video track: its id, its timescale and its sample table's average
/// frame duration (`None` when the table is empty: a fragmented file).
struct Track {
    id: u32,
    timescale: u32,
    table: Option<f64>,
}

fn video_track(file: &mut File, moov: &Box4) -> Option<Track> {
    for trak in boxes(file, moov.start, moov.end)
        .iter()
        .filter(|item| &item.kind == b"trak")
    {
        let inside = boxes(file, trak.start, trak.end);
        let mdia = child(&inside, b"mdia")?;
        let media = boxes(file, mdia.start, mdia.end);
        let handler = read(file, child(&media, b"hdlr")?, 16)?;
        if handler.get(8..12) != Some(b"vide") {
            continue;
        }
        let tkhd = read(file, child(&inside, b"tkhd")?, 24)?;
        // Version 1 has 64-bit times before the id.
        let id = if tkhd[0] == 1 {
            be32(&tkhd, 20)?
        } else {
            be32(&tkhd, 12)?
        };
        let mdhd = read(file, child(&media, b"mdhd")?, 32)?;
        let timescale = if mdhd[0] == 1 {
            be32(&mdhd, 20)?
        } else {
            be32(&mdhd, 12)?
        };
        let table = child(&media, b"minf")
            .map(|minf| boxes(file, minf.start, minf.end))
            .and_then(|minf| child(&minf, b"stbl").map(|stbl| boxes(file, stbl.start, stbl.end)))
            .and_then(|stbl| child(&stbl, b"stts").and_then(|stts| read(file, stts, 8 + 8 * 4096)))
            .and_then(|stts| {
                let entries = be32(&stts, 4)? as usize;
                let (mut frames, mut ticks) = (0u64, 0u64);
                for entry in 0..entries.min(4096) {
                    let count = u64::from(be32(&stts, 8 + entry * 8)?);
                    frames += count;
                    ticks += count * u64::from(be32(&stts, 12 + entry * 8)?);
                }
                (frames > 0 && ticks > 0).then(|| ticks as f64 / frames as f64)
            });
        return Some(Track {
            id,
            timescale,
            table,
        });
    }
    None
}

/// The default frame duration a fragmented file declares for `track`
/// (`mvex/trex`), or what its first fragment says (`tfhd`, then `trun`).
fn fragment_duration(file: &mut File, top: &[Box4], moov: &Box4, track: u32) -> Option<f64> {
    let declared = boxes(file, moov.start, moov.end)
        .iter()
        .find(|item| &item.kind == b"mvex")
        .map(|mvex| boxes(file, mvex.start, mvex.end))
        .and_then(|mvex| {
            mvex.iter()
                .filter(|item| &item.kind == b"trex")
                .find_map(|trex| {
                    let bytes = read(file, trex, 24)?;
                    (be32(&bytes, 4)? == track)
                        .then(|| be32(&bytes, 12))
                        .flatten()
                })
        })
        .filter(|duration| *duration > 0);
    let moof = top.iter().find(|item| &item.kind == b"moof")?;
    for traf in boxes(file, moof.start, moof.end)
        .iter()
        .filter(|item| &item.kind == b"traf")
    {
        let inside = boxes(file, traf.start, traf.end);
        let tfhd = read(file, child(&inside, b"tfhd")?, 64)?;
        if be32(&tfhd, 4)? != track {
            continue;
        }
        // tfhd: flags, id, then the optional fields its flags turn on.
        let flags = be32(&tfhd, 0)? & 0x00ff_ffff;
        let mut at = 8;
        for (flag, size) in [(0x01, 8), (0x02, 4)] {
            if flags & flag != 0 {
                at += size;
            }
        }
        let default = if flags & 0x08 != 0 {
            be32(&tfhd, at)
        } else {
            declared
        };
        // trun with per-sample durations: their average.
        if let Some(trun) = child(&inside, b"trun").and_then(|trun| read(file, trun, 8 + 16 * 4096))
        {
            let flags = be32(&trun, 0)? & 0x00ff_ffff;
            let count = be32(&trun, 4)? as usize;
            if flags & 0x100 != 0 && count > 0 {
                let mut at = 8
                    + if flags & 0x01 != 0 { 4 } else { 0 }
                    + if flags & 0x04 != 0 { 4 } else { 0 };
                let stride = [0x100, 0x200, 0x400, 0x800]
                    .iter()
                    .filter(|flag| flags & **flag != 0)
                    .count()
                    * 4;
                let mut ticks = 0u64;
                let mut seen = 0u64;
                for _ in 0..count.min(4096) {
                    let Some(duration) = be32(&trun, at) else {
                        break;
                    };
                    ticks += u64::from(duration);
                    seen += 1;
                    at += stride;
                }
                if seen > 0 && ticks > 0 {
                    return Some(ticks as f64 / seen as f64);
                }
            }
        }
        return default.map(f64::from).filter(|duration| *duration > 0.);
    }
    declared.map(f64::from)
}

/// Frames per second of the file's video track, if it is an MP4 or MOV with one.
pub fn frame_rate(path: &Path) -> Option<f64> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let top = boxes(&mut file, 0, length);
    let moov = child(&top, b"moov")?;
    let track = video_track(&mut file, moov)?;
    let duration = match track.table {
        Some(duration) => duration,
        None => fragment_duration(&mut file, &top, moov, track.id)?,
    };
    let rate = f64::from(track.timescale) / duration;
    (rate.is_finite() && rate > 0. && rate < 1000.).then_some(rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(kind: &[u8; 4], content: &[u8]) -> Vec<u8> {
        let mut out = ((content.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(content);
        out
    }

    /// A minimal fragmented MP4: video track 1 at 15360 ticks a second,
    /// an empty sample table, and a fragment of 256-tick frames (60 fps).
    fn fragmented(trun_durations: bool) -> Vec<u8> {
        let tkhd = {
            let mut bytes = vec![0u8; 24];
            bytes[12..16].copy_from_slice(&1u32.to_be_bytes());
            bytes
        };
        let mdhd = {
            let mut bytes = vec![0u8; 24];
            bytes[12..16].copy_from_slice(&15_360u32.to_be_bytes());
            bytes
        };
        let hdlr = {
            let mut bytes = vec![0u8; 24];
            bytes[8..12].copy_from_slice(b"vide");
            bytes
        };
        let stts = vec![0u8; 8];
        let stbl = boxed(b"stbl", &boxed(b"stts", &stts));
        let minf = boxed(b"minf", &stbl);
        let mdia = boxed(
            b"mdia",
            &[boxed(b"mdhd", &mdhd), boxed(b"hdlr", &hdlr), minf].concat(),
        );
        let trak = boxed(b"trak", &[boxed(b"tkhd", &tkhd), mdia].concat());
        let trex = {
            let mut bytes = vec![0u8; 24];
            bytes[4..8].copy_from_slice(&1u32.to_be_bytes());
            bytes[12..16].copy_from_slice(&512u32.to_be_bytes());
            bytes
        };
        let moov = boxed(
            b"moov",
            &[trak, boxed(b"mvex", &boxed(b"trex", &trex))].concat(),
        );
        let tfhd = [0u32.to_be_bytes(), 1u32.to_be_bytes()].concat();
        let trun = if trun_durations {
            let mut bytes = [0x100u32.to_be_bytes(), 3u32.to_be_bytes()].concat();
            for _ in 0..3 {
                bytes.extend_from_slice(&256u32.to_be_bytes());
            }
            bytes
        } else {
            [0u32.to_be_bytes(), 3u32.to_be_bytes()].concat()
        };
        let moof = boxed(
            b"moof",
            &boxed(
                b"traf",
                &[boxed(b"tfhd", &tfhd), boxed(b"trun", &trun)].concat(),
            ),
        );
        [
            boxed(b"ftyp", b"isom"),
            moov,
            moof,
            boxed(b"mdat", &[0u8; 16]),
        ]
        .concat()
    }

    fn rate_of(bytes: &[u8], name: &str) -> Option<f64> {
        let path =
            std::env::temp_dir().join(format!("files-mp4-{}-{name}.mp4", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let rate = frame_rate(&path);
        let _ = std::fs::remove_file(&path);
        rate
    }

    #[test]
    fn a_fragmented_mp4_has_its_frame_rate_in_its_fragments() {
        assert_eq!(
            rate_of(&fragmented(true), "trun"),
            Some(60.0),
            "per-frame durations in the fragment"
        );
        assert_eq!(
            rate_of(&fragmented(false), "trex"),
            Some(30.0),
            "no durations there: the track's default"
        );
        assert_eq!(rate_of(b"not a video at all", "junk"), None);
    }
}
