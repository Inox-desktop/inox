# Inox session lock — en-US.
#
# Every string the lock screen can put on screen lives here; the code only ever
# holds message ids (root AGENTS.md §7.3, PRD §11.2). `zh-CN` must define exactly
# the same ids with the same placeholders — `tests/catalog.rs` asserts it with
# `inox_common::i18n::compare_catalogs` (acceptance Q03).

# The name of the surface, and of the component.
lock-window-title = Inox Session Lock

# The prompt itself.
lock-prompt = Enter your password
lock-field-label = Password
lock-field-placeholder = Type your password
lock-hint = Press Enter to unlock
lock-submit = Unlock

# The states under the prompt. Deliberately free of counts, remaining attempts
# and remaining time: a rate limit that reports its own internal timing tells an
# attacker how much budget is left (ADR 0015).
lock-verifying = Checking your password…
lock-attempts-waiting = Too many attempts. Wait a moment before trying again.
lock-failed = That password was not accepted

# Why an attempt was refused, when the boundary says why.
lock-failure-rejected = the authentication service rejected the password
lock-failure-timed-out = the authentication service did not answer in time
lock-failure-unavailable = the authentication service is not reachable
lock-failure-cancelled = the attempt was abandoned before it was answered

# Why nothing can be typed, shown next to the disabled field (acceptance D05).
lock-disabled-boundary = The authentication service is not available
lock-disabled-waiting = Too many attempts for now
lock-disabled-verifying = An attempt is already being checked
lock-disabled-unlocked = The session is not locked
lock-disabled-protocol = The session lock protocol is not available
lock-disabled-no-ui = The lock screen is not running

# Held, with no trusted UI: the session stays locked while it restarts (ADR 0015).
lock-held-title = The session is locked
lock-held-detail = The lock screen is restarting. Your session stays locked.

# A suspend that could not be prepared (ADR 0015: lock first, then suspend).
lock-suspend-refused-lock-failed = The session was not suspended: the lock could not be confirmed
lock-suspend-refused-timeout = The session was not suspended: the lock did not confirm in time
lock-suspend-refused-no-protocol = The session was not suspended: this compositor cannot lock the session

# The lock cannot run at all.
lock-unavailable = The session lock cannot run
lock-unavailable-retry = Try again
lock-protocol-missing-global = the compositor does not offer the session lock protocol (ext-session-lock-v1)
lock-protocol-registry-unavailable = the compositor's registry could not be read
lock-boundary-disconnected = the authentication service is not reachable
lock-boundary-timeout = the authentication service did not answer in time

# What the binary reports before exiting non-zero (ADR 0015, ADR 0018).
lock-start-blocked = The session lock cannot start in this build: { $reason }
lock-start-blocked-reason = the compositor does not offer ext-session-lock-v1, and ADR 0015 requires a human security review of the authentication boundary before this screen may authenticate anyone
