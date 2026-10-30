use std::cell::Cell;
use std::path::Path;

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement as _,
    IntoElement,
    ParentElement as _, Render, Styled as _, Subscription, WeakEntity, Window, div, px,
};

use crate::app::{AppearanceSettings, FileExplorer};
use crate::model::{FileEntry, Location};
use crate::palette::Palette;

enum DeleteState {
    Confirm,
    Working,
    Failed { message: String, retryable: bool },
    /// Protected items as (name, blacklist pattern); nothing is deleted.
    Protected(Vec<(&'static str, String)>),
}

pub struct DeleteDialog {
    explorer: WeakEntity<FileExplorer>,
    entries: Vec<&'static FileEntry>,
    /// How many of `entries` are already gone; a retry continues from here.
    done: usize,
    /// Items already in the trash can only be deleted permanently.
    in_trash: bool,
    permanent: bool,
    state: DeleteState,
    appearance: Entity<AppearanceSettings>,
    focus_handle: FocusHandle,
    _appearance_subscription: Subscription,
}

impl Focusable for DeleteDialog {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl DeleteDialog {
    /// Enter in the dialog. Returns whether the dialog should close now.
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.state {
            DeleteState::Protected(_) => true,
            DeleteState::Working => false,
            DeleteState::Confirm | DeleteState::Failed { .. } => {
                self.delete(window, cx);
                false
            }
        }
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let use_trash = !self.permanent && !self.in_trash;
        let pending: Vec<_> = self.entries[self.done..]
            .iter()
            .filter_map(|entry| entry.path)
            .collect();
        self.state = DeleteState::Working;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            // One batch call: a single Trash operation (and a single Finder sound).
            let (deleted, failure) = cx
                .background_executor()
                .spawn(async move {
                    match crate::delete::delete_many(&pending, use_trash) {
                        Ok(()) => (pending.len(), None),
                        Err((gone, error)) => (gone, Some(error)),
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                let removed: Vec<_> = this.entries[this.done..this.done + deleted].to_vec();
                this.done += deleted;
                if deleted > 0 {
                    crate::model::invalidate_listings();
                }
                let _ = this.explorer.update(cx, |explorer, cx| {
                    for entry in &removed {
                        explorer.forget_selection(entry.id);
                    }
                    cx.notify();
                });
                match failure {
                    None => {
                        let what = match this.entries.as_slice() {
                            [single] => single.name.to_string(),
                            all => format!("{} items", all.len()),
                        };
                        let message = if use_trash {
                            format!("Moved {what} to {}", Location::Trash.label())
                        } else {
                            format!("Deleted {what} permanently")
                        };
                        files_sdk::log::record("files", files_sdk::log::Kind::Fs, message.clone(), None);
                        let _ = this.explorer.update(cx, |explorer, cx| {
                            explorer.notice = Some(message);
                            cx.notify();
                        });
                        window.close_dialog(cx);
                    }
                    Some(error) => {
                        let failure = crate::delete::describe(&error);
                        let name = this.entries[this.done].name;
                        let progress = if this.entries.len() > 1 {
                            format!(" ({} of {} done)", this.done, this.entries.len())
                        } else {
                            String::new()
                        };
                        this.state = DeleteState::Failed {
                            message: format!("{name}: {}{progress}", failure.message),
                            retryable: failure.busy,
                        };
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
}

impl DeleteDialog {
    fn single_details(
        &self,
        entry: &'static FileEntry,
        palette: Palette,
        settings: &crate::settings::AppSettings,
    ) -> gpui_kit::AnyElement {
        let location = entry
            .path
            .and_then(Path::parent)
            .map(|parent| parent.display().to_string())
            .unwrap_or_default();
        let size = crate::properties::demo_human_size(entry, settings.human_sizes);
        v_flex()
            .gap_1()
            .p_3()
            .rounded_md()
            .bg(palette.surface)
            .text_xs()
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(entry.name))
            .child(div().text_color(palette.muted).child(location))
            .child(div().text_color(palette.muted).child(if entry.is_folder() {
                format!("Folder · {} · {size} · everything inside is deleted too", entry.metadata)
            } else {
                format!("{} · {size}", entry.kind.label())
            }))
            .into_any_element()
    }

    fn many_details(
        &self,
        palette: Palette,
        settings: &crate::settings::AppSettings,
    ) -> gpui_kit::AnyElement {
        const SHOWN: usize = 6;
        let folders = self.entries.iter().filter(|entry| entry.is_folder()).count();
        let total: Option<u64> = self
            .entries
            .iter()
            .map(|entry| crate::properties::demo_size_bytes(entry))
            .sum();
        let total = total.map_or_else(
            || "calculating size…".to_string(),
            |bytes| crate::settings::format_size(bytes, settings.human_sizes),
        );
        let more = self.entries.len().saturating_sub(SHOWN);
        v_flex()
            .gap_1()
            .p_3()
            .rounded_md()
            .bg(palette.surface)
            .text_xs()
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(format!(
                "{} items · {total}",
                self.entries.len()
            )))
            .children((folders > 0).then(|| {
                div().text_color(palette.muted).child(format!(
                    "Includes {folders} folder{} · everything inside is deleted too",
                    if folders == 1 { "" } else { "s" }
                ))
            }))
            .children(self.entries.iter().take(SHOWN).enumerate().map(|(index, entry)| {
                div()
                    .truncate()
                    .text_color(if index < self.done { palette.faint } else { palette.text })
                    .child(if index < self.done {
                        format!("✓ {}", entry.name)
                    } else {
                        entry.name.to_string()
                    })
            }))
            .children((more > 0).then(|| div().text_color(palette.faint).child(format!("+{more} more"))))
            .into_any_element()
    }
}

impl Render for DeleteDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&settings);
        let details = if let [entry] = self.entries.as_slice() {
            self.single_details(entry, palette, &settings)
        } else {
            self.many_details(palette, &settings)
        };
        let mut body = v_flex().gap_4().child(details);
        let mut buttons = h_flex().gap_2().justify_end();
        let cancel = Button::new("delete-cancel")
            .ghost()
            .small()
            .label("Cancel")
            .on_click(|_, window, cx| window.close_dialog(cx));

        match &self.state {
            DeleteState::Protected(protected) => {
                let lines = protected
                    .iter()
                    .map(|(name, rule)| format!("{name} — rule \"{rule}\""))
                    .collect::<Vec<_>>()
                    .join("\n");
                body = body.child(
                    h_flex()
                        .gap_2()
                        .items_start()
                        .text_xs()
                        .child(Icon::new(IconName::ShieldAlert).small().text_color(palette.archive))
                        .child(
                            v_flex()
                                .flex_1()
                                .gap_1()
                                .child(if protected.len() == 1 && self.entries.len() == 1 {
                                    "This item is protected from deletion.".to_string()
                                } else {
                                    format!(
                                        "{} of the selected items are protected, so nothing was deleted:",
                                        protected.len()
                                    )
                                })
                                .child(div().text_color(palette.muted).child(lines))
                                .child("Change the list in Settings → Deletion if you really need to remove them."),
                        ),
                );
                buttons = buttons.child(
                    Button::new("delete-close")
                        .secondary()
                        .small()
                        .label("Close")
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                );
            }
            state => {
                let working = matches!(state, DeleteState::Working);
                if !self.in_trash {
                    body = body.child(
                        Checkbox::new("delete-permanently")
                            .label("Delete permanently instead of moving to Trash (cannot be undone)")
                            .checked(self.permanent)
                            .disabled(working)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.permanent = *checked;
                                cx.notify();
                            })),
                    );
                }
                if let DeleteState::Failed { message, retryable } = state {
                    body = body.child(
                        h_flex()
                            .gap_2()
                            .items_start()
                            .text_xs()
                            .child(Icon::new(IconName::TriangleAlert).small().text_color(palette.video))
                            .child(div().flex_1().text_color(palette.video).child(message.clone()))
                            .children(retryable.then(|| {
                                div().text_color(palette.muted).child("Retry when it is closed.")
                            })),
                    );
                }
                let permanent = self.permanent || self.in_trash;
                let label = match state {
                    DeleteState::Working => "Deleting…",
                    DeleteState::Failed { .. } => "Retry",
                    _ if permanent => "Delete permanently",
                    _ => "Move to Trash",
                };
                buttons = buttons.child(cancel.disabled(working)).child(
                    Button::new("delete-confirm")
                        .danger()
                        .small()
                        .icon(IconName::Trash)
                        .label(label)
                        .disabled(working)
                        .on_click(cx.listener(|this, _, window, cx| this.delete(window, cx))),
                );
            }
        }

        v_flex()
            .track_focus(&self.focus_handle)
            .gap_4()
            .text_color(palette.text)
            .child(body)
            .child(buttons)
    }
}

/// Confirmation for emptying the whole trash.
pub struct EmptyTrashDialog {
    explorer: WeakEntity<FileExplorer>,
    items: usize,
    working: bool,
    error: Option<String>,
    appearance: Entity<AppearanceSettings>,
    focus_handle: FocusHandle,
}

impl Focusable for EmptyTrashDialog {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EmptyTrashDialog {
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.working {
            self.empty(window, cx);
        }
        false
    }

    fn empty(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.working = true;
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async { crate::delete::empty_trash() }).await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(()) => {
                    crate::model::invalidate_listings();
                    let _ = this.explorer.update(cx, |explorer, cx| {
                        explorer.selected = None;
                        explorer.notice = Some(format!("{} emptied", Location::Trash.label()));
                        files_sdk::log::notify("files", files_sdk::log::Level::Success, format!("{} emptied", Location::Trash.label()), None);
                        cx.notify();
                    });
                    window.close_dialog(cx);
                }
                Err(error) => {
                    this.working = false;
                    this.error = Some(crate::delete::describe(&error).message);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Render for EmptyTrashDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&settings);
        let what = match self.items {
            0 => "All items".to_string(),
            1 => "1 item".to_string(),
            count => format!("{count} items"),
        };
        v_flex()
            .track_focus(&self.focus_handle)
            .gap_4()
            .text_color(palette.text)
            .child(div().text_sm().child(format!(
                "{what} in the {} will be deleted permanently. This cannot be undone.",
                Location::Trash.label()
            )))
            .children(self.error.clone().map(|error| {
                h_flex()
                    .gap_2()
                    .items_start()
                    .text_xs()
                    .child(Icon::new(IconName::TriangleAlert).small().text_color(palette.video))
                    .child(div().flex_1().text_color(palette.video).child(error))
            }))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("empty-trash-cancel")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .disabled(self.working)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("empty-trash-confirm")
                            .danger()
                            .small()
                            .icon(IconName::Trash)
                            .label(if self.working { "Emptying…" } else { "Empty Trash" })
                            .disabled(self.working)
                            .on_click(cx.listener(|this, _, window, cx| this.empty(window, cx))),
                    ),
            )
    }
}

impl FileExplorer {
    pub fn open_empty_trash_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let items = crate::model::entries(Location::Trash, "").len();
        let explorer = cx.entity().downgrade();
        let appearance = self.appearance.clone();
        let previous_focus = window.focused(cx);
        let dialog = cx.new(|cx| EmptyTrashDialog {
            explorer,
            items,
            working: false,
            error: None,
            appearance,
            focus_handle: cx.focus_handle(),
        });
        let initial_focus = Cell::new(true);
        window.open_dialog(cx, move |modal, window, cx| {
            if initial_focus.replace(false) {
                dialog.read(cx).focus_handle(cx).focus(window, cx);
            }
            modal
                .title(format!("Empty {}?", Location::Trash.label()))
                .centered()
                .w(px(440.))
                .close_button(false)
                .overlay(true)
                .overlay_closable(true)
                .child(dialog.clone())
                // Enter confirms; the dialog closes itself once the work succeeds.
                .on_ok({
                    let dialog = dialog.clone();
                    move |_, window, cx| dialog.update(cx, |dialog, cx| dialog.confirm(window, cx))
                })
                .on_close({
                    let previous_focus = previous_focus.clone();
                    move |_, window, cx| {
                        if let Some(focus) = &previous_focus {
                            focus.focus(window, cx);
                        }
                    }
                })
        });
    }

    pub fn open_delete_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let entries = self.selected_entries();
        if entries.is_empty() {
            return;
        }
        if let Some(remote) = entries.iter().find(|entry| entry.path.is_none()) {
            self.notice = Some(format!("{} is not on the local disk", remote.name));
            cx.notify();
            return;
        }
        let settings = self.appearance.read(cx).clone();
        let home = Location::Home.path().unwrap_or_default();
        let trash = crate::model::trash_dir();
        let in_trash = self.active_location() == Location::Trash
            || entries.iter().all(|entry| {
                trash.as_ref().zip(entry.path).is_some_and(|(trash, path)| path.starts_with(trash))
            });
        // One protected item blocks the whole batch: partial deletes surprise people.
        let protected: Vec<_> = entries
            .iter()
            .filter_map(|entry| {
                let rule = crate::delete::protection(entry.path?, &settings.deletion.protected, &home)?;
                Some((entry.name, rule.to_string()))
            })
            .collect();
        let state = if protected.is_empty() {
            DeleteState::Confirm
        } else {
            DeleteState::Protected(protected)
        };
        let protected = matches!(state, DeleteState::Protected(_));
        let label = match entries.as_slice() {
            [single] => format!("\"{}\"", single.name),
            all => format!("{} items", all.len()),
        };
        let explorer = cx.entity().downgrade();
        let appearance = self.appearance.clone();
        let previous_focus = window.focused(cx);
        let dialog = cx.new(|cx| DeleteDialog {
            explorer,
            entries,
            done: 0,
            in_trash,
            permanent: !settings.deletion.use_trash,
            state,
            _appearance_subscription: cx.observe(&appearance, |_, _, cx| cx.notify()),
            appearance,
            focus_handle: cx.focus_handle(),
        });
        let title = if protected {
            "Protected items".to_string()
        } else if in_trash || !settings.deletion.use_trash {
            format!("Delete {label} permanently?")
        } else {
            format!("Move {label} to {}?", Location::Trash.label())
        };
        let initial_focus = Cell::new(true);
        window.open_dialog(cx, move |modal, window, cx| {
            if initial_focus.replace(false) {
                dialog.read(cx).focus_handle(cx).focus(window, cx);
            }
            modal
                .title(title.clone())
                .centered()
                .w(px(480.))
                .close_button(false)
                .overlay(true)
                .overlay_closable(true)
                .child(dialog.clone())
                // Enter confirms; the dialog closes itself once the work succeeds.
                .on_ok({
                    let dialog = dialog.clone();
                    move |_, window, cx| dialog.update(cx, |dialog, cx| dialog.confirm(window, cx))
                })
                .on_close({
                    let previous_focus = previous_focus.clone();
                    move |_, window, cx| {
                        if let Some(focus) = &previous_focus {
                            focus.focus(window, cx);
                        }
                    }
                })
        });
    }

    /// Clears selections pointing at an entry that was just deleted.
    pub(crate) fn forget_selection(&mut self, id: &str) {
        self.selection.retain(|selected| *selected != id);
        let matches = |selected: Option<&'static str>| selected == Some(id);
        if matches(self.selected) {
            // Keep the rest of a multi-selection alive after a partial delete.
            self.selected = self.selection.last().copied();
        }
        if matches(self.selected) {
            self.selected = None;
        }
        if matches(self.left.selected) {
            self.left.selected = None;
        }
        if matches(self.right.selected) {
            self.right.selected = None;
        }
    }
}
