//! Notifications in the system's own center (macOS Notification Center, the
//! Windows action center, the Linux desktop's notifications), beside the
//! toast, when Settings → Sound allows it.

use files_sdk::log::{Level, Notification};

use crate::settings::{SystemNotificationSettings, SystemNotify};

/// Whether `notification` goes to the system now; `in_front`: a Files window
/// is active (its toast is seen then).
pub fn wanted(settings: &SystemNotificationSettings, notification: &Notification, in_front: bool) -> bool {
    let when = match settings.mode {
        SystemNotify::Off => false,
        SystemNotify::Background => !in_front,
        SystemNotify::Always => true,
    };
    when && settings.min_level.allows(notification.level)
}

/// Sends the notifications the settings allow, off the UI thread. Files'
/// own chime plays already, so the system one stays silent.
pub fn send(settings: &SystemNotificationSettings, fresh: &[Notification], in_front: bool) {
    let chosen: Vec<Notification> = fresh.iter().filter(|notification| wanted(settings, notification, in_front)).cloned().collect();
    if chosen.is_empty() || cfg!(test) {
        return;
    }
    std::thread::spawn(move || {
        for notification in chosen {
            show(&notification);
        }
    });
}

fn summary(notification: &Notification) -> String {
    let mark = match notification.level {
        Level::Error => "⛔ ",
        Level::Warning => "⚠️ ",
        _ => "",
    };
    format!("{mark}{}", notification.title)
}

fn show(notification: &Notification) {
    // macOS shows the app's name and icon for its bundle id.
    #[cfg(target_os = "macos")]
    {
        static APP: std::sync::Once = std::sync::Once::new();
        APP.call_once(|| {
            let _ = notify_rust::set_application(crate::desktop::APP_ID);
        });
    }
    let mut toast = notify_rust::Notification::new();
    toast.appname("file FORCE").summary(&summary(notification)).body(notification.body.as_deref().unwrap_or(&notification.source));
    // Windows: toasts under Files' AppUserModelID (the MSI's shortcut has it),
    // so they carry its name and icon instead of PowerShell's.
    #[cfg(windows)]
    toast.app_id(crate::desktop::APP_ID);
    // Linux: the desktop entry gives the name and icon; errors stay on screen.
    #[cfg(all(unix, not(target_os = "macos")))]
    toast.icon(crate::desktop::APP_ID).hint(notify_rust::Hint::DesktopEntry(crate::desktop::APP_ID.to_string())).urgency(match notification.level {
        Level::Error => notify_rust::Urgency::Critical,
        _ => notify_rust::Urgency::Normal,
    });
    if let Err(error) = toast.show() {
        files_sdk::log::record("notifications", files_sdk::log::Kind::Error, format!("System notification not shown: {error}"), None);
    }
}

/// Settings → Sound → "Send a test": one right away, whatever the mode.
pub fn test(language: crate::settings::Language) {
    let title = crate::settings::localized(language, "Notifications work", "Уведомления работают");
    let body = crate::settings::localized(language, "This is how Files tells you about finished work.", "Так Files сообщит о завершённой работе.");
    let notification = Notification {
        id: 0,
        time: 0,
        source: "Files".into(),
        level: Level::Success,
        title: title.into(),
        body: Some(body.into()),
        read: false,
        actions: Vec::new(),
    };
    if !cfg!(test) {
        std::thread::spawn(move || show(&notification));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::NotifyLevel;

    fn note(level: Level) -> Notification {
        Notification { id: 1, time: 0, source: "t".into(), level, title: "x".into(), body: None, read: false, actions: Vec::new() }
    }

    #[test]
    fn the_mode_and_level_decide() {
        let background = SystemNotificationSettings::default();
        assert!(wanted(&background, &note(Level::Info), false), "away from Files: sent");
        assert!(!wanted(&background, &note(Level::Info), true), "in front: the toast is enough");
        let always = SystemNotificationSettings { mode: SystemNotify::Always, min_level: NotifyLevel::Warnings };
        assert!(wanted(&always, &note(Level::Error), true));
        assert!(!wanted(&always, &note(Level::Success), true), "below the level");
        let off = SystemNotificationSettings { mode: SystemNotify::Off, ..Default::default() };
        assert!(!wanted(&off, &note(Level::Error), false));
    }
}
