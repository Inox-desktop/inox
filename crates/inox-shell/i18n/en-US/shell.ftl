# The user-visible strings of the Inox shell coordinator (`inox-shell`).
#
# Ids follow the `<component>-<area>-<name>` convention of the L3 milestone and
# are listed in `src/i18n.rs` under `id::ALL`, which `tests/catalog.rs` checks
# against this file and its zh-CN sibling. Fluent message identifiers cannot
# contain a dot, so areas are spelled with further dashes.

### The command line

shell-usage =
    Usage: inox-shell [--help]

    inox-shell is the Inox shell coordinator. It resolves the entries of the
    session — panel, launcher, notifications and settings — and reports the
    configuration each of them reads. The entries are separate processes that
    draw their own surfaces; this coordinator draws nothing itself.
shell-argument-unknown = `{ $argument }` is not an argument inox-shell knows.

### Startup

shell-starting = Inox shell { $version } starting in { $locale }.
shell-warn-no-layer-shell = The Inox compositor does not advertise wlr-layer-shell yet, so the entries of this session cannot become desktop surfaces and their placement policy is not applied (ADR 0018). inox-shell itself draws nothing and creates no surface.
shell-error-config = The Inox configuration could not be loaded: { $reason }
shell-error-catalog = The Inox shell message catalogs could not be loaded: { $reason }

### The entry registry

shell-registry-summary = { $count ->
        [one] One shell entry resolved: { $entries }.
       *[other] { $count } shell entries resolved: { $entries }.
    }
shell-registry-entry = { $required ->
        [yes] { $binary } serves the { $entry } entry, reads { $config } and is required
       *[no] { $binary } serves the { $entry } entry, reads { $config } and is optional
    }
shell-registry-unknown-entry = `{ $name }` is not an entry the Inox shell knows; the entries it knows are { $known }.
shell-registry-duplicate-entry = The `{ $entry }` entry is registered more than once.
shell-registry-invalid-config-path = The `{ $entry }` entry reads `{ $path }`, which is not a file inside the configuration directory: { $detail }
shell-registry-not-registered = The `{ $entry }` entry is not in the registry.

### Navigation between entries

shell-nav-opened = Opened the `{ $entry }` entry; the keyboard goes to `{ $focus }`.
shell-nav-opened-display = Opened the `{ $entry }` entry without taking the keyboard.
shell-nav-already-open = The `{ $entry }` entry is already open; nothing changed.
shell-nav-closed = Closed one layer; focus returns to window { $window }.
shell-nav-closed-no-window = Closed one layer; no window focus changes.
shell-nav-nothing-to-close = No entry is open, so there is no layer to close.

### Falling back when a return target is gone

shell-nav-returned-to-fallback = Closed one layer without focusing a window: { $reason }
shell-nav-fallback-window-destroyed = the window { $window } this layer was opened over no longer exists, so the compositor's own focus policy decides
shell-nav-fallback-session-changed = this layer was opened in session generation { $saved }, and the session is now in generation { $current }, so the window it saved is not reused

### Service states

shell-service-starting = starting
shell-service-ready = ready
shell-service-unavailable = unavailable; { $actionable }
shell-service-failed = failed; { $reason }

### Command results

shell-command-sent = sent; the service has not answered yet
shell-command-accepted = accepted by the service; the change is not applied yet
shell-command-applied = applied; the service reported the effect
shell-command-rejected = rejected by the service; { $reason }
shell-command-timed-out = no answer before the deadline
