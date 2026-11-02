//! The system's media controls for Music: the keyboard's play/next keys,
//! headphone buttons, and "Now Playing" (macOS Control Center, the Windows
//! media flyout, MPRIS on Linux) reach Files while it plays.

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection};

/// What the keys asked for, handled by the player on the UI thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
    /// Seconds, forward (+) or back (−).
    SeekBy(i64),
    /// Milliseconds from the start.
    SeekTo(u64),
}

pub fn command(event: MediaControlEvent) -> Option<Command> {
    Some(match event {
        MediaControlEvent::Play => Command::Play,
        MediaControlEvent::Pause => Command::Pause,
        MediaControlEvent::Toggle => Command::Toggle,
        MediaControlEvent::Next => Command::Next,
        MediaControlEvent::Previous => Command::Previous,
        MediaControlEvent::Stop => Command::Stop,
        MediaControlEvent::Seek(SeekDirection::Forward) => Command::SeekBy(10),
        MediaControlEvent::Seek(SeekDirection::Backward) => Command::SeekBy(-10),
        MediaControlEvent::SeekBy(SeekDirection::Forward, by) => Command::SeekBy(by.as_secs() as i64),
        MediaControlEvent::SeekBy(SeekDirection::Backward, by) => Command::SeekBy(-(by.as_secs() as i64)),
        MediaControlEvent::SetPosition(MediaPosition(at)) => Command::SeekTo(at.as_millis() as u64),
        _ => return None,
    })
}

/// The system controls, once Music has played something.
pub struct SystemControls {
    controls: MediaControls,
    pub commands: Receiver<Command>,
}

impl SystemControls {
    /// `hwnd`: the main window, which Windows ties the controls to.
    pub fn new(hwnd: Option<*mut std::ffi::c_void>) -> Result<Self, String> {
        let config = PlatformConfig { display_name: "file FORCE", dbus_name: "file_force", hwnd };
        let mut controls = MediaControls::new(config).map_err(|error| format!("{error:?}"))?;
        let (sender, commands) = channel();
        controls
            .attach(move |event| {
                if let Some(command) = command(event) {
                    let _ = sender.send(command);
                }
            })
            .map_err(|error| format!("{error:?}"))?;
        files_sdk::log::record("music", files_sdk::log::Kind::Info, "System media keys now control Music".to_string(), None);
        Ok(Self { controls, commands })
    }

    /// Title, artist, album, cover (a cached file) and length.
    pub fn show(&mut self, title: &str, artist: &str, album: &str, cover: Option<&std::path::Path>, length_ms: u64) {
        let cover_url = cover.map(|path| format!("file://{}", path.display()));
        let _ = self.controls.set_metadata(MediaMetadata {
            title: Some(title),
            artist: (!artist.is_empty()).then_some(artist),
            album: (!album.is_empty()).then_some(album),
            cover_url: cover_url.as_deref(),
            duration: (length_ms > 0).then(|| Duration::from_millis(length_ms)),
        });
    }

    pub fn state(&mut self, playing: Option<bool>, position_ms: u64) {
        let progress = Some(MediaPosition(Duration::from_millis(position_ms)));
        let _ = self.controls.set_playback(match playing {
            None => MediaPlayback::Stopped,
            Some(true) => MediaPlayback::Playing { progress },
            Some(false) => MediaPlayback::Paused { progress },
        });
    }
}

/// The main window's HWND (Windows needs it); `None` elsewhere.
pub fn hwnd(cx: &mut gpui_kit::App) -> Option<*mut std::ffi::c_void> {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
        let window = cx.active_window().or_else(|| cx.windows().first().copied())?;
        return window
            .update(cx, |_, window, _| match window.window_handle().ok()?.as_raw() {
                RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as *mut std::ffi::c_void),
                _ => None,
            })
            .ok()
            .flatten();
    }
    #[cfg(not(windows))]
    {
        let _ = cx;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_become_player_commands() {
        assert_eq!(command(MediaControlEvent::Toggle), Some(Command::Toggle));
        assert_eq!(command(MediaControlEvent::Next), Some(Command::Next));
        assert_eq!(command(MediaControlEvent::Seek(SeekDirection::Backward)), Some(Command::SeekBy(-10)));
        assert_eq!(command(MediaControlEvent::SetPosition(MediaPosition(Duration::from_secs(65)))), Some(Command::SeekTo(65_000)));
        assert_eq!(command(MediaControlEvent::SetVolume(0.5)), None, "volume stays in the app");
    }
}
