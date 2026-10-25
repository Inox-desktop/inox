#!/usr/bin/env bash
#
# Build the Inox release tarball — the artifact E4 hardware acceptance is run on.
#
#   packaging/package.sh                    # build, stage, tar, checksum
#   packaging/package.sh --bin-dir DIR      # stage binaries that already exist
#   packaging/package.sh --version 0.1.1-canary.1
#
# What it does, in order:
#
#   1. builds the workspace with `cargo build --release --workspace --locked`
#      — **default features**, not `--all-features`. That distinction is the
#      point of this script: `--all-features` turns on the compositor's
#      `test-input` feature, which opens a socket that accepts injected keyboard
#      and pointer events. A binary carrying that backdoor is not something to
#      hand to a tester.
#   2. copies the shippable binaries, the bundled wallpapers and this packaging
#      directory into a staging tree;
#   3. **asserts the staged compositor rejects `--test-input`**. If that ever
#      stops being true, packaging fails here rather than shipping the backdoor.
#   4. writes BUILD-INFO (commit, toolchain, target, feature set, build time) and
#      SHA256SUMS;
#   5. writes `target/dist/inox-<version>-<target>.tar.gz` and its `.sha256`.
#
# It never touches the network, never needs root, and never installs anything.
# Installing is `packaging/install.sh --bin-dir <extracted>/bin`.
#
# Decision: docs/adr/0042-release-artifacts-and-canary-channel.md.

set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly SELF="$SCRIPT_DIR/$(basename -- "${BASH_SOURCE[0]}")"
readonly REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"

# Every binary that ends up in `bin/`. Split by who needs it, because the two
# groups fail in very different ways when one is missing:
#
# - RELEASE_BINARIES: the login chain plus everything `inox-session` starts as
#   `required`. Without one of these a correct password turns into a session that
#   ends immediately, which looks exactly like a rejected password.
# - SESSION_BINARIES: started but not required. The session survives without
#   them; the desktop is just missing a piece.
#
# `inox-integration` (test aggregator) and `inox-qml-prototype` (the cxx-qt
# bridge prototype) have binaries too, and neither is shipped: one is a test
# harness, the other exists to prove the binding works.
readonly RELEASE_BINARIES=(
    inox-auth
    inox-host
    inox-compositor
    inox-greet
    inox-session
    inox-shell
    inox-panel
    inox-lock
)
readonly SESSION_BINARIES=(
    inox-wallpaper
    inox-notifications
    inox-idle
    inox-xwayland
    inox-launcher
    inox-settings
)
readonly SHIPPED_BINARIES=("${RELEASE_BINARIES[@]}" "${SESSION_BINARIES[@]}")

log() { printf '[inox-package] %s\n' "$*"; }
warn() { printf '[inox-package] warning: %s\n' "$*" >&2; }
die() {
    printf '[inox-package] error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<EOF
usage: $SELF [options]

Build the release tarball. The artifact lands in target/dist/ by default.

  --bin-dir DIR       stage binaries from DIR instead of building them. Use this
                      on a machine that already ran
                      \`cargo build --release --workspace\`, or to re-package a CI
                      artifact.
  --output-dir DIR    where the tarball goes [default: <repo>/target/dist]
  --version V         version to stamp [default: the workspace version in Cargo.toml]
  --target TRIPLE     target triple to label the artifact with
                      [default: the host triple from rustc]
  --no-verify         skip the assertion that the staged compositor rejects
                      --test-input. Only for debugging this script; CI never
                      passes it.
  -h, --help          print this text.

What is in the tarball:

  bin/                 the ${#SHIPPED_BINARIES[@]} shipped binaries
  data/wallpapers/     the bundled wallpapers
  packaging/           the install surface (install.sh, preflight.sh, pam.d, systemd)
  BUILD-INFO           commit, toolchain, target, feature set, build time
  SHA256SUMS           checksums of everything above
EOF
}

bin_dir=""
output_dir="$REPO_ROOT/target/dist"
version=""
target_triple=""
verify=true

while [ "$#" -gt 0 ]; do
    case "$1" in
        --bin-dir)
            [ "$#" -ge 2 ] || die "--bin-dir needs a value"
            bin_dir="$2"
            shift 2
            ;;
        --output-dir)
            [ "$#" -ge 2 ] || die "--output-dir needs a value"
            output_dir="$2"
            shift 2
            ;;
        --version)
            [ "$#" -ge 2 ] || die "--version needs a value"
            version="$2"
            shift 2
            ;;
        --target)
            [ "$#" -ge 2 ] || die "--target needs a value"
            target_triple="$2"
            shift 2
            ;;
        --no-verify) verify=false; shift ;;
        -h | --help)
            usage
            exit 0
            ;;
        *) die "unrecognized argument: $1 (see --help)" ;;
    esac
done

command -v cargo >/dev/null 2>&1 || die "cargo is not on PATH"

# --- version and target ----------------------------------------------------

if [ -z "$version" ]; then
    # Read it from the manifest rather than `cargo metadata`: the workspace table
    # is the single source of truth and this keeps the script usable without a
    # Cargo.lock present.
    version="$(sed -n '/^\[workspace\.package\]/,/^\[/p' "$REPO_ROOT/Cargo.toml" \
        | sed -n 's/^version *= *"\(.*\)"/\1/p' | head -1)"
    [ -n "$version" ] || die "could not read [workspace.package] version from Cargo.toml"
fi
if [ -z "$target_triple" ]; then
    target_triple="$(rustc -vV | sed -n 's/^host: //p')"
    [ -n "$target_triple" ] || die "could not determine the host target triple"
fi

readonly STAGE_NAME="inox-${version}-${target_triple}"

# --- build -----------------------------------------------------------------

if [ -z "$bin_dir" ]; then
    log "building the workspace with default features (release)"
    log "  not --all-features: that enables inox-compositor/test-input, which is a"
    log "  socket accepting injected input and must not reach a tester"
    # `--locked` so the artifact is built from the committed Cargo.lock; the lock
    # file is the compatibility record (acceptance.md L01).
    (cd -- "$REPO_ROOT" && cargo build --release --workspace --locked)
    bin_dir="$REPO_ROOT/target/release"
else
    log "staging binaries from $bin_dir (no build)"
fi

[ -d "$bin_dir" ] || die "no such directory: $bin_dir"

missing=()
for binary in "${SHIPPED_BINARIES[@]}"; do
    [ -x "$bin_dir/$binary" ] || missing+=("$binary")
done
if [ "${#missing[@]}" -gt 0 ]; then
    die "missing from $bin_dir: ${missing[*]}
  run: cargo build --release --workspace
  or point --bin-dir at the directory that already has them"
fi

# --- stage -----------------------------------------------------------------

staging="$(mktemp -d)"
trap 'rm -rf -- "$staging"' EXIT
readonly root="$staging/$STAGE_NAME"

mkdir -p -- "$root/bin" "$root/data" "$root/packaging"

log "staging into $root"

for binary in "${SHIPPED_BINARIES[@]}"; do
    install -m 0755 -- "$bin_dir/$binary" "$root/bin/$binary"
done

# Strip the *copies*, not the build tree: `target/release` keeps its symbols for
# local debugging, the artifact does not carry them around.
#
# Whether this worked is recorded in BUILD-INFO rather than assumed. A binary that
# did not get stripped is not a failure — but an artifact that silently claims to
# be stripped when it is not would mislead whoever debugs a tester's crash report.
if command -v strip >/dev/null 2>&1; then
    stripped_count=0
    for binary in "${SHIPPED_BINARIES[@]}"; do
        if strip --strip-unneeded "$root/bin/$binary" 2>/dev/null; then
            stripped_count=$((stripped_count + 1))
        else
            warn "could not strip $binary (left with its symbols)"
        fi
    done
    if [ "$stripped_count" -eq "${#SHIPPED_BINARIES[@]}" ]; then
        stripped_state="all ${#SHIPPED_BINARIES[@]} binaries"
        log "stripped all ${#SHIPPED_BINARIES[@]} staged binaries"
    else
        stripped_state="only $stripped_count of ${#SHIPPED_BINARIES[@]} (strip failed on the rest)"
        warn "only $stripped_count of ${#SHIPPED_BINARIES[@]} binaries were stripped; BUILD-INFO records this"
    fi
else
    stripped_state="no: strip is not installed"
    warn "strip is not available; the artifact carries its symbols"
fi

# The wallpapers are data, not build output: `inox-wallpaper` and
# `inox-settings` look them up under <datadir>/inox/wallpapers, and a tarball
# without them installs a desktop whose wallpaper chooser is empty.
if [ -d "$REPO_ROOT/data/wallpapers" ]; then
    cp -a -- "$REPO_ROOT/data/wallpapers" "$root/data/wallpapers"
    log "bundled $(find "$root/data/wallpapers" -type f | wc -l) wallpapers"
else
    warn "data/wallpapers is missing; the artifact ships no wallpapers"
fi

# The whole packaging directory, because install.sh resolves pam.d/ and systemd/
# relative to itself.
# `package.sh` ships with the tarball even though it is not needed to install one:
# the release notes tell a tester to read it, and somebody who wants to know what
# the artifact is made of should be able to read the script that made it without
# cloning the repository.
cp -a -- "$SCRIPT_DIR/install.sh" "$SCRIPT_DIR/uninstall.sh" \
    "$SCRIPT_DIR/preflight.sh" "$SCRIPT_DIR/verify-login.sh" \
    "$SCRIPT_DIR/try-inox-greeter.sh" "$SCRIPT_DIR/try-rollback.sh" \
    "$SCRIPT_DIR/rollback-to-greetd.sh" "$SCRIPT_DIR/rehearse-headless.sh" \
    "$SCRIPT_DIR/package.sh" "$SCRIPT_DIR/README.md" "$root/packaging/"
cp -a -- "$SCRIPT_DIR/pam.d" "$SCRIPT_DIR/systemd" "$root/packaging/"

# --- verify the artifact is not a test build -------------------------------
#
# This is the assertion that makes the default-features build meaningful. The
# binary a tester runs must reject `--test-input`; a build that accepts it can be
# talked into injecting keyboard and pointer events from another process.

if [ "$verify" = true ]; then
    log "verifying bin/inox-compositor rejects --test-input"
    probe_stderr="$staging/test-input-probe.stderr"

    # The timeout is load-bearing. A binary built *with* the feature does not
    # reject the option — it starts the compositor with an input-injection socket
    # and runs its event loop, which on a machine with a GPU would never return.
    # Without the timeout this check would hang the job instead of failing it,
    # which is the worse of the two outcomes.
    probe_status=0
    timeout 30 "$root/bin/inox-compositor" --test-input "$staging/should-not-exist.sock" \
        2>"$probe_stderr" >/dev/null || probe_status=$?

    case "$probe_status" in
        0)
            die "the staged inox-compositor accepted --test-input: this artifact was built with the test-input feature and must not be shipped"
            ;;
        124)
            die "the staged inox-compositor did not refuse --test-input within 30 s; it started instead, so this is a test-input build and must not be shipped"
            ;;
    esac

    # The rejection has to *name* the option. A binary that failed for some other
    # reason (a missing GPU, a bad socket path) is not proof of a default-features
    # build, and accepting it would make this check vacuous.
    if ! grep -q "test-input" "$probe_stderr"; then
        die "inox-compositor refused --test-input but did not say why; expected an error naming it, got:
$(cat "$probe_stderr")"
    fi
    log "ok: --test-input is rejected, this is a default-features build"
fi

# --- build metadata --------------------------------------------------------

commit="$(cd -- "$REPO_ROOT" && git rev-parse HEAD 2>/dev/null || printf 'unknown')"
commit_short="$(cd -- "$REPO_ROOT" && git rev-parse --short=12 HEAD 2>/dev/null || printf 'unknown')"
commit_date="$(cd -- "$REPO_ROOT" && git show -s --format=%cI HEAD 2>/dev/null || printf 'unknown')"
# `git status --porcelain` ignores whatever .gitignore covers, so build output
# under target/ does not make this dirty. Anything it *does* report is a tracked
# file modified or an untracked file that is not ignored — both mean the artifact
# does not correspond to the commit recorded below, which is the one thing
# BUILD-INFO exists to be trusted for.
worktree_state="clean"
worktree_dirty_list=""
if [ -n "$(cd -- "$REPO_ROOT" && git rev-parse --git-dir 2>/dev/null)" ]; then
    worktree_dirty_list="$(cd -- "$REPO_ROOT" && git status --porcelain 2>/dev/null)"
    if [ -n "$worktree_dirty_list" ]; then
        worktree_state="dirty"
    fi
else
    # No git at all (a tarball of a tarball, a CI checkout without history): the
    # commit is unknown rather than clean, and saying so is better than claiming a
    # cleanliness nobody checked.
    worktree_state="unknown: not a git worktree"
fi
rustc_version="$(rustc -V)"
cargo_version="$(cargo -V)"
built_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

cat >"$root/BUILD-INFO" <<EOF
name: inox
version: $version
stage: $STAGE_NAME
target: $target_triple
commit: $commit
commit-short: $commit_short
commit-date: $commit_date
worktree: $worktree_state
features: default
profile: release
stripped: $stripped_state
rustc: $rustc_version
cargo: $cargo_version
built-at: $built_at
builder: packaging/package.sh
license: LGPL-3.0
EOF

# A dirty worktree means the artifact does not correspond to any commit. That is
# useful for a local experiment and unacceptable for something a tester reports
# results against, so say it loudly — but do not refuse, because packaging a
# local build is a legitimate thing to want.
if [ "$worktree_state" = dirty ]; then
    warn "the worktree has uncommitted changes; BUILD-INFO records commit $commit_short but the binaries are not that commit"
    # Naming the files is what makes this actionable in a CI log: a warning that
    # only says "dirty" leaves whoever reads it to guess.
    printf '%s\n' "$worktree_dirty_list" | while IFS= read -r line; do
        [ -n "$line" ] && warn "  uncommitted: $line"
    done
fi

# --- checksums -------------------------------------------------------------

log "writing SHA256SUMS"
# Written to a scratch file first and moved into place: the list is produced by
# walking the tree it documents, so creating it in that tree before the walk
# finishes would make the checksum file describe itself.
checksums="$staging/SHA256SUMS"
(cd -- "$root" && find . -type f -printf '%P\n' | LC_ALL=C sort \
    | xargs -r -d '\n' sha256sum) >"$checksums"
mv -- "$checksums" "$root/SHA256SUMS"

# --- tarball ---------------------------------------------------------------

mkdir -p -- "$output_dir"
tarball="$output_dir/$STAGE_NAME.tar.gz"

log "writing $tarball"
# --sort=name and a fixed mtime keep two runs over identical inputs close to
# byte-identical; `--owner=0 --group=0` keeps the packaging user out of the
# archive. Reproducibility is not a gate here — but a diffable artifact is.
tar --create --gzip \
    --directory "$staging" \
    --sort=name \
    --owner=0 --group=0 --numeric-owner \
    --mtime="@${SOURCE_DATE_EPOCH:-$(date -u +%s)}" \
    "$STAGE_NAME" >"$tarball"

(cd -- "$output_dir" && sha256sum "$(basename -- "$tarball")" >"$(basename -- "$tarball").sha256")

size="$(du -h "$tarball" | cut -f1)"
log "done: $tarball ($size)"
log "checksum: $(cut -d' ' -f1 "$tarball.sha256")"
cat <<EOF

[inox-package] install it with:

  tar -xzf $(basename -- "$tarball")
  cd $STAGE_NAME
  sudo ./packaging/install.sh --bin-dir ./bin

  then read BUILD-INFO, and run sudo ./packaging/preflight.sh before switching
  the screen. Real-machine acceptance: docs/book/src/usage/login.md.

EOF
