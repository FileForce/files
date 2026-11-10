use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, actions, div, px,
};

use crate::app::AppearanceSettings;
use crate::model::{Accent, ColorScheme};
use crate::palette::{Palette, apply_theme};
use crate::settings::{
    AppSettings, ClockFormat, DateFormat, Language, SizeBase, format_demo_date, format_size,
    localized as tr, settings_path,
};

/// Tabs in the sidebar: General … Accessibility, Developer.
const SECTION_COUNT: usize = 11;
const EXTENSIONS_TAB: usize = 2;

actions!(
    settings_window,
    [
        CloseSettings,
        SettingsPrevious,
        SettingsNext,
        SettingsFocusNext,
        SettingsFocusPrevious
    ]
);

pub struct SettingsWindow {
    /// Shown in a tab, not the modal: Close and Cancel keep the tab.
    pub(crate) in_tab: bool,
    /// Settings → General → Command line: what Install or Uninstall said.
    cli_message: Option<String>,
    /// Settings → Extensions → License: where the key is pasted.
    license_input: Entity<InputState>,
    /// Settings → General → Check now: `None` until asked, then the answer.
    update_check: Option<Result<crate::updates::UpdateStatus, String>>,
    checking_updates: bool,
    /// What "Download & Install" did.
    update_install: Option<Result<std::path::PathBuf, String>>,
    installing_update: bool,
    /// Installed plugin packages and what happened to them.
    plugins: Vec<crate::plugins::Installed>,
    installing: bool,
    /// (worked, text) after Install.
    install_message: Option<(bool, String)>,
    /// Settings → Developer: the shell's prompt and what it printed.
    shell_input: Entity<InputState>,
    shell_output: Vec<(String, String)>,
    _shell_subscription: Subscription,
    appearance: Entity<AppearanceSettings>,
    original: AppSettings,
    draft: AppSettings,
    active_tab: usize,
    error: Option<String>,
    new_pattern: Entity<InputState>,
    new_extension: Entity<InputState>,
    /// The Default apps row whose app list is unfolded.
    expanded_extension: Option<String>,
    /// Where focus goes back to when the modal closes.
    return_focus: Option<FocusHandle>,
    /// Set by Save so the footer can confirm it; cleared by the next edit.
    just_saved: bool,
    /// Settings → Storage: the caches measured (`None` while measuring).
    storage_usage: Option<Vec<(crate::storage::Kind, crate::storage::Usage)>>,
    /// Each extension's own cache (`files_sdk::storage`).
    storage_extensions: Vec<crate::storage::ExtensionUsage>,
    storage_slider: Entity<gpui_kit::component::slider::SliderState>,
    /// Appearance → Font: every system font, searchable.
    font_select: Entity<gpui_kit::component::select::SelectState<gpui_kit::component::select::SearchableVec<String>>>,
    _font_subscription: Subscription,
    /// Linux: whether folders open in Files system-wide, and why making it
    /// so failed.
    default_file_manager: bool,
    default_file_manager_error: Option<String>,
    _storage_subscription: Subscription,
    /// Appearance: panel opacity slider and custom background color.
    opacity_slider: Entity<gpui_kit::component::slider::SliderState>,
    /// Settings → General → Power saving: below which charge it starts.
    power_slider: Entity<gpui_kit::component::slider::SliderState>,
    /// Settings → Sound: the notification volume, and the shared equalizer.
    sound_volume: Entity<gpui_kit::component::slider::SliderState>,
    _sound_subscription: Subscription,
    equalizer: Entity<crate::views::equalizer::EqualizerPanel>,
    _power_subscription: Subscription,
    text_scale_slider: Entity<gpui_kit::component::slider::SliderState>,
    _text_scale_subscription: Subscription,
    background_hex: Entity<InputState>,
    _opacity_subscription: Subscription,
    /// Why the last recorded shortcut was refused.
    hotkey_error: Option<String>,
    /// Keystrokes of a sequence being recorded (`cmd-n f f`).
    recorded: Vec<String>,
    /// Bumped per recorded key; a pause commits only if no key followed.
    record_generation: u64,
    /// Extensions in the side menu: unfolded, and which extension's page shows
    /// (`None` = the overview).
    extensions_expanded: bool,
    extension_page: Option<usize>,
    install_url: Entity<InputState>,
    /// Hotkeys tab: search box, and the command whose shortcut is being recorded.
    hotkey_query: Entity<InputState>,
    recording: Option<&'static str>,
    recorder_focus: FocusHandle,
    show_builtin_keys: bool,
    focus_handle: FocusHandle,
    _appearance_subscription: Subscription,
}

impl SettingsWindow {
    pub(crate) fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        appearance: Entity<AppearanceSettings>,
    ) -> Self {
        let current = appearance.read(cx).clone();
        crate::default_apps::prefetch();
        // The OS switches may have changed since launch.
        crate::settings::refresh_system_accessibility();
        let opacity_slider = cx.new(|_| {
            gpui_kit::component::slider::SliderState::new()
                .min(crate::settings::LookSettings::MIN_OPACITY as f32)
                .max(100.)
                .step(5.)
                .default_value(current.look.panel_opacity as f32)
        });
        let opacity_subscription = cx.subscribe(&opacity_slider, |this: &mut Self, _, event, cx| {
            if let gpui_kit::component::slider::SliderEvent::Change(value) = event {
                this.draft.look.panel_opacity = value.end().round() as u8;
                cx.notify();
            }
        });
        let power_slider = cx.new(|_| {
            gpui_kit::component::slider::SliderState::new().min(5.).max(100.).step(5.).default_value(current.power_saving_below as f32)
        });
        let power_subscription = cx.subscribe(&power_slider, |this: &mut Self, _, event, cx| {
            if let gpui_kit::component::slider::SliderEvent::Change(value) = event {
                this.draft.power_saving_below = value.end().round() as u8;
                cx.notify();
            }
        });
        let sound_volume = cx.new(|_| {
            gpui_kit::component::slider::SliderState::new().min(0.).max(100.).step(5.).default_value(current.sound.volume as f32)
        });
        let sound_subscription = cx.subscribe(&sound_volume, |this: &mut Self, _, event, cx| {
            if let gpui_kit::component::slider::SliderEvent::Change(value) = event {
                this.draft.sound.volume = value.end().round() as u8;
                cx.notify();
            }
        });
        let equalizer = {
            let appearance = appearance.clone();
            cx.new(|cx| crate::views::equalizer::EqualizerPanel::new(appearance, cx))
        };
        // Text size: a slider whose steps are the presets.
        let text_scale_slider = cx.new(|_| {
            gpui_kit::component::slider::SliderState::new()
                .min(0.)
                .max((crate::settings::TextScale::ALL.len() - 1) as f32)
                .step(1.)
                .default_value(current.accessibility.text_scale.index() as f32)
        });
        let text_scale_subscription = cx.subscribe(&text_scale_slider, |this: &mut Self, _, event, cx| {
            if let gpui_kit::component::slider::SliderEvent::Change(value) = event {
                this.draft.accessibility.text_scale =
                    crate::settings::TextScale::from_index(value.end().round() as usize);
                cx.notify();
            }
        });
        // Font: the theme's, or any system font (searchable).
        let fonts = font_choices(cx);
        let selected = current.font.as_ref().and_then(|font| fonts.iter().position(|choice| choice == font)).unwrap_or(0);
        let font_select = cx.new(|cx| {
            gpui_kit::component::select::SelectState::new(
                gpui_kit::component::select::SearchableVec::new(fonts),
                Some(gpui_kit::component::IndexPath::new(selected)),
                window,
                cx,
            )
            .searchable(true)
        });
        let font_subscription = cx.subscribe(&font_select, |this: &mut Self, _, event, cx| {
            let gpui_kit::component::select::SelectEvent::Confirm(choice) = event;
            this.draft.font = choice.clone().filter(|font| font != THEME_FONT);
            cx.notify();
        });
        // Cache limit: a slider whose steps are the limits.
        let storage_slider = cx.new(|_| {
            gpui_kit::component::slider::SliderState::new()
                .min(0.)
                .max((STORAGE_LIMITS.len() - 1) as f32)
                .step(1.)
                .default_value(storage_step(current.storage.limit_mb) as f32)
        });
        let storage_subscription = cx.subscribe(&storage_slider, |this: &mut Self, _, event, cx| {
            if let gpui_kit::component::slider::SliderEvent::Change(value) = event {
                this.draft.storage.limit_mb = STORAGE_LIMITS[(value.end().round() as usize).min(STORAGE_LIMITS.len() - 1)];
                cx.notify();
            }
        });
        let background_hex = cx.new(|cx| {
            let hex = match &current.look.background {
                crate::settings::Background::Color { hex } => hex.clone(),
                _ => String::new(),
            };
            InputState::new(window, cx).placeholder("#1e293b").default_value(hex)
        });
        let appearance_subscription = cx.observe(&appearance, |_, _, cx| cx.notify());
        let focus_handle = cx.focus_handle().tab_stop(true);
        window.focus(&focus_handle, cx);
        let shell_input = cx.new(|cx| InputState::new(window, cx).placeholder("help · log · ext · perm ssh · notify hi · run git --version"));
        let shell_subscription = cx.subscribe_in(&shell_input, window, |this: &mut Self, _, event, window, cx| {
            if matches!(event, gpui_kit::component::input::InputEvent::PressEnter { .. }) {
                this.run_shell(window, cx);
            }
        });
        Self {
            license_input: cx.new(|cx| InputState::new(window, cx).placeholder("FILES1.…  (license key)")),
            plugins: crate::plugins::load_installed(&current),
            update_check: None,
            checking_updates: false,
            update_install: None,
            installing_update: false,
            installing: false,
            install_message: None,
            shell_input,
            shell_output: vec![("help".to_string(), crate::dev_shell::HELP.to_string())],
            _shell_subscription: shell_subscription,
            appearance,
            original: current.clone(),
            draft: current,
            active_tab: 0,
            error: crate::settings::load_error(),
            new_extension: cx.new(|cx| InputState::new(window, cx).placeholder("Extension, e.g. odt")),
            expanded_extension: None,
            return_focus: None,
            just_saved: false,
            in_tab: false,
            cli_message: None,
            storage_usage: None,
            storage_extensions: Vec::new(),
            storage_slider,
            font_select,
            _font_subscription: font_subscription,
            default_file_manager: crate::desktop::is_default_file_manager(),
            default_file_manager_error: None,
            _storage_subscription: storage_subscription,
            opacity_slider,
            power_slider,
            _power_subscription: power_subscription,
            sound_volume,
            _sound_subscription: sound_subscription,
            equalizer,
            text_scale_slider,
            _text_scale_subscription: text_scale_subscription,
            background_hex,
            _opacity_subscription: opacity_subscription,
            hotkey_error: None,
            recorded: Vec::new(),
            record_generation: 0,
            extensions_expanded: false,
            extension_page: None,
            install_url: cx.new(|cx| {
                InputState::new(window, cx).placeholder("/path/to/extension.filesext or https://…/extension.filesext")
            }),
            hotkey_query: cx.new(|cx| InputState::new(window, cx).placeholder("Search commands or keys")),
            recording: None,
            recorder_focus: cx.focus_handle(),
            show_builtin_keys: false,
            new_pattern: cx.new(|cx| {
                InputState::new(window, cx).placeholder("e.g. ~/Projects/** or ~/.config/*.toml")
            }),
            focus_handle,
            _appearance_subscription: appearance_subscription,
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        // A conflicting keymap (e.g. from a hand-edited settings.toml) blocks saving.
        if !crate::keymap::conflicts(&self.draft.keymap, cx.platform_info().is_mac()).is_empty() {
            self.error = Some("Hotkey conflicts must be fixed before saving (Settings → Hotkeys)".to_string());
            cx.notify();
            return;
        }
        match crate::settings::save(&self.draft) {
            Ok(()) => {
                self.original = self.draft.clone();
                self.error = None;
                let draft = self.draft.clone();
                draft.apply_permissions();
                // A new license may unlock installed Pro plugins.
                self.plugins = crate::plugins::load_installed(&draft);
                self.appearance.update(cx, |appearance, cx| {
                    *appearance = draft;
                    cx.notify();
                });
                crate::palette::set_look(self.draft.theme, self.draft.icon_theme);
                crate::palette::set_font(self.draft.font.clone());
                crate::palette::set_corners(self.draft.corners);
                apply_theme(cx, self.draft.scheme, self.draft.accent);
                cx.notify();
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn toggle_theme_for_test(&mut self, cx: &mut Context<Self>) {
        self.draft.scheme = self.draft.scheme.toggled();
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn save_for_test(&mut self, cx: &mut Context<Self>) {
        self.save(cx);
    }

    #[cfg(test)]
    pub(crate) fn hotkey_error_for_test(&self) -> Option<String> {
        self.hotkey_error.clone()
    }

    /// Puts the sliders back on the draft values (after a reset or cancel).
    fn sync_sliders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let opacity = self.draft.look.panel_opacity as f32;
        let scale = self.draft.accessibility.text_scale.index() as f32;
        self.opacity_slider.update(cx, |slider, cx| slider.set_value(opacity, window, cx));
        let below = self.draft.power_saving_below as f32;
        self.power_slider.update(cx, |slider, cx| slider.set_value(below, window, cx));
        self.text_scale_slider.update(cx, |slider, cx| slider.set_value(scale, window, cx));
        let step = storage_step(self.draft.storage.limit_mb) as f32;
        self.storage_slider.update(cx, |slider, cx| slider.set_value(step, window, cx));
    }

    /// Closes the modal and gives focus back, so shortcuts work without a click.
    fn dismiss(&self, window: &mut Window, cx: &mut Context<Self>) {
        // In a tab there is no modal to close: the tab stays until closed.
        if self.in_tab {
            return;
        }
        window.close_dialog(cx);
        if let Some(focus) = &self.return_focus {
            focus.focus(window, cx);
        }
    }

    fn close(&mut self, _: &CloseSettings, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(window, cx);
    }

    fn select_section(&mut self, index: usize, cx: &mut Context<Self>) {
        self.active_tab = index.min(SECTION_COUNT - 1);
        if self.active_tab == STORAGE_TAB {
            self.measure_storage(cx);
        }
        cx.notify();
    }

    /// Measures the caches in the background (or clears one first).
    fn measure_storage(&mut self, cx: &mut Context<Self>) {
        self.run_storage(|| {}, cx);
    }

    fn run_storage(&mut self, work: impl FnOnce() + Send + 'static, cx: &mut Context<Self>) {
        let task = cx.background_executor().spawn(async move {
            work();
            (crate::storage::usage(), crate::storage::extensions())
        });
        cx.spawn(async move |this, cx| {
            let (usage, extensions) = task.await;
            let _ = this.update(cx, |this, cx| {
                this.storage_usage = Some(usage);
                this.storage_extensions = extensions;
                cx.notify();
            });
        })
        .detach();
    }

    /// Settings → Storage: a ring of what the caches hold, each kind with its
    /// size and a Clear button, and the limit they stay under.
    fn render_storage(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        use crate::storage::Kind;
        let language = self.draft.language;
        let color = move |kind: Kind| match kind {
            Kind::Thumbnails => palette.video,
            Kind::Extensions => palette.accent,
            Kind::Forks => palette.archive,
            Kind::SearchIndex => palette.document,
            Kind::Sounds => palette.code,
        };
        let usage = self.storage_usage.clone().unwrap_or_default();
        let total: u64 = usage.iter().map(|(_, usage)| usage.bytes).sum();
        let measured = self.storage_usage.is_some();
        let slices: Vec<(Kind, f32)> = usage.iter().filter(|(_, usage)| usage.bytes > 0).map(|(kind, usage)| (*kind, usage.bytes as f32)).collect();
        let ring = div()
            .relative()
            .size(px(180.))
            .flex_shrink_0()
            .child(if slices.is_empty() {
                div().size_full().rounded_full().border(px(28.)).border_color(palette.surface).into_any_element()
            } else {
                gpui_kit::component::chart::PieChart::new(slices)
                    .id("storage-ring")
                    .inner_radius(54.)
                    .outer_radius(84.)
                    .pad_angle(0.02)
                    .value(|slice: &(Kind, f32)| slice.1)
                    .color(move |slice: &(Kind, f32)| color(slice.0))
                    .tooltip_name(move |slice: &(Kind, f32)| slice.0.label(language).into())
                    .tooltip_value(|slice: &(Kind, f32), _, _| crate::resources::size(slice.1 as u64).into())
                    .into_any_element()
            })
            .child(
                v_flex()
                    .absolute()
                    .inset_0()
                    .items_center()
                    .justify_center()
                    .child(div().text_lg().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(if measured { crate::resources::size(total) } else { "…".to_string() }))
                    .child(div().text_xs().text_color(palette.muted).child(tr(language, "in cache", "в кэше"))),
            );
        let rows = usage.iter().map(|(kind, usage)| {
            let kind = *kind;
            let share = if total > 0 { usage.bytes as f32 / total as f32 } else { 0. };
            h_flex()
                .w_full()
                .gap_3()
                .items_center()
                .child(div().size(px(10.)).flex_shrink_0().rounded_full().bg(color(kind)))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_sm().child(kind.label(language)))
                        .child(div().text_xs().text_color(palette.muted).child(kind.note(language))),
                )
                .child(
                    v_flex()
                        .items_end()
                        .child(div().text_sm().child(crate::resources::size(usage.bytes)))
                        .child(div().text_xs().text_color(palette.faint).child(format!("{} · {:.0}%", files_count(usage.files, language), share * 100.))),
                )
                .when(!kind.clearable(), |this| this.child(div().w(px(84.))))
                .when(kind.clearable(), |this| this.child(
                    Button::new(gpui_kit::ElementId::Name(format!("storage-clear-{kind:?}").into()))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Trash)
                        .label(tr(language, "Clear", "Очистить"))
                        .disabled(usage.files == 0)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.run_storage(move || {
                                crate::storage::clear(kind);
                            }, cx);
                        })),
                ))
        });
        let limit = self.draft.storage.limit_mb;
        // Extensions one by one, under the Extensions row, so an SDK author
        // sees which cache grows.
        let extension_rows = self.storage_extensions.iter().map(|extension| {
            let id = extension.id.clone();
            h_flex()
                .w_full()
                .pl(px(22.))
                .gap_3()
                .items_center()
                .child(Icon::new(IconName::Puzzle).xsmall().text_color(palette.muted))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_sm().child(extension.name.clone()))
                        .child(div().text_xs().text_color(palette.faint).truncate().child(extension.id.clone())),
                )
                .child(
                    v_flex()
                        .items_end()
                        .child(div().text_sm().child(crate::resources::size(extension.usage.bytes)))
                        .child(div().text_xs().text_color(palette.faint).child(files_count(extension.usage.files, language))),
                )
                .child(
                    Button::new(gpui_kit::ElementId::Name(format!("storage-clear-ext-{id}").into()))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Trash)
                        .label(tr(language, "Clear", "Очистить"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let id = id.clone();
                            this.run_storage(move || {
                                crate::storage::clear_extension(&id);
                            }, cx);
                        })),
                )
        });
        let mut rows: Vec<AnyElement> = rows.map(IntoElement::into_any_element).collect();
        if let Some(at) = usage.iter().position(|(kind, _)| *kind == Kind::Extensions) {
            rows.splice(at + 1..at + 1, extension_rows.map(IntoElement::into_any_element));
        }
        let current_limit = limit_label(limit, language);
        let ticks = STORAGE_LIMITS.map(|megabytes| div().w(px(2.)).h(px(6.)).rounded_full().bg(if megabytes == limit { palette.accent } else { palette.border }));
        v_flex()
            .p_5()
            .gap_4()
            .child(
                card(palette)
                    .child(section(IconName::HardDrive, tr(language, "Cache on this computer", "Кэш на этом компьютере"), palette))
                    .child(h_flex().w_full().gap_6().items_center().child(ring).child(v_flex().flex_1().min_w_0().gap_3().children(rows)))
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(div().flex_1())
                            .child(Button::new("storage-refresh").ghost().small().icon(IconName::RefreshCw).label(tr(language, "Refresh", "Обновить")).on_click(cx.listener(|this, _, _, cx| this.measure_storage(cx))))
                            .child(
                                Button::new("storage-clear-all").danger().small().icon(IconName::Trash).label(tr(language, "Clear all", "Очистить всё")).disabled(total == 0).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.run_storage(|| {
                                            for kind in Kind::ALL {
                                                crate::storage::clear(kind);
                                            }
                                        }, cx);
                                    }),
                                ),
                            ),
                    ),
            )
            .child(
                card(palette)
                    .child(section(IconName::Gauge, tr(language, "Limit", "Лимит"), palette))
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(div().text_xs().text_color(palette.muted).child(limit_label(STORAGE_LIMITS[0], language)))
                            .child(div().flex_1().child(gpui_kit::component::slider::Slider::new(&self.storage_slider)))
                            .child(div().text_xs().text_color(palette.muted).child(limit_label(0, language))),
                    )
                    .child(h_flex().px(px(18.)).justify_between().children(ticks))
                    .child(
                        h_flex()
                            .justify_center()
                            .gap_2()
                            .text_sm()
                            .child(tr(language, "Keep the cache under", "Держать кэш не больше"))
                            .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(palette.accent).child(current_limit)),
                    )
                    .child(note(
                        tr(
                            language,
                            "Past the limit, files used least recently are deleted first. Your own files are never touched: only what Files downloaded or made.",
                            "Сверх лимита первыми удаляются давно не использованные файлы. Ваши файлы не затрагиваются — только то, что Files скачал или создал сам.",
                        ),
                        palette,
                    )),
            )
            .into_any_element()
    }

    #[cfg(test)]
    pub(crate) fn set_layout_for_test(&mut self, layout: crate::settings::SpeakerLayout, cx: &mut Context<Self>) {
        self.draft.sound.layout = layout;
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn select_for_test(&mut self, index: usize, cx: &mut Context<Self>) -> usize {
        self.select_section(index, cx);
        self.active_tab
    }

    fn previous_section(&mut self, _: &SettingsPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.select_section(self.active_tab.saturating_sub(1), cx);
    }

    fn next_section(&mut self, _: &SettingsNext, _: &mut Window, cx: &mut Context<Self>) {
        self.select_section((self.active_tab + 1).min(SECTION_COUNT - 1), cx);
    }

    fn focus_next(&mut self, _: &SettingsFocusNext, window: &mut Window, cx: &mut Context<Self>) {
        window.focus_next(cx);
    }

    fn focus_previous(
        &mut self,
        _: &SettingsFocusPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus_prev(cx);
    }

    fn section_menu_item(
        &self,
        index: usize,
        icon: IconName,
        label: &'static str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.active_tab == index;
        h_flex()
            .id(("settings-section", index))
            .h(px(32.))
            .px_3()
            .gap_2()
            .items_center()
            .rounded_md()
            .text_sm()
            .font_weight(if active {
                FontWeight::MEDIUM
            } else {
                FontWeight::NORMAL
            })
            .text_color(if active {
                palette.accent
            } else {
                palette.muted
            })
            .bg(if active {
                palette.selection
            } else {
                gpui_kit::transparent_black()
            })
            .hover(|style| style.bg(palette.hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_section(index, cx);
            }))
            .child(Icon::new(icon).small())
            .child(label)
            .into_any_element()
    }

    /// "Extensions" in the side menu: an accordion whose children open each
    /// extension's own page.
    fn extensions_menu(
        &self,
        icon: IconName,
        label: &'static str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on_tab = self.active_tab == EXTENSIONS_TAB;
        let overview = on_tab && self.extension_page.is_none();
        let mut menu = v_flex().gap_0p5().child(
            h_flex()
                .id(("settings-section", EXTENSIONS_TAB))
                .debug_selector(|| "settings-extensions-header".into())
                .h(px(32.))
                .px_3()
                .gap_2()
                .items_center()
                .rounded_md()
                .text_sm()
                .font_weight(if overview { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                .text_color(if on_tab { palette.accent } else { palette.muted })
                .bg(if overview { palette.selection } else { gpui_kit::transparent_black() })
                .hover(|style| style.bg(palette.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    // Opening shows the overview; clicking the open overview folds it.
                    if this.active_tab == EXTENSIONS_TAB && this.extension_page.is_none() {
                        this.extensions_expanded = !this.extensions_expanded;
                    } else {
                        this.extensions_expanded = true;
                    }
                    this.active_tab = EXTENSIONS_TAB;
                    this.extension_page = None;
                    cx.notify();
                }))
                .child(Icon::new(icon).small())
                .child(div().flex_1().child(label))
                .child(
                    Icon::new(if self.extensions_expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                        .xsmall()
                        .text_color(palette.faint),
                ),
        );
        if self.extensions_expanded {
            for (index, extension) in crate::extensions::CATALOG.iter().enumerate() {
                let selected = on_tab && self.extension_page == Some(index);
                let enabled = crate::extensions::enabled(&self.draft.extensions, index);
                menu = menu.child(
                    h_flex()
                        .id(("settings-extension", index))
                        .h(px(28.))
                        .pl(px(30.))
                        .pr_3()
                        .gap_2()
                        .items_center()
                        .rounded_md()
                        .text_xs()
                        .text_color(if selected { palette.accent } else { palette.muted })
                        .bg(if selected { palette.selection } else { gpui_kit::transparent_black() })
                        .hover(|style| style.bg(palette.hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.active_tab = EXTENSIONS_TAB;
                            this.extension_page = Some(index);
                            cx.notify();
                        }))
                        .child(crate::extensions::icon_of(extension).xsmall())
                        .child(div().flex_1().truncate().child(extension.name))
                        .child(
                            div()
                                .size(px(6.))
                                .rounded_full()
                                .bg(if enabled { palette.code } else { palette.faint }),
                        ),
                );
            }
        }
        menu.into_any_element()
    }

    fn render_general(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let language = self.draft.language;
        v_flex()
            .p_5()
            .gap_4()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(section(IconName::RefreshCw, tr(language, "Updates", "Обновления"), palette))
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(palette.faint).child(format!("Version {}", crate::updates::CURRENT_VERSION))),
            )
            .child(self.render_update_check(palette, cx))
            .child(
                Switch::new("updates-automatic")
                    .label(tr(language, "Check for updates automatically (once a day)", "Проверять обновления автоматически (раз в день)"))
                    .checked(self.draft.updates.automatic)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.draft.updates.automatic = *checked;
                        cx.notify();
                    })),
            )
            .child(section(IconName::AppWindow, tr(language, "Windows", "Окна"), palette))
            .child(
                Switch::new("single-instance")
                    .label(tr(
                        language,
                        "One Files at a time: opening a folder or Files again uses the running one (a new tab) (after a restart)",
                        "Один экземпляр Files: папка или повторный запуск открываются в уже запущенном (новой вкладкой) (после перезапуска)",
                    ))
                    .checked(self.draft.single_instance)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.draft.single_instance = *checked;
                        cx.notify();
                    })),
            )
            .child(section(IconName::Zap, tr(language, "Power saving", "Энергосбережение"), palette))
            .child(div().debug_selector(|| "settings-power-saving".into()).child(setting_row(
                tr(language, "Background activity and animations", "Фоновая активность и анимации"),
                h_flex().gap_2().children([
                    (crate::settings::PowerSavingMode::Automatic, tr(language, "Automatic", "Автоматически")),
                    (crate::settings::PowerSavingMode::On, tr(language, "On", "Включено")),
                    (crate::settings::PowerSavingMode::Off, tr(language, "Off", "Выключено")),
                ].map(|(mode, label)| {
                    let id = match mode {
                        crate::settings::PowerSavingMode::Automatic => "power-saving-automatic",
                        crate::settings::PowerSavingMode::On => "power-saving-on",
                        crate::settings::PowerSavingMode::Off => "power-saving-off",
                    };
                    div().debug_selector(move || id.into()).child(
                        option_button(id, label, self.draft.power_saving == mode).on_click(cx.listener(move |this, _, _, cx| {
                            this.draft.power_saving = mode;
                            cx.notify();
                        })),
                    )
                })),
                palette,
            )))
            .when(self.draft.power_saving == crate::settings::PowerSavingMode::Automatic, |this| {
                let below = self.draft.power_saving_below;
                let battery = crate::power::battery();
                this.child(div().debug_selector(|| "settings-power-below".into()).child(setting_row(
                    tr(language, "Turn on when the battery is at", "Включать при заряде"),
                    h_flex()
                        .flex_1()
                        .gap_3()
                        .items_center()
                        .child(div().flex_1().child(gpui_kit::component::slider::Slider::new(&self.power_slider)))
                        .child(div().w(px(140.)).text_xs().text_color(palette.muted).child(if below >= 100 {
                            tr(language, "any charge, on battery", "любом, от батареи").to_string()
                        } else {
                            format!("{} {below}%", tr(language, "or below", "или ниже"))
                        })),
                    palette,
                )))
                .child(div().debug_selector(|| "settings-power-now".into()).text_xs().text_color(palette.faint).child(
                    match (battery.percent, battery.on_battery) {
                        (Some(percent), true) => format!("{} {percent}% · {}", tr(language, "Now:", "Сейчас:"), tr(language, "on battery", "от батареи")),
                        (Some(percent), false) => format!("{} {percent}% · {}", tr(language, "Now:", "Сейчас:"), tr(language, "plugged in", "от сети")),
                        (None, _) => tr(language, "No battery found: power saving stays off in Automatic.", "Батарея не найдена: в автоматическом режиме энергосбережение не включится.").to_string(),
                    },
                ))
            })
            .child(note(tr(
                language,
                "Automatic reduces background work and turns off animations while on battery power, once the charge drops to the level above.",
                "В автоматическом режиме снижает фоновую активность и отключает анимации при питании от батареи, когда заряд опускается до уровня выше.",
            ), palette))
            .child(section(IconName::Languages, tr(language, "Language", "Язык"), palette))
            .child(setting_row(
                tr(language, "Interface language", "Язык интерфейса"),
                h_flex()
                    .gap_2()
                    .child(option_button("language-en", "English", language == Language::English).on_click(cx.listener(|this, _, _, cx| {
                        this.draft.language = Language::English;
                        cx.notify();
                    })))
                    .child(option_button("language-ru", "Русский", language == Language::Russian).on_click(cx.listener(|this, _, _, cx| {
                        this.draft.language = Language::Russian;
                        cx.notify();
                    }))),
                palette,
            ))
            .child(note(tr(language, "Applies to Settings and key Explorer, search and Properties labels; this is not a full translation.", "Применяется к Settings и основным подписям Explorer, поиска и Properties; это не полный перевод."), palette))
            .child(section(IconName::MousePointerClick, tr(language, "Behavior", "Поведение"), palette))
            .child(setting_row(
                tr(language, "Default view", "Вид по умолчанию"),
                h_flex().gap_2().children(
                    [
                        (crate::model::ViewMode::Expressive, "Expressive", IconName::LayoutGrid),
                        (crate::model::ViewMode::Pro, "Pro", IconName::Rows3),
                        (crate::model::ViewMode::Dual, "Dual", IconName::Columns2),
                    ]
                    .map(|(mode, label, icon)| {
                        option_button(
                            gpui_kit::ElementId::Name(format!("default-view-{label}").into()),
                            label,
                            self.draft.behavior.default_view == mode,
                        )
                        .icon(icon)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.draft.behavior.default_view = mode;
                            cx.notify();
                        }))
                    }),
                ),
                palette,
            ))
            .child(note(
                tr(language, "New windows and tabs open in this view.", "Новые окна и вкладки открываются в этом виде."),
                palette,
            ))
            .child(setting_row(
                tr(language, "Global search folder", "Папка глобального поиска"),
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .max_w(px(260.))
                            .truncate()
                            .text_sm()
                            .text_color(palette.muted)
                            .child(self.draft.search_filters.global_root().map(|root| root.display().to_string()).unwrap_or_default()),
                    )
                    .child(
                        Button::new("global-search-folder")
                            .small()
                            .icon(IconName::FolderOpen)
                            .label(tr(language, "Choose…", "Выбрать…"))
                            .on_click(cx.listener(|this, _, _, cx| this.choose_global_search_folder(cx))),
                    )
                    .when(self.draft.search_filters.global_folder.is_some(), |this| {
                        this.child(Button::new("global-search-folder-home").ghost().small().label(tr(language, "Home", "Домашняя")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.draft.search_filters.global_folder = None;
                                cx.notify();
                            },
                        )))
                    }),
                palette,
            ))
            .child(note(
                tr(language, "Where ⌘K searches. In the search window, Change… picks another folder just once.", "Где ищет ⌘K. В окне поиска «Сменить…» выбирает другую папку разово."),
                palette,
            ))
            .child(setting_row(
                tr(language, "Dropping from another window", "Перетаскивание из другого окна"),
                h_flex().gap_2().children(crate::settings::DropAction::ALL.map(|action| {
                    option_button(
                        gpui_kit::ElementId::Name(format!("drop-action-{}", action.label()).into()),
                        action.label(),
                        self.draft.behavior.drop_action == action,
                    )
                    .icon(match action {
                        crate::settings::DropAction::Copy => IconName::Copy,
                        crate::settings::DropAction::Move => IconName::MoveRight,
                        crate::settings::DropAction::Finder => IconName::HardDrive,
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.draft.behavior.drop_action = action;
                        cx.notify();
                    }))
                })),
                palette,
            ))
            .child(
                Switch::new("ask-on-drop")
                    .label(tr(language, "Ask Copy or Move on every drop", "Спрашивать «копировать или переместить» при каждом переносе"))
                    .checked(self.draft.behavior.ask_on_drop)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.draft.behavior.ask_on_drop = *checked;
                        cx.notify();
                    })),
            )
            .child(note(
                tr(
                    language,
                    "Like Finder moves on the same disk and copies to another disk. Holding ⌥ while dropping always copies.",
                    "«Like Finder» перемещает в пределах диска и копирует на другой диск. С зажатым ⌥ всегда копирует.",
                ),
                palette,
            ))
            .child(section(IconName::Star, tr(language, "Favorites", "Избранное"), palette))
            .child(note(
                tr(
                    language,
                    "Your own folders and files under Favorites in the sidebar. Right-click any folder or file → Add to Favorites.",
                    "Свои папки и файлы в разделе Избранное. Правый клик по папке или файлу → Add to Favorites.",
                ),
                palette,
            ))
            .child(v_flex().gap_1().children(self.draft.sidebar.favorites.clone().into_iter().enumerate().map(
                |(index, path)| {
                    h_flex()
                        .gap_2()
                        .items_center()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(palette.surface)
                        .child(Icon::new(IconName::Folder).small().text_color(palette.folder))
                        .child(div().flex_1().min_w_0().truncate().text_xs().child(path.display().to_string()))
                        .children((!path.is_dir()).then(|| {
                            div().text_size(px(10.)).text_color(palette.video).child(tr(language, "missing", "нет на диске"))
                        }))
                        .child(
                            Button::new(("remove-favorite", index))
                                .ghost()
                                .xsmall()
                                .icon(IconName::X)
                                .tooltip(tr(language, "Remove", "Убрать"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.draft.sidebar.favorites.remove(index);
                                    cx.notify();
                                })),
                        )
                },
            )))
            .child(
                h_flex().child(
                    Button::new("add-favorite")
                        .secondary()
                        .small()
                        .icon(IconName::Plus)
                        .label(tr(language, "Add folder…", "Добавить папку…"))
                        .on_click(cx.listener(|this, _, _, cx| this.choose_favorite(cx))),
                ),
            )
            .child(
                Switch::new("clear-selection-on-empty-click")
                    .label(tr(
                        language,
                        "Clicking empty space in a list clears the selection",
                        "Клик по пустому месту в списке снимает выделение",
                    ))
                    .checked(self.draft.behavior.clear_selection_on_empty_click)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.draft.behavior.clear_selection_on_empty_click = *checked;
                        cx.notify();
                    })),
            )
            .child(note(
                tr(
                    language,
                    "The preview panel empties too. Shift- and ⌘-clicks never clear the selection.",
                    "Панель предпросмотра тоже очищается. Клики с Shift и ⌘ выделение не снимают.",
                ),
                palette,
            ))
                        .child(section(IconName::SquareTerminal, tr(language, "Command line", "Командная строка"), palette))
            .child(setting_row(
                tr(language, "The files command", "Команда files"),
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(if crate::desktop::cli_installed() {
                        Button::new("cli-uninstall").ghost().small().icon(IconName::Trash).label(tr(language, "Uninstall", "Удалить")).on_click(cx.listener(|this, _, _, cx| {
                            this.cli_message = Some(match crate::desktop::uninstall_cli() {
                                Ok(()) => "Removed".to_string(),
                                Err(error) => error.to_string(),
                            });
                            cx.notify();
                        }))
                    } else {
                        Button::new("cli-install").primary().small().icon(IconName::SquareTerminal).label(tr(language, "Install", "Установить")).on_click(cx.listener(|this, _, _, cx| {
                            this.cli_message = Some(crate::desktop::install_cli().unwrap_or_else(|error| error.to_string()));
                            cx.notify();
                        }))
                    })
                    .children(self.cli_message.clone().map(|message| div().text_xs().text_color(palette.muted).child(message))),
                palette,
            ))
            .child(note(
                tr(
                    language,
                    "files . opens the current folder in Files, files ~/Music any folder, like code . does for VS Code. No administrator needed; new terminals pick it up.",
                    "files . открывает текущую папку в Files, files ~/Music — любую папку, как code . у VS Code. Права администратора не нужны; работает в новых окнах терминала.",
                ),
                palette,
            ))
            .child(setting_row(
                tr(language, "Uninstall", "Удаление"),
                Button::new("uninstall-files").ghost().small().icon(IconName::Trash).label(tr(language, "Uninstall Files…", "Удалить Files…")).on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(crate::menus::UninstallFiles), cx);
                }),
                palette,
            ))
            .child(note(
                tr(
                    language,
                    "Removes the files command, the open helper and their PATH lines, gives folders back to Finder or Explorer, clears the caches, then removes the app. Asks first.",
                    "Удаляет команду files, помощник open и их строки в PATH, возвращает папки Finder или Проводнику, чистит кэши и удаляет приложение. Сначала спрашивает.",
                ),
                palette,
            ))
            .into_any_element()
    }

    fn render_appearance(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let language = self.draft.language;
        v_flex()
            .p_5()
            .gap_4()
            .child(section(IconName::SunMoon, tr(language, "Theme", "Тема"), palette))
            .child(setting_row(
                tr(language, "Color scheme", "Цветовая схема"),
                h_flex()
                    .gap_2()
                    .child(
                        option_button(
                            "scheme-dark",
                            tr(language, "Dark", "Тёмная"),
                            self.draft.scheme == ColorScheme::Dark,
                        )
                        .icon(IconName::Moon)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft.scheme = ColorScheme::Dark;
                            cx.notify();
                        })),
                    )
                    .child(
                        option_button(
                            "scheme-light",
                            tr(language, "Light", "Светлая"),
                            self.draft.scheme == ColorScheme::Light,
                        )
                        .icon(IconName::Sun)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft.scheme = ColorScheme::Light;
                            cx.notify();
                        })),
                    )
                    .child(
                        option_button(
                            "scheme-system",
                            tr(language, "System", "Как в системе"),
                            self.draft.scheme == ColorScheme::System,
                        )
                        .icon(IconName::Monitor)
                        .tooltip(tr(language, "Follow the macOS appearance", "Следовать оформлению macOS"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft.scheme = ColorScheme::System;
                            cx.notify();
                        })),
                    ),
                palette,
            ))
            .child(setting_row(
                tr(language, "Theme", "Тема"),
                h_flex().gap_2().flex_wrap().children(crate::settings::ThemePreset::ALL.map(|theme| {
                    // Three dots of the theme's colors, in the scheme shown.
                    let colors = Palette::themed(theme, self.draft.scheme.is_dark(), self.draft.accent);
                    let swatch = h_flex().gap_0p5().children([colors.window, colors.accent, colors.folder].map(|color| {
                        div().size(px(9.)).rounded_full().border_1().border_color(palette.border).bg(color)
                    }));
                    h_flex().gap_1p5().items_center().child(
                        option_button(gpui_kit::ElementId::Name(format!("theme-{}", theme.label()).into()), theme.label(), self.draft.theme == theme)
                            .tooltip(match theme {
                                crate::settings::ThemePreset::Default => tr(language, "Files' own colors with your accent", "Собственные цвета Files с вашим акцентом"),
                                crate::settings::ThemePreset::Omarchy => tr(language, "Omarchy's terminal look: Tokyo Night, monospace, square corners", "Терминальный стиль Omarchy: Tokyo Night, моноширинный шрифт, прямые углы"),
                                crate::settings::ThemePreset::Catppuccin => tr(language, "Catppuccin Mocha in dark, Latte in light", "Catppuccin: Mocha в тёмной схеме, Latte в светлой"),
                                crate::settings::ThemePreset::Windows => tr(language, "Looks like Windows 11: its greys and blue, Segoe UI, small corners", "Как Windows 11: её цвета и синий, Segoe UI, небольшие скругления"),
                                crate::settings::ThemePreset::MacOs => tr(language, "Looks like macOS: its greys, system blue and font", "Как macOS: её цвета, системный синий и шрифт"),
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.draft.theme = theme;
                                cx.notify();
                            })),
                    ).child(swatch)
                })),
                palette,
            ))
            .child(setting_row(
                tr(language, "Font", "Шрифт"),
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div().w(px(240.)).child(
                            gpui_kit::component::select::Select::new(&self.font_select)
                                .small()
                                .search_placeholder(tr(language, "Search fonts…", "Поиск шрифта…")),
                        ),
                    )
                    // The choice, written in itself.
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.muted)
                            .when_some(self.draft.font.clone(), |this, font| this.font_family(font))
                            .child(tr(language, "The quick brown fox · 0123", "Съешь же ещё этих булок · 0123")),
                    ),
                palette,
            ))
            .child(setting_row(
                tr(language, "Corners", "Скругление"),
                h_flex().gap_2().flex_wrap().children(crate::settings::Corners::ALL.map(|corners| {
                    let label = match corners {
                        crate::settings::Corners::Theme => tr(language, "Theme", "По теме"),
                        crate::settings::Corners::Square => tr(language, "None", "Нет"),
                        crate::settings::Corners::Small => tr(language, "Small", "Малое"),
                        crate::settings::Corners::Medium => tr(language, "Medium", "Среднее"),
                        crate::settings::Corners::Large => tr(language, "Large", "Большое"),
                    };
                    // A small square with these corners, as the sample.
                    let radius = corners.radii().map_or(5., |(radius, _)| radius);
                    option_button(gpui_kit::ElementId::Name(format!("corners-{corners:?}").into()), label, self.draft.corners == corners)
                        .child(div().size(px(12.)).rounded(px(radius.min(6.))).border_1().border_color(palette.muted))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.draft.corners = corners;
                            cx.notify();
                        }))
                })),
                palette,
            ))
            .child(setting_row(
                tr(language, "Icons", "Иконки"),
                h_flex().gap_2().flex_wrap().children(crate::settings::IconTheme::ALL.map(|icons| {
                    let label = match icons {
                        crate::settings::IconTheme::Colorful => tr(language, "Colorful", "Цветные"),
                        crate::settings::IconTheme::Monochrome => tr(language, "Monochrome", "Монохромные"),
                        crate::settings::IconTheme::Accent => tr(language, "Accent", "Акцентные"),
                    };
                    let mut sample = palette;
                    sample.icon_theme = icons;
                    option_button(gpui_kit::ElementId::Name(format!("icons-{icons:?}").into()), label, self.draft.icon_theme == icons)
                        .child(h_flex().gap_0p5().children([crate::model::FileKind::Folder, crate::model::FileKind::Image, crate::model::FileKind::Code].map(|kind| crate::views::files::kind_icon(kind, sample, 12.))))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.draft.icon_theme = icons;
                            cx.notify();
                        }))
                })),
                palette,
            ))
            .child(setting_row(
                tr(language, "Accent", "Акцент"),
                h_flex()
                    .gap_2()
                    .child(
                        option_button("accent-iris", "Iris", self.draft.accent == Accent::Iris)
                            .child(div().size(px(9.)).rounded_full().border_1().border_color(palette.text).bg(Palette::new(self.draft.scheme, Accent::Iris).accent))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.draft.accent = Accent::Iris;
                                cx.notify();
                            })),
                    )
                    .child(
                        option_button("accent-azure", "Azure", self.draft.accent == Accent::Azure)
                            .child(div().size(px(9.)).rounded_full().border_1().border_color(palette.text).bg(Palette::new(self.draft.scheme, Accent::Azure).accent))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.draft.accent = Accent::Azure;
                                cx.notify();
                            })),
                    )
                    .child(
                        option_button("accent-amber", "Amber", self.draft.accent == Accent::Amber)
                            .child(div().size(px(9.)).rounded_full().border_1().border_color(palette.text).bg(Palette::new(self.draft.scheme, Accent::Amber).accent))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.draft.accent = Accent::Amber;
                                cx.notify();
                            })),
                    ),
                palette,
            ))
            .child(note(
                tr(
                    language,
                    "Applied to all open windows after Save.",
                    "Применяется ко всем открытым окнам после сохранения.",
                ),
                palette,
            ))
            .child(self.render_look(palette, cx))
            .child(self.render_menu_colors(palette, cx))
            .into_any_element()
    }

    /// The "none" ring plus the preset swatches for one menu-color key.
    fn color_swatches(&self, key: &'static str, name: &'static str, palette: Palette, cx: &mut Context<Self>) -> gpui_kit::Div {
        let current = crate::menu_colors::value_for(&self.draft, key);
        let swatch = |value: &'static str, cx: &mut Context<Self>| {
            let selected = current == value;
            let fill = crate::menu_colors::parse(value);
            let swatch = div()
                .id(gpui_kit::ElementId::Name(format!("menu-color-{key}-{value}").into()))
                .size(px(16.))
                .rounded_full()
                .cursor_pointer()
                .border_2()
                .border_color(if selected { palette.text } else { palette.border })
                .when_some(fill, |swatch, fill| swatch.bg(fill))
                .when(fill.is_none(), |swatch| {
                    // "None": an empty ring with a slash.
                    swatch.flex().items_center().justify_center().child(Icon::new(IconName::Slash).xsmall().text_color(palette.faint))
                })
                .aria_label(format!("{name} menu color {value}"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    crate::menu_colors::set(&mut this.draft, key, value);
                    cx.notify();
                }));
            #[cfg(test)]
            let swatch = swatch.debug_selector(move || format!("menu-color-{key}-{value}"));
            swatch
        };
        let mut swatches = h_flex().gap_1p5().items_center().child(swatch("none", cx));
        for value in crate::menu_colors::PRESETS {
            swatches = swatches.child(swatch(value, cx));
        }
        swatches
    }

    /// The `key:` filters an extension adds to search, with where each works.
    fn render_search_filters(&mut self, extension: &'static files_sdk::Manifest, enabled: bool, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let keys: Vec<files_sdk::SearchKey> = crate::views::extension_ui::registry()
            .into_iter()
            .find(|candidate| candidate.manifest().id == extension.id)
            .map(|candidate| candidate.search_keys().to_vec())
            .unwrap_or_default();
        if keys.is_empty() {
            return None;
        }
        let language = self.draft.language;
        let rows = keys.into_iter().map(|search| {
            let key = search.key;
            let folder_on = !self.draft.search_filters.folder_off.iter().any(|off| off == key);
            let global_on = !self.draft.search_filters.global_off.iter().any(|off| off == key);
            let toggle = |global: bool, checked: bool, label: &'static str, cx: &mut Context<Self>| {
                Switch::new(gpui_kit::ElementId::Name(format!("search-filter-{key}-{}", if global { "global" } else { "folder" }).into()))
                    .label(label)
                    .small()
                    .checked(checked && enabled)
                    .disabled(!enabled)
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        let list = if global { &mut this.draft.search_filters.global_off } else { &mut this.draft.search_filters.folder_off };
                        list.retain(|off| off != key);
                        if !*checked {
                            list.push(key.to_string());
                        }
                        cx.notify();
                    }))
            };
            v_flex()
                .gap_1()
                .py_1()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().px_1p5().rounded_sm().bg(palette.accent_soft).text_color(palette.accent).text_sm().font_family("Menlo").child(format!("{key}:")))
                        .child(div().text_sm().text_color(palette.muted).child(search.summary))
                        .child(div().text_xs().text_color(palette.faint).font_family("Menlo").child(format!("e.g. {}", search.example))),
                )
                .child(
                    h_flex()
                        .gap_4()
                        .child(toggle(false, folder_on, tr(language, "Folder search (⌘F)", "Поиск в папке (⌘F)"), cx))
                        .child(toggle(true, global_on, tr(language, "Global search (⌘K)", "Глобальный поиск (⌘K)"), cx)),
                )
        });
        Some(
            card(palette)
                .child(section(IconName::Search, tr(language, "Search filters", "Фильтры поиска"), palette))
                .child(note(
                    tr(
                        language,
                        "Typed in a search box, these use what this extension knows to filter files or add results. Switched off, the text is searched as a name.",
                        "Эти ключи используют данные расширения для фильтрации файлов или добавления результатов. Выключенный ключ ищется как обычный текст в имени.",
                    ),
                    palette,
                ))
                .children(rows)
                .into_any_element(),
        )
    }

    /// Settings → Sound: notification chimes, the speaker test, the equalizer.
    fn render_sound(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        use crate::settings::{Chime, SpeakerLayout};
        let language = self.draft.language;
        let sound = self.draft.sound.clone();
        let chimes = Chime::ALL.map(|chime| {
            option_button(gpui_kit::ElementId::Name(format!("sound-chime-{}", chime.label()).into()), chime.label(), sound.chime == chime)
                .icon(IconName::Bell)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.draft.sound.chime = chime;
                    // Hear it when choosing it.
                    crate::sound::notify(chime, files_sdk::log::Level::Success, this.draft.sound.volume);
                    cx.notify();
                }))
        });
        let test_levels = [
            (files_sdk::log::Level::Info, "Info"),
            (files_sdk::log::Level::Success, "Success"),
            (files_sdk::log::Level::Warning, "Warning"),
            (files_sdk::log::Level::Error, "Error"),
        ]
        .map(|(level, label)| {
            Button::new(gpui_kit::ElementId::Name(format!("sound-test-{label}").into())).ghost().xsmall().icon(IconName::Play).label(label).on_click(cx.listener(
                move |this, _, _, cx| {
                    crate::sound::notify(this.draft.sound.chime, level, this.draft.sound.volume);
                    cx.notify();
                },
            ))
        });
        let layouts = SpeakerLayout::ALL.map(|layout| {
            option_button(gpui_kit::ElementId::Name(format!("sound-layout-{}", layout.label()).into()), layout.label(), sound.layout == layout)
                .icon(if layout == SpeakerLayout::Headphones { IconName::Headphones } else { IconName::Speaker })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.draft.sound.layout = layout;
                    cx.notify();
                }))
        });
        let speakers = crate::sound::speakers(sound.layout).iter().enumerate().map(|(index, speaker)| {
            let layout = sound.layout;
            div().debug_selector(move || format!("sound-speaker-{index}")).child(
                Button::new(("sound-speaker", index))
                    .small()
                    .icon(if speaker.short == "LFE" { IconName::AudioWaveform } else { IconName::Volume2 })
                    .label(format!("{} · {}", speaker.short, speaker.name))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        crate::sound::test_speaker(layout, index, this.draft.sound.volume);
                        cx.notify();
                    })),
            )
        });
        use crate::settings::{NotifyLevel, SystemNotify};
        let system = self.draft.system_notifications.clone();
        let system_modes = SystemNotify::ALL.map(|mode| {
            let label = match mode {
                SystemNotify::Off => tr(language, "Never", "Никогда"),
                SystemNotify::Background => tr(language, "When Files is in the background", "Когда Files в фоне"),
                SystemNotify::Always => tr(language, "Always", "Всегда"),
            };
            option_button(gpui_kit::ElementId::Name(format!("system-notify-{mode:?}").into()), label, system.mode == mode).on_click(cx.listener(move |this, _, _, cx| {
                this.draft.system_notifications.mode = mode;
                cx.notify();
            }))
        });
        let system_levels = NotifyLevel::ALL.map(|level| {
            let label = match level {
                NotifyLevel::All => tr(language, "All", "Все"),
                NotifyLevel::Warnings => tr(language, "Warnings and errors", "Предупреждения и ошибки"),
                NotifyLevel::Errors => tr(language, "Errors only", "Только ошибки"),
            };
            option_button(gpui_kit::ElementId::Name(format!("system-level-{level:?}").into()), label, system.min_level == level)
                .disabled(system.mode == SystemNotify::Off)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.draft.system_notifications.min_level = level;
                    cx.notify();
                }))
        });
        let count = crate::sound::speakers(sound.layout).len();
        v_flex()
            .p_5()
            .gap_4()
            .child(
                card(palette)
                    .child(section(IconName::Bell, tr(language, "Notifications", "Уведомления"), palette))
                    .child(
                        Switch::new("sound-notifications")
                            .label(tr(language, "Play a sound when a notification arrives", "Звук при новом уведомлении"))
                            .checked(sound.notifications)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.sound.notifications = *checked;
                                cx.notify();
                            })),
                    )
                    .child(setting_row(tr(language, "Sound", "Звук"), h_flex().gap_2().children(chimes), palette))
                    .child(setting_row(
                        tr(language, "Volume", "Громкость"),
                        h_flex()
                            .flex_1()
                            .gap_3()
                            .items_center()
                            .child(div().flex_1().child(gpui_kit::component::slider::Slider::new(&self.sound_volume)))
                            .child(div().w(px(40.)).text_xs().text_color(palette.muted).child(format!("{}%", sound.volume))),
                        palette,
                    ))
                    .child(setting_row(tr(language, "Try", "Прослушать"), h_flex().gap_1().children(test_levels), palette))
                    .child(note(tr(language, "Each kind sounds different: errors lower, successes rising.", "У каждого вида свой звук: ошибка ниже, успех — восходящий."), palette)),
            )
            .child(
                card(palette)
                    .child(section(IconName::MessageSquare, tr(language, "System notifications", "Системные уведомления"), palette))
                    .child(note(
                        tr(
                            language,
                            "Notifications also appear in the system's notification center, so finished copies, updates and errors reach you outside Files.",
                            "Уведомления приходят и в центр уведомлений системы — о завершённом копировании, обновлениях и ошибках вы узнаете и вне Files.",
                        ),
                        palette,
                    ))
                    .child(setting_row(tr(language, "Send to the system", "Отправлять в систему"), h_flex().gap_2().flex_wrap().children(system_modes), palette))
                    .child(setting_row(tr(language, "Which ones", "Какие"), h_flex().gap_2().flex_wrap().children(system_levels), palette))
                    .child(
                        h_flex().child(
                            Button::new("system-notify-test").ghost().xsmall().icon(IconName::Send).label(tr(language, "Send a test", "Отправить тестовое")).on_click(
                                cx.listener(|this, _, _, cx| {
                                    crate::system_notify::test(this.draft.language);
                                    cx.notify();
                                }),
                            ),
                        ),
                    ),
            )
            .child(
                card(palette)
                    .child(section(IconName::Speaker, tr(language, "Speaker test", "Проверка динамиков"), palette))
                    .child(setting_row(tr(language, "Your speakers", "Ваши динамики"), h_flex().gap_2().flex_wrap().children(layouts), palette))
                    .child(h_flex().gap_2().flex_wrap().children(speakers))
                    .child(
                        Button::new("sound-play-all").primary().small().icon(IconName::Play).label(tr(language, "Play all in turn", "Все по очереди")).on_click(cx.listener(
                            move |this, _, _, _| {
                                let (layout, volume) = (this.draft.sound.layout, this.draft.sound.volume);
                                std::thread::spawn(move || {
                                    for index in 0..count {
                                        crate::sound::test_speaker(layout, index, volume);
                                        std::thread::sleep(std::time::Duration::from_millis(1500));
                                    }
                                });
                            },
                        )),
                    )
                    .child(note(
                        tr(
                            language,
                            "A tone in one speaker at a time (a low one for the subwoofer). A speaker you do not hear is not connected, or the system mixes it into another.",
                            "Тон в одном динамике за раз (низкий — для сабвуфера). Если динамик не слышно, он не подключён или система смешивает его в другой.",
                        ),
                        palette,
                    )),
            )
            .child(
                card(palette)
                    .child(section(IconName::SlidersVertical, tr(language, "Equalizer", "Эквалайзер"), palette))
                    .child(self.equalizer.clone())
                    .child(note(tr(language, "Used by Music and song previews; changes apply at once.", "Используется Music и превью песен; изменения применяются сразу."), palette)),
            )
            .into_any_element()
    }

    /// Settings → Extensions → Music: library folders; signed-in services.
    fn render_music_settings(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let language = self.draft.language;
        let config = files_ext_music::config();
        let folders = config.folders.iter().enumerate().map(|(index, folder)| {
            let folder = folder.clone();
            h_flex()
                .gap_2()
                .items_center()
                .child(Icon::new(IconName::Folder).small().text_color(palette.folder))
                .child(div().flex_1().min_w_0().truncate().text_sm().child(folder.display().to_string()))
                .child(Button::new(("music-folder-remove", index)).ghost().xsmall().icon(IconName::X).on_click(cx.listener(move |_, _, _, cx| {
                    let mut config = files_ext_music::config();
                    config.folders.retain(|known| *known != folder);
                    let _ = files_ext_music::save_config(&config);
                    cx.notify();
                })))
        });
        let service = |id: &'static str, name: &'static str, connected: bool, disconnect: fn() -> std::io::Result<()>, cx: &mut Context<Self>| {
            h_flex()
                .gap_2()
                .items_center()
                .child(div().w(px(130.)).text_sm().child(name))
                .child(div().flex_1().text_xs().text_color(palette.muted).child(if connected {
                    tr(language, "Signed in", "Вход выполнен")
                } else {
                    tr(language, "Not signed in · connect in the Music tab", "Не подключено · подключите во вкладке Music")
                }))
                .when(connected, |this| {
                    this.child(Button::new(id).ghost().xsmall().label(tr(language, "Sign out", "Выйти")).on_click(cx.listener(move |_, _, _, cx| {
                        let _ = disconnect();
                        cx.notify();
                    })))
                })
        };
        v_flex()
            .gap_2()
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(tr(language, "Library folders", "Папки библиотеки")))
            .children(folders)
            .child(
                h_flex().child(
                    Button::new("music-folder-add").small().icon(IconName::FolderPlus).label(tr(language, "Add folder…", "Добавить папку…")).on_click(cx.listener(
                        |_, _, _, cx| {
                            let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions { files: false, directories: true, multiple: true, prompt: Some("Add to the library".into()) });
                            cx.spawn(async move |this, cx| {
                                if let Ok(Ok(Some(paths))) = picked.await {
                                    let mut config = files_ext_music::config();
                                    for path in paths {
                                        if !config.folders.contains(&path) {
                                            config.folders.push(path);
                                        }
                                    }
                                    let _ = files_ext_music::save_config(&config);
                                    let _ = this.update(cx, |_, cx| cx.notify());
                                }
                            })
                            .detach();
                        },
                    )),
                ),
            )
            .child(div().pt_3().text_sm().font_weight(FontWeight::SEMIBOLD).child(tr(language, "Services", "Сервисы")))
            .child(service("music-spotify-sign-out", "Spotify", files_ext_music::spotify::connected(), files_ext_music::spotify::disconnect, cx))
            .child(service("music-yandex-sign-out", "Yandex Music", files_ext_music::yandex::connected(), files_ext_music::yandex::disconnect, cx))
            .child(div().text_xs().text_color(palette.faint).child(tr(
                language,
                "Spotify plays in the Spotify app (Premium); Files controls it. Yandex Music uses an unofficial API.",
                "Spotify играет в приложении Spotify (нужен Premium), Files им управляет. Яндекс Музыка — через неофициальный API.",
            )))
            .into_any_element()
    }

    /// Settings → Extensions → Mail: each special folder's color, and
    /// whether it is listed at all.
    fn render_mail_settings(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        use files_ext_mail::Special;
        let language = self.draft.language;
        let rows = Special::ALL.map(|special| {
            let key = special.key();
            let current = self.draft.mail.folder_colors.get(key).cloned().unwrap_or_else(|| "none".to_string());
            let shown = !self.draft.mail.hidden.iter().any(|hidden| hidden == key);
            let swatch = |value: &'static str, cx: &mut Context<Self>| {
                let fill = crate::menu_colors::parse(value);
                div()
                    .id(gpui_kit::ElementId::Name(format!("mail-color-{key}-{value}").into()))
                    .size(px(16.))
                    .rounded_full()
                    .cursor_pointer()
                    .border_2()
                    .border_color(if current == value { palette.text } else { palette.border })
                    .when_some(fill, |swatch, fill| swatch.bg(fill))
                    .when(fill.is_none(), |swatch| {
                        swatch.flex().items_center().justify_center().child(Icon::new(IconName::Slash).xsmall().text_color(palette.faint))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if value == "none" {
                            this.draft.mail.folder_colors.remove(key);
                        } else {
                            this.draft.mail.folder_colors.insert(key.to_string(), value.to_string());
                        }
                        cx.notify();
                    }))
            };
            let mut swatches = h_flex().gap_1p5().items_center().child(swatch("none", cx));
            for value in crate::menu_colors::PRESETS {
                swatches = swatches.child(swatch(value, cx));
            }
            let color = self.draft.mail.folder_colors.get(key).and_then(|name| crate::menu_colors::parse(name)).unwrap_or(palette.muted);
            h_flex()
                .gap_3()
                .min_h(px(32.))
                .items_center()
                .child(
                    gpui_kit::component::switch::Switch::new(gpui_kit::ElementId::Name(format!("mail-show-{key}").into()))
                        .checked(shown)
                        .small()
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            this.draft.mail.hidden.retain(|hidden| hidden != key);
                            if !*checked {
                                this.draft.mail.hidden.push(key.to_string());
                            }
                            cx.notify();
                        })),
                )
                .child(Icon::new(crate::views::mail::special_icon_for(special)).small().text_color(color))
                .child(div().w(px(110.)).text_sm().text_color(if shown { palette.text } else { palette.faint }).child(special.label()))
                .child(swatches)
        });
        v_flex()
            .gap_1()
            .child(div().text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(tr(language, "Special folders", "Специальные папки")))
            .child(div().text_xs().text_color(palette.muted).pb_2().child(tr(
                language,
                "Found from the server's folder roles (Gmail's Bin and Spam too). Switch off to hide one; pick a color for its icon.",
                "Определяются по ролям папок на сервере (в том числе Корзина и Спам Gmail). Выключите, чтобы скрыть; цвет — для значка.",
            )))
            .children(rows)
            .into_any_element()
    }

    /// Settings → Appearance → Context menu colors: the app's own items.
    /// Extensions color theirs on their own page.
    fn render_menu_colors(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let language = self.draft.language;
        let icons = [
            IconName::FolderOpen,
            IconName::Eye,
            IconName::Star,
            IconName::FolderPlus,
            IconName::FilePlus,
            IconName::SquareTerminal,
            IconName::Trash,
            IconName::Info,
            IconName::X,
            IconName::ArrowLeftToLine,
            IconName::Copy,
            IconName::ArrowRightToLine,
            IconName::Pin,
        ];
        let mut list = v_flex().gap_2();
        for ((key, name, _), icon) in crate::menu_colors::BASE.into_iter().zip(icons) {
            let color = crate::menu_colors::color_for(&self.draft, key);
            list = list.child(
                h_flex()
                    .gap_3()
                    .items_center()
                    // Preview: the row as it looks in the menu.
                    .child(
                        h_flex()
                            .w(px(150.))
                            .gap_2()
                            .items_center()
                            .text_sm()
                            .child(crate::views::context_menu::tinted_icon(Icon::new(icon).small(), color))
                            .child(div().truncate().text_color(color.unwrap_or(palette.text)).child(name)),
                    )
                    .child(self.color_swatches(key, name, palette, cx)),
            );
        }
        card(palette)
            .child(section(IconName::Palette, tr(language, "Context menu colors", "Цвета контекстного меню"), palette))
            .child(note(
                tr(
                    language,
                    "Colors of the app's own menu items. Each extension sets its color on its page (Extensions). Saved as [menu_colors] in settings.toml.",
                    "Цвета собственных пунктов меню. Цвет расширения задаётся на его странице (Extensions). Хранится в [menu_colors] в settings.toml.",
                ),
                palette,
            ))
            .child(list)
            .into_any_element()
    }

    /// An extension page's "Context menu" card: its color, and a preview of
    /// the items it adds, as they look in the menu.
    fn render_extension_menu(&mut self, manifest: &'static files_sdk::Manifest, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let preview = menu_preview(manifest);
        if preview.is_empty() {
            return None;
        }
        let language = self.draft.language;
        let color = crate::menu_colors::color_for(&self.draft, manifest.id);
        let row = |label: &str, icon: Icon, submenu: bool, faded: bool| {
            h_flex()
                .h(px(26.))
                .px_2()
                .gap_2()
                .items_center()
                .rounded_sm()
                .text_sm()
                .when(faded, |row| row.opacity(0.75))
                .child(crate::views::context_menu::tinted_icon(icon.small(), color))
                .child(div().flex_1().min_w_0().truncate().text_color(color.unwrap_or(palette.text)).child(label.to_string()))
                .when(submenu, |row| row.child(Icon::new(IconName::ChevronRight).xsmall().text_color(palette.faint)))
        };
        let popup = || {
            v_flex()
                .w(px(230.))
                .p_1()
                .rounded_md()
                .border_1()
                .border_color(palette.border)
                .bg(palette.elevated)
                .shadow_md()
        };
        let mut main = popup().child(row("Open", Icon::new(IconName::FolderOpen), false, true).text_color(palette.muted));
        let mut opened = None;
        for item in &preview {
            main = main.child(row(item.label, item.icon(manifest), !item.children.is_empty(), false));
            if opened.is_none() && !item.children.is_empty() {
                opened = Some(item);
            }
        }
        // The first submenu is shown open, as when hovering it.
        let side = opened.map(|item| {
            popup().children(item.children.iter().map(|(label, icon)| row(label, Icon::new(icon.clone()), false, false)))
        });
        Some(
            card(palette)
                .child(section(IconName::Menu, tr(language, "Context menu", "Контекстное меню"), palette))
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(div().text_sm().w(px(90.)).child(tr(language, "Color", "Цвет")))
                        .child(self.color_swatches(manifest.id, manifest.name, palette, cx)),
                )
                .child(div().text_xs().text_color(palette.muted).child(tr(language, "Preview", "Предпросмотр")))
                .child(h_flex().items_start().gap_1().child(main).children(side))
                .into_any_element(),
        )
    }

    fn choose_global_search_folder(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Search here".into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(folder) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| {
                    this.draft.search_filters.global_folder = (dirs::home_dir().as_ref() != Some(&folder)).then_some(folder);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn choose_favorite(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add to Favorites".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                let _ = this.update(cx, |this, cx| {
                    for path in paths {
                        if !this.draft.sidebar.favorites.contains(&path) {
                            this.draft.sidebar.favorites.push(path);
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn choose_background_image(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use as background".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = picked.await
                && !paths.is_empty()
            {
                let path = paths.remove(0);
                let _ = this.update(cx, |this, cx| {
                    this.draft.look.background = crate::settings::Background::Image { path };
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Window look: see-through panels and what shows behind them.
    fn render_look(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        use crate::settings::Background;
        const SWATCHES: [&str; 8] = [
            "#0f172a", "#1e1b4b", "#052e16", "#3f0d12", "#422006", "#1f2937", "#e2e8f0", "#fef3c7",
        ];
        let language = self.draft.language;
        let look = self.draft.look.clone();
        let kind = match look.background {
            Background::Default => 0,
            Background::Color { .. } => 1,
            Background::Image { .. } => 2,
        };
        let picker: AnyElement = match &look.background {
            Background::Default => note(
                tr(
                    language,
                    "Lower the opacity to see the desktop through the window, blurred.",
                    "Уменьшите непрозрачность, чтобы видеть рабочий стол сквозь окно (с размытием).",
                ),
                palette,
            ),
            Background::Color { hex } => v_flex()
                .gap_2()
                .child(h_flex().gap_2().children(SWATCHES.map(|swatch| {
                    let selected = hex.eq_ignore_ascii_case(swatch);
                    div()
                        .id(gpui_kit::ElementId::Name(format!("swatch-{swatch}").into()))
                        .size(px(22.))
                        .rounded_md()
                        .cursor_pointer()
                        .border_2()
                        .border_color(if selected { palette.accent } else { palette.border })
                        .bg(crate::settings::parse_hex(swatch).unwrap_or(palette.surface))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.draft.look.background = Background::Color { hex: swatch.to_string() };
                            this.background_hex.update(cx, |state, cx| state.set_value(swatch, window, cx));
                            cx.notify();
                        }))
                })))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().w(px(140.)).child(Input::new(&self.background_hex).small()))
                        .child(
                            Button::new("apply-hex")
                                .secondary()
                                .xsmall()
                                .label(tr(language, "Use color", "Применить"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let hex = this.background_hex.read(cx).value().trim().to_string();
                                    if crate::settings::parse_hex(&hex).is_some() {
                                        this.draft.look.background = Background::Color { hex };
                                        this.error = None;
                                    } else {
                                        this.error = Some(format!("\"{hex}\" is not a #rrggbb color"));
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element(),
            Background::Image { path } => h_flex()
                .gap_2()
                .items_center()
                .child(div().flex_1().min_w_0().truncate().text_xs().text_color(palette.muted).child(path.display().to_string()))
                .child(
                    Button::new("choose-background")
                        .secondary()
                        .xsmall()
                        .icon(IconName::Images)
                        .label(tr(language, "Choose image…", "Выбрать…"))
                        .on_click(cx.listener(|this, _, _, cx| this.choose_background_image(cx))),
                )
                .into_any_element(),
        };
        let kinds = [
            (Background::Default, tr(language, "Default", "Стандартный"), IconName::Sparkles),
            (Background::Color { hex: "#0f172a".to_string() }, tr(language, "Color", "Цвет"), IconName::Paintbrush),
            (Background::Image { path: std::path::PathBuf::new() }, tr(language, "Image", "Картинка"), IconName::Image),
        ];
        v_flex()
            .gap_4()
            .child(section(IconName::Layers, tr(language, "Window", "Окно"), palette))
            .child(setting_row(
                tr(language, "Panel opacity", "Непрозрачность панелей"),
                h_flex()
                    .flex_1()
                    .gap_3()
                    .items_center()
                    .child(div().flex_1().child(gpui_kit::component::slider::Slider::new(&self.opacity_slider)))
                    .child(div().w(px(40.)).text_xs().text_color(palette.muted).child(format!("{}%", look.panel_opacity))),
                palette,
            ))
            .child(setting_row(
                tr(language, "Background", "Фон"),
                h_flex().gap_2().children(kinds.into_iter().enumerate().map(|(index, (background, label, icon))| {
                    option_button(("background-kind", index), label, kind == index).icon(icon).on_click(cx.listener(
                        move |this, _, _, cx| {
                            if index == 2 {
                                this.choose_background_image(cx);
                            } else {
                                this.draft.look.background = background.clone();
                            }
                            cx.notify();
                        },
                    ))
                })),
                palette,
            ))
            .child(picker)
            .child(setting_row(
                tr(language, "Animations", "Анимации"),
                h_flex().gap_2().children(crate::settings::MotionStyle::ALL.map(|style| {
                    option_button(
                        gpui_kit::ElementId::Name(format!("motion-{}", style.label()).into()),
                        style.label(),
                        look.motion == style,
                    )
                    .icon(match style {
                        crate::settings::MotionStyle::Snappy => IconName::Zap,
                        crate::settings::MotionStyle::Smooth => IconName::Sparkles,
                        crate::settings::MotionStyle::Relaxed => IconName::Feather,
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.draft.look.motion = style;
                        cx.notify();
                    }))
                })),
                palette,
            ))
            .child(note(
                tr(
                    language,
                    "Dialogs, menus, switches and sliders. Accessibility → Reduce motion turns animation off entirely.",
                    "Диалоги, меню, переключатели и ползунки. Accessibility → Reduce motion полностью отключает анимации.",
                ),
                palette,
            ))
            .children((look.panel_opacity >= 100 && kind != 0).then(|| {
                note(
                    tr(
                        language,
                        "Lower the panel opacity to see the background through the panels.",
                        "Уменьшите непрозрачность панелей, чтобы фон был виден.",
                    ),
                    palette,
                )
            }))
            .into_any_element()
    }

    fn render_extensions(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        match self.extension_page {
            Some(index) => self.render_extension_page(index, palette, cx),
            None => self.render_extensions_overview(palette, cx),
        }
    }

    /// Asks for the newest version now (the daily check runs on its own).
    fn check_updates_now(&mut self, cx: &mut Context<Self>) {
        if self.checking_updates {
            return;
        }
        self.checking_updates = true;
        self.update_install = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async { crate::updates::check() }).await;
            let _ = this.update(cx, |this, cx| {
                this.checking_updates = false;
                this.update_check = Some(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn install_update(&mut self, download: crate::updates::Download, cx: &mut Context<Self>) {
        self.installing_update = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::updates::download(&download, &crate::licensing::trusted_updates()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.installing_update = false;
                if let Ok(path) = &result {
                    // Install, then this copy quits and the new one starts.
                    match crate::updates::install_and_relaunch(path) {
                        Ok(()) => cx.quit(),
                        // Could not do it here: the installer, for the user.
                        Err(_) => cx.open_url(&path.display().to_string()),
                    }
                }
                this.update_install = Some(result);
                cx.notify();
            });
        })
        .detach();
    }

    /// The Check now button and what it found.
    fn render_update_check(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        use crate::updates::UpdateStatus;
        let language = self.draft.language;
        let mut actions = Vec::new();
        let status: Option<(IconName, gpui_kit::Hsla, String)> = match self.update_check.clone() {
            None => None,
            Some(Ok(UpdateStatus::UpToDate { latest })) => {
                Some((IconName::CircleCheck, palette.code, format!("Files is up to date · the newest is {latest}")))
            }
            Some(Ok(UpdateStatus::Available { latest, page, download, .. })) => {
                if let Some(download) = download.filter(|download| download.signature.is_some()) {
                    actions.push(
                        Button::new("settings-update-install")
                            .primary()
                            .small()
                            .icon(IconName::Download)
                            .label(tr(language, "Install & Relaunch", "Установить и перезапустить"))
                            .loading(self.installing_update)
                            .disabled(self.installing_update)
                            .on_click(cx.listener(move |this, _, _, cx| this.install_update(download.clone(), cx)))
                            .into_any_element(),
                    );
                }
                if let Some(page) = page {
                    actions.push(
                        Button::new("settings-update-page")
                            .ghost()
                            .small()
                            .label(tr(language, "Release notes", "Что нового"))
                            .on_click(move |_, _, cx| cx.open_url(&page))
                            .into_any_element(),
                    );
                }
                Some((IconName::Download, palette.accent, format!("Files {latest} is available · you have {}", crate::updates::CURRENT_VERSION)))
            }
            Some(Ok(UpdateStatus::NoReleases)) => Some((IconName::Info, palette.muted, "No releases yet".to_string())),
            Some(Ok(UpdateStatus::NotConfigured)) => Some((IconName::Info, palette.muted, "Update checks are not set up for this build".to_string())),
            Some(Err(error)) => Some((IconName::TriangleAlert, palette.video, format!("Could not check: {error}"))),
        };
        let install = self.update_install.clone().map(|result| match result {
            Ok(path) => (palette.code, format!("Verified and opened {} · drag Files to Applications (or run the installer), then reopen it", path.display())),
            Err(error) => (palette.video, error),
        });
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div().debug_selector(|| "settings-check-updates".into()).child(Button::new("settings-check-updates")
                            .secondary()
                            .small()
                            .icon(IconName::RefreshCw)
                            .label(tr(language, "Check now", "Проверить сейчас"))
                            .loading(self.checking_updates)
                            .disabled(self.checking_updates)
                            .on_click(cx.listener(|this, _, _, cx| this.check_updates_now(cx)))),
                    )
                    .children(status.map(|(icon, color, text)| {
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .text_xs()
                            .child(Icon::new(icon).small().text_color(color))
                            .child(div().text_color(color).child(text))
                    })),
            )
            .when(!actions.is_empty(), |this| this.child(h_flex().gap_2().children(actions)))
            .children(install.map(|(color, text)| div().text_xs().text_color(color).whitespace_normal().child(text)))
            .into_any_element()
    }

    /// Installs the package at the path or URL in the field (in the background).
    fn install_plugin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = self.install_url.read(cx).value().trim().to_string();
        let Some(folder) = crate::plugins::folder() else { return };
        if source.is_empty() {
            self.install_message = Some((false, "Paste the path or https:// link of a .filesext package".to_string()));
            cx.notify();
            return;
        }
        self.installing = true;
        self.install_message = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::plugins::install(&source, &folder, &crate::licensing::trusted()) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.installing = false;
                this.install_message = Some(match result {
                    Ok(manifest) => {
                        this.install_url.update(cx, |input, cx| input.set_value("", window, cx));
                        (true, format!("Installed {} {}", manifest.name, manifest.version))
                    }
                    Err(error) => (false, error),
                });
                let saved = this.original.clone();
                this.plugins = crate::plugins::load_installed(&saved);
                cx.notify();
            });
        })
        .detach();
    }

    /// Settings → Extensions → License: enter the key that unlocks Pro extensions.
    fn render_license(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let language = self.draft.language;
        let status: AnyElement = match crate::licensing::status(&self.draft) {
            None => div()
                .text_xs()
                .text_color(palette.muted)
                .child(tr(language, "Free extensions work without a key. Pro extensions need a license.", "Бесплатные расширения работают без ключа. Pro-расширениям нужна лицензия."))
                .into_any_element(),
            Some(Ok(license)) => {
                let until = license.expires.map_or_else(
                    || "never expires".to_string(),
                    |expires| {
                        chrono::DateTime::from_timestamp(expires, 0).map_or("—".to_string(), |time| format!("until {}", time.format("%Y-%m-%d")))
                    },
                );
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .child(Icon::new(IconName::CircleCheck).small().text_color(palette.code))
                    .child(div().text_color(palette.text).child(format!(
                        "Licensed to {} · {} · {until}",
                        license.email,
                        if license.products.iter().any(|product| product == "*") { "all Pro extensions".to_string() } else { license.products.join(", ") }
                    )))
                    .into_any_element()
            }
            Some(Err(error)) => h_flex()
                .gap_2()
                .items_center()
                .text_xs()
                .child(Icon::new(IconName::CircleX).small().text_color(palette.video))
                .child(div().text_color(palette.video).child(error.to_string()))
                .into_any_element(),
        };
        card(palette)
            .child(section(IconName::KeyRound, tr(language, "License", "Лицензия"), palette))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.license_input).small().w_full()))
                    .child(
                        Button::new("license-activate")
                            .primary()
                            .small()
                            .label(tr(language, "Activate", "Активировать"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                let key = this.license_input.read(cx).value().trim().to_string();
                                this.draft.license.key = key;
                                this.license_input.update(cx, |input, cx| input.set_value("", window, cx));
                                cx.notify();
                            })),
                    )
                    .when(!self.draft.license.key.is_empty(), |row| {
                        row.child(Button::new("license-remove").ghost().small().label(tr(language, "Remove", "Удалить")).on_click(cx.listener(|this, _, _, cx| {
                            this.draft.license.key.clear();
                            cx.notify();
                        })))
                    }),
            )
            .child(status)
            .child(note(tr(language, "The key is checked on this Mac, offline. Saved with Save.", "Ключ проверяется на этом компьютере, без сети. Сохраняется кнопкой Save."), palette))
            .into_any_element()
    }

    fn render_extensions_overview(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let language = self.draft.language;
        let mut installed = v_flex();
        for (index, extension) in crate::extensions::CATALOG.iter().enumerate() {
            let locked = !crate::licensing::unlocked(extension, &self.draft);
            let enabled = crate::extensions::enabled(&self.draft.extensions, index) && !locked;
            installed = installed.child(
                h_flex()
                    .id(("extension-row", index))
                    .h(px(48.))
                    .px_3()
                    .gap_3()
                    .items_center()
                    .cursor_pointer()
                    .when(index > 0, |row| row.border_t_1().border_color(palette.border))
                    .hover(|style| style.bg(palette.hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.extensions_expanded = true;
                        this.extension_page = Some(index);
                        cx.notify();
                    }))
                    .child(crate::extensions::icon_of(extension).small().text_color(if enabled { palette.accent } else { palette.faint }))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(extension.name))
                            .child(div().text_xs().text_color(palette.muted).truncate().child(extension.summary)),
                    )
                    .child(div().text_size(px(10.)).text_color(palette.faint).child(format!("v{} · {}", extension.version, extension.source)))
                    .when(matches!(extension.tier, files_sdk::Tier::Pro { .. }), |row| {
                        row.child(
                            h_flex()
                                .gap_0p5()
                                .px_1p5()
                                .rounded_sm()
                                .bg(palette.archive.opacity(0.18))
                                .text_size(px(10.))
                                .text_color(palette.archive)
                                .when(locked, |badge| badge.child(Icon::new(IconName::Lock).xsmall()))
                                .child("Pro"),
                        )
                    })
                    .child(
                        div()
                            .px_1p5()
                            .rounded_sm()
                            .text_size(px(10.))
                            .bg(if enabled { palette.accent_soft } else { palette.surface })
                            .text_color(if enabled { palette.accent } else { palette.faint })
                            .child(if locked { "Locked" } else if enabled { "On" } else { "Off" }),
                    )
                    .child(Icon::new(IconName::ChevronRight).xsmall().text_color(palette.faint)),
            );
        }
        v_flex()
            .p_5()
            .gap_4()
            .child(self.render_license(palette, cx))
            .child(
                card(palette)
                    .child(section(IconName::Puzzle, tr(language, "Installed", "Установленные"), palette))
                    .child(note(
                        tr(language, "Open an extension to turn it on or off and change its settings.", "Откройте расширение, чтобы включить его или изменить настройки."),
                        palette,
                    ))
                    .child(div().rounded_md().border_1().border_color(palette.border).bg(palette.window).child(installed)),
            )
            .child(
                card(palette)
                    .child(section(IconName::Download, tr(language, "Pro extensions", "Pro-расширения"), palette))
                    .child(note(
                        tr(
                            language,
                            "Paid extensions come as signed packages (.filesext with a .sig). Files installs only packages signed by its publisher and loads a Pro one only when your license covers it.",
                            "Платные расширения приходят подписанными пакетами (.filesext и .sig). Files ставит только пакеты с подписью издателя и загружает Pro-расширение, только если его покрывает лицензия.",
                        ),
                        palette,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().flex_1().min_w_0().child(Input::new(&self.install_url).small().w_full()))
                            .child(
                                Button::new("install-extension")
                                    .primary()
                                    .small()
                                    .icon(IconName::Download)
                                    .label(tr(language, "Install", "Установить"))
                                    .loading(self.installing)
                                    .disabled(self.installing)
                                    .on_click(cx.listener(|this, _, window, cx| this.install_plugin(window, cx))),
                            ),
                    )
                    .children(self.install_message.clone().map(|(ok, message)| {
                        div().text_xs().text_color(if ok { palette.code } else { palette.video }).child(message)
                    }))
                    .children(self.plugins.iter().enumerate().map(|(index, plugin)| {
                        let (label, color) = match &plugin.status {
                            crate::plugins::Status::Loaded => ("Loaded".to_string(), palette.code),
                            crate::plugins::Status::Locked => ("Locked · needs a license".to_string(), palette.archive),
                            crate::plugins::Status::Rejected(reason) => (format!("Refused · {reason}"), palette.video),
                        };
                        h_flex()
                            .id(("plugin-row", index))
                            .px_2()
                            .py_1p5()
                            .gap_2()
                            .items_center()
                            .rounded_md()
                            .border_1()
                            .border_color(palette.border)
                            .child(Icon::new(IconName::Puzzle).small().text_color(palette.accent))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().text_sm().child(format!("{} {}", plugin.manifest.name, plugin.manifest.version)))
                                    .child(div().text_xs().text_color(palette.faint).truncate().child(plugin.manifest.summary.clone())),
                            )
                            .when(plugin.manifest.product.is_some(), |row| {
                                row.child(div().px_1p5().rounded_sm().bg(palette.archive.opacity(0.18)).text_size(px(10.)).text_color(palette.archive).child("Pro"))
                            })
                            .child(div().text_xs().text_color(color).child(label))
                            .child({
                                let path = plugin.path.clone();
                                Button::new(("plugin-remove", index))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Trash)
                                    .tooltip("Uninstall")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.install_message = Some(match crate::plugins::uninstall(&path) {
                                            Ok(()) => (true, "Uninstalled · it is fully unloaded after a restart".to_string()),
                                            Err(error) => (false, error),
                                        });
                                        let saved = this.original.clone();
                                        this.plugins = crate::plugins::load_installed(&saved);
                                        cx.notify();
                                    }))
                            })
                    })),
            )
            .into_any_element()
    }

    fn render_extension_page(&mut self, index: usize, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let language = self.draft.language;
        let extension = &crate::extensions::CATALOG[index];
        let enabled = crate::extensions::enabled(&self.draft.extensions, index);
        let locked = !crate::licensing::unlocked(extension, &self.draft);
        let menu_card = self.render_extension_menu(extension, palette, cx);
        let bullet = |text: &str, color| {
            h_flex()
                .gap_2()
                .items_center()
                .text_xs()
                .text_color(color)
                .child(div().size(px(4.)).rounded_full().bg(palette.faint))
                .child(text.to_string())
        };
        // Extensions with a view of their own: a tab here, or a window.
        let opens_in = matches!(extension.id, "mail" | "music").then(|| {
            let id = extension.id;
            setting_row(
                tr(language, "Opens in", "Открывается"),
                h_flex().gap_2().children(
                    [(crate::settings::OpenIn::Tab, tr(language, "New tab", "Новая вкладка"), IconName::PanelTop), (crate::settings::OpenIn::Window, tr(language, "New window", "Новое окно"), IconName::AppWindow)].map(
                        |(open_in, label, icon)| {
                            option_button(gpui_kit::ElementId::Name(format!("open-in-{id}-{open_in:?}").into()), label, self.draft.opens_in(id) == open_in)
                                .icon(icon)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.draft.extension_windows.insert(id.to_string(), open_in);
                                    cx.notify();
                                }))
                        },
                    ),
                ),
                palette,
            )
        });
        let settings: AnyElement = match extension.id {
            "archives" => v_flex()
                .gap_2()
                .child(
                    Switch::new("archives-create-folder")
                        .label(tr(language, "Extract into a new folder named after the archive", "Распаковывать в новую папку с именем архива"))
                        .checked(self.draft.extensions.archives_create_folder)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.draft.extensions.archives_create_folder = *checked;
                            cx.notify();
                        })),
                )
                .child(
                    Switch::new("archives-overwrite")
                        .label(tr(language, "Overwrite existing files without asking", "Перезаписывать файлы без вопроса"))
                        .checked(self.draft.extensions.archives_overwrite)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.draft.extensions.archives_overwrite = *checked;
                            cx.notify();
                        })),
                )
                .into_any_element(),
            "mail" => self.render_mail_settings(palette, cx),
            "music" => self.render_music_settings(palette, cx),
            "terminal" => setting_row(
                tr(language, "Terminal app", "Приложение терминала"),
                h_flex().gap_1().flex_wrap().children(crate::settings::TerminalApp::available().into_iter().map(|app| {
                    option_button(
                        gpui_kit::ElementId::Name(format!("terminal-app-{}", app.app_name()).into()),
                        app.app_name(),
                        self.draft.extensions.terminal_app.effective() == app,
                    )
                    .icon(IconName::SquareTerminal)
                    .xsmall()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.draft.extensions.terminal_app = app;
                        cx.notify();
                    }))
                })),
                palette,
            ),
            _ => div()
                .text_xs()
                .text_color(palette.faint)
                .child(tr(language, "This extension has no settings yet.", "У этого расширения пока нет настроек."))
                .into_any_element(),
        };
        v_flex()
            .p_5()
            .gap_4()
            .child(
                card(palette)
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(
                                div()
                                    .size(px(40.))
                                    .rounded_lg()
                                    .bg(palette.accent_soft)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(crate::extensions::icon_of(extension).text_color(palette.accent)),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(extension.name))
                                    .child(div().text_xs().text_color(palette.muted).child(format!(
                                        "{} · v{} · {}",
                                        extension.id, extension.version, extension.source
                                    ))),
                            )
                            .child(
                                Switch::new(("extension-enabled", index))
                                    .label(if locked {
                                        tr(language, "Needs a Pro license", "Нужна Pro-лицензия")
                                    } else if enabled {
                                        tr(language, "Enabled", "Вкл.")
                                    } else {
                                        tr(language, "Disabled", "Выкл.")
                                    })
                                    .checked(enabled && !locked)
                                    .disabled(locked)
                                    .on_click(cx.listener(move |this, _: &bool, _, cx| {
                                        crate::extensions::toggle(&mut this.draft.extensions, index);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(div().text_sm().child(extension.summary))
                    .when(locked, |this| {
                        this.child(div().text_xs().text_color(palette.archive).child(tr(
                            language,
                            "A Pro extension: enter a license key in Extensions → License to unlock it.",
                            "Pro-расширение: введите ключ лицензии в Extensions → License, чтобы открыть его.",
                        )))
                    }),
            )
            .child(
                card(palette)
                    .child(section(IconName::SlidersHorizontal, tr(language, "Settings", "Настройки"), palette))
                    .children(opens_in)
                    .child(settings),
            )
            .children(self.render_search_filters(extension, enabled, palette, cx))
            .child(
                card(palette)
                    .child(section(IconName::Blocks, tr(language, "Adds to the app", "Что добавляет"), palette))
                    .children(extension.adds.iter().map(|item| bullet(item, if enabled { palette.text } else { palette.faint })))
                    .child(section(IconName::Keyboard, tr(language, "Hotkeys", "Горячие клавиши"), palette))
                    .children({
                        let commands = crate::keymap::for_extension(extension.name);
                        if commands.is_empty() {
                            vec![bullet(tr(language, "No shortcuts", "Нет сочетаний"), palette.faint).into_any_element()]
                        } else {
                            commands.iter().map(|command| self.shortcut_row(command, enabled, palette, cx)).collect()
                        }
                    })
                    .children(self.hotkey_error.clone().map(|error| {
                        div().text_xs().text_color(palette.video).child(error)
                    }))
                    .child(note(
                        tr(language, "✎ records a new shortcut (sequences too), ↺ restores the default. Same list as Settings → Hotkeys.", "✎ записывает новое сочетание (и последовательности), ↺ возвращает исходное. Тот же список, что в Settings → Hotkeys."),
                        palette,
                    ))
                    .children(menu_card)
                    .child(section(IconName::ShieldCheck, tr(language, "Permissions", "Разрешения"), palette))
                    .children(extension.permissions.iter().map(|permission| {
                        let permission = *permission;
                        let id = extension.id;
                        v_flex()
                            .gap_0p5()
                            .child(
                                Switch::new(gpui_kit::ElementId::Name(format!("permission-{id}-{}", permission.key()).into()))
                                    .label(permission.title())
                                    .checked(!self.draft.revoked(id, permission))
                                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                        this.draft.set_revoked(id, permission, !*checked);
                                        cx.notify();
                                    })),
                            )
                            .child(div().pl(px(44.)).text_xs().text_color(palette.faint).child(permission.description()))
                            .into_any_element()
                    }))
                    .when(extension.permissions.is_empty(), |this| {
                        this.child(bullet(tr(language, "Needs no permissions", "Разрешения не нужны"), palette.faint))
                    })
                    .when(!extension.menu.is_empty() || !extension.sidebar.is_empty(), |this| {
                        this.child(section(IconName::Menu, tr(language, "Adds", "Добавляет"), palette))
                            .children(extension.menu.iter().map(|item| {
                                let mut places = Vec::new();
                                if item.targets.contains(files_sdk::Targets::FILE) { places.push("files"); }
                                if item.targets.contains(files_sdk::Targets::FOLDER) { places.push("folders"); }
                                if item.targets.contains(files_sdk::Targets::BACKGROUND) { places.push("empty area"); }
                                let text = format!("Context menu → {} ({})", item.title, places.join(", "));
                                bullet(&text, palette.muted).into_any_element()
                            }))
                            .children(extension.sidebar.iter().map(|section| {
                                bullet(&format!("Sidebar → {}", section.title), palette.muted).into_any_element()
                            }))
                    }),
            )
            .child(note(tr(language, "Changes here are applied with Save.", "Изменения применяются кнопкой Save."), palette))
            .into_any_element()
    }

    fn add_pattern(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pattern = self.new_pattern.read(cx).value().trim().to_string();
        if !pattern.is_empty() && !self.draft.deletion.protected.contains(&pattern) {
            self.draft.deletion.protected.push(pattern);
        }
        self.new_pattern.update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    fn render_deletion(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let language = self.draft.language;
        let rows = self
            .draft
            .deletion
            .protected
            .clone()
            .into_iter()
            .enumerate()
            .map(|(index, pattern)| {
                h_flex()
                    .h(px(30.))
                    .flex_shrink_0()
                    .gap_2()
                    .items_center()
                    .px_3()
                    .when(index > 0, |row| row.border_t_1().border_color(palette.border))
                    .hover(|style| style.bg(palette.hover))
                    .child(Icon::new(IconName::FolderLock).xsmall().text_color(palette.faint))
                    .child(div().flex_1().min_w_0().truncate().text_xs().child(pattern))
                    .child(
                        Button::new(("remove-protected", index))
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .tooltip(tr(language, "Remove", "Удалить"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.draft.deletion.protected.remove(index);
                                cx.notify();
                            })),
                    )
            });
        let count = self.draft.deletion.protected.len();
        v_flex()
            .p_5()
            .gap_4()
            .child(
                card(palette)
                    .child(section(IconName::Trash, tr(language, "Deleting", "Удаление"), palette))
                    .child(
                        Switch::new("use-trash")
                            .label(tr(
                                language,
                                "Move to Trash / Recycle Bin by default",
                                "По умолчанию перемещать в Корзину",
                            ))
                            .checked(self.draft.deletion.use_trash)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.deletion.use_trash = *checked;
                                cx.notify();
                            })),
                    )
                    .child(note(
                        tr(
                            language,
                            "Off: Delete removes items permanently. Items already in the Trash are always deleted permanently.",
                            "Выключено: удаление безвозвратное. Из Корзины всегда удаляется безвозвратно.",
                        ),
                        palette,
                    )),
            )
            .child(
                card(palette)
                    .child(
                        h_flex()
                            .items_center()
                            .child(section(
                                IconName::ShieldCheck,
                                tr(language, "Protected paths", "Защищённые пути"),
                                palette,
                            ))
                            .child(div().flex_1())
                            .child(div().text_xs().text_color(palette.faint).child(count.to_string())),
                    )
                    .child(note(
                        tr(
                            language,
                            "These items can never be deleted from the app. `~` is your home folder, a trailing `/**` also protects everything inside, `*` matches within a name. A folder that contains a protected item is protected too. Stored as [deletion] protected in settings.toml.",
                            "Эти элементы нельзя удалить из приложения. `~` — домашняя папка, `/**` в конце защищает и всё содержимое, `*` — любые символы в имени. Папка, внутри которой есть защищённый элемент, тоже защищена. Хранится как [deletion] protected в settings.toml.",
                        ),
                        palette,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().flex_1().min_w_0().child(Input::new(&self.new_pattern).small().w_full()))
                            .child(
                                Button::new("add-protected")
                                    .primary()
                                    .small()
                                    .icon(IconName::Plus)
                                    .label(tr(language, "Add", "Добавить"))
                                    .on_click(cx.listener(|this, _, window, cx| this.add_pattern(window, cx))),
                            )
                            .child(
                                Button::new("reset-protected")
                                    .ghost()
                                    .small()
                                    .icon(IconName::RotateCcw)
                                    .label(tr(language, "Reset", "Сбросить"))
                                    .tooltip(tr(language, "Restore the default list", "Вернуть список по умолчанию"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.draft.deletion.protected =
                                            crate::settings::DeletionSettings::default().protected;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        // One bordered list with its own scroll keeps the card a fixed size.
                        div()
                            .id("protected-list")
                            .max_h(px(240.))
                            .overflow_y_scrollbar()
                            .rounded_md()
                            .border_1()
                            .border_color(palette.border)
                            .bg(palette.window)
                            .child({
                                let list = v_flex().children(rows);
                                #[cfg(test)]
                                let list = list.debug_selector(|| "protected-list-content".into());
                                list
                            }),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn set_default_app(&mut self, extension: &str, app: Option<std::path::PathBuf>, cx: &mut Context<Self>) {
        match app {
            Some(app) => {
                self.draft
                    .default_apps
                    .insert(extension.to_string(), app.display().to_string());
            }
            None => {
                self.draft.default_apps.remove(extension);
            }
        }
        self.expanded_extension = None;
        self.apply_default_apps(cx);
        cx.notify();
    }

    /// Default app choices take effect at once, like macOS System Settings:
    /// only this section is saved, other unsaved edits still wait for Save.
    fn apply_default_apps(&mut self, cx: &mut Context<Self>) {
        let apps = self.draft.default_apps.clone();
        let mut saved = self.appearance.read(cx).clone();
        saved.default_apps = apps.clone();
        match crate::settings::save(&saved) {
            Ok(()) => {
                self.original.default_apps = apps.clone();
                self.appearance.update(cx, |settings, cx| {
                    settings.default_apps = apps;
                    cx.notify();
                });
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// "Other app…": the native chooser runs off the UI thread.
    fn choose_other_app(&mut self, extension: String, cx: &mut Context<Self>) {
        // Windows and Linux: the system file dialog, pointed at a program.
        if !cfg!(target_os = "macos") {
            let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Default app".into()),
            });
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(Some(paths))) = picked.await
                    && let Some(app) = paths.into_iter().next()
                {
                    let _ = this.update(cx, |this, cx| this.set_default_app(&extension, Some(app), cx));
                }
            })
            .detach();
            return;
        }
        cx.spawn(async move |this, cx| {
            let chosen = cx
                .background_executor()
                .spawn(async { crate::default_apps::choose_application() })
                .await;
            if let Ok(Some(app)) = chosen {
                let _ = this.update(cx, |this, cx| this.set_default_app(&extension, Some(app), cx));
            }
        })
        .detach();
    }

    fn add_extension(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let extension = crate::default_apps::normalize_extension(&self.new_extension.read(cx).value());
        self.new_extension.update(cx, |state, cx| state.set_value("", window, cx));
        if !extension.is_empty() {
            self.expanded_extension = Some(extension);
        }
        cx.notify();
    }

    fn render_default_apps(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let language = self.draft.language;
        let mut extensions: Vec<String> =
            crate::default_apps::COMMON.iter().map(|extension| extension.to_string()).collect();
        for extension in self.draft.default_apps.keys().chain(self.expanded_extension.iter()) {
            if !extensions.contains(extension) {
                extensions.push(extension.clone());
            }
        }
        let mut list = v_flex();
        for (index, extension) in extensions.iter().enumerate() {
            let system = crate::default_apps::system_default(extension);
            let custom = self.draft.default_apps.get(extension).cloned();
            let expanded = self.expanded_extension.as_deref() == Some(extension.as_str());
            let (app_label, badge) = match (&custom, &system) {
                (Some(app), _) => (crate::default_apps::AppChoice::from_path(app).name, Some("Custom")),
                (None, Some(app)) => (app.name.clone(), None),
                (None, None) => (tr(language, "Not set", "Не задано").to_string(), None),
            };
            let toggle = extension.clone();
            let reset = extension.clone();
            let row = h_flex()
                .h(px(40.))
                .flex_shrink_0()
                .px_3()
                .gap_3()
                .items_center()
                .when(index > 0, |row| row.border_t_1().border_color(palette.border))
                .child(div().w(px(64.)).text_xs().font_weight(FontWeight::MEDIUM).child(format!(".{extension}")))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .truncate()
                                .text_sm()
                                .text_color(if custom.is_some() { palette.text } else { palette.muted })
                                .child(app_label),
                        )
                        .children(badge.map(|badge| {
                            div()
                                .px_1p5()
                                .rounded_sm()
                                .bg(palette.accent_soft)
                                .text_size(px(10.))
                                .text_color(palette.accent)
                                .child(badge)
                        }))
                        .children((custom.is_some()).then(|| {
                            div().text_size(px(10.)).text_color(palette.faint).child(format!(
                                "system: {}",
                                system.as_ref().map_or("—".to_string(), |app| app.name.clone())
                            ))
                        })),
                )
                .children(custom.is_some().then(|| {
                    Button::new(gpui_kit::ElementId::Name(format!("reset-app-{extension}").into()))
                        .ghost()
                        .xsmall()
                        .icon(IconName::RotateCcw)
                        .tooltip(tr(language, "Use the system default", "Системное по умолчанию"))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_default_app(&reset, None, cx)))
                }))
                .child(
                    Button::new(gpui_kit::ElementId::Name(format!("change-app-{extension}").into()))
                        .secondary()
                        .xsmall()
                        .label(if expanded { tr(language, "Done", "Готово") } else { tr(language, "Change", "Изменить") })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.expanded_extension =
                                (this.expanded_extension.as_deref() != Some(toggle.as_str())).then(|| toggle.clone());
                            cx.notify();
                        })),
                );
            list = list.child(row);
            if expanded {
                let mut choices = v_flex().gap_0p5().px_3().pb_3().pl(px(88.));
                for app in crate::default_apps::candidates(extension) {
                    let selected = custom.as_deref() == Some(app.path.to_string_lossy().as_ref());
                    let is_system = system.as_ref().is_some_and(|system| system.path == app.path);
                    let pick = extension.clone();
                    let path = app.path.clone();
                    choices = choices.child(
                        h_flex()
                            .id(gpui_kit::ElementId::Name(format!("pick-{extension}-{}", app.path.display()).into()))
                            .h(px(28.))
                            .px_2()
                            .gap_2()
                            .items_center()
                            .rounded_md()
                            .text_sm()
                            .bg(if selected { palette.selection } else { gpui_kit::transparent_black() })
                            .hover(|style| style.bg(palette.hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_default_app(&pick, Some(path.clone()), cx)
                            }))
                            .child(Icon::new(IconName::AppWindow).xsmall().text_color(palette.muted))
                            .child(div().flex_1().truncate().child(app.name.clone()))
                            .children(is_system.then(|| {
                                div().text_size(px(10.)).text_color(palette.faint).child("system default")
                            })),
                    );
                }
                let other = extension.clone();
                choices = choices.child(
                    h_flex().pt_1().child(
                        Button::new(gpui_kit::ElementId::Name(format!("other-app-{extension}").into()))
                            .ghost()
                            .xsmall()
                            .icon(IconName::FolderOpen)
                            .label(tr(language, "Other app…", "Другое приложение…"))
                            .on_click(cx.listener(move |this, _, _, cx| this.choose_other_app(other.clone(), cx))),
                    ),
                );
                list = list.child(choices);
            }
        }

        // Linux and macOS: Files as the system's file manager (folders from
        // the desktop, browsers' "Show in folder", other apps).
        let mac = cfg!(target_os = "macos");
        let windows = cfg!(windows);
        let file_manager = Some(()).map(|()| {
            card(palette)
                .child(section(IconName::FolderOpen, tr(language, "Default file manager", "Файловый менеджер по умолчанию"), palette))
                .child(note(
                    if windows {
                        tr(
                            language,
                            "Double-clicked folders and drives open in Files: on the desktop, in Explorer windows, from downloads and other apps (your user's registry, no administrator). \"This PC\", Control Panel and Win+E stay Explorer's. Restore Explorer undoes it; uninstalling Files does too.",
                            "Папки и диски по двойному клику открываются в Files: на рабочем столе, в окнах Проводника, из загрузок и других приложений (реестр вашего пользователя, без администратора). «Этот компьютер», Панель управления и Win+E остаются за Проводником. «Вернуть Проводник» всё отменяет, удаление Files — тоже.",
                        )
                    } else if mac {
                        tr(
                            language,
                            "Folders that other apps reveal (\"Show in Finder\", downloads, the Terminal's open .) open in Files, each in a new tab. The desktop, the Dock and Spotlight stay Finder's: macOS does not let any app replace it there. Restore Finder undoes it.",
                            "Папки, которые показывают другие приложения («Показать в Finder», загрузки, open . в Терминале), открываются в Files, каждая в новой вкладке. Рабочий стол, Dock и Spotlight остаются за Finder — macOS не даёт заменить его там. «Вернуть Finder» всё отменяет.",
                        )
                    } else {
                        tr(
                            language,
                            "Folders open in Files everywhere: on the desktop, from a browser's \"Show in folder\" and from other apps (xdg-mime, inode/directory). To undo, choose another file manager in your system settings.",
                            "Папки открываются в Files везде: на рабочем столе, по «Показать в папке» из браузера и из других приложений (xdg-mime, inode/directory). Чтобы вернуть прежний, выберите другой менеджер в настройках системы.",
                        )
                    },
                    palette,
                ))
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(if self.default_file_manager {
                            h_flex()
                                .gap_2()
                                .text_sm()
                                .text_color(palette.accent)
                                .child(Icon::new(IconName::CircleCheck).small())
                                .child(tr(language, "Files opens folders", "Files открывает папки"))
                                .when(mac || windows, |this| {
                                    let label = if windows { tr(language, "Restore Explorer", "Вернуть Проводник") } else { tr(language, "Restore Finder", "Вернуть Finder") };
                                    this.child(Button::new("restore-finder").ghost().xsmall().icon(IconName::RotateCcw).label(label).on_click(
                                        cx.listener(|this, _, _, cx| {
                                            match crate::desktop::restore_system_file_manager() {
                                                Ok(()) => {
                                                    this.default_file_manager = crate::desktop::is_default_file_manager();
                                                    this.default_file_manager_error = None;
                                                }
                                                Err(error) => this.default_file_manager_error = Some(error.to_string()),
                                            }
                                            cx.notify();
                                        }),
                                    ))
                                })
                                .into_any_element()
                        } else {
                            Button::new("make-default-file-manager")
                                .primary()
                                .small()
                                .icon(IconName::FolderOpen)
                                .label(if windows { tr(language, "Open folders in Files instead of Explorer", "Открывать папки в Files вместо Проводника") } else if mac { tr(language, "Open folders in Files instead of Finder", "Открывать папки в Files вместо Finder") } else { tr(language, "Make Files the default", "Сделать Files менеджером по умолчанию") })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    match crate::desktop::make_default_file_manager() {
                                        Ok(()) => {
                                            this.default_file_manager = crate::desktop::is_default_file_manager();
                                            this.default_file_manager_error = None;
                                        }
                                        Err(error) => this.default_file_manager_error = Some(error.to_string()),
                                    }
                                    cx.notify();
                                }))
                                .into_any_element()
                        })
                        .children(self.default_file_manager_error.clone().map(|error| div().text_xs().text_color(palette.faint).child(error))),
                )
        });
        v_flex()
            .p_5()
            .gap_4()
            .children(file_manager)
            .child(
                card(palette)
                    .child(section(IconName::AppWindow, tr(language, "Default apps", "Приложения по умолчанию"), palette))
                    .child(note(
                        tr(
                            language,
                            "What opens each file type when you open it from this app. The system default is read from the OS; a custom choice takes effect immediately, applies only here and is stored as [default_apps] in settings.toml. Your system settings are not changed.",
                            "Чем открывается каждый тип файла из этого приложения. Системное значение читается из ОС; свой выбор применяется сразу, действует только здесь и хранится как [default_apps] в settings.toml. Системные настройки не меняются.",
                        ),
                        palette,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().flex_1().min_w_0().child(Input::new(&self.new_extension).small().w_full()))
                            .child(
                                Button::new("add-extension")
                                    .secondary()
                                    .small()
                                    .icon(IconName::Plus)
                                    .label(tr(language, "Add type", "Добавить тип"))
                                    .on_click(cx.listener(|this, _, window, cx| this.add_extension(window, cx))),
                            ),
                    )
                    .child(
                        div()
                            .rounded_md()
                            .border_1()
                            .border_color(palette.border)
                            .bg(palette.window)
                            .child(list),
                    ),
            )
            .into_any_element()
    }

    /// Shortcut changes apply at once, like Default apps.
    pub(crate) fn set_shortcut(&mut self, id: &'static str, keys: Option<String>, cx: &mut Context<Self>) {
        let previous = self.original.keymap.clone();
        let mut candidate = self.draft.keymap.clone();
        match &keys {
            Some(keys) => {
                candidate.insert(id.to_string(), keys.clone());
            }
            None => {
                candidate.remove(id);
            }
        }
        // Recording and resetting both refuse a shortcut another command uses.
        let mac = cx.platform_info().is_mac();
        let command = crate::keymap::commands().into_iter().find(|command| command.id == id);
        if let Some(command) = command {
            for wanted in crate::keymap::effective(&command, &candidate, mac) {
                let taken = crate::keymap::clashes(id, &wanted, &candidate, mac);
                if !taken.is_empty() {
                    self.hotkey_error = Some(format!(
                        "{} is already used by {}",
                        crate::keymap::display(&wanted, mac),
                        taken.join(", ")
                    ));
                    cx.notify();
                    return;
                }
            }
        }
        self.hotkey_error = None;
        self.draft.keymap = candidate;
        let keymap = self.draft.keymap.clone();
        let mut saved = self.appearance.read(cx).clone();
        saved.keymap = keymap.clone();
        match crate::settings::save(&saved) {
            Ok(()) => {
                self.original.keymap = keymap.clone();
                crate::keymap::apply(cx, &previous, &keymap);
                self.appearance.update(cx, |settings, cx| {
                    settings.keymap = keymap;
                    cx.notify();
                });
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        self.recording = None;
        cx.notify();
    }

    /// The recorder: the next key press becomes the shortcut.
    /// The recorder: keys pressed in a row form a sequence (like Zed's
    /// `cmd-k cmd-s`). A short pause, or a third key, commits it.
    fn record_key(&mut self, event: &gpui_kit::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        const MAX_STEPS: usize = 3;
        if self.recording.is_none() {
            return;
        }
        cx.stop_propagation();
        let keystroke = &event.keystroke;
        let plain = !keystroke.modifiers.modified();
        match keystroke.key.as_str() {
            "escape" if plain => {
                self.recording = None;
                self.recorded.clear();
                self.hotkey_error = None;
                self.focus_handle.focus(window, cx);
                cx.notify();
            }
            // Backspace as the first key removes the shortcut.
            "backspace" if plain && self.recorded.is_empty() => {
                if let Some(id) = self.recording {
                    self.set_shortcut(id, Some(String::new()), cx);
                }
                self.focus_handle.focus(window, cx);
            }
            _ => {
                self.recorded.push(crate::keymap::keystroke_text(keystroke));
                self.record_generation += 1;
                if self.recorded.len() >= MAX_STEPS {
                    self.commit_recording(window, cx);
                    return;
                }
                let generation = self.record_generation;
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(std::time::Duration::from_millis(900)).await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        if this.record_generation == generation && !this.recorded.is_empty() {
                            this.commit_recording(window, cx);
                        }
                    });
                })
                .detach();
                cx.notify();
            }
        }
    }

    fn commit_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.recording else {
            return;
        };
        let keys = std::mem::take(&mut self.recorded).join(" ");
        let mac = cx.platform_info().is_mac();
        let taken = crate::keymap::clashes(id, &keys, &self.draft.keymap, mac);
        if taken.is_empty() {
            self.hotkey_error = None;
            self.set_shortcut(id, Some(keys), cx);
            self.focus_handle.focus(window, cx);
        } else {
            // Refused: stay in recording mode so another shortcut can be tried.
            self.hotkey_error = Some(format!(
                "{} is already used by {} · press another shortcut",
                crate::keymap::display(&keys, mac),
                taken.join(", ")
            ));
            cx.notify();
        }
    }

    /// One editable shortcut: label, keys (or the live recorder), ✎ and ↺.
    fn shortcut_row(
        &self,
        command: &crate::keymap::Command,
        enabled: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let language = self.draft.language;
        let mac = cx.platform_info().is_mac();
        let id = command.id;
        let custom = self.draft.keymap.contains_key(id);
        let keys = crate::keymap::effective(command, &self.draft.keymap, mac)
            .iter()
            .map(|keys| crate::keymap::display(keys, mac))
            .collect::<Vec<_>>()
            .join(", ");
        let chip = if self.recording == Some(id) {
            h_flex()
                .id(gpui_kit::ElementId::Name(format!("ext-recorder-{id}").into()))
                .track_focus(&self.recorder_focus)
                .on_key_down(cx.listener(Self::record_key))
                .px_2()
                .py_0p5()
                .rounded_md()
                .border_1()
                .border_color(palette.accent)
                .text_color(palette.accent)
                .child(if self.recorded.is_empty() {
                    tr(language, "Press keys…", "Нажмите…").to_string()
                } else {
                    format!("{} …", crate::keymap::display(&self.recorded.join(" "), mac))
                })
                .into_any_element()
        } else {
            div()
                .px_1p5()
                .py_0p5()
                .rounded_sm()
                .border_1()
                .border_color(palette.border)
                .bg(palette.surface)
                .child(if keys.is_empty() { "—".to_string() } else { keys })
                .into_any_element()
        };
        h_flex()
            .gap_2()
            .items_center()
            .text_xs()
            .child(div().flex_1().text_color(if enabled { palette.text } else { palette.faint }).child(command.label))
            .child(chip)
            .child(
                Button::new(gpui_kit::ElementId::Name(format!("ext-record-{id}").into()))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Pencil)
                    .tooltip(tr(language, "Record a new shortcut", "Записать сочетание"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.recording = Some(id);
                        this.recorded.clear();
                        this.recorder_focus.focus(window, cx);
                        cx.notify();
                    })),
            )
            .child(if custom {
                Button::new(gpui_kit::ElementId::Name(format!("ext-reset-{id}").into()))
                    .ghost()
                    .xsmall()
                    .icon(IconName::RotateCcw)
                    .tooltip(tr(language, "Reset to default", "Сбросить"))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_shortcut(id, None, cx)))
                    .into_any_element()
            } else {
                div().w(px(24.)).into_any_element()
            })
            .into_any_element()
    }

    fn render_hotkeys(&mut self, palette: Palette, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let language = self.draft.language;
        let mac = cx.platform_info().is_mac();
        let query = self.hotkey_query.read(cx).value().to_lowercase();
        let overrides = self.draft.keymap.clone();
        let conflicts = crate::keymap::conflicts(&overrides, mac);
        if self.recording.is_some() && !self.recorder_focus.is_focused(window) {
            self.recorder_focus.focus(window, cx);
        }

        let mut table = v_flex();
        let mut group = "";
        for command in crate::keymap::commands() {
            let keys = crate::keymap::effective(&command, &overrides, mac);
            let shown: Vec<String> = keys.iter().map(|keys| crate::keymap::display(keys, mac)).collect();
            let haystack = format!("{} {} {} {} {}", command.label, command.id, command.group, keys.join(" "), shown.join(" ")).to_lowercase();
            if !query.is_empty() && !haystack.contains(&query) {
                continue;
            }
            if command.group != group {
                group = command.group;
                table = table.child(
                    div()
                        .px_3()
                        .pt_3()
                        .pb_1()
                        .text_size(px(10.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.faint)
                        .child(group.to_uppercase()),
                );
            }
            let custom = overrides.contains_key(command.id);
            let recording = self.recording == Some(command.id);
            let clash: Vec<&str> = keys
                .iter()
                .filter_map(|keys| conflicts.get(&format!("{}|{keys}", command.context)))
                .flatten()
                .copied()
                .filter(|other| *other != command.id)
                .collect();
            let id = command.id;
            let chips = if recording {
                h_flex()
                    .id(gpui_kit::ElementId::Name(format!("recorder-{id}").into()))
                    .track_focus(&self.recorder_focus)
                    .on_key_down(cx.listener(Self::record_key))
                    .px_2()
                    .py_0p5()
                    .rounded_md()
                    .border_1()
                    .border_color(palette.accent)
                    .text_xs()
                    .text_color(palette.accent)
                    .child(if self.recorded.is_empty() {
                        tr(language, "Press a shortcut or a sequence · Esc cancel · ⌫ remove", "Нажмите сочетание или последовательность · Esc отмена · ⌫ убрать").to_string()
                    } else {
                        format!("{} …", crate::keymap::display(&self.recorded.join(" "), mac))
                    })
                    .into_any_element()
            } else if shown.is_empty() {
                div().text_xs().text_color(palette.faint).child(tr(language, "Not bound", "Не назначено")).into_any_element()
            } else {
                h_flex()
                    .gap_1()
                    .children(shown.into_iter().map(|label| {
                        div()
                            .px_1p5()
                            .py_0p5()
                            .rounded_sm()
                            .border_1()
                            .border_color(palette.border)
                            .bg(palette.surface)
                            .text_xs()
                            .child(label)
                    }))
                    .into_any_element()
            };
            table = table.child(
                h_flex()
                    .min_h(px(36.))
                    .px_3()
                    .gap_3()
                    .items_center()
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        v_flex()
                            .w(px(190.))
                            .flex_shrink_0()
                            .child(div().text_sm().child(command.label))
                            .children((!clash.is_empty()).then(|| {
                                div().text_size(px(10.)).text_color(palette.video).child(format!("Conflicts with {}", clash.join(", ")))
                            })),
                    )
                    .child(div().flex_1().min_w_0().child(chips))
                    .child(div().w(px(90.)).text_size(px(10.)).text_color(palette.faint).child(command.context))
                    .child(
                        div()
                            .w(px(52.))
                            .text_size(px(10.))
                            .text_color(if custom { palette.accent } else { palette.faint })
                            .child(if custom { "User" } else { "Default" }),
                    )
                    .child(
                        Button::new(gpui_kit::ElementId::Name(format!("record-{id}").into()))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Pencil)
                            .tooltip(tr(language, "Record a new shortcut", "Записать сочетание"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.recording = Some(id);
                                this.recorded.clear();
                                this.recorder_focus.focus(window, cx);
                                cx.notify();
                            })),
                    )
                    .child(if custom {
                        Button::new(gpui_kit::ElementId::Name(format!("reset-key-{id}").into()))
                            .ghost()
                            .xsmall()
                            .icon(IconName::RotateCcw)
                            .tooltip(tr(language, "Reset to default", "Сбросить"))
                            .on_click(cx.listener(move |this, _, _, cx| this.set_shortcut(id, None, cx)))
                            .into_any_element()
                    } else {
                        div().w(px(24.)).into_any_element()
                    }),
            );
        }

        // Shortcuts that come with GPUI Kit components (inputs, lists, dialogs).
        let ours: std::collections::HashSet<&str> =
            crate::keymap::commands().iter().map(|command| (command.action)().name()).collect();
        let mut builtin: Vec<(String, String, String)> = cx
            .key_bindings()
            .borrow()
            .bindings()
            .filter(|binding| !gpui_kit::is_no_action(binding.action()) && !ours.contains(binding.action().name()))
            .map(|binding| {
                let keys = binding
                    .keystrokes()
                    .iter()
                    .map(|stroke| crate::keymap::keystroke_text(stroke.inner()))
                    .collect::<Vec<_>>()
                    .join(" ");
                let context = binding.predicate().map(|predicate| predicate.to_string()).unwrap_or_default();
                (binding.action().name().to_string(), crate::keymap::display(&keys, mac), context)
            })
            .filter(|(action, keys, context)| {
                query.is_empty() || format!("{action} {keys} {context}").to_lowercase().contains(&query)
            })
            .collect();
        builtin.sort();
        builtin.dedup();
        let builtin_count = builtin.len();

        v_flex()
            .p_5()
            .gap_4()
            .child(
                card(palette)
                    .child(section(IconName::Keyboard, tr(language, "Hotkeys", "Горячие клавиши"), palette))
                    .child(note(
                        tr(
                            language,
                            "Click ✎ and press the new shortcut. Changes apply at once and are stored as [keymap] in settings.toml; ↺ restores the default.",
                            "Нажмите ✎ и затем новое сочетание. Изменения применяются сразу и хранятся как [keymap] в settings.toml; ↺ возвращает значение по умолчанию.",
                        ),
                        palette,
                    ))
                    .child(Input::new(&self.hotkey_query).small().w_full())
                    .children(self.hotkey_error.clone().map(|error| {
                        h_flex()
                            .gap_2()
                            .items_center()
                            .text_xs()
                            .text_color(palette.video)
                            .child(Icon::new(IconName::TriangleAlert).xsmall())
                            .child(error)
                    }))
                    .children((!conflicts.is_empty()).then(|| {
                        h_flex()
                            .gap_2()
                            .items_center()
                            .text_xs()
                            .text_color(palette.video)
                            .child(Icon::new(IconName::TriangleAlert).xsmall())
                            .child(tr(
                                language,
                                "Some shortcuts conflict. Settings cannot be saved until each conflict is fixed (✎ or ↺).",
                                "Есть конфликты сочетаний. Настройки нельзя сохранить, пока они не исправлены (✎ или ↺).",
                            ))
                    }))
                    .child(
                        div()
                            .rounded_md()
                            .border_1()
                            .border_color(palette.border)
                            .bg(palette.window)
                            .child(table),
                    ),
            )
            .child(
                card(palette)
                    .child(
                        h_flex()
                            .id("builtin-keys-header")
                            .cursor_pointer()
                            .items_center()
                            .gap_2()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_builtin_keys = !this.show_builtin_keys;
                                cx.notify();
                            }))
                            .child(
                                Icon::new(if self.show_builtin_keys { IconName::ChevronDown } else { IconName::ChevronRight })
                                    .xsmall()
                                    .text_color(palette.faint),
                            )
                            .child(section(IconName::Blocks, tr(language, "Built-in component shortcuts", "Встроенные сочетания компонентов"), palette))
                            .child(div().flex_1())
                            .child(div().text_xs().text_color(palette.faint).child(format!("{builtin_count} · read-only"))),
                    )
                    .when(self.show_builtin_keys, |card| {
                        card.child(
                            v_flex().children(builtin.into_iter().map(|(action, keys, context)| {
                                h_flex()
                                    .min_h(px(28.))
                                    .gap_3()
                                    .items_center()
                                    .border_t_1()
                                    .border_color(palette.border)
                                    .text_xs()
                                    .child(div().w(px(220.)).truncate().child(action))
                                    .child(div().flex_1().child(keys))
                                    .child(div().w(px(160.)).truncate().text_color(palette.faint).child(context))
                            })),
                        )
                    }),
            )
            .into_any_element()
    }

    fn run_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let line = self.shell_input.read(cx).value().trim().to_string();
        if line.is_empty() {
            return;
        }
        self.shell_input.update(cx, |input, cx| input.set_value("", window, cx));
        if line == "cls" {
            self.shell_output.clear();
        } else {
            let output = crate::dev_shell::execute(&line, &self.draft);
            self.shell_output.push((line, output));
            // Keep the scrollback bounded.
            let extra = self.shell_output.len().saturating_sub(100);
            self.shell_output.drain(..extra);
        }
        cx.notify();
    }

    /// Settings → Developer: dev mode and the shell.
    fn render_developer(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let language = self.draft.language;
        let output = self.shell_output.iter().map(|(command, output)| {
            v_flex()
                .gap_0p5()
                .child(div().text_color(palette.accent).child(format!("› {command}")))
                .when(!output.is_empty(), |entry| entry.child(div().text_color(palette.text).whitespace_normal().child(output.clone())))
        });
        v_flex()
            .p_5()
            .gap_4()
            .child(
                card(palette)
                    .child(section(IconName::SquareTerminal, tr(language, "Developer mode", "Режим разработчика"), palette))
                    .child(
                        Switch::new("developer-mode")
                            .label(tr(language, "Show every call in the console", "Показывать все вызовы в консоли"))
                            .checked(self.draft.developer.enabled)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.developer.enabled = *checked;
                                cx.notify();
                            })),
                    )
                    .child(
                        Switch::new("developer-console")
                            .label(tr(language, "Show Console in the footer", "Показывать Console в футере"))
                            .checked(self.draft.developer.show_console)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.developer.show_console = *checked;
                                cx.notify();
                            })),
                    )
                    .child(note(
                        tr(
                            language,
                            "The console (footer → Console) then shows what Files and every extension run: programs with arguments, exit code, time and output, file and network operations. Off, it shows messages, notifications and errors.",
                            "Тогда консоль (футер → Console) показывает всё, что запускают Files и расширения: программы с аргументами, кодом выхода, временем и выводом, файловые и сетевые операции. Выключено — только сообщения, уведомления и ошибки.",
                        ),
                        palette,
                    )),
            )
            .child(
                card(palette)
                    .child(section(IconName::Code, tr(language, "Shell", "Командная строка"), palette))
                    .child(
                        div()
                            .id("developer-shell-output")
                            .h(px(300.))
                            .p_2()
                            .rounded_md()
                            .bg(palette.surface)
                            .border_1()
                            .border_color(palette.border)
                            .text_xs()
                            .font_family("Menlo")
                            .overflow_y_scrollbar()
                            .child(v_flex().gap_2().children(output)),
                    )
                    .child(Input::new(&self.shell_input).w_full())
                    .child(note(tr(language, "Enter runs a command · cls clears the screen.", "Enter выполняет команду · cls очищает экран."), palette)),
            )
            .into_any_element()
    }

    fn render_accessibility(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        use crate::settings::{TextScale, Toggle};
        let _: Option<TextScale> = None;
        let language = self.draft.language;
        let system = crate::settings::system_accessibility();
        let a11y = self.draft.accessibility;
        // (id, icon, label, hint, current value, macOS value, setter)
        type Setter = fn(&mut crate::settings::AccessibilitySettings, Toggle);
        let rows: [(&str, IconName, &str, &str, Toggle, bool, Setter); 5] = [
            (
                "reduce-motion",
                IconName::Wind,
                tr(language, "Reduce motion", "Уменьшить движение"),
                tr(language, "Dialogs, menus and sliders appear without animation.", "Диалоги, меню и ползунки появляются без анимации."),
                a11y.reduce_motion,
                system.reduce_motion,
                |settings, value| settings.reduce_motion = value,
            ),
            (
                "high-contrast",
                IconName::Contrast,
                tr(language, "High contrast", "Высокий контраст"),
                tr(language, "Brighter secondary text, visible borders, solid panels.", "Яркий вторичный текст, заметные рамки, плотные панели."),
                a11y.high_contrast,
                system.increase_contrast,
                |settings, value| settings.high_contrast = value,
            ),
            (
                "reduce-transparency",
                IconName::Layers,
                tr(language, "Reduce transparency", "Уменьшить прозрачность"),
                tr(language, "The window stays opaque even if Appearance makes panels see-through.", "Окно непрозрачное, даже если в Appearance включена прозрачность."),
                a11y.reduce_transparency,
                system.reduce_transparency,
                |settings, value| settings.reduce_transparency = value,
            ),
            (
                "text-contrast",
                IconName::Type,
                tr(language, "Text contrast", "Контраст текста"),
                tr(language, "Names get an outline of the opposite color: dark text a white edge, light text a dark one.", "Имена получают обводку противоположного цвета: тёмный текст — белую, светлый — тёмную."),
                a11y.text_contrast,
                system.increase_contrast,
                |settings, value| settings.text_contrast = value,
            ),
            (
                "shimmer",
                IconName::Sparkles,
                tr(language, "Shimmer", "Мерцание"),
                tr(language, "Loading text gently pulses. Always off while Reduce motion is on.", "Текст загрузки мягко пульсирует. Всегда выключено при «Уменьшить движение»."),
                a11y.shimmer,
                !system.reduce_motion,
                |settings, value| settings.shimmer = value,
            ),
        ];
        let mut card_rows = v_flex().gap_4();
        for (id, icon, label, hint, value, from_os, set) in rows {
            let choices = [
                (Toggle::System, tr(language, "System", "Как в системе"), IconName::Monitor),
                (Toggle::On, tr(language, "On", "Вкл."), IconName::Check),
                (Toggle::Off, tr(language, "Off", "Выкл."), IconName::X),
            ];
            card_rows = card_rows.child(
                v_flex()
                    .gap_1p5()
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(Icon::new(icon).small().text_color(palette.accent))
                            .child(div().flex_1().text_sm().child(label))
                            .child(h_flex().gap_1().children(choices.map(|(choice, text, choice_icon)| {
                                option_button(
                                    gpui_kit::ElementId::Name(format!("{id}-{text}").into()),
                                    text,
                                    value == choice,
                                )
                                .icon(choice_icon)
                                .xsmall()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    set(&mut this.draft.accessibility, choice);
                                    cx.notify();
                                }))
                            }))),
                    )
                    .child(div().pl(px(28.)).text_xs().text_color(palette.muted).child(format!(
                        "{hint} {}",
                        if from_os {
                            tr(language, "(System: on)", "(Система: вкл.)")
                        } else {
                            tr(language, "(System: off)", "(Система: выкл.)")
                        }
                    ))),
            );
        }
        v_flex()
            .p_5()
            .gap_4()
            .child(
                card(palette)
                    .child(section(IconName::PersonStanding, tr(language, "Accessibility", "Доступность"), palette))
                    .child(note(
                        tr(
                            language,
                            "System follows macOS → System Settings → Accessibility → Display.",
                            "System следует настройкам macOS → Универсальный доступ → Дисплей.",
                        ),
                        palette,
                    ))
                    .child(card_rows),
            )
            .child(
                card(palette)
                    .child(section(IconName::ALargeSmall, tr(language, "Text size", "Размер текста"), palette))
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(div().text_xs().text_color(palette.muted).child("A"))
                            .child(div().flex_1().child(gpui_kit::component::slider::Slider::new(&self.text_scale_slider)))
                            .child(div().text_xl().text_color(palette.muted).child("A")),
                    )
                    .child(
                        // One tick per preset under the slider, like macOS.
                        h_flex()
                            .px(px(18.))
                            .justify_between()
                            .children(TextScale::ALL.map(|scale| {
                                div()
                                    .w(px(2.))
                                    .h(px(6.))
                                    .rounded_full()
                                    .bg(if scale == a11y.text_scale { palette.accent } else { palette.border })
                            })),
                    )
                    .child(
                        h_flex()
                            .justify_center()
                            .gap_2()
                            .text_sm()
                            .child(a11y.text_scale.label())
                            .child(div().text_color(palette.muted).child(format!("{:.0}%", a11y.text_scale.factor() * 100.))),
                    )
                    .child(
                        div()
                            .p_3()
                            .rounded_md()
                            .bg(palette.surface)
                            .text_size(px(14. * a11y.text_scale.factor()))
                            .child(tr(language, "Preview: Documents · 12 items · Today, 10:42", "Пример: Документы · 12 элементов · Сегодня, 10:42")),
                    )
                    .child(note(
                        tr(language, "Scales text and controls across the window.", "Масштабирует текст и элементы окна."),
                        palette,
                    )),
            )
            .child(note(tr(language, "Changes here are applied with Save.", "Изменения применяются кнопкой Save."), palette))
            .into_any_element()
    }

    fn render_formats(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let language = self.draft.language;
        let size_example = format_size(4_816_896, self.draft.human_sizes);
        let date_example = format_demo_date("Today, 10:42", self.draft.date_time);
        v_flex()
            .p_5()
            .gap_4()
            .child(section(IconName::HardDrive, tr(language, "Human sizes", "Размеры файлов"), palette))
            .child(setting_row(
                tr(language, "Decimal places", "Знаков после запятой"),
                h_flex().gap_2().children((0..=2).map(|decimals| {
                    option_button(
                        ("decimals", decimals as usize),
                        decimals.to_string(),
                        self.draft.human_sizes.decimals == decimals,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.draft.human_sizes.decimals = decimals;
                        cx.notify();
                    }))
                })),
                palette,
            ))
            .child(setting_row(
                tr(language, "Size base", "Основание размера"),
                h_flex()
                    .gap_2()
                    .child(
                        option_button(
                            "size-binary",
                            "1024 · Binary",
                            self.draft.human_sizes.base == SizeBase::Binary,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft.human_sizes.base = SizeBase::Binary;
                            cx.notify();
                        })),
                    )
                    .child(
                        option_button(
                            "size-decimal",
                            "1000 · Decimal",
                            self.draft.human_sizes.base == SizeBase::Decimal,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft.human_sizes.base = SizeBase::Decimal;
                            cx.notify();
                        })),
                    ),
                palette,
            ))
            .child(setting_row(
                tr(language, "Size example", "Пример размера"),
                div().text_sm().child(size_example),
                palette,
            ))
            .child(section(IconName::CalendarClock, tr(language, "Date and time", "Дата и время"), palette))
            .child(setting_row(
                tr(language, "Date format", "Формат даты"),
                h_flex()
                    .gap_2()
                    .child(
                        option_button(
                            "date-relative",
                            tr(language, "Relative", "Относительная"),
                            self.draft.date_time.date_format == DateFormat::Relative,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft.date_time.date_format = DateFormat::Relative;
                            cx.notify();
                        })),
                    )
                    .child(
                        option_button(
                            "date-dmy",
                            "DMY",
                            self.draft.date_time.date_format == DateFormat::Dmy,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft.date_time.date_format = DateFormat::Dmy;
                            cx.notify();
                        })),
                    )
                    .child(
                        option_button(
                            "date-iso",
                            "ISO",
                            self.draft.date_time.date_format == DateFormat::Iso,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft.date_time.date_format = DateFormat::Iso;
                            cx.notify();
                        })),
                    ),
                palette,
            ))
            .child(setting_row(
                tr(language, "Clock", "Время"),
                h_flex()
                    .gap_2()
                    .child(
                        option_button(
                            "clock-24",
                            "24h",
                            self.draft.date_time.clock_format == ClockFormat::Hour24,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft.date_time.clock_format = ClockFormat::Hour24;
                            cx.notify();
                        })),
                    )
                    .child(
                        option_button(
                            "clock-12",
                            "12h",
                            self.draft.date_time.clock_format == ClockFormat::Hour12,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft.date_time.clock_format = ClockFormat::Hour12;
                            cx.notify();
                        })),
                    ),
                palette,
            ))
            .child(setting_row(
                tr(language, "Live example", "Живой пример"),
                div().text_sm().child(date_example),
                palette,
            ))
            .into_any_element()
    }

}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shared = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&shared);
        let language = self.draft.language;
        let dirty = self.draft != self.original;
        let has_conflicts = !crate::keymap::conflicts(&self.draft.keymap, cx.platform_info().is_mac()).is_empty();
        let tabs = [
            (IconName::SlidersHorizontal, tr(language, "General", "Основные")),
            (IconName::Palette, tr(language, "Appearance", "Оформление")),
            (IconName::Puzzle, tr(language, "Extensions", "Расширения")),
            (IconName::Ruler, tr(language, "Formats", "Форматы")),
            (IconName::Trash, tr(language, "Deletion", "Удаление")),
            (IconName::AppWindow, tr(language, "Default apps", "Приложения")),
            (IconName::Keyboard, tr(language, "Hotkeys", "Горячие клавиши")),
            (IconName::PersonStanding, tr(language, "Accessibility", "Доступность")),
            (IconName::SquareTerminal, tr(language, "Developer", "Разработчик")),
            (IconName::Volume2, tr(language, "Sound", "Звук")),
            (IconName::HardDrive, tr(language, "Storage", "Хранилище")),
        ];
        let content = match self.active_tab {
            0 => self.render_general(palette, cx),
            1 => self.render_appearance(palette, cx),
            2 => self.render_extensions(palette, cx),
            3 => self.render_formats(palette, cx),
            4 => self.render_deletion(palette, cx),
            5 => self.render_default_apps(palette, cx),
            6 => self.render_hotkeys(palette, window, cx),
            7 => self.render_accessibility(palette, cx),
            8 => self.render_developer(palette, cx),
            9 => self.render_sound(palette, cx),
            _ => self.render_storage(palette, cx),
        };
        let path = settings_path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| {
                tr(
                    language,
                    "Settings path unavailable",
                    "Путь к настройкам недоступен",
                )
                .to_string()
            });

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("Settings")
            .on_action(cx.listener(Self::close))
            .on_action(cx.listener(Self::previous_section))
            .on_action(cx.listener(Self::next_section))
            .on_action(cx.listener(Self::focus_next))
            .on_action(cx.listener(Self::focus_previous))
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .focus_visible(|style| style.border_color(palette.accent))
            .bg(palette.window)
            .text_color(palette.text)
            .child(
                h_flex()
                    .h(px(48.))
                    .flex_shrink_0()
                    .px_4()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(palette.border)
                    .bg(palette.sidebar)
                    .child(
                        div()
                            .text_base()
                            .child(tr(language, "Settings", "Настройки")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if dirty { palette.accent } else { palette.faint })
                            .child(if dirty {
                                tr(language, "Unsaved changes", "Есть несохранённые изменения")
                            } else {
                                tr(language, "Saved", "Сохранено")
                            }),
                    )
                    .child(div().flex_1())
                    .when(!self.in_tab, |this| {
                        this.child(
                            Button::new("settings-in-tab")
                                .ghost()
                                .small()
                                .icon(IconName::PanelTop)
                                .label(tr(language, "Open in tab", "Открыть во вкладке"))
                                .tooltip(tr(language, "Keep Settings open beside your files", "Держать настройки открытыми рядом с файлами"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    // Unsaved edits are not lost: the tab starts from this draft.
                                    let draft = this.draft.clone();
                                    this.dismiss(window, cx);
                                    PENDING_DRAFT.with(|pending| *pending.borrow_mut() = Some(draft));
                                    window.dispatch_action(Box::new(crate::app::OpenSettingsTab), cx);
                                })),
                        )
                        .child(
                            Button::new("close-settings")
                                .ghost()
                                .small()
                                .label(tr(language, "Close", "Закрыть"))
                                .on_click(cx.listener(|this, _, window, cx| this.dismiss(window, cx))),
                        )
                    }),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child({
                        // Scrolls when the unfolded extensions push the
                        // last sections past the bottom of the modal.
                        let items = v_flex().p_3().gap_1().children(tabs.into_iter().enumerate().map(|(index, (icon, label))| {
                            if index == EXTENSIONS_TAB {
                                self.extensions_menu(icon, label, palette, cx)
                            } else {
                                self.section_menu_item(index, icon, label, palette, cx)
                            }
                        }));
                        #[cfg(test)]
                        let items = items.debug_selector(|| "settings-menu-items".into());
                        div()
                            .id("settings-menu")
                            .w(px(196.))
                            .flex_shrink_0()
                            .h_full()
                            .min_h_0()
                            .border_r_1()
                            .border_color(palette.border)
                            .bg(palette.sidebar)
                            .overflow_y_scrollbar()
                            .child(items)
                    })
                    .child(
                        div()
                            .id("settings-scroll")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .min_h_0()
                            .overflow_y_scrollbar()
                            .child(content),
                    ),
            )
            .child(
                v_flex()
                    .flex_shrink_0()
                    .px_4()
                    .py_2()
                    .gap_1()
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(palette.faint)
                            .child(path),
                    )
                    .children(self.error.as_ref().map(|error| {
                        div()
                            .text_xs()
                            .text_color(palette.video)
                            .child(error.clone())
                            .into_any_element()
                    })),
            )
            .child(
                h_flex()
                    .h(px(54.))
                    .flex_shrink_0()
                    .px_4()
                    .items_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(palette.border)
                    .bg(palette.sidebar)
                    .child(
                        Button::new("reset-settings")
                            .ghost()
                            .small()
                            .label(tr(language, "Reset to defaults", "Сбросить настройки"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.draft = AppSettings::default();
                                this.sync_sliders(window, cx);
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .children((self.just_saved && !dirty).then(|| {
                        h_flex()
                            .gap_1()
                            .items_center()
                            .text_xs()
                            .text_color(palette.code)
                            .child(Icon::new(IconName::Check).xsmall())
                            .child(tr(language, "Saved", "Сохранено"))
                    }))
                    .child(footer_button(
                        "settings-cancel",
                        Button::new("cancel-settings")
                            .secondary()
                            .small()
                            .label(tr(language, "Cancel", "Отмена"))
                            // Drops unsaved edits and closes the modal.
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.draft = this.original.clone();
                                this.dismiss(window, cx);
                                cx.notify();
                            })),
                    ))
                    .child(footer_button(
                        "settings-save",
                        Button::new("save-settings")
                            .primary()
                            .small()
                            .label(tr(language, "Save", "Сохранить"))
                            .disabled(!dirty || has_conflicts)
                            .tooltip(if has_conflicts {
                                tr(language, "Fix the hotkey conflicts first", "Сначала исправьте конфликты сочетаний")
                            } else if !dirty {
                                tr(language, "No changes to save", "Нет изменений")
                            } else {
                                tr(language, "Save settings", "Сохранить настройки")
                            })
                            // Writes settings and keeps the window open.
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save(cx);
                                this.just_saved = this.error.is_none();
                                cx.notify();
                            })),
                    )),
            )
    }
}

thread_local! {
    /// The modal's unsaved edits, carried into the tab it moves to.
    static PENDING_DRAFT: std::cell::RefCell<Option<AppSettings>> = const { std::cell::RefCell::new(None) };
}

impl SettingsWindow {
    /// Takes over the edits of the modal it replaces, if any.
    pub(crate) fn adopt_pending_draft(&mut self) {
        if let Some(draft) = PENDING_DRAFT.with(|pending| pending.borrow_mut().take()) {
            self.draft = draft;
        }
    }
}

/// Opens Settings as a centered modal dialog over `window`.
pub fn open_settings_dialog(
    appearance: Entity<AppearanceSettings>,
    window: &mut Window,
    cx: &mut App,
) {
    if window.has_active_dialog(cx) {
        return;
    }
    // Restored on close so shortcuts keep working without a click.
    let previous_focus = window.focused(cx);
    let settings = cx.new(|cx| {
        let mut settings = SettingsWindow::new(window, cx, appearance);
        settings.return_focus = previous_focus.clone();
        settings
    });
    let focus = settings.read(cx).focus_handle.clone();
    let initial_focus = std::cell::Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if initial_focus.replace(false) {
            focus.focus(window, cx);
        }
        dialog
            .w(px(820.))
            .p_0()
            .centered()
            .close_button(false)
            .overlay(true)
            .overlay_closable(true)
            // Enter must not dismiss the dialog (and drop unsaved edits) like Cancel.
            .on_ok(|_, _, _| false)
            .on_close({
                let previous_focus = previous_focus.clone();
                move |_, window, cx| {
                    if let Some(focus) = &previous_focus {
                        focus.focus(window, cx);
                    }
                }
            })
            // The settings view fills its parent, so give it a definite height.
            .child(
                div()
                    .w_full()
                    .h(px(620.))
                    .overflow_hidden()
                    .rounded_lg()
                    .child(settings.clone()),
            )
    });
}


/// Wraps a footer button so tests can find and click it.
fn footer_button(_selector: &'static str, button: Button) -> AnyElement {
    let wrapper = div().child(button);
    #[cfg(test)]
    let wrapper = wrapper.debug_selector(move || _selector.into());
    wrapper.into_any_element()
}

/// One item an extension puts in the context menu, for the Settings preview.
struct PreviewItem {
    label: &'static str,
    /// `None`: the extension's own icon.
    icon: Option<IconName>,
    children: Vec<(&'static str, IconName)>,
}

impl PreviewItem {
    fn icon(&self, manifest: &files_sdk::Manifest) -> Icon {
        self.icon.clone().map_or_else(|| crate::extensions::icon_of(manifest), Icon::new)
    }
}

/// What each extension adds to context menus (its SDK items included).
fn menu_preview(manifest: &'static files_sdk::Manifest) -> Vec<PreviewItem> {
    let item = |label, icon: Option<IconName>, children: Vec<(&'static str, IconName)>| PreviewItem { label, icon, children };
    let mut items = match manifest.id {
        "open-with" => vec![item(
            "Open With",
            None,
            vec![("Default Application", IconName::SquareArrowOutUpRight), ("Choose Application…", IconName::LayoutGrid)],
        )],
        "archives" => vec![item("Compress…", Some(IconName::FileArchive), vec![]), item("Extract…", Some(IconName::FolderOpen), vec![])],
        "git" => vec![item(
            "Git",
            None,
            vec![
                ("Git Commit…", IconName::GitCommitHorizontal),
                ("Diff", IconName::GitCompare),
                ("Show Log", IconName::Clock3),
                ("Pull", IconName::ArrowDownToLine),
                ("Push", IconName::ArrowUpFromLine),
                ("Switch/Checkout…", IconName::GitBranch),
                ("Stash Save", IconName::Archive),
                ("Revert…", IconName::Undo2),
            ],
        )],
        "copy" => vec![item(
            "Copy",
            None,
            vec![("Copy Name", IconName::Copy), ("Copy Path", IconName::Copy), ("Copy File Reference", IconName::Link)],
        )],
        "ssh" => vec![item("Download to Downloads", Some(IconName::Download), vec![]), item("Refresh", Some(IconName::RefreshCw), vec![])],
        "forks" => vec![item("Fork folder", None, vec![])],
        "agents" => vec![item(
            "Open in Agents",
            None,
            vec![
                ("Open in Files", IconName::Bot),
                ("Open in Terminal", IconName::SquareTerminal),
                ("Open in Claude (desktop)", IconName::Bot),
                ("Open in Codex", IconName::Bot),
                ("Open in OpenCode", IconName::Bot),
            ],
        )],
        _ => Vec::new(),
    };
    items.extend(manifest.menu.iter().map(|menu| item(menu.title, None, vec![])));
    items
}

#[cfg(test)]
pub(crate) fn menu_preview_titles(manifest: &'static files_sdk::Manifest) -> Vec<(&'static str, Vec<&'static str>)> {
    menu_preview(manifest).into_iter().map(|item| (item.label, item.children.iter().map(|(label, _)| *label).collect())).collect()
}

/// The first font choice: whatever the theme uses.
const THEME_FONT: &str = "Theme default";

/// "Theme default", then every system font family, sorted.
fn font_choices(cx: &App) -> Vec<String> {
    let mut fonts: Vec<String> = cx.text_system().all_font_names().into_iter().filter(|name| !name.starts_with('.') && !name.trim().is_empty()).collect();
    fonts.sort_by_key(|name| name.to_lowercase());
    fonts.dedup();
    std::iter::once(THEME_FONT.to_string()).chain(fonts).collect()
}

/// Settings → Storage is the last section.
const STORAGE_TAB: usize = 10;

/// The cache limit slider's steps in MB, "no limit" (0) last.
const STORAGE_LIMITS: [u32; 8] = [256, 512, 1024, 2048, 5120, 10240, 20480, 0];

fn storage_step(megabytes: u32) -> usize {
    STORAGE_LIMITS.iter().position(|step| *step == megabytes).unwrap_or_else(|| {
        // A hand-edited value: the nearest step below it.
        STORAGE_LIMITS.iter().rposition(|step| *step != 0 && *step <= megabytes).unwrap_or(0)
    })
}

fn limit_label(megabytes: u32, language: crate::settings::Language) -> String {
    match megabytes {
        0 => tr(language, "No limit", "Без лимита").to_string(),
        megabytes if megabytes >= 1024 => format!("{} GB", megabytes / 1024),
        megabytes => format!("{megabytes} MB"),
    }
}

/// "1 file", "12 files", "12 файлов".
fn files_count(count: usize, language: crate::settings::Language) -> String {
    match language {
        crate::settings::Language::Russian => {
            let word = match (count % 10, count % 100) {
                (1, hundred) if hundred != 11 => "файл",
                (2..=4, hundred) if !(12..=14).contains(&hundred) => "файла",
                _ => "файлов",
            };
            format!("{count} {word}")
        }
        _ => format!("{count} {}", if count == 1 { "file" } else { "files" }),
    }
}

fn option_button(
    id: impl Into<gpui_kit::ElementId>,
    label: impl Into<gpui_kit::SharedString>,
    selected: bool,
) -> Button {
    let button = Button::new(id).small().label(label);
    if selected {
        button.primary()
    } else {
        button.secondary()
    }
}

fn setting_row(label: &'static str, control: impl IntoElement, palette: Palette) -> AnyElement {
    h_flex()
        .min_h(px(34.))
        .items_center()
        .gap_4()
        .child(
            div()
                .w(px(190.))
                .flex_shrink_0()
                .text_sm()
                .text_color(palette.muted)
                .child(selectable(label)),
        )
        .child(control)
        .into_any_element()
}

fn section(icon: IconName, label: &'static str, palette: Palette) -> AnyElement {
    h_flex()
        .gap_2()
        .items_center()
        .text_size(px(11.))
        .text_color(palette.accent)
        .child(Icon::new(icon).small())
        .child(label)
        .into_any_element()
}

/// A titled group with comfortable padding, used where a tab has several parts.
fn card(palette: Palette) -> gpui_kit::Div {
    v_flex()
        .gap_3()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(palette.border)
        .bg(palette.sidebar)
}

/// Text in Settings can be selected and copied (⌘C), like a document.
fn selectable(text: &'static str) -> gpui_kit::base::SelectableText {
    gpui_kit::base::SelectableText::new(gpui_kit::ElementId::Name(format!("settings-text-{text}").into()), text)
}

fn note(label: &'static str, palette: Palette) -> AnyElement {
    div()
        .max_w(px(560.))
        .text_xs()
        .text_color(palette.faint)
        .child(selectable(label))
        .into_any_element()
}

#[cfg(test)]
mod storage_tests {
    use super::*;

    #[test]
    fn limit_steps_round_trip_and_read() {
        for (index, megabytes) in STORAGE_LIMITS.iter().enumerate() {
            assert_eq!(storage_step(*megabytes), index);
        }
        assert_eq!(storage_step(3000), 3, "a hand-edited value snaps to the step below");
        assert_eq!(limit_label(1024, crate::settings::Language::English), "1 GB");
        assert_eq!(limit_label(512, crate::settings::Language::English), "512 MB");
        assert_eq!(limit_label(0, crate::settings::Language::Russian), "Без лимита");
        assert_eq!(files_count(22, crate::settings::Language::Russian), "22 файла");
        assert_eq!(files_count(11, crate::settings::Language::Russian), "11 файлов");
    }
}
