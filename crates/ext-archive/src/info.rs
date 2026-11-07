//! What the preview says about an archive: its format, the compression
//! method and level, the ratio, encryption, and for disc images the volume.

use std::io::{self, Read as _};
use std::path::Path;

use files_sdk::ArchiveInfo;

use crate::extract::{ArchiveKind, list_items, open_zip};

/// "1.2 GB", "340 KB".
fn size(bytes: u64) -> String {
    match bytes {
        bytes if bytes >= 1 << 30 => format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64),
        bytes if bytes >= 1 << 20 => format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64),
        bytes if bytes >= 1 << 10 => format!("{} KB", bytes / 1024),
        bytes => format!("{bytes} B"),
    }
}

/// "4.1 MB → 1.2 MB · saves 71%".
fn ratio(unpacked: u64, packed: u64) -> String {
    if unpacked == 0 {
        return size(packed);
    }
    let saved = 100. - packed as f64 * 100. / unpacked as f64;
    format!("{} → {} · saves {:.0}%", size(unpacked), size(packed), saved.max(0.))
}

fn row(details: &mut Vec<(String, String)>, label: &str, value: impl Into<String>) {
    let value = value.into();
    if !value.trim().is_empty() {
        details.push((label.to_string(), value));
    }
}

/// gzip's header: method, level hint (XFL), the system that made it.
fn gzip_rows(path: &Path, details: &mut Vec<(String, String)>) -> io::Result<u64> {
    let mut header = [0u8; 10];
    std::fs::File::open(path)?.read_exact(&mut header)?;
    if header[..2] != [0x1f, 0x8b] {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not gzip"));
    }
    let method = if header[2] == 8 { "Deflate (gzip)" } else { "Unknown gzip method" };
    let level = match header[8] {
        2 => "maximum (level 9)",
        4 => "fastest (level 1)",
        _ => "default (level 6)",
    };
    let system = match header[9] {
        0 => "Windows (FAT)",
        3 => "Unix",
        7 => "Macintosh",
        11 => "Windows (NTFS)",
        _ => "",
    };
    row(details, "Compression", method);
    row(details, "Level", level);
    row(details, "Made on", system);
    // ISIZE: the original size modulo 4 GB, in the last 4 bytes.
    let unpacked = crate::extract::gz_original_size(path).unwrap_or(0);
    Ok(unpacked)
}

/// Format, compression and the rest, for one archive.
pub fn info(path: &Path) -> io::Result<ArchiveInfo> {
    let kind = ArchiveKind::of(path).ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "not an archive"))?;
    let packed = std::fs::metadata(path)?.len();
    let mut details = Vec::new();
    let format = match kind {
        ArchiveKind::Zip => {
            let mut zip = open_zip(path)?;
            let (mut files, mut folders, mut encrypted, mut unpacked, mut compressed) = (0, 0, 0, 0u64, 0u64);
            let mut methods: std::collections::BTreeMap<String, usize> = Default::default();
            for index in 0..zip.len() {
                let Ok(file) = zip.by_index_raw(index) else { continue };
                if file.is_dir() {
                    folders += 1;
                    continue;
                }
                files += 1;
                encrypted += usize::from(file.encrypted());
                unpacked += file.size();
                compressed += file.compressed_size();
                let method = match format!("{:?}", file.compression()).as_str() {
                    "Stored" => "Stored (no compression)".to_string(),
                    "Deflated" => "Deflate".to_string(),
                    "Deflate64" => "Deflate64".to_string(),
                    "Bzip2" => "bzip2".to_string(),
                    "Zstd" => "Zstandard".to_string(),
                    "Lzma" => "LZMA".to_string(),
                    "Xz" => "XZ".to_string(),
                    "Aes" => "AES-encrypted".to_string(),
                    other => other.to_string(),
                };
                *methods.entry(method).or_default() += 1;
            }
            let mut methods: Vec<(String, usize)> = methods.into_iter().collect();
            methods.sort_by(|a, b| b.1.cmp(&a.1));
            let compression = if methods.len() == 1 {
                methods[0].0.clone()
            } else {
                methods.iter().map(|(method, count)| format!("{method} ({count})")).collect::<Vec<_>>().join(", ")
            };
            row(&mut details, "Compression", compression);
            row(&mut details, "Size", ratio(unpacked, compressed));
            row(&mut details, "Contents", format!("{files} files · {folders} folders"));
            if encrypted > 0 {
                row(&mut details, "Encrypted", if encrypted == files { "Every file (password)".to_string() } else { format!("{encrypted} of {files} files") });
            }
            row(&mut details, "Comment", String::from_utf8_lossy(zip.comment()).into_owned());
            "ZIP".to_string()
        }
        ArchiveKind::TarGz | ArchiveKind::Gz => {
            let unpacked = gzip_rows(path, &mut details)?;
            row(&mut details, "Size", ratio(unpacked, packed));
            if kind == ArchiveKind::TarGz {
                let items = list_items(path)?;
                let files = items.iter().filter(|item| !item.dir).count();
                row(&mut details, "Contents", format!("{files} files · {} folders", items.len() - files));
                "TAR + gzip".to_string()
            } else {
                "gzip".to_string()
            }
        }
        ArchiveKind::Tar => {
            let mut header = [0u8; 512];
            std::fs::File::open(path)?.read_exact(&mut header)?;
            let flavor = match &header[257..265] {
                b"ustar\x0000" => "POSIX ustar",
                b"ustar  \0" => "GNU tar",
                _ => "V7 tar",
            };
            row(&mut details, "Compression", "None (TAR only bundles files)");
            row(&mut details, "Flavor", flavor);
            let items = list_items(path)?;
            let files = items.iter().filter(|item| !item.dir).count();
            row(&mut details, "Contents", format!("{files} files · {} folders", items.len() - files));
            "TAR".to_string()
        }
        ArchiveKind::Rar => {
            let mut signature = [0u8; 8];
            std::fs::File::open(path)?.read_exact(&mut signature)?;
            let version = if signature[6..8] == [1, 0] { "RAR 5" } else { "RAR 4" };
            let archive = unrar::Archive::new(path).open_for_listing().map_err(|error| io::Error::other(format!("RAR: {error}")))?;
            let (solid, locked, recovery) = (archive.is_solid(), archive.is_locked(), archive.has_recovery_record());
            let headers_encrypted = archive.has_encrypted_headers();
            let (mut files, mut encrypted, mut unpacked) = (0usize, 0usize, 0u64);
            let mut methods: std::collections::BTreeMap<u32, usize> = Default::default();
            let mut archive = archive;
            while let Ok(Some(header)) = archive.read_header() {
                let entry = header.entry();
                if entry.is_file() {
                    files += 1;
                    encrypted += usize::from(entry.is_encrypted());
                    unpacked += entry.unpacked_size;
                    *methods.entry(entry.method).or_default() += 1;
                }
                match header.skip() {
                    Ok(next) => archive = next,
                    Err(_) => break,
                }
            }
            let method = methods.into_iter().max_by_key(|(_, count)| *count).map(|(method, _)| method).unwrap_or(0x33);
            let level = match method {
                0x30 => "Store (no compression)",
                0x31 => "Fastest",
                0x32 => "Fast",
                0x33 => "Normal",
                0x34 => "Good",
                0x35 => "Best",
                _ => "Unknown",
            };
            row(&mut details, "Compression", format!("{version} · {level}"));
            row(&mut details, "Size", ratio(unpacked, packed));
            row(&mut details, "Contents", format!("{files} files"));
            row(&mut details, "Solid", if solid { "Yes (smaller, but extracted as a whole)" } else { "No" });
            if encrypted > 0 || headers_encrypted {
                row(&mut details, "Encrypted", if headers_encrypted { "Names and files (password)" } else { "Files (password)" });
            }
            if recovery {
                row(&mut details, "Recovery record", "Yes");
            }
            if locked {
                row(&mut details, "Locked", "Yes (cannot be changed)");
            }
            version.to_string()
        }
        ArchiveKind::Iso => {
            let volume = crate::iso::volume(path)?;
            let entries = crate::iso::entries(path).unwrap_or_default();
            let files = entries.iter().filter(|entry| !entry.dir).count();
            row(&mut details, "Compression", "None (a disc image)");
            row(&mut details, "Label", volume.label);
            row(&mut details, "Names", if volume.joliet { "Joliet (long, Unicode)" } else { "ISO 9660 (8.3)" });
            row(&mut details, "Contents", format!("{files} files · {} folders", entries.len() - files));
            row(&mut details, "Disc size", format!("{} · {} blocks of {} B", size(u64::from(volume.blocks) * u64::from(volume.block_size)), volume.blocks, volume.block_size));
            row(&mut details, "Bootable", if volume.bootable { "Yes" } else { "No" });
            row(&mut details, "Created", volume.created);
            row(&mut details, "Publisher", volume.publisher);
            row(&mut details, "Prepared by", volume.preparer);
            row(&mut details, "Made with", volume.application);
            row(&mut details, "System", volume.system);
            "ISO 9660 disc image".to_string()
        }
    };
    Ok(ArchiveInfo { format, details })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zip_says_how_it_is_compressed() {
        let dir = std::env::temp_dir().join(format!("files-archive-info-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("a.txt", options).unwrap();
        std::io::Write::write_all(&mut zip, &[b'a'; 10_000]).unwrap();
        zip.finish().unwrap();
        let info = info(&path).unwrap();
        assert_eq!(info.format, "ZIP");
        let get = |label: &str| info.details.iter().find(|(key, _)| key == label).map(|(_, value)| value.clone());
        assert_eq!(get("Compression").as_deref(), Some("Deflate"));
        assert!(get("Size").unwrap().contains("→ 28 B · saves"), "{:?}", get("Size"));
        let iso = dir.join("b.iso");
        crate::iso::tests::tiny_image(&iso);
        let info = super::info(&iso).unwrap();
        assert!(info.details.iter().any(|(key, value)| key == "Label" && value == "TESTISO"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
