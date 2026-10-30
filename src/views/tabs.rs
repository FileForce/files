use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{AnyElement, Context, IntoElement, ParentElement as _, Styled as _, div, px};

use crate::app::FileExplorer;
use crate::model::Location;
use crate::palette::Palette;

impl FileExplorer {
    pub(crate) fn render_tabs(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let active_tab = self.active_tab;
        let active_location = self.active_location();
        let tab_count = self.tabs.len();
        let tabs = self.tabs.iter().enumerate().map(|(index, tab)| {
            let location = if index == active_tab {
                active_location
            } else {
                tab.location()
            };
            let active = index == active_tab;
            let close = Button::new(("close-workspace-tab", tab.id))
                .ghost()
                .xsmall()
                .icon(IconName::X)
                .tooltip("Close tab")
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_tab(index, window, cx);
                }));
            Tab::new()
                .label(crate::model::trim_middle(location.label(), 24).into_owned())
                .prefix(
                    Icon::new(location_icon(location))
                        .xsmall()
                        .text_color(if active {
                            palette.accent
                        } else {
                            palette.muted
                        }),
                )
                .when(tab_count > 1, |tab| tab.suffix(close))
        });
        let explorer = cx.entity();

        h_flex()
            .h(px(38.))
            .w_full()
            .flex_shrink_0()
            .border_b_1()
            .border_color(palette.border)
            .bg(palette.window)
            .child(
                TabBar::new("workspace-tabs")
                    .folder()
                    .menu(true)
                    .max_width(px(190.))
                    .track_scroll(&self.tab_scroll)
                    .selected_index(active_tab)
                    .on_click(move |index, window, cx| {
                        explorer.update(cx, |this, cx| {
                            this.select_tab(*index, window, cx);
                        });
                    })
                    .children(tabs)
                    .suffix(
                        h_flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new("new-workspace-tab")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Plus)
                                    .tooltip(if cfg!(target_os = "macos") {
                                        "New Home tab · ⌘T"
                                    } else {
                                        "New Home tab · Ctrl+T"
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.new_tab(window, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .text_size(px(9.))
                                    .text_color(palette.faint)
                                    .child("LOCAL WORKSPACE"),
                            ),
                    )
                    .flex_1()
                    .min_w_0()
                    .h_full(),
            )
            .into_any_element()
    }
}

fn location_icon(location: Location) -> IconName {
    match location {
        Location::Recents => IconName::Clock3,
        Location::Home => IconName::House,
        Location::Documents => IconName::FileText,
        Location::Pictures => IconName::Images,
        Location::Downloads => IconName::Download,
        Location::Remote(_) => IconName::Cloud,
        Location::Trash => IconName::Trash,
        Location::Tag(_) => IconName::Tag,
        Location::Dir(_) if location.path().and_then(|path| crate::forks::fork_of(&path)).is_some() => {
            IconName::GitFork
        }
        Location::Dir(_) => IconName::Folder,
    }
}
