# inox-wallpaper — en-US.
#
# Every user-visible string the component produces is a message here; nothing in
# the code is a literal (root AGENTS.md §7.3). Ids are
# `wallpaper-<area>-<name>`, and the same ids must exist in every catalog, with
# the same placeholders (asserted by tests/catalog.rs).

## The windowed preview.
# The title of the window a human looks at while no compositor advertises
# wlr-layer-shell (ADR 0018).
wallpaper-window-title = Inox wallpaper
# Shown at the foot of the preview so nobody mistakes it for the real thing.
wallpaper-windowed-notice =
    Previewing in a window: this compositor has no layer-shell host yet, so the
    wallpaper placement is computed and tested, and not applied (ADR 0018).

## What one output is drawing.
# The status card is read top to bottom: which output, what is on it, where that
# came from, how it was fitted, and what the component is doing about it. The
# path is a line of its own (`caption` + `text_muted`) rather than a clause in
# the sentence above it, because a path is metadata and has to stay readable
# when it is long (UI spec §4).
wallpaper-output-heading = Output { $output }
wallpaper-output-image = { $width }×{ $height } image
wallpaper-output-source = from { $path }
wallpaper-output-background = The theme background
wallpaper-status-loading = Loading the wallpaper…
# How the image was fitted to the output. The mode is always on screen: it is
# the one part of the geometry the user can act on (`wallpaper.mode`).
wallpaper-scaling = Scaling: { $mode }
# The values `wallpaper.mode` accepts, as the user writes them.
wallpaper-mode-fill = fill
wallpaper-mode-fit = fit
wallpaper-mode-stretch = stretch
wallpaper-mode-center = center
wallpaper-mode-tile = tile
# The phase the component is in, one line under the facts above. "Applied" is
# what the state contract confirms — the frame is committed for this output; it
# does not claim that a layer-shell host exists (ADR 0018).
wallpaper-phase-loading = Loading…
wallpaper-phase-ready = Applied
wallpaper-phase-degraded = Not applied
wallpaper-phase-idle = No image
# The empty state is a state of its own rather than the absence of one: it says
# what is on screen and what would change that (UI spec §6). It is reached by
# `wallpaper.image = "none"` and by nothing else — an *absent* key is the
# bundled default, not this state (ADR 0030) — so the remedy can name the value
# that produced it.
wallpaper-empty-remedy =
    wallpaper.image is set to none, so the theme background is drawn. Name a
    bundled wallpaper — or a path of your own — to show an image.

## A request that could not be honoured.
wallpaper-problem-heading = The selected wallpaper could not be used
wallpaper-problem-subject = { $output }: { $path }
wallpaper-remedy =
    Point wallpaper.image at a PNG or JPEG that exists, or remove it to use the
    theme background.
# Why the image cannot be used, one message per reason.
wallpaper-reason-missing = the file does not exist
# What the path is instead of a regular file. One message per kind, rather than
# one message with the kind substituted in: the word a user reads is copy, and
# copy is not something the code gets to write (root AGENTS.md §7.3).
wallpaper-reason-not-a-file-directory = the path is a directory, not a file
wallpaper-reason-not-a-file-other = the path is not a regular file
wallpaper-reason-unreadable = the file cannot be read: { $detail }
wallpaper-reason-undecodable = the image cannot be decoded: { $detail }
# One message per bound: an image can be over the pixel limit or over the side
# limit, and naming the wrong one tells the user about a bound they did not
# cross.
wallpaper-reason-too-large = the image has { $pixels } pixels, above the { $limit } pixel limit
wallpaper-reason-too-long-side = the image is { $width }×{ $height }, longer on one side than the { $limit } pixel limit
wallpaper-reason-degenerate = a { $width }×{ $height } image has no size on this output
wallpaper-reason-too-many-tiles = a { $width }×{ $height } tile would need { $tiles } copies, above the { $limit } copy limit

## The configuration as a whole.
wallpaper-config-error = The configuration could not be read: { $detail }
wallpaper-config-remedy = Fix the configuration, then start inox-wallpaper again.
# Why the retry control is disabled when the configuration is what failed.
wallpaper-config-read-once = The configuration is read once, when the component starts.

## The controls.
wallpaper-action-retry = Try again
wallpaper-action-nothing = There is nothing to reload
