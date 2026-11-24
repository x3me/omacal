//! The two zone derivations, and their inverses of each other.
//!
//! [`date_in_zone`] answers "what calendar date is this instant, read in this
//! zone"; [`midnight_in_zone`] answers "what instant is midnight on this date,
//! in this zone". Round-tripping an all-day event through both returns the
//! instant it started with, and that round trip is the whole all-day story on
//! this project — the store holds midnight in the *calendar's* zone for an
//! all-day event, so both halves must be read in that same zone or the answer
//! is a day out on one side of midnight or the other.
//!
//! They live together, in the crate below every caller, for one reason: this
//! project has twice nearly shipped a second date formatter, and two
//! derivations of this once disagreed silently. `date_in_zone` was in
//! `src-tauri/src/write.rs` and `midnight_in_zone` was private to
//! [`crate::recur`]; neither was reachable from the other, which is how a third
//! copy would have got written. `write.rs` re-exports this one rather than
//! keeping its own.
//!
//! Two date libraries, deliberately: `date_in_zone` moved here verbatim in
//! `jiff`, because every caller of it is pinned by tests that a
//! reimplementation could shift; `midnight_in_zone` moved here verbatim in
//! `chrono`, because it is handed a `chrono::NaiveDate` straight out of
//! `rrule`. Rewriting either to match the other would be exactly the silent
//! re-derivation this module exists to prevent. They meet at a `yyyy-mm-dd`
//! string, which is the same interchange `write.rs` already used.

use chrono::{LocalResult, TimeZone};

/// The calendar date `ms` falls on, read in `tz`, as `yyyy-mm-dd`.
///
/// How an all-day event's *date* is recovered from the instant the store holds
/// for it — and lossless for exactly one reason: the store holds midnight in
/// the **calendar's** zone, because Google sends a bare `date` and
/// `omacal_sync::resolve` falls back to `calendars.timezone`. Read back in that
/// same zone it returns the date sync put in. Read in any other zone it is a
/// day out on one side of midnight or the other, which is the defect Plan 6
/// exists to close — so no caller may hand it the browser's zone.
///
/// An unresolvable timestamp or zone still produces a date rather than
/// panicking: the Unix epoch, then UTC — the same fallback philosophy used
/// elsewhere for zone handling (`n_day_boundaries`, `local_midnight_ms` in
/// `commands.rs`).
pub fn date_in_zone(ms: i64, tz: &str) -> String {
    let ts = jiff::Timestamp::from_millisecond(ms).unwrap_or(jiff::Timestamp::UNIX_EPOCH);
    ts.in_tz(tz)
        .unwrap_or_else(|_| ts.in_tz("UTC").expect("UTC always resolves"))
        .date()
        .to_string()
}

/// Resolves a calendar date to midnight local time in `zone`, deterministically
/// and without ever consulting the host machine's system timezone.
///
/// `rrule` parses a bare (TZID-less) `VALUE=DATE` DTSTART against `Tz::LOCAL`
/// — the *host's* system zone — so the `DateTime<rrule::Tz>` it hands back
/// for each all-day occurrence carries a host-dependent instant. We sidestep
/// that by reading back only the *calendar date* it computed
/// (`DateTime::date_naive`, which reflects the wall-clock fields as
/// constructed and is therefore the same regardless of which offset
/// `Tz::LOCAL` happened to resolve to) and re-anchoring that date here,
/// explicitly, in the series' real zone.
pub fn midnight_in_zone(date: chrono::NaiveDate, zone: chrono_tz::Tz) -> i64 {
    let midnight = date
        .and_hms_opt(0, 0, 0)
        .expect("00:00:00 is always a valid NaiveTime");
    match zone.from_local_datetime(&midnight) {
        LocalResult::Single(dt) => dt.timestamp_millis(),
        // A fall-back transition spanning midnight: take the earlier
        // instant, matching the convention `rrule` itself uses (see
        // `add_time_to_date` in the `rrule` crate).
        LocalResult::Ambiguous(earlier, _later) => earlier.timestamp_millis(),
        // A spring-forward transition spanning midnight (rare, but not
        // impossible historically): advance to the first valid local
        // instant that day instead of panicking or unwrapping.
        LocalResult::None => (1..24 * 60)
            .find_map(
                |m: i64| match zone.from_local_datetime(&(midnight + chrono::Duration::minutes(m))) {
                    LocalResult::Single(dt) => Some(dt.timestamp_millis()),
                    _ => None,
                },
            )
            // Total fallback so this can never panic. Unreachable for any
            // real IANA zone — no calendar date is entirely invalid.
            .unwrap_or_else(|| chrono::Utc.from_utc_datetime(&midnight).timestamp_millis()),
    }
}

/// Midnight on the calendar date `ms` falls on, both read in `tz`.
///
/// The round trip of the two functions above, and the only thing that should
/// ever compose them: given any instant during an all-day occurrence — the one
/// the store holds, or one an expansion produced — it answers the instant that
/// occurrence *starts* at, in the calendar's own zone.
///
/// Idempotent on a well-formed input, and that is not a reason to skip it. The
/// store already holds midnight-in-the-calendar's-zone for an all-day event, so
/// for those this returns what it was given; what it is for is every input that
/// is *not* that, and the one that matters is a daylight-saving transition
/// moving an expanded occurrence off midnight by an hour. Anchoring on the date
/// rather than the instant is what makes a reminder "30 minutes before the
/// day begins" land at 23:30 the evening before, every time, in every zone.
///
/// An unparseable date cannot come out of [`date_in_zone`] — it answers a real
/// date for every input — and an unknown zone falls back to UTC, matching
/// [`date_in_zone`]'s own fallback rather than inventing a second policy.
pub fn midnight_of_instant_in_zone(ms: i64, tz: &str) -> i64 {
    let date: chrono::NaiveDate = match date_in_zone(ms, tz).parse() {
        Ok(d) => d,
        // Unreachable via `date_in_zone`; returning the instant unchanged beats
        // panicking on a path that fires notifications.
        Err(_) => return ms,
    };
    let zone: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::UTC);
    midnight_in_zone(date, zone)
}
