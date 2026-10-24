#!/usr/bin/env bash
#
# Give the boot chain back to greetd (ADR 0037 §决策 6, §兼容、迁移与回退).
#
# Order is the whole point, and it is the order this script enforces:
#
#   1. display-manager.service is pointed back at greetd and greetd is enabled.
#      Nothing is running yet — the Inox greeter keeps the screen. If this step
#      fails, the machine is exactly as it was.
#   2. Only then are the Inox units stopped (greeter instances first, boundary last).
#   3. greetd is started. If it refuses, the script says how to get back to a login
#      screen rather than leaving a blank VT1.
#
# The PAM stacks and the unit files are left installed: they do not affect greetd,
# and a later trial or switch needs them. packaging/uninstall.sh removes them.
#
# Idempotent: running it again on an already-rolled-back machine re-asserts the
# alias and finds nothing to stop.
#
# Usage: sudo packaging/rollback-to-greetd.sh

set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly SELF="$SCRIPT_DIR/$(basename -- "${BASH_SOURCE[0]}")"

readonly UNIT_DIR="/etc/systemd/system"
readonly DM_LINK="$UNIT_DIR/display-manager.service"
readonly AUTH_UNIT="inox-auth.service"
readonly BOOT_UNIT="inox-greeter@1.service"

log() { printf '[inox-rollback] %s\n' "$*"; }
warn() { printf '[inox-rollback] warning: %s\n' "$*" >&2; }

die() {
    printf '[inox-rollback] error: %s\n' "$*" >&2
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

if [ "$(id -u)" -ne 0 ]; then
    die "run as root: sudo $SELF"
fi

# --- preflight -------------------------------------------------------------
# A rollback needs somewhere to roll back TO. Finding that out before the first
# write is what makes "any failure leaves the machine bootable" true.

greetd_fragment="$(systemctl show -p FragmentPath --value greetd.service 2>/dev/null || true)"
if [ -z "$greetd_fragment" ] || [ ! -f "$greetd_fragment" ]; then
    die "greetd.service is not installed, so there is nothing to roll back to.
       If greetd was removed on purpose, install it first, or edit
       $DM_LINK by hand to point at the display manager you want."
fi

log "current chain: display-manager.service -> $(dm_target)"
log "target chain:  display-manager.service -> greetd.service ($greetd_fragment)"

# --- step 1: switch the chain, touch no running service --------------------

log "step 1/3: enabling greetd and pointing display-manager.service at it"
log "  nothing has been stopped; the Inox greeter still has the screen"

systemctl enable greetd.service
log "enabled greetd.service"

# `systemctl enable` recreates greetd's display-manager.service alias, but do not
# rely on that having replaced our symlink: write the link explicitly and then
# verify it.
ln -sfn "$greetd_fragment" "$DM_LINK"
systemctl daemon-reload

resolved="$(dm_target)"
if [ "$resolved" != "greetd.service" ]; then
    die "display-manager.service resolves to '$resolved', not greetd.service. Nothing was
       stopped. Fix by hand: ln -sfn $greetd_fragment $DM_LINK && systemctl daemon-reload"
fi

log "verified: display-manager.service resolves to $resolved"

# --- step 2: stop the Inox units -------------------------------------------

log "step 2/3: stopping the Inox login screen and the authentication boundary"

active_greeters="$(
    systemctl list-units 'inox-greeter@*.service' --state=active --no-legend --plain |
        awk '{ print $1 }'
)"
if [ -n "$active_greeters" ]; then
    while read -r unit; do
        [ -n "$unit" ] || continue
        if systemctl stop "$unit"; then
            log "stopped $unit"
        else
            warn "could not stop $unit; stop it by hand: systemctl stop $unit"
        fi
    done <<<"$active_greeters"
else
    log "no Inox login screen is running"
fi

if systemctl cat "$AUTH_UNIT" >/dev/null 2>&1; then
    systemctl disable --now "$AUTH_UNIT" || warn "could not stop/disable $AUTH_UNIT; do it by hand"
    log "stopped and disabled $AUTH_UNIT"
else
    log "$AUTH_UNIT is not installed; nothing to stop"
fi

# --- step 3: start greetd --------------------------------------------------

log "step 3/3: starting greetd.service"

if ! systemctl start greetd.service; then
    systemctl status greetd.service --no-pager -n 30 || true
    warn "greetd.service would not start. The boot chain now points at greetd, so the
       next boot will try greetd again; this boot has no display manager."
    warn "recovery, from this shell or from Ctrl+Alt+F3:"
    warn "  sudo systemctl start $BOOT_UNIT   # back to the Inox login screen on VT1"
    warn "  journalctl -u greetd.service -b --no-pager"
    exit 1
fi

if ! systemctl is-active --quiet greetd.service; then
    systemctl status greetd.service --no-pager -n 20 || true
    warn "greetd.service started but is not active. Recovery as above (start $BOOT_UNIT),"
    warn "or press Ctrl+Alt+F3 for the rescue console."
    exit 1
fi

log "done. greetd.service is running and owns display-manager.service."
log "the Inox PAM stacks and units are still installed (they do not affect greetd);"
log "  remove them with: sudo $SCRIPT_DIR/uninstall.sh"
