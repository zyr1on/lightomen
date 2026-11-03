use lightomen_common::*;
use std::path::PathBuf;

fn temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lightomen-test-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("sub").join("config.bin")
}

#[test]
fn default_config_values() {
    let cfg = DaemonConfig::default();
    assert_eq!(cfg.magic, CONFIG_MAGIC);
    assert_eq!(cfg.version, CONFIG_VERSION);
    assert_eq!(cfg.probe_interval_ms, 2000);
    assert_eq!(cfg.power_profile, POWER_PROFILE_BALANCED);
    assert_eq!(cfg.fan_mode, FAN_MODE_AUTO);
    assert_eq!(cfg.mux_mode, MUX_MODE_HYBRID);
    assert_eq!(cfg.cpu_curve, DEFAULT_CPU_CURVE);
    assert_eq!(cfg.gpu_curve, DEFAULT_GPU_CURVE);
    assert_eq!(cfg.hotkey_code, 149);
    assert_eq!(cfg.eco_on_battery(), 1);
    assert_eq!(cfg.low_hz_on_battery(), 1);
    assert!(cfg.cpu_boost_enabled());
    assert!(cfg.no_boost_on_battery());
}

#[test]
fn default_curves_are_sane() {
    for curve in [DEFAULT_CPU_CURVE, DEFAULT_GPU_CURVE] {
        assert!(curve.windows(2).all(|w| w[0] <= w[1]));
        assert!(curve.iter().all(|&p| p <= 100));
        assert_eq!(curve[6], 100);
    }
    assert!(CURVE_TEMPS.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn battery_flag_setters_are_independent() {
    let mut cfg = DaemonConfig::default();
    cfg.battery_flags = 0;
    cfg.set_eco_on_battery(1);
    assert_eq!(cfg.battery_flags, 1);
    cfg.set_low_hz_on_battery(1);
    assert_eq!(cfg.battery_flags, 3);
    cfg.set_cpu_boost_enabled(false);
    assert_eq!(cfg.battery_flags, 7);
    cfg.set_no_boost_on_battery(true);
    assert_eq!(cfg.battery_flags, 15);

    cfg.set_low_hz_on_battery(0);
    assert_eq!(cfg.battery_flags, 13);
    assert_eq!(cfg.eco_on_battery(), 1);
    assert_eq!(cfg.low_hz_on_battery(), 0);
    cfg.set_eco_on_battery(0);
    cfg.set_cpu_boost_enabled(true);
    cfg.set_no_boost_on_battery(false);
    assert_eq!(cfg.battery_flags, 0);
}

#[test]
fn from_bytes_rejects_bad_input() {
    let cfg = DaemonConfig::default();
    let bytes = cfg.as_bytes().to_vec();
    assert!(DaemonConfig::from_bytes(&bytes[..31]).is_none());
    assert!(DaemonConfig::from_bytes(&[0u8; 33]).is_none());

    let mut bad_magic = cfg;
    bad_magic.magic = 0xDEADBEEF;
    assert!(DaemonConfig::from_bytes(bad_magic.as_bytes()).is_none());

    let mut bad_version = cfg;
    bad_version.version = CONFIG_VERSION + 1;
    assert!(DaemonConfig::from_bytes(bad_version.as_bytes()).is_none());
}

#[test]
fn from_bytes_accepts_legacy_magic() {
    let mut cfg = DaemonConfig::default();
    cfg.magic = CONFIG_MAGIC_LEGACY;
    cfg.fan_mode = FAN_MODE_CURVE;
    let restored = DaemonConfig::from_bytes(cfg.as_bytes()).expect("legacy magic accepted");
    assert_eq!(restored, cfg);
}

#[test]
fn save_and_load_roundtrip() {
    let path = temp_path("roundtrip");
    let mut cfg = DaemonConfig::default();
    cfg.fan_mode = FAN_MODE_MAX;
    cfg.probe_interval_ms = 750;
    cfg.cpu_curve = [0, 10, 20, 30, 40, 50, 60];
    cfg.save_to_file(&path).expect("save");
    assert!(!path.with_extension("tmp").exists());
    assert_eq!(DaemonConfig::load_from_file(&path), cfg);
    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
}

#[test]
fn load_rejects_corrupt_file() {
    let path = temp_path("corrupt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, [0xFFu8; 32]).unwrap();
    // Falls back to the alt config path, then to defaults; on a test machine
    // the alt path does not exist, so defaults are expected.
    if !std::path::Path::new(ALT_CONFIG_FILE).exists() {
        assert_eq!(DaemonConfig::load_from_file(&path), DaemonConfig::default());
    }
    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
}

#[test]
fn ipc_message_roundtrip() {
    let mut msg = IpcMessage::new(CMD_SET_FAN_CURVES);
    msg.payload_len = 14;
    msg.data[..7].copy_from_slice(&DEFAULT_CPU_CURVE);
    msg.data[7..14].copy_from_slice(&DEFAULT_GPU_CURVE);
    let restored = IpcMessage::from_bytes(msg.as_bytes()).expect("valid message");
    assert_eq!(restored.cmd, CMD_SET_FAN_CURVES);
    assert_eq!(restored.payload_len, 14);
    assert_eq!(restored.data, msg.data);
    assert!(IpcMessage::from_bytes(&[0u8; 67]).is_none());
}

#[test]
fn daemon_status_rejects_wrong_length() {
    assert!(DaemonStatus::from_bytes(&[0u8; 63]).is_none());
    assert!(DaemonStatus::from_bytes(&[0u8; 65]).is_none());
}
