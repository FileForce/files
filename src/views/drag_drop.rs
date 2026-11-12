//! Drag-and-drop between explorer windows, with Finder's rules: on the same
//! volume items move, across volumes they are copied, and ⌥ always copies.

use std::path::PathBuf;

use gpui_kit::base::h_flex;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::{
    AnyElement, App, Context, DragMoveEvent, Entity, EntityId, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Window, div, px,
};

use crate::app::FileExplorer;
use crate::palette::Palette;
use crate::transfer::Operation;
use crate::views::files::file_icon;

#[derive(Clone)]
pub struct DraggedFile {
    /// The item under the pointer when the drag started.
    pub entry: crate::model::Entry,
    pub palette: Palette,
    pub source_window: EntityId,
    pub source: Entity<FileExplorer>,
}

impl DraggedFile {
    /// The dragged item, or the whole selection when it is part of one.
    pub fn paths(&self, cx: &App) -> Vec<PathBuf> {
        self.paths_from(self.source.read(cx))
    }

    /// Same as [`Self::paths`], for when the source explorer is the one being
    /// updated (a drop inside the same window) and cannot be read through `cx`.
    pub fn paths_from(&self, explorer: &FileExplorer) -> Vec<PathBuf> {
        let selection = explorer.selected_entries();
        let items: Vec<crate::model::Entry> = if selection.iter().any(|entry| entry.id == self.entry.id) {
            selection
        } else {
            vec![self.entry.clone()]
        };
        items.into_iter().filter_map(|entry| entry.path.as_deref().map(|path| path.to_path_buf())).collect()
    }
}

/// What the platform gets when a drag leaves the window: the real files, so
/// another window of this app (or Finder) can take them.
pub fn native_payload(drag: &DraggedFile, _: &mut Window, cx: &mut App) -> Option<gpui_kit::ExternalDragPayload> {
    let entries: Vec<(PathBuf, bool)> = drag
        .paths(cx)
        .into_iter()
        .map(|path| {
            let folder = path.is_dir();
            (path, folder)
        })
        .collect();
    (!entries.is_empty()).then(|| gpui_kit::ExternalDragPayload::Files(gpui_kit::FileDragPaths::new(entries)))
}

pub fn dragged_file(entry: crate::model::Entry, palette: Palette, cx: &mut Context<FileExplorer>) -> DraggedFile {
    DraggedFile {
        entry,
        palette,
        source_window: cx.entity_id(),
        source: cx.entity(),
    }
}

pub struct DragFilePreview {
    pub entry: crate::model::Entry,
    pub palette: Palette,
    pub count: usize,
}

impl DragFilePreview {
    pub fn new(drag: &DraggedFile, cx: &App) -> Self {
        Self {
            entry: drag.entry.clone(),
            palette: drag.palette,
            count: drag.paths(cx).len().max(1),
        }
    }
}

impl Render for DragFilePreview {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let name = if self.count > 1 {
            format!("{} items", self.count)
        } else {
            self.entry.name.to_string()
        };
        h_flex()
            .w(px(220.))
            .h(px(54.))
            .p_3()
            .gap_3()
            .items_center()
            .rounded_lg()
            .border_1()
            .border_color(self.palette.accent)
            .bg(self.palette.elevated)
            .text_color(self.palette.text)
            .shadow_lg()
            .child(file_icon(&self.entry, self.palette, 20.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(div().text_sm().truncate().child(name))
                    .child(
                        div()
                            .text_xs()
                            .text_color(self.palette.accent)
                            .child(if window.modifiers().alt { "Copy (⌥)" } else { "Move or copy" }),
                    ),
            )
    }
}

impl FileExplorer {
    pub(crate) fn render_drop_area(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        // Pictures decoded for this folder live in its own cache, freed
        // when another folder opens (the global one never lets go).
        let folder_key = format!("images-{}", self.active_location().label());
        let content = gpui_kit::image_cache(gpui_kit::retain_all(gpui_kit::ElementId::Name(folder_key.into())))
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(self.render_mode_content(palette, cx));
        let window_id = cx.entity_id();
        let location = self.active_location();
        let accepts = location.path().is_some();

        div()
            .id("file-drop-area")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .can_drop(move |value, _, _| {
                accepts
                    && (value.downcast_ref::<DraggedFile>().is_some_and(|drag| drag.source_window != window_id)
                        // A native drag: another window of this app, or Finder.
                        || value.downcast_ref::<gpui_kit::ExternalPaths>().is_some())
            })
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<DraggedFile>, _, cx| {
                let drag = event.drag(cx).clone();
                let target_id = cx.entity_id();
                let active = event.bounds.contains(&event.event.position) && drag.source_window != target_id;
                if active || this.drop_active {
                    let source = drag.source.clone();
                    let target = cx.entity();
                    let entry = drag.entry;
                    source.update(cx, |source, _| {
                        if active {
                            source.pending_drop = Some((target, entry));
                        } else if source
                            .pending_drop
                            .as_ref()
                            .is_some_and(|(pending, _)| pending.entity_id() == target_id)
                        {
                            source.pending_drop = None;
                        }
                    });
                }
                if this.drop_active != active {
                    this.drop_active = active;
                    cx.notify();
                }
            }))
            .on_drop(cx.listener(|this, drag: &DraggedFile, window, cx| {
                this.drop_active = false;
                if drag.source_window == cx.entity_id() {
                    this.pending_drop = None;
                    cx.notify();
                    return;
                }
                drag.source.update(cx, |source, _| source.pending_drop = None);
                if crate::model::remote::entry_path(&drag.entry).is_some() {
                    let files = drag.source.read(cx).remote_drag_entries(drag.entry.clone());
                    this.download_remote_into_active(files, cx);
                    return;
                }
                let paths = drag.paths(cx);
                let copy = window.modifiers().alt;
                this.receive_files(paths, copy, window, cx);
            }))
            .on_drop(cx.listener(|this, paths: &gpui_kit::ExternalPaths, window, cx| {
                this.drop_active = false;
                let copy = window.modifiers().alt;
                this.receive_files(paths.paths().to_vec(), copy, window, cx);
            }))
            .drag_over::<DraggedFile>(move |style, _, _, _| style.bg(palette.accent_soft))
            .drag_over::<gpui_kit::ExternalPaths>(move |style, _, _, _| style.bg(palette.accent_soft))
            .child(content)
            .context_menu(crate::views::context_menu::folder_context_menu(location, cx.entity()))
            .into_any_element()
    }

    /// Finishes a drop the target window could not see (pointer released
    /// outside the source window).
    pub fn complete_pending_drop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((target, entry)) = self.pending_drop.take() else {
            return;
        };
        let drag = DraggedFile {
            entry,
            palette: self.palette(),
            source_window: cx.entity_id(),
            source: cx.entity(),
        };
        let paths = drag.paths(cx);
        let copy = window.modifiers().alt;
        target.update(cx, |target, cx| {
            if target.drop_active {
                target.drop_active = false;
                target.receive_files(paths, copy, window, cx);
            }
        });
    }

    /// A drop into this window's folder, following Settings → Behavior:
    /// Copy, Move or Finder's rule, optionally asking every time. ⌥ copies.
    pub fn receive_files(&mut self, paths: Vec<PathBuf>, force_copy: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_location().is_remote() {
            let location = self.active_location();
            self.upload_remote(paths, location, cx);
            return;
        }
        let Some(target) = self.active_location().path() else {
            self.notice = Some(format!("Cannot drop into {}", self.active_location().label()));
            cx.notify();
            return;
        };
        if paths.is_empty() {
            return;
        }
        if force_copy {
            self.run_transfer(paths, target, Operation::Copy, window, cx);
            return;
        }
        let behavior = self.appearance.read(cx).behavior;
        let suggested = match behavior.drop_action {
            crate::settings::DropAction::Copy => Operation::Copy,
            crate::settings::DropAction::Move => Operation::Move,
            crate::settings::DropAction::Finder => {
                if paths.first().is_some_and(|first| crate::transfer::same_volume(first, &target)) {
                    Operation::Move
                } else {
                    Operation::Copy
                }
            }
        };
        if behavior.ask_on_drop {
            self.ask_drop_operation(paths, target, suggested, window, cx);
        } else {
            self.run_transfer(paths, target, suggested, window, cx);
        }
    }

    /// "Copy or move?" for one drop. Enter takes the suggested operation.
    fn ask_drop_operation(
        &mut self,
        paths: Vec<PathBuf>,
        target: PathBuf,
        suggested: Operation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let explorer = cx.entity().downgrade();
        let palette = self.palette();
        let what = match paths.as_slice() {
            [single] => format!("\"{}\"", single.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned())),
            all => format!("{} items", all.len()),
        };
        let stop_asking = std::rc::Rc::new(std::cell::Cell::new(false));
        window.open_dialog(cx, move |dialog, _, _| {
            let choose = {
                let explorer = explorer.clone();
                let paths = paths.clone();
                let target = target.clone();
                let stop_asking = stop_asking.clone();
                move |operation: Operation, window: &mut Window, cx: &mut App| {
                    let _ = explorer.update(cx, |explorer, cx| {
                        if stop_asking.get() {
                            explorer.stop_asking_on_drop(cx);
                        }
                        explorer.run_transfer(paths.clone(), target.clone(), operation, window, cx);
                    });
                    window.close_dialog(cx);
                }
            };
            let (copy, move_) = (choose.clone(), choose.clone());
            let on_ok = choose.clone();
            let stop = stop_asking.clone();
            dialog
                .title(format!("Copy or move {what}?"))
                .centered()
                .w(px(460.))
                .close_button(false)
                .overlay(true)
                .overlay_closable(true)
                .on_ok(move |_, window, cx| {
                    on_ok(suggested, window, cx);
                    false
                })
                .child(
                    gpui_kit::base::v_flex()
                        .gap_4()
                        .text_color(palette.text)
                        .child(div().text_sm().child(format!("Into {}", target.display())))
                        .child(
                            gpui_kit::component::checkbox::Checkbox::new("drop-stop-asking")
                                .label("Don't ask again (change it in Settings → General)")
                                .checked(stop.get())
                                .on_click(move |checked: &bool, window, _| {
                                    stop.set(*checked);
                                    window.refresh();
                                }),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .child(
                                    gpui_kit::component::button::Button::new("drop-cancel")
                                        .ghost()
                                        .small()
                                        .label("Cancel")
                                        .on_click(|_, window, cx| window.close_dialog(cx)),
                                )
                                .child({
                                    let button = gpui_kit::component::button::Button::new("drop-move")
                                        .small()
                                        .label("Move")
                                        .on_click(move |_, window, cx| move_(Operation::Move, window, cx));
                                    if suggested == Operation::Move { button.primary() } else { button.secondary() }
                                })
                                .child({
                                    let button = gpui_kit::component::button::Button::new("drop-copy")
                                        .small()
                                        .label("Copy")
                                        .on_click(move |_, window, cx| copy(Operation::Copy, window, cx));
                                    if suggested == Operation::Copy { button.primary() } else { button.secondary() }
                                }),
                        ),
                )
        });
    }

    /// "Don't ask again" in the drop dialog: saved like any other setting.
    fn stop_asking_on_drop(&mut self, cx: &mut Context<Self>) {
        self.appearance.update(cx, |settings, cx| {
            settings.behavior.ask_on_drop = false;
            let _ = crate::settings::save(settings);
            cx.notify();
        });
    }

    /// Copies or moves in the background, then selects the results.
    pub(crate) fn run_transfer(
        &mut self,
        paths: Vec<PathBuf>,
        target: PathBuf,
        operation: Operation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        let settings = self.appearance.read(cx).clone();
        let home = crate::model::Location::Home.path().unwrap_or_default();
        let verb = if operation == Operation::Move { "Moving" } else { "Copying" };
        self.notice = Some(format!("{verb} {} item{}…", paths.len(), if paths.len() == 1 { "" } else { "s" }));
        cx.notify();
        let destination = target.clone();
        cx.spawn_in(window, async move |this, cx| {
            let report = cx
                .background_executor()
                .spawn(async move {
                    crate::transfer::transfer(&paths, &destination, operation, &settings.deletion.protected, &home)
                })
                .await;
            let _ = this.update_in(cx, |this, _, cx| {
                let verb = if operation == Operation::Move { "Moved" } else { "Copied" };
                let mut notice = format!("{verb} {} item{}", report.done.len(), if report.done.len() == 1 { "" } else { "s" });
                if let Some((item, reason)) = report.skipped.first() {
                    notice.push_str(&format!(" · {} skipped ({item}: {reason})", report.skipped.len()));
                }
                files_sdk::log::record(
                    "files",
                    files_sdk::log::Kind::Fs,
                    notice.clone(),
                    Some(format!("into {}", target.display())),
                );
                // Big transfers finish while you do something else: tell when done.
                if report.done.len() >= 20 || !report.skipped.is_empty() {
                    let level = if report.skipped.is_empty() { files_sdk::log::Level::Success } else { files_sdk::log::Level::Warning };
                    files_sdk::log::notify("files", level, notice.clone(), Some(target.display().to_string()));
                }
                this.notice = Some(notice);
                // Select what arrived.
                let ids: Vec<gpui_kit::SharedString> = report
                    .done
                    .iter()
                    .filter_map(|path| crate::model::entry(&path.to_string_lossy()).map(|entry| entry.id.clone()))
                    .collect();
                if let Some(last) = ids.last().cloned() {
                    this.selection = ids.clone();
                    this.selected = Some(last.clone());
                    this.anchor = ids.first().cloned();
                    // In a long list the new item may be far below: scroll to it.
                    this.reveal = Some(last);
                }
                cx.notify();
            });
        })
        .detach();
    }
}
