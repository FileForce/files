//! Reading and extracting ZIP, TAR, TAR.GZ, GZ and RAR. Every member path is
//! made safe first: nothing is written outside the target folder, links and
//! devices are skipped, and set-uid/set-gid bits are dropped.

use std::path::{Path, PathBuf};

use files_sdk::fs::unique_path;
pub use files_sdk::{ConflictPolicy, ExtractReport};

/// Archive formats the app can browse and extract.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArchiveKind {
    Zip,
    Tar,
    TarGz,
    /// A single gzip-compressed file (not a tarball), e.g. `dump.dmw.gz`.
    Gz,
    /// Read-only: RARLAB's unrar library lists and extracts, never creates.
    Rar,
}
impl ArchiveKind {
    pub const SUFFIXES: [(&'static str, Self); 6] = [
        (".rar", Self::Rar),
        (".tar.gz", Self::TarGz),
        (".tgz", Self::TarGz),
        (".tar", Self::Tar),
        (".zip", Self::Zip),
        (".gz", Self::Gz),
    ];

    pub fn of(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().to_lowercase();
        Self::SUFFIXES
            .iter()
            .find(|(suffix, _)| name.ends_with(suffix))
            .map(|(_, kind)| *kind)
    }
}
/// Opens a zip, keeping the underlying I/O error instead of zip's bare "i/o error".
pub fn open_zip(path: &Path) -> std::io::Result<zip::ZipArchive<std::fs::File>> {
    zip::ZipArchive::new(std::fs::File::open(path)?).map_err(zip_error)
}
fn zip_error(error: zip::result::ZipError) -> std::io::Error {
    match error {
        zip::result::ZipError::Io(error) => error,
        other => std::io::Error::other(other),
    }
}
/// Error text with a hint for the common cloud-placeholder case.
pub fn describe_error(path: &Path, error: &std::io::Error) -> String {
    let in_cloud = path.components().any(|part| part.as_os_str() == "CloudStorage");
    if error.kind() == std::io::ErrorKind::TimedOut || in_cloud {
        format!(
            "{error} · the file may be online-only in cloud storage; download it locally first (Finder → Download Now)"
        )
    } else {
        error.to_string()
    }
}
/// One archive member, already made safe: relative, `/`-separated, no `..`.
#[derive(Clone)]
pub struct Item {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    pub modified: Option<chrono::NaiveDateTime>,
}
/// Rejects absolute paths and `..` so nothing can escape the target (zip-slip).
pub fn sanitize(path: &Path) -> Option<PathBuf> {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => clean.push(part),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    (!clean.as_os_str().is_empty()).then_some(clean)
}
pub fn slash_name(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
fn open_tar(path: &Path, gzipped: bool) -> std::io::Result<tar::Archive<Box<dyn std::io::Read>>> {
    let file = std::io::BufReader::new(std::fs::File::open(path)?);
    let reader: Box<dyn std::io::Read> = if gzipped {
        Box::new(flate2::read::MultiGzDecoder::new(file))
    } else {
        Box::new(file)
    };
    Ok(tar::Archive::new(reader))
}
/// The name stored in a gzip header, or the file name without `.gz`.
fn gz_member_name(path: &Path) -> std::io::Result<String> {
    let decoder = flate2::read::GzDecoder::new(std::fs::File::open(path)?);
    let stored = decoder
        .header()
        .and_then(|header| header.filename())
        .map(|name| String::from_utf8_lossy(name).into_owned())
        // Only a plain file name is trusted from the header.
        .filter(|name| sanitize(Path::new(name)).is_some_and(|clean| clean.components().count() == 1));
    let fallback = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let fallback = fallback
        .strip_suffix(".gz")
        .or_else(|| fallback.strip_suffix(".GZ"))
        .unwrap_or(&fallback)
        .to_string();
    Ok(stored.unwrap_or(fallback))
}
fn unix_time(seconds: u64) -> Option<chrono::NaiveDateTime> {
    (seconds > 0)
        .then(|| chrono::DateTime::from_timestamp(seconds as i64, 0))
        .flatten()
        .map(|time| time.with_timezone(&chrono::Local).naive_local())
}
/// The trailer's ISIZE: the original size modulo 2^32.
fn gz_original_size(path: &Path) -> std::io::Result<u64> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::End(-4))?;
    let mut trailer = [0u8; 4];
    file.read_exact(&mut trailer)?;
    Ok(u32::from_le_bytes(trailer) as u64)
}
fn unsupported() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Unsupported, "not a supported archive")
}
pub fn list_items(path: &Path) -> std::io::Result<Vec<Item>> {
    let kind = ArchiveKind::of(path).ok_or_else(unsupported)?;
    let mut items = Vec::new();
    match kind {
        ArchiveKind::Zip => {
            let mut zip = open_zip(path)?;
            for index in 0..zip.len() {
                let file = zip.by_index(index).map_err(zip_error)?;
                let Some(name) = file.enclosed_name() else {
                    continue;
                };
                items.push(Item {
                    name: slash_name(&name),
                    dir: file.is_dir(),
                    size: file.size(),
                    modified: file.last_modified().and_then(|time| {
                        chrono::NaiveDate::from_ymd_opt(
                            time.year().into(),
                            time.month().into(),
                            time.day().into(),
                        )?
                        .and_hms_opt(time.hour().into(), time.minute().into(), time.second().into())
                    }),
                });
            }
        }
        ArchiveKind::Tar | ArchiveKind::TarGz => {
            let mut archive = open_tar(path, kind == ArchiveKind::TarGz)?;
            for entry in archive.entries()? {
                let entry = entry?;
                let header = entry.header();
                let entry_type = header.entry_type();
                if !entry_type.is_dir() && !entry_type.is_file() {
                    continue;
                }
                let Some(name) = sanitize(&entry.path()?) else {
                    continue;
                };
                items.push(Item {
                    name: slash_name(&name),
                    dir: entry_type.is_dir(),
                    size: header.size().unwrap_or(0),
                    modified: header.mtime().ok().and_then(unix_time),
                });
            }
        }
        ArchiveKind::Rar => {
            let mut archive = open_rar(path)?.open_for_listing().map_err(rar_error)?;
            while let Some(header) = archive.read_header().map_err(rar_error)? {
                let entry = header.entry();
                if let Some(name) = sanitize(&entry.filename) {
                    items.push(Item {
                        name: slash_name(&name),
                        dir: entry.is_directory(),
                        size: entry.unpacked_size,
                        modified: dos_time(entry.file_time),
                    });
                }
                archive = header.skip().map_err(rar_error)?;
            }
        }
        ArchiveKind::Gz => {
            let decoder = flate2::read::GzDecoder::new(std::fs::File::open(path)?);
            let header_time = decoder.header().map(|header| header.mtime() as u64);
            items.push(Item {
                name: gz_member_name(path)?,
                dir: false,
                size: gz_original_size(path).unwrap_or(0),
                modified: header_time.and_then(unix_time),
            });
        }
    }
    Ok(items)
}
/// Archive paths whose destination already exists under `target`, including
/// files sitting where the archive needs a folder.
pub fn find_conflicts(archive: &Path, target: &Path) -> std::io::Result<Vec<String>> {
    let mut conflicts = std::collections::BTreeSet::new();
    for item in list_items(archive)? {
        let relative = PathBuf::from(&item.name);
        let components: Vec<_> = relative.components().collect();
        let folder_parts = if item.dir { components.len() } else { components.len() - 1 };
        let mut prefix = PathBuf::new();
        for component in &components[..folder_parts] {
            prefix.push(component);
            if target.join(&prefix).is_file() {
                conflicts.insert(format!("{} (file in place of a folder)", prefix.display()));
            }
        }
        if !item.dir && target.join(&relative).exists() {
            conflicts.insert(item.name);
        }
    }
    Ok(conflicts.into_iter().collect())
}
/// Writes archive members under `target`, resolving clashes with `policy`.
struct Extractor<'a> {
    target: &'a Path,
    policy: ConflictPolicy,
    renamed_dirs: std::collections::HashMap<PathBuf, PathBuf>,
    report: ExtractReport,
}
impl Extractor<'_> {
    fn entry(
        &mut self,
        relative: &Path,
        is_dir: bool,
        reader: &mut dyn std::io::Read,
        mode: Option<u32>,
    ) -> std::io::Result<()> {
        let Some(mut output) = resolve_folders(
            self.target,
            relative,
            is_dir,
            self.policy,
            &mut self.renamed_dirs,
            &mut self.report,
        )
        .map_err(|error| with_path(error, relative))?
        else {
            return Ok(());
        };
        if is_dir {
            return Ok(());
        }
        if output.exists() {
            match self.policy {
                ConflictPolicy::Skip => {
                    self.report.skipped += 1;
                    return Ok(());
                }
                ConflictPolicy::Rename => {
                    output = unique_path(&output);
                    self.report.renamed += 1;
                }
                ConflictPolicy::Overwrite => {
                    if output.is_dir() {
                        // A folder in the way of a file cannot be overwritten safely.
                        self.report.skipped += 1;
                        return Ok(());
                    }
                    // Removing first also replaces read-only files.
                    std::fs::remove_file(&output).map_err(|error| with_path(error, relative))?;
                    self.report.overwritten += 1;
                }
            }
        }
        let mut writer = std::fs::File::create(&output).map_err(|error| with_path(error, relative))?;
        std::io::copy(reader, &mut writer).map_err(|error| with_path(error, relative))?;
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&output, std::fs::Permissions::from_mode(mode & 0o777))?;
        }
        #[cfg(not(unix))]
        let _ = mode;
        self.report.files += 1;
        Ok(())
    }
}
/// Extracts every member of a ZIP, TAR, TAR.GZ or GZ into `target`.
pub fn extract_archive(
    archive: &Path,
    target: &Path,
    policy: ConflictPolicy,
) -> std::io::Result<ExtractReport> {
    let kind = ArchiveKind::of(archive).ok_or_else(unsupported)?;
    std::fs::create_dir_all(target)?;
    let mut extractor = Extractor {
        target,
        policy,
        renamed_dirs: std::collections::HashMap::new(),
        report: ExtractReport::default(),
    };
    match kind {
        ArchiveKind::Zip => {
            let mut zip = open_zip(archive)?;
            for index in 0..zip.len() {
                let mut file = zip.by_index(index).map_err(zip_error)?;
                // Symlinks are skipped like in TAR: their target could point
                // outside the destination folder.
                if file.is_symlink() {
                    extractor.report.skipped += 1;
                    continue;
                }
                let Some(relative) = file.enclosed_name() else {
                    extractor.report.skipped += 1;
                    continue;
                };
                let (is_dir, mode) = (file.is_dir(), file.unix_mode());
                extractor.entry(&relative, is_dir, &mut file, mode)?;
            }
        }
        ArchiveKind::Tar | ArchiveKind::TarGz => {
            let mut tar = open_tar(archive, kind == ArchiveKind::TarGz)?;
            for entry in tar.entries()? {
                let mut entry = entry?;
                let entry_type = entry.header().entry_type();
                // Links and devices are skipped: they could point outside the target.
                if !entry_type.is_dir() && !entry_type.is_file() {
                    extractor.report.skipped += 1;
                    continue;
                }
                let Some(relative) = sanitize(&entry.path()?) else {
                    extractor.report.skipped += 1;
                    continue;
                };
                let mode = entry.header().mode().ok();
                extractor.entry(&relative, entry_type.is_dir(), &mut entry, mode)?;
            }
        }
        ArchiveKind::Rar => {
            let mut rar = open_rar(archive)?.open_for_processing().map_err(rar_error)?;
            while let Some(header) = rar.read_header().map_err(rar_error)? {
                let entry = header.entry();
                let is_dir = entry.is_directory();
                let Some(relative) = sanitize(&entry.filename).filter(|_| is_dir || entry.is_file()) else {
                    extractor.report.skipped += 1;
                    rar = header.skip().map_err(rar_error)?;
                    continue;
                };
                if is_dir {
                    extractor.entry(&relative, true, &mut std::io::empty(), None)?;
                    rar = header.skip().map_err(rar_error)?;
                } else {
                    let (bytes, next) = header.read().map_err(rar_error)?;
                    extractor.entry(&relative, false, &mut bytes.as_slice(), None)?;
                    rar = next;
                }
            }
        }
        ArchiveKind::Gz => {
            let name = gz_member_name(archive)?;
            let mut decoder = flate2::read::MultiGzDecoder::new(std::io::BufReader::new(
                std::fs::File::open(archive)?,
            ));
            extractor.entry(Path::new(&name), false, &mut decoder, None)?;
        }
    }
    Ok(extractor.report)
}
fn open_rar(path: &Path) -> std::io::Result<unrar::Archive<'_>> {
    if !path.is_file() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("{} not found", path.display())));
    }
    Ok(unrar::Archive::new(path))
}

fn rar_error(error: unrar::error::UnrarError) -> std::io::Error {
    std::io::Error::other(format!("RAR: {error}"))
}

/// RAR stores MS-DOS local time: date in the high 16 bits, time in the low.
fn dos_time(value: u32) -> Option<chrono::NaiveDateTime> {
    let (date, time) = (value >> 16, value & 0xffff);
    chrono::NaiveDate::from_ymd_opt(1980 + (date >> 9) as i32, (date >> 5) & 0xf, date & 0x1f)?
        .and_hms_opt(time >> 11, (time >> 5) & 0x3f, (time & 0x1f) * 2)
}

fn with_path(error: std::io::Error, path: &Path) -> std::io::Error {
    std::io::Error::new(error.kind(), format!("{}: {error}", path.display()))
}
/// Creates the folders `relative` needs under `target`. A file sitting where a
/// folder must go is resolved with `policy`: overwritten (deleted), renamed
/// around (the folder becomes `name (1)`), or the entry is skipped (`None`).
fn resolve_folders(
    target: &Path,
    relative: &Path,
    is_dir: bool,
    policy: ConflictPolicy,
    renamed_dirs: &mut std::collections::HashMap<PathBuf, PathBuf>,
    report: &mut ExtractReport,
) -> std::io::Result<Option<PathBuf>> {
    let components: Vec<_> = relative.components().collect();
    let folder_parts = if is_dir { components.len() } else { components.len() - 1 };
    let mut current = target.to_path_buf();
    let mut original = PathBuf::new();
    for component in &components[..folder_parts] {
        original.push(component);
        let next = match renamed_dirs.get(&original) {
            Some(renamed) => renamed.clone(),
            None => current.join(component),
        };
        if next.is_file() {
            match policy {
                ConflictPolicy::Skip => {
                    report.skipped += usize::from(!is_dir);
                    return Ok(None);
                }
                ConflictPolicy::Overwrite => {
                    std::fs::remove_file(&next)?;
                    report.overwritten += 1;
                    std::fs::create_dir(&next)?;
                }
                ConflictPolicy::Rename => {
                    let renamed = unique_path(&next);
                    std::fs::create_dir(&renamed)?;
                    renamed_dirs.insert(original.clone(), renamed.clone());
                    report.renamed += 1;
                    current = renamed;
                    continue;
                }
            }
        } else {
            std::fs::create_dir_all(&next)?;
        }
        current = next;
    }
    if is_dir {
        return Ok(Some(current));
    }
    Ok(components.last().map(|name| current.join(name)))
}
pub fn parent_path(path: &str) -> &str {
    let path = path.trim_end_matches('/');
    path.rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("")
}

#[cfg(test)]
mod rar_tests {
    use super::*;

    /// `solid.rar` comes from the unrar crate's own test data.
    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/solid.rar")
    }

    #[test]
    fn rar_archives_list_and_extract() {
        assert_eq!(ArchiveKind::of(Path::new("Photos.RAR")), Some(ArchiveKind::Rar));
        let items = list_items(&fixture()).unwrap();
        assert!(!items.is_empty(), "nothing listed");
        assert!(items.iter().all(|item| item.modified.is_some()), "a RAR date did not parse");

        let target = std::env::temp_dir().join(format!("files-rar-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&target);
        let report = extract_archive(&fixture(), &target, ConflictPolicy::Rename).unwrap();
        let files = items.iter().filter(|item| !item.dir).count();
        assert_eq!(report.files, files);
        for item in items.iter().filter(|item| !item.dir) {
            let written = std::fs::metadata(target.join(&item.name)).unwrap();
            assert_eq!(written.len(), item.size, "{}", item.name);
        }
        std::fs::remove_dir_all(&target).unwrap();
    }
}
