#!/usr/bin/env bash
#
# Render the Inox desktop home — the top menu bar and the bottom dock — with the
# real compositor and the real panel binaries, and save one composited frame as a
# PNG. This is the "what does my desktop look like right now" check: no GPU, no
# DRM device, no session, no display.
#
# How it works: `inox-compositor --headless` creates one offscreen output and
# advertises `zwlr_layer_shell_v1`; two `inox-panel` processes (one per surface —
# a layer surface belongs to one client connection) place themselves on that
# output; the compositor's test-only control channel is then asked to dump the
# frame it just composited. That channel is the `test-input` feature, which is off
# by default and is not a Wayland global; this script only uses it to read back a
# frame, exactly as `crates/inox-compositor/tests/session_lock.rs` and the CI
# end-to-end job do.
#
# Build first (the compositor needs its test-only control channel):
#
#   cargo build -p inox-compositor --features test-input
#   cargo build -p inox-panel
#
# Usage:
#
#   scripts/desktop-home-smoke.sh [output.png] [WIDTHxHEIGHT]
#
# Everything it starts is killed on exit, and its runtime directory is private.

set -euo pipefail

output="${1:-target/ui-redesign/inox-compositor/inox-desktop-home.png}"
size="${2:-1280x720}"

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
compositor="$repo_root/target/debug/inox-compositor"
panel="$repo_root/target/debug/inox-panel"

for binary in "$compositor" "$panel"; do
    if [ ! -x "$binary" ]; then
        echo "missing $binary — build it first:" >&2
        echo "  cargo build -p inox-compositor --features test-input" >&2
        echo "  cargo build -p inox-panel" >&2
        exit 1
    fi
done

if ! python3 -c 'import PIL' >/dev/null 2>&1; then
    echo "python3 with Pillow is needed to turn the dumped PPM into a PNG" >&2
    exit 1
fi

runtime=$(mktemp -d "${TMPDIR:-/tmp}/inox-home.XXXXXX")
config=$(mktemp -d "${TMPDIR:-/tmp}/inox-home-config.XXXXXX")
chmod 700 "$runtime"

# The socket name is relative to XDG_RUNTIME_DIR, which is why that directory has
# to be the private one: a stale session's socket must never be touched.
socket="wayland-inox-home"
export XDG_RUNTIME_DIR="$runtime"
export XDG_CONFIG_HOME="$config"
export WAYLAND_DISPLAY="$socket"
# An empty XDG_CONFIG_HOME means built-in defaults: the dark theme and its
# `[glass]` tokens, `[panel] surface` ignored because both surfaces are named
# explicitly below. Set XDG_CONFIG_HOME yourself to try a configuration.
export RUST_LOG="${RUST_LOG:-info}"
export NO_COLOR=1
export LIBGL_ALWAYS_SOFTWARE=1

pids=()
cleanup() {
    for pid in "${pids[@]:-}"; do
        kill "$pid" 2>/dev/null || true
    done
    sleep 0.2
    for pid in "${pids[@]:-}"; do
        kill -9 "$pid" 2>/dev/null || true
    done
    rm -rf "$runtime" "$config"
}
trap cleanup EXIT

"$compositor" --headless --socket "$socket" \
    --test-input "$runtime/test-input.sock" --output-size "$size" \
    >"$runtime/compositor.log" 2>&1 &
pids+=($!)

for _ in $(seq 1 100); do
    [ -S "$runtime/$socket" ] && break
    sleep 0.1
done
if [ ! -S "$runtime/$socket" ]; then
    echo "the compositor never created $runtime/$socket:" >&2
    tail -5 "$runtime/compositor.log" >&2
    exit 1
fi

"$panel" --surface menubar >"$runtime/menubar.log" 2>&1 &
pids+=($!)
"$panel" --surface dock >"$runtime/dock.log" 2>&1 &
pids+=($!)

# Let both surfaces be configured, committed and composited. There is no event to
# wait for across processes; the panel logs when its surface is up, so wait for
# that and then for a few frames.
for _ in $(seq 1 100); do
    if grep -q "running as a wlr-layer-shell surface" "$runtime/menubar.log" 2>/dev/null &&
        grep -q "running as a wlr-layer-shell surface" "$runtime/dock.log" 2>/dev/null; then
        break
    fi
    sleep 0.1
done
sleep 1

frame="$runtime/frame.ppm"
python3 - "$runtime/test-input.sock" "$frame" <<'PY'
import socket, sys

command = "dump-frame %s\n" % sys.argv[2]
with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as control:
    control.connect(sys.argv[1])
    control.sendall(command.encode())
    control.shutdown(socket.SHUT_WR)
    print(control.recv(200).decode(errors="replace").strip())
PY

mkdir -p "$(dirname -- "$output")"
python3 - "$frame" "$output" <<'PY'
import sys
from PIL import Image

header, body = open(sys.argv[1], "rb").read().split(b"\n255\n", 1)
fields = header.split()
width, height = int(fields[1]), int(fields[2])
Image.frombytes("RGB", (width, height), body[: width * height * 3]).save(sys.argv[2])
print("wrote %s (%dx%d)" % (sys.argv[2], width, height))
PY

echo "menu bar: $(grep -m1 'created the panel' "$runtime/menubar.log" || true)"
echo "dock:     $(grep -m1 'created the panel' "$runtime/dock.log" || true)"
