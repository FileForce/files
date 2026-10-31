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
use crate::model::{FileEntry, FileKind};
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
}

impl Focusable for QuickLookView {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl QuickLookView {
    fn current(&self, cx: &gpui_kit::App) -> Option<&'static FileEntry> {
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
        let _ = self.explorer.update(cx, |explorer, cx| explorer.open_entry(file.id, window, cx));
    }
}

/// What the big preview area shows for a file.
fn body(file: &'static FileEntry, palette: Palette, settings: &crate::settings::AppSettings) -> AnyElement {
    let placeholder = |message: &str| {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(crate::views::files::file_icon(file, palette, 72.))
            .child(div().text_sm().text_color(palette.muted).child(message.to_string()))
            .into_any_element()
    };
    if file.is_folder() {
        return folder(file, palette, settings).unwrap_or_else(|| placeholder(file.metadata));
    }
    let Some(path) = file.path else {
        return placeholder("No preview · this item is not on the local disk");
    };
    if file.kind == FileKind::Image
        && let Some(source) = crate::thumbnails::source(path)
    {
        return image(source, palette);
    }
    if crate::archive::is_supported(file) {
        return archive(path, palette, settings);
    }
    if let Some(text) = read_text(path, file.kind) {
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
    match crate::thumbnails::quick_look_preview(path) {
        crate::thumbnails::Preview::Ready(rendered) => image(rendered, palette),
        crate::thumbnails::Preview::Pending => placeholder("Generating preview…"),
        crate::thumbnails::Preview::Unavailable(reason) => placeholder(&format!("No preview · {reason}")),
    }
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
fn folder(file: &'static FileEntry, palette: Palette, settings: &crate::settings::AppSettings) -> Option<AnyElement> {
    let target = file.target?;
    let mut items = crate::model::entries(target, "");
    items.sort_by_key(|entry| (!entry.is_folder(), entry.name.to_lowercase()));
    let folders = items.iter().filter(|entry| entry.is_folder()).count();
    let files = items.len() - folders;
    let size = crate::model::folder_size(file)
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
                .child(crate::views::files::file_icon(file, palette, 72.))
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
            .child(div().w(px(84.)).h(px(70.)).rounded_md().overflow_hidden().child(crate::views::files::file_thumbnail(entry, palette, 70.)))
            .child(
                div()
                    .w_full()
                    .text_center()
                    .text_xs()
                    .text_color(palette.text)
                    .truncate()
                    .child(crate::model::trim_middle(entry.name, 18).into_owned()),
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
                    .child(crate::views::files::file_icon(file, palette, 28.))
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
    div()
        .size_full()
        .rounded_md()
        .overflow_hidden()
        .bg(palette.surface)
        .child(
            img(source)
                .size_full()
                .object_fit(ObjectFit::Contain)
                .with_fallback(|| div().into_any_element()),
        )
        .into_any_element()
}

/// The start of a text-like file, or `None` for binary content.
fn read_text(path: &Path, kind: FileKind) -> Option<String> {
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&settings);
        let file = self.current(cx);
        let content = match file {
            Some(file) => {
                let size = crate::properties::demo_human_size(file, settings.human_sizes);
                let modified = crate::settings::format_demo_date(file.modified, settings.date_time);
                v_flex()
                    .size_full()
                    .gap_3()
                    .child(div().flex_1().min_h_0().child(body(file, palette, &settings)))
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap_3()
                            .items_center()
                            .child(crate::views::files::file_icon(file, palette, 18.))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .truncate()
                                            .child(file.name),
                                    )
                                    .child(div().text_xs().text_color(palette.muted).child(format!(
                                        "{} · {size} · {modified}",
                                        file.kind.label()
                                    ))),
                            )
                            .child(div().text_xs().text_color(palette.faint).child("↑↓ browse · Space close"))
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
            .child(content)
    }
}

impl FileExplorer {
    /// The selection in the active pane, in any view mode.
    pub(crate) fn selected_entry_any(&self) -> Option<&'static FileEntry> {
        self.active_selection().and_then(crate::model::entry)
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
        let id = files[next].id;
        if self.mode == crate::model::ViewMode::Dual {
            self.select_in_pane(self.active_pane, id, cx);
        } else {
            self.select_entry(id, cx);
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
        });
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
