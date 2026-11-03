// main.rs: LightOmen command line client
// Talks to lightomen-daemon over the existing binary IPC socket.

use lightomen_common::{
    DaemonStatus, CURVE_TEMPS, FAN_MODE_AUTO, FAN_MODE_CURVE, FAN_MODE_MAX, POWER_PROFILE_BALANCED,
    POWER_PROFILE_ECO, POWER_PROFILE_PERFORMANCE,
};
use std::fmt::Write as _;
use std::process::ExitCode;

const USAGE: &str = "\
Usage: lightomen-cli <command>

Commands:
  status                                Show temperatures, fan speeds and profile
  profile <eco|balanced|performance>    Set the power profile
  fan curve                             Print the CPU and GPU fan curves
  help, -h, --help                      Show this help
";

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Status,
    Profile(u8),
    FanCurve,
    Help,
}

fn parse_args<S: AsRef<str>>(args: &[S]) -> Result<Command, String> {
    let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    match args.as_slice() {
        [] | ["help"] | ["-h"] | ["--help"] => Ok(Command::Help),
        ["status"] => Ok(Command::Status),
        ["profile", name] => parse_profile(name).map(Command::Profile),
        ["profile"] => Err("missing profile name (eco, balanced or performance)".into()),
        ["fan", "curve"] => Ok(Command::FanCurve),
        ["fan", ..] => Err("usage: lightomen-cli fan curve".into()),
        [cmd, ..] => Err(format!("unknown command or arguments: {cmd}")),
    }
}

fn parse_profile(name: &str) -> Result<u8, String> {
    match name.to_ascii_lowercase().as_str() {
        "eco" => Ok(POWER_PROFILE_ECO),
        "balanced" => Ok(POWER_PROFILE_BALANCED),
        "performance" => Ok(POWER_PROFILE_PERFORMANCE),
        _ => Err(format!(
            "unknown profile \"{name}\" (expected eco, balanced or performance)"
        )),
    }
}

fn profile_name(profile: u8) -> &'static str {
    match profile {
        POWER_PROFILE_ECO => "eco",
        POWER_PROFILE_BALANCED => "balanced",
        POWER_PROFILE_PERFORMANCE => "performance",
        _ => "unknown",
    }
}

fn fan_mode_name(mode: u8) -> &'static str {
    match mode {
        FAN_MODE_AUTO => "auto",
        FAN_MODE_MAX => "max",
        FAN_MODE_CURVE => "curve",
        _ => "unknown",
    }
}

fn format_status(st: &DaemonStatus) -> String {
    let t = &st.telemetry;
    let mut out = String::new();
    let _ = writeln!(out, "CPU temp:  {} °C", t.cpu_temp);
    let _ = writeln!(out, "GPU temp:  {} °C", t.gpu_temp);
    let _ = writeln!(out, "Fan 1:     {} RPM", t.fan1_rpm);
    let _ = writeln!(out, "Fan 2:     {} RPM", t.fan2_rpm);
    let _ = writeln!(out, "Fan mode:  {}", fan_mode_name(st.config.fan_mode));
    let _ = writeln!(out, "Profile:   {}", profile_name(st.config.power_profile));
    let power = if st.on_battery != 0 { "battery" } else { "AC" };
    let _ = writeln!(out, "Power:     {} ({}%)", power, st.battery_pct);
    out
}

fn format_curves(cpu: &[u8; 7], gpu: &[u8; 7]) -> String {
    let mut out = String::from("Temp   CPU   GPU\n");
    for ((temp, c), g) in CURVE_TEMPS.iter().zip(cpu).zip(gpu) {
        let _ = writeln!(out, "{temp:>3}°C {c:>4}% {g:>4}%");
    }
    out
}

#[cfg(unix)]
mod ipc {
    use lightomen_common::{
        DaemonStatus, IpcMessage, ALT_SOCKET_PATH, CMD_GET_STATUS, CMD_SET_POWER_PROFILE,
        SOCKET_PATH,
    };
    use std::io::{self, Read, Write};
    use std::os::unix::net::UnixStream;

    fn connect() -> io::Result<UnixStream> {
        UnixStream::connect(SOCKET_PATH).or_else(|_| UnixStream::connect(ALT_SOCKET_PATH))
    }

    pub fn get_status() -> io::Result<DaemonStatus> {
        let mut stream = connect()?;
        stream.write_all(IpcMessage::new(CMD_GET_STATUS).as_bytes())?;
        let mut buf = [0u8; std::mem::size_of::<DaemonStatus>()];
        stream.read_exact(&mut buf)?;
        DaemonStatus::from_bytes(&buf)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad status reply"))
    }

    pub fn set_profile(profile: u8) -> io::Result<()> {
        let mut stream = connect()?;
        let mut msg = IpcMessage::new(CMD_SET_POWER_PROFILE);
        msg.data[0] = profile;
        msg.payload_len = 1;
        stream.write_all(msg.as_bytes())?;
        let mut reply = [0u8; 2];
        stream.read_exact(&mut reply)?;
        if &reply == b"OK" {
            Ok(())
        } else {
            Err(io::Error::other("daemon rejected request"))
        }
    }
}

#[cfg(not(unix))]
mod ipc {
    use lightomen_common::DaemonStatus;
    use std::io;

    fn unsupported() -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, "only supported on Linux")
    }

    pub fn get_status() -> io::Result<DaemonStatus> {
        Err(unsupported())
    }

    pub fn set_profile(_profile: u8) -> io::Result<()> {
        Err(unsupported())
    }
}

fn run(cmd: Command) -> Result<(), String> {
    let daemon_err = |e: std::io::Error| format!("cannot reach lightomen-daemon: {e}");
    match cmd {
        Command::Help => print!("{USAGE}"),
        Command::Status => print!("{}", format_status(&ipc::get_status().map_err(daemon_err)?)),
        Command::FanCurve => {
            let st = ipc::get_status().map_err(daemon_err)?;
            print!("{}", format_curves(&st.config.cpu_curve, &st.config.gpu_curve));
        }
        Command::Profile(p) => {
            ipc::set_profile(p).map_err(daemon_err)?;
            println!("Profile set to {}", profile_name(p));
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lightomen-cli: {e}");
            if !e.starts_with("cannot reach") {
                eprint!("\n{USAGE}");
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightomen_common::{DEFAULT_CPU_CURVE, DEFAULT_GPU_CURVE};

    #[test]
    fn parses_basic_commands() {
        assert_eq!(parse_args::<&str>(&[]), Ok(Command::Help));
        assert_eq!(parse_args(&["--help"]), Ok(Command::Help));
        assert_eq!(parse_args(&["-h"]), Ok(Command::Help));
        assert_eq!(parse_args(&["status"]), Ok(Command::Status));
        assert_eq!(parse_args(&["fan", "curve"]), Ok(Command::FanCurve));
    }

    #[test]
    fn parses_profiles() {
        assert_eq!(parse_args(&["profile", "eco"]), Ok(Command::Profile(POWER_PROFILE_ECO)));
        assert_eq!(
            parse_args(&["profile", "Balanced"]),
            Ok(Command::Profile(POWER_PROFILE_BALANCED))
        );
        assert_eq!(
            parse_args(&["profile", "performance"]),
            Ok(Command::Profile(POWER_PROFILE_PERFORMANCE))
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse_args(&["profile"]).is_err());
        assert!(parse_args(&["profile", "turbo"]).is_err());
        assert!(parse_args(&["profile", "eco", "extra"]).is_err());
        assert!(parse_args(&["fan"]).is_err());
        assert!(parse_args(&["fan", "speed"]).is_err());
        assert!(parse_args(&["status", "now"]).is_err());
        assert!(parse_args(&["reboot"]).is_err());
    }

    #[test]
    fn formats_status() {
        let mut st = DaemonStatus::default();
        st.telemetry.cpu_temp = 71;
        st.telemetry.gpu_temp = 64;
        st.telemetry.fan1_rpm = 3400;
        st.telemetry.fan2_rpm = 3550;
        st.config.power_profile = POWER_PROFILE_PERFORMANCE;
        st.config.fan_mode = FAN_MODE_CURVE;
        st.on_battery = 1;
        st.battery_pct = 80;
        let out = format_status(&st);
        assert!(out.contains("CPU temp:  71 °C"));
        assert!(out.contains("GPU temp:  64 °C"));
        assert!(out.contains("Fan 1:     3400 RPM"));
        assert!(out.contains("Fan 2:     3550 RPM"));
        assert!(out.contains("Fan mode:  curve"));
        assert!(out.contains("Profile:   performance"));
        assert!(out.contains("Power:     battery (80%)"));
    }

    #[test]
    fn formats_curves() {
        let out = format_curves(&DEFAULT_CPU_CURVE, &DEFAULT_GPU_CURVE);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 8);
        assert_eq!(lines[0], "Temp   CPU   GPU");
        assert_eq!(lines[1], " 30°C   20%   20%");
        assert_eq!(lines[7], " 90°C  100%  100%");
    }

    #[test]
    fn names_unknown_values() {
        assert_eq!(profile_name(9), "unknown");
        assert_eq!(fan_mode_name(9), "unknown");
    }
}
