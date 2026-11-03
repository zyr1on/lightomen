// lib.rs: LightOmen Common Library
// Zero-dependency #[repr(C)] binary IPC protocol & persistent configuration.
// Pure Rust std, no tokio, no serde.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

pub const SOCKET_PATH: &str = "/run/lightomen/lightomen.sock";
pub const ALT_SOCKET_PATH: &str = "/run/ohman/ohman.sock";
pub const CONFIG_DIR: &str = "/etc/lightomen";
pub const CONFIG_FILE: &str = "/etc/lightomen/config.bin";
pub const ALT_CONFIG_FILE: &str = "/etc/ohman/config.bin";

pub const CURVE_TEMPS: [u8; 7] = [30, 40, 50, 60, 70, 80, 90];
pub const DEFAULT_CPU_CURVE: [u8; 7] = [20, 30, 40, 52, 65, 80, 100];
pub const DEFAULT_GPU_CURVE: [u8; 7] = [20, 25, 35, 48, 60, 75, 100];

pub const POWER_PROFILE_ECO: u8 = 0;
pub const POWER_PROFILE_BALANCED: u8 = 1;
pub const POWER_PROFILE_PERFORMANCE: u8 = 2;

pub const FAN_MODE_AUTO: u8 = 0;
pub const FAN_MODE_MAX: u8 = 1;
pub const FAN_MODE_CURVE: u8 = 2;

pub const MUX_MODE_HYBRID: u8 = 0;
pub const MUX_MODE_DISCRETE: u8 = 1;

/// Battery charge limit range in percent. Stored in 5% steps so it fits in the
/// upper nibble of `battery_flags` without changing the config layout.
pub const CHARGE_LIMIT_MIN: u8 = 50;
pub const CHARGE_LIMIT_MAX: u8 = 100;
pub const CHARGE_LIMIT_STEP: u8 = 5;

pub const CONFIG_MAGIC: u32 = 0x4C4F4D4E; // "LOMN"
pub const CONFIG_MAGIC_LEGACY: u32 = 0x4F484D41; // "OHMA"
pub const CONFIG_VERSION: u32 = 1;

/// Telemetry structure representing hardware readings (24 bytes)
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Telemetry {
    pub cpu_temp: u16,      // in °C
    pub gpu_temp: u16,      // in °C
    pub fan1_rpm: u16,      // Fan 1 RPM
    pub fan2_rpm: u16,      // Fan 2 RPM
    pub cpu_power_mw: u32,  // CPU power in milliwatts
    pub gpu_power_mw: u32,  // GPU power in milliwatts
    pub board_id: [u8; 8],  // e.g. "8BD4\0\0\0\0"
}

impl Default for Telemetry {
    fn default() -> Self {
        Self {
            cpu_temp: 45,
            gpu_temp: 40,
            fan1_rpm: 2100,
            fan2_rpm: 2100,
            cpu_power_mw: 15000,
            gpu_power_mw: 12000,
            board_id: *b"8BD4\0\0\0\0",
        }
    }
}

/// Persistent configuration saved under /etc/lightomen/config.bin (32 bytes)
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DaemonConfig {
    pub magic: u32,               // 4 bytes: CONFIG_MAGIC
    pub version: u32,             // 4 bytes: CONFIG_VERSION
    pub probe_interval_ms: u16,   // 2 bytes: 500ms to 3000ms
    pub gpu_zero_rpm: u8,         // 1 byte: 1: Zero-RPM when dGPU is in D3cold, 0: standard idle floor
    pub battery_flags: u8,        // 1 byte: bit 0: eco_on_bat, bit 1: low_hz_on_bat, bit 2: no_boost, bit 3: no_boost_on_bat
    pub power_profile: u8,        // 1 byte: 0: Eco, 1: Balanced, 2: Performance
    pub fan_mode: u8,             // 1 byte: 0: Auto, 1: Max, 2: Curve
    pub mux_mode: u8,             // 1 byte: 0: Hybrid, 1: Discrete
    pub autostart: u8,            // 1 byte: 0: disabled, 1: enabled
    pub cpu_curve: [u8; 7],       // 7 bytes: Fan % at [30, 40, 50, 60, 70, 80, 90]°C
    pub gpu_curve: [u8; 7],       // 7 bytes: Fan % at [30, 40, 50, 60, 70, 80, 90]°C
    pub hotkey_code: u16,         // 2 bytes: learned evdev keycode (e.g. 149 for OMEN key)
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            magic: CONFIG_MAGIC,
            version: CONFIG_VERSION,
            probe_interval_ms: 2000,
            gpu_zero_rpm: 1,
            battery_flags: 11, // bit 0: eco=1, bit 1: low_hz=1, bit 2: boost=enabled (0), bit 3: no_boost_on_bat=1 (8)
            power_profile: POWER_PROFILE_BALANCED,
            fan_mode: FAN_MODE_AUTO,
            mux_mode: MUX_MODE_HYBRID,
            autostart: 1,
            cpu_curve: DEFAULT_CPU_CURVE,
            gpu_curve: DEFAULT_GPU_CURVE,
            hotkey_code: 149,
        }
    }
}

impl DaemonConfig {
    #[inline]
    pub fn eco_on_battery(&self) -> u8 {
        self.battery_flags & 1
    }

    #[inline]
    pub fn set_eco_on_battery(&mut self, val: u8) {
        if val == 1 {
            self.battery_flags |= 1;
        } else {
            self.battery_flags &= !1;
        }
    }

    #[inline]
    pub fn low_hz_on_battery(&self) -> u8 {
        (self.battery_flags >> 1) & 1
    }

    #[inline]
    pub fn set_low_hz_on_battery(&mut self, val: u8) {
        if val == 1 {
            self.battery_flags |= 2;
        } else {
            self.battery_flags &= !2;
        }
    }

    #[inline]
    pub fn cpu_boost_enabled(&self) -> bool {
        (self.battery_flags & 4) == 0
    }

    #[inline]
    pub fn set_cpu_boost_enabled(&mut self, enabled: bool) {
        if enabled {
            self.battery_flags &= !4;
        } else {
            self.battery_flags |= 4;
        }
    }

    #[inline]
    pub fn no_boost_on_battery(&self) -> bool {
        (self.battery_flags & 8) != 0
    }

    #[inline]
    pub fn set_no_boost_on_battery(&mut self, val: bool) {
        if val {
            self.battery_flags |= 8;
        } else {
            self.battery_flags &= !8;
        }
    }

    /// Configured charge limit in percent, or `None` when the limit is not managed.
    /// Encoded in bits 4..7 of `battery_flags`: 0 = off, n = 50 + (n - 1) * 5.
    pub fn charge_limit(&self) -> Option<u8> {
        match self.battery_flags >> 4 {
            0 => None,
            n => validate_charge_limit(CHARGE_LIMIT_MIN + (n - 1) * CHARGE_LIMIT_STEP),
        }
    }

    /// Set the charge limit. `None` (or an out-of-range value) disables it.
    /// In-range values are rounded down to the nearest 5%.
    pub fn set_charge_limit(&mut self, pct: Option<u8>) {
        let nibble = match pct.and_then(validate_charge_limit) {
            Some(v) => (v - CHARGE_LIMIT_MIN) / CHARGE_LIMIT_STEP + 1,
            None => 0,
        };
        self.battery_flags = (self.battery_flags & 0x0F) | (nibble << 4);
    }

    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                (self as *const Self) as *const u8,
                std::mem::size_of::<Self>(),
            )
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != std::mem::size_of::<Self>() {
            return None;
        }
        let config: Self = unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const Self) };
        if (config.magic != CONFIG_MAGIC && config.magic != CONFIG_MAGIC_LEGACY) || config.version != CONFIG_VERSION {
            return None;
        }
        Some(config)
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> std::io::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp_path = path.with_extension("tmp");
        {
            let mut file = File::create(&tmp_path)?;
            file.write_all(self.as_bytes())?;
            file.sync_all()?;
        }
        fs::rename(&tmp_path, path)?;
        Ok(())
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Self {
        let path = path.as_ref();
        if let Ok(mut file) = File::open(path) {
            let mut buf = vec![0u8; std::mem::size_of::<Self>()];
            if file.read_exact(&mut buf).is_ok() {
                if let Some(cfg) = Self::from_bytes(&buf) {
                    return cfg;
                }
            }
        }
        if let Ok(mut file) = File::open(ALT_CONFIG_FILE) {
            let mut buf = vec![0u8; std::mem::size_of::<Self>()];
            if file.read_exact(&mut buf).is_ok() {
                if let Some(cfg) = Self::from_bytes(&buf) {
                    return cfg;
                }
            }
        }
        Self::default()
    }
}

/// Validate a charge limit percentage. Returns the value rounded down to a
/// 5% step, or `None` if it is outside `CHARGE_LIMIT_MIN..=CHARGE_LIMIT_MAX`.
pub fn validate_charge_limit(pct: u8) -> Option<u8> {
    if (CHARGE_LIMIT_MIN..=CHARGE_LIMIT_MAX).contains(&pct) {
        Some(pct - (pct - CHARGE_LIMIT_MIN) % CHARGE_LIMIT_STEP)
    } else {
        None
    }
}

/// Full daemon status returned to GUI via IPC (64 bytes)
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DaemonStatus {
    pub telemetry: Telemetry,     // 24 bytes
    pub config: DaemonConfig,     // 32 bytes
    pub is_learning_key: u8,      // 1 byte: 1 if listening for key
    pub on_battery: u8,           // 1 byte: 1 if running on battery, 0 if AC
    pub last_learned_key: u16,    // 2 bytes: keycode captured
    pub battery_pct: u8,          // 1 byte: 0..100%
    pub charge_limit_supported: u8, // 1 byte: 1 if charge_control_end_threshold exists
    pub _pad1: [u8; 2],           // 2 bytes: alignment to 64 bytes
}

impl Default for DaemonStatus {
    fn default() -> Self {
        Self {
            telemetry: Telemetry::default(),
            config: DaemonConfig::default(),
            is_learning_key: 0,
            on_battery: 0,
            last_learned_key: 149,
            battery_pct: 100,
            charge_limit_supported: 0,
            _pad1: [0; 2],
        }
    }
}

impl DaemonStatus {
    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                (self as *const Self) as *const u8,
                std::mem::size_of::<Self>(),
            )
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != std::mem::size_of::<Self>() {
            return None;
        }
        Some(unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const Self) })
    }
}

pub const CMD_GET_STATUS: u8 = 1;
pub const CMD_SET_POWER_PROFILE: u8 = 2;
pub const CMD_SET_FAN_MODE: u8 = 3;
pub const CMD_SET_FAN_CURVES: u8 = 4;
pub const CMD_SET_MUX_MODE: u8 = 5;
pub const CMD_SET_PROBE_INTERVAL: u8 = 6;
pub const CMD_SPAWN_GUI: u8 = 7;
pub const CMD_TOGGLE_GUI: u8 = 8;
pub const CMD_SET_AUTOSTART: u8 = 9;
pub const CMD_START_KEY_LEARN: u8 = 10;
pub const CMD_CANCEL_KEY_LEARN: u8 = 11;
pub const CMD_SET_HOTKEY: u8 = 12;
pub const CMD_SET_GPU_ZERO_RPM: u8 = 13;
pub const CMD_SET_ECO_ON_BATTERY: u8 = 14;
pub const CMD_SET_LOW_HZ_ON_BATTERY: u8 = 15;
pub const CMD_SET_CPU_BOOST: u8 = 16;
pub const CMD_SET_NO_BOOST_ON_BATTERY: u8 = 17;
/// data[0] = charge limit percent (50..=100), 0 = disable
pub const CMD_SET_CHARGE_LIMIT: u8 = 18;

/// IPC Message Header & Payload (exactly 68 bytes)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IpcMessage {
    pub cmd: u8,
    pub _pad: u8,
    pub payload_len: u16,
    pub data: [u8; 64],
}

impl IpcMessage {
    pub fn new(cmd: u8) -> Self {
        Self {
            cmd,
            _pad: 0,
            payload_len: 0,
            data: [0; 64],
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                (self as *const Self) as *const u8,
                std::mem::size_of::<Self>(),
            )
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != std::mem::size_of::<Self>() {
            return None;
        }
        Some(unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const Self) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sizes() {
        assert_eq!(std::mem::size_of::<Telemetry>(), 24);
        assert_eq!(std::mem::size_of::<DaemonConfig>(), 32);
        assert_eq!(std::mem::size_of::<DaemonStatus>(), 64);
        assert_eq!(std::mem::size_of::<IpcMessage>(), 68);
    }

    #[test]
    fn test_config_serde_binary() {
        let mut cfg = DaemonConfig::default();
        assert!(cfg.cpu_boost_enabled());
        assert!(cfg.no_boost_on_battery());
        cfg.set_cpu_boost_enabled(false);
        assert!(!cfg.cpu_boost_enabled());
        cfg.set_no_boost_on_battery(false);
        assert!(!cfg.no_boost_on_battery());

        let bytes = cfg.as_bytes();
        let restored = DaemonConfig::from_bytes(bytes).expect("valid bytes");
        assert_eq!(cfg, restored);
    }

    #[test]
    fn test_daemon_status_layout() {
        let status = DaemonStatus::default();
        let bytes = status.as_bytes();
        let restored = DaemonStatus::from_bytes(bytes).expect("valid status");
        assert_eq!(status, restored);
    }

    #[test]
    fn test_validate_charge_limit() {
        assert_eq!(validate_charge_limit(49), None);
        assert_eq!(validate_charge_limit(101), None);
        assert_eq!(validate_charge_limit(0), None);
        assert_eq!(validate_charge_limit(50), Some(50));
        assert_eq!(validate_charge_limit(80), Some(80));
        assert_eq!(validate_charge_limit(84), Some(80));
        assert_eq!(validate_charge_limit(100), Some(100));
    }

    #[test]
    fn test_charge_limit_roundtrip_keeps_other_flags() {
        let mut cfg = DaemonConfig::default();
        assert_eq!(cfg.charge_limit(), None);
        let low_bits = cfg.battery_flags & 0x0F;
        for pct in (50..=100).step_by(5) {
            cfg.set_charge_limit(Some(pct));
            assert_eq!(cfg.charge_limit(), Some(pct));
            assert_eq!(cfg.battery_flags & 0x0F, low_bits);
        }
        cfg.set_charge_limit(Some(30));
        assert_eq!(cfg.charge_limit(), None);
        cfg.set_charge_limit(Some(80));
        cfg.set_charge_limit(None);
        assert_eq!(cfg.charge_limit(), None);
        assert_eq!(cfg.battery_flags, DaemonConfig::default().battery_flags);
    }

    #[test]
    fn test_charge_limit_ignores_invalid_nibble() {
        let mut cfg = DaemonConfig::default();
        cfg.battery_flags |= 0xF0;
        assert_eq!(cfg.charge_limit(), None);
    }
}
