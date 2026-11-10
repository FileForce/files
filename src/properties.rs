use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Window, WindowBounds,
    WindowOptions, actions, div, point, px, size,
};

use crate::app::AppearanceSettings;
use crate::model::{FileEntry, Location, SshConnection};
use crate::palette::Palette;
use crate::settings::{format_demo_date, format_size, localized as tr};
use crate::views::files::file_thumbnail;

actions!(properties, [CloseProperties]);

pub struct PropertiesWindow {
    entry: &'static FileEntry,
    location: Location,
    ssh_connection: Option<SshConnection>,
    appearance: Entity<AppearanceSettings>,
    focus_handle: FocusHandle,
    _appearance_subscription: Subscription,
}

impl PropertiesWindow {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        entry: &'static FileEntry,
        location: Location,
        ssh_connection: Option<SshConnection>,
        appearance: Entity<AppearanceSettings>,
    ) -> Self {
        let appearance_subscription = cx.observe(&appearance, |_, _, cx| cx.notify());
        watch_folder_sizes(cx);
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        Self {
            entry,
            location,
            ssh_connection,
            appearance,
            focus_handle,
            _appearance_subscription: appearance_subscription,
        }
    }

    fn on_close(&mut self, _: &CloseProperties, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }
}

impl Render for PropertiesWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&settings);
        let file = self.entry;
        let bytes = demo_size_bytes(file);
        let symlink = symlink_target(file);
        let location = if self.location == Location::Recents {
            file.parent
        } else {
            self.location
        };
        let location = if let Some(folder) = location.remote_path() {
            self.ssh_connection.as_ref().map_or_else(
                || format!("{folder} · disconnected"),
                |connection| format!("ssh://{}{folder}", connection.address()),
            )
        } else {
            file.path
                .and_then(|path| path.parent())
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| location.label().to_string())
        };
        let tags = if file.tags.is_empty() {
            "None".to_string()
        } else {
            file.tags.join(", ")
        };

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("Properties")
            .on_action(cx.listener(Self::on_close))
            .size_full()
            .min_h_0()
            .bg(palette.window)
            .text_color(palette.text)
            .child(
                h_flex()
                    .h(px(46.))
                    .flex_shrink_0()
                    .px_4()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(palette.border)
                    .bg(palette.sidebar)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Icon::new(IconName::FileText)
                                    .small()
                                    .text_color(palette.accent),
                            )
                            .child(div().text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(tr(settings.language, "Properties", "Свойства"))),
                    )
                    .child(
                        Button::new("close-properties")
                            .secondary()
                            .small()
                            .label(tr(settings.language, "Close", "Закрыть"))
                            .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                    ),
            )
            .child(
                div()
                    .id("properties-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .p_5()
                            .gap_5()
                            .child(file_thumbnail(file, palette, 170.))
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_lg()
                                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
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
                                v_flex()
                                    .gap_3()
                                    .child(section(tr(settings.language, "DETAILS", "СВЕДЕНИЯ"), palette))
                                    .child(detail_row(tr(settings.language, "Type", "Тип"), file.kind.label(), palette))
                                    .child(detail_row(tr(settings.language, "Location", "Расположение"), location, palette))
                                    .child(detail_row(tr(settings.language, "Size", "Размер"), demo_human_size(file, settings.human_sizes), palette))
                                    .child(detail_row(
                                        tr(settings.language, "Size (bytes)", "Размер (байты)"),
                                        bytes.map_or_else(|| "…".to_string(), format_grouped),
                                        palette,
                                    ))
                                    .child(detail_row(tr(settings.language, "Created", "Создан"), format_demo_date(&created(file), settings.date_time), palette))
                                    .child(detail_row(tr(settings.language, "Modified", "Изменён"), format_demo_date(file.modified, settings.date_time), palette)),
                            )
                            .child(
                                v_flex()
                                    .gap_3()
                                    .child(section(tr(settings.language, "FILE SYSTEM", "ФАЙЛОВАЯ СИСТЕМА"), palette))
                                    .child(detail_row(
                                        tr(settings.language, "Symlink", "Символическая ссылка"),
                                        if symlink.is_some() { tr(settings.language, "Yes", "Да") } else { tr(settings.language, "No", "Нет") },
                                        palette,
                                    ))
                                    .child(detail_row(
                                        tr(settings.language, "Symlink target", "Цель ссылки"),
                                        symlink.clone().unwrap_or_else(|| "—".to_string()),
                                        palette,
                                    ))
                                    .child(detail_row(
                                        tr(settings.language, "Permissions", "Права"),
                                        permissions(file, symlink.is_some()),
                                        palette,
                                    ))
                                    .children(access_rows(file, settings.language).into_iter().map(|(label, value)| {
                                        detail_row(label, value, palette)
                                    })),
                            )
                            .child(
                                v_flex()
                                    .gap_3()
                                    .child(section(tr(settings.language, "METADATA & TAGS", "МЕТАДАННЫЕ И ТЕГИ"), palette))
                                    .child(detail_row(tr(settings.language, "Metadata", "Метаданные"), file.metadata, palette))
                                    .child(detail_row(tr(settings.language, "Tags", "Теги"), tags, palette)),
                            )
                            .child(
                                div()
                                    .p_3()
                                    .rounded_md()
                                    .bg(palette.accent_soft)
                                    .text_xs()
                                    .text_color(palette.muted)
                                    .child(tr(settings.language, "Metadata is read from the local filesystem. Folder size is the recursive total of all files inside, refreshed every minute; symlinks are not followed.", "Метаданные прочитаны из локальной файловой системы. Размер папки — сумма всех вложенных файлов, обновляется раз в минуту; символические ссылки не учитываются.")),
                            ),
                    ),
            )
    }
}

pub fn open_properties(
    entry: &'static FileEntry,
    location: Location,
    ssh_connection: Option<SshConnection>,
    appearance: Entity<AppearanceSettings>,
    cx: &mut App,
) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(460.), px(120.)),
            size: size(px(540.), px(720.)),
        })),
        window_min_size: Some(size(px(480.), px(560.))),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some(format!("{} — Properties", entry.name).into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, move |window, cx| {
        let appearance = appearance.clone();
        let ssh_connection = ssh_connection.clone();
        cx.new(|cx| PropertiesWindow::new(window, cx, entry, location, ssh_connection, appearance))
    })
    .expect("Failed to open Properties window");
}


pub(crate) fn demo_human_size(
    file: &FileEntry,
    settings: crate::settings::HumanSizeSettings,
) -> String {
    demo_size_bytes(file).map_or_else(|| "…".to_string(), |bytes| format_size(bytes, settings))
}

/// Folder sizes arrive asynchronously; `None` means still calculating.
pub(crate) fn demo_size_bytes(file: &FileEntry) -> Option<u64> {
    if file.is_folder() && file.path.is_some() {
        crate::model::folder_size(file)
    } else {
        Some(file.bytes)
    }
}

/// Repaints `cx`'s view whenever a background folder size finishes.
pub(crate) fn watch_folder_sizes<T: 'static>(cx: &mut Context<T>) {
    cx.spawn(async move |this, cx| {
        // Folder sizes and converted previews both finish in the background.
        let generation = || {
            crate::model::folder_sizes_generation()
                + crate::thumbnails::generation()
                + crate::finder_tags::generation()
                + crate::forks::generation()
                + crate::transfer::generation()
                + crate::pdf_pages::generation()
                + crate::media_info::generation()
                + crate::ext_preview::generation()
                + crate::views::quick_look::generation()
        };
        let mut seen = generation();
        loop {
            cx.background_executor()
                .timer(crate::power::interval(
                    std::time::Duration::from_millis(300),
                    std::time::Duration::from_secs(1),
                ))
                .await;
            let current = generation();
            if current != seen {
                seen = current;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
        }
    })
    .detach();
}

fn format_grouped(bytes: u64) -> String {
    let digits = bytes.to_string();
    let mut grouped = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(character);
    }
    grouped
}

fn created(file: &FileEntry) -> String {
    file.path
        .and_then(|path| std::fs::metadata(path).ok()?.created().ok())
        .map(|time| {
            chrono::DateTime::<chrono::Local>::from(time)
                .format("%b %d, %Y, %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "—".to_string())
}

fn symlink_target(file: &FileEntry) -> Option<String> {
    let target = std::fs::read_link(file.path?).ok()?;
    Some(target.display().to_string())
}

fn permissions(file: &FileEntry, symlink: bool) -> String {
    #[cfg(unix)]
    if let Some(metadata) = file.path.and_then(|path| std::fs::metadata(path).ok()) {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = metadata.permissions().mode();
        let kind = if symlink {
            'l'
        } else if metadata.is_dir() {
            'd'
        } else {
            '-'
        };
        let bits: String = (0..9)
            .map(|index| {
                if mode & (0o400 >> index) == 0 {
                    '-'
                } else {
                    ['r', 'w', 'x'][index % 3]
                }
            })
            .collect();
        return format!("{kind}{bits}");
    }
    let _ = symlink;
    if file.is_folder() { "directory" } else { "file" }.to_string()
}

/// Explains the `rwxr-xr-x` string: who the owner and group are, and what
/// the owner, the group's members and everyone else may do.
fn access_rows(file: &FileEntry, language: crate::settings::Language) -> Vec<(&'static str, String)> {
    #[cfg(unix)]
    if let Some(metadata) = file.path.and_then(|path| std::fs::metadata(path).ok()) {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let mode = metadata.permissions().mode();
        let folder = metadata.is_dir();
        let (uid, gid) = (metadata.uid(), metadata.gid());
        let owner = user_name(uid).map_or_else(|| format!("uid {uid}"), |name| format!("{name} (uid {uid})"));
        let group = group_name(gid).map_or_else(|| format!("gid {gid}"), |name| format!("{name} (gid {gid})"));
        return vec![
            (tr(language, "Owner", "Владелец"), format!("{owner} — {}", access_text(mode >> 6, folder, language))),
            (tr(language, "Group", "Группа"), format!("{group} — {}", access_text(mode >> 3, folder, language))),
            (tr(language, "Others", "Остальные"), access_text(mode, folder, language)),
        ];
    }
    let _ = (file, language);
    vec![(tr(language, "Owner", "Владелец"), "—".to_string())]
}

/// One `rwx` triplet (in the low three bits) in words. For folders `x`
/// means entering the folder and `r` listing it.
fn access_text(bits: u32, folder: bool, language: crate::settings::Language) -> String {
    let mut parts = Vec::new();
    if bits & 0o4 != 0 {
        parts.push(if folder { tr(language, "list", "просмотр содержимого") } else { tr(language, "read", "чтение") });
    }
    if bits & 0o2 != 0 {
        parts.push(if folder { tr(language, "add & delete items", "создание и удаление") } else { tr(language, "write", "запись") });
    }
    if bits & 0o1 != 0 {
        parts.push(if folder { tr(language, "open", "вход в папку") } else { tr(language, "run", "запуск") });
    }
    if parts.is_empty() {
        return tr(language, "no access", "нет доступа").to_string();
    }
    parts.join(", ")
}

#[cfg(unix)]
fn user_name(uid: u32) -> Option<String> {
    // SAFETY: getpwuid returns a pointer into static storage or null; the
    // name is copied out immediately on the UI thread.
    unsafe {
        let entry = libc::getpwuid(uid);
        (!entry.is_null()).then(|| std::ffi::CStr::from_ptr((*entry).pw_name).to_string_lossy().into_owned())
    }
}

#[cfg(unix)]
fn group_name(gid: u32) -> Option<String> {
    // SAFETY: as in `user_name`.
    unsafe {
        let entry = libc::getgrgid(gid);
        (!entry.is_null()).then(|| std::ffi::CStr::from_ptr((*entry).gr_name).to_string_lossy().into_owned())
    }
}

fn section(title: &'static str, palette: Palette) -> AnyElement {
    div()
        .text_size(px(10.))
        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
        .text_color(palette.faint)
        .child(title)
        .into_any_element()
}

fn detail_row(label: &'static str, value: impl Into<SharedString>, palette: Palette) -> AnyElement {
    h_flex()
        .items_start()
        .gap_4()
        .text_xs()
        .child(
            div()
                .w(px(104.))
                .flex_shrink_0()
                .text_color(palette.muted)
                .child(label),
        )
        .child(div().flex_1().text_color(palette.text).child(value.into()))
        .into_any_element()
}

#[cfg(test)]
mod access_tests {
    use super::*;
    use crate::settings::Language;

    #[test]
    fn permission_triplets_are_explained() {
        assert_eq!(access_text(0o5, false, Language::English), "read, run");
        assert_eq!(access_text(0o7, true, Language::English), "list, add & delete items, open");
        assert_eq!(access_text(0o0, false, Language::English), "no access");
        assert_eq!(access_text(0o6, false, Language::Russian), "чтение, запись");
    }

    #[test]
    #[cfg(unix)]
    fn owner_and_group_resolve_to_names() {
        assert_eq!(user_name(0).as_deref(), Some("root"));
        assert!(group_name(0).is_some());
    }
}
