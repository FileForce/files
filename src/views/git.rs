//! Git in the UI (modeled on TortoiseGit): the context submenu (what it offers depends on
//! whether the item is inside a repository) and the Git window with its
//! Commit, Log, Diff, Branches and Clone pages.

use std::path::{Path, PathBuf};

use files_ext_git::{Branch, Change, ChangeKind, Commit, LineKind};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, WindowBounds, WindowOptions, actions, div, point, px, size,
};

use crate::app::{AppearanceSettings, FileExplorer};
use crate::palette::Palette;
use crate::views::context_menu::{tinted_icon, tinted_item};

actions!(git, [CloseGit]);

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum GitPage {
    Commit,
    /// Of the whole repository, or of one file.
    Log(Option<PathBuf>),
    Diff(String),
    Branches,
    Clone,
}

// ------------------------------------------------------------ context menu

/// The "Git" submenu for `path` (a file, a folder, or the folder
/// whose empty area was clicked).
pub(crate) fn git_submenu(
    menu: PopupMenu,
    view: Entity<FileExplorer>,
    path: PathBuf,
    color: Option<Hsla>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let root = files_ext_git::repo_root(&path);
    let icon = tinted_icon(crate::extensions::icon("Git"), color);
    menu.submenu_with_icon(Some(icon), "Git", window, cx, move |submenu, window, _| {
        let item = |label: &str, icon: IconName, action: GitAction, window: &mut Window| {
            let path = path.clone();
            tinted_item(label.to_string(), icon, color).on_click(window.listener_for(&view, move |this, _, window, cx| {
                this.git_action(action.clone(), path.clone(), window, cx);
            }))
        };
        let Some(root) = root.clone() else {
            // Outside a repository: what TortoiseGit offers there.
            return submenu
                .item(item("Git Clone…", IconName::Download, GitAction::Clone, window))
                .item(item("Git Create Repository Here…", IconName::FolderGit2, GitAction::Init, window));
        };
        let is_file = path.is_file();
        let stashes = files_ext_git::stash_count(&root);
        submenu
            .item(item("Git Commit…", IconName::GitCommitHorizontal, GitAction::Open(GitPage::Commit), window))
            .when(is_file, |menu| {
                let rel = path.strip_prefix(&root).map(|rel| rel.display().to_string()).unwrap_or_default();
                menu.item(item("Diff", IconName::GitCompare, GitAction::Open(GitPage::Diff(rel)), window))
            })
            .item(item(
                "Show Log",
                IconName::Clock3,
                GitAction::Open(GitPage::Log(is_file.then(|| path.clone()))),
                window,
            ))
            .item(item("Check for Modifications", IconName::ListChecks, GitAction::Open(GitPage::Commit), window))
            .separator()
            .item(item("Pull", IconName::ArrowDownToLine, GitAction::Pull, window))
            .item(item("Fetch", IconName::RefreshCw, GitAction::Fetch, window))
            .item(item("Push", IconName::ArrowUpFromLine, GitAction::Push, window))
            .separator()
            .item(item("Switch/Checkout…", IconName::GitBranch, GitAction::Open(GitPage::Branches), window))
            .item(item("Create Branch…", IconName::GitBranchPlus, GitAction::Open(GitPage::Branches), window))
            .separator()
            .item(item("Stash Save", IconName::Archive, GitAction::StashSave, window))
            .when(stashes > 0, |menu| {
                menu.item(item(&format!("Stash Pop ({stashes})"), IconName::ArchiveRestore, GitAction::StashPop, window))
            })
            .separator()
            .item(item("Add", IconName::Plus, GitAction::Add, window))
            .item(item("Revert…", IconName::Undo2, GitAction::Revert, window))
    })
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum GitAction {
    Open(GitPage),
    Pull,
    Fetch,
    Push,
    StashSave,
    StashPop,
    Add,
    Revert,
    Clone,
    Init,
}

impl FileExplorer {
    pub(crate) fn git_action(&mut self, action: GitAction, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let folder = if path.is_dir() { path.clone() } else { path.parent().map(Path::to_path_buf).unwrap_or_default() };
        let root = files_ext_git::repo_root(&path);
        match action {
            GitAction::Open(page) => {
                if let Some(root) = root {
                    let _ = open_git_window(root, page, self.appearance.clone(), cx);
                }
            }
            GitAction::Clone => {
                let _ = open_git_window(folder, GitPage::Clone, self.appearance.clone(), cx);
            }
            GitAction::Init => self.git_in_background("Create repository", move || {
                files_ext_git::init(&folder).map(|()| format!("Created a repository in {}", folder.display()))
            }, cx),
            GitAction::Revert => self.confirm_revert(path, window, cx),
            action => {
                let Some(root) = root else { return };
                let label = format!("{action:?}");
                self.git_in_background(&label, move || match action {
                    GitAction::Pull => files_ext_git::pull(&root),
                    GitAction::Fetch => files_ext_git::fetch(&root),
                    GitAction::Push => files_ext_git::push(&root),
                    GitAction::StashSave => files_ext_git::stash_save(&root, None),
                    GitAction::StashPop => files_ext_git::stash_pop(&root),
                    GitAction::Add => files_ext_git::add(&root, &[path]).map(|()| "Added to Git".to_string()),
                    _ => Ok(String::new()),
                }, cx);
            }
        }
    }

    /// Runs a git command off the UI thread and reports it in the status bar.
    fn git_in_background(
        &mut self,
        label: &str,
        job: impl FnOnce() -> std::io::Result<String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let label = label.to_string();
        self.notice = Some(format!("Git · {label}…"));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { job() }).await;
            let _ = this.update(cx, |this, cx| {
                use files_sdk::log::Level;
                let (level, title, body) = match &result {
                    Ok(message) => (Level::Success, format!("Git · {label} done"), Some(message.clone())),
                    Err(error) => (Level::Error, format!("Git · {label} failed"), Some(error.to_string())),
                };
                files_sdk::log::notify(files_ext_git::MANIFEST.id, level, title, body);
                this.notice = Some(match result {
                    Ok(message) => format!("Git · {message}"),
                    Err(error) => format!("Git · {label} failed: {error}"),
                });
                crate::model::invalidate_listings();
                cx.notify();
            });
        })
        .detach();
    }

    /// Revert throws work away: ask first, like TortoiseGit does.
    fn confirm_revert(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = files_ext_git::repo_root(&path) else { return };
        let palette = self.palette();
        let explorer = cx.entity().downgrade();
        let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
        window.open_dialog(cx, move |dialog, _, _| {
            let revert = {
                let explorer = explorer.clone();
                let (root, path) = (root.clone(), path.clone());
                move |window: &mut Window, cx: &mut App| {
                    let (root, path) = (root.clone(), path.clone());
                    let _ = explorer.update(cx, |explorer, cx| {
                        explorer.git_in_background("Revert", move || {
                            files_ext_git::revert(&root, &[path]).map(|()| "Reverted local changes".to_string())
                        }, cx);
                    });
                    window.close_dialog(cx);
                }
            };
            let on_ok = revert.clone();
            dialog
                .title(format!("Revert {name}?"))
                .centered()
                .w(px(440.))
                .close_button(false)
                .overlay(true)
                .overlay_closable(true)
                .on_ok(move |_, window, cx| {
                    on_ok(window, cx);
                    false
                })
                .child(
                    v_flex()
                        .gap_4()
                        .text_color(palette.text)
                        .child(div().text_sm().child("Local changes are thrown away and cannot be restored. New, not versioned files are kept."))
                        .child(
                            h_flex()
                                .gap_2()
                                .justify_end()
                                .child(Button::new("git-revert-cancel").ghost().small().label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                                .child(
                                    Button::new("git-revert-ok")
                                        .danger()
                                        .small()
                                        .label("Revert")
                                        .on_click(move |_, window, cx| revert(window, cx)),
                                ),
                        ),
                )
        });
    }
}

// ------------------------------------------------------------ window

pub struct GitWindow {
    appearance: Entity<AppearanceSettings>,
    /// The repository (Clone: the folder to clone into).
    root: PathBuf,
    page: GitPage,
    status: Option<files_ext_git::Status>,
    /// Commit page: the ticked paths.
    ticked: std::collections::BTreeSet<String>,
    message: Entity<InputState>,
    branch_name: Entity<InputState>,
    url: Entity<InputState>,
    commits: Vec<Commit>,
    selected_commit: Option<usize>,
    commit_files: Vec<Change>,
    branches: Vec<Branch>,
    /// The diff on the right: (title, text).
    diff: Option<(String, String)>,
    busy: bool,
    notice: Option<String>,
    focus_handle: FocusHandle,
    _subscription: Subscription,
}

impl GitWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>, root: PathBuf, page: GitPage, appearance: Entity<AppearanceSettings>) -> Self {
        let message = cx.new(|cx| InputState::new(window, cx).placeholder("Commit message"));
        let branch_name = cx.new(|cx| InputState::new(window, cx).placeholder("new-branch-name"));
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("https://github.com/owner/repo.git or git@host:owner/repo.git"));
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let mut this = Self {
            _subscription: cx.observe(&appearance, |_, _, cx| cx.notify()),
            appearance,
            root,
            page,
            status: None,
            ticked: Default::default(),
            message,
            branch_name,
            url,
            commits: Vec::new(),
            selected_commit: None,
            commit_files: Vec::new(),
            branches: Vec::new(),
            diff: None,
            busy: false,
            notice: None,
            focus_handle,
        };
        this.load(cx);
        this
    }

    /// (Re)loads what the current page shows.
    fn load(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let page = self.page.clone();
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    let status = files_ext_git::status(&root).ok();
                    let commits = match &page {
                        GitPage::Log(path) => files_ext_git::log(&root, path.as_deref(), 300).unwrap_or_default(),
                        _ => Vec::new(),
                    };
                    let branches = matches!(page, GitPage::Branches).then(|| files_ext_git::branches(&root).unwrap_or_default());
                    let diff = match &page {
                        GitPage::Diff(path) => files_ext_git::diff_worktree(&root, path).ok().map(|text| (path.clone(), text)),
                        _ => None,
                    };
                    (status, commits, branches, diff)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let (status, commits, branches, diff) = loaded;
                if let Some(status) = &status {
                    // Everything starts ticked, like TortoiseGit's commit dialog (but untracked files).
                    if this.ticked.is_empty() {
                        this.ticked = status
                            .changes
                            .iter()
                            .filter(|change| change.kind != ChangeKind::Untracked)
                            .map(|change| change.path.clone())
                            .collect();
                    }
                }
                this.status = status;
                this.commits = commits;
                if let Some(branches) = branches {
                    this.branches = branches;
                }
                if diff.is_some() {
                    this.diff = diff;
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_page(&mut self, page: GitPage, cx: &mut Context<Self>) {
        self.page = page;
        self.diff = None;
        self.selected_commit = None;
        self.commit_files.clear();
        self.load(cx);
        cx.notify();
    }

    fn show_worktree_diff(&mut self, path: String, cx: &mut Context<Self>) {
        let root = self.root.clone();
        cx.spawn(async move |this, cx| {
            let text = cx.background_executor().spawn({
                let path = path.clone();
                async move { files_ext_git::diff_worktree(&root, &path) }
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.diff = Some((path, text.unwrap_or_else(|error| error.to_string())));
                cx.notify();
            });
        })
        .detach();
    }

    fn select_commit(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected_commit = Some(index);
        self.diff = None;
        let root = self.root.clone();
        let hash = self.commits[index].hash.clone();
        cx.spawn(async move |this, cx| {
            let files = cx.background_executor().spawn(async move { files_ext_git::commit_files(&root, &hash) }).await;
            let _ = this.update(cx, |this, cx| {
                this.commit_files = files.unwrap_or_default();
                cx.notify();
            });
        })
        .detach();
    }

    fn show_commit_diff(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(commit) = self.selected_commit.and_then(|index| self.commits.get(index)) else { return };
        let (root, hash) = (self.root.clone(), commit.hash.clone());
        cx.spawn(async move |this, cx| {
            let text = cx.background_executor().spawn({
                let path = path.clone();
                async move { files_ext_git::diff_commit(&root, &hash, &path) }
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.diff = Some((path, text.unwrap_or_else(|error| error.to_string())));
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs a git job off the UI thread; reloads the page when it is done.
    fn run(
        &mut self,
        job: impl FnOnce(PathBuf) -> std::io::Result<String> + Send + 'static,
        after: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.notice = None;
        cx.notify();
        let root = self.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move { job(root) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(message) => {
                        this.notice = Some(message);
                        after(this, window, cx);
                    }
                    Err(error) => this.notice = Some(format!("⚠ {error}")),
                }
                crate::model::invalidate_listings();
                this.load(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn commit(&mut self, push: bool, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.message.read(cx).value().to_string();
        let paths: Vec<String> = self.ticked.iter().cloned().collect();
        self.run(
            move |root| {
                let made = files_ext_git::commit(&root, &paths, &message)?;
                if push {
                    let pushed = files_ext_git::push(&root)?;
                    return Ok(format!("Committed {made} · {pushed}"));
                }
                Ok(format!("Committed {made}"))
            },
            |this, window, cx| {
                this.ticked.clear();
                this.message.update(cx, |input, cx| input.set_value("", window, cx));
            },
            window,
            cx,
        );
    }

    fn close(&mut self, _: &CloseGit, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    fn tab(&self, label: &'static str, icon: IconName, page: GitPage, cx: &mut Context<Self>) -> Button {
        let active = std::mem::discriminant(&self.page) == std::mem::discriminant(&page);
        Button::new(gpui_kit::ElementId::Name(format!("git-tab-{label}").into()))
            .small()
            .icon(icon)
            .label(label)
            .when(active, |button| button.primary())
            .when(!active, |button| button.ghost())
            .on_click(cx.listener(move |this, _, _, cx| this.set_page(page.clone(), cx)))
    }

    fn render_commit(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let changes = self.status.as_ref().map(|status| status.changes.clone()).unwrap_or_default();
        let all_ticked = !changes.is_empty() && changes.iter().all(|change| self.ticked.contains(&change.path));
        let rows = changes.iter().enumerate().map(|(index, change)| {
            let path = change.path.clone();
            let toggle = change.path.clone();
            let ticked = self.ticked.contains(&change.path);
            let row = h_flex()
                .id(("git-change", index))
                .px_2()
                .py_1()
                .gap_2()
                .items_center()
                .rounded_sm()
                .cursor_pointer()
                .hover(|style| style.bg(palette.hover))
                .when(self.diff.as_ref().is_some_and(|(shown, _)| *shown == change.path), |row| row.bg(palette.selection))
                .on_click(cx.listener(move |this, _, _, cx| this.show_worktree_diff(path.clone(), cx)))
                .child(Checkbox::new(("git-tick", index)).checked(ticked).on_click(cx.listener(move |this, checked: &bool, _, cx| {
                    if *checked {
                        this.ticked.insert(toggle.clone());
                    } else {
                        this.ticked.remove(&toggle);
                    }
                    cx.notify();
                })))
                .child(div().w(px(14.)).text_xs().font_weight(FontWeight::BOLD).text_color(kind_color(change.kind, palette)).child(change.kind.letter()))
                .child(div().flex_1().min_w_0().truncate().text_sm().child(change.path.clone()))
                .child(div().text_xs().text_color(palette.faint).child(change.kind.label()));
            #[cfg(test)]
            let row = row.debug_selector(move || format!("git-change-{index}"));
            row
        });
        let can_commit = !self.ticked.is_empty() && !self.busy;
        v_flex()
            .size_full()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Checkbox::new("git-tick-all").label("Select all").checked(all_ticked).on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            this.ticked = if *checked {
                                this.status.iter().flat_map(|status| status.changes.iter().map(|change| change.path.clone())).collect()
                            } else {
                                Default::default()
                            };
                            cx.notify();
                        })),
                    )
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(palette.muted).child(format!("{} of {} selected", self.ticked.len(), changes.len()))),
            )
            .child(
                div()
                    .id("git-changes")
                    .flex_1()
                    .min_h_0()
                    .rounded_md()
                    .border_1()
                    .border_color(palette.border)
                    .overflow_y_scrollbar()
                    .child(v_flex().p_1().children(rows).when(changes.is_empty(), |list| {
                        list.child(div().p_3().text_sm().text_color(palette.faint).child("No local changes: the working tree is clean."))
                    })),
            )
            .child(Input::new(&self.message).w_full())
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("git-commit-push")
                            .ghost()
                            .label("Commit & Push")
                            .disabled(!can_commit)
                            .on_click(cx.listener(|this, _, window, cx| this.commit(true, window, cx))),
                    )
                    .child(
                        Button::new("git-commit")
                            .primary()
                            .icon(IconName::GitCommitHorizontal)
                            .label("Commit")
                            .loading(self.busy)
                            .disabled(!can_commit)
                            .on_click(cx.listener(|this, _, window, cx| this.commit(false, window, cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_log(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.commits.iter().enumerate().map(|(index, commit)| {
            let selected = self.selected_commit == Some(index);
            h_flex()
                .id(("git-commit-row", index))
                .px_2()
                .py_1()
                .gap_2()
                .items_center()
                .rounded_sm()
                .cursor_pointer()
                .text_xs()
                .when(selected, |row| row.bg(palette.selection))
                .hover(|style| style.bg(palette.hover))
                .on_click(cx.listener(move |this, _, _, cx| this.select_commit(index, cx)))
                .child(div().w(px(62.)).font_family("Menlo").text_color(palette.accent).child(commit.short.clone()))
                .children(commit.refs.iter().map(|name| {
                    div().px_1().rounded_sm().bg(palette.accent_soft).text_size(px(9.)).child(name.replace("HEAD -> ", "→ "))
                }))
                .child(div().flex_1().min_w_0().truncate().text_color(palette.text).child(commit.subject.clone()))
                .child(div().w(px(110.)).truncate().text_color(palette.muted).child(commit.author.clone()))
                .child(div().w(px(36.)).text_color(palette.faint).child(crate::views::agents::ago(commit.time)))
        });
        let files = self.commit_files.iter().enumerate().map(|(index, change)| {
            let path = change.path.clone();
            h_flex()
                .id(("git-commit-file", index))
                .px_2()
                .py_0p5()
                .gap_2()
                .text_xs()
                .cursor_pointer()
                .hover(|style| style.bg(palette.hover))
                .on_click(cx.listener(move |this, _, _, cx| this.show_commit_diff(path.clone(), cx)))
                .child(div().w(px(14.)).font_weight(FontWeight::BOLD).text_color(kind_color(change.kind, palette)).child(change.kind.letter()))
                .child(div().flex_1().min_w_0().truncate().child(change.path.clone()))
        });
        v_flex()
            .size_full()
            .gap_2()
            .child(
                div()
                    .id("git-log")
                    .flex_1()
                    .min_h_0()
                    .rounded_md()
                    .border_1()
                    .border_color(palette.border)
                    .overflow_y_scrollbar()
                    .child(v_flex().p_1().children(rows).when(self.commits.is_empty(), |list| {
                        list.child(div().p_3().text_sm().text_color(palette.faint).child("No commits yet"))
                    })),
            )
            .when(self.selected_commit.is_some(), |this| {
                this.child(
                    div()
                        .id("git-commit-files")
                        .h(px(140.))
                        .rounded_md()
                        .border_1()
                        .border_color(palette.border)
                        .overflow_y_scrollbar()
                        .child(v_flex().p_1().children(files)),
                )
            })
            .into_any_element()
    }

    fn render_branches(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.branches.iter().enumerate().map(|(index, branch)| {
            let (name, remote) = (branch.name.clone(), branch.remote);
            h_flex()
                .px_2()
                .py_1()
                .gap_2()
                .items_center()
                .text_sm()
                .child(Icon::new(if branch.remote { IconName::Cloud } else { IconName::GitBranch }).small().text_color(palette.muted))
                .child(div().flex_1().min_w_0().truncate().child(branch.name.clone()))
                .when(branch.current, |row| row.child(div().text_xs().text_color(palette.code).child("current")))
                .when(!branch.current, |row| {
                    row.child(
                        Button::new(("git-switch", index))
                            .ghost()
                            .xsmall()
                            .label("Switch")
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let name = name.clone();
                                this.run(move |root| files_ext_git::switch(&root, &name, remote).map(|()| format!("Switched to {name}")), |_, _, _| {}, window, cx);
                            })),
                    )
                })
        });
        v_flex()
            .size_full()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.branch_name).w_full()))
                    .child(
                        Button::new("git-create-branch")
                            .primary()
                            .icon(IconName::GitBranchPlus)
                            .label("Create & Switch")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                let name = this.branch_name.read(cx).value().trim().to_string();
                                this.run(
                                    move |root| files_ext_git::create_branch(&root, &name, true).map(|()| format!("Created and switched to {name}")),
                                    |this, window, cx| this.branch_name.update(cx, |input, cx| input.set_value("", window, cx)),
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .child(
                div()
                    .id("git-branches")
                    .flex_1()
                    .min_h_0()
                    .rounded_md()
                    .border_1()
                    .border_color(palette.border)
                    .overflow_y_scrollbar()
                    .child(v_flex().p_1().children(rows)),
            )
            .into_any_element()
    }

    fn render_clone(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_3()
            .child(div().text_sm().text_color(palette.muted).child(format!("Clones into a new folder in {}", self.root.display())))
            .child(Input::new(&self.url).w_full())
            .child(
                h_flex().justify_end().child(
                    Button::new("git-clone")
                        .primary()
                        .icon(IconName::Download)
                        .label("Clone")
                        .loading(self.busy)
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let url = this.url.read(cx).value().trim().to_string();
                            this.run(
                                move |parent| files_ext_git::clone(&url, &parent).map(|target| format!("Cloned into {}", target.display())),
                                |_, _, _| {},
                                window,
                                cx,
                            );
                        })),
                ),
            )
            .into_any_element()
    }
}

fn kind_color(kind: ChangeKind, palette: Palette) -> Hsla {
    match kind {
        ChangeKind::Added | ChangeKind::Untracked => palette.code,
        ChangeKind::Deleted | ChangeKind::Conflicted => palette.video,
        ChangeKind::Modified | ChangeKind::Renamed => palette.archive,
    }
}

/// A colored unified diff.
fn render_diff(title: &str, text: &str, palette: Palette) -> AnyElement {
    let lines = files_ext_git::diff_lines(text).into_iter().take(4000).enumerate().map(|(index, (kind, line))| {
        let (color, background) = match kind {
            LineKind::Added => (palette.code, Some(palette.code.opacity(0.12))),
            LineKind::Removed => (palette.video, Some(palette.video.opacity(0.12))),
            LineKind::Hunk => (palette.accent, None),
            LineKind::Header => (palette.faint, None),
            LineKind::Context => (palette.text, None),
        };
        div()
            .id(("git-diff-line", index))
            .px_2()
            .whitespace_nowrap()
            .text_color(color)
            .when_some(background, |line, background| line.bg(background))
            .child(if line.is_empty() { " ".to_string() } else { line })
    });
    v_flex()
        .size_full()
        .gap_1()
        .child(div().text_xs().text_color(palette.muted).truncate().child(title.to_string()))
        .child(
            div()
                .id("git-diff")
                .flex_1()
                .min_h_0()
                .rounded_md()
                .border_1()
                .border_color(palette.border)
                .bg(palette.surface)
                .text_xs()
                .font_family("Menlo")
                .overflow_scrollbar()
                .child(v_flex().py_1().children(lines).when(text.trim().is_empty(), |list| {
                    list.child(div().p_2().text_color(palette.faint).child("No differences"))
                })),
        )
        .into_any_element()
}

impl Focusable for GitWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GitWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.appearance.read(cx).clone();
        let palette = Palette::from_settings(&settings);
        let branch = self.status.as_ref().and_then(|status| status.branch.clone()).unwrap_or_else(|| "—".to_string());
        let sync = self
            .status
            .as_ref()
            .map(|status| match (status.upstream.as_ref(), status.ahead, status.behind) {
                (None, _, _) => "no upstream".to_string(),
                (Some(_), 0, 0) => "up to date".to_string(),
                (Some(_), ahead, behind) => format!("↑{ahead} ↓{behind}"),
            })
            .unwrap_or_default();
        let header = h_flex()
            .px_4()
            .py_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(palette.border)
            .child(Icon::new(IconName::GitBranch).small().text_color(palette.accent))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).truncate().child(self.root.display().to_string()))
                    .child(div().text_xs().text_color(palette.muted).child(format!("{branch} · {sync}"))),
            )
            .when(self.page != GitPage::Clone, |this| {
                this.child(self.tab("Commit", IconName::GitCommitHorizontal, GitPage::Commit, cx))
                    .child(self.tab("Log", IconName::Clock3, GitPage::Log(None), cx))
                    .child(self.tab("Branches", IconName::GitBranch, GitPage::Branches, cx))
            });
        let page = match self.page.clone() {
            GitPage::Commit => self.render_commit(palette, cx),
            GitPage::Log(_) => self.render_log(palette, cx),
            GitPage::Branches => self.render_branches(palette, cx),
            GitPage::Clone => self.render_clone(palette, cx),
            GitPage::Diff(path) => match &self.diff {
                Some((title, text)) => render_diff(title, text, palette),
                None => div().text_sm().text_color(palette.faint).child(format!("Loading the diff of {path}…")).into_any_element(),
            },
        };
        // The commit and log pages show the clicked file's diff beside the list.
        let side_diff = matches!(self.page, GitPage::Commit | GitPage::Log(_))
            .then(|| self.diff.as_ref().map(|(title, text)| render_diff(title, text, palette)))
            .flatten();
        v_flex()
            .key_context("Git")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            .size_full()
            .bg(palette.window)
            .text_color(palette.text)
            .child(header)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .p_3()
                    .gap_3()
                    .child(div().flex_1().min_w_0().h_full().child(page))
                    .children(side_diff.map(|diff| div().flex_1().min_w_0().h_full().child(diff))),
            )
            .children(self.notice.clone().map(|notice| {
                div().px_4().py_2().border_t_1().border_color(palette.border).text_xs().text_color(palette.muted).child(notice)
            }))
    }
}

pub(crate) fn open_git_window(root: PathBuf, page: GitPage, appearance: Entity<AppearanceSettings>, cx: &mut App) -> Option<Entity<GitWindow>> {
    let title = match &page {
        GitPage::Clone => "Git Clone".to_string(),
        _ => format!("{} — Git", root.file_name().map_or_else(|| root.display().to_string(), |name| name.to_string_lossy().into_owned())),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds { origin: point(px(180.), px(90.)), size: size(px(1100.), px(720.)) })),
        window_min_size: Some(size(px(640.), px(420.))),
        titlebar: Some(gpui_kit::TitlebarOptions { title: Some(title.into()), ..Default::default() }),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, move |window, cx| cx.new(|cx| GitWindow::new(window, cx, root, page, appearance)))
        .ok()
        .map(|(_, view)| view)
}

#[cfg(test)]
impl GitWindow {
    pub(crate) fn status_for_test(&self) -> Option<files_ext_git::Status> {
        self.status.clone()
    }

    pub(crate) fn ticked_for_test(&self) -> Vec<String> {
        self.ticked.iter().cloned().collect()
    }

    pub(crate) fn commits_for_test(&self) -> Vec<Commit> {
        self.commits.clone()
    }

    pub(crate) fn set_message_for_test(&mut self, message: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.message.update(cx, |input, cx| input.set_value(message.to_string(), window, cx));
    }

    pub(crate) fn commit_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit(false, window, cx);
    }

    pub(crate) fn busy_for_test(&self) -> bool {
        self.busy
    }

    pub(crate) fn diff_for_test(&self) -> Option<(String, String)> {
        self.diff.clone()
    }

    pub(crate) fn show_diff_for_test(&mut self, path: &str, cx: &mut Context<Self>) {
        self.show_worktree_diff(path.to_string(), cx);
    }

    pub(crate) fn set_page_for_test(&mut self, page: GitPage, cx: &mut Context<Self>) {
        self.set_page(page, cx);
    }
}
