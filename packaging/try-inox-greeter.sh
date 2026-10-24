#!/usr/bin/env bash
#
# Start the Inox login screen on a spare VT and switch to it — WITHOUT touching the
# boot chain. display-manager.service is not modified, greetd keeps running, and
# the whole trial is undone by packaging/try-rollback.sh (ADR 0037 §决策 6).
#
# The trial VT is 4 by default: VT1 belongs to the boot chain's display manager and
# VT3 is the rescue getty, so neither is a candidate. Override with
# INOX_TRIAL_VT=2|4|5|6.
#
# The unit pulls the authentication boundary up with it (Wants=inox-auth.service),
# so `systemctl start inox-greeter@4` is the whole trial.
#
# Before it starts anything it runs packaging/preflight.sh (read-only) and refuses
# to go on if that reports a FAIL: a trial whose environment is wrong would take
# the screen away and show nothing, which is worse than not trying.
#
# The screen is only switched after the unit is actually active: a failed start
# leaves you where you are instead of on a black VT.
#
# Usage: sudo packaging/try-inox-greeter.sh

set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly SELF="$SCRIPT_DIR/$(basename -- "${BASH_SOURCE[0]}")"

readonly TRIAL_VT="${INOX_TRIAL_VT:-4}"
readonly TRIAL_UNIT="inox-greeter@${TRIAL_VT}.service"
readonly GREETER_TEMPLATE="/etc/systemd/system/inox-greeter@.service"
readonly BOOT_UNIT="inox-greeter@1.service"
readonly DM_LINK="/etc/systemd/system/display-manager.service"
readonly HOST_BIN="/usr/bin/inox-host"

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
    1) die "VT1 belongs to the boot chain's display manager; pick 2, 4, 5 or 6" ;;
    3) die "VT3 is the rescue getty; pick 2, 4, 5 or 6" ;;
    *) die "VT$TRIAL_VT is outside the spare range (2, 4, 5 or 6)" ;;
esac

if [ "$(id -u)" -ne 0 ]; then
    die "run as root: sudo $SELF"
fi

[ -f "$GREETER_TEMPLATE" ] || die "$GREETER_TEMPLATE is missing — run packaging/install.sh first"
[ -x "$HOST_BIN" ] || die "$HOST_BIN is missing — install the binaries first (packaging/install.sh checks the whole set)"

if [ "$(dm_target)" = "$BOOT_UNIT" ]; then
    log "note: display-manager.service already points at $BOOT_UNIT, i.e. Inox is already"
    log "      the boot chain. A trial on VT${TRIAL_VT} proves nothing extra; this is harmless."
fi

log "boot chain stays as it is: display-manager.service -> $(dm_target)"

# Read-only pre-flight first, and before anything is started: the value of a trial
# is that it is the last step, not the first thing that fails. A FAIL here means
# the attempt cannot succeed, so the screen is never taken away from the desktop.
[ -f "$SCRIPT_DIR/preflight.sh" ] ||
    die "$SCRIPT_DIR/preflight.sh is missing — the trial refuses to run without its pre-flight"

log "pre-flight (read-only, changes nothing):"
if ! bash "$SCRIPT_DIR/preflight.sh"; then
    die "pre-flight reported FAIL lines above, so the VT was not switched and nothing was started"
fi

log "starting $TRIAL_UNIT (which pulls in inox-auth.service) …"

if ! systemctl start "$TRIAL_UNIT"; then
    systemctl status "$TRIAL_UNIT" --no-pager -n 20 || true
    die "$TRIAL_UNIT would not start, so the VT was not switched — the screen is where it was"
fi

# Wait for the unit to be active rather than assuming it: the host exits non-zero
# when it cannot reach the boundary or when the compositor never comes up, and
# systemd then shows the unit as failed/activating.
state=""
attempt=0
while [ "$attempt" -lt 20 ]; do
    attempt=$((attempt + 1))
    state="$(systemctl is-active "$TRIAL_UNIT" || true)"
    if [ "$state" = "active" ] || [ "$state" = "failed" ]; then
        break
    fi
    sleep 0.5
done

if [ "$state" != "active" ]; then
    systemctl status "$TRIAL_UNIT" --no-pager -n 30 || true
    die "$TRIAL_UNIT is '$state', so the VT was not switched — the screen is where it was. See journalctl -u 'inox-greeter@*' -n 50"
fi

if ! chvt "$TRIAL_VT"; then
    die "the unit is running but chvt $TRIAL_VT failed; press Ctrl+Alt+F$TRIAL_VT to reach it"
fi

log "switched to VT${TRIAL_VT}. Log in there."
log ""
log "if you are stuck: Ctrl+Alt+F1 is the boot chain's display manager, Ctrl+Alt+F3 is the"
log "rescue console, and 'sudo $SCRIPT_DIR/try-rollback.sh' stops the trial from anywhere."
log ""
log "after the login succeeds, the VT plumbing is what to check first:"
log "  loginctl list-sessions      # your session must be on a VT in 2..6, not VT${TRIAL_VT}"
log "  journalctl -u 'inox-greeter@*' -b --no-pager"
log "                              # no 'session is inactive' and no libseat/DRM error"
log ""
log "the login card is a single tiled window that fills the work area: inox-compositor has"
log "no fullscreen request handling yet. Looking wrong there is cosmetic, not a login failure."
