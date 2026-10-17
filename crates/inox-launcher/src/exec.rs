//! `Exec` field-code expansion into an argv.
//!
//! An `Exec` value is not a command line: it is a *list of arguments* with
//! field codes in it. The difference is the whole point of this module. The
//! entry's author writes separators, quotes and backslashes to protect
//! argument boundaries, and the reader's job is to recover the argument list
//! and hand it to `execve` — never to build a string that some shell will
//! interpret afterwards. So [`argv`] returns `Vec<String>` and there is no
//! function anywhere in this crate that returns a command line, which is what
//! makes acceptance D02's "the search query is never turned into a shell
//! command" a property of the types rather than a rule someone has to remember.
//! The query has no way into this function: [`ExecContext`] has no field for it.
//!
//! ## The grammar, as implemented
//!
//! Arguments are separated by unquoted spaces and tabs. A double quote groups
//! text into one argument; a backslash escapes the next character. Both are
//! resolved *before* field codes are expanded, so a quoted `%` is a literal
//! percent sign.
//!
//! | code | expands to |
//! |------|------------|
//! | `%i` | the two arguments `--icon` and the entry's `Icon`, or nothing |
//! | `%f` `%u` | the first file or URL, or nothing |
//! | `%F` `%U` | every file or URL, as separate arguments |
//! | `%c` | the entry's localized name |
//! | `%k` | the path of the desktop file |
//! | `%%` | a literal `%` |
//! | `%d` `%D` `%n` `%N` `%v` `%m` | nothing (deprecated by the specification) |
//!
//! `%i`, `%F` and `%U` are the codes that produce more than one argument, so
//! they are only meaningful as an entire argument: `--open=%F` has no defined
//! meaning and is rejected as [`FailureKind::MisplacedFieldCode`] rather than
//! guessed at. Any other `%x` is [`FailureKind::UnknownFieldCode`]: silently
//! passing `%x` through to the application would start it with an argument its
//! author never wrote.

use std::path::Path;

use crate::contract::FailureKind;

/// Everything a field code can be expanded from.
///
/// The absence of a query field is deliberate: see the module documentation.
#[derive(Debug, Clone)]
pub struct ExecContext<'a> {
    /// The entry's `Exec` value, exactly as the file wrote it.
    pub exec: &'a str,
    /// The entry's localized name, for `%c`.
    pub name: &'a str,
    /// The entry's icon name, for `%i`.
    pub icon: Option<&'a str>,
    /// The desktop file's path, for `%k`.
    pub desktop_file: &'a Path,
    /// The files or URLs the application is being opened with, for `%f`,
    /// `%F`, `%u` and `%U`. The interactive launcher passes none, and the field
    /// exists because the grammar defines those codes; `inox-launcher` is
    /// invoked without arguments today.
    pub files: &'a [String],
    /// Whether the entry sets `Terminal=true`.
    pub terminal: bool,
    /// `shell.toml`'s `launcher.terminal`: the argv prefix that opens a
    /// terminal, for example `["alacritty", "-e"]`.
    pub terminal_prefix: &'a [String],
}

/// Expands an entry's command line into the argv to start it with.
///
/// # Errors
///
/// Returns the [`FailureKind`] that makes the entry unlaunchable: an Exec value
/// this parser cannot read ([`FailureKind::UnknownFieldCode`],
/// [`FailureKind::MisplacedFieldCode`], [`FailureKind::UnterminatedQuote`],
/// [`FailureKind::TrailingEscape`], [`FailureKind::TrailingPercent`]), an entry
/// that asks for a terminal and has none configured
/// ([`FailureKind::MissingTerminalPrefix`]), or one whose expansion leaves no
/// program at all ([`FailureKind::NoCommand`]).
pub fn argv(context: &ExecContext<'_>) -> Result<Vec<String>, FailureKind> {
    let words = tokenize(context.exec)?;

    let mut expanded = Vec::with_capacity(words.len() + context.terminal_prefix.len());
    for word in &words {
        expand_word(word, context, &mut expanded)?;
    }

    if context.terminal {
        if context.terminal_prefix.is_empty() {
            return Err(FailureKind::MissingTerminalPrefix);
        }
        let mut prefixed = context.terminal_prefix.to_vec();
        prefixed.append(&mut expanded);
        expanded = prefixed;
    }

    if expanded.is_empty() {
        // `Exec=%F` with no files, for instance: there is nothing to run.
        return Err(FailureKind::NoCommand);
    }

    Ok(expanded)
}

/// One piece of an argument.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    /// Text that is passed through as written.
    Literal(String),
    /// A field code, which only exists outside double quotes.
    Code {
        /// The character after the `%`.
        code: char,
    },
}

/// Splits an `Exec` value into arguments, resolving quotes and escapes.
fn tokenize(exec: &str) -> Result<Vec<Vec<Segment>>, FailureKind> {
    let mut words = Vec::new();
    let mut word: Vec<Segment> = Vec::new();
    let mut quoted = false;
    let mut started = false;
    let mut characters = exec.chars();

    while let Some(character) = characters.next() {
        match character {
            ' ' | '\t' if !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' => {
                let Some(escaped) = characters.next() else {
                    return Err(FailureKind::TrailingEscape);
                };
                push_literal(&mut word, escaped);
                started = true;
            }
            // Inside double quotes a `%` is an ordinary character: the
            // specification leaves a quoted field code undefined, and reading
            // one as a code would also let a quote double as a code letter.
            '%' if !quoted => {
                let Some(code) = characters.next() else {
                    return Err(FailureKind::TrailingPercent);
                };
                word.push(Segment::Code { code });
                started = true;
            }
            other => {
                push_literal(&mut word, other);
                started = true;
            }
        }
    }

    if quoted {
        return Err(FailureKind::UnterminatedQuote);
    }
    if started {
        words.push(word);
    }

    Ok(words)
}

/// Appends `character` to the argument's trailing literal run.
fn push_literal(word: &mut Vec<Segment>, character: char) {
    match word.last_mut() {
        Some(Segment::Literal(literal)) => literal.push(character),
        _ => word.push(Segment::Literal(character.to_string())),
    }
}

/// Expands one argument.
fn expand_word(
    word: &[Segment],
    context: &ExecContext<'_>,
    expanded: &mut Vec<String>,
) -> Result<(), FailureKind> {
    // A field code that is an argument of its own is the normal shape, and the
    // only one in which it may produce zero or several arguments.
    if let [Segment::Code { code }] = word {
        return expand_alone(*code, context, expanded);
    }

    let mut argument = String::new();
    for segment in word {
        match segment {
            Segment::Literal(literal) => argument.push_str(literal),
            Segment::Code { code } => {
                // The codes that produce several arguments have no meaning
                // inside a larger argument, and guessing at one would start an
                // application with an argument its author never wrote.
                if is_multi_valued(*code) {
                    return Err(FailureKind::MisplacedFieldCode { code: *code });
                }
                argument.push_str(&single_value(*code, context)?);
            }
        }
    }

    expanded.push(argument);

    Ok(())
}

/// Expands a field code that forms an entire argument.
fn expand_alone(
    code: char,
    context: &ExecContext<'_>,
    expanded: &mut Vec<String>,
) -> Result<(), FailureKind> {
    match code {
        'i' => {
            if let Some(icon) = context.icon {
                expanded.push("--icon".to_owned());
                expanded.push(icon.to_owned());
            }
        }
        'F' | 'U' => expanded.extend(context.files.iter().cloned()),
        other => {
            let value = single_value(other, context)?;
            if !value.is_empty() {
                expanded.push(value);
            }
        }
    }
    Ok(())
}

/// Whether a field code can produce more than one argument.
fn is_multi_valued(code: char) -> bool {
    matches!(code, 'i' | 'F' | 'U')
}

/// The value of a field code that produces at most one argument.
fn single_value(code: char, context: &ExecContext<'_>) -> Result<String, FailureKind> {
    let value = match code {
        'f' | 'u' => context.files.first().cloned().unwrap_or_default(),
        'c' => context.name.to_owned(),
        // A desktop file path that is not valid UTF-8 cannot be passed to
        // `execve` as written; replacing the undecodable bytes is the
        // degradation that keeps an otherwise fine entry launchable.
        'k' => context.desktop_file.to_string_lossy().into_owned(),
        '%' => "%".to_owned(),
        'd' | 'D' | 'n' | 'N' | 'v' | 'm' => String::new(),
        other => return Err(FailureKind::UnknownFieldCode { code: other }),
    };
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn context<'a>(exec: &'a str, files: &'a [String], prefix: &'a [String]) -> ExecContext<'a> {
        ExecContext {
            exec,
            name: "Text Editor",
            icon: Some("editor"),
            desktop_file: Path::new("/usr/share/applications/editor.desktop"),
            files,
            terminal: false,
            terminal_prefix: prefix,
        }
    }

    fn expand(exec: &str) -> Result<Vec<String>, FailureKind> {
        argv(&context(exec, &[], &[]))
    }

    #[test]
    fn plain_words_become_arguments() {
        assert_eq!(
            expand("/usr/bin/editor --new-window").expect("valid"),
            ["/usr/bin/editor", "--new-window"]
        );
    }

    #[test]
    fn repeated_spaces_and_tabs_do_not_produce_empty_arguments() {
        assert_eq!(expand("  a \t  b  ").expect("valid"), ["a", "b"]);
    }

    #[test]
    fn double_quotes_keep_spaces_in_one_argument() {
        assert_eq!(
            expand("app \"one two\" three").expect("valid"),
            ["app", "one two", "three"]
        );
    }

    #[test]
    fn an_empty_quoted_argument_is_passed_through() {
        assert_eq!(expand("app \"\" next").expect("valid"), ["app", "", "next"]);
    }

    #[test]
    fn backslash_escapes_the_next_character() {
        assert_eq!(
            expand(r#"app one\ two \$HOME"#).expect("valid"),
            ["app", "one two", "$HOME"]
        );
        assert_eq!(
            expand(r#"app "a \"quoted\" word""#).expect("valid"),
            ["app", "a \"quoted\" word"]
        );
    }

    #[test]
    fn a_trailing_backslash_is_an_error() {
        assert_eq!(expand(r"app trailing\"), Err(FailureKind::TrailingEscape));
    }

    #[test]
    fn an_unterminated_quote_is_an_error() {
        assert_eq!(
            expand("app \"unfinished"),
            Err(FailureKind::UnterminatedQuote)
        );
    }

    #[test]
    fn a_trailing_percent_is_an_error() {
        assert_eq!(expand("app 100%"), Err(FailureKind::TrailingPercent));
    }

    #[test]
    fn the_icon_code_becomes_the_two_element_prefix() {
        assert_eq!(
            expand("app %i").expect("valid"),
            ["app", "--icon", "editor"]
        );
    }

    #[test]
    fn the_icon_code_vanishes_without_an_icon() {
        let mut exec_context = context("app %i", &[], &[]);
        exec_context.icon = None;
        assert_eq!(argv(&exec_context).expect("valid"), ["app"]);
    }

    #[test]
    fn the_name_and_path_codes_expand_in_place() {
        assert_eq!(
            expand("app --name=%c --file=%k").expect("valid"),
            [
                "app",
                "--name=Text Editor",
                "--file=/usr/share/applications/editor.desktop"
            ]
        );
    }

    #[test]
    fn a_quoted_code_is_literal_text() {
        assert_eq!(
            expand("app \"%c\"").expect("valid"),
            ["app", "%c"],
            "the specification leaves a quoted field code undefined; literal text \
             is the reading that cannot start an application with a wrong argument"
        );
    }

    #[test]
    fn percent_percent_is_a_literal_percent() {
        assert_eq!(expand("app 100%%").expect("valid"), ["app", "100%"]);
    }

    #[test]
    fn file_codes_expand_to_the_files() {
        let files = vec!["/tmp/one.txt".to_owned(), "/tmp/two.txt".to_owned()];
        let exec_context = context("app %F", &files, &[]);
        assert_eq!(
            argv(&exec_context).expect("valid"),
            ["app", "/tmp/one.txt", "/tmp/two.txt"]
        );

        let exec_context = context("app %f", &files, &[]);
        assert_eq!(argv(&exec_context).expect("valid"), ["app", "/tmp/one.txt"]);

        let exec_context = context("app %U", &files, &[]);
        assert_eq!(
            argv(&exec_context).expect("valid"),
            ["app", "/tmp/one.txt", "/tmp/two.txt"]
        );

        let exec_context = context("app %u", &files, &[]);
        assert_eq!(argv(&exec_context).expect("valid"), ["app", "/tmp/one.txt"]);
    }

    #[test]
    fn a_file_code_without_files_removes_the_argument() {
        assert_eq!(expand("app %F").expect("valid"), ["app"]);
        assert_eq!(expand("app %f").expect("valid"), ["app"]);
    }

    #[test]
    fn an_exec_that_is_only_a_file_code_leaves_nothing_to_run() {
        assert_eq!(expand("%F"), Err(FailureKind::NoCommand));
    }

    #[test]
    fn deprecated_codes_are_ignored() {
        assert_eq!(
            expand("app %d %D %n %N %v %m --real").expect("valid"),
            ["app", "--real"]
        );
    }

    #[test]
    fn an_undefined_code_is_rejected() {
        assert_eq!(
            expand("app %q"),
            Err(FailureKind::UnknownFieldCode { code: 'q' })
        );
    }

    #[test]
    fn a_multi_valued_code_needs_to_be_a_whole_argument() {
        assert_eq!(
            expand("app --open=%F"),
            Err(FailureKind::MisplacedFieldCode { code: 'F' })
        );
        assert_eq!(
            expand("app -i%i"),
            Err(FailureKind::MisplacedFieldCode { code: 'i' })
        );
    }

    #[test]
    fn a_terminal_entry_is_prefixed() {
        let prefix = vec!["alacritty".to_owned(), "-e".to_owned()];
        let mut exec_context = context("htop", &[], &prefix);
        exec_context.terminal = true;
        assert_eq!(
            argv(&exec_context).expect("valid"),
            ["alacritty", "-e", "htop"]
        );
    }

    #[test]
    fn a_terminal_entry_without_a_prefix_is_rejected() {
        let mut exec_context = context("htop", &[], &[]);
        exec_context.terminal = true;
        assert_eq!(argv(&exec_context), Err(FailureKind::MissingTerminalPrefix));
    }

    #[test]
    fn an_empty_exec_is_rejected() {
        assert_eq!(expand("   "), Err(FailureKind::NoCommand));
    }

    #[test]
    fn metacharacters_stay_inside_their_argument() {
        // D02: the search query never reaches here, and if a file name ever
        // did, it would arrive as one argument rather than as a command.
        let files = vec!["/tmp/a; rm -rf ~ $(whoami) | tee .x".to_owned()];
        let exec_context = context("app %U", &files, &[]);
        assert_eq!(
            argv(&exec_context).expect("valid"),
            ["app", "/tmp/a; rm -rf ~ $(whoami) | tee .x"]
        );
    }

    #[test]
    fn a_semicolon_in_an_exec_is_an_ordinary_character() {
        assert_eq!(
            expand("app foo;bar").expect("valid"),
            ["app", "foo;bar"],
            "semicolons separate list values, not arguments"
        );
    }

    /// The inverse of [`tokenize`]: a word written the way the specification
    /// asks an author to write one.
    ///
    /// It exists so the round-trip property can be stated at all; the launcher
    /// never writes an `Exec` value.
    fn quote_argument(argument: &str) -> String {
        let mut quoted = String::from("\"");
        for character in argument.chars() {
            match character {
                '"' | '\\' => {
                    quoted.push('\\');
                    quoted.push(character);
                }
                other => quoted.push(other),
            }
        }
        quoted.push('"');
        quoted
    }

    fn round_trip(argument: &str) -> Vec<String> {
        expand(&quote_argument(argument)).expect("a quoted argument parses")
    }

    #[test]
    fn the_round_trip_helper_works_on_a_known_case() {
        assert_eq!(round_trip("plain"), ["plain"]);
        assert_eq!(round_trip("two words"), ["two words"]);
        assert_eq!(round_trip(""), [""]);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// Any argument survives being quoted and read back.
        #[test]
        fn quoted_arguments_round_trip(argument in ".*") {
            prop_assert_eq!(round_trip(&argument), vec![argument]);
        }

        /// No expansion ever produces an argument that `execve` cannot take,
        /// and none of them is ever split on a metacharacter: what the entry
        /// wrote as one argument stays one argument.
        #[test]
        fn expansion_never_splits_an_argument(argument in "[ -~]{0,40}") {
            let files = vec![argument.clone()];
            let exec_context = context("app %U", &files, &[]);
            match argv(&exec_context) {
                Ok(argv) => {
                    prop_assert_eq!(argv.len(), 2);
                    prop_assert!(!argv.iter().any(|it| it.contains('\0')));
                }
                Err(error) => prop_assert_eq!(error, FailureKind::NoCommand),
            }
        }

        /// The tokenizer terminates and either reads the value or names a
        /// reason, for every value a file could contain.
        #[test]
        fn every_exec_value_is_read_or_rejected(exec in ".*") {
            let files = vec!["/tmp/a".to_owned()];
            let _ = argv(&context(&exec, &files, &[]));
        }
    }
}
