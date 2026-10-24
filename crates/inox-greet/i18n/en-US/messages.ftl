# Inox greeter — en-US.
#
# Every string the login interface can put on screen lives here; the code only
# ever holds message ids (root AGENTS.md §7.3, PRD §11.2). `zh-CN` must define
# exactly the same ids with the same placeholders — `tests/catalog.rs` asserts it
# with `inox_common::i18n::compare_catalogs` (acceptance Q03).

# A CLI's own usage line names options rather than prose, but it is still read by
# a person, so it is a message like any other.
greet-usage = inox-greet [--preview]

# The name of the window, and of the component.
greet-window-title = Inox login

# The password field, and the one who is logging in. The account's own name is
# not a message: it is data, and it comes from the system's account database.
greet-credential-label = Password
greet-credential-placeholder = Your password
greet-hint = Press Enter to log in; Escape clears the password
# What is left of the keyboard interface when the log-in control cannot be used:
# a hint that promised a login would be a hint about a key that does nothing.
greet-hint-clear = Press Escape to clear the password

# The controls.
greet-submit = Log in
greet-clear = Clear
greet-switch-user = Switch user
greet-back = Back

# The account list: what it is, and what it says about the person who is not in it.
greet-choose-account = Choose an account

# The session a login would start. The name is a proper noun and is the same
# message in every language; it is a catalog entry rather than a literal so that
# the rule "no user-visible text outside the catalogs" has no exceptions to
# remember.
greet-session-label = Session
greet-session-name = Inox

# What this build is, above everything else. Two banners, because the claim is
# about the build: a preview collects nothing and starts nothing, and a build with
# an authentication boundary behind it does both.
greet-banner = Preview only. No credentials are collected and no session is started.
greet-banner-live = Logging in starts an Inox session.
greet-blocked = The system authentication boundary is not available, so no login can be checked.

# The states under the form. Deliberately free of counts, remaining attempts and
# remaining time: a rate limit that reports its own internal timing tells
# whoever is guessing at it how much budget is left (ADR 0015, and the same rule
# the lock screen's limiter follows).
greet-unavailable = Log in cannot run
greet-checking = Checking your account…
greet-starting = Starting your session…
greet-started = Your session is starting
greet-failed = That login was not accepted

# What is wrong with what the screen was handed, rather than with what somebody
# typed: the account database is not this screen's to trust.
greet-identity-too-long = That account name is too long.
greet-credential-too-long = That password is too long.

# Why nothing can be typed, and why the log-in control cannot be used, shown next
# to the control it belongs to (acceptance D05).
greet-disabled-unavailable = The system authentication service is not available
greet-disabled-verifying = A login is already being checked
greet-disabled-started = The session is already starting
greet-disabled-invalid = That account name cannot be sent to the authentication service
greet-disabled-rate-limited = Too many attempts for now
greet-disabled-no-accounts = No account on this system can be logged in
greet-disabled-accounts-unreadable = The list of accounts could not be read
greet-disabled-account-unselected = Choose an account to log in as
