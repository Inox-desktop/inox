//! From a command line argument to the MIME type that names a handler.
//!
//! "Open this" has to become "this is a `text/plain`" before anything can be
//! asked who opens `text/plain`, and the answer comes from two places:
//!
//! - **a URI says so itself.** `https://…` is handled by whatever registered
//!   `x-scheme-handler/https`; the scheme is read straight off the argument.
//! - **a file does not.** Its type is content, and the desktop's answer is the
//!   shared MIME database's glob table (`$XDG_DATA_DIRS/mime/globs2`, with the
//!   older `mime/globs` as a fallback): patterns such as `*.png` matched
//!   against the file name.
//!
//! ## What this module deliberately does not do
//!
//! **No magic-number sniffing.** The real database also carries `magic` rules
//! that read the first bytes of a file; this reader does not, so a file whose
//! extension lies is opened by the application the extension names. That is a
//! stated gap rather than an accident: reading content would mean a second,
//! hand-written parser of a binary format for a benefit that only shows up on
//! files with wrong or missing extensions — and the extension is what the user
//! sees in their file manager anyway.
//!
//! **No cache reuse.** `mime.cache` is the database's compiled form, and
//! decoding it would be a versioned binary format to get wrong. `globs2` is
//! text, small, and the format this module can be honest about.

use std::path::PathBuf;

/// The URI scheme `target` names, if it names one.
///
/// A scheme is a letter followed by letters, digits and `+`, `-` or `.`, ended
/// by a colon (RFC 3986 §3.1). Anything else — an absolute path, a relative
/// path containing no colon — has no scheme, which is what keeps
/// `/etc/passwd` a file rather than a URI whose scheme is `/etc/passwd`.
///
/// A relative name that happens to look like a scheme (`a:b.txt`) is read as a
/// URI, exactly as `xdg-open` reads it: the ambiguity cannot be resolved
/// without looking at the file system, and the URI reading is the one the user
/// meant in every real case.
pub(crate) fn scheme(target: &str) -> Option<&str> {
    let (candidate, _rest) = target.split_once(':')?;

    // A bound that keeps a pathological argument from being scanned in full;
    // no real scheme is anywhere near this long.
    if candidate.is_empty() || candidate.len() > 32 {
        return None;
    }

    let mut characters = candidate.chars();
    let first = characters.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    let valid = characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.'));

    valid.then_some(candidate)
}

/// Resolves the `%XX` escapes of a `file:` URI.
///
/// An escape that is not two hexadecimal digits is left as written: the URI is
/// then wrong, and a path that contains the three characters `%`, `2`, `0` is
/// a better guess than dropping them.
pub(crate) fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = (bytes[index + 1] as char).to_digit(16);
            let low = (bytes[index + 2] as char).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                decoded.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

/// One glob rule of the shared MIME database.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    /// The rule's weight; the highest matching weight names the type.
    weight: u32,
    /// The MIME type the rule names.
    mime: String,
    /// The pattern, as written in the database.
    pattern: String,
    /// Whether the pattern is matched case-sensitively (`cs` in `globs2`).
    case_sensitive: bool,
}

/// The shared MIME database's glob rules, ordered so that the first match wins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GlobTable {
    rules: Vec<Rule>,
}

impl GlobTable {
    /// The MIME type of the file called `name`, if any rule matches it.
    ///
    /// Rules arrive sorted by descending weight, so the first match is the
    /// answer: `globs2` gives `*.tar.gz` a higher weight than `*.gz` precisely
    /// so that the more specific pattern wins.
    pub(crate) fn lookup(&self, name: &str) -> Option<&str> {
        self.rules
            .iter()
            .find(|rule| glob_matches(&rule.pattern, name, rule.case_sensitive))
            .map(|rule| rule.mime.as_str())
    }

    /// Whether the table holds no rule at all.
    ///
    /// True when the MIME database is not installed where this session looks
    /// for it, which is a different failure from "no rule matched this name".
    pub(crate) fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

/// Reads the glob table from the first database file `data_dirs` provides.
///
/// `globs2` is preferred over the deprecated `globs` in every directory, and
/// the first directory that has either wins: an XDG data directory earlier in
/// the search path overrides the system's database, which is the order the
/// specification asks for and the only one that lets a user fix a wrong entry.
pub(crate) fn load(data_dirs: &[PathBuf]) -> GlobTable {
    for directory in data_dirs {
        for (name, modern) in [("mime/globs2", true), ("mime/globs", false)] {
            let Ok(source) = std::fs::read_to_string(directory.join(name)) else {
                continue;
            };
            let table = parse(&source, modern);
            if !table.is_empty() {
                return table;
            }
        }
    }
    GlobTable::default()
}

/// Parses a `globs2` (`modern`) or `globs` table.
///
/// The two formats differ by one leading field, so one parser reads both. Lines
/// that are empty, comments or malformed are skipped: the database is a system
/// file that a distribution may have patched, and one bad line must not cost
/// every other type its handler.
pub(crate) fn parse(source: &str, modern: bool) -> GlobTable {
    let mut rules: Vec<Rule> = Vec::new();

    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (weight, mime, pattern, flags) = if modern {
            let fields: Vec<&str> = line.splitn(4, ':').collect();
            match fields.as_slice() {
                [weight, mime, pattern] => (field(weight, 0), *mime, *pattern, ""),
                [weight, mime, pattern, flags] => (field(weight, 0), *mime, *pattern, *flags),
                _ => continue,
            }
        } else {
            let fields: Vec<&str> = line.splitn(2, ':').collect();
            match fields.as_slice() {
                // The legacy format has no weight, and the specification ranks
                // a more specific pattern higher: pattern length is the closest
                // thing to that it offers.
                [mime, pattern] => (pattern_length(pattern), *mime, *pattern, ""),
                _ => continue,
            }
        };

        if mime.is_empty() || !mime.contains('/') || pattern.is_empty() {
            continue;
        }

        rules.push(Rule {
            weight,
            mime: mime.to_owned(),
            pattern: pattern.to_owned(),
            case_sensitive: flags.split(',').any(|flag| flag.trim() == "cs"),
        });
    }

    // Descending weight, and a stable sort so that two rules of equal weight
    // keep the database's own order rather than an arbitrary one.
    rules.sort_by_key(|rule| std::cmp::Reverse(rule.weight));
    GlobTable { rules }
}

/// The weight a legacy `globs` pattern gets: its own length.
fn pattern_length(pattern: &str) -> u32 {
    u32::try_from(pattern.len()).unwrap_or(u32::MAX)
}

/// Parses a leading numeric field, treating anything unreadable as `fallback`.
fn field(value: &str, fallback: u32) -> u32 {
    value.trim().parse().unwrap_or(fallback)
}

/// Matches one file name against one glob pattern.
///
/// The pattern language is the database's: `*` matches any run of characters,
/// `?` matches exactly one, `[…]` matches one character from a set (with ranges
/// and a leading `!` or `^` for negation), and every other character stands for
/// itself. A pattern whose `[` is never closed is matched literally, which is
/// what a reader looking at such a line would do.
fn glob_matches(pattern: &str, name: &str, case_sensitive: bool) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();

    let mut pattern_index = 0;
    let mut name_index = 0;
    // Where to resume after a `*` fails: the pattern position just past the
    // star, and the name position it has grown to cover.
    let mut star: Option<(usize, usize)> = None;

    while name_index < name.len() {
        if pattern_index < pattern.len() && pattern[pattern_index] == '*' {
            star = Some((pattern_index + 1, name_index));
            pattern_index += 1;
            continue;
        }

        if let Some(width) = width_of_one(&pattern, pattern_index, name[name_index], case_sensitive)
        {
            pattern_index += width;
            name_index += 1;
            continue;
        }

        match star {
            Some((resume_pattern, resume_name)) => {
                pattern_index = resume_pattern;
                name_index = resume_name + 1;
                star = Some((resume_pattern, resume_name + 1));
            }
            None => return false,
        }
    }

    pattern[pattern_index..]
        .iter()
        .all(|character| *character == '*')
}

/// How many pattern characters a single-character matcher consumes, if it
/// matches `subject`.
///
/// `None` means the pattern does not match there. The width is one for everything
/// but a character class, which occupies as many characters as it is written in.
fn width_of_one(
    pattern: &[char],
    index: usize,
    subject: char,
    case_sensitive: bool,
) -> Option<usize> {
    let &character = pattern.get(index)?;

    match character {
        '?' => Some(1),
        '[' => match class_end(pattern, index) {
            Some(end) => {
                let negated = matches!(pattern.get(index + 1), Some('!' | '^'));
                let mut matched = false;
                let mut cursor = if negated { index + 2 } else { index + 1 };

                while cursor < end {
                    let low = pattern[cursor];
                    // `a-z` is a range; a `-` at either end is a literal.
                    if cursor + 2 < end && pattern[cursor + 1] == '-' {
                        let high = pattern[cursor + 2];
                        if within(subject, low, high, case_sensitive) {
                            matched = true;
                        }
                        cursor += 3;
                        continue;
                    }
                    if same(subject, low, case_sensitive) {
                        matched = true;
                    }
                    cursor += 1;
                }

                (matched != negated).then_some(end - index + 1)
            }
            // An unclosed class is an ordinary character.
            None => same(subject, '[', case_sensitive).then_some(1),
        },
        _ => same(subject, character, case_sensitive).then_some(1),
    }
}

/// The index of the `]` that closes the class starting at `open`.
fn class_end(pattern: &[char], open: usize) -> Option<usize> {
    let mut cursor = open + 1;
    // `[]a]` names `]` and `a`: a `]` first is a member, not the end.
    if matches!(pattern.get(cursor), Some(']')) {
        cursor += 1;
    }
    while cursor < pattern.len() {
        if pattern[cursor] == ']' {
            return Some(cursor);
        }
        cursor += 1;
    }
    None
}

/// Whether two characters are the same, honouring the case-insensitive default.
fn same(left: char, right: char, case_sensitive: bool) -> bool {
    if case_sensitive {
        left == right
    } else {
        left.to_lowercase().eq(right.to_lowercase())
    }
}

/// Whether `subject` falls inside the inclusive range `low..=high`.
fn within(subject: char, low: char, high: char, case_sensitive: bool) -> bool {
    let subject = if case_sensitive {
        subject
    } else {
        subject.to_ascii_lowercase()
    };
    let low = if case_sensitive {
        low
    } else {
        low.to_ascii_lowercase()
    };
    let high = if case_sensitive {
        high
    } else {
        high.to_ascii_lowercase()
    };

    low <= subject && subject <= high
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scheme_is_read_before_the_first_colon() {
        assert_eq!(scheme("https://example.org/x"), Some("https"));
        assert_eq!(scheme("mailto:someone@example.org"), Some("mailto"));
        assert_eq!(scheme("file:///tmp/a.txt"), Some("file"));
        assert_eq!(scheme("x-scheme-handler+x:y"), Some("x-scheme-handler+x"));
    }

    #[test]
    fn an_absolute_or_relative_path_has_no_scheme() {
        assert_eq!(scheme("/etc/passwd"), None);
        assert_eq!(scheme("./notes.txt"), None);
        assert_eq!(scheme("notes.txt"), None);
        // A colon after a slash is a path, not a scheme.
        assert_eq!(scheme("/tmp/a:b"), None);
    }

    #[test]
    fn a_leading_digit_or_an_overlong_word_is_not_a_scheme() {
        assert_eq!(scheme("1http://example.org"), None);
        assert_eq!(scheme(""), None);
        let overlong = format!("{}:x", "a".repeat(33));
        assert_eq!(scheme(&overlong), None);
    }

    #[test]
    fn percent_escapes_are_decoded_and_bad_ones_are_kept() {
        assert_eq!(percent_decode("/tmp/a%20b.txt"), "/tmp/a b.txt");
        assert_eq!(percent_decode("/tmp/%E4%B8%AD.txt"), "/tmp/中.txt");
        // Not two hex digits: leave the three characters alone.
        assert_eq!(percent_decode("/tmp/100%zz"), "/tmp/100%zz");
        assert_eq!(percent_decode("%"), "%");
        assert_eq!(percent_decode("%4"), "%4");
    }

    #[test]
    fn a_star_matches_any_run_including_none() {
        assert!(glob_matches("*.txt", "notes.txt", false));
        assert!(glob_matches("*.txt", ".txt", false));
        assert!(glob_matches("notes*", "notes.txt", false));
        assert!(glob_matches("*", "anything", false));
        assert!(glob_matches("*", "", false));
        assert!(!glob_matches("*.txt", "notes.md", false));
    }

    #[test]
    fn a_question_mark_matches_exactly_one_character() {
        assert!(glob_matches("?.txt", "a.txt", false));
        assert!(!glob_matches("?.txt", "ab.txt", false));
        assert!(!glob_matches("?.txt", ".txt", false));
    }

    #[test]
    fn a_class_matches_its_set_and_its_ranges() {
        assert!(glob_matches("file[0-9].txt", "file7.txt", false));
        assert!(!glob_matches("file[0-9].txt", "filex.txt", false));
        assert!(glob_matches("file[abc].txt", "fileb.txt", false));
        assert!(!glob_matches("file[!abc].txt", "fileb.txt", false));
        assert!(glob_matches("file[!abc].txt", "filed.txt", false));
        assert!(glob_matches("file[]].txt", "file].txt", false));
    }

    #[test]
    fn an_unclosed_class_is_a_literal_bracket() {
        assert!(glob_matches("file[.txt", "file[.txt", false));
        assert!(!glob_matches("file[.txt", "filea.txt", false));
    }

    #[test]
    fn matching_is_case_insensitive_unless_the_rule_says_otherwise() {
        assert!(glob_matches("*.TXT", "notes.txt", false));
        assert!(!glob_matches("*.TXT", "notes.txt", true));
        assert!(glob_matches("*.txt", "notes.txt", true));
    }

    #[test]
    fn the_most_specific_glob_wins() {
        let table = parse(
            "50:application/gzip:*.gz\n80:application/x-compressed-tar:*.tar.gz\n",
            true,
        );
        assert_eq!(
            table.lookup("archive.tar.gz"),
            Some("application/x-compressed-tar")
        );
        assert_eq!(table.lookup("archive.gz"), Some("application/gzip"));
    }

    #[test]
    fn a_case_sensitive_rule_is_marked_by_its_flag() {
        let table = parse("50:text/x-upper:*.TXT:cs\n", true);
        assert_eq!(table.lookup("A.TXT"), Some("text/x-upper"));
        assert_eq!(table.lookup("a.txt"), None);
    }

    #[test]
    fn the_legacy_format_reads_without_a_weight() {
        let table = parse("text/plain:*.txt\nimage/png:*.png\n", false);
        assert_eq!(table.lookup("a.txt"), Some("text/plain"));
        assert_eq!(table.lookup("a.png"), Some("image/png"));
    }

    #[test]
    fn comments_blank_lines_and_junk_are_skipped() {
        let table = parse(
            "# comment\n\nnot a rule\nnonsense:*.x\n50:text/plain:*.txt\n",
            true,
        );
        assert_eq!(table.lookup("a.txt"), Some("text/plain"));
        // `nonsense` has no `/`, so it is not a MIME type and its rule is junk.
        assert_eq!(table.lookup("a.x"), None);
    }

    #[test]
    fn an_unknown_name_has_no_type() {
        let table = parse("50:text/plain:*.txt\n", true);
        assert_eq!(table.lookup("notes.unknown"), None);
        assert!(!table.is_empty());
        assert!(GlobTable::default().is_empty());
    }

    use proptest::prelude::*;

    /// Characters that mean nothing special to the pattern language, are not
    /// trimmed off a line, and do not separate the database's fields.
    const PLAIN: &[char] = &['a', 'b', 'z', 'A', 'Z', '0', '9', '.', '_', '-', '+', '~'];

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// The matcher terminates and answers for every pattern and name a
        /// database and a file system could hand it, including the malformed
        /// patterns a patched system file might contain.
        #[test]
        fn every_pattern_and_name_is_answered(
            pattern in "[ -~]{0,40}",
            name in "[ -~]{0,40}",
        ) {
            let table = parse(&format!("50:text/plain:{pattern}\n"), true);
            let _ = table.lookup(&name);
        }

        /// A pattern of nothing but stars matches every name.
        #[test]
        fn a_pattern_of_stars_matches_every_name(name in "[ -~]{0,40}") {
            let prefixed = format!("a{name}");
            prop_assert!(glob_matches("*", &name, true));
            prop_assert!(glob_matches("**", &name, true));
            prop_assert!(glob_matches("a*", &prefixed, true));
        }

        /// A pattern with no metacharacter in it matches exactly one name: the
        /// one it is.
        #[test]
        fn a_literal_pattern_matches_only_itself(
            literal in proptest::collection::vec(proptest::sample::select(PLAIN), 1..20),
        ) {
            let literal: String = literal.into_iter().collect();
            let table = parse(&format!("50:text/plain:{literal}\n"), true);
            let extended = format!("{literal}x");

            prop_assert_eq!(table.lookup(&literal), Some("text/plain"));
            prop_assert_eq!(table.lookup(&extended), None);
        }
    }
}
