//! Head tracking for spatial audio: which way the listener's head turned
//! since the track started, from AirPods and Beats on macOS (CoreMotion's
//! headphone motion, macOS 14+). Elsewhere, and without such headphones,
//! the head never turns (yaw 0) and the scene stays fixed to the head.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Radians to the left since [`recenter`], as f32 bits.
static YAW: AtomicU32 = AtomicU32::new(0);
static RUNNING: AtomicBool = AtomicBool::new(false);
static RECENTER: AtomicBool = AtomicBool::new(true);

pub fn yaw() -> f32 {
    f32::from_bits(YAW.load(Ordering::Relaxed))
}

/// Wherever the head faces now becomes straight ahead.
pub fn recenter() {
    RECENTER.store(true, Ordering::Relaxed);
}

/// Follows the head until [`stop`] (does nothing where it can't).
pub fn start() {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    #[cfg(target_os = "macos")]
    std::thread::spawn(macos::follow);
    #[cfg(not(target_os = "macos"))]
    RUNNING.store(false, Ordering::SeqCst);
}

pub fn stop() {
    RUNNING.store(false, Ordering::SeqCst);
    YAW.store(0f32.to_bits(), Ordering::Relaxed);
}

/// Yaw relative to `reference`, wrapped to −π…π.
fn relative(yaw: f32, reference: f32) -> f32 {
    let turned = (yaw - reference + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    if turned.is_finite() { turned } else { 0.0 }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use objc2_core_motion::CMHeadphoneMotionManager;

    /// Polls the headphones' attitude 100 times a second while running.
    pub fn follow() {
        // macOS ends an app that asks for motion without saying why in its
        // Info.plist: a development build (cargo run) has no Info.plist.
        let explained = objc2_foundation::NSBundle::mainBundle().objectForInfoDictionaryKey(&objc2_foundation::NSString::from_str("NSMotionUsageDescription")).is_some();
        if !explained {
            files_sdk::log::record("music", files_sdk::log::Kind::Info, "Head tracking: off outside the app bundle (no NSMotionUsageDescription)", None);
            RUNNING.store(false, Ordering::SeqCst);
            return;
        }
        // SAFETY: CoreMotion's manager is made, polled and stopped on this one thread.
        unsafe {
            let manager = CMHeadphoneMotionManager::new();
            if !manager.isDeviceMotionAvailable() {
                files_sdk::log::record("music", files_sdk::log::Kind::Info, "Head tracking: no headphones with motion sensors", None);
                RUNNING.store(false, Ordering::SeqCst);
                return;
            }
            manager.startDeviceMotionUpdates();
            let mut reference = None;
            while RUNNING.load(Ordering::SeqCst) {
                if let Some(motion) = manager.deviceMotion() {
                    let now = motion.attitude().yaw() as f32;
                    if RECENTER.swap(false, Ordering::Relaxed) || reference.is_none() {
                        reference = Some(now);
                    }
                    YAW.store(relative(now, reference.unwrap_or(now)).to_bits(), Ordering::Relaxed);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            manager.stopDeviceMotionUpdates();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_is_measured_from_the_reference_the_short_way_round() {
        assert!((relative(0.5, 0.2) - 0.3).abs() < 1e-6);
        assert!((relative(-3.0, 3.0) - (std::f32::consts::TAU - 6.0)).abs() < 1e-5, "across ±π, the short way");
        assert_eq!(relative(f32::NAN, 0.0), 0.0);
    }
}
