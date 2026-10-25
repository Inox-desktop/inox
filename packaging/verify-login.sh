#!/usr/bin/env bash
#
# Did the login that just happened really happen? This turns "it seems to work"
# into invariants that either hold or do not.
#
# Run it from inside the session you just started (no root needed): it asks logind
# what the session looks like, then checks the parts Inox is responsible for —
# which VT the session got, which PAM service opened it, whether its environment
# carries the four XDG_* variables pam_systemd writes, and whether the login screen
# survived the handover.
#
# Every FAIL line says what the failure means, not just what was expected, and the
# script ends with one summary line and a non-zero exit code if anything FAILed.
#
# Usage:
#   packaging/verify-login.sh                       # trial: greeter on VT4 (default)
#   packaging/verify-login.sh --greeter-vt 1        # boot chain: greeter on VT1
#   packaging/verify-login.sh --after-logout        # after logging out of the session
#   packaging/verify-login.sh --user alice --greeter-vt 4
#
# Related: packaging/preflight.sh (root, read-only, before the trial),
# packaging/rehearse-headless.sh (no root, before everything). Decision:
# docs/book/src/usage/login.md.

set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly SELF="$SCRIPT_DIR/$(basename -- "${BASH_SOURCE[0]}")"

# the boot login chain, never on the greeter's VT.
readonly LOGIN_FIRST_VT=2
readonly LOGIN_LAST_VT=6
readonly SEAT="seat0"

user="${SUDO_USER:-${USER:-}}"
greeter_vt=4
after_logout=false
user_given=false

failures=0
warnings=0

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
usage: $SELF [--user NAME] [--greeter-vt N] [--after-logout]

  --user NAME whose session to verify (default: ${user:-your user})
  --greeter-vt N the VT the login screen is on: 4 for a trial (default),
                    1 for the boot chain
  --after-logout verify the state AFTER logging out instead of during the
                    session: the session is gone and the screen was handed back

No root needed. Exit code: 0 if no FAIL, 1 if any FAIL, 2 on a usage error.
EOF
}

die() {
    printf '[inox-verify] error: %s\n' "$*" >&2
    exit 2
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --user)
            [ "$#" -ge 2 ] || die "--user needs a value"
            user="$2"
            user_given=true
            shift 2
            ;;
        --greeter-vt)
            [ "$#" -ge 2 ] || die "--greeter-vt needs a value"
            greeter_vt="$2"
            shift 2
            ;;
        --after-logout)
            after_logout=true
            shift
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

case "$greeter_vt" in
    '' | *[!0-9]*) die "--greeter-vt must be a VT number, not '$greeter_vt'" ;;
esac
case "$greeter_vt" in
    1 | 2 | 3 | 4 | 5 | 6) ;;
    *) die "--greeter-vt must be between 1 and 6, not $greeter_vt" ;;
esac

[ -n "$user" ] || die "no user to verify: pass --user NAME"

uid="$(id -u "$user" 2>/dev/null || true)"
[ -n "$uid" ] || die "no account named '$user'"

if [ "$user" = "root" ]; then
    warn "the account to verify is root, which Inox refuses to log in: pass --user NAME"
fi

printf '[inox-verify] verifying the login of %s (uid %s), greeter on VT%s%s\n' \
    "$user" "$uid" "$greeter_vt" "$([ "$after_logout" = true ] && printf ', after logout')"

# --- logind, asked per session --------------------------------------------
# `loginctl list-sessions` column order is not a stable interface (systemd 262 adds
# a LEADER column), so every fact below is queried per session id.

session_ids() {
    loginctl list-sessions --no-legend 2>/dev/null | awk '{ print $1 }' | grep -E '^[0-9]+$' || true
}

prop() {
    loginctl show-session "$1" -p "$2" --value 2>/dev/null || true
}

# Logins only. logind also opens a `manager` session for the user's `systemd --user`
# instance; that one has no TTY, no VT and no PAM stack of ours, and counting it as
# a login would make every correct machine look like it had two.
sessions_of_user() {
    local session_id
    for session_id in $(session_ids); do
        [ "$(prop "$session_id" Name)" = "$1" ] || continue
        [ "$(prop "$session_id" Class)" = "user" ] || continue
        printf '%s\n' "$session_id"
    done
}

sessions_on_tty() {
    local session_id
    for session_id in $(session_ids); do
        if [ "$(prop "$session_id" TTY)" = "$1" ]; then
            printf '%s\n' "$session_id"
        fi
    done
}

# VT as logind reports it, falling back to the TTY name: some setups (and any
# sandbox without /dev/ttyN) leave the VT property empty while TTY is set.
session_vt() {
    local session_id="$1" vt tty
    vt="$(prop "$session_id" VT)"
    if [ -n "$vt" ]; then
        printf '%s' "$vt"
        return 0
    fi
    tty="$(prop "$session_id" TTY)"
    case "$tty" in
        tty[0-9]*) printf '%s' "${tty#tty}" ;;
        *) printf '' ;;
    esac
}

if ! loginctl list-sessions >/dev/null 2>&1; then
    printf '[inox-verify] error: loginctl does not answer; the login chain needs logind\n' >&2
    exit 1
fi

# --- the greeter's own session (both modes need it) -----------------------

greeter_session="$(sessions_on_tty "tty$greeter_vt" | head -n1)"
if [ -z "$greeter_session" ]; then
    fail "no session on VT$greeter_vt: the login screen is not there"
    note "在试运行里这表示 inox-greeter@${greeter_vt} 没有起来或被停掉了；屏幕上的东西不是登录界面"
else
    greeter_name="$(prop "$greeter_session" Name)"
    if [ "$greeter_name" = "greeter" ]; then
        ok "the login screen has session $greeter_session on VT$greeter_vt (user $greeter_name)"
    else
        warn "VT$greeter_vt is session $greeter_session, but its user is '$greeter_name', not 'greeter'"
        note "登录界面应当以 greeter 账户运行（User=greeter）；别的账户意味着你看到的不是它"
    fi
fi

if [ "$after_logout" = true ]; then
    # --- after logout ------------------------------------------------------

    user_sessions="$(sessions_of_user "$user")"
    if [ -z "$user_sessions" ]; then
        ok "the session of $user is gone"
    else
        fail "$user still has session(s): $(printf '%s ' $user_sessions)"
        note "注销没有把会话结束掉：会话还在，VT 就不会交回登录界面"
    fi

    active_session="$(loginctl show-seat "$SEAT" -p ActiveSession --value 2>/dev/null || true)"
    if [ -n "$greeter_session" ]; then
        if [ "$active_session" = "$greeter_session" ]; then
            ok "the seat is back on the login screen (ActiveSession=$active_session)"
        else
            fail "ActiveSession=$active_session, not the greeter session $greeter_session"
            note "画面没有交给登录界面：inox-auth 应该在会话结束后 ActivateSession(记录的会话)"
            note "看 journalctl -u inox-auth —— 它记录了自己交回了哪个会话"
        fi
    fi
else
    # --- during the session ------------------------------------------------

    user_sessions="$(sessions_of_user "$user")"
    session_count="$(printf '%s\n' "$user_sessions" | grep -c . || true)"
    if [ "${session_count:-0}" -eq 1 ]; then
        session_id="$user_sessions"
        ok "$user has exactly one session ($session_id)"
    elif [ "${session_count:-0}" -eq 0 ]; then
        fail "$user has no session at all"
        note "先在这个 VT 的登录界面登录，再从那个会话里运行本脚本"
        session_id=""
    else
        fail "$user has $session_count sessions: $(printf '%s ' $user_sessions)"
        note "v1 只允许一个用户会话；多余的是上一次没被结束的会话"
        session_id="$(printf '%s\n' "$user_sessions" | head -n1)"
    fi

    if [ -n "${session_id:-}" ]; then
        vt="$(session_vt "$session_id")"
        if [ -z "$vt" ]; then
            fail "session $session_id has no VT at all"
            note "VT 空 = pam_systemd 没拿到 XDG_VTNR：会话负责人启动时环境里没有它，libseat 也会拿不到 seat"
        elif [ "$vt" -lt "$LOGIN_FIRST_VT" ] || [ "$vt" -gt "$LOGIN_LAST_VT" ]; then
            fail "session $session_id is on VT$vt, outside VT$LOGIN_FIRST_VT..=$LOGIN_LAST_VT"
            note "inox-auth 应当取范围内的第一个空闲 VT（--session-vt-min/--session-vt-max）"
        elif [ "$vt" = "$greeter_vt" ]; then
            fail "session $session_id is on VT$vt — the same VT as the login screen"
            note "登录界面和你的会话挤在同一个 VT 上：屏幕不会切换，两边互相踩"
        else
            ok "the session is on VT$vt (VT$greeter_vt belongs to the login screen)"
        fi

        service="$(prop "$session_id" Service)"
        if [ "$service" = "inox-auth-session" ]; then
            ok "Service=inox-auth-session (opened by the session leader)"
        else
            fail "Service='${service:-?}', not 'inox-auth-session'"
            note "会话没走负责人：它不是由 inox-auth 的会话负责人 pam_open_session 打开的"
        fi

        class="$(prop "$session_id" Class)"
        if [ "$class" = "user" ]; then
            ok "Class=user"
        else
            fail "Class='${class:-?}', expected 'user'"
        fi

        seat="$(prop "$session_id" Seat)"
        if [ "$seat" = "$SEAT" ]; then
            ok "Seat=$SEAT"
        else
            fail "Seat='${seat:-?}', expected '$SEAT'"
            note "没有 seat 就没有设备：合成器拿不到 DRM，屏幕是黑的"
        fi

        active="$(prop "$session_id" Active)"
        if [ "$active" = "yes" ]; then
            ok "the session is active"
        else
            fail "the session is not active (Active='${active:-?}')"
            note "inox-auth 应当在负责人建立会话后 ActivateSession，否则你看不到自己的桌面"
        fi

        active_session="$(loginctl show-seat "$SEAT" -p ActiveSession --value 2>/dev/null || true)"
        if [ "$active_session" = "$session_id" ]; then
            ok "seat0 ActiveSession=$session_id"
        else
            fail "seat0 ActiveSession='${active_session:-none}', expected $session_id"
        fi

        # /run/user/<uid>: pam_systemd creates it, and everything in the session
        # (WAYLAND_DISPLAY, the compositor's seat handle) lives in it.
        runtime_dir="/run/user/$uid"
        if [ -d "$runtime_dir" ]; then
            runtime_owner="$(stat -c '%u' "$runtime_dir")"
            runtime_mode="$(stat -c '%a' "$runtime_dir")"
            if [ "$runtime_owner" = "$uid" ] && [ "$runtime_mode" = "700" ]; then
                ok "$runtime_dir exists, owned by uid $uid, mode $runtime_mode"
            else
                fail "$runtime_dir is uid $runtime_owner mode $runtime_mode, expected uid $uid mode 700"
                note "这不是本会话的 runtime 目录；通常意味着 pam_systemd 没有给它建目录"
            fi
        else
            fail "$runtime_dir does not exist"
            note "没有 runtime 目录 = 没有 logind 会话给这个用户（pam_systemd 没跑或失败）"
        fi

        # The four XDG_* variables live in the session leader's environment; the
        # rest of the session inherits them.
        leader_pid="$(pgrep -u "$uid" -x inox-session | head -n1 || true)"
        if [ -z "$leader_pid" ]; then
            warn "no process named 'inox-session' for uid $uid"
            note "falling back to the session's leader pid from logind"
            leader_pid="$(prop "$session_id" Leader)"
            case "$leader_pid" in
                '' | *[!0-9]*) leader_pid="" ;;
            esac
            [ -n "$leader_pid" ] || note "logind reports no leader pid either; the environment checks are skipped"
        fi

        if [ -n "$leader_pid" ] && [ -r "/proc/$leader_pid/environ" ]; then
            ok "reading the session environment from pid $leader_pid"
            environ="$(tr '\0' '\n' <"/proc/$leader_pid/environ" 2>/dev/null || true)"
            env_value() { printf '%s\n' "$environ" | sed -n "s/^$1=//p" | head -n1; }

            env_session="$(env_value XDG_SESSION_ID)"
            if [ "$env_session" = "$session_id" ]; then
                ok "XDG_SESSION_ID=$env_session"
            else
                fail "XDG_SESSION_ID='${env_session:-unset}', logind says '$session_id'"
                note "会话进程环境里的会话 id 与 logind 的不一致：libseat 会用这个变量找自己的会话"
            fi

            env_runtime="$(env_value XDG_RUNTIME_DIR)"
            if [ "$env_runtime" = "$runtime_dir" ]; then
                ok "XDG_RUNTIME_DIR=$env_runtime"
            else
                fail "XDG_RUNTIME_DIR='${env_runtime:-unset}', expected '$runtime_dir'"
            fi

            env_vtnr="$(env_value XDG_VTNR)"
            if [ -z "${vt:-}" ]; then
                warn "no session VT to compare XDG_VTNR against; got '${env_vtnr:-unset}'"
            elif [ "$env_vtnr" = "$vt" ]; then
                ok "XDG_VTNR=$env_vtnr (matches the session's VT)"
            else
                fail "XDG_VTNR='${env_vtnr:-unset}', the session is on VT$vt"
                note "VT 号与 logind 的不一致 = 负责人把 XDG_VTNR 传错了（父进程给的 --vt 与激活的会话不同）"
            fi
        elif [ -z "$leader_pid" ]; then
            warn "skipped the environment checks: no leader process to read"
        else
            warn "cannot read /proc/$leader_pid/environ (permissions); environment checks skipped"
        fi
    fi
fi

# --- verdict ---------------------------------------------------------------

printf '\n[inox-verify] %d FAIL, %d WARN\n' "$failures" "$warnings"
if [ "$failures" -gt 0 ]; then
    printf '[inox-verify] FAIL — the invariants above do not all hold; see the note under each FAIL.\n'
    exit 1
fi

if [ "$after_logout" = true ]; then
    printf '[inox-verify] PASS — the session ended and the screen went back to the login screen.\n'
else
    printf '[inox-verify] PASS — this is a real logind session of %s, opened by the inox-auth session leader.\n' "$user"
fi
