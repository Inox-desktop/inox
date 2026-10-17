//! The error type every loader reports, and the two helpers that produce it.
//!
//! PRD §9.1 requires a parse failure to be locatable: it must name the file, the
//! line and the field path. [`ConfigError::Parse`] therefore carries all three,
//! and the field path comes from `serde_path_to_error`, so a bad value inside
//! `[input]` is reported as `input.keyboard_repeat_rate_hz` rather than as a
//! bare type error.
//!
//! This module also owns `read_text` and `parse`, the only two ways a
//! configuration file is turned into data, so that every loader — the root
//! file, the component files and the themes — reports failures identically.
//! They are crate-private and therefore deliberately not linked from here:
//! a public module must not link to private items (`rustdoc::private_intra_doc_links`).

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Everything that can go wrong while discovering, reading or parsing the
/// configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The Inox configuration directory could not be determined from the
    /// environment, because no home directory is available (PRD §9.1).
    #[error("cannot determine the Inox configuration directory: {source}")]
    Paths {
        /// The underlying XDG resolution failure.
        #[source]
        source: inox_common::paths::PathsError,
    },

    /// A configuration file exists but could not be read.
    #[error("cannot read `{path}`: {source}")]
    Read {
        /// The file that could not be read.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// A configuration file is not valid TOML for its schema.
    #[error("{}", render_parse(.path, .field.as_deref(), .line, .column, .message.as_str()))]
    Parse {
        /// The file that failed to parse.
        path: PathBuf,
        /// Dotted path of the offending field, when the parser reports one.
        field: Option<String>,
        /// One-based line of the offending value, when the parser reports one.
        line: Option<u32>,
        /// One-based column of the offending value, when the parser reports one.
        column: Option<u32>,
        /// What went wrong, as reported by the parser.
        message: String,
    },

    /// A theme name cannot be used as a single directory name.
    #[error(
        "invalid theme name `{name}`: expected a single directory name without separators, `.` or `..`"
    )]
    InvalidThemeName {
        /// The rejected name.
        name: String,
    },

    /// A theme was selected that has no file below the `themes` directory.
    #[error("theme `{name}` not found: `{path}` does not exist")]
    ThemeNotFound {
        /// The rejected name.
        name: String,
        /// The file that was looked for.
        path: PathBuf,
    },

    /// A `[components]` path points outside the configuration directory.
    #[error(
        "invalid component path `{path}`: expected a path relative to the configuration directory that does not escape it"
    )]
    InvalidComponentPath {
        /// The rejected path.
        path: PathBuf,
    },
}

/// Render the message of [`ConfigError::Parse`].
///
/// Everything the parser could tell us is included; nothing is invented for the
/// parts it left out.
fn render_parse(
    path: &Path,
    field: Option<&str>,
    line: &Option<u32>,
    column: &Option<u32>,
    message: &str,
) -> String {
    // The whole `path:line:column` is one quoted token, the way compilers print
    // a position, so it can be copied out and searched for as it stands.
    let mut location = path.display().to_string();

    if let Some(line) = line {
        location.push_str(&format!(":{line}"));
        if let Some(column) = column {
            location.push_str(&format!(":{column}"));
        }
    }

    let location = format!("`{location}`");

    let field = field
        .map(|field| format!(" (field `{field}`)"))
        .unwrap_or_default();

    format!("cannot parse {location}{field}: {message}")
}

/// Read a configuration file, reporting a missing file as `Ok(None)`.
///
/// Every configuration file is optional, so absence is not an error — but it is
/// also not the same as an empty file, and the caller has to decide what it
/// means (defaults for a component file, [`ConfigError::ThemeNotFound`] for a
/// theme).
///
/// # Errors
///
/// Returns [`ConfigError::Read`] for every other failure, for example a
/// permission problem or a path that is a directory rather than a file.
pub(crate) fn read_text(path: &Path) -> Result<Option<String>, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ConfigError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Deserialize TOML `text` that was read from `path`.
///
/// # Errors
///
/// Returns [`ConfigError::Parse`] for syntax errors as well as for values that
/// do not fit the schema.
pub(crate) fn parse<T>(path: &Path, text: &str) -> Result<T, ConfigError>
where
    T: serde::de::DeserializeOwned,
{
    serde_path_to_error::deserialize::<_, T>(toml::Deserializer::new(text)).map_err(|error| {
        // An empty path means the failure is at the document root, where there
        // is no field to name; `Path` renders itself as `.` in that case.
        let field = if error.path().iter().next().is_some() {
            Some(error.path().to_string())
        } else {
            None
        };

        let error = error.into_inner();
        let location = error.span().map(|span| line_column(text, span.start));

        ConfigError::Parse {
            path: path.to_path_buf(),
            field,
            line: location.map(|(line, _)| line),
            column: location.map(|(_, column)| column),
            message: error.message().to_owned(),
        }
    })
}

/// One-based line and column of the byte `offset` in `text`.
///
/// Characters are counted as characters, not as bytes, so a document with
/// non-ASCII text still points at the right column. Offsets that are not on a
/// character boundary, or beyond the end of the document, are reported as the
/// position just after the character that contains them, which keeps this total
/// for any input a parser may hand over.
fn line_column(text: &str, offset: usize) -> (u32, u32) {
    let mut line = 1;
    let mut column = 1;

    for (index, character) in text.char_indices() {
        if index >= offset {
            break;
        }

        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }

    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_column_counts_characters_rather_than_bytes() {
        // Two three-byte characters and three ASCII bytes precede the value; a
        // byte-based column would report 10 here.
        let text = "中中 = 1\n";
        let value = text.find('1').expect("the document contains the value");

        assert_eq!(line_column(text, value), (1, 6));
    }

    #[test]
    fn a_line_counts_newlines_rather_than_bytes() {
        let text = "first\nsecond\nthird";

        assert_eq!(line_column(text, 0), (1, 1));
        assert_eq!(line_column(text, 6), (2, 1));
        assert_eq!(line_column(text, 13), (3, 1));

        // An offset inside a character, or past the end of the document, still
        // resolves to a position instead of panicking.
        assert_eq!(line_column("中", 1), (1, 2));
        assert_eq!(line_column("中", 99), (1, 2));
    }
}

/// Helpers shared by the tests that assert on the position a parse error
/// reports.
///
/// Tests must derive the expected line and column from the document they write
/// rather than repeating a guessed number, so that a test keeps failing for the
/// right reason when the document changes.
#[cfg(test)]
pub(crate) mod test_support {
    /// One-based line of the first line of `contents` containing `needle`.
    pub(crate) fn line_of(contents: &str, needle: &str) -> u32 {
        u32::try_from(
            contents
                .lines()
                .position(|line| line.contains(needle))
                .expect("the document contains the needle"),
        )
        .expect("line fits in u32")
            + 1
    }

    /// One-based column at which `needle` starts on its line.
    pub(crate) fn column_of(contents: &str, needle: &str) -> u32 {
        let line = contents
            .lines()
            .find(|line| line.contains(needle))
            .expect("the document contains the needle");

        u32::try_from(line.find(needle).expect("found above")).expect("column fits in u32") + 1
    }
}
