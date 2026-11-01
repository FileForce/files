//! The mini player in Quick Look: VLC's frames, play/pause, seek, mute.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::{
    AppContext as _, Context, Entity, ImageSource, IntoElement, ObjectFit, ParentElement as _, Render, RenderImage, Styled as _,
    StyledImage as _, Subscription, Window, div, img, px,
};

use crate::palette::Palette;
use crate::video::{Player, clock};

/// How often the picture and the time are refreshed (≈30 fps).
const TICK: Duration = Duration::from_millis(33);

pub(crate) struct VideoPlayer {
    pub(crate) path: PathBuf,
    player: Result<Player, String>,
    picture: Option<Arc<RenderImage>>,
    sequence: u64,
    position: i64,
    length: i64,
    playing: bool,
    muted: bool,
    seek: Entity<SliderState>,
    /// While the knob is held, time updates do not move it back.
    seeking: bool,
    palette: Palette,
    _seek_events: Subscription,
}

impl VideoPlayer {
    pub(crate) fn new(path: PathBuf, palette: Palette, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let player = Player::open(&path);
        if let Ok(player) = &player {
            player.play();
        }
        let seek = cx.new(|_| SliderState::new().min(0.).max(1.).step(0.001));
        let seek_events = cx.subscribe(&seek, |this: &mut Self, _, event: &SliderEvent, cx| match event {
            SliderEvent::Change(_) => this.seeking = true,
            SliderEvent::Release(value) => {
                this.seeking = false;
                if let Ok(player) = &this.player {
                    player.seek(value.start());
                }
                cx.notify();
            }
        });
        // Pull new pictures while the player is open; dropping it stops this.
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update_in(cx, |this, window, cx| this.tick(window, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            path,
            playing: player.is_ok(),
            player,
            picture: None,
            sequence: 0,
            position: 0,
            length: 0,
            muted: false,
            seek,
            seeking: false,
            palette,
            _seek_events: seek_events,
        }
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(player) = &self.player else { return };
        let fresh = {
            let frame = player.frame.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            (frame.sequence != self.sequence && frame.width > 0)
                .then(|| (frame.sequence, image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra.clone())))
        };
        let (position, length) = player.progress();
        let playing = player.is_playing();
        let changed = fresh.is_some() || position != self.position || playing != self.playing;
        if let Some((sequence, Some(buffer))) = fresh {
            self.sequence = sequence;
            let picture = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
            // Each picture is a new texture: free the one it replaces.
            if let Some(old) = self.picture.replace(picture) {
                cx.drop_image(old, Some(window));
            }
        }
        self.position = position;
        self.length = length;
        self.playing = playing;
        if !self.seeking && length > 0 {
            let fraction = position as f32 / length as f32;
            self.seek.update(cx, |seek, cx| seek.set_value(fraction, window, cx));
        }
        if changed {
            cx.notify();
        }
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        let Ok(player) = &self.player else { return };
        if self.playing {
            player.pause();
        } else {
            player.play();
        }
        self.playing = !self.playing;
        cx.notify();
    }

    fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        let Ok(player) = &self.player else { return };
        self.muted = !self.muted;
        player.set_muted(self.muted);
        cx.notify();
    }

    pub(crate) fn release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(old) = self.picture.take() {
            cx.drop_image(old, Some(window));
        }
    }
}

impl Render for VideoPlayer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
        if let Err(reason) = &self.player {
            let path = self.path.clone();
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .child(Icon::new(IconName::Film).large().text_color(palette.video))
                .child(div().text_sm().text_color(palette.text).child("Install VLC to play videos here"))
                .child(div().max_w(px(420.)).text_xs().text_color(palette.faint).truncate().child(reason.clone()))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("video-get-vlc")
                                .primary()
                                .small()
                                .icon(IconName::Download)
                                .label("Get VLC")
                                .on_click(|_, _, cx| cx.open_url(crate::video::VLC_DOWNLOAD)),
                        )
                        .child(Button::new("video-open-default").small().label("Open in player").on_click(move |_, _, cx| {
                            cx.open_url(&format!("file://{}", path.display()));
                        })),
                )
                .into_any_element();
        }
        let picture = match &self.picture {
            Some(picture) => img(ImageSource::Render(picture.clone())).size_full().object_fit(ObjectFit::Contain).into_any_element(),
            None => div().text_sm().text_color(palette.faint).child("Starting…").into_any_element(),
        };
        v_flex()
            .size_full()
            .gap_2()
            .child(div().flex_1().min_h_0().rounded_md().overflow_hidden().bg(gpui_kit::black()).flex().items_center().justify_center().child(picture))
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("video-play")
                            .ghost()
                            .small()
                            .icon(if self.playing { IconName::Pause } else { IconName::Play })
                            .tooltip(if self.playing { "Pause" } else { "Play" })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle(cx))),
                    )
                    .child(div().w(px(96.)).text_xs().text_color(palette.muted).child(format!("{} / {}", clock(self.position), clock(self.length))))
                    .child(div().flex_1().child(Slider::new(&self.seek).horizontal()))
                    .child(
                        Button::new("video-mute")
                            .ghost()
                            .small()
                            .icon(if self.muted { IconName::VolumeOff } else { IconName::Volume2 })
                            .tooltip(if self.muted { "Sound on" } else { "Mute" })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_mute(cx))),
                    ),
            )
            .into_any_element()
    }
}
