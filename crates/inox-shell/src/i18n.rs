//! The catalogs this crate ships and the message ids it uses.
//!
//! `inox-common::i18n` owns the machinery — loading a catalog, negotiating a
//! locale, formatting a message — and this module is the crate's end of it: the
//! two `.ftl` files compiled into the binary with `include_str!`, the localizer
//! the coordinator and the tests build, and the list of ids in [`id`].
//!
//! Both shipped locales must define every id. `tests/catalog.rs` checks that
//! with [`inox_common::i18n::compare_catalogs`] (ids *and* placeholders, which
//! is what acceptance Q03 asks for) and by rendering every message in both
//! locales: a message that no catalog defines renders as its own id, which is
//! exactly what that test asserts against.
//!
//! Message ids follow the `<component>-<area>-<name>` convention of the L3
//! milestone, so `shell-nav-closed` is a navigation message and
//! `shell-registry-entry` a registry one. Fluent message identifiers cannot
//! contain a dot, so the areas are spelled with further dashes.

use inox_common::i18n::{I18nError, Locale, Localizer};
use tracing::warn;

/// The `en-US` catalog, compiled into the binary.
pub const EN_US: &str = include_str!("../i18n/en-US/shell.ftl");

/// The `zh-CN` catalog, compiled into the binary.
pub const ZH_CN: &str = include_str!("../i18n/zh-CN/shell.ftl");

/// The tag of the translated locale Inox ships besides the default.
pub const ZH_CN_TAG: &str = "zh-CN";

const EN_US_SOURCES: &[&str] = &[EN_US];
const ZH_CN_SOURCES: &[&str] = &[ZH_CN];

/// The message ids this crate can put on a user-visible surface.
///
/// Every id is a constant so that the catalog test can prove the two things
/// that would otherwise go wrong in opposite directions: an id the code uses
/// but no catalog defines (which would reach a user as a bare identifier), and
/// a message in the catalog that no code uses (which would be dead weight
/// nobody notices). [`id::ALL`] is the list checked against both catalogs.
pub mod id {
    /// The `--help` text.
    pub const USAGE: &str = "shell-usage";
    /// An argument the coordinator does not accept.
    pub const ARGUMENT_UNKNOWN: &str = "shell-argument-unknown";
    /// The startup line naming the version and the locale.
    pub const STARTING: &str = "shell-starting";
    /// Why the coordinator creates no desktop surface of its own.
    pub const NO_LAYER_SHELL: &str = "shell-warn-no-layer-shell";
    /// The configuration could not be loaded.
    pub const CONFIG_ERROR: &str = "shell-error-config";
    /// The message catalogs could not be loaded.
    pub const CATALOG_ERROR: &str = "shell-error-catalog";

    /// How many entries the session resolved.
    pub const REGISTRY_SUMMARY: &str = "shell-registry-summary";
    /// One resolved entry.
    pub const REGISTRY_ENTRY: &str = "shell-registry-entry";
    /// A name that is not an entry.
    pub const REGISTRY_UNKNOWN_ENTRY: &str = "shell-registry-unknown-entry";
    /// An entry registered twice.
    pub const REGISTRY_DUPLICATE_ENTRY: &str = "shell-registry-duplicate-entry";
    /// An entry reading a file outside the configuration directory.
    pub const REGISTRY_INVALID_CONFIG_PATH: &str = "shell-registry-invalid-config-path";
    /// An entry the registry does not hold.
    pub const REGISTRY_NOT_REGISTERED: &str = "shell-registry-not-registered";

    /// An entry was opened and the keyboard moved.
    pub const NAV_OPENED: &str = "shell-nav-opened";
    /// An entry was opened without taking the keyboard.
    pub const NAV_OPENED_DISPLAY: &str = "shell-nav-opened-display";
    /// An entry that was already open.
    pub const NAV_ALREADY_OPEN: &str = "shell-nav-already-open";
    /// A layer closed and focus returns to a window.
    pub const NAV_CLOSED: &str = "shell-nav-closed";
    /// A layer closed without moving window focus.
    pub const NAV_CLOSED_NO_WINDOW: &str = "shell-nav-closed-no-window";
    /// Nothing was open to close.
    pub const NAV_NOTHING_TO_CLOSE: &str = "shell-nav-nothing-to-close";
    /// The saved window is gone.
    pub const NAV_FALLBACK_WINDOW_DESTROYED: &str = "shell-nav-fallback-window-destroyed";
    /// The saved target belongs to an earlier session.
    pub const NAV_FALLBACK_SESSION_CHANGED: &str = "shell-nav-fallback-session-changed";

    /// A service that is coming up.
    pub const SERVICE_STARTING: &str = "shell-service-starting";
    /// A service that is usable.
    pub const SERVICE_READY: &str = "shell-service-ready";
    /// A service that is not usable, with something to do about it.
    pub const SERVICE_UNAVAILABLE: &str = "shell-service-unavailable";
    /// A service that failed.
    pub const SERVICE_FAILED: &str = "shell-service-failed";

    /// A command that only reached the transport.
    pub const COMMAND_SENT: &str = "shell-command-sent";
    /// A command the service acknowledged.
    pub const COMMAND_ACCEPTED: &str = "shell-command-accepted";
    /// A command whose effect was observed.
    pub const COMMAND_APPLIED: &str = "shell-command-applied";
    /// A command the service refused.
    pub const COMMAND_REJECTED: &str = "shell-command-rejected";
    /// A command that was not answered in time.
    pub const COMMAND_TIMED_OUT: &str = "shell-command-timed-out";

    /// A layer closed but its return target may not be used.
    pub const NAV_RETURNED_TO_FALLBACK: &str = "shell-nav-returned-to-fallback";

    /// Every id above, for the catalog test.
    pub const ALL: [&str; 30] = [
        USAGE,
        ARGUMENT_UNKNOWN,
        STARTING,
        NO_LAYER_SHELL,
        CONFIG_ERROR,
        CATALOG_ERROR,
        REGISTRY_SUMMARY,
        REGISTRY_ENTRY,
        REGISTRY_UNKNOWN_ENTRY,
        REGISTRY_DUPLICATE_ENTRY,
        REGISTRY_INVALID_CONFIG_PATH,
        REGISTRY_NOT_REGISTERED,
        NAV_OPENED,
        NAV_OPENED_DISPLAY,
        NAV_ALREADY_OPEN,
        NAV_CLOSED,
        NAV_CLOSED_NO_WINDOW,
        NAV_NOTHING_TO_CLOSE,
        NAV_RETURNED_TO_FALLBACK,
        NAV_FALLBACK_WINDOW_DESTROYED,
        NAV_FALLBACK_SESSION_CHANGED,
        SERVICE_STARTING,
        SERVICE_READY,
        SERVICE_UNAVAILABLE,
        SERVICE_FAILED,
        COMMAND_SENT,
        COMMAND_ACCEPTED,
        COMMAND_APPLIED,
        COMMAND_REJECTED,
        COMMAND_TIMED_OUT,
    ];
}

/// A localizer over the catalogs this crate ships, for `locale`.
///
/// A locale Inox has no catalog for is negotiated down to one it has, so this
/// only fails when a catalog does not parse — which the crate's own tests rule
/// out, and which is still better reported than shown as blank labels.
///
/// # Errors
///
/// Returns [`I18nError::Syntax`] when a catalog is not valid Fluent, and
/// [`I18nError::MissingDefaultCatalog`] if the default catalog is ever left out.
pub fn localizer(locale: Locale) -> Result<Localizer, I18nError> {
    Localizer::load(locale, &catalogs())
}

/// The catalogs to load: the default locale, then every other translated one.
fn catalogs() -> Vec<(Locale, &'static [&'static str])> {
    let mut catalogs: Vec<(Locale, &'static [&'static str])> =
        vec![(Locale::default_locale(), EN_US_SOURCES)];

    match Locale::parse(ZH_CN_TAG) {
        Ok(locale) => catalogs.push((locale, ZH_CN_SOURCES)),
        // The tag is one of the two locales the workspace supports and the
        // catalog test pins it; a build that somehow reached this line still
        // starts, in en-US, instead of refusing to run.
        Err(error) => warn!(%error, "the translated locale tag did not parse; shipping en-US only"),
    }

    catalogs
}
