//! Windows programs and libraries (.exe, .dll): the icon Explorer shows and
//! what the file says about itself — product, company, version, the
//! machine it is built for, console or windowed, .NET, signed. Read with
//! pelite on every system, so a game copied to a Mac shows its icon too.

use std::io;
use std::path::Path;

use pelite::Wrap;

use crate::media_info::Section;

/// Windows programs and libraries.
pub fn applies(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        let extension = extension.to_string_lossy().to_ascii_lowercase();
        matches!(extension.as_str(), "exe" | "dll" | "scr" | "cpl" | "ocx" | "sys" | "efi")
    })
}

fn open(path: &Path) -> io::Result<pelite::FileMap> {
    pelite::FileMap::open(path)
}

fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

/// The program's biggest icon, saved as a PNG no larger than `size`.
pub fn save_icon(path: &Path, target: &Path, size: u32) -> io::Result<()> {
    let map = open(path)?;
    let file = pelite::PeFile::from_bytes(&map).map_err(invalid)?;
    let resources = file.resources().map_err(invalid)?;
    // The first group is the one Explorer shows.
    let (_, group) = resources.icons().find_map(Result::ok).ok_or_else(|| invalid("no icon"))?;
    let mut ico = Vec::new();
    group.write(&mut ico)?;
    // The ICO decoder picks the largest image in the group.
    let picture = image::load_from_memory_with_format(&ico, image::ImageFormat::Ico).map_err(invalid)?;
    let picture = if picture.width() > size { picture.resize(size, size, image::imageops::FilterType::Lanczos3) } else { picture };
    let partial = target.with_extension("partial.png");
    picture.to_rgba8().save_with_format(&partial, image::ImageFormat::Png).map_err(invalid)?;
    std::fs::rename(&partial, target)
}

fn machine(code: u16) -> &'static str {
    match code {
        0x8664 => "x64",
        0x014c => "x86 (32-bit)",
        0xaa64 => "ARM64",
        0x01c4 => "ARM (32-bit)",
        0x0200 => "Itanium",
        _ => "Other",
    }
}

fn subsystem(code: u16) -> &'static str {
    match code {
        2 => "Windows app (with a window)",
        3 => "Console app",
        1 => "Driver",
        10..=13 => "UEFI",
        _ => "",
    }
}

/// The preview's sections for a program or library.
pub fn sections(path: &Path) -> Vec<Section> {
    let read = || -> Result<Vec<Section>, String> {
        let map = open(path).map_err(|error| error.to_string())?;
        let file = pelite::PeFile::from_bytes(&map).map_err(|error| error.to_string())?;
        let mut sections = Vec::new();
        // VERSIONINFO: what Explorer shows under Properties → Details.
        if let Ok(version) = file.resources().and_then(|resources| resources.version_info().map_err(|_| pelite::Error::Invalid)) {
            let mut rows = Vec::new();
            if let Some(language) = version.translation().first().copied() {
                for (label, key) in [
                    ("Product", "ProductName"),
                    ("Description", "FileDescription"),
                    ("Company", "CompanyName"),
                    ("Product version", "ProductVersion"),
                    ("File version", "FileVersion"),
                    ("Copyright", "LegalCopyright"),
                    ("Original name", "OriginalFilename"),
                    ("Internal name", "InternalName"),
                    ("Comments", "Comments"),
                ] {
                    if let Some(value) = version.value(language, key).map(|value| value.trim().to_string()).filter(|value| !value.is_empty()) {
                        rows.push((label.to_string(), value));
                    }
                }
            }
            if rows.iter().all(|(label, _)| label != "File version")
                && let Some(fixed) = version.fixed()
            {
                let v = fixed.dwFileVersion;
                rows.push(("File version".into(), format!("{}.{}.{}.{}", v.Major, v.Minor, v.Patch, v.Build)));
            }
            if !rows.is_empty() {
                sections.push(Section { title: "APPLICATION", rows });
            }
        }
        let header = file.file_header();
        let (subsystem_code, large_address) = match file.optional_header() {
            Wrap::T32(optional) => (optional.Subsystem, false),
            Wrap::T64(optional) => (optional.Subsystem, true),
        };
        let directories = file.data_directory();
        let directory = |index: usize| directories.get(index).is_some_and(|entry| entry.VirtualAddress != 0 && entry.Size != 0);
        let mut rows = vec![("Built for".to_string(), format!("{} · {}", machine(header.Machine), if large_address { "64-bit" } else { "32-bit" }))];
        let kind = if header.Characteristics & 0x2000 != 0 { "Library (DLL)" } else { subsystem(subsystem_code) };
        if !kind.is_empty() {
            rows.push(("Type".to_string(), kind.to_string()));
        }
        // Directory 14: the CLR header of .NET programs; 4: the signature.
        if directory(14) {
            rows.push(("Runtime".to_string(), ".NET".to_string()));
        }
        rows.push(("Signed".to_string(), if directory(4) { "Yes (Authenticode)".to_string() } else { "No".to_string() }));
        if header.TimeDateStamp != 0
            && let Some(time) = chrono::DateTime::from_timestamp(i64::from(header.TimeDateStamp), 0)
        {
            rows.push(("Built".to_string(), time.format("%Y-%m-%d %H:%M UTC").to_string()));
        }
        sections.push(Section { title: "PROGRAM", rows });
        Ok(sections)
    };
    read().unwrap_or_else(|error| vec![Section { title: "PROGRAM", rows: vec![("Error".into(), format!("Not a readable Windows program: {error}"))] }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_are_recognized_and_described() {
        assert!(applies(Path::new("Cyberpunk2077.EXE")) && applies(Path::new("x.dll")));
        assert!(!applies(Path::new("readme.txt")));
        assert_eq!(machine(0x8664), "x64");
        assert_eq!(subsystem(2), "Windows app (with a window)");
        let junk = std::env::temp_dir().join(format!("file-force-pe-{}.exe", std::process::id()));
        std::fs::write(&junk, b"MZ not really").unwrap();
        let sections = sections(&junk);
        assert!(sections[0].rows[0].1.starts_with("Not a readable Windows program"), "a broken file says so");
        let _ = std::fs::remove_file(&junk);
    }
}

#[cfg(test)]
mod live {
    #[test]
    #[ignore]
    fn icon_of_a_real_program() {
        let source = std::path::PathBuf::from(std::env::var("EXE").unwrap());
        let target = std::path::PathBuf::from(std::env::var("ICON").unwrap());
        super::save_icon(&source, &target, 256).unwrap();
        let (width, height) = image::image_dimensions(&target).unwrap();
        println!("icon {width}×{height}");
    }
}
