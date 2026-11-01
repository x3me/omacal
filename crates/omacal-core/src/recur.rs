use crate::layout::Interval;
use crate::windows_zone::iana_for_windows;
use crate::zone::{date_in_zone, midnight_in_zone};
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
/// An all-day series takes RFC 5545's `VALUE=DATE` form, which carries no
/// TZID — but the date it carries is still read **in the series' own zone**,
/// because that is where the instant came from: the store holds midnight in
/// the *calendar's* zone for an all-day event, and both resolvers put it
/// there (`omacal_sync::convert` for Google's bare `date`, `omacal_caldav`'s
/// `resolve` for `DTSTART;VALUE=DATE`). [`crate::zone`] says the same thing,
/// and reading it in any other zone is a day out on one side of midnight or
/// the other.
///
/// **Reading it in UTC was issue #44.** Every zone ahead of UTC puts local
/// midnight on the *previous* UTC day — Wednesday 12 August in Brisbane is
/// 14:00 Tuesday 11 August UTC — so the whole series expanded from the wrong
/// anchor and every occurrence landed a day early. Zones behind UTC were
/// unaffected, which is why the report named positive offsets, and why this
/// survived: the fixtures that covered it were built from midnight UTC, an
/// instant the store never actually holds.
///
/// It is [`crate::zone::date_in_zone`] rather than a second derivation for
/// the reason that module exists: this project has twice nearly shipped a
/// second date formatter, and the copy that got written here is what was
/// wrong. Re-anchoring on the way back out is `midnight_in_zone`'s job —
/// `rrule` parses a TZID-less `VALUE=DATE` DTSTART against `Tz::LOCAL`, the
/// *host machine's* zone, so the occurrences it hands back have to be pinned
/// to something host-independent before they mean anything.
fn dtstart_line(series: &Series, zone: chrono_tz::Tz) -> Result<String, RecurError> {
    if series.is_all_day {
        let date = date_in_zone(series.dtstart_ms, series.dtstart_tz);
        return Ok(format!("DTSTART;VALUE=DATE:{}", date.replace('-', "")));
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

/// `line` with a Windows `TZID` (Outlook's `EXDATE;TZID=India Standard
/// Time:…`, #158) swapped for its IANA zone, the only names `rrule` knows.
/// The stored line stays as the server wrote it; this is the expander's copy.
fn with_iana_tzid(line: &str) -> std::borrow::Cow<'_, str> {
    let Some(at) = line.find("TZID=") else { return line.into() };
    let start = at + "TZID=".len();
    let quoted = line[start..].starts_with('"');
    let name_start = start + usize::from(quoted);
    let len = line[name_start..].find(|c| if quoted { c == '"' } else { c == ';' || c == ':' });
    let Some(len) = len else { return line.into() };
    let name_end = name_start + len;
    match iana_for_windows(&line[name_start..name_end]) {
        Some(iana) => format!("{}{iana}{}", &line[..start], &line[name_end + usize::from(quoted)..]).into(),
        None => line.into(),
    }
}

const DAY_MS: i64 = 24 * 3_600_000;

/// Whether every rule in `recurrence` has finished before `from_ms`, so
/// expanding it cannot produce an occurrence in the window.
///
/// False — expand as usual — when any line is an `RDATE` (it can add a date
/// the rule itself has passed), when any rule has no `UNTIL` and no `COUNT`,
/// or when a bound cannot be read. The bound overestimates on purpose: a
/// series this skips has no occurrence left, and one it fails to skip only
/// costs the expansion it would have paid anyway.
fn rule_ended_before(recurrence: &[String], dtstart_ms: i64, from_ms: i64, duration_ms: i64) -> bool {
    if recurrence.iter().any(|line| {
        let t = line.trim();
        t.len() >= 5 && t[..5].eq_ignore_ascii_case("RDATE")
    }) {
        return false;
    }
    let horizon = from_ms.saturating_sub(duration_ms.max(0));
    let mut saw_rule = false;
    for line in recurrence {
        let upper = line.to_ascii_uppercase();
        if !upper.contains("RRULE") {
            continue;
        }
        saw_rule = true;
        let has_until = upper.contains("UNTIL=");
        let has_count = upper.contains("COUNT=");
        if !has_until && !has_count {
            return false;
        }
        // Two days of slack: a floating or date-only UNTIL is a local day,
        // and the last zone on earth ends it well after UTC midnight.
        let until_ended = match until_ms(&upper) {
            Some(until) => until.saturating_add(2 * DAY_MS) < horizon,
            None => false,
        };
        let count_ended = match count_end_ms(&upper, dtstart_ms) {
            Some(end) => end < horizon,
            None => false,
        };
        let ended = (!has_until || until_ended) && (!has_count || count_ended);
        if !ended {
            return false;
        }
    }
    saw_rule
}

fn param<'a>(upper: &'a str, key: &str) -> Option<&'a str> {
    let rest = upper.split(key).nth(1)?;
    Some(rest.split([';', ':']).next()?.trim())
}

fn until_ms(upper: &str) -> Option<i64> {
    let token = param(upper, "UNTIL=")?;
    if token.len() < 8 {
        return None;
    }
    let y: i16 = token[0..4].parse().ok()?;
    let m: i8 = token[4..6].parse().ok()?;
    let d: i8 = token[6..8].parse().ok()?;
    let date = jiff::civil::Date::new(y, m, d).ok()?;
    let (hh, mm, ss) = if token.len() >= 15 && token.as_bytes().get(8) == Some(&b'T') {
        (token[9..11].parse().ok()?, token[11..13].parse().ok()?, token[13..15].parse().ok()?)
    } else {
        (0, 0, 0)
    };
    date.at(hh, mm, ss, 0).to_zoned(jiff::tz::TimeZone::UTC).ok().map(|z| z.timestamp().as_millisecond())
}

/// A generous last instant for a `COUNT` rule: `(count + 1)` periods, so a
/// `BY*` shift cannot hide an occurrence that is still inside the window.
fn count_end_ms(upper: &str, dtstart_ms: i64) -> Option<i64> {
    let count = param(upper, "COUNT=")?.parse::<i64>().ok()?;
    let freq = param(upper, "FREQ=")?;
    let interval = param(upper, "INTERVAL=").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1).max(1);
    let period = match freq {
        "YEARLY" => 366 * DAY_MS,
        "MONTHLY" => 32 * DAY_MS,
        "WEEKLY" => 8 * DAY_MS,
        "DAILY" => 2 * DAY_MS,
        "HOURLY" => 2 * 3_600_000,
        "MINUTELY" => 2 * 60_000,
        "SECONDLY" => 2_000,
        _ => return None,
    };
    Some(dtstart_ms.saturating_add((count + 1).saturating_mul(interval).saturating_mul(period)))
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

    // A long history keeps every series that ever occurred. Expanding each of
    // them on a view of today is the cost that grows with the span, and a
    // rule that has already ended cannot overlap the window. Open-ended
    // series still expand — their own dates never say they stopped — and an
    // RDATE can land after UNTIL, so that line is never skipped.
    if rule_ended_before(series.recurrence, series.dtstart_ms, from_ms, series.duration_ms) {
        return Ok(Expansion { intervals: Vec::new(), truncated: false });
    }

    let mut source = String::with_capacity(128);
    source.push_str(&dtstart);
    for line in series.recurrence {
        source.push('\n');
        source.push_str(&with_iana_tzid(line.trim()));
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

    /// Monday 2026-08-03, all-day, as the **store** holds it: midnight in the
    /// calendar's own zone, not midnight UTC. One constant per zone, because
    /// that is the whole point — the same calendar date is a different
    /// instant in each, and a fixture that used one instant for both was
    /// asserting a convention no sync ever writes (issue #44). Both derived
    /// from the independently-verified `MON_0900_SOFIA` rather than a fresh
    /// literal: Sofia is EEST (+3) and Los Angeles PDT (-7) throughout early
    /// August 2026, so neither has a DST edge here.
    const ALL_DAY_AUG3_SOFIA: i64 = MON_0900_SOFIA - 9 * HOUR;
    const ALL_DAY_AUG3_LA: i64 = MON_0900_SOFIA + HOUR;

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
            dtstart_ms: ALL_DAY_AUG3_SOFIA, dtstart_tz: "Europe/Sofia",
            duration_ms: DAY, is_all_day: true,
            recurrence: &weekly(&["RRULE:FREQ=DAILY;COUNT=3"]),
        };
        let out = expand(&s, ALL_DAY_AUG3_SOFIA - DAY, ALL_DAY_AUG3_SOFIA + 10 * DAY, 50).unwrap().intervals;
        assert_eq!(out.len(), 3);
        for (k, interval) in out.iter().enumerate() {
            let k = k as i64;
            // Each occurrence is the same wall-clock midnight, a day further on.
            let expected_start = ALL_DAY_AUG3_SOFIA + k * DAY;
            assert_eq!(interval.start_ms, expected_start, "occurrence {k}: not local midnight in Europe/Sofia");
            assert_eq!(interval.end_ms - interval.start_ms, DAY, "occurrence {k}: duration not preserved");
        }
    }

    #[test]
    fn an_all_day_series_resolves_to_local_midnight_in_a_zone_behind_utc() {
        let s = Series {
            dtstart_ms: ALL_DAY_AUG3_LA, dtstart_tz: "America/Los_Angeles",
            duration_ms: DAY, is_all_day: true,
            recurrence: &weekly(&["RRULE:FREQ=DAILY;COUNT=3"]),
        };
        let out = expand(&s, ALL_DAY_AUG3_LA - DAY, ALL_DAY_AUG3_LA + 10 * DAY, 50).unwrap().intervals;
        assert_eq!(out.len(), 3);
        for (k, interval) in out.iter().enumerate() {
            let k = k as i64;
            let expected_start = ALL_DAY_AUG3_LA + k * DAY;
            assert_eq!(interval.start_ms, expected_start, "occurrence {k}: not local midnight in America/Los_Angeles");
            assert_eq!(interval.end_ms - interval.start_ms, DAY, "occurrence {k}: duration not preserved");
        }
    }

    /// Issue #44, reproduced from the reporter's own data rather than a
    /// simplification of it: a fortnightly all-day event in Brisbane
    /// (UTC+10, no DST), `DTSTART;VALUE=DATE:20260812` with
    /// `RRULE:FREQ=WEEKLY;INTERVAL=2`, whose September occurrence he expected
    /// on Wednesday the 9th and OmaCal showed on Tuesday the 8th.
    ///
    /// The master is right and the expansion was wrong: midnight on 12 August
    /// in Brisbane is 14:00 on 11 August UTC, so reading the anchor date in
    /// UTC started the series on Tuesday and every fortnight after it landed
    /// a Tuesday. Asserted on the *dates*, in the calendar's zone, because
    /// that is the language the bug was reported in.
    #[test]
    fn a_fortnightly_all_day_series_keeps_its_weekday_in_a_zone_ahead_of_utc() {
        // 2026-08-12 00:00 Australia/Brisbane, the instant the reporter's
        // stored master actually holds.
        const AUG12_BRISBANE: i64 = 1_786_456_800_000;
        let s = Series {
            dtstart_ms: AUG12_BRISBANE, dtstart_tz: "Australia/Brisbane",
            duration_ms: DAY, is_all_day: true,
            recurrence: &weekly(&["RRULE:FREQ=WEEKLY;INTERVAL=2"]),
        };
        let out = expand(&s, AUG12_BRISBANE - DAY, AUG12_BRISBANE + 30 * DAY, 50).unwrap().intervals;
        let dates: Vec<String> = out
            .iter()
            .map(|i| crate::zone::date_in_zone(i.start_ms, "Australia/Brisbane"))
            .collect();
        assert_eq!(dates, ["2026-08-12", "2026-08-26", "2026-09-09"]);
    }

    /// Regression guard for the fix: keeping DTSTART as `VALUE=DATE` (rather
    /// than switching all-day series to a timed, TZID-bearing DTSTART) means
    /// a date-valued EXDATE — the form Google actually sends for an all-day
    /// series' cancelled instances — must still line up and exclude.
    #[test]
    fn exdate_removes_an_instance_from_an_all_day_series() {
        let s = Series {
            dtstart_ms: ALL_DAY_AUG3_SOFIA, dtstart_tz: "Europe/Sofia",
            duration_ms: DAY, is_all_day: true,
            recurrence: &weekly(&[
                "RRULE:FREQ=DAILY;COUNT=3",
                // Tuesday 2026-08-04, the all-day convention's date-only EXDATE form.
                "EXDATE;VALUE=DATE:20260804",
            ]),
        };
        let out = expand(&s, ALL_DAY_AUG3_SOFIA - DAY, ALL_DAY_AUG3_SOFIA + 10 * DAY, 50).unwrap().intervals;
        assert_eq!(out.len(), 2, "the excluded instance should not be produced");
        let excluded_start = ALL_DAY_AUG3_SOFIA + DAY; // Aug 4 midnight Sofia
        assert!(out.iter().all(|i| i.start_ms != excluded_start));
    }

    /// Outlook writes Windows zone names into `EXDATE;TZID=…` too (#158).
    /// Stored verbatim, they reached `rrule`, which knows only IANA names.
    #[test]
    fn an_exdate_in_a_windows_zone_is_excluded() {
        const MON_OCT12_1230Z: i64 = 1_791_808_200_000; // 18:00 India Standard Time
        let recurrence = vec![
            "RRULE:FREQ=WEEKLY;BYDAY=MO".to_string(),
            "EXDATE;TZID=India Standard Time:20261019T180000".to_string(),
        ];
        let series = Series {
            dtstart_ms: MON_OCT12_1230Z,
            dtstart_tz: "Asia/Kolkata",
            duration_ms: HOUR / 2,
            is_all_day: false,
            recurrence: &recurrence,
        };
        let out = expand(&series, MON_OCT12_1230Z - DAY, MON_OCT12_1230Z + 15 * DAY, 64).unwrap().intervals;
        let starts: Vec<i64> = out.iter().map(|i| i.start_ms).collect();
        assert_eq!(starts, vec![MON_OCT12_1230Z, MON_OCT12_1230Z + 14 * DAY]);
    }

    #[test]
    fn only_a_windows_tzid_is_rewritten() {
        assert_eq!(with_iana_tzid("EXDATE;TZID=India Standard Time:20261019T180000"), "EXDATE;TZID=Asia/Kolkata:20261019T180000");
        assert_eq!(with_iana_tzid("EXDATE;TZID=\"India Standard Time\":20261019T180000"), "EXDATE;TZID=Asia/Kolkata:20261019T180000");
        assert_eq!(
            with_iana_tzid("RDATE;TZID=W. Europe Standard Time;VALUE=DATE-TIME:20261019T180000"),
            "RDATE;TZID=Europe/Berlin;VALUE=DATE-TIME:20261019T180000"
        );
        for kept in ["RRULE:FREQ=WEEKLY;BYDAY=MO", "EXDATE;TZID=Europe/Sofia:20260804T090000", "EXDATE;TZID=Customized Time Zone:20261019T180000"] {
            assert_eq!(with_iana_tzid(kept), kept);
        }
    }

    /// A long history keeps series that ended years ago. A view of today must
    /// not expand them: the rule cannot produce an occurrence in the window.
    #[test]
    fn a_series_that_ended_before_the_window_expands_to_nothing() {
        let day = 24 * 3_600_000;
        // 1990-01-01T00:00:00Z.
        let start = 631_152_000_000;
        let until = Series {
            dtstart_ms: start,
            dtstart_tz: "UTC",
            duration_ms: 3_600_000,
            is_all_day: false,
            recurrence: &weekly(&["RRULE:FREQ=DAILY;UNTIL=19901231T000000Z"]),
        };
        let now = 1_786_341_600_000; // 2026
        let out = expand(&until, now, now + 7 * day, 512).unwrap();
        assert!(out.intervals.is_empty());
        assert!(!out.truncated, "an ended series is finished, not cut short");

        let count = Series {
            dtstart_ms: start,
            dtstart_tz: "UTC",
            duration_ms: 3_600_000,
            is_all_day: false,
            recurrence: &weekly(&["RRULE:FREQ=DAILY;COUNT=10"]),
        };
        assert!(expand(&count, now, now + 7 * day, 512).unwrap().intervals.is_empty());

        // The same COUNT, asked while it is still running, is unaffected.
        let during = expand(&count, start, start + 5 * day, 50).unwrap();
        assert!(!during.intervals.is_empty());
        assert_eq!(during.intervals.len(), 5);
    }

    /// An RDATE can sit after UNTIL. Skipping on the rule alone would drop it.
    #[test]
    fn an_rdate_after_until_is_not_treated_as_ended() {
        let lines = weekly(&[
            "RRULE:FREQ=DAILY;UNTIL=19900102T000000Z",
            "RDATE:20260810T090000Z",
        ]);
        assert!(
            !rule_ended_before(&lines, 631_152_000_000, 1_786_341_600_000, 3_600_000),
            "an RDATE is another way for the series to occur"
        );
    }
}
