#!/usr/bin/env bash
#
# End the trial run: stop inox-greeter@<VT>, stop the authentication boundary it
# started, and put the screen back on the boot chain's greeter. The boot chain
# itself (display-manager.service, greetd) was never touched by the trial and is
# not touched here either.
#
# Idempotent: with nothing running it says so and only returns the screen.
#
# The trial VT is 4 by default; override with INOX_TRIAL_VT=2|4|5|6 to match what
# packaging/try-inox-greeter.sh was given.
#
# Usage: sudo packaging/try-rollback.sh

set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly SELF="$SCRIPT_DIR/$(basename -- "${BASH_SOURCE[0]}")"

readonly TRIAL_VT="${INOX_TRIAL_VT:-4}"
readonly TRIAL_UNIT="inox-greeter@${TRIAL_VT}.service"
readonly AUTH_UNIT="inox-auth.service"
readonly GREETER_TEMPLATE="/etc/systemd/system/inox-greeter@.service"
readonly DM_LINK="/etc/systemd/system/display-manager.service"

log() { printf '[inox-trial] %s\n' "$*"; }

die() {
    printf '[inox-trial] error: %s\n' "$*" >&2
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

case "$TRIAL_VT" in
    '' | *[!0-9]*) die "INOX_TRIAL_VT must be a VT number, not '$TRIAL_VT'" ;;
    2 | 4 | 5 | 6) ;;
    *) die "INOX_TRIAL_VT=$TRIAL_VT is not a trial VT (2, 4, 5 or 6)" ;;
esac

if [ "$(id -u)" -ne 0 ]; then
    die "run as root: sudo $SELF"
fi

if [ -f "$GREETER_TEMPLATE" ]; then
    if systemctl is-active --quiet "$TRIAL_UNIT"; then
        log "stopping $TRIAL_UNIT"
        systemctl stop "$TRIAL_UNIT"
        log "stopped $TRIAL_UNIT"
    else
        log "$TRIAL_UNIT is not running"
    fi
else
    log "$GREETER_TEMPLATE is not installed; nothing of the trial can be running"
fi

# The boundary is only stopped when no Inox login screen is left at all — the boot
# chain's own greeter (inox-greeter@1) needs it just as much as the trial one.
remaining="$(systemctl list-units 'inox-greeter@*.service' --state=active --no-legend --plain |
    awk 'END { print NR }')"
if [ "$remaining" -eq 0 ]; then
    if systemctl is-active --quiet "$AUTH_UNIT"; then
        log "stopping $AUTH_UNIT (no Inox login screen is running any more)"
        systemctl stop "$AUTH_UNIT"
    else
        log "$AUTH_UNIT is not running"
    fi
else
    log "keeping $AUTH_UNIT: $remaining Inox login screen unit(s) still active"
fi

# Put the screen back. The boot chain's greeter runs as the `greeter` account too,
# so the session list is filtered by VT instead of by user name.
target_vt="$(
    loginctl list-sessions --no-legend 2>/dev/null |
        awk -v trial="tty${TRIAL_VT}" '
            $3 == "greeter" && $5 ~ /^tty[0-9]+$/ && $5 != trial {
                sub(/^tty/, "", $5)
                print $5
                exit
            }
        ' || true
)"

if [ -n "$target_vt" ]; then
    log "switching the screen back to VT${target_vt}"
    chvt "$target_vt" || log "chvt ${target_vt} failed; press Ctrl+Alt+F${target_vt}"
else
    log "no greeter session other than VT${TRIAL_VT} was found; not switching the screen"
    log "press Ctrl+Alt+F1 (or F2..F6) to return to the boot chain's display manager"
fi

log "done. The boot chain is still display-manager.service -> $(dm_target)"
