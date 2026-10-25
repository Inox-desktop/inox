#!/usr/bin/env bash
#
# Pre-flight for the login chain: read the environment the chain will run in and
# list everything that would break a trial — BEFORE the screen is switched away
# from the current desktop.
#
# This script is strictly read-only. It starts, stops, enables and disables
# nothing; it writes nothing outside a temporary directory (and does not even need
# that). Every check prints one `ok` / `WARN` / `FAIL` line and, where there is
# something to do about it, one line saying what.
#
# WARN means "the chain can still work, but this is not what the packaging
# expects". FAIL means "this attempt cannot succeed" — the exit code is non-zero
# if there is at least one FAIL.
#
# Run it as root: it reads /etc/pam.d and /etc/systemd/system, and it asks logind
# about sessions that are not yours.
#
# Usage: sudo packaging/preflight.sh
#
# Related: packaging/rehearse-headless.sh (E2, no root: does the host stage run?),
# packaging/verify-login.sh (no root: did the login that just happened really
# happen?). Decision: docs/book/src/usage/login.md.

set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly SELF="$SCRIPT_DIR/$(basename -- "${BASH_SOURCE[0]}")"

readonly BIN_DIR="/usr/bin"
readonly PAM_DIR="/etc/pam.d"
readonly UNIT_DIR="/etc/systemd/system"
readonly DM_LINK="$UNIT_DIR/display-manager.service"
readonly STATE_DIR="/var/lib/inox-greeter"

readonly GREETER_ACCOUNT="greeter"
readonly TRIAL_VT="4"
readonly RESCUE_VT="3"
readonly BOOT_VT="1"

# inox-session and inox-auth are checked too: the trial starts neither, but the
# boot chain does, and a missing session program turns a correct password into a
# failed login (crates/inox-auth/src/session.rs).
readonly BINARIES=(inox-auth inox-host inox-compositor inox-greet inox-session)
# Three stacks, three jobs: the password, the session, the greeter's own session.
readonly STACKS=(inox-auth inox-auth-session inox-greeter)
# The fallback unit is checked too: inox-greeter@1.service names it in OnFailure=,
# and a missing OnFailure target is dropped with a log line nobody reads — the
# machine would boot Inox with no fallback and no sign that the fallback is gone.
readonly UNITS=(inox-auth.service inox-greeter@.service inox-login-fallback.service)
readonly SECURITY_DIRS=(/usr/lib/security /lib/security /usr/lib64/security /lib64/security)

failures=0
warnings=0

section() { printf '\n[inox-preflight] %s\n' "$*"; }
ok() { printf '  ok    %s\n' "$*"; }
warn() {
    warnings=$((warnings + 1))
    printf '  WARN  %s\n' "$*"
}
fail() {
    failures=$((failures + 1))
    printf '  FAIL  %s\n' "$*"
}
note() { printf '        %s\n' "$*"; }

usage() {
    cat <<EOF
usage: sudo $SELF

Read-only pre-flight for the login chain. Prints one ok / WARN / FAIL line per
check and exits non-zero if anything FAILed.

  ok the packaging and the environment are what the chain expects
  WARN the chain can still work, but this is not what the packaging expects
  FAIL this attempt cannot succeed; the line below says what to do

It starts, stops, enables and disables nothing and writes nothing outside a
temporary directory.

Related:
  packaging/rehearse-headless.sh E2, no root: does the host stage run at all?
  packaging/verify-login.sh no root: did the login that just happened work?
EOF
}

if [ "$#" -gt 0 ]; then
    case "$1" in
        -h | --help)
            usage
            exit 0
            ;;
        *)
            printf '[inox-preflight] error: unrecognized argument: %s (see --help)\n' "$1" >&2
            exit 2
            ;;
    esac
fi

if [ "$(id -u)" -ne 0 ]; then
    printf '[inox-preflight] error: run as root: sudo %s\n' "$SELF" >&2
    printf '[inox-preflight]        it reads /etc and asks logind about sessions that are not yours\n' >&2
    exit 2
fi

printf '[inox-preflight] read-only check of the login chain; nothing here changes state\n'

# --- binaries --------------------------------------------------------------

section "binaries in $BIN_DIR"
missing_binaries=()
for binary in "${BINARIES[@]}"; do
    if [ -x "$BIN_DIR/$binary" ]; then
        ok "$BIN_DIR/$binary"
    else
        missing_binaries+=("$binary")
        fail "$BIN_DIR/$binary is missing or not executable"
    fi
done
if [ "${#missing_binaries[@]}" -gt 0 ]; then
    note "build from the checkout, then install all five (the units hard-code this prefix):"
    note "  cargo build --workspace --release"
    note "  sudo install -m 0755 target/release/inox-auth target/release/inox-host \\"
    note "      target/release/inox-compositor target/release/inox-greet \\"
    note "      target/release/inox-session $BIN_DIR/"
fi

# --- PAM stacks ------------------------------------------------------------

section "PAM stacks and the modules they reach"

seen_modules=" "
module_count=0
seen_missing=" "

# One service's own lines, with `include`/`substack` marked so the caller can
# recurse: prints "include <service>" or "module <name> <required|optional>".
stack_entries() {
    local service="$1"
    awk '
        /^[[:space:]]*#/ { next }
        NF == 0 { next }
        {
            type = $1
            if ($0 ~ /\[[^]]*\]/) {
                rest = $0
                sub(/^[^[]*\[[^]]*\][[:space:]]*/, "", rest)
                split(rest, parts, /[[:space:]]+/)
                module = parts[1]
                optional = (type ~ /^-/) || ($0 ~ /optional/)
                control = ""
            } else {
                control = $2
                module = $3
                optional = (type ~ /^-/) || (control == "optional")
            }
            if (module == "") next
            if (control == "include" || control == "substack") {
                print "include", module
                next
            }
            if (module ~ /\.so$/) {
                print "module", module, (optional ? "optional" : "required")
            }
        }
    ' "$PAM_DIR/$service"
}

check_module() {
    local name="$1" requiredness="$2" dir
    case "$name" in
        */*)
            [ -f "$name" ] && return 0
            ;;
    esac
    for dir in "${SECURITY_DIRS[@]}"; do
        if [ -f "$dir/$name" ]; then
            case "$seen_modules" in
                *" $name "*) ;;
                *)
                    seen_modules="$seen_modules$name "
                    module_count=$((module_count + 1))
                    ;;
            esac
            return 0
        fi
    done

    case "$seen_missing" in
        *" $name "*) return 0 ;;
    esac
    seen_missing="$seen_missing$name "

    if [ "$requiredness" = "optional" ]; then
        warn "$name is not installed, but every line that reaches it is optional"
    else
        fail "$name is not installed, and a required line reaches it"
        note "install the package that provides $name, or the affected login fails inside PAM"
    fi
}

scan_stack() {
    local service="$1" depth="$2" kind first second
    if [ "$depth" -gt 8 ]; then
        warn "include depth exceeded while reading $service (loop in the PAM config?)"
        return 0
    fi
    while read -r kind first second; do
        case "$kind" in
            include) scan_stack "$first" $((depth + 1)) ;;
            module) check_module "$first" "${second:-required}" ;;
            *) ;;
        esac
    done < <(stack_entries "$service")
}

for stack in "${STACKS[@]}"; do
    if [ -f "$PAM_DIR/$stack" ]; then
        ok "$PAM_DIR/$stack"
        scan_stack "$stack" 0
    else
        fail "$PAM_DIR/$stack is not installed"
        note "run: sudo $SCRIPT_DIR/install.sh   (installs all three stacks)"
    fi
done

if [ "$module_count" -gt 0 ]; then
    ok "$module_count PAM modules resolve in /usr/lib/security"
    note "$(printf '%s' "$seen_modules" | sed 's/^ //; s/ $//; s/ /, /g')"
fi

# --- systemd units ---------------------------------------------------------

section "systemd units"
units_present=true
for unit in "${UNITS[@]}"; do
    if [ -f "$UNIT_DIR/$unit" ]; then
        ok "$UNIT_DIR/$unit"
    else
        units_present=false
        fail "$UNIT_DIR/$unit is not installed"
        note "run: sudo $SCRIPT_DIR/install.sh"
    fi
done

if [ "$units_present" = true ]; then
    # Everything systemd-analyze says, minus the note that the binary is not
    # installed yet (the binaries section has already judged that, and on an
    # installed system it is silent anyway) and minus the reference unit's own
    # noise, which belongs to the distribution's getty@.service, not to ours.
    verify_output="$(
        systemd-analyze verify \
            "$UNIT_DIR/inox-auth.service" \
            "$UNIT_DIR/inox-greeter@.service" \
            "$UNIT_DIR/inox-login-fallback.service" 2>&1 || true
    )"
    verify_problems="$(
        printf '%s\n' "$verify_output" |
            grep -v -E 'is not executable|/usr/lib/systemd/system/getty@\.service|^[[:space:]]*$' || true
    )"
    if [ -z "$verify_problems" ]; then
        ok "systemd-analyze verify is clean"
    else
        fail "systemd-analyze verify reports problems"
        printf '%s\n' "$verify_problems" | sed 's/^/        /'
    fi

    if [ "$(systemctl is-enabled inox-auth.service 2>/dev/null || true)" = "enabled" ]; then
        ok "inox-auth.service is enabled (it must be, for the boot chain)"
    else
        warn "inox-auth.service is not enabled: a trial still works (the greeter unit pulls it in), the boot chain does not"
        note "run: sudo $SCRIPT_DIR/install.sh --enable   (when the trial has passed)"
    fi
fi

# --- greeter account -------------------------------------------------------

section "the greeter account"
greeter_record="$(getent passwd "$GREETER_ACCOUNT" 2>/dev/null || true)"
if [ -z "$greeter_record" ]; then
    fail "no account named '$GREETER_ACCOUNT'"
    note "run: sudo $SCRIPT_DIR/install.sh   (creates it as a system account)"
else
    greeter_uid="$(printf '%s' "$greeter_record" | cut -d: -f3)"
    greeter_gid="$(printf '%s' "$greeter_record" | cut -d: -f4)"
    greeter_shell="$(printf '%s' "$greeter_record" | cut -d: -f7)"
    ok "account '$GREETER_ACCOUNT' exists (uid $greeter_uid, gid $greeter_gid, shell $greeter_shell)"

    if [ "$greeter_uid" -eq 0 ]; then
        fail "the greeter account has uid 0 — that is root, and the boundary would admit root"
    elif [ "$greeter_uid" -lt 1000 ]; then
        ok "uid $greeter_uid is a system uid, so inox-auth refuses it as a login account"
    else
        fail "uid $greeter_uid is not a system uid (>= 1000): inox-auth would treat the greeter as a person"
        note "recreate the account as a system account: useradd --system ..."
    fi

    case "$greeter_shell" in
        */nologin | */false | "")
            ok "shell is $greeter_shell (the greeter cannot be logged into)"
            ;;
        *)
            warn "shell is $greeter_shell, not a nologin shell"
            note "harmless for this chain (the greeter's own stack uses pam_permit, so pam_shells never runs,"
            note "and inox-auth refuses the system account anyway) — but a system account with a shell is a smell"
            ;;
    esac

    if [ -d "$STATE_DIR" ]; then
        state_owner="$(stat -c '%U:%G' "$STATE_DIR")"
        if [ "$state_owner" = "$GREETER_ACCOUNT:$GREETER_ACCOUNT" ]; then
            ok "$STATE_DIR is owned by $state_owner"
        else
            warn "$STATE_DIR is owned by $state_owner, expected $GREETER_ACCOUNT:$GREETER_ACCOUNT"
            note "fix: sudo chown -R $GREETER_ACCOUNT:$GREETER_ACCOUNT $STATE_DIR"
        fi
    else
        ok "$STATE_DIR does not exist yet (StateDirectory= creates it on first start)"
    fi
fi

# --- VT and device facts ---------------------------------------------------

section "VT and device facts"
active_vt="$(cat /sys/class/tty/tty0/active 2>/dev/null || true)"
if [ -n "$active_vt" ]; then
    ok "active VT: $active_vt"
else
    warn "cannot read the active VT (/sys/class/tty/tty0/active)"
fi

# VT4 is where the trial puts the login screen, so a missing /dev/tty4 means the
# trial cannot show anything. VT5/VT6 only matter once the user session is placed
# on them — whether it is depends on what is free at login time — so their absence
# is worth knowing but not a reason to refuse the trial.
if [ -c "/dev/tty$TRIAL_VT" ]; then
    ok "/dev/tty$TRIAL_VT exists (the trial's VT)"
else
    fail "/dev/tty$TRIAL_VT does not exist"
    note "a trial on VT$TRIAL_VT would come up on no terminal at all — that is what a black screen means"
fi
for vt in 5 6; do
    if [ -c "/dev/tty$vt" ]; then
        ok "/dev/tty$vt exists (a possible VT for the user session)"
    else
        warn "/dev/tty$vt does not exist: the user session cannot be placed there"
        note "inox-auth picks the first free VT in 2..=6; with one missing, fewer candidates"
    fi
done

if [ -c "/dev/tty$RESCUE_VT" ]; then
    ok "/dev/tty$RESCUE_VT exists (the rescue console can only help if it does)"
else
    fail "/dev/tty$RESCUE_VT does not exist: Ctrl+Alt+F$RESCUE_VT would reach nothing"
fi

drm_cards="$(find /dev/dri -maxdepth 1 -name 'card*' -print 2>/dev/null || true)"
if [ -n "$drm_cards" ]; then
    ok "DRM devices: $(printf '%s ' $drm_cards)"
else
    fail "no /dev/dri/card* — the compositor has nothing to take the display from"
    note "this is the other half of a black screen: libseat grants a session, the session needs a GPU"
fi

if systemctl is-enabled "getty@tty${RESCUE_VT}.service" >/dev/null 2>&1; then
    ok "getty@tty${RESCUE_VT}.service is enabled (rescue console)"
else
    warn "getty@tty${RESCUE_VT}.service is not enabled: there is no rescue console"
    note "fix: sudo systemctl enable --now getty@tty${RESCUE_VT}.service"
fi

# Who owns VT1 right now? On a greetd machine: the greeter's session, as greeter.
# The TTY is asked for per session rather than read out of a fixed column: the
# column order of `loginctl list-sessions` is not a stable interface (systemd 262
# prints a LEADER column that older versions did not).
vt1_found=false
for session in $(loginctl list-sessions --no-legend 2>/dev/null | awk '{ print $1 }'); do
    case "$session" in
        '' | *[!0-9]*) continue ;;
    esac
    if [ "$(loginctl show-session "$session" -p TTY --value 2>/dev/null || true)" = "tty$BOOT_VT" ]; then
        vt1_found=true
        ok "VT$BOOT_VT is held by session $session (user $(loginctl show-session "$session" -p Name --value 2>/dev/null || echo '?'), service $(loginctl show-session "$session" -p Service --value 2>/dev/null || echo '?'))"
    fi
done
if [ "$vt1_found" = false ]; then
    warn "no logind session is on VT$BOOT_VT right now (no display manager running on it?)"
fi

# --- seat and logind -------------------------------------------------------

section "seat and logind"
if active_session="$(loginctl show-seat seat0 -p ActiveSession --value 2>/dev/null)"; then
    if [ -n "$active_session" ]; then
        ok "seat0 ActiveSession=$active_session"
        active_name="$(loginctl show-session "$active_session" -p Name --value 2>/dev/null || true)"
        # VT is empty on some setups (and in sandboxes with no /dev/ttyN at all);
        # the TTY is the same fact with a different spelling.
        active_vt_field="$(loginctl show-session "$active_session" -p VT --value 2>/dev/null || true)"
        if [ -z "$active_vt_field" ]; then
            active_vt_field="$(loginctl show-session "$active_session" -p TTY --value 2>/dev/null || true)"
        fi
        invoking_user="${SUDO_USER:-$(logname 2>/dev/null || true)}"
        if [ -n "$invoking_user" ] && [ "$active_name" = "$invoking_user" ]; then
            ok "the active session belongs to $active_name (${active_vt_field:-no VT})"
        else
            warn "the active session belongs to '${active_name:-?}', not to '${invoking_user:-?}'"
            note "fine if you are not on that seat right now (ssh, another user); the trial needs the real seat"
        fi
    else
        fail "seat0 has no ActiveSession"
        note "without an active session libseat cannot grant the compositor anything"
    fi
else
    fail "loginctl cannot be reached"
    note "this machine's logind (or systemd's D-Bus) is not answering; the chain needs it"
fi

# --- the boot chain, reported not judged -----------------------------------

section "the boot chain (reported, not judged)"
if [ -e "$DM_LINK" ]; then
    dm_target="$(basename -- "$(readlink -f "$DM_LINK")")"
    ok "display-manager.service -> $dm_target"
else
    warn "no display-manager.service at all (no display manager on this machine?)"
fi

greetd_state="$(systemctl is-enabled greetd.service 2>/dev/null || true)"
inox_enabled=false
if [ "$(systemctl is-enabled inox-greeter@"$BOOT_VT".service 2>/dev/null || true)" = "enabled" ] ||
    [ "${dm_target:-}" = "inox-greeter@$BOOT_VT.service" ]; then
    inox_enabled=true
fi
ok "greetd.service: ${greetd_state:-not installed}"

if [ "$inox_enabled" = true ] && [ "$greetd_state" = "enabled" ]; then
    warn "both greetd.service and inox-greeter@$BOOT_VT.service are enabled: two display managers for one VT"
    note "if Inox is the chain, greetd must be disabled — else the winner depends on ordering, not on intent"
    note "fix: sudo $SCRIPT_DIR/install.sh --enable   (or: sudo $SCRIPT_DIR/rollback-to-greetd.sh)"
fi

# --- verdict ---------------------------------------------------------------

printf '\n[inox-preflight] %d FAIL, %d WARN\n' "$failures" "$warnings"
if [ "$failures" -gt 0 ]; then
    printf '[inox-preflight] NOT READY — fix the FAIL lines above before a trial or a switch.\n'
    exit 1
fi

printf '[inox-preflight] READY — the packaging and the environment are what the chain expects.\n'
printf '[inox-preflight] next: sudo %s/try-inox-greeter.sh   (VT%s, boot chain untouched)\n' "$SCRIPT_DIR" "$TRIAL_VT"
