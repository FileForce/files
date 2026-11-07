//! ISO 9660 disc images (.iso), read-only: the Joliet names when the image
//! has them (long, Unicode), else the plain ISO 9660 ones. Rock Ridge names
//! and UDF-only images are not read; links and other special entries are
//! skipped like in the other formats.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

const SECTOR: u64 = 2048;
/// Directories deeper than this are a broken (or hostile) image.
const MAX_DEPTH: usize = 64;
/// More members than this: the image is not listed further.
const MAX_ENTRIES: usize = 500_000;

/// One file or folder in the image.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// `/`-separated, without a leading slash.
    pub path: String,
    pub dir: bool,
    pub size: u64,
    /// Where its data starts, in bytes from the start of the image.
    pub offset: u64,
    /// Seconds since the Unix epoch.
    pub modified: Option<i64>,
}

/// What the volume descriptor says about the disc.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Volume {
    pub label: String,
    pub system: String,
    pub publisher: String,
    pub preparer: String,
    pub application: String,
    pub created: String,
    pub block_size: u16,
    pub blocks: u32,
    pub joliet: bool,
    pub bootable: bool,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("ISO: {message}"))
}

fn sector(file: &mut File, index: u64) -> io::Result<[u8; SECTOR as usize]> {
    let mut buffer = [0u8; SECTOR as usize];
    file.seek(SeekFrom::Start(index * SECTOR))?;
    file.read_exact(&mut buffer)?;
    Ok(buffer)
}

fn le32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn text(bytes: &[u8], joliet: bool) -> String {
    let text = if joliet {
        let units: Vec<u16> = bytes.chunks_exact(2).map(|pair| u16::from_be_bytes([pair[0], pair[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.trim_end_matches(['\0', ' ']).to_string()
}

/// "YYYYMMDDHHMMSS…" from a volume descriptor, as "YYYY-MM-DD HH:MM".
fn long_date(bytes: &[u8]) -> String {
    let digits = String::from_utf8_lossy(&bytes[..12]);
    if !digits.chars().all(|c| c.is_ascii_digit()) || digits.starts_with("0000") {
        return String::new();
    }
    format!("{}-{}-{} {}:{}", &digits[0..4], &digits[4..6], &digits[6..8], &digits[8..10], &digits[10..12])
}

/// A directory record's 7-byte date (years since 1900, GMT offset in 15 min).
fn record_date(bytes: &[u8]) -> Option<i64> {
    let date = chrono::NaiveDate::from_ymd_opt(1900 + bytes[0] as i32, bytes[1].into(), bytes[2].into())?;
    let time = date.and_hms_opt(bytes[3].into(), bytes[4].into(), bytes[5].into())?;
    Some(time.and_utc().timestamp() - (bytes[6] as i8 as i64) * 15 * 60)
}

/// The descriptors: the volume, and the root directory (Joliet's if any).
fn descriptors(file: &mut File) -> io::Result<(Volume, (u32, u32), bool)> {
    let mut volume = Volume::default();
    let mut primary_root = None;
    let mut joliet_root = None;
    for index in 16..64 {
        let block = sector(file, index)?;
        if &block[1..6] != b"CD001" {
            return Err(invalid("not an ISO 9660 image"));
        }
        match block[0] {
            0 => volume.bootable = true,
            1 => {
                volume.system = text(&block[8..40], false);
                volume.label = text(&block[40..72], false);
                volume.blocks = le32(&block[80..84]);
                volume.block_size = u16::from_le_bytes([block[128], block[129]]);
                volume.publisher = text(&block[318..446], false);
                volume.preparer = text(&block[446..574], false);
                volume.application = text(&block[574..702], false);
                volume.created = long_date(&block[813..830]);
                primary_root = Some((le32(&block[158..162]), le32(&block[166..170])));
            }
            // Joliet: a supplementary descriptor with a UCS-2 escape.
            2 if matches!(&block[88..91], b"%/@" | b"%/C" | b"%/E") => {
                volume.joliet = true;
                let label = text(&block[40..72], true);
                if !label.is_empty() {
                    volume.label = label;
                }
                joliet_root = Some((le32(&block[158..162]), le32(&block[166..170])));
            }
            255 => break,
            _ => {}
        }
    }
    let joliet = joliet_root.is_some();
    let root = joliet_root.or(primary_root).ok_or_else(|| invalid("no primary volume descriptor"))?;
    Ok((volume, root, joliet))
}

/// The disc's label, creator and other facts.
pub fn volume(path: &Path) -> io::Result<Volume> {
    Ok(descriptors(&mut File::open(path)?)?.0)
}

/// Every member, folders first in each directory's order.
pub fn entries(path: &Path) -> io::Result<Vec<Entry>> {
    let mut file = File::open(path)?;
    let image_size = file.metadata()?.len();
    let (_, root, joliet) = descriptors(&mut file)?;
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![(root, String::new(), 0usize)];
    while let Some(((extent, length), prefix, depth)) = stack.pop() {
        // A directory pointing back up would loop forever.
        if depth > MAX_DEPTH || !seen.insert(extent) || u64::from(extent) * SECTOR + u64::from(length) > image_size {
            continue;
        }
        let mut data = vec![0u8; length as usize];
        file.seek(SeekFrom::Start(u64::from(extent) * SECTOR))?;
        file.read_exact(&mut data)?;
        let mut at = 0usize;
        while at < data.len() {
            let size = data[at] as usize;
            if size == 0 {
                // Records never cross a sector: skip to the next one.
                at = (at / SECTOR as usize + 1) * SECTOR as usize;
                continue;
            }
            let Some(record) = data.get(at..at + size).filter(|record| record.len() >= 33) else { break };
            at += size;
            let name_length = record[32] as usize;
            let Some(raw) = record.get(33..33 + name_length) else { continue };
            // "." and "..".
            if name_length == 1 && raw[0] <= 1 {
                continue;
            }
            let flags = record[25];
            // Hidden entries stay listed; associated files and multi-extent pieces are skipped.
            if flags & 0x04 != 0 || flags & 0x80 != 0 {
                continue;
            }
            let mut name = text(raw, joliet);
            if let Some((stem, _)) = name.rsplit_once(';') {
                name = stem.to_string();
            }
            let name = name.trim_end_matches('.').to_string();
            if name.is_empty() || name.contains('/') || name == ".." {
                continue;
            }
            let entry = Entry {
                path: if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") },
                dir: flags & 0x02 != 0,
                size: if flags & 0x02 != 0 { 0 } else { u64::from(le32(&record[10..14])) },
                offset: u64::from(le32(&record[2..6])) * SECTOR,
                modified: record_date(&record[18..25]),
            };
            if entry.dir {
                stack.push(((le32(&record[2..6]), le32(&record[10..14])), entry.path.clone(), depth + 1));
            }
            entries.push(entry);
            if entries.len() > MAX_ENTRIES {
                return Err(invalid("too many entries"));
            }
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

/// A member's bytes, streamed from the image.
pub fn reader(path: &Path, entry: &Entry) -> io::Result<impl Read> {
    let mut file = File::open(path)?;
    if entry.offset + entry.size > file.metadata()?.len() {
        return Err(invalid("a file goes past the end of the image"));
    }
    file.seek(SeekFrom::Start(entry.offset))?;
    Ok(file.take(entry.size))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A tiny image: a root with README.TXT (`hello`) and a DOCS folder.
    pub(crate) fn tiny_image(path: &Path) {
        let mut image = vec![0u8; 24 * SECTOR as usize];
        let record = |extent: u32, size: u32, dir: bool, name: &[u8]| {
            let mut record = vec![0u8; 33 + name.len() + (1 - name.len() % 2)];
            record[0] = record.len() as u8;
            record[2..6].copy_from_slice(&extent.to_le_bytes());
            record[10..14].copy_from_slice(&size.to_le_bytes());
            record[18..25].copy_from_slice(&[124, 1, 2, 3, 4, 5, 0]);
            record[25] = if dir { 2 } else { 0 };
            record[32] = name.len() as u8;
            record[33..33 + name.len()].copy_from_slice(name);
            record
        };
        let pvd = 16 * SECTOR as usize;
        image[pvd] = 1;
        image[pvd + 1..pvd + 6].copy_from_slice(b"CD001");
        image[pvd + 40..pvd + 47].copy_from_slice(b"TESTISO");
        image[pvd + 128] = 0;
        image[pvd + 129] = 8;
        let root = record(20, SECTOR as u32, true, &[0]);
        image[pvd + 156..pvd + 156 + 34].copy_from_slice(&root);
        let end = 17 * SECTOR as usize;
        image[end] = 255;
        image[end + 1..end + 6].copy_from_slice(b"CD001");
        let mut dir = Vec::new();
        dir.extend(record(20, SECTOR as u32, true, &[0]));
        dir.extend(record(20, SECTOR as u32, true, &[1]));
        dir.extend(record(22, 5, false, b"README.TXT;1"));
        dir.extend(record(21, SECTOR as u32, true, b"DOCS"));
        image[20 * SECTOR as usize..20 * SECTOR as usize + dir.len()].copy_from_slice(&dir);
        let mut docs = Vec::new();
        docs.extend(record(21, SECTOR as u32, true, &[0]));
        docs.extend(record(20, SECTOR as u32, true, &[1]));
        image[21 * SECTOR as usize..21 * SECTOR as usize + docs.len()].copy_from_slice(&docs);
        image[22 * SECTOR as usize..22 * SECTOR as usize + 5].copy_from_slice(b"hello");
        std::fs::write(path, image).unwrap();
    }

    #[test]
    fn an_iso_lists_its_files_and_reads_them() {
        let path = std::env::temp_dir().join(format!("files-iso-test-{}.iso", std::process::id()));
        tiny_image(&path);
        let volume = volume(&path).unwrap();
        assert_eq!((volume.label.as_str(), volume.joliet), ("TESTISO", false));
        let entries = entries(&path).unwrap();
        let names: Vec<(&str, bool)> = entries.iter().map(|entry| (entry.path.as_str(), entry.dir)).collect();
        assert_eq!(names, [("DOCS", true), ("README.TXT", false)], "versions (;1) dropped, . and .. skipped");
        let readme = entries.iter().find(|entry| entry.path == "README.TXT").unwrap();
        let mut text = String::new();
        reader(&path, readme).unwrap().read_to_string(&mut text).unwrap();
        assert_eq!(text, "hello");
        let _ = std::fs::remove_file(&path);
    }
}
