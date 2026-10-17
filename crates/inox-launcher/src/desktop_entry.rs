//! A hand-written parser for the desktop entry format.
//!
//! The format is a list of `[Group]` sections of `Key=Value` lines, wrapped in
//! a handful of rules that decide whether an entry can be used at all: a
//! localized key (`Name[zh_CN]`), a boolean, a semicolon-separated list with
//! its own escaping, and the `Type`, `Hidden`, `NoDisplay`, `OnlyShowIn` and
//! `TryExec` gates. That is all of it, so the parser is handwritten: every rule
//! above is a line of code with a test next to it, and no dependency has to be
//! taken on for a grammar this small.
//!
//! Two decisions are worth stating because the specification leaves them open:
//!
//! - **A malformed line is skipped, not fatal.** A line that is neither a
//!   group header nor `Key=Value` is recorded in
//!   [`DesktopEntry::malformed_lines`] so the caller can log it, and the rest of
//!   the file is still read. One bad line does not discard an application.
//! - **The first definition of a key wins.** The specification forbids
//!   repeating a key within a group but does not say what a parser should do;
//!   taking the first means a file that has been merged twice cannot change an
//!   entry's command line behind the reader's back.
//!
//! Value-level escapes (`\s`, `\n`, `\t`, `\r`, `\\`) are decoded for every key
//! *except* `Exec` and `TryExec`, whose values are handed to [`crate::exec`]
//! unchanged: that grammar has its own quoting rule, and decoding the escapes
//! first would turn an escaped space into a word separator.

use std::path::{Path, PathBuf};

use inox_common::i18n::Locale;

/// A string value that may be translated.
///
/// Translations are kept in the order the file defines them, so that two keys
/// of the same specificity resolve to the one written first — a decision, not
/// an artefact of a hash map's iteration order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Localized {
    default: Option<String>,
    translations: Vec<(LocaleKey, String)>,
}

impl Localized {
    /// The untranslated value, which the specification requires and which every
    /// lookup falls back to.
    pub fn default_value(&self) -> Option<&str> {
        self.default.as_deref()
    }

    /// Whether the value is defined at all.
    pub fn is_defined(&self) -> bool {
        self.default.is_some() || !self.translations.is_empty()
    }

    /// The value for `locale`, or the untranslated value.
    ///
    /// A `lang_COUNTRY` translation is preferred over a `lang` one: the locale
    /// `zh-CN` takes `Name[zh_CN]` before `Name[zh]`, and falls back to `Name`.
    /// A translation whose country disagrees is not a candidate at all
    /// (`Name[zh_TW]` never answers a request for `zh-CN` through the
    /// `zh-CN` step; it is only reachable once the request drops to `zh`).
    pub fn resolve(&self, locale: &Locale) -> Option<&str> {
        for wanted in candidates(locale) {
            if let Some((_, value)) = self
                .translations
                .iter()
                .find(|(key, _)| key.same_place(&wanted))
            {
                return Some(value);
            }
        }
        self.default.as_deref()
    }
}

/// A language/country key as it appears inside `Name[...]`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LocaleKey {
    language: String,
    country: Option<String>,
}

impl LocaleKey {
    /// Parses `lang_COUNTRY.ENCODING@MODIFIER`, keeping the language and the
    /// country. The encoding and the modifier take no part in matching: Inox
    /// ships UTF-8 catalogs and has no modifier to negotiate, and a key that
    /// carries either still names the same language and country.
    fn parse(raw: &str) -> Option<Self> {
        let without_modifier = raw.split('@').next().unwrap_or(raw);
        let without_encoding = without_modifier
            .split('.')
            .next()
            .unwrap_or(without_modifier);
        let mut parts = without_encoding.split('_');
        let language = parts.next().unwrap_or_default().trim();
        if language.is_empty() {
            return None;
        }
        let country = parts.next().map(str::trim).filter(|it| !it.is_empty());
        Some(Self {
            language: language.to_ascii_lowercase(),
            country: country.map(str::to_ascii_lowercase),
        })
    }

    /// Whether this key names the same place as `other`.
    fn same_place(&self, other: &Self) -> bool {
        self.language == other.language && self.country == other.country
    }
}

/// The places `locale` asks for, most specific first.
fn candidates(locale: &Locale) -> Vec<LocaleKey> {
    let tag = locale.tag();
    let mut parts = tag.split('-');
    let language = parts.next().unwrap_or_default().to_ascii_lowercase();
    // A BCP-47 tag's second subtag is either a two-letter region or a
    // four-letter script (`zh-Hans`); only a region is a desktop entry country.
    let country = parts.next().and_then(|subtag| {
        let is_region = subtag.len() == 2 && subtag.chars().all(|c| c.is_ascii_alphabetic());
        is_region.then(|| subtag.to_ascii_lowercase())
    });

    let mut places = Vec::with_capacity(2);
    if let Some(country) = country.clone() {
        places.push(LocaleKey {
            language: language.clone(),
            country: Some(country),
        });
    }
    places.push(LocaleKey {
        language,
        country: None,
    });
    places
}

/// The rules that decide whether an entry is listed, as opposed to parsed.
///
/// These are the checks that need to know about the running session rather than
/// about the file, and they are parameters rather than process environment
/// lookups for the same reason [`ino x_common::i18n::Locale::from_env_with`]
/// exists: a test cannot change its own environment, but it can pass one in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryFilter {
    /// The locale `Name`, `GenericName` and `Comment` are resolved against.
    pub locale: Locale,
    /// The desktops the session reports (`XDG_CURRENT_DESKTOP`, split on `:`).
    ///
    /// An empty list means "this session does not say", in which case neither
    /// `OnlyShowIn` nor `NotShowIn` excludes anything: an entry restricted to
    /// another desktop is still shown, because there is no evidence that this
    /// is another desktop. Hiding applications on the strength of a missing
    /// environment variable would be the worse failure.
    pub desktop_names: Vec<String>,
    /// Whether `Hidden=true` and `NoDisplay=true` entries are listed as well
    /// (`shell.toml`'s `launcher.include_hidden`).
    pub include_hidden: bool,
    /// Directories the program named by `TryExec` is looked for in (`PATH`).
    pub executables: Vec<PathBuf>,
}

impl EntryFilter {
    /// A filter for `locale` that hides hidden entries and trusts the session's
    /// desktop names and `PATH`, both taken from the environment.
    pub fn from_env(locale: &Locale) -> Self {
        Self {
            locale: locale.clone(),
            desktop_names: std::env::var("XDG_CURRENT_DESKTOP")
                .map(|value| split_desktop_names(&value))
                .unwrap_or_default(),
            include_hidden: false,
            executables: std::env::var("PATH")
                .map(|value| std::env::split_paths(&value).collect())
                .unwrap_or_default(),
        }
    }
}

/// Splits `XDG_CURRENT_DESKTOP`, which is a colon-separated list.
fn split_desktop_names(value: &str) -> Vec<String> {
    value
        .split(':')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Why an entry cannot be used.
///
/// Every variant is a cause and is reported as a diagnostic by
/// [`crate::index::scan`]; none of them aborts the scan of the other files.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EntryProblem {
    /// The file has no `[Desktop Entry]` group.
    #[error("the file has no [Desktop Entry] group")]
    MissingGroup,
    /// The group has no `Type` key.
    #[error("the entry has no Type key")]
    MissingType,
    /// The `Type` is not `Application`.
    #[error("the entry has Type={found:?}, and only Type=Application is supported")]
    UnsupportedType {
        /// The value that was found.
        found: String,
    },
    /// No non-blank `Name` could be resolved for the requested locale.
    #[error("the entry has no Name")]
    MissingName,
    /// The entry has no `Exec` command.
    #[error("the entry has no Exec command")]
    MissingExec,
    /// `Hidden=true`: the entry is treated as deleted by its author.
    #[error("Hidden=true")]
    Hidden,
    /// `NoDisplay=true`: the entry exists but does not belong in a launcher.
    #[error("NoDisplay=true")]
    NoDisplay,
    /// `OnlyShowIn` names desktops, and none of them is this session's.
    #[error("OnlyShowIn limits the entry to {only_show_in:?}, and this session is not one of them")]
    NotThisDesktop {
        /// The desktops the entry allows.
        only_show_in: Vec<String>,
    },
    /// `NotShowIn` names this session's desktop.
    #[error("NotShowIn excludes the entry from this session's desktop")]
    ExcludedDesktop {
        /// The desktops the entry excludes.
        not_show_in: Vec<String>,
    },
    /// `TryExec` names a program that is not installed.
    #[error("TryExec names {program:?}, which is not on PATH")]
    TryExecMissing {
        /// The program the entry wanted.
        program: String,
    },
    /// The file or directory could not be read.
    #[error("the file could not be read: {detail}")]
    Unreadable {
        /// The operating system's own explanation.
        detail: String,
    },
}

/// A desktop action (`[Desktop Action <id>]`).
///
/// Parsed because the launcher owns the application index (ADR 0013) and an
/// action is part of what an entry defines; the interactive launcher currently
/// launches only the entry's own `Exec`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopAction {
    /// The action's id, as listed in `Actions=`.
    pub id: String,
    /// Its localized name.
    pub name: Localized,
    /// Its command line, kept as written for [`crate::exec`].
    pub exec: String,
    /// Its icon name.
    pub icon: Option<String>,
}

/// One usable `Type=Application` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntry {
    /// The desktop file id: the path relative to its `applications` directory
    /// with `/` replaced by `-` (XDG desktop entry specification).
    pub id: String,
    /// The absolute path of the file, which `%k` expands to.
    pub path: PathBuf,
    /// The localized application name.
    pub name: Localized,
    /// The localized generic name ("Web Browser", "Text Editor").
    pub generic_name: Localized,
    /// The localized comment (a tooltip).
    pub comment: Localized,
    /// The icon name, to be resolved against the icon theme.
    pub icon: Option<String>,
    /// The command line, exactly as the file wrote it.
    pub exec: String,
    /// A program that must be installed for the entry to be usable.
    pub try_exec: Option<String>,
    /// Whether the entry needs a terminal.
    pub terminal: bool,
    /// The working directory the process should start in.
    pub working_dir: Option<String>,
    /// The window class the application will use, for matching windows to
    /// entries.
    pub startup_wm_class: Option<String>,
    /// The entry's categories.
    pub categories: Vec<String>,
    /// The desktop environments the entry allows, from `OnlyShowIn`.
    pub only_show_in: Vec<String>,
    /// The desktop environments the entry excludes, from `NotShowIn`.
    pub not_show_in: Vec<String>,
    /// The actions the entry declares, in the order `Actions=` lists them.
    pub actions: Vec<DesktopAction>,
    /// 1-based line numbers of lines that were neither group headers nor
    /// `Key=Value`, and were therefore skipped.
    pub malformed_lines: Vec<usize>,
}

/// Parses one desktop file.
///
/// # Errors
///
/// Returns the [`EntryProblem`] that makes the entry unusable. A hidden entry
/// is an *error* here rather than a parsed entry with a flag, so that the index
/// has one path for "this file contributes nothing" and cannot forget to apply
/// one of the gates.
pub fn parse(
    source: &str,
    id: &str,
    path: &Path,
    filter: &EntryFilter,
) -> Result<DesktopEntry, EntryProblem> {
    let parsed = parse_groups(source);

    let Some(entry) = parsed.desktop_entry else {
        return Err(EntryProblem::MissingGroup);
    };

    match entry.get("Type") {
        None => return Err(EntryProblem::MissingType),
        Some(value) if value != APPLICATION => {
            return Err(EntryProblem::UnsupportedType {
                found: value.to_owned(),
            });
        }
        Some(_) => {}
    }

    if !filter.include_hidden {
        if entry.boolean("Hidden") {
            return Err(EntryProblem::Hidden);
        }
        if entry.boolean("NoDisplay") {
            return Err(EntryProblem::NoDisplay);
        }
    }

    let name = entry.localized("Name");
    if name.resolve(&filter.locale).is_none() {
        return Err(EntryProblem::MissingName);
    }

    let Some(exec) = entry.raw("Exec") else {
        return Err(EntryProblem::MissingExec);
    };
    if exec.trim().is_empty() {
        return Err(EntryProblem::MissingExec);
    }

    let only_show_in = entry.list("OnlyShowIn");
    let not_show_in = entry.list("NotShowIn");
    if !filter.desktop_names.is_empty() {
        if !only_show_in.is_empty()
            && !filter
                .desktop_names
                .iter()
                .any(|desktop| only_show_in.iter().any(|allowed| allowed == desktop))
        {
            return Err(EntryProblem::NotThisDesktop { only_show_in });
        }
        if filter
            .desktop_names
            .iter()
            .any(|desktop| not_show_in.iter().any(|excluded| excluded == desktop))
        {
            return Err(EntryProblem::ExcludedDesktop { not_show_in });
        }
    }

    let try_exec = entry.raw("TryExec").map(str::to_owned);
    if let Some(program) = &try_exec
        && !is_executable(program, &filter.executables)
    {
        return Err(EntryProblem::TryExecMissing {
            program: program.clone(),
        });
    }

    let actions = parse_actions(&entry, &parsed.actions);

    Ok(DesktopEntry {
        id: id.to_owned(),
        path: path.to_path_buf(),
        name,
        generic_name: entry.localized("GenericName"),
        comment: entry.localized("Comment"),
        icon: entry.get("Icon"),
        exec: exec.to_owned(),
        try_exec,
        terminal: entry.boolean("Terminal"),
        working_dir: entry.get("Path"),
        startup_wm_class: entry.get("StartupWMClass"),
        categories: entry.list("Categories"),
        only_show_in,
        not_show_in,
        actions,
        malformed_lines: parsed.malformed_lines,
    })
}

/// The `Type` an entry must have to be launched.
const APPLICATION: &str = "Application";

/// A program is usable if it names a path, or if it is on the search path.
fn is_executable(program: &str, directories: &[PathBuf]) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    directories
        .iter()
        .any(|directory| directory.join(program).is_file())
}

/// The groups of a parsed file.
struct ParsedFile {
    desktop_entry: Option<Values>,
    actions: Vec<(String, Values)>,
    malformed_lines: Vec<usize>,
}

/// A group's key/value pairs, in file order, with the raw key spelling.
struct Values(Vec<(String, String)>);

impl Values {
    /// The value of `key` with its string escapes decoded.
    fn get(&self, key: &str) -> Option<String> {
        self.raw(key).map(unescape_string)
    }

    /// The value of `key`, exactly as written.
    fn raw(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    /// The base key and every `key[locale]` translation, in file order.
    ///
    /// A value that is blank once its escapes are decoded is dropped rather
    /// than stored. `Name[zh_CN]=` means "this one is not translated" and has
    /// to fall back to `Name` instead of blanking the row, and dropping the
    /// blank here makes `Name=` fail [`parse`]'s `Name` gate exactly as `Exec=`
    /// already fails its own. [`Localized`] is the crate's single notion of "is
    /// this value defined", so the decision belongs here and not at each point
    /// of use.
    fn localized(&self, key: &str) -> Localized {
        let prefix = format!("{key}[");
        let mut localized = Localized::default();
        for (name, value) in &self.0 {
            let value = unescape_string(value);
            if value.trim().is_empty() {
                continue;
            }
            if name == key {
                localized.default = Some(value);
                continue;
            }
            let Some(inner) = name
                .strip_prefix(&prefix)
                .and_then(|rest| rest.strip_suffix(']'))
            else {
                continue;
            };
            let Some(locale_key) = LocaleKey::parse(inner) else {
                continue;
            };
            localized.translations.push((locale_key, value));
        }
        localized
    }

    /// A `true`/`false` value. Anything else counts as false, which is what the
    /// specification's own "the value is true or false" leaves for a parser to
    /// decide.
    fn boolean(&self, key: &str) -> bool {
        self.raw(key)
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
    }

    /// A semicolon-separated list with `\;` escaping.
    fn list(&self, key: &str) -> Vec<String> {
        self.raw(key).map(parse_list).unwrap_or_default()
    }
}

/// Which group the lines being read belong to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Section {
    /// Before the first group header, or inside a group Inox does not read.
    Ignored,
    /// `[Desktop Entry]`.
    Entry,
    /// `[Desktop Action <id>]`.
    Action(String),
}

/// Splits a file into its groups.
fn parse_groups(source: &str) -> ParsedFile {
    let mut parsed = ParsedFile {
        desktop_entry: None,
        actions: Vec::new(),
        malformed_lines: Vec::new(),
    };

    let mut section = Section::Ignored;
    let mut current: Vec<(String, String)> = Vec::new();

    for (index, line) in source.lines().enumerate() {
        let number = index + 1;
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix('[') {
            flush_section(&section, &mut current, &mut parsed);
            let header = rest.strip_suffix(']').unwrap_or(rest).trim();
            section = if header == "Desktop Entry" {
                Section::Entry
            } else if let Some(id) = header.strip_prefix("Desktop Action ") {
                Section::Action(id.trim().to_owned())
            } else {
                Section::Ignored
            };
            continue;
        }

        if section == Section::Ignored {
            parsed.malformed_lines.push(number);
            continue;
        }

        let Some((key, value)) = trimmed.split_once('=') else {
            parsed.malformed_lines.push(number);
            continue;
        };

        let key = key.trim();
        if key.is_empty() {
            parsed.malformed_lines.push(number);
            continue;
        }

        // The first definition wins; see the module documentation.
        if current.iter().any(|(name, _)| name == key) {
            continue;
        }
        current.push((key.to_owned(), value.trim().to_owned()));
    }

    flush_section(&section, &mut current, &mut parsed);
    parsed
}

/// Stores the lines read so far under the group they belong to.
fn flush_section(section: &Section, current: &mut Vec<(String, String)>, parsed: &mut ParsedFile) {
    let values = Values(std::mem::take(current));
    match section {
        Section::Entry => parsed.desktop_entry = Some(values),
        Section::Action(id) => parsed.actions.push((id.clone(), values)),
        Section::Ignored => {}
    }
}

/// The actions an entry declares, in the order `Actions=` lists them.
fn parse_actions(entry: &Values, groups: &[(String, Values)]) -> Vec<DesktopAction> {
    let mut actions = Vec::new();
    for id in entry.list("Actions") {
        let Some((_, values)) = groups.iter().find(|(name, _)| *name == id) else {
            continue;
        };
        let name = values.localized("Name");
        if !name.is_defined() {
            continue;
        }
        let Some(exec) = values.raw("Exec") else {
            continue;
        };
        if exec.trim().is_empty() {
            continue;
        }
        actions.push(DesktopAction {
            id,
            name,
            exec: exec.to_owned(),
            icon: values.get("Icon"),
        });
    }
    actions
}

/// Decodes the string-value escapes of the desktop entry specification.
///
/// The five defined escapes are decoded; any other backslash sequence is left
/// as written, because the specification does not define one and silently
/// dropping the backslash would corrupt a path.
fn unescape_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Splits a semicolon-separated list, with `\;` naming a literal semicolon.
///
/// Elements are trimmed and empty ones dropped, so `a; b ;;c;` is `[a, b, c]`.
fn parse_list(raw: &str) -> Vec<String> {
    let mut elements = Vec::new();
    let mut current = String::new();
    let mut chars = raw.chars();
    while let Some(character) = chars.next() {
        match character {
            '\\' => match chars.next() {
                Some(';') => current.push(';'),
                Some(other) => {
                    current.push('\\');
                    current.push(other);
                }
                None => current.push('\\'),
            },
            ';' => elements.push(std::mem::take(&mut current)),
            other => current.push(other),
        }
    }
    elements.push(current);

    elements
        .into_iter()
        .map(|element| unescape_string(element.trim()))
        .filter(|element| !element.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn filter() -> EntryFilter {
        EntryFilter {
            locale: Locale::parse("en-US").expect("a well-formed tag"),
            desktop_names: vec!["Inox".to_owned()],
            include_hidden: false,
            executables: Vec::new(),
        }
    }

    fn parse_with(source: &str, filter: &EntryFilter) -> Result<DesktopEntry, EntryProblem> {
        parse(
            source,
            "org.example.Example",
            Path::new("/usr/share/applications/org.example.Example.desktop"),
            filter,
        )
    }

    /// A minimal entry, as `org.example.Example` would ship it.
    const EXAMPLE: &str = "\
[Desktop Entry]
Type=Application
Name=Example
GenericName=Example Editor
Comment=Edit examples
Exec=/usr/bin/example %U
Icon=example
Categories=Utility;TextEditor;
Terminal=false
";

    #[test]
    fn a_minimal_entry_parses() {
        let entry = parse_with(EXAMPLE, &filter()).expect("a usable entry");
        assert_eq!(entry.id, "org.example.Example");
        assert_eq!(entry.name.resolve(&filter().locale), Some("Example"));
        assert_eq!(entry.generic_name.default_value(), Some("Example Editor"));
        assert_eq!(entry.comment.default_value(), Some("Edit examples"));
        assert_eq!(entry.exec, "/usr/bin/example %U");
        assert_eq!(entry.icon.as_deref(), Some("example"));
        assert_eq!(entry.categories, ["Utility", "TextEditor"]);
        assert!(!entry.terminal);
        assert!(entry.actions.is_empty());
        assert!(entry.malformed_lines.is_empty());
    }

    #[test]
    fn a_file_without_the_entry_group_is_rejected() {
        assert_eq!(
            parse_with("Name=Example\nExec=example\n", &filter()),
            Err(EntryProblem::MissingGroup)
        );
    }

    #[test]
    fn a_missing_type_is_rejected() {
        assert_eq!(
            parse_with("[Desktop Entry]\nName=Example\nExec=example\n", &filter()),
            Err(EntryProblem::MissingType)
        );
    }

    #[test]
    fn a_non_application_type_is_skipped_not_fatal() {
        let problem = parse_with(
            "[Desktop Entry]\nType=Link\nName=Example\nURL=https://example.org\n",
            &filter(),
        );
        assert_eq!(
            problem,
            Err(EntryProblem::UnsupportedType {
                found: "Link".to_owned()
            })
        );
    }

    #[test]
    fn an_entry_without_a_name_is_rejected() {
        assert_eq!(
            parse_with("[Desktop Entry]\nType=Application\nExec=x\n", &filter()),
            Err(EntryProblem::MissingName)
        );
    }

    #[test]
    fn an_entry_with_a_blank_name_is_rejected() {
        // The same gate `Exec=` already passes through: a name that is present
        // but blank leaves a nameless row that is still selectable and
        // launchable, so it is a missing name, not a name.
        assert_eq!(
            parse_with(
                "[Desktop Entry]\nType=Application\nName=\nExec=x\n",
                &filter()
            ),
            Err(EntryProblem::MissingName)
        );
        // `\n` decodes to a newline, which is blank even though the raw value
        // is not empty.
        assert_eq!(
            parse_with(
                "[Desktop Entry]\nType=Application\nName=\\n\nExec=x\n",
                &filter()
            ),
            Err(EntryProblem::MissingName)
        );
        // A blank name with only a blank translation is still blank.
        assert_eq!(
            parse_with(
                "[Desktop Entry]\nType=Application\nName=\nName[zh_CN]=\nExec=x\n",
                &filter()
            ),
            Err(EntryProblem::MissingName)
        );
    }

    #[test]
    fn a_blank_translation_falls_back_to_the_untranslated_name() {
        let source = "[Desktop Entry]\nType=Application\nName=Editor\nName[zh_CN]=\nExec=e\n";
        let entry = parse_with(source, &filter()).expect("a usable entry");
        let simplified = Locale::parse("zh-CN").expect("a tag");
        // An author who wrote `Name[zh_CN]=` declared no translation for that
        // locale; the row falls back to `Name=Editor` rather than going blank.
        assert_eq!(entry.name.resolve(&simplified), Some("Editor"));
    }

    #[test]
    fn an_entry_without_an_exec_is_rejected() {
        assert_eq!(
            parse_with("[Desktop Entry]\nType=Application\nName=X\n", &filter()),
            Err(EntryProblem::MissingExec)
        );
        assert_eq!(
            parse_with(
                "[Desktop Entry]\nType=Application\nName=X\nExec=\n",
                &filter()
            ),
            Err(EntryProblem::MissingExec)
        );
    }

    #[test]
    fn hidden_and_nodisplay_entries_are_skipped() {
        let hidden = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nHidden=true\n";
        assert_eq!(parse_with(hidden, &filter()), Err(EntryProblem::Hidden));

        let nodisplay = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\n";
        assert_eq!(
            parse_with(nodisplay, &filter()),
            Err(EntryProblem::NoDisplay)
        );

        let mut including = filter();
        including.include_hidden = true;
        assert!(parse_with(hidden, &including).is_ok());
        assert!(parse_with(nodisplay, &including).is_ok());
    }

    #[test]
    fn a_hidden_false_entry_is_kept() {
        let source = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nHidden=false\n";
        assert!(parse_with(source, &filter()).is_ok());
    }

    #[test]
    fn a_localized_name_wins_for_its_locale_and_falls_back_otherwise() {
        let source = "\
[Desktop Entry]
Type=Application
Name=Terminal
Name[zh_CN]=终端
Name[zh]=终端（简体）
Name[de]=Terminal
Exec=terminal
";
        let entry = parse_with(source, &filter()).expect("a usable entry");

        let english = Locale::parse("en-US").expect("a tag");
        let simplified = Locale::parse("zh-CN").expect("a tag");
        let taiwan = Locale::parse("zh-TW").expect("a tag");

        assert_eq!(entry.name.resolve(&english), Some("Terminal"));
        // `zh-CN` prefers the country-specific key.
        assert_eq!(entry.name.resolve(&simplified), Some("终端"));
        // `zh-TW` has no key of its own; the `zh` key answers it, and the
        // `zh_CN` one does not.
        assert_eq!(entry.name.resolve(&taiwan), Some("终端（简体）"));
    }

    #[test]
    fn a_language_key_answers_a_region_that_has_no_key_of_its_own() {
        let source = "[Desktop Entry]\nType=Application\nName=Editor\nName[de]=Editor\nExec=e\n";
        let entry = parse_with(source, &filter()).expect("a usable entry");
        let austrian = Locale::parse("de-AT").expect("a tag");
        assert_eq!(entry.name.resolve(&austrian), Some("Editor"));
    }

    #[test]
    fn the_first_definition_of_a_key_wins() {
        let source = "[Desktop Entry]\nType=Application\nName=First\nName=Second\nExec=x\n";
        let entry = parse_with(source, &filter()).expect("a usable entry");
        assert_eq!(entry.name.default_value(), Some("First"));
    }

    #[test]
    fn paths_may_contain_spaces() {
        let source = "\
[Desktop Entry]
Type=Application
Name=Spaces
Exec=\"/opt/My App/bin/app\" %U
Path=/home/user/My Documents
";
        let entry = parse_with(source, &filter()).expect("a usable entry");
        assert_eq!(entry.exec, "\"/opt/My App/bin/app\" %U");
        assert_eq!(
            entry.working_dir.as_deref(),
            Some("/home/user/My Documents")
        );
    }

    #[test]
    fn a_broken_exec_does_not_make_the_entry_unusable() {
        // The Exec grammar is applied when the entry is launched, not when it is
        // indexed: an entry with a field code Inox does not know is still listed,
        // and saying why it cannot be started is the launch path's job.
        let source = "[Desktop Entry]\nType=Application\nName=X\nExec=x %q\n";
        let entry = parse_with(source, &filter()).expect("a usable entry");
        assert_eq!(entry.exec, "x %q");
        assert_eq!(
            crate::exec::argv(&crate::exec::ExecContext {
                exec: &entry.exec,
                name: entry.name.default_value().unwrap_or_default(),
                icon: None,
                desktop_file: &entry.path,
                files: &[],
                terminal: false,
                terminal_prefix: &[],
            }),
            Err(crate::contract::FailureKind::UnknownFieldCode { code: 'q' })
        );
    }

    #[test]
    fn only_show_in_and_not_show_in_are_compared_with_the_session_desktop() {
        let inox = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nOnlyShowIn=Inox;GNOME;\n";
        assert!(parse_with(inox, &filter()).is_ok());

        let other = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nOnlyShowIn=GNOME;KDE;\n";
        assert_eq!(
            parse_with(other, &filter()),
            Err(EntryProblem::NotThisDesktop {
                only_show_in: vec!["GNOME".to_owned(), "KDE".to_owned()]
            })
        );

        let excluded = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNotShowIn=Inox;\n";
        assert_eq!(
            parse_with(excluded, &filter()),
            Err(EntryProblem::ExcludedDesktop {
                not_show_in: vec!["Inox".to_owned()]
            })
        );
    }

    #[test]
    fn a_session_that_names_no_desktop_excludes_nothing() {
        let mut anonymous = filter();
        anonymous.desktop_names.clear();
        let other = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nOnlyShowIn=GNOME;\n";
        assert!(
            parse_with(other, &anonymous).is_ok(),
            "with nothing to compare against, hiding an application would be a guess",
        );
    }

    #[test]
    fn a_missing_try_exec_program_is_reported() {
        let source =
            "[Desktop Entry]\nType=Application\nName=X\nExec=x\nTryExec=definitely-absent\n";
        assert_eq!(
            parse_with(source, &filter()),
            Err(EntryProblem::TryExecMissing {
                program: "definitely-absent".to_owned()
            })
        );
    }

    #[test]
    fn a_try_exec_program_on_the_search_path_is_accepted() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(directory.path().join("present"), "").expect("writing the program");

        let mut with_path = filter();
        with_path.executables = vec![directory.path().to_path_buf()];

        let source = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nTryExec=present\n";
        assert!(parse_with(source, &with_path).is_ok());
    }

    #[test]
    fn a_list_keeps_escaped_semicolons_and_drops_empty_elements() {
        let source = r"\
[Desktop Entry]
Type=Application
Name=X
Exec=x
Categories=  Audio;Video ;; a\;b ;
";
        let entry = parse_with(source, &filter()).expect("a usable entry");
        assert_eq!(entry.categories, ["Audio", "Video", "a;b"]);
    }

    #[test]
    fn string_escapes_are_decoded_for_values_but_not_for_exec() {
        let source = "\
[Desktop Entry]
Type=Application
Name=Two\\sWords\\nAnd\\tMore
Comment=back\\\\slash
Exec=/usr/bin/app one\\stwo
";
        let entry = parse_with(source, &filter()).expect("a usable entry");
        assert_eq!(entry.name.default_value(), Some("Two Words\nAnd\tMore"));
        assert_eq!(entry.comment.default_value(), Some("back\\slash"));
        assert_eq!(
            entry.exec, "/usr/bin/app one\\stwo",
            "the Exec value is handed to the Exec grammar untouched"
        );
    }

    #[test]
    fn an_unknown_escape_is_left_as_written() {
        let source = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nPath=/a\\q/b\n";
        let entry = parse_with(source, &filter()).expect("a usable entry");
        assert_eq!(entry.working_dir.as_deref(), Some("/a\\q/b"));
    }

    #[test]
    fn malformed_lines_are_recorded_and_skipped() {
        let source = "\
a line before any group
[Desktop Entry]
Type=Application
this line has no equals sign
Name=X
Exec=x
";
        let entry = parse_with(source, &filter()).expect("a usable entry");
        assert_eq!(entry.malformed_lines, [1, 4]);
        assert_eq!(entry.name.default_value(), Some("X"));
    }

    #[test]
    fn a_comment_and_a_blank_line_are_not_malformed() {
        let source =
            "# a comment\n\n[Desktop Entry]\n# another\nType=Application\nName=X\nExec=x\n";
        let entry = parse_with(source, &filter()).expect("a usable entry");
        assert!(entry.malformed_lines.is_empty());
    }

    #[test]
    fn a_hash_inside_a_value_is_not_a_comment() {
        let source = "[Desktop Entry]\nType=Application\nName=A # B\nExec=x\n";
        let entry = parse_with(source, &filter()).expect("a usable entry");
        assert_eq!(entry.name.default_value(), Some("A # B"));
    }

    #[test]
    fn other_groups_are_ignored() {
        let source = "\
[Desktop Entry]
Type=Application
Name=X
Exec=x

[Desktop Action Ignored]
Name=Not listed
Exec=other

[X-Whatever]
Key=Value
";
        let entry = parse_with(source, &filter()).expect("a usable entry");
        assert!(
            entry.actions.is_empty(),
            "an action group that Actions= does not list is not an action",
        );
    }

    #[test]
    fn declared_actions_are_parsed_in_order() {
        let source = "\
[Desktop Entry]
Type=Application
Name=Editor
Exec=editor
Actions=NewWindow;Private;
Icon=editor

[Desktop Action NewWindow]
Name=New Window
Exec=editor --new-window
Icon=window-new

[Desktop Action Private]
Name=Private Window
Exec=editor --private
";
        let entry = parse_with(source, &filter()).expect("a usable entry");
        assert_eq!(entry.actions.len(), 2);
        assert_eq!(entry.actions[0].id, "NewWindow");
        assert_eq!(entry.actions[0].name.default_value(), Some("New Window"));
        assert_eq!(entry.actions[0].exec, "editor --new-window");
        assert_eq!(entry.actions[0].icon.as_deref(), Some("window-new"));
        assert_eq!(entry.actions[1].id, "Private");
        assert_eq!(entry.actions[1].icon, None);
    }

    #[test]
    fn an_action_without_a_name_or_a_command_is_dropped() {
        let source = "\
[Desktop Entry]
Type=Application
Name=Editor
Exec=editor
Actions=Nameless;Commandless;

[Desktop Action Nameless]
Exec=editor --x

[Desktop Action Commandless]
Name=Commandless
";
        let entry = parse_with(source, &filter()).expect("a usable entry");
        assert!(entry.actions.is_empty());
    }

    #[test]
    fn a_carriage_return_does_not_leak_into_a_value() {
        let source = "[Desktop Entry]\r\nType=Application\r\nName=Editor\r\nExec=editor\r\n";
        let entry = parse_with(source, &filter()).expect("a usable entry");
        assert_eq!(entry.name.default_value(), Some("Editor"));
        assert_eq!(entry.exec, "editor");
    }

    #[test]
    fn the_d02_sample_table_parses_the_way_d02_describes() {
        // The shapes acceptance D02 names, one entry each: a localized name, a
        // path with spaces, a duplicate id (decided by `crate::index`), a
        // Hidden one, a NoDisplay one, and a broken Exec.
        let entries = [
            (
                "localized",
                "[Desktop Entry]\nType=Application\nName=Files\nName[zh_CN]=文件\nExec=files\n",
            ),
            (
                "spaces",
                "[Desktop Entry]\nType=Application\nName=Space App\nExec=\"/opt/a b/app\" %U\nPath=/opt/a b\n",
            ),
            (
                "hidden",
                "[Desktop Entry]\nType=Application\nName=Hidden\nExec=x\nHidden=true\n",
            ),
            (
                "nodisplay",
                "[Desktop Entry]\nType=Application\nName=NoDisplay\nExec=x\nNoDisplay=true\n",
            ),
            (
                "broken",
                "[Desktop Entry]\nType=Application\nName=Broken\nExec=x %q\n",
            ),
        ];

        let mut accepted = 0;
        for (name, source) in entries {
            match parse_with(source, &filter()) {
                Ok(_) => accepted += 1,
                Err(problem) => assert!(
                    matches!(problem, EntryProblem::Hidden | EntryProblem::NoDisplay),
                    "{name} was rejected for the wrong reason: {problem}"
                ),
            }
        }

        assert_eq!(
            accepted, 3,
            "the two hidden entries are the only rejections"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// Any text a file could contain is read or refused, never fatal.
        #[test]
        fn any_source_is_parsed_or_refused(source in "(?s).{0,400}") {
            let _ = parse_with(&source, &filter());
        }

        /// A syntactically valid entry with the required keys always parses.
        #[test]
        fn a_valid_entry_always_parses(
            name in "[A-Za-z0-9_]{1,40}",
            exec in "[A-Za-z0-9_./-]{1,60}",
        ) {
            let source = format!(
                "[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\n"
            );
            let entry = parse_with(&source, &filter()).expect("a usable entry");
            prop_assert_eq!(entry.name.default_value(), Some(name.as_str()));
            prop_assert_eq!(&entry.exec, &exec);
        }
    }
}
