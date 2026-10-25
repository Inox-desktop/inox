#!/usr/bin/env bash
#
# Remove what packaging/install.sh installed: the three PAM stacks and the three
# systemd units. The greeter account and /var/lib/inox-greeter are left alone (the
# account may predate Inox — greetd creates one with the same name), and so is
# getty@tty3.service: the rescue console stays enabled no matter what (the boot login chain
# §决策 2).
#
# It refuses to run while display-manager.service still points at Inox: removing a
# display manager's unit while it is the boot chain is exactly the half-switched
# state this packaging exists to avoid. Run packaging/rollback-to-greetd.sh first.
#
# Idempotent: with nothing installed it reports that and changes nothing.
#
# Usage: sudo packaging/uninstall.sh [--remove-binaries]
#
#   --remove-binaries also deletes the Inox binaries from /usr/bin and the bundled
#   wallpapers from /usr/share/inox/wallpapers. Off by default: those are the same
#   paths a distribution package owns, and removing them from under a package
#   manager is worse than leaving them. A tarball install removes them this way.

set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly SELF="$SCRIPT_DIR/$(basename -- "${BASH_SOURCE[0]}")"

readonly DATA_DIR="/usr/share"
readonly BIN_DIR="/usr/bin"
readonly PAM_DIR="/etc/pam.d"
readonly UNIT_DIR="/etc/systemd/system"
readonly DM_LINK="$UNIT_DIR/display-manager.service"

# The same list install.sh installs; see the comment there for why it is split.
readonly REQUIRED_BINARIES=(
    inox-auth
    inox-host
    inox-compositor
    inox-greet
    inox-session
    inox-shell
    inox-panel
    inox-lock
)
readonly OPTIONAL_BINARIES=(
    inox-wallpaper
    inox-notifications
    inox-idle
    inox-xwayland
    inox-launcher
    inox-settings
)
readonly BINARIES=("${REQUIRED_BINARIES[@]}" "${OPTIONAL_BINARIES[@]}")

readonly AUTH_UNIT="inox-auth.service"
readonly GREETER_TEMPLATE="inox-greeter@.service"
readonly FALLBACK_UNIT="inox-login-fallback.service"
readonly BOOT_UNIT="inox-greeter@1.service"
readonly RESCUE_UNIT="getty@tty3.service"

log() { printf '[inox-uninstall] %s\n' "$*"; }
warn() { printf '[inox-uninstall] warning: %s\n' "$*" >&2; }

die() {
    printf '[inox-uninstall] error: %s\n' "$*" >&2
    exit 1
}

# The unit display-manager.service currently resolves to, or "(none)".
dm_target() {
    if [ ! -L "$DM_LINK" ] && [ ! -e "$DM_LINK" ]; then
        printf '(none)'
    else
        basename -- "$(readlink -f "$DM_LINK")"
    fi
}

remove_binaries=false
for argument in "$@"; do
    case "$argument" in
        --remove-binaries) remove_binaries=true ;;
        -h | --help)
            printf 'usage: sudo %s [--remove-binaries]\n' "$SELF"
            exit 0
            ;;
        *) die "unrecognized argument: $argument (see --help)" ;;
    esac
done

if [ "$(id -u)" -ne 0 ]; then
    die "run as root: sudo $SELF"
fi

if [ "$(dm_target)" = "$BOOT_UNIT" ]; then
    die "display-manager.service still points at $BOOT_UNIT, so Inox is the boot chain.
       Run packaging/rollback-to-greetd.sh first, then run this again."
fi

# --- stop and disable ------------------------------------------------------

active_greeters="$(
    systemctl list-units 'inox-greeter@*.service' --state=active --no-legend --plain |
        awk '{ print $1 }'
)"
if [ -n "$active_greeters" ]; then
    while read -r unit; do
        [ -n "$unit" ] || continue
        systemctl disable --now "$unit" || warn "could not stop/disable $unit"
        log "stopped and disabled $unit"
    done <<<"$active_greeters"
else
    log "no Inox login screen is running"
fi

if systemctl cat "$AUTH_UNIT" >/dev/null 2>&1; then
    systemctl disable --now "$AUTH_UNIT" || warn "could not stop/disable $AUTH_UNIT"
    log "stopped and disabled $AUTH_UNIT"
else
    log "$AUTH_UNIT is not installed"
fi

# A template instance that was enabled individually (not via the alias) leaves a
# graphical.target.wants symlink behind; remove any that point at our template.
# `disable` on an unknown instance is not an error for systemd as long as the
# template exists, which is checked right below.
if [ -f "$UNIT_DIR/$GREETER_TEMPLATE" ]; then
    for wants in "$UNIT_DIR"/*.wants; do
        [ -d "$wants" ] || continue
        for link in "$wants"/inox-greeter@*.service; do
            [ -L "$link" ] || continue
            rm -f -- "$link"
            log "removed leftover enable symlink $link"
        done
    done
fi

# --- remove files ----------------------------------------------------------

removed_any=false

for unit in "$AUTH_UNIT" "$GREETER_TEMPLATE" "$FALLBACK_UNIT"; do
    if [ -f "$UNIT_DIR/$unit" ]; then
        rm -f -- "$UNIT_DIR/$unit"
        log "removed $UNIT_DIR/$unit"
        removed_any=true
    fi
done

# The guard above already refused the boot instance; an alias pointing at some
# other Inox greeter instance (a hand-rolled switch) is still ours to remove.
case "$(dm_target)" in
    inox-greeter@*)
        rm -f -- "$DM_LINK"
        log "removed the Inox display-manager.service alias"
        removed_any=true
        ;;
esac

systemctl daemon-reload
log "ran systemctl daemon-reload"

for stack in inox-auth inox-auth-session inox-greeter; do
    if [ -f "$PAM_DIR/$stack" ]; then
        rm -f -- "$PAM_DIR/$stack"
        log "removed $PAM_DIR/$stack"
        removed_any=true
    fi
done

# The binaries and the bundled wallpapers are removed only when they are ours to
# remove. A tarball install put them there via `install.sh --bin-dir`, but the same
# paths are where a distribution package would put its files, and deleting those
# from under the package manager is worse than leaving them behind. So this runs
# only when it was asked for explicitly.
if [ "$remove_binaries" = true ]; then
    for binary in "${BINARIES[@]}"; do
        if [ -f "$BIN_DIR/$binary" ]; then
            rm -f -- "$BIN_DIR/$binary"
            log "removed $BIN_DIR/$binary"
            removed_any=true
        fi
    done
    if [ -d "$DATA_DIR/inox/wallpapers" ]; then
        rm -rf -- "$DATA_DIR/inox/wallpapers"
        log "removed $DATA_DIR/inox/wallpapers"
        removed_any=true
    fi
fi

if [ "$removed_any" = false ]; then
    log "nothing was installed; nothing was removed"
fi

# --- what is deliberately kept --------------------------------------------

if systemctl is-enabled --quiet "$RESCUE_UNIT" 2>/dev/null; then
    log "kept $RESCUE_UNIT enabled: VT3 is the rescue console and is never removed"
else
    warn "$RESCUE_UNIT is not enabled. Inox did not disable it; enable it by hand if VT3"
    warn "  should be a rescue console: systemctl enable --now $RESCUE_UNIT"
fi

if getent passwd greeter >/dev/null 2>&1; then
    log "kept the greeter account (uid $(id -u greeter)) and /var/lib/inox-greeter"
    log "  remove them yourself if nothing else uses them: userdel greeter && rm -rf /var/lib/inox-greeter"
fi

log "done. The boot chain is display-manager.service -> $(dm_target)"
