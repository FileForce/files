//! The footer: the event console (like Postman's) and the notifications
//! bell. Both read `files_sdk::log`, which Files and every extension write.

use files_sdk::log::{self, Event, Kind, Level};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::app::FileExplorer;
use crate::palette::Palette;

/// Rows rendered in the console (the journal keeps more).
const SHOWN: usize = 400;
/// How long a toast stays, and how many stack.
const TOAST_MS: u64 = 6000;
const TOASTS: usize = 3;
/// Below this much free space a notification is posted (once).
const LOW_DISK: u64 = 5 * 1024 * 1024 * 1024;

fn kind_color(kind: Kind, palette: Palette) -> Hsla {
    match kind {
        Kind::Process => palette.accent,
        Kind::Fs => palette.folder,
        Kind::Network => palette.media,
        Kind::Notice => palette.muted,
        Kind::Notification => palette.archive,
        Kind::Error => palette.video,
        Kind::Info => palette.faint,
        Kind::Setting => palette.accent,
    }
}

fn level_icon(level: Level, palette: Palette) -> Icon {
    match level {
        Level::Info => Icon::new(IconName::Info).text_color(palette.accent),
        Level::Success => Icon::new(IconName::CircleCheck).text_color(palette.code),
        Level::Warning => Icon::new(IconName::TriangleAlert).text_color(palette.archive),
        Level::Error => Icon::new(IconName::CircleX).text_color(palette.video),
    }
}

/// A notification's buttons: open their link (release notes…), or run an
/// app command for `files:` links. The first one is the primary.
fn action_buttons(prefix: &'static str, notification: &log::Notification, explorer: &gpui_kit::WeakEntity<FileExplorer>) -> gpui_kit::Div {
    let id = notification.id;
    h_flex().gap_1().flex_wrap().children(notification.actions.iter().enumerate().map(|(index, action)| {
        let url = action.url.clone();
        let button = Button::new((prefix, (id as usize) * 8 + index)).xsmall().label(action.label.clone());
        let button = if index == 0 { button.primary() } else { button.ghost() };
        let explorer = explorer.clone();
        button.on_click(move |_, _, cx| {
            cx.stop_propagation();
            if let Some(command) = url.strip_prefix("files:") {
                let _ = explorer.update(cx, |this, cx| this.run_notification_command(command, id, cx));
            } else {
                cx.open_url(&url);
            }
        })
    }))
}

fn maintenance_due(last: std::time::Instant, now: std::time::Instant) -> bool {
    now.duration_since(last) >= std::time::Duration::from_secs(60)
}

/// The newest notification already chimed for, shared by every window.
static CHIMED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// One chime for what arrived since the last one (the most serious kind).
fn chime_for_new_notifications(settings: &crate::settings::AppSettings, in_front: bool) {
    use std::sync::atomic::Ordering;
    let seen = CHIMED.load(Ordering::Relaxed);
    let fresh: Vec<log::Notification> = log::notifications().into_iter().filter(|notification| notification.id > seen).collect();
    let Some(newest) = fresh.iter().map(|notification| notification.id).max() else { return };
    if CHIMED.fetch_max(newest, Ordering::Relaxed) >= newest {
        return;
    }
    // Also in the system's notification center, when Settings allow it.
    crate::system_notify::send(&settings.system_notifications, &fresh, in_front);
    if !settings.sound.notifications {
        return;
    }
    let level = fresh
        .iter()
        .map(|notification| notification.level)
        .max_by_key(|level| match level {
            Level::Error => 3,
            Level::Warning => 2,
            Level::Success => 1,
            Level::Info => 0,
        })
        .unwrap_or(Level::Info);
    crate::sound::notify(settings.sound.chime, level, settings.sound.volume);
}

/// At launch: what is already in the journal never chimes.
fn prime_chimes() {
    let newest = log::notifications().iter().map(|notification| notification.id).max().unwrap_or(0);
    CHIMED.fetch_max(newest, std::sync::atomic::Ordering::Relaxed);
}

impl FileExplorer {
    /// Redraws when the journal changes, and checks the free disk space.
    pub(crate) fn watch_journal(cx: &mut Context<Self>) {
        prime_chimes();
        cx.spawn(async move |this, cx| {
            let started = std::time::Instant::now();
            let mut last_disk = started - std::time::Duration::from_secs(60);
            let mut last_mail = started;
            let mut last_sample = started;
            let mut update_checked = false;
            loop {
                cx.background_executor()
                    .timer(crate::power::interval(
                        std::time::Duration::from_millis(400),
                        std::time::Duration::from_secs(2),
                    ))
                    .await;
                let now = std::time::Instant::now();
                let check_disk = maintenance_due(last_disk, now);
                let check_mail = maintenance_due(last_mail, now);
                let check_update = !update_checked && now.duration_since(started) >= std::time::Duration::from_secs(2);
                let sample_due = now.duration_since(last_sample) >= crate::resources::EVERY;
                let alive = this.update(cx, |this, cx| {
                    let generation = log::generation();
                    // Toasts on screen fade out on time.
                    if generation != this.journal_generation || !this.toasts().is_empty() {
                        if generation != this.journal_generation {
                            let in_front = cx.active_window().is_some();
                            chime_for_new_notifications(this.appearance.read(cx), in_front);
                        }
                        this.journal_generation = generation;
                        cx.notify();
                    }
                    // Soon after start: the daily update check (at most once a day).
                    if check_update {
                        update_checked = true;
                        if !crate::power::power_saving() {
                            this.auto_check_updates(cx);
                        }
                    }
                    if check_disk {
                        last_disk = now;
                        this.check_disk_space();
                    }
                    // Memory and CPU for Console → Resources (developer mode only).
                    if sample_due && this.appearance.read(cx).developer.enabled {
                        last_sample = now;
                        crate::resources::sample();
                        if this.console_open && this.console_tab != ConsoleTab::Events {
                            cx.notify();
                        }
                    }
                    // Extensions asked the music player for something.
                    let commands = crate::sound::take_pending();
                    if !commands.is_empty() {
                        crate::views::music::run_extension_commands(commands, cx);
                    }
                    // The slower UI tick must not slow scheduled mail.
                    if check_mail {
                        last_mail = now;
                        if this.appearance.read(cx).extensions.mail {
                            crate::views::mail::send_due_mail(cx);
                        }
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// "Low disk space" once when free space drops below 5 GB, again only
    /// after it went back above.
    fn check_disk_space(&mut self) {
        let Some(home) = crate::model::Location::Home.path() else { return };
        let Some(disk) = crate::model::disk_usage(&home) else { return };
        let low = disk.available < LOW_DISK;
        if low && !self.low_disk_warned {
            let free = crate::settings::format_size(disk.available, Default::default());
            log::notify(
                "files",
                Level::Warning,
                format!("Low disk space: {free} free on {}", disk.name),
                Some("Empty the Trash or remove large files; below a few GB, apps (and Spotlight) start to fail.".to_string()),
            );
        }
        self.low_disk_warned = low;
    }

    /// Every status-bar message also lands in the journal (once per change).
    pub(crate) fn journal_notice(&mut self) {
        if self.notice == self.journaled_notice {
            return;
        }
        if let Some(notice) = &self.notice {
            let lower = notice.to_lowercase();
            let kind = if lower.contains("failed") || lower.contains("cannot") || notice.starts_with('⚠') { Kind::Error } else { Kind::Notice };
            log::record("files", kind, notice.clone(), None);
        }
        self.journaled_notice = self.notice.clone();
    }

    /// The console's rows: everything in Developer mode, otherwise what a
    /// user reads (messages, notifications, errors).
    pub(crate) fn console_events(&self, developer: bool, query: &str) -> Vec<Event> {
        let query = query.to_lowercase();
        log::events()
            .into_iter()
            .filter(|event| developer || matches!(event.kind, Kind::Notice | Kind::Notification | Kind::Error))
            .filter(|event| self.console_kinds.is_empty() || self.console_kinds.contains(&event.kind))
            .filter(|event| {
                query.is_empty()
                    || event.message.to_lowercase().contains(&query)
                    || event.source.to_lowercase().contains(&query)
                    || event.detail.as_ref().is_some_and(|detail| detail.to_lowercase().contains(&query))
            })
            .collect()
    }

    /// The footer's left and right buttons: Console and the bell.
    pub(crate) fn footer_buttons(&self, palette: Palette, cx: &mut Context<Self>) -> (AnyElement, AnyElement) {
        let errors = log::events().iter().rev().take(200).filter(|event| !event.ok).count();
        let console = Button::new("footer-console")
            .debug_selector(|| "footer-console".into())
            .ghost()
            .xsmall()
            .icon(IconName::SquareTerminal)
            .label(if errors > 0 { format!("Console · {errors}") } else { "Console".to_string() })
            .when(self.console_open, |button| button.primary())
            .tooltip("Event journal: what Files and extensions did")
            .on_click(cx.listener(|this, _, _, cx| {
                this.console_open = !this.console_open;
                cx.notify();
            }));
        let unread = log::unread();
        let bell = h_flex()
            .id("footer-bell")
            .debug_selector(|| "footer-bell".into())
            .gap_1()
            .items_center()
            .px_1()
            .rounded_sm()
            .cursor_pointer()
            .hover(|style| style.bg(palette.hover))
            .aria_label(format!("Notifications, {unread} unread"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.notifications_open = !this.notifications_open;
                if this.notifications_open {
                    log::mark_all_read();
                }
                cx.notify();
            }))
            .child(Icon::new(IconName::Bell).xsmall().text_color(if unread > 0 { palette.accent } else { palette.muted }))
            .when(unread > 0, |bell| {
                bell.child(
                    div()
                        .px_1()
                        .rounded_full()
                        .bg(palette.accent)
                        .text_color(palette.accent_contrast)
                        .text_size(px(9.))
                        .child(unread.to_string()),
                )
            });
        (console.into_any_element(), bell.into_any_element())
    }

    pub(crate) fn render_console(&mut self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.console_open || !self.appearance.read(cx).developer.show_console {
            return None;
        }
        let developer = self.appearance.read(cx).developer.enabled;
        let query = self.console_query.read(cx).value().to_string();
        let events = self.console_events(developer, &query);
        let total = events.len();
        let kinds: &[Kind] = if developer { &Kind::ALL } else { &[Kind::Notice, Kind::Notification, Kind::Error] };
        let chips = kinds.iter().map(|kind| {
            let kind = *kind;
            let on = self.console_kinds.contains(&kind);
            Button::new(gpui_kit::ElementId::Name(format!("console-kind-{}", kind.label()).into()))
                .xsmall()
                .label(kind.label())
                .when(on, |button| button.primary())
                .when(!on, |button| button.ghost())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.console_kinds.remove(&kind) {
                        this.console_kinds.insert(kind);
                    }
                    cx.notify();
                }))
        }).collect::<Vec<_>>();
        let rows = events.into_iter().rev().take(SHOWN).map(|event| {
            let id = event.id;
            let expanded = self.console_expanded == Some(id);
            let row = v_flex()
                .id(("console-event", id as usize))
                .px_2()
                .py_0p5()
                .border_b_1()
                .border_color(palette.border)
                .cursor_pointer()
                .hover(|style| style.bg(palette.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.console_expanded = if this.console_expanded == Some(id) { None } else { Some(id) };
                    cx.notify();
                }))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_xs()
                        .child(div().w(px(84.)).flex_shrink_0().text_color(palette.faint).font_family("Menlo").child(crate::dev_shell::clock(event.time)))
                        .child(div().w(px(92.)).flex_shrink_0().truncate().text_color(palette.muted).child(event.source.clone()))
                        .child(
                            div()
                                .w(px(78.))
                                .flex_shrink_0()
                                .text_color(kind_color(event.kind, palette))
                                .child(event.kind.label()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(if event.ok { palette.text } else { palette.video })
                                .when(event.kind == Kind::Process, |message| message.font_family("Menlo"))
                                .child(event.message.clone()),
                        )
                        .children(event.duration_ms.map(|ms| div().flex_shrink_0().text_color(palette.faint).child(format!("{ms} ms"))))
                        .child({
                            // A program copies as the exact command to paste in a terminal.
                            let text = event.command.clone().unwrap_or_else(|| match &event.detail {
                                Some(detail) => format!("{}\n{detail}", event.message),
                                None => event.message.clone(),
                            });
                            let copy = Button::new(("console-copy", id as usize))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Copy)
                                .tooltip(if event.command.is_some() { "Copy command" } else { "Copy" })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text.clone()));
                                    this.notice = Some("Copied to the clipboard".to_string());
                                    cx.notify();
                                }));
                            #[cfg(test)]
                            let copy = div().debug_selector(move || format!("console-copy-{id}")).child(copy);
                            copy
                        })
                        .when(event.detail.is_some(), |row| {
                            row.child(Icon::new(if expanded { IconName::ChevronDown } else { IconName::ChevronRight }).xsmall().text_color(palette.faint))
                        }),
                )
                .when(expanded, |row| {
                    row.children(event.detail.clone().map(|detail| {
                        div()
                            .mt_1()
                            .mb_1()
                            .ml(px(84.))
                            .p_2()
                            .rounded_sm()
                            .bg(palette.surface)
                            .text_xs()
                            .font_family("Menlo")
                            .text_color(palette.text)
                            .whitespace_normal()
                            .child(crate::views::text_fx::selectable(("console-detail", id as usize), detail))
                    }))
                });
            #[cfg(test)]
            let row = row.debug_selector(move || format!("console-event-{id}"));
            row
        }).collect::<Vec<_>>();
        Some(
            v_flex()
                .h(px(240.))
                .flex_shrink_0()
                .border_t_1()
                .border_color(palette.border)
                .bg(palette.window)
                .child(
                    h_flex()
                        .px_2()
                        .py_1()
                        .gap_2()
                        .items_center()
                        .border_b_1()
                        .border_color(palette.border)
                        .child(div().text_xs().font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Console"))
                        .when(developer, |this| {
                            this.children(ConsoleTab::ALL.map(|tab| {
                                let button = Button::new(gpui_kit::ElementId::Name(format!("console-tab-{}", tab.label().to_lowercase()).into())).xsmall().label(tab.label());
                                let button = if self.console_tab == tab { button.primary() } else { button.ghost() };
                                button.on_click(cx.listener(move |this, _, _, cx| {
                                    this.console_tab = tab;
                                    cx.notify();
                                }))
                            }))
                        })
                        .child(div().text_xs().text_color(palette.faint).child(if developer {
                            format!("{total} events · developer mode")
                        } else {
                            format!("{total} events · turn on Developer mode in Settings to see every call")
                        }))
                        .child(div().flex_1())
                        .children(chips)
                        .child(div().w(px(200.)).child(Input::new(&self.console_query).xsmall()))
                        .child(Button::new("console-clear").ghost().xsmall().icon(IconName::Trash).tooltip("Clear").on_click(cx.listener(|_, _, _, cx| {
                            log::clear();
                            cx.notify();
                        })))
                        .child(Button::new("console-close").ghost().xsmall().icon(IconName::X).on_click(cx.listener(|this, _, _, cx| {
                            this.console_open = false;
                            cx.notify();
                        }))),
                )
                .child(if developer && self.console_tab == ConsoleTab::Resources {
                    render_resources(palette)
                } else if developer && self.console_tab == ConsoleTab::Tasks {
                    render_tasks(palette, cx)
                } else if developer && self.console_tab == ConsoleTab::Indexes {
                    render_indexes(palette, cx)
                } else if developer && self.console_tab == ConsoleTab::Search {
                    render_search(palette)
                } else {
                    div()
                        .id("console-rows")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .child(v_flex().children(rows).when(total == 0, |list| {
                            list.child(div().p_3().text_xs().text_color(palette.faint).child("Nothing yet"))
                        }))
                        .into_any_element()
                })
                .into_any_element(),
        )
    }

    /// Notifications young enough to show as toasts, newest first.
    pub(crate) fn toasts(&self) -> Vec<log::Notification> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_millis() as u64);
        log::notifications()
            .into_iter()
            .filter(|notification| now.saturating_sub(notification.time) < TOAST_MS && !self.hidden_toasts.contains(&notification.id))
            .take(TOASTS)
            .collect()
    }

    /// macOS-style banners in the top-right corner; they fade on their own.
    /// A click opens the notifications list, × closes just the toast.
    pub(crate) fn render_toasts(&mut self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let explorer = cx.entity().downgrade();
        let toasts = self.toasts();
        if toasts.is_empty() || self.notifications_open {
            return None;
        }
        let cards = toasts.into_iter().map(|notification| {
            let id = notification.id;
            let card = h_flex()
                .id(("toast", id as usize))
                .occlude()
                .w(px(340.))
                .px_3()
                .py_2p5()
                .gap_3()
                .items_start()
                .rounded(px(14.))
                .border_1()
                .border_color(palette.border)
                .bg(palette.elevated)
                .shadow_lg()
                .cursor_pointer()
                .aria_label(format!("Notification: {}", notification.title))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.notifications_open = true;
                    log::mark_all_read();
                    cx.notify();
                }))
                .child(
                    div()
                        .size(px(30.))
                        .flex_shrink_0()
                        .rounded_lg()
                        .bg(palette.surface)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(level_icon(notification.level, palette).small()),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().flex_1().min_w_0().truncate().text_xs().text_color(palette.muted).child(notification.source.to_uppercase()))
                                .child(div().text_xs().text_color(palette.faint).child("now")),
                        )
                        .child(div().text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(palette.text).whitespace_normal().child(notification.title.clone()))
                        .children(notification.body.clone().map(|body| {
                            div().text_xs().text_color(palette.muted).whitespace_normal().line_clamp(2).child(body)
                        }))
                        .when(!notification.actions.is_empty(), |this| this.child(action_buttons("toast-action", &notification, &explorer))),
                )
                .child(Button::new(("toast-close", id as usize)).ghost().xsmall().icon(IconName::X).on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.hidden_toasts.insert(id);
                    cx.notify();
                })));
            #[cfg(test)]
            let card = card.debug_selector(move || format!("toast-{id}"));
            // Slides in from the right, like macOS banners.
            gpui_kit::AnimationExt::with_animation(
                card,
                ("toast-in", id as usize),
                gpui_kit::Animation::new(std::time::Duration::from_millis(220)).with_easing(gpui_kit::ease_out_quint()),
                |card, delta| card.ml(px((1. - delta) * 40.)).opacity(delta),
            )
        });
        // Bottom right, above the status bar, newest at the bottom.
        Some(
            gpui_kit::deferred(v_flex().absolute().bottom(px(40.)).right(px(12.)).gap_2().children(cards.into_iter().rev()))
                .with_priority(1)
                .into_any_element(),
        )
    }

    /// The notifications card above the bell.
    pub(crate) fn render_notifications(&mut self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let explorer = cx.entity().downgrade();
        if !self.notifications_open {
            return None;
        }
        let notifications = log::notifications();
        let items = notifications.iter().map(|notification| {
            let id = notification.id;
            let item = h_flex()
                .id(("notification", id as usize))
                .px_3()
                .py_2()
                .gap_2()
                .items_start()
                .border_b_1()
                .border_color(palette.border)
                .child(level_icon(notification.level, palette).small())
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(div().text_sm().text_color(palette.text).whitespace_normal().child(notification.title.clone()))
                        .children(notification.body.clone().map(|body| div().text_xs().text_color(palette.muted).whitespace_normal().child(body)))
                        .when(!notification.actions.is_empty(), |this| this.child(action_buttons("notification-action", &notification, &explorer)))
                        .child(div().text_size(px(9.)).text_color(palette.faint).child(format!(
                            "{} · {}",
                            notification.source,
                            crate::views::agents::ago((notification.time / 1000) as i64)
                        ))),
                )
                .child(Button::new(("notification-dismiss", id as usize)).ghost().xsmall().icon(IconName::X).on_click(cx.listener(move |_, _, _, cx| {
                    log::dismiss(id);
                    cx.notify();
                })));
            #[cfg(test)]
            let item = item.debug_selector(move || format!("notification-{id}"));
            item
        });
        Some(
            div().relative().h_0().child(
                v_flex()
                    .id("notifications-card")
                    // Clicks, drags and text selection stop here instead of
                    // reaching the file list painted underneath.
                    .occlude()
                    .absolute()
                    .bottom(px(4.))
                    .right(px(8.))
                    .w(px(360.))
                    .max_h(px(420.))
                    .rounded_lg()
                    .border_1()
                    .border_color(palette.border)
                    .bg(palette.elevated)
                    .shadow_lg()
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .px_3()
                            .py_2()
                            .items_center()
                            .border_b_1()
                            .border_color(palette.border)
                            .child(div().flex_1().text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Notifications"))
                            .child(Button::new("notifications-clear").ghost().xsmall().label("Clear all").disabled(notifications.is_empty()).on_click(cx.listener(|_, _, _, cx| {
                                log::clear_notifications();
                                cx.notify();
                            })))
                            .child(Button::new("notifications-close").ghost().xsmall().icon(IconName::X).on_click(cx.listener(|this, _, _, cx| {
                                this.notifications_open = false;
                                cx.notify();
                            }))),
                    )
                    .child(
                        div()
                            .id("notifications-list")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scrollbar()
                            .child(v_flex().children(items).when(notifications.is_empty(), |list| {
                                list.child(div().p_4().text_xs().text_color(palette.faint).child("No notifications"))
                            })),
                    ),
            )
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn maintenance_schedule_does_not_depend_on_ui_tick_count() {
        let start = std::time::Instant::now();
        assert!(!super::maintenance_due(start, start + std::time::Duration::from_secs(58)));
        assert!(super::maintenance_due(start, start + std::time::Duration::from_secs(60)));
    }
}

/// A bar per sample, scaled to the highest one; the last bar is now.
fn sparkline(values: &[f32], color: gpui_kit::Hsla, palette: Palette) -> gpui_kit::Div {
    let peak = values.iter().copied().fold(0f32, f32::max).max(f32::EPSILON);
    h_flex()
        .h(px(56.))
        .w_full()
        .items_end()
        .gap(px(1.))
        .p_1()
        .rounded_sm()
        .bg(palette.surface)
        .children(values.iter().map(|value| div().flex_1().min_w(px(1.)).h(gpui_kit::relative((value / peak).clamp(0.02, 1.))).bg(color).rounded_t_sm()))
}

/// Console → Resources: memory and CPU over two minutes, and what the app keeps.
/// DevTools: the Console's tabs (all but Events need Developer mode).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ConsoleTab {
    #[default]
    Events,
    Tasks,
    Indexes,
    Search,
    Resources,
}

impl ConsoleTab {
    pub const ALL: [ConsoleTab; 5] = [ConsoleTab::Events, ConsoleTab::Tasks, ConsoleTab::Indexes, ConsoleTab::Search, ConsoleTab::Resources];

    pub fn label(self) -> &'static str {
        match self {
            ConsoleTab::Events => "Events",
            ConsoleTab::Tasks => "Tasks",
            ConsoleTab::Indexes => "Indexes",
            ConsoleTab::Search => "Search",
            ConsoleTab::Resources => "Resources",
        }
    }
}

fn millis(duration: std::time::Duration) -> String {
    let ms = duration.as_secs_f64() * 1000.;
    if ms >= 1000. { format!("{:.1} s", ms / 1000.) } else if ms >= 10. { format!("{ms:.0} ms") } else { format!("{ms:.1} ms") }
}

/// DevTools → Tasks: background jobs, newest first, with Retry and Cancel.
fn render_tasks(palette: Palette, cx: &mut Context<FileExplorer>) -> AnyElement {
    use crate::tasks::State;
    let jobs = crate::tasks::jobs();
    let count = |wanted: fn(&State) -> bool| jobs.iter().filter(|job| wanted(&job.state)).count();
    let (queued, running, failed) = (count(|state| *state == State::Queued), count(|state| *state == State::Running), count(|state| matches!(state, State::Failed(_))));
    let rows = jobs.iter().rev().take(200).map(|job| {
        let id = job.id;
        let color = match job.state {
            State::Queued => palette.muted,
            State::Running => palette.accent,
            State::Done => palette.code,
            State::Failed(_) => palette.video,
            State::Cancelled => palette.faint,
        };
        let error = if let State::Failed(error) = &job.state { Some(error.clone()) } else { None };
        h_flex()
            .gap_3()
            .px_2()
            .py_0p5()
            .text_xs()
            .border_b_1()
            .border_color(palette.border)
            .child(div().w(px(72.)).flex_shrink_0().text_color(color).child(job.state.label()))
            .child(div().w(px(110.)).flex_shrink_0().text_color(palette.muted).child(job.kind))
            .child(div().flex_1().min_w_0().truncate().text_color(palette.text).child(job.label.clone()))
            .children(error.map(|error| div().max_w(px(260.)).truncate().text_color(palette.video).child(error)))
            .child(div().w(px(70.)).flex_shrink_0().text_right().font_family("Menlo").text_color(palette.faint).child(format!("wait {}", millis(job.wait()))))
            .child(div().w(px(64.)).flex_shrink_0().text_right().font_family("Menlo").text_color(palette.text).child(job.duration().map(millis).unwrap_or_else(|| "—".into())))
            .child(
                h_flex()
                    .w(px(56.))
                    .justify_end()
                    .when(job.state == State::Queued, |this| {
                        this.child(Button::new(("task-cancel", id as usize)).ghost().xsmall().label("Cancel").on_click(cx.listener(move |_, _, _, cx| {
                            crate::tasks::cancel(id);
                            cx.notify();
                        })))
                    })
                    .when(matches!(job.state, State::Failed(_) | State::Cancelled) && crate::tasks::can_retry(job.kind), |this| {
                        this.child(Button::new(("task-retry", id as usize)).ghost().xsmall().label("Retry").on_click(cx.listener(move |_, _, _, cx| {
                            crate::tasks::retry(id);
                            cx.notify();
                        })))
                    }),
            )
    });
    v_flex()
        .id("console-tasks")
        .debug_selector(|| "console-tasks".into())
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .child(
            h_flex()
                .gap_3()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(palette.muted)
                .child(format!("{queued} queued · {running} running · {failed} failed · {} kept", jobs.len()))
                .child(div().flex_1())
                .child(Button::new("tasks-clear").ghost().xsmall().icon(IconName::Trash).label("Clear finished").on_click(cx.listener(|_, _, _, cx| {
                    crate::tasks::clear_finished();
                    cx.notify();
                }))),
        )
        .children(rows)
        .when(jobs.is_empty(), |this| this.child(div().p_3().text_xs().text_color(palette.faint).child("No background work yet")))
        .into_any_element()
}

/// DevTools → Indexes: what Files keeps to be fast, and how to rebuild it.
fn render_indexes(palette: Palette, cx: &mut Context<FileExplorer>) -> AnyElement {
    let (known, made, listings, search) = crate::model::index_stats();
    let (waiting, failed) = crate::thumbnails::stats();
    let rows: Vec<(&'static str, String, &'static str, Option<(&'static str, fn())>)> = vec![
        ("File entries", format!("{known} known · {made} made since launch"), "Every file and folder seen; kept until quit", None),
        ("Folder listings", format!("{listings} folders"), "Read again after a short while, or now", Some(("Read again", crate::model::invalidate_listings as fn()))),
        ("Global search", format!("{search} items"), "The ⌘K index: a few levels of the search folder", Some(("Rebuild", crate::model::clear_search_index as fn()))),
        ("File details", format!("{} files", crate::media_info::cached()), "EXIF, streams, archive and program details", Some(("Read again", crate::media_info::clear as fn()))),
        ("Folder sizes", format!("{} folders", crate::model::folder_sizes_count()), "Measured in the background as folders show", Some(("Measure again", crate::model::clear_folder_sizes as fn()))),
        ("Thumbnails", format!("{waiting} waiting · {failed} failed"), "Previews, Quick Look renders, program icons", Some(("Retry failed", crate::thumbnails::forget_failures as fn()))),
        ("Music library", format!("{} tracks", files_ext_music::local::library_len()), "Read when the Music tab opens", None),
    ];
    let rows = rows.into_iter().enumerate().map(|(index, (name, count, note, action))| {
        h_flex()
            .gap_3()
            .px_2()
            .py_1()
            .text_xs()
            .border_b_1()
            .border_color(palette.border)
            .child(div().w(px(130.)).flex_shrink_0().text_color(palette.text).child(name))
            .child(div().w(px(230.)).flex_shrink_0().text_color(palette.muted).child(count))
            .child(div().flex_1().min_w_0().truncate().text_color(palette.faint).child(note))
            .children(action.map(|(label, run)| {
                Button::new(("index-action", index)).ghost().xsmall().icon(IconName::RefreshCw).label(label).on_click(cx.listener(move |_, _, _, cx| {
                    run();
                    files_sdk::log::record("devtools", Kind::Setting, format!("{name}: {label}"), None);
                    cx.notify();
                }))
            }))
    });
    v_flex()
        .id("console-indexes")
        .debug_selector(|| "console-indexes".into())
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .children(rows)
        .child(div().p_2().text_xs().text_color(palette.faint).child(
            "Vector and graph indexes (semantic image, speech and entity search) come with issues #1–#4; their health and rebuild controls will appear here.",
        ))
        .into_any_element()
}

/// DevTools → Search: the last searches, split into where the time went.
fn render_search(palette: Palette) -> AnyElement {
    let searches = crate::tasks::searches();
    let widest = searches.iter().map(|search| search.total().as_secs_f32()).fold(0.0001f32, f32::max);
    let rows = searches.iter().rev().map(|search| {
        let bar = |duration: std::time::Duration, color| div().h(px(8.)).w(px((duration.as_secs_f32() / widest * 220.).max(1.))).bg(color);
        h_flex()
            .gap_3()
            .px_2()
            .py_1()
            .text_xs()
            .border_b_1()
            .border_color(palette.border)
            .child(div().w(px(52.)).flex_shrink_0().text_color(palette.muted).child(search.scope))
            .child(div().w(px(160.)).flex_shrink_0().truncate().text_color(palette.text).child(search.query.clone()))
            .child(div().w(px(120.)).flex_shrink_0().text_color(palette.faint).child(format!("{} of {}", search.results, search.candidates)))
            .child(h_flex().w(px(230.)).flex_shrink_0().child(bar(search.list, palette.folder)).child(bar(search.matching, palette.accent)).child(bar(search.sort, palette.code)))
            .child(div().flex_1().min_w_0().font_family("Menlo").text_color(palette.muted).child(format!(
                "list {} · match {} · sort {} = {}",
                millis(search.list),
                millis(search.matching),
                millis(search.sort),
                millis(search.total())
            )))
    });
    v_flex()
        .id("console-search")
        .debug_selector(|| "console-search".into())
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .child(
            h_flex().gap_3().px_2().py_1().text_xs().text_color(palette.faint)
                .child(h_flex().gap_1().items_center().child(div().size(px(8.)).bg(palette.folder)).child("listing / index"))
                .child(h_flex().gap_1().items_center().child(div().size(px(8.)).bg(palette.accent)).child("filters and fuzzy matching"))
                .child(h_flex().gap_1().items_center().child(div().size(px(8.)).bg(palette.code)).child("sorting")),
        )
        .children(rows)
        .when(searches.is_empty(), |this| this.child(div().p_3().text_xs().text_color(palette.faint).child("Type in a search box: each search shows up here")))
        .into_any_element()
}

fn render_resources(palette: Palette) -> AnyElement {
    let history = crate::resources::history();
    let now = history.last().copied().unwrap_or_default();
    let peak_memory = history.iter().map(|sample| sample.memory).max().unwrap_or(0);
    let peak_cpu = history.iter().map(|sample| sample.cpu).fold(0f32, f32::max);
    let memory: Vec<f32> = history.iter().map(|sample| sample.memory as f32).collect();
    let cpu: Vec<f32> = history.iter().map(|sample| sample.cpu).collect();
    let chart = |title: &'static str, value: String, detail: String, values: &[f32], color: gpui_kit::Hsla| {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_baseline()
                    .child(div().text_xs().text_color(palette.muted).child(title))
                    .child(div().text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(value))
                    .child(div().text_xs().text_color(palette.faint).child(detail)),
            )
            .child(sparkline(values, color, palette))
    };
    let memory_label = if cfg!(target_os = "macos") { "Memory (as Activity Monitor)" } else { "Memory (resident)" };
    let parts = crate::resources::parts().into_iter().map(|part| {
        h_flex()
            .gap_3()
            .px_2()
            .py_0p5()
            .text_xs()
            .border_b_1()
            .border_color(palette.border)
            .child(div().w(px(150.)).flex_shrink_0().text_color(palette.text).child(part.name))
            .child(div().w(px(260.)).flex_shrink_0().text_color(palette.muted).child(part.count))
            .child(div().w(px(70.)).flex_shrink_0().text_right().font_family("Menlo").text_color(palette.text).child(part.bytes.map(crate::resources::size).unwrap_or_else(|| "—".into())))
            .child(div().flex_1().min_w_0().truncate().text_color(palette.faint).child(part.note))
    });
    v_flex()
        .id("console-resources")
        .debug_selector(|| "console-resources".into())
        .flex_1()
        .min_h_0()
        .p_2()
        .gap_2()
        .overflow_y_scrollbar()
        .child(
            h_flex()
                .gap_3()
                .child(chart(memory_label, crate::resources::size(now.memory), format!("peak {}", crate::resources::size(peak_memory)), &memory, palette.accent))
                .child(chart("CPU", format!("{:.0}%", now.cpu), format!("peak {:.0}% · 100% = one core", peak_cpu), &cpu, palette.code)),
        )
        .child(v_flex().children(parts))
        .when(history.is_empty(), |this| this.child(div().text_xs().text_color(palette.faint).child("Measuring…")))
        .into_any_element()
}
