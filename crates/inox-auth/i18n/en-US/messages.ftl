# The Inox authentication boundary — en-US.
#
# The helper's only user-visible strings: the usage line and the sentences it
# refuses to start with. Everything else it says is a tracing event, which is for
# whoever runs the machine rather than for whoever reads the screen (ADR 0029).
# `tests/catalog.rs` keeps this file in step with the code and with zh-CN.

auth-usage = inox-auth [--socket PATH] [--service NAME] [--session-service NAME] [--session-program PATH] [--session-vt-min N] [--session-vt-max N] [--seat NAME] [--allow-uid UID] [--socket-group GID] [--minimum-uid UID] [--attempts N] [--window-seconds N] [--exit-after-session]

# Refusals, before anything is served.
auth-requires-root = the system socket requires root; use --socket to run an unprivileged instance
auth-session-program = the session program is not executable: { $path }
auth-argument = { $value } is not a valid value for { $option }
auth-vt-range = the session VT range names no VT: the lowest is { $min } and the highest is { $max }

# Refusals of the internal session-leader mode (ADR 0037 §决策 5).
auth-leader-root = the session leader must run as root; inox-auth starts it for one login and nothing else may
auth-leader-vt = the session VT { $vt } is outside the range this boundary places sessions on ({ $range })
auth-leader-account = { $user } is not an account this boundary starts a session for
auth-leader-arguments = --session-leader needs both --user and --vt
auth-leader-only = --user and --vt only apply together with --session-leader
