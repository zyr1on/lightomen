// ipc.rs: Zero-dependency Unix domain socket server
// Processes binary requests and serves DaemonStatus.

use crate::hotkey::HotkeyMonitor;
use lightomen_common::{
    DaemonStatus, IpcMessage, ALT_SOCKET_PATH, CMD_CANCEL_KEY_LEARN, CMD_GET_STATUS,
    CMD_SET_AUTOSTART, CMD_SET_CPU_BOOST, CMD_SET_ECO_ON_BATTERY, CMD_SET_FAN_CURVES,
    CMD_SET_FAN_MODE, CMD_SET_GPU_ZERO_RPM, CMD_SET_HOTKEY, CMD_SET_LOW_HZ_ON_BATTERY,
    CMD_SET_MUX_MODE, CMD_SET_NO_BOOST_ON_BATTERY, CMD_SET_POWER_PROFILE, CMD_SET_PROBE_INTERVAL,
    CMD_SET_CHARGE_LIMIT, CMD_SPAWN_GUI, CMD_START_KEY_LEARN, CMD_TOGGLE_GUI, CONFIG_FILE, SOCKET_PATH,
    validate_charge_limit,
};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

pub struct IpcServer;

impl IpcServer {
    pub fn start(status: Arc<Mutex<DaemonStatus>>) {
        let sock_path = Path::new(SOCKET_PATH);
        if let Some(parent) = sock_path.parent() {
            let _ = fs::create_dir_all(parent);
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o755));
        }
        if sock_path.exists() {
            let _ = fs::remove_file(sock_path);
        }

        let listener = UnixListener::bind(sock_path).expect("failed to bind IPC socket");
        let _ = fs::set_permissions(sock_path, fs::Permissions::from_mode(0o666));

        // Create compatibility symlink for legacy / alternative socket path
        let alt_path = Path::new(ALT_SOCKET_PATH);
        if let Some(parent) = alt_path.parent() {
            let _ = fs::create_dir_all(parent);
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o755));
        }
        if alt_path.exists() {
            let _ = fs::remove_file(alt_path);
        }
        let _ = std::os::unix::fs::symlink(sock_path, alt_path);

        thread::Builder::new()
            .name("lightomen-ipc".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(mut s) => {
                            let st = status.clone();
                            thread::spawn(move || {
                                Self::handle_client(&mut s, st);
                            });
                        }
                        Err(_) => break,
                    }
                }
            })
            .expect("failed to spawn IPC thread");
    }

    fn handle_client(stream: &mut UnixStream, status: Arc<Mutex<DaemonStatus>>) {
        let mut buf = [0u8; std::mem::size_of::<IpcMessage>()];
        if stream.read_exact(&mut buf).is_err() {
            return;
        }

        let msg = match IpcMessage::from_bytes(&buf) {
            Some(m) => m,
            None => return,
        };

        match msg.cmd {
            CMD_GET_STATUS => {
                // If key learning timed out, reflect it in status
                if HotkeyMonitor::check_timeout() {
                    let mut st = status.lock().unwrap();
                    st.is_learning_key = 0;
                }
                let st = status.lock().unwrap();
                let bytes = st.as_bytes();
                let _ = stream.write_all(bytes);
            }
            CMD_SET_POWER_PROFILE => {
                let profile = msg.data[0];
                let mut st = status.lock().unwrap();
                st.config.power_profile = profile;
                let _ = st.config.save_to_file(CONFIG_FILE);
                let _ = stream.write_all(b"OK");
            }
            CMD_SET_FAN_MODE => {
                let mode = msg.data[0];
                let mut st = status.lock().unwrap();
                st.config.fan_mode = mode;
                let _ = st.config.save_to_file(CONFIG_FILE);
                let _ = stream.write_all(b"OK");
            }
            CMD_SET_FAN_CURVES => {
                let mut cpu_c = [0u8; 7];
                let mut gpu_c = [0u8; 7];
                cpu_c.copy_from_slice(&msg.data[0..7]);
                gpu_c.copy_from_slice(&msg.data[7..14]);

                let mut st = status.lock().unwrap();
                st.config.cpu_curve = cpu_c;
                st.config.gpu_curve = gpu_c;
                let _ = st.config.save_to_file(CONFIG_FILE);
                let _ = stream.write_all(b"OK");
            }
            CMD_SET_MUX_MODE => {
                let mode = msg.data[0];
                let mut st = status.lock().unwrap();
                st.config.mux_mode = mode;
                let _ = st.config.save_to_file(CONFIG_FILE);
                let _ = stream.write_all(b"OK");
            }
            CMD_SET_PROBE_INTERVAL => {
                let interval = u16::from_le_bytes([msg.data[0], msg.data[1]])
                    .clamp(500, 3000);
                let mut st = status.lock().unwrap();
                st.config.probe_interval_ms = interval;
                let _ = st.config.save_to_file(CONFIG_FILE);
                let _ = stream.write_all(b"OK");
            }
            CMD_SET_GPU_ZERO_RPM => {
                let zero_rpm = msg.data[0];
                let mut st = status.lock().unwrap();
                st.config.gpu_zero_rpm = zero_rpm;
                let _ = st.config.save_to_file(CONFIG_FILE);
                let _ = stream.write_all(b"OK");
            }
            CMD_SET_ECO_ON_BATTERY => {
                let eco = msg.data[0];
                let mut st = status.lock().unwrap();
                st.config.set_eco_on_battery(eco);
                let _ = st.config.save_to_file(CONFIG_FILE);
                let _ = stream.write_all(b"OK");
            }
            CMD_SET_LOW_HZ_ON_BATTERY => {
                let low_hz = msg.data[0];
                let mut st = status.lock().unwrap();
                st.config.set_low_hz_on_battery(low_hz);
                let _ = st.config.save_to_file(CONFIG_FILE);
                let _ = stream.write_all(b"OK");
            }
            CMD_SET_CPU_BOOST => {
                let boost = msg.data[0];
                let mut st = status.lock().unwrap();
                st.config.set_cpu_boost_enabled(boost != 0);
                let _ = st.config.save_to_file(CONFIG_FILE);
                let _ = stream.write_all(b"OK");
            }
            CMD_SET_NO_BOOST_ON_BATTERY => {
                let no_boost = msg.data[0];
                let mut st = status.lock().unwrap();
                st.config.set_no_boost_on_battery(no_boost != 0);
                let _ = st.config.save_to_file(CONFIG_FILE);
                let _ = stream.write_all(b"OK");
            }
            CMD_SET_CHARGE_LIMIT => {
                let pct = msg.data[0];
                let limit = validate_charge_limit(pct);
                if pct != 0 && limit.is_none() {
                    let _ = stream.write_all(b"ERR");
                    return;
                }
                let mut st = status.lock().unwrap();
                st.config.set_charge_limit(limit);
                let _ = st.config.save_to_file(CONFIG_FILE);
                let _ = stream.write_all(b"OK");
            }
            CMD_SPAWN_GUI => {
                HotkeyMonitor::spawn_gui();
                let _ = stream.write_all(b"OK");
            }
            CMD_TOGGLE_GUI => {
                HotkeyMonitor::toggle_gui();
                let _ = stream.write_all(b"OK");
            }
            CMD_SET_AUTOSTART => {
                let autostart = msg.data[0];
                let mut st = status.lock().unwrap();
                st.config.autostart = autostart;
                let _ = st.config.save_to_file(CONFIG_FILE);
                let _ = stream.write_all(b"OK");
            }
            CMD_START_KEY_LEARN => {
                HotkeyMonitor::start_learning();
                let mut st = status.lock().unwrap();
                st.is_learning_key = 1;
                let _ = stream.write_all(b"OK");
            }
            CMD_CANCEL_KEY_LEARN => {
                HotkeyMonitor::cancel_learning();
                let mut st = status.lock().unwrap();
                st.is_learning_key = 0;
                let _ = stream.write_all(b"OK");
            }
            CMD_SET_HOTKEY => {
                let code = u16::from_le_bytes([msg.data[0], msg.data[1]]);
                let mut st = status.lock().unwrap();
                st.config.hotkey_code = code;
                st.last_learned_key = code;
                st.is_learning_key = 0;
                HotkeyMonitor::cancel_learning();
                let _ = st.config.save_to_file(CONFIG_FILE);
                let _ = stream.write_all(b"OK");
            }
            _ => {
                let _ = stream.write_all(b"ERR");
            }
        }
    }
}
