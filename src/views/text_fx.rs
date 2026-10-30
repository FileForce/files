//! Accessibility effects for text: an outline of the opposite color (Text
//! contrast) and a soft pulse for loading text (Shimmer).

use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, ElementId, Hsla, IntoElement, ParentElement as _, SharedString,
    Styled as _, div, px,
};

use crate::palette::Palette;

/// `text` in the surrounding text color; with Text contrast on, a 1px halo
/// of the opposite lightness sits behind it (dark text gets a white edge).
pub(crate) fn outlined(text: impl Into<SharedString>, palette: Palette) -> AnyElement {
    let text: SharedString = text.into();
    let label = || div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis();
    let Some(edge) = palette.text_outline else {
        return label().child(text).into_any_element();
    };
    let mut halo = div().relative().min_w_0();
    for (x, y) in [(-1., 0.), (1., 0.), (0., -1.), (0., 1.), (-1., -1.), (1., 1.), (-1., 1.), (1., -1.)] {
        halo = halo.child(
            label()
                .absolute()
                .top(px(y))
                .left(px(x))
                .w_full()
                .text_color(edge.opacity(if x != 0. && y != 0. { 0.55 } else { 0.9 }))
                .child(text.clone()),
        );
    }
    halo.child(label().relative().child(text)).into_any_element()
}

/// Monospace text that can be selected and copied (a code block).
pub(crate) fn selectable(id: impl Into<ElementId>, text: impl Into<SharedString>) -> AnyElement {
    let text: SharedString = text.into();
    // A fence longer than any run of backticks inside keeps the text literal.
    let fence = "`".repeat(text.split(|char| char != '`').map(str::len).max().unwrap_or(0).max(2) + 1);
    gpui_kit::component::text::TextView::markdown(id, format!("{fence}\n{text}\n{fence}"))
        .selectable(true)
        .into_any_element()
}

/// Loading text; pulses while Shimmer is on.
pub(crate) fn shimmer(id: impl Into<ElementId>, text: impl Into<SharedString>, color: Hsla, palette: Palette) -> AnyElement {
    let text = div().text_color(color).child(text.into());
    if !palette.shimmer {
        return text.into_any_element();
    }
    text.with_animation(
        id,
        Animation::new(std::time::Duration::from_millis(1400)).repeat(),
        |text, delta| {
            // Up and back down again: 0.4 → 1 → 0.4.
            let wave = 1. - (delta * 2. - 1.).abs();
            text.opacity(0.4 + 0.6 * wave)
        },
    )
    .into_any_element()
}
