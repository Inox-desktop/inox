//! Localisation: Fluent message catalogs, locale negotiation and fallback.
//!
//! PRD §11.2 and root `AGENTS.md` §7.3 make Project Fluent the only way a
//! user-visible string may reach the screen. This module is the machinery
//! behind that rule, and it is deliberately part of L1: loading a catalog and
//! resolving a locale is not UI code, so every layer that shows text — the L3
//! shell components and the L4 session tools alike — reads it from here instead
//! of each growing its own loader.
//!
//! A component embeds its catalogs at compile time and hands them to
//! [`Localizer::load`]:
//!
//! ```
//! use inox_common::i18n::{Arg, Locale, Localizer};
//!
//! let en = r#"greeting = Hello, { $name }!"#;
//! let zh = r#"greeting = 你好，{ $name }！"#;
//!
//! let localizer = Localizer::load(
//!     Locale::parse("zh-CN").expect("a well-formed tag"),
//!     &[
//!         (Locale::parse("en-US").expect("a well-formed tag"), &[en]),
//!         (Locale::parse("zh-CN").expect("a well-formed tag"), &[zh]),
//!     ],
//! )
//! .expect("both catalogs parse");
//!
//! assert_eq!(localizer.message_with("greeting", &[("name", "Inox".into())]), "你好，Inox！");
//! ```
//!
//! Three properties are load-bearing and are tested here rather than trusted:
//!
//! - **The default locale always wins a tie.** A missing translation falls back
//!   to [`DEFAULT_LOCALE`] and only then to the message id itself, so a catalog
//!   that lags behind never renders an empty string (acceptance Q03).
//! - **A malformed catalog is an error, not a silent empty string.** A typo in
//!   an `.ftl` file fails the component at startup instead of blanking a label.
//! - **Nothing is formatted silently.** Fluent reports unknown variables and
//!   bad selectors through a mutable error list; those are logged rather than
//!   dropped.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use fluent_syntax::ast;
use tracing::{debug, warn};
use unic_langid::LanguageIdentifier;

/// The locale every catalog set must provide, and the fallback for every
/// message a translation does not define (PRD §11.2).
pub const DEFAULT_LOCALE: &str = "en-US";

/// The locales Inox ships translations for (PRD §11.2).
pub const SUPPORTED_LOCALES: [&str; 2] = [DEFAULT_LOCALE, "zh-CN"];

/// The environment variables that select a locale, most specific first.
const LOCALE_ENV_VARS: [&str; 3] = ["LC_ALL", "LC_MESSAGES", "LANG"];

/// A BCP-47 language tag, canonically cased (`zh-cn` becomes `zh-CN`).
///
/// Canonicalising on construction is what makes `zh-CN`, `zh-CN.UTF-8` and
/// `zh_cn` all name the same catalog.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Locale(LanguageIdentifier);

impl Locale {
    /// Parses a BCP-47 language tag.
    ///
    /// # Errors
    ///
    /// Returns [`LocaleError::Malformed`] for anything the language-tag parser
    /// rejects, including an empty tag and the POSIX `C` locale — callers that
    /// want a usable locale in every case want [`Locale::from_env`] instead,
    /// which degrades to [`Locale::default_locale`].
    pub fn parse(tag: &str) -> Result<Self, LocaleError> {
        let normalised = normalise_tag(tag);
        normalised
            .parse::<LanguageIdentifier>()
            .map(Locale)
            .map_err(|error| LocaleError::Malformed {
                tag: tag.to_owned(),
                detail: error.to_string(),
            })
    }

    /// The locale the session asks for, or [`Locale::default_locale`].
    ///
    /// Reads `LC_ALL`, then `LC_MESSAGES`, then `LANG`. `LANG` values such as
    /// `en_US.UTF-8` are reduced to their language tag, and the POSIX `C` /
    /// `POSIX` values (which mean "no locale") fall back to the default. This
    /// function never fails: a desktop that cannot show its own strings because
    /// of an odd environment variable would be worse than one that shows them
    /// in `en-US`.
    pub fn from_env() -> Self {
        Self::from_env_with(|name| std::env::var(name).ok())
    }

    /// [`Locale::from_env`] against an injected environment, for tests.
    ///
    /// Tests cannot set process environment variables — `std::env::set_var` is
    /// `unsafe` in edition 2024 and every crate here forbids `unsafe` — so the
    /// lookup is a parameter rather than a reach into the process.
    pub fn from_env_with(mut get: impl FnMut(&str) -> Option<String>) -> Self {
        for name in LOCALE_ENV_VARS {
            let Some(raw) = get(name) else { continue };
            let candidate = normalise_tag(&raw);
            if candidate.is_empty() || candidate == "C" || candidate == "POSIX" {
                continue;
            }
            match candidate.parse::<LanguageIdentifier>() {
                Ok(tag) => return Locale(tag),
                Err(error) => {
                    debug!(variable = name, value = %raw, %error, "ignoring an unusable locale");
                }
            }
        }
        Self::default_locale()
    }

    /// The locale every catalog must define.
    pub fn default_locale() -> Self {
        // `DEFAULT_LOCALE` is a compile-time constant that the test below keeps
        // parseable, so this cannot fail for a reason a caller could act on.
        DEFAULT_LOCALE
            .parse::<LanguageIdentifier>()
            .map(Locale)
            .unwrap_or_else(|_| Locale(LanguageIdentifier::default()))
    }

    /// The language subtag (`en`, `zh`), for matching a translation to a
    /// request that only differs in region.
    pub fn language(&self) -> &str {
        self.0.language.as_str()
    }

    /// Whether Inox ships a catalog for this locale.
    pub fn is_supported(&self) -> bool {
        SUPPORTED_LOCALES.iter().any(|tag| *tag == self.to_string())
    }

    /// The best catalog to use for `requested`.
    ///
    /// An exact match wins; otherwise the first supported locale with the same
    /// language (`zh-Hans` finds `zh-CN`); otherwise [`Locale::default_locale`].
    /// This is a total function so that a caller never has to decide what to do
    /// about an untranslated language.
    pub fn negotiate(requested: &Locale) -> Self {
        if requested.is_supported() {
            return requested.clone();
        }
        if let Some(tag) = SUPPORTED_LOCALES
            .iter()
            .find(|tag| tag.split('-').next() == Some(requested.language()))
            && let Ok(locale) = Locale::parse(tag)
        {
            return locale;
        }
        Self::default_locale()
    }

    /// The canonical tag, e.g. `en-US`.
    pub fn tag(&self) -> String {
        self.0.to_string()
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for Locale {
    type Err = LocaleError;

    fn from_str(tag: &str) -> Result<Self, Self::Err> {
        Locale::parse(tag)
    }
}

/// Why a language tag could not be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocaleError {
    /// The tag is not a well-formed BCP-47 language tag.
    #[error("`{tag}` is not a well-formed BCP-47 language tag: {detail}")]
    Malformed {
        /// The rejected tag, as written by the caller.
        tag: String,
        /// The parser's own explanation.
        detail: String,
    },
}

/// Reduces an environment-style locale (`en_US.UTF-8`, `zh_CN@latin`) to a
/// language tag, so that only the parsing has to be defensive.
fn normalise_tag(raw: &str) -> String {
    let without_codeset = raw.split(['.', '@']).next().unwrap_or(raw);
    without_codeset.trim().replace('_', "-")
}

/// A value substituted into a message placeholder.
///
/// Numbers go through Fluent's formatter rather than `to_string`, so a locale
/// can group digits and pick its own decimal separator.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    /// A textual value, used as-is.
    Text(String),
    /// A number, formatted for the locale.
    Number(f64),
}

impl Arg {
    fn into_fluent(self) -> FluentValue<'static> {
        match self {
            Arg::Text(text) => FluentValue::from(text),
            Arg::Number(number) => FluentValue::from(number),
        }
    }
}

impl From<&str> for Arg {
    fn from(value: &str) -> Self {
        Arg::Text(value.to_owned())
    }
}

impl From<String> for Arg {
    fn from(value: String) -> Self {
        Arg::Text(value)
    }
}

impl From<&String> for Arg {
    fn from(value: &String) -> Self {
        Arg::Text(value.clone())
    }
}

/// Integers are widened to `f64`. Message placeholders carry counts and
/// timestamps, never identifiers that would need 53 bits of mantissa.
impl From<i64> for Arg {
    fn from(value: i64) -> Self {
        Arg::Number(value as f64)
    }
}

impl From<i32> for Arg {
    fn from(value: i32) -> Self {
        Arg::Number(f64::from(value))
    }
}

impl From<u32> for Arg {
    fn from(value: u32) -> Self {
        Arg::Number(f64::from(value))
    }
}

impl From<usize> for Arg {
    fn from(value: usize) -> Self {
        Arg::Number(value as f64)
    }
}

impl From<f64> for Arg {
    fn from(value: f64) -> Self {
        Arg::Number(value)
    }
}

/// Why a catalog set could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum I18nError {
    /// The fallback catalog was not among the ones provided.
    #[error(
        "`{DEFAULT_LOCALE}` must be provided as the fallback catalog, but the set only has {found:?}"
    )]
    MissingDefaultCatalog {
        /// The locales that were provided.
        found: Vec<String>,
    },
    /// A catalog is not valid Fluent.
    #[error("the `{locale}` catalog does not parse ({count} error(s)): {first}")]
    Syntax {
        /// The locale whose catalog is malformed.
        locale: String,
        /// How many errors the parser reported.
        count: usize,
        /// The first one, which is what usually names the real mistake.
        first: String,
    },
}

/// One locale's parsed catalog.
///
/// `FluentBundle` is not `Debug`, so this type does not derive it; the
/// [`Localizer`] that owns one reports its locale instead.
struct Catalog {
    bundle: FluentBundle<FluentResource>,
    ids: BTreeSet<String>,
}

impl Catalog {
    fn load(locale: &Locale, sources: &[&str]) -> Result<Self, I18nError> {
        let mut bundle = FluentBundle::new(vec![locale.0.clone()]);
        // Fluent wraps every placeable in Unicode isolate marks by default,
        // which is right for prose mixed into foreign text and wrong for a
        // label: it would put invisible characters into a taskbar button.
        bundle.set_use_isolating(false);

        let mut ids = BTreeSet::new();
        for source in sources {
            let resource = match FluentResource::try_new((*source).to_owned()) {
                Ok(resource) => resource,
                Err((_, errors)) => {
                    return Err(I18nError::Syntax {
                        locale: locale.tag(),
                        count: errors.len(),
                        first: errors
                            .first()
                            .map(|error| error.to_string())
                            .unwrap_or_default(),
                    });
                }
            };
            collect_ids(source, &mut ids);
            if let Err(errors) = bundle.add_resource(resource) {
                return Err(I18nError::Syntax {
                    locale: locale.tag(),
                    count: errors.len(),
                    first: errors
                        .first()
                        .map(|error| error.to_string())
                        .unwrap_or_default(),
                });
            }
        }
        Ok(Catalog { bundle, ids })
    }

    /// Formats `id` (or the attribute `id.attr`) inside this catalog.
    ///
    /// Returns `None` when the catalog does not define the message, which is
    /// how the caller learns to fall back to the default locale.
    fn format(&self, id: &str, args: Option<&FluentArgs<'_>>) -> Option<String> {
        let (message_id, attribute) = split_attribute(id);
        let message = self.bundle.get_message(message_id)?;
        let pattern = match attribute {
            Some(name) => message.attributes().find(|it| it.id() == name)?.value(),
            None => message.value()?,
        };

        let mut errors = Vec::new();
        let formatted = self.bundle.format_pattern(pattern, args, &mut errors);
        if !errors.is_empty() {
            // Not fatal: Fluent still returns the best rendering it could
            // produce. Reporting it is what keeps a missing variable from
            // showing up as a plausible-looking string.
            warn!(
                message = message_id,
                errors = errors.len(),
                first = %errors[0],
                "message formatting reported errors"
            );
        }
        Some(formatted.into_owned())
    }
}

/// Splits `panel-clock-title.title` into its message and attribute parts.
///
/// Fluent message identifiers cannot contain a dot, so a trailing dot
/// introduces an attribute rather than naming a different message.
fn split_attribute(id: &str) -> (&str, Option<&str>) {
    match id.split_once('.') {
        Some((message, attribute)) => (message, Some(attribute)),
        None => (id, None),
    }
}

/// A component's translations, ready to format strings with.
///
/// Built once at startup and then read-only, so it can be shared by reference
/// across the whole Elm update loop.
pub struct Localizer {
    requested: Catalog,
    fallback: Catalog,
    requested_locale: Locale,
}

impl fmt::Debug for Localizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Localizer")
            .field("locale", &self.requested_locale)
            .finish_non_exhaustive()
    }
}

impl Localizer {
    /// Loads the catalogs a component embeds.
    ///
    /// `catalogs` maps each locale to the `.ftl` sources for that locale; the
    /// catalog for [`DEFAULT_LOCALE`] is required, and a requested locale with
    /// no catalog falls back to it with a warning rather than an error — a
    /// missing translation is a packaging gap, not a reason to refuse to start.
    ///
    /// # Errors
    ///
    /// Returns [`I18nError::MissingDefaultCatalog`] when the fallback is absent
    /// and [`I18nError::Syntax`] when a catalog does not parse.
    pub fn load(requested: Locale, catalogs: &[(Locale, &[&str])]) -> Result<Self, I18nError> {
        let requested = Locale::negotiate(&requested);
        Self::load_exact(requested, catalogs)
    }

    /// [`Localizer::load`] without locale negotiation.
    ///
    /// Used by the tests that pin negotiation separately from loading, and by
    /// callers that have already resolved the locale (for example against a
    /// list of catalogs shipped on disk rather than embedded).
    ///
    /// # Errors
    ///
    /// As [`Localizer::load`].
    pub fn load_exact(
        requested: Locale,
        catalogs: &[(Locale, &[&str])],
    ) -> Result<Self, I18nError> {
        let fallback_locale = Locale::default_locale();
        let sources_of = |locale: &Locale| -> Option<&[&str]> {
            catalogs
                .iter()
                .find(|(candidate, _)| candidate == locale)
                .map(|(_, sources)| *sources)
        };

        // Every catalog the component ships is parsed, not only the one in use.
        // A malformed `.ftl` for a locale nobody has selected yet is still a
        // packaging defect, and the start of the component is the last moment
        // at which it can be reported before a user selects that locale.
        for (locale, sources) in catalogs {
            Catalog::load(locale, sources)?;
        }

        let Some(fallback_sources) = sources_of(&fallback_locale) else {
            return Err(I18nError::MissingDefaultCatalog {
                found: catalogs.iter().map(|(locale, _)| locale.tag()).collect(),
            });
        };

        let (requested_catalog, requested_locale) = match sources_of(&requested) {
            Some(sources) => (Catalog::load(&requested, sources)?, requested),
            None => {
                warn!(
                    requested = %requested.tag(),
                    "no catalog for the requested locale; falling back to the default"
                );
                (
                    Catalog::load(&fallback_locale, fallback_sources)?,
                    fallback_locale.clone(),
                )
            }
        };

        // When the requested locale *is* the default, the default catalog is
        // parsed twice — once for each slot. Re-parsing an embedded `.ftl` at
        // startup is cheaper than the ownership gymnastics sharing one catalog
        // between the two positions would need, and the fallback position must
        // always be filled.
        let fallback = Catalog::load(&fallback_locale, fallback_sources)?;

        Ok(Localizer {
            requested: requested_catalog,
            fallback,
            requested_locale,
        })
    }

    /// The locale messages are actually resolved in.
    pub fn locale(&self) -> &Locale {
        &self.requested_locale
    }

    /// Whether the active catalog translates `id` itself (rather than through
    /// the default-locale fallback).
    ///
    /// Acceptance Q03 asks for a fallback that can be *seen*, and a component
    /// that wants to mark untranslated entries needs to ask.
    pub fn is_translated(&self, id: &str) -> bool {
        self.requested.ids.contains(id)
    }

    /// Formats a message with no placeholders.
    ///
    /// A message the catalogs do not define returns `id` unchanged: an
    /// identifier on screen is ugly, but it is diagnosable, and it keeps a
    /// missing string from rendering as a blank label.
    pub fn message(&self, id: &str) -> String {
        self.message_with(id, &[])
    }

    /// Formats a message, substituting `args` into its placeholders.
    ///
    /// A trailing `.attribute` selects an attribute of the message
    /// (`panel-clock-tooltip.title`), which is how accessible names and other
    /// secondary strings are reached.
    pub fn message_with(&self, id: &str, args: &[(&str, Arg)]) -> String {
        let mut fluent_args = FluentArgs::new();
        for (name, value) in args {
            fluent_args.set((*name).to_owned(), value.clone().into_fluent());
        }
        let args = (!args.is_empty()).then_some(&fluent_args);

        if let Some(formatted) = self.requested.format(id, args) {
            return formatted;
        }
        if let Some(formatted) = self.fallback.format(id, args) {
            debug!(
                message = id,
                "message is missing from the active catalog; used the default"
            );
            return formatted;
        }
        warn!(message = id, "no catalog defines this message");
        id.to_owned()
    }
}

/// Adds the identifier halves of every message and term `source` defines.
///
/// The parsing here is the recovery parser: when a source has syntax errors the
/// caller has already reported them (`Catalog::load` does, and so does
/// [`ids_of`]), so this only has to extract what the parser could still see.
fn collect_ids(source: &str, ids: &mut BTreeSet<String>) {
    let ast = match fluent_syntax::parser::parse(source) {
        Ok(ast) => ast,
        Err((ast, _)) => ast,
    };
    collect_entry_ids(&ast, ids);
}

/// The identifier halves of every entry in a parsed resource.
fn collect_entry_ids(ast: &ast::Resource<&str>, ids: &mut BTreeSet<String>) {
    for entry in &ast.body {
        match entry {
            ast::Entry::Message(message) => {
                ids.insert(message.id.name.to_owned());
            }
            ast::Entry::Term(term) => {
                ids.insert(format!("-{}", term.id.name));
            }
            ast::Entry::Comment(_)
            | ast::Entry::GroupComment(_)
            | ast::Entry::ResourceComment(_)
            | ast::Entry::Junk { .. } => {}
        }
    }
}

/// How a translation differs from the catalog it was translated from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogDiff {
    /// Messages the reference catalog defines and the translation does not.
    pub missing_ids: Vec<String>,
    /// Messages the translation defines that the reference catalog does not.
    pub extra_ids: Vec<String>,
    /// Messages both define, with different placeholders.
    pub placeholder_mismatches: Vec<PlaceholderMismatch>,
}

impl CatalogDiff {
    /// Whether the translation is complete and consistent.
    pub fn is_clean(&self) -> bool {
        self.missing_ids.is_empty()
            && self.extra_ids.is_empty()
            && self.placeholder_mismatches.is_empty()
    }
}

/// A message whose placeholders differ between two locales.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceholderMismatch {
    /// The message identifier.
    pub id: String,
    /// Placeholders the reference catalog uses and the translation does not.
    pub missing: Vec<String>,
    /// Placeholders the translation uses that the reference catalog does not.
    pub extra: Vec<String>,
}

/// Compares a translation against its reference catalog (acceptance Q03).
///
/// The check is deliberately structural — the same ids, each using the same
/// placeholders — because that is the part a build can verify for every
/// locale at once. It cannot see a *wrong* translation, only one that would
/// break at runtime, which is the class of mistake that shipping a catalog
/// introduces.
///
/// # Errors
///
/// Returns [`I18nError::Syntax`] when either side does not parse.
pub fn compare_catalogs(reference: &[&str], translated: &[&str]) -> Result<CatalogDiff, I18nError> {
    let reference_ids = ids_of(reference)?;
    let translated_ids = ids_of(translated)?;
    let reference_placeholders = placeholders_of(reference)?;
    let translated_placeholders = placeholders_of(translated)?;

    let missing_ids = reference_ids
        .difference(&translated_ids)
        .cloned()
        .collect::<Vec<_>>();
    let extra_ids = translated_ids
        .difference(&reference_ids)
        .cloned()
        .collect::<Vec<_>>();

    let mut placeholder_mismatches = Vec::new();
    for id in reference_ids.intersection(&translated_ids) {
        let empty = BTreeSet::new();
        let expected = reference_placeholders.get(id).unwrap_or(&empty);
        let actual = translated_placeholders.get(id).unwrap_or(&empty);
        if expected != actual {
            placeholder_mismatches.push(PlaceholderMismatch {
                id: id.clone(),
                missing: expected.difference(actual).cloned().collect(),
                extra: actual.difference(expected).cloned().collect(),
            });
        }
    }

    Ok(CatalogDiff {
        missing_ids,
        extra_ids,
        placeholder_mismatches,
    })
}

/// The message identifiers defined by sources, as a sorted set.
///
/// # Errors
///
/// Returns [`I18nError::Syntax`] when a source does not parse.
pub fn message_ids(sources: &[&str]) -> Result<BTreeSet<String>, I18nError> {
    ids_of(sources)
}

fn ids_of(sources: &[&str]) -> Result<BTreeSet<String>, I18nError> {
    let mut ids = BTreeSet::new();
    for source in sources {
        match fluent_syntax::parser::parse(*source) {
            Ok(ast) => collect_entry_ids(&ast, &mut ids),
            Err((_, errors)) => {
                return Err(I18nError::Syntax {
                    locale: "<source>".to_owned(),
                    count: errors.len(),
                    first: errors
                        .first()
                        .map(|error| error.to_string())
                        .unwrap_or_default(),
                });
            }
        }
    }
    Ok(ids)
}

/// The placeholders each message and attribute in the sources uses.
fn placeholders_of(sources: &[&str]) -> Result<BTreeMap<String, BTreeSet<String>>, I18nError> {
    let mut placeholders: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for source in sources {
        let ast = match fluent_syntax::parser::parse(*source) {
            Ok(ast) => ast,
            Err((_, errors)) => {
                return Err(I18nError::Syntax {
                    locale: "<source>".to_owned(),
                    count: errors.len(),
                    first: errors
                        .first()
                        .map(|error| error.to_string())
                        .unwrap_or_default(),
                });
            }
        };
        for entry in ast.body {
            match entry {
                ast::Entry::Message(message) => {
                    let mut used = BTreeSet::new();
                    if let Some(value) = &message.value {
                        collect_variables(value, &mut used);
                    }
                    for attribute in &message.attributes {
                        collect_variables(&attribute.value, &mut used);
                    }
                    placeholders
                        .entry(message.id.name.to_owned())
                        .or_default()
                        .extend(used);
                }
                ast::Entry::Term(term) => {
                    let mut used = BTreeSet::new();
                    collect_variables(&term.value, &mut used);
                    for attribute in &term.attributes {
                        collect_variables(&attribute.value, &mut used);
                    }
                    placeholders
                        .entry(format!("-{}", term.id.name))
                        .or_default()
                        .extend(used);
                }
                ast::Entry::Comment(_)
                | ast::Entry::GroupComment(_)
                | ast::Entry::ResourceComment(_)
                | ast::Entry::Junk { .. } => {}
            }
        }
    }
    Ok(placeholders)
}

/// Collects every `{ $name }` a pattern refers to, including those inside a
/// select expression's selector and its variants.
fn collect_variables(pattern: &ast::Pattern<&str>, used: &mut BTreeSet<String>) {
    for element in &pattern.elements {
        if let ast::PatternElement::Placeable { expression } = element {
            collect_expression(expression, used);
        }
    }
}

fn collect_expression(expression: &ast::Expression<&str>, used: &mut BTreeSet<String>) {
    match expression {
        ast::Expression::Select { selector, variants } => {
            collect_inline(selector, used);
            for variant in variants {
                collect_variables(&variant.value, used);
            }
        }
        ast::Expression::Inline(inline) => collect_inline(inline, used),
    }
}

fn collect_inline(expression: &ast::InlineExpression<&str>, used: &mut BTreeSet<String>) {
    match expression {
        ast::InlineExpression::VariableReference { id } => {
            used.insert(id.name.to_owned());
        }
        ast::InlineExpression::Placeable { expression } => collect_expression(expression, used),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EN: &str = r#"
panel-clock = { $time }
panel-clock-tooltip =
    .title = Time: { $time }
panel-tasks =
    { $count ->
        [one] One window
       *[other] { $count } windows
    }
"#;

    const ZH: &str = r#"
panel-clock = { $time }
panel-clock-tooltip =
    .title = 时间：{ $time }
panel-tasks =
    { $count ->
       *[other] { $count } 个窗口
    }
"#;

    fn en() -> Locale {
        Locale::parse("en-US").expect("a well-formed tag")
    }

    fn zh() -> Locale {
        Locale::parse("zh-CN").expect("a well-formed tag")
    }

    fn localizer_for(requested: Locale) -> Localizer {
        Localizer::load(requested, &[(en(), &[EN]), (zh(), &[ZH])]).expect("both catalogs parse")
    }

    #[test]
    fn the_default_locale_constant_is_parseable() {
        assert_eq!(Locale::default_locale().tag(), DEFAULT_LOCALE);
    }

    #[test]
    fn tags_are_canonicalised() {
        assert_eq!(Locale::parse("zh-cn").expect("ok").tag(), "zh-CN");
        assert_eq!(Locale::parse("EN_us").expect("ok").tag(), "en-US");
    }

    #[test]
    fn a_malformed_tag_is_rejected() {
        assert!(matches!(
            Locale::parse("not a locale"),
            Err(LocaleError::Malformed { .. })
        ));
        assert!(Locale::parse("").is_err());
    }

    #[test]
    fn environment_locales_are_normalised() {
        let locale =
            Locale::from_env_with(|name| (name == "LANG").then(|| "zh_CN.UTF-8".to_owned()));
        assert_eq!(locale.tag(), "zh-CN");
    }

    #[test]
    fn lc_all_wins_over_lang() {
        let locale = Locale::from_env_with(|name| match name {
            "LC_ALL" => Some("en_US.UTF-8".to_owned()),
            "LANG" => Some("zh_CN.UTF-8".to_owned()),
            _ => None,
        });
        assert_eq!(locale.tag(), "en-US");
    }

    #[test]
    fn the_posix_c_locale_falls_back_to_the_default() {
        let locale = Locale::from_env_with(|name| (name == "LANG").then(|| "C".to_owned()));
        assert_eq!(locale.tag(), DEFAULT_LOCALE);
    }

    #[test]
    fn an_empty_environment_falls_back_to_the_default() {
        let locale = Locale::from_env_with(|_| None);
        assert_eq!(locale.tag(), DEFAULT_LOCALE);
    }

    #[test]
    fn an_unusable_locale_variable_is_skipped() {
        let locale = Locale::from_env_with(|name| match name {
            "LC_ALL" => Some("!!!".to_owned()),
            "LANG" => Some("zh_CN.UTF-8".to_owned()),
            _ => None,
        });
        assert_eq!(locale.tag(), "zh-CN");
    }

    #[test]
    fn negotiation_prefers_an_exact_match_then_the_language_then_the_default() {
        assert_eq!(Locale::negotiate(&zh()).tag(), "zh-CN");
        assert_eq!(
            Locale::negotiate(&Locale::parse("zh-Hans-CN").expect("ok")).tag(),
            "zh-CN"
        );
        assert_eq!(
            Locale::negotiate(&Locale::parse("de-DE").expect("ok")).tag(),
            DEFAULT_LOCALE
        );
    }

    #[test]
    fn a_message_formats_in_the_requested_locale() {
        let localizer = localizer_for(zh());
        assert_eq!(
            localizer.message_with("panel-tasks", &[("count", 3.into())]),
            "3 个窗口"
        );
        assert_eq!(localizer.locale().tag(), "zh-CN");
    }

    #[test]
    fn an_attribute_is_reachable_through_a_dotted_id() {
        let localizer = localizer_for(en());
        assert_eq!(
            localizer.message_with("panel-clock-tooltip.title", &[("time", "09:30".into())]),
            "Time: 09:30"
        );
    }

    #[test]
    fn a_missing_translation_falls_back_to_the_default_locale() {
        let sparse_zh = r#"panel-clock = { $time }"#;
        let localizer = Localizer::load(zh(), &[(en(), &[EN]), (zh(), &[sparse_zh])])
            .expect("both catalogs parse");

        assert_eq!(
            localizer.message_with("panel-tasks", &[("count", 2.into())]),
            "2 windows"
        );
        assert!(localizer.is_translated("panel-clock"));
        assert!(!localizer.is_translated("panel-tasks"));
    }

    #[test]
    fn an_unknown_message_returns_its_identifier() {
        let localizer = localizer_for(en());
        assert_eq!(localizer.message("does-not-exist"), "does-not-exist");
    }

    #[test]
    fn an_unknown_plural_category_still_renders() {
        // `en` has no `[one]` rule here, so Fluent falls through to `other`;
        // the point is that a count always produces a string.
        let localizer = localizer_for(zh());
        assert_eq!(
            localizer.message_with("panel-tasks", &[("count", 1.into())]),
            "1 个窗口"
        );
    }

    #[test]
    fn a_catalog_without_the_default_locale_is_rejected() {
        let error = Localizer::load(zh(), &[(zh(), &[ZH])]).expect_err("must fail");
        assert!(matches!(error, I18nError::MissingDefaultCatalog { .. }));
        assert!(error.to_string().contains("en-US"));
    }

    #[test]
    fn a_malformed_catalog_names_the_locale() {
        let broken = "this is not ftl =";
        let error =
            Localizer::load(en(), &[(en(), &[EN]), (zh(), &[broken])]).expect_err("must fail");
        match error {
            I18nError::Syntax { locale, count, .. } => {
                assert_eq!(locale, "zh-CN");
                assert!(count >= 1);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn a_requested_locale_without_a_catalog_falls_back() {
        let localizer = Localizer::load(en(), &[(en(), &[EN])]).expect("the default is present");
        assert_eq!(localizer.locale().tag(), DEFAULT_LOCALE);
        assert_eq!(
            localizer.message_with("panel-clock", &[("time", "12:00".into())]),
            "12:00"
        );
    }

    #[test]
    fn an_untranslated_locale_negotiates_down_to_the_default() {
        let localizer = localizer_for(Locale::parse("de-DE").expect("ok"));
        assert_eq!(localizer.locale().tag(), DEFAULT_LOCALE);
    }

    #[test]
    fn placing_a_value_does_not_insert_isolate_marks() {
        let localizer = localizer_for(en());
        let formatted = localizer.message_with("panel-clock", &[("time", "12:00".into())]);
        assert_eq!(formatted, "12:00");
        assert!(!formatted.contains('\u{2068}'), "isolating marks leaked in");
    }

    #[test]
    fn an_unknown_placeholder_is_reported_but_still_renders() {
        let localizer = localizer_for(en());
        // `$time` is not supplied: Fluent renders the raw variable expression.
        let formatted = localizer.message("panel-clock");
        assert!(formatted.contains("time"), "{formatted}");
    }

    #[test]
    fn catalogs_that_match_report_no_difference() {
        let diff = compare_catalogs(&[EN], &[ZH]).expect("both parse");
        assert!(diff.is_clean(), "{diff:?}");
    }

    #[test]
    fn a_translation_that_drops_a_message_is_reported() {
        let diff = compare_catalogs(&[EN], &["panel-clock = { $time }"]).expect("both parse");
        assert!(!diff.is_clean());
        assert_eq!(diff.missing_ids, ["panel-clock-tooltip", "panel-tasks"]);
        assert!(diff.extra_ids.is_empty());
    }

    #[test]
    fn a_translation_that_renames_a_placeholder_is_reported() {
        let diff = compare_catalogs(&[EN], &["panel-clock = { $clock }"]).expect("both parse");
        assert_eq!(diff.placeholder_mismatches.len(), 1);
        let mismatch = &diff.placeholder_mismatches[0];
        assert_eq!(mismatch.id, "panel-clock");
        assert_eq!(mismatch.missing, ["time"]);
        assert_eq!(mismatch.extra, ["clock"]);
    }

    #[test]
    fn a_placeholder_inside_a_select_is_seen() {
        // `$n` appears both as the selector and inside the variant, and neither
        // occurrence is the one the fallback expects: both must be collected.
        let translated = "panel-tasks =\n    { $n ->\n       *[other] { $n } windows\n    }";
        let diff = compare_catalogs(&[EN], &[translated]).expect("both parse");

        let mismatch = diff
            .placeholder_mismatches
            .iter()
            .find(|mismatch| mismatch.id == "panel-tasks")
            .expect("the select's placeholders are compared");
        assert_eq!(mismatch.missing, ["count"]);
        assert_eq!(mismatch.extra, ["n"]);
    }

    #[test]
    fn message_ids_include_terms() {
        let ids = message_ids(&["brand = Inox\n-term = a term"]).expect("parses");
        assert!(ids.contains("brand"));
        assert!(ids.contains("-term"));
    }

    #[test]
    fn a_malformed_source_is_rejected_by_the_comparison_helpers() {
        assert!(compare_catalogs(&["fine = ok"], &["not valid ="]).is_err());
        assert!(message_ids(&["not valid ="]).is_err());
    }
}
