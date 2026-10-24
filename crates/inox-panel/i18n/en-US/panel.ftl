# Inox panel — en-US.
#
# Every string the panel can put on screen lives here; the code only ever holds
# message ids (root AGENTS.md §7.3, PRD §11.2). `zh-CN` must define exactly the
# same ids with the same placeholders -- `tests/catalog.rs` asserts it with
# `inox_common::i18n::compare_catalogs` (acceptance Q03).
#
# Weekday and month names are numbered the way `chrono` numbers them:
# 1 = Monday .. 7 = Sunday, 1 = January .. 12 = December.

# Title of the panel's own surface: every surface the panel draws is a
# wlr-layer-shell surface (ADR 0018).
panel-window-title = Inox Panel

# Panel-level states (acceptance D05).
panel-state-starting = Waiting for the window service
panel-state-empty = No windows open
panel-service-unavailable = The window service is not available
panel-service-retry = Retry

# Task buttons (acceptance D03).
panel-task-unknown-app = Unknown application
panel-task-disabled-service = The window service is not available
panel-task-tooltip = { $app }: { $title }
panel-task-tooltip-empty = { $app }
panel-task-title-with-count = { $title } ({ $count })

# Dock tiles: the panel has no icon theme decoder, so a tile is a badge with the
# application's first character, and an application with no name gets this
# placeholder instead. It is short because it is drawn inside a square tile.
panel-dock-icon-placeholder = ?

# Tray items (acceptance D03).
panel-tray-unknown-item = Status item
panel-tray-tooltip = { $title } ({ $status })
panel-tray-status-passive = idle
panel-tray-status-active = active
panel-tray-status-attention = needs attention

# Clock (acceptance Q02, Q03).
panel-clock-tooltip = { $weekday }, { $month } { $day } { $year }
panel-clock-am = AM
panel-clock-pm = PM
panel-clock-zone-utc = UTC
panel-clock-zone-offset = UTC{ $sign }{ $hours }:{ $minutes }

panel-clock-weekday-abbr-1 = Mon
panel-clock-weekday-abbr-2 = Tue
panel-clock-weekday-abbr-3 = Wed
panel-clock-weekday-abbr-4 = Thu
panel-clock-weekday-abbr-5 = Fri
panel-clock-weekday-abbr-6 = Sat
panel-clock-weekday-abbr-7 = Sun
panel-clock-weekday-full-1 = Monday
panel-clock-weekday-full-2 = Tuesday
panel-clock-weekday-full-3 = Wednesday
panel-clock-weekday-full-4 = Thursday
panel-clock-weekday-full-5 = Friday
panel-clock-weekday-full-6 = Saturday
panel-clock-weekday-full-7 = Sunday
panel-clock-month-abbr-1 = Jan
panel-clock-month-abbr-2 = Feb
panel-clock-month-abbr-3 = Mar
panel-clock-month-abbr-4 = Apr
panel-clock-month-abbr-5 = May
panel-clock-month-abbr-6 = Jun
panel-clock-month-abbr-7 = Jul
panel-clock-month-abbr-8 = Aug
panel-clock-month-abbr-9 = Sep
panel-clock-month-abbr-10 = Oct
panel-clock-month-abbr-11 = Nov
panel-clock-month-abbr-12 = Dec
panel-clock-month-full-1 = January
panel-clock-month-full-2 = February
panel-clock-month-full-3 = March
panel-clock-month-full-4 = April
panel-clock-month-full-5 = May
panel-clock-month-full-6 = June
panel-clock-month-full-7 = July
panel-clock-month-full-8 = August
panel-clock-month-full-9 = September
panel-clock-month-full-10 = October
panel-clock-month-full-11 = November
panel-clock-month-full-12 = December
