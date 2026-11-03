# LightOmen

**Ultra-Lightweight Hardware Control Center for HP OMEN & Victus Laptops on Linux**

Pure, idiomatic Rust with zero runtime bloat — no Tokio, no Serde, pure standard library, `libc`, and `evdev`. Consumes under 1 MB RAM in the background.

<img width="540" height="680" alt="image" src="https://github.com/user-attachments/assets/59eec412-7ffb-4c52-948a-4aca34d4c621" />


---

## ⚠️ Important Prerequisites (EC & ACPI Setup)

The installer pulls in `dkms`, a C toolchain, kernel headers for the running kernel and the GUI build libraries automatically via `apt`, `pacman`, `dnf` or `zypper` (Debian/Ubuntu, Arch, Fedora, openSUSE). On other distros install those manually first. No udev rule is needed for the OMEN key: the daemon runs as root and reads `/dev/input` directly.

To allow the Linux kernel to access and monitor Embedded Controller (EC) registers without ACPI resource conflicts, configure the following once before running the installer:

### 1. Enable Lax ACPI Resource Enforcement (GRUB)
Open `/etc/default/grub` in an editor:
```bash
sudo nano /etc/default/grub
```
Append `acpi_enforce_resources=lax` to `GRUB_CMDLINE_LINUX_DEFAULT`. For example:
```bash
GRUB_CMDLINE_LINUX_DEFAULT="quiet acpi_enforce_resources=lax"
```
Update your bootloader and reboot:
```bash
sudo update-grub
sudo reboot
```

### 2. Enable EC Read/Write Support (`ec_sys`)
Ensure the kernel `ec_sys` module allows EC register access for hardware sensors:
```bash
echo "options ec_sys write_support=1" | sudo tee /etc/modprobe.d/ec_sys.conf
echo "ec_sys" | sudo tee /etc/modules-load.d/ec_sys.conf
sudo modprobe ec_sys
```

---

## Key Features

* **Dynamic Fan Curves & True 0 RPM Silent Mode**:
  * 7-point interactive curve editor for CPU & GPU.
  * **Silent Fan-Stop (0 RPM)**: Safely stops fans at idle without EC watchdog lockouts.
  * **1100 RPM Hardware Floor**: Smooth, whisper-quiet minimum spinning floor matching HP hardware.
  * **Emergency Thermal Override**: Automatically triggers Max Boost at ≥90°C.
* **CPU Turbo Boost Management**:
  * Toggle CPU Turbo Boost globally or automatically disable on Battery (caps aggressive 5.0 GHz spikes to base frequency, dropping temps by 10–15°C).
* **Power Profiles & MUX Switch**:
  * Eco, Balanced, and Performance modes with dynamic NVIDIA TGP/PPAB power limits.
  * Hardware MUX Switch: Hybrid (iGPU + dGPU) and Discrete (direct dGPU) toggling.
* **Minimal Footprint**:
  * Background daemon uses **< 1 MB RAM** and **0.0% CPU**.
  * GUI spawns on demand via physical OMEN key or system tray, exiting completely when closed (0 MB RAM).

---

## Installation & Removal

### Install / Update
```bash
git clone https://github.com/zyr1on/lightomen.git
cd lightomen
sudo ./install.sh
```
*The installer automatically cleans old processes/PIDs, builds optimized binaries, registers the DKMS driver, sets up module autoloading, and configures systemd.*

### Complete Uninstallation
```bash
sudo ./uninstall.sh
```
*Restores fan cooling to BIOS Auto, cleans DKMS drivers, removes all systemd services, binaries, autostart entries, and configuration files.*

---

## Usage

* **Open GUI:** Run `lightomen-gui` in terminal or click the LightOmen icon in your system tray.
* **Physical Hotkey:** Press the physical OMEN key on your laptop keyboard.
* **Daemon Status:**
  ```bash
  systemctl status lightomen-daemon.service
  ```

---

## Testing

Run workspace unit tests and verification suite:
```bash
cargo test --workspace
sudo ./scripts/verify_fan_hardware.sh
```

---

## Screenshots

<details>
<summary> Home Screen</summary>

<img width="540" height="680" alt="image" src="https://github.com/user-attachments/assets/b2abe5fa-a20f-406a-8e2d-0878fec4763b" />


</details>

<details>
<summary> Curve </summary>

<img width="540" height="680" alt="image" src="https://github.com/user-attachments/assets/816f682b-04ed-43ae-8590-9d689b2868a9" />

</details>
 

<details>
<summary> Graphics </summary>

<img src="https://github.com/user-attachments/assets/db739fdc-28b1-4fae-aa1f-62c3830f50ca" width="540" alt="Settings Screen" />

</details>

<details>
<summary> Settings </summary>

<img src="https://github.com/user-attachments/assets/e2e54ffa-91c5-4d32-ba65-8fc1fc442964" width="540" alt="Additional Features" />

</details>



## License

MIT License / GPL v2.
