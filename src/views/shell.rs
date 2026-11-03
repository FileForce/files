use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Icon, Selectable as _, Sizable as _, TitleBar};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{MouseButton, 
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::app::FileExplorer;
use crate::model::{Location, ViewMode};
use crate::palette::Palette;

impl FileExplorer {
    pub(crate) fn render_title_bar(
        &mut self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {

        TitleBar::new()
            .h(px(46.))
            .bg(palette.sidebar)
            .border_b_1()
            .border_color(palette.border)
            .child(
                h_flex()
                    .size_full()
                    .min_w_0()
                    .items_center()
                    .gap_4()
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .size(px(23.))
                                    .rounded_md()
                                    .bg(palette.accent)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Icon::new(IconName::FolderOpen)
                                            .xsmall()
                                            .text_color(palette.accent_contrast),
                                    ),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("file"),
                            )
                            .child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(palette.accent_soft)
                                    .text_color(palette.accent)
                                    .text_size(px(9.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("FORCE"),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_0p5()
                            // Windows treats the title bar as the window's caption:
                            // without this, clicks here would move the window instead.
                            .occlude()
                            .child(
                                Button::new("history-back")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ArrowLeft)
                                    .disabled(
                                        self.mode == ViewMode::Dual || self.history_index == 0,
                                    )
                                    .tooltip("Back")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.go_back(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("history-forward")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ArrowRight)
                                    .disabled(
                                        self.mode == ViewMode::Dual
                                            || self.history_index + 1 >= self.history.len(),
                                    )
                                    .tooltip("Forward")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.go_forward(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("location-up")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ArrowUp)
                                    .tooltip("Parent folder")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.go_up(window, cx);
                                    })),
                            ),
                    )
                    .child(
                        div().flex_1().min_w(px(160.)).max_w(px(500.)).occlude().child(
                            Input::new(&self.search)
                                .small()
                                .cleanable(true)
                                .prefix(
                                    Icon::new(IconName::Search)
                                        .small()
                                        .text_color(palette.muted),
                                )
                                .suffix(div().text_xs().text_color(palette.faint).child(format!(
                                    "Nucleo · {}",
                                    if cfg!(target_os = "macos") {
                                        "⌘F"
                                    } else {
                                        "Ctrl F"
                                    }
                                ))),
                        ),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_1()
                            .occlude()
                            .debug_selector(|| "title-bar-tools".into())
                            .child(
                                Button::new("global-search")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Search)
                                    .tooltip(if cfg!(target_os = "macos") {
                                        "Global search · ⌘K"
                                    } else {
                                        "Global search · Ctrl+K"
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_global_search(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("new-window")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::AppWindow)
                                    .tooltip(if cfg!(target_os = "macos") {
                                        "New window · ⇧⌘N"
                                    } else {
                                        "New window · Ctrl+Shift+N"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_new_window(cx);
                                    })),
                            )
                            .child(div().debug_selector(|| "toggle-inspector".into()).child(
                                Button::new("toggle-inspector")
                                    .ghost()
                                    .xsmall()
                                    .icon(if self.show_inspector {
                                        IconName::PanelRightClose
                                    } else {
                                        IconName::PanelRightOpen
                                    })
                                    .toggled(self.show_inspector && self.mode != ViewMode::Dual)
                                    .disabled(self.mode == ViewMode::Dual)
                                    .tooltip("Toggle inspector")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_inspector = !this.show_inspector;
                                        files_sdk::log::record(
                                            "files",
                                            files_sdk::log::Kind::Setting,
                                            format!("inspector: {}", if this.show_inspector { "shown" } else { "hidden" }),
                                            None,
                                        );
                                        cx.notify();
                                    })),
                            )),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn render_sidebar(
        &mut self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let settings = self.appearance.read(cx).clone();
        let active = self.active_location();
        let favorites = [
            (Location::Recents, IconName::Clock3),
            (Location::Home, IconName::House),
            (Location::Documents, IconName::FileText),
            (Location::Pictures, IconName::Images),
            (Location::Downloads, IconName::Download),
            (Location::Trash, IconName::Trash),
        ];
        let mut favorite_items = v_flex().gap_0p5();
        for (location, icon) in favorites {
            favorite_items =
                favorite_items.child(self.sidebar_item(location, icon, active, palette, cx));
        }

        v_flex()
            .size_full()
            .min_h_0()
            .bg(palette.sidebar)
            .border_r_1()
            .border_color(palette.border)
            .p_3()
            .gap_4()
            .child(
                h_flex()
                    .p_2()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .size(px(30.))
                            .rounded_full()
                            .bg(palette.accent_soft)
                            .text_color(palette.accent)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(crate::model::user_initial()),
                    )
                    .child(
                        v_flex()
                            .gap_0p5()
                            .min_w_0()
                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).truncate().child(crate::model::user_name()))
                            .child(div().text_xs().text_color(palette.muted).child(
                                crate::settings::localized(settings.language, "Local workspace", "Локальная рабочая область"),
                            )),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("settings")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Settings)
                            .tooltip(if cfg!(target_os = "macos") {
                                "Settings · ⌘,"
                            } else {
                                "Settings · Ctrl+,"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                crate::settings_window::open_settings_dialog(
                                    this.appearance.clone(),
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .child(
                // The sections scroll; the disk card below stays pinned to the
                // bottom, so long lists never cover it.
                div()
                    .id("sidebar-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.sidebar_scroll)
                    .child(
                v_flex()
                    .pr_1()
                    .pb_3()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(self.section_header(
                                "favorites",
                                crate::settings::localized(
                                    settings.language,
                                    "FAVORITES",
                                    "ИЗБРАННОЕ",
                                ),
                                palette,
                                cx,
                            ))
                            .when(!self.section_collapsed("favorites", cx), |this| {
                                this.child(favorite_items).children(
                                    settings
                                        .sidebar
                                        .favorites
                                        .clone()
                                        .into_iter()
                                        .map(|path| self.custom_favorite_item(path, active, palette, cx))
                                        .collect::<Vec<_>>(),
                                )
                            }),
                    )
                    .children(self.render_agents_section(palette, cx))
                    .children(self.render_forks(active, palette, cx))
                    .children(self.render_finder_tags(active, palette, cx))
                    .children(self.render_mail_section(palette, cx))
                    .children(self.render_music_section(palette, cx))
                    .children(self.render_extension_sections(palette, cx))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(self.section_header(
                                "connected",
                                crate::settings::localized(
                                    settings.language,
                                    "CONNECTED",
                                    "ПОДКЛЮЧЕНИЯ",
                                ),
                                palette,
                                cx,
                            ))
                            .when(!self.section_collapsed("connected", cx), |this| this
                            .children(self.volume_items(active, palette, cx))
                            .when(settings.extensions.ssh, |this| {
                                this.child(self.ssh_item(palette, cx))
                                    .children(self.saved_ssh_items(palette, cx))
                            })),
                    ),
                    )
                    // After the content, so it paints on top of the rows.
                    .vertical_scrollbar(&self.sidebar_scroll),
            )
            .children(self.render_disk_usage(palette, cx).map(|card| {
                let card = div().flex_shrink_0().pt_2().child(card);
                #[cfg(test)]
                let card = card.debug_selector(|| "sidebar-disk".into());
                card
            }))
            .into_any_element()
    }

    /// The volume of the active folder (home for Recents and Remote).
    /// Which disk the carousel shows: the one picked, or else the one
    /// holding the active folder.
    fn disk_carousel_index(&self, disks: &[(std::path::PathBuf, crate::model::DiskUsage)]) -> usize {
        if let Some(index) = self.disk_index.filter(|index| *index < disks.len()) {
            return index;
        }
        let path = self.active_location().path().or_else(|| Location::Home.path()).unwrap_or_default();
        disks
            .iter()
            .enumerate()
            .filter(|(_, (mount, _))| path.starts_with(mount))
            .max_by_key(|(_, (mount, _))| mount.components().count())
            .map_or(0, |(index, _)| index)
    }

    /// Shows the next (`1`) or previous (`-1`) disk, wrapping around.
    pub(crate) fn step_disk(&mut self, delta: isize, cx: &mut Context<Self>) {
        let disks = crate::model::user_disks();
        if disks.len() < 2 {
            return;
        }
        let current = self.disk_carousel_index(&disks) as isize;
        self.disk_index = Some((current + delta).rem_euclid(disks.len() as isize) as usize);
        self.disk_direction = delta.signum() as f32;
        cx.notify();
    }

    /// The disks at the bottom of the sidebar, one card at a time: arrows,
    /// dots and horizontal swipes move between them; a click opens the disk.
    fn render_disk_usage(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let disks = crate::model::user_disks();
        if disks.is_empty() {
            return None;
        }
        let index = self.disk_carousel_index(&disks);
        let (mount, disk) = disks[index].clone();
        let many = disks.len() > 1;
        let human = self.appearance.read(cx).human_sizes;
        let fraction = if disk.total > 0 { disk.used() as f32 / disk.total as f32 } else { 0. };
        let format = |bytes| crate::settings::format_size(bytes, human);
        let location = Location::from_path(&mount);
        let arrow = |id: &'static str, icon: IconName, delta: isize, cx: &mut Context<Self>| {
            Button::new(id)
                .ghost()
                .xsmall()
                .icon(icon)
                .tooltip(if delta < 0 { "Previous disk" } else { "Next disk" })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.step_disk(delta, cx);
                }))
        };
        let body = v_flex()
            .gap_2()
            .child(
                div().w_full().h(px(3.)).rounded_full().bg(palette.border).child(
                    div()
                        .w(gpui_kit::relative(fraction.clamp(0., 1.)))
                        .h_full()
                        .rounded_full()
                        // Nearly full disks are flagged like Finder does.
                        .bg(if fraction > 0.9 { palette.video } else { palette.accent }),
                ),
            )
            .child(div().text_size(px(10.)).text_color(palette.muted).child(format!(
                "{} of {} used · {} free",
                format(disk.used()),
                format(disk.total),
                format(disk.available)
            )));
        // The new disk slides in from the side it was swiped from.
        let direction = self.disk_direction;
        let animated = gpui_kit::AnimationExt::with_animation(
            v_flex()
                .gap_3()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(Icon::new(IconName::HardDrive).small().text_color(palette.muted))
                        .child(div().flex_1().min_w_0().truncate().text_xs().font_weight(FontWeight::MEDIUM).child(disk.name.clone())),
                )
                .child(body),
            ("disk-slide", index),
            gpui_kit::Animation::new(std::time::Duration::from_millis(220)).with_easing(gpui_kit::ease_out_quint()),
            move |card, delta| card.ml(px((1. - delta) * 24. * direction)).opacity(0.3 + 0.7 * delta),
        );
        let card = v_flex()
            .id("disk-carousel")
            .debug_selector(|| "disk-carousel".into())
            .p_3()
            .gap_2()
            .rounded_lg()
            .bg(palette.surface)
            .border_1()
            .border_color(palette.border)
            .cursor_pointer()
            .overflow_hidden()
            .aria_label(format!("Disk {} of {}: {}", index + 1, disks.len(), disk.name))
            .on_mouse_up(MouseButton::Middle, cx.listener(move |this, _, window, cx| this.open_in_new_tab(location, window, cx)))
            .on_click(cx.listener(move |this, _, window, cx| this.navigate(location, window, cx)))
            // Two-finger swipes (or shift + wheel): one disk per ~60 px.
            .on_scroll(cx.listener(|this, event: &gpui_kit::ScrollEvent, _, cx| {
                let delta = event.delta.pixel_delta(px(16.));
                if delta.x.abs() <= delta.y.abs() {
                    return;
                }
                this.disk_swipe += f32::from(delta.x);
                if this.disk_swipe.abs() >= 60. {
                    let step = if this.disk_swipe < 0. { 1 } else { -1 };
                    this.disk_swipe = 0.;
                    this.step_disk(step, cx);
                }
            }))
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .when(many, |row| row.child(arrow("disk-previous", IconName::ChevronLeft, -1, cx)))
                    .child(div().flex_1().min_w_0().child(animated))
                    .when(many, |row| row.child(arrow("disk-next", IconName::ChevronRight, 1, cx))),
            )
            .when(many, |card| {
                card.child(h_flex().justify_center().gap_1p5().children((0..disks.len()).map(|dot| {
                    let dot_view = div()
                        .id(("disk-dot", dot))
                        .size(px(if dot == index { 7. } else { 5. }))
                        .rounded_full()
                        .bg(if dot == index { palette.accent } else { palette.border })
                        .cursor_pointer()
                        .hover(|style| style.bg(palette.muted))
                        .aria_label(format!("Disk {}", dot + 1))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.disk_direction = if dot > this.disk_index.unwrap_or(0) { 1. } else { -1. };
                            this.disk_index = Some(dot);
                            cx.notify();
                        }));
                    #[cfg(test)]
                    let dot_view = dot_view.debug_selector(move || format!("disk-dot-{dot}"));
                    dot_view
                })))
            });
        Some(card.into_any_element())
    }

    /// Only the exact folder is highlighted: being inside Documents/git must
    /// not light up Documents as well.
    pub(crate) fn sidebar_item_selected(&self, location: Location) -> bool {
        self.active_location() == location
    }

    /// A foldable section title: click to collapse or expand (remembered).
    pub(crate) fn section_header(
        &self,
        key: &'static str,
        label: &'static str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let collapsed = self.section_collapsed(key, cx);
        h_flex()
            .id(gpui_kit::ElementId::Name(format!("sidebar-section-{key}").into()))
            .px_2()
            .py_0p5()
            .gap_1()
            .items_center()
            .rounded_sm()
            .cursor_pointer()
            .text_size(px(10.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(palette.faint)
            .hover(|style| style.text_color(palette.muted))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_section(key, cx)))
            .child(div().flex_1().child(label))
            .child(
                Icon::new(if collapsed { IconName::ChevronRight } else { IconName::ChevronDown })
                    .xsmall()
                    .text_color(palette.faint),
            )
            .into_any_element()
    }

    /// A folder the user added to Favorites; right-click removes it.
    fn custom_favorite_item(
        &self,
        path: std::path::PathBuf,
        active: Location,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let location = Location::from_path(&path);
        let selected = active == location;
        let missing = !path.is_dir();
        let view = cx.entity();
        let remove = path.clone();
        let item = div();
        #[cfg(test)]
        let item = {
            let probe = format!("favorite-probe-{}", path.display());
            item.debug_selector(move || probe.clone())
        };
        item
            .id(gpui_kit::ElementId::Name(format!("favorite-{}", path.display()).into()))
            .h(px(31.))
            .px_2()
            .rounded_md()
            .flex()
            .items_center()
            .gap_2()
            .text_sm()
            .text_color(if selected { palette.text } else { palette.muted })
            .bg(if selected { palette.selection } else { palette.sidebar })
            .when(missing, |this| this.opacity(0.5))
            .hover(|style| style.bg(palette.hover).text_color(palette.text))
            .on_mouse_up(MouseButton::Middle, cx.listener(move |this, _, window, cx| this.open_in_new_tab(location, window, cx)))
            .on_click(cx.listener(move |this, _, window, cx| {
                if !missing {
                    this.navigate(location, window, cx);
                }
            }))
            .child(Icon::new(IconName::Folder).small().text_color(if selected { palette.accent } else { palette.folder }))
            .child(div().flex_1().min_w_0().truncate().child(location.label()))
            .context_menu(move |menu, window, _| {
                let remove = remove.clone();
                menu.item(
                    PopupMenuItem::new("Remove from Favorites")
                        .icon(IconName::X)
                        .on_click(window.listener_for(&view, move |this, _, _, cx| {
                            this.remove_favorite(&remove, cx);
                        })),
                )
            })
            .into_any_element()
    }

    /// Mounted disks and network shares (USB drives, SMB/NFS, disk images).
    fn volume_items(&self, active: Location, palette: Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        crate::model::mounted_volumes()
            .into_iter()
            .enumerate()
            .map(|(index, path)| {
                let location = Location::from_path(&path);
                let selected = active == location;
                let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
                let (eject_path, menu_path) = (path.clone(), path.clone());
                let view = cx.entity();
                let group = gpui_kit::SharedString::from(format!("volume-{index}"));
                h_flex()
                    .id(("volume", index))
                    .group(group.clone())
                    .h(px(31.))
                    .px_2()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .text_sm()
                    .bg(if selected { palette.selection } else { palette.sidebar })
                    .text_color(if selected { palette.text } else { palette.muted })
                    .hover(|style| style.bg(palette.hover).text_color(palette.text))
                    .on_mouse_up(MouseButton::Middle, cx.listener(move |this, _, window, cx| this.open_in_new_tab(location, window, cx)))
                    .on_click(cx.listener(move |this, _, window, cx| this.navigate(location, window, cx)))
                    .child(Icon::new(IconName::HardDrive).small())
                    .child(div().flex_1().min_w_0().truncate().child(name))
                    // ⏏ on hover, like Finder.
                    .child(
                        div().invisible().group_hover(group, |style| style.visible()).child(
                            Button::new(("volume-eject", index)).ghost().xsmall().icon(IconName::Eject).tooltip("Eject").on_click(cx.listener(
                                move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.eject_volume(eject_path.clone(), window, cx);
                                },
                            )),
                        ),
                    )
                    .context_menu(move |menu, window, _| {
                        let (open, tab, copy, info, eject) = (menu_path.clone(), menu_path.clone(), menu_path.clone(), menu_path.clone(), menu_path.clone());
                        menu.item(PopupMenuItem::new("Open").icon(IconName::FolderOpen).on_click(window.listener_for(&view, move |this, _, window, cx| {
                            this.navigate(Location::from_path(&open), window, cx);
                        })))
                        .item(PopupMenuItem::new("Open in New Tab").icon(IconName::PanelTop).on_click(window.listener_for(&view, move |this, _, window, cx| {
                            this.new_tab(window, cx);
                            this.navigate(Location::from_path(&tab), window, cx);
                        })))
                        .item(PopupMenuItem::new("Copy Path").icon(IconName::Copy).on_click(move |_, _, cx| {
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(copy.display().to_string()));
                        }))
                        .item(PopupMenuItem::new("Properties").icon(IconName::Info).on_click(window.listener_for(&view, move |this, _, _, cx| {
                            let location = Location::from_path(&info);
                            this.run_folder_command(crate::commands::FolderCommand::Properties, location, cx);
                        })))
                        .separator()
                        .item(PopupMenuItem::new("Eject").icon(IconName::Eject).on_click(window.listener_for(&view, move |this, _, window, cx| {
                            this.eject_volume(eject.clone(), window, cx);
                        })))
                    })
                    .into_any_element()
            })
            .collect()
    }

    /// Ejects in the background; leaves the disk first if it is open.
    pub(crate) fn eject_volume(&mut self, path: std::path::PathBuf, window: &mut gpui_kit::Window, cx: &mut Context<Self>) {
        if self.active_location().path().is_some_and(|open| open.starts_with(&path)) {
            self.navigate(Location::Home, window, cx);
        }
        let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
        self.notice = Some(format!("Ejecting {name}…"));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let ejected = cx.background_executor().spawn(async move { crate::model::eject(&path) }).await;
            let _ = this.update(cx, |this, cx| {
                this.notice = Some(match ejected {
                    Ok(()) => format!("{name} ejected: safe to remove"),
                    Err(error) => {
                        files_sdk::log::notify("files", files_sdk::log::Level::Error, format!("{name} was not ejected"), Some(error.to_string()));
                        format!("{name} was not ejected: {error}")
                    }
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// Servers connected before: one click reconnects.
    fn saved_ssh_items(&self, palette: Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let current = self.ssh_connection.as_ref().map(|connection| connection.address());
        let saved = self.appearance.read(cx).sidebar.ssh.clone();
        saved
            .into_iter()
            .filter(|address| Some(address) != current.as_ref())
            .enumerate()
            .map(|(index, address)| {
                let connect = address.clone();
                let forget = address.clone();
                let menu_address = address.clone();
                h_flex()
                    .id(("saved-ssh", index))
                    .h(px(31.))
                    .px_2()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .text_sm()
                    .text_color(palette.muted)
                    .hover(|style| style.bg(palette.hover).text_color(palette.text))
                    .on_click(cx.listener(move |this, _, window, cx| this.connect_saved_ssh(&connect, window, cx)))
                    .child(Icon::new(IconName::Cloud).small().text_color(palette.faint))
                    .child(div().flex_1().min_w_0().truncate().child(address))
                    .child(
                        Button::new(("forget-ssh", index))
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .tooltip("Forget this server")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.forget_saved_ssh(&forget, cx);
                            })),
                    )
                    .context_menu({
                        let (view, address) = (cx.entity(), menu_address);
                        move |menu, window, _| {
                            let (connect, copy, forget) = (address.clone(), address.clone(), address.clone());
                            menu.item(PopupMenuItem::new("Connect").icon(IconName::Plug).on_click(window.listener_for(&view, move |this, _, window, cx| {
                                this.connect_saved_ssh(&connect, window, cx);
                            })))
                            .item(PopupMenuItem::new("Copy Address").icon(IconName::Copy).on_click(move |_, _, cx| {
                                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(copy.clone()));
                            }))
                            .separator()
                            .item(PopupMenuItem::new("Forget Server").icon(IconName::Trash).on_click(window.listener_for(&view, move |this, _, _, cx| {
                                this.forget_saved_ssh(&forget, cx);
                            })))
                        }
                    })
                    .into_any_element()
            })
            .collect()
    }

    fn ssh_item(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let active = self.active_location().is_remote();
        if let Some(connection) = &self.ssh_connection {
            let address = connection.address();
            let home = self.ssh_home;
            h_flex()
                .id("ssh-connection")
                .min_h(px(40.))
                .px_2()
                .items_center()
                .gap_2()
                .rounded_md()
                .bg(if active {
                    palette.selection
                } else {
                    palette.sidebar
                })
                .text_color(if active { palette.text } else { palette.muted })
                .hover(|style| style.bg(palette.hover))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.navigate(home, window, cx);
                }))
                .child(
                    Icon::new(IconName::Cloud)
                        .small()
                        .text_color(palette.accent),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .child(div().text_xs().child(address.clone()))
                        .child(
                            div()
                                .text_size(px(9.))
                                .text_color(palette.faint)
                                .child("Connected · SSH"),
                        ),
                )
                .child(div().flex_1())
                .child(
                    Button::new("disconnect-ssh")
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .tooltip("Disconnect")
                        .on_click(cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.disconnect_ssh(window, cx);
                        })),
                )
                .context_menu({
                    let (view, copy) = (cx.entity(), address.clone());
                    move |menu, window, _| {
                        let copy = copy.clone();
                        menu.item(PopupMenuItem::new("Open Home Folder").icon(IconName::House).on_click(window.listener_for(&view, move |this, _, window, cx| {
                            this.navigate(home, window, cx);
                        })))
                        .item(PopupMenuItem::new("Open in Terminal").icon(IconName::SquareTerminal).on_click(window.listener_for(&view, move |this, _, _, cx| {
                            this.run_folder_command(crate::commands::FolderCommand::OpenTerminal, home, cx);
                        })))
                        .item(PopupMenuItem::new("Copy Address").icon(IconName::Copy).on_click(move |_, _, cx| {
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(copy.clone()));
                        }))
                        .separator()
                        .item(PopupMenuItem::new("Disconnect").icon(IconName::Unplug).on_click(window.listener_for(&view, |this, _, window, cx| {
                            this.disconnect_ssh(window, cx);
                        })))
                    }
                })
                .into_any_element()
        } else {
            h_flex()
                .id("ssh-connection")
                .h(px(31.))
                .px_2()
                .items_center()
                .gap_2()
                .rounded_md()
                .text_sm()
                .text_color(palette.muted)
                .hover(|style| style.bg(palette.hover).text_color(palette.text))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_ssh_dialog(window, cx);
                }))
                .child(Icon::new(IconName::Cloud).small())
                .child("SSH")
                .child(div().flex_1())
                .child(div().text_xs().text_color(palette.faint).child("New…"))
                .into_any_element()
        }
    }

    fn sidebar_item(
        &self,
        location: Location,
        icon: IconName,
        active: Location,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _ = active;
        let selected = self.sidebar_item_selected(location);
        let item = div()
            .id(location.label())
            .h(px(31.))
            .px_2()
            .rounded_md()
            .flex()
            .items_center()
            .gap_2()
            .text_sm()
            .text_color(if selected {
                palette.text
            } else {
                palette.muted
            })
            .bg(if selected {
                palette.selection
            } else {
                palette.sidebar
            })
            .hover(|style| style.bg(palette.hover).text_color(palette.text))
            .on_mouse_up(MouseButton::Middle, cx.listener(move |this, _, window, cx| this.open_in_new_tab(location, window, cx)))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.navigate(location, window, cx);
            }))
            .child(Icon::new(icon).small().text_color(if selected {
                palette.accent
            } else {
                palette.muted
            }))
            .child(crate::views::text_fx::outlined(location.label(), palette))
            .child(div().flex_1())
            .when(selected, |this| {
                this.child(div().size(px(5.)).rounded_full().bg(palette.accent))
            });
        if location != Location::Trash {
            return item.into_any_element();
        }
        let view = cx.entity();
        item.context_menu(move |menu, window, _| {
                    menu.min_w(px(200.))
                        .item(
                            PopupMenuItem::new("Open")
                                .icon(IconName::FolderOpen)
                                .on_click(window.listener_for(&view, |this, _, window, cx| {
                                    this.navigate(Location::Trash, window, cx);
                                })),
                        )
                        .separator()
                        .item(
                            PopupMenuItem::new("Empty Trash…")
                                .icon(IconName::Trash)
                                .on_click(window.listener_for(&view, |this, _, window, cx| {
                                    this.open_empty_trash_dialog(window, cx);
                                })),
                        )
        })
        .into_any_element()
    }

    /// Buttons next to the folder title that only make sense for this location.
    /// The start of a contextual header (like Windows Explorer's ribbon tabs).
    fn location_actions(
        &self,
        location: Location,
        count: usize,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.mode == ViewMode::Dual {
            return None;
        }
        if let Some(fork) = location.path().and_then(|path| crate::forks::fork_of(&path)) {
            return self.fork_actions(fork, cx);
        }
        match location {
            Location::Trash => Some(
                Button::new("empty-trash")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Trash)
                    .label("Empty Trash")
                    // Without read access on macOS the count is 0 but Finder can still empty it.
                    .disabled(count == 0 && crate::model::trash_access_error().is_none())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_empty_trash_dialog(window, cx);
                    }))
                    .into_any_element(),
            ),
            _ => None,
        }
    }

    /// macOS-only: the Finder tags in use, imported through Spotlight.
    fn render_finder_tags(
        &self,
        active: Location,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !cfg!(target_os = "macos") {
            return None;
        }
        let language = self.appearance.read(cx).language;
        let tags = crate::finder_tags::all();
        let mut section = v_flex().gap_1().child(self.section_header(
            "tags",
            crate::settings::localized(language, "FINDER TAGS", "ТЕГИ FINDER"),
            palette,
            cx,
        ));
        if self.section_collapsed("tags", cx) {
            return Some(section.into_any_element());
        }
        if tags.is_empty() {
            let loading = !crate::finder_tags::is_loaded();
            let (english, russian) = if loading {
                ("Loading tags…", "Загрузка тегов…")
            } else {
                ("No tagged files found", "Файлов с тегами не найдено")
            };
            let text = crate::settings::localized(language, english, russian);
            section = section.child(div().px_2().text_xs().child(if loading {
                crate::views::text_fx::shimmer("tags-loading", text, palette.faint, palette)
            } else {
                div().text_color(palette.faint).child(text).into_any_element()
            }));
        }
        for tag in tags {
            let location = Location::tag(&tag.name);
            let selected = active == location;
            let color = tag.color.hsla();
            section = section.child(
                div()
                    .id(gpui_kit::ElementId::Name(format!("finder-tag-{}", tag.name).into()))
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
                    .on_mouse_up(MouseButton::Middle, cx.listener(move |this, _, window, cx| this.open_in_new_tab(location, window, cx)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.navigate(location, window, cx);
                    }))
                    .child(match color {
                        Some(color) => div().size(px(8.)).rounded_full().bg(color),
                        None => div()
                            .size(px(8.))
                            .rounded_full()
                            .border_1()
                            .border_color(palette.faint),
                    })
                    .child(div().flex_1().min_w_0().truncate().child(tag.name.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.faint)
                            .child(tag.count.to_string()),
                    ),
            );
        }
        Some(section.into_any_element())
    }

    pub(crate) fn render_toolbar(
        &mut self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let location = self.active_location();
        let count = if self.mode == ViewMode::Dual {
            self.entries_for(self.left.location).len() + self.entries_for(self.right.location).len()
        } else {
            self.current_entries().len()
        };
        let title = if self.mode == ViewMode::Dual {
            "Dual workspace"
        } else {
            location.label()
        };

        v_flex()
            .flex_shrink_0()
            .px_5()
            .pt_4()
            .pb_3()
            .gap_3()
            .border_b_1()
            .border_color(palette.border)
            .child(self.render_breadcrumbs(palette, cx))
            .child(
                h_flex()
                    .items_end()
                    .justify_between()
                    .gap_4()
                    .child(
                        h_flex()
                            .items_baseline()
                            .gap_3()
                            .child(
                                div()
                                    .text_size(px(24.))
                                    .line_height(px(28.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(crate::model::trim_middle(title, 40).into_owned()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(palette.muted)
                                    .child(item_count(count)),
                            )
                            .children(self.location_actions(location, count, cx)),
                    )
                    .child(
                        h_flex()
                            .p_1()
                            .gap_1()
                            .rounded_lg()
                            .bg(palette.sidebar)
                            .border_1()
                            .border_color(palette.border)
                            .child(self.mode_button(
                                ViewMode::Expressive,
                                IconName::LayoutGrid,
                                palette,
                                cx,
                            ))
                            .child(self.mode_button(ViewMode::Pro, IconName::Rows3, palette, cx))
                            .child(self.mode_button(
                                ViewMode::Dual,
                                IconName::Columns2,
                                palette,
                                cx,
                            )),
                    ),
            )
            .into_any_element()
    }

    fn render_breadcrumbs(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut breadcrumbs = h_flex().items_center().gap_1().text_xs();
        if self.mode == ViewMode::Dual {
            breadcrumbs = breadcrumbs
                .child(div().text_color(palette.muted).child("Workspace"))
                .child(
                    Icon::new(IconName::ChevronRight)
                        .xsmall()
                        .text_color(palette.faint),
                )
                .child(
                    div()
                        .text_color(palette.text)
                        .child("Two independent locations"),
                );
        } else {
            // Every crumb but the current folder opens that folder.
            let crumbs = self.current.ancestry();
            let last = crumbs.len().saturating_sub(1);
            for (index, location) in crumbs.into_iter().enumerate() {
                if index > 0 {
                    breadcrumbs = breadcrumbs.child(
                        Icon::new(IconName::ChevronRight)
                            .xsmall()
                            .text_color(palette.faint),
                    );
                }
                let label = crate::model::trim_middle(location.label(), 28).into_owned();
                let crumb = div()
                    .id(("breadcrumb", index))
                    .px_1()
                    .rounded_sm()
                    .text_color(if index == last { palette.text } else { palette.muted });
                #[cfg(test)]
                let crumb = crumb.debug_selector(move || format!("breadcrumb-{index}"));
                breadcrumbs = breadcrumbs.child(if index == last {
                    crumb.child(crate::views::text_fx::outlined(label, palette)).into_any_element()
                } else {
                    crumb
                        .cursor_pointer()
                        .hover(|style| style.bg(palette.hover).text_color(palette.text))
                        .on_mouse_up(MouseButton::Middle, cx.listener(move |this, _, window, cx| this.open_in_new_tab(location, window, cx)))
                        .on_click(cx.listener(move |this, _, window, cx| this.navigate(location, window, cx)))
                        .child(crate::views::text_fx::outlined(label, palette))
                        .into_any_element()
                });
            }
        }
        breadcrumbs.into_any_element()
    }

    fn mode_button(
        &self,
        mode: ViewMode,
        icon: IconName,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Button {
        let active = self.mode == mode;
        let button = Button::new(("mode", mode as usize))
            .small()
            .icon(Icon::new(icon).small().text_color(if active {
                palette.accent
            } else {
                palette.muted
            }))
            .label(mode.label())
            .selected(active)
            .toggled(active)
            .tooltip(format!(
                "{} view · {} {}",
                mode.label(),
                if cfg!(target_os = "macos") {
                    "⌘"
                } else {
                    "Ctrl"
                },
                mode as usize + 1
            ))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_mode(mode, cx);
            }));
        if active {
            button.secondary()
        } else {
            button.ghost()
        }
    }

    pub(crate) fn render_status_bar(
        &mut self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.journal_notice();
        let (console, bell) = self.footer_buttons(palette, cx);
        let language = self.appearance.read(cx).language;
        let selected = self.active_selection().and_then(crate::model::entry);
        let count = if self.mode == ViewMode::Dual {
            self.entries_for(self.left.location).len() + self.entries_for(self.right.location).len()
        } else {
            self.current_entries().len()
        };
        let status: SharedString = if self.drop_active {
            format!(
                "Drop to copy into {} (demo)",
                self.active_location().label()
            )
            .into()
        } else if let Some(notice) = &self.notice {
            notice.clone().into()
        } else if let Some(file) = selected {
            if language == crate::settings::Language::Russian {
                format!("Выбрано · {}", file.name).into()
            } else {
                format!("Selected · {}", file.name).into()
            }
        } else {
            crate::settings::localized(language, "Ready", "Готово").into()
        };

        h_flex()
            .h(px(27.))
            .flex_shrink_0()
            .px_3()
            .items_center()
            .gap_3()
            .border_t_1()
            .border_color(palette.border)
            .bg(palette.sidebar)
            .text_size(px(10.))
            .text_color(palette.muted)
            // The console is opt-in: Settings → Developer → Show Console.
            .when(self.appearance.read(cx).developer.show_console, |bar| bar.child(console))
            .child(div().size(px(5.)).rounded_full().bg(palette.code))
            .child(div().min_w_0().truncate().child(status))
            .child(div().text_color(palette.faint).child("•"))
            .child(item_count(count))
            .child(div().flex_1())
            .child(format!(
                "{} · {}",
                self.accent.label(),
                if self.scheme.is_dark() {
                    "Dark"
                } else {
                    "Light"
                }
            ))
            .child(div().text_color(palette.faint).child("•"))
            .child(if cfg!(target_os = "macos") {
                "⌘1–3 Views   ⌘F Search"
            } else {
                "Ctrl 1–3 Views   Ctrl F Search"
            })
            .child(bell)
            .into_any_element()
    }
}

fn item_count(count: usize) -> String {
    if count == 1 {
        "1 item".to_string()
    } else {
        format!("{count} items")
    }
}
