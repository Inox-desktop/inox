#!/usr/bin/env bash
#
# Publish one Inox release to the AUR as a source package.
#
#   packaging/aur/publish.sh --version 0.1.1-canary.2 --build --push
#   packaging/aur/publish.sh --version 0.1.1 --sha256 SKIP        # recipe check
#   packaging/aur/publish.sh --version 0.1.1 --local-source . --build
#
# Two AUR packages, one recipe (decision: docs/adr/0043):
#
#   version without a pre-release suffix  -> inox
#   version with -canary/-alpha/-beta/-rc/-dev -> inox-canary
#
# `--version` is the upstream version, the same string the Release workflow puts
# in the tag after the `v`. The source archive is the GitHub tag archive, so the
# recipe builds from the tagged tree, not from the release tarball: that tarball
# is linked against the Ubuntu runner's glibc/Qt and would be a bad Arch package.
#
# What this script does, in order:
#
#   1. derives the AUR package name and pkgver from the version. AUR forbids
#      hyphens in pkgver, so `0.1.1-canary.2` ships as `0.1.1_canary.2` while the
#      source URL still names the original tag;
#   2. renders packaging/aur/PKGBUILD.in;
#   3. generates .SRCINFO with `makepkg --printsrcinfo` (never written by hand:
#      a hand-written .SRCINFO is a second source of truth that drifts);
#   4. checks that every depends/makedepends resolves in the configured pacman
#      repositories;
#   5. with --build: runs `makepkg` in a scratch directory, which is the only
#      proof that the recipe works on Arch. The build runs `check()`, which
#      asserts the compositor rejects `--test-input`, and then the package's file
#      list is verified against the expected 14 binaries plus the install surface;
#   6. with --push: pushes PKGBUILD, .SRCINFO and inox.install to the AUR
#      repository over git+ssh. Pushing the same version again is a no-op.
#
# It never runs `git push` unless --push is given. `--sha256 SKIP` and
# `--local-source` are development/rehearsal modes and are refused together with
# --push: a published PKGBUILD must pin a real remote archive.
#
# Environment used by --push:
#   GIT_SSH_COMMAND   the transport (e.g. `ssh -i ~/.ssh/aur`); the workflow sets it
#   AUR_GIT_NAME      commit author name  [default: Inox release bot]
#   AUR_GIT_EMAIL     commit author email [default: release@inox.invalid]
#
# Decision: docs/adr/0043-aur-source-packages.md.

set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly TEMPLATE="$SCRIPT_DIR/PKGBUILD.in"
readonly INSTALL_FILE="$SCRIPT_DIR/inox.install"

# The binaries a desktop install has to carry. inox-integration (test aggregator)
# and inox-qml-prototype (binding prototype) are built by the workspace build but
# deliberately not shipped.
readonly SHIPPED_BINARIES=(
    inox-auth
    inox-host
    inox-compositor
    inox-greet
    inox-session
    inox-shell
    inox-panel
    inox-lock
    inox-wallpaper
    inox-notifications
    inox-idle
    inox-xwayland
    inox-launcher
    inox-settings
)

log() { printf '[inox-aur] %s\n' "$*"; }
warn() { printf '[inox-aur] warning: %s\n' "$*" >&2; }
die() {
    printf '[inox-aur] error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<EOF
usage: $0 --version V [options]

Render the AUR package for one Inox release, optionally build it and push it to
the AUR. Nothing is pushed unless --push is passed.

  --version V         upstream version, e.g. 0.1.1 or 0.1.1-canary.2 (required)
  --upstream-tag TAG  git tag the source archive comes from [default: v<V>]
  --source-url URL    override the source archive URL [default: the GitHub tag
                      archive of --upstream-tag]
  --sha256 SUM        checksum of the source archive; SKIP skips verification
                      (recipe checks only). Without this the script downloads the
                      archive and computes the checksum itself.
  --local-source DIR  create the source archive from DIR's committed HEAD
                      (git archive) instead of downloading a tag archive. For
                      rehearsal only; refused with --push.
  --pkgrel N          pkgrel [default: 1]
  --repo-url URL      [default: https://github.com/Inox-desktop/inox]
  --aur-url URL       git base URL of the AUR [default: ssh://aur@aur.archlinux.org]
  --build             run makepkg and verify the package contents
  --no-syncdeps       do not install missing build dependencies with pacman
                      (use when they are already installed, e.g. on a dev box)
  --push              commit and push PKGBUILD/.SRCINFO/inox.install to the AUR
  --keep              keep the scratch directory and print its path
  -h, --help          print this text.

Package name mapping (AUR forbids '-' in pkgver, hence '_'):

  0.1.1               -> inox          pkgver 0.1.1
  0.1.1-canary.2      -> inox-canary   pkgver 0.1.1_canary.2
EOF
}

version=""
upstream_tag=""
source_url=""
sha256=""
local_source=""
pkgrel=1
repo_url="https://github.com/Inox-desktop/inox"
aur_url="ssh://aur@aur.archlinux.org"
do_build=false
do_push=false
syncdeps=true
keep=false

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || die "--version needs a value"
            version="$2"
            shift 2
            ;;
        --upstream-tag)
            [ "$#" -ge 2 ] || die "--upstream-tag needs a value"
            upstream_tag="$2"
            shift 2
            ;;
        --source-url)
            [ "$#" -ge 2 ] || die "--source-url needs a value"
            source_url="$2"
            shift 2
            ;;
        --sha256)
            [ "$#" -ge 2 ] || die "--sha256 needs a value"
            sha256="$2"
            shift 2
            ;;
        --local-source)
            [ "$#" -ge 2 ] || die "--local-source needs a value"
            local_source="$2"
            shift 2
            ;;
        --pkgrel)
            [ "$#" -ge 2 ] || die "--pkgrel needs a value"
            pkgrel="$2"
            shift 2
            ;;
        --repo-url)
            [ "$#" -ge 2 ] || die "--repo-url needs a value"
            repo_url="$2"
            shift 2
            ;;
        --aur-url)
            [ "$#" -ge 2 ] || die "--aur-url needs a value"
            aur_url="$2"
            shift 2
            ;;
        --build) do_build=true; shift ;;
        --no-syncdeps) syncdeps=false; shift ;;
        --push) do_push=true; shift ;;
        --keep) keep=true; shift ;;
        -h | --help)
            usage
            exit 0
            ;;
        *) die "unrecognized argument: $1 (see --help)" ;;
    esac
done

[ -n "$version" ] || die "--version is required (see --help)"
case "$version" in
    *[!0-9A-Za-z.+-]*)
        die "refusing version '$version': expected characters [0-9A-Za-z.+-]"
        ;;
esac
# AUR/pkgver rules: pkgver may contain alphanumerics, dots, underscores and plus
# signs. A hyphen is the pkgver/pkgrel separator, so a pre-release suffix has to
# be rewritten rather than passed through.
pkgver="${version//-/_}"
case "$pkgver" in
    "" | *[!0-9A-Za-z._+]*)
        die "refusing pkgver '$pkgver': AUR allows only [0-9A-Za-z._+]"
        ;;
esac
[ -n "$upstream_tag" ] || upstream_tag="v$version"
case "$pkgrel" in
    "" | *[!0-9]*) die "refusing pkgrel '$pkgrel': expected a positive integer" ;;
esac

if [ "$do_push" = true ]; then
    [ -z "$local_source" ] || die "--push refuses --local-source: a published recipe must point at the tagged source archive, not at a local file"
    [ "$sha256" != "SKIP" ] || die "--push refuses a SKIP checksum: the published package must pin a real archive"
fi

# --- package identity -------------------------------------------------------

case "$version" in
    *-canary* | *-alpha* | *-beta* | *-rc* | *-dev*)
        pkgname="inox-canary"
        pkgdesc="Canary channel of the Inox Wayland desktop environment"
        provides_value="('inox')"
        conflicts_value="('inox')"
        ;;
    *)
        pkgname="inox"
        pkgdesc="Wayland desktop environment: login greeter, session manager and shell"
        provides_value="()"
        conflicts_value="('inox-canary')"
        ;;
esac

# GitHub's tag archive extracts to <repository>-<tag with the leading v removed>.
repo_name="$(basename -- "${repo_url%/}")"
repo_name="${repo_name%.git}"
srcdir_name="${repo_name}-${upstream_tag#v}"

log "release $version -> AUR package $pkgname (pkgver $pkgver-$pkgrel, source tag $upstream_tag)"

workdir="$(mktemp -d "${TMPDIR:-/tmp}/inox-aur.XXXXXXXX")"
cleanup() {
    if [ "$keep" = true ]; then
        log "kept scratch directory: $workdir"
    else
        rm -rf -- "$workdir"
    fi
}
trap cleanup EXIT

# --- source archive and checksum -------------------------------------------

source_value=""
if [ -n "$local_source" ]; then
    [ -d "$local_source" ] || die "--local-source: no such directory: $local_source"
    local_source="$(cd -- "$local_source" && pwd)"
    git -C "$local_source" rev-parse --git-dir >/dev/null 2>&1 ||
        die "--local-source needs a git worktree: $local_source"
    archive="$workdir/$pkgname-$pkgver.tar.gz"
    log "creating the source archive from $local_source (git archive HEAD)"
    git -C "$local_source" archive \
        --format=tar.gz \
        --prefix="$srcdir_name/" \
        --output="$archive" \
        HEAD
    sha256="$(sha256sum -- "$archive" | cut -d' ' -f1)"
    source_value="(\"$(basename -- "$archive")\")"
else
    if [ -z "$source_url" ]; then
        source_url="${repo_url%/}/archive/refs/tags/$upstream_tag.tar.gz"
    fi
    source_value="(\"$pkgname-$pkgver.tar.gz::$source_url\")"
    if [ -z "$sha256" ]; then
        log "downloading $source_url"
        curl --fail --silent --show-error --location --retry 3 \
            --output "$workdir/source.tar.gz" "$source_url"
        sha256="$(sha256sum -- "$workdir/source.tar.gz" | cut -d' ' -f1)"
    fi
fi

if [ "$sha256" != "SKIP" ]; then
    case "$sha256" in
        *[!0-9a-f]*)
            die "sha256 must be 64 lowercase hex characters, got '$sha256'"
            ;;
    esac
    if [ "${#sha256}" -ne 64 ]; then
        die "sha256 must be 64 lowercase hex characters, got ${#sha256}: '$sha256'"
    fi
fi

# --- render ----------------------------------------------------------------
#
# sed replacement text: escape the characters sed itself interprets, so a URL or
# description containing one cannot corrupt the rendered PKGBUILD.
sed_escape() {
    printf '%s' "$1" | sed -e 's/[\\&|]/\\&/g'
}

workdir_pkgbuild="$workdir/PKGBUILD"
sed \
    -e "s|@PKGNAME@|$(sed_escape "$pkgname")|g" \
    -e "s|@PKGVER@|$(sed_escape "$pkgver")|g" \
    -e "s|@PKGREL@|$(sed_escape "$pkgrel")|g" \
    -e "s|@PKGDESC@|$(sed_escape "$pkgdesc")|g" \
    -e "s|@PROVIDES@|$(sed_escape "$provides_value")|g" \
    -e "s|@CONFLICTS@|$(sed_escape "$conflicts_value")|g" \
    -e "s|@SOURCE@|$(sed_escape "$source_value")|g" \
    -e "s|@SHA256@|$(sed_escape "$sha256")|g" \
    -e "s|@SRCDIR_NAME@|$(sed_escape "$srcdir_name")|g" \
    "$TEMPLATE" >"$workdir_pkgbuild"
cp -- "$INSTALL_FILE" "$workdir/inox.install"

log "rendered $workdir_pkgbuild"

# --- validate the recipe ---------------------------------------------------

command -v bash >/dev/null 2>&1 || die "bash is required"
bash -n "$workdir_pkgbuild" || die "the rendered PKGBUILD is not valid shell"
bash -n "$workdir/inox.install" || die "packaging/aur/inox.install is not valid shell"

command -v makepkg >/dev/null 2>&1 ||
    die "makepkg is not on PATH; the AUR recipe can only be checked (and built) on Arch"

( cd "$workdir" && makepkg --printsrcinfo >.SRCINFO ) ||
    die "makepkg --printsrcinfo failed; the recipe is not valid"
# .SRCINFO indents every field with a tab inside the pkgbase/pkgname blocks.
grep -qE "^[[:space:]]*pkgname = $pkgname\$" "$workdir/.SRCINFO" ||
    die ".SRCINFO does not name $pkgname"
grep -qE "^[[:space:]]*pkgver = $pkgver\$" "$workdir/.SRCINFO" ||
    die ".SRCINFO does not carry the computed pkgver"
if [ "$sha256" != "SKIP" ]; then
    grep -qE "^[[:space:]]*sha256sums = $sha256\$" "$workdir/.SRCINFO" ||
        die ".SRCINFO does not carry the source checksum"
fi
log "generated .SRCINFO"

# Dependency names are the part of a PKGBUILD that a syntax check cannot catch: a
# typo there only shows up on an AUR user's machine, as a failed dependency
# resolution. `pacman -Si` answers against the same repositories that user has.
if command -v pacman >/dev/null 2>&1; then
    mapfile -t dependency_names < <(
        sed -n -E 's/^[[:space:]]*(depends|makedepends) = //p' "$workdir/.SRCINFO"
    )
    if [ "${#dependency_names[@]}" -eq 0 ]; then
        die "no depends/makedepends found in .SRCINFO; the recipe lost its dependency list"
    fi
    missing=()
    for dependency in "${dependency_names[@]}"; do
        if ! pacman -Si -- "$dependency" >/dev/null 2>&1; then
            missing+=("$dependency")
        fi
    done
    if [ "${#missing[@]}" -gt 0 ]; then
        die "these depends/makedepends do not resolve in the configured pacman repositories (run 'pacman -Sy' first): ${missing[*]}"
    fi
    log "all ${#dependency_names[@]} depends/makedepends resolve"
else
    warn "pacman is not on PATH; skipping the dependency-name check"
fi

# --- build and verify ------------------------------------------------------

verify_package() {
    local pkgfile listing
    pkgfile="$(find "$workdir/pkgs" -maxdepth 1 -type f \
        -name "$pkgname-$pkgver-$pkgrel-*.pkg.tar*" ! -name '*.sig' -print -quit)"
    [ -n "$pkgfile" ] || die "makepkg did not produce a .pkg.tar.* file for $pkgname-$pkgver-$pkgrel"
    log "built $(basename -- "$pkgfile")"

    listing="$workdir/package-files.txt"
    if command -v bsdtar >/dev/null 2>&1; then
        bsdtar -tf "$pkgfile" >"$listing"
    else
        tar -tf "$pkgfile" >"$listing"
    fi

    local expected=(
        "etc/pam.d/inox-auth"
        "etc/pam.d/inox-auth-session"
        "etc/pam.d/inox-greeter"
        "usr/lib/sysusers.d/inox.conf"
        "usr/lib/systemd/system/inox-auth.service"
        "usr/lib/systemd/system/inox-greeter@.service"
        "usr/lib/systemd/system/inox-login-fallback.service"
        "usr/share/licenses/$pkgname/LICENSE"
    )
    local binary
    for binary in "${SHIPPED_BINARIES[@]}"; do
        expected+=("usr/bin/$binary")
    done

    local missing=()
    local path
    for path in "${expected[@]}"; do
        grep -qxF -- "$path" "$listing" || missing+=("$path")
    done
    if [ "${#missing[@]}" -gt 0 ]; then
        printf 'the package is missing:\n' >&2
        printf '  %s\n' "${missing[@]}" >&2
        die "the built package does not have the shape a desktop install needs"
    fi
    grep -q '^usr/share/inox/wallpapers/.' "$listing" ||
        die "the package carries no wallpapers; the wallpaper chooser would be empty"

    log "package contents verified: ${#SHIPPED_BINARIES[@]} binaries, wallpapers, PAM stacks, units, sysusers"
}
if [ "$do_build" = true ]; then
    makepkg_args=(--cleanbuild --noconfirm)
    if [ "$syncdeps" = true ]; then
        makepkg_args+=(--syncdeps)
    fi
    log "running makepkg (whole workspace, default features; this is the long step)"
    (
        cd "$workdir"
        export PKGDEST="$workdir/pkgs"
        export SRCDEST="$workdir/sources"
        export BUILDDIR="$workdir/build"
        export LOGDEST="$workdir/logs"
        mkdir -p "$PKGDEST" "$SRCDEST" "$BUILDDIR" "$LOGDEST"
        makepkg "${makepkg_args[@]}"
    )
    verify_package
fi

# --- publish ---------------------------------------------------------------

if [ "$do_push" = true ]; then
    aur_repo="$workdir/aur"
    aur_remote="${aur_url%/}/$pkgname.git"
    log "pushing to $aur_remote"

    # A package that does not exist yet clones as an empty repository; AUR creates
    # it on the first push, provided the name is not taken by another account.
    if ! git clone --quiet "$aur_remote" "$aur_repo" 2>"$workdir/aur-clone.err"; then
        if grep -qiE 'empty repository|does not exist|not found' "$workdir/aur-clone.err"; then
            warn "first publication of $pkgname: the AUR repository is empty"
            mkdir -p "$aur_repo"
            git -C "$aur_repo" init --quiet
            git -C "$aur_repo" remote add origin "$aur_remote"
        else
            cat "$workdir/aur-clone.err" >&2
            die "could not clone $aur_remote (is AUR_SSH_PRIVATE_KEY the key on the AUR account?)"
        fi
    fi

    # AUR keeps the git repository of a *deleted* package, so `inox` (previously a
    # Chromium spin-off that was deleted years ago) clones with that history. The
    # commit below then lands on top of it and the push is a fast-forward; saying so
    # in the log keeps the old history from looking like a bug.
    if git -C "$aur_repo" rev-parse --verify --quiet HEAD >/dev/null; then
        log "the AUR repository already has $(git -C "$aur_repo" rev-list --count HEAD) commit(s); committing on top"
    fi

    cp -- "$workdir_pkgbuild" "$workdir/.SRCINFO" "$workdir/inox.install" "$aur_repo/"
    git -C "$aur_repo" add -- PKGBUILD .SRCINFO inox.install

    if git -C "$aur_repo" diff --cached --quiet; then
        log "the AUR package already carries this recipe; nothing to push"
    else
        git -C "$aur_repo" \
            -c user.name="${AUR_GIT_NAME:-Inox release bot}" \
            -c user.email="${AUR_GIT_EMAIL:-release@inox.invalid}" \
            commit --quiet -m "upgpkg: $pkgver-$pkgrel"

        # The AUR reads .SRCINFO from the commit; a commit that lost it would be
        # rejected by the AUR, or worse, describe a different version.
        git -C "$aur_repo" show HEAD:.SRCINFO | grep -qE "^[[:space:]]*pkgver = $pkgver\$" ||
            die "the commit to publish does not contain the expected .SRCINFO"

        git -C "$aur_repo" push --quiet origin HEAD:master
        log "published $pkgname $pkgver-$pkgrel (https://aur.archlinux.org/packages/$pkgname)"
    fi
fi

cat <<EOF

[inox-aur] done.
  package:  $pkgname $pkgver-$pkgrel
  tag:      $upstream_tag
  checksum: $sha256
  built:    $([ "$do_build" = true ] && printf 'yes' || printf 'no')
  pushed:   $([ "$do_push" = true ] && printf 'yes' || printf 'no (pass --push to publish)')
EOF
