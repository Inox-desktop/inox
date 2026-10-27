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
# attacker how much budget is left.
lock-verifying = Checking your password…
lock-attempts-waiting = Too many attempts. Wait a moment before trying again.
lock-failed = That password was not accepted

# Why an attempt was refused, when the boundary says why.
lock-failure-rejected = the authentication service rejected the password
lock-failure-timed-out = the authentication service did not answer in time
lock-failure-unavailable = the authentication service is not reachable
lock-failure-busy = the authentication service is busy with another attempt
lock-failure-rate-limited = the authentication service is refusing attempts for now
lock-failure-authenticator = the authentication service could not check the password
lock-failure-cancelled = the attempt was abandoned before it was answered

# Why nothing can be typed, shown next to the disabled field (acceptance D05).
lock-disabled-boundary = The authentication service is not available
# The other half of the same question: not a service that is not answering, but a
# session with no account to authenticate as.
lock-disabled-boundary-missing = This session has no authentication service to use
lock-disabled-no-input = This screen cannot take keyboard input
lock-disabled-waiting = Too many attempts for now
lock-disabled-verifying = An attempt is already being checked
lock-disabled-locking = The session is not locked yet
lock-disabled-unlocked = The session is not locked
lock-disabled-protocol = The session lock protocol is not available
lock-disabled-no-ui = The lock screen is not running

# Held, with no trusted UI: the session stays locked while it restarts.
lock-held-title = The session is locked
lock-held-detail = The lock screen is restarting. Your session stays locked.

# A suspend that could not be prepared (lock first, then suspend).
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
lock-boundary-not-provided = this session has no authentication boundary it can use: no account name could be resolved for the uid the session runs as, so nothing is checked here and the session is released by logging out
lock-boundary-no-input = the compositor offered no keyboard, so nothing can be typed here

# What the binary reports before exiting non-zero.
lock-start-blocked = The session lock cannot start here: { $reason }
lock-start-blocked-reason = no session-lock token was delivered, so nothing can be locked; inox-session starts this component and delivers the token. To take the lock directly, pass --take-lock with INOX_LOCK_TOKEN set

lock-request-accepted = Asked the session to lock; whether every output is covered is the compositor's report, not this command's
lock-request-busy = The session is not ready to lock yet, or the lock client is already starting
lock-request-no-session = no session answered: is inox-session running with its control socket?
lock-request-refused = the session does not offer a lock service

# The protocol leg: this mode takes the real session lock, draws the lock
# screen on every output, and asks for the release only when the
# authentication boundary accepts the user.
lock-taking = Taking the session lock as the trusted client
lock-taken = The session is locked; every output is covered
lock-unlocked = The user was authenticated; the session lock was released
lock-refused = The compositor refused the session lock
lock-released = The session lock was released
lock-stopped-locked = Stopped while the session is locked: the session stays locked until it is released
lock-stopped-unconfirmed = Stopped before the compositor confirmed the lock
lock-take-failed = The session lock could not be taken: { $reason }
lock-token-missing = no session lock token was delivered, so the compositor would refuse the claim; start this component from inox-session, which delivers it
lock-no-display = the session's Wayland display is not reachable
lock-missing-global = the compositor did not advertise { $interface }
