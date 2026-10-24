#!/usr/bin/env bash
#
# Install the login chain's files — and only its files:
#
#   /etc/pam.d/inox-auth              the PAM stack a login is authenticated through
#   /etc/pam.d/inox-auth-session      the PAM stack the session leader opens the session through
#   /etc/pam.d/inox-greeter           the PAM stack the greeter's own session uses
#   /etc/systemd/system/inox-auth.service
#   /etc/systemd/system/inox-greeter@.service
#   /etc/systemd/system/inox-login-fallback.service
#   the `greeter` system account      only if it does not exist yet
#   getty@tty3.service                enabled and started as the rescue console
#
# Three stacks, not one: the password is checked on inox-auth, the session is opened
# on inox-auth-session (the leader holds no credential, so its auth line permits),
# and inox-greeter is only the greeter's own logind session. Missing any one of them
# makes a login fail in a way that looks like a rejected password (ADR 0037 §决策 5).
#
# The boot chain is NOT touched by default: display-manager.service keeps pointing
# wherever it pointed before, so a machine that boots greetd keeps booting greetd
# (ADR 0037 §决策 6). Only --enable hands display-manager.service to
# inox-greeter@1.service and disables greetd.
#
# Nothing here starts a login screen. Run packaging/try-inox-greeter.sh for that:
# it uses a spare VT and does not touch the boot chain either.
#
# Every step is idempotent — re-running installs the same files again and changes
# nothing else. A pre-existing PAM stack with different content is copied aside
# once (/etc/pam.d/inox-auth.bak-<timestamp>) before it is replaced, because that
# file has opinions about who may log in.
#
# ADR: docs/adr/0037-greeter-host-vt-handoff-and-the-boot-login-chain.md

set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly SELF="$SCRIPT_DIR/$(basename -- "${BASH_SOURCE[0]}")"
readonly PAM_SRC="$SCRIPT_DIR/pam.d"
readonly UNIT_SRC="$SCRIPT_DIR/systemd"

readonly PAM_DIR="/etc/pam.d"
readonly UNIT_DIR="/etc/systemd/system"
readonly DM_LINK="$UNIT_DIR/display-manager.service"

readonly AUTH_UNIT="inox-auth.service"
readonly GREETER_TEMPLATE="inox-greeter@.service"
readonly GREETER_INSTANCE="1"
readonly GREETER_UNIT="inox-greeter@${GREETER_INSTANCE}.service"
readonly RESCUE_UNIT="getty@tty3.service"
readonly TRIAL_VT="4"

# The verification prefix. The units' ExecStart lines and this list must agree; a
# different prefix means editing packaging/systemd/*.service, not this variable.
readonly BIN_DIR="/usr/bin"
readonly BINARIES=(inox-auth inox-host inox-compositor inox-greet inox-session)

log() { printf '[inox-install] %s\n' "$*"; }
warn() { printf '[inox-install] warning: %s\n' "$*" >&2; }

die() {
    printf '[inox-install] error: %s\n' "$*" >&2
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

usage() {
    cat <<EOF
usage: sudo $SELF [--enable]

  (no option)   install the PAM stacks, the systemd units, the greeter account
                and the VT3 rescue console. The boot chain is not touched.
  --enable      do the above, then point display-manager.service at
                $GREETER_UNIT and disable greetd.service.
  -h, --help    print this text.

Nothing is started: use packaging/try-inox-greeter.sh for a trial on VT${TRIAL_VT},
packaging/rollback-to-greetd.sh to give the boot chain back to greetd, and
packaging/uninstall.sh to remove the files.
EOF
}

enable_chain=false
for argument in "$@"; do
    case "$argument" in
        --enable) enable_chain=true ;;
        -h | --help)
            usage
            exit 0
            ;;
        *) die "unrecognized argument: $argument (see --help)" ;;
    esac
done

# --- preflight -------------------------------------------------------------
# Everything checkable from files is checked before the first write, so a refused
# run leaves the machine exactly as it was.

if [ "$(id -u)" -ne 0 ]; then
    die "run as root: sudo $SELF"
fi

for source_file in \
    "$PAM_SRC/inox-auth" \
    "$PAM_SRC/inox-auth-session" \
    "$PAM_SRC/inox-greeter" \
    "$UNIT_SRC/inox-auth.service" \
    "$UNIT_SRC/inox-login-fallback.service" \
    "$UNIT_SRC/$GREETER_TEMPLATE"; do
    [ -f "$source_file" ] || die "missing packaging file: $source_file"
done

[ -d "$PAM_DIR" ] || die "$PAM_DIR does not exist — this is not a PAM-based system"

missing=()
for binary in "${BINARIES[@]}"; do
    if [ ! -x "$BIN_DIR/$binary" ]; then
        missing+=("$BIN_DIR/$binary")
    fi
done
if [ "${#missing[@]}" -gt 0 ]; then
    warn "these binaries are not installed:"
    printf '  %s\n' "${missing[@]}" >&2
    cat >&2 <<'EOF'
The units' ExecStart lines point at /usr/bin, so the binaries have to be there
before the units are. Build them from this checkout and install them:

  cargo build --workspace --release
  sudo install -m 0755 target/release/inox-auth target/release/inox-host \
      target/release/inox-compositor target/release/inox-greet \
      target/release/inox-session /usr/bin/

EOF
    die "refusing to install units that point at binaries which do not exist"
fi

# --- PAM stacks ------------------------------------------------------------

install_pam() {
    local name="$1"
    local target="$PAM_DIR/$name"
    if [ -f "$target" ] && ! cmp -s "$PAM_SRC/$name" "$target"; then
        local backup="$target.bak-$(date +%Y%m%d%H%M%S)"
        cp -a -- "$target" "$backup"
        log "existing $target differs from ours; kept a copy at $backup"
    fi
    install -m 0644 -- "$PAM_SRC/$name" "$target"
    log "installed PAM stack $target"
}

install_pam inox-auth
install_pam inox-auth-session
install_pam inox-greeter

# --- greeter system account -------------------------------------------------
# The boundary admits this account and chowns its socket to the account's group
# (the defaults come from the account database), so the uid is resolved here and
# never written down.

if getent passwd greeter >/dev/null 2>&1; then
    log "the greeter account already exists (uid $(id -u greeter)); leaving it alone"
else
    log "creating the system account 'greeter' (no login shell, home under /var/lib)"
    useradd --system --no-create-home --home-dir /var/lib/inox-greeter \
        --shell /usr/bin/nologin greeter
    log "created the greeter account (uid $(id -u greeter))"
fi

# --- systemd units ---------------------------------------------------------

install -m 0644 -- "$UNIT_SRC/inox-auth.service" "$UNIT_DIR/inox-auth.service"
log "installed $UNIT_DIR/inox-auth.service"

install -m 0644 -- "$UNIT_SRC/$GREETER_TEMPLATE" "$UNIT_DIR/$GREETER_TEMPLATE"
log "installed $UNIT_DIR/$GREETER_TEMPLATE (instance = VT number)"

# Installed, never enabled: it exists to be triggered by OnFailure= on the greeter
# unit, so nothing here calls `systemctl enable` for it.
install -m 0644 -- "$UNIT_SRC/inox-login-fallback.service" "$UNIT_DIR/inox-login-fallback.service"
log "installed $UNIT_DIR/inox-login-fallback.service (triggered by OnFailure=, not enabled)"

systemctl daemon-reload
log "ran systemctl daemon-reload"

# --- rescue console --------------------------------------------------------
# VT3 stays a getty forever: every failure in this chain is fixable from there
# (ADR 0037 §决策 2). rollback-to-greetd.sh and uninstall.sh do not remove it.

systemctl enable --now "$RESCUE_UNIT"
log "$RESCUE_UNIT is enabled and running (Ctrl+Alt+F3 is the rescue console)"

# --- boot chain ------------------------------------------------------------

if [ "$enable_chain" = true ]; then
    log "switching the boot chain to Inox"
    log "  before:   display-manager.service -> $(dm_target)"
    log "  after:    display-manager.service -> $GREETER_UNIT, greetd.service disabled"
    log "  rollback: sudo $SCRIPT_DIR/rollback-to-greetd.sh"
    log "  nothing is started or stopped now; the next boot uses Inox. Use"
    log "  try-inox-greeter.sh first if this chain has never run on this machine."

    systemctl enable "$AUTH_UNIT"
    log "enabled $AUTH_UNIT"

    # Order matters twice over: `systemctl disable greetd` removes greetd's own
    # display-manager.service alias by path, so it runs BEFORE our alias exists.
    # Disabling never touches the running greetd process; the old display manager
    # keeps working until the reboot.
    if systemctl cat greetd.service >/dev/null 2>&1; then
        systemctl disable greetd.service
        log "disabled greetd.service (its display-manager.service alias is gone)"
    else
        warn "greetd.service is not installed; nothing to disable, and inox-login-fallback.service"
        warn "  would skip itself on failure (there is no display manager to fall back to)"
    fi

    ln -sfn "$UNIT_DIR/$GREETER_UNIT" "$DM_LINK"
    systemctl daemon-reload
    log "pointed display-manager.service at $GREETER_UNIT"

    resolved="$(dm_target)"
    if [ "$resolved" != "$GREETER_UNIT" ]; then
        die "display-manager.service resolves to '$resolved', not $GREETER_UNIT — fix by hand: ln -sfn $UNIT_DIR/$GREETER_UNIT $DM_LINK && systemctl daemon-reload"
    fi
    log "verified: display-manager.service resolves to $resolved"
else
    log "the boot chain is unchanged: display-manager.service -> $(dm_target)"
fi

# --- what to do next -------------------------------------------------------

cat <<EOF

[inox-install] done. Three PAM stacks (inox-auth, inox-auth-session, inox-greeter),
the three units, the greeter account and the VT3 rescue console are in place; no login
chain is running yet.

  trial on VT${TRIAL_VT}, boot chain untouched:
    sudo $SCRIPT_DIR/try-inox-greeter.sh

  end the trial:
    sudo $SCRIPT_DIR/try-rollback.sh

  make Inox the display manager:
    sudo $SCRIPT_DIR/install.sh --enable
    sudo reboot

  back to greetd:
    sudo $SCRIPT_DIR/rollback-to-greetd.sh

  remove the files:
    sudo $SCRIPT_DIR/uninstall.sh
EOF
