use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::model::{Accent, ColorScheme};

static LOAD_ERROR: OnceLock<Mutex<Option<String>>> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub scheme: ColorScheme,
    pub accent: Accent,
    pub language: Language,
    pub extensions: ExtensionSettings,
    pub human_sizes: HumanSizeSettings,
    pub date_time: DateTimeSettings,
    pub deletion: DeletionSettings,
    pub behavior: BehaviorSettings,
    /// Window look: panel opacity and background. `[look]` in settings.toml.
    pub look: LookSettings,
    pub accessibility: AccessibilitySettings,
    /// Sidebar: folded sections and your own favorite folders. `[sidebar]`.
    pub sidebar: SidebarSettings,
    /// Per-extension app overrides used only by this app: `docx = "/Applications/LibreOffice.app"`.
    pub default_apps: std::collections::BTreeMap<String, String>,
    /// Shortcut overrides by command id (see `crate::keymap`): `"explorer.new_tab" = "cmd-y"`.
    pub keymap: std::collections::BTreeMap<String, String>,
    /// Permissions taken away from an extension, by extension id:
    /// `ssh = ["network"]` (see `files_sdk::Permission::key`).
    pub permissions: std::collections::BTreeMap<String, Vec<String>>,
    /// Context-menu color by extension id, plus `delete`: a name (`purple`,
    /// `red`…), `#rrggbb`, or `none`. Missing keys use the extension's own.
    pub menu_colors: std::collections::BTreeMap<String, String>,
    /// Settings → Developer.
    pub developer: DeveloperSettings,
    /// The license that unlocks Pro extensions.
    pub license: LicenseSettings,
    /// Update checks.
    pub updates: UpdateSettings,
    /// Reduce background work and animation, automatically or on demand.
    pub power_saving: PowerSavingMode,
    /// Automatic: power saving starts when the battery drops to this
    /// percent (100: as soon as the computer runs on battery).
    pub power_saving_below: u8,
    /// One Files at a time: opening another hands its folder to the running
    /// one (a new tab) instead of a second window.
    pub single_instance: bool,
    /// Mail: colors of the special folders (Inbox, Sent, Trash…).
    pub mail: MailSettings,
    /// Extension search filters (`git:`…) switched off per search box.
    pub search_filters: SearchFilterSettings,
    /// Notification sounds and the speaker layout.
    pub sound: SoundSettings,
    /// Settings → Storage: the cache limit.
    pub storage: StorageSettings,
    /// Notifications in the system's own center (macOS, Windows, Linux).
    pub system_notifications: SystemNotificationSettings,
}

/// `[system_notifications]` in settings.toml.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemNotificationSettings {
    pub mode: SystemNotify,
    /// The least serious kind still sent.
    pub min_level: NotifyLevel,
}

/// When a notification also goes to the system.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemNotify {
    Off,
    /// Only while no Files window is in front (the toast is seen otherwise).
    #[default]
    Background,
    Always,
}

impl SystemNotify {
    pub const ALL: [SystemNotify; 3] = [SystemNotify::Off, SystemNotify::Background, SystemNotify::Always];
}

/// Which notifications reach the system.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyLevel {
    #[default]
    All,
    Warnings,
    Errors,
}

impl NotifyLevel {
    pub const ALL: [NotifyLevel; 3] = [NotifyLevel::All, NotifyLevel::Warnings, NotifyLevel::Errors];

    pub fn allows(self, level: files_sdk::log::Level) -> bool {
        use files_sdk::log::Level;
        match self {
            NotifyLevel::All => true,
            NotifyLevel::Warnings => matches!(level, Level::Warning | Level::Error),
            NotifyLevel::Errors => level == Level::Error,
        }
    }
}

/// `[storage]` in settings.toml.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageSettings {
    /// Caches together stay under this many MB (0: no limit).
    pub limit_mb: u32,
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self { limit_mb: 1024 }
    }
}

/// `[sound]` in settings.toml.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundSettings {
    /// A chime when a notification arrives.
    pub notifications: bool,
    /// 0…100.
    pub volume: u8,
    pub chime: Chime,
    /// What the speaker test plays to.
    pub layout: SpeakerLayout,
}

impl Default for SoundSettings {
    fn default() -> Self {
        Self { notifications: true, volume: 60, chime: Chime::Soft, layout: SpeakerLayout::Stereo21 }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Chime {
    #[default]
    Soft,
    Bright,
    Low,
}

impl Chime {
    pub const ALL: [Self; 3] = [Self::Soft, Self::Bright, Self::Low];

    pub fn label(self) -> &'static str {
        match self {
            Self::Soft => "Soft",
            Self::Bright => "Bright",
            Self::Low => "Low",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerLayout {
    Headphones,
    #[default]
    Stereo21,
    Surround31,
    Surround51,
    Surround71,
}

impl SpeakerLayout {
    pub const ALL: [Self; 5] = [Self::Headphones, Self::Stereo21, Self::Surround31, Self::Surround51, Self::Surround71];

    pub fn label(self) -> &'static str {
        match self {
            Self::Headphones => "Headphones",
            Self::Stereo21 => "2.1",
            Self::Surround31 => "3.1",
            Self::Surround51 => "5.1",
            Self::Surround71 => "7.1",
        }
    }
}

/// `[search_filters]`: keys that stay plain text in each search box.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchFilterSettings {
    pub folder_off: Vec<String>,
    pub global_off: Vec<String>,
    /// Where Cmd-K searches; empty means the home folder.
    pub global_folder: Option<PathBuf>,
}

impl SearchFilterSettings {
    /// The folder Cmd-K searches by default.
    pub fn global_root(&self) -> Option<PathBuf> {
        self.global_folder.clone().filter(|folder| folder.is_dir()).or_else(dirs::home_dir)
    }
}

/// `[mail]` in settings.toml.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MailSettings {
    /// Special folder key (`inbox`, `trash`…) → menu color name (`red`…).
    pub folder_colors: std::collections::BTreeMap<String, String>,
    /// Special folders hidden from the folder list.
    pub hidden: Vec<String>,
}

/// `[updates]` in settings.toml.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateSettings {
    /// Look for a new version once a day and post a notification.
    pub automatic: bool,
    /// When the last automatic check ran (seconds since the Unix epoch).
    pub last_check: i64,
    /// The version that ran last time: a newer one means "just updated".
    pub last_version: String,
    /// "Skip this release": not offered again (a newer one is).
    pub skipped: String,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self { automatic: true, last_check: 0, last_version: String::new(), skipped: String::new() }
    }
}

/// `[license]` in settings.toml.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LicenseSettings {
    /// The key as sold (`FILES1.…`); checked offline against the built-in public keys.
    pub key: String,
}

/// `[developer]` in settings.toml.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeveloperSettings {
    /// The console shows every call Files and extensions make (programs with
    /// their arguments and output, file and network operations), not only
    /// messages and notifications; the shell's `run` works.
    pub enabled: bool,
    /// The Console button in the footer (off: only the bell).
    pub show_console: bool,
}

impl AppSettings {
    /// Settings written by older versions: TortoiseGit is now Git.
    pub fn migrate(&mut self) {
        for map in [&mut self.menu_colors] {
            if let Some(value) = map.remove("tortoise-git") {
                map.entry("git".to_string()).or_insert(value);
            }
        }
        if let Some(value) = self.permissions.remove("tortoise-git") {
            self.permissions.entry("git".to_string()).or_insert(value);
        }
    }

    pub fn revoked(&self, extension: &str, permission: files_sdk::Permission) -> bool {
        self.permissions.get(extension).is_some_and(|keys| keys.iter().any(|key| key == permission.key()))
    }

    pub fn set_revoked(&mut self, extension: &str, permission: files_sdk::Permission, revoked: bool) {
        let keys = self.permissions.entry(extension.to_string()).or_default();
        keys.retain(|key| key != permission.key());
        if revoked {
            keys.push(permission.key().to_string());
            keys.sort();
        }
        if keys.is_empty() {
            self.permissions.remove(extension);
        }
    }

    /// Hands the revocations to the SDK, which every guarded API checks.
    pub fn apply_permissions(&self) {
        let map = self
            .permissions
            .iter()
            .map(|(extension, keys)| {
                (extension.clone(), keys.iter().filter_map(|key| files_sdk::Permission::from_key(key)).collect())
            })
            .collect();
        files_sdk::permissions::set_revoked(map);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SidebarSettings {
    /// Folded sections: "favorites", "forks", "tags", "connected".
    pub collapsed: Vec<String>,
    /// Folders added to Favorites, in order.
    pub favorites: Vec<PathBuf>,
    /// SSH servers connected before, newest first, as `user@host:port`.
    pub ssh: Vec<String>,
    /// Agent sessions started from Files (all others are "external").
    pub agents: Vec<String>,
}

/// On, off, or follow the matching macOS setting.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Toggle {
    #[default]
    System,
    On,
    Off,
}

impl Toggle {
    pub fn resolve(self, system: bool) -> bool {
        match self {
            Self::System => system,
            Self::On => true,
            Self::Off => false,
        }
    }
}

/// Text size presets, shown as the steps of a slider (like macOS).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextScale {
    ExtraSmall,
    Small,
    #[default]
    Default,
    Large,
    Larger,
    ExtraLarge,
    Huge,
}

impl TextScale {
    pub const ALL: [Self; 7] = [
        Self::ExtraSmall,
        Self::Small,
        Self::Default,
        Self::Large,
        Self::Larger,
        Self::ExtraLarge,
        Self::Huge,
    ];

    pub fn factor(self) -> f32 {
        match self {
            Self::ExtraSmall => 0.85,
            Self::Small => 0.92,
            Self::Default => 1.0,
            Self::Large => 1.1,
            Self::Larger => 1.2,
            Self::ExtraLarge => 1.35,
            Self::Huge => 1.5,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ExtraSmall => "Extra small",
            Self::Small => "Small",
            Self::Default => "Default",
            Self::Large => "Large",
            Self::Larger => "Larger",
            Self::ExtraLarge => "Extra large",
            Self::Huge => "Huge",
        }
    }

    /// Position on the slider (0 = smallest).
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|scale| *scale == self).unwrap_or(2)
    }

    pub fn from_index(index: usize) -> Self {
        Self::ALL[index.min(Self::ALL.len() - 1)]
    }
}

/// `[accessibility]` in settings.toml.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccessibilitySettings {
    /// No sliding, fading or springing; changes happen at once.
    pub reduce_motion: Toggle,
    /// Stronger borders and secondary text, solid panels.
    pub high_contrast: Toggle,
    /// Opaque window even when Appearance asks for see-through panels.
    pub reduce_transparency: Toggle,
    pub text_scale: TextScale,
    /// A thin outline of the opposite color around text (dark text gets a
    /// white edge), so names stay readable over see-through panels.
    pub text_contrast: Toggle,
    /// Loading text gently pulses ("Thinking…", "Loading…").
    pub shimmer: Toggle,
}

/// The macOS Accessibility → Display switches (all off elsewhere).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SystemAccessibility {
    pub reduce_motion: bool,
    pub increase_contrast: bool,
    pub reduce_transparency: bool,
}

/// Reads the OS switches once per process (Settings re-reads on open).
pub fn system_accessibility() -> SystemAccessibility {
    let mut cache = SYSTEM_ACCESSIBILITY.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    *cache.get_or_insert_with(read_system_accessibility)
}

static SYSTEM_ACCESSIBILITY: Mutex<Option<SystemAccessibility>> = Mutex::new(None);

/// Re-reads the OS switches (they can change while the app runs).
pub fn refresh_system_accessibility() -> SystemAccessibility {
    let fresh = read_system_accessibility();
    *SYSTEM_ACCESSIBILITY.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(fresh);
    fresh
}

fn read_system_accessibility() -> SystemAccessibility {
    #[cfg(target_os = "macos")]
    {
        let read = |key: &str| {
            std::process::Command::new("/usr/bin/defaults")
                .args(["read", "com.apple.universalaccess", key])
                .output()
                .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "1")
        };
        SystemAccessibility {
            reduce_motion: read("reduceMotion"),
            increase_contrast: read("increaseContrast"),
            reduce_transparency: read("reduceTransparency"),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        SystemAccessibility::default()
    }
}

impl AccessibilitySettings {
    pub fn reduce_motion(&self) -> bool {
        self.reduce_motion.resolve(system_accessibility().reduce_motion)
    }

    pub fn high_contrast(&self) -> bool {
        self.high_contrast.resolve(system_accessibility().increase_contrast)
    }

    pub fn reduce_transparency(&self) -> bool {
        self.reduce_transparency.resolve(system_accessibility().reduce_transparency)
    }

    /// System: follows macOS Increase contrast.
    pub fn text_contrast(&self) -> bool {
        self.text_contrast.resolve(system_accessibility().increase_contrast)
    }

    /// System: on unless motion is reduced; never while Reduce motion is on.
    pub fn shimmer(&self) -> bool {
        !self.reduce_motion() && self.shimmer.resolve(true)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LookSettings {
    /// 40–100: below 100 the panels let the background (or the blurred
    /// desktop) show through.
    pub panel_opacity: u8,
    pub background: Background,
    /// How dialogs, menus, switches and sliders animate.
    pub motion: MotionStyle,
}

impl Default for LookSettings {
    fn default() -> Self {
        Self {
            panel_opacity: 100,
            background: Background::Default,
            motion: MotionStyle::Smooth,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionStyle {
    /// GPUI Kit's own short, crisp timing.
    Snappy,
    /// Longer, eased in and out: the default.
    #[default]
    Smooth,
    /// Slow and gentle.
    Relaxed,
}

impl MotionStyle {
    pub const ALL: [Self; 3] = [Self::Snappy, Self::Smooth, Self::Relaxed];

    pub fn label(self) -> &'static str {
        match self {
            Self::Snappy => "Snappy",
            Self::Smooth => "Smooth",
            Self::Relaxed => "Relaxed",
        }
    }
}

impl LookSettings {
    pub const MIN_OPACITY: u8 = 40;

    pub fn opacity(&self) -> f32 {
        self.panel_opacity.clamp(Self::MIN_OPACITY, 100) as f32 / 100.
    }
}

/// What sits behind the panels.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Background {
    /// The theme color; with opacity below 100 the desktop shows, blurred.
    #[default]
    Default,
    /// `#rrggbb`
    Color { hex: String },
    Image { path: PathBuf },
}

/// Parses `#rrggbb` (or `rrggbb`).
pub fn parse_hex(hex: &str) -> Option<gpui_kit::Hsla> {
    let digits = hex.trim().trim_start_matches('#');
    if digits.len() != 6 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok().map(|value| gpui_kit::rgb(value).into())
}

/// How the file lists react to the mouse. `[behavior]` in settings.toml.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BehaviorSettings {
    /// Clicking empty space in a list clears the selection (and the preview).
    pub clear_selection_on_empty_click: bool,
    /// The view new windows and tabs start in.
    pub default_view: crate::model::ViewMode,
    /// What dropping files from another window does.
    pub drop_action: DropAction,
    /// Ask Copy / Move on every drop.
    pub ask_on_drop: bool,
    /// Where Mail (and other extension windows) open.
    pub open_in: OpenIn,
}

/// Where an extension's window opens: on its own, or as a tab here.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenIn {
    #[default]
    Tab,
    Window,
}

impl Default for BehaviorSettings {
    fn default() -> Self {
        Self {
            clear_selection_on_empty_click: true,
            default_view: crate::model::ViewMode::Expressive,
            drop_action: DropAction::Copy,
            ask_on_drop: false,
            open_in: OpenIn::Tab,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropAction {
    /// Always copy: the original stays where it was.
    #[default]
    Copy,
    /// Always move.
    Move,
    /// Finder's rule: move on the same disk, copy to another disk.
    Finder,
}

impl DropAction {
    pub const ALL: [Self; 3] = [Self::Copy, Self::Move, Self::Finder];

    pub fn label(self) -> &'static str {
        match self {
            Self::Copy => "Copy",
            Self::Move => "Move",
            Self::Finder => "Like Finder",
        }
    }
}

/// Paths the app refuses to delete. Written to `settings.toml` as
/// `[deletion] protected = [...]`; see `crate::delete` for the pattern syntax.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeletionSettings {
    /// Move to Trash / Recycle Bin instead of deleting permanently.
    pub use_trash: bool,
    pub protected: Vec<String>,
}

impl Default for DeletionSettings {
    fn default() -> Self {
        Self {
            use_trash: true,
            protected: crate::delete::default_protected()
                .iter()
                .map(|pattern| pattern.to_string())
                .collect(),
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            scheme: ColorScheme::Dark,
            accent: Accent::Iris,
            language: Language::English,
            extensions: ExtensionSettings::default(),
            human_sizes: HumanSizeSettings::default(),
            date_time: DateTimeSettings::default(),
            deletion: DeletionSettings::default(),
            behavior: BehaviorSettings::default(),
            look: LookSettings::default(),
            accessibility: AccessibilitySettings::default(),
            sidebar: SidebarSettings::default(),
            default_apps: std::collections::BTreeMap::new(),
            keymap: std::collections::BTreeMap::new(),
            permissions: std::collections::BTreeMap::new(),
            menu_colors: std::collections::BTreeMap::new(),
            developer: DeveloperSettings::default(),
            license: LicenseSettings::default(),
            updates: UpdateSettings::default(),
            power_saving: PowerSavingMode::Automatic,
            power_saving_below: 100,
            single_instance: true,
            mail: MailSettings::default(),
            search_filters: SearchFilterSettings::default(),
            sound: SoundSettings::default(),
            storage: StorageSettings::default(),
            system_notifications: SystemNotificationSettings::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerSavingMode {
    #[default]
    Automatic,
    On,
    Off,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    English,
    Russian,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtensionSettings {
    pub open_with: bool,
    pub archives: bool,
    /// Git in the context menu (was TortoiseGit).
    #[serde(alias = "tortoise_git")]
    pub git: bool,
    pub copy: bool,
    pub ssh: bool,
    pub terminal: bool,
    /// Copy-on-write forks of folders (sidebar section, menu items, Apply).
    pub forks: bool,
    /// Claude Code, Codex and OpenCode sessions in the sidebar.
    pub agents: bool,
    /// Mail accounts in the sidebar, "Send as Attachment" on files.
    pub mail: bool,
    /// The music library, Spotify and Yandex Music.
    pub music: bool,
    /// Archives: "Create folder" is ticked when the extract page opens.
    pub archives_create_folder: bool,
    /// Archives: "Overwrite existing files without asking" starts ticked.
    pub archives_overwrite: bool,
    /// Terminal: which app "Open in Terminal" launches (macOS).
    pub terminal_app: TerminalApp,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalApp {
    /// macOS Terminal, or the system terminal on Linux.
    #[default]
    Terminal,
    Iterm,
    Warp,
    Ghostty,
    WindowsTerminal,
    PowerShell,
    Cmd,
}

impl TerminalApp {
    /// The ones this system can run.
    #[cfg(target_os = "macos")]
    const CANDIDATES: &[Self] = &[Self::Terminal, Self::Iterm, Self::Warp, Self::Ghostty];
    #[cfg(windows)]
    const CANDIDATES: &[Self] = &[Self::WindowsTerminal, Self::PowerShell, Self::Cmd];
    #[cfg(all(unix, not(target_os = "macos")))]
    const CANDIDATES: &[Self] = &[Self::Terminal, Self::Ghostty];

    /// The macOS application name passed to `open -a`, or the label.
    pub fn app_name(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal",
            Self::Iterm => "iTerm",
            Self::Warp => "Warp",
            Self::Ghostty => "Ghostty",
            Self::WindowsTerminal => "Windows Terminal",
            Self::PowerShell => "PowerShell",
            Self::Cmd => "Command Prompt",
        }
    }

    /// Terminals installed here, the built-in one first.
    pub fn available() -> Vec<Self> {
        Self::CANDIDATES.iter().copied().filter(|app| app.installed()).collect()
    }

    /// What "Open in Terminal" really uses: the chosen app, or the first
    /// installed one when it is missing (a setting from another computer).
    pub fn effective(self) -> Self {
        let available = Self::available();
        if available.contains(&self) { self } else { available.first().copied().unwrap_or(self) }
    }

    fn installed(self) -> bool {
        match self {
            Self::Terminal | Self::PowerShell | Self::Cmd => true,
            #[cfg(target_os = "macos")]
            Self::Iterm | Self::Warp | Self::Ghostty => {
                let bundle = if self == Self::Iterm { "iTerm.app".to_string() } else { format!("{}.app", self.app_name()) };
                let home = dirs::home_dir().map(|home| home.join("Applications").join(&bundle));
                std::path::Path::new("/Applications").join(&bundle).exists() || home.is_some_and(|path| path.exists())
            }
            #[cfg(not(target_os = "macos"))]
            Self::Ghostty => on_path("ghostty"),
            #[cfg(windows)]
            Self::WindowsTerminal => on_path("wt"),
            #[allow(unreachable_patterns)]
            _ => false,
        }
    }
}

/// Whether `program` (`.exe` on Windows) is in a `PATH` folder.
#[cfg(not(target_os = "macos"))]
fn on_path(program: &str) -> bool {
    let name = if cfg!(windows) { format!("{program}.exe") } else { program.to_string() };
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(&name).is_file()))
}

impl Default for ExtensionSettings {
    fn default() -> Self {
        Self {
            open_with: true,
            archives: true,
            git: true,
            copy: true,
            ssh: true,
            terminal: true,
            forks: true,
            agents: true,
            mail: true,
            music: true,
            archives_create_folder: true,
            archives_overwrite: false,
            terminal_app: TerminalApp::Terminal,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HumanSizeSettings {
    pub decimals: u8,
    pub base: SizeBase,
}

impl Default for HumanSizeSettings {
    fn default() -> Self {
        Self {
            decimals: 1,
            base: SizeBase::Binary,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeBase {
    Binary,
    Decimal,
}

impl SizeBase {
    pub fn divisor(self) -> f64 {
        match self {
            Self::Binary => 1024.,
            Self::Decimal => 1000.,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DateTimeSettings {
    pub date_format: DateFormat,
    pub clock_format: ClockFormat,
}

impl Default for DateTimeSettings {
    fn default() -> Self {
        Self {
            date_format: DateFormat::Relative,
            clock_format: ClockFormat::Hour24,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateFormat {
    Relative,
    Dmy,
    Iso,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockFormat {
    Hour24,
    Hour12,
}

pub fn localized(language: Language, english: &'static str, russian: &'static str) -> &'static str {
    match language {
        Language::English => english,
        Language::Russian => russian,
    }
}

pub fn format_size(bytes: u64, settings: HumanSizeSettings) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let divisor = settings.base.divisor();
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= divisor && unit + 1 < UNITS.len() {
        value /= divisor;
        unit += 1;
    }
    if unit == 0 {
        return format!("{bytes} bytes");
    }
    let precision = settings.decimals.min(2) as usize;
    format!("{value:.precision$} {}", UNITS[unit])
}

pub fn format_demo_date(value: &str, settings: DateTimeSettings) -> String {
    let Some(date) = DemoDate::parse(value) else {
        return value.to_string();
    };
    let time = match settings.clock_format {
        ClockFormat::Hour24 => format!("{:02}:{:02}", date.hour, date.minute),
        ClockFormat::Hour12 => {
            let suffix = if date.hour < 12 { "AM" } else { "PM" };
            let hour = match date.hour % 12 {
                0 => 12,
                hour => hour,
            };
            format!("{hour}:{:02} {suffix}", date.minute)
        }
    };
    match settings.date_format {
        DateFormat::Relative => {
            let days = demo_days_since(date);
            match days {
                0 => format!("Today, {time}"),
                1 => format!("Yesterday, {time}"),
                days if days > 1 => format!("{days} days ago, {time}"),
                _ => format!("{:02}.{:02}.{:04}, {time}", date.day, date.month, date.year),
            }
        }
        DateFormat::Dmy => format!("{:02}.{:02}.{:04}, {time}", date.day, date.month, date.year),
        DateFormat::Iso => format!("{:04}-{:02}-{:02} {time}", date.year, date.month, date.day),
    }
}

#[derive(Clone, Copy)]
struct DemoDate {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
}

impl DemoDate {
    fn parse(value: &str) -> Option<Self> {
        let (date, time) = value.rsplit_once(',')?;
        let (hour, minute) = time.trim().split_once(':')?;
        let hour = hour.parse().ok()?;
        let minute = minute.parse().ok()?;
        let mut parts = date.split_whitespace();
        let first = parts.next()?;
        let (year, month, day) = match first {
            "Today" => (2026, 10, 27),
            "Yesterday" => (2026, 10, 26),
            "Monday" => (2026, 10, 26),
            "Tuesday" => (2026, 10, 27),
            "Friday" => (2026, 10, 23),
            month => {
                let month = month_number(month)?;
                let day = parts.next()?.trim_end_matches(',').parse().ok()?;
                let year = parts
                    .next()
                    .map(|year| year.trim_end_matches(',').parse())
                    .transpose()
                    .ok()?
                    .unwrap_or(2026);
                (year, month, day)
            }
        };
        Some(Self {
            year,
            month,
            day,
            hour,
            minute,
        })
    }
}

fn month_number(month: &str) -> Option<u32> {
    Some(match month {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    })
}

fn demo_days_since(date: DemoDate) -> i32 {
    let today = chrono::Local::now().date_naive();
    chrono::NaiveDate::from_ymd_opt(date.year, date.month, date.day)
        .map_or(-1, |date| (today - date).num_days() as i32)
}

/// Before it was renamed, the app kept its folder under "file-explorer":
/// move it once so settings, the license, forks and plugins carry over.
fn migrate_legacy_folder(path: &Path) {
    let Some(folder) = path.parent() else { return };
    let legacy = folder.with_file_name("file-explorer");
    if !folder.exists() && legacy.is_dir() {
        let _ = fs::rename(&legacy, folder);
    }
}

/// Whether settings.toml was already there when the app started (so this is
/// not the first run, even if an older version did not record its number).
pub fn existed_at_start() -> bool {
    EXISTED_AT_START.load(std::sync::atomic::Ordering::Relaxed)
}

static EXISTED_AT_START: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn load() -> AppSettings {
    if let Some(path) = settings_path().filter(|_| !cfg!(test)) {
        migrate_legacy_folder(&path);
        if path.exists() {
            EXISTED_AT_START.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    let Some(path) = settings_path() else {
        set_load_error(Some(
            "Could not determine the settings directory".to_string(),
        ));
        return AppSettings::default();
    };
    match fs::read_to_string(&path) {
        Ok(source) => match toml::from_str::<AppSettings>(&source) {
            Ok(mut settings) => {
                settings.migrate();
                settings
            }
            Err(error) => {
                set_load_error(Some(format!(
                    "Could not parse {}: {error}. Defaults are in use; the file was not changed.",
                    path.display()
                )));
                AppSettings::default()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => AppSettings::default(),
        Err(error) => {
            set_load_error(Some(format!(
                "Could not read {}: {error}. Defaults are in use; the file was not changed.",
                path.display()
            )));
            AppSettings::default()
        }
    }
}

pub fn save(settings: &AppSettings) -> Result<(), String> {
    let path =
        settings_path().ok_or_else(|| "Could not determine the settings directory".to_string())?;
    let source = toml::to_string_pretty(settings)
        .map_err(|error| format!("Could not serialize settings: {error}"))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("Invalid settings path: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, source)
        .map_err(|error| format!("Could not write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, &path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("Could not replace {}: {error}", path.display())
    })
}

pub fn load_error() -> Option<String> {
    LOAD_ERROR
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|error| error.clone())
}

pub fn settings_path() -> Option<PathBuf> {
    // Tests must never overwrite the user's real settings.
    #[cfg(test)]
    {
        return Some(
            std::env::temp_dir()
                // Per test thread, so parallel tests never share a settings file.
                .join(format!(
                    "file-force-test-settings-{}-{:?}",
                    std::process::id(),
                    std::thread::current().id()
                ))
                .join("settings.toml"),
        );
    }
    #[cfg(all(not(test), target_os = "macos"))]
    {
        home_dir().map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("file-force")
                .join("settings.toml")
        })
    }
    #[cfg(all(not(test), target_os = "windows"))]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|home| home.join("AppData").join("Roaming")))
            .map(|root| root.join("file-force").join("settings.toml"))
    }
    #[cfg(all(not(test), unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|home| home.join(".config")))
            .map(|root| root.join("file-force").join("settings.toml"))
    }
    #[cfg(all(not(test), not(any(target_os = "macos", target_os = "windows", unix))))]
    {
        home_dir().map(|home| home.join("file-force").join("settings.toml"))
    }
}

#[cfg_attr(test, allow(dead_code))]
fn home_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

fn set_load_error(error: Option<String>) {
    if let Ok(mut slot) = LOAD_ERROR.get_or_init(|| Mutex::new(None)).lock() {
        *slot = error;
    }
}

#[cfg(test)]
mod deletion_tests {
    use super::*;

    #[test]
    fn deletion_settings_round_trip_through_toml() {
        let mut settings = AppSettings::default();
        settings.deletion.protected.push("~/Projects/**".to_string());
        settings.deletion.use_trash = false;
        let source = toml::to_string_pretty(&settings).unwrap();
        assert!(source.contains("[deletion]"), "{source}");
        assert!(source.contains("\"~/Projects/**\""));
        let parsed: AppSettings = toml::from_str(&source).unwrap();
        assert_eq!(parsed, settings);
    }

    #[test]
    fn default_app_overrides_round_trip_through_toml() {
        let mut settings = AppSettings::default();
        settings
            .default_apps
            .insert("docx".to_string(), "/Applications/LibreOffice.app".to_string());
        let source = toml::to_string_pretty(&settings).unwrap();
        assert!(source.contains("[default_apps]"), "{source}");
        assert!(source.contains("docx = \"/Applications/LibreOffice.app\""), "{source}");
        let parsed: AppSettings = toml::from_str(&source).unwrap();
        assert_eq!(parsed.default_apps, settings.default_apps);
        assert!(toml::from_str::<AppSettings>("").unwrap().default_apps.is_empty());
    }

    #[test]
    fn extension_settings_keep_old_files_working() {
        let old: AppSettings = toml::from_str("[extensions]\nssh = false\n").unwrap();
        assert!(!old.extensions.ssh);
        assert!(old.extensions.archives_create_folder, "new fields default on load");
        assert_eq!(old.extensions.terminal_app, TerminalApp::Terminal);

        let mut settings = AppSettings::default();
        settings.extensions.terminal_app = TerminalApp::Ghostty;
        settings.extensions.archives_overwrite = true;
        let source = toml::to_string_pretty(&settings).unwrap();
        assert!(source.contains("terminal_app = \"ghostty\""), "{source}");
        let parsed: AppSettings = toml::from_str(&source).unwrap();
        assert_eq!(parsed.extensions, settings.extensions);
    }

    #[test]
    fn default_view_round_trips() {
        let mut settings = AppSettings::default();
        settings.behavior.default_view = crate::model::ViewMode::Dual;
        let source = toml::to_string_pretty(&settings).unwrap();
        assert!(source.contains("default_view = \"dual\""), "{source}");
        assert_eq!(toml::from_str::<AppSettings>(&source).unwrap().behavior.default_view, crate::model::ViewMode::Dual);
    }

    #[test]
    fn power_saving_defaults_to_automatic_and_round_trips() {
        let old: AppSettings = toml::from_str("").unwrap();
        assert_eq!(old.power_saving, PowerSavingMode::Automatic);
        let mut settings = AppSettings::default();
        settings.power_saving = PowerSavingMode::On;
        let source = toml::to_string_pretty(&settings).unwrap();
        assert!(source.contains("power_saving = \"on\""), "{source}");
        assert_eq!(toml::from_str::<AppSettings>(&source).unwrap().power_saving, PowerSavingMode::On);
    }

    #[test]
    fn look_settings_round_trip_and_hex_parses() {
        let mut settings = AppSettings::default();
        settings.look.panel_opacity = 70;
        settings.look.background = Background::Image { path: PathBuf::from("/tmp/wall.jpg") };
        let source = toml::to_string_pretty(&settings).unwrap();
        assert!(source.contains("[look]") && source.contains("panel_opacity = 70"), "{source}");
        assert_eq!(toml::from_str::<AppSettings>(&source).unwrap().look, settings.look);
        assert!(parse_hex("#1E293B").is_some());
        assert!(parse_hex("zzz").is_none() && parse_hex("#12345").is_none());
        assert_eq!(LookSettings { panel_opacity: 10, ..Default::default() }.opacity(), 0.4, "clamped");
    }

    #[test]
    fn system_theme_follows_the_os_and_round_trips() {
        use crate::model::{ColorScheme, set_system_dark};
        set_system_dark(false);
        assert!(!ColorScheme::System.is_dark());
        assert_eq!(ColorScheme::System.toggled(), ColorScheme::Dark, "toggle flips what is showing");
        set_system_dark(true);
        assert!(ColorScheme::System.is_dark());
        let parsed: AppSettings = toml::from_str("scheme = \"system\"\n").unwrap();
        assert_eq!(parsed.scheme, ColorScheme::System);
    }

    #[test]
    fn accessibility_round_trips_and_system_toggles_resolve() {
        assert!(Toggle::System.resolve(true) && !Toggle::System.resolve(false));
        assert!(Toggle::On.resolve(false) && !Toggle::Off.resolve(true));
        let mut settings = AppSettings::default();
        settings.accessibility.high_contrast = Toggle::On;
        settings.accessibility.text_scale = TextScale::Large;
        let source = toml::to_string_pretty(&settings).unwrap();
        assert!(source.contains("[accessibility]") && source.contains("high_contrast = \"on\""), "{source}");
        assert_eq!(toml::from_str::<AppSettings>(&source).unwrap().accessibility, settings.accessibility);
        assert_eq!(AppSettings::default().accessibility.reduce_motion, Toggle::System, "follows macOS by default");
        // Values written before the slider still load.
        let old: AppSettings = toml::from_str("[accessibility]\ntext_scale = \"larger\"\n").unwrap();
        assert_eq!(old.accessibility.text_scale, TextScale::Larger);
        for (index, scale) in TextScale::ALL.into_iter().enumerate() {
            assert_eq!(TextScale::from_index(index), scale);
            assert_eq!(scale.index(), index);
        }
        assert!(TextScale::ALL.windows(2).all(|pair| pair[0].factor() < pair[1].factor()), "steps grow");
    }

    #[test]
    fn drop_settings_default_to_copy_and_round_trip() {
        assert_eq!(AppSettings::default().behavior.drop_action, DropAction::Copy);
        assert!(!AppSettings::default().behavior.ask_on_drop);
        let mut settings = AppSettings::default();
        settings.behavior.drop_action = DropAction::Finder;
        settings.behavior.ask_on_drop = true;
        let source = toml::to_string_pretty(&settings).unwrap();
        assert!(source.contains("drop_action = \"finder\""), "{source}");
        assert_eq!(toml::from_str::<AppSettings>(&source).unwrap().behavior, settings.behavior);
    }

    #[test]
    fn the_old_settings_folder_moves_to_the_new_name() {
        let root = std::env::temp_dir().join(format!("file-force-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("file-explorer")).unwrap();
        fs::write(root.join("file-explorer/settings.toml"), "scheme = \"dark\"").unwrap();
        let new = root.join("file-force/settings.toml");
        migrate_legacy_folder(&new);
        assert_eq!(fs::read_to_string(&new).unwrap(), "scheme = \"dark\"");
        assert!(!root.join("file-explorer").exists());
        // Never over an existing new folder.
        fs::create_dir_all(root.join("file-explorer")).unwrap();
        migrate_legacy_folder(&new);
        assert!(root.join("file-explorer").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sidebar_settings_round_trip() {
        let mut settings = AppSettings::default();
        settings.sidebar.collapsed = vec!["tags".into()];
        settings.sidebar.favorites = vec![PathBuf::from("/Users/me/Documents/git")];
        let source = toml::to_string_pretty(&settings).unwrap();
        assert!(source.contains("[sidebar]"), "{source}");
        assert_eq!(toml::from_str::<AppSettings>(&source).unwrap().sidebar, settings.sidebar);
    }

    #[test]
    fn old_settings_files_get_the_default_blacklist() {
        let parsed: AppSettings = toml::from_str("scheme = \"light\"\n").unwrap();
        assert!(parsed.deletion.use_trash);
        assert_eq!(parsed.deletion.protected.len(), crate::delete::default_protected().len());
    }
}

/// The last settings seen, to report what changed.
static LOGGED: std::sync::Mutex<Option<toml::Table>> = std::sync::Mutex::new(None);

/// Records every setting that changed since the last call in the developer
/// console (`look.panel_opacity: 90 → 75`). The first call only remembers.
/// Secrets (license key, passwords, tokens) are masked.
pub fn log_changes(settings: &AppSettings) {
    let Ok(toml::Value::Table(now)) = toml::Value::try_from(settings) else { return };
    let mut logged = LOGGED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(before) = logged.as_ref() {
        for (key, old, new) in changes(before, &now) {
            files_sdk::log::record("settings", files_sdk::log::Kind::Setting, format!("{key}: {old} → {new}"), None);
        }
    }
    *logged = Some(now);
}

/// Changed leaves of two tables as (`dotted.key`, old, new), in key order.
fn changes(before: &toml::Table, now: &toml::Table) -> Vec<(String, String, String)> {
    fn flatten(prefix: &str, value: &toml::Value, out: &mut std::collections::BTreeMap<String, String>) {
        match value {
            toml::Value::Table(table) => {
                for (key, value) in table {
                    let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
                    flatten(&path, value, out);
                }
            }
            other => {
                out.insert(prefix.to_string(), other.to_string());
            }
        }
    }
    let secret = |key: &str| ["key", "password", "token", "secret"].iter().any(|word| key.rsplit('.').next().is_some_and(|last| last.contains(word)));
    // Bookkeeping, not choices: when updates were last checked.
    let noise = |key: &str| key == "updates.last_check";
    let (mut old, mut new) = (std::collections::BTreeMap::new(), std::collections::BTreeMap::new());
    flatten("", &toml::Value::Table(before.clone()), &mut old);
    flatten("", &toml::Value::Table(now.clone()), &mut new);
    let keys: std::collections::BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    keys.into_iter()
        .filter(|key| old.get(*key) != new.get(*key) && !noise(key))
        .map(|key| {
            let show = |value: Option<&String>| match value {
                None => "(none)".to_string(),
                Some(_) if secret(key) => "••••".to_string(),
                Some(value) => value.clone(),
            };
            (key.clone(), show(old.get(key)), show(new.get(key)))
        })
        .collect()
}

#[cfg(test)]
mod change_log_tests {
    use super::*;

    #[test]
    fn changed_settings_are_listed_and_secrets_masked() {
        let before = AppSettings::default();
        let mut after = before.clone();
        after.power_saving = PowerSavingMode::On;
        after.power_saving_below = 20;
        after.license.key = "FILES1.secret".into();
        after.updates.last_check = 99;
        let table = |settings: &AppSettings| match toml::Value::try_from(settings).unwrap() {
            toml::Value::Table(table) => table,
            _ => unreachable!(),
        };
        let listed = changes(&table(&before), &table(&after));
        let keys: Vec<&str> = listed.iter().map(|(key, _, _)| key.as_str()).collect();
        assert_eq!(keys, ["license.key", "power_saving", "power_saving_below"], "last_check is bookkeeping");
        assert_eq!(listed[1], ("power_saving".into(), "\"automatic\"".into(), "\"on\"".into()));
        assert_eq!(listed[2].1, "100");
        assert_eq!(listed[0].2, "••••", "never the key itself");
    }
}
