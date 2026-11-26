use sqlx::{Sqlite, Transaction};

const HISTORY_CUTOFF_KEY: &str = "history_pruned_before_ms";
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

/// Store a span and prune its cached rows in the same write transaction.
pub async fn set_span(pool: &sqlx::SqlitePool, months: i64, now_ms: i64) -> sqlx::Result<()> {
    if !HISTORY_SPANS.contains(&months) {
        return Err(sqlx::Error::InvalidArgument("unsupported history span".into()));
    }
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let saved: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
        .bind(HISTORY_SPAN_KEY).fetch_optional(&mut *tx).await?;
    let previous = saved.and_then(|v| v.parse().ok()).filter(|v| HISTORY_SPANS.contains(v))
        .unwrap_or(DEFAULT_HISTORY_MONTHS);
    sqlx::query("INSERT OR REPLACE INTO settings (key,value) VALUES (?1,?2)")
        .bind(HISTORY_SPAN_KEY).bind(months.to_string()).execute(&mut *tx).await?;
    let start = history_window(now_ms, months).0;
    if start > history_window(now_ms, previous).0 {
        sqlx::query("INSERT OR REPLACE INTO settings (key,value) VALUES (?1,?2)")
            .bind(HISTORY_CUTOFF_KEY).bind(start.to_string()).execute(&mut *tx).await?;
        forget_events_ending_before(&mut tx, None, start).await?;
        sqlx::query("UPDATE sync_state SET window_start = ?1 WHERE window_start < ?1")
            .bind(start).execute(&mut *tx).await?;
    } else if months != previous {
        // Let the next wider fetch restore newly requested history. An install
        // that has never pruned has no cutoff to move.
        sqlx::query("UPDATE settings SET value = min(CAST(value AS INTEGER), ?2) WHERE key = ?1")
            .bind(HISTORY_CUTOFF_KEY).bind(start).execute(&mut *tx).await?;
    }
    tx.commit().await
}

/// Common coverage of every visible event calendar. A feed is complete once
/// fetched; a local calendar requires no fetch. A missing remote cursor
/// means we cannot yet call any day empty across all the calendars.
pub async fn displayed_window(pool: &sqlx::SqlitePool, requested: (i64, i64)) -> sqlx::Result<(i64, i64)> {
    let rows: Vec<(Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT s.window_start, s.window_end
           FROM calendars c JOIN accounts a ON a.id = c.account_id
           LEFT JOIN sync_state s ON s.calendar_id = c.id
          WHERE c.selected = 1 AND c.supports_events = 1
            AND (a.provider IN ('google', 'caldav') OR (a.provider = 'webcal' AND s.calendar_id IS NULL))",
    ).fetch_all(pool).await?;
    let mut covered = requested;
    for (start, end) in rows {
        let (Some(start), Some(end)) = (start, end) else { return Ok((0, 0)); };
        covered.0 = covered.0.max(start);
        covered.1 = covered.1.min(end);
    }
    Ok(covered)
}

/// Used with an exception row aliased as `e`. Cancelled Google instances
/// have zero duration, but still suppress the original multi-day occurrence.
pub(crate) const ORIGINAL_SLOT_END: &str = "COALESCE((
    SELECT CASE WHEN m.is_all_day THEN all_day_slot_end(m.start_utc, m.end_utc, m.start_tz, e.original_start_utc)
                ELSE e.original_start_utc + max(0, m.end_utc - m.start_utc) END
      FROM events m
     WHERE m.calendar_id = e.calendar_id AND m.google_id = e.recurring_event_id
), e.original_start_utc)";

/// One-off rows, and exceptions whose occurrence is also outside the window,
/// that end at or before `window_start_ms`. Series masters stay: their own
/// dates say nothing about later occurrences, and a shorter span must not
/// throw a series away. An exception whose original slot is still inside the
/// window stays too — it is what stops the master expanding a ghost there.
pub async fn forget_events_ending_before(
    tx: &mut Transaction<'_, Sqlite>,
    calendar_id: Option<i64>,
    window_start_ms: i64,
) -> Result<(), sqlx::Error> {
    // Keep the keys on this connection until both deletes finish. The event
    // delete trigger writes to event_changes, so its cleanup must run second.
    sqlx::query("DROP TABLE IF EXISTS temp.history_forgotten").execute(&mut **tx).await?;
    sqlx::query(&format!(
        "CREATE TEMP TABLE history_forgotten AS
         SELECT calendar_id, google_id FROM events AS e
          WHERE (?2 IS NULL OR calendar_id = ?2)
            AND calendar_id IN (
                SELECT c.id FROM calendars c JOIN accounts a ON a.id = c.account_id
                 WHERE a.provider IN ('google', 'caldav'))
            AND recurrence IS NULL
            AND end_utc <= ?1
            AND (recurring_event_id IS NULL OR original_start_utc IS NULL
                 OR (original_start_utc < ?1 AND {ORIGINAL_SLOT_END} <= ?1))",
    ))
    .bind(window_start_ms)
    .bind(calendar_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query("DELETE FROM events WHERE (calendar_id, google_id) IN
                 (SELECT calendar_id, google_id FROM history_forgotten)")
        .execute(&mut **tx).await?;
    sqlx::query("DELETE FROM event_changes WHERE (calendar_id, gid) IN
                 (SELECT calendar_id, google_id FROM history_forgotten)")
        .execute(&mut **tx).await?;
    sqlx::query("DROP TABLE temp.history_forgotten").execute(&mut **tx).await?;
    Ok(())
}

/// A shrink can commit while a provider is fetching the old, wider span.
/// Recheck the saved choice inside the transaction that applies that response,
/// then forget those rows again and record only the history we actually keep.
/// The write lock also makes the opposite ordering safe: a later settings
/// transaction performs its own deletion after this one has committed.
pub async fn retain_current_history(
    tx: &mut Transaction<'_, Sqlite>,
    calendar_id: i64,
    fetched_start: i64,
    wrote_events: bool,
) -> Result<i64, sqlx::Error> {
    // This is the cutoff the settings transaction actually used, including
    // for calendars whose first sync had no sync_state row at shrink time.
    let cutoff: Option<i64> = sqlx::query_scalar(
        "SELECT CAST(value AS INTEGER) FROM settings WHERE key = ?1",
    ).bind(HISTORY_CUTOFF_KEY).fetch_optional(&mut **tx).await?;
    if let Some(cutoff) = cutoff.filter(|_| wrote_events) {
        // A narrow REPORT may still contain an entire recurring resource.
        // Reapply only the explicit shrink cutoff, never today's sliding start.
        forget_events_ending_before(tx, Some(calendar_id), cutoff).await?;
    }
    Ok(cutoff.unwrap_or(fetched_start).max(fetched_start))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn retrying_a_failed_prune_recomputes_the_forgotten_keys() {
        let pool = crate::connect_memory().await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO accounts (google_sub,email,created_at) VALUES ('s','e',0);
                     INSERT INTO calendars (account_id,google_id,summary,timezone,access_role)
                     VALUES (1,'c','C','UTC','owner');
                     INSERT INTO events (calendar_id,google_id,start_utc,end_utc,start_tz,end_tz,updated_at)
                     VALUES (1,'old',1,2,'UTC','UTC',0), (1,'keep',4,5,'UTC','UTC',0);
                     CREATE TEMP TRIGGER interrupt_prune BEFORE DELETE ON events
                     BEGIN SELECT RAISE(ABORT, 'interrupted prune'); END;")
            .execute(&mut *tx).await.unwrap();

        let error = forget_events_ending_before(&mut tx, Some(1), 6).await.unwrap_err();
        assert!(error.to_string().contains("interrupted prune"));
        sqlx::query("DROP TRIGGER temp.interrupt_prune").execute(&mut *tx).await.unwrap();
        forget_events_ending_before(&mut tx, Some(1), 3).await.unwrap();
        tx.commit().await.unwrap();
        let remaining: Vec<String> = sqlx::query_scalar("SELECT google_id FROM events")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(remaining, ["keep"]);
    }
}
