// battery.rs: Battery charge limit via charge_control_end_threshold
// Optional feature: silently skipped when the kernel does not expose the attribute.

use lightomen_common::{validate_charge_limit, CHARGE_LIMIT_MAX};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const POWER_SUPPLY_DIR: &str = "/sys/class/power_supply";
const THRESHOLD_FILE: &str = "charge_control_end_threshold";

pub struct ChargeLimit {
    path: Option<PathBuf>,
    last_applied: Option<u8>,
}

impl ChargeLimit {
    pub fn new() -> Self {
        Self::with_root(Path::new(POWER_SUPPLY_DIR))
    }

    pub fn with_root(root: &Path) -> Self {
        Self {
            path: find_threshold_path(root),
            last_applied: None,
        }
    }

    pub fn is_supported(&self) -> bool {
        self.path.is_some()
    }

    /// Apply the configured limit if it changed. `None` restores 100% once,
    /// but only if we previously wrote a limit, so firmware defaults are left alone.
    pub fn apply(&mut self, limit: Option<u8>) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        let target = match limit {
            Some(pct) => pct,
            None if self.last_applied.is_some() => CHARGE_LIMIT_MAX,
            None => return,
        };
        if self.last_applied == Some(target) && limit.is_some() {
            return;
        }
        match write_threshold(path, target) {
            Ok(()) => self.last_applied = limit,
            Err(e) => {
                eprintln!("lightomen: failed to set charge limit on {}: {}", path.display(), e);
                // Avoid retrying every loop iteration on a persistent error.
                self.last_applied = limit;
            }
        }
    }
}

/// Find the first `BAT*` power supply exposing `charge_control_end_threshold`.
pub fn find_threshold_path(root: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("BAT"))
        .map(|e| e.path().join(THRESHOLD_FILE))
        .filter(|p| p.is_file())
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

/// Validate and write a threshold value.
pub fn write_threshold(path: &Path, pct: u8) -> io::Result<()> {
    let value = validate_charge_limit(pct).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, format!("charge limit {} out of range", pct))
    })?;
    fs::write(path, value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lightomen-bat-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn add_supply(root: &Path, name: &str, with_threshold: bool) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join(THRESHOLD_FILE);
        if with_threshold {
            fs::write(&file, "100").unwrap();
        }
        file
    }

    #[test]
    fn detects_battery_threshold() {
        let root = temp_root("detect");
        add_supply(&root, "AC", true);
        let bat1 = add_supply(&root, "BAT1", true);
        add_supply(&root, "BAT0", false);
        assert_eq!(find_threshold_path(&root), Some(bat1));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_threshold_is_unsupported() {
        let root = temp_root("missing");
        add_supply(&root, "ADP1", true);
        add_supply(&root, "BAT0", false);
        assert_eq!(find_threshold_path(&root), None);
        assert_eq!(find_threshold_path(&root.join("nope")), None);
        let mut cl = ChargeLimit::with_root(&root);
        assert!(!cl.is_supported());
        cl.apply(Some(80)); // must not panic or create files
        assert!(!root.join("BAT0").join(THRESHOLD_FILE).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn write_rejects_out_of_range() {
        let root = temp_root("range");
        let file = add_supply(&root, "BAT0", true);
        assert!(write_threshold(&file, 49).is_err());
        assert!(write_threshold(&file, 101).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "100");
        write_threshold(&file, 82).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "80");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_sets_and_restores() {
        let root = temp_root("apply");
        let file = add_supply(&root, "BAT0", true);
        fs::write(&file, "90").unwrap();
        let mut cl = ChargeLimit::with_root(&root);
        assert!(cl.is_supported());

        cl.apply(None); // never managed: leave firmware value alone
        assert_eq!(fs::read_to_string(&file).unwrap(), "90");

        cl.apply(Some(60));
        assert_eq!(fs::read_to_string(&file).unwrap(), "60");

        cl.apply(None); // disabling restores full charge
        assert_eq!(fs::read_to_string(&file).unwrap(), "100");
        let _ = fs::remove_dir_all(&root);
    }
}
