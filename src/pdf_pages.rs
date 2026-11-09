//! PDF pages for Quick Look and the preview, rendered with hayro (pure
//! Rust, every system): each page once, in the background, to a cached PNG,
//! so a document scrolls page after page instead of showing only the first.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Pages beyond this are not rendered (a 900-page manual stays light).
pub const MAX_PAGES: usize = 60;
/// Rendered width of a page, in pixels: sharp in Quick Look, not huge.
const PAGE_WIDTH: f32 = 1100.;

/// What is known about a document's pages.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pages {
    /// Pages in the document.
    pub count: usize,
    /// The rendered ones so far, in order (`None`: not yet).
    pub rendered: Vec<Option<PathBuf>>,
    pub error: Option<String>,
}

static READY: AtomicU64 = AtomicU64::new(0);

/// Bumped as pages arrive: Quick Look redraws.
pub fn generation() -> u64 {
    READY.load(Ordering::Relaxed)
}

fn state() -> &'static Mutex<std::collections::HashMap<PathBuf, Pages>> {
    static STATE: OnceLock<Mutex<std::collections::HashMap<PathBuf, Pages>>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}

/// The folder a document's pages are kept in (by path, size and date).
fn folder(path: &Path) -> Option<PathBuf> {
    use std::hash::{Hash as _, Hasher as _};
    let metadata = std::fs::metadata(path).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    metadata.modified().ok().hash(&mut hasher);
    Some(crate::thumbnails::cache_dir()?.join(format!("{:016x}-pdf", hasher.finish())))
}

/// The pages of `path`: what is rendered so far; the rest are made in the
/// background (asked once per document version).
pub fn pages(path: &Path) -> Pages {
    let Some(dir) = folder(path) else { return Pages { error: Some("Cannot cache pages".into()), ..Default::default() } };
    let mut known = state().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(pages) = known.get(&dir) {
        return pages.clone();
    }
    known.insert(dir.clone(), Pages::default());
    drop(known);
    let source = path.to_path_buf();
    let job = crate::tasks::queue("pdf pages", source.display().to_string());
    std::thread::spawn(move || {
        crate::tasks::start(job);
        let result = render(&source, &dir);
        crate::tasks::finish(job, result.clone());
        if let Err(error) = result {
            state().lock().unwrap_or_else(|poisoned| poisoned.into_inner()).entry(dir).or_default().error = Some(error);
            READY.fetch_add(1, Ordering::Relaxed);
        }
    });
    Pages::default()
}

fn render(source: &Path, dir: &Path) -> Result<(), String> {
    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::vello_cpu::color::palette::css::WHITE;
    let data = std::fs::read(source).map_err(|error| error.to_string())?;
    let pdf = hayro::hayro_syntax::Pdf::new(data).map_err(|error| format!("Not a readable PDF ({error:?})"))?;
    let pages = pdf.pages();
    let count = pages.len();
    let shown = count.min(MAX_PAGES);
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    {
        let mut known = state().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = known.entry(dir.to_path_buf()).or_default();
        entry.count = count;
        entry.rendered = vec![None; shown];
    }
    READY.fetch_add(1, Ordering::Relaxed);
    let cache = hayro::RenderCache::new();
    let settings = InterpreterSettings::default();
    for (index, page) in pages.iter().take(shown).enumerate() {
        let target = dir.join(format!("page-{index:03}.png"));
        if !target.is_file() {
            let (width, _) = page.render_dimensions();
            let scale = (PAGE_WIDTH / width.max(1.)).clamp(0.5, 4.);
            let pixmap = hayro::render(page, &cache, &settings, &hayro::RenderSettings::default(), &hayro::PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE });
            let png = pixmap.into_png().map_err(|error| error.to_string())?;
            let partial = target.with_extension("partial");
            std::fs::write(&partial, png).map_err(|error| error.to_string())?;
            std::fs::rename(&partial, &target).map_err(|error| error.to_string())?;
        }
        if let Some(slot) = state().lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get_mut(dir).and_then(|pages| pages.rendered.get_mut(index)) {
            *slot = Some(target);
        }
        READY.fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}

pub fn is_pdf(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_broken_pdf_says_why() {
        let path = std::env::temp_dir().join(format!("file-force-broken-{}.pdf", std::process::id()));
        std::fs::write(&path, b"%PDF-1.4 not really").unwrap();
        let dir = std::env::temp_dir().join(format!("file-force-broken-pages-{}", std::process::id()));
        assert!(super::render(&path, &dir).is_err());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod live {
    #[test]
    #[ignore]
    fn render_a_real_pdf() {
        let source = std::path::PathBuf::from(std::env::var("PDF").unwrap());
        let dir = std::env::temp_dir().join(format!("file-force-pdf-live-{}", std::process::id()));
        let started = std::time::Instant::now();
        super::render(&source, &dir).unwrap();
        let pages = std::fs::read_dir(&dir).unwrap().count();
        println!("{pages} pages in {:?}", started.elapsed());
        println!("{}", dir.display());
    }
}
