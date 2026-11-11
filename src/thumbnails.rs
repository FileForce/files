//! Previews for image formats GPUI cannot decode itself (HEIC, HEIF, AVIF)
//! and Quick Look renderings (PDF, Office, video…). They are converted once,
//! in the background, to a cached image. Every conversion runs through the
//! event journal with a timeout, so a hanging converter can never leave a
//! preview "generating" forever.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};

/// Longest edge of a cached preview, in pixels.
const PREVIEW_SIZE: u32 = 1024;
/// A converter taking longer than this is stopped and the preview given up.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
/// Journal source for conversions.
const SOURCE: &str = "preview";

pub fn needs_conversion(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(extension.to_ascii_lowercase().as_str(), "heic" | "heif" | "hif" | "avif")
        })
}

/// Longest edge of a list or grid thumbnail, in pixels.
const THUMB_SIZE: u32 = 320;
/// Images up to this size are shown as they are (decoding them is cheap).
const SMALL_IMAGE: u64 = 300 * 1024;

/// A small version of an image for lists and grids: decoded full size, a
/// 12-megapixel photo takes 48 MB, and GPUI keeps what it decoded. Small
/// files are used as they are; big ones get a 320 px JPEG, made once in the
/// background. `None` while it is being made (the icon shows).
pub fn thumbnail(path: &Path) -> Option<PathBuf> {
    if needs_conversion(path) {
        return source(path);
    }
    if is_svg(path) {
        // GPUI draws an SVG at its own size: a 1024 px one with filters is
        // ~290 ms and 4 MB per card. Rasterized once at 320 px instead.
        let cached = cache_path(path)?.with_extension("svg.png");
        if cached.is_file() {
            return Some(cached);
        }
        if state().failed.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).contains_key(&cached) {
            return Some(path.to_path_buf());
        }
        enqueue(path, cached);
        return None;
    }
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() <= SMALL_IMAGE {
        return Some(path.to_path_buf());
    }
    let cached = cache_path(path)?.with_extension("thumb.jpg");
    if cached.is_file() {
        return Some(cached);
    }
    if state().failed.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).contains_key(&cached) {
        // Cannot be shrunk (an odd format): the file itself, as before.
        return Some(path.to_path_buf());
    }
    enqueue(path, cached);
    None
}

fn is_svg(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
}

/// Rasterizes an SVG to a PNG at most THUMB_SIZE on its longer side (transparency kept).
pub(crate) fn rasterize_svg(source: &Path, target: &Path) -> std::io::Result<()> {
    let bytes = std::fs::read(source)?;
    let tree = usvg::Tree::from_data(&bytes, &usvg::Options::default()).map_err(|error| std::io::Error::other(error.to_string()))?;
    let size = tree.size();
    let scale = THUMB_SIZE as f32 / size.width().max(size.height());
    let width = ((size.width() * scale).round() as u32).max(1);
    let height = ((size.height() * scale).round() as u32).max(1);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or_else(|| std::io::Error::other("empty SVG"))?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let partial = target.with_extension("partial.png");
    pixmap.save_png(&partial).map_err(|error| std::io::Error::other(error.to_string()))?;
    std::fs::rename(&partial, target)
}

/// Decodes and shrinks an image to a JPEG thumbnail.
fn shrink(source: &Path, target: &Path) -> std::io::Result<()> {
    let image = image::ImageReader::open(source)?
        .with_guessed_format()?
        .decode()
        .map_err(|error| std::io::Error::other(format!("cannot decode: {error}")))?;
    let small = image.thumbnail(THUMB_SIZE, THUMB_SIZE).to_rgb8();
    let partial = target.with_extension("partial.jpg");
    small.save_with_format(&partial, image::ImageFormat::Jpeg).map_err(|error| std::io::Error::other(error.to_string()))?;
    std::fs::rename(&partial, target)
}

/// What to hand to `img()`: the file itself, a ready cached conversion, or
/// `None` while (or because) a conversion is not available.
pub fn source(path: &Path) -> Option<PathBuf> {
    if !needs_conversion(path) {
        return Some(path.to_path_buf());
    }
    let cached = cache_path(path)?;
    if cached.is_file() {
        return Some(cached);
    }
    enqueue(path, cached);
    None
}

#[derive(Clone, Debug, PartialEq)]
pub enum Preview {
    Ready(PathBuf),
    Pending,
    /// Why there is no preview.
    Unavailable(String),
}

/// macOS previews anything Quick Look can; elsewhere PDFs (first page).
fn renderable(path: &Path) -> bool {
    cfg!(target_os = "macos") || path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

/// Quick Look rendering state for the preview panel.
pub fn quick_look_preview(path: &Path) -> Preview {
    if !renderable(path) {
        return Preview::Unavailable("No preview for this kind of file".to_string());
    }
    let Some(target) = cache_path(path).map(|cached| cached.with_extension("ql.png")) else {
        return Preview::Unavailable("The file cannot be read".to_string());
    };
    if let Some(reason) = state().failed.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(&target) {
        return Preview::Unavailable(reason.clone());
    }
    match quick_look(path) {
        Some(ready) => Preview::Ready(ready),
        None => Preview::Pending,
    }
}

/// A large Quick Look rendering of any file macOS can preview (PDF, Office,
/// video, fonts…), for the Quick Look panel. `None` until it is ready or when
/// the system cannot preview the file.
pub fn quick_look(path: &Path) -> Option<PathBuf> {
    if !renderable(path) {
        return None;
    }
    let cached = cache_path(path)?.with_extension("ql.png");
    if cached.is_file() {
        return Some(cached);
    }
    enqueue(path, cached);
    None
}

fn enqueue(path: &Path, cached: PathBuf) {
    let state = state();
    let mut queued = state.queued.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if queued.insert(cached.clone()) {
        // Quick Look (the big preview the user is looking at) has its own
        // worker, so it never waits behind a folder full of thumbnails.
        let queue = if cached.to_string_lossy().ends_with(".ql.png") { &state.quick_look } else { &state.queue };
        let job = crate::tasks::queue(job_kind(&cached), path.display().to_string());
        let _ = queue.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).send((path.to_path_buf(), cached, job));
    }
}

/// A macOS app bundle (its icon is in Contents/Resources).
fn mac_app(path: &Path) -> bool {
    cfg!(target_os = "macos") && path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("app")) && path.is_dir()
}

/// Elsewhere there are no app bundles.
#[cfg(not(target_os = "macos"))]
fn mac_app_icon(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "app bundles are macOS's"))
}

/// The app's .icns (CFBundleIconFile in its Info.plist), as a 256 px PNG.
#[cfg(target_os = "macos")]
fn mac_app_icon(app: &Path, target: &Path) -> std::io::Result<()> {
    let contents = app.join("Contents");
    let info: plist::Dictionary = plist::from_file(contents.join("Info.plist")).map_err(|error| std::io::Error::other(error.to_string()))?;
    let name = info.get("CFBundleIconFile").and_then(|value| value.as_string()).unwrap_or("AppIcon");
    let name = if name.ends_with(".icns") { name.to_string() } else { format!("{name}.icns") };
    let icns = contents.join("Resources").join(name);
    if !icns.is_file() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no .icns (the icon is in an asset catalog)"));
    }
    let partial = target.with_extension("partial.png");
    let output = files_sdk::log::run(SOURCE, std::process::Command::new("/usr/bin/sips").args(["-s", "format", "png", "-Z", "256"]).arg(&icns).arg("--out").arg(&partial))?;
    if !output.status.success() || !partial.is_file() {
        return Err(std::io::Error::other("sips could not read the icon"));
    }
    std::fs::rename(&partial, target)
}

/// A program's own icon (from a Windows .exe or .dll), made once in the
/// background. `None` while it is made, or when it has none (the generic
/// icon shows).
pub fn app_icon(path: &Path) -> Option<PathBuf> {
    if !crate::pe_info::applies(path) && !mac_app(path) {
        return None;
    }
    let cached = cache_path(path)?.with_extension("icon.png");
    if cached.is_file() {
        return Some(cached);
    }
    if state().failed.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).contains_key(&cached) {
        return None;
    }
    enqueue(path, cached);
    None
}

/// What a conversion is, for DevTools → Tasks.
fn job_kind(target: &Path) -> &'static str {
    let name = target.to_string_lossy();
    if name.ends_with(".ql.png") {
        "quick look"
    } else if name.ends_with(".icon.png") {
        "program icon"
    } else if name.ends_with(".thumb.jpg") || name.ends_with(".svg.png") {
        "thumbnail"
    } else {
        "image conversion"
    }
}

/// DevTools → Retry: forgets that `source` failed (or is queued), so the
/// next time it shows, it is made again.
pub fn retry(source: &str) {
    let Some(base) = cache_path(Path::new(source)) else { return };
    let targets = [base.clone(), base.with_extension("thumb.jpg"), base.with_extension("svg.png"), base.with_extension("ql.png"), base.with_extension("icon.png")];
    let state = state();
    let mut failed = state.failed.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut queued = state.queued.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    for target in &targets {
        failed.remove(target);
        queued.remove(target);
    }
    state.generation.fetch_add(1, Ordering::Relaxed);
}

/// DevTools → Indexes: failed conversions forgotten, so all are tried again.
pub fn forget_failures() {
    let state = state();
    state.failed.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
    state.queued.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
    state.generation.fetch_add(1, Ordering::Relaxed);
}

/// (waiting, failed) conversions.
pub fn stats() -> (usize, usize) {
    let state = state();
    (state.queued.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).len(), state.failed.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).len())
}

/// Bumped whenever a conversion finishes, so views know to repaint.
pub fn generation() -> u64 {
    state().generation.load(Ordering::Relaxed)
}

struct State {
    queue: Mutex<Sender<(PathBuf, PathBuf, u64)>>,
    quick_look: Mutex<Sender<(PathBuf, PathBuf, u64)>>,
    queued: Mutex<HashSet<PathBuf>>,
    /// Targets whose conversion failed, and why, so callers stop waiting.
    failed: Mutex<HashMap<PathBuf, String>>,
    generation: AtomicU64,
}

fn worker(name: &str) -> Sender<(PathBuf, PathBuf, u64)> {
    let (sender, receiver) = channel::<(PathBuf, PathBuf, u64)>();
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            while let Ok((source, target, job)) = receiver.recv() {
                // Cancelled in DevTools → Tasks while it waited.
                if !crate::tasks::start(job) {
                    state().queued.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(&target);
                    continue;
                }
                let result = convert(&source, &target);
                crate::tasks::finish(job, result.as_ref().map(|_| ()).map_err(ToString::to_string));
                if let Err(error) = result {
                    state()
                        .failed
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .insert(target, error.to_string());
                }
                state().generation.fetch_add(1, Ordering::Relaxed);
            }
        })
        .expect("failed to spawn a preview worker");
    sender
}

fn state() -> &'static State {
    static STATE: OnceLock<State> = OnceLock::new();
    STATE.get_or_init(|| State {
        // One worker each: converters are heavy and previews are only for what is visible.
        queue: Mutex::new(worker("thumbnails")),
        quick_look: Mutex::new(worker("quick-look")),
        queued: Mutex::new(HashSet::new()),
        failed: Mutex::new(HashMap::new()),
        generation: AtomicU64::new(0),
    })
}

/// Cache file keyed by path, size and modification time, so an edited image
/// gets a fresh preview.
fn cache_path(path: &Path) -> Option<PathBuf> {
    let metadata = std::fs::metadata(path).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    metadata.modified().ok().hash(&mut hasher);
    Some(cache_dir()?.join(format!("{:016x}.jpg", hasher.finish())))
}

pub(crate) fn cache_dir() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = dirs::cache_dir()?.join("file-force").join("thumbnails");
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    })
    .clone()
}

/// Converts with the OS tool, writing through a temp file so a half-written
/// JPEG is never picked up as a finished preview.
fn convert(source: &Path, target: &Path) -> std::io::Result<()> {
    // Cloud placeholders would block the worker until they time out.
    if crate::model::is_online_only(source) {
        return Err(std::io::Error::other("online-only file"));
    }
    if target.to_string_lossy().ends_with(".ql.png") {
        return quick_look_render(source, target);
    }
    if target.to_string_lossy().ends_with(".thumb.jpg") {
        return shrink(source, target);
    }
    if target.to_string_lossy().ends_with(".svg.png") {
        return rasterize_svg(source, target);
    }
    if target.to_string_lossy().ends_with(".icon.png") {
        if mac_app(source) {
            return mac_app_icon(source, target);
        }
        return crate::pe_info::save_icon(source, target, 256);
    }
    let partial = target.with_extension("partial.jpg");
    let output = converter(source, &partial)?;
    if !output.status.success() || !partial.is_file() {
        let _ = std::fs::remove_file(&partial);
        return Err(std::io::Error::other(format!("converter exited with {}", output.status)));
    }
    std::fs::rename(&partial, target)
}

/// PDFs render fastest (and most reliably) with `sips`, which draws the
/// first page; everything else goes through Quick Look's `qlmanage -t`,
/// which writes `<name>.png` into a folder that is then moved into the cache.
#[cfg(target_os = "macos")]
fn quick_look_render(source: &Path, target: &Path) -> std::io::Result<()> {
    let is_pdf = source.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
    if is_pdf {
        let partial = target.with_extension("partial.png");
        let mut sips = std::process::Command::new("/usr/bin/sips");
        sips.args(["-s", "format", "png", "-Z", "1600"]).arg(source).arg("--out").arg(&partial);
        let rendered = files_sdk::log::run_with_timeout(SOURCE, &mut sips, TIMEOUT);
        if rendered.is_ok_and(|output| output.status.success()) && partial.is_file() {
            return std::fs::rename(&partial, target);
        }
        let _ = std::fs::remove_file(&partial);
    }
    let scratch = target.with_extension("ql-dir");
    std::fs::create_dir_all(&scratch)?;
    let mut qlmanage = std::process::Command::new("/usr/bin/qlmanage");
    qlmanage.args(["-t", "-s", "1200", "-o"]).arg(&scratch).arg(source);
    let output = files_sdk::log::run_with_timeout(SOURCE, &mut qlmanage, TIMEOUT);
    let rendered = std::fs::read_dir(&scratch)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "png"));
    let result = match (output, rendered) {
        (Err(error), _) if error.kind() == std::io::ErrorKind::TimedOut => {
            Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "Quick Look took too long and was stopped"))
        }
        (Ok(output), Some(rendered)) if output.status.success() => std::fs::rename(rendered, target),
        (Err(error), _) => Err(error),
        _ => Err(std::io::Error::other("Quick Look cannot preview this file")),
    };
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// Windows: the first page through the built-in `Windows.Data.Pdf` API
/// (Windows 10+), driven from PowerShell, so nothing has to be installed.
#[cfg(windows)]
fn quick_look_render(source: &Path, target: &Path) -> std::io::Result<()> {
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$await = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' } | Select-Object -First 1
function Await($operation, [Type]$type) { $task = $await.MakeGenericMethod($type).Invoke($null, @($operation)); $task.Wait(-1) | Out-Null; $task.Result }
$awaitAction = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncAction' } | Select-Object -First 1
[void][Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime]
[void][Windows.Data.Pdf.PdfDocument, Windows.Data.Pdf, ContentType = WindowsRuntime]
$file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($env:FILES_PDF)) ([Windows.Storage.StorageFile])
$pdf = Await ([Windows.Data.Pdf.PdfDocument]::LoadFromFileAsync($file)) ([Windows.Data.Pdf.PdfDocument])
$page = $pdf.GetPage(0)
$folder = Await ([Windows.Storage.StorageFolder]::GetFolderFromPathAsync($env:FILES_OUT_DIR)) ([Windows.Storage.StorageFolder])
$out = Await ($folder.CreateFileAsync($env:FILES_OUT_NAME, [Windows.Storage.CreationCollisionOption]::ReplaceExisting)) ([Windows.Storage.StorageFile])
$stream = Await ($out.OpenAsync([Windows.Storage.FileAccessMode]::ReadWrite)) ([Windows.Storage.Streams.IRandomAccessStream])
$options = New-Object Windows.Data.Pdf.PdfPageRenderOptions
$options.DestinationWidth = 1600
$awaitAction.Invoke($null, @($page.RenderToStreamAsync($stream, $options))).Wait(-1) | Out-Null
$stream.Dispose()
"#;
    let partial = target.with_extension("partial.png");
    let folder = partial.parent().ok_or_else(|| std::io::Error::other("no cache folder"))?;
    let name = partial.file_name().ok_or_else(|| std::io::Error::other("no file name"))?;
    let mut powershell = files_sdk::log::command("powershell");
    powershell
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT])
        .env("FILES_PDF", source)
        .env("FILES_OUT_DIR", folder)
        .env("FILES_OUT_NAME", name);
    finish_render(files_sdk::log::run_with_timeout(SOURCE, &mut powershell, TIMEOUT), &partial, target)
}

/// Linux: poppler's `pdftoppm` (package poppler-utils), first page only.
#[cfg(all(unix, not(target_os = "macos")))]
fn quick_look_render(source: &Path, target: &Path) -> std::io::Result<()> {
    let partial = target.with_extension("partial");
    let mut pdftoppm = std::process::Command::new("pdftoppm");
    pdftoppm.args(["-png", "-singlefile", "-f", "1", "-l", "1", "-scale-to", "1600"]).arg(source).arg(&partial);
    // pdftoppm adds `.png` itself.
    finish_render(files_sdk::log::run_with_timeout(SOURCE, &mut pdftoppm, TIMEOUT), &partial.with_extension("partial.png"), target)
}

/// Moves a finished rendering into the cache, or says why there is none.
#[cfg(not(target_os = "macos"))]
fn finish_render(output: std::io::Result<std::process::Output>, partial: &Path, target: &Path) -> std::io::Result<()> {
    match output {
        Ok(output) if output.status.success() && partial.is_file() => std::fs::rename(partial, target),
        Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
            let _ = std::fs::remove_file(partial);
            Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "The PDF took too long to render and was stopped"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "Install poppler-utils (pdftoppm) to preview PDFs"))
        }
        Err(error) => Err(error),
        Ok(_) => {
            let _ = std::fs::remove_file(partial);
            Err(std::io::Error::other("This PDF cannot be rendered"))
        }
    }
}

#[cfg(target_os = "macos")]
fn converter(source: &Path, target: &Path) -> std::io::Result<std::process::Output> {
    let mut sips = std::process::Command::new("/usr/bin/sips");
    sips.args(["-s", "format", "jpeg", "-Z", &PREVIEW_SIZE.to_string()]).arg(source).arg("--out").arg(target);
    files_sdk::log::run_with_timeout(SOURCE, &mut sips, TIMEOUT)
}

/// Linux: libheif's `heif-convert` (package libheif-examples / libheif-tools).
#[cfg(all(unix, not(target_os = "macos")))]
fn converter(source: &Path, target: &Path) -> std::io::Result<std::process::Output> {
    let _ = PREVIEW_SIZE;
    let mut convert = std::process::Command::new("heif-convert");
    convert.args(["-q", "85"]).arg(source).arg(target);
    files_sdk::log::run_with_timeout(SOURCE, &mut convert, TIMEOUT)
}

/// Windows has no converter we can count on; previews fall back to the icon.
#[cfg(not(unix))]
fn converter(_: &Path, _: &Path) -> std::io::Result<std::process::Output> {
    let _ = PREVIEW_SIZE;
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no HEIC converter"))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_undecodable_formats_are_converted() {
        assert!(needs_conversion(Path::new("IMG_0001.HEIC")));
        assert!(needs_conversion(Path::new("a.avif")));
        assert!(!needs_conversion(Path::new("a.webp")), "GPUI decodes WebP itself");
        assert!(!needs_conversion(Path::new("a.jpg")));
        assert_eq!(source(Path::new("/x/a.webp")), Some(PathBuf::from("/x/a.webp")));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn heic_is_converted_to_a_cached_jpeg() {
        let dir = std::env::temp_dir().join(format!("file-force-heic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let heic = dir.join("photo.heic");
        let jpg = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/images/facade.jpg");
        let made = std::process::Command::new("/usr/bin/sips")
            .args(["-s", "format", "heic"])
            .arg(&jpg)
            .arg("--out")
            .arg(&heic)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(made.success());

        assert_eq!(source(&heic), None, "first call only queues the conversion");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let cached = loop {
            if let Some(cached) = source(&heic) {
                break cached;
            }
            assert!(std::time::Instant::now() < deadline, "conversion never finished");
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let bytes = std::fs::read(&cached).unwrap();
        assert_eq!(&bytes[..3], &[0xFF, 0xD8, 0xFF], "cached preview is a JPEG");
        assert!(generation() > 0);
        let _ = std::fs::remove_file(cached);
    }

    /// The smallest valid one-page PDF.
    const PDF: &str = "%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";

    fn wait_for_preview(path: &Path) -> Preview {
        for _ in 0..400 {
            match quick_look_preview(path) {
                Preview::Pending => std::thread::sleep(std::time::Duration::from_millis(50)),
                done => return done,
            }
        }
        Preview::Pending
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn pdf_previews_finish_and_failures_say_why() {
        let dir = std::env::temp_dir().join(format!("file-force-preview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("one page.pdf");
        std::fs::write(&pdf, PDF).unwrap();
        assert!(matches!(wait_for_preview(&pdf), Preview::Ready(_)), "the PDF preview never finished");
        // The conversion is in the journal (Settings → Developer → Console).
        assert!(files_sdk::log::events().iter().any(|event| event.source == SOURCE && event.message.contains("one page.pdf")));

        // Something Quick Look cannot draw ends with a reason, not "generating" forever.
        let junk = dir.join("noise.zzqq");
        std::fs::write(&junk, [0u8, 159, 146, 150].repeat(64)).unwrap();
        match wait_for_preview(&junk) {
            Preview::Unavailable(reason) => assert!(!reason.is_empty()),
            other => panic!("expected a failure, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
