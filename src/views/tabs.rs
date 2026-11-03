use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{AnyElement, AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _, Styled as _, div, px};

use crate::app::FileExplorer;
use crate::model::Location;
use crate::palette::Palette;

impl FileExplorer {
    pub(crate) fn render_tabs(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let active_tab = self.active_tab;
        let active_location = self.active_location();
        let tab_count = self.tabs.len();
        let tabs = self.tabs.iter().enumerate().map(|(index, tab)| {
            let location = if index == active_tab {
                active_location
            } else {
                tab.location()
            };
            let active = index == active_tab;
            let (label, icon) = match tab.panel_icon {
                Some(icon) => (tab.panel_title.to_string(), icon),
                None => (crate::model::trim_middle(location.label(), 24).into_owned(), location_icon(location)),
            };
            let pinned = tab.pinned;
            let close = Button::new(("close-workspace-tab", tab.id))
                .ghost()
                .xsmall()
                .icon(IconName::X)
                .tooltip("Close tab")
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_tab(index, window, cx);
                }));
            let explorer = cx.entity();
            let drag_label = label.clone();
            // Right-click: close this / to the left / others / to the right, pin.
            let selector = tab.id;
            let title = h_flex()
                .id(("workspace-tab-title", tab.id))
                .debug_selector(move || format!("workspace-tab-{selector}"))
                .gap_1p5()
                .items_center()
                .child(Icon::new(if pinned { IconName::Pin } else { icon }).xsmall().text_color(if active { palette.accent } else { palette.muted }))
                .child(label)
                // Drag a tab onto another to move it there.
                .on_drag(DraggedTab { index, label: drag_label }, |tab, _, _, cx| cx.new(|_| tab.clone()))
                .drag_over::<DraggedTab>(move |style, _, _, _| style.bg(palette.accent_soft).rounded_md())
                .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| this.move_tab(dragged.index, index, window, cx)))
                .context_menu(move |menu, window, cx| {
                    let settings = explorer.read(cx).appearance.read(cx).clone();
                    // Each item: its color from Appearance, its hotkey (grey) from the keymap.
                    let item = |label: &'static str, icon: IconName, key: &str, action: Box<dyn gpui_kit::Action>| {
                        crate::views::context_menu::tinted_item(label, icon, crate::menu_colors::color_for(&settings, key)).action(action)
                    };
                    menu.item(item("Close", IconName::X, "tab-close", Box::new(crate::app::CloseTab)).disabled(tab_count == 1).on_click(window.listener_for(
                        &explorer,
                        move |this, _, window, cx| this.close_tab(index, window, cx),
                    )))
                    .item(item("Close Tabs to the Left", IconName::ArrowLeftToLine, "tab-close-left", Box::new(crate::app::CloseTabsLeft)).disabled(index == 0).on_click(
                        window.listener_for(&explorer, move |this, _, window, cx| this.close_tabs_where(index, |at, clicked| at < clicked, window, cx)),
                    ))
                    .item(item("Close Other Tabs", IconName::Copy, "tab-close-others", Box::new(crate::app::CloseOtherTabs)).disabled(tab_count == 1).on_click(
                        window.listener_for(&explorer, move |this, _, window, cx| this.close_tabs_where(index, |at, clicked| at != clicked, window, cx)),
                    ))
                    .item(
                        item("Close Tabs to the Right", IconName::ArrowRightToLine, "tab-close-right", Box::new(crate::app::CloseTabsRight))
                            .disabled(index + 1 == tab_count)
                            .on_click(window.listener_for(&explorer, move |this, _, window, cx| this.close_tabs_where(index, |at, clicked| at > clicked, window, cx))),
                    )
                    .separator()
                    .item(
                        item(if pinned { "Unpin Tab" } else { "Pin Tab" }, if pinned { IconName::PinOff } else { IconName::Pin }, "tab-pin", Box::new(crate::app::TogglePinTab))
                            .on_click(window.listener_for(&explorer, move |this, _, _, cx| this.toggle_pin_tab(index, cx))),
                    )
                });
            Tab::new()
                .prefix(title)
                .when(tab_count > 1 && !pinned, |tab| tab.suffix(close))
                // Middle click closes the tab, like a browser (never the last one).
                .on_mouse_up(
                    gpui_kit::MouseButton::Middle,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_tab(index, window, cx);
                    }),
                )
        });
        let explorer = cx.entity();

        h_flex()
            .h(px(38.))
            .w_full()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .bg(palette.window)
            .child(
                TabBar::new("workspace-tabs")
                    .folder()
                    .menu(true)
                    .max_width(px(190.))
                    .track_scroll(&self.tab_scroll)
                    .selected_index(active_tab)
                    .on_click(move |index, window, cx| {
                        explorer.update(cx, |this, cx| {
                            this.select_tab(*index, window, cx);
                        });
                    })
                    .children(tabs)
                    .suffix(
                        h_flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new("new-workspace-tab")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Plus)
                                    .tooltip(if cfg!(target_os = "macos") {
                                        "New Home tab · ⌘T"
                                    } else {
                                        "New Home tab · Ctrl+T"
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.new_tab(window, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .text_size(px(9.))
                                    .text_color(palette.faint)
                                    .child("LOCAL WORKSPACE"),
                            ),
                    )
                    .flex_1()
                    .min_w_0()
                    .h_full(),
            )
            .into_any_element()
    }
}

fn location_icon(location: Location) -> IconName {
    match location {
        Location::Recents => IconName::Clock3,
        Location::Home => IconName::House,
        Location::Documents => IconName::FileText,
        Location::Pictures => IconName::Images,
        Location::Downloads => IconName::Download,
        Location::Remote(_) => IconName::Cloud,
        Location::Trash => IconName::Trash,
        Location::Tag(_) => IconName::Tag,
        Location::Dir(_) if location.path().and_then(|path| crate::forks::fork_of(&path)).is_some() => {
            IconName::GitFork
        }
        Location::Dir(_) => IconName::Folder,
    }
}

/// A tab being dragged to a new place; also its floating preview.
#[derive(Clone)]
pub(crate) struct DraggedTab {
    pub(crate) index: usize,
    label: String,
}

impl gpui_kit::Render for DraggedTab {
    fn render(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) -> impl IntoElement {
        div().px_3().py_1().rounded_md().bg(gpui_kit::hsla(0., 0., 0.2, 0.9)).text_sm().text_color(gpui_kit::white()).child(self.label.clone())
    }
}
