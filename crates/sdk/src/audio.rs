//! Sound for extensions: a notification chime, a sound from a file or the
//! web, or tracks in the app's music player (Now Playing, media keys).
//! Needs [`Permission::Sound`]; the app plays everything through its
//! bundled VLC, so it works the same on macOS, Windows and Linux.

use std::io;
use std::path::PathBuf;

use crate::log::Level;
use crate::permissions::{Permission, check};

/// Where a sound comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    File(PathBuf),
    /// An `http(s)` address, streamed.
    Url(String),
}

/// A track for the music player.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub source: Source,
}

impl Track {
    pub fn new(title: impl Into<String>, source: Source) -> Self {
        Self { title: title.into(), artist: String::new(), album: String::new(), source }
    }
}

/// What an extension asks the player for.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// The notification chime of this kind, at the user's sound settings.
    Chime(Level),
    /// A one-off sound (not in the queue); `volume` 0…100, or the user's.
    Play { source: Source, volume: Option<u8> },
    /// Tracks into the music player: replacing the queue, or after it.
    Queue { tracks: Vec<Track>, replace: bool },
    Pause,
    Resume,
    Next,
    Previous,
    Stop,
}

fn send(extension: &str, command: Command) -> io::Result<()> {
    check(extension, Permission::Sound)?;
    crate::host().audio(extension, command)
}

/// The notification chime (info, success, warning, error).
pub fn chime(extension: &str, level: Level) -> io::Result<()> {
    send(extension, Command::Chime(level))
}

/// Plays a sound once, beside whatever music plays.
pub fn play(extension: &str, source: Source, volume: Option<u8>) -> io::Result<()> {
    send(extension, Command::Play { source, volume })
}

/// Puts tracks in the music player; `replace` starts them now.
pub fn queue(extension: &str, tracks: Vec<Track>, replace: bool) -> io::Result<()> {
    send(extension, Command::Queue { tracks, replace })
}

pub fn pause(extension: &str) -> io::Result<()> {
    send(extension, Command::Pause)
}

pub fn resume(extension: &str) -> io::Result<()> {
    send(extension, Command::Resume)
}

pub fn next(extension: &str) -> io::Result<()> {
    send(extension, Command::Next)
}

pub fn previous(extension: &str) -> io::Result<()> {
    send(extension, Command::Previous)
}

pub fn stop(extension: &str) -> io::Result<()> {
    send(extension, Command::Stop)
}
