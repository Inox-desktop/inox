# inox-launcher — user-visible strings (root AGENTS.md §7.3).
#
# Ids follow `<component>-<area>-<name>`. A trailing `.attribute` selects an
# attribute of a message; see `inox_common::i18n`. Every id here must exist in
# every catalog the crate ships, with the same placeholders (acceptance Q03).

launcher-window-title = Applications
launcher-search-placeholder = Search applications
launcher-search-hint = Up/Down to choose · Enter to launch · Esc to close

launcher-state-loading = Reading the application index…
launcher-state-empty-title = No applications found
launcher-state-empty-body = No usable desktop entries were found in { $dirs }.
launcher-state-empty-hint = Check that XDG_DATA_DIRS names your application directories.
launcher-state-nomatch-title = No matches
launcher-state-nomatch-body = No application matches “{ $query }”.
launcher-state-unavailable-title = The application index is unavailable
launcher-state-action-retry = Try again
launcher-state-truncated = Only the first { $limit } characters are searched.

launcher-row-icon-placeholder = ?
launcher-row-disabled = Unavailable: { $reason }

launcher-launch-requested = Launching { $name }…
launcher-launch-started = { $name } is running.
launcher-launch-failed = { $name } could not be started.

launcher-skipped-note = { $count } unusable entries were skipped.

launcher-failure-index-unavailable = the application index could not be read: { $detail }
launcher-failure-no-command = the entry defines no Exec command
launcher-failure-unknown-field-code = the Exec command uses the undefined field code { $code }
launcher-failure-misplaced-field-code = the Exec command uses { $code } inside a larger word
launcher-failure-unterminated-quote = the Exec command has an unbalanced double quote
launcher-failure-trailing-escape = the Exec command ends with a stray backslash
launcher-failure-trailing-percent = the Exec command ends with a stray % sign
launcher-failure-missing-terminal = the entry needs a terminal, but launcher.terminal is not set in shell.toml
launcher-failure-spawn-failed = the process could not be started: { $detail }
launcher-failure-exited-before-ready = the process exited before it was ready: { $detail }
launcher-failure-not-ready-in-time = the process gave no sign of running within { $seconds } s
