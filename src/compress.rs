use gpui_kit::component::scroll::ScrollableElement as _;


use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window, WindowBounds,
    WindowOptions, actions, div, point, px, size,
};

pub use files_ext_archive::compress::*;

use crate::app::AppearanceSettings;
use crate::model::FileEntry;
use crate::palette::Palette;
use crate::settings::format_size;

actions!(compress, [CloseCompress]);





















struct CompressJob {
    sources: Sources,
    loaded: bool,
}

pub struct CompressWindow {
    roots: Vec<PathBuf>,
    job: CompressJob,
    load_error: Option<String>,
    name: Entity<InputState>,
    destination: Entity<InputState>,
    pending_destination: Option<PathBuf>,
    settings: CompressSettings,
    overwrite: bool,
    running: bool,
    progress: Arc<AtomicUsize>,
    result: Option<Result<String, String>>,
    appearance: Entity<AppearanceSettings>,
    focus_handle: FocusHandle,
    _appearance_subscription: Subscription,
}

impl CompressWindow {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        source: PathBuf,
        appearance: Entity<AppearanceSettings>,
    ) -> Self {
        let appearance_subscription = cx.observe(&appearance, |_, _, cx| cx.notify());
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let settings = CompressSettings::default();
        let stem = source
            .file_name()
            .map_or_else(|| "Archive".to_string(), |name| name.to_string_lossy().into_owned());
        let default_name = format!("{stem}.{}", settings.format.extension());
        let default_destination = source
            .parent()
            .map(|parent| parent.display().to_string())
            .unwrap_or_default();
        let name = cx.new(|cx| InputState::new(window, cx).default_value(default_name));
        let destination = cx.new(|cx| InputState::new(window, cx).default_value(default_destination));
        let roots = vec![source];

        let scan_roots = roots.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { collect_sources(&scan_roots) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(sources) => this.job.sources = sources,
                    Err(error) => this.load_error = Some(error.to_string()),
                }
                this.job.loaded = true;
                cx.notify();
            });
        })
        .detach();

        Self {
            roots,
            job: CompressJob {
                sources: Sources::default(),
                loaded: false,
            },
            load_error: None,
            name,
            destination,
            pending_destination: None,
            settings,
            overwrite: false,
            running: false,
            progress: Arc::new(AtomicUsize::new(0)),
            result: None,
            appearance,
            focus_handle,
            _appearance_subscription: appearance_subscription,
        }
    }

    pub fn set_destination(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.pending_destination = Some(path);
        cx.notify();
    }

    fn on_close(&mut self, _: &CloseCompress, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    fn requested_output(&self, cx: &App) -> PathBuf {
        PathBuf::from(self.destination.read(cx).value().trim())
            .join(self.name.read(cx).value().trim())
    }

    /// Where the archive will really go: never over an existing file unless asked.
    fn final_output(&self, cx: &App) -> PathBuf {
        let requested = self.requested_output(cx);
        if requested.exists() && !self.overwrite {
            crate::archive::unique_path(&requested)
        } else {
            requested
        }
    }

    fn set_format(&mut self, format: ArchiveFormat, window: &mut Window, cx: &mut Context<Self>) {
        let previous = self.settings.format;
        self.settings.format = format;
        let name = self.name.read(cx).value().to_string();
        let stem = name
            .strip_suffix(&format!(".{}", previous.extension()))
            .unwrap_or(&name)
            .to_string();
        self.name.update(cx, |state, cx| {
            state.set_value(format!("{stem}.{}", format.extension()), window, cx);
        });
        cx.notify();
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        let current = PathBuf::from(self.destination.read(cx).value().trim());
        let start = if current.is_dir() {
            current
        } else {
            self.roots[0].parent().map(Path::to_path_buf).unwrap_or_default()
        };
        let target = cx.entity().downgrade();
        let picker = crate::app::FolderPicker {
            title: "Save archive in",
            on_pick: std::rc::Rc::new(move |path, cx| {
                let _ = target.update(cx, |window, cx| window.set_destination(path, cx));
            }),
        };
        crate::app::open_folder_picker(start, picker, self.appearance.clone(), cx);
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let destination = PathBuf::from(self.destination.read(cx).value().trim());
        if !destination.is_dir() {
            self.result = Some(Err(format!("Folder does not exist: {}", destination.display())));
            cx.notify();
            return;
        }
        if self.name.read(cx).value().trim().is_empty() {
            self.result = Some(Err("Enter an archive name".to_string()));
            cx.notify();
            return;
        }
        let output = self.final_output(cx);
        let sources = self.job.sources.clone();
        let roots = self.roots.clone();
        let settings = self.settings;
        let progress = self.progress.clone();
        progress.store(0, Ordering::Relaxed);
        self.running = true;
        self.result = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let job_output = output.clone();
            let job_progress = progress.clone();
            let task = cx.background_executor().spawn(async move {
                create_archive(&sources, &roots, &job_output, settings, &job_progress)
            });
            let original: u64 = this
                .update(cx, |this, _| this.job.sources.files.iter().map(|file| file.size).sum())
                .unwrap_or(0);
            // Repaint progress while the background job runs.
            let ticker = this.clone();
            let mut ticker_cx = cx.clone();
            cx.foreground_executor()
                .spawn(async move {
                    loop {
                        ticker_cx
                            .background_executor()
                            .timer(std::time::Duration::from_millis(200))
                            .await;
                        let alive = ticker
                            .update(&mut ticker_cx, |this, cx| {
                                cx.notify();
                                this.running
                            })
                            .unwrap_or(false);
                        if !alive {
                            break;
                        }
                    }
                })
                .detach();
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.running = false;
                let settings = this.appearance.read(cx).clone();
                this.result = Some(match result {
                    Ok(bytes) => {
                        let ratio = if original > 0 {
                            format!(" · {:.0}% of original", bytes as f64 * 100. / original as f64)
                        } else {
                            String::new()
                        };
                        Ok(format!(
                            "Created {} · {}{ratio}",
                            output.display(),
                            format_size(bytes, settings.human_sizes)
                        ))
                    }
                    Err(error) => Err(format!("Compression failed: {error}")),
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn level_picker(
        &self,
        id: &'static str,
        current: Level,
        palette: Palette,
        cx: &mut Context<Self>,
        on_pick: impl Fn(&mut Self, Level) + Clone + 'static,
    ) -> AnyElement {
        let _ = palette;
        h_flex()
            .gap_1()
            .children(Level::ALL.map(|level| {
                let on_pick = on_pick.clone();
                let button = Button::new(gpui_kit::ElementId::Name(format!("{id}-{}", level.label()).into()))
                    .xsmall()
                    .label(level.label())
                    .disabled(self.running)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        on_pick(this, level);
                        cx.notify();
                    }));
                if level == current { button.primary() } else { button.ghost() }
            }))
            .into_any_element()
    }
}

impl Render for CompressWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let appearance = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&appearance);
        if let Some(path) = self.pending_destination.take() {
            self.destination.update(cx, |state, cx| {
                state.set_value(path.display().to_string(), window, cx);
            });
        }
        let sources = &self.job.sources;
        let total: u64 = sources.files.iter().map(|file| file.size).sum();
        let rar_available = rar_tool().is_some();
        let requested = self.requested_output(cx);
        let output = self.final_output(cx);

        let format_row = h_flex().gap_1().children(ArchiveFormat::ALL.map(|format| {
            let unavailable = format == ArchiveFormat::Rar && !rar_available;
            let mut button = Button::new(gpui_kit::ElementId::Name(format!("format-{}", format.label()).into()))
                .small()
                .label(format.label())
                .disabled(self.running || unavailable)
                .on_click(cx.listener(move |this, _, window, cx| this.set_format(format, window, cx)));
            if unavailable {
                button = button.tooltip("RAR is proprietary: install RARLAB's `rar` tool to create it");
            }
            if format == self.settings.format { button.primary() } else { button.ghost() }
        }));

        let levels = if !self.job.loaded {
            div().text_xs().text_color(palette.muted).child("Scanning files…").into_any_element()
        } else if self.settings.format.per_category() {
            let mut rows = v_flex().gap_2();
            for category in Category::ALL {
                let files: Vec<_> = sources.files.iter().filter(|file| file.category == category).collect();
                if files.is_empty() {
                    continue;
                }
                let bytes: u64 = files.iter().map(|file| file.size).sum();
                let index = category.index();
                rows = rows.child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .p_2()
                        .rounded_md()
                        .bg(palette.surface)
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().text_sm().child(format!(
                                    "{} · {} files · {}",
                                    category.label(),
                                    files.len(),
                                    format_size(bytes, appearance.human_sizes)
                                )))
                                .child(div().text_size(px(10.)).text_color(palette.muted).child(category.hint())),
                        )
                        .child(self.level_picker(
                            match category {
                                Category::CompressedMedia => "level-media",
                                Category::RawMedia => "level-raw",
                                Category::Documents => "level-docs",
                                Category::Archives => "level-archives",
                                Category::TextCode => "level-text",
                                Category::Other => "level-other",
                            },
                            self.settings.levels[index],
                            palette,
                            cx,
                            move |this, level| this.settings.levels[index] = level,
                        )),
                );
            }
            rows.into_any_element()
        } else {
            h_flex()
                .gap_3()
                .items_center()
                .child(div().flex_1().text_sm().child(format!(
                    "{} compresses everything as one stream",
                    self.settings.format.label()
                )))
                .child(self.level_picker(
                    "level-single",
                    self.settings.single_level,
                    palette,
                    cx,
                    |this, level| this.settings.single_level = level,
                ))
                .into_any_element()
        };

        let progress = self.progress.load(Ordering::Relaxed);
        let can_start = self.job.loaded && !self.running && self.load_error.is_none()
            && (!sources.files.is_empty() || !sources.dirs.is_empty());

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("Compress")
            .on_action(cx.listener(Self::on_close))
            .size_full()
            .bg(palette.window)
            .text_color(palette.text)
            .child(
                div()
                    .id("compress-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .p_5()
                            .gap_4()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Compress"))
                                    .child(div().text_xs().text_color(palette.muted).child(if self.job.loaded {
                                        format!(
                                            "{} files · {} folders · {}",
                                            sources.files.len(),
                                            sources.dirs.len(),
                                            format_size(total, appearance.human_sizes)
                                        )
                                    } else {
                                        "Scanning files…".to_string()
                                    })),
                            )
                            .child(section("FORMAT", palette))
                            .child(format_row)
                            .child(section(
                                if self.settings.format.per_category() {
                                    "COMPRESSION BY FILE TYPE"
                                } else {
                                    "COMPRESSION"
                                },
                                palette,
                            ))
                            .child(levels)
                            .child(section("SAVE AS", palette))
                            .child(Input::new(&self.name).w_full())
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(div().flex_1().child(Input::new(&self.destination).w_full()))
                                    .child(
                                        Button::new("compress-browse")
                                            .secondary()
                                            .small()
                                            .icon(IconName::FolderOpen)
                                            .label("Browse…")
                                            .disabled(self.running)
                                            .on_click(cx.listener(|this, _, _, cx| this.browse(cx))),
                                    ),
                            )
                            .children(requested.exists().then(|| {
                                v_flex()
                                    .gap_2()
                                    .p_3()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(palette.archive)
                                    .text_xs()
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .child(Icon::new(IconName::TriangleAlert).small().text_color(palette.archive))
                                            .child(format!(
                                                "{} already exists",
                                                requested.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned())
                                            )),
                                    )
                                    .child(
                                        Checkbox::new("compress-overwrite")
                                            .label("Overwrite it instead of keeping both")
                                            .checked(self.overwrite)
                                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                                this.overwrite = *checked;
                                                cx.notify();
                                            })),
                                    )
                            }))
                            .child(
                                v_flex()
                                    .gap_1()
                                    .p_3()
                                    .rounded_md()
                                    .bg(palette.surface)
                                    .text_xs()
                                    .child(div().text_color(palette.muted).child("Archive will be written to"))
                                    .child(output.display().to_string()),
                            )
                            .children((!sources.online_only.is_empty()).then(|| {
                                div().text_xs().text_color(palette.archive).child(format!(
                                    "{} online-only cloud files will be skipped (not downloaded): {}",
                                    sources.online_only.len(),
                                    sources.online_only.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
                                ))
                            }))
                            .children(self.load_error.clone().map(|error| {
                                div().text_xs().text_color(palette.video).child(format!("Cannot read files: {error}"))
                            }))
                            .children(self.result.clone().map(|result| match result {
                                Ok(message) => div().text_xs().text_color(palette.code).child(message),
                                Err(message) => div().text_xs().text_color(palette.video).child(message),
                            })),
                    ),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .p_3()
                    .gap_2()
                    .border_t_1()
                    .border_color(palette.border)
                    .bg(palette.sidebar)
                    .child(div().flex_1().text_xs().text_color(palette.muted).child(if self.running {
                        if self.settings.format == ArchiveFormat::Rar {
                            "Compressing with rar…".to_string()
                        } else {
                            format!("Compressing… {progress} / {} files", sources.files.len())
                        }
                    } else {
                        String::new()
                    }))
                    .child(
                        Button::new("compress-cancel")
                            .ghost()
                            .small()
                            .label("Close")
                            .disabled(self.running)
                            .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                    )
                    .child(
                        Button::new("compress-start")
                            .primary()
                            .small()
                            .icon(IconName::FileArchive)
                            .label(if self.running { "Compressing…" } else { "Create archive" })
                            .disabled(!can_start)
                            .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                    ),
            )
    }
}

fn section(title: &'static str, palette: Palette) -> AnyElement {
    div()
        .pt_1()
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(palette.faint)
        .child(title)
        .into_any_element()
}

pub fn open_compress(entry: &FileEntry, appearance: Entity<AppearanceSettings>, cx: &mut App) {
    let Some(source) = entry.path.as_deref().map(Path::to_path_buf) else {
        return;
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(320.), px(110.)),
            size: size(px(640.), px(720.)),
        })),
        window_min_size: Some(size(px(520.), px(520.))),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some(format!("{} — Compress", entry.name).into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let opened = gpui_kit::open_window(options, cx, move |window, cx| {
        let appearance = appearance.clone();
        let source = source.clone();
        cx.new(|cx| CompressWindow::new(window, cx, source, appearance))
    });
    if let Err(error) = opened {
        eprintln!("Failed to open Compress window: {error}");
    }
}


