//! What the app uses, for the developer tools (Console → Resources): the
//! process's memory and CPU over the last minutes, and what the app keeps
//! in its caches. macOS reports the same "memory" as Activity Monitor
//! (physical footprint); Windows and Linux report the resident set.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

/// One sample a second, two minutes kept.
pub const EVERY: Duration = Duration::from_secs(1);
const KEPT: usize = 120;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    /// Bytes.
    pub memory: u64,
    /// Percent of one core (200 = two cores busy).
    pub cpu: f32,
}

struct Sampler {
    system: sysinfo::System,
    pid: sysinfo::Pid,
    history: VecDeque<Sample>,
}

static SAMPLER: Mutex<Option<Sampler>> = Mutex::new(None);

/// The physical footprint, as Activity Monitor shows it.
#[cfg(target_os = "macos")]
fn footprint() -> Option<u64> {
    // SAFETY: proc_pid_rusage fills the struct for our own pid.
    unsafe {
        let mut info: libc::rusage_info_v2 = std::mem::zeroed();
        let result = libc::proc_pid_rusage(std::process::id() as i32, libc::RUSAGE_INFO_V2, (&mut info as *mut libc::rusage_info_v2).cast());
        (result == 0).then_some(info.ri_phys_footprint)
    }
}

#[cfg(not(target_os = "macos"))]
fn footprint() -> Option<u64> {
    None
}

/// Takes a sample (call every [`EVERY`]); CPU needs two to compare.
pub fn sample() -> Sample {
    let mut sampler = SAMPLER.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let sampler = sampler.get_or_insert_with(|| Sampler {
        system: sysinfo::System::new(),
        pid: sysinfo::Pid::from_u32(std::process::id()),
        history: VecDeque::new(),
    });
    sampler.system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[sampler.pid]),
        sysinfo::ProcessRefreshKind::new().with_memory().with_cpu(),
    );
    let process = sampler.system.process(sampler.pid);
    let sample = Sample {
        memory: footprint().or_else(|| process.map(|process| process.memory())).unwrap_or(0),
        cpu: process.map_or(0., |process| process.cpu_usage()),
    };
    if sampler.history.len() == KEPT {
        sampler.history.pop_front();
    }
    sampler.history.push_back(sample);
    sample
}

/// The samples so far, oldest first.
pub fn history() -> Vec<Sample> {
    SAMPLER.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref().map(|sampler| sampler.history.iter().copied().collect()).unwrap_or_default()
}

/// One line of "what the app keeps".
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub name: &'static str,
    pub count: String,
    /// Bytes, when it can be told.
    pub bytes: Option<u64>,
    pub note: &'static str,
}

pub(crate) fn folder_size(dir: &std::path::Path) -> (usize, u64) {
    let mut files = 0;
    let mut bytes = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            match entry.metadata() {
                Ok(metadata) if metadata.is_dir() => stack.push(entry.path()),
                Ok(metadata) => {
                    files += 1;
                    bytes += metadata.len();
                }
                Err(_) => {}
            }
        }
    }
    (files, bytes)
}

/// Rough size of a file entry: the struct and its strings.
pub(crate) const ENTRY_BYTES: u64 = 400;

/// What the app holds, part by part (in memory, then on disk).
pub fn parts() -> Vec<Part> {
    let (known, made, listings, search) = crate::model::index_stats();
    let (mail, mail_bytes) = files_ext_mail::cache_stats();
    let events = files_sdk::log::events().len();
    let notifications = files_sdk::log::notifications().len();
    let players = crate::video::OPEN_PLAYERS.load(std::sync::atomic::Ordering::Relaxed);
    let tracks = files_ext_music::local::library_len();
    let thumbnails = crate::thumbnails::cache_dir().map(|dir| folder_size(&dir)).unwrap_or_default();
    let music_cache = folder_size(&files_ext_music::cache_dir());
    vec![
        Part {
            name: "File entries",
            count: format!("{known} known · {made} made since launch"),
            bytes: Some(made * ENTRY_BYTES),
            note: "Kept until quit; \"made\" climbing fast while nothing is browsed is a leak",
        },
        Part { name: "Folder listings", count: format!("{listings} folders"), bytes: None, note: "Cached briefly, then read again" },
        Part { name: "Global search index", count: format!("{search} items"), bytes: Some(search as u64 * 8), note: "The ⌘K folder, a few levels deep" },
        Part { name: "Mail in memory", count: format!("{mail} messages"), bytes: Some(mail_bytes), note: "Text and attachments of opened mail" },
        Part { name: "Music library", count: format!("{tracks} tracks"), bytes: Some(tracks as u64 * 300), note: "Read when the Music tab opens" },
        Part { name: "Media players", count: format!("{players} open"), bytes: None, note: "VLC: a video preview or the music player" },
        Part { name: "Console journal", count: format!("{events} events · {notifications} notifications"), bytes: None, note: "Capped" },
        Part { name: "Thumbnails on disk", count: format!("{} files", thumbnails.0), bytes: Some(thumbnails.1), note: "Disk, not memory" },
        Part { name: "Music cache on disk", count: format!("{} files", music_cache.0), bytes: Some(music_cache.1), note: "Downloaded tracks and covers; disk, not memory" },
    ]
}

/// "1.2 GB", "340 MB", "12 KB".
pub fn size(bytes: u64) -> String {
    match bytes {
        bytes if bytes >= 1 << 30 => format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64),
        bytes if bytes >= 1 << 20 => format!("{:.0} MB", bytes as f64 / (1u64 << 20) as f64),
        bytes => format!("{} KB", bytes / 1024),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_process_is_measured_and_parts_listed() {
        let first = sample();
        assert!(first.memory > 1 << 20, "our own memory is known: {}", first.memory);
        std::thread::sleep(Duration::from_millis(200));
        let second = sample();
        assert!(second.cpu >= 0.);
        assert!(history().len() >= 2);
        let parts = parts();
        assert!(parts.iter().any(|part| part.name == "File entries"));
        assert_eq!(size(3 << 30), "3.0 GB");
        assert_eq!(size(5 << 20), "5 MB");
    }
}
