//! The invitation ledger: which unanswered invitations exist that have not
//! been announced, and which calendars have had their pre-existing backlog
//! seeded so it never gets announced at all.
//!
//! Policy lives with the caller (`src-tauri`'s invite pass): this module only
//! answers the two questions a pass asks — "what is new?" and "is this
//! calendar's history already swallowed?" — and records the answers it is
//! told. See 0008_invite_notices.sql for why there is no prune here.

use crate::events::Attendee;
use sqlx::{Row, SqlitePool};

/// One unanswered invitation the pass has not yet dealt with, carrying
/// everything the notification and its Accept action need — nothing here
/// requires a second read at posting time.
#[derive(Debug, Clone, PartialEq)]
pub struct InviteCandidate {
    pub event_id: i64,
    pub calendar_id: i64,
    pub summary: Option<String>,
    pub start_utc: i64,
    pub is_all_day: bool,
    pub organizer_email: Option<String>,
    /// The owning calendar's zone — what an all-day date resolves against.
    pub calendar_timezone: String,
    /// Hidden calendars (`selected = 0`) stay in the result on purpose: the
    /// caller skips them *without recording them*, so an invitation that is
    /// still unanswered when the calendar is shown again gets announced then
    /// rather than never.
    pub calendar_selected: bool,
    /// The owning account's provider — `google` | `caldav` | `webcal` | `local`.
    /// Only Google can send an RSVP; every other provider lists buttonless.
    pub provider: String,
    pub access_role: String,
    pub attendees: Vec<Attendee>,
    /// The colour the calendar draws in — override first, Google's second,
    /// the same `COALESCE` the event queries use.
    pub color_hex: Option<String>,
    pub end_utc: i64,
}

/// The query both readers below share; `extra_clause` is the one line they
/// differ by. "Invitation" means: the user's own attendee row says
/// `needsAction`, on a non-cancelled master row (an exception inherits its
/// series' answer — surfacing it separately would ring twice for one
/// meeting), on a calendar that is still syncing. Past events are skipped —
/// except that a recurring master outlives its first occurrence's end, so a
/// rule keeps a series eligible on its own.
async fn invites_where(
    pool: &SqlitePool,
    now_ms: i64,
    extra_clause: &str,
) -> anyhow::Result<Vec<InviteCandidate>> {
    let sql = format!(
        "SELECT e.id, e.calendar_id, e.summary, e.start_utc, e.end_utc, e.is_all_day,
                e.organizer_email, e.attendees_json,
                c.timezone, c.selected, c.access_role, a.provider,
                COALESCE(c.color_override, c.color_hex) AS color_hex
           FROM events e
           JOIN calendars c ON c.id = e.calendar_id
           JOIN accounts a ON a.id = c.account_id
          WHERE e.self_response = 'needsAction'
            AND e.status != 'cancelled'
            AND e.recurring_event_id IS NULL
            AND (e.end_utc > ?1 OR e.recurrence IS NOT NULL)
            AND c.sync_enabled = 1
            -- Never a meeting the user organizes, whatever their own
            -- attendee row says: an event created with the creator in its
            -- guest list (a paste did this, 2026-08-20) comes back from
            -- Google with a needsAction self row, and announcing it invites
            -- the user to their own meeting. Same identity rule as
            -- `declines.rs`: the organizer's copy lives on the organizer's
            -- calendar, whose google_id is the address that organizes it.
            AND (e.organizer_email IS NULL OR e.organizer_email != c.google_id)
            {extra_clause}
          ORDER BY e.start_utc, e.id",
    );
    let rows = sqlx::query(&sql).bind(now_ms).fetch_all(pool).await?;

    Ok(rows
        .iter()
        .map(|row| InviteCandidate {
            event_id: row.get("id"),
            calendar_id: row.get("calendar_id"),
            summary: row.get("summary"),
            start_utc: row.get("start_utc"),
            end_utc: row.get("end_utc"),
            is_all_day: row.get::<i64, _>("is_all_day") != 0,
            organizer_email: row.get("organizer_email"),
            calendar_timezone: row.get("timezone"),
            calendar_selected: row.get::<i64, _>("selected") != 0,
            provider: row.get("provider"),
            access_role: row.get("access_role"),
            attendees: row
                .get::<Option<String>, _>("attendees_json")
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default(),
            color_hex: row.get("color_hex"),
        })
        .collect())
}

/// Every unanswered invitation not yet in `invite_notices`, oldest start
/// first — what the announcement pass consumes. Hidden calendars stay in the
/// result on purpose; see [`InviteCandidate::calendar_selected`].
pub async fn unanswered_invites(
    pool: &SqlitePool,
    now_ms: i64,
) -> anyhow::Result<Vec<InviteCandidate>> {
    invites_where(pool, now_ms, "AND e.id NOT IN (SELECT event_id FROM invite_notices)").await
}

/// Every unanswered invitation on a *shown* calendar, announced or not —
/// what the app's own invitation tray lists. The ledger deliberately plays
/// no part: it records what was said out loud, and a missed notification is
/// precisely the case this list exists for (2026-08-17, after the first
/// live click test's toast evaporated unseen).
pub async fn pending_invites(
    pool: &SqlitePool,
    now_ms: i64,
) -> anyhow::Result<Vec<InviteCandidate>> {
    invites_where(pool, now_ms, "AND c.selected = 1").await
}

/// Syncing calendars whose backlog has not been seeded yet — each one's
/// current candidates must be recorded without being announced.
pub async fn unseeded_calendars(pool: &SqlitePool) -> anyhow::Result<Vec<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM calendars
          WHERE sync_enabled = 1
            AND id NOT IN (SELECT calendar_id FROM invite_scan)
          ORDER BY id",
    )
    .fetch_all(pool)
    .await?)
}

/// Records that one invitation has been dealt with — `posted` says whether a
/// notification actually went out or the row was seeded silently. Idempotent,
/// so a pass that crashes between posting and recording can run again without
/// failing on the half-written row (it will re-post once; the other order
/// would swallow the invitation forever).
pub async fn record_invite_notice(
    pool: &SqlitePool,
    event_id: i64,
    posted: bool,
    now_ms: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO invite_notices (event_id, noticed_at_ms, posted)
         VALUES (?1, ?2, ?3)
         ON CONFLICT (event_id) DO NOTHING",
    )
    .bind(event_id)
    .bind(now_ms)
    .bind(posted as i64)
    .execute(pool)
    .await?;
    Ok(())
}

/// Marks one calendar's backlog as seeded, after its rows are in the ledger.
pub async fn mark_invites_seeded(
    pool: &SqlitePool,
    calendar_id: i64,
    now_ms: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO invite_scan (calendar_id, seeded_at_ms)
         VALUES (?1, ?2)
         ON CONFLICT (calendar_id) DO NOTHING",
    )
    .bind(calendar_id)
    .bind(now_ms)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{connect_memory, upsert_event, StoredEvent};

    const NOW: i64 = 1_786_352_400_000;
    const HOUR: i64 = 3_600_000;

    async fn seeded_pool() -> SqlitePool {
        let pool = connect_memory().await.unwrap();
        sqlx::query(
            "INSERT INTO accounts (google_sub, email, created_at, provider)
             VALUES ('s', 'me@x.com', 0, 'google')",
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
             VALUES (1, 'primary', 'Work', 'Europe/Sofia', 'owner')",
        )
        .execute(&pool).await.unwrap();
        pool
    }

    fn event(gid: &str, start: i64) -> StoredEvent {
        StoredEvent {
            id: 0,
            calendar_id: 1,
            google_id: gid.into(),
            summary: Some("Planning".into()),
            location: None,
            start_utc: start,
            end_utc: start + HOUR,
            start_tz: "Europe/Sofia".into(),
            end_tz: "Europe/Sofia".into(),
            is_all_day: false,
            recurrence: None,
            recurring_event_id: None,
            original_start_utc: None,
            status: "confirmed".into(),
            self_response: Some("needsAction".into()),
            conference_uri: None,
            color_hex: None,
            calendar_timezone: "Europe/Sofia".into(),
            description: None,
            etag: None,
            sequence: 0,
            organizer_email: Some("ana@x.com".into()),
            guests_can_modify: false,
            attendees: vec![Attendee {
                email: "me@x.com".into(),
                display_name: None,
                response_status: "needsAction".into(),
                optional: false,
                is_self: true,
                comment: None,
                additional_guests: 0,
            }],
            reminders: Default::default(),
            calendar_default_reminders: Vec::new(),
        }
    }

    #[tokio::test]
    async fn an_unanswered_future_invite_is_a_candidate_once() {
        let pool = seeded_pool().await;
        let id = upsert_event(&pool, &event("inv-1", NOW + HOUR)).await.unwrap();

        let found = unanswered_invites(&pool, NOW).await.unwrap();
        assert_eq!(found.len(), 1);
        let c = &found[0];
        assert_eq!(c.event_id, id);
        assert_eq!(c.summary.as_deref(), Some("Planning"));
        assert_eq!(c.provider, "google");
        assert_eq!(c.access_role, "owner");
        assert!(c.calendar_selected);
        assert!(c.attendees.iter().any(|a| a.is_self));

        record_invite_notice(&pool, id, true, NOW).await.unwrap();
        assert!(unanswered_invites(&pool, NOW).await.unwrap().is_empty(),
                "a noticed invitation must never come back");
    }

    #[tokio::test]
    async fn a_meeting_the_user_organizes_is_never_an_invitation() {
        // Whatever the self row says: an event created with its creator in
        // the guest list (a paste did this, 2026-08-20) comes back from
        // Google carrying a needsAction self row on a meeting the user
        // organizes, and the pass rang for it — an invitation to their own
        // copy. The organizer's copy lives on the organizer's calendar, so
        // `organizer_email = c.google_id` is the identity, the same rule
        // `declines.rs` reads "my meeting" by.
        let pool = seeded_pool().await;
        sqlx::query("UPDATE calendars SET google_id = 'me@x.com' WHERE id = 1")
            .execute(&pool).await.unwrap();

        let mut own = event("own-1", NOW + HOUR);
        own.organizer_email = Some("me@x.com".into());
        upsert_event(&pool, &own).await.unwrap();
        assert!(unanswered_invites(&pool, NOW).await.unwrap().is_empty(),
                "an event the user organizes must never ring as an invitation");

        // The control, or the clause proves nothing: the same shape with
        // somebody else's name on it is still a candidate.
        upsert_event(&pool, &event("inv-2", NOW + HOUR)).await.unwrap();
        assert_eq!(unanswered_invites(&pool, NOW).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn answered_past_cancelled_and_exception_rows_are_not_candidates() {
        let pool = seeded_pool().await;

        let mut answered = event("answered", NOW + HOUR);
        answered.self_response = Some("accepted".into());
        upsert_event(&pool, &answered).await.unwrap();

        let mut solo = event("solo", NOW + HOUR);
        solo.self_response = None; // no guests at all
        upsert_event(&pool, &solo).await.unwrap();

        upsert_event(&pool, &event("gone-by", NOW - 2 * HOUR)).await.unwrap();

        let mut cancelled = event("cancelled", NOW + HOUR);
        cancelled.status = "cancelled".into();
        upsert_event(&pool, &cancelled).await.unwrap();

        let mut exception = event("series_x", NOW + HOUR);
        exception.recurring_event_id = Some("series".into());
        upsert_event(&pool, &exception).await.unwrap();

        assert!(unanswered_invites(&pool, NOW).await.unwrap().is_empty());
    }

    /// A recurring master's `end_utc` is its first occurrence's end, which
    /// may be long past while the series is still very much alive.
    #[tokio::test]
    async fn a_recurring_master_stays_a_candidate_past_its_first_occurrence() {
        let pool = seeded_pool().await;
        let mut series = event("series", NOW - 24 * HOUR);
        series.recurrence = Some("RRULE:FREQ=WEEKLY".into());
        upsert_event(&pool, &series).await.unwrap();

        assert_eq!(unanswered_invites(&pool, NOW).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_hidden_calendars_invite_is_reported_as_such() {
        let pool = seeded_pool().await;
        upsert_event(&pool, &event("inv-1", NOW + HOUR)).await.unwrap();
        sqlx::query("UPDATE calendars SET selected = 0 WHERE id = 1")
            .execute(&pool).await.unwrap();

        let found = unanswered_invites(&pool, NOW).await.unwrap();
        assert_eq!(found.len(), 1, "hidden is the caller's decision, not this query's");
        assert!(!found[0].calendar_selected);
    }

    #[tokio::test]
    async fn a_disabled_calendars_invite_is_no_candidate() {
        let pool = seeded_pool().await;
        upsert_event(&pool, &event("inv-1", NOW + HOUR)).await.unwrap();
        sqlx::query("UPDATE calendars SET sync_enabled = 0 WHERE id = 1")
            .execute(&pool).await.unwrap();

        assert!(unanswered_invites(&pool, NOW).await.unwrap().is_empty());
    }

    /// The tray's list and the announcer's worklist part ways on exactly two
    /// clauses, both asserted here: the ledger silences the announcer but
    /// never the tray, and hiding a calendar empties the tray but leaves the
    /// announcer's (deliberately broader) view intact.
    #[tokio::test]
    async fn the_tray_ignores_the_ledger_and_honours_hiding() {
        let pool = seeded_pool().await;
        let id = upsert_event(&pool, &event("inv-1", NOW + HOUR)).await.unwrap();

        record_invite_notice(&pool, id, true, NOW).await.unwrap();
        assert!(unanswered_invites(&pool, NOW).await.unwrap().is_empty(),
                "announced once means announced");
        let pending = pending_invites(&pool, NOW).await.unwrap();
        assert_eq!(pending.len(), 1, "still unanswered, so still in the tray");
        assert_eq!(pending[0].end_utc, NOW + 2 * HOUR);
        assert!(pending[0].color_hex.is_none(), "no colour seeded on this calendar");

        sqlx::query("UPDATE calendars SET selected = 0 WHERE id = 1")
            .execute(&pool).await.unwrap();
        assert!(pending_invites(&pool, NOW).await.unwrap().is_empty(),
                "a hidden calendar is muted in the app too");
        assert_eq!(unanswered_invites(&pool, NOW).await.unwrap().len(), 0,
                   "(ledgered either way)");
    }

    /// The field failure of 2026-08-17, reproduced: delete a noticed
    /// invitation, let SQLite hand its rowid to a brand-new one, and the
    /// stale notice must not silence it. The 0009 trigger is what passes
    /// this; before it, the new invitation simply never announced.
    #[tokio::test]
    async fn a_reused_rowid_does_not_inherit_the_old_events_notice() {
        let pool = seeded_pool().await;
        let id = upsert_event(&pool, &event("doomed", NOW + HOUR)).await.unwrap();
        record_invite_notice(&pool, id, true, NOW).await.unwrap();
        crate::delete_event(&pool, 1, "doomed").await.unwrap();

        let reused = upsert_event(&pool, &event("fresh-invite", NOW + 2 * HOUR)).await.unwrap();
        assert_eq!(reused, id, "the premise: SQLite reuses the freed rowid");
        let found = unanswered_invites(&pool, NOW).await.unwrap();
        assert_eq!(found.len(), 1, "a stale notice silenced a brand-new invitation");
        assert_eq!(found[0].summary.as_deref(), Some("Planning"));
    }

    /// The trigger walks every deletion path — `delete_series` included.
    #[tokio::test]
    async fn a_series_delete_takes_its_notices_with_it() {
        let pool = seeded_pool().await;
        let mut series = event("series", NOW + HOUR);
        series.recurrence = Some("RRULE:FREQ=WEEKLY".into());
        let id = upsert_event(&pool, &series).await.unwrap();
        record_invite_notice(&pool, id, true, NOW).await.unwrap();

        crate::delete_series(&pool, 1, "series").await.unwrap();
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invite_notices")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(left, 0);
    }

    #[tokio::test]
    async fn seeding_is_once_per_calendar_and_idempotent() {
        let pool = seeded_pool().await;
        assert_eq!(unseeded_calendars(&pool).await.unwrap(), vec![1]);

        mark_invites_seeded(&pool, 1, NOW).await.unwrap();
        mark_invites_seeded(&pool, 1, NOW + 1).await.unwrap(); // no conflict error
        assert!(unseeded_calendars(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn recording_a_notice_twice_keeps_the_first() {
        let pool = seeded_pool().await;
        let id = upsert_event(&pool, &event("inv-1", NOW + HOUR)).await.unwrap();
        record_invite_notice(&pool, id, false, NOW).await.unwrap();
        record_invite_notice(&pool, id, true, NOW + 1).await.unwrap();

        let (at, posted): (i64, i64) = sqlx::query_as(
            "SELECT noticed_at_ms, posted FROM invite_notices WHERE event_id = ?1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((at, posted), (NOW, 0), "the first record stands");
    }
}
