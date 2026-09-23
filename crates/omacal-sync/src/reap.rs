//! Inferred deletion: removing the stored events a fetch did not name.
//!
//! All three providers do it, and they differ in exactly one thing — how much
//! of the calendar the fetch was entitled to speak for — which is [`Reach`].
//! The mechanics are shared so that a fix to them (the variable ceiling below
//! was one) lands on every path at once.

use sqlx::SqliteConnection;

/// How much of a calendar one fetch spoke for, and so how much of it may be
/// deleted for not appearing in the fetch. **Never widen one to match
/// another**: each is exactly what its fetch asked about, and a row outside
/// it was simply not examined.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Reach {
    /// The whole calendar. A WebCal feed arrives whole on every fetch, so
    /// anything it does not name is gone.
    Whole,
    /// CalDAV's time-range REPORT: rows overlapping `start..end`, and every
    /// series (a master's own dates say nothing about its occurrences). Runs
    /// on every successful sync, so it must not reach past what the REPORT
    /// asked for.
    Overlapping { start_ms: i64, end_ms: i64 },
    /// Google's full resync: rows ending at or after `start_ms`, and every
    /// series. No upper bound because it runs only after a *full* fetch of
    /// the window; rows ending before it were never asked about.
    EndingFrom { start_ms: i64 },
}

/// Deletes this calendar's events inside `reach` whose `google_id` is not in
/// `named`, and returns how many went.
///
/// **Through a temp table, not `NOT IN (?, ?, …)`.** SQLite refuses a
/// statement with more than 32,766 variables, so binding one id per
/// placeholder failed the whole sync of a large enough calendar. The table is
/// per connection, so two calendars syncing on two connections cannot see each
/// other's ids, and it is emptied before and after use.
///
/// A slice rather than any iterator: an iterator borrowed across the awaits
/// below stops the sync future from being `Send`.
pub(crate) async fn delete_unnamed<S: AsRef<str> + Sync>(
    conn: &mut SqliteConnection,
    calendar_id: i64,
    named: &[S],
    reach: Reach,
) -> sqlx::Result<u64> {
    sqlx::query("CREATE TEMP TABLE IF NOT EXISTS sync_named (google_id TEXT PRIMARY KEY)")
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM sync_named").execute(&mut *conn).await?;
    for id in named {
        sqlx::query("INSERT OR IGNORE INTO sync_named (google_id) VALUES (?1)")
            .bind(id.as_ref())
            .execute(&mut *conn)
            .await?;
    }
    const UNNAMED: &str = "DELETE FROM events WHERE calendar_id = ?1
        AND google_id NOT IN (SELECT google_id FROM sync_named)";
    let deleted = match reach {
        Reach::Whole => sqlx::query(UNNAMED).bind(calendar_id).execute(&mut *conn).await?,
        Reach::Overlapping { start_ms, end_ms } => {
            sqlx::query(&format!(
                "{UNNAMED} AND start_utc < ?2 AND (end_utc > ?3 OR recurrence IS NOT NULL)"
            ))
            .bind(calendar_id)
            .bind(end_ms)
            .bind(start_ms)
            .execute(&mut *conn)
            .await?
        }
        Reach::EndingFrom { start_ms } => {
            sqlx::query(&format!("{UNNAMED} AND (recurrence IS NOT NULL OR end_utc >= ?2)"))
                .bind(calendar_id)
                .bind(start_ms)
                .execute(&mut *conn)
                .await?
        }
    }
    .rows_affected();
    sqlx::query("DELETE FROM sync_named").execute(&mut *conn).await?;
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool_with(rows: &[(&str, i64, i64, bool)]) -> sqlx::SqlitePool {
        let pool = omacal_store::connect_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (google_sub, email, created_at) VALUES ('s','e',0)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO calendars (account_id, google_id, summary, timezone, access_role) VALUES (1,'c','C','UTC','owner')")
            .execute(&pool).await.unwrap();
        for (id, start, end, series) in rows {
            sqlx::query(
                "INSERT INTO events (calendar_id, google_id, start_utc, end_utc, start_tz, end_tz, recurrence, updated_at)
                 VALUES (1, ?1, ?2, ?3, 'UTC', 'UTC', ?4, 0)",
            )
            .bind(id).bind(start).bind(end)
            .bind(series.then_some(r#"["RRULE:FREQ=DAILY"]"#))
            .execute(&pool).await.unwrap();
        }
        pool
    }

    async fn left(pool: &sqlx::SqlitePool) -> Vec<String> {
        sqlx::query_scalar("SELECT google_id FROM events ORDER BY google_id").fetch_all(pool).await.unwrap()
    }

    /// Each reach deletes exactly its own slice. `before` ends before the
    /// window, `inside` overlaps it, `after` starts past its end, `series` is
    /// a master whose own dates are ancient; `kept` is named, so stays always.
    #[tokio::test]
    async fn each_reach_deletes_its_own_slice_and_never_a_named_row() {
        let rows = [
            ("before", 0, 10, false),
            ("inside", 150, 160, false),
            ("after", 300, 310, false),
            ("series", 0, 10, true),
            ("kept", 150, 160, false),
        ];
        for (reach, survivors) in [
            (Reach::Whole, vec!["kept"]),
            (Reach::Overlapping { start_ms: 100, end_ms: 200 }, vec!["after", "before", "kept"]),
            (Reach::EndingFrom { start_ms: 100 }, vec!["before", "kept"]),
        ] {
            let pool = pool_with(&rows).await;
            let mut conn = pool.acquire().await.unwrap();
            delete_unnamed(&mut conn, 1, &["kept"], reach).await.unwrap();
            drop(conn);
            assert_eq!(left(&pool).await, survivors, "{reach:?}");
        }
    }

    /// Past SQLite's 32,766-variable ceiling, which one placeholder per id hit.
    #[tokio::test]
    async fn more_names_than_sqlite_has_variables_is_not_an_error() {
        let pool = pool_with(&[("gone", 150, 160, false), ("id-7", 150, 160, false)]).await;
        let names: Vec<String> = (0..40_000).map(|i| format!("id-{i}")).collect();
        let mut conn = pool.acquire().await.unwrap();
        let n = delete_unnamed(&mut conn, 1, &names, Reach::Whole)
            .await
            .unwrap();
        drop(conn);
        assert_eq!(n, 1);
        assert_eq!(left(&pool).await, ["id-7"]);
    }
}
