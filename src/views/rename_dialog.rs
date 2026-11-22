use std::cell::Cell;
use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, WeakEntity, Window, div, px,
};

use crate::app::{AppearanceSettings, FileExplorer};
use crate::palette::Palette;

pub struct RenameDialog {
    explorer: WeakEntity<FileExplorer>,
    source: PathBuf,
    old_name: String,
    folder: bool,
    name: Entity<InputState>,
    error: Option<String>,
    appearance: Entity<AppearanceSettings>,
    _name_subscription: Subscription,
}

impl RenameDialog {
    fn rename(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let name = self.name.read(cx).value().to_string();
        match crate::rename::rename(&self.source, &name) {
            Ok(path) => {
                crate::model::invalidate_listings();
                let entry = crate::model::entry(&path.to_string_lossy());
                let old_name = self.old_name.clone();
                let _ = self.explorer.update(cx, |explorer, cx| {
                    if let Some(entry) = entry {
                        explorer.select_entry(&entry.id, cx);
                    }
                    explorer.notice = Some(format!("Renamed {old_name} to {}", name.trim()));
                    cx.notify();
                });
                window.close_dialog(cx);
                true
            }
            Err(error) => {
                self.error = Some(match error.kind() {
                    std::io::ErrorKind::AlreadyExists => {
                        format!("\"{}\" already exists here", name.trim())
                    }
                    std::io::ErrorKind::InvalidInput => error.to_string(),
                    _ => crate::delete::describe(&error).message,
                });
                cx.notify();
                false
            }
        }
    }
}

impl Render for RenameDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&settings);
        let hint = self
            .error
            .clone()
            .or_else(|| crate::create::invalid_name(self.name.read(cx).value().as_ref()));
        let content = v_flex();
        #[cfg(test)]
        let content =
            gpui_kit::InteractiveElement::debug_selector(content, || "rename-dialog".into());
        content
            .gap_3()
            .text_color(palette.text)
            .child(
                div()
                    .text_xs()
                    .text_color(palette.muted)
                    .child(format!("Rename {}", self.old_name)),
            )
            .child(Input::new(&self.name).w_full())
            .children(hint.map(|hint| {
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .text_color(palette.video)
                    .child(Icon::new(IconName::TriangleAlert).small())
                    .child(hint)
            }))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("rename-cancel")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("rename-confirm")
                            .primary()
                            .small()
                            .icon(IconName::Pencil)
                            .label("Rename")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.rename(window, cx);
                            })),
                    ),
            )
    }
}

impl FileExplorer {
    pub fn open_rename_dialog(
        &mut self,
        entry: crate::model::Entry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let Some(source) = entry.path.clone() else {
            self.notice = Some(format!("{} is not on the local disk", entry.name));
            cx.notify();
            return;
        };
        let explorer = cx.entity().downgrade();
        let appearance = self.appearance.clone();
        let previous_focus = window.focused(cx);
        let old_name = entry.name.to_string();
        let folder = entry.is_folder();
        let dialog = cx.new(|cx| {
            let name = cx.new(|cx| InputState::new(window, cx).default_value(old_name.clone()));
            let subscription = cx.subscribe_in(
                &name,
                window,
                |this: &mut RenameDialog, _, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => {
                        this.rename(window, cx);
                    }
                    InputEvent::Change => {
                        this.error = None;
                        cx.notify();
                    }
                    _ => {}
                },
            );
            RenameDialog {
                explorer,
                source: source.to_path_buf(),
                old_name: old_name.clone(),
                folder,
                name,
                error: None,
                appearance,
                _name_subscription: subscription,
            }
        });
        let initial_focus = Cell::new(true);
        window.open_dialog(cx, move |modal, window, cx| {
            if initial_focus.replace(false) {
                let (input, folder) = {
                    let dialog = dialog.read(cx);
                    (dialog.name.clone(), dialog.folder)
                };
                input.update(cx, |state, cx| {
                    state.focus(window, cx);
                    let range = crate::create::name_selection(state.value().as_ref(), folder);
                    state.set_selected_range(range, cx);
                });
            }
            modal
                .title("Rename")
                .centered()
                .w(px(440.))
                .close_button(false)
                .overlay(true)
                .overlay_closable(true)
                .child(dialog.clone())
                .on_ok({
                    let dialog = dialog.clone();
                    move |_, window, cx| dialog.update(cx, |dialog, cx| dialog.rename(window, cx))
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
}
