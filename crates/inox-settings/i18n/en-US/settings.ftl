# Inox settings — en-US.
#
# Every string the settings window can put on screen lives here; the code only
# ever holds message ids (root AGENTS.md §7.3, PRD §11.2). `zh-CN` must define
# exactly the same ids with the same placeholders -- `tests/catalog.rs` asserts
# it with `inox_common::i18n::compare_catalogs` (acceptance Q03).
#
# The ids follow `<component>-<area>-<name>`: `settings-field-*` is a field's
# label, `settings-choice-*` an option, `settings-error-*` what the validator
# says, `settings-status-*` the line that says how far a change has got.

# The window itself.
settings-window-title = Inox Settings

# The navigation list (PRD §9 areas).
settings-page-appearance = Appearance
settings-page-panel = Panel
settings-page-notifications = Notifications
settings-page-locale = Language

# Field labels.
settings-field-theme-appearance = Theme
settings-field-locale-language = Language
settings-field-panel-position = Edge
settings-field-panel-height = Height
settings-field-panel-autohide = Hide when not in use
settings-field-panel-show-clock = Clock
settings-field-notifications-position = Position
settings-field-notifications-do-not-disturb = Do not disturb
settings-field-notifications-low-timeout = Low urgency
settings-field-notifications-normal-timeout = Normal urgency
settings-field-notifications-critical-timeout = Critical urgency

# Options.
settings-choice-theme-light = Light
settings-choice-theme-dark = Dark
settings-choice-theme-system = Follow the session
settings-choice-locale-environment = Follow the environment
settings-choice-locale-en-us = English (United States)
settings-choice-locale-zh-cn = Chinese (Simplified)
settings-choice-panel-top = Top
settings-choice-panel-bottom = Bottom
settings-choice-notifications-top-left = Top left
settings-choice-notifications-top-right = Top right
settings-choice-notifications-bottom-left = Bottom left
settings-choice-notifications-bottom-right = Bottom right

# Switches and numbers.
settings-toggle-on = On
settings-toggle-off = Off
settings-number-with-unit = { $value } { $unit }
settings-number-decrease = −
settings-number-increase = +
settings-unit-px = px
settings-unit-ms = ms

# Actions.
settings-action-apply = Apply
settings-action-cancel = Cancel
settings-action-reset = Reset
settings-action-reset-all = Reset all to defaults
settings-action-reload = Read the files again

# Why something cannot be done (acceptance D05).
settings-disabled-in-flight = A save is in progress
settings-disabled-invalid = Some fields need fixing first
settings-disabled-no-changes = Nothing has been changed yet

# How far a change has got (acceptance D04, ADR 0014).
settings-status-unchanged = The files are unchanged
settings-status-modified = Changed here, not written yet
settings-status-saving = Writing the files
settings-status-pending = Saved; waiting for the components to confirm it
settings-status-applied = Saved and in effect
settings-status-saved-only = Saved; { $component } could not be asked to apply it
settings-status-conflict = { $file } changed after this window read it
settings-status-save-failed = { $file } could not be written
settings-status-component-rejected = { $component } refused the change
settings-status-reload-failed = The files could not be read again
settings-status-unreachable = { $component } cannot be asked to apply the change

# What each component answered, and what went wrong.
settings-detail-component-state = { $component }: { $state }
settings-detail-conflict-digest = The file now holds { $current }
settings-detail-save-failed = { $file }: { $reason }
settings-detail-fields = Refused for: { $fields }
settings-component-state-pending = not confirmed yet
settings-component-state-applied = applied
settings-component-state-rejected = refused: { $reason }
settings-component-state-unreachable = could not be asked: { $reason }

# The components a change is reported to. These are names, not a product list:
# they say which programme has to apply the values, so the status line can name
# the one that has not answered.
settings-component-compositor = the compositor
settings-component-shell = the shell
settings-component-notifications = the notification daemon

# What the validator says. `settings-error-field` is the sentence a user reads
# under the control: the dotted key `inox-config` would name in a parse error,
# then the rule that was broken.
settings-error-field = { $path }: { $message }
settings-error-height-range = between { $min } and { $max } logical pixels, found { $value }
settings-error-timeout-range = between { $min } and { $max } milliseconds, found { $value }
settings-error-locale = "{ $value }" is not a language tag the session can run in
