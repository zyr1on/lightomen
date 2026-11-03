// hwmon.rs: Persistent-FD Hardware Backend
// Pure std, zero tokio/serde, zero glob overhead, seek(0) stack buffer reads.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub struct HwmonBackend {
    cpu_temp_file: Option<File>,
    fan1_input_file: Option<File>,
    fan2_input_file: Option<File>,
    fan1_target_path: Option<PathBuf>,
    fan2_target_path: Option<PathBuf>,
    fan_pwm_enable_path: Option<PathBuf>,
    fan_pwm_duty_path: Option<PathBuf>,
    platform_profile_path: Option<PathBuf>,
    gpu_tgp_path: Option<PathBuf>,
    gpu_ppab_path: Option<PathBuf>,
    gpu_mux_path: Option<PathBuf>,
    graphics_mode_path: Option<PathBuf>,
    rapl_energy_file: Option<File>,
    last_energy_uj: u64,
    last_energy_time: Instant,
    gpu_pci_status_file: Option<File>,
    pub max_rpm: u16,
    pub board_id: [u8; 8],
    pub last_applied_mode: Option<u8>,
    last_target1: u16,
    last_target2: u16,
}

impl HwmonBackend {
    pub fn new() -> Self {
        // Ensure kernel modules are active before discovering hardware sysfs nodes
        let _ = std::process::Command::new("/sbin/modprobe")
            .arg("hp_wmi")
            .status();
        let _ = std::process::Command::new("/sbin/modprobe")
            .arg("hp_omen_extra")
            .status();

        let board_id = Self::read_board_id();
        let paths = Self::discover_hwmon_paths();

        let cpu_temp_file = paths.cpu_temp.and_then(|p| File::open(p).ok());
        let fan1_input_file = paths.fan1_input.and_then(|p| File::open(p).ok());
        let fan2_input_file = paths.fan2_input.and_then(|p| File::open(p).ok());

        let fan1_target_path = paths.fan1_target;
        let fan2_target_path = paths.fan2_target;
        let fan_pwm_enable_path = paths.fan_pwm_enable;
        let fan_pwm_duty_path = paths.fan_pwm_duty;

        let platform_profile_path = {
            let p = PathBuf::from("/sys/firmware/acpi/platform_profile");
            if p.exists() { Some(p) } else { None }
        };

        let gpu_tgp_path = {
            let p = PathBuf::from("/sys/devices/platform/hp-wmi/gpu_tgp");
            if p.exists() { Some(p) } else { None }
        };

        let gpu_ppab_path = {
            let p = PathBuf::from("/sys/devices/platform/hp-wmi/gpu_ppab");
            if p.exists() { Some(p) } else { None }
        };

        let gpu_mux_path = {
            let p = PathBuf::from("/sys/devices/platform/hp-wmi/gpu_mux_mode");
            if p.exists() { Some(p) } else { None }
        };

        let graphics_mode_path = {
            let p = PathBuf::from("/sys/devices/platform/hp-wmi/graphics_mode");
            if p.exists() { Some(p) } else { None }
        };

        let max_rpm = paths.max_rpm.unwrap_or(5800).clamp(4000, 7000);

        let rapl_energy_file = {
            let p1 = PathBuf::from("/sys/class/powercap/intel-rapl:0/energy_uj");
            let p2 = PathBuf::from("/sys/devices/virtual/powercap/intel-rapl/intel-rapl:0/energy_uj");
            File::open(p1).or_else(|_| File::open(p2)).ok()
        };

        let gpu_pci_status_file = {
            let p = PathBuf::from("/sys/bus/pci/devices/0000:01:00.0/power/runtime_status");
            if p.exists() {
                File::open(p).ok()
            } else {
                let mut found = None;
                if let Ok(entries) = fs::read_dir("/sys/bus/pci/devices") {
                    for entry in entries.flatten() {
                        let path = entry.path().join("power/runtime_status");
                        let class_path = entry.path().join("class");
                        if path.exists() && class_path.exists() {
                            if let Ok(c) = fs::read_to_string(&class_path) {
                                if c.trim().starts_with("0x03") {
                                    let vendor_path = entry.path().join("vendor");
                                    if let Ok(v) = fs::read_to_string(&vendor_path) {
                                        if v.trim().contains("0x10de") {
                                            found = File::open(path).ok();
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                found
            }
        };

        Self {
            cpu_temp_file,
            fan1_input_file,
            fan2_input_file,
            fan1_target_path,
            fan2_target_path,
            fan_pwm_enable_path,
            fan_pwm_duty_path,
            platform_profile_path,
            gpu_tgp_path,
            gpu_ppab_path,
            gpu_mux_path,
            graphics_mode_path,
            rapl_energy_file,
            last_energy_uj: 0,
            last_energy_time: Instant::now(),
            gpu_pci_status_file,
            max_rpm,
            board_id,
            last_applied_mode: None,
            last_target1: 0,
            last_target2: 0,
        }
    }

    fn read_board_id() -> [u8; 8] {
        let mut id = [0u8; 8];
        if let Ok(mut f) = File::open("/sys/class/dmi/id/board_name") {
            let mut buf = [0u8; 16];
            if let Ok(n) = f.read(&mut buf) {
                let trimmed = buf[..n].trim_ascii();
                let len = trimmed.len().min(7);
                id[..len].copy_from_slice(&trimmed[..len]);
            }
        }
        if id[0] == 0 {
            id[..4].copy_from_slice(b"8BD4");
        }
        id
    }

    fn discover_hwmon_paths() -> DiscoveredPaths {
        let mut res = DiscoveredPaths::default();

        if let Ok(entries) = fs::read_dir("/sys/class/hwmon") {
            for entry in entries.flatten() {
                let p = entry.path();
                let name_file = p.join("name");
                if let Ok(name_str) = fs::read_to_string(&name_file) {
                    let name = name_str.trim();
                    if name == "k10temp" || name == "coretemp" || name == "zenpower" {
                        let t = p.join("temp1_input");
                        if t.exists() {
                            res.cpu_temp = Some(t);
                        }
                    } else if name == "hp" || name == "hp_wmi" || name == "omen" {
                        let f1 = p.join("fan1_input");
                        let f2 = p.join("fan2_input");
                        let t1 = p.join("fan1_target");
                        let t2 = p.join("fan2_target");
                        let m1 = p.join("fan1_max");
                        let pwm_en = p.join("pwm1_enable");
                        let pwm_duty = p.join("pwm1");

                        if f1.exists() {
                            res.fan1_input = Some(f1);
                        }
                        if f2.exists() {
                            res.fan2_input = Some(f2);
                        }
                        if t1.exists() {
                            res.fan1_target = Some(t1);
                        }
                        if t2.exists() {
                            res.fan2_target = Some(t2);
                        }
                        if m1.exists() {
                            if let Ok(m_str) = fs::read_to_string(&m1) {
                                if let Ok(val) = m_str.trim().parse::<u16>() {
                                    if val >= 4000 {
                                        res.max_rpm = Some(val);
                                    }
                                }
                            }
                        }
                        if pwm_en.exists() {
                            res.fan_pwm_enable = Some(pwm_en);
                        }
                        if pwm_duty.exists() {
                            res.fan_pwm_duty = Some(pwm_duty);
                        }
                    }
                }
            }
        }

        res
    }

    fn read_fd_number(file_opt: &mut Option<File>) -> Option<i64> {
        let file = file_opt.as_mut()?;
        if file.seek(SeekFrom::Start(0)).is_err() {
            return None;
        }
        let mut buf = [0u8; 64];
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        let s = std::str::from_utf8(&buf[..n]).ok()?;
        s.trim().parse::<i64>().ok()
    }

    fn write_sysfs_string(path_opt: &Option<PathBuf>, s: &str) -> bool {
        if let Some(path) = path_opt {
            if fs::write(path, s.as_bytes()).is_ok() {
                return true;
            }
        }
        false
    }

    pub fn read_cpu_temp(&mut self) -> Option<u16> {
        Self::read_fd_number(&mut self.cpu_temp_file).map(|milli| {
            let c = (milli / 1000) as u16;
            if c > 0 && c < 125 {
                c
            } else {
                0
            }
        })
    }

    /// Check if NVIDIA dGPU is currently in D3cold (suspended) power-saving sleep.
    /// Safely inspects Linux kernel runtime PM in memory without waking the PCIe bus.
    pub fn is_gpu_suspended(&mut self) -> bool {
        if let Some(ref mut f) = self.gpu_pci_status_file {
            if f.seek(SeekFrom::Start(0)).is_ok() {
                let mut buf = [0u8; 32];
                if let Ok(n) = f.read(&mut buf) {
                    let s = std::str::from_utf8(&buf[..n]).unwrap_or("");
                    return s.trim() == "suspended";
                }
            }
        } else {
            // Lazy fallback open if not open at initialization
            let p = PathBuf::from("/sys/bus/pci/devices/0000:01:00.0/power/runtime_status");
            if let Ok(mut f) = File::open(&p) {
                let mut buf = [0u8; 32];
                if let Ok(n) = f.read(&mut buf) {
                    let s = std::str::from_utf8(&buf[..n]).unwrap_or("");
                    let is_susp = s.trim() == "suspended";
                    self.gpu_pci_status_file = Some(f);
                    return is_susp;
                }
            }
        }
        false
    }

    /// Check whether the laptop is currently running on AC power (true) or Battery (false)
    pub fn is_ac_online(&mut self) -> bool {
        let paths = [
            "/sys/class/power_supply/ACAD/online",
            "/sys/class/power_supply/AC/online",
            "/sys/class/power_supply/ADP1/online",
        ];
        for p in paths {
            if let Ok(s) = fs::read_to_string(p) {
                return s.trim() != "0";
            }
        }
        true
    }

    /// Read remaining battery charge percentage (0..100)
    pub fn read_battery_percent(&mut self) -> u8 {
        let paths = [
            "/sys/class/power_supply/BAT1/capacity",
            "/sys/class/power_supply/BAT0/capacity",
            "/sys/class/power_supply/BAT/capacity",
        ];
        for p in paths {
            if let Ok(s) = fs::read_to_string(p) {
                if let Ok(pct) = s.trim().parse::<u8>() {
                    return pct.min(100);
                }
            }
        }
        100
    }

    /// Read current battery discharge power in milliwatts when running on battery
    #[allow(dead_code)]
    pub fn read_battery_discharge_mw(&mut self) -> u32 {
        let paths = [
            "/sys/class/power_supply/BAT1/power_now",
            "/sys/class/power_supply/BAT0/power_now",
        ];
        for p in paths {
            if let Ok(s) = fs::read_to_string(p) {
                if let Ok(u_watts) = s.trim().parse::<u64>() {
                    if u_watts > 0 {
                        return (u_watts / 1000) as u32;
                    }
                }
            }
        }
        0
    }

    /// Dynamically discover and bind hwmon paths if they were not ready during initial startup
    pub fn ensure_hwmon_paths(&mut self) {
        if self.fan_pwm_enable_path.is_some() && self.fan1_target_path.is_some() {
            return;
        }
        let paths = Self::discover_hwmon_paths();
        if paths.fan_pwm_enable.is_some() {
            if self.fan1_input_file.is_none() {
                self.fan1_input_file = paths.fan1_input.and_then(|p| File::open(p).ok());
            }
            if self.fan2_input_file.is_none() {
                self.fan2_input_file = paths.fan2_input.and_then(|p| File::open(p).ok());
            }
            if self.cpu_temp_file.is_none() {
                self.cpu_temp_file = paths.cpu_temp.and_then(|p| File::open(p).ok());
            }
            self.fan1_target_path = paths.fan1_target;
            self.fan2_target_path = paths.fan2_target;
            self.fan_pwm_enable_path = paths.fan_pwm_enable;
            self.fan_pwm_duty_path = paths.fan_pwm_duty;
            if let Some(m) = paths.max_rpm {
                self.max_rpm = m.clamp(4000, 7000);
            }
        }
    }

    /// Read dynamic fan RPM directly from hp-wmi hwmon (fan1_input, fan2_input)
    pub fn read_fans(&mut self) -> (Option<u16>, Option<u16>) {
        self.ensure_hwmon_paths();
        let f1 = Self::read_fd_number(&mut self.fan1_input_file).and_then(|r| {
            if (0..10000).contains(&r) {
                Some(r as u16)
            } else {
                None
            }
        });
        let f2 = Self::read_fd_number(&mut self.fan2_input_file).and_then(|r| {
            if (0..10000).contains(&r) {
                Some(r as u16)
            } else {
                None
            }
        });
        (f1, f2)
    }

    pub fn set_platform_profile(&mut self, profile_str: &str) -> bool {
        Self::write_sysfs_string(&self.platform_profile_path, profile_str)
    }

    pub fn set_gpu_tgp(&mut self, enable: bool) -> bool {
        Self::write_sysfs_string(&self.gpu_tgp_path, if enable { "1\n" } else { "0\n" })
    }

    pub fn set_gpu_ppab(&mut self, enable: bool) -> bool {
        Self::write_sysfs_string(&self.gpu_ppab_path, if enable { "1\n" } else { "0\n" })
    }

    /// Enable or disable CPU Turbo Boost (AMD Core Performance Boost / Intel Turbo Boost)
    pub fn set_cpu_boost(&mut self, enable: bool) -> bool {
        let val = if enable { "1\n" } else { "0\n" };
        let global_boost = PathBuf::from("/sys/devices/system/cpu/cpufreq/boost");
        if global_boost.exists() {
            if fs::write(&global_boost, val).is_ok() {
                return true;
            }
        }
        let mut ok = false;
        for i in 0..32 {
            let p = PathBuf::from(format!("/sys/devices/system/cpu/cpufreq/policy{}/boost", i));
            if p.exists() {
                if fs::write(p, val).is_ok() {
                    ok = true;
                }
            }
        }
        ok
    }

    #[allow(dead_code)]
    pub fn is_cpu_boost_enabled(&self) -> bool {
        let p = PathBuf::from("/sys/devices/system/cpu/cpufreq/boost");
        if let Ok(s) = fs::read_to_string(p) {
            return s.trim() != "0";
        }
        true
    }

    #[allow(dead_code)]
    pub fn get_mux_mode(&mut self) -> Option<u8> {
        self.gpu_mux_path.as_ref().and_then(|p| {
            fs::read_to_string(p).ok().and_then(|s| s.trim().parse::<u8>().ok())
        })
    }

    pub fn set_mux_mode(&mut self, mode: u8) -> bool {
        let s = format!("{}\n", mode);
        let ok1 = Self::write_sysfs_string(&self.gpu_mux_path, &s);
        let ok2 = Self::write_sysfs_string(&self.graphics_mode_path, &s);
        ok1 || ok2
    }

    /// Apply fan speeds and modes via hp-wmi hwmon targets, pwm1_enable, and pwm1 duty
    pub fn apply_fan_mode(&mut self, mode: u8, target1_rpm: u16, target2_rpm: u16) {
        self.ensure_hwmon_paths();

        match mode {
            1 => {
                // FAN_MODE_MAX (1):
                // Directly write 0 to pwm1_enable. In hp-wmi, 0 triggers PWM_MODE_MAX.
                if self.last_applied_mode != Some(1) {
                    let _ = Self::write_sysfs_string(&self.fan_pwm_enable_path, "0\n");
                    self.last_applied_mode = Some(1);
                    self.last_target1 = self.max_rpm;
                    self.last_target2 = self.max_rpm;
                }
            }
            2 => {
                // FAN_MODE_CURVE (2):
                // Operational spinning floor on Victus 16 (8BD4) / OMEN is 1100 RPM.
                // 0 RPM means fan off (Silent idle / Zero-RPM mode).
                let t1 = if target1_rpm == 0 { 0 } else { target1_rpm.clamp(1100, self.max_rpm) };
                let t2 = if target2_rpm == 0 { 0 } else { target2_rpm.clamp(1100, self.max_rpm) };

                let was_max = self.last_applied_mode == Some(1);
                let mode_changed = self.last_applied_mode != Some(2);

                if was_max {
                    // INSTANT UNLATCH FROM MAX FAN:
                    // When transitioning from MAX mode, write targets FIRST at the operational floor
                    // so the kernel module's 0x2E query receives non-zero RPMs immediately,
                    // breaking the HP EC's 120-second watchdog timer without delay!
                    let active_t1 = t1.max(1100);
                    let active_t2 = t2.max(1100);
                    if self.fan1_target_path.is_some() && self.fan2_target_path.is_some() {
                        let _ = Self::write_sysfs_string(&self.fan1_target_path, &format!("{}\n", active_t1));
                        let _ = Self::write_sysfs_string(&self.fan2_target_path, &format!("{}\n", active_t2));
                    }
                    let _ = Self::write_sysfs_string(&self.fan_pwm_enable_path, "1\n");
                    self.last_applied_mode = Some(2);
                    self.last_target1 = active_t1;
                    self.last_target2 = active_t2;
                    return;
                }

                if t1 == 0 && t2 == 0 {
                    // When both fans are 0 RPM:
                    // Sending {0,0} via manual 0x2E query freezes the EC and arms a 120s watchdog.
                    // Instead, hand control over to BIOS (pwm1_enable = 2 AUTO mode).
                    // In AUTO mode, BIOS naturally stops the fans when cold (<40°C) without EC lockup.
                    if self.last_target1 != 0 || self.last_target2 != 0 || mode_changed {
                        let _ = Self::write_sysfs_string(&self.fan_pwm_enable_path, "2\n");
                        self.last_applied_mode = Some(2);
                        self.last_target1 = 0;
                        self.last_target2 = 0;
                    }
                    return;
                }

                // Switching from AUTO, MAX, or 0-RPM into active manual spinning
                if was_max || mode_changed || (self.last_target1 == 0 && self.last_target2 == 0) {
                    let _ = Self::write_sysfs_string(&self.fan_pwm_enable_path, "1\n");
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }

                let t1_diff = (t1 as i32 - self.last_target1 as i32).abs();
                let t2_diff = (t2 as i32 - self.last_target2 as i32).abs();

                // Only write when mode changed, targets shifted by >= 40 RPM, or transition to/from 0
                if mode_changed
                    || was_max
                    || t1_diff >= 40
                    || t2_diff >= 40
                    || (t1 == 0 && self.last_target1 != 0)
                    || (t1 != 0 && self.last_target1 == 0)
                    || (t2 == 0 && self.last_target2 != 0)
                    || (t2 != 0 && self.last_target2 == 0)
                {
                    if self.fan1_target_path.is_some() && self.fan2_target_path.is_some() {
                        let _ = Self::write_sysfs_string(&self.fan1_target_path, &format!("{}\n", t1));
                        std::thread::sleep(std::time::Duration::from_millis(15));
                        let _ = Self::write_sysfs_string(&self.fan2_target_path, &format!("{}\n", t2));
                    } else if self.fan_pwm_duty_path.is_some() {
                        let primary = t1.max(t2);
                        let duty = ((primary as u32 * 255) / self.max_rpm as u32).clamp(50, 255);
                        let _ = Self::write_sysfs_string(&self.fan_pwm_duty_path, &format!("{}\n", duty));
                    }

                    self.last_applied_mode = Some(2);
                    self.last_target1 = t1;
                    self.last_target2 = t2;
                }
            }
            _ => {
                // FAN_MODE_AUTO (0):
                if self.last_applied_mode != Some(0) {
                    let _ = Self::write_sysfs_string(&self.fan_pwm_enable_path, "2\n");
                    self.last_applied_mode = Some(0);
                    self.last_target1 = 0;
                    self.last_target2 = 0;
                }
            }
        }
    }

    /// Reads live CPU wattage from RAPL energy_uj (AMD/Intel energy driver)
    pub fn read_cpu_power_mw(&mut self, power_profile: u8) -> u32 {
        if let Some(ref mut f) = self.rapl_energy_file {
            if f.seek(SeekFrom::Start(0)).is_ok() {
                let mut buf = [0u8; 32];
                if let Ok(n) = f.read(&mut buf) {
                    if let Ok(s) = std::str::from_utf8(&buf[..n]) {
                        if let Ok(cur_uj) = s.trim().parse::<u64>() {
                            let elapsed = self.last_energy_time.elapsed().as_millis() as u64;
                            if self.last_energy_uj > 0 && cur_uj > self.last_energy_uj && elapsed > 200 {
                                let delta_uj = cur_uj - self.last_energy_uj;
                                let watts_f = (delta_uj as f64) / (elapsed as f64 * 1000.0);
                                self.last_energy_uj = cur_uj;
                                self.last_energy_time = Instant::now();
                                return (watts_f * 1000.0).clamp(1000.0, 150000.0) as u32;
                            } else if self.last_energy_uj == 0 || cur_uj < self.last_energy_uj {
                                self.last_energy_uj = cur_uj;
                                self.last_energy_time = Instant::now();
                            }
                        }
                    }
                }
            }
        }
        match power_profile {
            0 => 12000,
            1 => 25000,
            _ => 45000,
        }
    }
}

#[derive(Default)]
struct DiscoveredPaths {
    cpu_temp: Option<PathBuf>,
    fan1_input: Option<PathBuf>,
    fan2_input: Option<PathBuf>,
    fan1_target: Option<PathBuf>,
    fan2_target: Option<PathBuf>,
    fan_pwm_enable: Option<PathBuf>,
    fan_pwm_duty: Option<PathBuf>,
    max_rpm: Option<u16>,
}

/// How long a last-good sensor value may be reused before it counts as stale.
pub const SENSOR_STALE_AFTER: Duration = Duration::from_secs(10);

/// Temperature handed to the fan logic when a sensor has no usable reading.
/// High enough to push the curve toward cooling, below the emergency-max threshold.
pub const FAILSAFE_TEMP: u16 = 85;

/// Tracks the last successful read of one sensor so stale values are not reused forever.
pub struct SensorTracker {
    name: &'static str,
    last_value: Option<u16>,
    last_ok: Option<Instant>,
    failing: bool,
}

impl SensorTracker {
    pub const fn new(name: &'static str) -> Self {
        Self { name, last_value: None, last_ok: None, failing: false }
    }

    /// Feed a fresh reading. Returns the value to use: the new reading, the last good
    /// value while it is younger than `stale_after`, or `None` once it is stale.
    /// Logs a single warning when the sensor starts failing and a note when it recovers.
    pub fn update(&mut self, reading: Option<u16>, now: Instant, stale_after: Duration) -> Option<u16> {
        if let Some(v) = reading {
            if self.failing {
                eprintln!("lightomen-daemon: {} sensor recovered ({})", self.name, v);
                self.failing = false;
            }
            self.last_value = Some(v);
            self.last_ok = Some(now);
            return Some(v);
        }

        let fresh = self
            .last_ok
            .is_some_and(|t| now.saturating_duration_since(t) < stale_after);
        if fresh {
            return self.last_value;
        }

        if !self.failing {
            eprintln!(
                "lightomen-daemon: {} sensor read failed for over {}s, using failsafe",
                self.name,
                stale_after.as_secs()
            );
            self.failing = true;
        }
        None
    }

    pub fn is_failing(&self) -> bool {
        self.failing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_board_id_validity() {
        let id = HwmonBackend::read_board_id();
        assert_ne!(id[0], 0);
        let id_str = std::str::from_utf8(&id).unwrap_or("");
        assert!(id_str.starts_with("8BD4") || !id_str.is_empty());
    }

    #[test]
    fn test_discovered_paths_default() {
        let p = DiscoveredPaths::default();
        assert!(p.cpu_temp.is_none());
        assert!(p.fan1_target.is_none());
    }

    #[test]
    fn sensor_tracker_returns_fresh_reading() {
        let mut t = SensorTracker::new("cpu");
        let now = Instant::now();
        assert_eq!(t.update(Some(60), now, SENSOR_STALE_AFTER), Some(60));
        assert!(!t.is_failing());
    }

    #[test]
    fn sensor_tracker_reuses_value_until_stale() {
        let mut t = SensorTracker::new("cpu");
        let start = Instant::now();
        let stale = Duration::from_secs(10);
        t.update(Some(70), start, stale);
        assert_eq!(t.update(None, start + Duration::from_secs(9), stale), Some(70));
        assert!(!t.is_failing());
        assert_eq!(t.update(None, start + Duration::from_secs(10), stale), None);
        assert!(t.is_failing());
        assert_eq!(t.update(None, start + Duration::from_secs(30), stale), None);
    }

    #[test]
    fn sensor_tracker_never_read_is_stale() {
        let mut t = SensorTracker::new("gpu");
        assert_eq!(t.update(None, Instant::now(), SENSOR_STALE_AFTER), None);
        assert!(t.is_failing());
    }

    #[test]
    fn sensor_tracker_recovers() {
        let mut t = SensorTracker::new("gpu");
        let start = Instant::now();
        t.update(None, start, SENSOR_STALE_AFTER);
        assert!(t.is_failing());
        assert_eq!(t.update(Some(55), start + Duration::from_secs(1), SENSOR_STALE_AFTER), Some(55));
        assert!(!t.is_failing());
    }
}
