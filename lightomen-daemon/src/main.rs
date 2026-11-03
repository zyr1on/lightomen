// main.rs: lightomen-daemon entry point
// Pure std + libc + evdev, zero bloat, <2.5MB RAM footprint.

mod ec;
mod fan;
mod hotkey;
mod hwmon;
mod ipc;
mod profile;

use ec::EmbeddedController;
use fan::FanController;
use hotkey::HotkeyMonitor;
use hwmon::{HwmonBackend, SensorTracker, FAILSAFE_TEMP, SENSOR_STALE_AFTER};
use ipc::IpcServer;
use profile::ProfileController;

use lightomen_common::{
    DaemonConfig, DaemonStatus, Telemetry, CONFIG_FILE, FAN_MODE_AUTO, POWER_PROFILE_ECO,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

static RUNNING: AtomicBool = AtomicBool::new(true);

extern "C" fn sig_handler(_: libc::c_int) {
    RUNNING.store(false, Ordering::SeqCst);
}

fn main() {
    unsafe {
        libc::signal(libc::SIGINT, sig_handler as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, sig_handler as *const () as libc::sighandler_t);
    }
    // 1. Load persistent configuration
    let config = DaemonConfig::load_from_file(CONFIG_FILE);

    // 2. Initialize hardware backends
    let ec = EmbeddedController::new();
    let mut hwmon = HwmonBackend::new();

    let initial_status = DaemonStatus {
        telemetry: Telemetry {
            cpu_temp: 45,
            gpu_temp: 40,
            fan1_rpm: 0,
            fan2_rpm: 0,
            cpu_power_mw: 15000,
            gpu_power_mw: 12000,
            board_id: hwmon.board_id,
        },
        config,
        is_learning_key: 0,
        on_battery: 0,
        last_learned_key: config.hotkey_code,
        battery_pct: 100,
        _pad1: [0; 3],
    };

    let status = Arc::new(Mutex::new(initial_status));

    // 3. Start IPC server & Hotkey monitor
    IpcServer::start(status.clone());
    HotkeyMonitor::start(status.clone());

    // 4. Initial profile & fan configuration
    ProfileController::apply_power_profile(&mut hwmon, config.power_profile);
    ProfileController::apply_mux_mode(&mut hwmon, config.mux_mode);

    let mut last_profile = config.power_profile;
    let mut user_ac_profile = config.power_profile;
    let mut last_mux = config.mux_mode;
    let mut last_ac_online: Option<bool> = None;
    let mut last_cpu_boost: Option<bool> = None;
    let mut last_applied_refresh_hz: Option<u32> = None;

    // Previous valid readings
    let mut prev_cpu_temp: u16 = 0;
    let mut cpu_sensor = SensorTracker::new("CPU temperature");
    let mut gpu_sensor = SensorTracker::new("GPU temperature");
    let mut prev_fan1_rpm: u16 = 0;
    let mut prev_fan2_rpm: u16 = 0;

    // 5. Main telemetry & fan control loop
    while RUNNING.load(Ordering::SeqCst) {
        let (mut current_cfg, probe_interval_ms) = {
            let st = status.lock().unwrap();
            (st.config, st.config.probe_interval_ms.clamp(500, 3000))
        };

        // AC vs Battery detection & Auto-Eco power management
        let is_ac = hwmon.is_ac_online();
        let on_battery = !is_ac;
        let battery_pct = hwmon.read_battery_percent();

        if let Some(was_ac) = last_ac_online {
            if is_ac != was_ac {
                if !is_ac {
                    // Transitioned from AC to Battery (unplugged)
                    if current_cfg.eco_on_battery() == 1 && current_cfg.power_profile != POWER_PROFILE_ECO {
                        user_ac_profile = current_cfg.power_profile;
                        current_cfg.power_profile = POWER_PROFILE_ECO;
                        ProfileController::apply_power_profile(&mut hwmon, POWER_PROFILE_ECO);
                        last_profile = POWER_PROFILE_ECO;
                        let mut st = status.lock().unwrap();
                        st.config.power_profile = POWER_PROFILE_ECO;
                        let _ = st.config.save_to_file(CONFIG_FILE);
                    }
                } else {
                    // Transitioned from Battery to AC (plugged in)
                    if current_cfg.eco_on_battery() == 1 && current_cfg.power_profile != user_ac_profile {
                        current_cfg.power_profile = user_ac_profile;
                        ProfileController::apply_power_profile(&mut hwmon, user_ac_profile);
                        last_profile = user_ac_profile;
                        let mut st = status.lock().unwrap();
                        st.config.power_profile = user_ac_profile;
                        let _ = st.config.save_to_file(CONFIG_FILE);
                    }
                }
                last_ac_online = Some(is_ac);
            }
        } else {
            // Initial boot check
            if on_battery && current_cfg.eco_on_battery() == 1 && current_cfg.power_profile != POWER_PROFILE_ECO {
                user_ac_profile = current_cfg.power_profile;
                current_cfg.power_profile = POWER_PROFILE_ECO;
                ProfileController::apply_power_profile(&mut hwmon, POWER_PROFILE_ECO);
                last_profile = POWER_PROFILE_ECO;
                let mut st = status.lock().unwrap();
                st.config.power_profile = POWER_PROFILE_ECO;
            }
            last_ac_online = Some(is_ac);
        }

        // Apply display refresh rate once graphical user session is fully active
        let target_hz = if on_battery && current_cfg.low_hz_on_battery() == 1 { 60 } else { 144 };
        if current_cfg.low_hz_on_battery() == 1 {
            if last_applied_refresh_hz != Some(target_hz) && HotkeyMonitor::is_user_session_ready() {
                HotkeyMonitor::set_display_refresh_rate(target_hz);
                last_applied_refresh_hz = Some(target_hz);
            }
        } else if last_applied_refresh_hz == Some(60) && HotkeyMonitor::is_user_session_ready() {
            HotkeyMonitor::set_display_refresh_rate(144);
            last_applied_refresh_hz = Some(144);
        }

        // Check if power profile or MUX was changed via IPC
        if current_cfg.power_profile != last_profile {
            ProfileController::apply_power_profile(&mut hwmon, current_cfg.power_profile);
            last_profile = current_cfg.power_profile;
            if is_ac {
                user_ac_profile = current_cfg.power_profile;
            }
        }
        if current_cfg.mux_mode != last_mux {
            ProfileController::apply_mux_mode(&mut hwmon, current_cfg.mux_mode);
            last_mux = current_cfg.mux_mode;
        }

        // Apply CPU Turbo Boost according to config and battery state
        let desired_boost = if !current_cfg.cpu_boost_enabled() {
            false
        } else if on_battery && current_cfg.no_boost_on_battery() {
            false
        } else {
            true
        };
        if last_cpu_boost != Some(desired_boost) {
            hwmon.set_cpu_boost(desired_boost);
            last_cpu_boost = Some(desired_boost);
        }

        // Telemetry: CPU Temp (Hwmon primary, EC 0x57 fallback)
        let now = Instant::now();
        let cpu_reading = hwmon.read_cpu_temp().or_else(|| ec.read_cpu_temp());
        let cpu_temp = match cpu_sensor.update(cpu_reading, now, SENSOR_STALE_AFTER) {
            Some(raw) => {
                let raw_cpu_temp = raw.max(25);
                // Smooth momentary AMD Ryzen Precision Boost spikes (EMA filter: 2/3 weight on prev, 1/3 on new)
                if prev_cpu_temp > 0 && raw_cpu_temp < 85 {
                    ((prev_cpu_temp as u32 * 2 + raw_cpu_temp as u32 + 1) / 3) as u16
                } else {
                    raw_cpu_temp
                }
            }
            // No usable reading: report 0 instead of a made-up value
            None => 0,
        };
        prev_cpu_temp = cpu_temp;

        // Telemetry: GPU Temp & Power (Passive/D3cold Safe)
        // If NVIDIA dGPU is suspended in D3cold: DO NOT touch EC or sysfs for GPU!
        // Guarantees dGPU is never woke up unnecessarily by LightOmen.
        let is_gpu_suspended = hwmon.is_gpu_suspended();
        let (gpu_temp, gpu_power_mw) = if is_gpu_suspended {
            (0u16, 0u32)
        } else {
            let t = gpu_sensor
                .update(ec.read_gpu_temp(), now, SENSOR_STALE_AFTER)
                .map_or(0, |t| t.max(25));
            let pwr = match current_cfg.power_profile {
                0 => 10000,
                1 => 35000,
                _ => 75000,
            };
            (t, pwr)
        };

        // Telemetry: Fan RPMs (Hwmon fan1_input/fan2_input primary, EC fallback)
        let (hw_f1, hw_f2) = hwmon.read_fans();
        let (ec_f1, ec_f2) = ec.read_fan_rpms();

        let fan1_rpm = hw_f1.or(ec_f1).unwrap_or(prev_fan1_rpm);
        prev_fan1_rpm = fan1_rpm;

        let fan2_rpm = hw_f2.or(ec_f2).unwrap_or(prev_fan2_rpm);
        prev_fan2_rpm = fan2_rpm;

        // Telemetry: Live CPU Power via RAPL energy_uj
        let cpu_power_mw = hwmon.read_cpu_power_mw(current_cfg.power_profile);

        // Apply fan curve / mode (with keep-alive & >=90°C emergency max)
        FanController::apply(
            &ec,
            &mut hwmon,
            current_cfg.fan_mode,
            &current_cfg.cpu_curve,
            &current_cfg.gpu_curve,
            if cpu_sensor.is_failing() { FAILSAFE_TEMP } else { cpu_temp },
            if !is_gpu_suspended && gpu_sensor.is_failing() { FAILSAFE_TEMP } else { gpu_temp },
            current_cfg.gpu_zero_rpm,
            on_battery,
        );

        // Update shared status for IPC queries
        {
            let mut st = status.lock().unwrap();
            st.telemetry = Telemetry {
                cpu_temp,
                gpu_temp,
                fan1_rpm,
                fan2_rpm,
                cpu_power_mw,
                gpu_power_mw,
                board_id: hwmon.board_id,
            };
            st.on_battery = if on_battery { 1 } else { 0 };
            st.battery_pct = battery_pct;
        }

        thread::sleep(Duration::from_millis(probe_interval_ms as u64));
    }

    // Graceful teardown: ensure fans safely returned to BIOS Auto control
    hwmon.apply_fan_mode(FAN_MODE_AUTO, 0, 0);
}
