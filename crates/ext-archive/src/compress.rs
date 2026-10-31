//! Creating archives: ZIP and 7z with a compression level per file type,
//! TAR.GZ, and RAR through the `rar` tool when it is installed.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchiveFormat {
    Zip,
    SevenZip,
    TarGz,
    Rar,
}
impl ArchiveFormat {
    pub const ALL: [Self; 4] = [Self::Zip, Self::SevenZip, Self::TarGz, Self::Rar];

    pub fn label(self) -> &'static str {
        match self {
            Self::Zip => "ZIP",
            Self::SevenZip => "7z",
            Self::TarGz => "TAR.GZ",
            Self::Rar => "RAR",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::SevenZip => "7z",
            Self::TarGz => "tar.gz",
            Self::Rar => "rar",
        }
    }

    /// ZIP and 7z store each file with its own method; TAR.GZ and RAR
    /// compress one stream, so they only take a single level.
    pub fn per_category(self) -> bool {
        matches!(self, Self::Zip | Self::SevenZip)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Level {
    Store,
    Fast,
    Normal,
    Max,
}
impl Level {
    pub const ALL: [Self; 4] = [Self::Store, Self::Fast, Self::Normal, Self::Max];

    pub fn label(self) -> &'static str {
        match self {
            Self::Store => "Store",
            Self::Fast => "Fast",
            Self::Normal => "Normal",
            Self::Max => "Max",
        }
    }

    fn deflate(self) -> u32 {
        match self {
            Self::Store => 0,
            Self::Fast => 1,
            Self::Normal => 6,
            Self::Max => 9,
        }
    }

    fn lzma(self) -> u32 {
        match self {
            Self::Store => 0,
            Self::Fast => 1,
            Self::Normal => 5,
            Self::Max => 9,
        }
    }

    fn rar(self) -> u32 {
        match self {
            Self::Store => 0,
            Self::Fast => 1,
            Self::Normal => 3,
            Self::Max => 5,
        }
    }
}
/// Groups files by how well they compress, so each group gets a sensible default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Category {
    CompressedMedia,
    RawMedia,
    Documents,
    Archives,
    TextCode,
    Other,
}
impl Category {
    pub const ALL: [Self; 6] = [
        Self::CompressedMedia,
        Self::RawMedia,
        Self::Documents,
        Self::Archives,
        Self::TextCode,
        Self::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::CompressedMedia => "Photos, video, music",
            Self::RawMedia => "Uncompressed media",
            Self::Documents => "PDF & office documents",
            Self::Archives => "Archives",
            Self::TextCode => "Text & code",
            Self::Other => "Other files",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::CompressedMedia => "jpg, png, heic, mp4, mov, mp3 · already compressed, Store saves time",
            Self::RawMedia => "bmp, tiff, psd, wav, aiff · shrink a lot",
            Self::Documents => "pdf, docx, xlsx, key · mostly compressed inside",
            Self::Archives => "zip, 7z, gz, rar · cannot shrink further",
            Self::TextCode => "txt, md, json, source code · shrink the most",
            Self::Other => "everything else",
        }
    }

    pub fn default_level(self) -> Level {
        match self {
            Self::CompressedMedia | Self::Archives => Level::Store,
            Self::Documents => Level::Fast,
            Self::RawMedia | Self::TextCode => Level::Max,
            Self::Other => Level::Normal,
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|category| *category == self).unwrap_or(0)
    }

    pub fn of(path: &Path) -> Self {
        let extension = path
            .extension()
            .map(|extension| extension.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        match extension.as_str() {
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "heif" | "avif" | "mp4" | "mov"
            | "mkv" | "avi" | "webm" | "m4v" | "mp3" | "aac" | "m4a" | "flac" | "ogg" | "opus" => {
                Self::CompressedMedia
            }
            "bmp" | "tif" | "tiff" | "psd" | "raw" | "cr2" | "nef" | "dng" | "wav" | "aiff"
            | "aif" | "svg" => Self::RawMedia,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "key" | "pages"
            | "numbers" | "odt" | "ods" | "epub" | "fig" | "sketch" => Self::Documents,
            "zip" | "7z" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "rar" | "dmg" | "jar"
            | "crate" | "iso" => Self::Archives,
            "txt" | "md" | "json" | "csv" | "xml" | "html" | "css" | "js" | "ts" | "tsx"
            | "rs" | "py" | "go" | "java" | "kt" | "swift" | "c" | "h" | "cpp" | "hpp" | "rb"
            | "php" | "sh" | "toml" | "yaml" | "yml" | "sql" | "log" | "lock" | "rtf" => {
                Self::TextCode
            }
            _ => Self::Other,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompressSettings {
    pub format: ArchiveFormat,
    /// Indexed by `Category::ALL`; used by ZIP and 7z.
    pub levels: [Level; 6],
    /// Used by TAR.GZ and RAR.
    pub single_level: Level,
}
impl Default for CompressSettings {
    fn default() -> Self {
        Self {
            format: ArchiveFormat::Zip,
            levels: Category::ALL.map(Category::default_level),
            single_level: Level::Normal,
        }
    }
}
impl CompressSettings {
    pub fn level_for(&self, category: Category) -> Level {
        self.levels[category.index()]
    }
}
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub path: PathBuf,
    /// Archive-relative name with `/` separators.
    pub name: String,
    pub size: u64,
    pub category: Category,
}
#[derive(Clone, Debug, Default)]
pub struct Sources {
    pub files: Vec<SourceFile>,
    pub dirs: Vec<String>,
    /// Cloud placeholders without local data; reading them would hang.
    pub online_only: Vec<String>,
}
fn archive_name(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
/// Walks `sources` (without following symlinks). Names are relative to each
/// source's parent, so a folder keeps its own name as the archive root.
pub fn collect_sources(sources: &[PathBuf]) -> io::Result<Sources> {
    let mut collected = Sources::default();
    for source in sources {
        let base = source.parent().unwrap_or(Path::new(""));
        let mut stack = vec![source.clone()];
        while let Some(path) = stack.pop() {
            let metadata = fs::symlink_metadata(&path)?;
            let name = archive_name(path.strip_prefix(base).unwrap_or(&path));
            if metadata.is_dir() {
                collected.dirs.push(name);
                let mut children: Vec<_> = fs::read_dir(&path)?
                    .filter_map(Result::ok)
                    .map(|child| child.path())
                    .collect();
                children.sort();
                stack.extend(children.into_iter().rev());
            } else if metadata.is_file() {
                if files_sdk::fs::is_online_only(&path) {
                    collected.online_only.push(name);
                    continue;
                }
                collected.files.push(SourceFile {
                    category: Category::of(&path),
                    size: metadata.len(),
                    name,
                    path,
                });
            }
        }
    }
    Ok(collected)
}
/// Writes the archive to `output` via a temporary file, so a failed run never
/// leaves a half-written archive behind. Returns the archive size.
pub fn create_archive(
    sources: &Sources,
    roots: &[PathBuf],
    output: &Path,
    settings: CompressSettings,
    progress: &AtomicUsize,
) -> io::Result<u64> {
    let partial = output.with_file_name(format!(
        ".{}.partial",
        output.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned())
    ));
    let result = match settings.format {
        ArchiveFormat::Zip => write_zip(sources, &partial, settings, progress),
        ArchiveFormat::SevenZip => write_7z(sources, &partial, settings, progress),
        ArchiveFormat::TarGz => write_tar_gz(sources, &partial, settings, progress),
        ArchiveFormat::Rar => write_rar(roots, &partial, settings),
    };
    if let Err(error) = result {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    fs::rename(&partial, output)?;
    Ok(fs::metadata(output)?.len())
}
fn write_zip(
    sources: &Sources,
    output: &Path,
    settings: CompressSettings,
    progress: &AtomicUsize,
) -> io::Result<()> {
    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;

    let mut writer = zip::ZipWriter::new(File::create(output)?);
    for dir in &sources.dirs {
        writer
            .add_directory(format!("{dir}/"), SimpleFileOptions::default())
            .map_err(io::Error::other)?;
    }
    for file in &sources.files {
        let level = settings.level_for(file.category);
        let mut options = SimpleFileOptions::default().large_file(file.size >= u32::MAX as u64);
        options = if level == Level::Store {
            options.compression_method(CompressionMethod::Stored)
        } else {
            options
                .compression_method(CompressionMethod::Deflated)
                .compression_level(Some(level.deflate() as i64))
        };
        let metadata = fs::metadata(&file.path)?;
        if let Some(time) = metadata.modified().ok().and_then(zip_time) {
            options = options.last_modified_time(time);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            options = options.unix_permissions(metadata.permissions().mode());
        }
        writer.start_file(file.name.as_str(), options).map_err(io::Error::other)?;
        io::copy(&mut File::open(&file.path)?, &mut writer)?;
        progress.fetch_add(1, Ordering::Relaxed);
    }
    writer.finish().map_err(io::Error::other)?;
    Ok(())
}
/// Zip stores local time without a zone; dates before 1980 cannot be stored.
fn zip_time(time: std::time::SystemTime) -> Option<zip::DateTime> {
    use chrono::{Datelike as _, Timelike as _};
    let local = chrono::DateTime::<chrono::Local>::from(time);
    zip::DateTime::from_date_and_time(
        u16::try_from(local.year()).ok()?,
        local.month() as u8,
        local.day() as u8,
        local.hour() as u8,
        local.minute() as u8,
        local.second() as u8,
    )
    .ok()
}
fn write_7z(
    sources: &Sources,
    output: &Path,
    settings: CompressSettings,
    progress: &AtomicUsize,
) -> io::Result<()> {
    use sevenz_rust2::encoder_options::Lzma2Options;
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod};

    let mut writer = ArchiveWriter::create(output).map_err(io::Error::other)?;
    for dir in &sources.dirs {
        writer
            .push_archive_entry::<File>(ArchiveEntry::new_directory(dir), None)
            .map_err(io::Error::other)?;
    }
    for file in &sources.files {
        // Methods are per entry, so each category keeps its own setting.
        let method: EncoderConfiguration = match settings.level_for(file.category) {
            Level::Store => EncoderMethod::COPY.into(),
            level => Lzma2Options::from_level(level.lzma()).into(),
        };
        writer.set_content_methods(vec![method]);
        writer
            .push_archive_entry(
                ArchiveEntry::from_path(&file.path, file.name.clone()),
                Some(File::open(&file.path)?),
            )
            .map_err(io::Error::other)?;
        progress.fetch_add(1, Ordering::Relaxed);
    }
    writer.finish()?;
    Ok(())
}
fn write_tar_gz(
    sources: &Sources,
    output: &Path,
    settings: CompressSettings,
    progress: &AtomicUsize,
) -> io::Result<()> {
    let encoder = flate2::write::GzEncoder::new(
        File::create(output)?,
        flate2::Compression::new(settings.single_level.deflate()),
    );
    let mut builder = tar::Builder::new(encoder);
    builder.follow_symlinks(false);
    let mut files = sources.files.iter().peekable();
    for dir in &sources.dirs {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_mode(0o755);
        header.set_size(0);
        builder.append_data(&mut header, format!("{dir}/"), io::empty())?;
    }
    while let Some(file) = files.next() {
        builder.append_path_with_name(&file.path, &file.name)?;
        progress.fetch_add(1, Ordering::Relaxed);
    }
    builder.into_inner()?.finish()?;
    Ok(())
}
/// RAR is proprietary: creating it needs RARLAB's `rar` command-line tool.
pub fn rar_tool() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .chain(["/opt/homebrew/bin".into(), "/usr/local/bin".into()])
        .map(|dir| dir.join(if cfg!(windows) { "rar.exe" } else { "rar" }))
        .find(|candidate| candidate.is_file())
}
fn write_rar(roots: &[PathBuf], output: &Path, settings: CompressSettings) -> io::Result<()> {
    let tool = rar_tool().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "rar tool not found"))?;
    // rar picks the format from the extension, so give the temp file one.
    let named = output.with_extension("rar");
    let status = files_sdk::log::command(tool)
        .arg("a")
        .arg(format!("-m{}", settings.single_level.rar()))
        .args(["-r", "-ep1", "-idq", "-y"])
        .arg(&named)
        .args(roots)
        .status()?;
    if !status.success() {
        return Err(io::Error::other(format!("rar exited with {status}")));
    }
    fs::rename(named, output)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::Read as _;

    /// photo.jpg (media), notes/readme.md (text), data.bin (other).
    fn fixture(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("file-force-compress-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("project");
        fs::create_dir_all(source.join("notes")).unwrap();
        fs::write(source.join("photo.jpg"), vec![7u8; 4096]).unwrap();
        fs::write(source.join("notes/readme.md"), "hello ".repeat(500)).unwrap();
        fs::write(source.join("data.bin"), vec![1u8; 2048]).unwrap();
        (root, source)
    }

    fn build(format: ArchiveFormat, name: &str) -> (PathBuf, PathBuf, Sources) {
        let (root, source) = fixture(name);
        let sources = collect_sources(std::slice::from_ref(&source)).unwrap();
        let output = root.join(format!("out.{}", format.extension()));
        let settings = CompressSettings {
            format,
            ..CompressSettings::default()
        };
        let progress = AtomicUsize::new(0);
        create_archive(&sources, &[source.clone()], &output, settings, &progress).unwrap();
        assert_eq!(progress.load(Ordering::Relaxed), 3);
        (root, output, sources)
    }

    fn expected() -> BTreeMap<String, usize> {
        BTreeMap::from([
            ("project/data.bin".to_string(), 2048),
            ("project/notes/readme.md".to_string(), 3000),
            ("project/photo.jpg".to_string(), 4096),
        ])
    }

    #[test]
    fn categories_pick_sensible_defaults() {
        assert_eq!(Category::of(Path::new("a.JPG")), Category::CompressedMedia);
        assert_eq!(Category::of(Path::new("a.pdf")), Category::Documents);
        assert_eq!(Category::of(Path::new("a.tiff")), Category::RawMedia);
        assert_eq!(Category::of(Path::new("a.rs")), Category::TextCode);
        let settings = CompressSettings::default();
        assert_eq!(settings.level_for(Category::CompressedMedia), Level::Store);
        assert_eq!(settings.level_for(Category::Documents), Level::Fast);
        assert_eq!(settings.level_for(Category::TextCode), Level::Max);
    }

    #[test]
    fn zip_uses_a_method_per_file_type() {
        let (root, output, _) = build(ArchiveFormat::Zip, "zip");
        let mut zip = zip::ZipArchive::new(File::open(&output).unwrap()).unwrap();
        let mut found = BTreeMap::new();
        for index in 0..zip.len() {
            let mut file = zip.by_index(index).unwrap();
            if file.is_dir() {
                continue;
            }
            let method = file.compression();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            found.insert(file.name().unwrap().to_string(), bytes.len());
            assert!(file.last_modified().is_some_and(|time| time.year() > 1980));
            match file.name().unwrap().as_ref() {
                "project/photo.jpg" => assert_eq!(method, zip::CompressionMethod::Stored),
                _ => assert_eq!(method, zip::CompressionMethod::Deflated),
            }
        }
        assert_eq!(found, expected());
        assert!(!root.join(".out.zip.partial").exists());
    }

    #[test]
    fn seven_zip_round_trips() {
        let (_, output, _) = build(ArchiveFormat::SevenZip, "7z");
        let mut found = BTreeMap::new();
        let mut reader = sevenz_rust2::ArchiveReader::open(&output, sevenz_rust2::Password::empty()).unwrap();
        reader
            .for_each_entries(|entry, data| {
                if !entry.is_directory {
                    let mut bytes = Vec::new();
                    data.read_to_end(&mut bytes)?;
                    found.insert(entry.name.clone(), bytes.len());
                }
                Ok(true)
            })
            .unwrap();
        assert_eq!(found, expected());
    }

    #[test]
    fn tar_gz_round_trips() {
        let (_, output, _) = build(ArchiveFormat::TarGz, "targz");
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(File::open(&output).unwrap()));
        let mut found = BTreeMap::new();
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            if entry.header().entry_type().is_dir() {
                continue;
            }
            let name = entry.path().unwrap().to_string_lossy().into_owned();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            found.insert(name, bytes.len());
        }
        assert_eq!(found, expected());
    }

    #[test]
    fn failed_run_leaves_no_partial_file() {
        let (root, source) = fixture("fail");
        let mut sources = collect_sources(&[source.clone()]).unwrap();
        sources.files[0].path = root.join("missing.bin");
        let output = root.join("out.zip");
        let result = create_archive(&sources, &[source], &output, CompressSettings::default(), &AtomicUsize::new(0));
        assert!(result.is_err());
        assert!(!output.exists());
        assert!(!root.join(".out.zip.partial").exists());
    }
}
