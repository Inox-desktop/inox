//! The crate's evidence, inside the binary.
//!
//! cxx-qt 0.10 embeds the bridge's generated initialisers in the target that
//! declares the bridge, so an integration test — a separate target — cannot link
//! the crate's own C++ archive (KDAB/cxx-qt #770, contract §2.1). Everything that
//! used to live in `tests/` therefore lives here, in `#[cfg(test)]`, and the
//! tests that need a *process* spawn this test binary as a child whose one
//! selected test calls [`crate::run`].
//!
//! The mapping from the old layout is one-to-one:
//!
//! | before | now |
//! |---|---|
//! | `tests/catalog.rs` | [`catalog`] — the two catalogs and the code's message ids |
//! | `tests/cli.rs` | [`cli`] — the default path, through a child process |
//! | `tests/protocol.rs` | [`protocol`] — a real `ext-session-lock-v1` peer, the binary as a child |
//! | `tests/render.rs` + `tests/support` | [`render`] — the QML host's frames, asserted on pixels |
//!
//! `tests/render.rs` used to draw through `iced_runtime::UserInterface` and
//! `iced_tiny_skia`. Those are gone (contract §5); what replaces them is the same
//! question asked of the renderer that actually ships: the QML projection, drawn
//! through the same [`crate::render::Interface`] the binary commits frames with.
//! The acceptance scenarios Q02 asks for — light, dark, typed, limited, failed,
//! no trusted UI, no protocol, no boundary, 200 % text — are all still rendered
//! and asserted on; the interaction test that used to click the unlock button is
//! gone with the input path it never had (the lock surface reads no pointer).

mod catalog;
mod cli;
mod protocol;
mod render;
