//! All-day dates and boundaries use the same timezone database as provider
//! storage. The recurrence iterator uses chrono dates as civil coordinates.

use chrono::Datelike;
use jiff::{civil::Date, tz::{AmbiguousOffset, TimeZone}, Timestamp};

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

/// Read provider instants using the same timezone rules used to store them.
pub(crate) fn civil_date(ms: i64, zone: &TimeZone) -> Option<chrono::NaiveDate> {
    let d = zone.to_datetime(Timestamp::from_millisecond(ms).ok()?).date();
    chrono::NaiveDate::from_ymd_opt(i32::from(d.year()), d.month() as u32, d.day() as u32)
}

fn provider_date(date: chrono::NaiveDate) -> Option<Date> {
    Date::new(date.year().try_into().ok()?, date.month() as i8, date.day() as i8).ok()
}

/// Earliest instant of a civil day. When midnight is missing, the transition
/// ends the gap. It can lead directly to tomorrow after a dateline change.
pub(crate) fn day_start(date: chrono::NaiveDate, zone: &TimeZone) -> Option<i64> {
    let midnight = provider_date(date)?.at(0, 0, 0, 0);
    let mapping = zone.to_ambiguous_timestamp(midnight);
    let start = match mapping.offset() {
        AmbiguousOffset::Gap { .. } => zone.following(mapping.earlier().ok()?).next()?.timestamp(),
        _ => mapping.earlier().ok()?,
    };
    Some(start.as_millisecond())
}

pub fn midnight_in_zone(date: chrono::NaiveDate, zone: chrono_tz::Tz) -> i64 {
    TimeZone::get(zone.name()).ok().and_then(|tz| day_start(date, &tz))
        .unwrap_or_else(|| date.and_time(chrono::NaiveTime::MIN).and_utc().timestamp_millis())
}

/// A DATE recurrence ID uses the provider's compatible gap disambiguation.
/// It can fall later than the first valid time on the same calendar day.
pub(crate) fn stored_slot_start(ms: i64, zone: &TimeZone) -> Option<i64> {
    let date = civil_date(ms, zone)?;
    let start = day_start(date, zone)?;
    if start == ms { return Some(start); }
    let provider_id = provider_date(date)?.to_zoned(zone.clone()).ok()?.timestamp().as_millisecond();
    (provider_id == ms).then_some(start)
}

pub fn all_day_slot_start(ms: i64, tz: &str) -> Option<i64> {
    stored_slot_start(ms, &TimeZone::get(tz).ok()?)
}

/// Start of the calendar day containing a stored or expanded instant.
pub fn midnight_of_instant_in_zone(ms: i64, tz: &str) -> i64 {
    let zone = TimeZone::get(tz).unwrap_or(TimeZone::UTC);
    civil_date(ms, &zone).and_then(|date| day_start(date, &zone)).unwrap_or(ms)
}
