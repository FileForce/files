//! App lock (Settings → General → Lock): Files hides every window behind a
//! lock screen at launch, after a while without use, or when the computer
//! locks its screen or sleeps, and opens again only with the system's own
//! check — Touch ID or the Mac's password, Windows Hello, polkit on Linux.
//! No password of Files' own is kept anywhere.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// When the lock comes on (Settings → General → Lock).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LockSettings {
    pub enabled: bool,
    pub on_launch: bool,
    pub on_screen_lock: bool,
    /// Minutes without using Files; 0: never.
    pub idle_minutes: u32,
}

impl Default for LockSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            on_launch: true,
            on_screen_lock: true,
            idle_minutes: 15,
        }
    }
}

impl LockSettings {
    pub const IDLE_CHOICES: [u32; 6] = [0, 1, 5, 15, 30, 60];
}

static LOCKED: AtomicBool = AtomicBool::new(false);
/// Bumped on every lock and unlock: windows redraw when it moves.
static GENERATION: AtomicU64 = AtomicU64::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static ON_SCREEN_LOCK: AtomicBool = AtomicBool::new(false);
static IDLE_MINUTES: AtomicU32 = AtomicU32::new(0);
/// Seconds since [`start`] at the last key, click or move in a Files window.
static LAST_USE: AtomicU64 = AtomicU64::new(0);
static CHECKING: AtomicBool = AtomicBool::new(false);

fn started() -> Instant {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *START.get_or_init(Instant::now)
}

pub fn locked() -> bool {
    LOCKED.load(Ordering::SeqCst)
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

/// Hides every window behind the lock screen (when the lock is on).
pub fn lock() {
    if ENABLED.load(Ordering::SeqCst) && !LOCKED.swap(true, Ordering::SeqCst) {
        GENERATION.fetch_add(1, Ordering::SeqCst);
        files_sdk::log::record("files", files_sdk::log::Kind::Setting, "Locked", None);
    }
}

fn unlock() {
    if LOCKED.swap(false, Ordering::SeqCst) {
        touch();
        GENERATION.fetch_add(1, Ordering::SeqCst);
        files_sdk::log::record("files", files_sdk::log::Kind::Setting, "Unlocked", None);
    }
}

/// Files was used just now (it is not idle).
pub fn touch() {
    LAST_USE.store(started().elapsed().as_secs(), Ordering::Relaxed);
}

/// Applies the settings; `launching`: locks at once when it should at launch.
pub fn configure(settings: &LockSettings, launching: bool) {
    let enabled = settings.enabled && available();
    ENABLED.store(enabled, Ordering::SeqCst);
    ON_SCREEN_LOCK.store(settings.on_screen_lock, Ordering::SeqCst);
    IDLE_MINUTES.store(settings.idle_minutes, Ordering::SeqCst);
    if !enabled && LOCKED.load(Ordering::SeqCst) {
        // Turned off while locked (another window's settings): open up.
        LOCKED.store(false, Ordering::SeqCst);
        GENERATION.fetch_add(1, Ordering::SeqCst);
    }
    if enabled && launching && settings.on_launch {
        lock();
    }
    if enabled {
        watch();
    }
    touch();
}

/// Checks every two seconds for a locked screen, a sleep, or idle time.
fn watch() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            const TICK: Duration = Duration::from_secs(2);
            let mut screen_was_locked = false;
            let mut wall = SystemTime::now();
            let mut steady = Instant::now();
            loop {
                std::thread::sleep(TICK);
                if !ENABLED.load(Ordering::SeqCst) {
                    continue;
                }
                // The wall clock jumped far past the steady one: the computer slept.
                let wall_passed = SystemTime::now().duration_since(wall).unwrap_or_default();
                let slept = wall_passed > steady.elapsed() + Duration::from_secs(30);
                (wall, steady) = (SystemTime::now(), Instant::now());
                let screen_locked = ON_SCREEN_LOCK.load(Ordering::SeqCst) && screen_is_locked();
                if (screen_locked && !screen_was_locked)
                    || (slept && ON_SCREEN_LOCK.load(Ordering::SeqCst))
                {
                    lock();
                }
                screen_was_locked = screen_locked;
                let idle = IDLE_MINUTES.load(Ordering::SeqCst);
                let since = started()
                    .elapsed()
                    .as_secs()
                    .saturating_sub(LAST_USE.load(Ordering::Relaxed));
                if idle > 0 && since >= u64::from(idle) * 60 {
                    lock();
                }
            }
        });
    });
}

/// Asks the system who is there (in the background); unlocks when it says
/// it is the owner. `done` gets the outcome: `Ok(true)` unlocked, `Ok(false)`
/// cancelled, `Err` the check failed.
pub fn check(reason: String, done: impl FnOnce(Result<bool, String>) + Send + 'static) {
    if CHECKING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let result = authenticate(&reason);
        if result == Ok(true) {
            unlock();
        }
        CHECKING.store(false, Ordering::SeqCst);
        done(result);
    });
}

/// Whether a lock screen may ask by itself for lock `generation`: the first
/// window to ask takes it, so a cancel is not answered by another window asking.
pub fn auto_ask(generation: u64) -> bool {
    static ASKED: AtomicU64 = AtomicU64::new(u64::MAX);
    ASKED.swap(generation, Ordering::SeqCst) != generation
}

pub fn checking() -> bool {
    CHECKING.load(Ordering::SeqCst)
}

/// Whether this system can check who is there (else the lock stays off:
/// it could never be opened again).
pub fn available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(platform::available)
}

fn authenticate(reason: &str) -> Result<bool, String> {
    platform::authenticate(reason)
}

fn screen_is_locked() -> bool {
    platform::screen_is_locked()
}

#[cfg(target_os = "macos")]
mod platform {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAPolicy};

    pub fn available() -> bool {
        // SAFETY: a fresh context, asked and dropped here.
        unsafe {
            LAContext::new()
                .canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthentication)
                .is_ok()
        }
    }

    pub fn authenticate(reason: &str) -> Result<bool, String> {
        let (sender, receiver) = std::sync::mpsc::channel();
        // SAFETY: the context lives until the reply arrives (we wait for it below).
        let context = unsafe { LAContext::new() };
        let reply = RcBlock::new(move |success: Bool, error: *mut NSError| {
            let outcome = if success.as_bool() {
                Ok(true)
            } else if error.is_null() {
                Ok(false)
            } else {
                // SAFETY: LocalAuthentication hands a live NSError on failure.
                let error = unsafe { &*error };
                // LAErrorUserCancel (-2), SystemCancel (-4), AppCancel (-9): not a failure.
                if matches!(error.code(), -2 | -4 | -9) {
                    Ok(false)
                } else {
                    Err(error.localizedDescription().to_string())
                }
            };
            let _ = sender.send(outcome);
        });
        // SAFETY: the reply block is retained by LocalAuthentication until called.
        unsafe {
            context.evaluatePolicy_localizedReason_reply(
                LAPolicy::DeviceOwnerAuthentication,
                &NSString::from_str(reason),
                &reply,
            )
        };
        receiver
            .recv()
            .unwrap_or_else(|_| Err("No answer from the system".into()))
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGSessionCopyCurrentDictionary() -> core_foundation::dictionary::CFDictionaryRef;
    }

    /// The login window's "screen is locked" flag of this session.
    pub fn screen_is_locked() -> bool {
        use core_foundation::base::TCFType as _;
        // SAFETY: a copied dictionary, wrapped so it is released.
        unsafe {
            let raw = CGSessionCopyCurrentDictionary();
            if raw.is_null() {
                return false;
            }
            let session: core_foundation::dictionary::CFDictionary<
                core_foundation::string::CFString,
                core_foundation::base::CFType,
            > = core_foundation::dictionary::CFDictionary::wrap_under_create_rule(raw);
            session
                .find(core_foundation::string::CFString::from_static_string(
                    "CGSSessionScreenIsLocked",
                ))
                .and_then(|value| value.downcast::<core_foundation::boolean::CFBoolean>())
                .is_some_and(bool::from)
        }
    }
}

#[cfg(windows)]
mod platform {
    use windows::Security::Credentials::UI::{
        UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
    };
    use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
    use windows::core::{HSTRING, factory};

    pub fn available() -> bool {
        UserConsentVerifier::CheckAvailabilityAsync()
            .and_then(|operation| operation.join())
            .is_ok_and(|availability| availability == UserConsentVerifierAvailability::Available)
    }

    pub fn authenticate(reason: &str) -> Result<bool, String> {
        // SAFETY: plain Win32 / WinRT calls; the window is the one in front (ours: it was just clicked).
        unsafe {
            let window = windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow();
            let interop: IUserConsentVerifierInterop =
                factory::<UserConsentVerifier, IUserConsentVerifierInterop>()
                    .map_err(|error| error.to_string())?;
            let operation: windows_future::IAsyncOperation<UserConsentVerificationResult> = interop
                .RequestVerificationForWindowAsync(window, &HSTRING::from(reason))
                .map_err(|error| error.to_string())?;
            match operation.join().map_err(|error| error.to_string())? {
                UserConsentVerificationResult::Verified => Ok(true),
                UserConsentVerificationResult::Canceled
                | UserConsentVerificationResult::RetriesExhausted => Ok(false),
                other => Err(format!("Windows Hello: {other:?}")),
            }
        }
    }

    /// The secure desktop of a locked session takes the input desktop away.
    pub fn screen_is_locked() -> bool {
        use windows_sys::Win32::System::StationsAndDesktops::{
            CloseDesktop, DESKTOP_SWITCHDESKTOP, OpenInputDesktop, SwitchDesktop,
        };
        // SAFETY: the desktop handle is closed right after the test.
        unsafe {
            let desktop = OpenInputDesktop(0, 0, DESKTOP_SWITCHDESKTOP);
            if desktop.is_null() {
                return true;
            }
            let switched = SwitchDesktop(desktop) != 0;
            CloseDesktop(desktop);
            !switched
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    /// polkit's own dialog asks for the user's password (`pkexec`).
    pub fn available() -> bool {
        which("pkexec")
    }

    fn which(program: &str) -> bool {
        std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
    }

    pub fn authenticate(_reason: &str) -> Result<bool, String> {
        let status = std::process::Command::new("pkexec")
            .arg("true")
            .status()
            .map_err(|error| error.to_string())?;
        // 126: the dialog was dismissed; 127: not authorized.
        match status.code() {
            Some(0) => Ok(true),
            Some(126) | Some(127) => Ok(false),
            other => Err(format!("pkexec ended with {other:?}")),
        }
    }

    /// systemd-logind's LockedHint for this session.
    pub fn screen_is_locked() -> bool {
        let session = std::env::var("XDG_SESSION_ID").unwrap_or_else(|_| "auto".into());
        std::process::Command::new("loginctl")
            .args(["show-session", &session, "-p", "LockedHint", "--value"])
            .output()
            .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "yes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_comes_on_only_when_enabled_and_goes_with_the_setting() {
        ENABLED.store(false, Ordering::SeqCst);
        lock();
        assert!(!locked(), "off: never locks");
        ENABLED.store(true, Ordering::SeqCst);
        let before = generation();
        lock();
        assert!(locked() && generation() > before, "on: locks and redraws");
        configure(
            &LockSettings {
                enabled: false,
                ..Default::default()
            },
            false,
        );
        assert!(!locked(), "turning it off opens up");
    }

    #[test]
    fn a_lock_asks_by_itself_once_and_then_waits_for_unlock() {
        assert!(auto_ask(1_000), "the first lock screen asks at once");
        assert!(
            !auto_ask(1_000),
            "after a cancel: no second prompt for the same lock"
        );
        assert!(auto_ask(1_001), "the next lock asks again");
    }
}

#[cfg(test)]
mod system {
    /// What this computer offers (`cargo test -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn reports_the_system_check() {
        println!(
            "available: {}, screen locked: {}",
            super::available(),
            super::screen_is_locked()
        );
    }
}
