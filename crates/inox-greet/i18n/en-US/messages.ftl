# Inox greeter — en-US.
#
# Every string the login interface can put on screen lives here; the code only
# ever holds message ids (root AGENTS.md §7.3, PRD §11.2). `zh-CN` must define
# exactly the same ids with the same placeholders — `tests/catalog.rs` asserts it
# with `inox_common::i18n::compare_catalogs` (acceptance Q03).

# A CLI's own usage line names options rather than prose, but it is still read by
# a person, so it is a message like any other.
greet-usage = inox-greet --preview

# The name of the window, and of the component.
greet-window-title = Inox login

# The prompt itself.
greet-headline = Log in to Inox
greet-field-label = Username
greet-placeholder = Your username
greet-hint = Press Enter to log in; Escape clears the username
# What is left of the keyboard interface when the log-in control cannot be used:
# a hint that promised a login would be a hint about a key that does nothing.
greet-hint-clear = Press Escape to clear the username

# The controls.
greet-submit = Log in
greet-clear = Clear

# The session a login would start. The name is a proper noun and is the same
# message in every language; it is a catalog entry rather than a literal so that
# the rule "no user-visible text outside the catalogs" has no exceptions to
# remember.
greet-session-label = Session
greet-session-name = Inox

# What this build is, above everything else.
greet-banner = Preview only. No credentials are collected and no session is started.
greet-blocked = Login is unavailable until the system authentication boundary passes security review.

# The states under the form. Deliberately free of counts, remaining attempts and
# remaining time: a rate limit that reports its own internal timing tells
# whoever is guessing at it how much budget is left (ADR 0015, and the same rule
# the lock screen's limiter follows).
greet-unavailable = Log in cannot run
greet-checking = Checking your account…
greet-starting = Starting your session…
greet-started = Your session is starting
greet-failed = That login was not accepted

# What is wrong with the identity the user typed.
greet-invalid-user = Enter a valid username.
greet-identity-too-long = That username is too long.

# Why nothing can be typed, and why the log-in control cannot be used, shown next
# to the control it belongs to (acceptance D05).
greet-disabled-unavailable = The system authentication service is not available
greet-disabled-verifying = A login is already being checked
greet-disabled-started = The session is already starting
greet-disabled-invalid = Enter a valid username
greet-disabled-rate-limited = Too many attempts for now
