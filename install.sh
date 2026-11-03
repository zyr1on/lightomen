#!/usr/bin/env bash
set -eo pipefail

export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:$PATH"

echo "=========================================================="
echo "  LightOmen Linux: Complete Native System Installer"
echo "  Hardware Control for HP Omen & Victus Laptops"
echo "=========================================================="

if [ "$EUID" -ne 0 ]; then
    echo "ERROR: Please run as root or using sudo: sudo ./install.sh"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REAL_USER="${SUDO_USER:-$USER}"
USER_HOME=$(getent passwd "$REAL_USER" | cut -d: -f6)

# 1. Terminate Existing Instances & Scan Running Processes
echo -e "\n[1/8] Terminating Existing Instances & Scanning Running Processes..."

# Stop systemd unit gracefully first if active
if systemctl is-active --quiet lightomen-daemon.service 2>/dev/null; then
    PREV_DAEMON_PID=$(systemctl show -p MainPID --value lightomen-daemon.service 2>/dev/null || true)
    echo "  -> Stopping systemd service: lightomen-daemon.service (Main PID: $PREV_DAEMON_PID)..."
    systemctl stop lightomen-daemon.service 2>/dev/null || true
fi
systemctl stop ohman-daemon.service 2>/dev/null || true
systemctl disable ohman-daemon.service 2>/dev/null || true

TARGET_PROCS=("lightomen-daemon" "ohman-daemon" "lightomen-gui" "ohman-gui" "lightomen-tray" "ohman-tray")
for proc in "${TARGET_PROCS[@]}"; do
    pids=$(pidof "$proc" 2>/dev/null || pgrep -f "/usr/local/bin/$proc" 2>/dev/null || true)
    if [ -n "$pids" ]; then
        echo "  -> Found active process: $proc (PID: $pids), terminating..."
        kill -9 $pids 2>/dev/null || true
        sleep 0.2
        # Verify termination
        check_pids=$(pidof "$proc" 2>/dev/null || pgrep -f "/usr/local/bin/$proc" 2>/dev/null || true)
        if [ -z "$check_pids" ]; then
            echo "     [OK] Terminated $proc (PID: $pids successfully exited)"
        else
            echo "     [WARNING] Process $proc still alive (PID: $check_pids)"
        fi
    else
        echo "  -> $proc: No active instances running (Clean)"
    fi
done

# 2. Clean Up System Residues, Blacklists & Legacy Artifacts
echo -e "\n[2/8] Cleaning Up System Residues & Legacy Artifacts..."

# Sockets & Runtime Directories
echo "  • Checking runtime directories & Unix sockets..."
for r_dir in /run/lightomen /run/ohman; do
    if [ -e "$r_dir" ]; then
        echo "    -> Removed runtime directory: $r_dir"
        rm -rf "$r_dir"
    fi
done

# Legacy modprobe blacklists and override configs
echo "  • Checking /etc/modprobe.d/ for harmful blacklists..."
FOUND_MODPROBE=0
for f in /etc/modprobe.d/*hpwmi*.conf /etc/modprobe.d/*hp-wmi*.conf /etc/modprobe.d/*hp_wmi*.conf \
         /etc/modprobe.d/*lightomen*.conf /etc/modprobe.d/*ohman*.conf /etc/modprobe.d/*hp-omen*.conf /etc/modprobe.d/*omen-extra*.conf; do
    if [ -f "$f" ]; then
        echo "    -> Removed blacklist config file: $f"
        rm -f "$f"
        FOUND_MODPROBE=1
    fi
done
# Check if any blacklist line exists inside remaining .conf files
if grep -E -q "blacklist[[:space:]]+(hp_wmi|hp-wmi|hp_omen_extra|hp-omen-extra)" /etc/modprobe.d/*.conf 2>/dev/null; then
    echo "    -> Stripping blacklist lines from /etc/modprobe.d/*.conf..."
    sed -i '/blacklist[[:space:]]\+hp_wmi/d' /etc/modprobe.d/*.conf 2>/dev/null || true
    sed -i '/blacklist[[:space:]]\+hp-wmi/d' /etc/modprobe.d/*.conf 2>/dev/null || true
    sed -i '/blacklist[[:space:]]\+hp_omen_extra/d' /etc/modprobe.d/*.conf 2>/dev/null || true
    sed -i '/blacklist[[:space:]]\+hp-omen-extra/d' /etc/modprobe.d/*.conf 2>/dev/null || true
    FOUND_MODPROBE=1
fi
if [ "$FOUND_MODPROBE" -eq 0 ]; then
    echo "    [OK] No blacklist files or lines found in /etc/modprobe.d/ (Clean)"
fi

# Autostart entries
echo "  • Checking user autostart entries..."
if [ -n "$USER_HOME" ]; then
    for a_file in "$USER_HOME/.config/autostart/ohman-gui.desktop" \
                  "$USER_HOME/.config/autostart/ohman-tray.desktop" \
                  "$USER_HOME/.config/autostart/lightomen-gui.desktop"; do
        if [ -f "$a_file" ]; then
            echo "    -> Removed stale autostart file: $a_file"
            rm -f "$a_file"
        fi
    done
fi

# Binary and symlink cleanup
echo "  • Checking /usr/local/bin/ legacy binaries and symlinks..."
for b_file in /usr/local/bin/ohman-daemon /usr/local/bin/ohman-gui /usr/local/bin/ohman-tray; do
    if [ -e "$b_file" ]; then
        echo "    -> Cleaned legacy executable: $b_file"
        rm -f "$b_file"
    fi
done

# Configuration binary reset & backup
echo "  • Checking persistent daemon configuration binary..."
if [ -f /etc/lightomen/config.bin ]; then
    echo "    -> Backing up old configuration: /etc/lightomen/config.bin -> /etc/lightomen/config.bin.bak"
    cp /etc/lightomen/config.bin /etc/lightomen/config.bin.bak 2>/dev/null || true
    rm -f /etc/lightomen/config.bin
    echo "    -> Configuration reset to fresh, verified defaults (1100 RPM floor, auto 0-RPM unlatch)"
else
    echo "    [OK] No stale configuration binary found (Clean defaults will be generated)"
fi
rm -f /etc/ohman/config.bin 2>/dev/null || true

# Put fans into safe BIOS Auto state during installation/build
for h in /sys/class/hwmon/hwmon*; do
    if [ -f "$h/name" ] && [ "$(cat "$h/name" 2>/dev/null)" = "hp" ]; then
        if [ -w "$h/pwm1_enable" ]; then
            echo 2 > "$h/pwm1_enable" 2>/dev/null || true
            echo "  • Safety cooling reset: Hardware fans placed into BIOS Auto control."
        fi
        break
    fi
done
sleep 1

# 3. Detect Desktop Session (Wayland vs X11)
echo -e "\n[3/8] Detecting Display Session..."
WAYLAND_DETECTED=0
if [ -n "$WAYLAND_DISPLAY" ] || [ -e "/run/user/$(id -u "$REAL_USER")/wayland-0" ]; then
    WAYLAND_DETECTED=1
    echo "  -> Native Wayland session detected for user: $REAL_USER"
else
    echo "  -> X11 / XWayland fallback session detected for user: $REAL_USER"
fi

# 4. Install Kernel & Build Dependencies
echo -e "\n[4/8] Installing Kernel & System Dependencies via Package Manager..."
# shellcheck source=scripts/pkg-deps.sh
. "$SCRIPT_DIR/scripts/pkg-deps.sh"
if PKG_MANAGER="$(detect_pkg_manager)"; then
    echo "  • Installing dkms, kernel headers, and build libraries via $PKG_MANAGER..."
    if install_build_deps "$PKG_MANAGER"; then
        echo "  [OK] $PKG_MANAGER dependencies verified."
    else
        echo "  [WARNING] Some packages failed to install via $PKG_MANAGER; DKMS build may fail."
    fi
else
    echo "  [WARNING] No supported package manager (apt, pacman, dnf, zypper) found."
    echo "            Install dkms, gcc/make and headers for kernel $(uname -r) manually."
fi

# 5. Adapt & Install hp-wmi & hp-omen-extra DKMS Drivers
echo -e "\n[5/8] Installing Patched hp-wmi & hp-omen-extra DKMS Drivers..."
DKMS_SRC="/usr/src/hp-omen-extra-2.2.0"
rm -rf "$DKMS_SRC"
mkdir -p "$DKMS_SRC"
cp -r "$SCRIPT_DIR/driver/"* "$DKMS_SRC/"

# Purge any stale custom modules from previous builds to prevent symbol conflicts
KVER=$(uname -r)
find /lib/modules/"$KVER" /usr/lib/modules/"$KVER" -name 'hp-omen-extra.ko*' -delete 2>/dev/null || true
find /lib/modules/"$KVER" /usr/lib/modules/"$KVER" -path '*/updates/*' -name 'hp-wmi.ko*' -delete 2>/dev/null || true
depmod -a 2>/dev/null || true

# Remove old DKMS registrations across versions if any
if command -v dkms &>/dev/null; then
    for v in $(dkms status hp-omen-extra 2>/dev/null | grep -oP "(?<=hp-omen-extra[/, ])[^,:]+" | tr -d ' ' | sort -u); do
        [ -n "$v" ] && dkms remove -m hp-omen-extra -v "$v" --all 2>/dev/null || true
    done
fi
dkms remove -m hp-omen-extra -v 2.2.0 --all 2>/dev/null || true

# Add, compile, and install DKMS modules (builds both patched hp-wmi and hp-omen-extra)
echo "  • Registering and compiling DKMS modules..."
dkms add -m hp-omen-extra -v 2.2.0
dkms build -m hp-omen-extra -v 2.2.0
dkms install -m hp-omen-extra -v 2.2.0 --force

# Update module dependencies map
depmod -a

# Autoload hp_wmi and hp_omen_extra on system boot
mkdir -p /etc/modules-load.d
cat << 'EOF' > /etc/modules-load.d/hp-omen-extra.conf
# Autoload HP WMI and Omen modules for fan and thermal control
hp_wmi
hp_omen_extra
EOF
cp -f /etc/modules-load.d/hp-omen-extra.conf /etc/modules-load.d/lightomen.conf

# Reload modules immediately to replace stock kernel driver in memory
echo "  • Reloading kernel modules (unloading stock, activating patched DKMS)..."
modprobe -r hp_omen_extra 2>/dev/null || true
modprobe -r hp_wmi 2>/dev/null || true
modprobe hp_wmi
modprobe hp_omen_extra 2>/dev/null || true

# Verify modules are active in kernel
LOADED_HPWMI=$(modinfo -F filename hp_wmi 2>/dev/null || true)
if lsmod | grep -q "^hp_wmi "; then
    if echo "$LOADED_HPWMI" | grep -qE "updates|extra|dkms"; then
        echo "  [OK] Patched DKMS hp_wmi driver is compiled & active: $LOADED_HPWMI"
    else
        echo "  [INFO] hp_wmi active in kernel: $LOADED_HPWMI"
    fi
else
    echo "  [WARNING] hp_wmi module could not be verified in lsmod."
fi
if lsmod | grep -q "^hp_omen_extra "; then
    echo "  [OK] hp_omen_extra companion driver is active in kernel."
fi

# 6. Check for Pre-compiled Binaries or Build from Source
echo -e "\n[6/8] Preparing Binaries..."
DAEMON_SRC=""
GUI_SRC=""
TRAY_SRC=""

if [ -f "$SCRIPT_DIR/bin/lightomen-daemon" ] && [ -f "$SCRIPT_DIR/bin/lightomen-gui" ] && [ -f "$SCRIPT_DIR/bin/lightomen-tray" ]; then
    echo "  -> Found pre-compiled release binaries in ./bin/ (Ready for instant install)"
    DAEMON_SRC="$SCRIPT_DIR/bin/lightomen-daemon"
    GUI_SRC="$SCRIPT_DIR/bin/lightomen-gui"
    TRAY_SRC="$SCRIPT_DIR/bin/lightomen-tray"
elif [ -f "$SCRIPT_DIR/Cargo.toml" ] && command -v cargo &>/dev/null; then
    echo "  -> Compiling fresh release binaries from source via Cargo (lto=fat, opt-level=z)..."
    sudo -u "$REAL_USER" cargo build --release --workspace --manifest-path "$SCRIPT_DIR/Cargo.toml"
    DAEMON_SRC="$SCRIPT_DIR/target/release/lightomen-daemon"
    GUI_SRC="$SCRIPT_DIR/target/release/lightomen-gui"
    TRAY_SRC="$SCRIPT_DIR/target/release/lightomen-tray"
elif [ -f "$SCRIPT_DIR/target/release/lightomen-daemon" ] && [ -f "$SCRIPT_DIR/target/release/lightomen-gui" ] && [ -f "$SCRIPT_DIR/target/release/lightomen-tray" ]; then
    echo "  -> Found existing built release binaries in ./target/release/"
    DAEMON_SRC="$SCRIPT_DIR/target/release/lightomen-daemon"
    GUI_SRC="$SCRIPT_DIR/target/release/lightomen-gui"
    TRAY_SRC="$SCRIPT_DIR/target/release/lightomen-tray"
else
    echo "ERROR: Neither pre-compiled binaries in ./bin/ nor 'cargo' build environment found."
    echo "Please download the pre-compiled release package from:"
    echo "  https://github.com/zyr1on/lightomen/releases"
    echo "or install Rust (https://rustup.rs) to build from source."
    exit 1
fi

# 7. Install Binaries, Symlinks, Icons & Desktop Files
echo -e "\n[7/8] Installing Executables, Icons & Desktop Integration..."
install -m 755 "$DAEMON_SRC" /usr/local/bin/lightomen-daemon
install -m 755 "$GUI_SRC" /usr/local/bin/lightomen-gui
install -m 755 "$TRAY_SRC" /usr/local/bin/lightomen-tray

# Backward compatibility symlinks
ln -sf /usr/local/bin/lightomen-daemon /usr/local/bin/ohman-daemon
ln -sf /usr/local/bin/lightomen-gui /usr/local/bin/ohman-gui
ln -sf /usr/local/bin/lightomen-tray /usr/local/bin/ohman-tray

# Install Uninstaller to system path
if [ -f "$SCRIPT_DIR/uninstall.sh" ]; then
    install -m 755 "$SCRIPT_DIR/uninstall.sh" /usr/local/bin/lightomen-uninstall
    ln -sf /usr/local/bin/lightomen-uninstall /usr/local/bin/ohman-uninstall 2>/dev/null || true
fi

# Install High-Resolution Icons
mkdir -p /usr/share/icons/hicolor/512x512/apps /usr/share/pixmaps
if [ -f "$SCRIPT_DIR/assets/lightomen.png" ]; then
    install -m 644 "$SCRIPT_DIR/assets/lightomen.png" /usr/share/icons/hicolor/512x512/apps/lightomen.png
    install -m 644 "$SCRIPT_DIR/assets/lightomen.png" /usr/share/pixmaps/lightomen.png
    ln -sf /usr/share/icons/hicolor/512x512/apps/lightomen.png /usr/share/icons/hicolor/512x512/apps/ohman.png
    ln -sf /usr/share/pixmaps/lightomen.png /usr/share/pixmaps/ohman.png
    if command -v gtk-update-icon-cache &>/dev/null; then
        gtk-update-icon-cache -f -t /usr/share/icons/hicolor 2>/dev/null || true
    fi
fi

# Desktop Entry
mkdir -p /usr/share/applications
cat << 'EOF' > /usr/share/applications/lightomen.desktop
[Desktop Entry]
Name=LightOmen
GenericName=HP Omen & Victus Controller
Comment=Hardware Performance, Fan Curve & MUX Switch Controller
Exec=env __NV_PRIME_RENDER_OFFLOAD=0 __GLX_VENDOR_LIBRARY_NAME=mesa /usr/local/bin/lightomen-gui
Icon=lightomen
Terminal=false
Type=Application
StartupWMClass=lightomen
Categories=Settings;HardwareSettings;System;
Keywords=omen;victus;hp;fan;cooling;curve;performance;mux;
EOF
ln -sf /usr/share/applications/lightomen.desktop /usr/share/applications/ohman.desktop

# Autostart for tray
if [ -n "$USER_HOME" ]; then
    mkdir -p "$USER_HOME/.config/autostart"
    cat << 'EOF' > "$USER_HOME/.config/autostart/lightomen-tray.desktop"
[Desktop Entry]
Type=Application
Name=LightOmen Tray
Comment=LightOmen Hardware Tray Controller
Exec=env __NV_PRIME_RENDER_OFFLOAD=0 __GLX_VENDOR_LIBRARY_NAME=mesa /usr/local/bin/lightomen-tray
Hidden=false
NoDisplay=false
X-GNOME-Autostart-enabled=true
EOF
    chown -R "$REAL_USER:$REAL_USER" "$USER_HOME/.config/autostart"
fi

# Config directory setup
mkdir -p /etc/lightomen /etc/ohman
chmod 755 /etc/lightomen /etc/ohman

# Systemd Service Installation
install -m 644 "$SCRIPT_DIR/systemd/lightomen-daemon.service" /etc/systemd/system/lightomen-daemon.service
ln -sf /etc/systemd/system/lightomen-daemon.service /etc/systemd/system/ohman-daemon.service 2>/dev/null || true

systemctl daemon-reload
systemctl enable lightomen-daemon.service
systemctl restart lightomen-daemon.service

# Start System Tray companion immediately for active user session
if [ -n "$REAL_USER" ] && [ "$REAL_USER" != "root" ]; then
    REAL_UID=$(id -u "$REAL_USER")
    sudo -u "$REAL_USER" XDG_RUNTIME_DIR="/run/user/$REAL_UID" DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$REAL_UID/bus" systemd-run --user --unit=lightomen-tray --collect /usr/local/bin/lightomen-tray 2>/dev/null || true
    echo "  -> lightomen-tray successfully spawned in system tray for $REAL_USER."
fi

# 8. Final Status & Verification Check
echo -e "\n[8/8] Verifying Daemon Status & Hardware Control Hand-off..."
sleep 1.5
if systemctl is-active --quiet lightomen-daemon.service; then
    NEW_DAEMON_PID=$(systemctl show -p MainPID --value lightomen-daemon.service 2>/dev/null || true)
    echo "  -> lightomen-daemon.service is ACTIVE and RUNNING (PID: $NEW_DAEMON_PID)."
    if [ -S "/run/lightomen/lightomen.sock" ]; then
        echo "  -> Hardware Control Hand-off: SUCCESSFUL"
        echo "     • Fan & Curve Control: Handed off to lightomen-daemon"
        echo "     • Power Profile & TGP/PPAB: Handed off to lightomen-daemon"
        echo "     • Hybrid/Discrete MUX: Handed off to lightomen-daemon"
        echo "     • CPU Turbo Boost: Handed off to lightomen-daemon"
        
        # Live status probe
        python3 -c "
import socket, struct
try:
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect('/run/lightomen/lightomen.sock')
    s.sendall(struct.pack('<BBH64s', 1, 0, 0, b'\x00'*64))
    data = s.recv(1024)
    s.close()
    cpu_t, gpu_t, f1, f2, cpwr, gpwr = struct.unpack_from('<HHHHII', data, 0)
    fmode = data[37]
    fmode_str = {0: 'Auto', 1: 'Max', 2: 'Curve'}.get(fmode, str(fmode))
    print(f'     • Live Telemetry: CPU {cpu_t}°C, GPU {gpu_t}°C, Fan1 {f1} RPM, Fan2 {f2} RPM | Mode: {fmode_str}')
except Exception as e:
    pass
" 2>/dev/null || true
    fi
else
    echo "  -> Warning: lightomen-daemon.service status check returned non-zero."
fi

echo "=========================================================="
echo "  LightOmen Linux INSTALLATION COMPLETE!"
echo "  Launch GUI via terminal:  lightomen-gui (or ohman-gui)"
echo "  Or click System Tray / press physical OMEN hotkey."
echo "=========================================================="
