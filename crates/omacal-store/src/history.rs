use sqlx::SqliteConnection;

pub const HISTORY_SPAN_KEY: &str = "history_span_months";
/// How far back sync keeps events, in months. `0` is everything (back to
/// 1900). Ahead is a year for every one of them. A fresh install reads an
/// absent row as [`DEFAULT_HISTORY_MONTHS`]; migration 0020 writes
/// [`SPAN_6_MONTHS`] where a calendar already exists, so an update does not
/// lengthen a calendar that was syncing six months.
pub const HISTORY_SPANS: [i64; 4] = [6, 24, 60, 0];
pub const DEFAULT_HISTORY_MONTHS: i64 = 24;
/// Six months: 180 days, the window sync kept before this setting existed.
pub const SPAN_6_MONTHS: i64 = 6;

const DAY_MS: i64 = 24 * 3_600_000;
/// 1900-01-01T00:00:00Z. Everything reaches back to here and no further;
/// the forward edge stays a year out and still slides.
const EVERYTHING_START_MS: i64 = -2_208_988_800_000;

/// The window sync fetches for `months`, and the one the year views hatch
/// outside of. Six months is 180 days — the window this app already kept —
/// and a year is 365 days, the same length the window looks ahead. Anything
/// that is not an offered span reads as the two-year default.
pub fn history_window(now_ms: i64, months: i64) -> (i64, i64) {
    let ahead = now_ms + 365 * DAY_MS;
    let back_days: i64 = match months {
        6 => 180,
        24 => 2 * 365,
        60 => 5 * 365,
        0 => return (EVERYTHING_START_MS, ahead),
        _ => 2 * 365,
    };
    (now_ms - back_days * DAY_MS, ahead)
}

/// One-off rows, and exceptions whose occurrence is also outside the window,
/// that end at or before `window_start_ms`. Series masters stay: their own
/// dates say nothing about later occurrences, and a shorter span must not
/// throw a series away. An exception whose original slot is still inside the
/// window stays too — it is what stops the master expanding a ghost there.
pub async fn forget_events_ending_before(
    conn: &mut SqliteConnection,
    calendar_id: Option<i64>,
    window_start_ms: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM events
          WHERE (?2 IS NULL OR calendar_id = ?2)
            AND recurrence IS NULL
            AND end_utc <= ?1
            AND (recurring_event_id IS NULL
                 OR original_start_utc IS NULL
                 OR original_start_utc < ?1)",
    )
    .bind(window_start_ms)
    .bind(calendar_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// A shrink can commit while a provider is fetching the old, wider span.
/// Recheck the saved choice inside the transaction that applies that response,
/// then forget those rows again and record only the history we actually keep.
/// The write lock also makes the opposite ordering safe: a later settings
/// transaction performs its own deletion after this one has committed.
pub async fn retain_current_history(
    conn: &mut SqliteConnection,
    calendar_id: i64,
    fetched_start: i64,
    now_ms: i64,
) -> Result<i64, sqlx::Error> {
    let saved: Option<String> = sqlx::query_scalar(
        "SELECT value FROM settings WHERE key = ?1"
    ).bind(HISTORY_SPAN_KEY).fetch_optional(&mut *conn).await?;
    // No saved choice means no shrink has happened. In particular, callers
    // asking for a specific event's dates keep their original window.
    let Some(saved) = saved else { return Ok(fetched_start); };
    let months = saved.parse().ok().filter(|m| HISTORY_SPANS.contains(m))
        .unwrap_or(DEFAULT_HISTORY_MONTHS);
    let start = history_window(now_ms, months).0.max(fetched_start);
    if start > fetched_start {
        forget_events_ending_before(conn, Some(calendar_id), start).await?;
    }
    Ok(start)
}
