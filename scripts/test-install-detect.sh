#!/usr/bin/env bash
# Tests for scripts/pkg-deps.sh using stub commands on PATH.
set -u
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/pkg-deps.sh
. "$SCRIPT_DIR/pkg-deps.sh"

FAILS=0
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

check() {
    if [ "$2" = "$3" ]; then
        echo "ok   - $1"
    else
        echo "FAIL - $1: expected '$3', got '$2'"
        FAILS=$((FAILS + 1))
    fi
}

# Run detect_pkg_manager with only the given stub commands on PATH.
detect_with() {
    local dir="$TMP/bin-$RANDOM$RANDOM" c
    mkdir -p "$dir"
    for c in "$@"; do
        printf '#!/bin/sh\nexit 0\n' > "$dir/$c"
        chmod +x "$dir/$c"
    done
    PATH="$dir" detect_pkg_manager
}

check "apt"    "$(detect_with apt-get)" "apt"
check "pacman" "$(detect_with pacman)" "pacman"
check "dnf"    "$(detect_with dnf)" "dnf"
check "zypper" "$(detect_with zypper)" "zypper"
check "apt wins over dnf" "$(detect_with dnf apt-get)" "apt"
check "none"   "$(detect_with || echo none)" "none"

check "apt headers"    "$(kernel_headers_pkg apt 6.8.0-1-generic)" "linux-headers-6.8.0-1-generic"
check "dnf headers"    "$(kernel_headers_pkg dnf 6.9.1-200.fc40.x86_64)" "kernel-devel-6.9.1-200.fc40.x86_64"
check "zypper headers" "$(kernel_headers_pkg zypper 6.4.0)" "kernel-default-devel"
check "pacman headers" "$(kernel_headers_pkg pacman 0.0.0-nonexistent)" "linux-headers"
check "unknown deps"   "$(build_deps foo || echo none)" "none"
for pm in apt pacman dnf zypper; do
    case " $(build_deps "$pm") " in
        *" dkms "*) check "$pm deps include dkms" yes yes ;;
        *) check "$pm deps include dkms" no yes ;;
    esac
done

[ "$FAILS" -eq 0 ] && echo "All tests passed." || { echo "$FAILS test(s) failed."; exit 1; }
