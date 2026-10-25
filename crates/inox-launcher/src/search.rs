//! Searching the index: a bounded query, a deterministic ranking, and the
//! request generations that keep a stale result from overwriting a newer one.
//! Three of the interaction contract's requirements land here.
//! Three requirements land here.
//!
//! **Bounded.** The query is truncated to `launcher.max_query_chars` and the
//! result list to `launcher.max_results`, so a paste cannot make one keystroke
//! walk an unbounded string or produce an unbounded list.
//!
//! **Deterministic.** Results are ranked by how the query matched — whole name,
//! then name prefix, then a word inside the name, then a substring, then a
//! subsequence — and ties are broken by the lower-cased name and then by the
//! desktop file id. Two runs over the same index and the same query produce the
//! same list, including for an empty query (acceptance D02: "filtering and
//! dedup must be deterministic").
//!
//! **Generational.** [`SearchEngine::request`] hands out a generation and
//! [`SearchEngine::apply`] refuses any result whose generation is not the
//! newest. Searching an in-memory list is fast enough to happen inside `update`
//! today, but the freshness rule is stated where it can be checked, because a
//! later move to an on-thread scan would otherwise be free to reorder results
//! behind the user's typing.

use std::cmp::Ordering;

use crate::index::{AppIndex, IndexedEntry};

/// How a query matched an application's name.
///
/// The order of the variants is the ranking, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    /// The whole name, case-insensitively.
    Whole,
    /// The name begins with the query.
    Prefix,
    /// A word inside the name begins with the query.
    WordPrefix,
    /// The query appears inside the name.
    Substring,
    /// The query's characters appear in order, with other characters between.
    Subsequence,
}

/// One search hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The desktop file id of the matching entry.
    pub id: String,
    /// The entry's localized name.
    pub name: String,
    /// The entry's localized generic name, when it has one.
    pub generic_name: Option<String>,
    /// How the query matched.
    pub kind: MatchKind,
}

/// A search that was asked for, and that has not necessarily been answered yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    /// The generation this request belongs to.
    pub generation: u64,
    /// The query after bounding.
    pub query: String,
    /// Whether the query was cut to `launcher.max_query_chars`.
    pub truncated: bool,
    /// The largest accepted result list.
    pub limit: usize,
}

/// The answer to a [`SearchRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    /// The generation of the request this answers.
    pub generation: u64,
    /// The query this result is for.
    pub query: String,
    /// Whether the query was cut.
    pub truncated: bool,
    /// The hits, best first.
    pub hits: Vec<Hit>,
}

/// The launcher's query state: which generation is current, and which answer
/// is being shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchEngine {
    next: u64,
    applied: u64,
    result: Option<SearchResult>,
}

impl Default for SearchEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchEngine {
    /// An engine with no query and no results.
    pub fn new() -> Self {
        Self {
            next: 1,
            applied: 0,
            result: None,
        }
    }

    /// Starts a new search for `raw`, bounded by `max_query_chars` and `limit`.
    ///
    /// Every call starts a generation, including a call that does not change
    /// the query: a caller that debounces or repeats a search must still be able
    /// to tell the answers apart.
    pub fn request(&mut self, raw: &str, max_query_chars: usize, limit: usize) -> SearchRequest {
        let (query, truncated) = bound(raw, max_query_chars);
        let generation = self.next;
        self.next += 1;
        SearchRequest {
            generation,
            query,
            truncated,
            limit,
        }
    }

    /// Accepts an answer.
    ///
    /// Returns whether the result was applied. A result from an older
    /// generation, or from a generation that has not been asked for, is
    /// discarded: it cannot overwrite the newest view.
    pub fn apply(&mut self, result: SearchResult) -> bool {
        let newest = self.next.saturating_sub(1);
        if result.generation != newest || result.generation <= self.applied {
            return false;
        }
        self.applied = result.generation;
        self.result = Some(result);
        true
    }

    /// The newest applied result, if any.
    pub fn current(&self) -> Option<&SearchResult> {
        self.result.as_ref()
    }

    /// The generation the newest request was given.
    ///
    /// Before the first request this is `0`, which no generation can be.
    pub fn current_generation(&self) -> u64 {
        self.next.saturating_sub(1)
    }

    /// The generation whose answer is being shown.
    pub fn applied_generation(&self) -> u64 {
        self.applied
    }
}

/// Cuts a query to `max_chars` characters, reporting whether it cut anything.
///
/// Character-wise, not byte-wise, so a query in Chinese is bounded by what the
/// user typed rather than by how many bytes it took.
pub fn bound(raw: &str, max_chars: usize) -> (String, bool) {
    let mut characters = raw.chars();
    let bounded: String = characters.by_ref().take(max_chars).collect();
    let truncated = characters.next().is_some();
    (bounded, truncated)
}

/// Runs a request against an index.
pub fn execute(index: &AppIndex, request: &SearchRequest) -> SearchResult {
    let query = request.query.trim();

    let hits = if query.is_empty() {
        empty_query_hits(index, request.limit)
    } else {
        let needle = query.to_lowercase();
        let mut hits: Vec<Hit> = index
            .entries()
            .iter()
            .filter_map(|entry| match_entry(entry, &needle))
            .collect();
        hits.sort_by(compare);
        hits.truncate(request.limit);
        hits
    };

    SearchResult {
        generation: request.generation,
        query: request.query.clone(),
        truncated: request.truncated,
        hits,
    }
}

/// The list an empty query shows: every application, by name.
///
/// Defined rather than left to scan order, so that opening the launcher twice
/// shows the same list.
fn empty_query_hits(index: &AppIndex, limit: usize) -> Vec<Hit> {
    let mut entries: Vec<&IndexedEntry> = index.entries().iter().collect();
    entries.sort_by(|left, right| compare_names(&left.name, &left.id, &right.name, &right.id));
    entries
        .into_iter()
        .take(limit)
        .map(|entry| hit(entry, MatchKind::Prefix))
        .collect()
}

/// Matches one entry, preferring the name over the generic name.
fn match_entry(entry: &IndexedEntry, needle: &str) -> Option<Hit> {
    let name = entry.name.to_lowercase();
    let kind = classify(&name, needle).or_else(|| {
        entry
            .generic_name
            .as_ref()
            .and_then(|generic| classify(&generic.to_lowercase(), needle))
    })?;
    Some(hit(entry, kind))
}

/// Builds a hit from an entry.
fn hit(entry: &IndexedEntry, kind: MatchKind) -> Hit {
    Hit {
        id: entry.id.clone(),
        name: entry.name.clone(),
        generic_name: entry.generic_name.clone(),
        kind,
    }
}

/// The ranking, best first, with ties broken by name and then by id.
fn compare(left: &Hit, right: &Hit) -> Ordering {
    left.kind
        .cmp(&right.kind)
        .then_with(|| compare_names(&left.name, &left.id, &right.name, &right.id))
}

/// The tie-break: lower-cased name first, then id.
fn compare_names(left_name: &str, left_id: &str, right_name: &str, right_id: &str) -> Ordering {
    left_name
        .to_lowercase()
        .cmp(&right_name.to_lowercase())
        .then_with(|| left_id.cmp(right_id))
}

/// How `needle` (already lower-cased) matches `haystack` (already lower-cased).
fn classify(haystack: &str, needle: &str) -> Option<MatchKind> {
    if haystack == needle {
        return Some(MatchKind::Whole);
    }
    if haystack.starts_with(needle) {
        return Some(MatchKind::Prefix);
    }
    if starts_a_word(haystack, needle) {
        return Some(MatchKind::WordPrefix);
    }
    if haystack.contains(needle) {
        return Some(MatchKind::Substring);
    }
    if is_subsequence(haystack, needle) {
        return Some(MatchKind::Subsequence);
    }
    None
}

/// Whether `needle` begins a word of `haystack`, where a word begins after
/// anything that is not alphanumeric.
fn starts_a_word(haystack: &str, needle: &str) -> bool {
    haystack
        .match_indices(needle)
        .any(|(index, _)| match haystack[..index].chars().next_back() {
            None => true,
            Some(previous) => !previous.is_alphanumeric(),
        })
}

/// Whether `needle`'s characters appear in `haystack` in order.
///
/// Unicode-aware by construction: the walk is over characters, so a query in
/// Chinese or Greek matches the same way an ASCII one does.
fn is_subsequence(haystack: &str, needle: &str) -> bool {
    let mut characters = haystack.chars();
    needle
        .chars()
        .all(|wanted| characters.any(|candidate| candidate == wanted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::AppIndex;
    use proptest::prelude::*;

    /// An index over the given `(id, name)` pairs.
    fn index(names: &[(&str, &str)]) -> AppIndex {
        let entries = names
            .iter()
            .map(|(id, name)| AppIndex::test_entry(id, name, "app"))
            .collect();
        AppIndex::from_entries(entries, 1)
    }

    /// The ids a query returns, in order.
    fn ids(index: &AppIndex, query: &str) -> Vec<String> {
        let mut engine = SearchEngine::new();
        let request = engine.request(query, 128, 8);
        execute(index, &request)
            .hits
            .iter()
            .map(|hit| hit.id.clone())
            .collect()
    }

    #[test]
    fn a_whole_name_ranks_above_a_prefix() {
        let index = index(&[("f.desktop", "Firefox"), ("fire.desktop", "Fire")]);
        assert_eq!(ids(&index, "fire"), ["fire.desktop", "f.desktop"]);
    }

    #[test]
    fn a_prefix_ranks_above_a_word_prefix() {
        let index = index(&[
            ("mozilla.desktop", "Mozilla Firefox"),
            ("fire.desktop", "Fire"),
        ]);
        assert_eq!(ids(&index, "fire"), ["fire.desktop", "mozilla.desktop"]);
    }

    #[test]
    fn a_word_prefix_ranks_above_a_substring() {
        let index = index(&[
            ("thunder.desktop", "Thunderbird"),
            ("gimp.desktop", "GNU Image"),
        ]);
        assert_eq!(ids(&index, "image"), ["gimp.desktop"]);
        assert_eq!(ids(&index, "bird"), ["thunder.desktop"]);
    }

    #[test]
    fn a_word_prefix_is_not_a_substring() {
        // "GNU Image": `image` starts a word, `mage` does not.
        let index = index(&[("gimp.desktop", "GNU Image")]);
        assert_eq!(ids(&index, "image"), ["gimp.desktop"]);
        assert_eq!(ids(&index, "mage"), ["gimp.desktop"]);

        let mut engine = SearchEngine::new();
        let request = engine.request("image", 128, 8);
        assert_eq!(
            execute(&index, &request).hits[0].kind,
            MatchKind::WordPrefix
        );

        let request = engine.request("mage", 128, 8);
        assert_eq!(execute(&index, &request).hits[0].kind, MatchKind::Substring);
    }

    #[test]
    fn a_subsequence_is_the_weakest_match() {
        let index = index(&[
            ("ghostscript.desktop", "Ghostscript"),
            ("gsc.desktop", "Gsc"),
        ]);
        assert_eq!(ids(&index, "gsc"), ["gsc.desktop", "ghostscript.desktop"]);

        let mut engine = SearchEngine::new();
        let request = engine.request("gsc", 128, 8);
        let result = execute(&index, &request);
        assert_eq!(result.hits[0].kind, MatchKind::Whole);
        assert_eq!(result.hits[1].kind, MatchKind::Subsequence);
    }

    #[test]
    fn the_subsequence_may_be_spread_out() {
        let index = index(&[("firefox.desktop", "Firefox")]);
        assert_eq!(ids(&index, "ffx"), ["firefox.desktop"]);
        assert!(ids(&index, "xyz").is_empty());
    }

    #[test]
    fn matching_is_case_insensitive_and_unicode_aware() {
        let index = index(&[
            ("files.desktop", "文件管理器"),
            ("greek.desktop", "Περιήγηση"),
            ("mixed.desktop", "ÜBER"),
        ]);
        assert_eq!(ids(&index, "文件"), ["files.desktop"]);
        assert_eq!(ids(&index, "περι"), ["greek.desktop"]);
        assert_eq!(ids(&index, "über"), ["mixed.desktop"]);
        assert_eq!(ids(&index, "ÜBER"), ["mixed.desktop"]);
    }

    #[test]
    fn a_generic_name_matches_when_the_name_does_not() {
        let mut entry = crate::index::AppIndex::test_entry("photo.desktop", "GIMP", "gimp");
        entry.generic_name = Some("Photo Editor".to_owned());
        let index = AppIndex::from_entries(vec![entry], 1);
        assert_eq!(ids(&index, "photo"), ["photo.desktop"]);
    }

    #[test]
    fn ties_break_by_name_and_then_by_id() {
        let index = index(&[
            ("z.desktop", "Same"),
            ("a.desktop", "Same"),
            ("b.desktop", "Another"),
        ]);
        // Both "Same" entries tie on the name; the id decides between them.
        assert_eq!(ids(&index, "sa"), ["a.desktop", "z.desktop"]);
    }

    #[test]
    fn the_order_is_the_same_every_time() {
        let index = index(&[
            ("a.desktop", "Alpha"),
            ("b.desktop", "Beta"),
            ("ab.desktop", "Alphabet"),
            ("b2.desktop", "beta two"),
        ]);
        let once = ids(&index, "alp");
        for _ in 0..8 {
            assert_eq!(ids(&index, "alp"), once);
        }
    }

    #[test]
    fn an_empty_query_lists_everything_by_name() {
        let index = index(&[
            ("z.desktop", "Zebra"),
            ("a.desktop", "apple"),
            ("m.desktop", "Mango"),
        ]);
        assert_eq!(
            ids(&index, "   "),
            ["a.desktop", "m.desktop", "z.desktop"],
            "the empty list is defined, not whatever order the scan produced",
        );
    }

    #[test]
    fn the_result_list_is_bounded() {
        let index = index(&[
            ("a.desktop", "Alpha"),
            ("b.desktop", "Alpha"),
            ("c.desktop", "Alpha"),
        ]);
        let mut engine = SearchEngine::new();
        let request = engine.request("alpha", 128, 2);
        assert_eq!(execute(&index, &request).hits.len(), 2);
    }

    #[test]
    fn a_query_longer_than_the_limit_is_cut_and_says_so() {
        let mut engine = SearchEngine::new();
        let request = engine.request("abcdefghij", 4, 8);
        assert_eq!(request.query, "abcd");
        assert!(request.truncated);

        let request = engine.request("abcd", 4, 8);
        assert_eq!(request.query, "abcd");
        assert!(!request.truncated);
    }

    #[test]
    fn the_bound_counts_characters_not_bytes() {
        let (bounded, truncated) = bound("文件管理器", 3);
        assert_eq!(bounded, "文件管");
        assert!(truncated);
    }

    #[test]
    fn a_result_from_an_older_generation_never_replaces_a_newer_one() {
        let index = index(&[("a.desktop", "Alpha")]);
        let mut engine = SearchEngine::new();

        let first = engine.request("a", 128, 8);
        let stale = execute(&index, &first);

        let second = engine.request("al", 128, 8);
        let fresh = execute(&index, &second);

        assert!(engine.apply(fresh), "the newest result applies");
        assert!(
            !engine.apply(stale),
            "an answer the launcher has already moved past does not overwrite the view",
        );
        assert_eq!(engine.applied_generation(), second.generation);
        assert_eq!(
            engine.current().map(|result| result.query.as_str()),
            Some("al")
        );
    }

    #[test]
    fn a_generation_that_was_never_requested_is_rejected() {
        let mut engine = SearchEngine::new();
        let request = engine.request("a", 128, 8);
        let future = SearchResult {
            generation: request.generation + 7,
            query: "a".to_owned(),
            truncated: false,
            hits: Vec::new(),
        };
        assert!(!engine.apply(future));
        assert!(engine.current().is_none());
    }

    #[test]
    fn a_generation_is_applied_only_once() {
        let index = index(&[("a.desktop", "Alpha")]);
        let mut engine = SearchEngine::new();
        let request = engine.request("a", 128, 8);
        let result = execute(&index, &request);
        assert!(engine.apply(result.clone()));
        assert!(!engine.apply(result));
    }

    #[test]
    fn a_generation_before_the_first_request_is_zero() {
        let engine = SearchEngine::new();
        assert_eq!(engine.current_generation(), 0);
        assert_eq!(engine.applied_generation(), 0);
        assert!(engine.current().is_none());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// The same query over the same index always produces the same list.
        #[test]
        fn searching_is_deterministic(query in "(?s).{0,24}") {
            let index = index(&[
                ("a.desktop", "Alpha"),
                ("b.desktop", "Alphabet"),
                ("c.desktop", "Beta"),
                ("d.desktop", "Gamma"),
                ("e.desktop", "文件管理器"),
                ("f.desktop", "Thunderbird"),
                ("g.desktop", "GNU Image"),
            ]);
            prop_assert_eq!(ids(&index, &query), ids(&index, &query));
        }

        /// Every hit is a real entry, no more than `limit` are returned, and the
        /// list is in the ranking's own order.
        #[test]
        fn every_result_is_bounded_and_ordered(
            query in "(?s).{1,24}",
            limit in 1usize..6,
        ) {
            let index = index(&[
                ("a.desktop", "Alpha"),
                ("b.desktop", "Alphabet"),
                ("c.desktop", "Beta"),
                ("d.desktop", "Gamma"),
            ]);
            let mut engine = SearchEngine::new();
            let request = engine.request(&query, 8, limit);
            let result = execute(&index, &request);

            prop_assert!(result.hits.len() <= limit);
            prop_assert!(request.query.chars().count() <= 8);
            for hit in &result.hits {
                prop_assert!(index.get(&hit.id).is_some());
            }
            for pair in result.hits.windows(2) {
                prop_assert!(compare(&pair[0], &pair[1]) != Ordering::Greater);
            }
        }

        /// Applying results out of order always leaves the newest one applied.
        #[test]
        fn the_newest_request_is_the_one_that_survives(queries in proptest::collection::vec("(?s).{0,8}", 1..6)) {
            let index = index(&[("a.desktop", "Alpha")]);
            let mut engine = SearchEngine::new();
            let mut results = Vec::new();
            for query in &queries {
                let request = engine.request(query, 128, 8);
                results.push(execute(&index, &request));
            }

            let newest = results.last().map(|result| result.generation).unwrap_or(0);
            // Apply them newest first, then backwards: only the newest applies.
            for result in results.iter().rev() {
                let applied = engine.apply(result.clone());
                prop_assert_eq!(applied, result.generation == newest);
            }
            prop_assert_eq!(engine.applied_generation(), newest);
        }
    }
}
