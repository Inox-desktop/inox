# Bundled wallpapers — sources and licences

Every image in this directory ships with Inox and is offered by the wallpaper
chooser in `inox-settings` (see `docs/adr/0030-bundled-wallpapers-and-the-default-background.md`).
The identifiers below are the values `wallpaper.image` accepts and the file names
are the ones installed into an Inox wallpaper directory
(`$XDG_DATA_DIRS/inox/wallpapers`).

| id | file | photographer | source | licence |
|----|------|--------------|--------|---------|
| `nebula` (default) | `nebula.jpg` | Scott Lord | <https://unsplash.com/photos/OHCl_XMUcZ4> | Unsplash License |
| `fuji` | `fuji.jpg` | Daniel Sessler | <https://unsplash.com/photos/AUPja_8Smd0> | Unsplash License |
| `bricks` | `bricks.jpg` | Joe Dudeck | <https://unsplash.com/photos/e2MLl-80u4E> | Unsplash License |
| `lake` | `lake.jpg` | Johannes Plenio | <https://unsplash.com/photos/gWFjkBDYtxg> | Unsplash License |
| `rainbow` | `rainbow.jpg` | Karla Hernandez | <https://unsplash.com/photos/BkjgNMEcfg8> | Unsplash License |
| `monument` | `monument.jpg` | Tom Gainor | <https://unsplash.com/photos/XprGan-6wy4> | Unsplash License |

## The licence this is under, and what it means for packagers

These are **not** public-domain works. They are distributed under the
[Unsplash License](https://unsplash.com/license), which grants the right to
download, copy, modify and distribute the photos free of charge, without
attribution. It does **not** grant the right to "compile photos from Unsplash to
replicate a similar or competing service".

Two consequences are worth stating plainly rather than leaving implied:

- **Attribution is not required by the licence, but is recorded here anyway**, so
  that a redistributor can always find out where an image came from.
- **Bundling a handful of wallpapers into a desktop environment is a judgement
  call.** Shipping this repository's packages in a distribution is the
  packager's decision to make, and a packager that would rather not rely on the
  Unsplash License can drop `data/wallpapers/` — the session then falls back to
  the theme background with a warning and keeps working (ADR 0030).

## How the files were produced

Each image was downscaled from the original download so that its **long edge is at
most 3840 pixels**, re-encoded as a progressive JPEG at quality 88, and stripped of
metadata. That keeps every image well under `inox-wallpaper`'s decode bounds
(`MAX_PIXELS`, `MAX_DIMENSION`) and keeps the installed set to a few megabytes; the
originals were 3.5–15 MB each and one was 8640×5760, which the decoder would have
refused outright.

The identifier/file-name table above is the single source of truth in code too —
`inox_config::wallpapers::BUNDLED` — and a unit test in `inox-settings` checks
that the chooser still matches it.
