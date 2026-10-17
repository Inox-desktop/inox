//! The body sanitiser: the subset of markup a notification may use, as spans.
//!
//! A notification body is untrusted input from any application on the session,
//! and the specification lets it carry a little markup (`<b>`, `<i>`, `<u>`,
//! `<a href="…">`, `<img src="…" alt="…">`). Acceptance D06 asks for the part
//! that matters: **the text must not execute anything**. This module is where
//! that is decided.
//!
//! Three properties are worth stating before the code, because they are the
//! rules the tests are written against:
//!
//! - **The output is spans, never a string of markup.** There is no HTML on the
//!   way out, so there is no second interpreter to get wrong: [`Span::Link`] and
//!   [`Span::Image`] are the only values that carry a target, and both are
//!   constructed here and nowhere else.
//! - **"Escaping" means the characters stay as text.** Anything that is not
//!   accepted markup — an unknown tag, a stray `<`, a link with a scheme this
//!   daemon refuses — is kept *literally*, exactly as the client wrote it. It is
//!   not entity-encoded, because entities would only matter to an HTML renderer
//!   that does not exist here; and it is not stripped, because a user reading a
//!   notification should see what was sent. `plain_text` of the result is
//!   therefore the input, character for character, whenever the input carried no
//!   accepted markup.
//! - **Sanitising never grows the text.** Every output span takes its characters
//!   from the input, so a body never expands on its way to the screen — which is
//!   what stops a hostile body from being a denial of service by amplification.
//!
//! A link is accepted only for `http`, `https` and `mailto`, and an image source
//! only when it is an `http`/`https` URL or a plain path: `javascript:`,
//! `data:`, `file:` and friends never reach a [`Span::Link`] or a
//! [`Span::Image`], so no drawing path can be handed one.

/// The longest tag this sanitiser will look at, in characters.
///
/// A `<` followed by more than this many characters before a `>` is text, not a
/// tag: the bound keeps a body that is one enormous unterminated `<` from being
/// scanned twice.
pub const MAX_TAG_CHARS: usize = 256;

/// The most text one body may contribute, in characters.
///
/// A notification body is a message, not a document. The limit bounds the work
/// one `Notify` call can ask for and the memory one popup can hold; a client
/// that sends more gets the first [`MAX_TEXT_CHARS`] characters shown.
pub const MAX_TEXT_CHARS: usize = 4096;

/// The most spans one body may produce.
///
/// Bound for the same reason: `<b></b>` repeated produces no text but does
/// produce work.
pub const MAX_SPANS: usize = 256;

/// The markup an accepted link may carry.
const LINK_SCHEMES: [&str; 3] = ["http://", "https://", "mailto:"];

/// The markup an accepted image source may carry, when it is a URL.
const IMAGE_SCHEMES: [&str; 2] = ["http://", "https://"];

/// Which of the three styling elements a run of text sits inside.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Emphasis {
    /// Whether the text is inside a `<b>`.
    pub bold: bool,
    /// Whether the text is inside an `<i>`.
    pub italic: bool,
    /// Whether the text is inside a `<u>`.
    pub underline: bool,
}

impl Emphasis {
    /// Text with no styling at all.
    pub const PLAIN: Self = Self {
        bold: false,
        italic: false,
        underline: false,
    };

    /// Whether the text carries no styling.
    pub const fn is_plain(self) -> bool {
        !self.bold && !self.italic && !self.underline
    }

    /// The styling of nested elements: the union of both.
    pub const fn union(self, other: Self) -> Self {
        Self {
            bold: self.bold || other.bold,
            italic: self.italic || other.italic,
            underline: self.underline || other.underline,
        }
    }
}

/// One piece of a sanitised body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    /// Text, with whatever styling was open around it.
    Text {
        /// The characters, exactly as the client sent them.
        text: String,
        /// The styling in force where the text stood.
        emphasis: Emphasis,
    },
    /// A hyperlink, accepted only for the schemes in `LINK_SCHEMES`.
    Link {
        /// The text the user sees and clicks.
        text: String,
        /// The target, which passed the scheme check when the span was built.
        href: String,
        /// The styling in force where the link stood.
        emphasis: Emphasis,
    },
    /// An image the daemon does not decode; the alt text stands in for it.
    Image {
        /// Where the image is, accepted only when it is an HTTP(S) URL or a path.
        src: String,
        /// The text to show instead of the image, as the client wrote it.
        alt: String,
        /// The styling in force where the image stood.
        emphasis: Emphasis,
    },
}

impl Span {
    /// The characters this span contributes to the text a user reads.
    pub fn text(&self) -> &str {
        match self {
            Self::Text { text, .. } | Self::Link { text, .. } => text,
            Self::Image { alt, .. } => alt,
        }
    }

    /// The styling this span is drawn with.
    pub const fn emphasis(&self) -> Emphasis {
        match self {
            Self::Text { emphasis, .. }
            | Self::Link { emphasis, .. }
            | Self::Image { emphasis, .. } => *emphasis,
        }
    }

    /// How many characters this span contributes.
    pub fn char_count(&self) -> usize {
        self.text().chars().count()
    }
}

/// Sanitises a notification body into the spans a popup draws.
///
/// See the module documentation for what is accepted and what happens to
/// everything else. The output is bounded by [`MAX_TEXT_CHARS`] and
/// [`MAX_SPANS`].
pub fn sanitize(body: &str) -> Vec<Span> {
    let characters: Vec<char> = body.chars().collect();
    let mut sanitiser = Sanitiser::default();
    let mut index = 0;

    while index < characters.len() {
        if sanitiser.is_full() {
            break;
        }

        if characters[index] == '<'
            && let Some(end) = tag_end(&characters, index)
            && sanitiser.tag(&characters[index + 1..end])
        {
            index = end + 1;
            continue;
        }

        sanitiser.push_char(characters[index]);
        index += 1;
    }

    sanitiser.finish()
}

/// The text of a sanitised body, with every span flattened.
///
/// This is what a tooltip, a log line or a test compares against the original
/// input; it is also how the redaction tests ask whether a body is present.
pub fn plain_text(spans: &[Span]) -> String {
    let mut text = String::new();

    for span in spans {
        text.push_str(span.text());
    }

    text
}

/// An element that is currently open.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Open {
    /// `<b>`
    Bold,
    /// `<i>`
    Italic,
    /// `<u>`
    Underline,
    /// `<a href="…">`, with the accepted target.
    Link(String),
}

/// An element a closing tag can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Closing {
    /// `</b>`
    Bold,
    /// `</i>`
    Italic,
    /// `</u>`
    Underline,
    /// `</a>`
    Link,
}

/// What a tag the scanner found means.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Tag {
    /// An element that opens here.
    Open(Open),
    /// An element that closes here.
    Close(Closing),
    /// An image, which is a leaf: it is emitted where it stands rather than
    /// opening anything.
    Image {
        /// The accepted source.
        src: String,
        /// The alt text the client gave, which may be empty.
        alt: String,
    },
    /// A recognised shape with an unusable target, an element that is already
    /// open, or a closing tag that matches nothing: text, not markup.
    Literal,
}

/// The sanitiser's working state.
#[derive(Default)]
struct Sanitiser {
    /// The spans emitted so far, in order.
    spans: Vec<Span>,
    /// Text not yet emitted, belonging to the innermost open element.
    pending: String,
    /// The elements currently open, outermost first.
    open: Vec<Open>,
    /// How many characters of text have been emitted or buffered.
    characters: usize,
}

impl Sanitiser {
    /// Whether the output has reached either of its bounds.
    fn is_full(&self) -> bool {
        self.spans.len() >= MAX_SPANS || self.characters >= MAX_TEXT_CHARS
    }

    /// Appends one character of text, if there is room for it.
    fn push_char(&mut self, character: char) {
        if self.characters >= MAX_TEXT_CHARS {
            return;
        }

        self.pending.push(character);
        self.characters += 1;
    }

    /// Emits the pending text, as a link when `href` is given.
    fn flush(&mut self, href: Option<String>) {
        if self.pending.is_empty() {
            return;
        }

        let text = std::mem::take(&mut self.pending);
        let emphasis = self.emphasis();
        let span = match href {
            Some(href) => Span::Link {
                text,
                href,
                emphasis,
            },
            None => Span::Text { text, emphasis },
        };

        self.spans.push(span);
    }

    /// The styling in force where the scanner currently stands.
    fn emphasis(&self) -> Emphasis {
        let mut emphasis = Emphasis::PLAIN;

        for element in &self.open {
            emphasis = emphasis.union(match element {
                Open::Bold => Emphasis {
                    bold: true,
                    ..Emphasis::PLAIN
                },
                Open::Italic => Emphasis {
                    italic: true,
                    ..Emphasis::PLAIN
                },
                Open::Underline => Emphasis {
                    underline: true,
                    ..Emphasis::PLAIN
                },
                Open::Link(_) => Emphasis::PLAIN,
            });
        }

        emphasis
    }

    /// The target of the innermost open link, if one is open.
    fn open_link(&self) -> Option<String> {
        self.open.iter().rev().find_map(|element| match element {
            Open::Link(href) => Some(href.clone()),
            Open::Bold | Open::Italic | Open::Underline => None,
        })
    }

    /// Handles one tag. Returns whether it was markup (and was consumed).
    fn tag(&mut self, inner: &[char]) -> bool {
        match parse_tag(inner) {
            Tag::Open(Open::Link(href)) => {
                // A link inside a link cannot be two things at once; the inner
                // tag is text, and the text after it stays in the outer link.
                if self.open_link().is_some() {
                    return false;
                }

                self.flush(None);
                self.open.push(Open::Link(href));
                true
            }
            Tag::Open(Open::Bold) => {
                self.flush(self.open_link());
                self.open.push(Open::Bold);
                true
            }
            Tag::Open(Open::Italic) => {
                self.flush(self.open_link());
                self.open.push(Open::Italic);
                true
            }
            Tag::Open(Open::Underline) => {
                self.flush(self.open_link());
                self.open.push(Open::Underline);
                true
            }
            Tag::Close(closing) => {
                // Only the innermost element may be closed: `</b>` inside an
                // open `<i>` matches nothing and is shown as text.
                let matches = matches!(
                    (self.open.last(), closing),
                    (Some(Open::Bold), Closing::Bold)
                        | (Some(Open::Italic), Closing::Italic)
                        | (Some(Open::Underline), Closing::Underline)
                        | (Some(Open::Link(_)), Closing::Link)
                );

                if !matches {
                    return false;
                }

                // Whatever was open around the closing tag is what the text
                // belonged to, so the link target comes from the open element
                // rather than from the tag that closed it.
                let href = self.open_link();

                self.flush(href);
                self.open.pop();
                true
            }
            Tag::Image { src, alt } => {
                // An image is a leaf with no text of its own: whatever was
                // pending belongs to the element it stood in, so it is emitted
                // first, and the image takes the styling in force around it.
                self.flush(self.open_link());
                self.spans.push(Span::Image {
                    src,
                    alt,
                    emphasis: self.emphasis(),
                });
                true
            }
            Tag::Literal => false,
        }
    }

    /// Emits what is left, and returns the sanitised body.
    ///
    /// An element that was never closed still contributes the text it held: a
    /// truncated `<b>bold` shows "bold", in bold. An unterminated `<a>` shows
    /// its text as a link, which is safe because the target was already checked
    /// when the tag was read.
    fn finish(mut self) -> Vec<Span> {
        let href = self.open_link();
        self.flush(href);

        self.spans
    }
}

/// The index of the `>` that ends the tag starting at `start`, if there is one.
fn tag_end(characters: &[char], start: usize) -> Option<usize> {
    let limit = (start + 1 + MAX_TAG_CHARS).min(characters.len());

    (start + 1..limit).find(|index| characters[*index] == '>')
}

/// What a tag means.
///
/// `inner` is what stood between `<` and `>`.
fn parse_tag(inner: &[char]) -> Tag {
    let closing = inner.first() == Some(&'/');
    let body = if closing { &inner[1..] } else { inner };

    let name: String = body
        .iter()
        .take_while(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect();

    let attributes = attributes(body);

    match (name.as_str(), closing) {
        ("b", false) => Tag::Open(Open::Bold),
        ("i", false) => Tag::Open(Open::Italic),
        ("u", false) => Tag::Open(Open::Underline),
        ("b", true) => Tag::Close(Closing::Bold),
        ("i", true) => Tag::Close(Closing::Italic),
        ("u", true) => Tag::Close(Closing::Underline),
        ("a", false) => match attribute(&attributes, "href").and_then(accepted_link) {
            Some(href) => Tag::Open(Open::Link(href)),
            None => Tag::Literal,
        },
        ("a", true) => Tag::Close(Closing::Link),
        ("img", false) => {
            let Some(src) = attribute(&attributes, "src").and_then(accepted_image) else {
                return Tag::Literal;
            };
            let alt = attribute(&attributes, "alt").unwrap_or_default().to_owned();

            Tag::Image { src, alt }
        }
        _ => Tag::Literal,
    }
}

/// The attributes of a tag, as name/value pairs with lower-case names.
///
/// Unknown attributes are read and ignored: a span carries no attributes, so a
/// `class` or a `target` has nowhere to go. A malformed attribute ends the
/// scan rather than being guessed at.
fn attributes(input: &[char]) -> Vec<(String, String)> {
    let mut attributes = Vec::new();
    let mut index = 0;

    while index < input.len() {
        while index < input.len() && (input[index].is_whitespace() || input[index] == '/') {
            index += 1;
        }

        let name_start = index;
        while index < input.len()
            && (input[index].is_ascii_alphanumeric()
                || input[index] == '-'
                || input[index] == '_'
                || input[index] == ':')
        {
            index += 1;
        }

        if name_start == index {
            // Not a name: skip the character and look for the next one.
            index += 1;
            continue;
        }

        let name: String = input[name_start..index]
            .iter()
            .map(|character| character.to_ascii_lowercase())
            .collect();

        while index < input.len() && input[index].is_whitespace() {
            index += 1;
        }

        if input.get(index) != Some(&'=') {
            attributes.push((name, String::new()));
            continue;
        }

        index += 1;
        while index < input.len() && input[index].is_whitespace() {
            index += 1;
        }

        let Some(quote) = input.get(index).copied() else {
            attributes.push((name, String::new()));
            break;
        };

        let value = if quote == '"' || quote == '\'' {
            index += 1;
            let start = index;
            while index < input.len() && input[index] != quote {
                index += 1;
            }
            let value: String = input[start..index].iter().collect();
            index += 1;

            value
        } else {
            let start = index;
            while index < input.len() && !input[index].is_whitespace() {
                index += 1;
            }

            input[start..index].iter().collect()
        };

        attributes.push((name, value));
    }

    attributes
}

/// The value of the attribute called `name`, if the tag carries one.
fn attribute<'a>(attributes: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, value)| value.as_str())
}

/// A link target, if it may become a [`Span::Link`].
///
/// The accepted set is a closed list of prefixes, so nothing that is not
/// HTTP(S) or mail cannot pass. Whitespace and control characters are refused
/// outright rather than stripped: `java\nscript:` must not become `javascript:`
/// by normalisation, and a target this daemon cannot spell out exactly is one it
/// does not follow.
pub fn accepted_link(href: &str) -> Option<String> {
    let trimmed = href.trim();

    if !is_simple(trimmed) {
        return None;
    }

    let lowered = trimmed.to_ascii_lowercase();

    LINK_SCHEMES
        .iter()
        .any(|scheme| lowered.starts_with(scheme))
        .then(|| trimmed.to_owned())
}

/// An image source, if it may become a [`Span::Image`].
///
/// An HTTP(S) URL is accepted, and so is a path — `image-path` hints are paths,
/// and clients write body images the same way. Anything else that names a scheme
/// (`data:`, `file:`, `javascript:`) is refused.
pub fn accepted_image(src: &str) -> Option<String> {
    let trimmed = src.trim();

    if !is_simple(trimmed) {
        return None;
    }

    if !names_scheme(trimmed) {
        return Some(trimmed.to_owned());
    }

    let lowered = trimmed.to_ascii_lowercase();

    IMAGE_SCHEMES
        .iter()
        .any(|scheme| lowered.starts_with(scheme))
        .then(|| trimmed.to_owned())
}

/// Whether a target is free of whitespace and control characters.
fn is_simple(candidate: &str) -> bool {
    !candidate.is_empty()
        && !candidate
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

/// Whether a target begins with a URI scheme.
///
/// Per RFC 3986 a scheme is a letter followed by letters, digits, `+`, `-` or
/// `.`, and it is only a scheme when a `:` follows it before any `/` — which is
/// what tells `/usr/share/icons/x.png` (a path) from `file:///usr/…` (a URI).
fn names_scheme(candidate: &str) -> bool {
    let mut characters = candidate.chars();

    match characters.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }

    for character in characters {
        match character {
            ':' => return true,
            '/' => return false,
            character
                if character.is_ascii_alphanumeric()
                    || character == '+'
                    || character == '-'
                    || character == '.' => {}
            _ => return false,
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn text(value: &str) -> Span {
        Span::Text {
            text: value.to_owned(),
            emphasis: Emphasis::PLAIN,
        }
    }

    fn bold(value: &str) -> Span {
        Span::Text {
            text: value.to_owned(),
            emphasis: Emphasis {
                bold: true,
                ..Emphasis::PLAIN
            },
        }
    }

    fn link(value: &str, href: &str) -> Span {
        Span::Link {
            text: value.to_owned(),
            href: href.to_owned(),
            emphasis: Emphasis::PLAIN,
        }
    }

    /// Every target the sanitised body carries, from links and images alike.
    fn targets(spans: &[Span]) -> Vec<&str> {
        spans
            .iter()
            .filter_map(|span| match span {
                Span::Link { href, .. } => Some(href.as_str()),
                Span::Image { src, .. } => Some(src.as_str()),
                Span::Text { .. } => None,
            })
            .collect()
    }

    /// Whether a target could name something executable on any platform.
    fn is_executable(target: &str) -> bool {
        let lowered = target.to_ascii_lowercase();

        [
            "javascript:",
            "data:",
            "file:",
            "vbscript:",
            "blob:",
            "jar:",
        ]
        .iter()
        .any(|scheme| lowered.starts_with(scheme))
    }

    #[test]
    fn plain_text_is_one_span_of_its_own_characters() {
        let spans = sanitize("A message arrived");

        assert_eq!(spans, [text("A message arrived")]);
        assert_eq!(plain_text(&spans), "A message arrived");
    }

    #[test]
    fn an_empty_body_produces_no_spans() {
        assert!(sanitize("").is_empty());
    }

    #[test]
    fn stray_markup_characters_are_kept_as_text() {
        for body in [
            "a < b",
            "5 < 6 > 3",
            "fish & chips",
            "she said \"hi\" and 'bye'",
            "a lone < at the end",
            "a > bang",
            "<3",
        ] {
            let spans = sanitize(body);

            assert_eq!(
                plain_text(&spans),
                body,
                "`{body}` must survive sanitising unchanged"
            );
            assert!(
                !spans.is_empty() || body.is_empty(),
                "`{body}` must still be shown"
            );
        }
    }

    #[test]
    fn the_three_styling_elements_are_applied() {
        assert_eq!(sanitize("<b>bold</b>"), [bold("bold")]);
        assert_eq!(
            sanitize("<i>italic</i>"),
            [Span::Text {
                text: "italic".to_owned(),
                emphasis: Emphasis {
                    italic: true,
                    ..Emphasis::PLAIN
                },
            }]
        );
        assert_eq!(
            sanitize("<u>underlined</u>"),
            [Span::Text {
                text: "underlined".to_owned(),
                emphasis: Emphasis {
                    underline: true,
                    ..Emphasis::PLAIN
                },
            }]
        );
    }

    #[test]
    fn nested_styling_is_the_union_of_the_open_elements() {
        let spans = sanitize("<b>a<i>b</i>c</b>d");

        assert_eq!(
            spans,
            [
                bold("a"),
                Span::Text {
                    text: "b".to_owned(),
                    emphasis: Emphasis {
                        bold: true,
                        italic: true,
                        underline: false,
                    },
                },
                bold("c"),
                text("d"),
            ]
        );
    }

    #[test]
    fn an_unknown_tag_is_shown_as_text() {
        let spans = sanitize("Hello <script>alert(1)</script>");

        assert_eq!(plain_text(&spans), "Hello <script>alert(1)</script>");
        assert!(spans.iter().all(|span| matches!(span, Span::Text { .. })));
    }

    #[test]
    fn an_unterminated_tag_is_shown_as_text() {
        let spans = sanitize("<b");

        assert_eq!(plain_text(&spans), "<b");
        assert_eq!(spans, [text("<b")]);
    }

    #[test]
    fn a_tag_longer_than_the_limit_is_shown_as_text() {
        let body = format!("<{}>", "b".repeat(MAX_TAG_CHARS + 1));
        let spans = sanitize(&body);

        assert_eq!(plain_text(&spans), body);
    }

    #[test]
    fn a_closing_tag_that_matches_nothing_is_shown_as_text() {
        assert_eq!(sanitize("</b>"), [text("</b>")]);
        assert_eq!(
            sanitize("<i>a</b></i>"),
            [Span::Text {
                text: "a</b>".to_owned(),
                emphasis: Emphasis {
                    italic: true,
                    ..Emphasis::PLAIN
                },
            },]
        );
    }

    #[test]
    fn an_element_left_open_still_shows_its_text() {
        assert_eq!(sanitize("<b>bold"), [bold("bold")]);
        assert_eq!(
            sanitize("<b>bo<i>ld"),
            [
                bold("bo"),
                Span::Text {
                    text: "ld".to_owned(),
                    emphasis: Emphasis {
                        bold: true,
                        italic: true,
                        ..Emphasis::PLAIN
                    },
                },
            ]
        );
    }

    #[test]
    fn a_link_is_accepted_for_the_three_schemes_it_may_carry() {
        for href in [
            "http://example.com",
            "https://example.com/a?b=c#d",
            "mailto:someone@example.com",
            "HTTPS://EXAMPLE.COM",
        ] {
            let spans = sanitize(&format!("<a href=\"{href}\">click</a>"));

            assert_eq!(
                spans,
                [Span::Link {
                    text: "click".to_owned(),
                    href: href.to_owned(),
                    emphasis: Emphasis::PLAIN,
                }],
                "`{href}` is an accepted link"
            );
        }
    }

    #[test]
    fn an_executable_link_scheme_is_shown_as_text() {
        for href in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "data:text/html;base64,PHNjcmlwdD4=",
            "file:///etc/passwd",
            "vbscript:msgbox(1)",
            "jar:http://example.com/x.jar!/",
        ] {
            let body = format!("<a href=\"{href}\">click</a>");
            let spans = sanitize(&body);

            assert_eq!(
                plain_text(&spans),
                body,
                "`{href}` must be shown, not followed"
            );
            assert!(
                targets(&spans).is_empty(),
                "`{href}` must not reach a link span"
            );
        }
    }

    #[test]
    fn a_link_target_the_daemon_cannot_spell_out_exactly_is_refused() {
        for href in [
            " javascript:alert(1)",
            "java\tscript:alert(1)",
            "java\nscript:alert(1)",
            "http://example.com/a b",
            "",
            "//example.com",
            "example.com",
        ] {
            let spans = sanitize(&format!("<a href=\"{href}\">click</a>"));

            assert!(
                targets(&spans).is_empty(),
                "`{href:?}` must not become a link"
            );
        }
    }

    #[test]
    fn a_link_without_a_target_is_shown_as_text() {
        let body = "<a>click</a>";

        assert_eq!(plain_text(&sanitize(body)), body);
        assert!(targets(&sanitize(body)).is_empty());
    }

    #[test]
    fn a_link_inside_a_link_is_refused() {
        let spans = sanitize(
            "<a href=\"https://a.example\">outer <a href=\"https://b.example\">inner</a></a>",
        );

        assert_eq!(
            targets(&spans),
            ["https://a.example"],
            "only the outer target survives"
        );
        assert_eq!(
            plain_text(&spans),
            "outer <a href=\"https://b.example\">inner</a>"
        );
    }

    #[test]
    fn styling_inside_a_link_stays_inside_the_link() {
        let spans = sanitize("<a href=\"https://example.com\"><b>bold</b></a>");

        assert_eq!(
            spans,
            [Span::Link {
                text: "bold".to_owned(),
                href: "https://example.com".to_owned(),
                emphasis: Emphasis {
                    bold: true,
                    ..Emphasis::PLAIN
                },
            }]
        );
    }

    #[test]
    fn an_image_is_accepted_from_a_url_or_a_path() {
        for src in [
            "https://example.com/a.png",
            "/usr/share/icons/hicolor/48x48/apps/terminal.png",
            "icons/a.png",
        ] {
            let spans = sanitize(&format!("<img src=\"{src}\" alt=\"a picture\">"));

            assert_eq!(
                spans,
                [Span::Image {
                    src: src.to_owned(),
                    alt: "a picture".to_owned(),
                    emphasis: Emphasis::PLAIN,
                }],
                "`{src}` is an accepted image source"
            );
        }
    }

    #[test]
    fn an_image_source_that_names_a_refused_scheme_is_shown_as_text() {
        for src in [
            "data:image/png;base64,AAAA",
            "file:///etc/shadow",
            "javascript:alert(1)",
        ] {
            let body = format!("<img src=\"{src}\" alt=\"x\">");
            let spans = sanitize(&body);

            assert_eq!(plain_text(&spans), body);
            assert!(targets(&spans).is_empty(), "`{src}` must not reach a span");
        }
    }

    #[test]
    fn an_image_without_a_source_is_shown_as_text() {
        let body = "<img alt=\"x\">";

        assert_eq!(plain_text(&sanitize(body)), body);
        assert!(
            sanitize(body)
                .iter()
                .all(|span| matches!(span, Span::Text { .. }))
        );
    }

    #[test]
    fn an_image_without_alt_text_carries_an_empty_one() {
        let spans = sanitize("<img src=\"/a.png\">");

        assert_eq!(
            spans,
            [Span::Image {
                src: "/a.png".to_owned(),
                alt: String::new(),
                emphasis: Emphasis::PLAIN,
            }]
        );
    }

    #[test]
    fn an_image_inside_a_link_does_not_swallow_the_link() {
        let spans = sanitize(
            "<a href=\"https://example.com\">a <img src=\"https://example.com/b.png\"> b</a>",
        );

        assert_eq!(
            spans,
            [
                Span::Link {
                    text: "a ".to_owned(),
                    href: "https://example.com".to_owned(),
                    emphasis: Emphasis::PLAIN,
                },
                Span::Image {
                    src: "https://example.com/b.png".to_owned(),
                    alt: String::new(),
                    emphasis: Emphasis::PLAIN,
                },
                Span::Link {
                    text: " b".to_owned(),
                    href: "https://example.com".to_owned(),
                    emphasis: Emphasis::PLAIN,
                },
            ]
        );
    }

    #[test]
    fn single_quoted_and_unquoted_attributes_are_read() {
        assert_eq!(
            sanitize("<a href='https://example.com'>x</a>"),
            [link("x", "https://example.com")]
        );
        assert_eq!(
            sanitize("<a href=https://example.com>x</a>"),
            [link("x", "https://example.com")]
        );
    }

    #[test]
    fn an_attribute_name_is_matched_without_regard_to_case() {
        assert_eq!(
            sanitize("<A HREF=\"https://example.com\">x</a>"),
            [link("x", "https://example.com")]
        );
    }

    #[test]
    fn unknown_attributes_are_ignored() {
        assert_eq!(
            sanitize("<a href=\"https://example.com\" target=\"_blank\" class=\"x\">x</a>"),
            [link("x", "https://example.com")]
        );
    }

    #[test]
    fn no_output_span_is_empty() {
        for body in [
            "<b></b>",
            "<a href=\"https://example.com\"></a>",
            "<b> </b>",
        ] {
            for span in sanitize(body) {
                assert!(
                    !span.text().is_empty(),
                    "`{body}` produced an empty span: {span:?}"
                );
            }
        }
    }

    #[test]
    fn a_body_longer_than_the_limit_is_cut_off_at_the_limit() {
        let body = "x".repeat(MAX_TEXT_CHARS * 2);
        let spans = sanitize(&body);

        assert_eq!(plain_text(&spans).chars().count(), MAX_TEXT_CHARS);
    }

    #[test]
    fn a_body_with_more_spans_than_the_limit_produces_no_more() {
        let body = "<i>x</i>".repeat(MAX_SPANS * 2);
        let spans = sanitize(&body);

        assert!(spans.len() <= MAX_SPANS, "{} spans", spans.len());
        assert!(spans.len() > 1, "the limit must not stop the first spans");
    }

    #[test]
    fn a_multibyte_body_is_not_cut_through_a_character() {
        let body = "中".repeat(MAX_TEXT_CHARS * 2);
        let spans = sanitize(&body);

        assert_eq!(
            plain_text(&spans).chars().count(),
            MAX_TEXT_CHARS,
            "a character is a character, not a byte"
        );
    }

    #[test]
    fn plain_text_flattens_every_kind_of_span() {
        let spans = sanitize(
            "a <b>b</b><a href=\"https://example.com\">c</a><img src=\"/d.png\" alt=\"d\">",
        );

        assert_eq!(plain_text(&spans), "a bcd");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn plain_text_round_trips_exactly(body in "[a-zA-Z0-9 é中> &\"'\\n\\t]{0,200}") {
            let spans = sanitize(&body);

            prop_assert_eq!(plain_text(&spans), body);
        }

        #[test]
        fn a_stray_less_than_never_becomes_markup(
            fragments in proptest::collection::vec("[a-zA-Z0-9 ]{0,20}", 0..20),
            separators in proptest::collection::vec("[ &\"'>\t]", 0..20),
        ) {
            // Every `<` is followed by something that cannot start a tag name,
            // so the whole body is text and nothing may be styled or targeted.
            let mut body = String::new();
            for (index, fragment) in fragments.iter().enumerate() {
                body.push_str(fragment);
                body.push('<');
                if let Some(separator) = separators.get(index) {
                    body.push_str(separator);
                }
            }

            let spans = sanitize(&body);

            prop_assert_eq!(plain_text(&spans), body.clone());
            prop_assert!(spans.iter().all(|span| span.emphasis().is_plain()));
            prop_assert!(targets(&spans).is_empty());
        }

        #[test]
        fn no_sanitised_body_carries_an_executable_target(body in "\\PC{0,300}") {
            let spans = sanitize(&body);

            for target in targets(&spans) {
                prop_assert!(
                    !is_executable(target),
                    "`{}` reached a span as `{}`", body, target
                );
            }
        }

        #[test]
        fn sanitising_never_grows_the_text(body in "\\PC{0,300}") {
            let spans = sanitize(&body);

            prop_assert!(
                plain_text(&spans).chars().count() <= body.chars().count()
            );
        }

        #[test]
        fn sanitising_is_deterministic(body in "\\PC{0,300}") {
            prop_assert_eq!(sanitize(&body), sanitize(&body));
        }

        #[test]
        fn the_output_stays_within_its_bounds(body in "\\PC{0,4000}") {
            let spans = sanitize(&body);

            prop_assert!(spans.len() <= MAX_SPANS);
            prop_assert!(plain_text(&spans).chars().count() <= MAX_TEXT_CHARS);
        }
    }
}
