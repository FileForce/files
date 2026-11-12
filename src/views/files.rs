use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::{Icon, InteractiveElementExt as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, FontWeight, InteractiveElement as _, IntoElement,
    MouseButton, ObjectFit, ParentElement as _, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _, div, img, px, transparent_black,
};

use crate::app::FileExplorer;
use crate::model::{FileEntry, FileKind, SortKey, ViewMode};
use crate::palette::Palette;
use crate::views::context_menu::file_context_menu;
use crate::views::drag_drop::{DragFilePreview, dragged_file};

impl FileExplorer {
    pub(crate) fn render_mode_content(
        &mut self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.mode {
            ViewMode::Expressive => self.render_expressive(palette, cx),
            ViewMode::Pro => self.render_pro(palette, cx),
            ViewMode::Dual => self.render_dual_workspace(palette, cx),
        }
    }

    fn render_expressive(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let files = self.current_entries();
        if files.is_empty() {
            return self.render_empty_state(palette, cx);
        }
        let folders: Vec<_> = files
            .iter()
            .cloned()
            .filter(|entry| entry.is_folder())
            .collect();
        let documents: Vec<_> = files
            .iter()
            .cloned()
            .filter(|entry| !entry.is_folder())
            .collect();
        let selection = self.selection_ids();
        let sort_bar = h_flex()
            .gap_3()
            .items_center()
            .text_size(px(10.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(palette.faint)
            .child("SORT")
            .children(
                [SortKey::Name, SortKey::Kind, SortKey::Modified, SortKey::Size]
                    .map(|key| self.sort_header(key, "expressive", palette, cx)),
            );
        let mut content = v_flex().p_5().gap_5().min_w_0().child(sort_bar);

        if !folders.is_empty() {
            content = content
                .child(section_title("Folders", folders.len(), palette))
                .child({
                    let reveal = self.reveal.clone().and_then(|id| folders.iter().position(|entry| entry.id == id));
                    let columns = self.grid_columns(Grid::Folders);
                    self.virtual_grid(Grid::Folders, folders.len(), columns, FOLDER_CARD_HEIGHT, CARD_GAP, reveal, |index| {
                        let entry = folders[index].clone();
                        { let selected = selection.contains(&entry.id); self.folder_card(entry, selected, palette, cx) }.into_any_element()
                    })
                });
        }
        if !documents.is_empty() {
            content = content
                .child(section_title("Files", documents.len(), palette))
                .child({
                    let reveal = self.reveal.clone().and_then(|id| documents.iter().position(|entry| entry.id == id));
                    let columns = self.grid_columns(Grid::Files);
                    self.virtual_grid(Grid::Files, documents.len(), columns, FILE_CARD_HEIGHT, CARD_GAP, reveal, |index| {
                        let entry = documents[index].clone();
                        { let selected = selection.contains(&entry.id); self.file_card(entry, selected, palette, cx) }.into_any_element()
                    })
                });
        }

        div()
            .id("expressive-scroll")
            .flex_1()
            .min_h_0()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event, _, cx| this.clear_selection_on_background(event, None, cx)),
            )
            .overflow_y_scroll()
            .track_scroll(&self.list_scroll)
            .child(content)
            // After the content, so it paints on top of the cards.
            .vertical_scrollbar(&self.list_scroll)
            .into_any_element()
    }

    fn folder_card(
        &self,
        entry: crate::model::Entry,
        selected: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = entry.id.clone();
        h_flex()
            .id(entry.id.clone())
            // Middle click on a folder: open it in a new tab, like a browser.
            .when(entry.is_folder(), |this| {
                this.on_mouse_up(MouseButton::Middle, { let entry = entry.clone(); cx.listener(move |this, _, window, cx| this.open_entry_in_new_tab(entry.clone(), window, cx)) })
            })
            .when(entry.path.as_deref().is_some_and(crate::transfer::is_cut), |this| this.opacity(0.45))
            .relative()
            .children(self.reveal_marker(&entry.id))
            .w(px(176.))
            .h(px(58.))
            .p_2()
            .gap_3()
            .items_center()
            .rounded_lg()
            .border_1()
            .border_color(if selected {
                palette.accent
            } else {
                palette.border
            })
            .bg(if selected {
                palette.selection
            } else {
                palette.surface
            })
            .hover(|style| style.bg(palette.hover))
            .on_click({ let id = id.clone(); cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                this.focus_main(window, cx);
                this.click_entry(&id, event.modifiers(), cx);
            }) })
            .on_double_click({ let id = id.clone(); cx.listener(move |this, _, window, cx| {
                this.open_entry(&id, window, cx);
            }) })
            .when(entry.path.is_some(), |this| {
                this.on_drag(dragged_file(entry.clone(), palette, cx), |drag, _, _, cx| {
                    let preview = DragFilePreview::new(drag, cx);
                    cx.new(|_| preview)
                })
                .external_drag_payload(crate::views::drag_drop::native_payload)
            })
            .on_mouse_down(
                MouseButton::Right,
                { let id = id.clone(); cx.listener(move |this, _, window, cx| {
                    this.focus_main(window, cx);
                    this.select_for_context_menu(&id, cx);
                }) },
            )
            .context_menu(file_context_menu(entry.clone(), cx.entity()))
            .child(
                div()
                    .size(px(34.))
                    .rounded_md()
                    .bg(palette.accent_soft)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(file_icon(&entry, palette, 19.)),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(name_with_badges(&entry, 16, palette)),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(palette.muted)
                            .child(entry.metadata.clone()),
                    ),
            )
            .into_any_element()
    }

    fn file_card(
        &self,
        entry: crate::model::Entry,
        selected: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = entry.id.clone();
        let size = crate::properties::demo_human_size(&entry, self.appearance.read(cx).human_sizes);
        let card = v_flex();
        #[cfg(test)]
        let card = card.debug_selector({ let name = entry.name.clone(); move || format!("card-{name}") });
        card
            .id(entry.id.clone())
            // Middle click on a folder: open it in a new tab, like a browser.
            .when(entry.is_folder(), |this| {
                this.on_mouse_up(MouseButton::Middle, { let entry = entry.clone(); cx.listener(move |this, _, window, cx| this.open_entry_in_new_tab(entry.clone(), window, cx)) })
            })
            .when(entry.path.as_deref().is_some_and(crate::transfer::is_cut), |this| this.opacity(0.45))
            .relative()
            .children(self.reveal_marker(&entry.id))
            .w(px(CARD_WIDTH))
            .h(px(FILE_CARD_HEIGHT))
            .overflow_hidden()
            .p_1()
            .gap_2()
            .rounded_lg()
            .border_1()
            .border_color(if selected {
                palette.accent
            } else {
                transparent_black()
            })
            .bg(if selected {
                palette.selection
            } else {
                transparent_black()
            })
            .hover(|style| style.bg(palette.hover))
            .on_click({ let id = id.clone(); cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                this.focus_main(window, cx);
                this.click_entry(&id, event.modifiers(), cx);
            }) })
            .on_double_click({ let id = id.clone(); cx.listener(move |this, _, window, cx| {
                this.open_entry(&id, window, cx);
            }) })
            .when(entry.path.is_some(), |this| {
                this.on_drag(dragged_file(entry.clone(), palette, cx), |drag, _, _, cx| {
                    let preview = DragFilePreview::new(drag, cx);
                    cx.new(|_| preview)
                })
                .external_drag_payload(crate::views::drag_drop::native_payload)
            })
            .on_mouse_down(
                MouseButton::Right,
                { let id = id.clone(); cx.listener(move |this, _, window, cx| {
                    this.focus_main(window, cx);
                    this.select_for_context_menu(&id, cx);
                }) },
            )
            .context_menu(file_context_menu(entry.clone(), cx.entity()))
            .child(file_thumbnail(&entry, palette, 108.))
            .child(
                v_flex()
                    .px_1()
                    .pb_1()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(name_with_badges(&entry, 22, palette)),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .text_size(px(10.))
                            .text_color(palette.muted)
                            .child(entry.kind.label())
                            .child(size),
                    ),
            )
            .into_any_element()
    }

    fn render_pro(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let files = self.current_entries();
        let settings = self.appearance.read(cx).clone();
        let parent = self
            .current
            .parent()
            .filter(|_| self.query.trim().is_empty())
            .map(|_| {
                h_flex()
                    .id("pro-parent-folder")
                    .h(px(35.))
                    .px_4()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(palette.border)
                    .bg(if self.parent_selected {
                        palette.selection
                    } else {
                        transparent_black()
                    })
                    .text_color(palette.muted)
                    .hover(|style| style.bg(palette.hover))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.focus_main(window, cx);
                        this.parent_selected = true;
                        this.selected = None;
                        cx.notify();
                    }))
                    .on_double_click(cx.listener(|this, _, window, cx| {
                        this.go_up(window, cx);
                    }))
                    .child(
                        h_flex()
                            .flex_1()
                            .items_center()
                            .gap_3()
                            .child(
                                Icon::new(IconName::Folder)
                                    .small()
                                    .text_color(palette.folder),
                            )
                            .child(div().text_sm().child("..")),
                    )
                    .child(div().w(px(76.)))
                    .child(div().w(px(132.)))
                    .child(div().w(px(64.)))
                    .into_any_element()
            });
        if files.is_empty() && parent.is_none() {
            return self.render_empty_state(palette, cx);
        }
        let selection = self.selection_ids();
        let clear_on_background =
            cx.listener(|this, event, _, cx| this.clear_selection_on_background(event, None, cx));
        let list_scroll = self.list_scroll.clone();
        let [name_header, kind_header, modified_header, size_header] =
            [SortKey::Name, SortKey::Kind, SortKey::Modified, SortKey::Size]
                .map(|key| self.sort_header(key, "pro", palette, cx));
        let reveal = self.reveal.clone().and_then(|id| files.iter().position(|entry| entry.id == id));
        let count = files.len();
        let mut make_row = |entry: crate::model::Entry| {
            let id = entry.id.clone();
            let is_selected = selection.contains(&id);
            let row = h_flex();
            #[cfg(test)]
            let row = row.debug_selector({ let name = entry.name.clone(); move || format!("pro-row-{name}") });
            row
                .id(entry.id.clone())
                // Middle click on a folder: open it in a new tab, like a browser.
                .when(entry.is_folder(), |this| {
                    this.on_mouse_up(MouseButton::Middle, { let entry = entry.clone(); cx.listener(move |this, _, window, cx| this.open_entry_in_new_tab(entry.clone(), window, cx)) })
                })
            .when(entry.path.as_deref().is_some_and(crate::transfer::is_cut), |this| this.opacity(0.45))
            .relative()
            .children(self.reveal_marker(&entry.id))
                .w_full()
                .h(px(PRO_ROW_HEIGHT))
                .px_4()
                .items_center()
                .gap_3()
                .border_b_1()
                .border_color(palette.border)
                .bg(if is_selected {
                    palette.selection
                } else {
                    transparent_black()
                })
                .hover(|style| style.bg(palette.hover))
                .on_click({ let id = id.clone(); cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                    this.focus_main(window, cx);
                    this.click_entry(&id, event.modifiers(), cx);
                }) })
                .on_double_click({ let id = id.clone(); cx.listener(move |this, _, window, cx| {
                    this.open_entry(&id, window, cx);
                }) })
                .when(entry.path.is_some(), |this| {
                    this.on_drag(dragged_file(entry.clone(), palette, cx), |drag, _, _, cx| {
                        let preview = DragFilePreview::new(drag, cx);
                        cx.new(|_| preview)
                    })
                    .external_drag_payload(crate::views::drag_drop::native_payload)
                })
                .on_mouse_down(
                    MouseButton::Right,
                    { let id = id.clone(); cx.listener(move |this, _, window, cx| {
                        this.focus_main(window, cx);
                        this.select_for_context_menu(&id, cx);
                    }) },
                )
                .context_menu(file_context_menu(entry.clone(), cx.entity()))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap_3()
                        .child(file_icon_or_app(&entry, palette, 16.))
                        .child(div().text_sm().flex_1().min_w_0().child(name_with_badges(&entry, 64, palette))),
                )
                .child(
                    div()
                        .w(px(76.))
                        .text_xs()
                        .text_color(palette.muted)
                        .child(entry.kind.label()),
                )
                .child(div().w(px(132.)).text_xs().text_color(palette.muted).child(
                    crate::settings::format_demo_date(&entry.modified, settings.date_time),
                ))
                .child(
                    div()
                        .w(px(64.))
                        .text_right()
                        .text_xs()
                        .text_color(palette.muted)
                        .child(crate::properties::demo_human_size(
                            &entry,
                            settings.human_sizes,
                        )),
                )
                .into_any_element()
        };
        let rows = self.virtual_grid(Grid::ProRows, count, 1, PRO_ROW_HEIGHT, 0., reveal, |index| make_row(files[index].clone()));

        v_flex()
            .flex_1()
            .min_h_0()
            .child(
                h_flex()
                    .h(px(31.))
                    .px_4()
                    .gap_3()
                    .items_center()
                    .bg(palette.surface)
                    .border_b_1()
                    .border_color(palette.border)
                    .text_size(px(10.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(palette.faint)
                    .child(div().flex_1().child(name_header))
                    .child(div().w(px(76.)).child(kind_header))
                    .child(div().w(px(132.)).child(modified_header))
                    .child(
                        h_flex()
                            .w(px(64.))
                            .justify_end()
                            .child(size_header),
                    ),
            )
            .child(
                div()
                    .id("pro-scroll")
                    .flex_1()
                    .min_h_0()
                    .on_mouse_down(MouseButton::Left, clear_on_background)
                    .overflow_y_scroll()
                    .track_scroll(&list_scroll)
                    .child({
                        let list = v_flex().children(parent).child(rows);
                        #[cfg(test)]
                        let list = list.debug_selector(|| "pro-scroll-probe".into());
                        list
                    })
                    // After the rows, so it paints on top of them.
                    .vertical_scrollbar(&list_scroll),
            )
            .into_any_element()
    }

    /// A clickable column label; the active column shows its direction.
    pub(crate) fn sort_header(
        &self,
        key: SortKey,
        scope: &'static str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = match key {
            SortKey::Name => "NAME",
            SortKey::Kind => "KIND",
            SortKey::Modified => "MODIFIED",
            SortKey::Size => "SIZE",
        };
        // The default order shows no arrow, so a reset is visible.
        let active = self.sort.key == key && self.sort != crate::model::SortOrder::default();
        h_flex()
            .id(gpui_kit::ElementId::Name(format!("sort-{scope}-{label}").into()))
            .gap_0p5()
            .items_center()
            .cursor_pointer()
            .text_color(if active { palette.text } else { palette.faint })
            .hover(|style| style.text_color(palette.text))
            .child(label)
            .when(active, |this| {
                this.child(if self.sort.ascending { "▲" } else { "▼" })
            })
            .on_click(cx.listener(move |this, _, _, cx| this.sort_by(key, cx)))
            .into_any_element()
    }

    fn render_empty_state(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let language = self.appearance.read(cx).language;
        // Nothing typed: the folder itself is empty, there is no filter to clear.
        let searching = !self.query.trim().is_empty();
        let (icon, title, hint) = if searching {
            (
                IconName::Search,
                crate::settings::localized(language, "No matching files", "Файлы не найдены"),
                crate::settings::localized(language, "Try a different name or clear the current filter.", "Измените запрос или очистите текущий фильтр."),
            )
        } else {
            (
                IconName::FolderOpen,
                crate::settings::localized(language, "This folder is empty", "Папка пуста"),
                crate::settings::localized(language, "Drop files here or create something new.", "Перетащите сюда файлы или создайте новый."),
            )
        };
        v_flex()
            .flex_1()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .p_8()
            .child(
                div()
                    .size(px(50.))
                    .rounded_full()
                    .bg(palette.accent_soft)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Icon::new(icon).large().text_color(palette.accent)),
            )
            .child(div().debug_selector(|| "empty-state-title".into()).text_lg().font_weight(FontWeight::SEMIBOLD).child(title))
            .child(div().text_sm().text_color(palette.muted).child(hint))
            .when(searching, |this| this.child(div().debug_selector(|| "clear-search".into()).child(
                Button::new("clear-search")
                    .secondary()
                    .small()
                    .label(crate::settings::localized(
                        language,
                        "Clear search",
                        "Очистить поиск",
                    ))
                    .icon(IconName::X)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.clear_search(window, cx);
                        cx.notify();
                    })),
            )))
            .into_any_element()
    }
}

fn section_title(title: &'static str, count: usize, palette: Palette) -> AnyElement {
    h_flex()
        .items_baseline()
        .gap_2()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .text_size(px(10.))
                .text_color(palette.faint)
                .child(count.to_string()),
        )
        .into_any_element()
}

/// The virtual grids of a folder view.
#[derive(Clone, Copy)]
pub(crate) enum Grid {
    ProRows = 0,
    Folders = 1,
    Files = 2,
}

const CARD_WIDTH: f32 = 176.;
const CARD_GAP: f32 = 12.;
const FOLDER_CARD_HEIGHT: f32 = 58.;
const FILE_CARD_HEIGHT: f32 = 168.;
const PRO_ROW_HEIGHT: f32 = 35.;
/// Rows built above and below the screen, so a fast scroll shows no gap.
const OVERSCAN: usize = 6;

impl FileExplorer {
    /// Cards per row in an Expressive grid of last frame's width.
    fn grid_columns(&self, grid: Grid) -> usize {
        let width = self.grid_bounds[grid as usize].get().map(|bounds| bounds.size.width).unwrap_or_else(|| self.list_scroll.bounds().size.width - px(40.));
        let width = if width > px(0.) { width.as_f32() } else { 960. };
        (((width + CARD_GAP) / (CARD_WIDTH + CARD_GAP)).floor() as usize).max(1)
    }

    /// The rows of `grid` (of `rows`, each `row_height` tall) on screen, from
    /// where it and the list were painted last frame. An item to reveal
    /// that is off screen scrolls into view first.
    fn visible_rows(&self, grid: Grid, rows: usize, row_height: f32, reveal_row: Option<usize>) -> std::ops::Range<usize> {
        let view = self.list_scroll.bounds();
        let Some(bounds) = self.grid_bounds[grid as usize].get().filter(|_| view.size.height > px(0.)) else {
            // Not painted yet: enough rows for a tall screen.
            let first = reveal_row.unwrap_or(0).saturating_sub(OVERSCAN);
            return first..rows.min(first + (1600. / row_height).ceil() as usize + OVERSCAN);
        };
        let above = (view.top() - bounds.top()).as_f32();
        let mut start = (above.max(0.) / row_height).floor() as usize;
        let mut end = ((above + view.size.height.as_f32()) / row_height).ceil().max(0.) as usize;
        if let Some(row) = reveal_row
            && (row < start || row >= end)
        {
            // Jump there: centre the row, the reveal marker refines it.
            let offset = self.list_scroll.offset();
            let grid_in_content = (bounds.top() - view.top()).as_f32() - offset.y.as_f32();
            let target = grid_in_content + row as f32 * row_height + row_height / 2. - view.size.height.as_f32() / 2.;
            let max = self.list_scroll.max_offset().y.as_f32().max(grid_in_content + rows as f32 * row_height);
            self.list_scroll.set_offset(gpui_kit::point(offset.x, px(-target.clamp(0., max))));
            let visible = (view.size.height.as_f32() / row_height).ceil() as usize;
            start = row.saturating_sub(visible / 2 + 1);
            end = row + visible / 2 + 1;
        }
        start.saturating_sub(OVERSCAN).min(rows)..(end + OVERSCAN).min(rows)
    }

    /// A grid of `count` items `columns` wide, built only where it shows:
    /// full height, with the rows on screen placed in it.
    fn virtual_grid(
        &self,
        grid: Grid,
        count: usize,
        columns: usize,
        row_height: f32,
        gap: f32,
        reveal: Option<usize>,
        mut item: impl FnMut(usize) -> AnyElement,
    ) -> AnyElement {
        let rows = count.div_ceil(columns.max(1));
        let range = self.visible_rows(grid, rows, row_height + gap, reveal.map(|index| index / columns.max(1)));
        let cell = self.grid_bounds[grid as usize].clone();
        let height = (rows as f32 * (row_height + gap) - gap).max(0.);
        div()
            .relative()
            .w_full()
            .h(px(height))
            .child(gpui_kit::canvas(move |bounds, _, _| cell.set(Some(bounds)), |_, _, _, _| {}).absolute().inset_0())
            .children(range.map(|row| {
                let first = row * columns;
                h_flex()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(row as f32 * (row_height + gap)))
                    .h(px(row_height))
                    .gap(px(gap))
                    .children((first..(first + columns).min(count)).map(&mut item))
            }))
            .into_any_element()
    }
}

/// The program's own icon when it has one (a Windows .exe), else the kind's.
pub(crate) fn file_icon_or_app(entry: &FileEntry, palette: Palette, size: f32) -> AnyElement {
    match entry.path.as_deref().and_then(crate::thumbnails::app_icon) {
        Some(icon) => img(icon).size(px(size)).flex_shrink_0().object_fit(ObjectFit::Contain).into_any_element(),
        None => file_icon(entry, palette, size).into_any_element(),
    }
}

pub(crate) fn file_icon(entry: &FileEntry, palette: Palette, size: f32) -> Icon {
    kind_icon(entry.kind, palette, size)
}

/// The icon and color files of `kind` have everywhere (lists, mail…).
pub(crate) fn kind_icon(kind: FileKind, palette: Palette, size: f32) -> Icon {
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
    let color = match palette.icon_theme {
        crate::settings::IconTheme::Colorful => color,
        crate::settings::IconTheme::Monochrome if kind == FileKind::Folder => palette.text,
        crate::settings::IconTheme::Monochrome => palette.muted,
        crate::settings::IconTheme::Accent => palette.accent,
    };
    Icon::new(icon).with_size(px(size)).text_color(color)
}

/// Small status icons shown before a name: cloud placeholder, downloaded cloud
/// copy, and "this folder has forks".
pub(crate) fn status_badges(entry: &FileEntry, palette: Palette) -> Option<AnyElement> {
    let cloud = match entry.cloud {
        crate::model::CloudState::OnlineOnly => Some(
            Icon::new(IconName::Cloud).with_size(px(12.)).text_color(palette.muted),
        ),
        crate::model::CloudState::Downloaded => Some(
            Icon::new(IconName::CloudCheck).with_size(px(12.)).text_color(palette.code),
        ),
        crate::model::CloudState::Local => None,
    };
    let forked = entry.is_folder() && entry.path.as_deref().is_some_and(crate::forks::has_forks);
    if cloud.is_none() && !forked {
        return None;
    }
    Some(
        h_flex()
            .flex_shrink_0()
            .gap_1()
            .items_center()
            .children(cloud)
            .children(forked.then(|| {
                Icon::new(IconName::GitFork).with_size(px(12.)).text_color(palette.accent)
            }))
            .into_any_element(),
    )
}

/// Status badges followed by the name; only the name is truncated.
/// `max_chars` trims long names in the middle (extension kept); CSS
/// truncation stays as a safety net for narrow widths.
pub(crate) fn name_with_badges(entry: &FileEntry, max_chars: usize, palette: Palette) -> AnyElement {
    let name = crate::model::trim_middle(&entry.name, max_chars).into_owned();
    h_flex()
        .w_full()
        .min_w_0()
        .gap_1p5()
        .items_center()
        // Badges sit right after the file icon, before the name.
        .children(status_badges(entry, palette))
        .child(crate::views::text_fx::outlined(name, palette))
        .into_any_element()
}

fn kind_placeholder(kind: FileKind, palette: Palette) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Icon::new(IconName::FileImage).with_size(px(32.)).text_color(palette.media))
        .child(
            div()
                .text_size(px(9.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.faint)
                .child(kind.label().to_uppercase()),
        )
        .into_any_element()
}

pub(crate) fn file_thumbnail(entry: &FileEntry, palette: Palette, height: f32) -> AnyElement {
    file_thumbnail_fit(entry, palette, height, ObjectFit::Cover)
}

/// A thumbnail whose picture fits (whole) or fills (cropped) its frame.
pub(crate) fn file_thumbnail_fit(entry: &FileEntry, palette: Palette, height: f32, fit: ObjectFit) -> AnyElement {
    let base = div()
        .relative()
        .w_full()
        .h(px(height))
        .rounded_md()
        .overflow_hidden()
        .bg(palette.surface);

    let source = match (entry.preview, entry.path.clone()) {
        (Some(preview), _) => Some(gpui_kit::ImageSource::from(preview)),
        // A small cached copy (HEIC/AVIF converted); until it is ready the icon shows.
        (None, Some(path)) if entry.kind == FileKind::Image => {
            crate::thumbnails::thumbnail(&path).map(gpui_kit::ImageSource::from)
        }
        // A program's own icon, centred like Explorer shows it.
        (None, Some(path)) => {
            if let Some(icon) = crate::thumbnails::app_icon(&path) {
                let size = (height * 0.62).min(128.);
                return base.flex().items_center().justify_center().child(img(icon).size(px(size)).object_fit(ObjectFit::Contain)).into_any_element();
            }
            None
        }
        _ => None,
    };
    if let Some(source) = source {
        let kind = entry.kind;
        let image = base.child(
            img(source)
                .w_full()
                .h_full()
                .object_fit(fit)
                // A file that fails to decode shows its kind instead of a blank card.
                .with_fallback(move || kind_placeholder(kind, palette)),
        );
        if entry.kind == FileKind::Video {
            return image
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .size(px(36.))
                                .rounded_full()
                                .bg(palette.elevated)
                                .border_1()
                                .border_color(palette.border)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(IconName::Play)
                                        .with_size(px(16.))
                                        .text_color(palette.accent),
                                ),
                        ),
                )
                .into_any_element();
        }
        return image.into_any_element();
    }

    base.child(
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(file_icon(entry, palette, 32.))
            .child(
                div()
                    .text_size(px(9.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(palette.faint)
                    .child(entry.kind.label().to_uppercase()),
            ),
    )
    .into_any_element()
}
