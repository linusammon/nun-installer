//! Environment checks shown before the install starts.

use std::fs;
use std::path::Path;

/// 2025-01-01T00:00:00Z. A clock before this is certainly wrong on a machine
/// booting today, not merely a little off.
const PLAUSIBLE_EPOCH_FLOOR: u64 = 1_735_689_600;

/// Why a wrong clock matters: HTTPS certificate checks depend on it, so fetches
/// fail with an error that looks like "no network".
pub fn clock_note(now_secs: u64) -> Option<String> {
    (now_secs < PLAUSIBLE_EPOCH_FLOOR).then(|| {
        format!(
            "The system clock looks wrong (it reads {now_secs}s since 1970). \
             This can make downloads fail with a confusing certificate error."
        )
    })
}

pub fn system_clock_note() -> Option<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    clock_note(now)
}

/// `Some(false)` on battery, `Some(true)` on mains power, `None` when there is
/// no battery to speak of (desktop, VM). Reads a sysfs `power_supply` dir.
pub fn on_ac_power(power_supply: &Path) -> Option<bool> {
    let mut has_battery = false;
    let mut mains_online = false;
    for entry in fs::read_dir(power_supply).ok()?.flatten() {
        let dir = entry.path();
        // Wireless mice and keyboards show up as batteries with scope "Device";
        // they say nothing about the machine's own power.
        let scope = fs::read_to_string(dir.join("scope")).unwrap_or_default();
        if scope.trim() == "Device" {
            continue;
        }
        let kind = fs::read_to_string(dir.join("type")).unwrap_or_default();
        match kind.trim() {
            "Battery" => has_battery = true,
            "Mains" | "USB" => {
                let online = fs::read_to_string(dir.join("online")).unwrap_or_default();
                mains_online |= online.trim() == "1";
            }
            _ => {}
        }
    }
    has_battery.then_some(mains_online)
}

pub fn system_on_ac_power() -> Option<bool> {
    on_ac_power(Path::new("/sys/class/power_supply"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn flags_implausible_clocks_only() {
        assert!(clock_note(0).unwrap().contains("clock looks wrong"));
        assert!(clock_note(PLAUSIBLE_EPOCH_FLOOR - 1).is_some());
        assert!(clock_note(PLAUSIBLE_EPOCH_FLOOR).is_none());
        assert!(clock_note(1_800_000_000).is_none());
    }

    fn supply(root: &Path, name: &str, kind: &str, online: Option<&str>) {
        let d = root.join(name);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("type"), format!("{kind}\n")).unwrap();
        if let Some(o) = online {
            fs::write(d.join("online"), format!("{o}\n")).unwrap();
        }
    }

    #[test]
    fn battery_with_mains_unplugged_is_on_battery() {
        let r = TempDir::new().unwrap();
        supply(r.path(), "BAT0", "Battery", None);
        supply(r.path(), "AC", "Mains", Some("0"));
        assert_eq!(on_ac_power(r.path()), Some(false));
    }

    #[test]
    fn battery_with_mains_plugged_is_on_ac_whatever_the_order() {
        let r = TempDir::new().unwrap();
        supply(r.path(), "AC", "Mains", Some("1"));
        supply(r.path(), "BAT0", "Battery", None);
        assert_eq!(on_ac_power(r.path()), Some(true));
    }

    #[test]
    fn peripheral_batteries_are_ignored() {
        let r = TempDir::new().unwrap();
        supply(r.path(), "hidpp_battery_0", "Battery", None);
        fs::write(r.path().join("hidpp_battery_0/scope"), "Device\n").unwrap();
        assert_eq!(
            on_ac_power(r.path()),
            None,
            "a mouse battery is not the machine's battery"
        );
    }

    #[test]
    fn no_battery_means_nothing_to_warn_about() {
        let r = TempDir::new().unwrap();
        supply(r.path(), "AC", "Mains", Some("1"));
        assert_eq!(on_ac_power(r.path()), None);
        assert_eq!(on_ac_power(Path::new("/definitely/not/here")), None);
    }
}
