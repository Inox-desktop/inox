//! The panel clock: expanding a configured format into a localized label.
//!
//! `panel.clock_format` is not a `strftime` string. `inox-config` accepts only
//! the directives in [`inox_config::CLOCK_DIRECTIVES`] and rejects everything
//! else while the file is parsed, so a format the panel cannot honour never
//! reaches it. This module is the other half of that bargain: it implements
//! exactly those directives, and returns [`ClockError`] rather than panicking if
//! handed one it does not know (a hand-built `PanelConfig`, or a `%` that
//! escapes nothing).
//!
//! Two decisions are worth spelling out.
//!
//! **The names are the panel's own translations, not `chrono`'s.** `%a`, `%A`,
//! `%b` and `%B` are read from the Fluent catalog, so a Chinese session shows
//! `星期五` where an English one shows `Friday`. `chrono`'s locale support would
//! need an extra feature and would still put English names in a crate that is
//! forbidden from holding user-visible text (root AGENTS.md §7.3).
//!
//! **`%Z` is the UTC offset, not a zone abbreviation.** The abbreviation lives in
//! the C library's `tzname` and is not reachable through the `clock` feature, so
//! `%Z` renders `UTC`, `UTC+08:00` or `UTC-05:30` — which is what the directive
//! has to mean for the value to be correct for every user, including the ones
//! whose zone has no abbreviation at all. This is recorded as not-implemented in
//! the crate README.

use chrono::{DateTime, Datelike, Offset, TimeZone, Timelike};
use inox_common::i18n::Localizer;
use tracing::warn;

/// Why a clock format could not be rendered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClockError {
    /// A directive the panel does not implement.
    ///
    /// `inox-config` rejects these while parsing, so this is the answer to a
    /// format that was built by hand — an error, never a panic and never
    /// literal `%Q` on the panel.
    #[error("`%{directive}` at position {index} is not a supported clock directive")]
    UnsupportedDirective {
        /// The directive character, without its `%`.
        directive: char,
        /// Byte position of the `%` in the format string.
        index: usize,
    },
    /// A `%` with nothing after it.
    #[error("the clock format ends with a `%` at position {index} that escapes nothing")]
    DanglingPercent {
        /// Byte position of the trailing `%`.
        index: usize,
    },
}

/// The text the panel shows for one clock reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockLabel {
    /// The clock itself, in the configured format.
    pub text: String,
    /// The tooltip: the full date, so the short format on the panel does not
    /// have to carry the year, the month name or the weekday (acceptance Q02).
    pub tooltip: String,
}

/// Expands `format` for `when`.
///
/// The supported set is exactly [`inox_config::CLOCK_DIRECTIVES`]: `%H` `%M` `%S`
/// `%I` `%p` `%Y` `%m` `%d` `%a` `%A` `%b` `%B` `%e` `%Z`, plus `%%` for a
/// literal percent sign. Anything between directives is copied unchanged.
///
/// # Errors
///
/// Returns [`ClockError::UnsupportedDirective`] for a directive outside that set
/// and [`ClockError::DanglingPercent`] for a trailing `%`.
pub fn format<Z: TimeZone>(
    format: &str,
    when: &DateTime<Z>,
    localizer: &Localizer,
) -> Result<String, ClockError> {
    let mut out = String::with_capacity(format.len() + 8);
    let mut characters = format.char_indices();

    while let Some((index, character)) = characters.next() {
        if character != '%' {
            out.push(character);
            continue;
        }

        let Some((_, directive)) = characters.next() else {
            return Err(ClockError::DanglingPercent { index });
        };

        match directive {
            'H' => out.push_str(&format!("{:02}", when.hour())),
            'M' => out.push_str(&format!("{:02}", when.minute())),
            'S' => out.push_str(&format!("{:02}", when.second())),
            'I' => out.push_str(&format!("{:02}", hour_on_a_twelve_hour_clock(when))),
            'p' => out.push_str(&localizer.message(meridiem_id(when.hour()))),
            'Y' => out.push_str(&format!("{:04}", when.year())),
            'm' => out.push_str(&format!("{:02}", when.month())),
            'd' => out.push_str(&format!("{:02}", when.day())),
            'a' => out.push_str(&localizer.message(&weekday_id("abbr", when))),
            'A' => out.push_str(&localizer.message(&weekday_id("full", when))),
            'b' => out.push_str(&localizer.message(&month_id("abbr", when))),
            'B' => out.push_str(&localizer.message(&month_id("full", when))),
            // `%e` is the space-padded day, which is what keeps a column of
            // dates aligned without a leading zero.
            'e' => out.push_str(&format!("{:>2}", when.day())),
            'Z' => out.push_str(&zone(when, localizer)),
            '%' => out.push('%'),
            other => {
                return Err(ClockError::UnsupportedDirective {
                    directive: other,
                    index,
                });
            }
        }
    }

    Ok(out)
}

/// The rendered clock and its tooltip, as the panel draws them.
///
/// This is what the view model carries; the binary and the tests both come
/// through here so the two cannot drift apart.
///
/// # Errors
///
/// As [`format()`]: the tooltip is only built once the format has been expanded
/// successfully.
pub fn label<Z: TimeZone>(
    format: &str,
    when: &DateTime<Z>,
    localizer: &Localizer,
) -> Result<ClockLabel, ClockError> {
    let text = self::format(format, when, localizer)?;

    let tooltip = localizer.message_with(
        "panel-clock-tooltip",
        &[
            (
                "weekday",
                localizer.message(&weekday_id("full", when)).into(),
            ),
            ("month", localizer.message(&month_id("full", when)).into()),
            ("day", format!("{}", when.day()).into()),
            ("year", format!("{}", when.year()).into()),
        ],
    );

    Ok(ClockLabel { text, tooltip })
}

/// [`label`], with a format error logged instead of propagated.
///
/// A format string cannot be invalid: `inox-config` rejects the unsupported
/// directives while parsing `shell.toml`. This is the path a hand-built
/// configuration takes, and the panel answers it by showing no clock at all
/// rather than a broken one.
pub fn label_or_none<Z: TimeZone>(
    format: &str,
    when: &DateTime<Z>,
    localizer: &Localizer,
) -> Option<ClockLabel> {
    match label(format, when, localizer) {
        Ok(label) => Some(label),
        Err(error) => {
            warn!(
                format,
                %error,
                "the panel clock format cannot be rendered; showing no clock"
            );
            None
        }
    }
}

/// The hour on a twelve-hour clock: 12, 1 .. 11.
fn hour_on_a_twelve_hour_clock<Z: TimeZone>(when: &DateTime<Z>) -> u32 {
    let hour = when.hour() % 12;

    if hour == 0 { 12 } else { hour }
}

/// The catalog id of the AM/PM marker for an hour on a 24-hour clock.
fn meridiem_id(hour: u32) -> &'static str {
    if hour < 12 {
        "panel-clock-am"
    } else {
        "panel-clock-pm"
    }
}

/// The catalog id of the weekday name, numbered from Monday as `chrono` does.
fn weekday_id<Z: TimeZone>(length: &str, when: &DateTime<Z>) -> String {
    format!(
        "panel-clock-weekday-{length}-{}",
        when.weekday().number_from_monday()
    )
}

/// The catalog id of the month name.
fn month_id<Z: TimeZone>(length: &str, when: &DateTime<Z>) -> String {
    format!("panel-clock-month-{length}-{}", when.month())
}

/// The UTC offset as `UTC`, `UTC+HH:MM` or `UTC-HH:MM`.
fn zone<Z: TimeZone>(when: &DateTime<Z>, localizer: &Localizer) -> String {
    let offset = when.offset().fix().local_minus_utc();

    if offset == 0 {
        return localizer.message("panel-clock-zone-utc");
    }

    let total_minutes = offset.unsigned_abs() / 60;

    localizer.message_with(
        "panel-clock-zone-offset",
        &[
            ("sign", if offset < 0 { "-" } else { "+" }.into()),
            ("hours", format!("{:02}", total_minutes / 60).into()),
            ("minutes", format!("{:02}", total_minutes % 60).into()),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use inox_common::i18n::Locale;

    use crate::i18n;

    fn localized(tag: &str) -> Localizer {
        let locale = Locale::parse(tag).expect("a well-formed tag");

        i18n::load(&locale).expect("the catalogs parse")
    }

    /// A fixed instant, so no test depends on the machine's clock or zone.
    fn moment(offset_hours: i32) -> DateTime<FixedOffset> {
        let zone = FixedOffset::east_opt(offset_hours * 3600).expect("a valid offset");

        zone.with_ymd_and_hms(2026, 10, 16, 9, 5, 7)
            .single()
            .expect("an unambiguous local time")
    }

    #[test]
    fn numeric_directives_are_zero_padded() {
        let localizer = localized("en-US");
        let when = moment(0);

        assert_eq!(format("%H", &when, &localizer).expect("supported"), "09");
        assert_eq!(format("%M", &when, &localizer).expect("supported"), "05");
        assert_eq!(format("%S", &when, &localizer).expect("supported"), "07");
        assert_eq!(format("%I", &when, &localizer).expect("supported"), "09");
        assert_eq!(format("%Y", &when, &localizer).expect("supported"), "2026");
        assert_eq!(format("%m", &when, &localizer).expect("supported"), "10");
        assert_eq!(format("%d", &when, &localizer).expect("supported"), "16");
    }

    #[test]
    fn the_twelve_hour_clock_counts_midnight_as_twelve() {
        let localizer = localized("en-US");
        let midnight = FixedOffset::east_opt(0)
            .expect("a valid offset")
            .with_ymd_and_hms(2026, 10, 16, 0, 30, 0)
            .single()
            .expect("an unambiguous local time");
        let noon = FixedOffset::east_opt(0)
            .expect("a valid offset")
            .with_ymd_and_hms(2026, 10, 16, 12, 30, 0)
            .single()
            .expect("an unambiguous local time");

        assert_eq!(
            format("%I", &midnight, &localizer).expect("supported"),
            "12"
        );
        assert_eq!(format("%I", &noon, &localizer).expect("supported"), "12");
        assert_eq!(
            format("%p", &midnight, &localizer).expect("supported"),
            "AM"
        );
        assert_eq!(format("%p", &noon, &localizer).expect("supported"), "PM");
    }

    #[test]
    fn the_space_padded_day_keeps_a_column_straight() {
        let localizer = localized("en-US");
        let zone = FixedOffset::east_opt(0).expect("a valid offset");
        let early = zone
            .with_ymd_and_hms(2026, 10, 5, 9, 0, 0)
            .single()
            .expect("an unambiguous local time");

        assert_eq!(format("%e", &early, &localizer).expect("supported"), " 5");
        assert_eq!(
            format("%e", &moment(0), &localizer).expect("supported"),
            "16"
        );
    }

    #[test]
    fn weekday_and_month_names_follow_the_locale() {
        let english = localized("en-US");
        let chinese = localized("zh-CN");
        // 2026-10-16 is a Friday, in the tenth month.
        let when = moment(0);

        assert_eq!(format("%a", &when, &english).expect("supported"), "Fri");
        assert_eq!(format("%A", &when, &english).expect("supported"), "Friday");
        assert_eq!(format("%b", &when, &english).expect("supported"), "Oct");
        assert_eq!(format("%B", &when, &english).expect("supported"), "October");

        assert_eq!(format("%a", &when, &chinese).expect("supported"), "周五");
        assert_eq!(format("%A", &when, &chinese).expect("supported"), "星期五");
        assert_eq!(format("%b", &when, &chinese).expect("supported"), "10 月");
        assert_eq!(format("%B", &when, &chinese).expect("supported"), "10 月");
    }

    #[test]
    fn the_zone_directive_reports_the_utc_offset() {
        let localizer = localized("en-US");

        assert_eq!(
            format("%Z", &moment(0), &localizer).expect("supported"),
            "UTC"
        );
        assert_eq!(
            format("%Z", &moment(8), &localizer).expect("supported"),
            "UTC+08:00"
        );
        assert_eq!(
            format("%Z", &moment(-6), &localizer).expect("supported"),
            "UTC-06:00"
        );
    }

    #[test]
    fn a_half_hour_zone_is_reported_to_the_minute() {
        let localizer = localized("en-US");
        let zone = FixedOffset::east_opt(5 * 3600 + 30 * 60).expect("a valid offset");
        let when = zone
            .with_ymd_and_hms(2026, 10, 16, 9, 5, 7)
            .single()
            .expect("an unambiguous local time");

        assert_eq!(
            format("%Z", &when, &localizer).expect("supported"),
            "UTC+05:30"
        );
    }

    #[test]
    fn a_percent_escape_renders_one_percent() {
        let localizer = localized("en-US");

        assert_eq!(
            format("100%% %H:%M", &moment(0), &localizer).expect("supported"),
            "100% 09:05"
        );
    }

    #[test]
    fn text_between_directives_is_copied_unchanged() {
        let localizer = localized("en-US");

        assert_eq!(
            format("%H时%M分", &moment(0), &localizer).expect("supported"),
            "09时05分"
        );
        assert_eq!(format("", &moment(0), &localizer).expect("supported"), "");
    }

    #[test]
    fn an_unsupported_directive_is_an_error_with_its_position() {
        let localizer = localized("en-US");

        assert_eq!(
            format("%H:%Q", &moment(0), &localizer),
            Err(ClockError::UnsupportedDirective {
                directive: 'Q',
                index: 3,
            })
        );
        assert_eq!(
            format("nope %", &moment(0), &localizer),
            Err(ClockError::DanglingPercent { index: 5 })
        );
    }

    #[test]
    fn every_directive_the_configuration_accepts_can_be_rendered() {
        let localizer = localized("en-US");
        let when = moment(-3);

        for directive in inox_config::CLOCK_DIRECTIVES {
            let format_string = format!("%{directive}");
            let rendered = format(&format_string, &when, &localizer);

            assert!(
                rendered.is_ok(),
                "`{format_string}` is accepted by `inox-config` but not rendered: {rendered:?}"
            );
            assert!(
                !rendered.expect("checked above").is_empty(),
                "`{format_string}` rendered nothing"
            );
        }

        // ... and the ones it rejects are rejected here too.
        for directive in ['Q', 'x', 'F', 'T', 'n'] {
            assert!(matches!(
                format(&format!("%{directive}"), &when, &localizer),
                Err(ClockError::UnsupportedDirective { .. })
            ));
        }
    }

    #[test]
    fn the_tooltip_carries_the_full_date_in_the_session_locale() {
        let english = localized("en-US");
        let chinese = localized("zh-CN");
        let when = moment(0);

        let english_label = label("%H:%M", &when, &english).expect("supported");
        assert_eq!(english_label.text, "09:05");
        assert_eq!(english_label.tooltip, "Friday, October 16 2026");

        let chinese_label = label("%H:%M", &when, &chinese).expect("supported");
        assert_eq!(chinese_label.text, "09:05");
        assert_eq!(chinese_label.tooltip, "2026 年 10 月 16 日 星期五");
    }

    #[test]
    fn an_unrenderable_format_produces_no_clock_rather_than_a_panic() {
        let localizer = localized("en-US");

        assert_eq!(label_or_none("%Q", &moment(0), &localizer), None);
        assert_eq!(
            label_or_none("%H:%M", &moment(0), &localizer).map(|label| label.text),
            Some("09:05".to_owned())
        );
    }
}
