//! Settings → General → Uninstall Files…: takes away everything Files put
//! outside its own folder — the `files` and `open` commands and their PATH
//! lines, being the folder manager, the Linux desktop entry — then its
//! caches, its settings if asked, and the app itself.

use std::path::{Path, PathBuf};

/// What the confirmation lists and what runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub settings: bool,
}

/// The steps, as the dialog shows them.
pub fn steps(plan: &Plan) -> Vec<String> {
    let mut steps = vec![
        "The files command and the open helper, and their PATH lines".to_string(),
        if cfg!(target_os = "macos") {
            "Finder opens folders again".to_string()
        } else if cfg!(windows) {
            "Explorer opens folders again".to_string()
        } else {
            "The desktop entry and icon".to_string()
        },
        "Caches: thumbnails, previews, music, extensions' caches".to_string(),
    ];
    if plan.settings {
        steps.push("Settings, favorites, accounts' settings and keys Files saved".to_string());
    }
    steps.push(match app_location() {
        Some(app) => format!("The app: {}", app.display()),
        None => "The app (Windows: through its uninstaller)".to_string(),
    });
    steps
}

/// The app bundle, AppImage or folder to remove.
pub fn app_location() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        // …/Files.app/Contents/MacOS/file-force
        let exe = std::env::current_exe().ok()?;
        return exe.ancestors().find(|path| path.extension().is_some_and(|extension| extension == "app")).map(Path::to_path_buf);
    }
    if cfg!(windows) {
        return None;
    }
    std::env::var_os("APPIMAGE").map(PathBuf::from)
}

fn remove(path: &Path) {
    let _ = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
}

/// Everything but the app: safe to run, then the app goes.
pub fn clean(plan: &Plan) -> Vec<String> {
    // Tests never touch the user's real caches and settings.
    if cfg!(test) {
        return Vec::new();
    }
    let mut problems = Vec::new();
    let mut step = |name: &str, result: std::io::Result<()>| {
        if let Err(error) = result {
            problems.push(format!("{name}: {error}"));
        }
    };
    step("files command", crate::desktop::uninstall_cli());
    step("open helper", crate::desktop::set_open_script(false));
    if crate::desktop::is_default_file_manager() {
        step("folder manager", crate::desktop::restore_system_file_manager());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(data) = dirs::data_dir() {
            remove(&data.join("applications").join(format!("{}.desktop", crate::desktop::APP_ID)));
            remove(&data.join("icons/hicolor/scalable/apps").join(format!("{}.svg", crate::desktop::APP_ID)));
        }
    }
    for kind in crate::storage::Kind::ALL {
        crate::storage::clear(kind);
    }
    if let Some(cache) = dirs::cache_dir() {
        remove(&cache.join("file-force"));
    }
    remove(&std::env::temp_dir().join("file-force-sounds"));
    crate::instance::release();
    if plan.settings
        && let Some(folder) = crate::settings::settings_path().and_then(|path| path.parent().map(Path::to_path_buf))
    {
        remove(&folder);
        if let Some(data) = dirs::data_dir() {
            remove(&data.join("file-force"));
        }
    }
    problems
}

/// Removes the app and quits: macOS to the Trash, Linux the AppImage,
/// Windows its own uninstaller (which also takes the shortcuts and the
/// folder verb's registry keys).
pub fn remove_app() -> std::io::Result<()> {
    #[cfg(windows)]
    {
        // The MSI's product, found by its name and publisher.
        let script = "$k = Get-ChildItem HKLM:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall, HKLM:\\Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall -ErrorAction SilentlyContinue | Get-ItemProperty | Where-Object { $_.DisplayName -eq 'Files' -and $_.Publisher -eq 'FileForce' } | Select-Object -First 1; if ($k) { Start-Process msiexec.exe -ArgumentList '/x', $k.PSChildName, '/passive' } else { exit 1 }";
        let status = files_sdk::log::command("powershell").args(["-NoProfile", "-Command", script]).status()?;
        if !status.success() {
            // Not installed by the MSI (a copied folder): Settings → Apps.
            let _ = files_sdk::log::command("explorer").arg("ms-settings:appsfeatures").spawn();
        }
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        let Some(app) = app_location() else {
            return Err(std::io::Error::other("Files is not running from an installed app (a development build?)"));
        };
        if cfg!(target_os = "macos") {
            trash::delete(&app).map_err(|error| std::io::Error::other(error.to_string()))
        } else {
            std::fs::remove_file(&app)
        }
    }
}

// ---------------------------------------------------------------- the dialog

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, WindowExt as _};
use gpui_kit::{AppContext as _, Context, Entity, FontWeight, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px};

pub struct UninstallDialog {
    appearance: Entity<crate::app::AppearanceSettings>,
    plan: Plan,
    error: Option<String>,
    working: bool,
}

impl UninstallDialog {
    fn run(&mut self, cx: &mut Context<Self>) {
        self.working = true;
        cx.notify();
        let plan = self.plan.clone();
        let task = cx.background_executor().spawn(async move {
            let problems = clean(&plan);
            (problems, remove_app())
        });
        cx.spawn(async move |this, cx| {
            let (problems, removed) = task.await;
            let _ = this.update(cx, |this, cx| match removed {
                Ok(()) => {
                    files_sdk::log::record("files", files_sdk::log::Kind::Setting, format!("Uninstalled ({} problems)", problems.len()), Some(problems.join("\n")));
                    cx.quit();
                }
                Err(error) => {
                    this.working = false;
                    this.error = Some(format!("Everything else is cleaned up, but the app stays: {error}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Render for UninstallDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = crate::palette::Palette::from_settings(self.appearance.read(cx));
        let steps = steps(&self.plan).into_iter().map(|step| {
            h_flex().gap_2().items_start().text_sm().child(Icon::new(IconName::Check).small().text_color(palette.muted)).child(div().flex_1().child(step))
        });
        v_flex()
            .gap_3()
            .p_5()
            .child(h_flex().gap_2().items_center().child(Icon::new(IconName::Trash).text_color(palette.video)).child(div().text_base().font_weight(FontWeight::SEMIBOLD).child("Uninstall Files")))
            .child(div().text_sm().text_color(palette.muted).child("Files removes what it added to your system, then itself, and quits. Your own files are not touched."))
            .child(v_flex().gap_1p5().children(steps))
            .child(
                Checkbox::new("uninstall-settings")
                    .label("Also remove settings and favorites (otherwise kept for a later install)")
                    .checked(self.plan.settings)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.plan.settings = *checked;
                        cx.notify();
                    })),
            )
            .children(self.error.clone().map(|error| div().text_xs().text_color(palette.video).child(error)))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(Button::new("uninstall-cancel").ghost().small().label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(
                        Button::new("uninstall-run")
                            .danger()
                            .small()
                            .icon(IconName::Trash)
                            .label(if self.working { "Uninstalling…" } else { "Uninstall" })
                            .disabled(self.working)
                            .on_click(cx.listener(|this, _, _, cx| this.run(cx))),
                    ),
            )
    }
}

pub fn open(appearance: Entity<crate::app::AppearanceSettings>, window: &mut Window, cx: &mut gpui_kit::App) {
    let view = cx.new(|_| UninstallDialog { appearance, plan: Plan { settings: false }, error: None, working: false });
    window.open_dialog(cx, move |dialog, _, _| dialog.w(px(560.)).p_0().centered().close_button(false).overlay(true).overlay_closable(false).on_ok(|_, _, _| false).child(view.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_lists_settings_only_when_asked() {
        let without = steps(&Plan { settings: false });
        let with = steps(&Plan { settings: true });
        assert!(without.iter().any(|step| step.contains("files command")));
        assert!(!without.iter().any(|step| step.starts_with("Settings")));
        assert_eq!(with.len(), without.len() + 1);
        assert!(app_location().is_none() || cfg!(not(test)), "a test binary is not an installed app");
    }
}
