use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle,
    Styled as _, Subscription, Window, actions, div, px,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _};

use crate::model::{Accent, ColorScheme, FileEntry, Location, Pane, ViewMode, entries, entry};
use crate::palette::{Palette, apply_theme};
use crate::views::global_search::{GlobalSearch, close_global_search};

static NEXT_EXPLORER_WINDOW: AtomicUsize = AtomicUsize::new(1);

actions!(
    file_force,
    [
        ShowExpressive,
        ShowPro,
        ShowDual,
        FocusSearch,
        ShowGlobalSearch,
        NewTab,
        CloseTab,
        ToggleTheme,
        ToggleInspector,
        ShowSettings,
        SelectPrevious,
        SelectNext,
        OpenSelected,
        FocusLeftPane,
        FocusRightPane,
        FocusNext,
        FocusPrevious,
        NewWindow,
        DeleteSelected,
        ExtendSelectionUp,
        ExtendSelectionDown,
        SelectAll,
        QuickLook,
        OpenWithApp,
        CompressSelected,
        ExtractSelected,
        CopySelectedPath,
        OpenTerminalHere,
        NewFile,
        NewFolder,
        ForkHere,
        CopyFiles,
        CutFiles,
        PasteFiles,
    ]
);

pub use crate::settings::AppSettings as AppearanceSettings;

#[derive(Clone)]
pub struct FolderPicker {
    /// Shown in the picker bar, e.g. "Extract into".
    pub title: &'static str,
    pub on_pick: std::rc::Rc<dyn Fn(std::path::PathBuf, &mut App)>,
}

#[derive(Clone)]
pub struct PaneState {
    pub location: Location,
    pub selected: Option<&'static str>,
}

#[derive(Clone)]
pub struct WorkspaceTab {
    pub id: u64,
    pub current: Location,
    pub selected: Option<&'static str>,
    pub history: Vec<Location>,
    pub history_index: usize,
    pub query: String,
    pub mode: ViewMode,
    pub left: PaneState,
    pub right: PaneState,
    pub active_pane: Pane,
}

impl WorkspaceTab {
    fn new(id: u64, location: Location) -> Self {
        Self {
            id,
            current: location,
            selected: None,
            history: vec![location],
            history_index: 0,
            query: String::new(),
            mode: ViewMode::Expressive,
            left: PaneState {
                location,
                selected: None,
            },
            right: PaneState {
                location: Location::Downloads,
                selected: None,
            },
            active_pane: Pane::Left,
        }
    }

    pub fn location(&self) -> Location {
        if self.mode == ViewMode::Dual {
            return match self.active_pane {
                Pane::Left => self.left.location,
                Pane::Right => self.right.location,
            };
        }
        self.current
    }
}

pub struct FileExplorer {
    pub mode: ViewMode,
    pub scheme: ColorScheme,
    pub accent: Accent,
    pub current: Location,
    pub selected: Option<&'static str>,
    /// Multi-selection (Expressive/Pro). Only meaningful while it contains
    /// `selected`; a plain assignment to `selected` therefore means "just that".
    pub selection: Vec<&'static str>,
    /// Where a shift-click or shift-arrow range starts.
    pub(crate) anchor: Option<&'static str>,
    pub(crate) parent_selected: bool,
    pub show_inspector: bool,
    pub left: PaneState,
    pub right: PaneState,
    pub active_pane: Pane,
    pub search: Entity<InputState>,
    /// Panel opacity and background from Settings → Appearance.
    pub look: crate::settings::LookSettings,
    pub accessibility: crate::settings::AccessibilitySettings,
    /// The window backdrop last asked of the OS (blurred when see-through).
    applied_backdrop: Option<gpui_kit::WindowBackgroundAppearance>,
    pub query: String,
    pub sort: crate::model::SortOrder,
    pub notice: Option<String>,
    pub history: Vec<Location>,
    pub history_index: usize,
    pub drop_active: bool,
    pub pending_drop: Option<(Entity<FileExplorer>, &'static FileEntry)>,
    pub appearance: Entity<AppearanceSettings>,
    pub ssh_connection: Option<crate::model::SshConnection>,
    /// The remote home folder of `ssh_connection`.
    pub(crate) ssh_home: Location,
    /// Remote folders being fetched right now.
    pub(crate) remote_loading: std::collections::HashSet<Location>,
    /// When set, this window is a folder picker; the callback receives the choice.
    pub folder_picker: Option<FolderPicker>,
    pub tabs: Vec<WorkspaceTab>,
    pub active_tab: usize,
    pub(crate) global_search: Option<Entity<GlobalSearch>>,
    pub(crate) ssh_dialog: Option<Entity<crate::views::ssh::SshDialog>>,
    pub(crate) tab_scroll: ScrollHandle,
    /// The Expressive / Pro list's scroll position.
    pub(crate) list_scroll: ScrollHandle,
    pub(crate) sidebar_scroll: ScrollHandle,
    /// An item to bring into view (after a paste, a drop or arrow keys) and
    /// where it was painted last frame.
    pub(crate) reveal: Option<&'static str>,
    pub(crate) reveal_bounds: std::rc::Rc<std::cell::Cell<Option<gpui_kit::Bounds<gpui_kit::Pixels>>>>,
    next_tab_id: u64,
    focus_handle: FocusHandle,
    _appearance_subscription: Subscription,
    /// Window activation, for extension events.
    _extension_events: Option<Subscription>,
    /// Claude Code / Codex / OpenCode sessions, newest first.
    pub(crate) agent_sessions: Vec<files_ext_agents::Session>,
    /// Footer: the event console and the notifications card.
    pub(crate) console_open: bool,
    pub(crate) notifications_open: bool,
    pub(crate) console_query: Entity<InputState>,
    /// Kinds shown (empty: all).
    pub(crate) console_kinds: std::collections::HashSet<files_sdk::log::Kind>,
    pub(crate) console_expanded: Option<u64>,
    pub(crate) journal_generation: u64,
    /// The last status message written to the journal.
    pub(crate) journaled_notice: Option<String>,
    pub(crate) low_disk_warned: bool,
    /// Toasts closed with × before they faded.
    pub(crate) hidden_toasts: std::collections::HashSet<u64>,
    pub(crate) agents_scanning: bool,
    _search_subscription: Subscription,
}

impl FileExplorer {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        appearance: Entity<AppearanceSettings>,
    ) -> Self {
        crate::host::install();
        let settings = appearance.read(cx).clone();
        let appearance_subscription = cx.observe(&appearance, |this, appearance, cx| {
            let settings = appearance.read(cx).clone();
            this.scheme = settings.scheme;
            this.accent = settings.accent;
            this.look = settings.look;
            this.accessibility = settings.accessibility;
            cx.set_reduce_motion(settings.accessibility.reduce_motion());
            crate::palette::apply_motion(cx, this.look.motion);
            cx.notify();
        });
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::settings::localized(
                    settings.language,
                    "Search · kind:image size:>10mb modified:<7d",
                    "Поиск · kind:image size:>10mb modified:<7d",
                ))
                .clean_on_escape()
        });
        let search_subscription = cx.subscribe_in(&search, window, |this, state, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = state.read(cx).value().to_string();
                this.parent_selected = false;
                if let Some(selected) = this.selected {
                    let visible = this.entries_for(this.current);
                    if !visible.iter().any(|entry| entry.id == selected) {
                        this.selected = None;
                    }
                }
                cx.notify();
            }
        });
        crate::properties::watch_folder_sizes(cx);
        let focus_handle = cx.focus_handle().tab_stop(true);
        window.focus(&focus_handle, cx);
        // "System" theme: follow macOS light/dark switches while running.
        let appearance_for_os = appearance.clone();
        cx.observe_window_appearance(window, move |_, window, cx| {
            crate::model::set_system_dark(matches!(
                window.appearance(),
                gpui_kit::WindowAppearance::Dark | gpui_kit::WindowAppearance::VibrantDark
            ));
            let settings = appearance_for_os.read(cx).clone();
            if settings.scheme == ColorScheme::System {
                crate::palette::apply_theme(cx, settings.scheme, settings.accent);
                cx.refresh_windows();
            }
        })
        .detach();
        let initial_tab = WorkspaceTab::new(0, Location::Home);
        
        

        let default_view = settings.behavior.default_view;
        let mut initial_tab = initial_tab;
        initial_tab.mode = default_view;
        Self::watch_agents(cx);
        Self::watch_journal(cx);
        Self {
            mode: default_view,
            scheme: settings.scheme,
            accent: settings.accent,
            current: Location::Home,
            selected: None,
            selection: Vec::new(),
            anchor: None,
            parent_selected: false,
            show_inspector: true,
            left: PaneState {
                location: Location::Home,
                selected: None,
            },
            right: PaneState {
                location: Location::Downloads,
                selected: None,
            },
            active_pane: Pane::Left,
            search,
            look: settings.look.clone(),
            accessibility: settings.accessibility,
            applied_backdrop: None,
            query: String::new(),
            sort: crate::model::SortOrder::default(),
            notice: None,
            history: vec![Location::Home],
            history_index: 0,
            drop_active: false,
            pending_drop: None,
            appearance,
            ssh_connection: None,
            ssh_home: Location::Home,
            remote_loading: Default::default(),
            folder_picker: None,
            tabs: vec![initial_tab],
            active_tab: 0,
            global_search: None,
            ssh_dialog: None,
            tab_scroll: ScrollHandle::new(),
            list_scroll: ScrollHandle::new(),
            sidebar_scroll: ScrollHandle::new(),
            reveal: None,
            reveal_bounds: Default::default(),
            next_tab_id: 1,
            focus_handle,
            _appearance_subscription: appearance_subscription,
            _extension_events: Some(Self::watch_extension_events(window, cx)),
            agent_sessions: Vec::new(),
            console_open: false,
            notifications_open: false,
            console_query: cx.new(|cx| InputState::new(window, cx).placeholder("Filter: text, source, output")),
            console_kinds: Default::default(),
            console_expanded: None,
            journal_generation: 0,
            journaled_notice: None,
            low_disk_warned: false,
            hidden_toasts: Default::default(),
            agents_scanning: false,
            _search_subscription: search_subscription,
        }
    }

    pub fn palette(&self) -> Palette {
        let palette = Palette::new(self.scheme, self.accent)
            .with_opacity(self.effective_opacity())
            .with_accessibility(&self.accessibility);
        if self.accessibility.high_contrast() {
            palette.with_high_contrast(self.scheme.is_dark())
        } else {
            palette
        }
    }

    /// Appearance opacity, unless Accessibility asks for solid or high-contrast panels.
    pub(crate) fn effective_opacity(&self) -> f32 {
        if self.accessibility.reduce_transparency() || self.accessibility.high_contrast() {
            1.0
        } else {
            self.look.opacity()
        }
    }

    pub(crate) fn focus_main(&self, window: &mut Window, cx: &mut App) {
        self.focus_handle.focus(window, cx);
    }

    pub fn entries_for(&self, location: Location) -> Vec<&'static FileEntry> {
        if location.is_remote() && self.ssh_connection.is_none() {
            return Vec::new();
        }
        let mut candidates = entries(location, "");
        // A folder picker only offers destinations, never files.
        if self.folder_picker.is_some() {
            candidates.retain(|candidate| candidate.is_folder());
        }
        let mut ranked = crate::model::rank_entries(candidates, &self.query);
        // Fuzzy relevance wins while typing, unless the user picked a column.
        let fuzzy = !crate::model::SearchQuery::parse(&self.query).text.is_empty();
        if !fuzzy || self.sort != crate::model::SortOrder::default() {
            crate::model::sort_entries(&mut ranked, self.sort);
        }
        ranked
    }

    pub fn sort_by(&mut self, key: crate::model::SortKey, cx: &mut Context<Self>) {
        self.sort = self.sort.toggled(key);
        cx.notify();
    }

    pub fn current_entries(&self) -> Vec<&'static FileEntry> {
        self.entries_for(self.current)
    }

    pub fn selected_entry(&self) -> Option<&'static FileEntry> {
        entry(self.selected?)
    }

    pub fn active_location(&self) -> Location {
        if self.mode == ViewMode::Dual {
            return match self.active_pane {
                Pane::Left => self.left.location,
                Pane::Right => self.right.location,
            };
        }
        self.current
    }

    pub fn active_selection(&self) -> Option<&'static str> {
        if self.mode == ViewMode::Dual {
            return match self.active_pane {
                Pane::Left => self.left.selected,
                Pane::Right => self.right.selected,
            };
        }
        self.selected
    }

    pub fn set_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
        if mode == ViewMode::Dual {
            self.left.location = self.current;
            self.left.selected = self.selected;
            if self.right.location == self.left.location {
                self.right.location = Location::Downloads;
                self.right.selected = None;
            }
            self.active_pane = Pane::Left;
        } else if self.mode == ViewMode::Dual {
            let (location, selected) = match self.active_pane {
                Pane::Left => (self.left.location, self.left.selected),
                Pane::Right => (self.right.location, self.right.selected),
            };
            self.current = location;
            self.selected = selected;
            self.history.truncate(self.history_index + 1);
            if self.history.last() != Some(&self.current) {
                self.history.push(self.current);
                self.history_index = self.history.len() - 1;
            }
        }
        self.mode = mode;
        self.notice = Some(format!("{} view", mode.label()));
        cx.notify();
    }

    pub fn set_scheme(&mut self, scheme: ColorScheme, cx: &mut Context<Self>) {
        self.scheme = scheme;
        self.appearance.update(cx, |appearance, cx| {
            appearance.scheme = scheme;
            cx.notify();
        });
        apply_theme(cx, self.scheme, self.accent);
        cx.notify();
    }


    pub fn open_new_window(&mut self, cx: &mut Context<Self>) {
        let appearance = self.appearance.clone();
        let index = NEXT_EXPLORER_WINDOW.fetch_add(1, Ordering::Relaxed);
        gpui_kit::open_window(
            crate::explorer_window_options(index),
            cx,
            move |window, cx| {
                let appearance = appearance.clone();
                cx.new(|cx| {
                    let mut explorer = FileExplorer::new(window, cx, appearance);
                    explorer.current = Location::Downloads;
                    explorer.selected = None;
                    explorer.left.location = Location::Downloads;
                    explorer.left.selected = None;
                    explorer.history = vec![Location::Downloads];
                    explorer.tabs = vec![WorkspaceTab::new(0, Location::Downloads)];
                    explorer
                })
            },
        )
        .expect("Failed to open another File Explorer window");
    }

    /// The folder the picker would return: a selected folder, else the current one.
    fn picked_folder(&self) -> Option<std::path::PathBuf> {
        self.active_selection()
            .and_then(entry)
            .filter(|file| file.is_folder())
            .and_then(|file| file.path.map(std::path::Path::to_path_buf))
            .or_else(|| self.active_location().path())
    }

    fn render_folder_picker_bar(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let title = self.folder_picker.as_ref()?.title;
        let picked = self.picked_folder();
        let label = picked.as_ref().map_or_else(
            || "Open a folder on disk".to_string(),
            |path| path.display().to_string(),
        );
        Some(
            gpui_kit::base::h_flex()
                .flex_shrink_0()
                .h(px(44.))
                .px_4()
                .gap_3()
                .items_center()
                .bg(palette.accent_soft)
                .border_b_1()
                .border_color(palette.border)
                .child(div().text_xs().text_color(palette.muted).child(title))
                .child(div().flex_1().min_w_0().text_sm().truncate().child(label))
                .child(
                    Button::new("folder-picker-select")
                        .primary()
                        .small()
                        .label("Select folder")
                        .disabled(picked.is_none())
                        .on_click(cx.listener(|this, _, window, cx| {
                            let (Some(path), Some(picker)) =
                                (this.picked_folder(), this.folder_picker.clone())
                            else {
                                return;
                            };
                            (picker.on_pick)(path, cx);
                            window.remove_window();
                        })),
                )
                .child(
                    Button::new("folder-picker-cancel")
                        .ghost()
                        .small()
                        .label("Cancel")
                        .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                )
                .into_any_element(),
        )
    }

    fn save_active_tab(&mut self) {
        let tab = &mut self.tabs[self.active_tab];
        tab.current = self.current;
        tab.selected = self.selected;
        tab.history = self.history.clone();
        tab.history_index = self.history_index;
        tab.query = self.query.clone();
        tab.mode = self.mode;
        tab.left = self.left.clone();
        tab.right = self.right.clone();
        tab.active_pane = self.active_pane;
    }

    fn restore_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab = self.tabs[self.active_tab].clone();
        self.current = tab.current;
        self.selected = tab.selected;
        self.history = tab.history;
        self.history_index = tab.history_index;
        self.query = tab.query.clone();
        self.mode = tab.mode;
        self.left = tab.left;
        self.right = tab.right;
        self.active_pane = tab.active_pane;
        self.parent_selected = false;
        self.notice = None;
        self.search.update(cx, |state, cx| {
            state.set_value(tab.query, window, cx);
        });
        self.tab_scroll.scroll_to_item(self.active_tab);
        cx.notify();
    }

    pub fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_active_tab();
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let mut tab = WorkspaceTab::new(id, Location::Home);
        tab.mode = self.appearance.read(cx).behavior.default_view;
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        self.restore_active_tab(window, cx);
    }

    pub fn select_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() || index == self.active_tab {
            return;
        }
        self.save_active_tab();
        self.active_tab = index;
        self.restore_active_tab(window, cx);
    }

    pub fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() == 1 || index >= self.tabs.len() {
            return;
        }
        if index == self.active_tab {
            self.tabs.remove(index);
            self.active_tab = index.min(self.tabs.len() - 1);
            self.restore_active_tab(window, cx);
        } else {
            self.tabs.remove(index);
            if index < self.active_tab {
                self.active_tab -= 1;
            }
            cx.notify();
        }
    }

    pub fn open_global_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.global_search.is_some() || window.has_active_dialog(cx) {
            return;
        }
        let previous_focus = window.focused(cx);
        let explorer = cx.entity().downgrade();
        let language = self.appearance.read(cx).language;
        let appearance = self.appearance.clone();
        let search = cx.new(|cx| {
            GlobalSearch::new(
                window,
                cx,
                explorer.clone(),
                previous_focus.clone(),
                appearance,
                self.ssh_connection.is_some() && self.appearance.read(cx).extensions.ssh,
            )
        });
        self.global_search = Some(search.clone());
        let initial_focus = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                let focus = search.read(cx).focus_handle(cx);
                focus.focus(window, cx);
            }
            dialog
                .title(crate::settings::localized(
                    language,
                    "Global search · home folder",
                    "Глобальный поиск · домашняя папка",
                ))
                .centered()
                .w(px(640.))
                .close_button(false)
                .overlay(true)
                .overlay_closable(true)
                .child(search.clone())
                .on_close({
                    let explorer = explorer.clone();
                    let previous_focus = previous_focus.clone();
                    move |_, window, cx| {
                        close_global_search(&explorer, &previous_focus, window, cx);
                    }
                })
        });
    }

    pub fn open_global_result(
        &mut self,
        file: &'static FileEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if file.is_folder() {
            if let Some(target) = file.target {
                self.navigate(target, window, cx);
            }
            return;
        }
        if self.mode == ViewMode::Dual {
            self.set_mode(ViewMode::Expressive, cx);
        }
        self.navigate(file.parent, window, cx);
        self.selected = Some(file.id);
        self.show_inspector = true;
        self.notice = Some(format!("Preview · {}", file.name));
        cx.notify();
    }

    pub fn navigate(&mut self, location: Location, window: &mut Window, cx: &mut Context<Self>) {
        self.parent_selected = false;
        if self.mode == ViewMode::Dual {
            let pane = match self.active_pane {
                Pane::Left => &mut self.left,
                Pane::Right => &mut self.right,
            };
            pane.location = location;
            pane.selected = None;
        } else {
            if self.current != location {
                self.history.truncate(self.history_index + 1);
                self.history.push(location);
                self.history_index = self.history.len() - 1;
            }
            self.current = location;
            self.selected = None;
        }
        self.notice = if location == Location::Trash {
            crate::model::trash_access_error()
        } else {
            None
        };
        self.clear_search(window, cx);
        if location.is_remote() {
            self.load_remote(location, false, cx);
        }
        cx.notify();
    }


    pub fn open_entry(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = entry(id) else {
            return;
        };
        if file.target.is_none() && crate::model::remote::entry_path(file).is_some() {
            self.open_remote_file(file, cx);
            return;
        }
        if file.kind == crate::model::FileKind::Archive {
            crate::archive::open_archive(file, self.appearance.clone(), cx);
            return;
        }
        if let Some(location) = file.target {
            self.navigate(location, window, cx);
        } else {
            self.open_file(file, cx);
        }
    }

    /// Opens a file in its app: the override from Settings → Default apps, or
    /// the system default.
    pub fn open_file(&mut self, file: &'static FileEntry, cx: &mut Context<Self>) {
        let Some(path) = file.path else {
            self.notice = Some(format!("{} is not on the local disk", file.name));
            cx.notify();
            return;
        };
        let overrides = self.appearance.read(cx).default_apps.clone();
        self.notice = Some(match crate::default_apps::open(path, &overrides) {
            Ok(Some(app)) => format!("Opened {} in {app}", file.name),
            Ok(None) => format!("Opened {}", file.name),
            Err(error) => format!("Open failed · {}: {error}", file.name),
        });
        cx.notify();
    }

    fn move_selection(&mut self, direction: isize, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection_with(direction, false, window, cx);
    }

    fn move_selection_with(
        &mut self,
        direction: isize,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.search.read(cx).focus_handle(cx).is_focused(window)
            || !matches!(self.mode, ViewMode::Pro | ViewMode::Dual)
        {
            return;
        }
        let location = self.active_location();
        let files = self.entries_for(location);
        let has_parent = location.parent().is_some() && self.query.trim().is_empty();
        if files.is_empty() && !has_parent {
            return;
        }
        let current = self
            .active_selection()
            .and_then(|selected| files.iter().position(|file| file.id == selected));
        let target = if self.parent_selected {
            if direction > 0 && !files.is_empty() {
                0
            } else {
                -1
            }
        } else if let Some(index) = current {
            if direction < 0 {
                if index == 0 && has_parent {
                    -1
                } else {
                    index.saturating_sub(1) as isize
                }
            } else {
                (index + 1).min(files.len() - 1) as isize
            }
        } else if direction > 0 {
            if has_parent { -1 } else { 0 }
        } else if files.is_empty() {
            -1
        } else {
            files.len() as isize - 1
        };

        if target < 0 {
            self.parent_selected = true;
            match self.mode {
                ViewMode::Dual => match self.active_pane {
                    Pane::Left => self.left.selected = None,
                    Pane::Right => self.right.selected = None,
                },
                _ => self.selected = None,
            }
            self.notice = None;
            cx.notify();
        } else if let Some(file) = files.get(target as usize) {
            let id = file.id;
            self.parent_selected = false;
            if self.mode == ViewMode::Dual {
                self.select_in_pane(self.active_pane, id, cx);
            } else if extend {
                self.select_range_to(id, cx);
            } else {
                self.select_entry(id, cx);
            }
            self.reveal = Some(id);
        }
    }

    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.read(cx).focus_handle(cx).is_focused(window)
            || !matches!(self.mode, ViewMode::Pro | ViewMode::Dual)
        {
            return;
        }
        if self.parent_selected {
            self.go_up(window, cx);
            return;
        }
        if let Some(selected) = self.active_selection() {
            self.open_entry(selected, window, cx);
        }
    }

    /// Everything selected in the active pane, in list order of selection.
    pub fn selection_ids(&self) -> Vec<&'static str> {
        if self.mode == ViewMode::Dual {
            return self.active_selection().into_iter().collect();
        }
        match self.selected {
            None => Vec::new(),
            Some(selected) if self.selection.contains(&selected) => self.selection.clone(),
            Some(selected) => vec![selected],
        }
    }

    pub fn selected_entries(&self) -> Vec<&'static FileEntry> {
        self.selection_ids().into_iter().filter_map(entry).collect()
    }

    pub fn is_selected(&self, id: &str) -> bool {
        self.selection_ids().contains(&id)
    }

    /// Mouse selection: plain click selects one, shift-click selects the range
    /// from the anchor, cmd-click (ctrl on Windows/Linux) toggles one item.
    pub fn click_entry(
        &mut self,
        id: &'static str,
        modifiers: gpui_kit::Modifiers,
        cx: &mut Context<Self>,
    ) {
        // `FILE_FORCE_DEBUG_CLICKS=1 cargo run` shows what each click carries.
        if std::env::var_os("FILE_FORCE_DEBUG_CLICKS").is_some() {
            eprintln!(
                "click {id}: mode={:?} shift={} cmd={} ctrl={} alt={} secondary={} before={:?}",
                self.mode,
                modifiers.shift,
                modifiers.platform,
                modifiers.control,
                modifiers.alt,
                modifiers.secondary(cx),
                self.selection_ids(),
            );
        }
        if self.mode == ViewMode::Dual {
            self.select_in_pane(self.active_pane, id, cx);
            return;
        }
        if modifiers.shift {
            self.select_range_to(id, cx);
        } else if modifiers.secondary(cx) {
            let mut selection = self.selection_ids();
            if let Some(index) = selection.iter().position(|selected| *selected == id) {
                selection.remove(index);
                self.selected = selection.last().copied();
            } else {
                selection.push(id);
                self.selected = Some(id);
            }
            self.selection = selection;
            self.anchor = Some(id);
            self.parent_selected = false;
            self.notice = None;
            cx.notify();
        } else {
            self.select_entry(id, cx);
        }
    }

    /// A press on a list's empty space clears the selection when the setting
    /// is on. Rows re-select themselves on click, so this only "sticks" when
    /// the press missed every row. Shift / cmd presses keep the selection.
    pub(crate) fn clear_selection_on_background(
        &mut self,
        event: &gpui_kit::MouseDownEvent,
        pane: Option<Pane>,
        cx: &mut Context<Self>,
    ) {
        if !self.appearance.read(cx).behavior.clear_selection_on_empty_click
            || event.modifiers.shift
            || event.modifiers.secondary(cx)
        {
            return;
        }
        match pane {
            Some(pane) => {
                self.active_pane = pane;
                let state = match pane {
                    Pane::Left => &mut self.left,
                    Pane::Right => &mut self.right,
                };
                state.selected = None;
            }
            None => {
                self.selected = None;
                self.selection.clear();
            }
        }
        self.parent_selected = false;
        cx.notify();
    }

    /// Right-click keeps a multi-selection that contains the item, so its
    /// context menu acts on all of them.
    pub fn select_for_context_menu(&mut self, id: &'static str, cx: &mut Context<Self>) {
        if !self.is_selected(id) {
            self.select_entry(id, cx);
        }
    }

    fn select_range_to(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let files = self.entries_for(self.current);
        let anchor = self.anchor.or(self.selected).unwrap_or(id);
        let start = files.iter().position(|file| file.id == anchor);
        let end = files.iter().position(|file| file.id == id);
        let (Some(start), Some(end)) = (start, end) else {
            self.select_entry(id, cx);
            return;
        };
        let (low, high) = (start.min(end), start.max(end));
        self.selection = files[low..=high].iter().map(|file| file.id).collect();
        self.selected = Some(id);
        self.anchor = Some(anchor);
        self.parent_selected = false;
        self.notice = None;
        cx.notify();
    }

    #[cfg(test)]
    fn select_all_for_test(&mut self, cx: &mut Context<Self>) {
        self.selection = self.entries_for(self.current).iter().map(|file| file.id).collect();
        self.selected = self.selection.last().copied();
        cx.notify();
    }

    fn select_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == ViewMode::Dual || self.search.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        let files = self.entries_for(self.current);
        self.selection = files.iter().map(|file| file.id).collect();
        self.selected = self.selection.last().copied();
        self.anchor = self.selection.first().copied();
        cx.notify();
    }

    pub fn select_entry(&mut self, id: &'static str, cx: &mut Context<Self>) {
        self.parent_selected = false;
        if self.mode == ViewMode::Dual {
            self.select_in_pane(self.active_pane, id, cx);
            return;
        }
        self.selection = vec![id];
        self.anchor = Some(id);
        self.selected = Some(id);
        self.notice = None;
        cx.notify();
    }

    pub fn select_in_pane(&mut self, pane: Pane, id: &'static str, cx: &mut Context<Self>) {
        self.parent_selected = false;
        self.active_pane = pane;
        let state = match pane {
            Pane::Left => &mut self.left,
            Pane::Right => &mut self.right,
        };
        state.selected = Some(id);
        cx.notify();
    }

    pub fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == ViewMode::Dual || self.history_index == 0 {
            return;
        }
        self.history_index -= 1;
        self.current = self.history[self.history_index];
        self.selected = None;
        self.clear_search(window, cx);
        cx.notify();
    }

    pub fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == ViewMode::Dual || self.history_index + 1 >= self.history.len() {
            return;
        }
        self.history_index += 1;
        self.current = self.history[self.history_index];
        self.selected = None;
        self.clear_search(window, cx);
        cx.notify();
    }

    pub fn go_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(parent) = self.active_location().parent() {
            self.navigate(parent, window, cx);
        }
    }

    pub fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.parent_selected = false;
        self.query.clear();
        self.search.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
    }

    pub fn copy_demo_path(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.active_selection().and_then(entry) else {
            return;
        };
        let path = file.path.map_or_else(
            || format!("{}/{}", self.active_location().label(), file.name),
            |path| path.display().to_string(),
        );
        cx.write_to_clipboard(ClipboardItem::new_string(path));
        self.notice = Some("Path copied".to_string());
        cx.notify();
    }

    fn on_show_expressive(&mut self, _: &ShowExpressive, _: &mut Window, cx: &mut Context<Self>) {
        self.set_mode(ViewMode::Expressive, cx);
    }

    fn on_show_pro(&mut self, _: &ShowPro, _: &mut Window, cx: &mut Context<Self>) {
        self.set_mode(ViewMode::Pro, cx);
    }

    fn on_show_dual(&mut self, _: &ShowDual, _: &mut Window, cx: &mut Context<Self>) {
        self.set_mode(ViewMode::Dual, cx);
    }

    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.search.read(cx).focus_handle(cx).focus(window, cx);
    }

    fn on_global_search(
        &mut self,
        _: &ShowGlobalSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_global_search(window, cx);
    }

    fn on_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        self.new_tab(window, cx);
    }

    fn on_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tab(self.active_tab, window, cx);
    }

    fn on_toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        self.set_scheme(self.scheme.toggled(), cx);
    }

    fn on_toggle_inspector(&mut self, _: &ToggleInspector, _: &mut Window, cx: &mut Context<Self>) {
        self.show_inspector = !self.show_inspector;
        cx.notify();
    }

    fn on_show_settings(&mut self, _: &ShowSettings, window: &mut Window, cx: &mut Context<Self>) {
        crate::settings_window::open_settings_dialog(self.appearance.clone(), window, cx);
    }

    fn on_select_previous(
        &mut self,
        _: &SelectPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(-1, window, cx);
    }

    fn on_select_next(&mut self, _: &SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, window, cx);
    }

    fn on_extend_up(&mut self, _: &ExtendSelectionUp, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection_with(-1, true, window, cx);
    }

    fn on_extend_down(&mut self, _: &ExtendSelectionDown, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection_with(1, true, window, cx);
    }

    fn on_select_all(&mut self, _: &SelectAll, window: &mut Window, cx: &mut Context<Self>) {
        self.select_all(window, cx);
    }

    fn on_open_selected(&mut self, _: &OpenSelected, window: &mut Window, cx: &mut Context<Self>) {
        self.open_selected(window, cx);
    }

    fn on_focus_left_pane(&mut self, _: &FocusLeftPane, _: &mut Window, cx: &mut Context<Self>) {
        self.switch_pane(Pane::Left, cx);
    }

    fn on_focus_right_pane(&mut self, _: &FocusRightPane, _: &mut Window, cx: &mut Context<Self>) {
        self.switch_pane(Pane::Right, cx);
    }

    fn switch_pane(&mut self, pane: Pane, cx: &mut Context<Self>) {
        if self.mode != ViewMode::Dual || self.active_pane == pane {
            return;
        }
        self.active_pane = pane;
        self.parent_selected = false;
        cx.notify();
    }

    fn on_focus_next(&mut self, _: &FocusNext, window: &mut Window, cx: &mut Context<Self>) {
        window.focus_next(cx);
    }

    fn on_focus_previous(
        &mut self,
        _: &FocusPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus_prev(cx);
    }

    fn on_delete_selected(&mut self, _: &DeleteSelected, window: &mut Window, cx: &mut Context<Self>) {
        // Backspace/Delete belong to the search box while it has focus.
        if self.search.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        self.open_delete_dialog(window, cx);
    }

    fn on_quick_look(&mut self, _: &QuickLook, window: &mut Window, cx: &mut Context<Self>) {
        // Space types into the search box while it has focus.
        if self.search.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        self.toggle_quick_look(window, cx);
    }

    /// Extension shortcuts do nothing while their extension is turned off.
    fn extension_enabled(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        let settings = self.appearance.read(cx).clone();
        let enabled = crate::extensions::CATALOG
            .iter()
            .position(|extension| extension.name == name)
            .is_some_and(|index| crate::licensing::active(&settings, index));
        if !enabled {
            self.notice = Some(format!("{name} extension is disabled · turn it on in Settings → Extensions"));
            cx.notify();
        }
        enabled
    }

    fn on_open_with_app(&mut self, _: &OpenWithApp, window: &mut Window, cx: &mut Context<Self>) {
        if self.extension_enabled("Open With", cx) {
            self.run_demo_command(crate::commands::DemoCommand::OpenWithChooser, window, cx);
        }
    }

    fn on_compress_selected(&mut self, _: &CompressSelected, window: &mut Window, cx: &mut Context<Self>) {
        if self.extension_enabled("Archives", cx) {
            self.run_demo_command(crate::commands::DemoCommand::Compress, window, cx);
        }
    }

    fn on_extract_selected(&mut self, _: &ExtractSelected, window: &mut Window, cx: &mut Context<Self>) {
        if self.extension_enabled("Archives", cx) {
            self.run_demo_command(crate::commands::DemoCommand::Extract, window, cx);
        }
    }

    fn on_copy_selected_path(&mut self, _: &CopySelectedPath, window: &mut Window, cx: &mut Context<Self>) {
        if self.extension_enabled("Copy", cx) {
            self.run_demo_command(crate::commands::DemoCommand::CopyPath, window, cx);
        }
    }

    fn on_open_terminal_here(&mut self, _: &OpenTerminalHere, _: &mut Window, cx: &mut Context<Self>) {
        if self.extension_enabled("Terminal", cx) {
            let location = self.active_location();
            self.run_folder_command(crate::commands::FolderCommand::OpenTerminal, location, cx);
        }
    }

    fn on_new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        let location = self.active_location();
        self.open_create_dialog(location, false, window, cx);
    }

    fn on_new_folder(&mut self, _: &NewFolder, window: &mut Window, cx: &mut Context<Self>) {
        let location = self.active_location();
        self.open_create_dialog(location, true, window, cx);
    }

    /// Forks the selected folder, or the current one when no folder is selected.
    fn on_fork_here(&mut self, _: &ForkHere, window: &mut Window, cx: &mut Context<Self>) {
        if !self.extension_enabled("Forks", cx) {
            return;
        }
        let target = self
            .selected_entry_any()
            .filter(|entry| entry.is_folder() && entry.path.is_some())
            .and_then(|entry| entry.target)
            .unwrap_or_else(|| self.active_location());
        self.fork_folder(target, window, cx);
    }

    /// Changes sidebar settings and saves them right away (they are layout,
    /// not something to confirm with Save).
    pub(crate) fn update_sidebar(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut crate::settings::SidebarSettings)) {
        self.appearance.update(cx, |settings, cx| {
            change(&mut settings.sidebar);
            let _ = crate::settings::save(settings);
            cx.notify();
        });
        cx.notify();
    }

    pub(crate) fn section_collapsed(&self, key: &str, cx: &App) -> bool {
        self.appearance.read(cx).sidebar.collapsed.iter().any(|folded| folded == key)
    }

    pub(crate) fn toggle_section(&mut self, key: &'static str, cx: &mut Context<Self>) {
        self.update_sidebar(cx, |sidebar| {
            if let Some(index) = sidebar.collapsed.iter().position(|folded| folded == key) {
                sidebar.collapsed.remove(index);
            } else {
                sidebar.collapsed.push(key.to_string());
            }
        });
    }

    pub(crate) fn is_favorite(&self, path: &std::path::Path, cx: &App) -> bool {
        self.appearance.read(cx).sidebar.favorites.iter().any(|favorite| favorite == path)
    }

    pub(crate) fn add_favorite(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
        self.update_sidebar(cx, |sidebar| {
            if !sidebar.favorites.contains(&path) {
                sidebar.favorites.push(path);
            }
        });
        self.notice = Some(format!("Added {name} to Favorites"));
    }

    pub(crate) fn remove_favorite(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        self.update_sidebar(cx, |sidebar| sidebar.favorites.retain(|favorite| favorite != path));
        self.notice = Some("Removed from Favorites".to_string());
    }

    fn selected_paths(&self) -> Vec<std::path::PathBuf> {
        self.selected_entries()
            .into_iter()
            .filter_map(|entry| entry.path.map(std::path::Path::to_path_buf))
            .collect()
    }

    pub(crate) fn put_on_clipboard(&mut self, cut: bool, cx: &mut Context<Self>) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            self.notice = Some("Select files to copy".to_string());
            cx.notify();
            return;
        }
        // Other apps get the paths as text; this app also keeps the files.
        let text = paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        let count = paths.len();
        crate::transfer::set_clipboard(paths, cut);
        let what = if count == 1 { "1 item".to_string() } else { format!("{count} items") };
        self.notice = Some(if cut {
            format!("Cut {what} · paste with ⌘V to move")
        } else {
            format!("Copied {what} · paste with ⌘V")
        });
        cx.notify();
    }

    fn on_copy_files(&mut self, _: &CopyFiles, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        self.put_on_clipboard(false, cx);
    }

    fn on_cut_files(&mut self, _: &CutFiles, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        self.put_on_clipboard(true, cx);
    }

    fn on_paste_files(&mut self, _: &PasteFiles, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        if self.active_location().is_remote() {
            let (paths, _) = crate::transfer::clipboard_contents();
            let location = self.active_location();
            self.upload_remote(paths, location, cx);
            return;
        }
        let Some(target) = self.active_location().path() else {
            self.notice = Some(format!("Cannot paste into {}", self.active_location().label()));
            cx.notify();
            return;
        };
        let (mut paths, cut) = crate::transfer::clipboard_contents();
        if paths.is_empty() {
            // Paths copied as text elsewhere (one per line) paste as files too.
            paths = cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .map(|text| {
                    text.lines()
                        .map(|line| std::path::PathBuf::from(line.trim()))
                        .filter(|path| path.is_absolute() && path.exists())
                        .collect()
                })
                .unwrap_or_default();
        }
        if paths.is_empty() {
            self.notice = Some("Nothing to paste".to_string());
            cx.notify();
            return;
        }
        if cut {
            crate::transfer::clear_clipboard();
        }
        let operation = if cut { crate::transfer::Operation::Move } else { crate::transfer::Operation::Copy };
        self.run_transfer(paths, target, operation, window, cx);
    }

    fn on_about(&mut self, _: &crate::menus::About, window: &mut Window, cx: &mut Context<Self>) {
        self.open_about_dialog(window, cx);
    }

    fn on_check_for_updates(&mut self, _: &crate::menus::CheckForUpdates, window: &mut Window, cx: &mut Context<Self>) {
        self.open_update_dialog(window, cx);
    }

    fn on_new_window(&mut self, _: &NewWindow, _: &mut Window, cx: &mut Context<Self>) {
        self.open_new_window(cx);
    }
}

impl FileExplorer {
    /// Scrolls the list so `reveal` is visible: centred when it was out of view,
    /// untouched when already visible. Bounds come from the previous frame.
    fn apply_reveal(&mut self, window: &mut Window) {
        if self.reveal.is_none() {
            return;
        }
        let Some(item) = self.reveal_bounds.take() else {
            // Not painted yet: try again on the next frame.
            window.request_animation_frame();
            return;
        };
        self.reveal = None;
        let view = self.list_scroll.bounds();
        if view.size.height <= px(0.) {
            return;
        }
        let above = item.top() < view.top();
        let below = item.bottom() > view.bottom();
        if above || below {
            let offset = self.list_scroll.offset();
            let shift = item.center().y - view.center().y;
            let max = self.list_scroll.max_offset();
            let y = (offset.y - shift).min(px(0.)).max(-max.y);
            self.list_scroll.set_offset(gpui_kit::point(offset.x, y));
            window.refresh();
        }
    }

    /// Records where the item to reveal was painted.
    pub(crate) fn reveal_marker(&self, id: &str) -> Option<AnyElement> {
        (self.reveal == Some(id)).then(|| {
            let cell = self.reveal_bounds.clone();
            gpui_kit::canvas(move |bounds, _, _| cell.set(Some(bounds)), |_, _, _, _| {})
                .absolute()
                .inset_0()
                .into_any_element()
        })
    }
}

impl Render for FileExplorer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.apply_reveal(window);
        let palette = self.palette();
        // The desktop shows (blurred) only when there is no own background.
        // Text size: every rem-based size in the window scales with this.
        window.set_rem_size(px(16. * self.accessibility.text_scale.factor()));
        let backdrop = if self.effective_opacity() < 1. && self.look.background == crate::settings::Background::Default {
            gpui_kit::WindowBackgroundAppearance::Blurred
        } else {
            gpui_kit::WindowBackgroundAppearance::Opaque
        };
        if self.applied_backdrop != Some(backdrop) {
            window.set_background_appearance(backdrop);
            self.applied_backdrop = Some(backdrop);
        }
        // GPUI Kit's Root and title bar paint the theme background under us.
        crate::palette::apply_chrome_opacity(cx, self.effective_opacity(), self.scheme, self.accent);
        let backdrop_color = match &self.look.background {
            crate::settings::Background::Color { hex } => crate::settings::parse_hex(hex),
            _ => None,
        };
        let backdrop_image = match &self.look.background {
            crate::settings::Background::Image { path } if path.is_file() => Some(path.clone()),
            _ => None,
        };
        let inspector_visible = self.show_inspector && self.mode != ViewMode::Dual;

        div()
            .track_focus(&self.focus_handle)
            .key_context("FileExplorer")
            .on_action(cx.listener(Self::on_show_expressive))
            .on_action(cx.listener(Self::on_show_pro))
            .on_action(cx.listener(Self::on_show_dual))
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_global_search))
            .on_action(cx.listener(Self::on_new_tab))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_toggle_theme))
            .on_action(cx.listener(Self::on_toggle_inspector))
            .on_action(cx.listener(Self::on_show_settings))
            .on_action(cx.listener(Self::on_select_previous))
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_open_selected))
            .on_action(cx.listener(Self::on_focus_left_pane))
            .on_action(cx.listener(Self::on_focus_right_pane))
            .on_action(cx.listener(Self::on_focus_next))
            .on_action(cx.listener(Self::on_focus_previous))
            .on_action(cx.listener(Self::on_new_window))
            .on_action(cx.listener(Self::on_delete_selected))
            .on_action(cx.listener(Self::on_extend_up))
            .on_action(cx.listener(Self::on_extend_down))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_quick_look))
            .on_action(cx.listener(Self::on_open_with_app))
            .on_action(cx.listener(Self::on_compress_selected))
            .on_action(cx.listener(Self::on_extract_selected))
            .on_action(cx.listener(Self::on_copy_selected_path))
            .on_action(cx.listener(Self::on_open_terminal_here))
            .on_action(cx.listener(Self::on_new_file))
            .on_action(cx.listener(Self::on_new_folder))
            .on_action(cx.listener(Self::on_fork_here))
            .on_action(cx.listener(Self::on_about))
            .on_action(cx.listener(Self::on_copy_files))
            .on_action(cx.listener(Self::on_cut_files))
            .on_action(cx.listener(Self::on_paste_files))
            .on_action(cx.listener(Self::on_check_for_updates))
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if window.focused(cx).is_none() {
                        this.focus_main(window, cx);
                    }
                }),
            )
            // Mouse side buttons walk the history, like in Finder.
            .on_mouse_down(
                gpui_kit::MouseButton::Navigate(gpui_kit::NavigationDirection::Back),
                cx.listener(|this, _, window, cx| this.go_back(window, cx)),
            )
            .on_mouse_down(
                gpui_kit::MouseButton::Navigate(gpui_kit::NavigationDirection::Forward),
                cx.listener(|this, _, window, cx| this.go_forward(window, cx)),
            )
            .on_mouse_up_out(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, window, cx| this.complete_pending_drop(window, cx)),
            )
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .focus_visible(|style| style.border_color(palette.accent))
            .relative()
            .bg(backdrop_color.unwrap_or(palette.window))
            .text_color(palette.text)
            .children(backdrop_image.map(|path| {
                div()
                    .absolute()
                    .inset_0()
                    .child(gpui_kit::StyledImage::object_fit(gpui_kit::img(path).size_full(), gpui_kit::ObjectFit::Cover))
            }))
            .child(self.render_title_bar(palette, cx))
            .children(self.render_toasts(palette, cx))
            .children(self.render_folder_picker_bar(palette, cx))
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("workspace-panels")
                        .child(
                            resizable_panel()
                                .size(px(218.))
                                .size_range(px(184.)..px(280.))
                                .fixed()
                                .child(self.render_sidebar(palette, cx)),
                        )
                        .child(
                            resizable_panel().child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .size_full()
                                    .min_w_0()
                                    .min_h_0()
                                    .child(self.render_tabs(palette, cx))
                                    .child(self.render_toolbar(palette, cx))
                                    .child(self.render_drop_area(palette, cx)),
                            ),
                        )
                        .child(
                            resizable_panel()
                                .visible(inspector_visible)
                                .size(px(292.))
                                .size_range(px(252.)..px(380.))
                                .fixed()
                                .child(self.render_inspector(palette, cx)),
                        ),
                ),
            )
            .children(self.render_console(palette, cx))
            .children(self.render_notifications(palette, cx))
            .child(self.render_status_bar(palette, cx))
    }
}

/// Opens an explorer window whose only job is choosing a destination folder.
pub fn open_folder_picker(
    start: std::path::PathBuf,
    picker: FolderPicker,
    appearance: Entity<AppearanceSettings>,
    cx: &mut App,
) {
    let index = NEXT_EXPLORER_WINDOW.fetch_add(1, Ordering::Relaxed);
    let mut options = crate::explorer_window_options(index);
    options.titlebar = Some(gpui_kit::TitlebarOptions {
        title: Some("Choose destination folder".into()),
        ..Default::default()
    });
    let opened = gpui_kit::open_window(options, cx, move |window, cx| {
        let appearance = appearance.clone();
        let picker = picker.clone();
        let start = start.clone();
        cx.new(|cx| {
            let mut explorer = FileExplorer::new(window, cx, appearance);
            let location = Location::from_path(&start);
            explorer.current = location;
            explorer.left.location = location;
            explorer.history = vec![location];
            explorer.tabs = vec![WorkspaceTab::new(0, location)];
            explorer.mode = ViewMode::Pro;
            explorer.show_inspector = false;
            explorer.folder_picker = Some(picker);
            explorer
        })
    });
    if let Err(error) = opened {
        eprintln!("Failed to open folder picker: {error}");
    }
}

/// Binds every shortcut with its defaults; user overrides are applied on top
/// by `crate::keymap::apply` once settings are loaded.
pub fn register_keys(cx: &mut App) {
    let none = std::collections::BTreeMap::new();
    crate::keymap::apply(cx, &none, &none);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ViewMode;
    use gpui_kit::{TestAppContext, VisualTestContext, point, px};

    /// A real folder with three files; returns the folder and the middle file's id.
    fn fixture_dir() -> (Location, &'static str) {
        let dir = std::env::temp_dir().join(format!(
            "file-force-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let nested = dir.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(nested.join(name), name).unwrap();
        }
        let location = Location::from_path(&nested);
        let middle = crate::model::entries(location, "")[1].id;
        (location, middle)
    }

    fn pro_window() -> (Entity<FileExplorer>, &'static mut VisualTestContext) {
        let cx = Box::leak(Box::new(TestAppContext::single()));
        cx.update(|cx| {
            gpui_kit::init(cx);
            register_keys(cx);
        });
        let appearance = cx.update(|cx| cx.new(|_| AppearanceSettings::default()));
        let (view, cx) =
            cx.add_window_view(move |window, cx| FileExplorer::new(window, cx, appearance));
        let (location, middle) = fixture_dir();
        view.update(cx, |this, cx| {
            this.current = location;
            this.history = vec![location];
            this.left.location = location;
            this.set_mode(ViewMode::Pro, cx);
            this.selected = Some(middle);
        });
        cx.run_until_parked();
        (view, cx)
    }

    #[test]
    fn pro_up_down_change_selection_and_enter_opens() {
        let (view, cx) = pro_window();
        let middle = cx.read(|app| view.read(app).selected);
        cx.simulate_keystrokes("down");
        assert_ne!(
            cx.read(|app| view.read(app).selected),
            middle
        );
        cx.simulate_keystrokes("up");
        assert_eq!(
            cx.read(|app| view.read(app).selected),
            middle
        );

        // Left/Right must not change the selection in Pro.
        cx.simulate_keystrokes("left right");
        assert_eq!(
            cx.read(|app| view.read(app).selected),
            middle
        );

        // Up from the first row selects the ".." parent row, Enter navigates up.
        let location = cx.read(|app| view.read(app).current);
        let parent = location.parent().expect("test location has a parent");
        for _ in 0..16 {
            if cx.read(|app| view.read(app).parent_selected) {
                break;
            }
            cx.simulate_keystrokes("up");
        }
        assert!(cx.read(|app| view.read(app).parent_selected));
        cx.simulate_keystrokes("enter");
        assert_eq!(cx.read(|app| view.read(app).current), parent);
    }

    fn explorer_set_right(
        view: &Entity<FileExplorer>,
        cx: &mut VisualTestContext,
        selected: Option<&'static str>,
    ) {
        view.update(cx, |this, cx| {
            this.right.selected = selected;
            this.parent_selected = false;
            cx.notify();
        });
    }

    #[test]
    fn dual_arrows_move_selection_in_active_pane_and_left_right_switch_pane() {
        let (view, cx) = pro_window();
        view.update(cx, |this, cx| this.set_mode(ViewMode::Dual, cx));
        cx.run_until_parked();

        // Left pane is active initially.
        let before = cx.read(|app| view.read(app).left.selected);
        cx.simulate_keystrokes("down");
        let after = cx.read(|app| view.read(app).left.selected);
        assert_ne!(before, after, "down did not move selection in active pane");

        // Right switches to the right pane; arrows then move the right selection.
        cx.simulate_keystrokes("right");
        assert_eq!(
            cx.read(|app| view.read(app).active_pane),
            crate::model::Pane::Right
        );
        explorer_set_right(&view, cx, None);
        cx.simulate_keystrokes("down");
        let right_state = cx.read(|app| {
            let this = view.read(app);
            (this.right.selected, this.parent_selected)
        });
        assert!(
            right_state.0.is_some() || right_state.1,
            "down did not move right pane selection"
        );

        // Left switches back.
        cx.simulate_keystrokes("left");
        assert_eq!(
            cx.read(|app| view.read(app).active_pane),
            crate::model::Pane::Left
        );
    }

    #[test]
    fn arrows_work_after_mouse_click() {
        let (view, cx) = pro_window();
        cx.simulate_click(point(px(700.), px(400.)), Default::default());
        // The click may land on empty space and clear the selection; the ".."
        // row then counts as a selection too.
        let state = |cx: &mut VisualTestContext| {
            cx.read(|app| (view.read(app).selected, view.read(app).parent_selected))
        };
        let after_click = state(cx);
        let focused = cx.update(|window, cx| window.focused(cx).is_some());
        assert!(focused, "click left the window unfocused");
        cx.simulate_keystrokes("down");
        assert_ne!(state(cx), after_click, "down after click did not change selection");
    }

    #[test]
    fn arrows_do_not_steal_search_input() {
        let (view, cx) = pro_window();
        cx.update(|window, cx| {
            let focus = view.read(cx).search.read(cx).focus_handle(cx);
            focus.focus(window, cx);
        });
        let before = cx.read(|app| view.read(app).selected);
        cx.simulate_keystrokes("down up");
        assert_eq!(
            cx.read(|app| view.read(app).selected),
            before,
            "arrows moved list selection while search input was focused"
        );
    }

    #[test]
    fn pro_list_scrolls_in_large_folder() {
        let dir = std::env::temp_dir().join(format!("file-force-scroll-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for index in 0..300 {
            std::fs::write(dir.join(format!("file-{index:03}.txt")), "x").unwrap();
        }
        let (view, cx) = pro_window();
        let location = Location::from_path(&dir);
        view.update(cx, |this, cx| {
            this.current = location;
            this.selected = None;
            cx.notify();
        });
        cx.run_until_parked();
        let bounds = |cx: &mut VisualTestContext| {
            cx.debug_bounds("pro-scroll-probe")
                .expect("probe row rendered")
        };
        let before = bounds(cx);
        cx.simulate_event(gpui_kit::ScrollEvent {
            position: point(px(700.), px(400.)),
            delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-2000.))),
            ..Default::default()
        });
        cx.run_until_parked();
        let after = bounds(cx);
        assert!(
            after.origin.y < before.origin.y,
            "list did not scroll: {before:?} -> {after:?}"
        );
    }

    #[test]
    fn folder_picker_lists_only_folders() {
        let dir = std::env::temp_dir().join(format!("file-force-picker-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        std::fs::write(dir.join("file.txt"), "x").unwrap();
        let (view, cx) = pro_window();
        let location = Location::from_path(&dir);
        let names = |view: &Entity<FileExplorer>, cx: &mut VisualTestContext| {
            cx.read(|app| {
                view.read(app)
                    .entries_for(location)
                    .iter()
                    .map(|entry| entry.name)
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(names(&view, cx), ["inner", "file.txt"]);
        view.update(cx, |this, _| {
            this.folder_picker = Some(FolderPicker {
                title: "Test",
                on_pick: std::rc::Rc::new(|_, _| {}),
            });
        });
        assert_eq!(names(&view, cx), ["inner"]);
    }

    /// A real (dialog-capable) explorer window showing `dir` in Pro mode.
    fn rooted_window(dir: &std::path::Path) -> (Entity<FileExplorer>, &'static mut VisualTestContext) {
        let cx = Box::leak(Box::new(TestAppContext::single()));
        cx.update(|cx| {
            gpui_kit::init(cx);
            register_keys(cx);
        });
        let appearance = cx.update(|cx| {
            cx.new(|_| {
                let mut settings = AppearanceSettings::default();
                // Permanent delete keeps tests away from the user's real Trash.
                settings.deletion.use_trash = false;
                settings
            })
        });
        let (window, view) = cx
            .update(|cx| {
                gpui_kit::open_window(crate::explorer_window_options(0), cx, move |window, cx| {
                    cx.new(|cx| FileExplorer::new(window, cx, appearance.clone()))
                })
            })
            .unwrap();
        let cx = VisualTestContext::from_window(window, cx).into_mut();
        let location = Location::from_path(dir);
        view.update(cx, |this, cx| {
            this.current = location;
            this.set_mode(ViewMode::Pro, cx);
            cx.notify();
        });
        cx.run_until_parked();
        (view, cx)
    }

    #[test]
    fn create_dialog_makes_file_and_folder_and_selects_it() {
        let dir = std::env::temp_dir().join(format!("file-force-create-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (view, cx) = rooted_window(&dir);
        let location = Location::from_path(&dir);

        // A file keeps its suggested ".txt": only the stem is selected.
        for (folder, name) in [(false, "draft.txt"), (true, "assets")] {
            view.update_in(cx, |this, window, cx| this.open_create_dialog(location, folder, window, cx));
            cx.run_until_parked();
            assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
            // The suggested name is pre-selected: typing replaces it directly.
            let typed = if folder { name } else { name.trim_end_matches(".txt") };
            cx.simulate_input(typed);
            cx.simulate_keystrokes("enter");
            cx.run_until_parked();
            let path = dir.join(name);
            assert_eq!(path.is_dir(), folder, "{name} has wrong type");
            assert!(path.exists(), "{name} was not created");
            assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
            let selected = cx.read(|app| view.read(app).selected);
            assert_eq!(selected.and_then(crate::model::entry).map(|entry| entry.name), Some(name));
        }
    }

    #[test]
    fn enter_in_delete_dialog_deletes_instead_of_cancelling() {
        let dir = std::env::temp_dir().join(format!("file-force-enter-delete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("doomed.txt");
        std::fs::write(&file, "bye").unwrap();

        let cx = Box::leak(Box::new(TestAppContext::single()));
        cx.update(|cx| {
            gpui_kit::init(cx);
            register_keys(cx);
        });
        // Permanent delete keeps the test away from the user's real Trash.
        let appearance = cx.update(|cx| {
            cx.new(|_| {
                let mut settings = AppearanceSettings::default();
                settings.deletion.use_trash = false;
                settings
            })
        });
        let (window, view) = cx
            .update(|cx| {
                gpui_kit::open_window(crate::explorer_window_options(0), cx, move |window, cx| {
                    cx.new(|cx| FileExplorer::new(window, cx, appearance.clone()))
                })
            })
            .unwrap();
        let cx = VisualTestContext::from_window(window, cx).into_mut();
        let location = Location::from_path(&dir);
        let id = crate::model::entries(location, "")[0].id;
        view.update(cx, |this, cx| {
            this.current = location;
            this.set_mode(ViewMode::Pro, cx);
            this.selected = Some(id);
            cx.notify();
        });
        cx.run_until_parked();
        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-backspace" } else { "delete" });
        cx.run_until_parked();
        assert!(cx.update(|window, cx| window.has_active_dialog(cx)), "dialog did not open");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(!file.exists(), "enter did not delete the file");
        assert!(!cx.update(|window, cx| window.has_active_dialog(cx)), "dialog stayed open");
    }

    fn secondary() -> gpui_kit::Modifiers {
        if cfg!(target_os = "macos") {
            gpui_kit::Modifiers { platform: true, ..Default::default() }
        } else {
            gpui_kit::Modifiers { control: true, ..Default::default() }
        }
    }

    #[test]
    fn shift_selects_ranges_and_cmd_toggles_items() {
        let (view, cx) = pro_window();
        let ids: Vec<_> = cx.read(|app| view.read(app).current_entries().iter().map(|entry| entry.id).collect());
        let [a, b, c] = ids[..] else { panic!("fixture has three files") };
        let selection = |cx: &mut VisualTestContext| cx.read(|app| view.read(app).selection_ids());
        let shift = gpui_kit::Modifiers { shift: true, ..Default::default() };

        view.update(cx, |this, cx| this.click_entry(a, Default::default(), cx));
        assert_eq!(selection(cx), [a]);
        view.update(cx, |this, cx| this.click_entry(c, shift, cx));
        assert_eq!(selection(cx), [a, b, c], "shift-click selects the range");
        view.update(cx, |this, cx| this.click_entry(b, secondary(), cx));
        assert_eq!(selection(cx), [a, c], "cmd-click removes one");
        view.update(cx, |this, cx| this.click_entry(b, secondary(), cx));
        assert_eq!(selection(cx), [a, c, b], "cmd-click adds one");
        view.update(cx, |this, cx| this.click_entry(b, Default::default(), cx));
        assert_eq!(selection(cx), [b], "plain click resets");

        // Shift-click works from the anchor in both directions.
        view.update(cx, |this, cx| this.click_entry(a, shift, cx));
        assert_eq!(selection(cx), [a, b]);

        cx.simulate_keystrokes("shift-down shift-down");
        assert_eq!(selection(cx), [b, c], "shift-down extends from the anchor");

        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-a" } else { "ctrl-a" });
        assert_eq!(selection(cx), [a, b, c]);

        // Right-click inside the selection keeps it for the context menu.
        view.update(cx, |this, cx| this.select_for_context_menu(b, cx));
        assert_eq!(selection(cx).len(), 3);
    }

    #[test]
    fn deleting_a_multi_selection_removes_every_item() {
        let dir = std::env::temp_dir().join(format!("file-force-multi-delete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["a.txt", "b.txt", "keep.txt"] {
            std::fs::write(dir.join(name), name).unwrap();
        }
        let (view, cx) = rooted_window(&dir);
        let ids: Vec<_> = cx.read(|app| view.read(app).current_entries().iter().map(|entry| entry.id).collect());
        view.update(cx, |this, cx| {
            this.click_entry(ids[0], Default::default(), cx);
            this.click_entry(ids[1], secondary(), cx);
        });
        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-backspace" } else { "delete" });
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(!dir.join("a.txt").exists());
        assert!(!dir.join("b.txt").exists());
        assert!(dir.join("keep.txt").exists(), "unselected file was deleted");
        assert!(cx.read(|app| view.read(app).selection_ids()).is_empty());
    }

    #[test]
    fn one_protected_item_blocks_the_whole_batch() {
        let dir = std::env::temp_dir().join(format!("file-force-protected-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("free.txt"), "x").unwrap();
        std::fs::write(dir.join("guarded.txt"), "x").unwrap();
        let (view, cx) = rooted_window(&dir);
        view.update(cx, |this, cx| {
            let guarded = dir.join("guarded.txt").to_string_lossy().into_owned();
            this.appearance.update(cx, |settings, _| settings.deletion.protected.push(guarded));
            this.select_all_for_test(cx);
        });
        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-backspace" } else { "delete" });
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(dir.join("free.txt").exists(), "batch with a protected item must delete nothing");
        assert!(dir.join("guarded.txt").exists());
    }

    #[test]
    fn real_mouse_clicks_with_modifiers_build_a_multi_selection() {
        let (view, cx) = pro_window();
        let ids: Vec<_> = cx.read(|app| view.read(app).current_entries().iter().map(|entry| entry.id).collect());
        let names = ["a.txt", "b.txt", "c.txt"];
        let center = |cx: &mut VisualTestContext, name: &str| {
            cx.debug_bounds(Box::leak(format!("pro-row-{name}").into_boxed_str()))
                .expect("row rendered")
                .center()
        };
        let selection = |cx: &mut VisualTestContext| cx.read(|app| view.read(app).selection_ids());

        let a = center(cx, names[0]);
        cx.simulate_click(a, Default::default());
        assert_eq!(selection(cx), [ids[0]]);

        let c = center(cx, names[2]);
        cx.simulate_click(c, gpui_kit::Modifiers { shift: true, ..Default::default() });
        assert_eq!(selection(cx), ids, "shift-click");

        let b = center(cx, names[1]);
        cx.simulate_click(b, secondary());
        assert_eq!(selection(cx), [ids[0], ids[2]], "cmd-click");
    }

    #[test]
    fn expressive_cards_support_shift_and_cmd_clicks() {
        let (view, cx) = pro_window();
        view.update(cx, |this, cx| this.set_mode(ViewMode::Expressive, cx));
        cx.run_until_parked();
        let ids: Vec<_> = cx.read(|app| view.read(app).current_entries().iter().map(|entry| entry.id).collect());
        let center = |cx: &mut VisualTestContext, name: &str| {
            cx.debug_bounds(Box::leak(format!("card-{name}").into_boxed_str()))
                .expect("card rendered")
                .center()
        };
        let selection = |cx: &mut VisualTestContext| cx.read(|app| view.read(app).selection_ids());
        let a = center(cx, "a.txt");
        cx.simulate_click(a, Default::default());
        let c = center(cx, "c.txt");
        cx.simulate_click(c, gpui_kit::Modifiers { shift: true, ..Default::default() });
        assert_eq!(selection(cx), ids, "shift-click on cards");
        let b = center(cx, "b.txt");
        cx.simulate_click(b, secondary());
        assert_eq!(selection(cx), [ids[0], ids[2]], "cmd-click on cards");
    }

    #[test]
    fn settings_open_as_a_modal_that_enter_does_not_close() {
        let dir = std::env::temp_dir();
        let (_view, cx) = rooted_window(&dir);
        let open = |cx: &mut VisualTestContext| cx.update(|window, cx| window.has_active_dialog(cx));
        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-," } else { "ctrl-," });
        cx.run_until_parked();
        assert!(open(cx), "settings did not open as a dialog");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(open(cx), "enter dismissed settings");
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(!open(cx), "escape did not close settings");
    }

    #[test]
    fn protected_paths_list_scrolls_with_the_wheel() {
        let (_view, cx) = rooted_window(&std::env::temp_dir());
        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-," } else { "ctrl-," });
        cx.run_until_parked();
        cx.simulate_keystrokes("down down down down");
        cx.run_until_parked();
        let before = cx.debug_bounds("protected-list-content").expect("deletion tab shows the list");
        let over_list = point(before.origin.x + px(40.), before.origin.y + px(60.));
        cx.simulate_event(gpui_kit::ScrollEvent {
            position: over_list,
            delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-200.))),
            ..Default::default()
        });
        cx.run_until_parked();
        let after = cx.debug_bounds("protected-list-content").unwrap();
        assert!(after.origin.y < before.origin.y, "list did not scroll: {before:?} -> {after:?}");
    }

    fn double_click(cx: &mut VisualTestContext, selector: &'static str) {
        let position = cx.debug_bounds(selector).expect("element rendered").center();
        for click_count in [1, 2] {
            cx.simulate_event(gpui_kit::MouseDownEvent {
                position,
                button: gpui_kit::MouseButton::Left,
                click_count,
                ..Default::default()
            });
            cx.simulate_event(gpui_kit::MouseUpEvent {
                position,
                button: gpui_kit::MouseButton::Left,
                click_count,
                ..Default::default()
            });
        }
        cx.run_until_parked();
    }

    /// Forks are made in the background; wait for it and return the fork folder.
    fn fork_and_wait(view: &Entity<FileExplorer>, cx: &mut VisualTestContext, folder: &std::path::Path) -> std::path::PathBuf {
        let location = Location::from_path(folder);
        view.update_in(cx, |this, window, cx| this.fork_folder(location, window, cx));
        for _ in 0..200 {
            cx.run_until_parked();
            if let Some(path) = cx.read(|app| view.read(app).current.path())
                && crate::forks::fork_of(&path).is_some()
            {
                return path;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("fork was not created");
    }

    #[test]
    fn fork_workflow_never_touches_the_disk() {
        let dir = std::env::temp_dir().join(format!("file-force-fork-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("original.txt"), "keep").unwrap();
        let listing = |dir: &std::path::Path| {
            let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().file_name()).collect();
            names.sort();
            names
        };
        let before = listing(&dir);
        let (view, cx) = rooted_window(&dir);
        let fork = fork_and_wait(&view, cx, &dir);
        assert_ne!(fork, dir);
        let names = |view: &Entity<FileExplorer>, cx: &mut VisualTestContext| {
            cx.read(|app| view.read(app).current_entries().iter().map(|entry| entry.name).collect::<Vec<_>>())
        };
        assert_eq!(names(&view, cx), ["original.txt"], "the fork shows the original's files");

        // New file in the fork, through the real dialog.
        let location = cx.read(|app| view.read(app).current);
        view.update_in(cx, |this, window, cx| this.open_create_dialog(location, false, window, cx));
        cx.run_until_parked();
        cx.simulate_input("draft");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(names(&view, cx), ["draft.txt", "original.txt"]);

        // Deleting inside the fork deletes only the fork's copy.
        let original = cx.read(|app| {
            view.read(app).current_entries().into_iter().find(|entry| entry.name == "original.txt").unwrap().id
        });
        view.update(cx, |this, cx| this.select_entry(original, cx));
        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-backspace" } else { "delete" });
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(names(&view, cx), ["draft.txt"]);
        assert_eq!(listing(&dir), before, "fork changes reached the original");
        assert_eq!(std::fs::read_to_string(dir.join("original.txt")).unwrap(), "keep");

        // Discard deletes the copy and returns to the untouched original.
        let id = crate::forks::fork_of(&fork).unwrap();
        view.update_in(cx, |this, window, cx| this.open_discard_fork_dialog(id, window, cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(cx.read(|app| view.read(app).current), Location::from_path(&dir));
        assert!(!fork.exists(), "discard left the copy behind");
        assert_eq!(listing(&dir), before);
    }

    #[test]
    fn double_clicking_a_file_opens_it_with_its_default_app() {
        let dir = std::env::temp_dir().join(format!("file-force-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.txt");
        std::fs::write(&file, "hello").unwrap();
        let (view, cx) = rooted_window(&dir);
        // The record is shared by parallel tests; count only this file.
        let opened = || crate::default_apps::opened().lock().unwrap().iter().filter(|(path, _)| *path == file).count();

        double_click(cx, "pro-row-notes.txt");
        assert_eq!(opened(), 1, "double-click on a Pro row did not open the file");

        view.update(cx, |this, cx| this.set_mode(ViewMode::Expressive, cx));
        cx.run_until_parked();
        double_click(cx, "card-notes.txt");
        assert_eq!(opened(), 2, "double-click on an Expressive card did not open the file");
    }

    #[test]
    fn choosing_a_default_app_applies_at_once_and_is_used_on_double_click() {
        let dir = std::env::temp_dir().join(format!("file-force-default-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.txt");
        std::fs::write(&file, "hello").unwrap();
        // Any existing ".app"-like path works as an override target.
        let app = dir.join("Editor.app");
        std::fs::create_dir_all(&app).unwrap();
        let (view, cx) = rooted_window(&dir);

        let appearance = cx.read(|app| view.read(app).appearance.clone());
        let settings = cx.update(|window, cx| {
            cx.new(|cx| crate::settings_window::SettingsWindow::new(window, cx, appearance.clone()))
        });
        settings.update(cx, |settings, cx| settings.set_default_app("txt", Some(app.clone()), cx));
        // No Save pressed: the choice is already live and on disk.
        let live = cx.read(|cx| appearance.read(cx).default_apps.get("txt").cloned());
        assert_eq!(live.as_deref(), Some(app.to_string_lossy().as_ref()));
        let saved = std::fs::read_to_string(crate::settings::settings_path().unwrap()).unwrap();
        assert!(saved.contains("[default_apps]") && saved.contains("Editor.app"), "{saved}");

        double_click(cx, "pro-row-notes.txt");
        let opened = crate::default_apps::opened().lock().unwrap().clone();
        assert!(
            opened.iter().any(|(path, used)| *path == file && used.as_deref() == Some("Editor")),
            "{opened:?}"
        );
    }

    #[test]
    fn settings_save_writes_and_stays_open_while_cancel_closes() {
        let (_view, cx) = rooted_window(&std::env::temp_dir());
        let open = |cx: &mut VisualTestContext| cx.update(|window, cx| window.has_active_dialog(cx));
        let click = |cx: &mut VisualTestContext, button: &'static str| {
            let center = cx.debug_bounds(button).expect("footer button rendered").center();
            cx.simulate_click(center, Default::default());
            cx.run_until_parked();
        };
        let settings_file = crate::settings::settings_path().unwrap();
        let _ = std::fs::remove_file(&settings_file);

        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-," } else { "ctrl-," });
        cx.run_until_parked();
        click(cx, "settings-cancel");
        assert!(!open(cx), "Cancel did not close settings");
        assert!(!settings_file.exists(), "Cancel wrote settings");

        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-," } else { "ctrl-," });
        cx.run_until_parked();
        assert!(open(cx), "settings did not reopen from the keyboard");
        // Nothing changed: Save is disabled and writes nothing.
        click(cx, "settings-save");
        assert!(!settings_file.exists(), "Save without changes wrote settings");
        assert!(open(cx), "Save must not close settings");
    }

    #[test]
    fn save_writes_only_when_something_changed() {
        let (view, cx) = rooted_window(&std::env::temp_dir());
        let appearance = cx.read(|app| view.read(app).appearance.clone());
        let settings = cx.update(|window, cx| {
            cx.new(|cx| crate::settings_window::SettingsWindow::new(window, cx, appearance.clone()))
        });
        let file = crate::settings::settings_path().unwrap();
        let _ = std::fs::remove_file(&file);
        settings.update(cx, |settings, cx| settings.toggle_theme_for_test(cx));
        settings.update(cx, |settings, cx| settings.save_for_test(cx));
        assert!(file.exists(), "a change is saved");
    }

    #[test]
    fn conflicting_shortcuts_are_refused() {
        let (view, cx) = rooted_window(&std::env::temp_dir());
        let appearance = cx.read(|app| view.read(app).appearance.clone());
        let settings = cx.update(|window, cx| {
            cx.new(|cx| crate::settings_window::SettingsWindow::new(window, cx, appearance.clone()))
        });
        let taken = if cfg!(target_os = "macos") { "cmd-w" } else { "ctrl-w" };
        settings.update(cx, |settings, cx| settings.set_shortcut("explorer.new_tab", Some(taken.to_string()), cx));
        let keymap = cx.read(|cx| appearance.read(cx).keymap.clone());
        assert!(keymap.is_empty(), "a conflicting shortcut was saved: {keymap:?}");
        let error = cx.read(|cx| settings.read(cx).hotkey_error_for_test());
        assert!(error.is_some_and(|error| error.contains("Close tab")));
        // A free shortcut still works.
        let free = if cfg!(target_os = "macos") { "cmd-y" } else { "ctrl-y" };
        settings.update(cx, |settings, cx| settings.set_shortcut("explorer.new_tab", Some(free.to_string()), cx));
        assert_eq!(cx.read(|cx| appearance.read(cx).keymap.get("explorer.new_tab").cloned()).as_deref(), Some(free));
    }

    #[test]
    fn space_opens_quick_look_and_arrows_browse_inside_it() {
        let dir = std::env::temp_dir().join(format!("file-force-quick-look-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "first").unwrap();
        std::fs::write(dir.join("b.txt"), "second").unwrap();
        let (view, cx) = rooted_window(&dir);
        let open = |cx: &mut VisualTestContext| cx.update(|window, cx| window.has_active_dialog(cx));
        let selected = |cx: &mut VisualTestContext| {
            cx.read(|app| view.read(app).selected_entry_any().map(|entry| entry.name))
        };
        let first = cx.read(|app| view.read(app).current_entries()[0].id);
        view.update(cx, |this, cx| this.select_entry(first, cx));

        cx.simulate_keystrokes("space");
        cx.run_until_parked();
        assert!(open(cx), "space did not open Quick Look");
        cx.simulate_keystrokes("down");
        cx.run_until_parked();
        assert_eq!(selected(cx), Some("b.txt"), "down did not move to the next file");
        assert!(open(cx), "browsing closed the panel");
        cx.simulate_keystrokes("space");
        cx.run_until_parked();
        assert!(!open(cx), "space did not close Quick Look");
        // Focus is back: the shortcut works again without a click.
        cx.simulate_keystrokes("space");
        cx.run_until_parked();
        assert!(open(cx));
    }

    #[test]
    fn rebinding_a_shortcut_moves_the_action_to_the_new_keys() {
        let (view, cx) = rooted_window(&std::env::temp_dir());
        let tabs = |cx: &mut VisualTestContext| cx.read(|app| view.read(app).tabs.len());
        let (old, new) = if cfg!(target_os = "macos") { ("cmd-t", "cmd-y") } else { ("ctrl-t", "ctrl-y") };

        cx.simulate_keystrokes(old);
        assert_eq!(tabs(cx), 2, "default shortcut works");

        let appearance = cx.read(|app| view.read(app).appearance.clone());
        let settings = cx.update(|window, cx| {
            cx.new(|cx| crate::settings_window::SettingsWindow::new(window, cx, appearance.clone()))
        });
        settings.update(cx, |settings, cx| settings.set_shortcut("explorer.new_tab", Some(new.to_string()), cx));
        cx.run_until_parked();
        // The settings view took focus; in the app closing it hands focus back.
        view.update_in(cx, |this, window, cx| this.focus_main(window, cx));

        cx.simulate_keystrokes(new);
        assert_eq!(tabs(cx), 3, "new shortcut opens a tab");
        cx.simulate_keystrokes(old);
        assert_eq!(tabs(cx), 3, "old shortcut must stop working");
        let saved = std::fs::read_to_string(crate::settings::settings_path().unwrap()).unwrap();
        assert!(saved.contains("[keymap]") && saved.contains(new), "{saved}");

        // Resetting brings the default back.
        settings.update(cx, |settings, cx| settings.set_shortcut("explorer.new_tab", None, cx));
        cx.run_until_parked();
        view.update_in(cx, |this, window, cx| this.focus_main(window, cx));
        cx.simulate_keystrokes(old);
        assert_eq!(tabs(cx), 4, "reset restores the default");
        cx.simulate_keystrokes(new);
        assert_eq!(tabs(cx), 4, "reset removes the custom shortcut");
    }

    #[test]
    fn forking_a_selected_folder_and_applying_writes_the_changes() {
        let dir = std::env::temp_dir().join(format!("file-force-fork-apply-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("project")).unwrap();
        std::fs::write(dir.join("project/old.txt"), "old").unwrap();
        let (view, cx) = rooted_window(&dir);

        let fork = fork_and_wait(&view, cx, &dir.join("project"));
        // A terminal or editor in the fork would do exactly this.
        std::fs::write(fork.join("new.md"), "new").unwrap();
        std::fs::remove_file(fork.join("old.txt")).unwrap();
        assert!(dir.join("project/old.txt").exists(), "nothing is written before Apply");

        let id = crate::forks::fork_of(&fork).unwrap();
        view.update_in(cx, |this, window, cx| this.open_apply_fork_dialog(id, window, cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();

        assert_eq!(std::fs::read_to_string(dir.join("project/new.md")).unwrap(), "new");
        assert!(!dir.join("project/old.txt").exists(), "removed file is still there");
        assert_eq!(cx.read(|app| view.read(app).current), Location::from_path(&dir.join("project")));
        assert!(!fork.exists(), "fork stays after Apply");
    }

    #[test]
    fn a_fork_is_a_real_folder_that_stops_going_up_at_its_root() {
        let dir = std::env::temp_dir().join(format!("file-force-fork-real-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let (view, cx) = rooted_window(&dir);
        let fork = fork_and_wait(&view, cx, &dir);
        let root = Location::from_path(&fork);
        // Terminal, apps and file operations all get this real path.
        assert!(root.path().is_some_and(|path| path.is_dir()));
        assert!(root.parent().is_none(), "going up must not leave the fork");
        assert!(Location::from_path(&fork.join("sub")).parent() == Some(root));
        assert!(root.label().contains("fork"), "{}", root.label());
        crate::forks::discard(crate::forks::fork_of(&fork).unwrap());
    }

    #[test]
    fn extension_shortcuts_follow_the_extension_switch() {
        let (view, cx) = pro_window();
        let first = cx.read(|app| view.read(app).current_entries()[0].id);
        view.update(cx, |this, cx| this.select_entry(first, cx));
        let keys = if cfg!(target_os = "macos") { "cmd-alt-c" } else { "ctrl-alt-c" };
        let notice = |cx: &mut VisualTestContext| cx.read(|app| view.read(app).notice.clone()).unwrap_or_default();

        cx.simulate_keystrokes(keys);
        assert_eq!(notice(cx), "Path copied", "Copy extension shortcut did not run");

        view.update(cx, |this, cx| {
            this.appearance.update(cx, |settings, _| settings.extensions.copy = false);
            this.notice = None;
        });
        cx.simulate_keystrokes(keys);
        assert!(notice(cx).contains("Copy extension is disabled"), "{}", notice(cx));
    }

    #[test]
    fn clicking_empty_space_clears_the_selection_unless_disabled() {
        let (view, cx) = pro_window();
        let row = cx.debug_bounds("pro-row-a.txt").expect("row rendered").center();
        let list = cx.debug_bounds("pro-scroll-probe").expect("list rendered");
        // Below the last row, still inside the scroll area.
        let empty = point(list.center().x, list.origin.y + list.size.height + px(60.));
        let selected = |cx: &mut VisualTestContext| cx.read(|app| view.read(app).selection_ids());

        cx.simulate_click(row, Default::default());
        assert_eq!(selected(cx).len(), 1);
        cx.simulate_click(empty, Default::default());
        assert!(selected(cx).is_empty(), "empty-space click kept the selection");
        assert!(cx.read(|app| view.read(app).selected_entry().is_none()), "preview still shows the item");

        // A plain click on a row still selects it (the background press comes first).
        cx.simulate_click(row, Default::default());
        assert_eq!(selected(cx).len(), 1);

        view.update(cx, |this, cx| {
            this.appearance
                .update(cx, |settings, _| settings.behavior.clear_selection_on_empty_click = false)
        });
        cx.simulate_click(empty, Default::default());
        assert_eq!(selected(cx).len(), 1, "setting off: empty click must keep the selection");
    }

    #[test]
    fn key_sequences_pick_new_file_new_folder_or_fork() {
        let dir = std::env::temp_dir().join(format!("file-force-sequences-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (view, cx) = rooted_window(&dir);
        let n = if cfg!(target_os = "macos") { "cmd-n" } else { "ctrl-n" };
        let dialog_title_has = |cx: &mut VisualTestContext, folder: bool| {
            // The create dialog suggests "untitled folder" or "untitled.txt".
            let open = cx.update(|window, cx| window.has_active_dialog(cx));
            open && cx.debug_bounds(if folder { "create-dialog-folder" } else { "create-dialog-file" }).is_some()
        };
        let wait = |cx: &mut VisualTestContext| {
            // Longer than GPUI's pending-sequence timeout.
            cx.executor().advance_clock(std::time::Duration::from_secs(2));
            cx.run_until_parked();
        };

        cx.simulate_keystrokes(n);
        wait(cx);
        assert!(dialog_title_has(cx, false), "cmd-n did not open New File");
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();

        cx.simulate_keystrokes(&format!("{n} f"));
        wait(cx);
        assert!(dialog_title_has(cx, true), "cmd-n f did not open New Folder");
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();

        cx.simulate_keystrokes(&format!("{n} f f"));
        let mut forked = None;
        for _ in 0..200 {
            cx.run_until_parked();
            forked = cx.read(|app| view.read(app).current.path()).and_then(|path| crate::forks::fork_of(&path));
            if forked.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        crate::forks::discard(forked.expect("cmd-n f f did not fork"));
    }

    #[test]
    fn disabling_the_forks_extension_blocks_forking() {
        let dir = std::env::temp_dir().join(format!("file-force-forks-off-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (view, cx) = rooted_window(&dir);
        view.update(cx, |this, cx| this.appearance.update(cx, |settings, _| settings.extensions.forks = false));
        view.update_in(cx, |this, window, cx| this.fork_folder(Location::from_path(&dir), window, cx));
        assert_eq!(cx.read(|app| view.read(app).current), Location::from_path(&dir), "forked while disabled");
        let notice = cx.read(|app| view.read(app).notice.clone()).unwrap_or_default();
        assert!(notice.contains("Forks extension is disabled"), "{notice}");
    }

    #[test]
    fn windows_and_tabs_open_in_the_default_view() {
        let cx = Box::leak(Box::new(TestAppContext::single()));
        cx.update(|cx| {
            gpui_kit::init(cx);
            register_keys(cx);
        });
        let appearance = cx.update(|cx| {
            cx.new(|_| {
                let mut settings = AppearanceSettings::default();
                settings.behavior.default_view = ViewMode::Pro;
                settings
            })
        });
        let (view, cx) = cx.add_window_view(move |window, cx| FileExplorer::new(window, cx, appearance));
        assert_eq!(cx.read(|app| view.read(app).mode), ViewMode::Pro, "window did not start in Pro");
        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-t" } else { "ctrl-t" });
        assert_eq!(cx.read(|app| view.read(app).tabs.len()), 2);
        assert_eq!(cx.read(|app| view.read(app).mode), ViewMode::Pro, "new tab did not start in Pro");
    }

    #[test]
    fn holding_cmd_through_the_sequence_still_forks() {
        let dir = std::env::temp_dir().join(format!("file-force-held-cmd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (view, cx) = rooted_window(&dir);
        let keys = if cfg!(target_os = "macos") { "cmd-n cmd-f cmd-f" } else { "ctrl-n ctrl-f ctrl-f" };
        cx.simulate_keystrokes(keys);
        let mut forked = None;
        for _ in 0..200 {
            cx.run_until_parked();
            forked = cx.read(|app| view.read(app).current.path()).and_then(|path| crate::forks::fork_of(&path));
            if forked.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!cx.update(|window, cx| window.has_active_dialog(cx)), "opened the New File dialog instead");
        crate::forks::discard(forked.expect("holding cmd did not fork"));
    }

    #[test]
    fn accessibility_settings_take_effect() {
        use crate::settings::{TextScale, Toggle};
        let (view, cx) = pro_window();
        let default_border = cx.read(|app| view.read(app).palette().border);
        view.update(cx, |this, cx| {
            this.appearance.update(cx, |settings, cx| {
                settings.look.panel_opacity = 60;
                settings.accessibility.reduce_motion = Toggle::On;
                settings.accessibility.high_contrast = Toggle::On;
                settings.accessibility.text_scale = TextScale::Larger;
                cx.notify();
            });
        });
        cx.run_until_parked();
        assert!(cx.update(|_, cx| cx.reduce_motion()), "reduce motion did not reach GPUI");
        let explorer = |cx: &mut VisualTestContext| cx.read(|app| (view.read(app).palette(), view.read(app).effective_opacity()));
        let (palette, opacity) = explorer(cx);
        assert!(palette.border.l > default_border.l, "high contrast did not brighten borders");
        assert_eq!(palette.muted, palette.text, "secondary text is as strong as primary text");
        assert_eq!(opacity, 1.0, "high contrast keeps panels solid");
        assert_eq!(cx.update(|window, _| window.rem_size()), px(16. * TextScale::Larger.factor()), "text size not applied");

        view.update(cx, |this, cx| {
            this.appearance.update(cx, |settings, cx| {
                settings.accessibility.high_contrast = Toggle::Off;
                settings.accessibility.reduce_transparency = Toggle::On;
                settings.accessibility.reduce_motion = Toggle::Off;
                cx.notify();
            });
        });
        cx.run_until_parked();
        assert_eq!(explorer(cx).1, 1.0, "reduce transparency overrides Appearance opacity");
        assert!(!cx.update(|_, cx| cx.reduce_motion()));
        view.update(cx, |this, cx| {
            this.appearance.update(cx, |settings, cx| {
                settings.accessibility.reduce_transparency = Toggle::Off;
                cx.notify();
            });
        });
        cx.run_until_parked();
        assert!((explorer(cx).1 - 0.6).abs() < 1e-6, "opacity comes back when nothing overrides it");
    }

    /// Two explorer windows in one app, showing `left` and `right`.
    fn two_windows(
        left: &std::path::Path,
        right: &std::path::Path,
    ) -> (Entity<FileExplorer>, Entity<FileExplorer>, &'static mut TestAppContext) {
        let cx = Box::leak(Box::new(TestAppContext::single()));
        cx.update(|cx| {
            gpui_kit::init(cx);
            register_keys(cx);
        });
        let appearance = cx.update(|cx| {
            cx.new(|_| {
                let mut settings = AppearanceSettings::default();
                settings.deletion.use_trash = false;
                settings
            })
        });
        let open = |dir: &std::path::Path, index: usize| {
            let appearance = appearance.clone();
            let location = Location::from_path(dir);
            cx.update(|cx| {
                gpui_kit::open_window(crate::explorer_window_options(index), cx, move |window, cx| {
                    cx.new(|cx| {
                        let mut explorer = FileExplorer::new(window, cx, appearance.clone());
                        explorer.current = location;
                        explorer.mode = ViewMode::Pro;
                        explorer
                    })
                })
            })
            .unwrap()
        };
        let (left_window, left_view) = open(left, 0);
        let (right_window, right_view) = open(right, 1);
        let _ = (left_window, right_window);
        (left_view, right_view, cx)
    }

    fn wait_for(cx: &mut TestAppContext, done: impl Fn() -> bool) {
        for _ in 0..300 {
            cx.run_until_parked();
            if done() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("timed out");
    }

    #[test]
    fn copy_and_cut_in_one_window_paste_in_another() {
        let _clipboard = crate::transfer::clipboard_test_lock();
        let root = std::env::temp_dir().join(format!("file-force-two-windows-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (left, right) = (root.join("left"), root.join("right"));
        std::fs::create_dir_all(&left).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        std::fs::write(left.join("copy-me.txt"), "copy").unwrap();
        std::fs::write(left.join("move-me.txt"), "move").unwrap();
        let (left_view, _right_view, cx) = two_windows(&left, &right);
        let select = |view: &Entity<FileExplorer>, cx: &mut TestAppContext, name: &str| {
            let id = cx.read(|app| {
                view.read(app).current_entries().into_iter().find(|entry| entry.name == name).unwrap().id
            });
            view.update(cx, |this, cx| this.select_entry(id, cx));
        };

        // ⌘C there, ⌘V here.
        select(&left_view, cx, "copy-me.txt");
        left_view.update(cx, |this, cx| this.put_on_clipboard(false, cx));
        let windows: Vec<_> = cx.windows();
        let right_window = windows[1];
        cx.update_window(right_window, |_, window, cx| window.dispatch_action(Box::new(PasteFiles), cx)).unwrap();
        wait_for(cx, || right.join("copy-me.txt").exists());
        assert!(left.join("copy-me.txt").exists(), "copy removed the original");

        // ⌘X dims the item until it is pasted elsewhere, then it moves.
        select(&left_view, cx, "move-me.txt");
        left_view.update(cx, |this, cx| this.put_on_clipboard(true, cx));
        assert!(crate::transfer::is_cut(&left.join("move-me.txt")), "cut item is not marked");
        cx.update_window(right_window, |_, window, cx| window.dispatch_action(Box::new(PasteFiles), cx)).unwrap();
        wait_for(cx, || right.join("move-me.txt").exists());
        assert!(!left.join("move-me.txt").exists(), "cut did not move the file");
        assert!(!crate::transfer::is_cut(&left.join("move-me.txt")), "dimming stays after paste");
    }

    #[test]
    fn drops_follow_the_drag_and_drop_setting() {
        use crate::settings::DropAction;
        let root = std::env::temp_dir().join(format!("file-force-drop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (left, right) = (root.join("left"), root.join("right"));
        std::fs::create_dir_all(left.join("folder")).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        for name in ["default.txt", "moved.txt", "finder.txt", "option.txt", "asked.txt"] {
            std::fs::write(left.join(name), name).unwrap();
        }
        std::fs::write(left.join("folder/inside.txt"), "x").unwrap();
        let (_, right_view, cx) = two_windows(&left, &right);
        let right_window = cx.windows()[1];
        let drop = |cx: &mut TestAppContext, name: &str, action: DropAction, ask: bool, option: bool| {
            let path = left.join(name);
            cx.update_window(right_window, |_, window, cx| {
                right_view.update(cx, |this, cx| {
                    this.appearance.update(cx, |settings, _| {
                        settings.behavior.drop_action = action;
                        settings.behavior.ask_on_drop = ask;
                    });
                    this.receive_files(vec![path], option, window, cx);
                });
            })
            .unwrap();
        };

        drop(cx, "default.txt", DropAction::Copy, false, false);
        wait_for(cx, || right.join("default.txt").exists());
        assert!(left.join("default.txt").exists(), "Copy is the default: the original stays");

        drop(cx, "moved.txt", DropAction::Move, false, false);
        wait_for(cx, || right.join("moved.txt").exists());
        assert!(!left.join("moved.txt").exists(), "Move setting should move");

        drop(cx, "finder.txt", DropAction::Finder, false, false);
        wait_for(cx, || right.join("finder.txt").exists());
        assert!(!left.join("finder.txt").exists(), "Like Finder moves on the same disk");

        drop(cx, "option.txt", DropAction::Move, false, true);
        wait_for(cx, || right.join("option.txt").exists());
        assert!(left.join("option.txt").exists(), "⌥ always copies");

        // Ask every time: a dialog opens; Enter takes the suggested action.
        drop(cx, "asked.txt", DropAction::Move, true, false);
        cx.run_until_parked();
        let open = cx.update_window(right_window, |_, window, cx| window.has_active_dialog(cx)).unwrap();
        assert!(open, "ask-on-drop did not open the dialog");
        assert!(!right.join("asked.txt").exists(), "nothing moves before the user answers");
        cx.update_window(right_window, |_, window, cx| {
            window.dispatch_keystroke(gpui_kit::Keystroke::parse("enter").unwrap(), cx);
        })
        .unwrap();
        wait_for(cx, || right.join("asked.txt").exists());
        assert!(!left.join("asked.txt").exists(), "Enter should take the suggested Move");
    }

    #[test]
    fn cmd_c_and_cmd_x_reach_the_file_list() {
        let _clipboard = crate::transfer::clipboard_test_lock();
        let (view, cx) = pro_window();
        let first = cx.read(|app| view.read(app).current_entries()[0]);
        view.update(cx, |this, cx| this.select_entry(first.id, cx));
        let path = first.path.unwrap().to_path_buf();
        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-x" } else { "ctrl-x" });
        assert!(crate::transfer::is_cut(&path), "cmd-x did not cut the selection");
        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-c" } else { "ctrl-c" });
        let (paths, cut) = crate::transfer::clipboard_contents();
        assert!(paths.contains(&path) && !cut, "cmd-c did not copy the selection");
        crate::transfer::clear_clipboard();
    }

    #[test]
    fn pasting_into_a_long_list_scrolls_to_the_new_item() {
        let _clipboard = crate::transfer::clipboard_test_lock();
        let root = std::env::temp_dir().join(format!("file-force-reveal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (list, outside) = (root.join("list"), root.join("outside"));
        std::fs::create_dir_all(&list).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        for index in 0..300 {
            std::fs::write(list.join(format!("file-{index:03}.txt")), "x").unwrap();
        }
        // Sorted by name it lands at the very end of the list.
        std::fs::write(outside.join("zzz-pasted.txt"), "new").unwrap();
        let (view, cx) = rooted_window(&list);
        let visible = |cx: &mut VisualTestContext, name: &str| {
            let row = cx.debug_bounds(Box::leak(format!("pro-row-{name}").into_boxed_str()));
            let viewport = cx.read(|app| view.read(app).list_scroll.bounds());
            row.is_some_and(|row| row.top() >= viewport.top() && row.bottom() <= viewport.bottom())
        };
        assert!(!visible(cx, "file-299.txt"), "fixture: the end of the list starts off screen");

        crate::transfer::set_clipboard(vec![outside.join("zzz-pasted.txt")], false);
        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-v" } else { "ctrl-v" });
        for _ in 0..300 {
            cx.run_until_parked();
            if list.join("zzz-pasted.txt").exists() && visible(cx, "zzz-pasted.txt") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            cx.refresh().ok();
        }
        crate::transfer::clear_clipboard();
        assert!(list.join("zzz-pasted.txt").exists(), "paste did not happen");
        assert!(visible(cx, "zzz-pasted.txt"), "the pasted item is not scrolled into view");
    }

    #[test]
    fn arrow_keys_keep_the_selection_on_screen() {
        let dir = std::env::temp_dir().join(format!("file-force-arrows-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for index in 0..120 {
            std::fs::write(dir.join(format!("row-{index:03}.txt")), "x").unwrap();
        }
        let (view, cx) = rooted_window(&dir);
        for _ in 0..60 {
            cx.simulate_keystrokes("down");
        }
        for _ in 0..5 {
            cx.run_until_parked();
            cx.refresh().ok();
        }
        let name = cx.read(|app| view.read(app).selected_entry().map(|entry| entry.name)).unwrap();
        let row = cx.debug_bounds(Box::leak(format!("pro-row-{name}").into_boxed_str())).unwrap();
        let viewport = cx.read(|app| view.read(app).list_scroll.bounds());
        assert!(row.top() >= viewport.top() && row.bottom() <= viewport.bottom(), "{name} is off screen");
    }

    #[test]
    fn sidebar_scrolls_and_never_covers_the_disk_card() {
        let (view, cx) = pro_window();
        // Short enough that Favorites, Tags and Connected cannot all fit.
        cx.simulate_resize(gpui_kit::size(px(1100.), px(420.)));
        cx.run_until_parked();
        let disk = cx.debug_bounds("sidebar-disk").expect("disk card rendered");
        let sections = cx.read(|app| view.read(app).sidebar_scroll.bounds());
        assert!(sections.bottom() <= disk.top() + px(0.5), "sections {sections:?} overlap the disk card {disk:?}");
        let overflow = cx.read(|app| view.read(app).sidebar_scroll.max_offset().y);
        assert!(overflow > px(0.), "the sidebar should scroll when it does not fit");
    }

    #[test]
    fn sidebar_sections_fold_and_custom_favorites_come_and_go() {
        let dir = std::env::temp_dir().join(format!("file-force-favorite-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("git")).unwrap();
        let (view, cx) = rooted_window(&dir);
        let header = |cx: &mut VisualTestContext| cx.debug_bounds("sidebar-section-favorites-probe");
        let _ = header;
        let item_shown = |cx: &mut VisualTestContext, path: &std::path::Path| {
            cx.debug_bounds(Box::leak(format!("favorite-probe-{}", path.display()).into_boxed_str())).is_some()
        };

        // Add a favorite (what the context menu does) and see it in the sidebar.
        let favorite = dir.join("git");
        view.update(cx, |this, cx| this.add_favorite(favorite.clone(), cx));
        cx.run_until_parked();
        assert!(cx.read(|app| view.read(app).is_favorite(&favorite, app)));
        assert!(item_shown(cx, &favorite), "favorite is not in the sidebar");

        // Folding Favorites hides it and is saved.
        view.update(cx, |this, cx| this.toggle_section("favorites", cx));
        cx.run_until_parked();
        assert!(!item_shown(cx, &favorite), "folded section still shows its items");
        let saved = std::fs::read_to_string(crate::settings::settings_path().unwrap()).unwrap();
        assert!(saved.contains("collapsed = [\"favorites\"]"), "{saved}");
        view.update(cx, |this, cx| this.toggle_section("favorites", cx));
        cx.run_until_parked();
        assert!(item_shown(cx, &favorite));

        view.update(cx, |this, cx| this.remove_favorite(&favorite, cx));
        cx.run_until_parked();
        assert!(!item_shown(cx, &favorite), "removed favorite is still shown");
    }

    #[test]
    fn breadcrumbs_navigate_and_mouse_side_buttons_walk_history() {
        let root = std::env::temp_dir().join(format!("file-force-crumbs-{}", std::process::id()));
        let deep = root.join("one").join("two");
        std::fs::create_dir_all(&deep).unwrap();
        let (view, cx) = rooted_window(&root);
        view.update_in(cx, |this, window, cx| this.navigate(Location::from_path(&deep), window, cx));
        cx.run_until_parked();
        let current = |cx: &mut VisualTestContext| cx.read(|app| view.read(app).current);

        // The crumb for "one" sits just before the last one.
        let crumbs = Location::from_path(&deep).ancestry().len();
        let one = cx.debug_bounds(Box::leak(format!("breadcrumb-{}", crumbs - 2).into_boxed_str())).unwrap();
        cx.simulate_click(one.center(), Default::default());
        cx.run_until_parked();
        assert_eq!(current(cx), Location::from_path(&root.join("one")), "breadcrumb click did not navigate");

        let side = |cx: &mut VisualTestContext, direction| {
            let position = point(px(700.), px(400.));
            let button = gpui_kit::MouseButton::Navigate(direction);
            cx.simulate_event(gpui_kit::MouseDownEvent { position, button, click_count: 1, ..Default::default() });
            cx.simulate_event(gpui_kit::MouseUpEvent { position, button, click_count: 1, ..Default::default() });
            cx.run_until_parked();
        };
        side(cx, gpui_kit::NavigationDirection::Back);
        assert_eq!(current(cx), Location::from_path(&deep), "mouse Back did not go back");
        side(cx, gpui_kit::NavigationDirection::Forward);
        assert_eq!(current(cx), Location::from_path(&root.join("one")), "mouse Forward did not go forward");
    }

    #[test]
    fn only_the_exact_sidebar_item_is_highlighted() {
        let (view, cx) = pro_window();
        let documents = Location::Documents.path().unwrap();
        let inside = documents.join("file-force-highlight-test");
        std::fs::create_dir_all(&inside).unwrap();
        view.update(cx, |this, cx| this.add_favorite(inside.clone(), cx));
        view.update_in(cx, |this, window, cx| this.navigate(Location::from_path(&inside), window, cx));
        cx.run_until_parked();
        let highlighted = |cx: &mut VisualTestContext, location: Location| {
            cx.read(|app| view.read(app).sidebar_item_selected(location))
        };
        assert!(highlighted(cx, Location::from_path(&inside)), "the favorite itself must be highlighted");
        assert!(!highlighted(cx, Location::Documents), "Documents must not light up for a folder inside it");
        assert!(!highlighted(cx, Location::Home));
        view.update(cx, |this, cx| this.remove_favorite(&inside, cx));
        let _ = std::fs::remove_dir(&inside);
    }

    #[test]
    fn dual_panes_accept_drops_and_have_clickable_breadcrumbs() {
        let root = std::env::temp_dir().join(format!("file-force-dual-drop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (left, right) = (root.join("left"), root.join("right"));
        std::fs::create_dir_all(&left).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        std::fs::write(left.join("a.txt"), "a").unwrap();
        let (view, cx) = pro_window();
        view.update(cx, |this, cx| {
            this.set_mode(ViewMode::Dual, cx);
            this.left.location = Location::from_path(&left);
            this.right.location = Location::from_path(&right);
            this.appearance.update(cx, |settings, _| {
                settings.behavior.drop_action = crate::settings::DropAction::Copy;
                settings.behavior.ask_on_drop = false;
            });
        });
        cx.run_until_parked();

        // Dropping onto its own pane does nothing; onto the other pane copies.
        view.update_in(cx, |this, window, cx| {
            this.drop_into_pane(Pane::Left, vec![left.join("a.txt")], window, cx)
        });
        cx.run_until_parked();
        assert!(!left.join("a copy.txt").exists(), "drop onto the same folder must be a no-op");
        // A real pane-to-pane drag: its source is this same explorer.
        let entry = cx.read(|app| {
            view.read(app).entries_for(Location::from_path(&left)).into_iter().find(|entry| entry.name == "a.txt")
        })
        .unwrap();
        view.update_in(cx, |this, window, cx| {
            let palette = this.palette();
            let drag = crate::views::drag_drop::dragged_file(entry, palette, cx);
            this.drop_dragged(Pane::Right, &drag, window, cx)
        });
        wait_for(cx, || right.join("a.txt").exists());
        assert!(left.join("a.txt").exists(), "Copy keeps the original");

        // A breadcrumb in the right pane opens its folder in that pane.
        let crumbs = Location::from_path(&right).ancestry().len();
        let crumb = cx.debug_bounds(Box::leak(format!("dual-breadcrumb-1-{}", crumbs - 2).into_boxed_str())).unwrap();
        cx.simulate_click(crumb.center(), gpui_kit::Modifiers::none());
        cx.run_until_parked();
        let (left_now, right_now) = cx.read(|app| (view.read(app).left.location, view.read(app).right.location));
        assert_eq!(right_now, Location::from_path(&root));
        assert_eq!(left_now, Location::from_path(&left), "the other pane stays put");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn ssh_session_browses_the_server_and_is_remembered() {
        let dir = std::env::temp_dir().join(format!("file-force-ssh-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let server = dir.join("home");
        std::fs::create_dir_all(server.join("projects")).unwrap();
        std::fs::write(server.join("projects/readme.md"), "x").unwrap();
        crate::ssh::use_fake_ssh(&dir);
        let (view, cx) = pro_window();
        let connection = crate::model::SshConnection { host: "box".into(), user: "me".into(), port: 2222 };
        let home = server.to_string_lossy().into_owned();
        view.update_in(cx, |this, window, cx| this.start_ssh_session(connection, &home, window, cx));
        cx.run_until_parked();

        let names = |cx: &mut VisualTestContext| {
            cx.read(|app| {
                let this = view.read(app);
                this.entries_for(this.current).iter().map(|entry| entry.name).collect::<Vec<_>>()
            })
        };
        assert_eq!(cx.read(|app| view.read(app).current), Location::remote(&home));
        assert_eq!(names(cx), ["projects"]);
        let saved = cx.read(|app| view.read(app).appearance.read(app).sidebar.ssh.clone());
        assert_eq!(saved.first().map(String::as_str), Some("me@box:2222"));

        let projects = cx.read(|app| view.read(app).entries_for(Location::remote(&home))[0]);
        view.update_in(cx, |this, window, cx| this.open_entry(projects.id, window, cx));
        cx.run_until_parked();
        assert_eq!(names(cx), ["readme.md"]);
        let crumbs = cx.read(|app| view.read(app).current.breadcrumbs());
        assert_eq!(crumbs.last().copied(), Some("projects"));

        view.update_in(cx, |this, window, cx| this.disconnect_ssh(window, cx));
        assert_eq!(cx.read(|app| view.read(app).current), Location::Home);
        let _ = std::fs::remove_dir_all(&dir);
    }

    mod extension_contributions {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use files_sdk::{
            Accessibility, Context, Event, Extension, Icon, Manifest, MenuItem, Outcome, Permission, Scope,
            SidebarItem, SidebarSection, Targets,
        };

        use super::*;
        use crate::views::extension_ui::test_support;

        static BACKGROUND_EVENTS: AtomicUsize = AtomicUsize::new(0);
        static WAKES: AtomicUsize = AtomicUsize::new(0);

        struct Probe;

        static MENU: [MenuItem; 2] = [
            MenuItem {
                command: "probe.copy",
                title: "Probe: copy name",
                icon: None,
                targets: Targets::FILE,
                scope: Scope::Local,
                accessibility: Accessibility { label: None, hint: None },
            },
            MenuItem {
                command: "probe.here",
                title: "Probe: here",
                icon: None,
                targets: Targets::BACKGROUND,
                scope: Scope::Anywhere,
                accessibility: Accessibility { label: None, hint: None },
            },
        ];

        static MANIFEST: Manifest = Manifest {
            id: "probe",
            name: "Probe",
            version: "0.0.1",
            source: "Test",
            summary: "",
            icon: Icon::Builtin("cloud"),
            adds: &[],
            permissions: &[Permission::Clipboard, Permission::Background],
            commands: &[],
            menu: &MENU,
            sidebar: &[SidebarSection {
                id: "probe-section",
                title: "PROBE",
                accessibility: Accessibility { label: Some("Probe places"), hint: None },
            }],
            menu_color: None,
            tier: files_sdk::Tier::Free,
        };

        impl Extension for Probe {
            fn manifest(&self) -> &'static Manifest {
                &MANIFEST
            }

            fn run(&self, command: &str, context: &Context) -> std::io::Result<Outcome> {
                Ok(match command {
                    "probe.copy" => Outcome {
                        clipboard: Some(context.selection.join(",")),
                        notice: Some("probe copied".into()),
                        ..Default::default()
                    },
                    "probe.open" => Outcome {
                        navigate: context.sidebar_item.clone().map(Into::into),
                        ..Default::default()
                    },
                    _ => Outcome { notice: context.folder.clone(), ..Default::default() },
                })
            }

            fn sidebar_items(&self, _: &str) -> Vec<SidebarItem> {
                let downloads = Location::Downloads.path().unwrap();
                vec![SidebarItem {
                    title: "Probe Downloads".into(),
                    icon: None,
                    badge: Some("LAN".into()),
                    command: "probe.open",
                    id: downloads.display().to_string(),
                    accessibility_label: None,
                }]
            }

            fn on_event(&self, event: Event) {
                match event {
                    Event::WindowActive(false) => BACKGROUND_EVENTS.fetch_add(1, Ordering::SeqCst),
                    Event::Woke => WAKES.fetch_add(1, Ordering::SeqCst),
                    Event::WindowActive(true) => 0,
                };
            }
        }

        static PROBE: Probe = Probe;

        #[test]
        fn menus_sidebar_permissions_and_events() {
            let (view, cx) = pro_window();
            test_support::register(&PROBE);
            files_sdk::permissions::set_revoked(Default::default());

            // Menu items are offered only for their target and scope.
            let titles = |cx: &mut VisualTestContext, target: Targets, location: Location| {
                cx.read(|app| {
                    view.read(app)
                        .extension_menu_items(target, location, app)
                        .into_iter()
                        .map(|(_, item)| item.title)
                        .filter(|title| title.starts_with("Probe"))
                        .collect::<Vec<_>>()
                })
            };
            assert_eq!(titles(cx, Targets::FILE, Location::Home), ["Probe: copy name"]);
            assert!(titles(cx, Targets::FOLDER, Location::Home).is_empty());
            assert_eq!(titles(cx, Targets::BACKGROUND, Location::Home), ["Probe: here"]);
            assert!(titles(cx, Targets::FILE, Location::remote("/srv")).is_empty(), "local-only item on a server");

            // Running a command: the clipboard needs the permission.
            let run = |cx: &mut VisualTestContext| {
                view.update_in(cx, |this, window, cx| {
                    let context = files_sdk::Context { selection: vec!["a.txt".into()], ..Default::default() };
                    this.run_extension_command(&PROBE, "probe.copy", context, window, cx);
                    this.notice.clone()
                })
            };
            assert_eq!(run(cx).as_deref(), Some("probe copied"));
            assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("a.txt"));
            let mut revoked = std::collections::BTreeMap::new();
            revoked.insert("probe".to_string(), std::collections::BTreeSet::from([Permission::Clipboard]));
            files_sdk::permissions::set_revoked(revoked);
            assert!(run(cx).unwrap_or_default().contains("Settings → Extensions"), "revoked clipboard must be refused");

            // Going to the background reaches only extensions allowed to run there.
            let before = BACKGROUND_EVENTS.load(Ordering::SeqCst);
            cx.read(|app| view.read(app).broadcast_extension_event(Event::WindowActive(false), app));
            assert_eq!(BACKGROUND_EVENTS.load(Ordering::SeqCst), before + 1);
            let mut revoked = std::collections::BTreeMap::new();
            revoked.insert("probe".to_string(), std::collections::BTreeSet::from([Permission::Background]));
            files_sdk::permissions::set_revoked(revoked);
            cx.read(|app| view.read(app).broadcast_extension_event(Event::WindowActive(false), app));
            assert_eq!(BACKGROUND_EVENTS.load(Ordering::SeqCst), before + 1, "no Background permission, no event");
            let wakes = WAKES.load(Ordering::SeqCst);
            cx.read(|app| view.read(app).broadcast_extension_event(Event::Woke, app));
            assert_eq!(WAKES.load(Ordering::SeqCst), wakes + 1);
            files_sdk::permissions::set_revoked(Default::default());

            // The sidebar section renders its rows; a click runs the command.
            // Other sections (agents found by other tests…) are folded so the row stays on screen.
            view.update(cx, |this, cx| {
                this.appearance.update(cx, |settings, _| {
                    settings.sidebar.collapsed = ["favorites", "agents", "forks", "tags", "connected"].map(String::from).to_vec();
                });
                cx.notify();
            });
            cx.run_until_parked();
            let row = cx.debug_bounds("ext-probe-section-0").expect("extension sidebar row not rendered");
            cx.simulate_click(row.center(), gpui_kit::Modifiers::none());
            cx.run_until_parked();
            assert_eq!(cx.read(|app| view.read(app).current), Location::Downloads);
        }

        #[test]
        fn revoked_permissions_round_trip_through_settings() {
            let mut settings = crate::settings::AppSettings::default();
            settings.set_revoked("ssh", Permission::Network, true);
            let source = toml::to_string(&settings).unwrap();
            assert!(source.contains("[permissions]") && source.contains("network"), "{source}");
            let back: crate::settings::AppSettings = toml::from_str(&source).unwrap();
            assert!(back.revoked("ssh", Permission::Network));
            assert!(!back.revoked("ssh", Permission::Secrets));
            let mut back = back;
            back.set_revoked("ssh", Permission::Network, false);
            assert!(back.permissions.is_empty());
        }
    }

    #[test]
    #[cfg(unix)]
    fn agent_sessions_show_in_the_sidebar_and_stream_replies() {
        use files_ext_agents::{Agent, Role};
        use std::os::unix::fs::PermissionsExt as _;

        let dir = std::env::temp_dir().join(format!("file-force-agents-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let project = dir.join("claude/projects/-work");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("old-1.jsonl"),
            r#"{"type":"user","message":{"role":"user","content":"Explain the build"},"timestamp":"2026-10-01T10:00:00Z","cwd":"/work"}
{"type":"assistant","message":{"id":"m","model":"claude-sonnet-5","content":[{"type":"text","text":"It uses cargo."}],"usage":{"input_tokens":1200,"output_tokens":300}},"timestamp":"2026-10-01T10:00:05Z"}
"#,
        )
        .unwrap();
        // A fake `claude` that streams like `claude -p --output-format stream-json`.
        let fake = dir.join("claude-fake");
        std::fs::write(
            &fake,
            "#!/bin/sh\n\
             echo '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"new-42\"}'\n\
             echo '{\"type\":\"stream_event\",\"event\":{\"type\":\"content_block_delta\",\"delta\":{\"text\":\"Hello \"}}}'\n\
             echo '{\"type\":\"stream_event\",\"event\":{\"type\":\"content_block_delta\",\"delta\":{\"text\":\"there\"}}}'\n\
             echo '{\"type\":\"result\",\"usage\":{\"input_tokens\":7,\"output_tokens\":3}}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        files_ext_agents::use_program(Agent::Claude, fake);
        *crate::views::agents::TEST_ROOTS.lock().unwrap() = Some(files_ext_agents::Roots {
            claude: dir.join("claude"),
            claude_account: dir.join("claude.json"),
            codex: dir.join("codex"),
            opencode_db: dir.join("opencode.db"),
        });

        let (view, cx) = pro_window();
        view.update(cx, |this, cx| this.refresh_agents(cx));
        for _ in 0..300 {
            cx.run_until_parked();
            if cx.read(|app| !view.read(app).agent_sessions.is_empty()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let session = cx.read(|app| view.read(app).agent_sessions[0].clone());
        assert_eq!((session.agent, session.title.as_str()), (Agent::Claude, "Explain the build"));
        assert!(session.external, "not started from Files");
        cx.run_until_parked();
        assert!(cx.debug_bounds("agent-session-0").is_some(), "session row missing in the sidebar");

        // A new session from Files streams its answer and is remembered as ours.
        let windows = cx.windows().len();
        let weak = view.downgrade();
        let appearance = cx.read(|app| view.read(app).appearance.clone());
        let agent_view = cx
            .update(|_, app| crate::views::agents::open_agent_window(None, dir.clone(), appearance, weak, Vec::new(), app))
            .expect("agent window did not open");
        cx.run_until_parked();
        assert_eq!(cx.windows().len(), windows + 1);
        let agent_window = *cx.windows().last().unwrap();
        cx.update_window(agent_window, |_, window, app| {
            agent_view.update(app, |this, cx| {
                this.set_prompt_for_test("Say hello", window, cx);
                this.send_for_test(window, cx);
            })
        })
        .unwrap();
        for _ in 0..300 {
            // The reader thread is real; the UI polls it on gpui's test clock.
            cx.executor().advance_clock(std::time::Duration::from_millis(100));
            cx.run_until_parked();
            if cx.read(|app| !agent_view.read(app).is_running()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let (messages, id) = cx.read(|app| {
            let window = agent_view.read(app);
            (window.messages_for_test(), window.session_id_for_test())
        });
        let notice = cx.read(|app| agent_view.read(app).notice_for_test());
        assert_eq!(messages.iter().map(|message| message.role).collect::<Vec<_>>(), [Role::User, Role::Assistant], "{notice:?}");
        assert_eq!(messages[1].text, "Hello there");
        assert_eq!(id.as_deref(), Some("new-42"));
        let ours = cx.read(|app| view.read(app).appearance.read(app).sidebar.agents.clone());
        assert!(ours.contains(&"new-42".to_string()), "{ours:?}");

        *crate::views::agents::TEST_ROOTS.lock().unwrap() = None;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn usage_cards_fold_fit_the_window_and_scroll() {
        use files_ext_agents::{Agent, Session, Usage};
        let session = |agent: Agent, index: i64| Session {
            agent,
            id: format!("{agent:?}-{index}"),
            // Long text used to push the cards wider than the window.
            title: "a very long session title that goes on and on ".repeat(4),
            cwd: Some("/work".into()),
            model: Some("model-with-a-really-long-name-".repeat(6)),
            updated: 1_800_000_000 - index,
            usage: Usage { input: 1_000_000, output: 2_000, ..Default::default() },
            origin: None,
            running: false,
            external: true,
            source: None,
        };
        let sessions: Vec<Session> =
            (0..60).map(|index| session(Agent::ALL[index as usize % 3], index)).collect();
        let (view, cx) = pro_window();
        let appearance = cx.read(|app| view.read(app).appearance.clone());
        let usage = cx.update(|_, app| crate::views::agents::open_usage_window(sessions, appearance, app)).unwrap();
        cx.run_until_parked();
        let window = *cx.windows().last().unwrap();
        let mut usage_cx = gpui_kit::VisualTestContext::from_window(window, cx);
        usage_cx.run_until_parked();

        // Only the most recent agent starts unfolded.
        assert_eq!(usage_cx.read(|app| usage.read(app).expanded_for_test()), [Agent::Claude]);
        let width = usage_cx.update(|window, _| window.viewport_size().width);
        for agent in Agent::ALL {
            let card = usage_cx.debug_bounds(Box::leak(format!("usage-card-{}", agent.short()).into_boxed_str())).unwrap();
            assert!(card.origin.x >= gpui_kit::px(0.) && card.right() <= width, "{agent:?} card leaves the window: {card:?}");
        }
        // Clicking a header unfolds it; clicking again folds it.
        let codex = usage_cx.debug_bounds("usage-card-codex").unwrap();
        usage_cx.simulate_click(codex.center(), gpui_kit::Modifiers::none());
        assert_eq!(usage_cx.read(|app| usage.read(app).expanded_for_test()), [Agent::Claude, Agent::Codex]);
        usage_cx.simulate_click(codex.center(), gpui_kit::Modifiers::none());
        assert_eq!(usage_cx.read(|app| usage.read(app).expanded_for_test()), [Agent::Claude]);

        // The content is taller than the window and scrolls.
        let before = usage_cx.debug_bounds("usage-card-opencode").unwrap();
        usage_cx.simulate_event(gpui_kit::ScrollEvent {
            position: before.center(),
            delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-400.))),
            ..Default::default()
        });
        usage_cx.run_until_parked();
        let after = usage_cx.debug_bounds("usage-card-opencode").unwrap();
        assert!(after.origin.y < before.origin.y, "did not scroll: {before:?} → {after:?}");
    }

    #[test]
    fn open_in_agents_files_starts_a_session_window_in_that_folder() {
        let (view, cx) = pro_window();
        let folder = std::env::temp_dir();
        let windows = cx.windows().len();
        view.update(cx, |this, cx| {
            this.open_in_agents(crate::views::agents::AgentTarget::Files, folder.clone(), cx)
        });
        cx.run_until_parked();
        assert_eq!(cx.windows().len(), windows + 1, "no session window");
        let notice = cx.read(|app| view.read(app).notice.clone()).unwrap_or_default();
        assert!(notice.contains("New agent session"), "{notice}");
    }

    #[test]
    fn an_open_chat_follows_a_session_running_elsewhere() {
        use files_ext_agents::Role;
        use std::io::Write as _;

        let dir = std::env::temp_dir().join(format!("file-force-agents-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let project = dir.join("claude/projects/-work");
        std::fs::create_dir_all(&project).unwrap();
        let log = project.join("live-1.jsonl");
        std::fs::write(
            &log,
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"Run the tests\"},\"timestamp\":\"2026-10-30T10:00:00Z\",\"cwd\":\"/work\"}\n",
        )
        .unwrap();
        let session = files_ext_agents::claude_session(&log).unwrap();
        let (view, cx) = pro_window();
        let appearance = cx.read(|app| view.read(app).appearance.clone());
        let weak = view.downgrade();
        let chat = cx
            .update(|_, app| crate::views::agents::open_agent_window(Some(session), "/work".into(), appearance, weak, Vec::new(), app))
            .unwrap();
        let tick = |cx: &mut VisualTestContext| {
            cx.executor().advance_clock(std::time::Duration::from_millis(1100));
            cx.run_until_parked();
        };
        tick(cx);
        assert_eq!(cx.read(|app| chat.read(app).messages_for_test().len()), 1);

        // The agent, running in a terminal, writes its answer to the log.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        writeln!(
            file,
            "{{\"type\":\"assistant\",\"message\":{{\"id\":\"a1\",\"content\":[{{\"type\":\"text\",\"text\":\"All 12 passed\"}}]}},\"timestamp\":\"2026-10-30T10:00:09Z\"}}"
        )
        .unwrap();
        drop(file);
        for _ in 0..50 {
            tick(cx);
            if cx.read(|app| chat.read(app).messages_for_test().len()) == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let messages = cx.read(|app| chat.read(app).messages_for_test());
        assert_eq!(messages.iter().map(|message| message.role).collect::<Vec<_>>(), [Role::User, Role::Assistant]);
        assert_eq!(messages[1].text, "All 12 passed");
        assert!(cx.read(|app| chat.read(app).is_live_for_test()), "a changing log is shown as live");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chat_messages_render_markdown_and_copy_their_source() {
        use files_ext_agents::{ChatMessage, Role};
        let (view, cx) = pro_window();
        let appearance = cx.read(|app| view.read(app).appearance.clone());
        let weak = view.downgrade();
        let chat = cx
            .update(|_, app| crate::views::agents::open_agent_window(None, "/work".into(), appearance, weak, Vec::new(), app))
            .unwrap();
        let window = *cx.windows().last().unwrap();
        let mut chat_cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let answer = "**Тест** passed:\n\n- `cargo test`\n- 12 ok";
        chat_cx.update(|_, app| {
            chat.update(app, |this, cx| {
                this.set_messages_for_test(vec![
                    ChatMessage { role: Role::User, text: "run tests".into() },
                    ChatMessage { role: Role::Assistant, text: answer.into() },
                ]);
                cx.notify();
            })
        });
        chat_cx.run_until_parked();

        // Hovering the answer reveals its copy button; it copies the markdown source.
        let bubble = chat_cx.debug_bounds("agent-bubble-1").expect("answer not rendered");
        chat_cx.simulate_mouse_move(bubble.center(), None, gpui_kit::Modifiers::none());
        chat_cx.run_until_parked();
        let copy = chat_cx.debug_bounds("agent-copy-1").expect("no copy button");
        chat_cx.simulate_click(copy.center(), gpui_kit::Modifiers::none());
        assert_eq!(chat_cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some(answer));

        // Tool lines are shown literally, not as markdown.
        assert_eq!(crate::views::agents::escape_markdown_for_test("⚙ rm *.tmp"), "⚙ rm \\*\\.tmp");
    }

    #[test]
    fn a_long_chat_scrolls() {
        use files_ext_agents::{ChatMessage, Role};
        let (view, cx) = pro_window();
        let appearance = cx.read(|app| view.read(app).appearance.clone());
        let weak = view.downgrade();
        let chat = cx
            .update(|_, app| crate::views::agents::open_agent_window(None, "/work".into(), appearance, weak, Vec::new(), app))
            .unwrap();
        let window = *cx.windows().last().unwrap();
        let mut chat_cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let messages = (0..40)
            .map(|index| ChatMessage {
                role: if index % 2 == 0 { Role::User } else { Role::Assistant },
                text: format!(
                    "message {index}\n\n```rust\nfn main() {{ println!(\"{}\"); }}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |",
                    "x".repeat(200)
                ),
            })
            .collect();
        chat_cx.update(|_, app| {
            chat.update(app, |this, cx| {
                this.set_messages_for_test(messages);
                cx.notify();
            })
        });
        chat_cx.run_until_parked();
        let first = chat_cx.debug_bounds("agent-bubble-0").expect("first message");
        for target in [first.center(), first.center() + gpui_kit::point(gpui_kit::px(300.), gpui_kit::px(0.))] {
            chat_cx.simulate_event(gpui_kit::ScrollEvent {
                position: target,
                delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-300.))),
                ..Default::default()
            });
        }
        chat_cx.run_until_parked();
        let after = chat_cx.debug_bounds("agent-bubble-0").unwrap();
        assert!(after.origin.y < first.origin.y, "chat did not scroll: {first:?} → {after:?}");
    }

    #[test]
    fn scrolling_up_in_a_live_chat_is_not_undone() {
        use std::io::Write as _;
        let dir = std::env::temp_dir().join(format!("file-force-agents-scroll-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let project = dir.join("claude/projects/-work");
        std::fs::create_dir_all(&project).unwrap();
        let log = project.join("busy.jsonl");
        let line = |index: usize| {
            format!(
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"message {index}\\n\\nwith a second paragraph\"}},\"timestamp\":\"2026-10-30T10:00:00Z\",\"cwd\":\"/work\"}}\n"
            )
        };
        std::fs::write(&log, (0..60).map(line).collect::<String>()).unwrap();
        let session = files_ext_agents::claude_session(&log).unwrap();
        let (view, cx) = pro_window();
        let appearance = cx.read(|app| view.read(app).appearance.clone());
        let weak = view.downgrade();
        let chat = cx
            .update(|_, app| crate::views::agents::open_agent_window(Some(session), "/work".into(), appearance, weak, Vec::new(), app))
            .unwrap();
        let window = *cx.windows().last().unwrap();
        let mut chat_cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let tick = |cx: &mut VisualTestContext| {
            cx.executor().advance_clock(std::time::Duration::from_millis(1100));
            cx.run_until_parked();
        };
        for _ in 0..20 {
            tick(&mut chat_cx);
            if chat_cx.read(|app| chat.read(app).messages_for_test().len()) == 60 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        tick(&mut chat_cx);
        let bottom = chat_cx.read(|app| chat.read(app).scroll_offset_for_test());
        assert!(bottom < gpui_kit::px(0.), "a loaded chat opens at its end");

        // A small trackpad flick up, while the agent keeps writing.
        let bubble = chat_cx.debug_bounds("agent-bubble-59").unwrap();
        chat_cx.simulate_event(gpui_kit::ScrollEvent {
            position: bubble.center(),
            delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(30.))),
            ..Default::default()
        });
        chat_cx.run_until_parked();
        let scrolled = chat_cx.read(|app| chat.read(app).scroll_offset_for_test());
        assert!(scrolled > bottom, "did not scroll up: {bottom:?} → {scrolled:?}");
        for index in 60..63 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            std::fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(line(index).as_bytes()).unwrap();
            tick(&mut chat_cx);
            tick(&mut chat_cx);
        }
        assert_eq!(chat_cx.read(|app| chat.read(app).messages_for_test().len()), 63, "new messages still arrive");
        assert_eq!(chat_cx.read(|app| chat.read(app).scroll_offset_for_test()), scrolled, "the reader was pulled back down");

        // The floating button brings the reader back to the latest message.
        let button = chat_cx.debug_bounds("agent-scroll-bottom").expect("no scroll-to-bottom button while scrolled up");
        chat_cx.simulate_click(button.center(), gpui_kit::Modifiers::none());
        chat_cx.run_until_parked();
        assert!(chat_cx.read(|app| chat.read(app).follows_for_test()));
        assert!(chat_cx.read(|app| chat.read(app).scroll_offset_for_test()) < scrolled, "did not scroll down");
        chat_cx.run_until_parked();
        assert!(chat_cx.debug_bounds("agent-scroll-bottom").is_none(), "button stays while at the end");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn the_picked_model_and_effort_reach_the_agent() {
        use files_ext_agents::Agent;
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("file-force-agents-model-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Answers with the arguments it was started with.
        let fake = dir.join("codex-fake");
        std::fs::write(
            &fake,
            "#!/bin/sh\nprintf '{\"type\":\"thread.started\",\"thread_id\":\"t-9\"}\\n'\nprintf '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"%s\"}}\\n' \"$(echo \"$*\" | tr '\"' \"'\")\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        files_ext_agents::use_program(Agent::Codex, fake);

        let (view, cx) = pro_window();
        let appearance = cx.read(|app| view.read(app).appearance.clone());
        let weak = view.downgrade();
        let chat = cx
            .update(|_, app| crate::views::agents::open_agent_window(None, dir.clone(), appearance, weak, Vec::new(), app))
            .unwrap();
        let window = *cx.windows().last().unwrap();
        cx.update_window(window, |_, window, app| {
            chat.update(app, |this, cx| {
                this.set_agent_for_test(Agent::Codex, cx);
                this.pick_for_test(Some("gpt-5.5"), Some("xhigh"));
                this.set_prompt_for_test("hi", window, cx);
                this.send_for_test(window, cx);
            })
        })
        .unwrap();
        for _ in 0..300 {
            cx.executor().advance_clock(std::time::Duration::from_millis(100));
            cx.run_until_parked();
            if cx.read(|app| !chat.read(app).is_running()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let answer = cx.read(|app| chat.read(app).messages_for_test()).last().map(|message| message.text.clone()).unwrap_or_default();
        assert!(answer.contains("-m gpt-5.5") && answer.contains("model_reasoning_effort='xhigh'"), "{answer}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn menu_tints_and_text_accessibility() {
        use crate::settings::Toggle;
        use gpui_kit::component::menu::PopupMenuItem;

        // A color makes a tinted element item; no color keeps the plain item.
        let plain = crate::views::context_menu::tinted_item("Delete…", gpui_kit::assets::IconName::Trash, None);
        assert!(matches!(plain, PopupMenuItem::Item { .. }));
        let red = crate::menu_colors::color_for(&crate::settings::AppSettings::default(), crate::menu_colors::DELETE);
        let tinted = crate::views::context_menu::tinted_item("Delete…", gpui_kit::assets::IconName::Trash, red);
        assert!(matches!(tinted, PopupMenuItem::ElementItem { .. }), "Delete is red by default");

        // Text contrast: dark text gets a white outline, light text a dark one.
        let mut settings = crate::settings::AppSettings::default();
        settings.accessibility.text_contrast = Toggle::On;
        settings.scheme = crate::model::ColorScheme::Light;
        assert_eq!(Palette::from_settings(&settings).text_outline, Some(gpui_kit::white()));
        settings.scheme = crate::model::ColorScheme::Dark;
        assert_eq!(Palette::from_settings(&settings).text_outline, Some(gpui_kit::black()));
        settings.accessibility.text_contrast = Toggle::Off;
        assert_eq!(Palette::from_settings(&settings).text_outline, None);

        // Shimmer is on by default and never runs while motion is reduced.
        settings.accessibility.reduce_motion = Toggle::Off;
        assert!(Palette::from_settings(&settings).shimmer);
        settings.accessibility.shimmer = Toggle::On;
        settings.accessibility.reduce_motion = Toggle::On;
        assert!(!Palette::from_settings(&settings).shimmer);
        settings.accessibility.reduce_motion = Toggle::Off;
        settings.accessibility.shimmer = Toggle::Off;
        assert!(!Palette::from_settings(&settings).shimmer);

        let source = toml::to_string(&settings).unwrap();
        assert!(source.contains("text_contrast = \"off\"") && source.contains("shimmer = \"off\""), "{source}");
    }

    #[test]
    fn text_contrast_outlines_file_names() {
        let (view, cx) = pro_window();
        let names_with = |cx: &mut VisualTestContext, on: bool| {
            view.update(cx, |this, cx| {
                this.accessibility.text_contrast =
                    if on { crate::settings::Toggle::On } else { crate::settings::Toggle::Off };
                cx.notify();
            });
            cx.run_until_parked();
            cx.read(|app| view.read(app).palette().text_outline.is_some())
        };
        assert!(names_with(cx, true), "the explorer palette follows Text contrast");
        assert!(!names_with(cx, false));
    }

    #[test]
    fn quick_look_on_a_folder_shows_what_is_inside() {
        let dir = std::env::temp_dir().join(format!("file-force-quick-look-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("project/src")).unwrap();
        for name in ["README.md", "main.rs", "notes.txt"] {
            std::fs::write(dir.join("project").join(name), "x").unwrap();
        }
        let (view, cx) = rooted_window(&dir);
        let folder = cx.read(|app| view.read(app).current_entries().into_iter().find(|entry| entry.name == "project").unwrap().id);
        view.update(cx, |this, cx| this.select_entry(folder, cx));
        cx.simulate_keystrokes("space");
        cx.run_until_parked();
        assert!(cx.update(|window, cx| window.has_active_dialog(cx)), "Quick Look did not open");
        let tiles = (0..10).filter(|index| cx.debug_bounds(Box::leak(format!("quick-look-item-{index}").into_boxed_str())).is_some()).count();
        assert_eq!(tiles, 4, "the folder's 1 subfolder and 3 files are previewed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_window_commits_the_ticked_files_and_shows_the_log() {
        use crate::views::git::GitPage;
        files_sdk::permissions::declare(&files_ext_git::MANIFEST);
        let dir = std::env::temp_dir().join(format!("file-force-git-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&dir).output().unwrap().status.success());
        git(&["init", "-q"]);
        git(&["config", "user.name", "T"]);
        git(&["config", "user.email", "t@e.com"]);
        git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("tracked.txt"), "1\n").unwrap();
        git(&["add", "tracked.txt"]);
        git(&["commit", "-qm", "first"]);
        std::fs::write(dir.join("tracked.txt"), "1\n2\n").unwrap();
        std::fs::write(dir.join("new.txt"), "n\n").unwrap();

        let (view, cx) = pro_window();
        let appearance = cx.read(|app| view.read(app).appearance.clone());
        let git_view = cx
            .update(|_, app| crate::views::git::open_git_window(dir.clone(), GitPage::Commit, appearance, app))
            .unwrap();
        let window = *cx.windows().last().unwrap();
        let mut git_cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let settle = |cx: &mut VisualTestContext, done: &dyn Fn(&mut VisualTestContext) -> bool| {
            for _ in 0..300 {
                cx.run_until_parked();
                if done(cx) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("timed out");
        };
        settle(&mut git_cx, &|cx| cx.read(|app| git_view.read(app).status_for_test().is_some()));
        // Untracked files start unticked, like TortoiseGit's commit dialog.
        assert_eq!(git_cx.read(|app| git_view.read(app).ticked_for_test()), ["tracked.txt"]);
        assert!(git_cx.debug_bounds("git-change-1").is_some(), "both changes are listed");

        git_cx.update(|_, app| git_view.update(app, |this, cx| this.show_diff_for_test("tracked.txt", cx)));
        settle(&mut git_cx, &|cx| cx.read(|app| git_view.read(app).diff_for_test().is_some()));
        assert!(git_cx.read(|app| git_view.read(app).diff_for_test()).unwrap().1.contains("+2"));

        git_cx.update(|window, app| {
            git_view.update(app, |this, cx| {
                this.set_message_for_test("Second line", window, cx);
                this.commit_for_test(window, cx);
            })
        });
        settle(&mut git_cx, &|cx| cx.read(|app| !git_view.read(app).busy_for_test()));
        let status = files_ext_git::status(&dir).unwrap();
        assert_eq!(status.changes.iter().map(|change| change.path.as_str()).collect::<Vec<_>>(), ["new.txt"], "only the ticked file was committed");

        git_cx.update(|_, app| git_view.update(app, |this, cx| this.set_page_for_test(GitPage::Log(None), cx)));
        settle(&mut git_cx, &|cx| cx.read(|app| git_view.read(app).commits_for_test().len() == 2));
        assert_eq!(git_cx.read(|app| git_view.read(app).commits_for_test())[0].subject, "Second line");

        // Outside a repository: "Create Repository Here".
        let plain = dir.with_extension("plain");
        let _ = std::fs::remove_dir_all(&plain);
        std::fs::create_dir_all(&plain).unwrap();
        cx.update(|window, app| view.update(app, |this, cx| this.git_action(crate::views::git::GitAction::Init, plain.clone(), window, cx)));
        for _ in 0..300 {
            cx.run_until_parked();
            if plain.join(".git").exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(files_ext_git::repo_root(&plain).is_some(), "git init did not run");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&plain);
    }

    #[test]
    fn extension_pages_preview_their_menu_items() {
        let find = |id: &str| *crate::extensions::CATALOG.iter().find(|manifest| manifest.id == id).unwrap();
        let git = crate::settings_window::menu_preview_titles(find("git"));
        assert_eq!(git[0].0, "Git");
        assert!(git[0].1.contains(&"Git Commit…"), "{git:?}");
        assert_eq!(crate::settings_window::menu_preview_titles(find("forks"))[0].0, "Fork folder");
        assert!(crate::settings_window::menu_preview_titles(find("terminal")).is_empty(), "Terminal is a base item: colored in Appearance");
        // The real Git extension replaced the TortoiseGit demo in Settings.
        assert_eq!(find("git").source, "Built-in");
        assert_eq!(find("git").version, files_ext_git::MANIFEST.version);
    }

    #[test]
    fn console_and_notifications_in_the_footer() {
        use files_sdk::log::{self, Kind, Level};
        let (view, cx) = pro_window();

        // A status message lands in the journal once; a failure as an error.
        view.update(cx, |this, cx| {
            this.notice = Some("Copied 3 items".into());
            cx.notify();
        });
        cx.run_until_parked();
        view.update(cx, |this, cx| {
            this.notice = Some("Open failed · x: no app".into());
            cx.notify();
        });
        cx.run_until_parked();
        let events = log::events();
        assert_eq!(events.iter().filter(|event| event.message == "Copied 3 items").count(), 1);
        assert!(events.iter().any(|event| event.kind == Kind::Error && event.message.starts_with("Open failed")));

        // An extension runs a program: only Developer mode shows it.
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "echo console-probe"]);
        log::run("git", &mut command).unwrap();
        let process = log::events().into_iter().rev().find(|event| event.message.contains("console-probe")).unwrap();
        let shows = |cx: &mut VisualTestContext, developer: bool| {
            cx.read(|app| view.read(app).console_events(developer, "").iter().any(|event| event.id == process.id))
        };
        assert!(!shows(cx, false), "without Developer mode the console shows messages, not calls");
        assert!(shows(cx, true));
        assert!(cx.read(|app| view.read(app).console_events(true, "probe").len()) >= 1, "filter by text");

        // Other tests write to the same journal: narrow the console to this call.
        view.update_in(cx, |this, window, cx| {
            this.appearance.update(cx, |settings, _| {
                settings.developer.enabled = true;
                settings.developer.show_console = true;
            });
            this.console_open = true;
            this.console_query.update(cx, |input, cx| input.set_value("console-probe", window, cx));
            cx.notify();
        });
        cx.run_until_parked();
        let row = cx.debug_bounds(Box::leak(format!("console-event-{}", process.id).into_boxed_str())).expect("call not in the console");
        cx.simulate_click(row.center(), gpui_kit::Modifiers::none());
        assert_eq!(cx.read(|app| view.read(app).console_expanded), Some(process.id), "a click shows arguments and output");
        // The copy button gives the exact command, ready to paste in a terminal.
        let copy = cx.debug_bounds(Box::leak(format!("console-copy-{}", process.id).into_boxed_str())).expect("no copy button");
        cx.simulate_click(copy.center(), gpui_kit::Modifiers::none());
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()).as_deref(),
            Some("sh -c 'echo console-probe'")
        );

        // A notification: unread on the bell until the card is opened.
        let id = log::notify("git", Level::Success, "Git · Fetch done", None);
        cx.executor().advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        let read = |id: u64| log::notifications().iter().find(|notification| notification.id == id).map(|notification| notification.read);
        assert_eq!(read(id), Some(false));
        let bell = cx.debug_bounds("footer-bell").expect("no bell");
        cx.simulate_click(bell.center(), gpui_kit::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(read(id), Some(true), "opening the card marks it read");
        assert!(cx.debug_bounds(Box::leak(format!("notification-{id}").into_boxed_str())).is_some(), "notification not listed");
        log::dismiss(id);
        assert!(!log::notifications().iter().any(|notification| notification.id == id));
    }

    #[test]
    fn old_tortoise_git_settings_still_apply_to_git() {
        let old = "[extensions]\ntortoise_git = false\n\n[menu_colors]\ntortoise-git = \"blue\"\n\n[permissions]\ntortoise-git = [\"network\"]\n";
        let mut settings: crate::settings::AppSettings = toml::from_str(old).unwrap();
        settings.migrate();
        assert!(!settings.extensions.git, "turned off before, still off");
        assert_eq!(settings.menu_colors.get("git").map(String::as_str), Some("blue"));
        assert!(settings.revoked("git", files_sdk::Permission::Network));
        assert!(!settings.menu_colors.contains_key("tortoise-git"));
        let written = toml::to_string(&settings).unwrap();
        assert!(written.contains("git = false") && !written.contains("tortoise"), "{written}");
    }

    #[test]
    fn developer_tab_opens_console_is_opt_in_and_toasts_fade() {
        use files_sdk::log::{self, Level};
        let (view, cx) = pro_window();
        let appearance = cx.read(|app| view.read(app).appearance.clone());
        let settings = cx.update(|window, cx| cx.new(|cx| crate::settings_window::SettingsWindow::new(window, cx, appearance.clone())));
        // Developer is the 9th tab; it used to be clamped to Accessibility.
        assert_eq!(cx.update(|_, app| settings.update(app, |this, cx| this.select_for_test(8, cx))), 8);
        assert_eq!(cx.update(|_, app| settings.update(app, |this, cx| this.select_for_test(42, cx))), 8);

        // No Console button until the user asks for it.
        cx.run_until_parked();
        assert!(cx.debug_bounds("footer-console").is_none(), "Console must be opt-in");
        view.update(cx, |this, cx| {
            this.appearance.update(cx, |settings, _| settings.developer.show_console = true);
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("footer-console").is_some(), "Show Console in the footer did not add it");

        // A notification shows as a toast, × hides it, and it fades by itself.
        let id = log::notify("git", Level::Success, "Git · Fetch done", Some("origin".into()));
        cx.executor().advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        let selector: &'static str = Box::leak(format!("toast-{id}").into_boxed_str());
        assert!(cx.debug_bounds(selector).is_some(), "no toast");
        assert!(cx.read(|app| view.read(app).toasts().iter().any(|toast| toast.id == id)));
        view.update(cx, |this, cx| {
            this.hidden_toasts.insert(id);
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds(selector).is_none(), "× did not hide the toast");
        assert!(log::notifications().iter().any(|notification| notification.id == id), "it stays in the list");

        let fading = log::notify("files", Level::Info, "Trash emptied", None);
        cx.run_until_parked();
        assert!(cx.read(|app| view.read(app).toasts().iter().any(|toast| toast.id == fading)));
        std::thread::sleep(std::time::Duration::from_millis(6100));
        assert!(!cx.read(|app| view.read(app).toasts().iter().any(|toast| toast.id == fading)), "toasts fade after 6 s");
    }

    #[test]
    fn dragging_the_list_scrollbar_scrolls_and_selects_nothing() {
        let dir = std::env::temp_dir().join(format!("file-force-scrollbar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for index in 0..120 {
            std::fs::write(dir.join(format!("file-{index:03}.txt")), "x").unwrap();
        }
        let (view, cx) = rooted_window(&dir);
        cx.run_until_parked();
        let list = cx.read(|app| view.read(app).list_scroll.bounds());
        assert!(list.size.height > gpui_kit::px(0.), "list not laid out");
        // The scrollbar shows once the list scrolls: nudge it, then press on
        // the scrollbar at the right edge and drag down.
        cx.simulate_event(gpui_kit::ScrollEvent {
            position: list.center(),
            delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-2.))),
            ..Default::default()
        });
        cx.run_until_parked();
        let nudged = cx.read(|app| view.read(app).list_scroll.offset().y);
        let thumb = gpui_kit::point(list.right() - gpui_kit::px(8.), list.top() + gpui_kit::px(12.));
        let draw = |cx: &mut VisualTestContext| cx.update(|window, cx| window.draw(cx).clear(cx));
        let below = thumb + gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(160.));
        cx.simulate_mouse_move(thumb, None, gpui_kit::Modifiers::none());
        draw(cx);
        cx.simulate_mouse_down(thumb, gpui_kit::MouseButton::Left, gpui_kit::Modifiers::none());
        draw(cx);
        cx.simulate_mouse_move(below, Some(gpui_kit::MouseButton::Left), gpui_kit::Modifiers::none());
        draw(cx);
        cx.simulate_mouse_up(below, gpui_kit::MouseButton::Left, gpui_kit::Modifiers::none());
        cx.run_until_parked();
        let offset = cx.read(|app| view.read(app).list_scroll.offset().y);
        assert!(offset < nudged - gpui_kit::px(50.), "dragging the scrollbar did not scroll ({nudged:?} → {offset:?}): it is under the rows");
        assert!(cx.read(|app| view.read(app).selection.is_empty()), "the press went to a row instead of the scrollbar");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clicks_on_the_notifications_card_do_not_reach_the_list_behind() {
        let dir = std::env::temp_dir().join(format!("file-force-occlude-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for index in 0..80 {
            std::fs::write(dir.join(format!("file-{index:03}.txt")), "x").unwrap();
        }
        let (view, cx) = rooted_window(&dir);
        // Without the inspector the list runs under the card's corner.
        let id = files_sdk::log::notify("files", files_sdk::log::Level::Info, "Behind me is the list", Some("a body line".into()));
        view.update(cx, |this, cx| {
            this.show_inspector = false;
            this.notifications_open = true;
            cx.notify();
        });
        cx.run_until_parked();
        let item = cx.debug_bounds(Box::leak(format!("notification-{id}").into_boxed_str())).expect("card not shown");
        let list = cx.read(|app| view.read(app).list_scroll.bounds());
        assert!(list.contains(&item.center()), "the card should cover the list for this test");
        // Press and drag across the card, as when selecting its text.
        let start = item.origin + gpui_kit::point(gpui_kit::px(60.), gpui_kit::px(14.));
        cx.simulate_mouse_down(start, gpui_kit::MouseButton::Left, gpui_kit::Modifiers::none());
        cx.simulate_mouse_move(start + gpui_kit::point(gpui_kit::px(120.), gpui_kit::px(0.)), Some(gpui_kit::MouseButton::Left), gpui_kit::Modifiers::none());
        cx.simulate_mouse_up(start + gpui_kit::point(gpui_kit::px(120.), gpui_kit::px(0.)), gpui_kit::MouseButton::Left, gpui_kit::Modifiers::none());
        cx.run_until_parked();
        cx.simulate_click(start, gpui_kit::Modifiers::none());
        cx.run_until_parked();
        assert!(cx.read(|app| view.read(app).selected.is_none() && view.read(app).selection.is_empty()), "a file behind the card got the click");
        files_sdk::log::dismiss(id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_browser_sign_in_shows_its_link_and_the_connection_continues() {
        let dir = std::env::temp_dir().join(format!("file-force-ssh-auth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::ssh::use_fake_ssh(&dir);
        let (view, cx) = rooted_window(&dir);
        view.update_in(cx, |this, window, cx| this.connect_saved_ssh("me@auth-host:22", window, cx));
        let dialog = cx.read(|app| view.read(app).ssh_dialog.clone()).expect("the SSH dialog did not open");
        let mut saw_link = None;
        for _ in 0..200 {
            cx.executor().advance_clock(std::time::Duration::from_millis(200));
            cx.run_until_parked();
            if saw_link.is_none() {
                saw_link = cx.read(|app| dialog.read(app).auth_for_test()).map(|auth| auth.url);
            }
            if cx.read(|app| view.read(app).ssh_connection.is_some()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(saw_link.as_deref(), Some("https://login.tailscale.com/a/test123"), "the sign-in link was not shown");
        assert!(cx.read(|app| view.read(app).ssh_connection.is_some()), "it did not connect after the sign-in");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pro_extension_stays_locked_without_a_license() {
        use files_sdk::{Accessibility, Extension, Icon, Manifest, MenuItem, Scope, Targets, Tier};
        struct Kube;
        static MENU: [MenuItem; 1] = [MenuItem {
            command: "kube.pods",
            title: "Kube: show pods",
            icon: None,
            targets: Targets::ALL,
            scope: Scope::Anywhere,
            accessibility: Accessibility { label: None, hint: None },
        }];
        static MANIFEST: Manifest = Manifest {
            id: "kube-test",
            name: "Kube test",
            version: "1.0.0",
            source: "Marketplace",
            summary: "",
            icon: Icon::Builtin("cloud"),
            adds: &[],
            permissions: &[],
            commands: &[],
            menu: &MENU,
            sidebar: &[],
            menu_color: None,
            tier: Tier::Pro { product: "kube-test" },
        };
        impl Extension for Kube {
            fn manifest(&self) -> &'static Manifest {
                &MANIFEST
            }
        }
        static KUBE: Kube = Kube;
        crate::views::extension_ui::test_support::register(&KUBE);
        let (view, cx) = pro_window();
        let offered = |cx: &mut VisualTestContext| {
            cx.read(|app| {
                view.read(app)
                    .extension_menu_items(Targets::FILE, Location::Home, app)
                    .iter()
                    .any(|(_, item)| item.command == "kube.pods")
            })
        };
        assert!(!offered(cx), "a Pro extension must not appear without a license");
        // A key that looks right but is not signed by the publisher changes nothing.
        let (private, _) = files_sdk::license::generate_keys().unwrap();
        let forged = files_sdk::license::issue(
            &files_sdk::license::License { id: "X".into(), email: "x@y.z".into(), products: vec!["*".into()], issued: 0, expires: None },
            &private,
        )
        .unwrap();
        view.update(cx, |this, cx| this.appearance.update(cx, |settings, _| settings.license.key = forged));
        assert!(!offered(cx), "a self-made key unlocked a Pro extension");
        assert!(matches!(
            cx.read(|app| crate::licensing::status(view.read(app).appearance.read(app))),
            Some(Err(files_sdk::license::LicenseError::BadSignature))
        ));
    }

    #[test]
    fn about_and_update_dialogs_open() {
        let (view, cx) = rooted_window(&std::env::temp_dir());
        view.update_in(cx, |this, window, cx| this.open_about_dialog(window, cx));
        cx.run_until_parked();
        assert!(cx.update(|window, cx| window.has_active_dialog(cx)), "About did not open");
        cx.update(|window, cx| window.close_dialog(cx));
        cx.run_until_parked();
        view.update_in(cx, |this, window, cx| this.open_update_dialog(window, cx));
        for _ in 0..100 {
            cx.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(cx.update(|window, cx| window.has_active_dialog(cx)), "Check for Updates did not open");
        // Daily checks: due after a day, never when turned off.
        let mut updates = crate::settings::UpdateSettings::default();
        let configured = !crate::updates::UPDATE_URL.is_empty() || crate::updates::github_repository().is_some();
        assert_eq!(crate::updates::auto_check_due(&updates), configured);
        updates.last_check = crate::updates::now_seconds();
        assert!(!crate::updates::auto_check_due(&updates), "checked just now");
        updates.last_check = 0;
        updates.automatic = false;
        assert!(!crate::updates::auto_check_due(&updates), "turned off");
    }

    #[test]
    fn the_settings_menu_scrolls_when_extensions_are_unfolded() {
        let (_view, cx) = rooted_window(&std::env::temp_dir());
        // A short window: the unfolded list cannot fit.
        cx.simulate_resize(gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(460.)));
        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-," } else { "ctrl-," });
        cx.run_until_parked();
        let header = cx.debug_bounds("settings-extensions-header").expect("settings menu not shown");
        cx.simulate_click(header.center(), gpui_kit::Modifiers::none());
        cx.run_until_parked();
        let before = cx.debug_bounds("settings-menu-items").unwrap();
        cx.simulate_event(gpui_kit::ScrollEvent {
            position: header.center(),
            delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-300.))),
            ..Default::default()
        });
        cx.run_until_parked();
        let after = cx.debug_bounds("settings-menu-items").unwrap();
        assert!(after.origin.y < before.origin.y, "the settings menu did not scroll: {before:?} → {after:?}");
    }
}
