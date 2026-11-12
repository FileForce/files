//! Space-bar preview, like Finder's Quick Look: a large modal showing the
//! selected item, with arrows to move through the folder.

use std::cell::Cell;
use std::path::Path;

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _, Render, Styled as _,
    StyledImage as _, Subscription, WeakEntity, Window, actions, div, img, px,
};

use crate::app::{AppearanceSettings, FileExplorer};
use crate::model::FileKind;
use crate::palette::Palette;

actions!(quick_look, [CloseQuickLook, QuickLookPrevious, QuickLookNext, QuickLookOpen]);

/// Text files larger than this are shown truncated.
const TEXT_LIMIT: usize = 64 * 1024;
/// Items shown in a folder preview; the rest are counted.
const FOLDER_ITEMS: usize = 60;

pub struct QuickLookView {
    explorer: WeakEntity<FileExplorer>,
    appearance: Entity<AppearanceSettings>,
    focus_handle: FocusHandle,
    return_focus: Option<FocusHandle>,
    _repaint: Subscription,
    /// The mini player of the video being shown; replaced when moving on.
    video: Option<Entity<crate::views::video_player::VideoPlayer>>,
}

impl Focusable for QuickLookView {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl QuickLookView {
    fn current(&self, cx: &gpui_kit::App) -> Option<crate::model::Entry> {
        self.explorer.upgrade()?.read(cx).selected_entry_any()
    }

    fn close(&mut self, _: &CloseQuickLook, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        if let Some(focus) = &self.return_focus {
            focus.focus(window, cx);
        }
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let _ = self.explorer.update(cx, |explorer, cx| explorer.step_selection(delta, cx));
        cx.notify();
    }

    fn previous(&mut self, _: &QuickLookPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, cx);
    }

    fn next(&mut self, _: &QuickLookNext, _: &mut Window, cx: &mut Context<Self>) {
        self.step(1, cx);
    }

    fn open(&mut self, _: &QuickLookOpen, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.current(cx) else {
            return;
        };
        let _ = self.explorer.update(cx, |explorer, cx| explorer.open_entry(&file.id, window, cx));
    }
}

/// What the big preview area shows for a file.
fn body(file: crate::model::Entry, palette: Palette, settings: &crate::settings::AppSettings) -> AnyElement {
    let placeholder = |message: &str| {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(crate::views::files::file_icon(&file, palette, 72.))
            .child(div().text_sm().text_color(palette.muted).child(message.to_string()))
            .into_any_element()
    };
    if file.is_folder() {
        if let Some(path) = file.path.clone()
            && !warmed(&path, file.kind)
        {
            return placeholder("Loading…");
        }
        return folder(file.clone(), palette, settings).unwrap_or_else(|| placeholder(&file.metadata));
    }
    let Some(path) = file.path.clone() else {
        return placeholder("No preview · this item is not on the local disk");
    };
    // Opens at once; the file is read in the background, then shown.
    if !warmed(&path, file.kind) {
        return placeholder("Loading preview…");
    }
    // A format an extension knows (files_sdk::preview).
    if let Some((name, content)) = crate::ext_preview::preview(&path) {
        return extension_preview(&name, content, palette);
    }
    if file.kind == FileKind::Image
        && let Some(source) = crate::thumbnails::source(&path)
    {
        return image_with(source, palette, settings.image_fit);
    }
    if crate::archive::is_supported(&file) {
        return archive(&path, palette, settings);
    }
    if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("eml")) {
        if let Some(message) = email(&path) {
            return v_flex()
                .id("quick-look-email")
                .debug_selector(|| "quick-look-email".into())
                .size_full()
                .rounded_md()
                .bg(palette.surface)
                .overflow_y_scrollbar()
                .child(div().px_3().pt_3().text_base().font_weight(FontWeight::SEMIBOLD).child(message.subject.clone()))
                .child(div().px_3().text_xs().text_color(palette.muted).child(message.from.clone()))
                .child(div().px_3().pt_2().text_sm().whitespace_normal().child(message.body.chars().take(2000).collect::<String>()))
                .child(crate::views::mail::render_details(&message, palette))
                .into_any_element();
        }
    }
    if let Some(text) = cached_text(&path, file.kind) {
        return div()
            .id("quick-look-text")
            .size_full()
            .p_4()
            .rounded_md()
            .bg(palette.surface)
            .overflow_y_scrollbar()
            .child(
                div()
                    .text_xs()
                    .font_family("Menlo")
                    .text_color(palette.text)
                    .whitespace_normal()
                    .child(text),
            )
            .into_any_element();
    }
    // Word, OpenDocument, PowerPoint: their text, scrolling.
    if crate::office_text::applies(&path)
        && let Some(text) = crate::office_text::cached(&path)
    {
        return div()
            .id("quick-look-document")
            .debug_selector(|| "quick-look-document".into())
            .size_full()
            .p_5()
            .rounded_md()
            .bg(palette.surface)
            .overflow_y_scrollbar()
            .child(div().text_sm().text_color(palette.text).whitespace_normal().child(text))
            .into_any_element();
    }
    // PDFs: every page, one under the other, scrolling.
    if crate::pdf_pages::is_pdf(&path) {
        return pdf(&path, palette);
    }
    match crate::thumbnails::quick_look_preview(&path) {
        crate::thumbnails::Preview::Ready(rendered) => image(rendered, palette),
        crate::thumbnails::Preview::Pending => placeholder("Generating preview…"),
        crate::thumbnails::Preview::Unavailable(reason) => placeholder(&format!("No preview · {reason}")),
    }
}

/// Fit ⇄ Fill for pictures; remembered in settings.
pub(crate) fn fit_button(id: &'static str, fit: crate::settings::ImageFit, appearance: Entity<AppearanceSettings>) -> Button {
    let (icon, label, tooltip) = match fit {
        crate::settings::ImageFit::Fit => (IconName::Shrink, "Fit", "The whole picture · click to fill the frame"),
        crate::settings::ImageFit::Fill => (IconName::Expand, "Fill", "The frame filled · click to see the whole picture"),
    };
    Button::new(id).ghost().xsmall().icon(icon).label(label).tooltip(tooltip).on_click(move |_, _, cx| {
        appearance.update(cx, |settings, cx| {
            settings.image_fit = settings.image_fit.toggled();
            let _ = crate::settings::save(settings);
            cx.notify();
        });
    })
}

/// What an extension made for a file, with its name.
fn extension_preview(name: &str, content: files_sdk::preview::PreviewContent, palette: Palette) -> AnyElement {
    use files_sdk::preview::PreviewContent;
    let body: AnyElement = match content {
        PreviewContent::Text { text, monospace } => div()
            .text_sm()
            .when(monospace, |this| this.text_xs().font_family(crate::palette::terminal_font()))
            .whitespace_normal()
            .child(text)
            .into_any_element(),
        PreviewContent::Image(source) => return image(source, palette),
        PreviewContent::Pages(pages) => v_flex()
            .gap_3()
            .children(pages.into_iter().map(|page| img(page).w_full().object_fit(ObjectFit::Contain).into_any_element()))
            .into_any_element(),
        PreviewContent::Details(rows) => v_flex()
            .gap_1()
            .children(rows.into_iter().map(|(label, value)| {
                h_flex().gap_3().text_sm().child(div().w(px(160.)).text_color(palette.muted).child(label)).child(div().flex_1().child(value))
            }))
            .into_any_element(),
    };
    v_flex()
        .id("quick-look-extension")
        .size_full()
        .p_4()
        .gap_2()
        .rounded_md()
        .bg(palette.surface)
        .overflow_y_scrollbar()
        .child(div().text_xs().text_color(palette.faint).child(format!("Preview by {name}")))
        .child(body)
        .into_any_element()
}

/// A PDF's pages as they are rendered, scrolling like a document.
fn pdf(path: &Path, palette: Palette) -> AnyElement {
    let pages = crate::pdf_pages::pages(path);
    let note = |text: String| div().p_4().text_sm().text_color(palette.muted).child(text);
    if let Some(error) = pages.error {
        return note(format!("No preview · {error}")).into_any_element();
    }
    let shown = pages.rendered.len();
    v_flex()
        .id("quick-look-pdf")
        .debug_selector(|| "quick-look-pdf".into())
        .size_full()
        .gap_3()
        .p_3()
        .rounded_md()
        .bg(palette.surface)
        .overflow_y_scrollbar()
        .when(pages.count == 0, |this| this.child(note("Rendering pages…".into())))
        .children(pages.rendered.into_iter().enumerate().map(|(index, page)| match page {
            Some(page) => div()
                .w_full()
                .flex_shrink_0()
                .child(img(page).w_full().object_fit(ObjectFit::Contain).with_fallback(|| div().into_any_element()))
                .into_any_element(),
            // Letter-shaped until it arrives, so the scroll height is stable.
            None => div()
                .w_full()
                .h(px(1000.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(palette.elevated)
                .text_xs()
                .text_color(palette.faint)
                .child(format!("Page {}", index + 1))
                .into_any_element(),
        }))
        .when(pages.count > shown && shown > 0, |this| this.child(note(format!("Showing the first {shown} of {} pages · Open to read on", pages.count))))
        .into_any_element()
}

/// Files whose preview has been read (kept in the caches below).
static WARM: std::sync::Mutex<Option<std::collections::HashSet<std::path::PathBuf>>> = std::sync::Mutex::new(None);
static WARMED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Bumped as previews finish reading: Quick Look redraws.
pub(crate) fn generation() -> u64 {
    WARMED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether `path`'s preview is ready to draw without waiting; if not, it is
/// read on a background thread (once) and Quick Look redraws after.
fn warmed(path: &Path, kind: FileKind) -> bool {
    if WARM_DONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref().is_some_and(|done| done.contains(path)) {
        return true;
    }
    let mut warm = WARM.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // Reading began already: still the placeholder.
    if !warm.get_or_insert_with(Default::default).insert(path.to_path_buf()) {
        return false;
    }
    drop(warm);
    let owned = path.to_path_buf();
    let job = crate::tasks::queue("quick look read", owned.display().to_string());
    std::thread::spawn(move || {
        crate::tasks::start(job);
        if owned.is_dir() {
            let _ = crate::model::entries(crate::model::Location::from_path(&owned), "");
        } else if crate::office_text::applies(&owned) {
            let _ = crate::office_text::cached(&owned);
        } else if owned.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("eml")) {
            let _ = email(&owned);
        } else if files_ext_archive::ArchiveKind::of(&owned).is_some() {
            let _ = archive_items(&owned);
        } else {
            let _ = cached_text(&owned, kind);
        }
        crate::tasks::finish(job, Ok(()));
        // Not ready before the thread is done: mark it as read now.
        WARM_DONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get_or_insert_with(Default::default).insert(owned);
        WARMED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
    false
}

static WARM_DONE: std::sync::Mutex<Option<std::collections::HashSet<std::path::PathBuf>>> = std::sync::Mutex::new(None);

/// Tests: waits until every preview being read is ready, then redraws.
#[cfg(test)]
pub(crate) fn settle(cx: &mut gpui_kit::VisualTestContext) {
    for _ in 0..200 {
        let started = WARM.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone().unwrap_or_default();
        let done = WARM_DONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone().unwrap_or_default();
        if started.is_subset(&done) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if let Some(view) = last_for_test() {
        view.update(cx, |_, cx| cx.notify());
    }
    cx.run_until_parked();
}

/// The start of a text file, kept for the last one shown.
fn cached_text(path: &Path, kind: FileKind) -> Option<String> {
    type Entry = (std::path::PathBuf, Option<std::time::SystemTime>, Option<String>);
    static LAST: std::sync::Mutex<Option<Entry>> = std::sync::Mutex::new(None);
    let modified = std::fs::metadata(path).ok()?.modified().ok();
    let mut last = LAST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((cached, when, text)) = &*last
        && cached == path
        && *when == modified
    {
        return text.clone();
    }
    let text = read_text(path, kind);
    *last = Some((path.to_path_buf(), modified, text.clone()));
    text
}

/// A saved message (.eml), parsed once per file version.
fn email(path: &Path) -> Option<files_ext_mail::Message> {
    type Parsed = (Option<std::time::SystemTime>, Option<files_ext_mail::Message>);
    static CACHE: std::sync::Mutex<Option<(std::path::PathBuf, Parsed)>> = std::sync::Mutex::new(None);
    let modified = std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok();
    let mut cache = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((cached, (time, message))) = cache.as_ref()
        && cached == path
        && *time == modified
    {
        return message.clone();
    }
    let message = std::fs::read(path).ok().and_then(|raw| files_ext_mail::parse_message(&raw));
    *cache = Some((path.to_path_buf(), (modified, message.clone())));
    message
}

/// Items shown in an archive preview; the rest are counted.
const ARCHIVE_ITEMS: usize = 300;

/// An archive's listing, read once per file version (the view redraws often).
fn archive_items(path: &Path) -> Result<Vec<files_ext_archive::extract::Item>, String> {
    type Listing = (Option<std::time::SystemTime>, Result<Vec<files_ext_archive::extract::Item>, String>);
    static CACHE: std::sync::Mutex<Option<(std::path::PathBuf, Listing)>> = std::sync::Mutex::new(None);
    let modified = std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok();
    let mut cache = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((cached, (time, listing))) = cache.as_ref()
        && cached == path
        && *time == modified
    {
        return listing.clone();
    }
    let listing = files_ext_archive::extract::list_items(path)
        .map_err(|error| files_ext_archive::extract::describe_error(path, &error));
    *cache = Some((path.to_path_buf(), (modified, listing.clone())));
    listing
}

/// What an archive holds: a summary and its members, folders first.
fn archive(path: &Path, palette: Palette, settings: &crate::settings::AppSettings) -> AnyElement {
    let mut items = match archive_items(path) {
        Ok(items) => items,
        Err(reason) => {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(div().text_sm().text_color(palette.muted).child(format!("No preview · {reason}")))
                .into_any_element();
        }
    };
    items.sort_by(|a, b| (!a.dir, a.name.to_lowercase()).cmp(&(!b.dir, b.name.to_lowercase())));
    let files = items.iter().filter(|item| !item.dir).count();
    let total: u64 = items.iter().map(|item| item.size).sum();
    let summary = format!(
        "{files} {} · {} unpacked",
        if files == 1 { "file" } else { "files" },
        crate::settings::format_size(total, settings.human_sizes)
    );
    let more = items.len().saturating_sub(ARCHIVE_ITEMS);
    let rows = items.into_iter().take(ARCHIVE_ITEMS).map(|item| {
        h_flex()
            .gap_2()
            .py_0p5()
            .child(Icon::new(if item.dir { IconName::Folder } else { IconName::File }).small().text_color(if item.dir { palette.folder } else { palette.muted }))
            .child(div().flex_1().min_w_0().truncate().text_xs().text_color(palette.text).child(item.name))
            .child(div().w(px(80.)).text_right().text_xs().text_color(palette.faint).child(if item.dir {
                String::new()
            } else {
                crate::settings::format_size(item.size, settings.human_sizes)
            }))
    });
    v_flex()
        .size_full()
        .gap_3()
        .child(div().debug_selector(|| "quick-look-archive".into()).text_sm().text_color(palette.muted).child(summary))
        .child(
            div()
                .id("quick-look-archive-list")
                .flex_1()
                .min_h_0()
                .p_3()
                .rounded_md()
                .bg(palette.surface)
                .overflow_y_scrollbar()
                .child(v_flex().children(rows).when(more > 0, |list| {
                    list.child(div().pt_2().text_xs().text_color(palette.faint).child(format!("+{more} more")))
                })),
        )
        .into_any_element()
}

/// What a folder holds: a summary and a grid of its items with thumbnails.
fn folder(file: crate::model::Entry, palette: Palette, settings: &crate::settings::AppSettings) -> Option<AnyElement> {
    let target = file.target?;
    let mut items = crate::model::entries(target, "");
    items.sort_by_key(|entry| (!entry.is_folder(), entry.name.to_lowercase()));
    let folders = items.iter().filter(|entry| entry.is_folder()).count();
    let files = items.len() - folders;
    let size = crate::model::folder_size(&file)
        .map_or_else(|| "calculating size…".to_string(), |bytes| crate::settings::format_size(bytes, settings.human_sizes));
    let plural = |count: usize, one: &str, many: &str| format!("{count} {}", if count == 1 { one } else { many });
    let summary = format!("{} · {} · {size}", plural(folders, "folder", "folders"), plural(files, "file", "files"));

    if items.is_empty() {
        return Some(
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .child(crate::views::files::file_icon(&file, palette, 72.))
                .child(div().text_sm().text_color(palette.muted).child("This folder is empty"))
                .into_any_element(),
        );
    }
    let more = items.len().saturating_sub(FOLDER_ITEMS);
    let tiles = items.into_iter().take(FOLDER_ITEMS).enumerate().map(|(index, entry)| {
        let tile = v_flex()
            .id(("quick-look-item", index))
            .w(px(104.))
            .gap_1()
            .items_center()
            .child(div().w(px(84.)).h(px(70.)).rounded_md().overflow_hidden().child(crate::views::files::file_thumbnail(&entry, palette, 70.)))
            .child(
                div()
                    .w_full()
                    .text_center()
                    .text_xs()
                    .text_color(palette.text)
                    .truncate()
                    .child(crate::model::trim_middle(&entry.name, 18).into_owned()),
            );
        #[cfg(test)]
        let tile = tile.debug_selector(move || format!("quick-look-item-{index}"));
        tile
    });
    Some(
        v_flex()
            .size_full()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(crate::views::files::file_icon(&file, palette, 28.))
                    .child(div().text_sm().text_color(palette.muted).child(summary)),
            )
            .child(
                div()
                    .id("quick-look-folder")
                    .flex_1()
                    .min_h_0()
                    .p_3()
                    .rounded_md()
                    .bg(palette.surface)
                    .overflow_y_scrollbar()
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_3()
                            .children(tiles)
                            .when(more > 0, |grid| {
                                grid.child(
                                    div()
                                        .w(px(104.))
                                        .h(px(90.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_xs()
                                        .text_color(palette.faint)
                                        .child(format!("+{more} more")),
                                )
                            }),
                    ),
            )
            .into_any_element(),
    )
}

fn image(source: std::path::PathBuf, palette: Palette) -> AnyElement {
    image_with(source, palette, crate::settings::ImageFit::Fit)
}

/// A picture in the frame, whole or filling it. Placed over the frame (not
/// in its flow), so a big picture can never push past and get cut.
fn image_with(source: std::path::PathBuf, palette: Palette, fit: crate::settings::ImageFit) -> AnyElement {
    div()
        .relative()
        .size_full()
        .rounded_md()
        .overflow_hidden()
        .bg(palette.surface)
        .child(
            img(source)
                .absolute()
                .inset_0()
                .size_full()
                .object_fit(fit.object_fit())
                .with_fallback(|| div().into_any_element()),
        )
        .into_any_element()
}

/// The start of a text-like file, or `None` for binary content.
pub(crate) fn read_text(path: &Path, kind: FileKind) -> Option<String> {
    use std::io::Read as _;
    let textual = matches!(kind, FileKind::Text | FileKind::Code)
        || path.extension().is_some_and(|extension| {
            matches!(extension.to_string_lossy().to_lowercase().as_str(), "csv" | "log" | "md")
        });
    if !textual || crate::model::is_online_only(path) {
        return None;
    }
    let mut buffer = Vec::with_capacity(TEXT_LIMIT);
    std::fs::File::open(path).ok()?.take(TEXT_LIMIT as u64 + 1).read_to_end(&mut buffer).ok()?;
    let truncated = buffer.len() > TEXT_LIMIT;
    buffer.truncate(TEXT_LIMIT);
    if buffer.contains(&0) {
        return None;
    }
    let mut text = String::from_utf8_lossy(&buffer).into_owned();
    if truncated {
        text.push_str("\n\n… (preview shows the first 64 KB)");
    }
    Some(text)
}

impl Render for QuickLookView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&settings);
        let file = self.current(cx);
        // Videos play in place (through VLC); another item stops the player.
        let video_path = file
            .clone()
            .filter(|file| file.kind == FileKind::Video || (file.kind == FileKind::Audio && file.path.as_deref().is_some_and(files_ext_music::local::is_audio)))
            .and_then(|file| file.path.as_deref().map(Path::to_path_buf));
        let current = self.video.as_ref().map(|player| player.read(cx).path.clone());
        if video_path != current {
            if let Some(old) = self.video.take() {
                old.update(cx, |player, cx| player.release(window, cx));
            }
            self.video = video_path.map(|path| cx.new(|cx| crate::views::video_player::VideoPlayer::new(path, palette, window, cx)));
        }
        let content = match file {
            Some(ref file) => {
                let size = crate::properties::demo_human_size(&file, settings.human_sizes);
                let modified = crate::settings::format_demo_date(&file.modified, settings.date_time);
                v_flex()
                    .size_full()
                    .gap_3()
                    .child(v_flex().flex_1().min_h_0().child(match &self.video {
                        Some(player) => player.clone().into_any_element(),
                        None => body(file.clone(), palette, &settings),
                    }))
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap_3()
                            .items_center()
                            .child(crate::views::files::file_icon(&file, palette, 18.))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .truncate()
                                            .child(file.name.clone()),
                                    )
                                    .child(div().text_xs().text_color(palette.muted).child(format!(
                                        "{} · {size} · {modified}",
                                        file.kind.label()
                                    ))),
                            )
                            .child(div().text_xs().text_color(palette.faint).child("↑↓ browse · Space close"))
                            .when(file.kind == FileKind::Image, |this| this.child(fit_button("quick-look-fit", settings.image_fit, self.appearance.clone())))
                            .child(
                                Button::new("quick-look-open")
                                    .primary()
                                    .small()
                                    .icon(IconName::SquareArrowOutUpRight)
                                    .label(if file.is_folder() { "Open folder" } else { "Open" })
                                    .on_click(cx.listener(|this, _, window, cx| this.open(&QuickLookOpen, window, cx))),
                            ),
                    )
                    .into_any_element()
            }
            None => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(Icon::new(IconName::Eye).large().text_color(palette.faint))
                .child(div().text_sm().text_color(palette.muted).child("Select a file to preview"))
                .into_any_element(),
        };
        div()
            .track_focus(&self.focus_handle)
            .key_context("QuickLook")
            .on_action(cx.listener(Self::close))
            .on_action(cx.listener(Self::previous))
            .on_action(cx.listener(Self::next))
            .on_action(cx.listener(Self::open))
            .w_full()
            .h(px(560.))
            .text_color(palette.text)
            // The shown file's pictures in their own cache, freed on moving on:
            // GPUI's global one keeps every image it ever decoded.
            .child(
                gpui_kit::image_cache(gpui_kit::retain_all(gpui_kit::ElementId::Name(
                    format!("quick-look-{:?}", file.as_ref().and_then(|file| file.path.clone())).into(),
                )))
                .size_full()
                .child(content),
            )
    }
}

impl FileExplorer {
    /// The selection in the active pane, in any view mode.
    pub(crate) fn selected_entry_any(&self) -> Option<crate::model::Entry> {
        self.active_selection().and_then(|id| crate::model::entry(&id))
    }

    /// Moves the selection by `delta` in the active list (all view modes).
    pub(crate) fn step_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let location = self.active_location();
        let files = self.entries_for(location);
        if files.is_empty() {
            return;
        }
        let current = self
            .active_selection()
            .and_then(|selected| files.iter().position(|file| file.id == selected));
        let next = match current {
            Some(index) => (index as isize + delta).clamp(0, files.len() as isize - 1) as usize,
            None => 0,
        };
        let id = files[next].id.clone();
        if self.mode == crate::model::ViewMode::Dual {
            self.select_in_pane(self.active_pane, &id, cx);
        } else {
            self.select_entry(&id, cx);
        }
    }

    /// Space: open the preview of the selection, or close it if it is open.
    pub fn toggle_quick_look(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        if self.selected_entry_any().is_none() {
            self.step_selection(0, cx);
        }
        let explorer = cx.entity().downgrade();
        let appearance = self.appearance.clone();
        let return_focus = window.focused(cx);
        let view = cx.new(|cx| QuickLookView {
            explorer,
            _repaint: cx.observe(&appearance, |_, _, cx| cx.notify()),
            appearance,
            focus_handle: cx.focus_handle(),
            return_focus: return_focus.clone(),
            video: None,
        });
        #[cfg(test)]
        LAST.with(|last| *last.borrow_mut() = Some(view.clone()));
        // Conversions and folder sizes finish in the background; repaint then.
        view.update(cx, |_, cx| crate::properties::watch_folder_sizes(cx));
        let initial_focus = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                let focus = view.read(cx).focus_handle.clone();
                focus.focus(window, cx);
            }
            dialog
                .w(px(880.))
                .centered()
                .close_button(false)
                .overlay(true)
                .overlay_closable(true)
                // Enter opens the item instead of dismissing the panel.
                .on_ok({
                    let view = view.clone();
                    move |_, window, cx| {
                        view.update(cx, |view, cx| view.open(&QuickLookOpen, window, cx));
                        true
                    }
                })
                .on_close({
                    let return_focus = return_focus.clone();
                    move |_, window, cx| {
                        if let Some(focus) = &return_focus {
                            focus.focus(window, cx);
                        }
                    }
                })
                .child(view.clone())
        });
    }
}

#[cfg(test)]
thread_local! {
    static LAST: std::cell::RefCell<Option<Entity<QuickLookView>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn last_for_test() -> Option<Entity<QuickLookView>> {
    LAST.with(|last| last.borrow().clone())
}

#[cfg(test)]
impl QuickLookView {
    pub(crate) fn video_for_test(&self, cx: &gpui_kit::App) -> Option<std::path::PathBuf> {
        self.video.as_ref().map(|player| player.read(cx).path.clone())
    }
}
