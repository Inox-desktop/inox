#!/usr/bin/env bash
#
# Rehearse the greeter stage with no VT, no root, no systemd and no /etc:
# a private XDG_RUNTIME_DIR, a stand-in authentication boundary that only listens,
# the real inox-host, the real inox-compositor (headless, llvmpipe) and the real
# inox-greet. Everything lives in one mktemp -d and is gone afterwards.
#
# This is the check to run before touching any VT or config: it answers "does the
# host orchestrate, does the Wayland handshake complete, does the login screen
# actually draw a frame". It cannot answer anything about DRM, seats, VT handoff,
# PAM or session start — those need the real machine (S07/S08/S09, E4).
#
# Usage:
#   packaging/rehearse-headless.sh [--bin-dir DIR] [--seconds N]
#
#   --bin-dir DIR   where inox-host/inox-compositor/inox-greet live
#                   (default: <repo>/target/debug)
#   --seconds N     how long the chain must stay healthy before the verdict
#                   (default: 12)
#
# Exits 0 only when every assertion passed AND nothing was left running.
#
# Why root is refused: a rehearsal that runs as root proves less, not more — it
# can silently pick up /etc, the system bus or a real seat, which is exactly what
# this check is meant to leave out of the picture. The install path is
# packaging/install.sh; this is not it.

set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
readonly SELF="$SCRIPT_DIR/$(basename -- "${BASH_SOURCE[0]}")"

# What the greeter stage needs from the build tree. inox-session and inox-auth are
# deliberately absent: this rehearsal starts no session and has no PAM.
readonly BINARIES=(inox-host inox-compositor inox-greet)

bin_dir="$REPO_ROOT/target/debug"
seconds=12

log() { printf '[inox-rehearse] %s\n' "$*"; }
warn() { printf '[inox-rehearse] warning: %s\n' "$*" >&2; }

die() {
    printf '[inox-rehearse] error: %s\n' "$*" >&2
    exit 1
}

usage() {
    sed -n '/^# Usage:/,/^# Exits 0/p' "$SELF" | sed 's/^# \{0,1\}//'
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --bin-dir)
            [ "$#" -ge 2 ] || die "--bin-dir needs a value"
            bin_dir="$2"
            shift 2
            ;;
        --seconds)
            [ "$#" -ge 2 ] || die "--seconds needs a value"
            seconds="$2"
            shift 2
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            die "unrecognized argument: $1 (see --help)"
            ;;
    esac
done

case "$seconds" in
    '' | *[!0-9]*) die "--seconds must be a whole number, not '$seconds'" ;;
esac
[ "$seconds" -ge 3 ] || die "--seconds must be at least 3, not $seconds"

# --- preflight -------------------------------------------------------------

if [ "${EUID:-$(id -u)}" -eq 0 ]; then
    die "run this as your own user, not as root: this is a rehearsal, not an install.
       Root can reach /etc, the system bus and a real seat, which is precisely what
       this check leaves out. For the deployment, use packaging/install.sh."
fi

# Resolve a relative --bin-dir against the repository root, so the documented
# default keeps working when the script is called from anywhere.
case "$bin_dir" in
    /*) ;;
    *) bin_dir="$REPO_ROOT/$bin_dir" ;;
esac

missing=()
for binary in "${BINARIES[@]}"; do
    if [ ! -x "$bin_dir/$binary" ]; then
        missing+=("$bin_dir/$binary")
    fi
done
if [ "${#missing[@]}" -gt 0 ]; then
    warn "missing binaries:"
    printf '  %s\n' "${missing[@]}" >&2
    die "build them first:
       cargo build -p inox-host -p inox-compositor -p inox-greet
       (or pass --bin-dir with the directory that already has them)"
fi

command -v python3 >/dev/null 2>&1 ||
    die "python3 is not installed, and this script needs it for the stand-in authentication boundary (no nc/socat substitute: their syntax differs between versions)."

# --- private playground ----------------------------------------------------

work="$(mktemp -d "${TMPDIR:-/tmp}/inox-rehearse.XXXXXX")"
runtime="$work/run"
auth_socket="$work/auth.sock"
host_log="$work/host.log"
boundary_log="$work/boundary.log"
# A name unique to this run inside a directory unique to this run: two rehearsals
# cannot collide, and neither can collide with a real session's socket.
socket_name="wayland-inox-rehearse-$$"

mkdir -p "$runtime" "$work/config" "$work/data" "$work/state" "$work/cache"
chmod 700 "$runtime"

host_pid=""
boundary_pid=""
failures=()

cleanup_done=false

# Processes started by this run: the host and its children carry this run's private
# runtime directory in their environment. Matching on that is exact and never
# touches another Inox process on the machine (no pkill -f inox anywhere here).
processes_of_this_run() {
    local pid
    for pid in /proc/[0-9]*; do
        pid="${pid#/proc/}"
        [ "$pid" = "$$" ] && continue
        if tr '\0' '\n' <"/proc/$pid/environ" 2>/dev/null |
            grep -qxF "XDG_RUNTIME_DIR=$runtime"; then
            printf '%s ' "$pid"
        fi
    done
}

cleanup() {
    if [ "$cleanup_done" = true ]; then
        return
    fi
    cleanup_done=true

    if [ -n "$host_pid" ] && kill -0 "$host_pid" 2>/dev/null; then
        kill -TERM "$host_pid" 2>/dev/null || true
        for _ in $(seq 1 100); do
            kill -0 "$host_pid" 2>/dev/null || break
            sleep 0.1
        done
        if kill -0 "$host_pid" 2>/dev/null; then
            warn "the host did not exit within 10 s of SIGTERM; sending SIGKILL"
            kill -KILL "$host_pid" 2>/dev/null || true
        fi
    fi

    if [ -n "$boundary_pid" ] && kill -0 "$boundary_pid" 2>/dev/null; then
        kill -TERM "$boundary_pid" 2>/dev/null || true
        for _ in $(seq 1 30); do
            kill -0 "$boundary_pid" 2>/dev/null || break
            sleep 0.1
        done
        kill -KILL "$boundary_pid" 2>/dev/null || true
    fi

    # Anything still carrying this run's runtime directory is a leftover: the host
    # is supposed to take its children with it, so this is a failure, not tidying.
    local leftovers
    leftovers="$(processes_of_this_run)"
    if [ -n "$leftovers" ]; then
        warn "leftover processes from this run: $leftovers"
        for pid in $leftovers; do
            kill -TERM "$pid" 2>/dev/null || true
        done
        sleep 0.5
        for pid in $(processes_of_this_run); do
            kill -KILL "$pid" 2>/dev/null || true
        done
    fi

    rm -rf -- "$work"
}

trap cleanup EXIT

log "runtime dir: $runtime"
log "wayland socket: $socket_name"
log "boundary: $auth_socket (stand-in: accepts and closes, speaks no protocol)"

# --- stand-in boundary -----------------------------------------------------

cat >"$work/boundary.py" <<'PYTHON'
import socket
import sys

path = sys.argv[1]
listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
listener.bind(path)
listener.listen(16)
while True:
    connection, _ = listener.accept()
    connection.close()
PYTHON

python3 -u "$work/boundary.py" "$auth_socket" >"$boundary_log" 2>&1 &
boundary_pid=$!

for _ in $(seq 1 50); do
    [ -S "$auth_socket" ] && break
    kill -0 "$boundary_pid" 2>/dev/null || die "the stand-in boundary exited immediately; see $boundary_log"
    sleep 0.1
done
[ -S "$auth_socket" ] || die "the stand-in boundary did not create $auth_socket within 5 s"
log "stand-in boundary is listening"

# --- run the greeter stage -------------------------------------------------

# WAYLAND_DISPLAY/DISPLAY/WAYLAND_SOCKET are cleared for the host so a rehearsal
# started from a graphical session cannot be steered at that session's compositor.
env \
    -u WAYLAND_DISPLAY -u WAYLAND_SOCKET -u DISPLAY \
    LIBGL_ALWAYS_SOFTWARE=1 \
    XDG_RUNTIME_DIR="$runtime" \
    XDG_CONFIG_HOME="$work/config" \
    XDG_DATA_HOME="$work/data" \
    XDG_STATE_HOME="$work/state" \
    XDG_CACHE_HOME="$work/cache" \
    INOX_AUTH_SOCKET="$auth_socket" \
    "$bin_dir/inox-host" \
    --bin-dir "$bin_dir" \
    --socket "$socket_name" \
    --compositor-arg --headless \
    >"$host_log" 2>&1 &
host_pid=$!

log "host pid $host_pid; watching it for ${seconds}s"

deadline=$((SECONDS + seconds))
host_died_early=false
while [ "$SECONDS" -lt "$deadline" ]; do
    if ! kill -0 "$host_pid" 2>/dev/null; then
        host_died_early=true
        break
    fi
    sleep 0.25
done

# --- assertions ------------------------------------------------------------

# Each assertion records its own failure so one run reports every problem at once
# instead of the first one.
assert_grep() {
    local label="$1" pattern="$2" log_file="$3" line
    if line="$(grep -E -m1 -- "$pattern" "$log_file" 2>/dev/null)"; then
        printf '  ok    %s\n' "$label"
        printf '        %s\n' "${line:0:160}"
    else
        printf '  FAIL  %s (no line matching "%s" in %s)\n' "$label" "$pattern" "$(basename -- "$log_file")"
        failures+=("$label")
    fi
}

printf '[inox-rehearse] assertions:\n'

if [ -S "$runtime/$socket_name" ]; then
    printf '  ok    compositor bound the Wayland socket (%s)\n' "$runtime/$socket_name"
else
    printf '  FAIL  no Wayland socket at %s\n' "$runtime/$socket_name"
    failures+=("wayland socket")
fi

assert_grep "compositor reports a bound socket" "inox-compositor ready|Wayland socket bound" "$host_log"
assert_grep "login screen found the authentication boundary" "found its authentication boundary" "$host_log"
assert_grep "login screen read the system accounts" "accounts were read" "$host_log"
assert_grep "login screen mapped a toplevel window" "mapped toplevel" "$host_log"

# The host starts the compositor first and the login screen once the socket is up,
# so two lines here mean both halves of the stage were launched by this host (not,
# say, a login screen that found a boundary somebody else had already started).
started_children="$(grep -c 'started child process' "$host_log" 2>/dev/null || true)"
if [ "${started_children:-0}" -ge 2 ]; then
    printf '  ok    host started both children (%s processes)\n' "$started_children"
else
    printf '  FAIL  host started %s child process(es), expected 2\n' "${started_children:-0}"
    failures+=("host started both children")
fi

if [ "$host_died_early" = true ]; then
    printf '  FAIL  the host exited before the deadline\n'
    failures+=("host alive at deadline")
elif kill -0 "$host_pid" 2>/dev/null; then
    printf '  ok    host still alive after %ss\n' "$seconds"
else
    printf '  FAIL  the host is no longer running\n'
    failures+=("host alive at deadline")
fi

# --- stop it and check nothing is left -------------------------------------

host_exit=""
if ! kill -0 "$host_pid" 2>/dev/null; then
    # It exited on its own; that is already reported above. Its status was not a
    # successful SIGTERM shutdown, so there is nothing to assert here.
    printf '  n/a   the host had already exited; nothing to signal\n'
    host_pid=""
elif kill -TERM "$host_pid" 2>/dev/null; then
    log "sent SIGTERM to the host"
    for _ in $(seq 1 100); do
        kill -0 "$host_pid" 2>/dev/null || break
        sleep 0.1
    done
    if kill -0 "$host_pid" 2>/dev/null; then
        printf '  FAIL  the host did not exit within 10 s of SIGTERM\n'
        failures+=("host exited on SIGTERM")
    else
        wait "$host_pid" || host_exit=$?
        host_exit="${host_exit:-0}"
        if [ "$host_exit" -eq 0 ]; then
            printf '  ok    host exited 0 on SIGTERM\n'
        else
            printf '  FAIL  host exited %s on SIGTERM (expected 0)\n' "$host_exit"
            failures+=("host exit status")
        fi
    fi
else
    printf '  FAIL  could not signal the host\n'
    failures+=("host signal")
fi
host_pid=""

# The children are in their own process groups, so give them a moment to be reaped
# before deciding something was orphaned.
leftovers=""
for _ in $(seq 1 30); do
    leftovers="$(processes_of_this_run)"
    [ -z "$leftovers" ] && break
    sleep 0.1
done
if [ -z "$leftovers" ]; then
    printf '  ok    no processes left behind\n'
else
    printf '  FAIL  processes still running after the host exited: %s\n' "$leftovers"
    failures+=("no leftovers")
fi

# --- verdict ---------------------------------------------------------------

if [ "${#failures[@]}" -eq 0 ]; then
    printf '\n[inox-rehearse] PASS — host orchestration, the Wayland handshake and the login screen rendering all work\n'
    printf '[inox-rehearse] note: this is E2 rehearsal evidence, not a login. DRM/seat, VT handoff, PAM and\n'
    printf '[inox-rehearse]       session start are not exercised here; those are S07/S08/S09 on the machine.\n'
    exit 0
fi

printf '\n[inox-rehearse] FAIL (%d problem(s)): %s\n' "${#failures[@]}" "${failures[*]}"
printf '[inox-rehearse] host log (last 60 lines) follows:\n'
tail -n 60 "$host_log" 2>/dev/null || true
printf '[inox-rehearse] boundary log (last 10 lines) follows:\n'
tail -n 10 "$boundary_log" 2>/dev/null || true
exit 1
