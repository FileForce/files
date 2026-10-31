//! "About Files" and "Check for Updates…" (app menu, Settings → General,
//! and a daily automatic check that posts a notification).

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div, img, px,
};

use crate::app::{AppearanceSettings, FileExplorer};
use crate::palette::Palette;
use crate::updates::{CURRENT_VERSION, Download, UpdateStatus};

/// The app icon, from the same SVG the bundle's icons are made from.
pub(crate) fn app_icon(size: f32) -> impl IntoElement {
    static ICON: std::sync::OnceLock<Arc<gpui_kit::Image>> = std::sync::OnceLock::new();
    let image = ICON
        .get_or_init(|| Arc::new(gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Svg, include_bytes!("../../packaging/icon/icon.svg").to_vec())))
        .clone();
    img(image).size(px(size))
}

fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("/usr/bin/open").arg(url).spawn();
    #[cfg(not(target_os = "macos"))]
    let _ = std::process::Command::new(if cfg!(windows) { "explorer" } else { "xdg-open" }).arg(url).spawn();
}

/// Where the update stands after "Download & Install".
#[derive(Clone, Debug, PartialEq)]
enum Install {
    Idle,
    Downloading,
    /// Downloaded, verified and opened.
    Opened(std::path::PathBuf),
    Failed(String),
}

pub struct UpdateDialog {
    appearance: Entity<AppearanceSettings>,
    /// `None` while the check runs.
    result: Option<Result<UpdateStatus, String>>,
    install: Install,
}

impl UpdateDialog {
    fn check(&mut self, cx: &mut Context<Self>) {
        self.result = None;
        self.install = Install::Idle;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async { crate::updates::check() }).await;
            let _ = this.update(cx, |this, cx| {
                this.result = Some(result);
                cx.notify();
            });
        })
        .detach();
    }

    /// Downloads into Downloads, checks the publisher's signature, opens it.
    fn download(&mut self, download: Download, cx: &mut Context<Self>) {
        self.install = Install::Downloading;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::updates::download(&download, &crate::licensing::trusted()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.install = match result {
                    Ok(path) => {
                        open_url(&path.display().to_string());
                        files_sdk::log::notify(
                            "files",
                            files_sdk::log::Level::Success,
                            "Update downloaded and verified",
                            Some(format!("{} · drag Files to Applications, then reopen it", path.display())),
                        );
                        Install::Opened(path)
                    }
                    Err(error) => Install::Failed(error),
                };
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for UpdateDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&settings);
        let source = if crate::updates::UPDATE_URL.is_empty() { "GitHub" } else { "the Files website" };
        let mut notes = None;
        let mut actions = Vec::new();
        let (icon, color, title, detail): (IconName, _, String, String) = match self.result.clone() {
            None => (IconName::RefreshCw, palette.muted, "Checking for updates…".into(), format!("Asking {source} for the newest version.")),
            Some(Ok(UpdateStatus::UpToDate { latest })) => (
                IconName::CircleCheck,
                palette.code,
                "Files is up to date".into(),
                format!("You have {CURRENT_VERSION}; the newest is {latest}."),
            ),
            Some(Ok(UpdateStatus::Available { latest, notes: text, page, download })) => {
                notes = (!text.trim().is_empty()).then_some(text);
                if let Some(download) = download.filter(|download| download.signature.is_some()) {
                    actions.push(
                        Button::new("update-install")
                            .primary()
                            .small()
                            .icon(IconName::Download)
                            .label("Download & Install")
                            .loading(self.install == Install::Downloading)
                            .disabled(self.install == Install::Downloading)
                            .on_click(cx.listener(move |this, _, _, cx| this.download(download.clone(), cx)))
                            .into_any_element(),
                    );
                }
                if let Some(page) = page {
                    actions.push(
                        Button::new("update-open")
                            .ghost()
                            .small()
                            .label("Release page")
                            .on_click(move |_, _, _| open_url(&page))
                            .into_any_element(),
                    );
                }
                (IconName::Download, palette.accent, format!("Files {latest} is available"), format!("You have {CURRENT_VERSION}."))
            }
            Some(Ok(UpdateStatus::NoReleases)) => (IconName::Info, palette.muted, "No releases yet".into(), "Nothing has been published yet.".into()),
            Some(Ok(UpdateStatus::NotConfigured)) => (
                IconName::Info,
                palette.muted,
                "Update checks are not set up".into(),
                "Set [updates] url in Bundle.toml to your site's latest.json (cargo xtask bundle writes it), or `repository` in Cargo.toml for GitHub releases.".into(),
            ),
            Some(Err(error)) => (IconName::TriangleAlert, palette.video, "Could not check for updates".into(), error),
        };
        let install = match &self.install {
            Install::Idle | Install::Downloading => None,
            Install::Opened(path) => Some((palette.code, format!("Verified and opened {} · drag Files to Applications, then reopen it.", path.display()))),
            Install::Failed(error) => Some((palette.video, error.clone())),
        };
        v_flex()
            .gap_4()
            .text_color(palette.text)
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(Icon::new(icon).text_color(color))
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                            .child(div().text_xs().text_color(palette.muted).whitespace_normal().child(detail)),
                    ),
            )
            .children(notes.map(|notes| {
                div()
                    .id("update-notes")
                    .max_h(px(200.))
                    .p_3()
                    .rounded_md()
                    .bg(palette.surface)
                    .text_xs()
                    .overflow_y_scrollbar()
                    .child(gpui_kit::component::text::TextView::markdown("update-notes-text", notes).selectable(true))
            }))
            .children(install.map(|(color, text)| div().text_xs().text_color(color).whitespace_normal().child(text)))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(Button::new("update-again").ghost().small().label("Check again").on_click(cx.listener(|this, _, _, cx| this.check(cx))))
                    .children(actions)
                    .child(Button::new("update-close").secondary().small().label("Close").on_click(|_, window, cx| window.close_dialog(cx))),
            )
    }
}

impl FileExplorer {
    pub fn open_update_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let appearance = self.appearance.clone();
        let dialog = cx.new(|cx| {
            let mut dialog = UpdateDialog { appearance, result: None, install: Install::Idle };
            dialog.check(cx);
            dialog
        });
        window.open_dialog(cx, move |modal, _, _| {
            modal
                .title("Check for Updates")
                .centered()
                .w(px(480.))
                .close_button(false)
                .overlay(true)
                .overlay_closable(true)
                .child(dialog.clone())
        });
    }

    /// Once a day (Settings → General → Check for updates automatically):
    /// a notification when a newer version is out.
    pub(crate) fn auto_check_updates(&mut self, cx: &mut Context<Self>) {
        self.announce_update(cx);
        let updates = self.appearance.read(cx).updates.clone();
        if !crate::updates::auto_check_due(&updates) {
            return;
        }
        self.appearance.update(cx, |settings, _| {
            settings.updates.last_check = crate::updates::now_seconds();
            let _ = crate::settings::save(settings);
        });
        cx.spawn(async move |_, cx| {
            let result = cx.background_executor().spawn(async { crate::updates::check() }).await;
            if let Ok(UpdateStatus::Available { latest, .. }) = result {
                files_sdk::log::notify(
                    "files",
                    files_sdk::log::Level::Info,
                    format!("Files {latest} is available"),
                    Some("Files → Check for Updates… to download it".to_string()),
                );
            }
        })
        .detach();
    }

    /// First run of a new version: "Updated to Files X" with a button that
    /// opens its release notes. Remembered, so it shows once.
    fn announce_update(&mut self, cx: &mut Context<Self>) {
        let last = self.appearance.read(cx).updates.last_version.clone();
        if last == CURRENT_VERSION {
            return;
        }
        if let Some(version) = crate::updates::just_updated(&last, crate::settings::existed_at_start()) {
            let from = if last.is_empty() { "From an earlier version".to_string() } else { format!("From {last}") };
            files_sdk::log::notify_with(
                "files",
                files_sdk::log::Level::Success,
                format!("Updated to Files {version}"),
                Some(format!("{from}. See what is new.")),
                Some(files_sdk::log::Action { label: "Open release notes".into(), url: crate::updates::release_notes_url(&version) }),
            );
        }
        self.appearance.update(cx, |settings, _| {
            settings.updates.last_version = CURRENT_VERSION.to_string();
            let _ = crate::settings::save(settings);
        });
    }

    pub fn open_about_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let palette = self.palette();
        let repository = crate::updates::github_repository();
        let licensed_to = crate::licensing::status(self.appearance.read(cx))
            .and_then(Result::ok)
            .map(|license| format!("Pro licensed to {} · license {}", license.email, license.id));
        let plugins = crate::plugins::registry().len();
        let explorer = cx.entity();
        window.open_dialog(cx, move |modal, _, _| {
            let repository = repository.clone();
            let explorer = explorer.clone();
            modal
                .centered()
                .w(px(400.))
                .overlay(true)
                .overlay_closable(true)
                .child(
                    v_flex()
                        .items_center()
                        .gap_2()
                        .py_2()
                        .text_color(palette.text)
                        .child(app_icon(96.))
                        .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(env!("FILES_APP_NAME")))
                        .child(div().text_xs().text_color(palette.muted).child(format!(
                            "Version {CURRENT_VERSION} · {} {}",
                            std::env::consts::OS,
                            std::env::consts::ARCH
                        )))
                        .child(div().text_xs().text_color(palette.faint).child("A file explorer with extensions, built with GPUI"))
                        // Who the license belongs to: a key shared online shows its owner.
                        .children(licensed_to.clone().map(|text| div().text_xs().text_color(palette.muted).child(text)))
                        .when(plugins > 0, |this| this.child(div().text_xs().text_color(palette.faint).child(format!("{plugins} Pro extension(s) loaded"))))
                        .child(
                            h_flex()
                                .gap_1()
                                .pt_2()
                                .child(
                                    Button::new("about-updates")
                                        .secondary()
                                        .small()
                                        .icon(IconName::RefreshCw)
                                        .label("Check for Updates…")
                                        .on_click(move |_, window, cx| {
                                            window.close_dialog(cx);
                                            explorer.update(cx, |explorer, cx| explorer.open_update_dialog(window, cx));
                                        }),
                                )
                                .when(!crate::updates::WEBSITE.is_empty(), |row| {
                                    row.child(
                                        Button::new("about-website")
                                            .ghost()
                                            .small()
                                            .label("Website")
                                            .on_click(|_, _, _| open_url(crate::updates::WEBSITE)),
                                    )
                                })
                                .children(repository.map(|repository| {
                                    let url = format!("https://github.com/{repository}");
                                    Button::new("about-repository").ghost().small().label("GitHub").on_click(move |_, _, _| open_url(&url))
                                })),
                        )
                        .when(!crate::updates::COPYRIGHT.is_empty(), |this| {
                            this.child(div().pt_1().text_size(px(10.)).text_color(palette.faint).child(crate::updates::COPYRIGHT))
                        }),
                )
        });
    }
}
