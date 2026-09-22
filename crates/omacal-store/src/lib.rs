use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

pub mod calendars;
pub mod changes;
pub mod contacts;
pub mod declines;
pub mod events;
pub mod invites;
pub mod reminders;
pub mod tasks;
pub use calendars::{
    create_local_list, delete_local_list, ensure_webcal_subscription, ensure_local_task_list,
    is_local_calendar, rename_local_list, WEBCAL_PROVIDER, LOCAL_PROVIDER, calendar_for_write,
    delete_account, list_calendars, set_color_override, set_label_override, set_selected,
    set_sync_enabled, CalendarRow,
};
pub use changes::{
    changed_meetings, dismiss_all_changes, dismiss_change, forget_changes, ChangedMeeting,
};
pub use contacts::{known_contacts, replace_account_contacts, KnownContact, StoredContact};
pub use declines::{declined_guests, dismiss_all_declines, dismiss_decline, DeclinedGuest};
pub use invites::{
    mark_invites_seeded, pending_invites, record_invite_notice, unanswered_invites,
    unseeded_calendars, InviteCandidate,
};
pub use reminders::{fired_keys, prune_fired, record_fired};
pub use tasks::{
    completed_tasks_for_ui, delete_task, delete_tasks_not_in, fired_task_keys, mark_task_status,
    move_task, prune_fired_tasks, record_fired_task, task_by_id, tasks_for_ui, tasks_to_announce,
    update_task_fields, upsert_task,
    StoredTask, TaskRow,
};
pub use events::{
    delete_event, delete_series, event_by_id, event_for_detail, event_for_write, events_in_window,
    exceptions_from,
    known_guests, known_locations, move_series_to_calendar, search_events, upsert_event, Attendee,
    KnownGuest, KnownLocation, Reminder, Reminders, StoredEvent,
};

/// Opens an existing database read-only: no create, no migrations, no
/// permission sweep — nothing about the file changes because it was read.
/// The CLI's door (`src-tauri/src/cli.rs`): it must be able to answer
/// beside a running app without ever being able to damage what the app
/// maintains, and WAL already lets readers ride beside the writer.
pub async fn connect_readonly(url: &str) -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(url)?
        .read_only(true)
        .foreign_keys(true);
    Ok(SqlitePoolOptions::new().max_connections(1).connect_with(opts).await?)
}

/// Opens (creating if needed) the database at `url` and runs migrations.
pub async fn connect(url: &str) -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .foreign_keys(true)
        // WAL keeps the UI's reads from blocking the sync task's writes.
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new().max_connections(5).connect_with(opts).await?;
    // Between connect and migrate, deliberately: the file exists now, and
    // SQLite creates `-wal`/`-shm` with the main file's permissions — so a
    // database tightened *before* the first write bears sidecars that are
    // born tight, and the explicit sweep below catches whatever an earlier
    // release already left at the umask default.
    harden_permissions(url);
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Owner-only permissions on the database and both its sidecars — all three
/// in one place, deliberately: these are cached calendars, readable by every
/// local account until now (0644 under the usual umask), and hardening the
/// database while forgetting the WAL leaves the same rows readable in the
/// log. Calix shipped exactly that split — db in one release, `-wal`/`-shm`
/// as a security fix the day after — and this sweep exists so nobody has to
/// re-learn it here.
///
/// Best-effort and silent: the files may sit on a filesystem without POSIX
/// modes, and refusing to open someone's calendar over a failed chmod would
/// cost more than it protects.
#[cfg(unix)]
fn harden_permissions(url: &str) {
    use std::os::unix::fs::PermissionsExt;
    let Some(db) = db_file_of(url) else { return };
    for suffix in ["", "-wal", "-shm"] {
        let path = format!("{db}{suffix}");
        if std::path::Path::new(&path).exists() {
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

#[cfg(not(unix))]
fn harden_permissions(_url: &str) {}

/// The filesystem path inside a `sqlite://…` URL, `None` for every other
/// shape — `:memory:` most of all, which has no file to chmod.
#[cfg(unix)]
fn db_file_of(url: &str) -> Option<&str> {
    url.strip_prefix("sqlite://").filter(|p| !p.is_empty() && !p.contains(":memory:"))
}

/// An isolated in-memory database for tests. `max_connections(1)` is required:
/// each new connection to `:memory:` would otherwise get its own empty database.
pub async fn connect_memory() -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(opts).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrations_apply_to_a_fresh_database() {
        let pool = connect_memory().await.unwrap();
        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let names: Vec<String> = tables.into_iter().map(|t| t.0).collect();
        for expected in ["accounts", "attendees", "calendars", "events",
                         "mutations", "settings", "sync_state"] {
            assert!(names.contains(&expected.to_string()), "missing table {expected}");
        }
    }

    /// A fresh database comes up owner-only, and so do the WAL and shm files
    /// SQLite creates for it — the inheritance half of `harden_permissions`.
    /// The WAL's existence is asserted, not assumed: a test that shrugged at
    /// a missing sidecar could not witness the claim it is here to make.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_fresh_database_and_its_sidecars_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("omacal-store-fresh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("t.db");

        let pool = connect(&format!("sqlite://{}", db.display())).await.unwrap();
        let mode = |p: &std::path::Path| {
            std::fs::metadata(p).map(|m| m.permissions().mode() & 0o777)
        };
        assert_eq!(mode(&db).unwrap(), 0o600, "the database itself");
        let wal = dir.join("t.db-wal");
        assert_eq!(
            mode(&wal).expect("the WAL must exist after migrations, or this test is blind"),
            0o600,
            "the write-ahead log carries the same rows as the database"
        );

        drop(pool);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The retroactive half: files an earlier release created at the umask
    /// default (0644 — world-readable) are tightened on the next open, the
    /// sidecar included.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_existing_world_readable_database_is_tightened_on_open() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("omacal-store-old-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("t.db");
        // Zero bytes is a valid fresh database to SQLite, which makes it a
        // faithful stand-in for any pre-hardening install.
        for path in [&db, &dir.join("t.db-wal")] {
            std::fs::write(path, b"").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }

        let pool = connect(&format!("sqlite://{}", db.display())).await.unwrap();
        for suffix in ["", "-wal"] {
            let path = dir.join(format!("t.db{suffix}"));
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "t.db{suffix} was left readable by other accounts");
        }

        drop(pool);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The URL parser feeding the chmod: real files in, every other shape out.
    #[cfg(unix)]
    #[test]
    fn only_file_backed_urls_are_hardened() {
        assert_eq!(db_file_of("sqlite:///home/x/omacal.db"), Some("/home/x/omacal.db"));
        assert_eq!(db_file_of("sqlite::memory:"), None);
        assert_eq!(db_file_of("sqlite://"), None);
        assert_eq!(db_file_of("postgres://elsewhere/db"), None);
    }

    #[tokio::test]
    async fn foreign_keys_are_enforced() {
        let pool = connect_memory().await.unwrap();
        let res = sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
             VALUES (999, 'x', 'x', 'UTC', 'owner')",
        )
        .execute(&pool)
        .await;
        assert!(res.is_err(), "insert with a dangling account_id should fail");
    }
}
