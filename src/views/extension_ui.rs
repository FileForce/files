//! Where extensions show up in the UI: their context-menu items (for files,
//! folders or the empty area), their sidebar sections, and the app events
//! they receive (window active/inactive, wake from sleep).

use files_sdk::{Context as ExtContext, Event, Extension, Manifest, MenuItem, Outcome, Permission, Scope, Targets};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::{
    AnyElement, App, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::app::FileExplorer;
use crate::model::{FileEntry, Location};
use crate::palette::Palette;

/// Every extension with code behind its manifest.
pub(crate) fn registry() -> Vec<&'static dyn Extension> {
    #[allow(unused_mut)]
    let mut all: Vec<&'static dyn Extension> =
        vec![
            &files_ext_archive::EXTENSION,
            &files_ext_ssh::EXTENSION,
            &files_ext_forks::EXTENSION,
            &files_ext_agents::EXTENSION,
            &files_ext_git::EXTENSION,
            &files_ext_mail::EXTENSION,
        ];
    // Signed, licensed plugins installed outside the app.
    all.extend(crate::plugins::registry());
    #[cfg(test)]
    all.extend(test_support::extra());
    all
}

/// Whether the user has the extension turned on in Settings.
fn is_enabled(manifest: &Manifest, cx: &App, explorer: &FileExplorer) -> bool {
    let settings = explorer.appearance.read(cx);
    match crate::extensions::CATALOG.iter().position(|known| known.id == manifest.id) {
        Some(index) => crate::licensing::active(settings, index),
        // Not in the catalog (tests): on, unless it is Pro without a license.
        None => crate::licensing::unlocked(manifest, settings),
    }
}

fn icon_for(manifest: &Manifest, icon: Option<files_sdk::Icon>) -> Icon {
    match icon {
        Some(icon) => crate::extensions::icon_of(&Manifest { icon, ..*manifest }),
        None => crate::extensions::icon_of(manifest),
    }
}

impl FileExplorer {
    /// Context-menu items extensions add for `target`, in the current place.
    pub(crate) fn extension_menu_items(&self, target: Targets, location: Location, cx: &App) -> Vec<(&'static dyn Extension, MenuItem)> {
        let remote = location.is_remote();
        registry()
            .into_iter()
            .filter(|extension| is_enabled(extension.manifest(), cx, self))
            .flat_map(|extension| extension.manifest().menu.iter().map(move |item| (extension, *item)))
            .filter(|(_, item)| item.targets.contains(target))
            .filter(|(_, item)| match item.scope {
                Scope::Anywhere => true,
                Scope::Remote => remote,
                Scope::Local => !remote,
            })
            .collect()
    }

    fn extension_context(&self, entry: Option<&'static FileEntry>, location: Location) -> ExtContext {
        let path_of = |entry: &FileEntry| {
            crate::model::remote::entry_path(entry)
                .map(str::to_string)
                .or_else(|| entry.path.map(|path| path.display().to_string()))
        };
        let selection = match entry {
            Some(entry) => {
                let selected = self.selected_entries();
                let items = if selected.iter().any(|item| item.id == entry.id) { selected } else { vec![entry] };
                items.into_iter().filter_map(path_of).collect()
            }
            None => Vec::new(),
        };
        ExtContext {
            selection,
            folder: location
                .remote_path()
                .map(str::to_string)
                .or_else(|| location.path().map(|path| path.display().to_string())),
            remote: location.is_remote(),
            sidebar_item: None,
            server: self.ssh_connection.as_ref().filter(|_| location.is_remote()).map(|connection| connection.address()),
        }
    }

    pub(crate) fn run_extension_command(
        &mut self,
        extension: &'static dyn Extension,
        command: &'static str,
        context: ExtContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match extension.run(command, &context) {
            Ok(outcome) => self.apply_outcome(extension.manifest(), outcome, window, cx),
            Err(error) => self.notice = Some(format!("{} · {error}", extension.manifest().name)),
        }
        cx.notify();
    }

    fn apply_outcome(&mut self, manifest: &Manifest, outcome: Outcome, window: &mut Window, cx: &mut Context<Self>) {
        self.notice = outcome.notice;
        if let Some(text) = outcome.clipboard {
            match files_sdk::permissions::check(manifest.id, Permission::Clipboard) {
                Ok(()) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
                Err(error) => self.notice = Some(error.to_string()),
            }
        }
        if let Some(folder) = outcome.navigate {
            self.navigate(Location::from_path(&folder), window, cx);
        }
    }

    /// Sends an app event to every enabled extension that may receive it:
    /// going to the background needs [`Permission::Background`].
    pub(crate) fn broadcast_extension_event(&self, event: Event, cx: &App) {
        for extension in registry() {
            let manifest = extension.manifest();
            if !is_enabled(manifest, cx, self) {
                continue;
            }
            if event == Event::WindowActive(false) && !files_sdk::permissions::granted(manifest.id, Permission::Background) {
                continue;
            }
            extension.on_event(event);
        }
    }

    /// Watches the window and the clock: the window going to the background,
    /// and the computer waking up (wall time jumps ahead of the monotonic
    /// clock, which stops while asleep).
    pub(crate) fn watch_extension_events(window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::Subscription {
        cx.spawn(async move |this, cx| {
            const TICK: std::time::Duration = std::time::Duration::from_secs(5);
            const SAVING_TICK: std::time::Duration = std::time::Duration::from_secs(30);
            loop {
                let wall = std::time::SystemTime::now();
                let monotonic = std::time::Instant::now();
                cx.background_executor().timer(crate::power::interval(TICK, SAVING_TICK)).await;
                let slept = wall.elapsed().unwrap_or_default().saturating_sub(monotonic.elapsed());
                if slept > std::time::Duration::from_secs(30) {
                    let alive = this.update(cx, |this, cx| this.woke_up(cx));
                    if alive.is_err() {
                        break;
                    }
                }
                if this.upgrade().is_none() {
                    break;
                }
            }
        })
        .detach();
        cx.observe_window_activation(window, |this, window, cx| {
            this.broadcast_extension_event(Event::WindowActive(window.is_window_active()), cx);
        })
    }

    fn woke_up(&mut self, cx: &mut Context<Self>) {
        self.broadcast_extension_event(Event::Woke, cx);
        // The SSH master connection rarely survives sleep: refetch what is shown.
        let location = self.active_location();
        if location.is_remote() {
            self.load_remote(location, true, cx);
        }
    }

    /// Sidebar sections extensions add.
    pub(crate) fn render_extension_sections(&self, palette: Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut sections = Vec::new();
        for extension in registry() {
            let manifest = extension.manifest();
            if !is_enabled(manifest, cx, self) {
                continue;
            }
            for section in manifest.sidebar {
                let mut column = v_flex()
                    .id(gpui_kit::ElementId::Name(format!("ext-section-{}", section.id).into()))
                    .gap_1()
                    .aria_label(section.accessibility.label.unwrap_or(section.title))
                    .child(self.section_header(section.id, section.title, palette, cx));
                if let Some(hint) = section.accessibility.hint {
                    column = column.aria_description(hint);
                }
                if !self.section_collapsed(section.id, cx) {
                    for (index, item) in extension.sidebar_items(section.id).into_iter().enumerate() {
                        column = column.child(self.extension_sidebar_row(extension, section.id, index, item, palette, cx));
                    }
                }
                sections.push(column.into_any_element());
            }
        }
        sections
    }

    fn extension_sidebar_row(
        &self,
        extension: &'static dyn Extension,
        section: &'static str,
        index: usize,
        item: files_sdk::SidebarItem,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let manifest = extension.manifest();
        let command = item.command;
        let item_id = item.id.clone();
        let row = h_flex()
            .id(gpui_kit::ElementId::Name(format!("ext-{section}-{index}").into()))
            .h(px(31.))
            .px_2()
            .items_center()
            .gap_2()
            .rounded_md()
            .text_sm()
            .text_color(palette.muted)
            .hover(|style| style.bg(palette.hover).text_color(palette.text))
            .aria_label(item.accessibility_label.clone().unwrap_or_else(|| item.title.clone()))
            .aria_description(format!("{} extension", manifest.name))
            .on_click(cx.listener(move |this, _, window, cx| {
                let location = this.active_location();
                let mut context = this.extension_context(None, location);
                context.sidebar_item = Some(item_id.clone());
                this.run_extension_command(extension, command, context, window, cx);
            }))
            .child(icon_for(manifest, item.icon).small())
            .child(div().flex_1().min_w_0().truncate().child(item.title))
            .children(item.badge.map(|badge| div().text_xs().text_color(palette.faint).child(badge)));
        #[cfg(test)]
        let row = row.debug_selector(move || format!("ext-{section}-{index}"));
        row.into_any_element()
    }
}

/// Adds the extensions' items for `target` to a context menu.
pub(crate) fn add_extension_items(
    mut menu: PopupMenu,
    view: &Entity<FileExplorer>,
    target: Targets,
    entry: Option<&'static FileEntry>,
    location: Location,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let items = view.read(cx).extension_menu_items(target, location, cx);
    if items.is_empty() {
        return menu;
    }
    menu = menu.separator();
    for (extension, item) in items {
        let manifest = extension.manifest();
        let color = crate::menu_colors::color_for(view.read(cx).appearance.read(cx), manifest.id);
        menu = menu.item(
            crate::views::context_menu::tinted_item(item.title, icon_for(manifest, item.icon), color)
                .on_click(window.listener_for(view, move |this, _, window, cx| {
                    let context = this.extension_context(entry, location);
                    this.run_extension_command(extension, item.command, context, window, cx);
                })),
        );
    }
    menu
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::RefCell;

    use files_sdk::Extension;

    thread_local! {
        static EXTRA: RefCell<Vec<&'static dyn Extension>> = const { RefCell::new(Vec::new()) };
    }

    /// Registers an extension for this test thread only.
    pub fn register(extension: &'static dyn Extension) {
        files_sdk::permissions::declare(extension.manifest());
        EXTRA.with(|extra| extra.borrow_mut().push(extension));
    }

    pub fn extra() -> Vec<&'static dyn Extension> {
        EXTRA.with(|extra| extra.borrow().clone())
    }
}
