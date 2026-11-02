use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

use gpui_kit::{App, Entity};

use crate::settings::{AppSettings, PowerSavingMode};

static ON_BATTERY: AtomicBool = AtomicBool::new(false);
/// Battery charge in percent; 255 while unknown (no battery, or not read).
static CHARGE: AtomicU8 = AtomicU8::new(UNKNOWN);
static POWER_SAVING: AtomicBool = AtomicBool::new(false);
static MODE: AtomicU8 = AtomicU8::new(0);
static BELOW: AtomicU8 = AtomicU8::new(100);

const UNKNOWN: u8 = u8::MAX;

/// Where the power comes from, and how full the battery is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Battery {
    pub on_battery: bool,
    pub percent: Option<u8>,
}

/// The last reading, for Settings ("Battery 64%").
pub fn battery() -> Battery {
    let charge = CHARGE.load(Ordering::Relaxed);
    Battery { on_battery: ON_BATTERY.load(Ordering::Relaxed), percent: (charge != UNKNOWN).then_some(charge) }
}

fn store(battery: Battery) {
    ON_BATTERY.store(battery.on_battery, Ordering::Relaxed);
    CHARGE.store(battery.percent.unwrap_or(UNKNOWN), Ordering::Relaxed);
}

pub fn power_saving() -> bool {
    POWER_SAVING.load(Ordering::Relaxed)
}

pub fn power_saving_for(mode: PowerSavingMode, below: u8) -> bool {
    resolves(mode, battery(), below)
}

pub fn interval(normal: Duration, saving: Duration) -> Duration {
    select_interval(normal, saving, power_saving())
}

fn select_interval(normal: Duration, saving: Duration, active: bool) -> Duration {
    if active { saving } else { normal }
}

pub fn set_mode(mode: PowerSavingMode, below: u8) {
    BELOW.store(below.clamp(1, 100), Ordering::Relaxed);
    MODE.store(
        match mode {
            PowerSavingMode::Automatic => 0,
            PowerSavingMode::On => 1,
            PowerSavingMode::Off => 2,
        },
        Ordering::Relaxed,
    );
    update_active();
}

fn mode() -> PowerSavingMode {
    match MODE.load(Ordering::Relaxed) {
        1 => PowerSavingMode::On,
        2 => PowerSavingMode::Off,
        _ => PowerSavingMode::Automatic,
    }
}

/// Automatic: on battery, and (when the charge is known) at or below the
/// threshold; 100 means as soon as the computer runs on battery.
fn resolves(mode: PowerSavingMode, battery: Battery, below: u8) -> bool {
    match mode {
        PowerSavingMode::Automatic => battery.on_battery && (below >= 100 || battery.percent.is_none_or(|percent| percent <= below)),
        PowerSavingMode::On => true,
        PowerSavingMode::Off => false,
    }
}

fn update_active() -> bool {
    let active = resolves(mode(), battery(), BELOW.load(Ordering::Relaxed));
    let changed = POWER_SAVING.swap(active, Ordering::Relaxed) != active;
    if changed {
        let charge = battery().percent.map_or(String::new(), |percent| format!(" · battery {percent}%"));
        files_sdk::log::record(
            "files",
            files_sdk::log::Kind::Setting,
            format!("power saving: {}{charge}", if active { "on" } else { "off" }),
            None,
        );
    }
    changed
}

pub fn refresh() -> bool {
    store(read_battery());
    update_active()
}

pub fn watch(appearance: Entity<AppSettings>, cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(30)).await;
            let reading = cx.background_executor().spawn(async move { read_battery() }).await;
            let changed_reading = reading != battery();
            store(reading);
            // A new charge also redraws Settings' "Battery 64%".
            if update_active() || changed_reading {
                let _ = appearance.update(cx, |_, cx| cx.notify());
            }
        }
    })
    .detach();
}

#[cfg(target_os = "macos")]
fn read_battery() -> Battery {
    files_sdk::log::command("/usr/bin/pmset")
        .args(["-g", "batt"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| parse_pmset(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

/// `pmset -g batt`: "…'Battery Power'… -InternalBattery-0 (id=…)	64%; discharging; …".
#[cfg(target_os = "macos")]
fn parse_pmset(output: &str) -> Battery {
    let battery_line = output.lines().find(|line| line.contains("InternalBattery"));
    Battery {
        on_battery: output.lines().any(|line| line.contains("; discharging;")),
        percent: battery_line
            .and_then(|line| line.split('%').next())
            .and_then(|before| before.rsplit(|c: char| !c.is_ascii_digit()).next())
            .and_then(|digits| digits.parse().ok()),
    }
}

#[cfg(target_os = "linux")]
fn read_battery() -> Battery {
    let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") else {
        return Battery::default();
    };
    let batteries = entries.filter_map(Result::ok).filter_map(|entry| {
        let path = entry.path();
        (std::fs::read_to_string(path.join("type")).ok()?.trim() == "Battery").then(|| {
            (std::fs::read_to_string(path.join("status")).unwrap_or_default(), std::fs::read_to_string(path.join("capacity")).ok())
        })
    });
    linux_battery(batteries)
}

/// (status, capacity) of each battery: on battery when one discharges; the
/// lowest charge counts.
#[cfg(target_os = "linux")]
fn linux_battery(batteries: impl IntoIterator<Item = (String, Option<String>)>) -> Battery {
    let mut battery = Battery::default();
    for (status, capacity) in batteries {
        battery.on_battery |= status.trim().eq_ignore_ascii_case("discharging");
        if let Some(percent) = capacity.and_then(|capacity| capacity.trim().parse::<u8>().ok()) {
            battery.percent = Some(battery.percent.map_or(percent, |lowest| lowest.min(percent)));
        }
    }
    battery
}

#[cfg(windows)]
fn read_battery() -> Battery {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    // SAFETY: GetSystemPowerStatus fills the struct it is given.
    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
        return Battery::default();
    }
    windows_battery(status.ACLineStatus, status.BatteryFlag, status.BatteryLifePercent)
}

/// ACLineStatus 0 = on battery; BatteryFlag 128 = no battery; percent 255 = unknown.
#[cfg(any(windows, test))]
fn windows_battery(ac_line: u8, flag: u8, percent: u8) -> Battery {
    let has_battery = flag != 128 && flag != 255;
    Battery { on_battery: has_battery && ac_line == 0, percent: (has_battery && percent <= 100).then_some(percent) }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn read_battery() -> Battery {
    Battery::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_resolves_against_the_power_source() {
        let battery = |on_battery, percent| Battery { on_battery, percent };
        assert!(!resolves(PowerSavingMode::Automatic, battery(false, Some(10)), 100));
        assert!(resolves(PowerSavingMode::Automatic, battery(true, Some(90)), 100), "100%: whenever on battery");
        assert!(resolves(PowerSavingMode::On, battery(false, None), 20));
        assert!(!resolves(PowerSavingMode::Off, battery(true, Some(5)), 20));
    }

    #[test]
    fn automatic_waits_for_the_threshold() {
        let battery = |percent| Battery { on_battery: true, percent };
        assert!(!resolves(PowerSavingMode::Automatic, battery(Some(64)), 20), "64% is above 20%");
        assert!(resolves(PowerSavingMode::Automatic, battery(Some(20)), 20), "at the threshold");
        assert!(resolves(PowerSavingMode::Automatic, battery(Some(7)), 20));
        assert!(resolves(PowerSavingMode::Automatic, battery(None), 20), "charge unknown: on battery is enough");
        assert!(!resolves(PowerSavingMode::Automatic, Battery { on_battery: false, percent: Some(5) }, 20), "charging");
    }

    #[test]
    fn windows_power_status_reads_battery_and_charge() {
        assert_eq!(windows_battery(0, 1, 64), Battery { on_battery: true, percent: Some(64) });
        assert_eq!(windows_battery(1, 8, 90), Battery { on_battery: false, percent: Some(90) });
        assert_eq!(windows_battery(1, 128, 255), Battery::default(), "a desktop without a battery");
    }

    #[test]
    fn interval_uses_the_saving_value_only_when_active() {
        let normal = Duration::from_millis(400);
        let saving = Duration::from_secs(2);
        assert_eq!(select_interval(normal, saving, false), normal);
        assert_eq!(select_interval(normal, saving, true), saving);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn pmset_distinguishes_battery_from_ac() {
        let on_battery = parse_pmset("Now drawing from 'Battery Power'\n -InternalBattery-0 (id=123)\t80%; discharging; 4:00 remaining");
        assert_eq!(on_battery, Battery { on_battery: true, percent: Some(80) });
        let charging = parse_pmset("Now drawing from 'AC Power'\n -InternalBattery-0 (id=123)\t100%; charged; 0:00 remaining");
        assert_eq!(charging, Battery { on_battery: false, percent: Some(100) });
        assert_eq!(parse_pmset("Now drawing from 'AC Power'\n"), Battery::default(), "a Mac without a battery");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_battery_status_distinguishes_battery_from_ac() {
        assert_eq!(linux_battery([("Discharging\n".into(), Some("42\n".into()))]), Battery { on_battery: true, percent: Some(42) });
        assert!(!linux_battery([("Charging\n".into(), None), ("Full\n".into(), None)]).on_battery);
        assert_eq!(linux_battery([]), Battery::default());
    }
}
