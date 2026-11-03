#!/usr/bin/env bash
# Package manager helpers shared by install.sh (and tested by test-install-detect.sh).

# Print the first supported package manager found in PATH: apt, pacman, dnf, zypper.
# Returns 1 and prints nothing if none is found.
detect_pkg_manager() {
    local pm
    for pm in apt-get pacman dnf zypper; do
        if command -v "$pm" >/dev/null 2>&1; then
            [ "$pm" = "apt-get" ] && pm="apt"
            echo "$pm"
            return 0
        fi
    done
    return 1
}

# Print the kernel headers package name for the running kernel.
# $1 = package manager, $2 = kernel release (defaults to uname -r)
kernel_headers_pkg() {
    local pm="$1" krel="${2:-$(uname -r)}" pkgbase
    case "$pm" in
        apt) echo "linux-headers-$krel" ;;
        pacman)
            # Arch ships headers per kernel flavour (linux-lts, linux-zen, ...)
            pkgbase="linux"
            if [ -r "/usr/lib/modules/$krel/pkgbase" ]; then
                pkgbase="$(cat "/usr/lib/modules/$krel/pkgbase")"
            fi
            echo "${pkgbase}-headers"
            ;;
        dnf) echo "kernel-devel-$krel" ;;
        zypper) echo "kernel-default-devel" ;;
        *) return 1 ;;
    esac
}

# Print the build/runtime dependency list (without kernel headers).
build_deps() {
    case "$1" in
        apt) echo "dkms build-essential libudev-dev libfontconfig1-dev libx11-dev libdbus-1-dev libxkbcommon-dev" ;;
        pacman) echo "dkms base-devel dbus fontconfig libx11 libxkbcommon systemd-libs" ;;
        dnf) echo "dkms gcc make dbus-devel systemd-devel fontconfig-devel libX11-devel libxkbcommon-devel" ;;
        zypper) echo "dkms gcc make dbus-1-devel libudev-devel fontconfig-devel libX11-devel libxkbcommon-devel" ;;
        *) return 1 ;;
    esac
}

# Install build deps and kernel headers using the given package manager.
install_build_deps() {
    local pm="$1" headers
    local -a deps
    read -r -a deps <<< "$(build_deps "$pm")"
    headers="$(kernel_headers_pkg "$pm")"
    case "$pm" in
        apt)
            apt-get update -qq || true
            apt-get install -y -qq "${deps[@]}" "$headers" ||
                apt-get install -y -qq "${deps[@]}" linux-headers-generic
            ;;
        pacman) pacman -Sy --noconfirm --needed "${deps[@]}" "$headers" ;;
        dnf) dnf install -y -q "${deps[@]}" "$headers" || dnf install -y -q "${deps[@]}" kernel-devel ;;
        zypper) zypper --non-interactive install "${deps[@]}" "$headers" ;;
        *) return 1 ;;
    esac
}
