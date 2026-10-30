use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::{Enter, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::{Icon, Sizable as _, WindowExt as _};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div, px,
};

use crate::app::{AppearanceSettings, FileExplorer};
use crate::model::{FileEntry, rank_entries, searchable_entries};
use crate::palette::Palette;
use crate::properties::demo_human_size;
use crate::views::files::file_icon;

const MAX_RESULTS: usize = 8;

pub struct GlobalSearch {
    explorer: WeakEntity<FileExplorer>,
    previous_focus: Option<FocusHandle>,
    appearance: Entity<AppearanceSettings>,
    include_remote: bool,
    input: Entity<InputState>,
    query: String,
    results: Vec<&'static FileEntry>,
    selected: usize,
    scroll_handle: ScrollHandle,
    _input_subscription: Subscription,
    _appearance_subscription: Subscription,
}

impl GlobalSearch {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        explorer: WeakEntity<FileExplorer>,
        previous_focus: Option<FocusHandle>,
        appearance: Entity<AppearanceSettings>,
        include_remote: bool,
    ) -> Self {
        let language = appearance.read(cx).language;
        crate::properties::watch_folder_sizes(cx);
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(crate::settings::localized(
                language,
                "Search home · try kind:video size:>100mb modified:<30d",
                "Поиск в home · например kind:video size:>100mb modified:<30d",
            ))
        });
        let input_subscription = cx.subscribe(&input, |this, state, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = state.read(cx).value().to_string();
                this.results = ranked_results(&this.query, this.include_remote);
                this.selected = 0;
                this.scroll_handle.scroll_to_item(0);
                cx.notify();
            }
        });
        let appearance_subscription = cx.observe(&appearance, |_, _, cx| cx.notify());

        Self {
            explorer,
            previous_focus,
            appearance,
            include_remote,
            input,
            query: String::new(),
            results: ranked_results("", include_remote),
            selected: 0,
            scroll_handle: ScrollHandle::new(),
            _input_subscription: input_subscription,
            _appearance_subscription: appearance_subscription,
        }
    }

    fn previous(&mut self, cx: &mut Context<Self>) {
        self.selected = self.selected.saturating_sub(1);
        self.scroll_handle.scroll_to_item(self.selected);
        cx.notify();
    }

    fn next(&mut self, cx: &mut Context<Self>) {
        if !self.results.is_empty() {
            self.selected = (self.selected + 1).min(self.results.len() - 1);
            self.scroll_handle.scroll_to_item(self.selected);
            cx.notify();
        }
    }

    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(file) = self.results.get(self.selected).copied() {
            self.open_file(file, window, cx);
        }
    }

    fn open_file(&mut self, file: &'static FileEntry, window: &mut Window, cx: &mut Context<Self>) {
        let explorer = self.explorer.clone();
        let _ = explorer.update(cx, |explorer, cx| {
            explorer.open_global_result(file, window, cx);
        });
        self.close(window, cx);
    }

    fn close(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        close_global_search(&self.explorer, &self.previous_focus, window, cx);
    }

    fn result_row(
        &self,
        index: usize,
        file: &'static FileEntry,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = index == self.selected;
        h_flex()
            .id(("global-search-result", index))
            .h(px(48.))
            .flex_shrink_0()
            .px_3()
            .gap_3()
            .items_center()
            .rounded_md()
            .border_1()
            .border_color(if selected {
                palette.accent
            } else {
                palette.window
            })
            .bg(if selected {
                palette.selection
            } else {
                palette.window
            })
            .hover(|style| style.bg(palette.hover))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = index;
                this.open_file(file, window, cx);
            }))
            .child(file_icon(file, palette, 20.))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(div().text_sm().child(file.name))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(palette.muted)
                            .child(file.parent.breadcrumbs().join(" / ")),
                    ),
            )
            .child(
                v_flex()
                    .flex_shrink_0()
                    .items_end()
                    .gap_0p5()
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .child(file.kind.label()),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(palette.faint)
                            .child(demo_human_size(file, self.appearance.read(cx).human_sizes)),
                    ),
            )
            .into_any_element()
    }
}

impl Focusable for GlobalSearch {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl Render for GlobalSearch {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&settings);
        let focus_handle = self.focus_handle(cx);
        let results = self
            .results
            .iter()
            .enumerate()
            .map(|(index, file)| self.result_row(index, file, palette, cx))
            .collect::<Vec<_>>();

        v_flex()
            .track_focus(&focus_handle)
            .key_context("GlobalSearch")
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.previous(cx)))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.next(cx)))
            .on_action(cx.listener(|this, _: &Enter, window, cx| {
                this.open_selected(window, cx);
            }))
            .w_full()
            .gap_3()
            .px_2()
            .pb_2()
            .text_color(palette.text)
            .child(
                Input::new(&self.input)
                    .w_full()
                    .cleanable(true)
                    .prefix(
                        Icon::new(IconName::Search)
                            .small()
                            .text_color(palette.muted),
                    )
                    .suffix(div().text_xs().text_color(palette.faint).child(
                        crate::settings::localized(
                            settings.language,
                            "HOME FOLDER",
                            "ДОМАШНЯЯ ПАПКА",
                        ),
                    )),
            )
            .child(
                div()
                    .px_1()
                    .text_size(px(10.))
                    .text_color(palette.faint)
                    .child(if self.query.trim().is_empty() {
                        crate::settings::localized(
                            settings.language,
                            "Names + filters: kind: ext: size: modified: tag: mode:",
                            "Имена + фильтры: kind: ext: size: modified: tag: mode:",
                        )
                        .to_string()
                    } else if settings.language == crate::settings::Language::Russian {
                        format!("Глобальные совпадения для «{}»", self.query)
                    } else {
                        format!("Global matches for “{}”", self.query)
                    }),
            )
            .child(if results.is_empty() {
                v_flex()
                    .h(px(120.))
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(
                        Icon::new(IconName::Search)
                            .large()
                            .text_color(palette.faint),
                    )
                    .child(div().text_sm().text_color(palette.muted).child(
                        crate::settings::localized(
                            settings.language,
                            "No matching files",
                            "Совпадений нет",
                        ),
                    ))
                    .into_any_element()
            } else {
                v_flex()
                    .id("global-search-results")
                    .max_h(px(336.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll_handle)
                    .children(results)
                    .vertical_scrollbar(&self.scroll_handle)
                    .into_any_element()
            })
            .child(
                h_flex()
                    .px_1()
                    .gap_2()
                    .text_size(px(10.))
                    .text_color(palette.faint)
                    .child(format!("{} shown", self.results.len()))
                    .child("·")
                    .child(crate::settings::localized(
                        settings.language,
                        "↑↓ Navigate",
                        "↑↓ Навигация",
                    ))
                    .child(crate::settings::localized(
                        settings.language,
                        "Enter Open",
                        "Enter Открыть",
                    ))
                    .child(crate::settings::localized(
                        settings.language,
                        "Esc Close",
                        "Esc Закрыть",
                    )),
            )
    }
}

fn ranked_results(query: &str, include_remote: bool) -> Vec<&'static FileEntry> {
    let candidates = searchable_entries()
        .into_iter()
        // Remote folders are searched as far as they have been browsed.
        .chain(crate::model::remote::loaded_entries().into_iter().filter(move |_| include_remote));
    rank_entries(candidates, query)
        .into_iter()
        .take(MAX_RESULTS)
        .collect()
}

pub(crate) fn close_global_search(
    explorer: &WeakEntity<FileExplorer>,
    previous_focus: &Option<FocusHandle>,
    window: &mut Window,
    cx: &mut App,
) {
    let _ = explorer.update(cx, |explorer, cx| {
        explorer.global_search = None;
        cx.notify();
    });
    if let Some(focus) = previous_focus {
        focus.focus(window, cx);
    }
}
