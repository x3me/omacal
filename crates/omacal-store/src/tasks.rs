//! VTODO storage — the task half of a CalDAV account.
//!
//! Shapes mirror `events.rs` where the concepts overlap: `uid` + `etag` +
//! `caldav_href` + `raw_ics` exist for the same write-back reasons an event
//! carries them (a CalDAV write rewrites a whole resource, and properties this
//! app does not model must survive the round trip). Unlike events, tasks have
//! no recurrence expansion here — a recurring VTODO is stored as its current
//! instance, which is how every mainstream tasks client treats it.

use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, PartialEq)]
pub struct StoredTask {
    pub id: i64,
    pub calendar_id: i64,
    pub uid: String,
    pub etag: Option<String>,
    pub caldav_href: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub due_utc: Option<i64>,
    pub due_tz: Option<String>,
    pub due_all_day: bool,
    /// `needs-action` | `in-process` | `completed` | `cancelled`.
    pub status: String,
    pub completed_utc: Option<i64>,
    /// ICS PRIORITY: 0 = undefined, 1 highest .. 9 lowest.
    pub priority: i64,
    pub updated_at: i64,
    pub raw_ics: Option<String>,
}

/// A task joined with what the UI draws it with — its list's name and colour.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskRow {
    pub task: StoredTask,
    pub calendar_summary: String,
    pub color_hex: Option<String>,
    /// The list's access role — `reader` lists show their tasks but refuse
    /// completion toggles, the same honesty rule events follow.
    pub access_role: String,
}

fn row_to_task(row: &SqliteRow) -> StoredTask {
    StoredTask {
        id: row.get("id"),
        calendar_id: row.get("calendar_id"),
        uid: row.get("uid"),
        etag: row.get("etag"),
        caldav_href: row.get("caldav_href"),
        summary: row.get("summary"),
        description: row.get("description"),
        due_utc: row.get("due_utc"),
        due_tz: row.get("due_tz"),
        due_all_day: row.get::<i64, _>("due_all_day") != 0,
        status: row.get("status"),
        completed_utc: row.get("completed_utc"),
        priority: row.get("priority"),
        updated_at: row.get("updated_at"),
        raw_ics: row.get("raw_ics"),
    }
}

const COLS: &str = "t.id, t.calendar_id, t.uid, t.etag, t.caldav_href, t.summary,
    t.description, t.due_utc, t.due_tz, t.due_all_day, t.status, t.completed_utc,
    t.priority, t.updated_at, t.raw_ics";

/// Inserts or updates one task by `(calendar_id, uid)`. Returns the row id.
pub async fn upsert_task(pool: &SqlitePool, t: &StoredTask) -> anyhow::Result<i64> {
    let id = sqlx::query_scalar(
        "INSERT INTO tasks
            (calendar_id, uid, etag, caldav_href, summary, description, due_utc,
             due_tz, due_all_day, status, completed_utc, priority, updated_at, raw_ics)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT (calendar_id, uid) DO UPDATE SET
            etag = excluded.etag,
            caldav_href = excluded.caldav_href,
            summary = excluded.summary,
            description = excluded.description,
            due_utc = excluded.due_utc,
            due_tz = excluded.due_tz,
            due_all_day = excluded.due_all_day,
            status = excluded.status,
            completed_utc = excluded.completed_utc,
            priority = excluded.priority,
            updated_at = excluded.updated_at,
            raw_ics = excluded.raw_ics
         RETURNING id",
    )
    .bind(t.calendar_id)
    .bind(&t.uid)
    .bind(&t.etag)
    .bind(&t.caldav_href)
    .bind(&t.summary)
    .bind(&t.description)
    .bind(t.due_utc)
    .bind(&t.due_tz)
    .bind(t.due_all_day as i64)
    .bind(&t.status)
    .bind(t.completed_utc)
    .bind(t.priority)
    .bind(t.updated_at)
    .bind(&t.raw_ics)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Removes every task of `calendar_id` whose uid is not in `keep` — the
/// reconciliation step after a full list fetch, exactly parallel to how a
/// windowed event sync deletes what the server no longer returns.
pub async fn delete_tasks_not_in(
    pool: &SqlitePool,
    calendar_id: i64,
    keep: &[String],
) -> anyhow::Result<u64> {
    // SQLite has no array binds; build the placeholder list. `keep` is a
    // task list's worth of uids, not unbounded user input.
    let placeholders: Vec<String> = (0..keep.len()).map(|i| format!("?{}", i + 2)).collect();
    let sql = if keep.is_empty() {
        "DELETE FROM tasks WHERE calendar_id = ?1".to_string()
    } else {
        format!(
            "DELETE FROM tasks WHERE calendar_id = ?1 AND uid NOT IN ({})",
            placeholders.join(", ")
        )
    };
    let mut q = sqlx::query(&sql).bind(calendar_id);
    for uid in keep {
        q = q.bind(uid);
    }
    Ok(q.execute(pool).await?.rows_affected())
}

/// Every task the UI shows: from selected, task-capable calendars — open
/// tasks all of them, done ones only from the recent past (`done_since_ms`),
/// so a years-old completed list does not bury today. Open tasks sort by due
/// (undated last), then priority (undefined last), then title; done tasks by
/// completion, newest first.
pub async fn tasks_for_ui(pool: &SqlitePool, done_since_ms: i64) -> anyhow::Result<Vec<TaskRow>> {
    let sql = format!(
        "SELECT {COLS}, COALESCE(c.label_override, c.summary) AS cal_summary,
                COALESCE(c.color_override, c.color_hex) AS cal_color,
                c.access_role AS cal_role
         FROM tasks t
         JOIN calendars c ON c.id = t.calendar_id
         WHERE c.selected = 1
           AND t.status != 'cancelled'
           AND (t.status != 'completed' OR t.completed_utc IS NULL
                OR t.completed_utc >= ?1)
         ORDER BY
           CASE WHEN t.status = 'completed' THEN 1 ELSE 0 END,
           CASE WHEN t.due_utc IS NULL THEN 1 ELSE 0 END,
           t.due_utc,
           CASE WHEN t.priority = 0 THEN 10 ELSE t.priority END,
           t.summary,
           t.completed_utc DESC"
    );
    let rows = sqlx::query(&sql).bind(done_since_ms).fetch_all(pool).await?;
    Ok(rows
        .iter()
        .map(|row| TaskRow {
            task: row_to_task(row),
            calendar_summary: row.get("cal_summary"),
            color_hex: row.get("cal_color"),
            access_role: row.get("cal_role"),
        })
        .collect())
}

/// Completed tasks from selected lists, newest completion first — the
/// Done list's history, past the week `tasks_for_ui` carries.
///
/// `before_ms` leaves out what was completed at or after it (the window
/// passes its own midnight, so "earlier" never repeats "today"). A completed
/// task with no completion stamp — a server that set STATUS without
/// COMPLETED — cannot be dated, so it is never "today" and sorts last.
/// Unfiltered by text: SQLite's `LOWER` folds ASCII only, and a list kept in
/// Bulgarian must search the same as one kept in English, so the caller
/// matches in Rust.
pub async fn completed_tasks_for_ui(
    pool: &SqlitePool,
    before_ms: Option<i64>,
) -> anyhow::Result<Vec<TaskRow>> {
    let sql = format!(
        "SELECT {COLS}, COALESCE(c.label_override, c.summary) AS cal_summary,
                COALESCE(c.color_override, c.color_hex) AS cal_color,
                c.access_role AS cal_role
         FROM tasks t
         JOIN calendars c ON c.id = t.calendar_id
         WHERE c.selected = 1
           AND t.status = 'completed'
           AND (?1 IS NULL OR t.completed_utc IS NULL OR t.completed_utc < ?1)
         ORDER BY
           CASE WHEN t.completed_utc IS NULL THEN 1 ELSE 0 END,
           t.completed_utc DESC,
           t.summary"
    );
    let rows = sqlx::query(&sql).bind(before_ms).fetch_all(pool).await?;
    Ok(rows
        .iter()
        .map(|row| TaskRow {
            task: row_to_task(row),
            calendar_summary: row.get("cal_summary"),
            color_hex: row.get("cal_color"),
            access_role: row.get("cal_role"),
        })
        .collect())
}

/// One task by id, for the write path.
pub async fn task_by_id(pool: &SqlitePool, id: i64) -> anyhow::Result<Option<StoredTask>> {
    let sql = format!("SELECT {COLS} FROM tasks t WHERE t.id = ?1");
    Ok(sqlx::query(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .map(|row| row_to_task(&row)))
}

/// The optimistic local half of completing (or reopening) a task: the server
/// write happens first, then this records what the server now holds without
/// waiting for the next sync to say so.
///
/// The etag is written as given, `None` included: after a write the old one
/// no longer names the resource, and keeping it guarded the next write with
/// a precondition the server was bound to refuse.
pub async fn mark_task_status(
    pool: &SqlitePool,
    id: i64,
    status: &str,
    completed_utc: Option<i64>,
    etag: Option<&str>,
    raw_ics: Option<&str>,
    updated_at: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE tasks SET status = ?2, completed_utc = ?3,
            etag = ?4, raw_ics = COALESCE(?5, raw_ics),
            updated_at = ?6
         WHERE id = ?1",
    )
    .bind(id)
    .bind(status)
    .bind(completed_utc)
    .bind(etag)
    .bind(raw_ics)
    .bind(updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Writes an edited task's fields back — after (never before) the server
/// took the same edit, which is the rule every write here follows.
///
/// The etag and the resource are written unconditionally rather than
/// through `COALESCE`, unlike `mark_task_status`: an edit that could not be
/// stored would leave the row describing the old text with the new etag,
/// and the next sync would see a matching etag and never correct it.
#[allow(clippy::too_many_arguments)]
pub async fn update_task_fields(
    pool: &SqlitePool,
    id: i64,
    summary: &str,
    description: Option<&str>,
    due_utc: Option<i64>,
    due_tz: Option<&str>,
    due_all_day: bool,
    priority: i64,
    etag: Option<&str>,
    raw_ics: &str,
    updated_at: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE tasks SET summary = ?2, description = ?3, due_utc = ?4, due_tz = ?5,
            due_all_day = ?6, priority = ?7, etag = ?8, raw_ics = ?9, updated_at = ?10
         WHERE id = ?1",
    )
    .bind(id)
    .bind(summary)
    .bind(description)
    .bind(due_utc)
    .bind(due_tz)
    .bind(due_all_day)
    .bind(priority)
    .bind(etag)
    .bind(raw_ics)
    .bind(updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Moves a task row to another list, keeping its id — after (never before)
/// both ends took the move. Every column comes from `t`, and `t.id` names the
/// row.
///
/// A sync can run between the PUT onto the new list and this write. If the
/// new list's sync ran, it already stored the moved task as a row of its own,
/// which the `(calendar_id, uid)` key would refuse a second of, so that copy
/// is dropped first. If the old list's sync ran, it already deleted this row,
/// so the task is inserted again rather than vanishing until the next sync.
pub async fn move_task(pool: &SqlitePool, t: &StoredTask) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM tasks WHERE calendar_id = ?1 AND uid = ?2 AND id <> ?3")
        .bind(t.calendar_id)
        .bind(&t.uid)
        .bind(t.id)
        .execute(&mut *tx)
        .await?;
    let moved = sqlx::query(
        "UPDATE tasks SET calendar_id = ?2, etag = ?3, caldav_href = ?4, summary = ?5,
            description = ?6, due_utc = ?7, due_tz = ?8, due_all_day = ?9, status = ?10,
            completed_utc = ?11, priority = ?12, updated_at = ?13, raw_ics = ?14
         WHERE id = ?1",
    )
    .bind(t.id)
    .bind(t.calendar_id)
    .bind(&t.etag)
    .bind(&t.caldav_href)
    .bind(&t.summary)
    .bind(&t.description)
    .bind(t.due_utc)
    .bind(&t.due_tz)
    .bind(t.due_all_day as i64)
    .bind(&t.status)
    .bind(t.completed_utc)
    .bind(t.priority)
    .bind(t.updated_at)
    .bind(&t.raw_ics)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if moved == 0 {
        sqlx::query(
            "INSERT INTO tasks
                (calendar_id, uid, etag, caldav_href, summary, description, due_utc,
                 due_tz, due_all_day, status, completed_utc, priority, updated_at, raw_ics)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )
        .bind(t.calendar_id)
        .bind(&t.uid)
        .bind(&t.etag)
        .bind(&t.caldav_href)
        .bind(&t.summary)
        .bind(&t.description)
        .bind(t.due_utc)
        .bind(&t.due_tz)
        .bind(t.due_all_day as i64)
        .bind(&t.status)
        .bind(t.completed_utc)
        .bind(t.priority)
        .bind(t.updated_at)
        .bind(&t.raw_ics)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Every task the scheduler could announce (#137): open, due at an instant
/// rather than on a date, on a list the user has selected, with a due in
/// `[from_ms, to_ms]`.
///
/// `selected = 1` is the same gate `tasks_for_ui` applies, and for the same
/// reason the notification loop reads events through `events_in_window`: a
/// list not worth drawing is not worth interrupting for. Completed and
/// cancelled tasks are left out here rather than filtered later, so completing
/// one is all it takes to call an announcement off.
pub async fn tasks_to_announce(
    pool: &SqlitePool,
    from_ms: i64,
    to_ms: i64,
) -> anyhow::Result<Vec<TaskRow>> {
    let sql = format!(
        "SELECT {COLS}, COALESCE(c.label_override, c.summary) AS cal_summary,
                COALESCE(c.color_override, c.color_hex) AS cal_color,
                c.access_role AS cal_role
         FROM tasks t
         JOIN calendars c ON c.id = t.calendar_id
         WHERE c.selected = 1
           AND t.status NOT IN ('completed', 'cancelled')
           AND t.due_all_day = 0
           AND t.due_utc IS NOT NULL
           AND t.due_utc BETWEEN ?1 AND ?2
         ORDER BY t.due_utc"
    );
    let rows = sqlx::query(&sql).bind(from_ms).bind(to_ms).fetch_all(pool).await?;
    Ok(rows
        .iter()
        .map(|row| TaskRow {
            task: row_to_task(row),
            calendar_summary: row.get("cal_summary"),
            color_hex: row.get("cal_color"),
            access_role: row.get("cal_role"),
        })
        .collect())
}

/// Task announcements already posted, as `(task_id, due_ms)`.
pub async fn fired_task_keys(pool: &SqlitePool) -> anyhow::Result<Vec<(i64, i64)>> {
    Ok(sqlx::query_as("SELECT task_id, due_ms FROM fired_task_reminders").fetch_all(pool).await?)
}

/// Records one announcement. Idempotent: a pass that posts again after a crash
/// between posting and recording writes the same row.
pub async fn record_fired_task(
    pool: &SqlitePool,
    task_id: i64,
    due_ms: i64,
    fired_at_ms: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO fired_task_reminders (task_id, due_ms, fired_at_ms)
         VALUES (?1, ?2, ?3)
         ON CONFLICT (task_id, due_ms) DO UPDATE SET fired_at_ms = excluded.fired_at_ms",
    )
    .bind(task_id)
    .bind(due_ms)
    .bind(fired_at_ms)
    .execute(pool)
    .await?;
    Ok(())
}

/// Forgets announcements nothing answers to any more: the task is gone, or its
/// due has moved. Not a horizon (see the migration): an overdue task left
/// undone keeps its record for as long as it keeps that due, which is what
/// stops it announcing itself again every launch.
pub async fn prune_fired_tasks(pool: &SqlitePool) -> anyhow::Result<u64> {
    Ok(sqlx::query(
        "DELETE FROM fired_task_reminders
         WHERE NOT EXISTS (
           SELECT 1 FROM tasks t WHERE t.id = task_id AND t.due_utc = due_ms
         )",
    )
    .execute(pool)
    .await?
    .rows_affected())
}

/// Deletes one task row — after (never before) the server delete succeeded.
pub async fn delete_task(pool: &SqlitePool, id: i64) -> anyhow::Result<u64> {
    Ok(sqlx::query("DELETE FROM tasks WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect_memory;

    async fn seeded() -> (SqlitePool, i64) {
        let pool = connect_memory().await.unwrap();
        sqlx::query(
            "INSERT INTO accounts (google_sub, email, created_at, provider)
             VALUES ('caldav:p@x', 'p@x', 0, 'caldav')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role,
                                    supports_events, supports_tasks)
             VALUES (1, '/cal/tasks/', 'Chores', 'Europe/Sofia', 'owner', 0, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        (pool, 1)
    }

    fn task(uid: &str, due: Option<i64>) -> StoredTask {
        StoredTask {
            id: 0,
            calendar_id: 1,
            uid: uid.into(),
            etag: Some("\"1\"".into()),
            caldav_href: Some(format!("/cal/tasks/{uid}.ics")),
            summary: Some("Water plants".into()),
            description: None,
            due_utc: due,
            due_tz: Some("Europe/Sofia".into()),
            due_all_day: false,
            status: "needs-action".into(),
            completed_utc: None,
            priority: 0,
            updated_at: 1,
            raw_ics: Some("BEGIN:VCALENDAR...".into()),
        }
    }

    #[tokio::test]
    async fn upsert_inserts_then_updates_by_uid() {
        let (pool, _) = seeded().await;
        let id1 = upsert_task(&pool, &task("a", Some(1000))).await.unwrap();
        let mut changed = task("a", Some(2000));
        changed.summary = Some("Water plants twice".into());
        let id2 = upsert_task(&pool, &changed).await.unwrap();
        assert_eq!(id1, id2, "same uid is the same row");
        let got = task_by_id(&pool, id1).await.unwrap().unwrap();
        assert_eq!(got.due_utc, Some(2000));
        assert_eq!(got.summary.as_deref(), Some("Water plants twice"));
    }

    #[tokio::test]
    async fn an_edit_stores_the_priority_it_was_given() {
        let (pool, _) = seeded().await;
        let id = upsert_task(&pool, &task("p", Some(1_000))).await.unwrap();

        update_task_fields(
            &pool, id, "Water plants", None, Some(1_000), Some("Europe/Sofia"),
            false, 9, Some("\"2\""), "BEGIN:VCALENDAR...", 2,
        )
        .await
        .unwrap();

        let got = task_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(got.priority, 9, "the edit writes priority, not only the sync");
    }

    #[tokio::test]
    async fn reconciliation_deletes_what_the_server_dropped() {
        let (pool, _) = seeded().await;
        upsert_task(&pool, &task("keep", None)).await.unwrap();
        upsert_task(&pool, &task("drop", None)).await.unwrap();
        let n = delete_tasks_not_in(&pool, 1, &["keep".to_string()]).await.unwrap();
        assert_eq!(n, 1);
        let rows = tasks_for_ui(&pool, 0).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].task.uid, "keep");
        // And an empty keep-list empties the calendar — the list was deleted.
        let n = delete_tasks_not_in(&pool, 1, &[]).await.unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn the_ui_ordering_puts_the_urgent_first_and_the_done_last() {
        let (pool, _) = seeded().await;
        let mut undated = task("undated", None);
        undated.summary = Some("Someday".into());
        let mut soon = task("soon", Some(1_000));
        soon.summary = Some("Now-ish".into());
        let mut later = task("later", Some(2_000));
        later.summary = Some("After".into());
        let mut done = task("done", Some(500));
        done.status = "completed".into();
        done.completed_utc = Some(900);
        let mut cancelled = task("cancelled", None);
        cancelled.status = "cancelled".into();
        for t in [&undated, &soon, &later, &done, &cancelled] {
            upsert_task(&pool, t).await.unwrap();
        }

        let rows = tasks_for_ui(&pool, 0).await.unwrap();
        let uids: Vec<&str> = rows.iter().map(|r| r.task.uid.as_str()).collect();
        assert_eq!(uids, vec!["soon", "later", "undated", "done"], "cancelled never shows");
        assert_eq!(rows[0].calendar_summary, "Chores");
    }

    /// Same due date, different priorities: the smaller number (higher) first,
    /// and a task with none last. The order the pane reads off the store.
    #[tokio::test]
    async fn the_ui_ordering_breaks_a_due_tie_by_priority() {
        let (pool, _) = seeded().await;
        let mut low = task("low", Some(1_000));
        low.priority = 9;
        let mut high = task("high", Some(1_000));
        high.priority = 1;
        let mut none = task("none", Some(1_000));
        none.priority = 0;
        for t in [&low, &none, &high] {
            upsert_task(&pool, t).await.unwrap();
        }

        let rows = tasks_for_ui(&pool, 0).await.unwrap();
        let uids: Vec<&str> = rows.iter().map(|r| r.task.uid.as_str()).collect();
        assert_eq!(uids, vec!["high", "low", "none"], "priority breaks the tie, none last");
    }

    #[tokio::test]
    async fn old_completed_tasks_age_out_of_the_ui() {
        let (pool, _) = seeded().await;
        let mut ancient = task("ancient", None);
        ancient.status = "completed".into();
        ancient.completed_utc = Some(1_000);
        let mut recent = task("recent", None);
        recent.status = "completed".into();
        recent.completed_utc = Some(9_000);
        upsert_task(&pool, &ancient).await.unwrap();
        upsert_task(&pool, &recent).await.unwrap();

        let rows = tasks_for_ui(&pool, 5_000).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].task.uid, "recent");
    }

    #[tokio::test]
    async fn completed_history_is_newest_first_before_the_cutoff_and_undated_last() {
        let (pool, _) = seeded().await;
        let done = |uid: &str, at: Option<i64>| {
            let mut t = task(uid, None);
            t.status = "completed".into();
            t.completed_utc = at;
            t
        };
        for t in [
            done("monday", Some(1_000)),
            done("wednesday", Some(3_000)),
            done("today", Some(9_000)),
            done("undated", None),
            task("open", None),
        ] {
            upsert_task(&pool, &t).await.unwrap();
        }

        let all: Vec<String> = completed_tasks_for_ui(&pool, None).await.unwrap()
            .into_iter().map(|r| r.task.uid).collect();
        assert_eq!(all, vec!["today", "wednesday", "monday", "undated"], "an open task never shows");

        // The window's midnight: today's completion is the Done list's own,
        // and must not come back as "earlier". The undated one has no day,
        // so it is always earlier.
        let earlier: Vec<String> = completed_tasks_for_ui(&pool, Some(9_000)).await.unwrap()
            .into_iter().map(|r| r.task.uid).collect();
        assert_eq!(earlier, vec!["wednesday", "monday", "undated"]);

        sqlx::query("UPDATE calendars SET selected = 0 WHERE id = 1")
            .execute(&pool).await.unwrap();
        assert!(completed_tasks_for_ui(&pool, None).await.unwrap().is_empty(), "a hidden list's history is hidden too");
    }

    #[tokio::test]
    async fn an_unselected_list_contributes_nothing() {
        let (pool, _) = seeded().await;
        upsert_task(&pool, &task("hidden", None)).await.unwrap();
        sqlx::query("UPDATE calendars SET selected = 0 WHERE id = 1")
            .execute(&pool)
            .await
            .unwrap();
        assert!(tasks_for_ui(&pool, 0).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn marking_status_updates_in_place() {
        let (pool, _) = seeded().await;
        let id = upsert_task(&pool, &task("t", None)).await.unwrap();
        mark_task_status(&pool, id, "completed", Some(42), Some("\"2\""), None, 43)
            .await
            .unwrap();
        let got = task_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(got.status, "completed");
        assert_eq!(got.completed_utc, Some(42));
        assert_eq!(got.etag.as_deref(), Some("\"2\""));
        assert_eq!(got.raw_ics.as_deref(), Some("BEGIN:VCALENDAR..."), "None left it alone");
    }

    /// A move keeps the row's id, whichever sync ran in the middle of it: a
    /// copy the new list's sync already stored is dropped, and a row the old
    /// list's sync already deleted comes back rather than vanishing.
    #[tokio::test]
    async fn a_moved_task_keeps_its_id_whichever_sync_got_there_first() {
        let (pool, _) = seeded().await;
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role,
                                    supports_events, supports_tasks)
             VALUES (1, '/cal/errands/', 'Errands', 'UTC', 'owner', 0, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let id = upsert_task(&pool, &task("a", None)).await.unwrap();

        // The new list's sync stored its own copy already.
        let mut synced = task("a", None);
        synced.calendar_id = 2;
        upsert_task(&pool, &synced).await.unwrap();

        let mut moved = task_by_id(&pool, id).await.unwrap().unwrap();
        moved.calendar_id = 2;
        moved.caldav_href = Some("/cal/errands/x.ics".into());
        moved.etag = Some("\"9\"".into());
        move_task(&pool, &moved).await.unwrap();
        let rows = tasks_for_ui(&pool, 0).await.unwrap();
        assert_eq!(rows.len(), 1, "one task, not the moved row and the synced copy");
        assert_eq!(rows[0].task.id, id);
        assert_eq!(rows[0].task.calendar_id, 2);
        assert_eq!(rows[0].task.caldav_href.as_deref(), Some("/cal/errands/x.ics"));

        // The old list's sync deleted the row before the move was written.
        delete_task(&pool, id).await.unwrap();
        moved.calendar_id = 1;
        move_task(&pool, &moved).await.unwrap();
        let rows = tasks_for_ui(&pool, 0).await.unwrap();
        assert_eq!(rows.len(), 1, "written again, not lost");
        assert_eq!((rows[0].task.calendar_id, rows[0].task.uid.as_str()), (1, "a"));
    }

    /// #137's set: open, timed, on a selected list, inside the window — and
    /// the record that keeps one from being announced twice, forgotten when
    /// the task is gone or its due has moved.
    #[tokio::test]
    async fn only_open_timed_tasks_are_announced_and_a_record_outlives_only_its_due() {
        let (pool, _) = seeded().await;
        let mut timed = task("timed", Some(5_000));
        timed.summary = Some("Call the bank".into());
        let id = upsert_task(&pool, &timed).await.unwrap();
        let mut all_day = task("all-day", Some(5_000));
        all_day.due_all_day = true;
        upsert_task(&pool, &all_day).await.unwrap();
        let mut done = task("done", Some(5_000));
        done.status = "completed".into();
        upsert_task(&pool, &done).await.unwrap();
        upsert_task(&pool, &task("undated", None)).await.unwrap();
        let mut far = task("far", Some(9_000_000));
        far.due_all_day = false;
        upsert_task(&pool, &far).await.unwrap();

        let rows = tasks_to_announce(&pool, 0, 10_000).await.unwrap();
        assert_eq!(
            rows.iter().map(|r| r.task.uid.as_str()).collect::<Vec<_>>(),
            vec!["timed"],
            "a date, a done one, an undated one and one past the window all say nothing",
        );
        assert_eq!(rows[0].calendar_summary, "Chores", "the list's name rides along");

        // A hidden list says nothing either.
        sqlx::query("UPDATE calendars SET selected = 0").execute(&pool).await.unwrap();
        assert!(tasks_to_announce(&pool, 0, 10_000).await.unwrap().is_empty());
        sqlx::query("UPDATE calendars SET selected = 1").execute(&pool).await.unwrap();

        record_fired_task(&pool, id, 5_000, 6_000).await.unwrap();
        record_fired_task(&pool, id, 5_000, 7_000).await.unwrap();
        assert_eq!(fired_task_keys(&pool).await.unwrap(), vec![(id, 5_000)], "recorded once");
        assert_eq!(prune_fired_tasks(&pool).await.unwrap(), 0, "its due is still its due");

        // The due moves: the old record has nothing to answer to.
        let mut moved = timed.clone();
        moved.due_utc = Some(8_000);
        upsert_task(&pool, &moved).await.unwrap();
        assert_eq!(prune_fired_tasks(&pool).await.unwrap(), 1);
        assert!(fired_task_keys(&pool).await.unwrap().is_empty());

        // And a record outlives a deleted task no longer than the next prune.
        record_fired_task(&pool, id, 8_000, 9_000).await.unwrap();
        delete_task(&pool, id).await.unwrap();
        assert_eq!(prune_fired_tasks(&pool).await.unwrap(), 1);
    }
}
