// main.rs: LightOmen GUI (LightOmen Linux)
// Pure Rust + eframe/egui with zero tokio/serde bloat.
// Features: Left Navigation Rail (Dashboard, Fan Curve, Graphics MUX, System Specs, Settings),
// 1:1 Ohman dark aesthetic, smoothly draggable window on Wayland, Link CPU & GPU fan curves,
// MUX Switch with Restart action, System hardware inspector, and OMEN key learning (default 149).

#![allow(unused_variables)]
#![allow(float_literal_f32_fallback)]

mod icon_data;
mod i18n;

use i18n::Language;

use eframe::egui::{
    self, Color32, FontFamily, FontId, Pos2, Rounding, Sense, Stroke, Ui, Vec2,
};
use lightomen_common::{
    DaemonStatus, IpcMessage, ALT_SOCKET_PATH, CMD_CANCEL_KEY_LEARN, CMD_GET_STATUS,
    CMD_SET_AUTOSTART, CMD_SET_CPU_BOOST, CMD_SET_ECO_ON_BATTERY, CMD_SET_FAN_CURVES,
    CMD_SET_FAN_MODE, CMD_SET_GPU_ZERO_RPM, CMD_SET_LOW_HZ_ON_BATTERY, CMD_SET_MUX_MODE,
    CMD_SET_CHARGE_LIMIT, CMD_SET_NO_BOOST_ON_BATTERY, CMD_SET_POWER_PROFILE, CMD_SET_PROBE_INTERVAL,
    CMD_START_KEY_LEARN, CURVE_TEMPS, DEFAULT_CPU_CURVE, DEFAULT_GPU_CURVE, FAN_MODE_AUTO,
    FAN_MODE_CURVE, FAN_MODE_MAX, MUX_MODE_DISCRETE, MUX_MODE_HYBRID, POWER_PROFILE_BALANCED,
    POWER_PROFILE_ECO, POWER_PROFILE_PERFORMANCE, SOCKET_PATH,
    CHARGE_LIMIT_MAX, CHARGE_LIMIT_MIN, CHARGE_LIMIT_STEP,
};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

// 1:1 Palette from ref-ohman/src/Theme.cs
const BG_SUNKEN: Color32 = Color32::from_rgb(0x0F, 0x0D, 0x0B); // #0F0D0B Main sunken bg
const CARD_BG: Color32 = Color32::from_rgb(0x16, 0x13, 0x11);   // #161311 Card background
const PILL_BG: Color32 = Color32::from_rgb(0x20, 0x1C, 0x19);   // #201C19 Pill background
const PILL_ACTIVE: Color32 = Color32::from_rgb(0x2C, 0x28, 0x25); // #2C2825
const LINE_BORDER: Color32 = Color32::from_rgb(0x2C, 0x28, 0x25); // #2C2825 Border lines
const LINE_LIGHT: Color32 = Color32::from_rgb(0x3B, 0x37, 0x34);  // #3B3734 Dividers

const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xED, 0xEA, 0xE8); // #EDEAE8 Main text
const TEXT_HI: Color32 = Color32::from_rgb(0xF4, 0xF1, 0xEF);      // #F4F1EF Numbers
const TEXT_MUTED: Color32 = Color32::from_rgb(0x96, 0x91, 0x8D);   // #96918D Subtitle
const TEXT_DIM: Color32 = Color32::from_rgb(0x78, 0x73, 0x70);     // #787370 Section header
const TEXT_AXIS: Color32 = Color32::from_rgb(0x84, 0x7F, 0x7B);    // #847F7B Graph axes

// Mode Accents
const COLOR_ECO: Color32 = Color32::from_rgb(0x2F, 0xBF, 0x8F);      // #2FBF8F
const COLOR_BALANCED: Color32 = Color32::from_rgb(0x3F, 0x8C, 0xFF); // #3F8CFF
const COLOR_PERF: Color32 = Color32::from_rgb(0xE2, 0x57, 0x2C);     // #E2572C

// Status indicators
const COLOR_OK: Color32 = Color32::from_rgb(0x4A, 0xC0, 0x6C);       // #4AC06C
const COLOR_WARN: Color32 = Color32::from_rgb(0xF3, 0x82, 0x1D);     // #F3821D

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActiveTab {
    Dashboard = 0,
    FanCurve = 1,
    GraphicsMux = 2,
    SystemInfo = 3,
    Settings = 4,
}

#[derive(Clone, Default)]
struct SystemSpecs {
    cpu_model: String,
    gpu_igpu: String,
    gpu_dgpu: String,
    total_ram_gb: u32,
    active_renderer: String,
    kernel_version: String,
    nvidia_driver: String,
}

impl SystemSpecs {
    fn collect() -> Self {
        // Read CPU model
        let mut cpu_model = "AMD Ryzen Processor".to_string();
        if let Ok(content) = fs::read_to_string("/proc/cpuinfo") {
            for line in content.lines() {
                if line.starts_with("model name") {
                    if let Some((_, val)) = line.split_once(':') {
                        cpu_model = val.trim().to_string();
                        break;
                    }
                }
            }
        }

        // Read Total RAM
        let mut total_ram_gb = 16;
        if let Ok(content) = fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    if let Some((_, val)) = line.split_once(':') {
                        let kb_str = val.trim().split_whitespace().next().unwrap_or("0");
                        if let Ok(kb) = kb_str.parse::<u64>() {
                            total_ram_gb = ((kb as f64 / 1024.0 / 1024.0).round()) as u32;
                        }
                        break;
                    }
                }
            }
        }

        // Read NVIDIA driver
        let mut nvidia_driver = "NVIDIA Driver Loaded".to_string();
        if let Ok(content) = fs::read_to_string("/proc/driver/nvidia/version") {
            if let Some(line) = content.lines().next() {
                if let Some(pos) = line.find("Kernel Module") {
                    nvidia_driver = line[pos..].trim().to_string();
                } else {
                    nvidia_driver = line.trim().to_string();
                }
            }
        }

        // Read Kernel version
        let mut kernel_version = "Linux".to_string();
        if let Ok(content) = fs::read_to_string("/proc/version") {
            let parts: Vec<&str> = content.split_whitespace().collect();
            if parts.len() >= 3 {
                kernel_version = format!("Linux {}", parts[2]);
            }
        }

        // Check active display output connector
        let mut active_renderer = "AMD Radeon Graphics (Hybrid Optimus - PRIME Active)".to_string();
        if let Ok(entries) = fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let name = entry.file_name().into_string().unwrap_or_default();
                if name.contains("eDP-1") {
                    let vendor_path = entry.path().join("device/vendor");
                    if let Ok(vendor) = fs::read_to_string(vendor_path) {
                        if vendor.trim().to_lowercase().contains("10de") {
                            active_renderer = "NVIDIA GeForce RTX 4050 (Discrete Direct Mode)".to_string();
                        }
                    }
                    break;
                }
            }
        }

        Self {
            cpu_model,
            gpu_igpu: "AMD Radeon 760M (Phoenix1 / RDNA3)".to_string(),
            gpu_dgpu: "NVIDIA GeForce RTX 4050 Mobile (6GB GDDR6)".to_string(),
            total_ram_gb,
            active_renderer,
            kernel_version,
            nvidia_driver,
        }
    }
}

struct LightOmenApp {
    status: DaemonStatus,
    last_poll: Instant,
    cached_cpu_ghz: Option<f32>,
    active_tab: ActiveTab,
    curve_tab_gpu: bool,
    link_curves: bool,
    active_drag_idx: Option<usize>,
    learn_started_at: Option<Instant>,
    specs: Option<SystemSpecs>,
    needs_focus: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ctx_holder: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>>,
    reset_window_level_at: Option<Instant>,
    diagnostic_report: Option<String>,
    diagnostic_status: Option<String>,
    pub language: Language,
}

impl LightOmenApp {
    fn new(
        needs_focus: std::sync::Arc<std::sync::atomic::AtomicBool>,
        ctx_holder: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>>,
    ) -> Self {
        Self::ensure_tray_running();
        let initial_status = Self::fetch_daemon_status().unwrap_or_default();
        let link_curves = Self::load_gui_config();
        let cached_cpu_ghz = Self::read_cpu_ghz();
        let language = Language::load();
        Self {
            status: initial_status,
            last_poll: Instant::now(),
            cached_cpu_ghz,
            active_tab: ActiveTab::Dashboard,
            curve_tab_gpu: false,
            link_curves,
            active_drag_idx: None,
            learn_started_at: None,
            specs: None,
            needs_focus,
            ctx_holder,
            reset_window_level_at: None,
            diagnostic_report: None,
            diagnostic_status: None,
            language,
        }
    }

    fn load_gui_config() -> bool {
        if let Some(home) = std::env::var_os("HOME") {
            let path = PathBuf::from(home).join(".config/lightomen/gui.conf");
            if let Ok(content) = fs::read_to_string(path) {
                for line in content.lines() {
                    if let Some((k, v)) = line.split_once('=') {
                        if k.trim() == "link_curves" {
                            return v.trim() == "true" || v.trim() == "1";
                        }
                    }
                }
            }
        }
        true
    }

    fn save_gui_config(link_curves: bool) {
        if let Some(home) = std::env::var_os("HOME") {
            let dir = PathBuf::from(home).join(".config/lightomen");
            let _ = fs::create_dir_all(&dir);
            let path = dir.join("gui.conf");
            let content = format!("link_curves={}\n", link_curves);
            let _ = fs::write(path, content);
        }
    }

    fn ensure_tray_running() {
        let is_running = Command::new("sh")
            .arg("-c")
            .arg("pgrep -x lightomen-tray || pgrep -x ohman-tray")
            .output()
            .map(|o| o.status.success() && !o.stdout.is_empty())
            .unwrap_or(false);

        if !is_running {
            let tray_bin = if std::path::Path::new("/usr/local/bin/lightomen-tray").exists() {
                "/usr/local/bin/lightomen-tray".to_string()
            } else if std::path::Path::new("/usr/local/bin/ohman-tray").exists() {
                "/usr/local/bin/ohman-tray".to_string()
            } else if let Ok(exe) = std::env::current_exe() {
                if let Some(parent) = exe.parent() {
                    let cand = parent.join("lightomen-tray");
                    if cand.exists() {
                        cand.to_str().unwrap_or("lightomen-tray").to_string()
                    } else {
                        "lightomen-tray".to_string()
                    }
                } else {
                    "lightomen-tray".to_string()
                }
            } else {
                "lightomen-tray".to_string()
            };
            // Run lightomen-tray in its own systemd user unit so that closing lightomen-gui
            // does NOT cause systemd to terminate lightomen-tray as part of GUI's cgroup.
            let res = Command::new("systemd-run")
                .args(["--user", "--unit=lightomen-tray", "--collect", &tray_bin])
                .spawn();
            if res.is_err() {
                let _ = Command::new("nohup")
                    .arg(&tray_bin)
                    .spawn();
            }
        }
    }

    /// Read live CPU maximum boost frequency across active cores in GHz
    fn read_cpu_ghz() -> Option<f32> {
        let mut max_khz = 0u32;
        for i in 0..16 {
            let path = format!("/sys/devices/system/cpu/cpufreq/policy{}/scaling_cur_freq", i);
            if let Ok(s) = fs::read_to_string(&path) {
                if let Ok(khz) = s.trim().parse::<u32>() {
                    if khz > max_khz {
                        max_khz = khz;
                    }
                }
            } else {
                break;
            }
        }
        if max_khz > 0 {
            Some(max_khz as f32 / 1_000_000.0)
        } else {
            None
        }
    }

    fn draw_charge_limit_card(&mut self, ui: &mut Ui) {
        let tr = self.language.is_tr();
        egui::Frame::none()
            .fill(CARD_BG)
            .stroke(Stroke::new(1.0, LINE_BORDER))
            .rounding(Rounding::same(8.0))
            .inner_margin(14.0)
            .show(ui, |ui| {
                let current = self.status.config.charge_limit();
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        let title = if tr { "Şarj Sınırı" } else { "Battery Charge Limit" };
                        let desc = if tr {
                            "Pil ömrünü uzatmak için şarjı belirlenen yüzdede durdurur (%50-100)."
                        } else {
                            "Stop charging at the chosen level to extend battery lifespan (50-100%)."
                        };
                        ui.label(egui::RichText::new(title).font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_PRIMARY));
                        ui.label(egui::RichText::new(desc).font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_MUTED));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut enabled = current.is_some();
                        if ui.checkbox(&mut enabled, "").changed() {
                            let limit = if enabled { Some(80) } else { None };
                            self.status.config.set_charge_limit(limit);
                            Self::send_ipc(CMD_SET_CHARGE_LIMIT, &[limit.unwrap_or(0)]);
                        }
                    });
                });
                if let Some(pct) = current {
                    ui.add_space(6.0);
                    let mut val = pct;
                    let resp = ui.add(
                        egui::Slider::new(&mut val, CHARGE_LIMIT_MIN..=CHARGE_LIMIT_MAX)
                            .step_by(CHARGE_LIMIT_STEP as f64)
                            .suffix("%"),
                    );
                    // Only 11 discrete steps, so sending on every change is cheap and
                    // keeps the daemon in sync with the periodic status poll.
                    if resp.changed() {
                        self.status.config.set_charge_limit(Some(val));
                        Self::send_ipc(CMD_SET_CHARGE_LIMIT, &[val]);
                    }
                }
            });
    }

    fn send_ipc(cmd: u8, data: &[u8]) {
        let stream_opt = UnixStream::connect(SOCKET_PATH)
            .or_else(|_| UnixStream::connect(ALT_SOCKET_PATH));
        if let Ok(mut stream) = stream_opt {
            let mut msg = IpcMessage::new(cmd);
            let len = data.len().min(64);
            msg.data[..len].copy_from_slice(&data[..len]);
            msg.payload_len = len as u16;
            let _ = stream.write_all(msg.as_bytes());
        }
    }

    fn fetch_daemon_status() -> Option<DaemonStatus> {
        let stream_opt = UnixStream::connect(SOCKET_PATH)
            .or_else(|_| UnixStream::connect(ALT_SOCKET_PATH));
        if let Ok(mut stream) = stream_opt {
            let msg = IpcMessage::new(CMD_GET_STATUS);
            if stream.write_all(msg.as_bytes()).is_ok() {
                let mut buf = [0u8; std::mem::size_of::<DaemonStatus>()];
                if stream.read_exact(&mut buf).is_ok() {
                    return DaemonStatus::from_bytes(&buf);
                }
            }
        }
        None
    }

    fn accent_color(&self) -> Color32 {
        match self.status.config.power_profile {
            POWER_PROFILE_ECO => COLOR_ECO,
            POWER_PROFILE_BALANCED => COLOR_BALANCED,
            _ => COLOR_PERF,
        }
    }

    fn keycode_display_name(code: u16) -> &'static str {
        match code {
            149 => "KEY_PROG2 / OMEN Key (149)",
            148 => "KEY_PROG1 / OMEN Key (148)",
            200 => "XF86Launch2",
            194 => "KEY_F24",
            67 => "KEY_F9",
            68 => "KEY_F10",
            87 => "KEY_F11",
            88 => "KEY_F12",
            133 => "KEY_CONFIG",
            _ => "Custom Hotkey",
        }
    }

    fn toggle_autostart(enable: bool) {
        if let Some(home) = std::env::var_os("HOME") {
            let autostart_dir = PathBuf::from(home).join(".config/autostart");
            let desktop_path = autostart_dir.join("lightomen-tray.desktop");
            let legacy_path = autostart_dir.join("lightomen-gui.desktop");
            let _ = fs::remove_file(legacy_path);

            if enable {
                let _ = fs::create_dir_all(&autostart_dir);
                let content = "[Desktop Entry]\nType=Application\nName=LightOmen Tray\nExec=env __NV_PRIME_RENDER_OFFLOAD=0 /usr/local/bin/lightomen-tray\nHidden=false\nNoDisplay=false\nX-GNOME-Autostart-enabled=true\n";
                let _ = fs::write(desktop_path, content);
            } else {
                let _ = fs::remove_file(desktop_path);
            }
        }
    }

    /// Brings the LightOmen GUI window to the front and focuses it natively under KWin Wayland or X11
    pub fn activate_window_native() {
        // 1. KWin Wayland DBus scripting
        let script = r#"
            let clients = workspace.windowList();
            for (let i = 0; i < clients.length; i++) {
                let c = clients[i];
                if (c.resourceClass === 'lightomen' || c.caption.indexOf('LightOmen') !== -1) {
                    c.minimized = false;
                    workspace.activeWindow = c;
                    break;
                }
            }
        "#;
        let mut temp = std::env::temp_dir();
        temp.push("lightomen_focus.js");
        if let Ok(mut f) = fs::File::create(&temp) {
            let _ = f.write_all(script.as_bytes());
            let _ = f.flush();
            if let Ok(output) = Command::new("qdbus6")
                .args(["org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting.loadScript", temp.to_str().unwrap_or(""), "lightomen_focus"])
                .output()
            {
                let sid = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !sid.is_empty() {
                    let _ = Command::new("qdbus6")
                        .args(["org.kde.KWin", &format!("/Scripting/Script{}", sid), "org.kde.kwin.Script.run"])
                        .output();
                    let _ = Command::new("qdbus6")
                        .args(["org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting.unloadScript", "lightomen_focus"])
                        .output();
                }
            }
            let _ = fs::remove_file(&temp);
        }

        // 2. X11 / XWayland wmctrl fallback
        let _ = Command::new("wmctrl")
            .args(["-x", "-a", "lightomen"])
            .spawn();
    }

    /// Collects hardware, kernel driver, sysfs, and daemon diagnostic information
    pub fn collect_diagnostics() -> String {
        let mut out = String::new();
        out.push_str("====================================================\n");
        out.push_str("       LIGHTOMEN SYSTEM & HARDWARE DIAGNOSTICS      \n");
        out.push_str("====================================================\n\n");

        // 1. Board & DMI
        out.push_str("[1] DMI & Hardware Platform:\n");
        for (path, name) in [
            ("/sys/class/dmi/id/board_name", "Board Name"),
            ("/sys/class/dmi/id/product_name", "Product Name"),
            ("/sys/class/dmi/id/bios_version", "BIOS Version"),
            ("/sys/class/dmi/id/sys_vendor", "Vendor"),
        ] {
            let val = fs::read_to_string(path).unwrap_or_else(|_| "N/A".to_string());
            out.push_str(&format!("  • {}: {}\n", name, val.trim()));
        }

        // 2. Kernel & Drivers
        out.push_str("\n[2] Kernel & Drivers:\n");
        let kver = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_else(|_| "Unknown".to_string());
        out.push_str(&format!("  • Kernel Release: {}\n", kver.trim()));

        let hpwmi_path = Command::new("sh")
            .arg("-c")
            .arg("export PATH=/usr/sbin:/sbin:$PATH; modinfo -F filename hp_wmi 2>/dev/null || echo 'not found'")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "error".to_string());
        out.push_str(&format!("  • hp_wmi module path: {}\n", hpwmi_path));

        let hpomen_path = Command::new("sh")
            .arg("-c")
            .arg("export PATH=/usr/sbin:/sbin:$PATH; modinfo -F filename hp_omen_extra 2>/dev/null || echo 'not found'")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "error".to_string());
        out.push_str(&format!("  • hp_omen_extra path: {}\n", hpomen_path));

        let lsmod_out = Command::new("sh")
            .arg("-c")
            .arg("lsmod | grep -E 'hp_wmi|hp_omen_extra' || echo 'none loaded'")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "error".to_string());
        out.push_str(&format!("  • Active modules: {}\n", lsmod_out.replace('\n', " | ")));

        let dkms_out = Command::new("sh")
            .arg("-c")
            .arg("export PATH=/usr/sbin:/sbin:$PATH; dkms status 2>/dev/null || echo 'dkms not available'")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "error".to_string());
        out.push_str(&format!("  • DKMS status: {}\n", dkms_out.replace('\n', " | ")));

        // 3. Hwmon Fan & Power Sysfs
        out.push_str("\n[3] Hwmon & ACPI Sysfs Status:\n");
        for (path, name) in [
            ("/sys/firmware/acpi/platform_profile", "platform_profile"),
            ("/sys/firmware/acpi/platform_profile_choices", "platform_profile_choices"),
            ("/sys/devices/platform/hp-wmi/gpu_mux_mode", "gpu_mux_mode"),
            ("/sys/devices/platform/hp-wmi/graphics_mode", "graphics_mode"),
            ("/sys/devices/platform/hp-wmi/gpu_tgp", "gpu_tgp"),
            ("/sys/devices/platform/hp-wmi/gpu_ppab", "gpu_ppab"),
        ] {
            if Path::new(path).exists() {
                let val = fs::read_to_string(path).unwrap_or_else(|_| "error".to_string());
                out.push_str(&format!("  • {}: {}\n", name, val.trim()));
            }
        }

        // Search hwmon nodes under hp-wmi
        if let Ok(entries) = fs::read_dir("/sys/devices/platform/hp-wmi/hwmon") {
            for entry in entries.flatten() {
                let p = entry.path();
                for node in ["pwm1_enable", "fan1_target", "fan2_target", "fan1_input", "fan2_input", "fan1_max", "pwm1"] {
                    let np = p.join(node);
                    if np.exists() {
                        let val = fs::read_to_string(&np).unwrap_or_else(|_| "err".to_string());
                        out.push_str(&format!("  • hwmon/{}: {}\n", node, val.trim()));
                    }
                }
            }
        }

        // 4. Daemon IPC & Telemetry
        out.push_str("\n[4] Daemon IPC Status:\n");
        let sock_exists = Path::new(SOCKET_PATH).exists();
        out.push_str(&format!("  • Socket {} exists: {}\n", SOCKET_PATH, sock_exists));
        if let Some(status) = Self::fetch_daemon_status() {
            out.push_str(&format!("  • Telemetry: CPU {}°C | GPU {}°C | Fan1: {} RPM | Fan2: {} RPM\n",
                status.telemetry.cpu_temp, status.telemetry.gpu_temp, status.telemetry.fan1_rpm, status.telemetry.fan2_rpm));
            out.push_str(&format!("  • Power State: CPU {} mW | GPU {} mW | On Battery: {}\n",
                status.telemetry.cpu_power_mw, status.telemetry.gpu_power_mw, status.on_battery == 1));
            out.push_str(&format!("  • Config: FanMode={} (0=Auto, 1=Max, 2=Curve), Profile={}, MUX={}\n",
                status.config.fan_mode, status.config.power_profile, status.config.mux_mode));
            out.push_str(&format!("  • CPU Fan Curve: {:?}\n", status.config.cpu_curve));
            out.push_str(&format!("  • GPU Fan Curve: {:?}\n", status.config.gpu_curve));
        } else {
            out.push_str("  • Could not connect to daemon IPC socket.\n");
        }

        // 5. Recent Systemd Journal
        out.push_str("\n[5] Recent Daemon Logs (journalctl -u lightomen-daemon -n 25):\n");
        let journal_out = Command::new("journalctl")
            .args(["-u", "lightomen-daemon.service", "--no-pager", "-n", "25"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "journalctl not available".to_string());
        out.push_str(&journal_out);
        out.push_str("\n\n====================================================\n");

        out
    }
}

impl eframe::App for LightOmenApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Cache context for background IPC wakeup
        if let Ok(mut guard) = self.ctx_holder.lock() {
            if guard.is_none() {
                *guard = Some(ctx.clone());
            }
        }

        // Focus & bring-to-front handling when requested from tray / hotkey
        if self.needs_focus.swap(false, std::sync::atomic::Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Critical));
            Self::activate_window_native();
            self.reset_window_level_at = Some(Instant::now() + Duration::from_millis(250));
        }

        if let Some(reset_time) = self.reset_window_level_at {
            if Instant::now() >= reset_time {
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
                self.reset_window_level_at = None;
            }
        }

        // Poll daemon regularly
        let interval = Duration::from_millis(self.status.config.probe_interval_ms as u64);
        if self.last_poll.elapsed() >= interval {
            if let Some(st) = Self::fetch_daemon_status() {
                if self.active_drag_idx.is_some() {
                    self.status.telemetry = st.telemetry;
                } else {
                    self.status = st;
                }
            }
            self.cached_cpu_ghz = Self::read_cpu_ghz();
            self.last_poll = Instant::now();
        }

        ctx.request_repaint_after(Duration::from_millis(60));

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            std::process::exit(0);
        }

        let accent = self.accent_color();

        // 1. LEFT NAVIGATION RAIL (Fixed 58px)
        egui::SidePanel::left("nav_rail")
            .resizable(false)
            .exact_width(58.0)
            .frame(egui::Frame::none().fill(BG_SUNKEN).inner_margin(Vec2::new(8.0, 14.0)))
            .show(ctx, |ui| {
                self.render_navigation_rail(ui, accent);
            });

        // 2. MAIN CONTENT AREA (Takes all remaining window space, naturally vertical)
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG_SUNKEN).inner_margin(16.0))
            .show(ctx, |ui| {
                // Top Header (Draggable on Wayland)
                self.render_top_header(ui, accent);

                ui.add_space(12.0);

                match self.active_tab {
                    ActiveTab::Dashboard => self.render_dashboard(ui, accent),
                    ActiveTab::FanCurve => self.render_fan_curve_tab(ui, accent),
                    ActiveTab::GraphicsMux => self.render_graphics_tab(ui, accent),
                    ActiveTab::SystemInfo => self.render_system_info_tab(ui, accent),
                    ActiveTab::Settings => self.render_settings_tab(ui, accent),
                }
            });
    }
}

impl LightOmenApp {
    /// Left vertical navigation rail with tab buttons (Dashboard, Curve, MUX, Specs, Settings)
    fn render_navigation_rail(&mut self, ui: &mut Ui, accent: Color32) {
        ui.vertical_centered(|ui| {
            // Brand Icon / OMEN Diamond
            let (diamond_rect, _) = ui.allocate_exact_size(Vec2::new(28.0, 28.0), Sense::hover());
            let p = diamond_rect.center();
            let d = 8.0;
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(p.x, p.y - d),
                    Pos2::new(p.x + d, p.y),
                    Pos2::new(p.x, p.y + d),
                    Pos2::new(p.x - d, p.y),
                ],
                accent,
                Stroke::NONE,
            ));

            ui.add_space(18.0);

            // Tab 0: Dashboard (Home / Telemetry)
            let is_active0 = self.active_tab == ActiveTab::Dashboard;
            let tip0 = if self.language.is_tr() { "Kontrol Paneli" } else { "Dashboard" };
            if render_rail_tab_icon(ui, "⚡", is_active0, accent, tip0).clicked() {
                self.active_tab = ActiveTab::Dashboard;
            }

            ui.add_space(12.0);

            // Tab 1: Fan Curve
            let is_active1 = self.active_tab == ActiveTab::FanCurve;
            let tip1 = if self.language.is_tr() { "Fan Grafiği" } else { "Fan Curve" };
            if render_rail_tab_icon(ui, "📈", is_active1, accent, tip1).clicked() {
                self.active_tab = ActiveTab::FanCurve;
            }

            ui.add_space(12.0);

            // Tab 2: Graphics / MUX Switch
            let is_active2 = self.active_tab == ActiveTab::GraphicsMux;
            let tip2 = if self.language.is_tr() { "Grafik MUX Anahtarı" } else { "Graphics MUX Switch" };
            if render_rail_tab_icon(ui, "🎮", is_active2, accent, tip2).clicked() {
                self.active_tab = ActiveTab::GraphicsMux;
            }

            ui.add_space(12.0);

            // Tab 3: System Hardware Specs
            let is_active3 = self.active_tab == ActiveTab::SystemInfo;
            let tip3 = if self.language.is_tr() { "Sistem Özellikleri" } else { "System Specifications" };
            if render_rail_tab_icon(ui, "💻", is_active3, accent, tip3).clicked() {
                self.active_tab = ActiveTab::SystemInfo;
            }

            ui.add_space(12.0);

            // Tab 4: Settings (Gear)
            let is_active4 = self.active_tab == ActiveTab::Settings;
            let tip4 = if self.language.is_tr() { "Ayarlar ve OMEN Tuşu" } else { "Settings & OMEN Key" };
            if render_rail_tab_icon(ui, "⚙", is_active4, accent, tip4).clicked() {
                self.active_tab = ActiveTab::Settings;
            }
        });
    }

    /// Top header: allows smooth window dragging on Wayland (left or right click drag) and provides sleek close button
    fn render_top_header(&mut self, ui: &mut Ui, accent: Color32) {
        let avail_w = ui.available_width();
        let header_h = 32.0;

        let start_pos = ui.cursor().min;
        let header_rect = egui::Rect::from_min_size(start_pos, Vec2::new(avail_w, header_h));

        // Drag handle across the header area (Left Click or Right Click drag on Wayland)
        let drag_resp = ui.interact(
            header_rect,
            ui.id().with("header_drag_area"),
            Sense::click_and_drag(),
        );

        let is_secondary_moving = ui.input(|i| i.pointer.button_down(egui::PointerButton::Secondary) && i.pointer.is_moving());
        if drag_resp.drag_started_by(egui::PointerButton::Primary)
            || drag_resp.drag_started_by(egui::PointerButton::Secondary)
            || (drag_resp.hovered() && is_secondary_moving)
        {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        ui.horizontal(|ui| {
            // Title & Board ID badge
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("LightOmen")
                        .font(FontId::new(17.0, FontFamily::Proportional))
                        .color(TEXT_HI)
                        .strong(),
                );

                // Board ID Badge (8BD4)
                let board_str = std::str::from_utf8(&self.status.telemetry.board_id)
                    .unwrap_or("8BD4")
                    .trim_matches('\0');
                let badge_text = if board_str.is_empty() { "8BD4" } else { board_str };

                egui::Frame::none()
                    .fill(PILL_BG)
                    .stroke(Stroke::new(1.0, LINE_BORDER))
                    .rounding(Rounding::same(4.0))
                    .inner_margin(Vec2::new(6.0, 2.0))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(badge_text)
                                .font(FontId::new(10.5, FontFamily::Monospace))
                                .color(accent),
                        );
                    });
            });

            // Right side of header: Close Button with dedicated vector icon & Mode Name
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Sleek Close Button [X] Icon (Vector drawn, hover lights up crimson red)
                let (close_rect, close_resp) = ui.allocate_exact_size(Vec2::new(26.0, 26.0), Sense::click());
                let is_hovered = close_resp.hovered();
                let bg_color = if is_hovered { Color32::from_rgb(0xC0, 0x39, 0x2B) } else { PILL_BG };
                let stroke_color = if is_hovered { Color32::from_rgb(0xE7, 0x4C, 0x3C) } else { LINE_BORDER };
                let icon_color = if is_hovered { Color32::WHITE } else { TEXT_MUTED };

                ui.painter().rect(
                    close_rect,
                    Rounding::same(5.0),
                    bg_color,
                    Stroke::new(1.0, stroke_color),
                );

                // Anti-aliased geometric cross (X)
                let c = close_rect.center();
                let d = 4.5;
                ui.painter().line_segment(
                    [Pos2::new(c.x - d, c.y - d), Pos2::new(c.x + d, c.y + d)],
                    Stroke::new(1.8, icon_color),
                );
                ui.painter().line_segment(
                    [Pos2::new(c.x - d, c.y + d), Pos2::new(c.x + d, c.y - d)],
                    Stroke::new(1.8, icon_color),
                );

                if close_resp.clicked() {
                    std::process::exit(0);
                }

                ui.add_space(10.0);

                // Header active mode label
                let mode_name = match self.status.config.power_profile {
                    POWER_PROFILE_ECO => "Eco",
                    POWER_PROFILE_BALANCED => "Balanced",
                    _ => "Performance",
                };
                ui.label(
                    egui::RichText::new(mode_name)
                        .font(FontId::new(12.0, FontFamily::Proportional))
                        .color(accent),
                );
            });
        });
    }

    /// Tab 0: Dashboard (Temps, RPM, Power Profiles, Fan Auto/Max/Curve, Real-time Power Card)
    fn render_dashboard(&mut self, ui: &mut Ui, accent: Color32) {
        let is_gpu_asleep = self.status.telemetry.gpu_temp == 0 || self.status.telemetry.gpu_power_mw == 0;

        ui.columns(2, |cols| {
            // CPU Telemetry Card (Clean without watt clutter)
            egui::Frame::none()
                .fill(CARD_BG)
                .stroke(Stroke::new(1.0, LINE_BORDER))
                .rounding(Rounding::same(8.0))
                .inner_margin(14.0)
                .show(&mut cols[0], |ui| {
                    ui.label(egui::RichText::new("CPU").font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_MUTED));
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{}", self.status.telemetry.cpu_temp))
                                .font(FontId::new(36.0, FontFamily::Proportional))
                                .color(TEXT_HI)
                                .strong(),
                        );
                        ui.label(egui::RichText::new("°C").font(FontId::new(18.0, FontFamily::Proportional)).color(TEXT_MUTED));
                    });
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("AMD Ryzen 5 7640HS")
                            .font(FontId::new(11.0, FontFamily::Proportional))
                            .color(TEXT_DIM),
                    );
                    ui.add_space(6.0);
                    let f1 = self.status.telemetry.fan1_rpm;
                    let is_bat = self.status.on_battery == 1;
                    let boost_disabled = !self.status.config.cpu_boost_enabled()
                        || (is_bat && self.status.config.no_boost_on_battery());
                    let boost_suffix = if boost_disabled {
                        if self.language.is_tr() { " (Boost Kapalı)" } else { " (Boost Off)" }
                    } else {
                        ""
                    };
                    let ghz_str = if let Some(ghz) = self.cached_cpu_ghz {
                        format!(" · {:.2} GHz{}", ghz, boost_suffix)
                    } else {
                        String::new()
                    };
                    let fan_label = if self.language.is_tr() {
                        if f1 > 0 {
                            format!("CPU fanı: {} rpm{}", f1, ghz_str)
                        } else {
                            format!("CPU fanı: 0 rpm (Sessiz){}", ghz_str)
                        }
                    } else {
                        if f1 > 0 {
                            format!("CPU fan: {} rpm{}", f1, ghz_str)
                        } else {
                            format!("CPU fan: 0 rpm (Quiet){}", ghz_str)
                        }
                    };
                    ui.label(egui::RichText::new(fan_label).font(FontId::new(11.5, FontFamily::Monospace)).color(accent));
                });

            // GPU Telemetry Card (Zero wake-up, D3cold asleep indicator)
            egui::Frame::none()
                .fill(CARD_BG)
                .stroke(Stroke::new(1.0, LINE_BORDER))
                .rounding(Rounding::same(8.0))
                .inner_margin(14.0)
                .show(&mut cols[1], |ui| {
                    ui.label(egui::RichText::new("GPU").font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_MUTED));
                    ui.add_space(2.0);
                    if is_gpu_asleep {
                        ui.horizontal(|ui| {
                            let sleep_label = if self.language.is_tr() { "Uyku" } else { "Sleep" };
                            ui.label(
                                egui::RichText::new(sleep_label)
                                    .font(FontId::new(32.0, FontFamily::Proportional))
                                    .color(COLOR_OK)
                                    .strong(),
                            );
                            ui.label(egui::RichText::new("D3cold").font(FontId::new(11.0, FontFamily::Monospace)).color(COLOR_OK));
                        });
                        ui.add_space(8.0);
                        let standby_label = if self.language.is_tr() { "NVIDIA RTX 4050 · Beklemede" } else { "NVIDIA RTX 4050 · Standby" };
                        ui.label(
                            egui::RichText::new(standby_label)
                                .font(FontId::new(11.0, FontFamily::Proportional))
                                .color(TEXT_DIM),
                        );
                    } else {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{}", self.status.telemetry.gpu_temp))
                                    .font(FontId::new(36.0, FontFamily::Proportional))
                                    .color(TEXT_HI)
                                    .strong(),
                            );
                            ui.label(egui::RichText::new("°C").font(FontId::new(18.0, FontFamily::Proportional)).color(TEXT_MUTED));
                        });
                        ui.add_space(4.0);
                        let active_label = if self.language.is_tr() { "NVIDIA RTX 4050 · Aktif" } else { "NVIDIA RTX 4050 · Active" };
                        ui.label(
                            egui::RichText::new(active_label)
                                .font(FontId::new(11.0, FontFamily::Proportional))
                                .color(TEXT_DIM),
                        );
                    }
                    ui.add_space(6.0);
                    let f2 = self.status.telemetry.fan2_rpm;
                    let fan_label = if self.language.is_tr() {
                        if f2 > 0 { format!("GPU fanı: {} rpm", f2) } else { "GPU fanı: 0 rpm (Sessiz)".to_string() }
                    } else {
                        if f2 > 0 { format!("GPU fan: {} rpm", f2) } else { "GPU fan: 0 rpm (Quiet)".to_string() }
                    };
                    ui.label(egui::RichText::new(fan_label).font(FontId::new(11.5, FontFamily::Monospace)).color(accent));
                });
        });

        ui.add_space(10.0);

        // Power Profiles (Eco, Balanced, Performance)
        let pwr_title = if self.language.is_tr() { "GÜÇ PROFİLLERİ" } else { "POWER PROFILES" };
        ui.label(egui::RichText::new(pwr_title).font(FontId::new(10.5, FontFamily::Proportional)).color(TEXT_DIM));
        ui.columns(3, |cols| {
            let cur = self.status.config.power_profile;

            // Eco
            let is_eco = cur == POWER_PROFILE_ECO;
            let btn_eco = egui::Button::new(
                egui::RichText::new("Eco")
                    .font(FontId::new(13.0, FontFamily::Proportional))
                    .color(if is_eco { COLOR_ECO } else { TEXT_PRIMARY }),
            )
            .fill(if is_eco { PILL_BG } else { CARD_BG })
            .stroke(Stroke::new(1.0, if is_eco { COLOR_ECO } else { LINE_BORDER }))
            .rounding(Rounding::same(6.0))
            .min_size(Vec2::new(cols[0].available_width(), 38.0));

            if cols[0].add(btn_eco).clicked() {
                self.status.config.power_profile = POWER_PROFILE_ECO;
                Self::send_ipc(CMD_SET_POWER_PROFILE, &[POWER_PROFILE_ECO]);
            }

            // Balanced
            let is_bal = cur == POWER_PROFILE_BALANCED;
            let btn_bal = egui::Button::new(
                egui::RichText::new("Balanced")
                    .font(FontId::new(13.0, FontFamily::Proportional))
                    .color(if is_bal { COLOR_BALANCED } else { TEXT_PRIMARY }),
            )
            .fill(if is_bal { PILL_BG } else { CARD_BG })
            .stroke(Stroke::new(1.0, if is_bal { COLOR_BALANCED } else { LINE_BORDER }))
            .rounding(Rounding::same(6.0))
            .min_size(Vec2::new(cols[1].available_width(), 38.0));

            if cols[1].add(btn_bal).clicked() {
                self.status.config.power_profile = POWER_PROFILE_BALANCED;
                Self::send_ipc(CMD_SET_POWER_PROFILE, &[POWER_PROFILE_BALANCED]);
            }

            // Performance
            let is_perf = cur == POWER_PROFILE_PERFORMANCE;
            let btn_perf = egui::Button::new(
                egui::RichText::new("Performance")
                    .font(FontId::new(13.0, FontFamily::Proportional))
                    .color(if is_perf { COLOR_PERF } else { TEXT_PRIMARY }),
            )
            .fill(if is_perf { PILL_BG } else { CARD_BG })
            .stroke(Stroke::new(1.0, if is_perf { COLOR_PERF } else { LINE_BORDER }))
            .rounding(Rounding::same(6.0))
            .min_size(Vec2::new(cols[2].available_width(), 38.0));

            if cols[2].add(btn_perf).clicked() {
                self.status.config.power_profile = POWER_PROFILE_PERFORMANCE;
                Self::send_ipc(CMD_SET_POWER_PROFILE, &[POWER_PROFILE_PERFORMANCE]);
            }
        });

        ui.add_space(10.0);

        // Fan Control (Auto, Max, Curve / Grafik)
        let fans_title = if self.language.is_tr() { "FANLAR" } else { "FANS" };
        ui.label(egui::RichText::new(fans_title).font(FontId::new(10.5, FontFamily::Proportional)).color(TEXT_DIM));
        ui.columns(3, |cols| {
            let cur = self.status.config.fan_mode;

            // Auto
            let is_auto = cur == FAN_MODE_AUTO;
            let btn_auto = egui::Button::new(
                egui::RichText::new("Auto")
                    .font(FontId::new(13.0, FontFamily::Proportional))
                    .color(if is_auto { accent } else { TEXT_PRIMARY }),
            )
            .fill(if is_auto { PILL_BG } else { CARD_BG })
            .stroke(Stroke::new(1.0, if is_auto { accent } else { LINE_BORDER }))
            .rounding(Rounding::same(6.0))
            .min_size(Vec2::new(cols[0].available_width(), 38.0));

            if cols[0].add(btn_auto).clicked() {
                self.status.config.fan_mode = FAN_MODE_AUTO;
                Self::send_ipc(CMD_SET_FAN_MODE, &[FAN_MODE_AUTO]);
            }

            // Max
            let is_max = cur == FAN_MODE_MAX;
            let btn_max = egui::Button::new(
                egui::RichText::new("Max")
                    .font(FontId::new(13.0, FontFamily::Proportional))
                    .color(if is_max { COLOR_PERF } else { TEXT_PRIMARY }),
            )
            .fill(if is_max { PILL_BG } else { CARD_BG })
            .stroke(Stroke::new(1.0, if is_max { COLOR_PERF } else { LINE_BORDER }))
            .rounding(Rounding::same(6.0))
            .min_size(Vec2::new(cols[1].available_width(), 38.0));

            if cols[1].add(btn_max).clicked() {
                self.status.config.fan_mode = FAN_MODE_MAX;
                Self::send_ipc(CMD_SET_FAN_MODE, &[FAN_MODE_MAX]);
            }

            // Curve: When clicked, sets Curve mode AND navigates directly to Tab 1!
            let is_curve = cur == FAN_MODE_CURVE;
            let curve_label = if self.language.is_tr() { "Grafik →" } else { "Curve →" };
            let btn_curve = egui::Button::new(
                egui::RichText::new(curve_label)
                    .font(FontId::new(13.0, FontFamily::Proportional))
                    .color(if is_curve { accent } else { TEXT_PRIMARY }),
            )
            .fill(if is_curve { PILL_BG } else { CARD_BG })
            .stroke(Stroke::new(1.0, if is_curve { accent } else { LINE_BORDER }))
            .rounding(Rounding::same(6.0))
            .min_size(Vec2::new(cols[2].available_width(), 38.0));

            if cols[2].add(btn_curve).clicked() {
                self.status.config.fan_mode = FAN_MODE_CURVE;
                Self::send_ipc(CMD_SET_FAN_MODE, &[FAN_MODE_CURVE]);
                self.active_tab = ActiveTab::FanCurve;
            }
        });

        ui.add_space(10.0);

        // Real-time Power Consumption Card (RAPL Live CPU & D3cold dGPU Zero-Watt)
        let cpu_watts = self.status.telemetry.cpu_power_mw as f32 / 1000.0;
        let gpu_watts = self.status.telemetry.gpu_power_mw as f32 / 1000.0;
        let total_watts = cpu_watts + gpu_watts;
        let on_battery = self.status.on_battery == 1;
        let battery_pct = self.status.battery_pct;

        egui::Frame::none()
            .fill(CARD_BG)
            .stroke(Stroke::new(1.0, LINE_BORDER))
            .rounding(Rounding::same(8.0))
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let card_title = if self.language.is_tr() {
                        if on_battery {
                            format!("🔋 BATARYA DEŞARJI (%{battery_pct})")
                        } else {
                            "⚡ ANLIK GÜÇ TÜKETİMİ (AC BAĞLI)".to_string()
                        }
                    } else {
                        if on_battery {
                            format!("🔋 BATTERY DISCHARGE ({battery_pct}%)")
                        } else {
                            "⚡ REAL-TIME POWER DRAIN (AC ONLINE)".to_string()
                        }
                    };
                    ui.label(
                        egui::RichText::new(card_title)
                            .font(FontId::new(10.5, FontFamily::Proportional))
                            .color(if on_battery { COLOR_ECO } else { TEXT_MUTED }),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let comb_suffix = if self.language.is_tr() { "Toplam" } else { "Combined" };
                        ui.label(
                            egui::RichText::new(format!("{:.1} W {}", total_watts, comb_suffix))
                                .font(FontId::new(11.5, FontFamily::Monospace))
                                .color(accent)
                                .strong(),
                        );
                    });
                });

                ui.add_space(8.0);

                ui.columns(2, |cols| {
                    // CPU Package (RAPL)
                    egui::Frame::none()
                        .fill(PILL_BG)
                        .stroke(Stroke::new(1.0, LINE_BORDER))
                        .rounding(Rounding::same(6.0))
                        .inner_margin(Vec2::new(10.0, 8.0))
                        .show(&mut cols[0], |ui| {
                            ui.label(egui::RichText::new("CPU Package (RAPL)").font(FontId::new(10.0, FontFamily::Proportional)).color(TEXT_MUTED));
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("{:.1}", cpu_watts))
                                        .font(FontId::new(17.0, FontFamily::Monospace))
                                        .color(TEXT_HI)
                                        .strong(),
                                );
                                ui.label(egui::RichText::new("W").font(FontId::new(11.0, FontFamily::Monospace)).color(TEXT_DIM));
                            });
                            let rapl_sub = if self.language.is_tr() { "Donanım güç sınırı (Uyanma yok)" } else { "Hardware powercap (No wake)" };
                            ui.label(egui::RichText::new(rapl_sub).font(FontId::new(9.5, FontFamily::Proportional)).color(TEXT_DIM));
                        });

                    // NVIDIA dGPU (PCIe D3cold)
                    egui::Frame::none()
                        .fill(PILL_BG)
                        .stroke(Stroke::new(1.0, LINE_BORDER))
                        .rounding(Rounding::same(6.0))
                        .inner_margin(Vec2::new(10.0, 8.0))
                        .show(&mut cols[1], |ui| {
                            ui.label(egui::RichText::new("NVIDIA dGPU").font(FontId::new(10.0, FontFamily::Proportional)).color(TEXT_MUTED));
                            if is_gpu_asleep {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("0.0")
                                            .font(FontId::new(17.0, FontFamily::Monospace))
                                            .color(COLOR_OK)
                                            .strong(),
                                    );
                                    ui.label(egui::RichText::new("W").font(FontId::new(11.0, FontFamily::Monospace)).color(COLOR_OK));
                                    ui.label(egui::RichText::new("· D3cold").font(FontId::new(10.5, FontFamily::Monospace)).color(COLOR_OK));
                                });
                                let deep_sub = if self.language.is_tr() { "Derin uyku / Sıfır güç (0 W)" } else { "Deep sleep / Zero watt (0 W)" };
                                ui.label(egui::RichText::new(deep_sub).font(FontId::new(9.5, FontFamily::Proportional)).color(COLOR_OK));
                            } else {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(format!("~{:.0}", gpu_watts))
                                            .font(FontId::new(17.0, FontFamily::Monospace))
                                            .color(TEXT_HI)
                                            .strong(),
                                    );
                                    ui.label(egui::RichText::new("W").font(FontId::new(11.0, FontFamily::Monospace)).color(TEXT_DIM));
                                    let act_label = if self.language.is_tr() { "· Aktif" } else { "· Active" };
                                    ui.label(egui::RichText::new(act_label).font(FontId::new(10.5, FontFamily::Monospace)).color(COLOR_PERF));
                                });
                                let act_sub = if self.language.is_tr() { "Aktif güç tüketimi" } else { "Active power draw" };
                                ui.label(egui::RichText::new(act_sub).font(FontId::new(9.5, FontFamily::Proportional)).color(TEXT_DIM));
                            }
                        });
                });
            });

        ui.add_space(14.0);

        // Footer status
        ui.horizontal(|ui| {
            let (dot_rect, _) = ui.allocate_exact_size(Vec2::new(8.0, 8.0), Sense::hover());
            ui.painter().circle_filled(dot_rect.center(), 3.5, COLOR_OK);
            let daemon_msg = if self.language.is_tr() { "Daemon Aktif · /run/lightomen/lightomen.sock" } else { "Daemon Active · /run/lightomen/lightomen.sock" };
            ui.label(egui::RichText::new(daemon_msg).font(FontId::new(10.5, FontFamily::Monospace)).color(TEXT_DIM));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let power_str = if self.language.is_tr() {
                    if on_battery { format!("🔋 Batarya %{}", battery_pct) } else { "⚡ AC Bağlı".to_string() }
                } else {
                    if on_battery { format!("🔋 Battery {}%", battery_pct) } else { "⚡ AC Online".to_string() }
                };
                let probe_word = if self.language.is_tr() { "Aralık" } else { "Probe" };
                ui.label(egui::RichText::new(format!("{} · {} {}s", power_str, probe_word, self.status.config.probe_interval_ms as f32 / 1000.0)).font(FontId::new(10.5, FontFamily::Monospace)).color(TEXT_DIM));
            });
        });
    }

    /// Tab 1: Fan Curve Editor with Link curves toggle, bounds checking, and live temp marker
    fn render_fan_curve_tab(&mut self, ui: &mut Ui, accent: Color32) {
        let curve_title = if self.language.is_tr() { "FAN GRAFİĞİ DÜZENLEYİCİ" } else { "FAN CURVE EDITOR" };
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(curve_title).font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_HI).strong());

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Link CPU and GPU Curve toggle button
                let link_label = if self.language.is_tr() {
                    if self.link_curves { "🔗 Bağlı (CPU + GPU)" } else { "🔓 Bağımsız Grafikler" }
                } else {
                    if self.link_curves { "🔗 Linked (CPU + GPU)" } else { "🔓 Independent Curves" }
                };
                let link_btn = ui.add(
                    egui::Button::new(egui::RichText::new(link_label).font(FontId::new(11.0, FontFamily::Proportional)).color(if self.link_curves { accent } else { TEXT_MUTED }))
                        .fill(PILL_BG)
                        .stroke(Stroke::new(1.0, if self.link_curves { accent } else { LINE_BORDER }))
                        .rounding(Rounding::same(4.0)),
                );
                if link_btn.clicked() {
                    self.link_curves = !self.link_curves;
                    Self::save_gui_config(self.link_curves);
                    if self.link_curves {
                        // Sync GPU curve to CPU curve
                        self.status.config.gpu_curve = self.status.config.cpu_curve;
                        let mut data = [0u8; 14];
                        data[0..7].copy_from_slice(&self.status.config.cpu_curve);
                        data[7..14].copy_from_slice(&self.status.config.gpu_curve);
                        Self::send_ipc(CMD_SET_FAN_CURVES, &data);
                    }
                }
            });
        });

        ui.add_space(6.0);

        // Sub-tabs if unlinked
        if !self.link_curves {
            ui.horizontal(|ui| {
                let cpu_tab_label = if self.language.is_tr() { "CPU Fan Grafiği" } else { "CPU Fan Curve" };
                let gpu_tab_label = if self.language.is_tr() { "GPU Fan Grafiği" } else { "GPU Fan Curve" };
                if ui.selectable_label(!self.curve_tab_gpu, cpu_tab_label).clicked() {
                    self.curve_tab_gpu = false;
                }
                if ui.selectable_label(self.curve_tab_gpu, gpu_tab_label).clicked() {
                    self.curve_tab_gpu = true;
                }
            });
            ui.add_space(4.0);
        }

        // Curve Canvas
        egui::Frame::none()
            .fill(CARD_BG)
            .stroke(Stroke::new(1.0, LINE_BORDER))
            .rounding(Rounding::same(8.0))
            .inner_margin(14.0)
            .show(ui, |ui| {
                let available_w = ui.available_width();
                let height = 180.0;
                let (rect, response) = ui.allocate_exact_size(Vec2::new(available_w, height), Sense::click_and_drag());

                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, Rounding::same(6.0), BG_SUNKEN);
                painter.rect_stroke(rect, Rounding::same(6.0), Stroke::new(1.0, LINE_BORDER));

                let pad_l = 32.0;
                let pad_r = 16.0;
                let pad_t = 16.0;
                let pad_b = 24.0;
                let plot_w = rect.width() - pad_l - pad_r;
                let plot_h = rect.height() - pad_t - pad_b;

                // Horizontal grid lines (0%, 25%, 50%, 75%, 100%)
                for p in [0.0, 0.25, 0.5, 0.75, 1.0] {
                    let y = rect.min.y + pad_t + (1.0 - p) * plot_h;
                    painter.line_segment([Pos2::new(rect.min.x + pad_l, y), Pos2::new(rect.max.x - pad_r, y)], Stroke::new(1.0, LINE_LIGHT));
                    painter.text(Pos2::new(rect.min.x + pad_l - 4.0, y), egui::Align2::RIGHT_CENTER, format!("{}%", (p * 100.0) as u32), FontId::new(9.5, FontFamily::Monospace), TEXT_AXIS);
                }

                // Vertical grid lines (30°C to 90°C)
                let num_points = CURVE_TEMPS.len();
                let mut screen_points = Vec::with_capacity(num_points);

                for (i, &temp) in CURVE_TEMPS.iter().enumerate() {
                    let x = rect.min.x + pad_l + (i as f32 / (num_points - 1) as f32) * plot_w;
                    painter.line_segment([Pos2::new(x, rect.min.y + pad_t), Pos2::new(x, rect.min.y + pad_t + plot_h)], Stroke::new(1.0, LINE_LIGHT));
                    painter.text(Pos2::new(x, rect.max.y - pad_b + 12.0), egui::Align2::CENTER_CENTER, format!("{}°", temp), FontId::new(9.5, FontFamily::Monospace), TEXT_AXIS);

                    let pct = if self.curve_tab_gpu && !self.link_curves {
                        self.status.config.gpu_curve[i] as f32
                    } else {
                        self.status.config.cpu_curve[i] as f32
                    };
                    let y = rect.min.y + pad_t + (1.0 - (pct / 100.0).clamp(0.0, 1.0)) * plot_h;
                    screen_points.push(Pos2::new(x, y));
                }

                // Draw curve line
                for i in 0..(num_points - 1) {
                    painter.line_segment([screen_points[i], screen_points[i + 1]], Stroke::new(2.5, accent));
                }

                // Live Temperature Indicator Marker
                let cur_temp = if self.curve_tab_gpu && !self.link_curves { self.status.telemetry.gpu_temp } else { self.status.telemetry.cpu_temp };
                if (30..=90).contains(&cur_temp) {
                    let frac = (cur_temp - 30) as f32 / 60.0;
                    let live_x = rect.min.x + pad_l + frac * plot_w;
                    let live_pct = fan_speed_at(&self.status.config.cpu_curve, cur_temp) as f32;
                    let live_y = rect.min.y + pad_t + (1.0 - (live_pct / 100.0)) * plot_h;
                    painter.circle_filled(Pos2::new(live_x, live_y), 3.5, Color32::WHITE);
                }

                // Interactive control points
                let mouse_pos = response.interact_pointer_pos();
                let mut curves_changed = false;

                for (i, &pt) in screen_points.iter().enumerate() {
                    let is_hovered = mouse_pos.map_or(false, |m| m.distance(pt) < 14.0);

                    if response.drag_started() && is_hovered {
                        self.active_drag_idx = Some(i);
                    }

                    if response.dragged() && self.active_drag_idx == Some(i) {
                        if let Some(m) = mouse_pos {
                            let rel_y = (m.y - (rect.min.y + pad_t)).clamp(0.0, plot_h);
                            let pct_float = (1.0 - (rel_y / plot_h)) * 100.0;
                            // ClampOrOff: Only point 0 (30°C / idle) can be 0% (Fan Off / 0 RPM).
                            // Points 1..5 (40°C..80°C) must be in operational spinning range (18%..100%).
                            // Point 6 (90°C) is fixed at 100% for thermal safety.
                            let raw_pct = if i == 0 {
                                if pct_float < 10.0 {
                                    0u8
                                } else {
                                    pct_float.round().clamp(18.0, 100.0) as u8
                                }
                            } else if i == 6 {
                                100u8
                            } else {
                                pct_float.round().clamp(18.0, 100.0) as u8
                            };
                            let new_pct = raw_pct;

                            if self.link_curves {
                                self.status.config.cpu_curve[i] = new_pct;
                                // Enforce monotonicity: fan curve never falls as it gets hotter
                                for j in (i + 1)..7 {
                                    if self.status.config.cpu_curve[j] < new_pct {
                                        self.status.config.cpu_curve[j] = new_pct;
                                    }
                                }
                                for j in 0..i {
                                    if self.status.config.cpu_curve[j] > new_pct {
                                        self.status.config.cpu_curve[j] = new_pct;
                                    }
                                }
                                self.status.config.gpu_curve = self.status.config.cpu_curve;
                                curves_changed = true;
                            } else if self.curve_tab_gpu {
                                self.status.config.gpu_curve[i] = new_pct;
                                for j in (i + 1)..7 {
                                    if self.status.config.gpu_curve[j] < new_pct {
                                        self.status.config.gpu_curve[j] = new_pct;
                                    }
                                }
                                for j in 0..i {
                                    if self.status.config.gpu_curve[j] > new_pct {
                                        self.status.config.gpu_curve[j] = new_pct;
                                    }
                                }
                                curves_changed = true;
                            } else {
                                self.status.config.cpu_curve[i] = new_pct;
                                for j in (i + 1)..7 {
                                    if self.status.config.cpu_curve[j] < new_pct {
                                        self.status.config.cpu_curve[j] = new_pct;
                                    }
                                }
                                for j in 0..i {
                                    if self.status.config.cpu_curve[j] > new_pct {
                                        self.status.config.cpu_curve[j] = new_pct;
                                    }
                                }
                                curves_changed = true;
                            }
                        }
                    }

                    let radius = if is_hovered || self.active_drag_idx == Some(i) { 6.0 } else { 4.5 };
                    painter.circle_filled(pt, radius, accent);
                    painter.circle_stroke(pt, radius + 1.0, Stroke::new(1.5, BG_SUNKEN));

                    if is_hovered || self.active_drag_idx == Some(i) {
                        let cur_val = if self.curve_tab_gpu && !self.link_curves { self.status.config.gpu_curve[i] } else { self.status.config.cpu_curve[i] };
                        let off_label = if self.language.is_tr() { "0 RPM (Kapalı)" } else { "0 RPM (Off)" };
                        let label_str = if cur_val == 0 { off_label.to_string() } else { format!("{}%", cur_val) };
                        let label_col = if cur_val == 0 { COLOR_OK } else { TEXT_HI };
                        painter.text(Pos2::new(pt.x, pt.y - 12.0), egui::Align2::CENTER_BOTTOM, label_str, FontId::new(10.5, FontFamily::Monospace), label_col);
                    }
                }

                if response.drag_stopped() {
                    self.active_drag_idx = None;
                }

                if curves_changed {
                    let mut data = [0u8; 14];
                    data[0..7].copy_from_slice(&self.status.config.cpu_curve);
                    data[7..14].copy_from_slice(&self.status.config.gpu_curve);
                    Self::send_ipc(CMD_SET_FAN_CURVES, &data);
                }
            });

        ui.add_space(8.0);

        // Presets & Controls
        ui.horizontal(|ui| {
            let presets_label = if self.language.is_tr() { "Hazır Profiller:" } else { "Presets:" };
            ui.label(egui::RichText::new(presets_label).font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_DIM));

            let quiet_text = if self.language.is_tr() { "Sessiz" } else { "Quiet" };
            if ui.button(quiet_text).clicked() {
                let quiet = [18, 20, 25, 35, 50, 70, 100];
                self.status.config.cpu_curve = quiet;
                self.status.config.gpu_curve = quiet;
                self.status.config.fan_mode = FAN_MODE_CURVE;
                let mut data = [0u8; 14];
                data[0..7].copy_from_slice(&quiet);
                data[7..14].copy_from_slice(&quiet);
                Self::send_ipc(CMD_SET_FAN_CURVES, &data);
                Self::send_ipc(CMD_SET_FAN_MODE, &[FAN_MODE_CURVE]);
            }

            let balanced_text = if self.language.is_tr() { "Dengeli" } else { "Balanced" };
            if ui.button(balanced_text).clicked() {
                self.status.config.cpu_curve = DEFAULT_CPU_CURVE;
                self.status.config.gpu_curve = DEFAULT_GPU_CURVE;
                self.status.config.fan_mode = FAN_MODE_CURVE;
                let mut data = [0u8; 14];
                data[0..7].copy_from_slice(&DEFAULT_CPU_CURVE);
                data[7..14].copy_from_slice(&DEFAULT_GPU_CURVE);
                Self::send_ipc(CMD_SET_FAN_CURVES, &data);
                Self::send_ipc(CMD_SET_FAN_MODE, &[FAN_MODE_CURVE]);
            }

            let agg_text = if self.language.is_tr() { "Agresif" } else { "Aggressive" };
            if ui.button(agg_text).clicked() {
                let agg = [35, 45, 55, 68, 80, 92, 100];
                self.status.config.cpu_curve = agg;
                self.status.config.gpu_curve = agg;
                self.status.config.fan_mode = FAN_MODE_CURVE;
                let mut data = [0u8; 14];
                data[0..7].copy_from_slice(&agg);
                data[7..14].copy_from_slice(&agg);
                Self::send_ipc(CMD_SET_FAN_CURVES, &data);
                Self::send_ipc(CMD_SET_FAN_MODE, &[FAN_MODE_CURVE]);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.status.config.fan_mode != FAN_MODE_CURVE {
                    let apply_label = if self.language.is_tr() { "⚡ Grafiği Uygula ve Başlat" } else { "⚡ Apply & Activate Curve" };
                    let apply_btn = egui::Button::new(
                        egui::RichText::new(apply_label)
                            .font(FontId::new(12.0, FontFamily::Proportional))
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(accent)
                    .rounding(Rounding::same(6.0));
                    if ui.add(apply_btn).clicked() {
                        self.status.config.fan_mode = FAN_MODE_CURVE;
                        Self::send_ipc(CMD_SET_FAN_MODE, &[FAN_MODE_CURVE]);
                    }
                } else {
                    let active_mode_label = if self.language.is_tr() { "● Grafik Modu Aktif" } else { "● Curve Mode Active" };
                    ui.label(
                        egui::RichText::new(active_mode_label)
                            .font(FontId::new(12.0, FontFamily::Proportional))
                            .color(COLOR_OK)
                            .strong(),
                    );
                }
            });
        });
    }

    /// Tab 2: Graphics MUX Switch Tab (Hybrid vs Discrete, text wrapping fixed, restart prompt)
    fn render_graphics_tab(&mut self, ui: &mut Ui, accent: Color32) {
        let title = if self.language.is_tr() { "DONANIM GRAFİK (MUX) GEÇİŞİ" } else { "HARDWARE GRAPHICS MUX SWITCH" };
        ui.label(egui::RichText::new(title).font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_HI).strong());
        ui.add_space(4.0);

        let is_discrete = self.status.config.mux_mode == MUX_MODE_DISCRETE;

        // Card 1: Hybrid Mode (Optimus / PRIME)
        egui::Frame::none()
            .fill(if !is_discrete { PILL_BG } else { CARD_BG })
            .stroke(Stroke::new(1.0, if !is_discrete { COLOR_BALANCED } else { LINE_BORDER }))
            .rounding(Rounding::same(8.0))
            .inner_margin(14.0)
            .show(ui, |ui| {
                let avail_w = ui.available_width();
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(avail_w - 130.0);
                        ui.label(egui::RichText::new("Hybrid Mode (Optimus / PRIME)").font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_PRIMARY).strong());
                        ui.add_space(2.0);
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        let hybrid_desc = if self.language.is_tr() {
                            "AMD Radeon iGPU dahili ekranı yönetir; NVIDIA RTX 4050 dGPU ağır iş yüklerinde PRIME Render Offload ile otomatik devreye girer. Maksimum pil ömrü ve serin çalışma sunar."
                        } else {
                            "AMD Radeon iGPU manages the internal display; NVIDIA RTX 4050 dGPU activates seamlessly via PRIME Render Offload during intense workloads. Maximizes battery runtime and cool operation."
                        };
                        ui.label(
                            egui::RichText::new(hybrid_desc)
                                .font(FontId::new(11.0, FontFamily::Proportional))
                                .color(TEXT_MUTED),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let btn_label = if !is_discrete {
                            if self.language.is_tr() { "● Aktif" } else { "● Active" }
                        } else {
                            if self.language.is_tr() { "Hybrid Seç" } else { "Select Hybrid" }
                        };
                        let btn = ui.add(
                            egui::Button::new(egui::RichText::new(btn_label).font(FontId::new(12.0, FontFamily::Proportional)).color(if !is_discrete { COLOR_BALANCED } else { TEXT_PRIMARY }))
                                .fill(if !is_discrete { PILL_ACTIVE } else { PILL_BG })
                                .stroke(Stroke::new(1.0, if !is_discrete { COLOR_BALANCED } else { LINE_BORDER }))
                                .rounding(Rounding::same(4.0))
                                .min_size(Vec2::new(105.0, 32.0)),
                        );
                        if btn.clicked() && is_discrete {
                            self.status.config.mux_mode = MUX_MODE_HYBRID;
                            Self::send_ipc(CMD_SET_MUX_MODE, &[MUX_MODE_HYBRID]);
                        }
                    });
                });
            });

        ui.add_space(8.0);

        // Card 2: Discrete Mode (Direct dGPU)
        egui::Frame::none()
            .fill(if is_discrete { PILL_BG } else { CARD_BG })
            .stroke(Stroke::new(1.0, if is_discrete { COLOR_PERF } else { LINE_BORDER }))
            .rounding(Rounding::same(8.0))
            .inner_margin(14.0)
            .show(ui, |ui| {
                let avail_w = ui.available_width();
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(avail_w - 130.0);
                        ui.label(egui::RichText::new("Discrete Mode (Direct dGPU)").font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_PRIMARY).strong());
                        ui.add_space(2.0);
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        let discrete_desc = if self.language.is_tr() {
                            "Dahili ekran doğrudan NVIDIA GeForce RTX 4050 dGPU'ya bağlanır. Optimus köprüsü devre dışı kalarak en yüksek FPS, en düşük giriş gecikmesi ve tam G-Sync desteği sağlanır."
                        } else {
                            "Internal display connects directly to NVIDIA GeForce RTX 4050 dGPU. Completely bypasses the Optimus bridge for maximum gaming FPS, lowest latency, and full G-Sync support."
                        };
                        ui.label(
                            egui::RichText::new(discrete_desc)
                                .font(FontId::new(11.0, FontFamily::Proportional))
                                .color(TEXT_MUTED),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let btn_label = if is_discrete {
                            if self.language.is_tr() { "● Aktif" } else { "● Active" }
                        } else {
                            if self.language.is_tr() { "Discrete Seç" } else { "Select Discrete" }
                        };
                        let btn = ui.add(
                            egui::Button::new(egui::RichText::new(btn_label).font(FontId::new(12.0, FontFamily::Proportional)).color(if is_discrete { COLOR_PERF } else { TEXT_PRIMARY }))
                                .fill(if is_discrete { PILL_ACTIVE } else { PILL_BG })
                                .stroke(Stroke::new(1.0, if is_discrete { COLOR_PERF } else { LINE_BORDER }))
                                .rounding(Rounding::same(4.0))
                                .min_size(Vec2::new(105.0, 32.0)),
                        );
                        if btn.clicked() && !is_discrete {
                            self.status.config.mux_mode = MUX_MODE_DISCRETE;
                            Self::send_ipc(CMD_SET_MUX_MODE, &[MUX_MODE_DISCRETE]);
                        }
                    });
                });
            });

        ui.add_space(12.0);

        // Reboot prompt card
        egui::Frame::none()
            .fill(CARD_BG)
            .stroke(Stroke::new(1.0, LINE_BORDER))
            .rounding(Rounding::same(8.0))
            .inner_margin(14.0)
            .show(ui, |ui| {
                let avail_w = ui.available_width();
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(avail_w - 130.0);
                        let reboot_title = if self.language.is_tr() {
                            "Donanım MUX Geçişi ve Yeniden Başlatma"
                        } else {
                            "Hardware MUX Switch Reboot Required"
                        };
                        ui.label(egui::RichText::new(reboot_title).font(FontId::new(12.5, FontFamily::Proportional)).color(TEXT_PRIMARY));
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        let reboot_desc = if self.language.is_tr() {
                            "Donanım seviyesindeki MUX anahtarı BIOS tarafından bilgisayar açılırken uygulanır. Değişikliğin tam olarak geçerli olması için sistemi yeniden başlatmanız gerekir."
                        } else {
                            "The hardware-level MUX switch is applied by the BIOS during system boot. You must reboot your computer for the switch to take full physical effect."
                        };
                        ui.label(
                            egui::RichText::new(reboot_desc)
                                .font(FontId::new(11.0, FontFamily::Proportional))
                                .color(TEXT_MUTED),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let restart_label = if self.language.is_tr() { "🔄 Şimdi Yeniden Başlat" } else { "🔄 Restart Now" };
                        let restart_btn = ui.add(
                            egui::Button::new(egui::RichText::new(restart_label).font(FontId::new(12.0, FontFamily::Proportional)).color(COLOR_WARN))
                                .fill(PILL_BG)
                                .stroke(Stroke::new(1.0, COLOR_WARN))
                                .rounding(Rounding::same(4.0))
                                .min_size(Vec2::new(105.0, 32.0)),
                        );
                        if restart_btn.clicked() {
                            let _ = Command::new("systemctl").arg("reboot").spawn();
                        }
                    });
                });
            });
    }

    /// Tab 3: System Hardware Specifications (Loaded once on demand)
    fn render_system_info_tab(&mut self, ui: &mut Ui, accent: Color32) {
        if self.specs.is_none() {
            self.specs = Some(SystemSpecs::collect());
        }
        let specs = self.specs.as_ref().unwrap();

        let sys_title = if self.language.is_tr() { "SİSTEM ÖZELLİKLERİ" } else { "SYSTEM SPECIFICATIONS" };
        ui.label(egui::RichText::new(sys_title).font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_HI).strong());
        ui.add_space(6.0);

        // Hardware Specs Card
        egui::Frame::none()
            .fill(CARD_BG)
            .stroke(Stroke::new(1.0, LINE_BORDER))
            .rounding(Rounding::same(8.0))
            .inner_margin(14.0)
            .show(ui, |ui| {
                // Active Display Renderer badge
                let ren_title = if self.language.is_tr() { "AKTİF EKRAN İŞLEMCİSİ" } else { "ACTIVE DISPLAY RENDERER" };
                ui.label(egui::RichText::new(ren_title).font(FontId::new(10.5, FontFamily::Proportional)).color(TEXT_DIM));
                ui.add_space(2.0);
                egui::Frame::none()
                    .fill(PILL_BG)
                    .stroke(Stroke::new(1.0, accent))
                    .rounding(Rounding::same(4.0))
                    .inner_margin(Vec2::new(10.0, 6.0))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(&specs.active_renderer).font(FontId::new(12.5, FontFamily::Proportional)).color(TEXT_HI).strong());
                    });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // CPU
                let cpu_label = if self.language.is_tr() { "İşlemci (CPU)" } else { "Processor (CPU)" };
                render_spec_row(ui, cpu_label, &specs.cpu_model);
                ui.add_space(6.0);

                // Dedicated GPU
                let dgpu_label = if self.language.is_tr() { "Harici Ekran Kartı (dGPU)" } else { "Discrete GPU (dGPU)" };
                render_spec_row(ui, dgpu_label, &specs.gpu_dgpu);
                ui.add_space(6.0);

                // Integrated GPU
                let igpu_label = if self.language.is_tr() { "Dahili Grafik Kartı (iGPU)" } else { "Integrated GPU (iGPU)" };
                render_spec_row(ui, igpu_label, &specs.gpu_igpu);
                ui.add_space(6.0);

                // RAM
                let ram_label = if self.language.is_tr() { "Sistem Belleği (RAM)" } else { "System Memory (RAM)" };
                let ram_val = if self.language.is_tr() {
                    format!("{} GB Fiziksel Bellek", specs.total_ram_gb)
                } else {
                    format!("{} GB Physical Memory", specs.total_ram_gb)
                };
                render_spec_row(ui, ram_label, &ram_val);
                ui.add_space(6.0);

                // Motherboard
                let mb_label = if self.language.is_tr() { "Anakart Modeli" } else { "Motherboard Model" };
                render_spec_row(ui, mb_label, "HP Victus 16-s0xxx (Board ID: 8BD4)");
                ui.add_space(6.0);

                // Kernel & Drivers
                let kernel_label = if self.language.is_tr() { "Linux Çekirdeği" } else { "Operating Kernel" };
                render_spec_row(ui, kernel_label, &specs.kernel_version);
                ui.add_space(6.0);

                let drv_label = if self.language.is_tr() { "NVIDIA Çekirdek Sürücüsü" } else { "NVIDIA Kernel Driver" };
                render_spec_row(ui, drv_label, &specs.nvidia_driver);
            });
    }

    /// Tab 4: Settings (OMEN Key Learn default 149, Autostart, Probe Interval, Reset Defaults)
    fn render_settings_tab(&mut self, ui: &mut Ui, accent: Color32) {
        let tab_title = if self.language.is_tr() { "AYARLAR VE OMEN TUŞU" } else { "SETTINGS & OMEN KEY" };
        ui.label(egui::RichText::new(tab_title).font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_HI).strong());
        ui.add_space(6.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // 0. APPLICATION LANGUAGE
                egui::Frame::none()
                    .fill(CARD_BG)
                    .stroke(Stroke::new(1.0, LINE_BORDER))
                    .rounding(Rounding::same(8.0))
                    .inner_margin(14.0)
                    .show(ui, |ui| {
                        let lang_sec_hdr = if self.language.is_tr() { "UYGULAMA DİLİ" } else { "APPLICATION LANGUAGE" };
                        ui.label(egui::RichText::new(lang_sec_hdr).font(FontId::new(10.5, FontFamily::Proportional)).color(TEXT_DIM));
                        ui.add_space(4.0);

                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                let lang_title = if self.language.is_tr() { "Arayüz Dili" } else { "Interface Language" };
                                let lang_desc = if self.language.is_tr() { "Uygulama dilini Türkçe veya İngilizce olarak seçin." } else { "Choose Turkish or English interface language." };
                                ui.label(egui::RichText::new(lang_title).font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_PRIMARY));
                                ui.label(egui::RichText::new(lang_desc).font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_MUTED));
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                // English button
                                let is_en = !self.language.is_tr();
                                let en_btn = ui.add(
                                    egui::Button::new(
                                        egui::RichText::new("🇬🇧 English")
                                            .font(FontId::new(12.0, FontFamily::Proportional))
                                            .color(if is_en { accent } else { TEXT_PRIMARY }),
                                    )
                                    .fill(if is_en { PILL_ACTIVE } else { PILL_BG })
                                    .stroke(Stroke::new(1.0, if is_en { accent } else { LINE_BORDER }))
                                    .rounding(Rounding::same(4.0))
                                    .min_size(Vec2::new(95.0, 30.0)),
                                );
                                if en_btn.clicked() && is_en == false {
                                    self.language = Language::En;
                                    self.language.save();
                                }

                                ui.add_space(6.0);

                                // Turkish button
                                let is_tr = self.language.is_tr();
                                let tr_btn = ui.add(
                                    egui::Button::new(
                                        egui::RichText::new("🇹🇷 Türkçe")
                                            .font(FontId::new(12.0, FontFamily::Proportional))
                                            .color(if is_tr { accent } else { TEXT_PRIMARY }),
                                    )
                                    .fill(if is_tr { PILL_ACTIVE } else { PILL_BG })
                                    .stroke(Stroke::new(1.0, if is_tr { accent } else { LINE_BORDER }))
                                    .rounding(Rounding::same(4.0))
                                    .min_size(Vec2::new(95.0, 30.0)),
                                );
                                if tr_btn.clicked() && is_tr == false {
                                    self.language = Language::Tr;
                                    self.language.save();
                                }
                            });
                        });
                    });

                ui.add_space(8.0);

                // 1. OMEN KEY LEARNING
                egui::Frame::none()
                    .fill(CARD_BG)
                    .stroke(Stroke::new(1.0, LINE_BORDER))
                    .rounding(Rounding::same(8.0))
                    .inner_margin(14.0)
                    .show(ui, |ui| {
                        let omen_hdr = if self.language.is_tr() { "OMEN TUŞU TANIMLAMA" } else { "OMEN KEY LEARNING" };
                        ui.label(egui::RichText::new(omen_hdr).font(FontId::new(10.5, FontFamily::Proportional)).color(TEXT_DIM));
                        ui.add_space(4.0);

                        let is_learning = self.status.is_learning_key == 1;

                        ui.horizontal(|ui| {
                            let dot_color = if is_learning { COLOR_WARN } else { COLOR_OK };
                            let (dot_rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), Sense::hover());
                            ui.painter().circle_filled(dot_rect.center(), 4.0, dot_color);

                            if is_learning {
                                let elapsed = self.learn_started_at.map_or(0, |t| t.elapsed().as_secs());
                                let remain = 10u64.saturating_sub(elapsed);
                                let prompt_str = if self.language.is_tr() {
                                    format!("OMEN tuşuna şimdi basın… ({} sn)", remain)
                                } else {
                                    format!("Press your OMEN key now… ({} s)", remain)
                                };
                                ui.label(egui::RichText::new(prompt_str).font(FontId::new(13.0, FontFamily::Proportional)).color(COLOR_WARN).strong());

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let cancel_lbl = if self.language.is_tr() { "İptal" } else { "Cancel" };
                                    if ui.button(cancel_lbl).clicked() {
                                        Self::send_ipc(CMD_CANCEL_KEY_LEARN, &[]);
                                        self.status.is_learning_key = 0;
                                        self.learn_started_at = None;
                                    }
                                });
                            } else {
                                let key_code = self.status.config.hotkey_code;
                                let label_text = if self.language.is_tr() {
                                    if key_code != 0 {
                                        format!("OMEN Tuşu: Tuş Kodu {} ({})", key_code, Self::keycode_display_name(key_code))
                                    } else {
                                        "OMEN Tuşu: Tuş Kodu 149 (Varsayılan)".to_string()
                                    }
                                } else {
                                    if key_code != 0 {
                                        format!("OMEN Key: Keycode {} ({})", key_code, Self::keycode_display_name(key_code))
                                    } else {
                                        "OMEN Key: Keycode 149 (Default)".to_string()
                                    }
                                };

                                ui.label(egui::RichText::new(label_text).font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_PRIMARY));

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let btn_label = if self.language.is_tr() {
                                        if key_code != 0 { "Yeniden Tanıt" } else { "Tanıt" }
                                    } else {
                                        if key_code != 0 { "Relearn" } else { "Learn" }
                                    };
                                    let learn_btn = ui.add(
                                        egui::Button::new(egui::RichText::new(btn_label).font(FontId::new(12.0, FontFamily::Proportional)).color(accent))
                                            .fill(PILL_BG)
                                            .stroke(Stroke::new(1.0, LINE_BORDER))
                                            .rounding(Rounding::same(4.0)),
                                    );

                                    if learn_btn.clicked() {
                                        Self::send_ipc(CMD_START_KEY_LEARN, &[]);
                                        self.status.is_learning_key = 1;
                                        self.learn_started_at = Some(Instant::now());
                                    }
                                });
                            }
                        });

                        ui.add_space(4.0);
                        let omen_sub = if self.language.is_tr() {
                            "Fiziksel OMEN tuşunu Wayland üzerinde pencereyi açıp kapatmak üzere eşler (Varsayılan: 149)."
                        } else {
                            "Binds the physical OMEN hotkey to summon or hide the window on Wayland (Default: 149)."
                        };
                        ui.label(egui::RichText::new(omen_sub).font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_MUTED));
                    });

                ui.add_space(8.0);

                // 2. SYSTEM & SENSORS
                egui::Frame::none()
                    .fill(CARD_BG)
                    .stroke(Stroke::new(1.0, LINE_BORDER))
                    .rounding(Rounding::same(8.0))
                    .inner_margin(14.0)
                    .show(ui, |ui| {
                        let auto_hdr = if self.language.is_tr() { "OTOMASYON VE SENSÖRLER" } else { "AUTOMATION & SENSORS" };
                        ui.label(egui::RichText::new(auto_hdr).font(FontId::new(10.5, FontFamily::Proportional)).color(TEXT_DIM));
                        ui.add_space(4.0);

                        // Autostart
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                let autostart_title = if self.language.is_tr() { "Açılışta Başlat (LightOmen Sistem Tepsisi)" } else { "Start at Login (LightOmen Tray)" };
                                let autostart_desc = if self.language.is_tr() { "Kullanıcı oturumu açıldığında lightomen-tray uygulamasını otomatik başlatır." } else { "Launch lightomen-tray automatically on user login." };
                                ui.label(egui::RichText::new(autostart_title).font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_PRIMARY));
                                ui.label(egui::RichText::new(autostart_desc).font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_MUTED));
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let mut auto = self.status.config.autostart == 1;
                                if ui.checkbox(&mut auto, "").changed() {
                                    self.status.config.autostart = if auto { 1 } else { 0 };
                                    Self::toggle_autostart(auto);
                                    Self::send_ipc(CMD_SET_AUTOSTART, &[self.status.config.autostart]);
                                }
                            });
                        });

                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);

                        // Probe interval
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                let probe_title = if self.language.is_tr() { "Donanım Algılama Aralığı" } else { "Hardware Probe Interval" };
                                let probe_desc = if self.language.is_tr() { "Sensör güncelleme periyodu (0.5 sn - 3.0 sn)." } else { "Sensor refresh rate (0.5s to 3.0s)." };
                                ui.label(egui::RichText::new(probe_title).font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_PRIMARY));
                                ui.label(egui::RichText::new(probe_desc).font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_MUTED));
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let intervals = [(500u16, "0.5s"), (1000u16, "1.0s"), (2000u16, "2.0s"), (3000u16, "3.0s")];
                                for (val, label) in intervals {
                                    let selected = self.status.config.probe_interval_ms == val;
                                    let btn = ui.selectable_label(selected, label);
                                    if btn.clicked() {
                                        self.status.config.probe_interval_ms = val;
                                        let data = val.to_le_bytes();
                                        Self::send_ipc(CMD_SET_PROBE_INTERVAL, &data);
                                    }
                                }
                            });
                        });

                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);

                        // Zero-RPM GPU Fan on Battery
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                let zero_title = if self.language.is_tr() { "Bataryada Sıfır RPM GPU Fanı" } else { "Zero-RPM GPU Fan on Battery" };
                                let zero_desc = if self.language.is_tr() {
                                    "Bataryada iken ve NVIDIA D3cold uyku modundayken GPU fanını durdurur (0 RPM). CPU 75°C'ye ulaşırsa fanı otomatik çalıştırır."
                                } else {
                                    "Stop GPU fan (0 RPM) when running on battery and NVIDIA is in D3cold sleep. Ramps up if CPU reaches 75°C."
                                };
                                ui.label(egui::RichText::new(zero_title).font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_PRIMARY));
                                ui.label(egui::RichText::new(zero_desc).font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_MUTED));
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let mut zero_rpm = self.status.config.gpu_zero_rpm == 1;
                                if ui.checkbox(&mut zero_rpm, "").changed() {
                                    self.status.config.gpu_zero_rpm = if zero_rpm { 1 } else { 0 };
                                    Self::send_ipc(CMD_SET_GPU_ZERO_RPM, &[self.status.config.gpu_zero_rpm]);
                                }
                            });
                        });

                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);

                        // Auto Eco on Battery
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                let eco_title = if self.language.is_tr() { "Bataryada Otomatik Eco Modu" } else { "Auto Eco Mode on Battery" };
                                let eco_desc = if self.language.is_tr() {
                                    "Şarjdan çıkarıldığında otomatik Eco profiline geçer, şarja takıldığında önceki profile döner."
                                } else {
                                    "Automatically switch to Eco profile when unplugged, and restore previous profile on AC."
                                };
                                ui.label(egui::RichText::new(eco_title).font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_PRIMARY));
                                ui.label(egui::RichText::new(eco_desc).font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_MUTED));
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let mut eco = self.status.config.eco_on_battery() == 1;
                                if ui.checkbox(&mut eco, "").changed() {
                                    self.status.config.set_eco_on_battery(if eco { 1 } else { 0 });
                                    Self::send_ipc(CMD_SET_ECO_ON_BATTERY, &[if eco { 1 } else { 0 }]);
                                }
                            });
                        });

                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);

                        // Lowest Refresh Rate on Battery (60 Hz)
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                let low_title = if self.language.is_tr() { "Bataryada Düşük Yenileme Hızı (60 Hz)" } else { "Lowest Refresh Rate on Battery (60 Hz)" };
                                let low_desc = if self.language.is_tr() {
                                    "Bataryada iken ekranı 60 Hz'e düşürür, fişe takıldığında en yüksek yenileme hızına (144 Hz) geri alır."
                                } else {
                                    "Automatically switch screen to 60 Hz on battery, restore maximum refresh rate (144 Hz) on AC."
                                };
                                ui.label(egui::RichText::new(low_title).font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_PRIMARY));
                                ui.label(egui::RichText::new(low_desc).font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_MUTED));
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let mut low_hz = self.status.config.low_hz_on_battery() == 1;
                                if ui.checkbox(&mut low_hz, "").changed() {
                                    self.status.config.set_low_hz_on_battery(if low_hz { 1 } else { 0 });
                                    Self::send_ipc(CMD_SET_LOW_HZ_ON_BATTERY, &[if low_hz { 1 } else { 0 }]);
                                    let is_bat = self.status.on_battery == 1;
                                    let target_hz = if low_hz && is_bat { 60 } else { 144 };
                                    let _ = Command::new("kscreen-doctor")
                                        .arg(format!("output.1.mode.1920x1080@{}", target_hz))
                                        .spawn();
                                }
                            });
                        });

                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);

                        // 1. Disable CPU Turbo Boost (AC & Battery)
                        let disable_boost_global = !self.status.config.cpu_boost_enabled();
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                let boost_title = if self.language.is_tr() { "CPU Turbo Boost'u Devre Dışı Bırak" } else { "Disable CPU Turbo Boost" };
                                let boost_desc = if self.language.is_tr() {
                                    "Hem şarjda hem bataryada 5.0 GHz turbo artışını kapatır. İşlemci saatini temel frekansta (~4.3 GHz) sınırlandırarak voltaj sıçramalarını ve sıcaklıkları 10-15°C düşürür (Windows %95 işlemci durumu gibi)."
                                } else {
                                    "Disable dynamic boost to 5.0 GHz across AC & Battery. Limits CPU clock to base frequency (~4.3 GHz), eliminating aggressive voltage spikes and dropping thermals by 10-15°C (Windows 95% processor state)."
                                };
                                ui.label(egui::RichText::new(boost_title).font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_PRIMARY));
                                ui.label(egui::RichText::new(boost_desc).font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_MUTED));
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let mut disable_val = disable_boost_global;
                                if ui.checkbox(&mut disable_val, "").changed() {
                                    let enable_boost = !disable_val;
                                    self.status.config.set_cpu_boost_enabled(enable_boost);
                                    Self::send_ipc(CMD_SET_CPU_BOOST, &[if enable_boost { 1 } else { 0 }]);
                                }
                            });
                        });

                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);

                        // 2. Disable Turbo Boost on Battery (Battery Only)
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                let title_color = if !disable_boost_global { TEXT_PRIMARY } else { TEXT_DIM };
                                let sub_color = if !disable_boost_global { TEXT_MUTED } else { TEXT_DIM };
                                let no_boost_title = if self.language.is_tr() { "Bataryada Turbo Boost'u Kapat" } else { "Disable Turbo Boost on Battery" };
                                ui.label(egui::RichText::new(no_boost_title).font(FontId::new(13.0, FontFamily::Proportional)).color(title_color));
                                let desc = if !disable_boost_global {
                                    if self.language.is_tr() {
                                        "Bataryada çalışırken ani güç tüketimini önlemek ve pil ömrünü uzatmak için CPU Turbo Boost'u otomatik kapatır."
                                    } else {
                                        "Automatically disable CPU Turbo Boost when running on battery to prevent power spikes and maximize battery life."
                                    }
                                } else {
                                    if self.language.is_tr() {
                                        "Devre dışı: CPU Turbo Boost zaten tüm sistem genelinde kapatıldı."
                                    } else {
                                        "Inactive: CPU Turbo Boost is already turned off globally across both AC and battery."
                                    }
                                };
                                ui.label(egui::RichText::new(desc).font(FontId::new(11.0, FontFamily::Proportional)).color(sub_color));
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_enabled_ui(!disable_boost_global, |ui| {
                                    let mut no_boost_bat = self.status.config.no_boost_on_battery();
                                    if ui.checkbox(&mut no_boost_bat, "").changed() {
                                        self.status.config.set_no_boost_on_battery(no_boost_bat);
                                        Self::send_ipc(CMD_SET_NO_BOOST_ON_BATTERY, &[if no_boost_bat { 1 } else { 0 }]);
                                    }
                                });
                            });
                        });
                    });

                ui.add_space(8.0);

                // Battery charge limit (only when the kernel exposes charge_control_end_threshold)
                if self.status.charge_limit_supported != 0 {
                    self.draw_charge_limit_card(ui);
                    ui.add_space(8.0);
                }

                // 3. FACTORY RESET
                egui::Frame::none()
                    .fill(CARD_BG)
                    .stroke(Stroke::new(1.0, LINE_BORDER))
                    .rounding(Rounding::same(8.0))
                    .inner_margin(14.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let reset_title = if self.language.is_tr() { "Fan Grafikleri ve Ayarları Sıfırla" } else { "Reset Fan Curves & Settings" };
                            ui.label(egui::RichText::new(reset_title).font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_PRIMARY));

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let reset_btn_lbl = if self.language.is_tr() { "Varsayılana Sıfırla" } else { "Reset Defaults" };
                                if ui.button(reset_btn_lbl).clicked() {
                                    self.status.config.cpu_curve = DEFAULT_CPU_CURVE;
                                    self.status.config.gpu_curve = DEFAULT_GPU_CURVE;
                                    self.status.config.probe_interval_ms = 2000;
                                    self.status.config.gpu_zero_rpm = 1;
                                    self.status.config.set_eco_on_battery(1);
                                    self.status.config.set_low_hz_on_battery(1);
                                    self.status.config.set_cpu_boost_enabled(true);
                                    self.status.config.set_no_boost_on_battery(true);
                                    self.status.config.hotkey_code = 149;
                                    let mut data = [0u8; 14];
                                    data[0..7].copy_from_slice(&DEFAULT_CPU_CURVE);
                                    data[7..14].copy_from_slice(&DEFAULT_GPU_CURVE);
                                    Self::send_ipc(CMD_SET_FAN_CURVES, &data);
                                    Self::send_ipc(CMD_SET_PROBE_INTERVAL, &2000u16.to_le_bytes());
                                    Self::send_ipc(CMD_SET_GPU_ZERO_RPM, &[1]);
                                    Self::send_ipc(CMD_SET_ECO_ON_BATTERY, &[1]);
                                    Self::send_ipc(CMD_SET_LOW_HZ_ON_BATTERY, &[1]);
                                    Self::send_ipc(CMD_SET_CPU_BOOST, &[1]);
                                    Self::send_ipc(CMD_SET_NO_BOOST_ON_BATTERY, &[1]);
                                }
                            });
                        });
                    });

                ui.add_space(8.0);

                // 4. HARDWARE & DRIVER DIAGNOSTICS (OmenSpace / Diagnostic Report)
                egui::Frame::none()
                    .fill(CARD_BG)
                    .stroke(Stroke::new(1.0, LINE_BORDER))
                    .rounding(Rounding::same(8.0))
                    .inner_margin(14.0)
                    .show(ui, |ui| {
                        let diag_sec_hdr = if self.language.is_tr() { "SİSTEM VE DONANIM TANILAMA" } else { "SYSTEM & HARDWARE DIAGNOSTICS" };
                        ui.label(egui::RichText::new(diag_sec_hdr).font(FontId::new(10.5, FontFamily::Proportional)).color(TEXT_DIM));
                        ui.add_space(4.0);

                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                let diag_title = if self.language.is_tr() { "Tanılama Raporu Topla" } else { "Collect Diagnostics Report" };
                                let diag_desc = if self.language.is_tr() {
                                    "hp-wmi çekirdek sürücüsü, sysfs düğümleri, EC sensörleri, MUX durumu ve daemon günlüklerini tek tıkla inceleyin."
                                } else {
                                    "Inspect hp-wmi kernel driver, sysfs nodes, EC sensors, MUX status, and daemon logs in one click."
                                };
                                ui.label(egui::RichText::new(diag_title).font(FontId::new(13.0, FontFamily::Proportional)).color(TEXT_PRIMARY));
                                ui.label(egui::RichText::new(diag_desc).font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_MUTED));
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let run_diag_lbl = if self.language.is_tr() { "📋 Tanılamayı Başlat" } else { "📋 Run Diagnostics" };
                                if ui.button(run_diag_lbl).clicked() {
                                    self.diagnostic_report = Some(Self::collect_diagnostics());
                                    let success_msg = if self.language.is_tr() {
                                        "Tanılama raporu başarıyla oluşturuldu!".to_string()
                                    } else {
                                        "Diagnostics collected successfully!".to_string()
                                    };
                                    self.diagnostic_status = Some(success_msg);
                                }
                            });
                        });

                        if let Some(report) = self.diagnostic_report.clone() {
                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(6.0);

                            let mut close_clicked = false;
                            let mut copy_clicked = false;
                            let mut save_clicked = false;

                            ui.horizontal(|ui| {
                                if let Some(ref st) = self.diagnostic_status {
                                    ui.label(egui::RichText::new(st).font(FontId::new(11.0, FontFamily::Proportional)).color(COLOR_OK));
                                }

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let close_lbl = if self.language.is_tr() { "✕ Kapat" } else { "✕ Close" };
                                    if ui.button(close_lbl).clicked() {
                                        close_clicked = true;
                                    }
                                    let save_lbl = if self.language.is_tr() { "💾 Dosyaya Kaydet" } else { "💾 Save to File" };
                                    if ui.button(save_lbl).clicked() {
                                        save_clicked = true;
                                    }
                                    let copy_lbl = if self.language.is_tr() { "📋 Panoya Kopyala" } else { "📋 Copy to Clipboard" };
                                    if ui.button(copy_lbl).clicked() {
                                        copy_clicked = true;
                                    }
                                });
                            });

                            if close_clicked {
                                self.diagnostic_report = None;
                                self.diagnostic_status = None;
                            } else if copy_clicked {
                                ui.output_mut(|o| o.copied_text = report.clone());
                                let copied_msg = if self.language.is_tr() {
                                    "Panoya kopyalandı!".to_string()
                                } else {
                                    "Copied to clipboard!".to_string()
                                };
                                self.diagnostic_status = Some(copied_msg);
                            } else if save_clicked {
                                if let Some(home) = std::env::var_os("HOME") {
                                    let p = PathBuf::from(home).join("lightomen-diagnostics.txt");
                                    if fs::write(&p, &report).is_ok() {
                                        let saved_msg = if self.language.is_tr() {
                                            format!("{} konumuna kaydedildi", p.display())
                                        } else {
                                            format!("Saved to {}", p.display())
                                        };
                                        self.diagnostic_status = Some(saved_msg);
                                    }
                                }
                            }

                            if self.diagnostic_report.is_some() {
                                ui.add_space(6.0);
                                egui::ScrollArea::vertical()
                                    .max_height(220.0)
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        let mut text_slice = report.as_str();
                                        ui.add(
                                            egui::TextEdit::multiline(&mut text_slice)
                                                .font(FontId::new(10.5, FontFamily::Monospace))
                                                .text_color(TEXT_HI)
                                                .desired_rows(12)
                                                .desired_width(f32::INFINITY)
                                                .lock_focus(true)
                                        );
                                    });
                            }
                        }
                    });
            });
    }
}

fn render_rail_tab_icon(ui: &mut Ui, icon: &str, active: bool, accent: Color32, tooltip: &str) -> egui::Response {
    let size = Vec2::new(42.0, 42.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());

    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, Rounding::same(8.0), PILL_BG);
        painter.rect_stroke(rect, Rounding::same(8.0), Stroke::new(1.0, accent));
    } else if response.hovered() {
        painter.rect_filled(rect, Rounding::same(8.0), PILL_ACTIVE);
    }

    let color = if active { accent } else { TEXT_MUTED };
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        FontId::new(18.0, FontFamily::Proportional),
        color,
    );

    response.on_hover_text(tooltip)
}

fn render_spec_row(ui: &mut Ui, title: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).font(FontId::new(11.5, FontFamily::Proportional)).color(TEXT_MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(value).font(FontId::new(11.5, FontFamily::Monospace)).color(TEXT_PRIMARY));
        });
    });
}

pub fn fan_speed_at(curve: &[u8; 7], temp: u16) -> u8 {
    if temp <= CURVE_TEMPS[0] as u16 {
        return curve[0];
    }
    if temp >= CURVE_TEMPS[6] as u16 {
        return curve[6];
    }
    for i in 0..6 {
        let t0 = CURVE_TEMPS[i] as u16;
        let t1 = CURVE_TEMPS[i + 1] as u16;
        if temp >= t0 && temp <= t1 {
            let p0 = curve[i] as f32;
            let p1 = curve[i + 1] as f32;
            let f = (temp - t0) as f32 / (t1 - t0) as f32;
            if p0 == 0.0 && p1 > 0.0 && f < 0.5 {
                return 0;
            }
            return (p0 + f * (p1 - p0)).round() as u8;
        }
    }
    curve[6]
}

fn main() -> eframe::Result<()> {
    // Force Mesa/integrated AMD graphics: never probe or wake NVIDIA dGPU
    std::env::remove_var("DRI_PRIME");
    std::env::set_var("__NV_PRIME_RENDER_OFFLOAD", "0");
    std::env::set_var("__GLX_VENDOR_LIBRARY_NAME", "mesa");

    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    let gui_sock = format!("{}/lightomen-gui.sock", runtime_dir);

    // Single-instance check: If already running, notify it to focus and bring to front, then exit
    if let Ok(mut stream) = UnixStream::connect(&gui_sock) {
        let _ = stream.write_all(b"FOCUS\n");
        let _ = stream.flush();
        return Ok(());
    }

    let _ = fs::remove_file(&gui_sock);
    let needs_focus = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ctx_holder: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));

    if let Ok(listener) = std::os::unix::net::UnixListener::bind(&gui_sock) {
        let nf = needs_focus.clone();
        let holder = ctx_holder.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if let Ok(mut s) = stream {
                    let mut buf = [0u8; 16];
                    if let Ok(n) = s.read(&mut buf) {
                        if &buf[..n] == b"FOCUS\n" || &buf[..n] == b"FOCUS" {
                            nf.store(true, std::sync::atomic::Ordering::SeqCst);
                            if let Ok(guard) = holder.lock() {
                                if let Some(ref c) = *guard {
                                    c.request_repaint();
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    let icon = egui::IconData {
        rgba: icon_data::ICON_32_RGBA.to_vec(),
        width: 32,
        height: 32,
    };

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([540.0, 680.0])
            .with_resizable(false)
            .with_decorations(false)
            .with_transparent(false)
            .with_app_id("lightomen")
            .with_icon(std::sync::Arc::new(icon)),
        ..Default::default()
    };

    let nf_app = needs_focus.clone();
    let holder_app = ctx_holder.clone();
    let res = eframe::run_native(
        "LightOmen",
        native_options,
        Box::new(move |_cc| Ok(Box::new(LightOmenApp::new(nf_app, holder_app)))),
    );

    let _ = fs::remove_file(&gui_sock);
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fan_curve_interpolation() {
        let curve = [25, 30, 40, 52, 65, 80, 100];
        assert_eq!(fan_speed_at(&curve, 20), 25);
        assert_eq!(fan_speed_at(&curve, 30), 25);
        assert_eq!(fan_speed_at(&curve, 35), 28);
        assert_eq!(fan_speed_at(&curve, 60), 52);
        assert_eq!(fan_speed_at(&curve, 95), 100);

        let zero_curve = [0, 20, 30, 40, 50, 60, 100];
        assert_eq!(fan_speed_at(&zero_curve, 30), 0);
        assert_eq!(fan_speed_at(&zero_curve, 32), 0);
        assert_eq!(fan_speed_at(&zero_curve, 34), 0);
        assert_eq!(fan_speed_at(&zero_curve, 35), 10);
    }

    #[test]
    fn test_keycode_names() {
        assert_eq!(LightOmenApp::keycode_display_name(149), "KEY_PROG2 / OMEN Key (149)");
        assert_eq!(LightOmenApp::keycode_display_name(148), "KEY_PROG1 / OMEN Key (148)");
        assert_eq!(LightOmenApp::keycode_display_name(200), "XF86Launch2");
    }
}
