use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Icon, InteractiveElementExt as _, Sizable as _};
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription,
    Window, WindowBounds, WindowOptions, actions, div, img, point, px, size,
};

use std::path::{Path, PathBuf};

pub(crate) use files_ext_archive::extract::{
    ArchiveKind, ConflictPolicy, describe_error, extract_archive, find_conflicts, list_items,
    parent_path,
};
pub(crate) use files_sdk::fs::unique_path;

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::component::input::{Input, InputState};

use crate::app::AppearanceSettings;
use crate::model::{FileEntry, FileKind};
use crate::palette::Palette;
use crate::settings::{AppSettings, HumanSizeSettings, format_demo_date, format_size};

actions!(archive, [CloseArchive]);

struct ArchiveEntry {
    path: &'static str,
    kind: FileKind,
    modified: &'static str,
    size: u64,
    preview: Option<&'static str>,
}

impl ArchiveEntry {
    fn is_folder(&self) -> bool {
        self.kind == FileKind::Folder
    }

    fn name(&self) -> &'static str {
        self.path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(self.path)
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum ArchivePage {
    Browse,
    Extract,
}

pub struct ArchiveWindow {
    archive: &'static FileEntry,
    entries: &'static [ArchiveEntry],
    page: ArchivePage,
    destination: Entity<InputState>,
    create_folder: bool,
    /// Pre-chosen answer to "file exists": overwrite without asking.
    overwrite_existing: bool,
    /// Set by the folder picker window; applied on next render (needs our window).
    pending_destination: Option<PathBuf>,
    loading: bool,
    extracting: bool,
    extract_error: Option<String>,
    /// Existing files found in the target, waiting for a conflict decision.
    conflicts: Option<(PathBuf, Vec<String>)>,
    current_folder: String,
    selected: Option<&'static str>,
    notice: Option<String>,
    appearance: Entity<AppearanceSettings>,
    focus_handle: FocusHandle,
    _appearance_subscription: Subscription,
}

impl ArchiveWindow {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        archive: &'static FileEntry,
        page: ArchivePage,
        appearance: Entity<AppearanceSettings>,
    ) -> Self {
        let appearance_subscription = cx.observe(&appearance, |_, _, cx| cx.notify());
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        // Reading can block for a long time (cloud placeholders, network
        // volumes), so the listing is loaded off the UI thread.
        let online_only = archive.path.is_some_and(crate::model::is_online_only);
        let loading = archive.path.is_some() && is_supported(archive) && !online_only;
        if let Some(path) = archive.path.filter(|_| loading) {
            cx.spawn(async move |this, cx| {
                let result = cx.background_executor().spawn(async move { read_archive(path) }).await;
                let _ = this.update(cx, |this, cx| {
                    this.loading = false;
                    match result {
                        Ok(entries) => this.entries = Box::leak(entries.into_boxed_slice()),
                        Err(error) => {
                            this.notice = Some(format!("Cannot read archive: {}", describe_error(path, &error)));
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        let notice = if archive.path.is_none() {
            Some("Archive is not on the local disk".to_string())
        } else if online_only {
            Some(
                "This file is online-only in cloud storage · download it first (Finder → Download Now), then reopen"
                    .to_string(),
            )
        } else {
            None
        };
        let default_destination = archive
            .path
            .and_then(Path::parent)
            .map(|parent| parent.display().to_string())
            .unwrap_or_default();
        let destination = cx.new(|cx| InputState::new(window, cx).default_value(default_destination));
        Self {
            archive,
            // Leaked once per opened archive window, like model entries.
            entries: &[],
            loading,
            page: if is_supported(archive) { page } else { ArchivePage::Browse },
            destination,
            create_folder: appearance.read(cx).extensions.archives_create_folder,
            overwrite_existing: appearance.read(cx).extensions.archives_overwrite,
            pending_destination: None,
            extracting: false,
            extract_error: None,
            conflicts: None,
            current_folder: String::new(),
            selected: None,
            notice,
            appearance,
            focus_handle,
            _appearance_subscription: appearance_subscription,
        }
    }

    fn on_close(&mut self, _: &CloseArchive, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    fn go_up(&mut self, cx: &mut Context<Self>) {
        if self.current_folder.is_empty() {
            return;
        }
        self.current_folder = parent_path(&self.current_folder).to_string();
        self.selected = None;
        cx.notify();
    }

    fn open_folder(&mut self, path: &'static str, cx: &mut Context<Self>) {
        self.current_folder = path.trim_end_matches('/').to_string();
        self.selected = None;
        self.notice = None;
        cx.notify();
    }

    /// Called by the folder picker window once the user chose a folder.
    pub fn set_destination(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.pending_destination = Some(path);
        cx.notify();
    }

    fn destination_base(&self, cx: &App) -> PathBuf {
        PathBuf::from(self.destination.read(cx).value().trim())
    }

    fn extract_target(&self, cx: &App) -> PathBuf {
        let base = self.destination_base(cx);
        if self.create_folder {
            let folder = base.join(archive_stem(self.archive));
            // A file already named like the new folder: create "name (1)" instead.
            if folder.is_file() { unique_path(&folder) } else { folder }
        } else {
            base
        }
    }

    fn browse_destination(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = self.destination_base(cx);
        let start = if base.is_dir() {
            base
        } else {
            self.archive
                .path
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .unwrap_or_default()
        };
        let _ = window;
        let target = cx.entity().downgrade();
        let picker = crate::app::FolderPicker {
            title: "Extract into",
            on_pick: std::rc::Rc::new(move |path, cx| {
                let _ = target.update(cx, |archive, cx| archive.set_destination(path, cx));
            }),
        };
        crate::app::open_folder_picker(start, picker, self.appearance.clone(), cx);
    }

    fn start_extract(&mut self, cx: &mut Context<Self>) {
        let Some(archive) = self.archive.path else {
            return;
        };
        let base = self.destination_base(cx);
        if base.as_os_str().is_empty() {
            self.extract_error = Some("Choose a destination folder".to_string());
            cx.notify();
            return;
        }
        if !base.is_dir() {
            self.extract_error = Some(format!("Folder does not exist: {}", base.display()));
            cx.notify();
            return;
        }
        let target = self.extract_target(cx);
        self.extracting = true;
        self.extract_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let job_target = target.clone();
            let result = cx
                .background_executor()
                .spawn(async move { find_conflicts(archive, &job_target) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.extracting = false;
                match result {
                    Ok(conflicts) if !conflicts.is_empty() && this.overwrite_existing => {
                        this.run_extract(ConflictPolicy::Overwrite, cx);
                    }
                    Ok(conflicts) if !conflicts.is_empty() => {
                        this.conflicts = Some((target, conflicts));
                    }
                    Ok(_) => this.run_extract(ConflictPolicy::Skip, cx),
                    Err(error) => {
                        this.extract_error =
                            Some(format!("Cannot read archive: {}", describe_error(archive, &error)));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn run_extract(&mut self, policy: ConflictPolicy, cx: &mut Context<Self>) {
        let Some(archive) = self.archive.path else {
            return;
        };
        let target = self.extract_target(cx);
        self.conflicts = None;
        self.extracting = true;
        self.extract_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let job_target = target.clone();
            let result = cx
                .background_executor()
                .spawn(async move { extract_archive(archive, &job_target, policy) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.extracting = false;
                match result {
                    Ok(report) => {
                        let mut notice = format!(
                            "Extracted {} files to {}",
                            report.files,
                            target.display()
                        );
                        for (count, label) in [
                            (report.renamed, "renamed"),
                            (report.overwritten, "overwritten"),
                            (report.skipped, "skipped"),
                        ] {
                            if count > 0 {
                                notice.push_str(&format!(" · {count} {label}"));
                            }
                        }
                        this.notice = Some(notice);
                        this.page = ArchivePage::Browse;
                    }
                    Err(error) => {
                        this.extract_error =
                            Some(format!("Extract failed: {}", describe_error(archive, &error)));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_extract_page(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(path) = self.pending_destination.take() {
            self.destination.update(cx, |state, cx| {
                state.set_value(path.display().to_string(), window, cx);
            });
        }
        let target = self.extract_target(cx);
        // A changed destination invalidates the previous conflict check.
        if self
            .conflicts
            .as_ref()
            .is_some_and(|(checked, _)| *checked != target)
        {
            self.conflicts = None;
        }
        let conflict_panel = self.conflicts.as_ref().map(|(_, names)| {
            const SHOWN: usize = 5;
            let more = names.len().saturating_sub(SHOWN);
            v_flex()
                .gap_2()
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(palette.archive)
                .bg(palette.surface)
                .text_xs()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(Icon::new(IconName::TriangleAlert).small().text_color(palette.archive))
                        .child(
                            div()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .child(format!("{} files already exist in this folder", names.len())),
                        ),
                )
                .children(
                    names
                        .iter()
                        .take(SHOWN)
                        .map(|name| div().text_color(palette.muted).child(name.clone())),
                )
                .when(more > 0, |this| {
                    this.child(div().text_color(palette.faint).child(format!("…and {more} more")))
                })
                .child(
                    h_flex()
                        .gap_2()
                        .pt_1()
                        .child(
                            Button::new("conflict-rename")
                                .primary()
                                .small()
                                .label("Keep both")
                                .tooltip("Extract as \"name (1).ext\"")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.run_extract(ConflictPolicy::Rename, cx)
                                })),
                        )
                        .child(
                            Button::new("conflict-overwrite")
                                .danger()
                                .small()
                                .label("Overwrite")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.run_extract(ConflictPolicy::Overwrite, cx)
                                })),
                        )
                        .child(
                            Button::new("conflict-skip")
                                .secondary()
                                .small()
                                .label("Skip existing")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.run_extract(ConflictPolicy::Skip, cx)
                                })),
                        ),
                )
        });
        let folder_name = archive_stem(self.archive);
        v_flex()
            .flex_1()
            .min_h_0()
            .p_6()
            .gap_5()
            .max_w(px(640.))
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(format!("Extract {}", self.archive.name)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .child(format!("{} items", self.entries.iter().filter(|entry| !entry.is_folder()).count())),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_xs().text_color(palette.muted).child("Destination folder"))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().flex_1().child(Input::new(&self.destination).w_full()))
                            .child(
                                Button::new("extract-browse")
                                    .secondary()
                                    .small()
                                    .icon(IconName::FolderOpen)
                                    .label("Browse…")
                                    .disabled(self.extracting)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.browse_destination(window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                Checkbox::new("extract-create-folder")
                    .label(format!("Create folder \"{folder_name}\""))
                    .checked(self.create_folder)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.create_folder = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("extract-overwrite")
                    .label("Overwrite existing files without asking")
                    .checked(self.overwrite_existing)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.overwrite_existing = *checked;
                        this.conflicts = None;
                        cx.notify();
                    })),
            )
            .child(
                v_flex()
                    .gap_1()
                    .p_3()
                    .rounded_md()
                    .bg(palette.surface)
                    .text_xs()
                    .child(div().text_color(palette.muted).child("Files will be written to"))
                    .child(div().child(target.display().to_string())),
            )
            .children(conflict_panel)
            .children(self.extract_error.clone().map(|error| {
                div().text_xs().text_color(palette.video).child(error)
            }))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("extract-confirm")
                            .primary()
                            .small()
                            .icon(IconName::PackageOpen)
                            .label(if self.extracting { "Extracting…" } else { "Extract" })
                            .disabled(self.extracting || self.conflicts.is_some())
                            .on_click(cx.listener(|this, _, _, cx| this.start_extract(cx))),
                    )
                    .child(
                        Button::new("extract-cancel")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .disabled(self.extracting)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.page = ArchivePage::Browse;
                                this.extract_error = None;
                                this.conflicts = None;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_preview(&self, palette: Palette, settings: AppSettings) -> AnyElement {
        let selected = self
            .selected
            .and_then(|path| self.entries.iter().find(|entry| entry.path == path));
        let mut content = v_flex()
            .h_full()
            .w(px(260.))
            .flex_shrink_0()
            .border_l_1()
            .border_color(palette.border)
            .bg(palette.sidebar)
            .child(
                div()
                    .h(px(36.))
                    .px_3()
                    .border_b_1()
                    .border_color(palette.border)
                    .text_sm()
                    .child("Preview"),
            );
        if let Some(entry) = selected {
            let preview = if let Some(image) = entry.preview {
                div()
                    .h(px(150.))
                    .rounded_md()
                    .overflow_hidden()
                    .child(img(image).w_full().h_full().object_fit(ObjectFit::Cover))
                    .into_any_element()
            } else {
                v_flex()
                    .h(px(150.))
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .rounded_md()
                    .bg(palette.surface)
                    .child(archive_icon(entry.kind, palette, 36.))
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .child(entry.kind.label()),
                    )
                    .into_any_element()
            };
            content = content.child(
                v_flex()
                    .p_4()
                    .gap_4()
                    .child(preview)
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child(entry.name()))
                            .child(div().text_xs().text_color(palette.muted).child(entry.path)),
                    )
                    .child(preview_row("Type", entry.kind.label(), palette))
                    .child(preview_row(
                        "Modified",
                        format_demo_date(entry.modified, settings.date_time),
                        palette,
                    ))
                    .child(preview_row(
                        "Size",
                        archive_entry_size(entry, settings.human_sizes),
                        palette,
                    )),
            );
        } else {
            content = content.child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .p_5()
                    .gap_2()
                    .child(archive_icon(FileKind::Archive, palette, 32.))
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .child("Select an archived file or folder"),
                    ),
            );
        }
        content.into_any_element()
    }
}

impl Render for ArchiveWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&settings);
        let extract_page = (self.page == ArchivePage::Extract)
            .then(|| self.render_extract_page(palette, window, cx));
        let parent = (!self.current_folder.is_empty()).then(|| {
            h_flex()
                .id("archive-parent")
                .h(px(34.))
                .px_3()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(palette.border)
                .text_color(palette.muted)
                .on_double_click(cx.listener(|this, _, _, cx| this.go_up(cx)))
                .child(archive_icon(FileKind::Folder, palette, 15.))
                .child(div().flex_1().text_xs().child(".."))
                .child(div().w(px(130.)))
                .child(div().w(px(80.)))
                .into_any_element()
        });
        let rows = self
            .entries
            .iter()
            .filter(|entry| parent_path(entry.path) == self.current_folder.as_str())
            .map(|entry| {
                let path = entry.path;
                let selected = self.selected == Some(path);
                h_flex()
                    .id(entry.path)
                    .h(px(34.))
                    .px_3()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(palette.border)
                    .bg(if selected {
                        palette.selection
                    } else {
                        palette.window
                    })
                    .hover(|style| style.bg(palette.hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = Some(path);
                        cx.notify();
                    }))
                    .on_double_click(cx.listener(move |this, _, _, cx| {
                        if entry.is_folder() {
                            this.open_folder(path, cx);
                        }
                    }))
                    .child(archive_icon(entry.kind, palette, 15.))
                    .child(div().flex_1().min_w_0().text_xs().child(entry.name()))
                    .child(
                        div()
                            .w(px(130.))
                            .text_size(px(10.))
                            .text_color(palette.muted)
                            .child(format_demo_date(entry.modified, settings.date_time)),
                    )
                    .child(
                        div()
                            .w(px(80.))
                            .text_right()
                            .text_size(px(10.))
                            .text_color(palette.muted)
                            .child(archive_entry_size(entry, settings.human_sizes)),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let breadcrumbs = if self.current_folder.is_empty() {
            self.archive.name.to_string()
        } else {
            format!(
                "{} / {}",
                self.archive.name,
                self.current_folder.replace('/', " / ")
            )
        };
        let status = self.notice.clone().unwrap_or_else(|| {
            if self.loading {
                "Reading archive…".to_string()
            } else if is_supported(self.archive) {
                format!("{} entries", self.entries.len())
            } else {
                "Supported archives: .zip, .tar, .tar.gz, .tgz, .gz".to_string()
            }
        });

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("Archive")
            .on_action(cx.listener(Self::on_close))
            .size_full()
            .min_h_0()
            .bg(palette.window)
            .text_color(palette.text)
            .child(
                h_flex()
                    .h(px(48.))
                    .flex_shrink_0()
                    .px_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(palette.border)
                    .bg(palette.sidebar)
                    .child(
                        Button::new("archive-up")
                            .ghost()
                            .small()
                            .icon(IconName::ArrowUp)
                            .disabled(self.current_folder.is_empty())
                            .tooltip("Parent archive folder")
                            .on_click(cx.listener(|this, _, _, cx| this.go_up(cx))),
                    )
                    .child(div().text_sm().child("Archive"))
                    .child(div().flex_1())
                    .child(
                        Button::new("extract-all")
                            .secondary()
                            .small()
                            .icon(IconName::PackageOpen)
                            .label("Extract all")
                            .disabled(
                                !is_supported(self.archive)
                                    || self.loading
                                    || self.entries.is_empty()
                                    || self.page == ArchivePage::Extract,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.page = ArchivePage::Extract;
                                this.extract_error = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("close-archive")
                            .ghost()
                            .small()
                            .label("Close")
                            .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                    ),
            )
            .child(
                h_flex()
                    .h(px(36.))
                    .flex_shrink_0()
                    .px_4()
                    .items_center()
                    .border_b_1()
                    .border_color(palette.border)
                    .text_xs()
                    .text_color(palette.muted)
                    .child(archive_icon(FileKind::Archive, palette, 15.))
                    .child(breadcrumbs),
            )
            .child(match extract_page {
                Some(page) => page,
                None => h_flex()
                    .flex_1()
                    .min_h_0()
                    // h_flex centers children; both columns must span the full height.
                    .items_start()
                    .child(
                        v_flex()
                            .h_full()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(
                                h_flex()
                                    .h(px(30.))
                                    .px_3()
                                    .gap_2()
                                    .items_center()
                                    .bg(palette.surface)
                                    .border_b_1()
                                    .border_color(palette.border)
                                    .text_size(px(9.))
                                    .text_color(palette.faint)
                                    .child(div().flex_1().child("NAME"))
                                    .child(div().w(px(130.)).child("MODIFIED"))
                                    .child(div().w(px(80.)).text_right().child("SIZE")),
                            )
                            .child(
                                div()
                                    .id("archive-scroll")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scrollbar()
                                    .child({
                                        let empty = rows.is_empty() && parent.is_none();
                                        let message = if self.loading {
                                            "Reading archive…".to_string()
                                        } else if !is_supported(self.archive) {
                                            "This archive format is not supported yet · supported: .zip, .tar, .tar.gz, .tgz, .gz".to_string()
                                        } else {
                                            self.notice.clone().unwrap_or_else(|| "Archive is empty".to_string())
                                        };
                                        v_flex()
                                            .children(parent)
                                            .children(rows)
                                            .when(empty, |this| {
                                                this.child(
                                                    div()
                                                        .p_6()
                                                        .text_xs()
                                                        .text_color(palette.muted)
                                                        .child(message),
                                                )
                                            })
                                    }),
                            ),
                    )
                    .child(self.render_preview(palette, settings))
                    .into_any_element(),
            })
            .child(
                div()
                    .h(px(28.))
                    .flex_shrink_0()
                    .px_3()
                    .border_t_1()
                    .border_color(palette.border)
                    .bg(palette.sidebar)
                    .text_size(px(10.))
                    .text_color(palette.muted)
                    .child(status),
            )
    }
}

pub fn open_archive(
    archive: &'static FileEntry,
    appearance: Entity<AppearanceSettings>,
    cx: &mut App,
) {
    open_archive_page(archive, ArchivePage::Browse, appearance, cx);
}

pub fn open_archive_page(
    archive: &'static FileEntry,
    page: ArchivePage,
    appearance: Entity<AppearanceSettings>,
    cx: &mut App,
) {
    if !appearance.read(cx).extensions.archives {
        return;
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(170.), px(100.)),
            size: size(px(1100.), px(720.)),
        })),
        window_min_size: Some(size(px(850.), px(560.))),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some(format!("{} — Archive", archive.name).into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, move |window, cx| {
        let appearance = appearance.clone();
        cx.new(|cx| ArchiveWindow::new(window, cx, archive, page, appearance))
    })
    .expect("Failed to open Archive window");
}


fn leak(value: String) -> &'static str {
    Box::leak(value.into_boxed_str())
}



pub(crate) fn is_supported(archive: &FileEntry) -> bool {
    archive.path.and_then(ArchiveKind::of).is_some()
}

/// The archive's name without its archive suffix (`a.tar.gz` → `a`).
fn archive_stem(archive: &FileEntry) -> String {
    let lower = archive.name.to_lowercase();
    ArchiveKind::SUFFIXES
        .iter()
        .find(|(suffix, _)| lower.ends_with(suffix) && lower.len() > suffix.len())
        .map_or_else(
            || archive.name.to_string(),
            |(suffix, _)| archive.name[..archive.name.len() - suffix.len()].to_string(),
        )
}














/// Lists an archive for the window, adding folder rows for parents that have
/// no explicit entry.
fn read_archive(path: &Path) -> std::io::Result<Vec<ArchiveEntry>> {
    let mut folders = std::collections::BTreeSet::new();
    let mut entries = Vec::new();
    for item in list_items(path)? {
        let mut parent = parent_path(&item.name).to_string();
        while !parent.is_empty() {
            folders.insert(parent.clone());
            parent = parent_path(&parent).to_string();
        }
        if item.dir {
            folders.insert(item.name.trim_end_matches('/').to_string());
            continue;
        }
        let extension = Path::new(&item.name)
            .extension()
            .map(|extension| extension.to_string_lossy().into_owned())
            .unwrap_or_default();
        entries.push(ArchiveEntry {
            kind: FileKind::from_extension(&extension),
            modified: leak(
                item.modified
                    .map(|time| time.format("%b %d, %Y, %H:%M").to_string())
                    .unwrap_or_else(|| "—".to_string()),
            ),
            size: item.size,
            path: leak(item.name),
            preview: None,
        });
    }
    let mut all: Vec<_> = folders
        .into_iter()
        .map(|folder| ArchiveEntry {
            path: leak(format!("{folder}/")),
            kind: FileKind::Folder,
            modified: "—",
            size: 0,
            preview: None,
        })
        .collect();
    all.append(&mut entries);
    Ok(all)
}













fn archive_entry_size(entry: &ArchiveEntry, settings: HumanSizeSettings) -> String {
    if entry.is_folder() {
        "—".to_string()
    } else {
        format_size(entry.size, settings)
    }
}

fn archive_icon(kind: FileKind, palette: Palette, size: f32) -> Icon {
    let (icon, color) = match kind {
        FileKind::Folder => (IconName::Folder, palette.folder),
        FileKind::Image => (IconName::FileImage, palette.media),
        FileKind::Video => (IconName::Film, palette.video),
        FileKind::Document => (IconName::FileText, palette.document),
        FileKind::Design => (IconName::FileType, palette.document),
        FileKind::Code => (IconName::FileCode, palette.code),
        FileKind::Archive => (IconName::FileArchive, palette.archive),
        FileKind::Audio => (IconName::FileMusic, palette.video),
        FileKind::Application => (IconName::AppWindow, palette.accent),
        FileKind::Text => (IconName::FileText, palette.muted),
    };
    Icon::new(icon).with_size(px(size)).text_color(color)
}

fn preview_row(
    label: &'static str,
    value: impl Into<SharedString>,
    palette: Palette,
) -> AnyElement {
    h_flex()
        .gap_3()
        .text_xs()
        .child(div().w(px(64.)).text_color(palette.muted).child(label))
        .child(div().flex_1().child(value.into()))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use files_ext_archive::extract::open_zip;
    use files_sdk::ExtractReport;
    use std::io::Write as _;

    fn fixture_zip(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("file-force-zip-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bundle.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("readme.md", options).unwrap();
        writer.write_all(b"hello").unwrap();
        writer.start_file("src/lib/main.rs", options).unwrap();
        writer.write_all(b"fn main() {}").unwrap();
        writer.start_file("../evil.txt", options).unwrap();
        writer.write_all(b"escape").unwrap();
        writer.finish().unwrap();
        path
    }

    #[test]
    fn read_zip_lists_files_and_implied_folders() {
        let path = fixture_zip("read");
        let entries = read_archive(&path).unwrap();
        let paths: Vec<_> = entries.iter().map(|entry| entry.path).collect();
        assert_eq!(paths, ["src/", "src/lib/", "readme.md", "src/lib/main.rs"]);
        assert_eq!(entries[3].kind, FileKind::Code);
        assert_eq!(entries[2].size, 5);
    }

    #[test]
    fn extract_writes_files_skips_existing_and_escapes() {
        let path = fixture_zip("extract");
        let target = path.parent().unwrap().join("out");
        assert!(find_conflicts(&path, &target).unwrap().is_empty());
        let report = extract_archive(&path, &target, ConflictPolicy::Skip).unwrap();
        assert_eq!(report, ExtractReport { files: 2, skipped: 1, ..Default::default() });
        assert_eq!(std::fs::read_to_string(target.join("src/lib/main.rs")).unwrap(), "fn main() {}");
        assert!(!path.parent().unwrap().join("evil.txt").exists(), "zip-slip entry escaped");

        std::fs::write(target.join("readme.md"), "mine").unwrap();
        assert_eq!(
            find_conflicts(&path, &target).unwrap(),
            ["readme.md", "src/lib/main.rs"]
        );
        let again = extract_archive(&path, &target, ConflictPolicy::Skip).unwrap();
        assert_eq!(again, ExtractReport { skipped: 3, ..Default::default() });
        assert_eq!(std::fs::read_to_string(target.join("readme.md")).unwrap(), "mine");
    }

    #[test]
    fn extract_rename_keeps_both_and_overwrite_replaces() {
        let path = fixture_zip("policies");
        let target = path.parent().unwrap().join("out");
        extract_archive(&path, &target, ConflictPolicy::Skip).unwrap();
        std::fs::write(target.join("readme.md"), "mine").unwrap();

        let renamed = extract_archive(&path, &target, ConflictPolicy::Rename).unwrap();
        assert_eq!(renamed.renamed, 2);
        assert_eq!(std::fs::read_to_string(target.join("readme.md")).unwrap(), "mine");
        assert_eq!(std::fs::read_to_string(target.join("readme (1).md")).unwrap(), "hello");
        assert!(target.join("src/lib/main (1).rs").exists());

        let again = extract_archive(&path, &target, ConflictPolicy::Rename).unwrap();
        assert_eq!(again.renamed, 2);
        assert!(target.join("readme (2).md").exists());

        let overwritten = extract_archive(&path, &target, ConflictPolicy::Overwrite).unwrap();
        assert_eq!(overwritten.overwritten, 2);
        assert_eq!(std::fs::read_to_string(target.join("readme.md")).unwrap(), "hello");
    }

    #[test]
    fn open_errors_keep_their_real_cause() {
        let missing = std::env::temp_dir().join("file-force-missing.zip");
        let error = open_zip(&missing).err().unwrap();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert_ne!(error.to_string(), "i/o error");

        let not_zip = std::env::temp_dir().join(format!("file-force-notzip-{}.zip", std::process::id()));
        std::fs::write(&not_zip, b"plain text").unwrap();
        let error = open_zip(&not_zip).err().unwrap();
        assert!(!error.to_string().contains("i/o error"), "{error}");

        let timed_out = std::io::Error::from(std::io::ErrorKind::TimedOut);
        assert!(describe_error(&not_zip, &timed_out).contains("online-only"));
    }

    #[test]
    fn file_in_place_of_folder_and_read_only_files_are_resolved() {
        let path = fixture_zip("blocked");
        let setup = |name: &str| {
            let target = path.parent().unwrap().join(name);
            let _ = std::fs::remove_dir_all(&target);
            std::fs::create_dir_all(&target).unwrap();
            // A file where the archive has the folder `src/`, and a read-only readme.
            std::fs::write(target.join("src"), "file").unwrap();
            std::fs::write(target.join("readme.md"), "mine").unwrap();
            let mut permissions = std::fs::metadata(target.join("readme.md")).unwrap().permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(target.join("readme.md"), permissions).unwrap();
            target
        };

        let target = setup("blocked-overwrite");
        assert_eq!(
            find_conflicts(&path, &target).unwrap(),
            ["readme.md", "src (file in place of a folder)"]
        );
        let report = extract_archive(&path, &target, ConflictPolicy::Overwrite).unwrap();
        assert_eq!(report.overwritten, 2);
        assert_eq!(std::fs::read_to_string(target.join("readme.md")).unwrap(), "hello");
        assert!(target.join("src/lib/main.rs").is_file());

        let target = setup("blocked-rename");
        extract_archive(&path, &target, ConflictPolicy::Rename).unwrap();
        assert_eq!(std::fs::read_to_string(target.join("src")).unwrap(), "file");
        assert!(target.join("src (1)/lib/main.rs").is_file());
        assert!(target.join("readme (1).md").is_file());

        let target = setup("blocked-skip");
        let report = extract_archive(&path, &target, ConflictPolicy::Skip).unwrap();
        assert_eq!(report.files, 0);
        assert_eq!(std::fs::read_to_string(target.join("src")).unwrap(), "file");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("file-force-arch-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn single_gz_file_lists_and_extracts_its_member() {
        let dir = scratch("gz");
        let path = dir.join("report.log.gz");
        let mut encoder = flate2::GzBuilder::new()
            .filename("report.log")
            .write(std::fs::File::create(&path).unwrap(), flate2::Compression::default());
        encoder.write_all(b"line one\nline two\n").unwrap();
        encoder.finish().unwrap();

        let entries = read_archive(&path).unwrap();
        assert_eq!(entries.iter().map(|entry| (entry.path, entry.size)).collect::<Vec<_>>(), [("report.log", 18)]);

        let out = dir.join("out");
        let report = extract_archive(&path, &out, ConflictPolicy::Skip).unwrap();
        assert_eq!(report.files, 1);
        assert_eq!(std::fs::read_to_string(out.join("report.log")).unwrap(), "line one\nline two\n");
        assert_eq!(find_conflicts(&path, &out).unwrap(), ["report.log"]);
        extract_archive(&path, &out, ConflictPolicy::Rename).unwrap();
        assert!(out.join("report (1).log").exists());
    }

    #[test]
    fn tar_gz_lists_extracts_and_blocks_escapes() {
        let dir = scratch("targz");
        let path = dir.join("bundle.tar.gz");
        let encoder = flate2::write::GzEncoder::new(std::fs::File::create(&path).unwrap(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, body) in [("docs/readme.md", "hi"), ("src/main.rs", "fn main() {}")] {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_mtime(1_700_000_000);
            header.set_cksum();
            builder.append_data(&mut header, name, body.as_bytes()).unwrap();
        }
        // A hostile member: the name field bypasses the builder's own checks.
        let mut header = tar::Header::new_gnu();
        header.as_gnu_mut().unwrap().name[..13].copy_from_slice(b"../escape.txt");
        header.set_size(1);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, &b"x"[..]).unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let paths: Vec<_> = read_archive(&path).unwrap().iter().map(|entry| entry.path).collect();
        assert_eq!(paths, ["docs/", "src/", "docs/readme.md", "src/main.rs"]);

        let out = dir.join("out");
        let report = extract_archive(&path, &out, ConflictPolicy::Skip).unwrap();
        assert_eq!(report.files, 2);
        assert_eq!(report.skipped, 1, "the ../ member is skipped");
        assert!(!dir.join("escape.txt").exists());
        assert_eq!(std::fs::read_to_string(out.join("src/main.rs")).unwrap(), "fn main() {}");
    }

    #[test]
    fn archive_kinds_and_stems() {
        assert_eq!(ArchiveKind::of(Path::new("a.TAR.GZ")), Some(ArchiveKind::TarGz));
        assert_eq!(ArchiveKind::of(Path::new("a.tgz")), Some(ArchiveKind::TarGz));
        assert_eq!(ArchiveKind::of(Path::new("dump.dmw.gz")), Some(ArchiveKind::Gz));
        assert_eq!(ArchiveKind::of(Path::new("a.7z")), None);
    }

    #[test]
    #[cfg(unix)]
    fn zip_extract_skips_symlinks_and_strips_setuid() {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("file-force-zip-safety-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("evil.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        writer.add_symlink("escape", "/etc", options).unwrap();
        writer.start_file("escape/passwd", options).unwrap();
        writer.write_all(b"x").unwrap();
        writer.start_file("../outside.txt", options).unwrap();
        writer.write_all(b"x").unwrap();
        writer.start_file("tool", options.unix_permissions(0o4755)).unwrap();
        writer.write_all(b"x").unwrap();
        writer.finish().unwrap();

        let target = dir.join("out");
        extract_archive(&path, &target, ConflictPolicy::Overwrite).unwrap();
        assert!(std::fs::symlink_metadata(target.join("escape")).map_or(true, |meta| !meta.file_type().is_symlink()));
        assert!(!dir.join("outside.txt").exists(), "path traversal must be blocked");
        let mode = std::fs::metadata(target.join("tool")).unwrap().permissions().mode();
        assert_eq!(mode & 0o7000, 0, "setuid/setgid bits must be dropped");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
