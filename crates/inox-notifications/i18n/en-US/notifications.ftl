# Inox notifications daemon — en-US.
#
# Every string the daemon can put on screen lives here; the code only ever holds
# message ids (root AGENTS.md §7.3, PRD §11.2). `zh-CN` must define exactly the
# same ids with the same placeholders -- `tests/catalog.rs` asserts it with
# `inox_common::i18n::compare_catalogs` (acceptance Q03).
#
# Application names, notification summaries, bodies and action labels are *data*
# that arrives with the notification, never messages: they are displayed as they
# were received (after sanitising) and are not translated here.

# Title of the daemon's own window: the popup stack is a layer surface on a real
# session and an ordinary window only while no layer-shell host exists
# (ADR 0018).
notifications-window-title = Inox Notifications

# The daemon's own state, shown where the popups would be so that an empty
# window explains itself instead of being a blank rectangle.
notifications-empty = No notifications
notifications-suppressed = Notifications are muted while do not disturb is on

# Shown in place of the summary, the application name and the icon while the
# session is locked (ADR 0015, acceptance D06): the sensitive parts are not in
# the view model at all.
notifications-generic-summary = A notification arrived

# The close button of one popup.
notifications-close = Close

# Why an action button cannot be used: `policy.show_actions` is off (D05).
notifications-action-disabled-policy = Actions are turned off in notifications.toml

# More notifications are held than the stack can show (D05, acceptance D06).
notifications-overflow = { $count } more waiting

# A body image this daemon does not decode; its alt text is shown instead, and
# this is what is shown when the image carries no alt text at all.
notifications-image-placeholder = Image
