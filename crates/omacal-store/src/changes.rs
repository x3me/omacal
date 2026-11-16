//! Reading the change ledger 0011's triggers write: which meetings the user
//! attends moved or were cancelled, minus what they have acknowledged.
//!
//! The triggers record every time-change and cancellation dumbly; the
//! policy lives here, where a test can reach it: only meetings somebody
//! *else* organizes (the user's own edits are not news — and times on other
//! people's meetings can only change via sync, so this one filter excludes
//! every local write), only what is still ahead, only shown calendars, and
//! a move that has since moved *back* is no move at all.

use crate::events::Attendee;
use sqlx::{Row, SqlitePool};

/// One meeting that changed under the user, ready for the tray.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangedMeeting {
    pub calendar_id: i64,
    pub gid: String,
    /// `moved` | `cancelled`.
    pub kind: String,
    pub summary: Option<String>,
    pub is_all_day: bool,
    pub old_start_utc: i64,
    /// Absent for a moved occurrence, whose ledger row only knows the
    /// vacated start.
    pub old_end_utc: Option<i64>,
    /// The times as they now stand — `None` for a cancellation, which has
    /// no "now".
    pub new_start_utc: Option<i64>,
    pub new_end_utc: Option<i64>,
    pub calendar_timezone: String,
    pub color_hex: Option<String>,
    /// The live row's id, for answering the new time — `None` for a
    /// cancellation, which has nothing left to answer.
    pub event_id: Option<i64>,
    /// Whether an answer covers the whole series or this occurrence: a
    /// moved *master* carrying a rule is the series moving, a moved
    /// exception is one occurrence. Meaningless when `event_id` is `None`.
    pub respond_all: bool,
    /// Raw materials for the caller's `can_respond` gate, same as the
    /// pending-invites reader: the policy lives one layer up, beside the
    /// popover's own.
    pub provider: String,
    pub access_role: String,
    pub attendees: Vec<Attendee>,
}

/// Every unacknowledged change worth telling the user about, soonest first
/// (by the time they knew, for a cancellation; by the new time, for a move).
pub async fn changed_meetings(
    pool: &SqlitePool,
    now_ms: i64,
) -> anyhow::Result<Vec<ChangedMeeting>> {
    let rows = sqlx::query(
        "SELECT ec.calendar_id, ec.gid, ec.series_gid, ec.kind, ec.summary,
                ec.organizer_email, ec.is_all_day, ec.old_start_utc, ec.old_end_utc,
                e.id AS event_id, e.recurring_event_id AS cur_parent,
                e.attendees_json,
                e.start_utc AS cur_start, e.end_utc AS cur_end, e.status AS cur_status,
                e.recurrence AS cur_recurrence, e.summary AS cur_summary,
                e.organizer_email AS cur_organizer,
                c.google_id AS cal_gid, c.timezone, c.access_role, a.provider,
                COALESCE(c.color_override, c.color_hex) AS color_hex
           FROM event_changes ec
           JOIN calendars c ON c.id = ec.calendar_id
           JOIN accounts a ON a.id = c.account_id
           LEFT JOIN events e
             ON e.calendar_id = ec.calendar_id AND e.google_id = ec.gid
          WHERE ec.dismissed = 0
            AND c.selected = 1
            AND c.sync_enabled = 1
          ORDER BY ec.old_start_utc, ec.gid",
    )
    .fetch_all(pool)
    .await?;

    let mut out: Vec<ChangedMeeting> = Vec::new();
    for row in &rows {
        let cal_gid: String = row.get("cal_gid");
        // The current row's organizer where one still exists, the snapshot's
        // otherwise. Unknown organizer reads as "mine" — silence over noise.
        let organizer: Option<String> = row
            .get::<Option<String>, _>("cur_organizer")
            .or_else(|| row.get("organizer_email"));
        match organizer {
            Some(o) if o != cal_gid => {}
            _ => continue,
        }

        let kind: String = row.get("kind");
        let cur_status: Option<String> = row.get("cur_status");
        let summary: Option<String> = row
            .get::<Option<String>, _>("cur_summary")
            .or_else(|| row.get("summary"));
        let base = ChangedMeeting {
            calendar_id: row.get("calendar_id"),
            gid: row.get("gid"),
            kind: kind.clone(),
            summary,
            is_all_day: row.get::<i64, _>("is_all_day") != 0,
            old_start_utc: row.get("old_start_utc"),
            old_end_utc: row.get("old_end_utc"),
            new_start_utc: None,
            new_end_utc: None,
            calendar_timezone: row.get("timezone"),
            color_hex: row.get("color_hex"),
            event_id: None,
            respond_all: false,
            provider: row.get("provider"),
            access_role: row.get("access_role"),
            attendees: Vec::new(),
        };

        if kind == "moved" {
            // A move needs a live row to say where the meeting is *now* —
            // and if that row has since been cancelled or deleted, the
            // cancellation entry is the one that speaks.
            let (Some(cur_start), Some(cur_end)) = (
                row.get::<Option<i64>, _>("cur_start"),
                row.get::<Option<i64>, _>("cur_end"),
            ) else {
                continue;
            };
            if cur_status.as_deref() == Some("cancelled") {
                continue;
            }
            // Moved back to where the user last knew it: no move at all.
            if cur_start == base.old_start_utc
                && base.old_end_utc.is_none_or(|oe| oe == cur_end)
            {
                continue;
            }
            let recurring: Option<String> = row.get("cur_recurrence");
            if cur_end <= now_ms && recurring.is_none() {
                continue; // already over; nothing to adjust for
            }
            let parent: Option<String> = row.get("cur_parent");
            out.push(ChangedMeeting {
                new_start_utc: Some(cur_start),
                new_end_utc: Some(cur_end),
                event_id: row.get("event_id"),
                respond_all: recurring.is_some() && parent.is_none(),
                attendees: row
                    .get::<Option<String>, _>("attendees_json")
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default(),
                ..base
            });
        } else {
            // A cancellation whose row resurfaced un-cancelled was
            // un-cancelled; state wins over the ledger.
            if row.get::<Option<i64>, _>("cur_start").is_some()
                && cur_status.as_deref() != Some("cancelled")
            {
                continue;
            }
            if base.old_end_utc.unwrap_or(base.old_start_utc) <= now_ms {
                continue; // the vacated slot has passed
            }
            out.push(base);
        }
    }

    // Tombstones carry no title of their own; borrow the master's.
    for m in &mut out {
        if m.summary.is_some() {
            continue;
        }
        let series: Option<String> = sqlx::query_scalar(
            "SELECT series_gid FROM event_changes WHERE calendar_id = ?1 AND gid = ?2",
        )
        .bind(m.calendar_id)
        .bind(&m.gid)
        .fetch_one(pool)
        .await?;
        if let Some(series) = series {
            m.summary = sqlx::query_scalar(
                "SELECT summary FROM events WHERE calendar_id = ?1 AND google_id = ?2",
            )
            .bind(m.calendar_id)
            .bind(&series)
            .fetch_optional(pool)
            .await?
            .flatten();
        }
    }
    Ok(out)
}

/// The ×. A later change to the same meeting un-dismisses (the trigger
/// resets the flag), which is the deliberate difference from a decline's
/// acknowledgement: a meeting that moves again is new news.
pub async fn dismiss_change(
    pool: &SqlitePool,
    calendar_id: i64,
    gid: &str,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE event_changes SET dismissed = 1 WHERE calendar_id = ?1 AND gid = ?2")
        .bind(calendar_id)
        .bind(gid)
        .execute(pool)
        .await?;
    Ok(())
}

/// The section's "Dismiss all", for one kind: resolved against the same
/// query that fills the list, so the stroke acknowledges exactly what was
/// shown. Returns the count.
pub async fn dismiss_all_changes(
    pool: &SqlitePool,
    kind: &str,
    now_ms: i64,
) -> anyhow::Result<usize> {
    let listed: Vec<ChangedMeeting> = changed_meetings(pool, now_ms)
        .await?
        .into_iter()
        .filter(|m| m.kind == kind)
        .collect();
    for m in &listed {
        dismiss_change(pool, m.calendar_id, &m.gid).await?;
    }
    Ok(listed.len())
}

/// Erases the ledger's memory of one meeting — the user's own delete flow
/// calls this, because leaving a meeting on purpose must not come back as
/// "cancelled" news. Takes the exceptions with the series.
pub async fn forget_changes(
    pool: &SqlitePool,
    calendar_id: i64,
    gid: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM event_changes
          WHERE calendar_id = ?1 AND (gid = ?2 OR series_gid = ?2)",
    )
    .bind(calendar_id)
    .bind(gid)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{delete_event, upsert_event, Attendee, StoredEvent};

    const NOW: i64 = 1_786_352_400_000;
    const HOUR: i64 = 3_600_000;

    async fn seeded_pool() -> SqlitePool {
        let pool = crate::connect_memory().await.unwrap();
        sqlx::query(
            "INSERT INTO accounts (google_sub, email, created_at, provider)
             VALUES ('s', 'me@x.com', 0, 'google')",
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
             VALUES (1, 'me@x.com', 'Personal', 'Europe/Sofia', 'owner')",
        )
        .execute(&pool).await.unwrap();
        pool
    }

    fn attended(gid: &str, start: i64) -> StoredEvent {
        StoredEvent {
            id: 0,
            calendar_id: 1,
            google_id: gid.into(),
            summary: Some("NVP sync".into()),
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
            self_response: Some("accepted".into()),
            conference_uri: None,
            color_hex: None,
            calendar_timezone: "Europe/Sofia".into(),
            description: None,
            etag: None,
            sequence: 0,
            // Somebody else's meeting — the shape the ledger reports on.
            organizer_email: Some("ana@x.com".into()),
            guests_can_modify: false,
            transparency: "opaque".into(), visibility: "default".into(),
            attendees: vec![Attendee {
                email: "me@x.com".into(),
                display_name: None,
                response_status: "accepted".into(),
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
    async fn a_moved_meeting_reports_from_and_to_until_acknowledged() {
        let pool = seeded_pool().await;
        upsert_event(&pool, &attended("m1", NOW + HOUR)).await.unwrap();
        upsert_event(&pool, &attended("m1", NOW + 5 * HOUR)).await.unwrap();

        let found = changed_meetings(&pool, NOW).await.unwrap();
        assert_eq!(found.len(), 1);
        let m = &found[0];
        assert_eq!(m.kind, "moved");
        assert_eq!(m.old_start_utc, NOW + HOUR);
        assert_eq!(m.new_start_utc, Some(NOW + 5 * HOUR));

        dismiss_change(&pool, 1, "m1").await.unwrap();
        assert!(changed_meetings(&pool, NOW).await.unwrap().is_empty());

        // Moving again after the × is new news — the trigger un-dismisses.
        upsert_event(&pool, &attended("m1", NOW + 9 * HOUR)).await.unwrap();
        let again = changed_meetings(&pool, NOW).await.unwrap();
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].old_start_utc, NOW + HOUR, "the from is the time last known");
        assert_eq!(again[0].new_start_utc, Some(NOW + 9 * HOUR));
    }

    /// My own meetings move because I moved them.
    #[tokio::test]
    async fn my_own_meetings_moves_are_not_news() {
        let pool = seeded_pool().await;
        let mut mine = attended("mine", NOW + HOUR);
        mine.organizer_email = Some("me@x.com".into());
        upsert_event(&pool, &mine).await.unwrap();
        mine.start_utc = NOW + 5 * HOUR;
        mine.end_utc = NOW + 6 * HOUR;
        upsert_event(&pool, &mine).await.unwrap();

        assert!(changed_meetings(&pool, NOW).await.unwrap().is_empty());
    }

    /// A move that came back is no move: the state, not the hops, is the
    /// story.
    #[tokio::test]
    async fn a_meeting_moved_and_moved_back_says_nothing() {
        let pool = seeded_pool().await;
        upsert_event(&pool, &attended("m1", NOW + HOUR)).await.unwrap();
        upsert_event(&pool, &attended("m1", NOW + 5 * HOUR)).await.unwrap();
        upsert_event(&pool, &attended("m1", NOW + HOUR)).await.unwrap();

        assert!(changed_meetings(&pool, NOW).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_deleted_meeting_reports_cancelled_with_its_snapshot() {
        let pool = seeded_pool().await;
        upsert_event(&pool, &attended("gone", NOW + 2 * HOUR)).await.unwrap();
        delete_event(&pool, 1, "gone").await.unwrap();

        let found = changed_meetings(&pool, NOW).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, "cancelled");
        assert_eq!(found[0].summary.as_deref(), Some("NVP sync"));
        assert_eq!(found[0].new_start_utc, None);
    }

    /// A cancelled occurrence arrives as a tombstone exception — title-less,
    /// so the tray borrows the master's.
    #[tokio::test]
    async fn a_cancelled_occurrence_borrows_the_masters_title() {
        let pool = seeded_pool().await;
        let mut master = attended("ser", NOW + HOUR);
        master.recurrence = Some("RRULE:FREQ=WEEKLY".into());
        upsert_event(&pool, &master).await.unwrap();

        let mut tomb = attended("ser_x", NOW + 8 * 24 * HOUR);
        tomb.summary = None;
        tomb.status = "cancelled".into();
        tomb.recurring_event_id = Some("ser".into());
        tomb.original_start_utc = Some(NOW + 8 * 24 * HOUR);
        upsert_event(&pool, &tomb).await.unwrap();

        let found = changed_meetings(&pool, NOW).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, "cancelled");
        assert_eq!(found[0].summary.as_deref(), Some("NVP sync"), "borrowed");
    }

    /// A moved occurrence materialises as an *insert*; its original start is
    /// the from.
    #[tokio::test]
    async fn a_moved_occurrence_reports_from_its_original_slot() {
        let pool = seeded_pool().await;
        let mut exc = attended("ser_x", NOW + 30 * HOUR);
        exc.recurring_event_id = Some("ser".into());
        exc.original_start_utc = Some(NOW + 26 * HOUR);
        upsert_event(&pool, &exc).await.unwrap();

        let found = changed_meetings(&pool, NOW).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, "moved");
        assert_eq!(found[0].old_start_utc, NOW + 26 * HOUR);
        assert_eq!(found[0].new_start_utc, Some(NOW + 30 * HOUR));
    }

    /// The materials an answer needs (2026-08-21): a moved row carries the
    /// live event's id and whether an RSVP covers the series or the one
    /// occurrence; a cancellation carries nothing to answer.
    #[tokio::test]
    async fn a_moved_row_carries_what_an_answer_needs() {
        let pool = seeded_pool().await;

        // A one-off: answerable, occurrence-scoped by shape.
        upsert_event(&pool, &attended("one", NOW + HOUR)).await.unwrap();
        upsert_event(&pool, &attended("one", NOW + 2 * HOUR)).await.unwrap();
        // A moved series master: the whole series moved, so the answer does.
        let mut master = attended("ser", NOW + 3 * HOUR);
        master.recurrence = Some("RRULE:FREQ=WEEKLY".into());
        upsert_event(&pool, &master).await.unwrap();
        master.start_utc = NOW + 4 * HOUR;
        master.end_utc = NOW + 5 * HOUR;
        upsert_event(&pool, &master).await.unwrap();
        // A moved exception: one occurrence, one answer.
        let mut exc = attended("ser_x", NOW + 30 * HOUR);
        exc.recurring_event_id = Some("ser".into());
        exc.original_start_utc = Some(NOW + 26 * HOUR);
        upsert_event(&pool, &exc).await.unwrap();
        // A cancellation: nothing left to answer.
        let mut gone = attended("gone", NOW + 8 * HOUR);
        upsert_event(&pool, &gone).await.unwrap();
        gone.status = "cancelled".into();
        upsert_event(&pool, &gone).await.unwrap();

        let found = changed_meetings(&pool, NOW).await.unwrap();
        let by_gid = |g: &str| found.iter().find(|m| m.gid == g).unwrap();

        let one = by_gid("one");
        assert!(one.event_id.is_some());
        assert!(!one.respond_all);
        assert_eq!(one.provider, "google");
        assert_eq!(one.access_role, "owner");
        assert!(one.attendees.iter().any(|a| a.is_self));

        assert!(by_gid("ser").respond_all, "a moved master answers the series");
        assert!(!by_gid("ser_x").respond_all, "a moved exception answers one occurrence");
        assert!(by_gid("ser_x").event_id.is_some());

        let gone = by_gid("gone");
        assert_eq!(gone.kind, "cancelled");
        assert_eq!(gone.event_id, None, "a cancellation offers no answer");
    }

    #[tokio::test]
    async fn dismiss_all_is_per_kind_and_forget_erases() {
        let pool = seeded_pool().await;
        upsert_event(&pool, &attended("m1", NOW + HOUR)).await.unwrap();
        upsert_event(&pool, &attended("m1", NOW + 5 * HOUR)).await.unwrap();
        upsert_event(&pool, &attended("m2", NOW + 2 * HOUR)).await.unwrap();
        delete_event(&pool, 1, "m2").await.unwrap();

        assert_eq!(dismiss_all_changes(&pool, "cancelled", NOW).await.unwrap(), 1);
        let left = changed_meetings(&pool, NOW).await.unwrap();
        assert_eq!(left.len(), 1, "the move survived the cancelled stroke");
        assert_eq!(left[0].kind, "moved");

        // The user's own delete flow: no news about a departure they chose.
        forget_changes(&pool, 1, "m1").await.unwrap();
        assert!(changed_meetings(&pool, NOW).await.unwrap().is_empty());
    }

    /// Past changes are not news: a move of something already over, a
    /// cancellation whose slot has passed.
    #[tokio::test]
    async fn changes_to_the_past_say_nothing() {
        let pool = seeded_pool().await;
        upsert_event(&pool, &attended("old", NOW - 5 * HOUR)).await.unwrap();
        upsert_event(&pool, &attended("old", NOW - 3 * HOUR)).await.unwrap();
        upsert_event(&pool, &attended("old2", NOW - 2 * HOUR)).await.unwrap();
        delete_event(&pool, 1, "old2").await.unwrap();

        assert!(changed_meetings(&pool, NOW).await.unwrap().is_empty());
    }
}
