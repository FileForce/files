use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::{Enter, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::AnimationExt as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div, px,
};

use crate::app::{AppearanceSettings, FileExplorer};
use crate::model::{FileEntry, searchable_entries};
use crate::palette::Palette;
use crate::properties::demo_human_size;
use crate::views::files::file_icon;

const MAX_RESULTS: usize = 8;

#[derive(Clone, Debug)]
enum SearchResult {
    File(&'static FileEntry),
    Mail(files_ext_mail::MailResult),
    Track(files_ext_music::Track),
}

#[cfg(test)]
mod path_tests {
    #[test]
    fn a_pasted_path_is_offered_and_a_half_typed_one_completes() {
        let dir = std::env::temp_dir().join(format!("file-force-typed-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Documents")).unwrap();
        std::fs::create_dir_all(dir.join("Downloads")).unwrap();
        let text = dir.display().to_string();
        assert_eq!(super::typed_path(&text), vec![dir.clone()], "an existing folder");
        assert_eq!(super::typed_path(&format!("\"{text}\"")), vec![dir.clone()], "in quotes, as copied from a terminal");
        assert_eq!(super::typed_path(&format!("{text}/Do")), vec![dir.join("Documents"), dir.join("Downloads")], "half-typed: the folders it could be");
        assert_eq!(super::typed_path(&format!("{text}/")).first(), Some(&dir), "ending in /: the folder, then its folders");
        assert!(super::typed_path("invoice 2026").is_empty(), "words stay a search");
        assert!(!super::typed_path("~").is_empty(), "~ is home");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
impl SearchResult {
    fn name(&self) -> &str {
        match self {
            Self::File(file) => file.name,
            Self::Mail(mail) => &mail.summary.subject,
            Self::Track(track) => &track.title,
        }
    }
}

pub struct GlobalSearch {
    explorer: WeakEntity<FileExplorer>,
    /// The folder searched; changing it here lasts until the dialog closes.
    root: std::path::PathBuf,
    previous_focus: Option<FocusHandle>,
    appearance: Entity<AppearanceSettings>,
    include_remote: bool,
    input: Entity<InputState>,
    query: String,
    results: Vec<SearchResult>,
    /// Bumped per keystroke: only the newest search's results are shown.
    generation: u64,
    searching: bool,
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
        let root = appearance.read(cx).search_filters.global_root().unwrap_or_default();
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
                this.search(cx);
            }
        });
        let appearance_subscription = cx.observe(&appearance, |_, _, cx| cx.notify());

        // Opens at once: the index walk and the first results come from a
        // background thread.
        let first_root = root.clone();
        cx.spawn(async move |this, cx| {
            let results = cx.background_executor().spawn(async move { ranked_results(&first_root, "", include_remote) }).await;
            let _ = this.update(cx, |this, cx| {
                // Typing began meanwhile: its own search answers.
                if this.generation == 0 {
                    this.results = results;
                    this.searching = false;
                    cx.notify();
                }
            });
        })
        .detach();
        let results = Vec::new();
        Self {
            explorer,
            root,
            previous_focus,
            appearance,
            include_remote,
            input,
            query: String::new(),
            results,
            generation: 0,
            searching: true,
            selected: 0,
            scroll_handle: ScrollHandle::new(),
            _input_subscription: input_subscription,
            _appearance_subscription: appearance_subscription,
        }
    }

    /// Ranks off the UI thread after a short pause in typing, so filters that
    /// look at every file (`git:`, `size:` on folders) never freeze typing.
    fn search(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let (query, include_remote, root) = (self.query.clone(), self.include_remote, self.root.clone());
        self.searching = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(120)).await;
            if this.read_with(cx, |this, _| this.generation != generation).unwrap_or(true) {
                return;
            }
            let results = cx.background_executor().spawn(async move { ranked_results(&root, &query, include_remote) }).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.results = results;
                this.searching = false;
                this.selected = 0;
                this.scroll_handle.scroll_to_item(0);
                cx.notify();
            });
        })
        .detach();
    }

    /// Searches another folder, just this time.
    fn choose_root(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Search here".into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(root) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| {
                    this.root = root;
                    this.search(cx);
                });
            }
        })
        .detach();
    }

    /// Makes the current folder Cmd-K's default (Settings → General).
    fn save_root(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let home = dirs::home_dir();
        self.appearance.update(cx, |settings, cx| {
            settings.search_filters.global_folder = (Some(&root) != home.as_ref()).then_some(root);
            let _ = crate::settings::save(settings);
            cx.notify();
        });
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
        if let Some(result) = self.results.get(self.selected).cloned() {
            self.open_result(result, window, cx);
        }
    }

    fn open_result(&mut self, result: SearchResult, window: &mut Window, cx: &mut Context<Self>) {
        let explorer = self.explorer.clone();
        let _ = explorer.update(cx, |explorer, cx| match result {
            SearchResult::File(file) => explorer.open_global_result(file, window, cx),
            SearchResult::Mail(mail) => explorer.open_mail_result(mail, window, cx),
            SearchResult::Track(track) => explorer.play_track(track, cx),
        });
        self.close(window, cx);
    }

    fn close(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        close_global_search(&self.explorer, &self.previous_focus, window, cx);
    }

    fn result_row(&self, index: usize, result: SearchResult, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let selected = index == self.selected;
        let (icon, title, subtitle, kind, detail) = match &result {
            SearchResult::File(file) => (
                file_icon(file, palette, 20.).into_any_element(),
                file.name.to_string(),
                file.parent.breadcrumbs().join(" / "),
                file.kind.label().to_string(),
                demo_human_size(file, self.appearance.read(cx).human_sizes),
            ),
            SearchResult::Track(track) => {
                let service = match &track.source {
                    files_ext_music::Source::Local(_) => "Library",
                    files_ext_music::Source::Spotify(_) => "Spotify",
                    files_ext_music::Source::Yandex(_) => "Yandex Music",
                    files_ext_music::Source::Url(_) => "Stream",
                };
                (
                    Icon::new(IconName::Music).text_color(palette.accent).into_any_element(),
                    track.title.clone(),
                    [track.artist.as_str(), track.album.as_str()].iter().filter(|part| !part.is_empty()).copied().collect::<Vec<_>>().join(" · "),
                    service.to_string(),
                    track.duration(),
                )
            }
            SearchResult::Mail(mail) => {
                let date = mail
                    .summary
                    .date
                    .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
                    .map(|date| date.with_timezone(&chrono::Local).format("%b %d, %H:%M").to_string())
                    .unwrap_or_default();
                (
                    Icon::new(IconName::Mail).text_color(palette.accent).into_any_element(),
                    mail.summary.subject.clone(),
                    format!("{} · {} / {}", mail.summary.from, mail.account.email, mail.folder.label),
                    "Email".to_string(),
                    date,
                )
            }
        };
        h_flex()
            .id(("global-search-result", index))
            .h(px(48.))
            .flex_shrink_0()
            .px_3()
            .gap_3()
            .items_center()
            .rounded_md()
            .border_1()
            .border_color(if selected { palette.accent } else { palette.window })
            .bg(if selected { palette.selection } else { palette.window })
            .hover(|style| style.bg(palette.hover))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = index;
                this.open_result(result.clone(), window, cx);
            }))
            .child(icon)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(div().truncate().text_sm().child(title))
                    .child(div().truncate().text_size(px(10.)).text_color(palette.muted).child(subtitle)),
            )
            .child(
                v_flex()
                    .flex_shrink_0()
                    .items_end()
                    .gap_0p5()
                    .child(div().text_xs().text_color(palette.muted).child(kind))
                    .child(div().text_size(px(10.)).text_color(palette.faint).child(detail)),
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
        let is_default = settings.search_filters.global_root().as_ref() == Some(&self.root);
        let root_label = if music_query(&self.query).is_some() {
            "Music".to_string()
        } else if mail_query(&self.query).is_some() {
            "Mail".to_string()
        } else if dirs::home_dir().as_ref() == Some(&self.root) {
            crate::settings::localized(settings.language, "Home folder", "Домашняя папка").to_string()
        } else {
            self.root.file_name().map_or_else(|| self.root.display().to_string(), |name| name.to_string_lossy().into_owned())
        };
        let results = self
            .results
            .iter()
            .enumerate()
            .map(|(index, result)| self.result_row(index, result.clone(), palette, cx))
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
                    .suffix(div().text_xs().text_color(palette.faint).child(root_label.to_uppercase())),
            )
            .child(
                h_flex()
                    .px_1()
                    .gap_1()
                    .items_center()
                    .child(Icon::new(IconName::Folder).xsmall().text_color(palette.faint))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(10.)).text_color(palette.muted).child(self.root.display().to_string()))
                    .child(
                        Button::new("global-search-root")
                            .ghost()
                            .xsmall()
                            .label(crate::settings::localized(settings.language, "Change…", "Сменить…"))
                            .tooltip(crate::settings::localized(settings.language, "Search another folder this time", "Искать в другой папке в этот раз"))
                            .on_click(cx.listener(|this, _, _, cx| this.choose_root(cx))),
                    )
                    .when(!is_default, |this| {
                        this.child(
                            Button::new("global-search-root-default")
                                .ghost()
                                .xsmall()
                                .label(crate::settings::localized(settings.language, "Always search here", "Всегда искать здесь"))
                                .on_click(cx.listener(|this, _, _, cx| this.save_root(cx))),
                        )
                    }),
            )
            .child(
                div()
                    .px_1()
                    .text_size(px(10.))
                    .text_color(palette.faint)
                    .child(if self.query.trim().is_empty() {
                        // Extensions add their own, e.g. `git:main`.
                        let contributed: String = crate::model::contributed_search_keys(crate::model::SearchScope::Global)
                            .iter()
                            .map(|search| format!(" {}:", search.key))
                            .collect();
                        format!(
                            "{}{contributed}",
                            crate::settings::localized(
                                settings.language,
                                "Names + filters: kind: ext: size: modified: tag: mode:",
                                "Имена + фильтры: kind: ext: size: modified: tag: mode:",
                            )
                        )
                    } else if settings.language == crate::settings::Language::Russian {
                        format!("Глобальные совпадения для «{}»", self.query)
                    } else {
                        format!("Global matches for “{}”", self.query)
                    }),
            )
            .child(if results.is_empty() && self.searching {
                // Filters like `git:` look at every file: say it is working.
                v_flex()
                    .h(px(120.))
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .debug_selector(|| "global-search-searching".into())
                    .child(Icon::new(IconName::LoaderCircle).large().text_color(palette.accent).with_animation(
                        "global-search-spin",
                        gpui_kit::Animation::new(std::time::Duration::from_secs(1)).repeat(),
                        |icon, delta| icon.transform(gpui_kit::Transformation::rotate(gpui_kit::percentage(delta))),
                    ))
                    .child(crate::views::text_fx::shimmer(
                        "global-search-shimmer",
                        crate::settings::localized(settings.language, "Searching…", "Идёт поиск…"),
                        palette.muted,
                        palette,
                    ))
                    .child(div().text_xs().text_color(palette.faint).child(self.root.display().to_string()))
                    .into_any_element()
            } else if results.is_empty() {
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
                    .child(if self.searching { "Searching…".to_string() } else { format!("{} shown", self.results.len()) })
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

/// `music:words` when Music is on and its key is enabled for Cmd-K.
fn music_query(query: &str) -> Option<&str> {
    let (key, value) = query.split_once(':')?;
    let enabled = crate::model::contributed_search_keys(crate::model::SearchScope::Global).iter().any(|search| search.key == "music");
    (enabled && key.eq_ignore_ascii_case("music") && !value.trim().is_empty()).then_some(value)
}

fn mail_query(query: &str) -> Option<&str> {
    let (key, value) = query.split_once(':')?;
    let enabled = crate::model::contributed_search_keys(crate::model::SearchScope::Global)
        .iter()
        .any(|search| search.key == "email");
    (enabled && key.eq_ignore_ascii_case("email") && !value.trim().is_empty()).then_some(value)
}

/// A path typed or pasted into the search: `~/Music`, `/etc`, `C:\Users`,
/// with quotes or as `file://`. The folder (or file) itself when it exists;
/// else the folders in its parent starting with the last part, so a path
/// half-typed completes.
pub(crate) fn typed_path(query: &str) -> Vec<std::path::PathBuf> {
    use std::path::{Path, PathBuf};
    let text = query.trim().trim_matches(['"', '\'']);
    let looks_like_path = text.starts_with('/') || text.starts_with('~') || text.starts_with("file://") || text.starts_with("\\\\")
        || (text.len() >= 2 && text.as_bytes()[1] == b':' && text.as_bytes()[0].is_ascii_alphabetic());
    if !looks_like_path {
        return Vec::new();
    }
    let expanded: PathBuf = if let Some(rest) = text.strip_prefix('~') {
        let Some(home) = dirs::home_dir() else { return Vec::new() };
        home.join(rest.trim_start_matches(['/', '\\']))
    } else if let Some(path) = crate::desktop::launch_path(["files".to_string(), text.to_string()]).filter(|_| text.starts_with("file://")) {
        path
    } else {
        PathBuf::from(text)
    };
    if expanded.exists() && !text.ends_with(['/', '\\']) {
        return vec![expanded];
    }
    // "…/Docu" → the folders of "…/" starting with "Docu"; "…/" → its folders.
    let (parent, prefix) = if text.ends_with(['/', '\\']) { (expanded.as_path(), String::new()) } else { (expanded.parent().unwrap_or(Path::new("/")), expanded.file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default()) };
    let Ok(read) = std::fs::read_dir(parent) else { return Vec::new() };
    let mut found: Vec<PathBuf> = read
        .flatten()
        .filter(|item| item.file_name().to_string_lossy().to_lowercase().starts_with(&prefix) && !item.file_name().to_string_lossy().starts_with('.'))
        .map(|item| item.path())
        .filter(|path| path.is_dir())
        .collect();
    found.sort();
    found.truncate(8);
    // "…/" alone: the folder itself first, then what is in it.
    if prefix.is_empty() && parent.is_dir() {
        found.insert(0, parent.to_path_buf());
    }
    found
}

fn ranked_results(root: &std::path::Path, query: &str, include_remote: bool) -> Vec<SearchResult> {
    // A path: go there.
    let paths: Vec<SearchResult> = typed_path(query).iter().filter_map(|path| crate::model::entry_at(path)).map(SearchResult::File).collect();
    if !paths.is_empty() {
        return paths;
    }
    if let Some(query) = music_query(query) {
        return files_ext_music::search_everywhere(query, MAX_RESULTS).into_iter().map(SearchResult::Track).collect();
    }
    if let Some(query) = mail_query(query) {
        return files_ext_mail::global_search(query)
            .unwrap_or_default()
            .into_iter()
            .map(SearchResult::Mail)
            .take(MAX_RESULTS)
            .collect();
    }
    let started = std::time::Instant::now();
    let candidates: Vec<_> = searchable_entries(root)
        .into_iter()
        // Remote folders are searched as far as they have been browsed.
        .chain(crate::model::remote::loaded_entries().into_iter().filter(move |_| include_remote))
        .collect();
    let listed = std::time::Instant::now();
    let count = candidates.len();
    let ranked = crate::model::rank_entries_in(candidates, query, crate::model::SearchScope::Global);
    // DevTools → Search: the index walk, then matching and ranking.
    crate::tasks::record_search(crate::tasks::SearchProfile {
        scope: "Global",
        query: query.to_string(),
        candidates: count,
        results: ranked.len(),
        list: listed - started,
        matching: listed.elapsed(),
        sort: std::time::Duration::ZERO,
    });
    ranked.into_iter().map(SearchResult::File).take(MAX_RESULTS).collect()
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

#[cfg(test)]
impl GlobalSearch {
    pub(crate) fn type_for_test(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.set_value(text.to_string(), window, cx));
        // Typing emits Change; setting the value from code does not.
        if self.query != text {
            self.query = text.to_string();
            self.search(cx);
        }
    }

    pub(crate) fn results_for_test(&self) -> Vec<String> {
        self.results.iter().map(|entry| entry.name().to_string()).collect()
    }

    pub(crate) fn searching_for_test(&self) -> bool {
        self.searching
    }

    pub(crate) fn root_for_test(&self) -> std::path::PathBuf {
        self.root.clone()
    }
}
