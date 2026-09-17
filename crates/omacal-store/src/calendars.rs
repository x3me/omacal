use serde::Serialize;
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, Serialize)]
pub struct CalendarRow {
    pub id: i64,
    pub account_id: i64,
    pub account_email: String,
    pub summary: String,
    pub provider_summary: String,
    pub label_override: Option<String>,
    /// **The colour to draw this calendar in** — its override if it has one,
    /// and Google's own otherwise, resolved by the same `COALESCE` the event
    /// read uses. Every consumer that only wants to *draw* something reads
    /// this and needs to know nothing about overrides.
    pub color_hex: Option<String>,
    /// The override itself, or `None` when there is not one.
    ///
    /// Separate from the field above because **clearing an override is a
    /// different state from setting it to whatever Google currently uses**,
    /// and only this field can tell them apart — the settings row needs it to
    /// show which swatch is chosen and whether there is anything to clear. See
    /// `0006_calendar_colour.sql`.
    pub color_override: Option<String>,
    /// Drawn in the grid.
    pub selected: bool,
    /// Fetched from Google at all.
    pub sync_enabled: bool,
    pub is_primary: bool,
    /// Google's own word for what this account may do here: `owner`, `writer`,
    /// `reader`, `freeBusyReader`.
    ///
    /// Carried all the way to the UI because the event form has to offer only
    /// the calendars a create could actually land on — a subscribed holiday
    /// calendar is a `reader`, and offering it produces a Save that
    /// `create_impl` can only refuse. That refusal already exists and stays
    /// (`can_edit`, applied server-side against this same column via
    /// `calendar_for_write`); this field is what stops the UI walking into it.
    pub access_role: String,
    /// The owning account's provider (`google` | `caldav` | `webcal` | `local`)
    /// — what the UI gates provider-specific affordances on.
    pub provider: String,
}

/// Every calendar across every account, primary first, then alphabetical —
/// stable ordering so the popover does not reshuffle between renders.
pub async fn list_calendars(pool: &SqlitePool) -> anyhow::Result<Vec<CalendarRow>> {
    let rows = sqlx::query(
        "SELECT c.id, c.account_id, a.email AS account_email, COALESCE(c.label_override, c.summary) AS summary,
                c.summary AS provider_summary, c.label_override,
                COALESCE(c.color_override, c.color_hex) AS color_hex,
                c.color_override,
                c.selected, c.sync_enabled, c.is_primary, c.access_role, a.provider
         FROM calendars c
         JOIN accounts a ON a.id = c.account_id
         ORDER BY a.email, c.is_primary DESC, COALESCE(c.label_override, c.summary) COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| CalendarRow {
            id: r.get("id"),
            account_id: r.get("account_id"),
            account_email: r.get("account_email"),
            summary: r.get("summary"),
            provider_summary: r.get("provider_summary"),
            label_override: r.get("label_override"),
            color_hex: r.get("color_hex"),
            color_override: r.get("color_override"),
            selected: r.get::<i64, _>("selected") != 0,
            sync_enabled: r.get::<i64, _>("sync_enabled") != 0,
            is_primary: r.get::<i64, _>("is_primary") != 0,
            access_role: r.get("access_role"),
            provider: r.get("provider"),
        })
        .collect())
}

/// One calendar's `google_id`, `access_role`, owning account's email, and own
/// stored `timezone`, by the calendar's own row id.
///
/// The counterpart to [`crate::events::event_for_write`] for a write that
/// creates an event rather than changing one that already exists: there is no
/// event row yet to key a lookup on, only the calendar it will be created on,
/// so this starts from `calendars` instead of `events` and skips straight to
/// the two joins that query already does.
///
/// `timezone` is included because it must be the *calendar's own* zone that a
/// caller later hands to `omacal_sync::to_stored`, not whatever zone the
/// caller happens to be authoring an event in — see `create_via_client`'s doc
/// comment for why an all-day create is the case that makes those two differ.
pub async fn calendar_for_write(
    pool: &SqlitePool,
    id: i64,
) -> anyhow::Result<Option<(String, String, String, String)>> {
    let row = sqlx::query(
        "SELECT c.google_id, c.access_role, a.email AS account_email, c.timezone
         FROM calendars c
         JOIN accounts a ON a.id = c.account_id
         WHERE c.id = ?1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        (r.get("google_id"), r.get("access_role"), r.get("account_email"), r.get("timezone"))
    }))
}

/// Show or hide a calendar. Pure display — no data is fetched or discarded.
/// The account every on-this-device list belongs to, and the list itself.
///
/// A Google account brings no tasks — Google keeps those in a different
/// product with a different API — so an install with only Google had a Tasks
/// pane that could never hold anything (Plamen, 2026-09-16). This is the
/// answer: a task list that belongs to no server.
///
/// It is an ordinary account and calendar row with `provider = 'local'`,
/// which is what keeps the rest of the app unchanged. The sync driver picks
/// accounts by provider name, so nothing ever tries to fetch or push these;
/// the Tasks pane, the CLI and the store all read them as they read any
/// other list. `sync_enabled = 0` says the same thing again, for a human
/// reading the row.
pub const LOCAL_PROVIDER: &str = "local";
/// Provider value for a read-only WebCal subscription (public `webcal://` /
/// `https://…ics` feed). One account row per feed URL; the feed URL itself
/// rides in `accounts.server_url` and in the calendar's `google_id` (the
/// "provider identifier" column), so no schema change was needed — `provider`
/// has no CHECK constraint. Always `access_role = 'reader'`, events-only.
pub const WEBCAL_PROVIDER: &str = "webcal";
const LOCAL_ACCOUNT_SUB: &str = "local:device";
const LOCAL_TASKS_ID: &str = "local:tasks";

/// Creates the on-this-device task list if it is not there, and returns its
/// calendar id either way.
///
/// Idempotent, because the button that calls it is a button: pressing it
/// twice must not leave two lists with the same name, and an install that
/// already has one must land on it.
pub async fn ensure_local_task_list(
    pool: &SqlitePool,
    name: &str,
    timezone: &str,
    now_ms: i64,
) -> anyhow::Result<i64> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT OR IGNORE INTO accounts (google_sub, email, display_name, created_at, provider)
         VALUES (?1, ?1, 'On this device', ?2, ?3)",
    )
    .bind(LOCAL_ACCOUNT_SUB)
    .bind(now_ms)
    .bind(LOCAL_PROVIDER)
    .execute(&mut *tx)
    .await?;
    let account_id: i64 =
        sqlx::query_scalar("SELECT id FROM accounts WHERE google_sub = ?1")
            .bind(LOCAL_ACCOUNT_SUB)
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query(
        "INSERT OR IGNORE INTO calendars
            (account_id, google_id, summary, timezone, access_role, selected, is_primary,
             sync_enabled, supports_events, supports_tasks)
         VALUES (?1, ?2, ?3, ?4, 'owner', 1, 0, 0, 0, 1)",
    )
    .bind(account_id)
    .bind(LOCAL_TASKS_ID)
    .bind(name)
    .bind(timezone)
    .execute(&mut *tx)
    .await?;
    let calendar_id: i64 = sqlx::query_scalar(
        "SELECT id FROM calendars WHERE account_id = ?1 AND google_id = ?2",
    )
    .bind(account_id)
    .bind(LOCAL_TASKS_ID)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(calendar_id)
}

/// Another list on this device, with the name and colour the user gave it.
///
/// Not idempotent, unlike [`ensure_local_task_list`]: this is "New list", and
/// two presses with two names are two lists (the caller refuses a repeated
/// name). Each gets its own `google_id` — random, since nothing addresses it
/// but this row — under the same one on-this-device account.
pub async fn create_local_list(
    pool: &SqlitePool,
    name: &str,
    timezone: &str,
    color: &str,
    now_ms: i64,
) -> anyhow::Result<i64> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT OR IGNORE INTO accounts (google_sub, email, display_name, created_at, provider)
         VALUES (?1, ?1, 'On this device', ?2, ?3)",
    )
    .bind(LOCAL_ACCOUNT_SUB)
    .bind(now_ms)
    .bind(LOCAL_PROVIDER)
    .execute(&mut *tx)
    .await?;
    let account_id: i64 = sqlx::query_scalar("SELECT id FROM accounts WHERE google_sub = ?1")
        .bind(LOCAL_ACCOUNT_SUB)
        .fetch_one(&mut *tx)
        .await?;
    let calendar_id: i64 = sqlx::query_scalar(
        "INSERT INTO calendars
            (account_id, google_id, summary, color_hex, timezone, access_role, selected,
             is_primary, sync_enabled, supports_events, supports_tasks)
         VALUES (?1, 'local:list:' || lower(hex(randomblob(8))), ?2, ?3, ?4, 'owner', 1, 0, 0, 0, 1)
         RETURNING id",
    )
    .bind(account_id)
    .bind(name)
    .bind(color)
    .bind(timezone)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(calendar_id)
}

/// Renames a list on this device. Its own name, not a label over a
/// provider's (`set_label_override`): there is no provider name underneath
/// to go back to, so any override is cleared with it.
pub async fn rename_local_list(pool: &SqlitePool, calendar_id: i64, name: &str) -> anyhow::Result<()> {
    if !is_local_calendar(pool, calendar_id).await? {
        anyhow::bail!("only a list on this device can be renamed here");
    }
    sqlx::query("UPDATE calendars SET summary = ?2, label_override = NULL WHERE id = ?1")
        .bind(calendar_id)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(())
}

/// Deletes a list on this device and every task on it, answering how many
/// tasks went. Refuses anything else: a server's list is the server's to
/// delete, and removing its row here would only bring it back on the next
/// sync.
pub async fn delete_local_list(pool: &SqlitePool, calendar_id: i64) -> anyhow::Result<u64> {
    if !is_local_calendar(pool, calendar_id).await? {
        anyhow::bail!("only a list on this device can be deleted here");
    }
    let mut tx = pool.begin().await?;
    let tasks = sqlx::query("DELETE FROM tasks WHERE calendar_id = ?1")
        .bind(calendar_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    sqlx::query("DELETE FROM calendars WHERE id = ?1")
        .bind(calendar_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(tasks)
}

/// Whether this calendar is one of the on-this-device lists: the question
/// every task write asks before reaching for a server.
pub async fn is_local_calendar(pool: &SqlitePool, calendar_id: i64) -> anyhow::Result<bool> {
    let provider: Option<String> = sqlx::query_scalar(
        "SELECT a.provider FROM calendars c JOIN accounts a ON a.id = c.account_id WHERE c.id = ?1",
    )
    .bind(calendar_id)
    .fetch_optional(pool)
    .await?;
    Ok(provider.as_deref() == Some(LOCAL_PROVIDER))
}

/// Creates (or re-opens) a read-only WebCal subscription: one `webcal` account row
/// per normalized feed URL holding one `reader` calendar.
///
/// Idempotent on the normalized URL: re-subscribing the same feed returns the
/// existing calendar rather than duplicating it. The provider's fields
/// (`summary`, `timezone`) update; the user's (`selected`, `sync_enabled`,
/// colour/label overrides) are never touched — the same contract the CalDAV
/// and Google upserts keep.
///
/// `feed_url` must already be normalized (`webcal://` → `https://`, trimmed);
/// `account_sub` is `webcal:<feed_url>`, mirroring `caldav:<email>`, so the same
/// address subscribed twice cannot collide on one row.
pub async fn ensure_webcal_subscription(
    pool: &SqlitePool,
    feed_url: &str,
    name: &str,
    timezone: &str,
    now_ms: i64,
) -> anyhow::Result<(i64, i64)> {
    let sub = format!("webcal:{feed_url}");
    let mut tx = pool.begin().await?;
    let account_id: i64 = sqlx::query_scalar(
        "INSERT INTO accounts (google_sub, email, display_name, created_at, provider, server_url)
         VALUES (?1, ?2, ?2, ?3, 'webcal', ?4)
         ON CONFLICT (google_sub) DO UPDATE SET
            email = excluded.email, display_name = excluded.display_name,
            server_url = excluded.server_url
         RETURNING id",
    )
    .bind(&sub)
    .bind(name)
    .bind(now_ms)
    .bind(feed_url)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO calendars
            (account_id, google_id, summary, timezone, access_role, selected, is_primary,
             sync_enabled, supports_events, supports_tasks)
         VALUES (?1, ?2, ?3, ?4, 'reader', 1, 0, 1, 1, 0)
         ON CONFLICT (account_id, google_id) DO UPDATE SET
            summary = excluded.summary,
            timezone = excluded.timezone,
            access_role = 'reader',
            supports_events = 1,
            supports_tasks = 0",
    )
    .bind(account_id)
    .bind(feed_url)
    .bind(name)
    .bind(timezone)
    .execute(&mut *tx)
    .await?;
    let calendar_id: i64 = sqlx::query_scalar(
        "SELECT id FROM calendars WHERE account_id = ?1 AND google_id = ?2",
    )
    .bind(account_id)
    .bind(feed_url)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((account_id, calendar_id))
}

pub async fn set_selected(pool: &SqlitePool, id: i64, on: bool) -> anyhow::Result<()> {
    sqlx::query("UPDATE calendars SET selected = ?2 WHERE id = ?1")
        .bind(id)
        .bind(on as i64)
        .execute(pool)
        .await?;
    Ok(())
}

/// Sets or clears a calendar's colour override.
///
/// `None` **clears** it, which is not the same as storing whatever Google
/// currently uses: a cleared calendar follows Google's colour from then on,
/// including when Google changes it. See `0006_calendar_colour.sql`.
///
/// Nothing about this reaches Google. It is a display preference of this
/// install's, and the phone, the web UI and anyone sharing the calendar are
/// untouched by it.
pub async fn set_color_override(
    pool: &SqlitePool,
    id: i64,
    hex: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE calendars SET color_override = ?2 WHERE id = ?1")
        .bind(id)
        .bind(hex)
        .execute(pool)
        .await?;
    Ok(())
}

/// Local display label; blank restores the provider's current name.
pub async fn set_label_override(pool: &SqlitePool, id: i64, label: Option<&str>) -> anyhow::Result<()> {
    let label = label.map(str::trim).filter(|s| !s.is_empty());
    anyhow::ensure!(label.is_none_or(|s| s.chars().count() <= 200 && !s.chars().any(char::is_control)),
        "calendar label must be at most 200 characters without control characters");
    let result = sqlx::query("UPDATE calendars SET label_override = ?2 WHERE id = ?1")
        .bind(id).bind(label).execute(pool).await?;
    anyhow::ensure!(result.rows_affected() == 1, "calendar not found");
    Ok(())
}

/// Add or remove a calendar from syncing.
///
/// Turning it off deletes its events and its sync cursor: keeping stale rows
/// that never update again would grow the store for no benefit, and a stale
/// `syncToken` would make the next re-enable fetch an incremental diff against
/// events that are no longer there. Returns the number of events removed.
pub async fn set_sync_enabled(pool: &SqlitePool, id: i64, on: bool) -> anyhow::Result<u64> {
    let mut tx = pool.begin().await?;

    sqlx::query("UPDATE calendars SET sync_enabled = ?2 WHERE id = ?1")
        .bind(id)
        .bind(on as i64)
        .execute(&mut *tx)
        .await?;

    let removed = if on {
        0
    } else {
        // The invite ledger goes with the events — through them, so before
        // them. Left behind, a stale `invite_scan` row would make a re-added
        // calendar's backlog read as news, and orphaned notices could
        // silence a fresh invitation if SQLite ever reissued a rowid.
        sqlx::query(
            "DELETE FROM invite_notices WHERE event_id IN (
                SELECT id FROM events WHERE calendar_id = ?1)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM invite_scan WHERE calendar_id = ?1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let n = sqlx::query("DELETE FROM events WHERE calendar_id = ?1")
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        // AFTER the events delete, in the same transaction: the 0011 delete
        // trigger just recorded every one of those rows as "cancelled", and
        // a removed calendar is not a hundred cancellations.
        sqlx::query("DELETE FROM event_changes WHERE calendar_id = ?1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM sync_state WHERE calendar_id = ?1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        n
    };

    tx.commit().await?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{connect_memory, upsert_event, StoredEvent};

    async fn seed(pool: &SqlitePool) {
        sqlx::query("INSERT INTO accounts (google_sub, email, created_at)
                     VALUES ('s','me@x.com',0)").execute(pool).await.unwrap();
        for (gid, name, primary) in [("primary", "Work", 1), ("hols", "Holidays", 0)] {
            sqlx::query(
                "INSERT INTO calendars
                     (account_id, google_id, summary, color_hex, timezone, access_role, is_primary)
                 VALUES (1, ?1, ?2, '#5b8def', 'UTC', 'owner', ?3)")
                .bind(gid).bind(name).bind(primary)
                .execute(pool).await.unwrap();
        }
    }

    /// A pool with the two seeded calendars, which is what every colour test
    /// below starts from.
    async fn seeded() -> SqlitePool {
        let pool = connect_memory().await.unwrap();
        seed(&pool).await;
        pool
    }

    /// "New list", twice, then a rename and a delete — with the server
    /// calendars beside them untouched and unreachable.
    #[tokio::test]
    async fn lists_on_this_device_are_made_renamed_and_deleted_and_nothing_else_is() {
        let pool = seeded().await;
        let default = ensure_local_task_list(&pool, "Tasks on this device", "UTC", 0).await.unwrap();
        let groceries = create_local_list(&pool, "Groceries", "UTC", "#5aa84f", 1).await.unwrap();
        let books = create_local_list(&pool, "Books", "UTC", "#e2a03f", 2).await.unwrap();
        assert_ne!(groceries, books, "two presses, two lists");
        for id in [default, groceries, books] {
            assert!(is_local_calendar(&pool, id).await.unwrap());
        }
        let row: (String, String, i64, i64, i64) = sqlx::query_as(
            "SELECT summary, color_hex, supports_tasks, supports_events, selected FROM calendars WHERE id = ?1",
        )
        .bind(groceries).fetch_one(&pool).await.unwrap();
        assert_eq!(row, ("Groceries".into(), "#5aa84f".into(), 1, 0, 1));
        let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE provider = 'local'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(accounts, 1, "one on-this-device account holds them all");

        set_label_override(&pool, groceries, Some("Shopping")).await.unwrap();
        rename_local_list(&pool, groceries, "Food").await.unwrap();
        let (summary, label): (String, Option<String>) =
            sqlx::query_as("SELECT summary, label_override FROM calendars WHERE id = ?1")
                .bind(groceries).fetch_one(&pool).await.unwrap();
        assert_eq!((summary.as_str(), label), ("Food", None), "a rename is the list's own name");

        for i in 0..3 {
            sqlx::query("INSERT INTO tasks (calendar_id, uid, status, updated_at) VALUES (?1, ?2, 'needs-action', 0)")
                .bind(groceries).bind(format!("t{i}")).execute(&pool).await.unwrap();
        }
        assert_eq!(delete_local_list(&pool, groceries).await.unwrap(), 3);
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM calendars WHERE id = ?1")
            .bind(groceries).fetch_one(&pool).await.unwrap();
        assert_eq!(left, 0);

        let work = list_calendars(&pool).await.unwrap().into_iter().find(|c| c.summary == "Work").unwrap().id;
        assert!(rename_local_list(&pool, work, "Mine").await.is_err(), "a server's calendar is not renamed here");
        assert!(delete_local_list(&pool, work).await.is_err(), "nor deleted");
        assert!(list_calendars(&pool).await.unwrap().iter().any(|c| c.id == work));
    }

    #[tokio::test]
    async fn local_label_survives_provider_rename_and_resets() {
        let pool = seeded().await;
        let before = list_calendars(&pool).await.unwrap();
        let id = before[0].id;
        set_label_override(&pool, id, Some("  Jon Kinney  ")).await.unwrap();
        sqlx::query("UPDATE calendars SET summary = 'Provider renamed' WHERE id = ?")
            .bind(id).execute(&pool).await.unwrap();
        let rows = list_calendars(&pool).await.unwrap();
        let cal = rows.iter().find(|c| c.id == id).unwrap();
        assert_eq!(cal.summary, "Jon Kinney");
        assert_eq!(cal.provider_summary, "Provider renamed");
        assert_eq!(cal.label_override.as_deref(), Some("Jon Kinney"));
        assert_eq!(rows.iter().find(|c| c.id == before[1].id).unwrap().summary, before[1].summary);
        assert!(set_label_override(&pool, id, Some(&"x".repeat(201))).await.is_err());
        assert!(set_label_override(&pool, id, Some("bad\nlabel")).await.is_err());
        set_label_override(&pool, id, Some("  ")).await.unwrap();
        let rows = list_calendars(&pool).await.unwrap();
        let cal = rows.iter().find(|c| c.id == id).unwrap();
        assert_eq!(cal.summary, "Provider renamed");
        assert_eq!(cal.label_override, None);
    }

    fn ev(cal: i64, gid: &str) -> StoredEvent {
        StoredEvent {
            id: 0, calendar_id: cal, google_id: gid.into(), summary: Some("x".into()),
            location: None, start_utc: 1000, end_utc: 2000,
            start_tz: "UTC".into(), end_tz: "UTC".into(), is_all_day: false,
            recurrence: None, recurring_event_id: None, original_start_utc: None,
            status: "confirmed".into(), self_response: None, conference_uri: None,
            color_hex: None, calendar_timezone: "UTC".into(),
            description: None, etag: None, sequence: 0, organizer_email: None,
            guests_can_modify: false,
            attendees: Vec::new(),
            reminders: Default::default(), calendar_default_reminders: Vec::new(),
        }
    }

    #[tokio::test]
    async fn calendar_for_write_returns_the_calendars_google_id_role_email_and_timezone() {
        let pool = connect_memory().await.unwrap();
        seed(&pool).await;
        let (google_id, access_role, account_email, timezone) =
            calendar_for_write(&pool, 1).await.unwrap().expect("calendar exists");
        assert_eq!(google_id, "primary");
        assert_eq!(access_role, "owner");
        assert_eq!(account_email, "me@x.com");
        assert_eq!(timezone, "UTC", "seed()'s calendar timezone");
    }

    #[tokio::test]
    async fn calendar_for_write_returns_none_for_an_unknown_id() {
        let pool = connect_memory().await.unwrap();
        // Seeded first, so this proves the `WHERE` clause actually filters —
        // against a bare empty pool the same assertion would pass whether or
        // not the query filters on `id` at all.
        seed(&pool).await;
        assert!(calendar_for_write(&pool, 999).await.unwrap().is_none());
    }

    /// Crosses calendar and account ids the same way
    /// `omacal_store::events::tests::seed_two_accounts` does — calendar id 1
    /// belongs to account 2, calendar id 2 belongs to account 1 — so a join on
    /// the wrong column (`a.id = c.id` instead of `a.id = c.account_id`)
    /// still returns *an* account row, just the wrong one, rather than
    /// failing loudly. Returns the id of the calendar owned by account "a".
    async fn seed_two_accounts(pool: &SqlitePool) -> i64 {
        sqlx::query("INSERT INTO accounts (google_sub, email, created_at) VALUES ('a','a@x',0)")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO accounts (google_sub, email, created_at) VALUES ('b','b@x',0)")
            .execute(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
             VALUES (2, 'cal-on-b', 'On B', 'UTC', 'reader')",
        ).execute(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
             VALUES (1, 'cal-on-a', 'On A', 'Pacific/Auckland', 'owner')",
        ).execute(pool).await.unwrap();

        let (id, account_id): (i64, i64) =
            sqlx::query_as("SELECT id, account_id FROM calendars WHERE google_id = 'cal-on-a'")
                .fetch_one(pool)
                .await
                .unwrap();
        debug_assert_ne!(
            id, account_id,
            "cal-on-a: the calendar's id must not equal its own account_id, or a join on the \
             wrong column still returns the right row"
        );
        id
    }

    /// The account JOIN, unbound without this: `seed_two_accounts` crosses
    /// calendar and account ids so joining `accounts` on the wrong column
    /// (e.g. `a.id = c.id` instead of `a.id = c.account_id`) still returns an
    /// account, just account "b"'s instead of "a"'s. `account_email` is what
    /// `access_token_for` uses to pick a Google account's token, so a wrong
    /// join here means creating the event under a different Google account
    /// than the calendar's own.
    #[tokio::test]
    async fn calendar_for_write_resolves_the_owning_account_not_one_sharing_an_id() {
        let pool = connect_memory().await.unwrap();
        let cal_on_a = seed_two_accounts(&pool).await;

        let (google_id, _access_role, account_email, _timezone) =
            calendar_for_write(&pool, cal_on_a).await.unwrap().expect("calendar exists");
        assert_eq!(google_id, "cal-on-a");
        assert_eq!(
            account_email, "a@x",
            "must be account a's own email, not account b's merely sharing an id with cal_on_a"
        );
    }

    #[tokio::test]
    async fn listing_returns_every_calendar_with_its_account() {
        let pool = connect_memory().await.unwrap();
        seed(&pool).await;
        let cals = list_calendars(&pool).await.unwrap();
        assert_eq!(cals.len(), 2);
        assert!(cals.iter().all(|c| c.account_email == "me@x.com"));
        assert!(cals.iter().any(|c| c.is_primary));
    }

    /// Re-subscribing the same feed must not duplicate rows: the normalized
    /// URL is the identity. Provider fields update; the user's own
    /// (`selected`, `sync_enabled`, overrides) are never touched — the same
    /// contract the CalDAV and Google upserts keep.
    #[tokio::test]
    async fn webcal_resubscribe_is_idempotent_and_keeps_user_fields() {
        let pool = connect_memory().await.unwrap();
        let url = "https://example.com/feed.ics";
        let (a1, c1) = ensure_webcal_subscription(&pool, url, "Feed", "UTC", 0)
            .await
            .unwrap();
        set_selected(&pool, c1, false).await.unwrap();
        set_sync_enabled(&pool, c1, false).await.unwrap();
        set_label_override(&pool, c1, Some("Mine")).await.unwrap();

        let (a2, c2) = ensure_webcal_subscription(&pool, url, "Renamed", "Pacific/Auckland", 1)
            .await
            .unwrap();
        assert_eq!((a1, c1), (a2, c2), "same URL, same rows");
        let row: (String, String, String, i64, i64, Option<String>) = sqlx::query_as(
            "SELECT summary, timezone, access_role, selected, sync_enabled, label_override
             FROM calendars WHERE id = ?1",
        )
        .bind(c1)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, "Renamed", "provider field updates");
        assert_eq!(row.1, "Pacific/Auckland", "provider field updates");
        assert_eq!(row.2, "reader", "feeds stay read-only");
        assert_eq!((row.3, row.4, row.5.as_deref()), (0, 0, Some("Mine")), "user fields untouched");
        let (email, display, provider): (String, String, String) =
            sqlx::query_as("SELECT email, display_name, provider FROM accounts WHERE id = ?1")
                .bind(a1)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!((email.as_str(), display.as_str(), provider.as_str()), ("Renamed", "Renamed", "webcal"));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM calendars WHERE google_id = ?1")
            .bind(url)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "no duplicate calendar row");
    }

    #[tokio::test]
    async fn listing_puts_the_primary_calendar_first() {
        let pool = connect_memory().await.unwrap();
        seed(&pool).await;
        let cals = list_calendars(&pool).await.unwrap();
        assert!(cals[0].is_primary, "the primary calendar should lead the list");
    }

    /// The event form has nothing but this column to decide which calendars it
    /// may offer, and the query dropped it until Task 9 — every calendar
    /// reached the UI looking equally writable, subscribed holiday calendars
    /// included. `seed_two_accounts` seeds one `owner` and one `reader`, so a
    /// query that hard-coded either value still fails here.
    #[tokio::test]
    async fn listing_reports_each_calendars_access_role() {
        let pool = connect_memory().await.unwrap();
        seed_two_accounts(&pool).await;
        let cals = list_calendars(&pool).await.unwrap();
        let role = |summary: &str| {
            cals.iter()
                .find(|c| c.summary == summary)
                .unwrap_or_else(|| panic!("no calendar named {summary}"))
                .access_role
                .clone()
        };
        assert_eq!(role("On A"), "owner");
        assert_eq!(role("On B"), "reader");
    }

    #[tokio::test]
    async fn hiding_a_calendar_keeps_its_events_and_its_sync() {
        let pool = connect_memory().await.unwrap();
        seed(&pool).await;
        upsert_event(&pool, &ev(1, "a")).await.unwrap();

        set_selected(&pool, 1, false).await.unwrap();

        let row = list_calendars(&pool).await.unwrap();
        let c = row.iter().find(|c| c.id == 1).unwrap();
        assert!(!c.selected, "hidden");
        assert!(c.sync_enabled, "still syncing — hiding is not removing");

        let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE calendar_id = 1")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(kept, 1, "hiding must not discard data");
    }

    #[tokio::test]
    async fn removing_a_calendar_deletes_its_events_but_keeps_the_row() {
        let pool = connect_memory().await.unwrap();
        seed(&pool).await;
        upsert_event(&pool, &ev(1, "a")).await.unwrap();
        upsert_event(&pool, &ev(1, "b")).await.unwrap();
        upsert_event(&pool, &ev(2, "c")).await.unwrap();

        let removed = set_sync_enabled(&pool, 1, false).await.unwrap();
        assert_eq!(removed, 2);

        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE calendar_id = 1")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(left, 0);

        let other: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE calendar_id = 2")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(other, 1, "removing one calendar must not touch another");

        // The row survives so the calendar can be re-enabled, and so the next
        // calendarList.list cannot silently re-import what was removed.
        let still_listed = list_calendars(&pool).await.unwrap();
        assert!(still_listed.iter().any(|c| c.id == 1 && !c.sync_enabled));
    }

    #[tokio::test]
    async fn re_enabling_a_calendar_leaves_it_ready_to_refetch() {
        let pool = connect_memory().await.unwrap();
        seed(&pool).await;
        upsert_event(&pool, &ev(1, "a")).await.unwrap();
        // Plant a sync cursor, or the assertion below passes whether or not the
        // code deletes anything — `seed` creates no sync_state row of its own.
        sqlx::query(
            "INSERT INTO sync_state (calendar_id, sync_token, window_start, window_end)
             VALUES (1, 'stale-token', 0, 0)")
            .execute(&pool).await.unwrap();

        set_sync_enabled(&pool, 1, false).await.unwrap();
        set_sync_enabled(&pool, 1, true).await.unwrap();

        let c = list_calendars(&pool).await.unwrap();
        let c = c.iter().find(|c| c.id == 1).unwrap();
        assert!(c.sync_enabled);
        // The cursor went with the events. Keeping it would make the next sync
        // ask Google for a diff against events that are no longer here, and the
        // calendar would come back empty until the token went stale on its own.
        let tok: Option<String> = sqlx::query_scalar(
            "SELECT sync_token FROM sync_state WHERE calendar_id = 1")
            .fetch_optional(&pool).await.unwrap().flatten();
        assert!(tok.is_none(), "a re-enabled calendar must resync from scratch");
    }

    #[tokio::test]
    async fn toggling_an_unknown_calendar_is_not_an_error() {
        let pool = connect_memory().await.unwrap();
        seed(&pool).await;
        // The popover can race a sync that removed a calendar; a no-op beats a
        // failure the user cannot act on.
        assert!(set_selected(&pool, 999, false).await.is_ok());
        assert_eq!(set_sync_enabled(&pool, 999, false).await.unwrap(), 0);
    }

    /// **The colour to draw is the override when there is one, and Google's
    /// otherwise** — resolved in the `SELECT`, which is why nothing downstream
    /// of it knows an override exists.
    #[tokio::test]
    async fn an_override_is_the_colour_the_list_reports() {
        let pool = seeded().await;
        let before = list_calendars(&pool).await.unwrap();
        let id = before[0].id;
        assert_eq!(before[0].color_hex.as_deref(), Some("#5b8def"), "Google's own");
        assert_eq!(before[0].color_override, None, "nothing chosen yet");

        set_color_override(&pool, id, Some("#e2a03f")).await.unwrap();

        let after = list_calendars(&pool).await.unwrap();
        assert_eq!(after[0].color_hex.as_deref(), Some("#e2a03f"), "the colour to draw");
        assert_eq!(after[0].color_override.as_deref(), Some("#e2a03f"), "and it is a choice");
    }

    /// **Clearing is not the same as choosing Google's current colour**, and
    /// this is the test that says so: after a clear, a *change on Google's
    /// side* is followed. Store the colour instead of a NULL and the calendar
    /// silently stops following it, with nothing recording which the user
    /// meant.
    #[tokio::test]
    async fn a_cleared_override_follows_google_again_even_when_google_changes() {
        let pool = seeded().await;
        let id = list_calendars(&pool).await.unwrap()[0].id;
        set_color_override(&pool, id, Some("#e2a03f")).await.unwrap();

        set_color_override(&pool, id, None).await.unwrap();
        let cleared = list_calendars(&pool).await.unwrap();
        assert_eq!(cleared[0].color_hex.as_deref(), Some("#5b8def"));
        assert_eq!(cleared[0].color_override, None);

        // The half that a stored-copy implementation passes right up until
        // here: Google recolours the calendar on its next sign-in.
        sqlx::query("UPDATE calendars SET color_hex = '#b58900' WHERE id = ?1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();

        let followed = list_calendars(&pool).await.unwrap();
        assert_eq!(
            followed[0].color_hex.as_deref(),
            Some("#b58900"),
            "a cleared calendar must follow Google's colour, including its changes",
        );
    }

    /// And an override does **not** follow Google — that is what choosing one
    /// means, and without this the rule above is satisfied by never storing an
    /// override at all.
    #[tokio::test]
    async fn an_override_survives_google_changing_its_own_colour() {
        let pool = seeded().await;
        let id = list_calendars(&pool).await.unwrap()[0].id;
        set_color_override(&pool, id, Some("#e2a03f")).await.unwrap();

        sqlx::query("UPDATE calendars SET color_hex = '#b58900' WHERE id = ?1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(list_calendars(&pool).await.unwrap()[0].color_hex.as_deref(), Some("#e2a03f"));
    }
}

/// Removes one account and everything it owned: its calendars, their events,
/// tasks and sync cursors through the schema's cascades — and its
/// fired-reminder records by hand, because that table deliberately carries no
/// foreign key (the scheduler prunes it by time instead). Left behind, those
/// rows could suppress a reminder if SQLite ever handed a new event the same
/// rowid. Returns how many account rows went (0 or 1) — the caller decides
/// whether 0 is an error.
///
/// Lives in the store rather than in a command so the removal chain is
/// testable against the real schema: a migration that broke a CASCADE would
/// otherwise only be discovered by a user with orphaned rows.
pub async fn delete_account(pool: &SqlitePool, account_id: i64) -> anyhow::Result<u64> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "DELETE FROM fired_reminders WHERE event_id IN (
            SELECT e.id FROM events e
            JOIN calendars c ON c.id = e.calendar_id
            WHERE c.account_id = ?1)",
    )
    .bind(account_id)
    .execute(&mut *tx)
    .await?;
    // The invite ledger carries no foreign keys either, for the same reason
    // as fired_reminders — so the same by-hand sweep, and through the events
    // before the cascade takes them.
    sqlx::query(
        "DELETE FROM invite_notices WHERE event_id IN (
            SELECT e.id FROM events e
            JOIN calendars c ON c.id = e.calendar_id
            WHERE c.account_id = ?1)",
    )
    .bind(account_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM invite_scan WHERE calendar_id IN (
            SELECT id FROM calendars WHERE account_id = ?1)",
    )
    .bind(account_id)
    .execute(&mut *tx)
    .await?;
    // The calendar ids, captured before the cascade takes the rows they
    // would be read from — the change-ledger sweep below needs them *after*
    // the cascade's deletes have fired the 0011 trigger.
    let calendar_ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM calendars WHERE account_id = ?1")
            .bind(account_id)
            .fetch_all(&mut *tx)
            .await?;
    let gone = sqlx::query("DELETE FROM accounts WHERE id = ?1")
        .bind(account_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    // Same reasoning as set_sync_enabled's sweep: the cascade just recorded
    // every deleted event as "cancelled", and signing out is not that.
    for id in calendar_ids {
        sqlx::query("DELETE FROM event_changes WHERE calendar_id = ?1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(gone)
}

#[cfg(test)]
mod cascade_tests {
    use super::*;

    #[tokio::test]
    async fn deleting_an_account_cascades_through_everything_it_owned() {
        let pool = crate::connect_memory().await.unwrap();
        sqlx::query(
            "INSERT INTO accounts (google_sub, email, created_at, provider)
             VALUES ('caldav:x', 'x@x', 0, 'caldav')",
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role, supports_tasks)
             VALUES (1, 'url', 'Cal', 'UTC', 'owner', 1)",
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO events (calendar_id, google_id, start_utc, end_utc, start_tz, end_tz, updated_at)
             VALUES (1, 'ev', 0, 1, 'UTC', 'UTC', 0)",
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO tasks (calendar_id, uid, updated_at) VALUES (1, 't', 0)",
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO sync_state (calendar_id, sync_token, window_start, window_end)
             VALUES (1, 'ctag', 0, 1)",
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO fired_reminders (event_id, occurrence_ms, minutes, occurrence_end_ms, fired_at_ms)
             VALUES (1, 0, 5, 1, 0)",
        )
        .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO invite_notices (event_id, noticed_at_ms, posted) VALUES (1, 0, 1)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO invite_scan (calendar_id, seeded_at_ms) VALUES (1, 0)")
            .execute(&pool).await.unwrap();

        // An event the cascade will delete — whatever 0011's delete trigger
        // records about it, the sweep must take back out.
        assert_eq!(delete_account(&pool, 1).await.unwrap(), 1);
        for table in ["accounts", "calendars", "events", "tasks", "sync_state", "fired_reminders",
                      "invite_notices", "invite_scan", "event_changes"] {
            let n: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(&pool).await.unwrap();
            assert_eq!(n, 0, "{table} should be empty after the cascade");
        }
        assert_eq!(delete_account(&pool, 1).await.unwrap(), 0, "second delete finds nothing");
    }
}
