use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::app::FileExplorer;
use crate::forks::{Change, CloneKind};
use crate::model::Location;
use crate::palette::Palette;

/// `+`, `~` or `−` with its color, for change lists.
fn change_row(change: &Change, palette: Palette) -> AnyElement {
    let (sign, color, rel) = match change {
        Change::Added { rel, folder } => ("+", palette.code, if *folder { format!("{rel}/") } else { rel.clone() }),
        Change::Modified { rel } => ("~", palette.archive, rel.clone()),
        Change::Removed { rel } => ("−", palette.video, rel.clone()),
    };
    h_flex()
        .gap_2()
        .text_xs()
        .child(div().w(px(12.)).text_color(color).child(sign))
        .child(div().flex_1().truncate().child(rel))
        .into_any_element()
}

impl FileExplorer {
    /// Makes a fork of a real folder in the background, then opens it.
    pub fn fork_folder(&mut self, location: Location, window: &mut Window, cx: &mut Context<Self>) {
        if !self.appearance.read(cx).extensions.forks {
            self.notice = Some("Forks extension is disabled · turn it on in Settings → Extensions".to_string());
            cx.notify();
            return;
        }
        let Some(path) = location.path() else {
            self.notice = Some(format!("{} cannot be forked", location.label()));
            cx.notify();
            return;
        };
        if crate::forks::fork_of(&path).is_some() {
            self.notice = Some("This folder is already inside a fork".to_string());
            cx.notify();
            return;
        }
        self.notice = Some(format!("Forking {}…", location.label()));
        cx.notify();
        let label = location.label();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move { crate::forks::create(&path) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok((_, dir, kind)) => {
                        this.navigate(Location::from_path(&dir), window, cx);
                        this.notice = Some(match kind {
                            CloneKind::CopyOnWrite => format!(
                                "Forked {label} · copy-on-write clone: instant, no extra space until files change"
                            ),
                            CloneKind::Copy => format!(
                                "Forked {label} · plain copy (this disk has no copy-on-write support)"
                            ),
                        });
                    }
                    Err(error) => this.notice = Some(format!("Fork failed: {}", crate::delete::describe(&error).message)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks, then deletes a fork's copy and returns to the original.
    pub fn open_discard_fork_dialog(&mut self, fork: u32, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let Some(summary) = crate::forks::list().into_iter().find(|summary| summary.id == fork) else {
            return;
        };
        let explorer = cx.entity().downgrade();
        let palette = self.palette();
        window.open_dialog(cx, move |dialog, _, _| {
            let explorer = explorer.clone();
            dialog
                .title(format!("Discard \"{}\"?", summary.name))
                .centered()
                .w(px(440.))
                .close_button(false)
                .overlay(true)
                .overlay_closable(true)
                .on_ok({
                    let explorer = explorer.clone();
                    move |_, window, cx| {
                        discard(&explorer, fork, window, cx);
                        true
                    }
                })
                .child(
                    v_flex()
                        .gap_4()
                        .text_color(palette.text)
                        .child(div().text_sm().child(format!(
                            "The fork's copy and its {} change{} are deleted. {} itself is not touched.",
                            summary.changes,
                            if summary.changes == 1 { "" } else { "s" },
                            summary.base.display()
                        )))
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .child(
                                    Button::new("discard-fork-cancel")
                                        .ghost()
                                        .small()
                                        .label("Keep fork")
                                        .on_click(|_, window, cx| window.close_dialog(cx)),
                                )
                                .child(
                                    Button::new("discard-fork-confirm")
                                        .danger()
                                        .small()
                                        .icon(IconName::Trash)
                                        .label("Discard fork")
                                        .on_click(move |_, window, cx| {
                                            discard(&explorer, fork, window, cx);
                                            window.close_dialog(cx);
                                        }),
                                ),
                        ),
                )
        });
    }

    /// Lists what the fork changed relative to the original folder.
    pub fn open_fork_changes_dialog(&mut self, fork: u32, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let palette = self.palette();
        let changes = crate::forks::changes_now(fork);
        let base = crate::forks::base(fork).unwrap_or_default();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Fork changes")
                .centered()
                .w(px(520.))
                .overlay(true)
                .overlay_closable(true)
                .child(
                    v_flex()
                        .gap_3()
                        .text_color(palette.text)
                        .child(div().text_xs().text_color(palette.muted).child(format!(
                            "Relative to {} · + added  ~ modified  − removed",
                            base.display()
                        )))
                        .child(if changes.is_empty() {
                            div().text_sm().text_color(palette.muted).child("No changes yet").into_any_element()
                        } else {
                            div()
                                .id("fork-changes")
                                .max_h(px(320.))
                                .overflow_y_scroll()
                                .child(v_flex().gap_1().children(changes.iter().map(|change| change_row(change, palette))))
                                .into_any_element()
                        }),
                )
        });
    }

    /// Confirms, then writes the fork's changes into the original folder.
    pub fn open_apply_fork_dialog(&mut self, fork: u32, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let changes = crate::forks::changes_now(fork);
        let Some(base) = crate::forks::base(fork) else {
            return;
        };
        let explorer = cx.entity().downgrade();
        let palette = self.palette();
        let use_trash = self.appearance.read(cx).deletion.use_trash;
        let removals = changes.iter().filter(|change| matches!(change, Change::Removed { .. })).count();
        window.open_dialog(cx, move |dialog, _, _| {
            let removal_note = match (removals, use_trash) {
                (0, _) => "Nothing will be removed.".to_string(),
                (count, true) => format!("{count} removed item(s) go to the Trash and can be restored."),
                (count, false) => format!("{count} removed item(s) are deleted permanently (Settings → Deletion)."),
            };
            let explorer = explorer.clone();
            dialog
                .title(format!("Apply {} change{}?", changes.len(), if changes.len() == 1 { "" } else { "s" }))
                .centered()
                .w(px(520.))
                .close_button(false)
                .overlay(true)
                .overlay_closable(true)
                .on_ok({
                    let explorer = explorer.clone();
                    move |_, window, cx| {
                        apply(&explorer, fork, window, cx);
                        true
                    }
                })
                .child(
                    v_flex()
                        .gap_3()
                        .text_color(palette.text)
                        .child(div().text_sm().child(format!("These changes will be written to {}.", base.display())))
                        .child(
                            div()
                                .id("apply-fork-changes")
                                .max_h(px(260.))
                                .overflow_y_scroll()
                                .p_2()
                                .rounded_md()
                                .bg(palette.surface)
                                .child(v_flex().gap_1().children(changes.iter().map(|change| change_row(change, palette)))),
                        )
                        .child(div().text_xs().text_color(palette.muted).child(removal_note))
                        .child(div().text_xs().text_color(palette.muted).child(
                            "Files changed in the original after forking are kept, and protected paths are never removed.",
                        ))
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .child(
                                    Button::new("apply-fork-cancel")
                                        .ghost()
                                        .small()
                                        .label("Cancel")
                                        .on_click(|_, window, cx| window.close_dialog(cx)),
                                )
                                .child(
                                    Button::new("apply-fork-confirm")
                                        .primary()
                                        .small()
                                        .icon(IconName::Check)
                                        .label("Apply to folder")
                                        .on_click(move |_, window, cx| {
                                            apply(&explorer, fork, window, cx);
                                            window.close_dialog(cx);
                                        }),
                                ),
                        ),
                )
        });
    }

    pub(crate) fn render_forks(&self, active: Location, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.appearance.read(cx).extensions.forks {
            return None;
        }
        let forks = crate::forks::list();
        if forks.is_empty() {
            return None;
        }
        let active_fork = active.path().and_then(|path| crate::forks::fork_of(&path));
        let mut section = v_flex().gap_1().child(self.section_header("forks", "FORKS", palette, cx));
        if self.section_collapsed("forks", cx) {
            return Some(section.into_any_element());
        }
        for fork in forks {
            let selected = active_fork == Some(fork.id);
            let id = fork.id;
            let dir = fork.dir.clone();
            let view = cx.entity();
            section = section.child(
                div()
                    .id(("fork-item", fork.id as usize))
                    .h(px(30.))
                    .px_2()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .text_color(if selected { palette.text } else { palette.muted })
                    .bg(if selected { palette.selection } else { palette.sidebar })
                    .hover(|style| style.bg(palette.hover).text_color(palette.text))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.navigate(Location::from_path(&dir), window, cx);
                    }))
                    .child(Icon::new(IconName::GitFork).small().text_color(palette.accent))
                    .child(div().flex_1().min_w_0().truncate().child(fork.name.clone()))
                    .children((fork.changes > 0).then(|| {
                        div().text_xs().text_color(palette.faint).child(fork.changes.to_string())
                    }))
                    .context_menu(move |menu, window, _| {
                        menu.item(
                            PopupMenuItem::new("Open fork")
                                .icon(IconName::FolderOpen)
                                .on_click(window.listener_for(&view, move |this, _, window, cx| {
                                    if let Some(root) = crate::forks::root(id).map(|dir| Location::from_path(&dir)) {
                                        this.navigate(root, window, cx);
                                    }
                                })),
                        )
                        .item(
                            PopupMenuItem::new("Apply to folder…")
                                .icon(IconName::Check)
                                .on_click(window.listener_for(&view, move |this, _, window, cx| {
                                    this.open_apply_fork_dialog(id, window, cx);
                                })),
                        )
                        .item(
                            PopupMenuItem::new("Show changes…")
                                .icon(IconName::GitCompare)
                                .on_click(window.listener_for(&view, move |this, _, window, cx| {
                                    this.open_fork_changes_dialog(id, window, cx);
                                })),
                        )
                        .separator()
                        .item(
                            PopupMenuItem::new("Discard fork…")
                                .icon(IconName::Trash)
                                .on_click(window.listener_for(&view, move |this, _, window, cx| {
                                    this.open_discard_fork_dialog(id, window, cx);
                                })),
                        )
                    }),
            );
        }
        Some(section.into_any_element())
    }

    /// Header actions while browsing anywhere inside a fork: a compact fork
    /// marker (original in its tooltip), changes, apply and discard.
    pub(crate) fn fork_actions(&self, fork: u32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let base = crate::forks::base(fork)?;
        let count = crate::forks::changes(fork).len();
        Some(
            h_flex()
                .gap_1()
                .items_center()
                .child(
                    Button::new("fork-marker")
                        .ghost()
                        .xsmall()
                        .icon(IconName::GitFork)
                        .tooltip(format!("Fork of {}", base.display())),
                )
                .child(
                    Button::new("fork-changes")
                        .ghost()
                        .xsmall()
                        .icon(IconName::GitCompare)
                        .label(format!("Changes ({count})"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_fork_changes_dialog(fork, window, cx);
                        })),
                )
                .child(
                    Button::new("fork-apply")
                        .primary()
                        .xsmall()
                        .icon(IconName::Check)
                        .label("Apply")
                        .disabled(count == 0)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_apply_fork_dialog(fork, window, cx);
                        })),
                )
                .child(
                    Button::new("fork-discard")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Trash)
                        .tooltip("Discard fork")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_discard_fork_dialog(fork, window, cx);
                        })),
                )
                .into_any_element(),
        )
    }
}

fn apply(explorer: &gpui_kit::WeakEntity<FileExplorer>, fork: u32, window: &mut Window, cx: &mut gpui_kit::App) {
    let _ = explorer.update(cx, |explorer, cx| {
        let settings = explorer.appearance.read(cx).clone();
        let home = Location::Home.path().unwrap_or_default();
        match crate::forks::apply(fork, &settings.deletion.protected, &home, settings.deletion.use_trash) {
            Ok((base, report)) => {
                explorer.navigate(Location::from_path(&base), window, cx);
                let mut notice = format!(
                    "Fork applied · {} created · {} updated · {} removed",
                    report.created, report.updated, report.removed
                );
                if let Some((item, reason)) = report.skipped.first() {
                    notice.push_str(&format!(" · {} skipped ({item}: {reason})", report.skipped.len()));
                }
                explorer.notice = Some(notice);
            }
            Err(error) => explorer.notice = Some(error),
        }
        cx.notify();
    });
}

fn discard(explorer: &gpui_kit::WeakEntity<FileExplorer>, fork: u32, window: &mut Window, cx: &mut gpui_kit::App) {
    let inside = explorer
        .upgrade()
        .and_then(|explorer| explorer.read(cx).active_location().path())
        .and_then(|path| crate::forks::fork_of(&path))
        == Some(fork);
    let base = crate::forks::discard(fork);
    crate::model::invalidate_listings();
    let _ = explorer.update(cx, |explorer, cx| {
        if inside && let Some(base) = &base {
            explorer.navigate(Location::from_path(base), window, cx);
        }
        explorer.notice = Some("Fork discarded · the original folder was never changed".to_string());
        cx.notify();
    });
}
