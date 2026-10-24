# Inox greeter host — en-US.
#
# Everything this stage can say to a person lives here; the code only ever holds
# message ids (root AGENTS.md §7.3, PRD §11.2). The host is a systemd unit, so these
# sentences are read in the journal — by whoever is asking why their machine did not
# reach a login screen. `zh-CN` must define exactly the same ids with the same
# placeholders; `tests/catalog.rs` asserts it with `inox_common::i18n::compare_catalogs`
# and also checks both directions between the sources and this catalog (acceptance Q03).

# The usage line. A CLI's own syntax is still read by a person, so it is a message
# like any other rather than an English literal in the parser.
host-usage = inox-host [--bin-dir DIR] [--socket NAME] [--auth-socket PATH] [--compositor-arg ARG]…

# The version line. `inox-host` is a proper noun and stays as it is.
host-version = inox-host { $version }

# ADR 0037 §决策 3.1: the boundary is checked before a compositor exists, and the
# refusal has to name both what is missing and the process that provides it —
# `tests/cli.rs` depends on those two facts being in the same sentence.
host-boundary-missing = The system authentication boundary (inox-auth) is not available on { $socket }, so the greeter host did not start a compositor.

# Why the stage ended. Each one names the thing that failed and says that the host is
# stopping, because that is the question an operator has when the unit restarts.
host-compositor-exited = The compositor exited, so the greeter host is stopping. The login screen's clients no longer have a display.
host-socket-timeout = The compositor did not create { $socket } within { $seconds } seconds, so the greeter host is stopping.
host-compositor-unstartable = The compositor could not be started from { $program }, so the greeter host is stopping.
host-greeter-unstartable = The login screen could not be started from { $program }, so the greeter host is stopping.
host-restarts-exhausted = The login screen exited { $count } times within { $window } seconds, so the greeter host is stopping. systemd will record the failed start and try again.

# The one thing the host says about a shutdown that somebody asked for. It is not an
# error: the children are stopped in reverse order and the unit exits successfully.
host-stopping = The greeter host was asked to stop; it is stopping the login screen and then the compositor.
