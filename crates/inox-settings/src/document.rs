//! The configuration files as documents: what was read, and what a save writes.
//!
//! The settings application is the only component that *writes* the Inox
//! configuration, and it writes three files:
//! `config.toml`, `shell.toml` and `notifications.toml` ([`SettingFile`]).
//! Each is held as a [`Document`] — the path, the bytes as they were read, and
//! the [`Digest`] of those bytes — which is what lets a save be checked against
//! what the window actually saw.
//!
//! **Only the keys the user changed are rewritten.** A save produces the new
//! text by editing the document in place: the lines of the changed keys are
//! replaced with `key = value`, a key that was not there is appended to its
//! table, and every other line is copied byte for byte. That is what makes
//! `draft.rs`'s "a field the user never touched is never written back" true at
//! the level of the file: comments, blank lines, ordering and values this
//! component does not model survive a save untouched.
//!
//! A key is a key however the file spells it. The walk reads it as a bare,
//! quoted or dotted key below its table, as a dotted key at the top level, and as
//! a member of an inline table, and it reads the table's own header however that
//! is quoted. Hand-written files are a first-class case here, and the cost of
//! not reading a spelling is severe rather than cosmetic: the key would look
//! absent, be written a second time, and leave a file `inox-config` refuses to
//! parse — a session that cannot start.
//!
//! The price is that a comment on a line the user changed goes with the line
//! it annotated. That is deliberate: the alternative is a TOML round trip
//! through the typed model, which would rewrite every line, reorder every
//! table and drop every comment — a much larger edit than the user asked for.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};

use inox_config::{ConfigError, ConfigPaths, InoxConfig};

use crate::draft::Field;
use crate::persist::{Digest, Guard};

/// One of the four configuration files the settings application writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SettingFile {
    /// `compositor.toml`: input and window layout preferences.
    Compositor,
    /// `config.toml`: the theme selection and the session language.
    Root,
    /// `shell.toml`: the panel, the launcher and the wallpaper.
    Shell,
    /// `notifications.toml`: popups, timeouts, queue and policy.
    Notifications,
}

impl SettingFile {
    /// Every file, in the order they are listed.
    pub const ALL: [Self; 4] = [
        Self::Root,
        Self::Compositor,
        Self::Shell,
        Self::Notifications,
    ];

    /// The file's name, for a message or a log line.
    ///
    /// These are the names PRD §9.1 gives the layout and the same ones
    /// [`inox_config::ComponentFiles`] defaults to; a user who pointed
    /// `[components] shell` somewhere else gets a document under their own
    /// name, which is why the name in a message comes from the resolved path
    /// rather than from here.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Compositor => inox_config::paths::COMPOSITOR_FILE,
            Self::Root => inox_config::paths::ROOT_FILE,
            Self::Shell => inox_config::paths::SHELL_FILE,
            Self::Notifications => inox_config::paths::NOTIFICATIONS_FILE,
        }
    }

    /// Where the file is, resolved the same way [`InoxConfig::load`] resolves
    /// it: the top-level file from the configuration directory, the component
    /// files from the paths `config.toml` points at.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidComponentPath`] when a `[components]` path
    /// is absolute or escapes the configuration directory — the same rejection
    /// the loader applies, so a save can never write outside `~/.config/inox`.
    pub fn path(self, config: &InoxConfig, paths: &ConfigPaths) -> Result<PathBuf, ConfigError> {
        match self {
            Self::Compositor => paths.file(&config.root.components.compositor),
            Self::Root => Ok(paths.root_file()),
            Self::Shell => paths.file(&config.root.components.shell),
            Self::Notifications => paths.file(&config.root.components.notifications),
        }
    }
}

/// A value as it is written into a TOML file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Toml {
    /// An array, including arrays of inline tables.
    Array(Vec<Toml>),
    /// An inline table with explicit keys.
    Table(BTreeMap<String, Toml>),
    /// A finite, already parsed floating point value in canonical notation.
    Decimal(String),
    /// `true` or `false`.
    Bool(bool),
    /// A decimal integer.
    Integer(i64),
    /// A quoted string.
    String(String),
}

impl Toml {
    /// The value as TOML source.
    ///
    /// Strings are quoted and their quotes, backslashes and control characters
    /// escaped, so a value that came from a language tag or a file name can
    /// never produce a document that does not parse back.
    pub fn render(&self) -> String {
        match self {
            Self::Array(values) => format!(
                "[{}]",
                values
                    .iter()
                    .map(Toml::render)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Table(entries) => format!(
                "{{ {} }}",
                entries
                    .iter()
                    .map(|(key, value)| format!(
                        "{} = {}",
                        Toml::String(key.clone()).render(),
                        value.render()
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Decimal(value) => value.clone(),
            Self::Bool(value) => value.to_string(),
            Self::Integer(value) => value.to_string(),
            Self::String(value) => {
                let mut rendered = String::with_capacity(value.len() + 2);
                rendered.push('"');

                for character in value.chars() {
                    match character {
                        '"' => rendered.push_str("\\\""),
                        '\\' => rendered.push_str("\\\\"),
                        '\n' => rendered.push_str("\\n"),
                        '\r' => rendered.push_str("\\r"),
                        '\t' => rendered.push_str("\\t"),
                        character if character.is_control() => {
                            let _ = write!(rendered, "\\u{:04X}", u32::from(character));
                        }
                        character => rendered.push(character),
                    }
                }

                rendered.push('"');
                rendered
            }
        }
    }
}

/// One key to write, or one key to take away.
///
/// The dotted [`Field::path`] is the vocabulary `inox-config` uses in its parse
/// errors (`panel.height`, `timeouts.normal_ms`), so the message a user reads
/// in this window names the key the same way a startup failure would.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    field: Field,
    value: Option<Toml>,
}

impl Edit {
    /// Writes `value` at the field's key, creating the table when it is absent.
    pub fn set(field: Field, value: Toml) -> Self {
        Self {
            field,
            value: Some(value),
        }
    }

    /// Takes the field's key away, which is how "follow the environment" is
    /// written: the absence of `locale.language` is the setting.
    pub fn remove(field: Field) -> Self {
        Self { field, value: None }
    }

    /// The field this edit is about.
    pub const fn field(&self) -> Field {
        self.field
    }

    /// The value to write, or `None` to take the key away.
    pub const fn value(&self) -> Option<&Toml> {
        self.value.as_ref()
    }

    /// The dotted path of the key, below its file.
    pub const fn path(&self) -> &'static str {
        self.field.path()
    }

    /// The table the key lives in, or `None` for the file's top level.
    pub fn section(&self) -> Option<&'static str> {
        self.path().rsplit_once('.').map(|(section, _)| section)
    }

    /// The key itself.
    pub fn key(&self) -> &'static str {
        self.path()
            .rsplit_once('.')
            .map_or_else(|| self.path(), |(_, key)| key)
    }
}

/// Why a configuration file could not be read as a document.
#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    /// The file exists but could not be read.
    #[error("cannot read `{path}`: {source}")]
    Read {
        /// The file that could not be read.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// A `[components]` path was rejected before anything was opened.
    #[error(transparent)]
    Paths(#[from] ConfigError),
}

/// A configuration file as it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    path: PathBuf,
    text: String,
    digest: Digest,
}

impl Document {
    /// Reads a configuration file.
    ///
    /// A file that is not there is not an error — every Inox configuration file
    /// is optional and falls back to its defaults (ADR 0009, decision 8) — so
    /// it is held as an empty document whose digest is [`Digest::Absent`], and
    /// the first save of a changed key creates it.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentError::Read`] when the file exists but cannot be read,
    /// including when it is not valid UTF-8: a configuration file this
    /// component cannot decode is a file it must not overwrite.
    pub fn read(path: PathBuf) -> Result<Self, DocumentError> {
        match std::fs::read(&path) {
            Ok(bytes) => {
                let digest = Digest::of(&bytes);
                let text = String::from_utf8(bytes).map_err(|error| DocumentError::Read {
                    path: path.clone(),
                    source: std::io::Error::new(std::io::ErrorKind::InvalidData, error),
                })?;

                Ok(Self { path, text, digest })
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                path,
                text: String::new(),
                digest: Digest::Absent,
            }),
            Err(source) => Err(DocumentError::Read { path, source }),
        }
    }

    /// The file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file's content as it was read.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The digest of the content as it was read.
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    /// The guard a save of this document has to carry.
    pub const fn guard(&self) -> Guard {
        Guard::new(self.digest)
    }

    /// The content a save of `edits` would write.
    ///
    /// The document as it stands, with only the keys `edits` name touched: a
    /// save of a key the document already holds at that value writes the same
    /// bytes back, and no other line changes.
    pub fn patch(&self, edits: &[Edit]) -> String {
        patch(&self.text, edits)
    }
}

/// Every configuration file the settings application writes, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Documents {
    files: BTreeMap<SettingFile, Document>,
}

impl Documents {
    /// Reads all three files.
    ///
    /// # Errors
    ///
    /// Returns an error as soon as one of them cannot be read or its path is
    /// rejected; a window that is missing one of its documents cannot honestly
    /// report what it is about to change.
    pub fn open(config: &InoxConfig, paths: &ConfigPaths) -> Result<Self, DocumentError> {
        let mut files = BTreeMap::new();

        for file in SettingFile::ALL {
            files.insert(file, Document::read(file.path(config, paths)?)?);
        }

        Ok(Self { files })
    }

    /// The document for `file`.
    pub fn get(&self, file: SettingFile) -> Option<&Document> {
        self.files.get(&file)
    }

    /// The content a save of `edits` would write to `file`.
    pub fn patch(&self, file: SettingFile, edits: &[Edit]) -> Option<String> {
        self.files.get(&file).map(|document| document.patch(edits))
    }

    /// Installs the content a successful save wrote.
    ///
    /// The digest is recomputed from the same bytes that were handed to
    /// [`crate::persist::save`], so a second save of the same document compares
    /// against the file the first one produced.
    pub fn install(&mut self, file: SettingFile, content: String) {
        if let Some(document) = self.files.get_mut(&file) {
            document.digest = Digest::of(content.as_bytes());
            document.text = content;
        }
    }
}

/// Edits a TOML document in place.
///
/// Only the keys named by `edits` are touched. The walk reads a key in every
/// shape TOML gives it, because these files are hand-written as often as they
/// are written by this component, and a key spelled some other way is still the
/// same key: a bare, quoted or dotted key below a `[table]` header (however that
/// header is spelled), a dotted key at the top level, and a member of an inline
/// table. That completeness is load-bearing rather than cosmetic — a key this
/// walk did not recognise was answered by appending a *second* definition of it,
/// and a `shell.toml` holding `panel.height` twice is a file `inox-config`
/// refuses to read, which is a session that cannot start. A line the walk does
/// not read at all is copied through unchanged.
///
/// Three rules the tests pin down:
///
/// - a key that is already there is rewritten **where it stands**, so the
///   document keeps its order;
/// - a key that is missing is added beside its siblings: at the end of its own
///   table, in the dotted shape the file itself uses for that table, or — when
///   the table is not there either — in a block appended to the end of the file;
/// - the document keeps its final newline if it had one, and a file this
///   application creates ends with one.
pub fn patch(text: &str, edits: &[Edit]) -> String {
    if edits.iter().any(|edit| edit.field().is_collection()) {
        // These documents have already passed the authoritative configuration loader.
        // Parse failures are checked again by the runtime before any save.
        match text.parse::<toml_edit::DocumentMut>() {
            Ok(mut document) => {
                for edit in edits.iter().filter(|edit| edit.field().is_collection()) {
                    let keys: Vec<_> = edit.path().split('.').collect();
                    let mut table: &mut dyn toml_edit::TableLike = document.as_table_mut();
                    for key in &keys[..keys.len() - 1] {
                        if !table.contains_key(key) {
                            table.insert(key, toml_edit::Item::Table(toml_edit::Table::new()));
                        }
                        let Some(next) = table
                            .get_mut(key)
                            .and_then(toml_edit::Item::as_table_like_mut)
                        else {
                            return patch_collection_inline(text, edits);
                        };
                        table = next;
                    }
                    if let Some(key) = keys.last() {
                        if let Some(value) = edit.value() {
                            let source = format!("value = {}", value.render());
                            match source.parse::<toml_edit::DocumentMut>() {
                                Ok(mut value_doc) => {
                                    if let Some(item) = value_doc.remove("value") {
                                        table.insert(key, item);
                                    }
                                }
                                Err(error) => {
                                    tracing::error!(%error, "invalid collection edit");
                                    return text.to_owned();
                                }
                            }
                        } else {
                            table.remove(key);
                        }
                    }
                }
                let scalars: Vec<_> = edits
                    .iter()
                    .filter(|edit| !edit.field().is_collection())
                    .cloned()
                    .collect();
                return patch(&document.to_string(), &scalars);
            }
            Err(error) => {
                tracing::error!(%error, "cannot patch configuration collection");
                return text.to_owned();
            }
        }
    }

    patch_scalars(text, edits)
}

fn patch_scalars(text: &str, edits: &[Edit]) -> String {
    if edits.is_empty() {
        return text.to_owned();
    }

    // `split` yields one element per line plus a final empty one when the text
    // ends with a newline; that final element is the newline, not a line. An
    // empty document has no lines at all.
    let mut lines: Vec<String> = if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').map(str::to_owned).collect()
    };
    let had_final_newline = lines.len() > 1 && lines.last().is_some_and(String::is_empty);
    if had_final_newline {
        lines.pop();
    }

    let mut written: Vec<bool> = vec![false; edits.len()];

    for (index, edit) in edits.iter().enumerate() {
        // The document is read again for every edit, and each one is applied
        // before the next is placed: an edit that added a member to an inline
        // table moves where its siblings go, and one that took a line away
        // shifts every line below it, so a placement worked out earlier is not
        // a placement of the document as it now stands.
        let views = lines_of(&lines);

        match place(&lines, &views, edit) {
            Some(Placement::Replace { line, range, text }) => {
                lines[line].replace_range(range, &text);
                written[index] = true;
            }
            Some(Placement::Remove(line)) => {
                lines.remove(line);
                written[index] = true;
            }
            None => {}
        }
    }

    insert_missing(&mut lines, edits, &written);

    let mut result = lines.join("\n");
    if result.is_empty() {
        return result;
    }
    if had_final_newline || text.is_empty() {
        // A file this component creates ends with a newline; a file that already
        // existed keeps the one it had (or does not gain one).
        result.push('\n');
    }

    result
}

fn patch_collection_inline(text: &str, edits: &[Edit]) -> String {
    // The scalar walker handles inline table members as well as scalar values.
    patch_scalars(text, edits)
}

/// A key path: the components a dotted key or a `[table]` header names, with the
/// quoting taken off.
///
/// This is what makes `"height"`, `'height'` and `height` one key, and `panel`
/// below `["panel"]` the same table as below `[panel]`.
type KeyPath = Vec<String>;

/// One `key = value` in the document, as the walk reads it.
#[derive(Debug)]
struct Entry {
    /// The components the key names, with the quoting taken off.
    path: KeyPath,
    /// Where the entry's text begins: the key's first byte on the line.
    start: usize,
    /// Where the key's text ends, just before the `=`.
    key_end: usize,
    /// Where the value begins.
    value_at: usize,
    /// Where the value's text ends: the end of the line for an entry that is a
    /// line, and the last character of the value for a member of an inline table.
    value_end: usize,
    /// The inline table the value is, when it is one.
    table: Option<InlineTable>,
}

/// A `{ ... }` value, and the keys it holds.
#[derive(Debug)]
struct InlineTable {
    /// The byte offsets of the `{` and of the `}` that closes it.
    open: usize,
    /// The `}` that closes it.
    close: usize,
    /// Its members, in the order they are written.
    members: Vec<Entry>,
}

/// What one line of the document is.
#[derive(Debug)]
enum LineKind {
    /// A table declaration: `[a.b]` or `[[a.b]]`.
    Header(KeyPath),
    /// A `key = value`.
    Entry(Entry),
    /// A blank line, a comment, a string's own content, or a shape this walk does
    /// not read.
    Other,
}

/// One line of the document, once it is read.
#[derive(Debug)]
struct LineView {
    /// The table the line's keys belong to: the last header above it.
    section: Option<KeyPath>,
    /// Whether the line is itself a table declaration.
    declares: bool,
    /// The `key = value` the line is, when it is one.
    entry: Option<Entry>,
}

/// What one edit does to the document.
#[derive(Debug)]
enum Placement {
    /// Replaces the bytes of `line` in `range` with `text`.
    Replace {
        /// The line.
        line: usize,
        /// The bytes of that line to take away.
        range: Range<usize>,
        /// What takes their place.
        text: String,
    },
    /// Takes one whole line away.
    Remove(usize),
}

/// Reads the document, line by line.
fn lines_of(lines: &[String]) -> Vec<LineView> {
    let mut section: Option<KeyPath> = None;
    let mut open: Option<char> = None;
    let mut views = Vec::with_capacity(lines.len());

    for line in lines {
        let (kind, next) = scan_line(line, open);
        open = next;

        match kind {
            LineKind::Header(path) => {
                // A header names the table the lines below it belong to. An array
                // of tables names it too: it is not a shape this application
                // writes, and reading it as a table is what keeps a save from
                // adding a second table of the same name.
                section = Some(path);
                views.push(LineView {
                    section: section.clone(),
                    declares: true,
                    entry: None,
                });
            }
            LineKind::Entry(entry) => views.push(LineView {
                section: section.clone(),
                declares: false,
                entry: Some(entry),
            }),
            LineKind::Other => views.push(LineView {
                section: section.clone(),
                declares: false,
                entry: None,
            }),
        }
    }

    views
}

/// Reads one line: what it is, and the string it leaves open for the line below.
fn scan_line(line: &str, open: Option<char>) -> (LineKind, Option<char>) {
    if let Some(quote) = open {
        // Inside a `"""` or `'''` string every line is content until the run of
        // quotes that closes it, so nothing written there is a key or a header.
        let closer = if quote == '"' { "\"\"\"" } else { "'''" };

        return match line.contains(closer) {
            true => (LineKind::Other, None),
            false => (LineKind::Other, Some(quote)),
        };
    }

    if let Some(path) = header_at(line) {
        return (LineKind::Header(path), None);
    }

    let Some(entry) = entry_of(line) else {
        return (LineKind::Other, None);
    };

    let open = open_multiline(line, entry.value_at);

    (LineKind::Entry(entry), open)
}

/// The string one line leaves open for the lines below it.
///
/// A `"""` or `'''` value that does not close on its own line means the rest of
/// the file, until the quotes that close it, is that string's content rather
/// than a key this walk may read.
fn open_multiline(line: &str, value_at: usize) -> Option<char> {
    let value = line.get(value_at..)?;
    let quote = match value.as_bytes() {
        [b'"', b'"', b'"', ..] => '"',
        [b'\'', b'\'', b'\'', ..] => '\'',
        _ => return None,
    };
    let closer = if quote == '"' { "\"\"\"" } else { "'''" };

    // The three characters that opened the string are not the ones that close it.
    (!value[3..].contains(closer)).then_some(quote)
}

/// The table a header line declares.
///
/// `[a.b]`, `["a"."b"]` and `[[a.b]]` all name the table `a.b`.
fn header_at(line: &str) -> Option<KeyPath> {
    let start = skip_spaces(line, 0);

    let (closing, at) = if line[start..].starts_with("[[") {
        ("]]", start + 2)
    } else if line[start..].starts_with('[') {
        ("]", start + 1)
    } else {
        return None;
    };

    let (end, path) = key_at(line, at)?;
    let rest = line[end..].trim_start().strip_prefix(closing)?;
    let trailing = rest.trim_start();

    (trailing.is_empty() || trailing.starts_with('#')).then_some(path)
}

/// Reads a `key = value` line, when the line is one.
///
/// A blank line, a comment, a header, and any shape this walk does not read have
/// no entry, and are copied through unchanged.
fn entry_of(line: &str) -> Option<Entry> {
    let bytes = line.as_bytes();
    let start = skip_spaces(line, 0);
    let (key_end, path) = key_at(line, start)?;
    let equals = skip_spaces(line, key_end);

    if bytes.get(equals) != Some(&b'=') {
        return None;
    }

    let value_at = skip_spaces(line, equals + 1);
    let table = if bytes.get(value_at) == Some(&b'{') {
        inline_table(line, value_at)
    } else {
        None
    };

    Some(Entry {
        path,
        start,
        key_end,
        value_at,
        // The line the user changed is rewritten whole, so a comment that
        // annotated its value goes with the value (see `patch`).
        value_end: line.len(),
        table,
    })
}

/// Reads a key — one or more components separated by dots — starting at `at`.
///
/// Returns where the key's own text ends and the components it names, or `None`
/// when no key starts there.
fn key_at(line: &str, at: usize) -> Option<(usize, KeyPath)> {
    let mut path = KeyPath::new();
    let mut cursor = at;
    let mut end;

    loop {
        cursor = skip_spaces(line, cursor);
        let (component, next) = component_at(line, cursor)?;
        path.push(component);
        end = next;
        cursor = skip_spaces(line, next);

        if line.as_bytes().get(cursor) == Some(&b'.') {
            cursor += 1;
        } else {
            return Some((end, path));
        }
    }
}

/// One key component: a bare key, or a quoted one.
///
/// A quoted component is read back to the text it names, so that `"height"` and
/// `height` compare as one key — and an escape is not a way of writing a key
/// this walk would fail to see.
fn component_at(line: &str, at: usize) -> Option<(String, usize)> {
    match line.as_bytes().get(at)? {
        b'"' | b'\'' => {
            let end = string_end(line, at)?;

            Some((quoted_key(line, at, end)?, end))
        }
        _ => bare_key(line, at),
    }
}

/// A bare key: the letters, digits, `_` and `-` TOML allows unquoted.
fn bare_key(line: &str, at: usize) -> Option<(String, usize)> {
    let bytes = line.as_bytes();
    let mut end = at;

    while end < bytes.len() && is_bare_key_byte(bytes[end]) {
        end += 1;
    }

    (end > at).then(|| (line[at..end].to_owned(), end))
}

/// Whether a byte may stand bare in a key.
fn is_bare_key_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

/// Where a `"..."` or `'...'` string that starts at `at` ends, quotes included.
fn string_end(line: &str, at: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    let quote = *bytes.get(at)?;
    let mut cursor = at + 1;

    while cursor < bytes.len() {
        match bytes[cursor] {
            // An escape covers the character after it, so the `"` of a `\"` is
            // not the end of the string.
            b'\\' if quote == b'"' => cursor += 2,
            byte if byte == quote => return Some(cursor + 1),
            _ => cursor += 1,
        }
    }

    None
}

/// The text a quoted key names, with its quoting and its escapes taken off.
fn quoted_key(line: &str, at: usize, end: usize) -> Option<String> {
    let raw = line.get(at + 1..end.checked_sub(1)?)?;

    // A literal string has no escapes: `'a.b'` is the key `a.b`, and nothing else.
    if line.as_bytes().get(at) == Some(&b'\'') {
        return Some(raw.to_owned());
    }

    let mut text = String::with_capacity(raw.len());
    let mut characters = raw.chars();

    while let Some(character) = characters.next() {
        if character != '\\' {
            text.push(character);
            continue;
        }

        let escape = characters.next()?;

        match escape {
            '"' => text.push('"'),
            '\\' => text.push('\\'),
            'b' => text.push('\u{8}'),
            't' => text.push('\t'),
            'n' => text.push('\n'),
            'f' => text.push('\u{c}'),
            'r' => text.push('\r'),
            'u' | 'U' => {
                let length = if escape == 'u' { 4 } else { 8 };
                let digits: String = characters.by_ref().take(length).collect();

                if digits.len() != length {
                    return None;
                }

                text.push(char::from_u32(u32::from_str_radix(&digits, 16).ok()?)?);
            }
            _ => return None,
        }
    }

    Some(text)
}

/// Reads a `{ ... }` value, and the keys it holds.
///
/// An inline table is one line in TOML — it can hold neither a comment nor a line
/// break — so its members are read here, below the line walk.
fn inline_table(line: &str, open: usize) -> Option<InlineTable> {
    let mut members: Vec<Entry> = Vec::new();
    let mut cursor = open + 1;
    let close;

    loop {
        cursor = skip_spaces(line, cursor);

        match line.as_bytes().get(cursor)? {
            b'}' => {
                close = cursor;
                break;
            }
            b',' => cursor += 1,
            _ => {
                let (key_end, path) = key_at(line, cursor)?;
                let equals = skip_spaces(line, key_end);

                if line.as_bytes().get(equals) != Some(&b'=') {
                    return None;
                }

                let value_at = skip_spaces(line, equals + 1);
                let table = if line.as_bytes().get(value_at) == Some(&b'{') {
                    inline_table(line, value_at)
                } else {
                    None
                };
                let value_end = match &table {
                    Some(inner) => inner.close + 1,
                    None => scalar_end(line, value_at)?,
                };

                members.push(Entry {
                    path,
                    start: cursor,
                    key_end,
                    value_at,
                    value_end,
                    table,
                });

                cursor = value_end;
            }
        }
    }

    Some(InlineTable {
        open,
        close,
        members,
    })
}

/// Where a scalar value inside an inline table ends.
///
/// The value runs to the comma or the `}` that ends it, with a string's own
/// commas and braces skipped: `panel = { format = "a,b", height = 12 }`.
fn scalar_end(line: &str, at: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut cursor = at;
    let mut end = at;

    while cursor < bytes.len() {
        match bytes[cursor] {
            b',' | b'}' => return Some(end),
            b'"' | b'\'' => {
                cursor = string_end(line, cursor)?;
                end = cursor;
            }
            // Trailing whitespace is not part of the value, and whitespace inside
            // one (`1979-05-27 07:32:00`) is followed by more of it.
            b' ' | b'\t' => cursor += 1,
            _ => {
                cursor += 1;
                end = cursor;
            }
        }
    }

    None
}

/// The first byte at or after `at` that is not a space or a tab.
fn skip_spaces(line: &str, at: usize) -> usize {
    let bytes = line.as_bytes();
    let mut cursor = at;

    while matches!(bytes.get(cursor), Some(&b' ') | Some(&b'\t')) {
        cursor += 1;
    }

    cursor
}

/// The whole path of the key an edit names, below its file.
fn target_of(edit: &Edit) -> KeyPath {
    edit.path().split('.').map(str::to_owned).collect()
}

/// The table the key an edit names lives in.
///
/// The paths of this component's fields are bare and two components long, so the
/// text before the last dot is the table — the same vocabulary
/// [`inox_config`] uses in a parse error.
fn table_of(edit: &Edit) -> Option<KeyPath> {
    edit.section()
        .map(|table| table.split('.').map(str::to_owned).collect())
}

/// Where one edit goes, or `None` when the document does not hold its key.
fn place(lines: &[String], views: &[LineView], edit: &Edit) -> Option<Placement> {
    let target = target_of(edit);

    // A key the document has, in whatever shape it spells it: rewritten where it
    // stands, or taken away with its line.
    for (index, view) in views.iter().enumerate() {
        let Some(entry) = &view.entry else {
            continue;
        };

        if absolute(&view.section, &entry.path) != target {
            continue;
        }

        return Some(match edit.value() {
            Some(value) => Placement::Replace {
                line: index,
                range: entry.start..lines[index].len(),
                // The key keeps the spelling the file gave it, so a quoted or
                // dotted key is not turned into a second definition of itself.
                text: format!(
                    "{} = {}",
                    &lines[index][entry.start..entry.key_end],
                    value.render()
                ),
            },
            None => Placement::Remove(index),
        });
    }

    // A key of a table the file spells inline: `panel = { height = 12 }`.
    for (index, view) in views.iter().enumerate() {
        let Some(entry) = &view.entry else {
            continue;
        };

        let path = absolute(&view.section, &entry.path);

        if let Some(table) = &entry.table
            && target.len() > path.len()
            && target.starts_with(&path)
        {
            return splice(lines, index, table, &target[path.len()..], edit.value());
        }
    }

    None
}

/// Places a value inside an inline table, `rest` below the key it holds.
///
/// A member the table holds is rewritten where it stands, a member it does not
/// hold is written in beside the others, and a removal takes the member and the
/// comma that separated it away.
fn splice(
    lines: &[String],
    line: usize,
    table: &InlineTable,
    rest: &[String],
    value: Option<&Toml>,
) -> Option<Placement> {
    let (component, below) = rest.split_first()?;

    let found = table
        .members
        .iter()
        .enumerate()
        .find(|(_, member)| member.path.len() == 1 && member.path.first() == Some(component));

    let Some((position, member)) = found else {
        // A key the inline table does not hold is written into it, beside the
        // members it does hold.
        let value = value?;
        let last = table.members.last();
        let written = inline_text(component, below, value);
        let at = last.map_or(table.open + 1, |member| member.value_end);
        let text = match last {
            Some(_) => format!(", {written}"),
            None if lines[line][table.open + 1..table.close].is_empty() => {
                format!(" {written} ")
            }
            None => format!(" {written}"),
        };

        return Some(Placement::Replace {
            line,
            range: at..at,
            text,
        });
    };

    match (below, value) {
        ([], Some(value)) => Some(Placement::Replace {
            line,
            range: member.value_at..member.value_end,
            text: value.render(),
        }),
        ([], None) => Some(Placement::Replace {
            line,
            range: member_range(table, position, member),
            text: String::new(),
        }),
        // A key even deeper: the member has to be a table itself.
        (_, _) => member
            .table
            .as_ref()
            .and_then(|inner| splice(lines, line, inner, below, value)),
    }
}

/// One member of an inline table, with the tables the key is deeper in wrapped
/// around it.
fn inline_text(component: &str, below: &[String], value: &Toml) -> String {
    below.iter().rev().fold(
        format!("{component} = {}", value.render()),
        |text, table| format!("{table} = {{ {text} }}"),
    )
}

/// The bytes a member of an inline table takes with it when it goes.
///
/// The comma that separated it from its neighbour goes as well, so that the
/// table it leaves behind is still one TOML can read.
fn member_range(table: &InlineTable, position: usize, member: &Entry) -> Range<usize> {
    if let Some(next) = table.members.get(position + 1) {
        member.start..next.start
    } else if let Some(previous) = position
        .checked_sub(1)
        .and_then(|index| table.members.get(index))
    {
        previous.value_end..member.value_end
    } else {
        member.start..member.value_end
    }
}

/// A key's path from the document's root: its table, then the key itself.
fn absolute(section: &Option<KeyPath>, path: &[String]) -> KeyPath {
    let mut absolute = section.clone().unwrap_or_default();
    absolute.extend_from_slice(path);

    absolute
}

/// Adds the keys the document does not hold.
///
/// A key goes where its siblings are: into the `[table]` the file declares, into
/// the dotted spellings it uses when it spells that table without a header, or
/// into a block appended to the end of the file when the table is not there at
/// all. A key the walk did write is not added again, and a removal that found
/// nothing adds nothing.
fn insert_missing(lines: &mut Vec<String>, edits: &[Edit], written: &[bool]) {
    // The edits are grouped by the table they belong to, so that one block is
    // built per table and its keys keep the order the fields are declared in.
    let mut groups: BTreeMap<Option<KeyPath>, Vec<&Edit>> = BTreeMap::new();

    for (index, edit) in edits.iter().enumerate() {
        if !written[index] && edit.value().is_some() {
            groups.entry(table_of(edit)).or_default().push(edit);
        }
    }

    for (table, edits) in groups {
        let views = lines_of(lines);
        let (at, block) = block_for(lines, &views, table.as_deref(), &edits);

        lines.splice(at..at, block);
    }
}

/// Where a group of new keys goes, and the lines that go there.
fn block_for(
    lines: &[String],
    views: &[LineView],
    table: Option<&[String]>,
    edits: &[&Edit],
) -> (usize, Vec<String>) {
    let Some(table) = table else {
        // A key of the file's own top level.
        return (top_level_at(views), key_lines(edits));
    };

    if declared_by_header(views, table) {
        return (table_end(views, lines, table), key_lines(edits));
    }

    if spelled_as_dotted_keys(views, table) {
        // A table the file spells as dotted keys has to keep them dotted: a
        // `[panel]` header beside `panel.height` is a second definition of the
        // same table, which is a document nothing can read.
        return (top_level_at(views), dotted_lines(edits));
    }

    let mut block: Vec<String> = Vec::new();
    // A table appended to the end of a file gets a blank line in front of it,
    // which is how the rest of the file separates its tables.
    if lines.last().is_some_and(|line| !line.trim().is_empty()) {
        block.push(String::new());
    }
    block.push(format!("[{}]", table.join(".")));
    block.extend(key_lines(edits));

    (lines.len(), block)
}

/// `key = value` for each edit, in the order they were given.
fn key_lines(edits: &[&Edit]) -> Vec<String> {
    edits
        .iter()
        .filter_map(|edit| {
            edit.value()
                .map(|value| format!("{} = {}", edit.key(), value.render()))
        })
        .collect()
}

/// `table.key = value` for each edit — the shape the file uses for that table.
fn dotted_lines(edits: &[&Edit]) -> Vec<String> {
    edits
        .iter()
        .filter_map(|edit| {
            edit.value()
                .map(|value| format!("{} = {}", edit.path(), value.render()))
        })
        .collect()
}

/// Whether the file declares `table` with a header.
fn declared_by_header(views: &[LineView], table: &[String]) -> bool {
    views
        .iter()
        .any(|view| view.declares && view.section.as_deref() == Some(table))
}

/// Whether the file spells `table`'s keys as dotted keys at the top level.
///
/// `panel.height = 12` is the table `panel` written without a header, and a
/// `[panel]` header beside it would be a second definition of that table.
fn spelled_as_dotted_keys(views: &[LineView], table: &[String]) -> bool {
    views.iter().any(|view| {
        view.section.is_none()
            && view.entry.as_ref().is_some_and(|entry| {
                entry.path.len() > table.len() && entry.path.starts_with(table)
            })
    })
}

/// Where a new key of `table` goes: at the end of that table, before the blank
/// line that separates it from the next one.
fn table_end(views: &[LineView], lines: &[String], table: &[String]) -> usize {
    let Some(first) = views
        .iter()
        .position(|view| view.section.as_deref() == Some(table))
    else {
        return lines.len();
    };

    let last = views
        .iter()
        .rposition(|view| view.section.as_deref() == Some(table))
        .unwrap_or(first);
    let mut at = last + 1;

    while at > first + 1 && lines[at - 1].trim().is_empty() {
        at -= 1;
    }

    at
}

/// Where a top-level key goes: the end of the document's own lines, before the
/// first table.
fn top_level_at(views: &[LineView]) -> usize {
    views
        .iter()
        .position(|view| view.section.is_some())
        .unwrap_or(views.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::{Field, Value};

    fn edit(field: Field, value: Value) -> Edit {
        match field
            .toml_value(&value)
            .expect("a value of the right shape")
        {
            Some(value) => Edit::set(field, value),
            None => Edit::remove(field),
        }
    }

    #[test]
    fn a_boolean_is_written_the_way_toml_writes_it() {
        assert_eq!(Toml::Bool(true).render(), "true");
        assert_eq!(Toml::Bool(false).render(), "false");
    }

    #[test]
    fn an_integer_is_written_as_a_decimal() {
        assert_eq!(Toml::Integer(0).render(), "0");
        assert_eq!(Toml::Integer(-12).render(), "-12");
        assert_eq!(Toml::Integer(600_000).render(), "600000");
    }

    #[test]
    fn a_string_is_quoted_and_escaped() {
        assert_eq!(Toml::String("zh-CN".to_owned()).render(), "\"zh-CN\"");
        assert_eq!(
            Toml::String("a \"b\" \\ c\nd".to_owned()).render(),
            "\"a \\\"b\\\" \\\\ c\\nd\""
        );
        assert_eq!(Toml::String("\u{7}".to_owned()).render(), "\"\\u0007\"");
    }

    #[test]
    fn an_edit_derives_its_table_and_key_from_the_field_path() {
        let edit = Edit::set(Field::PanelHeight, Toml::Integer(40));

        assert_eq!(edit.path(), "panel.height");
        assert_eq!(edit.section(), Some("panel"));
        assert_eq!(edit.key(), "height");

        let nested = Edit::set(Field::NotificationsNormalTimeout, Toml::Integer(8_000));
        assert_eq!(nested.path(), "timeouts.normal_ms");
        assert_eq!(nested.section(), Some("timeouts"));
        assert_eq!(nested.key(), "normal_ms");
    }

    #[test]
    fn every_field_has_a_key_and_reconstructs_its_path() {
        for field in Field::ALL {
            let edit = Edit::set(field, Toml::Bool(true));

            assert_eq!(
                edit.section().is_none(),
                field == Field::Outputs,
                "unexpected top-level key: {field}"
            );
            assert!(!edit.key().is_empty(), "`{}` has no key", field.path());
            assert_eq!(
                edit.section().map_or_else(
                    || edit.key().to_owned(),
                    |section| format!("{section}.{key}", key = edit.key())
                ),
                field.path().to_owned()
            );
        }
    }

    #[test]
    fn patching_an_empty_file_creates_the_table() {
        let patched = patch("", &[edit(Field::PanelHeight, Value::Number(40))]);

        assert_eq!(patched, "[panel]\nheight = 40\n");
    }

    #[test]
    fn a_key_that_is_already_there_is_replaced_where_it_stands() {
        let text = "# the panel\n[panel]\nheight = 32\nshow_clock = true\n";
        let patched = patch(text, &[edit(Field::PanelHeight, Value::Number(48))]);

        assert_eq!(
            patched,
            "# the panel\n[panel]\nheight = 48\nshow_clock = true\n"
        );
    }

    #[test]
    fn every_line_the_edit_does_not_name_is_copied_byte_for_byte() {
        let text =
            "# a comment\n\n[panel]\n# the strip\nheight = 32\n\n[launcher]\nmax_results = 8\n";

        for edit in [
            edit(Field::PanelHeight, Value::Number(64)),
            edit(Field::PanelPosition, Value::Choice("top".to_owned())),
        ] {
            let patched = patch(text, std::slice::from_ref(&edit));

            // Everything but the changed line is present, in the same order.
            for line in text.lines().filter(|line| !line.starts_with("height =")) {
                assert!(
                    patched.contains(line),
                    "`{line}` disappeared from\n{patched}"
                );
            }
            assert!(
                patched.starts_with("# a comment\n"),
                "the head is untouched"
            );
        }
    }

    #[test]
    fn a_key_that_is_missing_is_appended_to_its_own_table() {
        let text = "[panel]\nheight = 32\n\n[launcher]\nmax_results = 8\n";
        let patched = patch(text, &[edit(Field::PanelAutohide, Value::Flag(true))]);

        assert_eq!(
            patched,
            "[panel]\nheight = 32\nautohide = true\n\n[launcher]\nmax_results = 8\n"
        );
    }

    #[test]
    fn a_table_that_is_missing_is_appended_to_the_file() {
        let text = "[panel]\nheight = 32\n";
        let patched = patch(
            text,
            &[
                edit(
                    Field::NotificationsPosition,
                    Value::Choice("bottom-left".to_owned()),
                ),
                edit(Field::NotificationsDoNotDisturb, Value::Flag(true)),
            ],
        );

        assert_eq!(
            patched,
            "[panel]\nheight = 32\n\n[display]\nposition = \"bottom-left\"\n\n[policy]\ndo_not_disturb = true\n"
        );
    }

    #[test]
    fn two_tables_with_the_same_key_do_not_collide() {
        let text = "[panel]\nposition = \"bottom\"\n\n[wallpaper]\nmode = \"fill\"\n";
        let patched = patch(
            text,
            &[edit(
                Field::NotificationsPosition,
                Value::Choice("top-left".to_owned()),
            )],
        );

        assert_eq!(
            patched,
            "[panel]\nposition = \"bottom\"\n\n[wallpaper]\nmode = \"fill\"\n\n[display]\nposition = \"top-left\"\n",
            "panel.position is not what was asked for"
        );
    }

    #[test]
    fn removing_a_key_takes_its_line_away() {
        let text = "[locale]\nlanguage = \"zh-CN\"\n";
        let patched = patch(text, &[Edit::remove(Field::LocaleLanguage)]);

        assert_eq!(patched, "[locale]\n");
    }

    #[test]
    fn removing_a_key_that_is_not_there_changes_nothing() {
        let text = "[locale]\n";
        let patched = patch(text, &[Edit::remove(Field::LocaleLanguage)]);

        assert_eq!(patched, "[locale]\n");
    }

    #[test]
    fn a_dotted_key_at_the_top_level_is_the_key_of_that_table() {
        // `panel.height = 12` is `[panel] height = 12` written without a header.
        // Appending a `[panel]` block beside it would be a second definition of
        // the same table, which is a document `inox-config` refuses to read.
        let text = "panel.height = 12\n";

        assert_eq!(
            patch(text, &[edit(Field::PanelHeight, Value::Number(48))]),
            "panel.height = 48\n"
        );
    }

    #[test]
    fn a_table_spelled_as_dotted_keys_takes_dotted_siblings() {
        let text = "panel.height = 12\n";

        assert_eq!(
            patch(text, &[edit(Field::PanelShowClock, Value::Flag(true))]),
            "panel.height = 12\npanel.show_clock = true\n"
        );

        // A dotted key with a header further down still belongs to the top level,
        // so its siblings go there and not under that header.
        let text = "panel.height = 12\n\n[launcher]\nmax_results = 4\n";

        assert_eq!(
            patch(text, &[edit(Field::PanelShowClock, Value::Flag(true))]),
            "panel.height = 12\n\npanel.show_clock = true\n[launcher]\nmax_results = 4\n"
        );
    }

    #[test]
    fn a_quoted_key_is_the_same_key_as_a_bare_one() {
        for text in ["[panel]\n\"height\" = 12\n", "[panel]\n'height' = 12\n"] {
            let patched = patch(text, &[edit(Field::PanelHeight, Value::Number(48))]);

            assert_eq!(
                patched.matches("height").count(),
                1,
                "`{text}` was written a second time:\n{patched}"
            );
            assert!(
                patched.contains("48"),
                "`{text}` was not rewritten:\n{patched}"
            );
        }
    }

    #[test]
    fn a_quoted_key_is_read_back_to_the_text_it_names() {
        // The key of the file is what it names, escapes and all: `\u0069` is an
        // `i`, and a key written that way is not a key this walk is blind to.
        let text = "[panel]\n\"he\\u0069ght\" = 12\n";

        assert_eq!(
            patch(text, &[edit(Field::PanelHeight, Value::Number(48))]),
            "[panel]\n\"he\\u0069ght\" = 48\n"
        );
    }

    #[test]
    fn a_key_is_the_same_key_however_its_dots_are_spaced() {
        let text = "panel . \"height\" = 12\n";

        assert_eq!(
            patch(text, &[edit(Field::PanelHeight, Value::Number(48))]),
            "panel . \"height\" = 48\n"
        );
    }

    #[test]
    fn a_header_is_read_however_it_is_spelled() {
        // `["panel"]` and `[[panel]]` both declare the table `panel`, so a key
        // added to it goes inside rather than into a second table of that name.
        for text in ["[\"panel\"]\nheight = 12\n", "[[panel]]\nheight = 12\n"] {
            let patched = patch(text, &[edit(Field::PanelShowClock, Value::Flag(true))]);

            assert_eq!(
                patched,
                format!("{text}show_clock = true\n"),
                "`{text}` was not written into"
            );
        }
    }

    #[test]
    fn a_key_of_an_inline_table_is_written_into_that_table() {
        // `panel = { height = 12 }` is the table `panel` again, and it has to stay
        // inline: a `[panel]` header beside it is the same table a second time.
        let text = "panel = { height = 12 }\n";

        assert_eq!(
            patch(text, &[edit(Field::PanelHeight, Value::Number(48))]),
            "panel = { height = 48 }\n"
        );
        assert_eq!(
            patch(text, &[edit(Field::PanelShowClock, Value::Flag(true))]),
            "panel = { height = 12, show_clock = true }\n"
        );
        assert_eq!(
            patch(
                "panel = {}\n",
                &[edit(Field::PanelShowClock, Value::Flag(true))]
            ),
            "panel = { show_clock = true }\n"
        );

        // Two keys into a table that holds neither: each is written beside the
        // other, comma and all.
        for empty in ["panel = {}\n", "panel = { }\n"] {
            assert_eq!(
                patch(
                    empty,
                    &[
                        edit(Field::PanelHeight, Value::Number(48)),
                        edit(Field::PanelShowClock, Value::Flag(true)),
                    ]
                ),
                "panel = { height = 48, show_clock = true }\n",
                "`{empty}`"
            );
        }

        // A member the user did not touch keeps every byte it had.
        let several = "panel = { autohide = true, height = 12, max_task_buttons = 12 }\n";

        assert_eq!(
            patch(several, &[edit(Field::PanelHeight, Value::Number(48))]),
            "panel = { autohide = true, height = 48, max_task_buttons = 12 }\n"
        );

        // Two keys of the same table, both written into it.
        assert_eq!(
            patch(
                several,
                &[
                    edit(Field::PanelHeight, Value::Number(48)),
                    edit(Field::PanelAutohide, Value::Flag(false)),
                ]
            ),
            "panel = { autohide = false, height = 48, max_task_buttons = 12 }\n"
        );
    }

    #[test]
    fn removing_a_key_of_an_inline_table_takes_its_member_away() {
        let text = "locale = { language = \"zh-CN\", note = \"hand-written\" }\n";

        assert_eq!(
            patch(text, &[Edit::remove(Field::LocaleLanguage)]),
            "locale = { note = \"hand-written\" }\n"
        );
        assert_eq!(
            patch(
                "locale = { note = \"hand-written\", language = \"zh-CN\" }\n",
                &[Edit::remove(Field::LocaleLanguage)]
            ),
            "locale = { note = \"hand-written\" }\n"
        );
        assert_eq!(
            patch(
                "locale = { language = \"zh-CN\" }\n",
                &[Edit::remove(Field::LocaleLanguage)]
            ),
            "locale = {  }\n"
        );
    }

    #[test]
    fn a_line_inside_a_multiline_string_is_not_a_key() {
        // The string is a value, not structure: the key the window writes goes to
        // the end of its table, and the string keeps every byte of its content.
        let text = "[panel]\nclock_note = \"\"\"\nheight = 12\n\"\"\"\n";

        assert_eq!(
            patch(text, &[edit(Field::PanelHeight, Value::Number(48))]),
            "[panel]\nclock_note = \"\"\"\nheight = 12\n\"\"\"\nheight = 48\n"
        );
    }

    #[test]
    fn a_comment_on_a_changed_line_goes_with_it() {
        // Documented in the module comment: the line the user changed is
        // rewritten, and the comment that annotated it is part of the line.
        let text = "[panel]\nheight = 32 # tall enough\n";
        let patched = patch(text, &[edit(Field::PanelHeight, Value::Number(40))]);

        assert_eq!(patched, "[panel]\nheight = 40\n");
    }

    #[test]
    fn a_hash_inside_a_string_is_not_a_comment() {
        // `clock_format` is not a field this application writes, but it is part
        // of the same table, and the walk must not mistake it for a comment.
        let text = "[panel]\nclock_format = \"#H:%M\"\nshow_clock = true\n";
        let patched = patch(text, &[edit(Field::PanelShowClock, Value::Flag(false))]);

        assert_eq!(
            patched,
            "[panel]\nclock_format = \"#H:%M\"\nshow_clock = false\n"
        );
    }

    #[test]
    fn a_table_header_with_a_comment_still_declares_the_table() {
        let text = "# panel\n[panel] # the strip\nheight = 32\n";
        let patched = patch(text, &[edit(Field::PanelHeight, Value::Number(40))]);

        assert_eq!(patched, "# panel\n[panel] # the strip\nheight = 40\n");
    }

    #[test]
    fn a_document_that_is_already_what_the_edits_ask_for_is_unchanged() {
        let text = "[panel]\nheight = 32\n";
        let patched = patch(text, &[edit(Field::PanelHeight, Value::Number(32))]);

        assert_eq!(patched, text);
    }

    #[test]
    fn an_empty_patch_list_leaves_the_document_alone() {
        let text = "# nothing to do\n[panel]\nheight = 32\n";

        assert_eq!(patch(text, &[]), text);
    }

    #[test]
    fn a_document_remembers_its_path_its_text_and_its_digest() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("shell.toml");
        std::fs::write(&path, "[panel]\nheight = 32\n").expect("the file is written");

        let document = Document::read(path.clone()).expect("the file is readable");

        assert_eq!(document.path(), path);
        assert_eq!(document.text(), "[panel]\nheight = 32\n");
        assert_eq!(document.digest(), Digest::of(b"[panel]\nheight = 32\n"));
        assert_eq!(document.guard().digest(), document.digest());
    }

    #[test]
    fn a_missing_file_is_an_empty_document_that_a_save_creates() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("shell.toml");

        let document = Document::read(path.clone()).expect("a missing file is not an error");

        assert_eq!(document.text(), "");
        assert_eq!(document.digest(), Digest::Absent);

        let content = document.patch(&[edit(Field::PanelHeight, Value::Number(40))]);
        assert_eq!(content, "[panel]\nheight = 40\n");
    }

    #[test]
    fn a_file_that_is_not_utf8_is_refused_rather_than_overwritten() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("shell.toml");
        std::fs::write(&path, [0xff, 0xfe, 0x00]).expect("the file is written");

        let error = Document::read(path).expect_err("not a configuration file");
        assert!(matches!(error, DocumentError::Read { .. }), "{error:?}");
    }

    #[test]
    fn installing_a_saved_document_moves_its_guard_forward() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("shell.toml");
        std::fs::write(&path, "[panel]\nheight = 32\n").expect("the file is written");

        let mut documents = Documents {
            files: BTreeMap::new(),
        };
        documents
            .files
            .insert(SettingFile::Shell, Document::read(path).expect("readable"));

        let content = "[panel]\nheight = 40\n".to_owned();
        std::fs::write(directory.path().join("shell.toml"), &content).expect("the save");
        documents.install(SettingFile::Shell, content.clone());

        let document = documents.get(SettingFile::Shell).expect("the shell file");
        assert_eq!(document.text(), content);
        assert_eq!(document.digest(), Digest::of(content.as_bytes()));
    }

    #[test]
    fn a_file_name_is_the_one_prd_9_1_gives_it() {
        assert_eq!(SettingFile::Root.name(), "config.toml");
        assert_eq!(SettingFile::Shell.name(), "shell.toml");
        assert_eq!(SettingFile::Notifications.name(), "notifications.toml");
    }
}
