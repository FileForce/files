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
            .copied()
            .filter(|entry| entry.is_folder())
            .collect();
        let documents: Vec<_> = files
            .iter()
            .copied()
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
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_3()
                        .children(folders.into_iter().map(|entry| {
                            self.folder_card(entry, selection.contains(&entry.id), palette, cx)
                        })),
                );
        }
        if !documents.is_empty() {
            content = content
                .child(section_title("Files", documents.len(), palette))
                .child(h_flex().flex_wrap().gap_3().children(
                    documents.into_iter().map(|entry| {
                        self.file_card(entry, selection.contains(&entry.id), palette, cx)
                    }),
                ));
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
        entry: &'static FileEntry,
        selected: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = entry.id;
        h_flex()
            .id(entry.id)
            .when(entry.path.is_some_and(crate::transfer::is_cut), |this| this.opacity(0.45))
            .relative()
            .children(self.reveal_marker(entry.id))
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
            .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                this.focus_main(window, cx);
                this.click_entry(id, event.modifiers(), cx);
            }))
            .on_double_click(cx.listener(move |this, _, window, cx| {
                this.open_entry(id, window, cx);
            }))
            .when(entry.path.is_some(), |this| {
                this.on_drag(dragged_file(entry, palette, cx), |drag, _, _, cx| {
                    let preview = DragFilePreview::new(drag, cx);
                    cx.new(|_| preview)
                })
                .external_drag_payload(crate::views::drag_drop::native_payload)
            })
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    this.focus_main(window, cx);
                    this.select_for_context_menu(id, cx);
                }),
            )
            .context_menu(file_context_menu(entry, cx.entity()))
            .child(
                div()
                    .size(px(34.))
                    .rounded_md()
                    .bg(palette.accent_soft)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(file_icon(entry, palette, 19.)),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(name_with_badges(entry, 16, palette)),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(palette.muted)
                            .child(entry.metadata),
                    ),
            )
            .into_any_element()
    }

    fn file_card(
        &self,
        entry: &'static FileEntry,
        selected: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = entry.id;
        let size = crate::properties::demo_human_size(entry, self.appearance.read(cx).human_sizes);
        let card = v_flex();
        #[cfg(test)]
        let card = card.debug_selector(move || format!("card-{}", entry.name));
        card
            .id(entry.id)
            .when(entry.path.is_some_and(crate::transfer::is_cut), |this| this.opacity(0.45))
            .relative()
            .children(self.reveal_marker(entry.id))
            .w(px(176.))
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
            .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                this.focus_main(window, cx);
                this.click_entry(id, event.modifiers(), cx);
            }))
            .on_double_click(cx.listener(move |this, _, window, cx| {
                this.open_entry(id, window, cx);
            }))
            .when(entry.path.is_some(), |this| {
                this.on_drag(dragged_file(entry, palette, cx), |drag, _, _, cx| {
                    let preview = DragFilePreview::new(drag, cx);
                    cx.new(|_| preview)
                })
                .external_drag_payload(crate::views::drag_drop::native_payload)
            })
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    this.focus_main(window, cx);
                    this.select_for_context_menu(id, cx);
                }),
            )
            .context_menu(file_context_menu(entry, cx.entity()))
            .child(file_thumbnail(entry, palette, 108.))
            .child(
                v_flex()
                    .px_1()
                    .pb_1()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(name_with_badges(entry, 22, palette)),
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
        let rows = files.into_iter().map(|entry| {
            let id = entry.id;
            let is_selected = selection.contains(&id);
            let row = h_flex();
            #[cfg(test)]
            let row = row.debug_selector(move || format!("pro-row-{}", entry.name));
            row
                .id(entry.id)
            .when(entry.path.is_some_and(crate::transfer::is_cut), |this| this.opacity(0.45))
            .relative()
            .children(self.reveal_marker(entry.id))
                .h(px(35.))
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
                .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                    this.focus_main(window, cx);
                    this.click_entry(id, event.modifiers(), cx);
                }))
                .on_double_click(cx.listener(move |this, _, window, cx| {
                    this.open_entry(id, window, cx);
                }))
                .when(entry.path.is_some(), |this| {
                    this.on_drag(dragged_file(entry, palette, cx), |drag, _, _, cx| {
                        let preview = DragFilePreview::new(drag, cx);
                        cx.new(|_| preview)
                    })
                    .external_drag_payload(crate::views::drag_drop::native_payload)
                })
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, _, window, cx| {
                        this.focus_main(window, cx);
                        this.select_for_context_menu(id, cx);
                    }),
                )
                .context_menu(file_context_menu(entry, cx.entity()))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap_3()
                        .child(file_icon(entry, palette, 16.))
                        .child(div().text_sm().flex_1().min_w_0().child(name_with_badges(entry, 64, palette))),
                )
                .child(
                    div()
                        .w(px(76.))
                        .text_xs()
                        .text_color(palette.muted)
                        .child(entry.kind.label()),
                )
                .child(div().w(px(132.)).text_xs().text_color(palette.muted).child(
                    crate::settings::format_demo_date(entry.modified, settings.date_time),
                ))
                .child(
                    div()
                        .w(px(64.))
                        .text_right()
                        .text_xs()
                        .text_color(palette.muted)
                        .child(crate::properties::demo_human_size(
                            entry,
                            settings.human_sizes,
                        )),
                )
                .into_any_element()
        });

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
                        let list = v_flex().children(parent).children(rows);
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
        FileKind::Text => (IconName::FileText, palette.muted),
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
    let forked = entry.is_folder() && entry.path.is_some_and(crate::forks::has_forks);
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
    let name = crate::model::trim_middle(entry.name, max_chars).into_owned();
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
    let base = div()
        .relative()
        .w_full()
        .h(px(height))
        .rounded_md()
        .overflow_hidden()
        .bg(palette.surface);

    let source = match (entry.preview, entry.path) {
        (Some(preview), _) => Some(gpui_kit::ImageSource::from(preview)),
        // HEIC/AVIF go through a cached conversion; until it is ready the icon shows.
        (None, Some(path)) if entry.kind == FileKind::Image => {
            crate::thumbnails::source(path).map(gpui_kit::ImageSource::from)
        }
        _ => None,
    };
    if let Some(source) = source {
        let kind = entry.kind;
        let image = base.child(
            img(source)
                .w_full()
                .h_full()
                .object_fit(ObjectFit::Cover)
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
