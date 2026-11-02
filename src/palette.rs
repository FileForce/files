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
        let dark = scheme.is_dark();
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
    Theme::update(cx, |theme| {
        theme.font_size = px(14.);
        theme.radius = px(7.);
        theme.radius_lg = px(11.);

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
