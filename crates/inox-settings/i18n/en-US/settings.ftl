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

# How far a change has got.
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

# The display trial. A mode, scale or position change takes effect
# immediately, so the window asks before it becomes permanent: the compositor
# keeps it only if it is confirmed, and puts the previous display settings back
# when the countdown runs out. Only the display page does this.
settings-status-display-trial = The display settings are applied, waiting for confirmation
settings-display-trial-title = Keep these display settings?
settings-display-trial-message = The display settings are in effect. Unless you confirm, the previous settings come back in { $seconds } seconds.
settings-display-trial-confirm = Keep the changes
settings-display-trial-revert = Restore the previous settings
settings-display-trial-active = The display settings are waiting for an answer
settings-display-trial-answering = The answer is on its way
settings-display-trial-notice-confirmed = The display settings were confirmed
settings-display-trial-notice-reverted = The previous display settings were restored
settings-display-trial-notice-unreachable = The previous display settings were restored in the files, but the compositor could not be told: { $reason }
settings-display-trial-notice-failed = The previous display settings could not be restored: { $reason }

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

settings-field-panel-margin = Edge margin
settings-field-panel-exclusive = Reserve panel space
settings-field-panel-clock-format = Clock format
settings-field-panel-max-task-buttons = Maximum task buttons
settings-field-panel-icon-size = Icon size
settings-field-launcher-max-results = Maximum results
settings-field-launcher-icon-size = Result icon size
settings-field-launcher-include-hidden = Include hidden applications
settings-field-launcher-max-query-chars = Search length limit
settings-field-wallpaper-image = Background image
settings-field-wallpaper-mode = Image placement
settings-field-keyboard-layout = Keyboard layout
settings-field-keyboard-variant = Keyboard variant
settings-field-keyboard-repeat-delay = Repeat delay
settings-field-keyboard-repeat-rate = Repeat rate (zero disables repeat)
settings-field-tap-to-click = Tap to click
settings-field-natural-scroll = Natural scrolling
settings-field-layout-master-ratio = Master area width
settings-field-layout-master-count = Master windows
settings-field-layout-gap = Window gap
settings-field-notifications-max-visible = Visible notifications
settings-field-notifications-width = Popup width
settings-field-notifications-margin = Popup margin
settings-field-notifications-icon-size = Notification icon size
settings-field-notifications-max-pending = Pending notification limit
settings-field-notifications-queue-policy = When the queue is full
settings-field-notifications-show-actions = Show notification actions
settings-field-notifications-hide-body-when-locked = Hide message contents when locked
settings-page-input = Keyboard & Mouse
settings-page-layout = Window Layout
settings-page-launcher = Launcher
settings-page-wallpaper = Wallpaper
settings-choice-fill = Fill
settings-choice-fit = Fit
settings-choice-stretch = Stretch
settings-choice-center = Center
settings-choice-tile = Tile
settings-choice-drop-oldest = Drop oldest
settings-choice-coalesce-by-app = Group by application
settings-choice-wallpaper-default = Default wallpaper
settings-choice-wallpaper-nebula = Nebula
settings-choice-wallpaper-fuji = Mount Fuji
settings-choice-wallpaper-bricks = Brick pavement
settings-choice-wallpaper-lake = Mountain lake
settings-choice-wallpaper-rainbow = Rainbow road
settings-choice-wallpaper-monument = Washington Monument
settings-choice-wallpaper-background = Theme background
settings-unit-count = items
settings-unit-hz = Hz
settings-unit-percent = %
settings-error-number-range = Enter a value between { $min } and { $max }; received { $value }.
settings-error-clock-format = Use a clock format supported by the panel, such as %H:%M.
settings-error-integer = Enter a whole number.

settings-field-outputs = Display preferences
settings-field-launcherterminal = Terminal command and arguments
settings-field-wallpaperperoutput = Per-display wallpapers
settings-page-displays = Displays
settings-output-name = Connector (empty matches any display)
settings-output-width = Width in pixels
settings-output-height = Height in pixels
settings-output-refresh = Refresh rate in Hz
settings-output-scale = Scale (0.25–8)
settings-output-x = Horizontal position (empty for automatic)
settings-output-y = Vertical position (empty for automatic)
settings-terminal-argument = Executable or one argument
settings-wallpaper-path = Wallpaper
settings-list-add = Add entry
settings-list-remove = Remove entry
settings-outputs-help = Changing these applies the new mode, scale and position at once, and the window asks you to confirm: without an answer the previous settings come back after 30 seconds. An empty list uses the backend defaults. Leave both positions empty for automatic placement; a single empty coordinate uses zero.
settings-terminal-help = Enter the executable first, then one argument per entry. Arguments are passed directly, without a shell. An empty list uses the launcher default.
settings-wallpapers-help = Enter a connector name and a wallpaper — a bundled name, none, or an image path — for each override. Use * for other displays. Empty lists use the main wallpaper.
settings-error-collection = Check entry { $row }: dimensions 1–16384, refresh 1–1000, scale 0.25–8, positions within ±4194304.

settings-page-input-method = Input Method
settings-field-input-method-command = Engine command and arguments
settings-input-method-help = Enter fcitx5 or a compatible executable first, then one argument per entry. Leave the list empty to disable automatic startup. Changes take effect at the next physical session; command-line options override them. Install the engine separately and use its configuration tool for languages, dictionaries and switching shortcuts. Do not add a daemonize option.

settings-error-input-method-command = Check entry { $row }: the first entry must name an executable and arguments cannot contain a NUL character.
settings-error-wallpaper-entry = Check entry { $row }: display names must be unique.

settings-ctl-help = Usage: inoxctl reload (reload running components) or inoxctl --help
settings-ctl-applied = { $component }: applied
settings-ctl-absent = { $component }: not running
settings-ctl-failed = { $component }: reload was not confirmed
settings-ctl-incomplete = Configuration reload did not complete successfully.

# What each screen is for. One screen carries one title (the page's name) and one
# of these sentences under it, drawn as a caption (ui-guidelines §2).
settings-page-appearance-description = Which theme the session uses.
settings-page-panel-description = The strip along an output's edge, and what it shows.
settings-page-notifications-description = Where popups appear, how long they stay, and whether the session starts quiet.
settings-page-locale-description = The language the session runs in.
settings-page-input-description = How the keyboard and the pointer behave.
settings-page-layout-description = How tiled windows are arranged.
settings-page-launcher-description = What the launcher runs and how much it shows.
settings-page-wallpaper-description = What is drawn behind every window.
settings-page-displays-description = Saved display preferences for the next compositor start.
settings-page-input-method-description = The input method the session starts automatically.

# The cards a page's fields are grouped into (ui-guidelines §4). A card is a
# heading and the fields of one topic; cards are separated by a large step.
settings-group-appearance-theme = Theme
settings-group-panel-strip = The strip
settings-group-panel-behaviour = Behaviour
settings-group-panel-clock = Clock
settings-group-panel-tasks = Task buttons
settings-group-notifications-display = Popups
settings-group-notifications-timeouts = How long they stay
settings-group-notifications-queue = When the queue is full
settings-group-notifications-policy = Policy
settings-group-locale-language = Language
settings-group-input-keyboard = Keyboard
settings-group-input-repeat = Key repeat
settings-group-input-pointer = Pointer
settings-group-layout-tiling = Tiling
settings-group-launcher-command = Command
settings-group-launcher-results = Results
settings-group-wallpaper-image = Background image
settings-group-wallpaper-outputs = Per-display images
settings-group-displays-outputs = Outputs
settings-group-input-method-command = Engine

# What a list says when it holds nothing yet. An empty list is a state the user
# has to be able to read, not a blank space (ui-guidelines §6).
settings-empty-outputs = No display preferences yet; the backend defaults are used.
settings-empty-input-method = No engine command; automatic startup is off.
settings-empty-launcher-terminal = No terminal command; the launcher default is used.
settings-empty-wallpaper-per-output = No per-display images; every display uses the main wallpaper.

# A change a consumer refused or could not be asked about is not stuck: Apply is
# offered again for exactly that state, and this line says so.
settings-status-retry-hint = Press Apply to send the change to the components again.
