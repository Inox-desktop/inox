//! The settings window's Fluent catalogs, and the loader that assembles them.
//!
//! Every user-visible string the window can show lives in
//! `i18n/en-US/settings.ftl` and `i18n/zh-CN/settings.ftl` (root `AGENTS.md`
//! §7.3, PRD §11.2). They are embedded with `include_str!`, so a missing catalog
//! is a build failure rather than a window that renders message ids, and both
//! are parsed at startup: a typo in either locale is reported before the window
//! opens instead of when a user switches language.
//!
//! The loading itself is [`inox_common::i18n::Localizer`]'s job — this module
//! only says which catalogs this component ships. The parity check that keeps the
//! two locales in step lives with the rest of the evidence, in
//! `headless_tests/catalog.rs`.
//!
//! This module has no dependency on anything but `inox-common`: the control
//! binary (`inoxctl`) compiles it to print its own messages, and it must not drag
//! the window in (see the crate `AGENTS.md`, "the second binary").

use inox_common::i18n::{I18nError, Locale, Localizer};

/// The `en-US` catalog, the fallback every other locale resolves against.
pub const EN_US: &[&str] = &[include_str!("../i18n/en-US/settings.ftl")];

/// The `zh-CN` catalog.
pub const ZH_CN: &[&str] = &[include_str!("../i18n/zh-CN/settings.ftl")];

/// The catalogs this component ships, in the shape [`Localizer::load`] wants.
pub fn sources() -> Vec<(Locale, &'static [&'static str])> {
    vec![(Locale::default_locale(), EN_US), (zh_cn(), ZH_CN)]
}

/// Loads the window's catalogs for `requested`.
///
/// A requested locale the window does not ship falls back to `en-US` with a
/// warning, so a user who configured a language Inox has not translated yet gets
/// a working window rather than an error.
///
/// # Errors
///
/// Returns [`I18nError::Syntax`] when either catalog does not parse — a
/// packaging defect that must stop the window at startup, not a blank label
/// later.
pub fn load(requested: &Locale) -> Result<Localizer, I18nError> {
    Localizer::load(requested.clone(), &sources())
}

/// The `zh-CN` locale.
///
/// The tag is a compile-time constant, so a parse failure is not a runtime
/// condition. Falling back to the default locale rather than unwrapping keeps
/// the promise that no user-visible flag can make the window panic (root
/// `AGENTS.md` §3.3); the fallback is unreachable for the constant written here.
fn zh_cn() -> Locale {
    Locale::parse("zh-CN").unwrap_or_else(|_| Locale::default_locale())
}
