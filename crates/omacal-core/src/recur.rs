use crate::layout::Interval;
use crate::zone::midnight_in_zone;
use chrono::TimeZone;
use rrule::{RRuleSet, Tz};
use std::str::FromStr;

#[derive(Debug, thiserror::Error)]
pub enum RecurError {
    #[error("unknown time zone: {0}")]
    UnknownTimeZone(String),
    #[error("invalid recurrence rule: {0}")]
    InvalidRule(String),
    #[error("timestamp out of range: {0}")]
    OutOfRange(i64),
}

/// A recurring (or single) event as stored: a start instant, the IANA zone it
/// was authored in, a duration, and Google's raw recurrence lines.
#[derive(Debug, Clone)]
pub struct Series<'a> {
    pub dtstart_ms: i64,
    pub dtstart_tz: &'a str,
    pub duration_ms: i64,
    pub is_all_day: bool,
    pub recurrence: &'a [String],
}

/// The result of expanding a series: the concrete occurrences, plus whether
/// `limit` cut the expansion short before the `[from_ms, to_ms)` window was
/// fully covered. A caller that ignores `truncated` cannot tell "there were
/// exactly this many occurrences" from "there may be more we didn't see".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expansion {
    pub intervals: Vec<Interval>,
    pub truncated: bool,
}

fn to_chrono(ms: i64) -> Result<chrono::DateTime<Tz>, RecurError> {
    Tz::UTC
        .timestamp_millis_opt(ms)
        .single()
        .ok_or(RecurError::OutOfRange(ms))
}

/// Renders the DTSTART line in the series' own zone, which is what makes a
/// "09:00 every Monday" meeting stay at 09:00 across a DST transition.
///
/// All-day events are the exception: RFC 5545's `VALUE=DATE` form carries no
/// TZID, so its calendar date must be read directly off `dtstart_ms` *in
/// UTC* (the convention this crate stores all-day dates under) rather than
/// converted through `zone` first. Converting through `zone` here silently
/// shifts the date by ±1 day whenever `zone`'s offset carries UTC midnight
/// across a day boundary (e.g. `America/Los_Angeles`, UTC-7/-8) — that was a
/// real bug in an earlier version of this function. It also matters because
/// `rrule` parses a TZID-less `VALUE=DATE` DTSTART against `Tz::LOCAL` (the
/// *host machine's* system zone) internally; see `midnight_in_zone` for how
/// the occurrences it produces are re-anchored to something host-independent
/// on the way back out.
fn dtstart_line(series: &Series, zone: chrono_tz::Tz) -> Result<String, RecurError> {
    if series.is_all_day {
        let date = chrono::Utc
            .timestamp_millis_opt(series.dtstart_ms)
            .single()
            .ok_or(RecurError::OutOfRange(series.dtstart_ms))?
            .date_naive();
        return Ok(format!("DTSTART;VALUE=DATE:{}", date.format("%Y%m%d")));
    }

    let local = chrono::Utc
        .timestamp_millis_opt(series.dtstart_ms)
        .single()
        .ok_or(RecurError::OutOfRange(series.dtstart_ms))?
        .with_timezone(&zone);

    Ok(format!(
        "DTSTART;TZID={}:{}",
        series.dtstart_tz,
        local.format("%Y%m%dT%H%M%S")
    ))
}

/// Expands `series` into concrete intervals overlapping `[from_ms, to_ms)`.
///
/// `limit` bounds the number of occurrences generated, guarding against
/// unbounded rules such as `FREQ=MINUTELY` with no `COUNT`/`UNTIL`; when it
/// does cut the expansion short, `Expansion::truncated` is set so the caller
/// can tell.
pub fn expand(
    series: &Series,
    from_ms: i64,
    to_ms: i64,
    limit: u16,
) -> Result<Expansion, RecurError> {
    // Validate the zone even when there is no rule, so callers get a
    // consistent error rather than a silent pass.
    let zone: chrono_tz::Tz = series
        .dtstart_tz
        .parse()
        .map_err(|_| RecurError::UnknownTimeZone(series.dtstart_tz.to_string()))?;
    let dtstart = dtstart_line(series, zone)?;

    if series.recurrence.is_empty() {
        let end = series.dtstart_ms + series.duration_ms;
        let intervals = if series.dtstart_ms < to_ms && end > from_ms {
            vec![Interval { start_ms: series.dtstart_ms, end_ms: end }]
        } else {
            Vec::new()
        };
        return Ok(Expansion { intervals, truncated: false });
    }

    let mut source = String::with_capacity(128);
    source.push_str(&dtstart);
    for line in series.recurrence {
        source.push('\n');
        source.push_str(line.trim());
    }

    let set = RRuleSet::from_str(&source)
        .map_err(|e| RecurError::InvalidRule(format!("{e}: {source}")))?;

    // Widen the query by the event duration so an occurrence that started
    // before the window but is still running is not dropped.
    let query_from = from_ms.saturating_sub(series.duration_ms);
    let result = set
        .after(to_chrono(query_from)?)
        .before(to_chrono(to_ms)?)
        .all(limit);

    let intervals = result
        .dates
        .into_iter()
        .map(|d| {
            let start = if series.is_all_day {
                midnight_in_zone(d.date_naive(), zone)
            } else {
                d.timestamp_millis()
            };
            Interval { start_ms: start, end_ms: start + series.duration_ms }
        })
        .filter(|i| i.start_ms < to_ms && i.end_ms > from_ms)
        .collect();

    Ok(Expansion { intervals, truncated: result.limited })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monday 2026-08-03 09:00:00 Europe/Sofia == 06:00:00Z (EEST, UTC+3).
    /// Verify with: `python3 -c "import datetime as d; print(int(d.datetime(2026,8,3,6,tzinfo=d.timezone.utc).timestamp()*1000))"`
    const MON_0900_SOFIA: i64 = 1_785_736_800_000;
    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 24 * HOUR;

    /// Midnight UTC of Monday 2026-08-03 — the convention this crate stores
    /// an all-day event's calendar date under. Derived from the
    /// independently-verified `MON_0900_SOFIA` rather than a fresh literal.
    const ALL_DAY_AUG3: i64 = MON_0900_SOFIA - 6 * HOUR;

    fn weekly(rules: &[&str]) -> Vec<String> {
        rules.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_non_recurring_event_yields_itself_when_in_window() {
        let s = Series {
            dtstart_ms: MON_0900_SOFIA, dtstart_tz: "Europe/Sofia",
            duration_ms: 30 * 60_000, is_all_day: false, recurrence: &[],
        };
        let out = expand(&s, MON_0900_SOFIA - DAY, MON_0900_SOFIA + DAY, 50).unwrap().intervals;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].start_ms, MON_0900_SOFIA);
        assert_eq!(out[0].end_ms, MON_0900_SOFIA + 30 * 60_000);
    }

    #[test]
    fn a_non_recurring_event_outside_the_window_yields_nothing() {
        let s = Series {
            dtstart_ms: MON_0900_SOFIA, dtstart_tz: "Europe/Sofia",
            duration_ms: 30 * 60_000, is_all_day: false, recurrence: &[],
        };
        let out = expand(&s, MON_0900_SOFIA + 10 * DAY, MON_0900_SOFIA + 20 * DAY, 50).unwrap().intervals;
        assert!(out.is_empty());
    }

    #[test]
    fn a_daily_standup_yields_one_per_day() {
        let s = Series {
            dtstart_ms: MON_0900_SOFIA, dtstart_tz: "Europe/Sofia",
            duration_ms: 30 * 60_000, is_all_day: false,
            recurrence: &weekly(&["RRULE:FREQ=DAILY"]),
        };
        // Window covering Mon..Fri inclusive.
        let out = expand(&s, MON_0900_SOFIA - HOUR, MON_0900_SOFIA + 5 * DAY, 50).unwrap().intervals;
        assert_eq!(out.len(), 5);
        assert_eq!(out[0].start_ms, MON_0900_SOFIA);
        assert_eq!(out[1].start_ms, MON_0900_SOFIA + DAY);
    }

    #[test]
    fn a_custom_weekly_pattern_yields_each_selected_day_and_no_others() {
        let s = Series {
            dtstart_ms: MON_0900_SOFIA, dtstart_tz: "Europe/Sofia",
            duration_ms: 30 * 60_000, is_all_day: false,
            recurrence: &weekly(&["RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR"]),
        };
        let out = expand(
            &s,
            MON_0900_SOFIA - HOUR,
            MON_0900_SOFIA + 7 * DAY,
            50,
        )
        .unwrap()
        .intervals;
        let starts = out.iter().map(|interval| interval.start_ms).collect::<Vec<_>>();
        assert_eq!(
            starts,
            [
                MON_0900_SOFIA,
                MON_0900_SOFIA + 2 * DAY,
                MON_0900_SOFIA + 4 * DAY,
            ]
        );
    }

    #[test]
    fn every_instance_keeps_the_series_duration() {
        let s = Series {
            dtstart_ms: MON_0900_SOFIA, dtstart_tz: "Europe/Sofia",
            duration_ms: 90 * 60_000, is_all_day: false,
            recurrence: &weekly(&["RRULE:FREQ=DAILY;COUNT=3"]),
        };
        let out = expand(&s, MON_0900_SOFIA - HOUR, MON_0900_SOFIA + 5 * DAY, 50).unwrap().intervals;
        for i in &out {
            assert_eq!(i.end_ms - i.start_ms, 90 * 60_000);
        }
    }

    #[test]
    fn exdate_removes_an_instance() {
        let s = Series {
            dtstart_ms: MON_0900_SOFIA, dtstart_tz: "Europe/Sofia",
            duration_ms: 30 * 60_000, is_all_day: false,
            recurrence: &weekly(&[
                "RRULE:FREQ=DAILY",
                // Tuesday 2026-08-04 09:00 Sofia == 06:00Z
                "EXDATE;TZID=Europe/Sofia:20260804T090000",
            ]),
        };
        let out = expand(&s, MON_0900_SOFIA - HOUR, MON_0900_SOFIA + 3 * DAY, 50).unwrap().intervals;
        assert!(out.iter().all(|i| i.start_ms != MON_0900_SOFIA + DAY));
    }

    /// The shape a real "custom" series arrives in (seen live, 2026-08-16):
    /// a monthly nth-weekday rule, plus one EXDATE line naming *several*
    /// deleted occurrences in a comma list, in a zone with a half-hour
    /// offset. Everything before this covered only single-value EXDATEs.
    #[test]
    fn a_comma_list_exdate_removes_each_named_instance() {
        // Wednesday 2025-08-20 13:30 Asia/Kolkata (+5:30) == 08:00Z, the
        // third Wednesday of its month. Verify with:
        // `python3 -c "import datetime as d; print(int(d.datetime(2025,8,20,8,tzinfo=d.timezone.utc).timestamp()*1000))"`
        let s = Series {
            dtstart_ms: 1_755_676_800_000, dtstart_tz: "Asia/Kolkata",
            duration_ms: 3 * HOUR, is_all_day: false,
            recurrence: &weekly(&[
                "RRULE:FREQ=MONTHLY;BYDAY=3WE",
                "EXDATE;TZID=Asia/Kolkata:20260218T133000,20260318T133000,20260520T133000,20260617T133000,20260715T133000",
            ]),
        };
        // Feb 1 .. Sep 1 2026 (UTC): five of the seven third Wednesdays are
        // excluded, leaving exactly Apr 15 and Aug 19, both at 08:00Z.
        let out = expand(&s, 1_769_904_000_000, 1_788_220_800_000, 50).unwrap().intervals;
        let starts: Vec<i64> = out.iter().map(|i| i.start_ms).collect();
        assert_eq!(starts, vec![1_776_240_000_000, 1_787_126_400_000]);
    }

    #[test]
    fn count_is_respected() {
        let s = Series {
            dtstart_ms: MON_0900_SOFIA, dtstart_tz: "Europe/Sofia",
            duration_ms: 30 * 60_000, is_all_day: false,
            recurrence: &weekly(&["RRULE:FREQ=DAILY;COUNT=2"]),
        };
        let out = expand(&s, MON_0900_SOFIA - HOUR, MON_0900_SOFIA + 30 * DAY, 50).unwrap().intervals;
        assert_eq!(out.len(), 2);
    }

    /// The reason `jiff`/IANA zones matter. Europe/Sofia leaves DST on
    /// 2026-10-25. A 09:00 local weekly meeting must stay at 09:00 local,
    /// which means its UTC instant shifts by an hour across the boundary.
    #[test]
    fn a_local_time_series_survives_a_dst_transition() {
        // Monday 2026-09-28 09:00 Sofia (EEST, +3) == 06:00Z
        let sep28 = 1_790_575_200_000;
        let s = Series {
            dtstart_ms: sep28, dtstart_tz: "Europe/Sofia",
            duration_ms: HOUR, is_all_day: false,
            recurrence: &weekly(&["RRULE:FREQ=WEEKLY;BYDAY=MO"]),
        };
        let out = expand(&s, sep28 - HOUR, sep28 + 45 * DAY, 50).unwrap().intervals;
        let deltas: Vec<i64> = out.windows(2).map(|w| w[1].start_ms - w[0].start_ms).collect();
        // Exactly one gap is 7 days + 1 hour: the week DST ends.
        assert_eq!(deltas.iter().filter(|&&d| d == 7 * DAY + HOUR).count(), 1,
                   "expected one DST-adjusted gap, got {:?}", deltas);
        assert!(deltas.iter().all(|&d| d == 7 * DAY || d == 7 * DAY + HOUR));
    }

    /// Pins `rrule`'s actual behaviour when a weekly local anchor time falls
    /// in a spring-forward gap, so a future `rrule` upgrade that resolves
    /// gaps differently fails loudly instead of silently moving a meeting.
    ///
    /// Europe/Sofia springs forward at local 03:00 -> 04:00 on 2026-03-29,
    /// so a 03:30 weekly meeting has no such instant that week. `rrule`
    /// resolves this internally by adding the nominal time-of-day (03:30) as
    /// an *elapsed duration* from local midnight rather than searching for a
    /// nearby valid wall-clock time. Because Sofia's midnight that day is
    /// still on the pre-transition (+2) offset, this produces a flat 7-day
    /// UTC delta from the prior occurrence while the displayed local time
    /// silently jumps forward an hour, to 04:30 EEST. Verified independently
    /// via `zoneinfo`:
    /// `python3 -c "import datetime as d,zoneinfo as z; sofia=z.ZoneInfo('Europe/Sofia'); print(int(d.datetime(2026,3,29,0,tzinfo=sofia).timestamp()*1000)+3*3600000+30*60000)"`
    #[test]
    fn a_weekly_series_crosses_the_spring_forward_gap() {
        // Sunday 2026-03-22 03:30 Europe/Sofia (EET, +2) == 01:30:00Z.
        let dtstart = 1_774_143_000_000;
        let s = Series {
            dtstart_ms: dtstart, dtstart_tz: "Europe/Sofia",
            duration_ms: HOUR, is_all_day: false,
            recurrence: &weekly(&["RRULE:FREQ=WEEKLY"]),
        };
        let out = expand(&s, dtstart - HOUR, dtstart + 8 * DAY, 10).unwrap().intervals;
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].start_ms, dtstart);
        assert_eq!(out[1].start_ms, dtstart + 7 * DAY);
        assert_eq!(out[1].start_ms, 1_774_747_800_000, "expected the gap week's occurrence to resolve to 2026-03-29 04:30 EEST");
    }

    /// Pins `rrule`'s actual behaviour when a weekly local anchor time falls
    /// in a fall-back's *ambiguous* window, so a future `rrule` upgrade that
    /// resolves ambiguity differently fails loudly instead of silently
    /// moving a meeting.
    ///
    /// Europe/Sofia falls back at local 03:59:59 EEST -> 03:00:00 EET on
    /// 2026-10-25, so local 03:00-03:59 occurs twice that day (NOT 02:30,
    /// which is unambiguous — verified with `zoneinfo`'s `fold` parameter
    /// before writing this test). `rrule` resolves the ambiguity via the
    /// same "elapsed duration from local midnight" fallback used for gaps,
    /// which — because Sofia's midnight that day is still on the
    /// pre-transition (+3) offset — lands on the *earlier* (EEST) instance,
    /// again producing a flat 7-day UTC delta from the prior occurrence.
    /// Verified independently via `zoneinfo`:
    /// `python3 -c "import datetime as d,zoneinfo as z; sofia=z.ZoneInfo('Europe/Sofia'); print(int(d.datetime(2026,10,25,3,30,fold=0,tzinfo=sofia).timestamp()*1000))"`
    #[test]
    fn a_weekly_series_crosses_the_fall_back_ambiguity() {
        // Sunday 2026-10-18 03:30 Europe/Sofia (EEST, +3) == 00:30:00Z.
        let dtstart = 1_792_283_400_000;
        let s = Series {
            dtstart_ms: dtstart, dtstart_tz: "Europe/Sofia",
            duration_ms: HOUR, is_all_day: false,
            recurrence: &weekly(&["RRULE:FREQ=WEEKLY"]),
        };
        let out = expand(&s, dtstart - HOUR, dtstart + 8 * DAY, 10).unwrap().intervals;
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].start_ms, dtstart);
        assert_eq!(out[1].start_ms, dtstart + 7 * DAY);
        assert_eq!(out[1].start_ms, 1_792_888_200_000, "expected the ambiguous week's occurrence to resolve to the earlier (EEST) instance, 2026-10-25 03:30 EEST");
    }

    #[test]
    fn the_limit_caps_runaway_expansion() {
        let s = Series {
            dtstart_ms: MON_0900_SOFIA, dtstart_tz: "Europe/Sofia",
            duration_ms: 30 * 60_000, is_all_day: false,
            recurrence: &weekly(&["RRULE:FREQ=MINUTELY"]),
        };
        let out = expand(&s, MON_0900_SOFIA, MON_0900_SOFIA + DAY, 10).unwrap().intervals;
        assert_eq!(out.len(), 10);
    }

    #[test]
    fn truncation_is_reported_when_the_limit_is_hit() {
        let s = Series {
            dtstart_ms: MON_0900_SOFIA, dtstart_tz: "Europe/Sofia",
            duration_ms: 30 * 60_000, is_all_day: false,
            recurrence: &weekly(&["RRULE:FREQ=MINUTELY"]),
        };
        let out = expand(&s, MON_0900_SOFIA, MON_0900_SOFIA + DAY, 10).unwrap();
        assert_eq!(out.intervals.len(), 10);
        assert!(out.truncated, "a FREQ=MINUTELY series capped at 10 must report truncation");
    }

    #[test]
    fn truncation_is_not_reported_for_a_fully_expanded_series() {
        let s = Series {
            dtstart_ms: MON_0900_SOFIA, dtstart_tz: "Europe/Sofia",
            duration_ms: 30 * 60_000, is_all_day: false,
            recurrence: &weekly(&["RRULE:FREQ=DAILY;COUNT=2"]),
        };
        let out = expand(&s, MON_0900_SOFIA - HOUR, MON_0900_SOFIA + 30 * DAY, 50).unwrap();
        assert_eq!(out.intervals.len(), 2);
        assert!(!out.truncated, "a COUNT=2 series with room to spare under the limit must not report truncation");
    }

    #[test]
    fn a_malformed_rule_is_an_error_not_a_panic() {
        let s = Series {
            dtstart_ms: MON_0900_SOFIA, dtstart_tz: "Europe/Sofia",
            duration_ms: 30 * 60_000, is_all_day: false,
            recurrence: &weekly(&["RRULE:FREQ=NONSENSE"]),
        };
        assert!(expand(&s, MON_0900_SOFIA, MON_0900_SOFIA + DAY, 10).is_err());
    }

    #[test]
    fn an_unknown_timezone_is_an_error_not_a_panic() {
        let s = Series {
            dtstart_ms: MON_0900_SOFIA, dtstart_tz: "Mars/Olympus_Mons",
            duration_ms: 30 * 60_000, is_all_day: false,
            recurrence: &weekly(&["RRULE:FREQ=DAILY"]),
        };
        assert!(expand(&s, MON_0900_SOFIA, MON_0900_SOFIA + DAY, 10).is_err());
    }

    /// The critical bug fixed in fix round 1: a recurring all-day series
    /// must resolve to the same calendar date regardless of `dtstart_tz`'s
    /// offset, and each occurrence's instant must be deterministic (midnight
    /// local in `dtstart_tz`) rather than dependent on the host machine's
    /// system timezone (which `rrule` falls back to internally for a
    /// TZID-less `VALUE=DATE` DTSTART).
    #[test]
    fn an_all_day_series_resolves_to_local_midnight_in_a_zone_ahead_of_utc() {
        let s = Series {
            dtstart_ms: ALL_DAY_AUG3, dtstart_tz: "Europe/Sofia",
            duration_ms: DAY, is_all_day: true,
            recurrence: &weekly(&["RRULE:FREQ=DAILY;COUNT=3"]),
        };
        let out = expand(&s, ALL_DAY_AUG3 - DAY, ALL_DAY_AUG3 + 10 * DAY, 50).unwrap().intervals;
        assert_eq!(out.len(), 3);
        for (k, interval) in out.iter().enumerate() {
            let k = k as i64;
            // Sofia is EEST (+3) throughout early August 2026: no DST edge here.
            let expected_start = ALL_DAY_AUG3 + k * DAY - 3 * HOUR;
            assert_eq!(interval.start_ms, expected_start, "occurrence {k}: not local midnight in Europe/Sofia");
            assert_eq!(interval.end_ms - interval.start_ms, DAY, "occurrence {k}: duration not preserved");
        }
    }

    #[test]
    fn an_all_day_series_resolves_to_local_midnight_in_a_zone_behind_utc() {
        let s = Series {
            dtstart_ms: ALL_DAY_AUG3, dtstart_tz: "America/Los_Angeles",
            duration_ms: DAY, is_all_day: true,
            recurrence: &weekly(&["RRULE:FREQ=DAILY;COUNT=3"]),
        };
        let out = expand(&s, ALL_DAY_AUG3 - DAY, ALL_DAY_AUG3 + 10 * DAY, 50).unwrap().intervals;
        assert_eq!(out.len(), 3);
        for (k, interval) in out.iter().enumerate() {
            let k = k as i64;
            // Los Angeles is PDT (-7) throughout early August 2026.
            let expected_start = ALL_DAY_AUG3 + k * DAY + 7 * HOUR;
            assert_eq!(interval.start_ms, expected_start, "occurrence {k}: not local midnight in America/Los_Angeles");
            assert_eq!(interval.end_ms - interval.start_ms, DAY, "occurrence {k}: duration not preserved");
        }
    }

    /// Regression guard for the fix: keeping DTSTART as `VALUE=DATE` (rather
    /// than switching all-day series to a timed, TZID-bearing DTSTART) means
    /// a date-valued EXDATE — the form Google actually sends for an all-day
    /// series' cancelled instances — must still line up and exclude.
    #[test]
    fn exdate_removes_an_instance_from_an_all_day_series() {
        let s = Series {
            dtstart_ms: ALL_DAY_AUG3, dtstart_tz: "Europe/Sofia",
            duration_ms: DAY, is_all_day: true,
            recurrence: &weekly(&[
                "RRULE:FREQ=DAILY;COUNT=3",
                // Tuesday 2026-08-04, the all-day convention's date-only EXDATE form.
                "EXDATE;VALUE=DATE:20260804",
            ]),
        };
        let out = expand(&s, ALL_DAY_AUG3 - DAY, ALL_DAY_AUG3 + 10 * DAY, 50).unwrap().intervals;
        assert_eq!(out.len(), 2, "the excluded instance should not be produced");
        let excluded_start = ALL_DAY_AUG3 + DAY - 3 * HOUR; // would-be Aug 4 midnight Sofia
        assert!(out.iter().all(|i| i.start_ms != excluded_start));
    }
}
