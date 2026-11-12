use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::{Icon, InteractiveElementExt as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, FontWeight, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, StatefulInteractiveElement as _, Styled as _, div, px,
    transparent_black,
};

use crate::app::FileExplorer;
use crate::model::{Location, Pane};
use crate::palette::Palette;
use crate::views::context_menu::{file_context_menu, folder_context_menu};
use crate::views::drag_drop::{DragFilePreview, DraggedFile};
use crate::views::files::file_icon;

impl FileExplorer {
    pub(crate) fn render_dual_workspace(
        &mut self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .flex_1()
            .min_h_0()
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("dual-panes")
                        .child(
                            resizable_panel()
                                .size(px(500.))
                                .size_range(px(300.)..px(850.))
                                .child(self.render_dual_pane(Pane::Left, palette, cx)),
                        )
                        .child(resizable_panel().child(self.render_dual_pane(
                            Pane::Right,
                            palette,
                            cx,
                        ))),
                ),
            )
            .child(
                h_flex()
                    .h(px(34.))
                    .px_4()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(palette.border)
                    .bg(palette.surface)
                    .text_size(px(10.))
                    .text_color(palette.muted)
                    .child("Click to select · Double-click to open")
                    .child(if cfg!(target_os = "macos") {
                        "⌘1 Expressive    ⌘2 Pro    ⌘3 Dual    ⌘F Search"
                    } else {
                        "Ctrl 1 Expressive    Ctrl 2 Pro    Ctrl 3 Dual    Ctrl F Search"
                    }),
            )
            .into_any_element()
    }

    fn render_dual_pane(
        &mut self,
        pane: Pane,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (location, selected) = match pane {
            Pane::Left => (self.left.location, self.left.selected.clone()),
            Pane::Right => (self.right.location, self.right.selected.clone()),
        };
        let active = self.active_pane == pane;
        let header = self.dual_pane_header(pane, location, active, palette, cx);
        let files = self.entries_for(location);
        let settings = self.appearance.read(cx).clone();
        let source_window = cx.entity_id();
        let scope = if pane == Pane::Left { "dual-left" } else { "dual-right" };
        let [name_header, modified_header, size_header] = [
            crate::model::SortKey::Name,
            crate::model::SortKey::Modified,
            crate::model::SortKey::Size,
        ]
        .map(|key| self.sort_header(key, scope, palette, cx));
        let rows = files.into_iter().map(|entry| {
            let id = entry.id.clone();
            let is_selected = selected == Some(id.clone());
            h_flex()
                .id(entry.id.clone())
                // Middle click on a folder: open it in a new tab, like a browser.
                .when(entry.is_folder(), |this| {
                    this.on_mouse_up(MouseButton::Middle, { let entry = entry.clone(); cx.listener(move |this, _, window, cx| this.open_entry_in_new_tab(entry.clone(), window, cx)) })
                })
                .when(entry.path.as_deref().is_some_and(crate::transfer::is_cut), |this| this.opacity(0.45))
                .h(px(34.))
                .px_3()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(palette.border)
                .bg(if is_selected {
                    palette.selection
                } else {
                    transparent_black()
                })
                .hover(|style| style.bg(palette.hover))
                .on_click({ let id = id.clone(); cx.listener(move |this, _, window, cx| {
                    this.focus_main(window, cx);
                    this.select_in_pane(pane, &id, cx);
                }) })
                .on_double_click({ let id = id.clone(); cx.listener(move |this, _, window, cx| {
                    this.focus_main(window, cx);
                    this.active_pane = pane;
                    this.open_entry(&id, window, cx);
                }) })
                .when(entry.path.is_some(), |this| {
                    this.on_drag(
                        crate::views::drag_drop::DraggedFile {
                            entry: entry.clone(),
                            palette,
                            source_window,
                            source: cx.entity(),
                        },
                        |drag, _, _, cx| {
                            let preview = DragFilePreview::new(drag, cx);
                            cx.new(|_| preview)
                        },
                    )
                    .external_drag_payload(crate::views::drag_drop::native_payload)
                })
                .on_mouse_down(
                    MouseButton::Right,
                    { let id = id.clone(); cx.listener(move |this, _, window, cx| {
                        this.focus_main(window, cx);
                        this.select_in_pane(pane, &id, cx);
                    }) },
                )
                .context_menu(file_context_menu(entry.clone(), cx.entity()))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap_2()
                        .child(file_icon(&entry, palette, 15.))
                        .child(div().text_xs().min_w_0().child(
                            crate::views::files::name_with_badges(&entry, 36, palette),
                        )),
                )
                .child(
                    div()
                        .w(px(116.))
                        .text_size(px(10.))
                        .text_color(palette.muted)
                        .child(crate::settings::format_demo_date(
                            &entry.modified,
                            settings.date_time,
                        )),
                )
                .child(
                    div()
                        .w(px(58.))
                        .text_right()
                        .text_size(px(10.))
                        .text_color(palette.muted)
                        .child(crate::properties::demo_human_size(
                            &entry,
                            settings.human_sizes,
                        )),
                )
                .into_any_element()
        });
        let parent = location
            .parent()
            .filter(|_| self.query.trim().is_empty())
            .map(|_| {
                h_flex()
                    .id(("dual-parent-folder", pane as usize))
                    .h(px(34.))
                    .px_3()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(palette.border)
                    .bg(if active && self.parent_selected {
                        palette.selection
                    } else {
                        transparent_black()
                    })
                    .text_color(palette.muted)
                    .hover(|style| style.bg(palette.hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.focus_main(window, cx);
                        this.active_pane = pane;
                        this.parent_selected = true;
                        match pane {
                            Pane::Left => this.left.selected = None,
                            Pane::Right => this.right.selected = None,
                        }
                        cx.notify();
                    }))
                    .on_double_click(cx.listener(move |this, _, window, cx| {
                        this.active_pane = pane;
                        this.go_up(window, cx);
                    }))
                    .child(
                        h_flex()
                            .flex_1()
                            .items_center()
                            .gap_2()
                            .child(
                                Icon::new(IconName::Folder)
                                    .small()
                                    .text_color(palette.folder),
                            )
                            .child(div().text_xs().child("..")),
                    )
                    .child(div().w(px(116.)))
                    .child(div().w(px(58.)))
                    .into_any_element()
            });

        v_flex()
            .id(("dual-pane", pane as usize))
            .size_full()
            .min_w_0()
            .min_h_0()
            .border_t_2()
            .border_color(if active {
                palette.accent
            } else {
                transparent_black()
            })
            .bg(if active {
                palette.window
            } else {
                palette.sidebar
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.focus_main(window, cx);
                    if this.active_pane != pane {
                        this.parent_selected = false;
                    }
                    this.active_pane = pane;
                    cx.notify();
                }),
            )
            // Each pane is its own drop target: files dropped here land in
            // this pane's folder, whichever pane they were dragged from.
            .drag_over::<DraggedFile>(move |style, _, _, _| style.bg(palette.accent_soft))
            .drag_over::<gpui_kit::ExternalPaths>(move |style, _, _, _| style.bg(palette.accent_soft))
            .on_drop(cx.listener(move |this, drag: &DraggedFile, window, cx| {
                cx.stop_propagation();
                this.drop_dragged(pane, drag, window, cx);
            }))
            .on_drop(cx.listener(move |this, paths: &gpui_kit::ExternalPaths, window, cx| {
                cx.stop_propagation();
                this.drop_active = false;
                this.drop_into_pane(pane, paths.paths().to_vec(), window, cx);
            }))
            .context_menu(folder_context_menu(location, cx.entity()))
            .child(header)
            .child(
                h_flex()
                    .h(px(29.))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .bg(palette.surface)
                    .border_b_1()
                    .border_color(palette.border)
                    .text_size(px(9.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(palette.faint)
                    .child(div().flex_1().child(name_header))
                    .child(div().w(px(116.)).child(modified_header))
                    .child(h_flex().w(px(58.)).justify_end().child(size_header)),
            )
            .child(
                div()
                    .id(("dual-scroll", pane as usize))
                    .flex_1()
                    .min_h_0()
                    .on_mouse_down(
                        gpui_kit::MouseButton::Left,
                        cx.listener(move |this, event, _, cx| {
                            this.clear_selection_on_background(event, Some(pane), cx)
                        }),
                    )
                    .overflow_y_scrollbar()
                    .child(v_flex().children(parent).children(rows)),
            )
            .into_any_element()
    }

    /// A gpui drag dropped onto a pane. The source may be this very explorer
    /// (pane to pane), which is mid-update here and must not be read via `cx`.
    pub(crate) fn drop_dragged(
        &mut self,
        pane: Pane,
        drag: &DraggedFile,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        self.drop_active = false;
        self.pending_drop = None;
        if crate::model::remote::entry_path(&drag.entry).is_some() {
            let files = if drag.source.entity_id() == cx.entity_id() {
                self.remote_drag_entries(drag.entry.clone())
            } else {
                drag.source.update(cx, |source, _| source.pending_drop = None);
                drag.source.read(cx).remote_drag_entries(drag.entry.clone())
            };
            self.active_pane = pane;
            self.download_remote_into_active(files, cx);
            return;
        }
        let paths = if drag.source.entity_id() == cx.entity_id() {
            drag.paths_from(self)
        } else {
            drag.source.update(cx, |source, _| source.pending_drop = None);
            drag.paths(cx)
        };
        self.drop_into_pane(pane, paths, window, cx);
    }

    pub(crate) fn drop_into_pane(
        &mut self,
        pane: Pane,
        paths: Vec<std::path::PathBuf>,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        self.active_pane = pane;
        let target = self.active_location().path();
        // Dropping back onto the folder the files already live in is a no-op.
        if paths.is_empty() || paths.iter().all(|path| path.parent() == target.as_deref()) {
            cx.notify();
            return;
        }
        let copy = window.modifiers().alt;
        self.receive_files(paths, copy, window, cx);
    }

    fn dual_pane_header(
        &self,
        pane: Pane,
        location: Location,
        active: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Every crumb but the current folder opens that folder in this pane.
        let crumbs = location.ancestry();
        let last = crumbs.len().saturating_sub(1);
        let mut path = h_flex().gap_0p5().items_center().text_size(px(9.)).text_color(palette.muted);
        for (index, crumb_location) in crumbs.into_iter().enumerate() {
            if index > 0 {
                path = path.child(div().text_color(palette.faint).child("/"));
            }
            let label = crate::model::trim_middle(crumb_location.label(), 24).into_owned();
            let crumb = div().id(("dual-breadcrumb", pane as usize * 1000 + index)).rounded_sm();
            #[cfg(test)]
            let crumb = crumb.debug_selector(move || format!("dual-breadcrumb-{}-{index}", pane as usize));
            path = path.child(if index == last {
                crumb.child(label).into_any_element()
            } else {
                crumb
                    .cursor_pointer()
                    .hover(|style| style.bg(palette.hover).text_color(palette.text))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.active_pane = pane;
                        this.navigate(crumb_location, window, cx);
                    }))
                    .child(label)
                    .into_any_element()
            });
        }
        h_flex()
            .h(px(41.))
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(palette.border)
            .child(
                Icon::new(if active {
                    IconName::FolderOpen
                } else {
                    IconName::Folder
                })
                .small()
                .text_color(if active {
                    palette.accent
                } else {
                    palette.muted
                }),
            )
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(location.label()),
                    )
                    .child(path),
            )
            .child(div().flex_1())
            .child(
                div()
                    .px_1p5()
                    .py_0p5()
                    .rounded_sm()
                    .bg(if active {
                        palette.accent_soft
                    } else {
                        palette.surface
                    })
                    .text_size(px(9.))
                    .text_color(if active {
                        palette.accent
                    } else {
                        palette.faint
                    })
                    .child(if active { "ACTIVE" } else { "PANE" }),
            )
            .into_any_element()
    }
}
