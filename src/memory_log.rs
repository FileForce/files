//! Once an hour, how much memory Files takes and what holds it, in the log
//! (DevTools → Log): after days open, the line that grew says what leaked.

use std::time::Duration;

/// The process's physical footprint in bytes (what Activity Monitor shows
/// as Memory on macOS; the resident set elsewhere).
pub fn footprint() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
        // SAFETY: our own pid and a buffer of the flavor's size.
        let done = unsafe {
            libc::proc_pid_rusage(
                std::process::id() as i32,
                libc::RUSAGE_INFO_V2,
                (&mut info as *mut libc::rusage_info_v2).cast(),
            )
        };
        (done == 0).then_some(info.ri_phys_footprint)
    }
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
        Some(line.split_whitespace().nth(1)?.parse::<u64>().ok()? * 1024)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// One line: the footprint and the sizes of the caches that could grow.
pub fn summary() -> String {
    let (entries, made, listings, search) = crate::model::index_stats();
    let live = crate::model::LIVE_ENTRIES.load(std::sync::atomic::Ordering::Relaxed);
    let allocs = crate::alloc_stats::counts();
    format!(
        "Memory {} · heap blocks {} alive · file entries {live} alive ({entries} indexed, {made} made) · {listings} folder listings · search {search} · details {} · extension previews {}",
        footprint().map_or_else(|| "?".to_string(), crate::resources::size),
        allocs.allocs - allocs.frees,
        crate::media_info::cached(),
        crate::ext_preview::cached(),
    )
}

pub fn start() {
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(Duration::from_secs(60 * 60));
            files_sdk::log::record("files", files_sdk::log::Kind::Info, summary(), None);
        }
    });
}
