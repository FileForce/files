//! The equalizer, one panel for Settings → Sound and the Music tab: VLC's
//! presets as tags, ten bands and the preamp. It is kept in music.toml and
//! heard at once by whatever plays (music, song previews).

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};

use crate::app::AppearanceSettings;
use crate::palette::Palette;

pub(crate) struct EqualizerPanel {
    appearance: Entity<AppearanceSettings>,
    /// Preamp, then one slider per band.
    sliders: Vec<Entity<SliderState>>,
    _subscriptions: Vec<Subscription>,
}

impl EqualizerPanel {
    pub(crate) fn new(appearance: Entity<AppearanceSettings>, cx: &mut Context<Self>) -> Self {
        let saved = files_ext_music::config().equalizer;
        let sliders: Vec<Entity<SliderState>> = (0..=10)
            .map(|index| {
                let value = saved.as_ref().map_or(0., |saved| if index == 0 { saved.preamp } else { saved.bands.get(index - 1).copied().unwrap_or(0.) });
                cx.new(|_| SliderState::new().min(-20.).max(20.).step(0.5).default_value(value))
            })
            .collect();
        let mut subscriptions = vec![cx.observe(&appearance, |_, _, cx| cx.notify())];
        for slider in &sliders {
            subscriptions.push(cx.subscribe(slider, |this: &mut Self, _, event: &SliderEvent, cx| {
                if let SliderEvent::Release(_) = event {
                    // Moved by hand: no longer the preset it came from.
                    this.save(None, cx);
                }
            }));
        }
        Self { appearance, sliders, _subscriptions: subscriptions }
    }

    fn save(&mut self, preset: Option<String>, cx: &mut Context<Self>) {
        let values: Vec<f32> = self.sliders.iter().map(|slider| slider.read(cx).value().start()).collect();
        let mut config = files_ext_music::config();
        config.equalizer = Some(files_ext_music::EqualizerConfig { preset, preamp: values[0], bands: values[1..].to_vec() });
        let _ = files_ext_music::save_config(&config);
        crate::views::music::sound_changed(cx);
        cx.notify();
    }

    fn turn_off(&mut self, cx: &mut Context<Self>) {
        let mut config = files_ext_music::config();
        config.equalizer = None;
        let _ = files_ext_music::save_config(&config);
        crate::views::music::sound_changed(cx);
        cx.notify();
    }

    fn use_preset(&mut self, index: usize, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(values) = crate::video::equalizer_preset(index) else { return };
        for (slot, value) in std::iter::once(values.preamp).chain(values.bands).enumerate() {
            if let Some(slider) = self.sliders.get(slot) {
                slider.update(cx, |slider, cx| slider.set_value(value, window, cx));
            }
        }
        self.save(Some(name), cx);
    }
}

impl Render for EqualizerPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::from_settings(self.appearance.read(cx));
        let config = files_ext_music::config();
        let on = config.equalizer.is_some();
        let current = config.equalizer.and_then(|equalizer| equalizer.preset);
        // Presets as tags: small, wrapping, the chosen one filled.
        let presets = crate::video::equalizer_presets().into_iter().enumerate().map(|(index, name)| {
            let selected = current.as_deref() == Some(name.as_str());
            div()
                .id(("eq-preset", index))
                .px_2()
                .py_0p5()
                .rounded_full()
                .border_1()
                .text_xs()
                .cursor_pointer()
                .border_color(if selected { palette.accent } else { palette.border })
                .bg(if selected { palette.accent_soft } else { palette.window })
                .text_color(if selected { palette.accent } else { palette.muted })
                .hover(|style| style.border_color(palette.accent).text_color(palette.text))
                .child(name.clone())
                .on_click(cx.listener(move |this, _, window, cx| this.use_preset(index, name.clone(), window, cx)))
        });
        let labels = std::iter::once("Pre".to_string()).chain(crate::video::equalizer_bands().into_iter().map(|hz| {
            if hz >= 1000. { format!("{}k", (hz / 1000.).round()) } else { format!("{}", hz.round()) }
        }));
        // The bands spread over the whole width.
        let sliders = self.sliders.iter().zip(labels).enumerate().map(|(index, (slider, label))| {
            v_flex()
                .flex_1()
                .min_w(px(28.))
                .gap_1()
                .items_center()
                .when(index == 0, |this| this.border_r_1().border_color(palette.border))
                .child(div().text_size(px(10.)).text_color(palette.faint).child(format!("{:+.0}", slider.read(cx).value().start())))
                .child(div().h(px(96.)).child(Slider::new(slider).vertical()))
                .child(div().text_size(px(10.)).text_color(palette.muted).child(label))
        });
        let available = !crate::video::equalizer_bands().is_empty();
        v_flex()
            .w_full()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(Icon::new(IconName::SlidersVertical).small().text_color(palette.accent))
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Equalizer"))
                    .child(div().text_xs().text_color(palette.muted).child(if on { "on" } else { "off" }))
                    .child(div().flex_1())
                    .when(on, |this| this.child(Button::new("eq-off").ghost().xsmall().label("Turn off").on_click(cx.listener(|this, _, _, cx| this.turn_off(cx))))),
            )
            .when(!available, |this| this.child(div().text_xs().text_color(palette.faint).child("The equalizer needs VLC, which ships with Files")))
            .child(h_flex().w_full().gap_1().flex_wrap().children(presets))
            .child(h_flex().w_full().children(sliders))
    }
}
