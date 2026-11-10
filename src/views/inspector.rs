use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::app::FileExplorer;
use crate::model::{FileEntry, FileKind};
use crate::palette::Palette;
use crate::views::files::file_thumbnail;

impl FileExplorer {
    pub(crate) fn render_inspector(
        &mut self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let settings = self.appearance.read(cx).clone();
        let mut content = v_flex()
            .size_full()
            .min_h_0()
            .bg(palette.sidebar)
            .border_l_1()
            .border_color(palette.border)
            .child(
                h_flex()
                    .h(px(40.))
                    .px_4()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Preview"),
                    )
                    .child(
                        Button::new("close-inspector")
                            .ghost()
                            .xsmall()
                            .icon(IconName::PanelRightClose)
                            .tooltip("Hide inspector")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_inspector = false;
                                cx.notify();
                            })),
                    ),
            );

        let selected = self.selected_entries();
        if selected.len() > 1 {
            return content
                .child(self.render_multi_preview(&selected, palette, cx))
                .into_any_element();
        }

        if let Some(file) = self.selected_entry() {
            let location = if self.current == crate::model::Location::Recents {
                file.parent.label()
            } else {
                self.current.label()
            };
            let mut details = v_flex().p_4().gap_5();
            let look_inside = crate::ext_preview::enabled(&settings);
            details = details
                .child(if look_inside { preview_block(file, palette, settings.image_fit) } else { file_thumbnail(file, palette, 184.) })
                .when(file.kind == crate::model::FileKind::Image, |this| {
                    this.child(h_flex().justify_end().child(crate::views::quick_look::fit_button("inspector-fit", settings.image_fit, self.appearance.clone())))
                })
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(file.name),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.muted)
                                .child(file.metadata),
                        ),
                )
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_2()
                        .when(settings.extensions.copy, |this| {
                            this.child(
                                Button::new("copy-demo-path")
                                    .secondary()
                                    .small()
                                    .icon(IconName::Copy)
                                    .label("Copy path")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.copy_demo_path(cx);
                                    })),
                            )
                        })
                        .child(
                            Button::new("reveal-file")
                                .ghost()
                                .small()
                                .icon(IconName::FolderOpen)
                                .label("Reveal")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.navigate(file.parent, window, cx);
                                })),
                        )
                        .child(
                            Button::new("file-properties")
                                .ghost()
                                .small()
                                .icon(IconName::FileText)
                                .label("Properties")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.run_demo_command(
                                        crate::commands::DemoCommand::Properties,
                                        window,
                                        cx,
                                    );
                                })),
                        ),
                )
                .child(
                    v_flex()
                        .gap_3()
                        .child(inspector_section("DETAILS", palette))
                        .child(detail_row("Kind", file.kind.label(), palette))
                        .child(detail_row(
                            "Size",
                            crate::properties::demo_human_size(file, settings.human_sizes),
                            palette,
                        ))
                        .child(detail_row(
                            "Modified",
                            crate::settings::format_demo_date(file.modified, settings.date_time),
                            palette,
                        ))
                        .child(detail_row("Location", location, palette))
                        .children(match file.cloud {
                            crate::model::CloudState::OnlineOnly => Some(detail_row(
                                "Cloud",
                                "Online only · not downloaded",
                                palette,
                            )),
                            crate::model::CloudState::Downloaded => {
                                Some(detail_row("Cloud", "Downloaded · kept in sync", palette))
                            }
                            crate::model::CloudState::Local => None,
                        })
                        .children(
                            (file.is_folder() && file.path.is_some_and(crate::forks::has_forks))
                                .then(|| detail_row("Forks", "This folder has forks", palette)),
                        ),
                );

            // Everything the file itself says: EXIF, streams, compression.
            if let Some(path) = file.path.filter(|path| look_inside && crate::media_info::applies(path)) {
                match crate::media_info::sections(path) {
                    Some(sections) => {
                        for section in sections {
                            details = details.child(
                                v_flex()
                                    .gap_3()
                                    .child(inspector_section(section.title, palette))
                                    .children(section.rows.into_iter().map(|(label, value)| owned_row(label, value, palette))),
                            );
                        }
                    }
                    None => {
                        details = details.child(div().text_xs().text_color(palette.faint).child("Reading details…"));
                        self.redraw_when_details_arrive(cx);
                    }
                }
            }

            if !file.tags.is_empty() {
                details = details.child(
                    v_flex()
                        .gap_3()
                        .child(inspector_section("TAGS", palette))
                        .child(h_flex().flex_wrap().gap_2().children(file.tags.iter().map(
                            |tag| {
                                let tag: &'static str = tag;
                                // Finder's own color for the tag; accent when it has none.
                                let color = crate::finder_tags::color_of(tag)
                                    .hsla()
                                    .unwrap_or(palette.accent);
                                h_flex()
                                    .id(gpui_kit::ElementId::Name(format!("inspector-tag-{tag}").into()))
                                    .px_2()
                                    .py_1()
                                    .gap_1p5()
                                    .rounded_md()
                                    .bg(palette.surface)
                                    .border_1()
                                    .border_color(palette.border)
                                    .text_xs()
                                    .text_color(palette.text)
                                    .cursor_pointer()
                                    .hover(|style| style.bg(palette.hover))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.navigate(crate::model::Location::tag(tag), window, cx);
                                    }))
                                    .child(div().size(px(7.)).rounded_full().bg(color))
                                    .child(tag)
                                    .into_any_element()
                            },
                        ))),
                );
            }

            content = content.child(
                div()
                    .id("inspector-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(details),
            );
        } else {
            content = content.child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .p_6()
                    .gap_3()
                    .child(
                        div()
                            .size(px(48.))
                            .rounded_full()
                            .bg(palette.surface)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::new(IconName::Images)
                                    .large()
                                    .text_color(palette.faint),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Select a file"),
                    )
                    .child(
                        div()
                            .text_center()
                            .text_xs()
                            .text_color(palette.muted)
                            .child("Its preview, metadata and tags will appear here."),
                    ),
            );
        }

        content.into_any_element()
    }
}

impl FileExplorer {
    /// macOS-style preview for several items: a stack of thumbnail cards,
    /// a per-kind summary and the combined size.
    fn render_multi_preview(
        &self,
        entries: &[&'static FileEntry],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        const CARD: f32 = 132.;
        const SHOWN_NAMES: usize = 6;
        let settings = self.appearance.read(cx).clone();

        // The most recently picked item sits on top; up to three cards fan out behind it.
        let top: Vec<_> = entries.iter().rev().take(3).collect();
        let mut stack = div().relative().w_full().h(px(CARD + 40.));
        for (depth, entry) in top.iter().enumerate().rev() {
            let offset = depth as f32 * 14.;
            let scale = 1. - depth as f32 * 0.08;
            let size = CARD * scale;
            stack = stack.child(
                div()
                    .absolute()
                    .top(px(28. - offset))
                    .left(gpui_kit::relative(0.5))
                    .ml(px(-size / 2. + offset * 1.6))
                    .w(px(size))
                    .h(px(size))
                    .rounded_lg()
                    .overflow_hidden()
                    .border_2()
                    .border_color(palette.window)
                    .shadow_lg()
                    .bg(palette.surface)
                    .opacity(1. - depth as f32 * 0.18)
                    .child(file_thumbnail(entry, palette, size)),
            );
        }
        stack = stack.child(
            div()
                .absolute()
                .top(px(16.))
                .right(px(24.))
                .px_2()
                .py_0p5()
                .rounded_full()
                .bg(palette.accent)
                .text_color(palette.accent_contrast)
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .child(entries.len().to_string()),
        );

        let mut counts: Vec<(FileKind, usize)> = Vec::new();
        for entry in entries {
            match counts.iter_mut().find(|(kind, _)| *kind == entry.kind) {
                Some((_, count)) => *count += 1,
                None => counts.push((entry.kind, 1)),
            }
        }
        counts.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        let summary = counts
            .iter()
            .map(|(kind, count)| {
                let label = kind.label().to_lowercase();
                if *count == 1 { format!("1 {label}") } else { format!("{count} {label}s") }
            })
            .collect::<Vec<_>>()
            .join(", ");

        // Folder sizes arrive in the background; show "…" until all are known.
        let sizes: Option<u64> = entries
            .iter()
            .map(|entry| crate::properties::demo_size_bytes(entry))
            .sum();
        let total = sizes.map_or_else(
            || "Calculating…".to_string(),
            |bytes| crate::settings::format_size(bytes, settings.human_sizes),
        );

        let names = entries.iter().take(SHOWN_NAMES).map(|entry| {
            h_flex()
                .gap_2()
                .items_center()
                .text_xs()
                .child(crate::views::files::file_icon(entry, palette, 14.))
                .child(div().flex_1().min_w_0().truncate().child(entry.name))
        });
        let more = entries.len().saturating_sub(SHOWN_NAMES);

        v_flex()
            .p_4()
            .gap_4()
            .child(stack)
            .child(
                v_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{} items", entries.len())),
                    )
                    .child(div().text_xs().text_color(palette.muted).child(summary)),
            )
            .child(inspector_section("SELECTION", palette))
            .child(detail_row("Total size", total, palette))
            .child(v_flex().gap_1p5().children(names).when(more > 0, |this| {
                this.child(div().text_xs().text_color(palette.faint).child(format!("+{more} more")))
            }))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("multi-delete")
                            .secondary()
                            .small()
                            .icon(IconName::Trash)
                            .label("Delete…")
                            .on_click(cx.listener(|this, _, window, cx| this.open_delete_dialog(window, cx))),
                    )
                    .child(
                        Button::new("multi-clear")
                            .ghost()
                            .small()
                            .label("Clear selection")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.selected = None;
                                this.selection.clear();
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }
}

/// The top of the preview: the beginning of a text or code file (as
/// Quick Look shows it), a PDF's first page, else the thumbnail.
fn preview_block(file: &'static crate::model::FileEntry, palette: Palette, fit: crate::settings::ImageFit) -> AnyElement {
    use crate::model::FileKind;
    if let Some(path) = file.path {
        if matches!(file.kind, FileKind::Text | FileKind::Code)
            && let Some(text) = cached_text(path, file.kind)
        {
            return div()
                .id("inspector-text")
                .debug_selector(|| "inspector-text".into())
                .w_full()
                .h(px(220.))
                .p_3()
                .rounded_md()
                .bg(palette.surface)
                .overflow_y_scrollbar()
                .child(div().text_size(px(11.)).font_family(crate::palette::terminal_font()).text_color(palette.text).whitespace_normal().child(text))
                .into_any_element();
        }
        if crate::pdf_pages::is_pdf(path)
            && let Some(Some(first)) = crate::pdf_pages::pages(path).rendered.first().cloned()
        {
            return div()
                .w_full()
                .h(px(240.))
                .rounded_md()
                .overflow_hidden()
                .bg(palette.surface)
                .child({
                    use gpui_kit::StyledImage as _;
                    gpui_kit::img(first).size_full().object_fit(gpui_kit::ObjectFit::Contain)
                })
                .into_any_element();
        }
    }
    crate::views::files::file_thumbnail_fit(file, palette, 184., fit.object_fit())
}

/// The start of a text file, kept for the file shown (the preview redraws
/// often; the file is read once).
fn cached_text(path: &std::path::Path, kind: crate::model::FileKind) -> Option<String> {
    static LAST: std::sync::Mutex<Option<(std::path::PathBuf, Option<std::time::SystemTime>, Option<String>)>> = std::sync::Mutex::new(None);
    let modified = std::fs::metadata(path).ok()?.modified().ok();
    let mut last = LAST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((cached, when, text)) = &*last
        && cached == path
        && *when == modified
    {
        return text.clone();
    }
    // A short excerpt: the panel is small; Quick Look shows more.
    let text = crate::views::quick_look::read_text(path, kind).map(|text| text.lines().take(120).collect::<Vec<_>>().join("\n"));
    *last = Some((path.to_path_buf(), modified, text.clone()));
    text
}

/// Waits for the details being read in the background, then redraws.
static WAITING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

impl FileExplorer {
    fn redraw_when_details_arrive(&self, cx: &mut Context<Self>) {
        use std::sync::atomic::Ordering;
        if WAITING.swap(true, Ordering::Relaxed) {
            return;
        }
        let seen = crate::media_info::generation();
        cx.spawn(async move |this, cx| {
            for _ in 0..80 {
                cx.background_executor().timer(std::time::Duration::from_millis(120)).await;
                if crate::media_info::generation() != seen {
                    break;
                }
            }
            WAITING.store(false, Ordering::Relaxed);
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }
}

/// A detail row whose label and value are made at run time.
fn owned_row(label: String, value: String, palette: Palette) -> AnyElement {
    h_flex()
        .w_full()
        .gap_3()
        .items_start()
        .text_xs()
        .child(div().w(px(104.)).flex_shrink_0().text_color(palette.muted).child(label))
        .child(div().flex_1().min_w_0().text_color(palette.text).child(value))
        .into_any_element()
}

fn inspector_section(title: &'static str, palette: Palette) -> AnyElement {
    div()
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(palette.faint)
        .child(title)
        .into_any_element()
}

fn detail_row(label: &'static str, value: impl Into<SharedString>, palette: Palette) -> AnyElement {
    h_flex()
        .items_start()
        .gap_3()
        .text_xs()
        .child(div().w(px(68.)).text_color(palette.muted).child(label))
        .child(div().flex_1().text_color(palette.text).child(value.into()))
        .into_any_element()
}
