//! The projection from state to what the view shows.
//!
//! Everything the user reads is decided here, and only here: labels come from
//! the Fluent catalogs, disabled rows carry the reason they cannot be launched,
//! and the banner says which of the states the acceptance criteria name this is
//! (loading, empty, no matches, index unavailable, acceptance D05). The view
//! then has no decisions left to make — it walks the rows and draws them — which
//! is what keeps a colour or a message out of the widget tree.
//!
//! The module is pure: it reads the index, the service state, the newest search
//! result and the launch in progress, and returns a value. No I/O, no Qt, no
//! clock, so every state above is a unit test.

use inox_common::i18n::{Arg, Localizer};
use inox_config::LauncherConfig;

use crate::activation::{Activation, LaunchOutcome};
use crate::contract::{Failure, FailureKind, ServiceState};
use crate::index::AppIndex;
use crate::search::SearchResult;

/// The character a truncated label ends with.
pub const ELLIPSIS: char = '…';

/// One line of the result list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The entry's desktop file id, which the view sends back when the row is
    /// chosen.
    pub id: String,
    /// What the icon column shows: the entry's first character, or the
    /// placeholder when the entry names no icon at all.
    pub icon: String,
    /// Whether the icon column shows the placeholder because the entry names no
    /// icon at all, as opposed to standing in for one it does name.
    pub icon_placeholder: bool,
    /// The application's name, truncated to the space available.
    pub label: String,
    /// Whether the name had to be truncated.
    pub truncated: bool,
    /// The application's generic name, when it has one.
    pub detail: Option<String>,
    /// Whether this row is the selection.
    pub selected: bool,
    /// Whether the row can be launched.
    pub enabled: bool,
    /// Why it cannot be, when it cannot.
    pub disabled_reason: Option<String>,
}

/// The message that takes the result list's place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Banner {
    /// Which of the states this banner is, so the view can draw them apart: an
    /// index that is still being read, an index with nothing in it, a query
    /// that matched nothing, and unreadable index are four different screens,
    /// not one screen with four sentences.
    pub kind: BannerKind,
    /// The headline.
    pub title: String,
    /// The explanation, when there is one.
    pub body: Option<String>,
    /// A suggestion, when there is one.
    pub hint: Option<String>,
    /// The label of the banner's one action, when the state can be acted on.
    pub action: Option<String>,
}

/// Which empty, in-progress or failed state a [`Banner`] draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerKind {
    /// The index is being read; nothing can be listed yet.
    Loading,
    /// The index was read and holds no usable entry.
    Empty,
    /// The index has entries, and the query matched none of them.
    NoMatch,
    /// The index could not be read at all.
    Unavailable,
}

impl BannerKind {
    /// The message the banner's action sends, or `None` for a state with no
    /// action to offer.
    ///
    /// A banner only ever offers an action it can carry out: an unreadable or
    /// empty index is re-read, and a query that matched nothing is cleared. The
    /// mapping lives next to the kind so a new kind has to answer the question.
    pub fn action_message(&self) -> Option<crate::app::Message> {
        match self {
            Self::Unavailable | Self::Empty => Some(crate::app::Message::IndexRetried),
            Self::NoMatch => Some(crate::app::Message::QueryCleared),
            Self::Loading => None,
        }
    }
}

/// The line under the list: what the launcher is doing, and why not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// The sentence, for example "Launching Firefox…".
    pub line: String,
    /// The cause, when the line reports a failure.
    pub reason: Option<String>,
    /// The label of the retry action, when the failure can be retried.
    pub retry: Option<String>,
}

/// Everything the view draws, already localized.
///
/// The focus order is the order of [`ViewModel::rows`]: the search field holds
/// the keyboard focus — and therefore the launcher's one focus ring — while the
/// launcher is open, Up and Down move the selection through the rows, and the
/// one action a banner offers is reachable with the pointer. That is acceptance
/// Q01's "unique, visible focus, no keyboard trap" as far as this component can
/// state it; the semantic tree a screen reader needs is not exported yet (see
/// the crate README).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewModel {
    /// The window title.
    pub title: String,
    /// The search field's placeholder.
    pub placeholder: String,
    /// The text in the search field.
    pub query: String,
    /// What the list says while the query is empty.
    ///
    /// This is the launcher's *resting* state, and it is not a failure: the
    /// index is readable and the rows below are the whole list. A query that
    /// matches nothing is a different screen — see [`ViewModel::banner`] — and
    /// the two must never be worded the same way.
    pub prompt: Option<String>,
    /// The key hints under the list.
    pub hint: String,
    /// The result rows.
    pub rows: Vec<Row>,
    /// What stands in for the rows, when nothing can be listed.
    pub banner: Option<Banner>,
    /// A note about the list itself (a truncated query, skipped entries).
    pub notice: Option<String>,
    /// The launch in progress or its outcome.
    pub status: Option<Status>,
}

impl ViewModel {
    /// The selected row.
    pub fn selected(&self) -> Option<&Row> {
        self.rows.iter().find(|row| row.selected)
    }

    /// The position of the selected row.
    pub fn selected_index(&self) -> Option<usize> {
        self.rows.iter().position(|row| row.selected)
    }

    /// Whether the view should offer to read the index again.
    ///
    /// True for the two states a re-read can fix: an index that could not be
    /// read, and one that came back empty. A query that matched nothing is not
    /// one of them — it is cleared, not re-read.
    pub fn can_retry_index(&self) -> bool {
        self.banner.as_ref().is_some_and(|banner| {
            matches!(banner.kind, BannerKind::Unavailable | BannerKind::Empty)
                && banner.action.is_some()
        })
    }

    /// Whether the view should offer to launch again.
    pub fn can_retry_launch(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|status| status.retry.is_some())
    }
}

/// The state the projection reads.
///
/// Borrowed rather than cloned: the projection runs on every frame, and cloning
/// the index to draw a list would be the one thing that makes it slow.
#[derive(Debug)]
pub struct Projection<'a> {
    /// The catalogs.
    pub localizer: &'a Localizer,
    /// `shell.toml`'s `[launcher]`.
    pub config: &'a LauncherConfig,
    /// The index, when there is one.
    pub index: Option<&'a AppIndex>,
    /// How the index itself is doing.
    pub service: &'a ServiceState,
    /// The newest applied search.
    pub result: Option<&'a SearchResult>,
    /// The text in the search field.
    pub query: &'a str,
    /// Which row is selected.
    pub selected: usize,
    /// The launch in progress or its outcome.
    pub activation: Option<&'a Activation>,
    /// How many characters fit in a row's label.
    pub label_budget: usize,
}

/// Builds the view model.
pub fn project(projection: &Projection<'_>) -> ViewModel {
    let localizer = projection.localizer;

    let rows = projection
        .result
        .map(|result| {
            result
                .hits
                .iter()
                .enumerate()
                .map(|(position, hit)| {
                    row(
                        projection,
                        &hit.id,
                        &hit.name,
                        hit.generic_name.as_deref(),
                        position == projection.selected,
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // The list's own line only belongs on the screen while the list is: a
    // banner takes the list's place, so it takes this line's place too.
    let banner = banner(projection);
    let prompt = if banner.is_none() {
        prompt(projection, &rows)
    } else {
        None
    };

    ViewModel {
        title: localizer.message("launcher-window-title"),
        placeholder: localizer.message("launcher-search-placeholder"),
        query: projection.query.to_owned(),
        prompt,
        hint: localizer.message("launcher-search-hint"),
        rows,
        banner,
        notice: notice(projection),
        status: projection
            .activation
            .map(|activation| status(activation, localizer)),
    }
}

/// What the list says while the query is empty.
///
/// The launcher opens showing every application it indexed and a line telling
/// the user that typing narrows it. That is the screen an empty query is,
/// and it is deliberately not the same screen as a query that matched nothing
/// (a [`BannerKind::NoMatch`] banner): one lists applications, the other says
/// there are none and offers to clear the query.
fn prompt(projection: &Projection<'_>, rows: &[Row]) -> Option<String> {
    if !projection.query.trim().is_empty() || rows.is_empty() {
        return None;
    }

    Some(projection.localizer.message("launcher-list-prompt"))
}

/// The message that stands in for the rows.
fn banner(projection: &Projection<'_>) -> Option<Banner> {
    let localizer = projection.localizer;

    match projection.service {
        ServiceState::Loading { .. } => Some(Banner {
            kind: BannerKind::Loading,
            title: localizer.message("launcher-state-loading"),
            body: None,
            hint: None,
            action: None,
        }),
        ServiceState::Unavailable { failure, .. } => Some(Banner {
            kind: BannerKind::Unavailable,
            title: localizer.message("launcher-state-unavailable-title"),
            body: Some(failure_message(failure, localizer)),
            hint: None,
            action: Some(localizer.message("launcher-state-action-retry")),
        }),
        ServiceState::Ready {
            entries,
            directories,
            ..
        } => {
            if *entries == 0 {
                return Some(Banner {
                    kind: BannerKind::Empty,
                    title: localizer.message("launcher-state-empty-title"),
                    body: Some(localizer.message_with(
                        "launcher-state-empty-body",
                        &[("dirs", Arg::from(join_directories(directories)))],
                    )),
                    hint: Some(localizer.message("launcher-state-empty-hint")),
                    // An empty index is often a stale environment rather than a
                    // permanent fact, so it can be read again after the user has
                    // fixed what the hint points at.
                    action: Some(localizer.message("launcher-state-action-retry")),
                });
            }

            // The index has applications but the query matched none of them.
            match projection.result {
                Some(result) if result.hits.is_empty() && !result.query.trim().is_empty() => {
                    Some(Banner {
                        kind: BannerKind::NoMatch,
                        title: localizer.message("launcher-state-nomatch-title"),
                        body: Some(localizer.message_with(
                            "launcher-state-nomatch-body",
                            &[("query", Arg::from(result.query.as_str()))],
                        )),
                        hint: None,
                        action: Some(localizer.message("launcher-state-action-clear")),
                    })
                }
                _ => None,
            }
        }
    }
}

/// The note about the list: a truncated query, or skipped entries.
fn notice(projection: &Projection<'_>) -> Option<String> {
    let localizer = projection.localizer;

    if projection.result.is_some_and(|result| result.truncated) {
        return Some(localizer.message_with(
            "launcher-state-truncated",
            &[(
                "limit",
                Arg::from(u32::from(projection.config.max_query_chars)),
            )],
        ));
    }

    match projection.service {
        ServiceState::Ready { skipped, .. } if *skipped > 0 => {
            Some(localizer.message_with("launcher-skipped-note", &[("count", Arg::from(*skipped))]))
        }
        _ => None,
    }
}

/// The launch status line.
fn status(activation: &Activation, localizer: &Localizer) -> Status {
    let name = Arg::from(activation.name.as_str());
    match &activation.outcome {
        LaunchOutcome::Requested => Status {
            line: localizer.message_with("launcher-launch-requested", &[("name", name)]),
            reason: None,
            retry: None,
        },
        LaunchOutcome::Started { .. } => Status {
            line: localizer.message_with("launcher-launch-started", &[("name", name)]),
            reason: None,
            retry: None,
        },
        LaunchOutcome::Failed { failure } => Status {
            line: localizer.message_with("launcher-launch-failed", &[("name", name)]),
            reason: Some(failure_message(failure, localizer)),
            // A launch can be tried again; a launch that already succeeded
            // cannot, and a row that is unusable is fixed by editing it.
            retry: Some(localizer.message("launcher-state-action-retry")),
        },
    }
}

/// One row.
fn row(
    projection: &Projection<'_>,
    id: &str,
    name: &str,
    generic_name: Option<&str>,
    selected: bool,
) -> Row {
    let (enabled, disabled_reason) = launchability(projection, id, name);
    let (label, truncated) = truncate(name, projection.label_budget);

    Row {
        id: id.to_owned(),
        icon: icon_glyph(projection, id),
        icon_placeholder: projection
            .index
            .and_then(|index| index.get(id))
            .is_none_or(|entry| entry.icon.is_none()),
        label,
        truncated,
        detail: generic_name.map(str::to_owned),
        selected,
        enabled,
        disabled_reason,
    }
}

/// What the icon column shows.
///
/// The icon theme is not loaded yet (see the crate README), so an entry that
/// names an icon is drawn with its first character and an entry that names none
/// is drawn with the catalog's placeholder. Both are degradations, and they are
/// kept distinguishable so that a missing icon is visible rather than silent.
fn icon_glyph(projection: &Projection<'_>, id: &str) -> String {
    match projection.index.and_then(|index| index.get(id)) {
        Some(entry) if entry.icon.is_some() => entry
            .name
            .chars()
            .next()
            .map(|character: char| character.to_uppercase().collect::<String>())
            .unwrap_or_else(|| {
                projection
                    .localizer
                    .message("launcher-row-icon-placeholder")
            }),
        _ => projection
            .localizer
            .message("launcher-row-icon-placeholder"),
    }
}

/// Whether an entry can be launched, and why not when it cannot.
fn launchability(projection: &Projection<'_>, id: &str, name: &str) -> (bool, Option<String>) {
    let Some(entry) = projection.index.and_then(|index| index.get(id)) else {
        return (
            false,
            Some(
                projection
                    .localizer
                    .message("launcher-state-unavailable-title"),
            ),
        );
    };

    match entry.argv(&projection.config.terminal) {
        Ok(_) => (true, None),
        Err(kind) => {
            let reason = failure_message(&Failure::for_entry(kind, name), projection.localizer);
            (
                false,
                Some(
                    projection
                        .localizer
                        .message_with("launcher-row-disabled", &[("reason", Arg::from(reason))]),
                ),
            )
        }
    }
}

/// The localized reason for a failure.
///
/// Shared with the status line so that a row's "cannot launch" reason and a
/// launch's failure are worded the same way.
pub fn failure_message(failure: &Failure, localizer: &Localizer) -> String {
    localizer.message_with(message_id(&failure.kind), &arguments(&failure.kind))
}

/// The message id a failure is rendered with.
fn message_id(kind: &FailureKind) -> &'static str {
    match kind {
        FailureKind::IndexUnavailable { .. } => "launcher-failure-index-unavailable",
        FailureKind::NoCommand => "launcher-failure-no-command",
        FailureKind::UnknownFieldCode { .. } => "launcher-failure-unknown-field-code",
        FailureKind::MisplacedFieldCode { .. } => "launcher-failure-misplaced-field-code",
        FailureKind::UnterminatedQuote => "launcher-failure-unterminated-quote",
        FailureKind::TrailingEscape => "launcher-failure-trailing-escape",
        FailureKind::TrailingPercent => "launcher-failure-trailing-percent",
        FailureKind::MissingTerminalPrefix => "launcher-failure-missing-terminal",
        FailureKind::SpawnFailed { .. } => "launcher-failure-spawn-failed",
        FailureKind::ExitedBeforeReady { .. } => "launcher-failure-exited-before-ready",
        FailureKind::NotReadyInTime { .. } => "launcher-failure-not-ready-in-time",
    }
}

/// The placeholders a failure's message needs.
fn arguments(kind: &FailureKind) -> Vec<(&'static str, Arg)> {
    match kind {
        FailureKind::IndexUnavailable { detail } => {
            vec![("detail", Arg::from(detail.as_str()))]
        }
        FailureKind::UnknownFieldCode { code } | FailureKind::MisplacedFieldCode { code } => {
            vec![("code", Arg::from(format!("%{code}")))]
        }
        FailureKind::SpawnFailed { detail } | FailureKind::ExitedBeforeReady { detail } => {
            vec![("detail", Arg::from(detail.as_str()))]
        }
        FailureKind::NotReadyInTime { seconds } => {
            vec![("seconds", Arg::from(*seconds as i64))]
        }
        FailureKind::NoCommand
        | FailureKind::UnterminatedQuote
        | FailureKind::TrailingEscape
        | FailureKind::TrailingPercent
        | FailureKind::MissingTerminalPrefix => Vec::new(),
    }
}

/// Cuts `text` to `budget` characters, ending it with an ellipsis.
///
/// Character-wise rather than byte-wise, so a name in Chinese or Greek is cut
/// where the user sees it and never in the middle of a byte; the ellipsis is
/// added only when something was actually removed, and it counts towards the
/// budget so that a truncated label is never wider than an untruncated one.
pub fn truncate(text: &str, budget: usize) -> (String, bool) {
    if budget == 0 {
        return (String::new(), !text.is_empty());
    }
    let mut characters = text.chars();
    let mut kept: String = characters.by_ref().take(budget).collect();
    if characters.next().is_none() {
        return (kept, false);
    }
    kept.pop();
    kept.push(ELLIPSIS);
    (kept, true)
}

/// The directories an empty index looked in, as one string.
fn join_directories(directories: &[std::path::PathBuf]) -> String {
    directories
        .iter()
        .map(|directory| directory.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{AppIndex, IndexedEntry};
    use crate::search::{self, SearchEngine};
    use inox_common::i18n::Locale;

    /// Everything the projection reads, owned so that a test can build it up.
    struct Fixture {
        localizer: Localizer,
        config: LauncherConfig,
        index: AppIndex,
        service: ServiceState,
        engine: SearchEngine,
        query: String,
        selected: usize,
        activation: Option<Activation>,
        label_budget: usize,
    }

    impl Fixture {
        fn new(entries: Vec<IndexedEntry>) -> Self {
            let index = AppIndex::from_entries(entries, 1);
            let service = index.state();
            Self {
                localizer: crate::i18n::localizer(&Locale::parse("en-US").expect("a tag"))
                    .expect("the catalogs load"),
                config: LauncherConfig::default(),
                index,
                service,
                engine: SearchEngine::new(),
                query: String::new(),
                selected: 0,
                activation: None,
                label_budget: 40,
            }
        }

        fn search(&mut self, query: &str) {
            self.query = query.to_owned();
            let request = self
                .engine
                .request(query, 128, usize::from(self.config.max_results));
            let result = search::execute(&self.index, &request);
            self.engine.apply(result);
        }

        fn project(&self) -> ViewModel {
            project(&Projection {
                localizer: &self.localizer,
                config: &self.config,
                index: Some(&self.index),
                service: &self.service,
                result: self.engine.current(),
                query: &self.query,
                selected: self.selected,
                activation: self.activation.as_ref(),
                label_budget: self.label_budget,
            })
        }
    }

    fn fixture() -> Fixture {
        Fixture::new(vec![
            AppIndex::test_entry("firefox.desktop", "Firefox", "firefox"),
            AppIndex::test_entry("editor.desktop", "Text Editor", "editor"),
        ])
    }

    #[test]
    fn every_string_the_user_reads_comes_from_the_catalogs() {
        let mut fixture = fixture();
        fixture.search("fire");
        let model = fixture.project();

        let expected_title = fixture.localizer.message("launcher-window-title");
        assert_eq!(model.title, expected_title);
        assert_eq!(
            model.placeholder,
            fixture.localizer.message("launcher-search-placeholder")
        );
        assert_eq!(
            model.hint,
            fixture.localizer.message("launcher-search-hint")
        );
        assert_ne!(model.title, "");
    }

    #[test]
    fn the_rows_follow_the_search_result_and_mark_the_selection() {
        let mut fixture = fixture();
        fixture.search("fi");
        fixture.selected = 0;
        let model = fixture.project();

        assert_eq!(model.rows.len(), 1);
        assert_eq!(model.rows[0].id, "firefox.desktop");
        assert_eq!(model.rows[0].label, "Firefox");
        assert!(model.rows[0].selected);
        assert_eq!(model.selected_index(), Some(0));
        assert_eq!(
            model.selected().map(|row| row.id.as_str()),
            Some("firefox.desktop")
        );
    }

    #[test]
    fn an_entry_with_no_icon_gets_the_placeholder_and_is_marked_degraded() {
        let mut entry = AppIndex::test_entry("plain.desktop", "Plain", "plain");
        entry.icon = Some("plain-icon".to_owned());
        let mut fixture = Fixture::new(vec![entry]);
        fixture.search("plain");
        let model = fixture.project();

        assert_eq!(model.rows[0].icon, "P");
        assert!(
            !model.rows[0].icon_placeholder,
            "the entry names an icon, so the letter stands in for it",
        );

        let mut bare = Fixture::new(vec![AppIndex::test_entry("bare.desktop", "Bare", "bare")]);
        bare.search("bare");
        let model = bare.project();
        assert_eq!(
            model.rows[0].icon,
            bare.localizer.message("launcher-row-icon-placeholder")
        );
        assert!(model.rows[0].icon_placeholder);
    }

    #[test]
    fn an_entry_that_needs_a_terminal_that_is_not_configured_is_disabled_with_a_reason() {
        let mut entry = AppIndex::test_entry("htop.desktop", "htop", "htop");
        entry.terminal = true;
        let mut fixture = Fixture::new(vec![entry]);
        fixture.search("htop");
        let model = fixture.project();

        let row = &model.rows[0];
        assert!(!row.enabled);
        let reason = row.disabled_reason.as_deref().expect("a readable reason");
        assert!(
            reason.contains(
                &fixture
                    .localizer
                    .message("launcher-failure-missing-terminal")
            ),
            "{reason}"
        );
    }

    #[test]
    fn configuring_a_terminal_makes_the_row_launchable() {
        let mut entry = AppIndex::test_entry("htop.desktop", "htop", "htop");
        entry.terminal = true;
        let mut fixture = Fixture::new(vec![entry]);
        fixture.config.terminal = vec!["alacritty".to_owned(), "-e".to_owned()];
        fixture.search("htop");

        assert!(fixture.project().rows[0].enabled);
    }

    #[test]
    fn an_entry_with_an_unreadable_command_is_disabled_and_says_which_code() {
        let mut fixture = Fixture::new(vec![AppIndex::test_entry(
            "broken.desktop",
            "Broken",
            "broken %q",
        )]);
        fixture.search("broken");
        let model = fixture.project();

        assert!(!model.rows[0].enabled);
        let reason = model.rows[0].disabled_reason.as_deref().expect("a reason");
        assert!(reason.contains("%q"), "{reason}");
    }

    #[test]
    fn a_long_name_is_truncated_to_the_budget_and_says_so() {
        let mut fixture = Fixture::new(vec![AppIndex::test_entry(
            "long.desktop",
            "An Extremely Long Application Name",
            "long",
        )]);
        fixture.label_budget = 10;
        fixture.search("extremely");
        let model = fixture.project();

        assert_eq!(model.rows[0].label.chars().count(), 10);
        assert!(model.rows[0].truncated);
        assert!(model.rows[0].label.ends_with(ELLIPSIS));
    }

    #[test]
    fn a_name_that_fits_is_not_truncated() {
        let mut fixture = fixture();
        fixture.label_budget = 40;
        fixture.search("fire");
        assert!(!fixture.project().rows[0].truncated);
    }

    #[test]
    fn truncating_counts_characters_not_bytes() {
        assert_eq!(truncate("abc", 3), ("abc".to_owned(), false));
        assert_eq!(truncate("abcd", 3), ("ab…".to_owned(), true));
        assert_eq!(truncate("文件管理器", 3), ("文件…".to_owned(), true));
        assert_eq!(truncate("文件", 4), ("文件".to_owned(), false));
        assert_eq!(truncate("", 0), (String::new(), false));
        assert_eq!(truncate("x", 0), (String::new(), true));
        assert_eq!(truncate("x", 1), ("x".to_owned(), false));
        assert_eq!(truncate("xy", 1), ("…".to_owned(), true));
    }

    #[test]
    fn an_index_that_is_still_being_read_shows_loading() {
        let mut fixture = fixture();
        fixture.service = ServiceState::Loading { generation: 2 };
        let model = fixture.project();

        let banner = model.banner.as_ref().expect("a banner");
        assert_eq!(banner.kind, BannerKind::Loading);
        assert_eq!(
            banner.title,
            fixture.localizer.message("launcher-state-loading")
        );
        assert!(banner.action.is_none());
        assert!(!model.can_retry_index());
    }

    #[test]
    fn an_empty_index_names_the_directories_it_looked_in() {
        let mut fixture = Fixture::new(Vec::new());
        fixture.service = ServiceState::Ready {
            generation: 1,
            entries: 0,
            skipped: 0,
            directories: vec![std::path::PathBuf::from("/usr/share")],
        };
        fixture.search("");
        let model = fixture.project();

        let banner = model.banner.as_ref().expect("a banner");
        assert_eq!(banner.kind, BannerKind::Empty);
        assert_eq!(
            banner.title,
            fixture.localizer.message("launcher-state-empty-title")
        );
        assert!(
            banner
                .body
                .as_deref()
                .unwrap_or_default()
                .contains("/usr/share"),
            "{:?}",
            banner.body
        );
        assert!(banner.hint.is_some());
        assert!(
            banner.action.is_some(),
            "an empty state the user can fix has to offer the re-read",
        );
        assert!(model.can_retry_index());
    }

    #[test]
    fn a_query_that_matches_nothing_says_which_query() {
        let mut fixture = fixture();
        fixture.search("zzzz");
        let model = fixture.project();

        let banner = model.banner.as_ref().expect("a banner");
        assert_eq!(banner.kind, BannerKind::NoMatch);
        assert_eq!(
            banner.title,
            fixture.localizer.message("launcher-state-nomatch-title")
        );
        assert!(
            banner.body.as_deref().unwrap_or_default().contains("zzzz"),
            "{:?}",
            banner.body
        );
        assert_eq!(
            banner.action.as_deref(),
            Some(
                fixture
                    .localizer
                    .message("launcher-state-action-clear")
                    .as_str()
            ),
            "a query that matched nothing is cleared, not re-read",
        );
        assert!(
            !model.can_retry_index(),
            "re-reading an index that just answered would change nothing",
        );
    }

    #[test]
    fn the_resting_state_and_the_no_match_state_are_two_different_screens() {
        let mut fixture = fixture();
        fixture.search("");
        let resting = fixture.project();
        fixture.search("zzzz");
        let no_match = fixture.project();

        let prompt = resting.prompt.as_deref().expect("the resting prompt");
        assert_eq!(prompt, fixture.localizer.message("launcher-list-prompt"));
        assert_eq!(
            resting.rows.len(),
            2,
            "the resting state lists what was indexed",
        );
        assert!(resting.banner.is_none());

        assert!(no_match.rows.is_empty());
        assert!(no_match.prompt.is_none());
        let banner = no_match.banner.as_ref().expect("a banner");
        assert_eq!(banner.kind, BannerKind::NoMatch);
        assert_ne!(
            banner.title, prompt,
            "the one line must not be the other line",
        );
    }

    #[test]
    fn a_query_of_nothing_but_spaces_is_still_the_resting_state() {
        let mut fixture = fixture();
        fixture.search("   ");
        let model = fixture.project();

        assert!(
            model.prompt.is_some(),
            "a query that only holds spaces narrows nothing",
        );
        assert!(model.banner.is_none());
    }

    #[test]
    fn every_banner_kind_offers_an_action_it_can_carry_out() {
        assert_eq!(
            BannerKind::Loading.action_message(),
            None,
            "there is nothing to do while the index is being read",
        );
        assert_eq!(
            BannerKind::Empty.action_message(),
            Some(crate::app::Message::IndexRetried)
        );
        assert_eq!(
            BannerKind::Unavailable.action_message(),
            Some(crate::app::Message::IndexRetried)
        );
        assert_eq!(
            BannerKind::NoMatch.action_message(),
            Some(crate::app::Message::QueryCleared)
        );
    }

    #[test]
    fn an_unavailable_index_offers_a_retry() {
        let mut fixture = fixture();
        fixture.service = ServiceState::Unavailable {
            generation: 4,
            failure: Failure::service(FailureKind::IndexUnavailable {
                detail: "no HOME".to_owned(),
            }),
        };
        let model = fixture.project();

        let banner = model.banner.as_ref().expect("a banner");
        assert_eq!(banner.kind, BannerKind::Unavailable);
        assert_eq!(
            banner.title,
            fixture
                .localizer
                .message("launcher-state-unavailable-title")
        );
        assert!(
            banner
                .body
                .as_deref()
                .unwrap_or_default()
                .contains("no HOME")
        );
        assert_eq!(
            banner.action.as_deref(),
            Some(
                fixture
                    .localizer
                    .message("launcher-state-action-retry")
                    .as_str()
            )
        );
        assert_eq!(
            banner.kind.action_message(),
            Some(crate::app::Message::IndexRetried)
        );
        assert!(model.can_retry_index());
    }

    #[test]
    fn a_truncated_query_is_noticed() {
        let mut fixture = fixture();
        fixture.config.max_query_chars = 3;
        fixture.query = "abcdef".to_owned();
        let request = fixture.engine.request("abcdef", 3, 8);
        let result = search::execute(&fixture.index, &request);
        fixture.engine.apply(result);

        let model = fixture.project();
        let notice = model.notice.expect("a notice");
        assert!(notice.contains('3'), "{notice}");
    }

    #[test]
    fn skipped_entries_are_counted_in_a_notice() {
        let mut fixture = fixture();
        fixture.service = ServiceState::Ready {
            generation: 1,
            entries: 2,
            skipped: 5,
            directories: Vec::new(),
        };
        fixture.search("");

        let notice = fixture.project().notice.expect("a notice");
        assert!(notice.contains('5'), "{notice}");
    }

    #[test]
    fn a_launch_in_progress_says_what_is_launching() {
        let mut fixture = fixture();
        fixture.search("fire");
        fixture.activation = Some(Activation {
            selection: 1,
            entry: "firefox.desktop".to_owned(),
            name: "Firefox".to_owned(),
            outcome: LaunchOutcome::Requested,
        });

        let status = fixture.project().status.expect("a status");
        assert!(status.line.contains("Firefox"), "{}", status.line);
        assert!(status.reason.is_none());
        assert!(status.retry.is_none());
    }

    #[test]
    fn a_verified_launch_says_the_application_is_running() {
        let mut fixture = fixture();
        fixture.activation = Some(Activation {
            selection: 1,
            entry: "firefox.desktop".to_owned(),
            name: "Firefox".to_owned(),
            outcome: LaunchOutcome::Started { pid: 12 },
        });

        let status = fixture.project().status.expect("a status");
        assert!(status.line.contains("Firefox"));
        assert!(status.retry.is_none());
    }

    #[test]
    fn a_failed_launch_gives_the_reason_and_a_retry() {
        let mut fixture = fixture();
        fixture.activation = Some(Activation {
            selection: 1,
            entry: "firefox.desktop".to_owned(),
            name: "Firefox".to_owned(),
            outcome: LaunchOutcome::Failed {
                failure: Failure::for_entry(
                    FailureKind::SpawnFailed {
                        detail: "Permission denied".to_owned(),
                    },
                    "Firefox",
                ),
            },
        });

        let model = fixture.project();
        let status = model.status.as_ref().expect("a status");
        assert!(
            status
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("Permission denied")
        );
        assert_eq!(
            status.retry.as_deref(),
            Some(
                fixture
                    .localizer
                    .message("launcher-state-action-retry")
                    .as_str()
            )
        );
        assert!(model.can_retry_launch());
    }

    #[test]
    fn every_failure_has_its_own_message_in_the_catalog() {
        let localizer = crate::i18n::localizer(&Locale::parse("en-US").expect("a tag"))
            .expect("the catalogs load");

        let kinds = [
            FailureKind::IndexUnavailable {
                detail: "d".to_owned(),
            },
            FailureKind::NoCommand,
            FailureKind::UnknownFieldCode { code: 'q' },
            FailureKind::MisplacedFieldCode { code: 'F' },
            FailureKind::UnterminatedQuote,
            FailureKind::TrailingEscape,
            FailureKind::TrailingPercent,
            FailureKind::MissingTerminalPrefix,
            FailureKind::SpawnFailed {
                detail: "d".to_owned(),
            },
            FailureKind::ExitedBeforeReady {
                detail: "d".to_owned(),
            },
            FailureKind::NotReadyInTime { seconds: 5 },
        ];

        for kind in kinds {
            let failure = Failure::for_entry(kind.clone(), "App");
            let message = failure_message(&failure, &localizer);
            assert!(
                localizer.is_translated(message_id(&kind)),
                "{kind:?} has no catalog entry",
            );
            assert!(!message.is_empty(), "{kind:?} renders as an empty string");
            assert!(
                !message.contains('{'),
                "{kind:?} left a placeholder unsubstituted: {message}",
            );
        }
    }

    #[test]
    fn the_placeholder_for_a_field_code_is_written_as_the_file_wrote_it() {
        let localizer = crate::i18n::localizer(&Locale::parse("en-US").expect("a tag"))
            .expect("the catalogs load");
        let message = failure_message(
            &Failure::service(FailureKind::UnknownFieldCode { code: 'q' }),
            &localizer,
        );
        assert!(message.contains("%q"), "{message}");
    }

    #[test]
    fn a_row_for_an_entry_the_index_does_not_hold_is_disabled() {
        let mut fixture = fixture();
        fixture.index = AppIndex::from_entries(Vec::new(), 1);
        fixture.service = ServiceState::Ready {
            generation: 1,
            entries: 0,
            skipped: 0,
            directories: Vec::new(),
        };
        // A result that names an entry the index does not hold cannot happen
        // through `search`, so it is built by hand: the projection is defensive
        // about it rather than panicking.
        fixture.engine = SearchEngine::new();
        let request = fixture.engine.request("ghost", 128, 8);
        fixture.engine.apply(crate::search::SearchResult {
            generation: request.generation,
            query: "ghost".to_owned(),
            truncated: false,
            hits: vec![crate::search::Hit {
                id: "ghost.desktop".to_owned(),
                name: "Ghost".to_owned(),
                generic_name: None,
                kind: crate::search::MatchKind::Prefix,
            }],
        });
        fixture.query = "ghost".to_owned();

        let model = fixture.project();
        assert_eq!(model.rows.len(), 1);
        assert!(!model.rows[0].enabled);
        assert!(model.rows[0].disabled_reason.is_some());
    }

    #[test]
    fn the_generic_name_is_shown_as_the_row_details() {
        let mut entry = AppIndex::test_entry("gimp.desktop", "GIMP", "gimp");
        entry.generic_name = Some("Photo Editor".to_owned());
        let mut fixture = Fixture::new(vec![entry]);
        fixture.search("gimp");

        assert_eq!(
            fixture.project().rows[0].detail.as_deref(),
            Some("Photo Editor")
        );
    }
}
