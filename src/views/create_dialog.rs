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
use crate::model::Location;
use crate::palette::Palette;

#[derive(Clone)]
enum Target {
    Disk(PathBuf),
}

pub struct CreateDialog {
    explorer: WeakEntity<FileExplorer>,
    target: Target,
    folder: bool,
    name: Entity<InputState>,
    error: Option<String>,
    appearance: Entity<AppearanceSettings>,
    _name_subscription: Subscription,
}

impl CreateDialog {
    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let name = self.name.read(cx).value().to_string();
        let created = match &self.target {
            Target::Disk(dir) => crate::create::create(dir, &name, self.folder).map(|path| {
                crate::model::invalidate_listings();
                crate::model::entry(&path.to_string_lossy())
            }),
        };
        match created {
            Ok(entry) => {
                let folder = self.folder;
                let _ = self.explorer.update(cx, |explorer, cx| {
                    if let Some(entry) = entry {
                        explorer.select_entry(entry.id, cx);
                    }
                    explorer.notice = Some(format!(
                        "Created {} {}",
                        if folder { "folder" } else { "file" },
                        name.trim()
                    ));
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

impl Render for CreateDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&settings);
        // Live validation; "already exists" is only known when creating.
        let hint = self
            .error
            .clone()
            .or_else(|| crate::create::invalid_name(self.name.read(cx).value().as_ref()));
        let content = v_flex();
        #[cfg(test)]
        let content = gpui_kit::InteractiveElement::debug_selector(content, {
            let folder = self.folder;
            move || if folder { "create-dialog-folder".into() } else { "create-dialog-file".into() }
        });
        content
            .gap_3()
            .text_color(palette.text)
            .child(div().text_xs().text_color(palette.muted).child(match &self.target {
                Target::Disk(dir) => format!("In {}", dir.display()),
            }))
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
                        Button::new("create-cancel")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("create-confirm")
                            .primary()
                            .small()
                            .icon(if self.folder { IconName::FolderPlus } else { IconName::FilePlus })
                            .label("Create")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.create(window, cx);
                            })),
                    ),
            )
    }
}

impl FileExplorer {
    /// Asks for a name, then creates an empty file or folder in `location`.
    pub fn open_create_dialog(
        &mut self,
        location: Location,
        folder: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let target = match location.path() {
            Some(dir) => Target::Disk(dir),
            None => {
                self.notice = Some(format!("Cannot create items in {}", location.label()));
                cx.notify();
                return;
            }
        };
        let explorer = cx.entity().downgrade();
        let appearance = self.appearance.clone();
        let previous_focus = window.focused(cx);
        let default_name = match &target {
            Target::Disk(dir) => crate::create::free_name(dir, folder),
        };
        let dialog = cx.new(|cx| {
            let name = cx.new(|cx| InputState::new(window, cx).default_value(default_name));
            let subscription = cx.subscribe_in(&name, window, |this: &mut CreateDialog, _, event, window, cx| {
                match event {
                    // The input swallows Enter, so it creates from here.
                    InputEvent::PressEnter { .. } => {
                        this.create(window, cx);
                    }
                    InputEvent::Change => {
                        this.error = None;
                        cx.notify();
                    }
                    _ => {}
                }
            });
            CreateDialog {
                explorer,
                target,
                folder,
                name,
                error: None,
                appearance,
                _name_subscription: subscription,
            }
        });
        let title = if folder { "New Folder" } else { "New File" };
        let initial_focus = Cell::new(true);
        window.open_dialog(cx, move |modal, window, cx| {
            if initial_focus.replace(false) {
                let (input, folder) = {
                    let dialog = dialog.read(cx);
                    (dialog.name.clone(), dialog.folder)
                };
                // Pre-select the name so typing replaces it right away.
                input.update(cx, |state, cx| {
                    state.focus(window, cx);
                    let range = crate::create::name_selection(state.value().as_ref(), folder);
                    state.set_selected_range(range, cx);
                });
            }
            modal
                .title(title)
                .centered()
                .w(px(440.))
                .close_button(false)
                .overlay(true)
                .overlay_closable(true)
                .child(dialog.clone())
                .on_ok({
                    let dialog = dialog.clone();
                    move |_, window, cx| dialog.update(cx, |dialog, cx| dialog.create(window, cx))
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
