//! The liquid-glass material of the Inox desktop, and the QML components that
//! draw it.
//!
//! A glass pane is drawn in two halves that cannot be merged (ADR 0040):
//!
//! - the **material** — a translucent tint, a hairline highlight along the top
//!   edge and a shade along the bottom one, plus the radii of the shape — is
//!   drawn by the component, from design tokens only;
//! - the **backdrop** — the softening and the refraction of the pixels *behind*
//!   the pane — can only be drawn by the compositor, because a client cannot
//!   sample the framebuffer behind its own surface. The component asks for it
//!   over `inox-glass-v1` v2 and is told what the compositor will actually do.
//!
//! This crate is the one place those two halves are decided, so every L3
//! component clamps and degrades the same way instead of growing its own copy of
//! the arithmetic (which is what `crates/inox-panel/src/frost.rs` was before it).
//!
//! # The decision, end to end
//!
//! ```
//! use inox_config::Theme;
//! use inox_glass::{Capabilities, Corners, Material, Outcome, Region, decide};
//!
//! // What the theme asks for: a pane with a 12 px corner radius.
//! let tokens = Theme::builtin_dark();
//! let material = Material::from_tokens(&tokens.glass, tokens.radii.md);
//!
//! // What the compositor answered. A version 1 compositor has no refraction;
//! // a version 2 one reports both maxima through `liquid_capabilities`.
//! let capabilities = Capabilities::from_legacy(64);
//!
//! // The pane is the whole configured surface.
//! let outcome = decide(material, capabilities, Region::pane(640, 68));
//!
//! match outcome {
//!     Outcome::Effect(request) => {
//!         assert_eq!(request.radius, u32::from(tokens.glass.blur).min(64));
//!         assert!(request.refraction.is_none(), "a v1 compositor cannot refract");
//!     }
//!     // The compositor will not soften this surface: plain translucency, and
//!     // the reason is available for the one `tracing` line a caller writes
//!     // (`info!`, in a crate that depends on `tracing` — this one does not).
//!     Outcome::Plain(reason) => assert!(!matches!(reason, inox_glass::Plain::EmptyPane)),
//! }
//! ```
//!
//! # Applying the decision
//!
//! [`Request::apply`] writes a request to any [`GlassTarget`]. A test double
//! records the calls; [`protocol`] implements the trait for the real
//! `inox_glass_v1` client object and silently skips the `since=2` requests when
//! the object is a version 1 one.
//!
//! # The QML components
//!
//! [`qml_files`] lists the assets in `qml/`: `GlassPane`, `GlassButton`,
//! `GlassToggle`, `GlassSlider`, `GlassTabBar` and the `CatalogWindow` page that
//! draws all five from one JSON projection ([`catalog`]). They are **assets**,
//! not a QML module: this crate depends on no Qt binding, and the host crate
//! compiles them into its own module from its `build.rs` (`qml_files()` is
//! path-correct from a consumer's build script because it resolves against
//! `CARGO_MANIFEST_DIR`). Each file is a stateless renderer: properties in, no
//! input handling, no colours or sentences of its own.
//!
//! # Layering
//!
//! This crate is **L3** and depends only downwards: [`inox_config`] (L1),
//! [`inox_protocols`] (L2) and `wayland-client`. The `inox-panel -> inox-glass`
//! edge is the same-layer exception ADR 0040 authorised and the root
//! `Cargo.toml` records in `same_layer_exceptions`; nothing else in L3 may
//! depend on this crate without its own entry there.

#![forbid(unsafe_code)]

pub mod capabilities;
pub mod catalog;
pub mod material;
pub mod protocol;
pub mod qml;
pub mod region;
pub mod request;

pub use capabilities::Capabilities;
pub use catalog::from_theme as catalog_model;
pub use material::{Corners, Material, Refraction};
pub use qml::{QML_FILES, catalog_qml, qml_files, qml_sources};
pub use region::Region;
pub use request::{GlassTarget, Outcome, Plain, RefractionRequest, Request, decide};

/// Largest corner radius the protocol will carry, frozen by ADR 0040 §1.
///
/// A client never has to respect this itself: the compositor clamps to it and
/// to half of the region's shorter side. It is here so the material can clamp
/// before it sends, and so the drift test can pin the number.
pub const MAX_CORNER_RADIUS: u16 = 16_384;

/// Largest refraction band thickness, in logical pixels (ADR 0040 §1).
pub const MAX_REFRACTION_HEIGHT: u16 = 128;

/// Largest refraction displacement, in logical pixels (ADR 0040 §1).
pub const MAX_REFRACTION_AMOUNT: u16 = 64;

/// The object version at which the refraction vocabulary exists.
///
/// `set_corner_radii`, `set_refraction` and the `liquid_capabilities` event are
/// all `since=2`. [`protocol`] gates every `since=2` request on the object's own
/// version being at least this, and the drift test in
/// `tests/protocol_drift.rs` checks the number against the generated bindings so
/// a future version bump cannot leave a stale threshold behind.
pub const REFRACTION_SINCE_VERSION: u32 = 2;

/// Bit of `refraction_flag` that adds the radial (towards the centre) component.
///
/// The bit is a wire value: [`protocol`] maps it onto the generated
/// `RefractionFlag::Depth`, and the drift test in `tests/protocol_drift.rs`
/// fails if the two ever disagree.
pub const REFRACTION_FLAG_DEPTH: u32 = 1;

/// Bit of `refraction_flag` that splits the refracted sample into a chromatic
/// separation. See [`REFRACTION_FLAG_DEPTH`] for why it is pinned here.
pub const REFRACTION_FLAG_DISPERSION: u32 = 2;
