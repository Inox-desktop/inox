//! Structured logging setup shared by every Inox binary.
//!
//! Inox binaries must initialise `tracing` through this module instead of
//! configuring a subscriber themselves, so that log level, format and filtering
//! behave identically across the desktop. Initialisation never panics: a broken
//! filter or an already-installed subscriber is reported as a [`LoggingError`]
//! for the caller to propagate.

use tracing_subscriber::EnvFilter;

/// Environment variable holding the log filter directives.
pub const ENV_VAR: &str = "RUST_LOG";

/// Log filter used when [`ENV_VAR`] is unset, empty or whitespace only.
pub const DEFAULT_LEVEL: &str = "info";

/// Errors returned while configuring or installing the tracing subscriber.
#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    /// The filter directives could not be parsed.
    #[error("invalid log filter `{filter}`: {source}")]
    InvalidFilter {
        /// The rejected filter directives.
        filter: String,
        /// The underlying parse error.
        #[source]
        source: tracing_subscriber::filter::ParseError,
    },

    /// A global tracing subscriber was already installed by someone else.
    #[error("a global tracing subscriber is already installed")]
    AlreadyInitialised,
}

/// Build an [`EnvFilter`] from `directives`, falling back to `default`.
///
/// `directives` comes from [`ENV_VAR`] and is ignored when it is `None`, empty
/// or whitespace only, in which case `default` is used instead.
///
/// # Errors
///
/// Returns [`LoggingError::InvalidFilter`] when the selected directives cannot
/// be parsed.
pub fn env_filter(directives: Option<&str>, default: &str) -> Result<EnvFilter, LoggingError> {
    let selected = directives
        .map(str::trim)
        .filter(|directives| !directives.is_empty())
        .unwrap_or(default);

    EnvFilter::try_new(selected).map_err(|source| LoggingError::InvalidFilter {
        filter: selected.to_owned(),
        source,
    })
}

/// Install the global tracing subscriber, filtered by [`ENV_VAR`].
///
/// The default level is [`DEFAULT_LEVEL`]; users override it per component via
/// the environment, for example `RUST_LOG=inox_compositor=debug`.
///
/// # Errors
///
/// Returns [`LoggingError::InvalidFilter`] for unusable [`ENV_VAR`] contents and
/// [`LoggingError::AlreadyInitialised`] when a subscriber is already installed.
pub fn init() -> Result<(), LoggingError> {
    let directives = std::env::var(ENV_VAR).ok();
    let filter = env_filter(directives.as_deref(), DEFAULT_LEVEL)?;

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init()
        .map_err(|_| LoggingError::AlreadyInitialised)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_directives_win_over_default() {
        let filter = env_filter(Some("debug"), DEFAULT_LEVEL);

        assert!(filter.is_ok(), "expected `debug` to parse, got {filter:?}");
    }

    #[test]
    fn missing_blank_or_padded_directives_fall_back_to_default() {
        for directives in [None, Some(""), Some("   "), Some("\t\n")] {
            let filter = env_filter(directives, DEFAULT_LEVEL);

            assert!(
                filter.is_ok(),
                "expected fallback for {directives:?}, got {filter:?}"
            );
        }
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        let filter = env_filter(Some("  info  "), DEFAULT_LEVEL);

        assert!(
            filter.is_ok(),
            "expected `  info  ` to parse, got {filter:?}"
        );
    }

    #[test]
    fn invalid_directives_report_the_rejected_filter() {
        let error = env_filter(Some("inox=not-a-level"), DEFAULT_LEVEL)
            .expect_err("expected `inox=not-a-level` to be rejected");

        assert!(matches!(
            &error,
            LoggingError::InvalidFilter { filter, .. } if filter == "inox=not-a-level"
        ));
        assert!(error.to_string().contains("inox=not-a-level"));
    }
}
