use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, px, rgb};

use crate::model::{Accent, ColorScheme};

#[derive(Clone, Copy)]
pub struct Palette {
    pub window: Hsla,
    pub sidebar: Hsla,
    pub surface: Hsla,
    pub elevated: Hsla,
    pub hover: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub accent: Hsla,
    pub accent_soft: Hsla,
    pub accent_contrast: Hsla,
    pub selection: Hsla,
    pub folder: Hsla,
    pub media: Hsla,
    pub document: Hsla,
    pub code: Hsla,
    pub video: Hsla,
    pub archive: Hsla,
    /// Accessibility → Text contrast: outline text with the opposite color.
    pub text_outline: Option<Hsla>,
    /// Accessibility → Shimmer: loading text pulses.
    pub shimmer: bool,
    /// Appearance → Icons.
    pub icon_theme: crate::settings::IconTheme,
}

use crate::settings::{IconTheme, ThemePreset};

/// The theme and icon style in use, set from Settings (every window reads
/// them, so they need no plumbing through each view).
static LOOK: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub fn set_look(theme: ThemePreset, icons: IconTheme) {
    let theme = ThemePreset::ALL.iter().position(|preset| *preset == theme).unwrap_or(0) as u8;
    let icons = IconTheme::ALL.iter().position(|style| *style == icons).unwrap_or(0) as u8;
    LOOK.store(theme | icons << 4, std::sync::atomic::Ordering::Relaxed);
}

fn look() -> (ThemePreset, IconTheme) {
    let value = LOOK.load(std::sync::atomic::Ordering::Relaxed);
    (ThemePreset::ALL[(value & 0xf) as usize % ThemePreset::ALL.len()], IconTheme::ALL[(value >> 4) as usize % IconTheme::ALL.len()])
}

/// Settings → Appearance → Font: the interface font chosen over the theme's.
static FONT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn set_font(font: Option<String>) {
    *FONT.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = font.filter(|font| !font.trim().is_empty());
}

fn chosen_font() -> Option<String> {
    FONT.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
}

/// Omarchy's terminal look is on: monospace, square, flat.
pub fn terminal_look() -> bool {
    look().0 == ThemePreset::Omarchy
}

impl Palette {
    /// The palette for any window: theme, accent and Accessibility → High
    /// contrast. (Only the main window also applies Appearance opacity.)
    pub fn from_settings(settings: &crate::settings::AppSettings) -> Self {
        let palette = Self::new(settings.scheme, settings.accent).with_accessibility(&settings.accessibility);
        let mut palette = if settings.accessibility.high_contrast() {
            palette.with_high_contrast(settings.scheme.is_dark())
        } else {
            palette
        };
        palette.shimmer &= !crate::power::power_saving_for(settings.power_saving, settings.power_saving_below);
        palette
    }

    /// Text contrast and Shimmer from Accessibility.
    pub fn with_accessibility(mut self, accessibility: &crate::settings::AccessibilitySettings) -> Self {
        self.text_outline = accessibility
            .text_contrast()
            .then(|| if self.text.l < 0.5 { gpui_kit::white() } else { gpui_kit::black() });
        self.shimmer = accessibility.shimmer();
        self
    }

    /// Stronger separation for low vision: brighter secondary text, visible
    /// borders, solid hover and selection.
    pub fn with_high_contrast(mut self, dark: bool) -> Self {
        self.muted = self.text;
        self.faint = alpha(self.text, 0.78);
        self.border = alpha(self.text, if dark { 0.45 } else { 0.55 });
        self.selection = alpha(self.accent, 0.38);
        self.accent_soft = alpha(self.accent, 0.26);
        self.hover = alpha(self.text, 0.12);
        self
    }

    /// Makes the panel backgrounds see-through (1.0 = opaque).
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        if opacity < 1. {
            for color in [&mut self.window, &mut self.sidebar, &mut self.surface, &mut self.hover] {
                color.a *= opacity;
            }
            // Raised surfaces (menus, cards) stay more solid for legibility.
            self.elevated.a *= (opacity + 1.) / 2.;
        }
        self
    }

    pub fn new(scheme: ColorScheme, accent: Accent) -> Self {
        let (theme, icons) = look();
        let mut palette = Self::themed(theme, scheme.is_dark(), accent);
        palette.icon_theme = icons;
        palette
    }

    /// The colors of `theme` in light or dark.
    pub fn themed(theme: ThemePreset, dark: bool, accent: Accent) -> Self {
        match theme {
            ThemePreset::Default => Self::files(dark, accent),
            ThemePreset::Omarchy => Self::tokyo_night(dark),
            ThemePreset::Catppuccin => Self::catppuccin(dark, accent),
            ThemePreset::Windows => Self::windows(dark, accent),
            ThemePreset::MacOs => Self::macos(dark, accent),
        }
    }

    fn files(dark: bool, accent: Accent) -> Self {
        let accent = match (dark, accent) {
            (true, Accent::Iris) => color(0x938ffb),
            (true, Accent::Azure) => color(0x64b5f6),
            (true, Accent::Amber) => color(0xe8b565),
            (false, Accent::Iris) => color(0x6659d9),
            (false, Accent::Azure) => color(0x247dc5),
            (false, Accent::Amber) => color(0xa56b18),
        };
        let accent_contrast = if dark {
            color(0x15151d)
        } else {
            color(0xffffff)
        };

        if dark {
            Self {
                window: color(0x101115),
                sidebar: color(0x15161b),
                surface: color(0x1b1d23),
                elevated: color(0x22252c),
                hover: color(0x262931),
                border: color(0x2b2e37),
                text: color(0xf1f2f5),
                muted: color(0x9b9fac),
                faint: color(0x686e7a),
                accent,
                accent_soft: alpha(accent, 0.14),
                accent_contrast,
                text_outline: None,
                shimmer: true,
                icon_theme: IconTheme::Colorful,
                selection: alpha(accent, 0.19),
                folder: color(0xe3b45f),
                media: color(0x8eb8ef),
                document: color(0xb8a5ed),
                code: color(0x78c7a4),
                video: color(0xe795a3),
                archive: color(0xc5a37e),
            }
        } else {
            Self {
                window: color(0xf5f5f3),
                sidebar: color(0xecedf0),
                surface: color(0xffffff),
                elevated: color(0xffffff),
                hover: color(0xe8eaee),
                border: color(0xdadee4),
                text: color(0x22232a),
                muted: color(0x6d7380),
                faint: color(0x989ea8),
                accent,
                accent_soft: alpha(accent, 0.11),
                accent_contrast,
                text_outline: None,
                shimmer: true,
                icon_theme: IconTheme::Colorful,
                selection: alpha(accent, 0.13),
                folder: color(0xbd7f25),
                media: color(0x397fbd),
                document: color(0x7961b6),
                code: color(0x328365),
                video: color(0xb65164),
                archive: color(0x88643f),
            }
        }
    }
}

/// Timing for every animated GPUI Kit component. Smooth and Relaxed ease in
/// *and* out (the stock exit curve accelerates into the end, which reads as
/// abrupt) and use slower, overshoot-free springs.
pub fn motion_tokens(style: crate::settings::MotionStyle) -> gpui_kit::component::theme::MotionTokens {
    use crate::settings::MotionStyle;
    use gpui_kit::base::{Easing, Spring};
    use std::time::Duration;
    let stock = gpui_kit::component::theme::MotionTokens::default();
    let (fast, normal, slow, spring, travel) = match style {
        MotionStyle::Snappy => return stock,
        MotionStyle::Smooth => (170, 260, 380, 260, 380),
        MotionStyle::Relaxed => (240, 360, 520, 360, 520),
    };
    let curve = |a, b, c, d| Easing::cubic_bezier(a, b, c, d).expect("static curve is valid");
    gpui_kit::component::theme::MotionTokens {
        duration_fast: Duration::from_millis(fast),
        duration_normal: Duration::from_millis(normal),
        duration_slow: Duration::from_millis(slow),
        // Ease-out quint in, gentle ease-out away, ease-in-out for moves.
        easing_enter: curve(0.22, 1.0, 0.36, 1.0),
        easing_exit: curve(0.33, 0.0, 0.2, 1.0),
        easing_move: curve(0.65, 0.0, 0.35, 1.0),
        spring_control: Spring::new(Duration::from_millis(spring)).with_damping(1.0),
        spring_move: Spring::new(Duration::from_millis(travel)).with_damping(0.95).with_epsilon(0.1),
        ..stock
    }
}

/// Applies the animation style app-wide.
pub fn apply_motion(cx: &mut App, style: crate::settings::MotionStyle) {
    Theme::update(cx, |theme| theme.motion = motion_tokens(style));
}

/// Panel opacity (×1000) for the window chrome GPUI Kit draws itself: the
/// Root background under our view and the title bar. Without this they stay
/// opaque and hide the blurred desktop or the custom background.
static CHROME_OPACITY: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1000);

/// Sets how see-through the window chrome is and re-applies the theme.
pub fn apply_chrome_opacity(cx: &mut App, opacity: f32, scheme: ColorScheme, accent: Accent) {
    let value = (opacity.clamp(0., 1.) * 1000.).round() as u32;
    if CHROME_OPACITY.swap(value, std::sync::atomic::Ordering::Relaxed) != value {
        apply_theme(cx, scheme, accent);
    }
}

pub fn apply_theme(cx: &mut App, scheme: ColorScheme, accent: Accent) {
    let chrome = CHROME_OPACITY.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.;
    let palette = Palette::new(scheme, accent);
    Theme::update(cx, |theme| {
        theme.mode = if scheme.is_dark() {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };
    });
    // The UI font before any theme changed it, for switching back.
    static UI_FONT: std::sync::OnceLock<gpui_kit::SharedString> = std::sync::OnceLock::new();
    let ui_font = UI_FONT.get_or_init(|| cx.global::<Theme>().font_family.clone()).clone();
    let tui = terminal_look();
    let preset = look().0;
    // Corners: a terminal's none, Windows 11's 4/8, Finder's 6/10.
    let (radius, radius_lg) = match preset {
        ThemePreset::Omarchy => (0., 0.),
        ThemePreset::Windows => (4., 8.),
        ThemePreset::MacOs => (6., 10.),
        _ => (7., 11.),
    };
    // Segoe UI where it exists; elsewhere the system font stands in.
    let font: gpui_kit::SharedString = match preset {
        // The user's own choice wins over every theme's.
        _ if chosen_font().is_some() => chosen_font().unwrap_or_default().into(),
        ThemePreset::Omarchy => terminal_font().into(),
        ThemePreset::Windows if cfg!(windows) => "Segoe UI".into(),
        _ => ui_font.clone(),
    };
    Theme::update(cx, |theme| {
        theme.font_size = px(if tui { 13. } else { 14. });
        theme.radius = px(radius);
        theme.radius_lg = px(radius_lg);
        theme.shadow = !tui;
        theme.font_family = font;

        theme.colors.background = palette.window;
        theme.colors.foreground = palette.text;
        theme.colors.sidebar = palette.sidebar;
        theme.colors.sidebar_foreground = palette.text;
        theme.colors.sidebar_border = palette.border;
        theme.colors.sidebar_accent = palette.selection;
        theme.colors.sidebar_accent_foreground = palette.text;
        theme.colors.border = palette.border;
        theme.colors.input = palette.border;
        theme.colors.muted = palette.surface;
        theme.colors.muted_foreground = palette.muted;
        theme.colors.secondary = palette.surface;
        theme.colors.secondary_foreground = palette.text;
        theme.colors.secondary_hover = palette.hover;
        theme.colors.secondary_active = palette.selection;
        theme.colors.accent = palette.accent_soft;
        theme.colors.accent_foreground = palette.text;
        theme.colors.primary = palette.accent;
        theme.colors.primary_hover = palette.accent;
        theme.colors.primary_active = palette.accent;
        theme.colors.primary_foreground = palette.accent_contrast;
        theme.colors.button_primary = palette.accent;
        theme.colors.button_primary_hover = palette.accent;
        theme.colors.button_primary_active = palette.accent;
        theme.colors.button_primary_foreground = palette.accent_contrast;
        theme.colors.list = palette.window;
        theme.colors.list_even = palette.surface;
        theme.colors.list_head = palette.surface;
        theme.colors.list_hover = palette.hover;
        theme.colors.list_active = palette.selection;
        theme.colors.list_active_border = palette.accent;
        theme.colors.popover = palette.elevated;
        theme.colors.popover_foreground = palette.text;
        theme.colors.selection = palette.accent_soft;
        theme.colors.caret = palette.accent;
        theme.colors.ring = palette.accent;
        theme.colors.progress_bar = palette.accent;
        theme.colors.scrollbar = palette.window;
        // gpui-kit hides scrollbars until you scroll; always show them so long
        // lists visibly scroll, with a thumb that stands out from the borders.
        theme.scrollbar_mode = gpui_kit::component::scroll::ScrollbarMode::Always;
        theme.colors.scrollbar_thumb = palette.faint;
        theme.colors.scrollbar_thumb_hover = palette.muted;
        let see_through = palette.with_opacity(chrome);
        theme.colors.background = see_through.window;
        theme.colors.title_bar = see_through.sidebar;
    });
}

/// A palette from its colors in order.
#[allow(clippy::too_many_arguments)]
fn palette(
    [window, sidebar, surface, elevated, hover, border, text, muted, faint]: [u32; 9],
    accent: Hsla,
    selection: Hsla,
    contrast: u32,
    [folder, media, document, code, video, archive]: [u32; 6],
) -> Palette {
    Palette {
        window: color(window),
        sidebar: color(sidebar),
        surface: color(surface),
        elevated: color(elevated),
        hover: color(hover),
        border: color(border),
        text: color(text),
        muted: color(muted),
        faint: color(faint),
        accent,
        accent_soft: alpha(accent, 0.16),
        accent_contrast: color(contrast),
        selection,
        text_outline: None,
        shimmer: true,
        icon_theme: IconTheme::Colorful,
        folder: color(folder),
        media: color(media),
        document: color(document),
        code: color(code),
        video: color(video),
        archive: color(archive),
    }
}

impl Palette {
    /// Omarchy's default, Tokyo Night (Day in light): its own blue, and
    /// borders visible like a terminal UI's boxes.
    fn tokyo_night(dark: bool) -> Self {
        if dark {
            palette(
                [0x1a1b26, 0x16161e, 0x1f2335, 0x24283b, 0x292e42, 0x414868, 0xc0caf5, 0xa9b1d6, 0x565f89],
                color(0x7aa2f7),
                color(0x283457),
                0x1a1b26,
                [0xe0af68, 0x7dcfff, 0xbb9af7, 0x9ece6a, 0xf7768e, 0xff9e64],
            )
        } else {
            palette(
                [0xe1e2e7, 0xd0d5e3, 0xe9e9ed, 0xf2f2f5, 0xc4c8da, 0xa8aecb, 0x3760bf, 0x6172b0, 0x8990b3],
                color(0x2e7de9),
                color(0xb7c1e3),
                0xffffff,
                [0x8c6c3e, 0x007197, 0x9854f1, 0x587539, 0xf52a65, 0xb15c00],
            )
        }
    }

    /// Windows 11's Fluent colors: Mica greys, its blue (the accent picks
    /// blue, purple or orange from the same palette).
    fn windows(dark: bool, accent: Accent) -> Self {
        if dark {
            let accent = color(match accent {
                Accent::Azure => 0x4cc2ff,
                Accent::Iris => 0xb4a0ff,
                Accent::Amber => 0xffb557,
            });
            palette(
                [0x202020, 0x1c1c1c, 0x2b2b2b, 0x2d2d2d, 0x353535, 0x3b3b3b, 0xffffff, 0xc5c5c5, 0x8a8a8a],
                accent,
                alpha(accent, 0.22),
                0x000000,
                [0xffc83d, 0x60cdff, 0xb4a0ff, 0x6ccb5f, 0xff99a4, 0xf7b870],
            )
        } else {
            let accent = color(match accent {
                Accent::Azure => 0x005fb8,
                Accent::Iris => 0x744da9,
                Accent::Amber => 0xc04d00,
            });
            palette(
                [0xf3f3f3, 0xebebeb, 0xfbfbfb, 0xffffff, 0xe5e5e5, 0xe0e0e0, 0x1a1a1a, 0x5d5d5d, 0x8a8a8a],
                accent,
                alpha(accent, 0.14),
                0xffffff,
                [0xe3a21a, 0x0078d4, 0x8764b8, 0x107c10, 0xc42b1c, 0xca5010],
            )
        }
    }

    /// macOS: window and sidebar greys, the system blue (or purple and
    /// orange for the other accents), separators as thin as Finder's.
    fn macos(dark: bool, accent: Accent) -> Self {
        if dark {
            let accent = color(match accent {
                Accent::Azure => 0x0a84ff,
                Accent::Iris => 0xbf5af2,
                Accent::Amber => 0xff9f0a,
            });
            palette(
                [0x1e1e1e, 0x282828, 0x232323, 0x2c2c2e, 0x323234, 0x3a3a3c, 0xf5f5f7, 0xa1a1a6, 0x6e6e73],
                accent,
                alpha(accent, 0.32),
                0xffffff,
                [0x64d2ff, 0x64d2ff, 0xbf5af2, 0x30d158, 0xff453a, 0xff9f0a],
            )
        } else {
            let accent = color(match accent {
                Accent::Azure => 0x007aff,
                Accent::Iris => 0xaf52de,
                Accent::Amber => 0xff9500,
            });
            palette(
                [0xffffff, 0xf0f0f2, 0xfafafa, 0xffffff, 0xe8e8ed, 0xd8d8dc, 0x1d1d1f, 0x6e6e73, 0x98989d],
                accent,
                alpha(accent, 0.2),
                0xffffff,
                [0x1e9bf0, 0x007aff, 0xaf52de, 0x28cd41, 0xff3b30, 0xff9500],
            )
        }
    }

    /// Catppuccin Mocha (dark) and Latte (light); the accent picks mauve,
    /// blue or peach.
    fn catppuccin(dark: bool, accent: Accent) -> Self {
        if dark {
            let accent = color(match accent {
                Accent::Iris => 0xcba6f7,
                Accent::Azure => 0x89b4fa,
                Accent::Amber => 0xfab387,
            });
            palette(
                [0x1e1e2e, 0x181825, 0x313244, 0x313244, 0x45475a, 0x45475a, 0xcdd6f4, 0xa6adc8, 0x7f849c],
                accent,
                alpha(accent, 0.22),
                0x11111b,
                [0xf9e2af, 0x74c7ec, 0xb4befe, 0xa6e3a1, 0xf38ba8, 0xfab387],
            )
        } else {
            let accent = color(match accent {
                Accent::Iris => 0x8839ef,
                Accent::Azure => 0x1e66f5,
                Accent::Amber => 0xfe640b,
            });
            palette(
                [0xeff1f5, 0xe6e9ef, 0xffffff, 0xffffff, 0xdce0e8, 0xccd0da, 0x4c4f69, 0x6c6f85, 0x8c8fa1],
                accent,
                alpha(accent, 0.16),
                0xffffff,
                [0xdf8e1d, 0x209fb5, 0x7287fd, 0x40a02b, 0xd20f39, 0xfe640b],
            )
        }
    }
}

/// The font a terminal would use on this system.
pub fn terminal_font() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(windows) {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    }
}

fn color(value: u32) -> Hsla {
    rgb(value).into()
}

fn alpha(value: Hsla, alpha: f32) -> Hsla {
    Hsla { a: alpha, ..value }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooth_motion_is_slower_and_eases_out() {
        use crate::settings::MotionStyle;
        let snappy = motion_tokens(MotionStyle::Snappy);
        let smooth = motion_tokens(MotionStyle::Smooth);
        let relaxed = motion_tokens(MotionStyle::Relaxed);
        assert!(snappy.duration_normal < smooth.duration_normal);
        assert!(smooth.duration_normal < relaxed.duration_normal);
        // The exit curve must decelerate into the end, not accelerate.
        let exit = |tokens: &gpui_kit::component::theme::MotionTokens, t: f32| tokens.easing_exit.sample(t);
        assert!(exit(&smooth, 0.9) > exit(&snappy, 0.9), "smooth exit should be nearly done near the end");
    }

    #[test]
    fn themes_have_light_and_dark_colors() {
        let omarchy = Palette::themed(ThemePreset::Omarchy, true, Accent::Amber);
        assert_eq!((omarchy.window, omarchy.accent), (color(0x1a1b26), color(0x7aa2f7)), "Tokyo Night, its own blue");
        assert_eq!(Palette::themed(ThemePreset::Omarchy, false, Accent::Iris).window, color(0xe1e2e7), "Tokyo Night Day");
        let mocha = Palette::themed(ThemePreset::Catppuccin, true, Accent::Azure);
        assert_eq!((mocha.window, mocha.accent), (color(0x1e1e2e), color(0x89b4fa)));
        assert_eq!(Palette::themed(ThemePreset::Catppuccin, false, Accent::Iris).accent, color(0x8839ef), "Latte mauve");
        assert_eq!(Palette::themed(ThemePreset::MacOs, false, Accent::Azure).accent, color(0x007aff), "system blue");
        assert_eq!(Palette::themed(ThemePreset::Windows, true, Accent::Azure).window, color(0x202020), "Windows 11 dark");
        set_look(ThemePreset::Windows, IconTheme::Accent);
        assert_eq!(look(), (ThemePreset::Windows, IconTheme::Accent), "five themes fit the packed setting");
        set_look(ThemePreset::Default, IconTheme::Colorful);
    }

    #[test]
    fn opacity_makes_panels_see_through_but_keeps_text_solid() {
        let opaque = Palette::new(ColorScheme::Dark, Accent::Iris);
        let glass = opaque.with_opacity(0.6);
        assert!((glass.window.a - 0.6).abs() < 1e-6);
        assert!((glass.sidebar.a - 0.6).abs() < 1e-6);
        assert!(glass.elevated.a > glass.window.a, "raised surfaces stay more solid");
        assert_eq!(glass.text, opaque.text);
        assert_eq!(opaque.with_opacity(1.0).window, opaque.window);
    }
}
