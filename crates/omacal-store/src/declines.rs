//! The organizer's side of the invitation ledger: who has declined a meeting
//! of yours, minus what you have already acknowledged.
//!
//! State-derived on purpose, like the invitation tray and unlike the
//! notification pass: there is no "new since" ledger here because there is
//! no toast — the tray shows the *current* set of unacknowledged declines,
//! a guest who un-declines drops out on the next sync by construction, and
//! the only stored fact is the acknowledgement itself
//! (0010_dismissed_declines.sql).

use sqlx::{Row, SqlitePool};

/// One guest who declined one of the user's own meetings.
#[derive(Debug, Clone, PartialEq)]
pub struct DeclinedGuest {
    pub calendar_id: i64,
    /// The series' google_id for a recurring meeting (whichever row carried
    /// the decline), the event's own for a one-off — the stable key the
    /// dismissal is recorded under.
    pub gid: String,
    pub email: String,
    pub display_name: Option<String>,
    pub summary: Option<String>,
    /// The earliest still-relevant occurrence that carries this decline —
    /// what the tray's "when" line shows.
    pub start_utc: i64,
    pub end_utc: i64,
    pub is_all_day: bool,
    pub calendar_timezone: String,
    pub color_hex: Option<String>,
}

/// Every unacknowledged decline on the user's own upcoming meetings, one row
/// per guest per meeting, earliest occurrence first.
///
/// "The user's own" is decided the only way the store can: the event's
/// organizer is the calendar it lives on (`organizer_email = c.google_id`) —
/// the organizer's copy of a meeting lives on the organizer's calendar, and
/// for a primary calendar the google_id is the account's address.
///
/// Exception rows are deliberately *included* (a per-occurrence decline
/// lives on one) and then collapsed: one guest declining a series often
/// shows on the master and several materialised exceptions at once, and the
/// tray should say "Victor declined this meeting" once, dated at the
/// earliest occurrence still ahead, not once per row.
pub async fn declined_guests(
    pool: &SqlitePool,
    now_ms: i64,
) -> anyhow::Result<Vec<DeclinedGuest>> {
    let rows = sqlx::query(
        "SELECT e.calendar_id, e.google_id, e.recurring_event_id, e.summary,
                e.start_utc, e.end_utc, e.is_all_day, e.attendees_json,
                c.timezone, COALESCE(c.color_override, c.color_hex) AS color_hex
           FROM events e
           JOIN calendars c ON c.id = e.calendar_id
          WHERE e.organizer_email = c.google_id
            AND e.status != 'cancelled'
            AND (e.end_utc > ?1 OR e.recurrence IS NOT NULL)
            AND c.selected = 1
            AND c.sync_enabled = 1
          ORDER BY e.start_utc, e.id",
    )
    .bind(now_ms)
    .fetch_all(pool)
    .await?;

    let dismissed: std::collections::HashSet<(i64, String, String)> =
        sqlx::query("SELECT calendar_id, gid, email FROM dismissed_declines")
            .fetch_all(pool)
            .await?
            .iter()
            .map(|r| (r.get("calendar_id"), r.get("gid"), r.get("email")))
            .collect();

    let mut out: Vec<DeclinedGuest> = Vec::new();
    let mut seen: std::collections::HashSet<(i64, String, String)> =
        std::collections::HashSet::new();
    for row in &rows {
        let attendees: Vec<crate::Attendee> = row
            .get::<Option<String>, _>("attendees_json")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let calendar_id: i64 = row.get("calendar_id");
        let gid: String = row
            .get::<Option<String>, _>("recurring_event_id")
            .unwrap_or_else(|| row.get("google_id"));
        for a in attendees {
            if a.response_status != "declined" || a.is_self {
                continue;
            }
            let key = (calendar_id, gid.clone(), a.email.clone());
            if dismissed.contains(&key) || !seen.insert(key) {
                // Rows arrive earliest-first, so the first sighting of a
                // (meeting, guest) pair is the one the tray dates it by.
                continue;
            }
            out.push(DeclinedGuest {
                calendar_id,
                gid: gid.clone(),
                email: a.email,
                display_name: a.display_name,
                summary: row.get("summary"),
                start_utc: row.get("start_utc"),
                end_utc: row.get("end_utc"),
                is_all_day: row.get::<i64, _>("is_all_day") != 0,
                calendar_timezone: row.get("timezone"),
                color_hex: row.get("color_hex"),
            });
        }
    }
    Ok(out)
}

/// Records the ×: this guest's decline of this meeting is acknowledged and
/// stops being listed. Idempotent.
pub async fn dismiss_decline(
    pool: &SqlitePool,
    calendar_id: i64,
    gid: &str,
    email: &str,
    now_ms: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO dismissed_declines (calendar_id, gid, email, dismissed_at_ms)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (calendar_id, gid, email) DO NOTHING",
    )
    .bind(calendar_id)
    .bind(gid)
    .bind(email)
    .bind(now_ms)
    .execute(pool)
    .await?;
    Ok(())
}

/// The tray's "dismiss all": acknowledges every decline currently listed —
/// exactly the set [`declined_guests`] answers, so the two cannot disagree
/// about what "all" means. Returns how many were acknowledged.
pub async fn dismiss_all_declines(pool: &SqlitePool, now_ms: i64) -> anyhow::Result<usize> {
    let all = declined_guests(pool, now_ms).await?;
    for d in &all {
        dismiss_decline(pool, d.calendar_id, &d.gid, &d.email, now_ms).await?;
    }
    Ok(all.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{upsert_event, Attendee, StoredEvent};

    const NOW: i64 = 1_786_352_400_000;
    const HOUR: i64 = 3_600_000;

    async fn seeded_pool() -> SqlitePool {
        let pool = crate::connect_memory().await.unwrap();
        sqlx::query(
            "INSERT INTO accounts (google_sub, email, created_at, provider)
             VALUES ('s', 'me@x.com', 0, 'google')",
        )
        .execute(&pool).await.unwrap();
        // The primary calendar: google_id IS the address, which is what makes
        // `organizer_email = c.google_id` mean "my own meeting".
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
             VALUES (1, 'me@x.com', 'Personal', 'Europe/Sofia', 'owner')",
        )
        .execute(&pool).await.unwrap();
        pool
    }

    fn guest(email: &str, status: &str) -> Attendee {
        Attendee {
            email: email.into(),
            display_name: Some(email.split('@').next().unwrap().into()),
            response_status: status.into(),
            optional: false,
            is_self: false,
            comment: None,
            additional_guests: 0,
        }
    }

    fn me(status: &str) -> Attendee {
        Attendee { is_self: true, ..guest("me@x.com", status) }
    }

    fn mine(gid: &str, start: i64, attendees: Vec<Attendee>) -> StoredEvent {
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
            self_response: Some("accepted".into()),
            conference_uri: None,
            color_hex: None,
            calendar_timezone: "Europe/Sofia".into(),
            description: None,
            etag: None,
            sequence: 0,
            organizer_email: Some("me@x.com".into()),
            guests_can_modify: false,
            transparency: "opaque".into(), visibility: "default".into(),
            attendees,
            reminders: Default::default(),
            calendar_default_reminders: Vec::new(),
        }
    }

    #[tokio::test]
    async fn a_declined_guest_on_my_meeting_lists_until_acknowledged() {
        let pool = seeded_pool().await;
        upsert_event(&pool, &mine("m1", NOW + HOUR, vec![
            me("accepted"), guest("ana@x.com", "declined"), guest("bo@x.com", "accepted"),
        ])).await.unwrap();

        let found = declined_guests(&pool, NOW).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].email, "ana@x.com");
        assert_eq!(found[0].gid, "m1");

        dismiss_decline(&pool, 1, "m1", "ana@x.com", NOW).await.unwrap();
        dismiss_decline(&pool, 1, "m1", "ana@x.com", NOW + 1).await.unwrap(); // idempotent
        assert!(declined_guests(&pool, NOW).await.unwrap().is_empty());
    }

    /// Somebody else's meeting is theirs to worry about — a decline on an
    /// event whose organizer is not the calendar it lives on never lists.
    /// And my own row declining someone's meeting is my answer, not news.
    #[tokio::test]
    async fn other_peoples_meetings_and_my_own_answers_are_not_news() {
        let pool = seeded_pool().await;
        let mut theirs = mine("theirs", NOW + HOUR, vec![
            guest("ana@x.com", "declined"), me("declined"),
        ]);
        theirs.organizer_email = Some("ana@x.com".into());
        upsert_event(&pool, &theirs).await.unwrap();

        upsert_event(&pool, &mine("mine-ok", NOW + HOUR, vec![
            me("declined"), guest("bo@x.com", "accepted"),
        ])).await.unwrap();

        assert!(declined_guests(&pool, NOW).await.unwrap().is_empty());
    }

    /// One guest, one meeting, one row — however many exception rows carry
    /// the decline — dated at the earliest occurrence.
    #[tokio::test]
    async fn a_series_decline_collapses_to_one_row_at_the_earliest_occurrence() {
        let pool = seeded_pool().await;
        let mut master = mine("ser", NOW + HOUR, vec![me("accepted"), guest("ana@x.com", "declined")]);
        master.recurrence = Some("RRULE:FREQ=WEEKLY".into());
        upsert_event(&pool, &master).await.unwrap();

        let mut exc = mine("ser_x1", NOW + 8 * 24 * HOUR, vec![me("accepted"), guest("ana@x.com", "declined")]);
        exc.recurring_event_id = Some("ser".into());
        upsert_event(&pool, &exc).await.unwrap();

        let found = declined_guests(&pool, NOW).await.unwrap();
        assert_eq!(found.len(), 1, "master and exception collapsed");
        assert_eq!(found[0].start_utc, NOW + HOUR, "dated at the earliest row");

        // One × acknowledges the meeting, exceptions included.
        dismiss_decline(&pool, 1, "ser", "ana@x.com", NOW).await.unwrap();
        assert!(declined_guests(&pool, NOW).await.unwrap().is_empty());
    }

    /// Dismiss-all acknowledges exactly what is listed — several guests
    /// across several meetings in one stroke — and nothing that is not.
    #[tokio::test]
    async fn dismiss_all_clears_the_list_and_reports_the_count() {
        let pool = seeded_pool().await;
        upsert_event(&pool, &mine("m1", NOW + HOUR, vec![
            me("accepted"), guest("ana@x.com", "declined"), guest("bo@x.com", "declined"),
        ])).await.unwrap();
        upsert_event(&pool, &mine("m2", NOW + 2 * HOUR, vec![
            me("accepted"), guest("cid@x.com", "declined"),
        ])).await.unwrap();

        assert_eq!(dismiss_all_declines(&pool, NOW).await.unwrap(), 3);
        assert!(declined_guests(&pool, NOW).await.unwrap().is_empty());
        // A decline arriving after the stroke is new news, not pre-forgiven.
        upsert_event(&pool, &mine("m3", NOW + 3 * HOUR, vec![
            me("accepted"), guest("dee@x.com", "declined"),
        ])).await.unwrap();
        assert_eq!(declined_guests(&pool, NOW).await.unwrap().len(), 1);
    }

    /// The list is state-derived: a guest who un-declines disappears with no
    /// bookkeeping, and a meeting already over stops being worth a row.
    #[tokio::test]
    async fn an_undeclined_guest_and_a_past_meeting_drop_out() {
        let pool = seeded_pool().await;
        upsert_event(&pool, &mine("m1", NOW + HOUR, vec![me("accepted"), guest("ana@x.com", "declined")]))
            .await.unwrap();
        upsert_event(&pool, &mine("m1", NOW + HOUR, vec![me("accepted"), guest("ana@x.com", "accepted")]))
            .await.unwrap();
        upsert_event(&pool, &mine("gone", NOW - 2 * HOUR, vec![me("accepted"), guest("bo@x.com", "declined")]))
            .await.unwrap();

        assert!(declined_guests(&pool, NOW).await.unwrap().is_empty());
    }

    /// A hidden calendar is muted here exactly as it is everywhere else.
    #[tokio::test]
    async fn a_hidden_calendars_declines_are_muted() {
        let pool = seeded_pool().await;
        upsert_event(&pool, &mine("m1", NOW + HOUR, vec![me("accepted"), guest("ana@x.com", "declined")]))
            .await.unwrap();
        sqlx::query("UPDATE calendars SET selected = 0 WHERE id = 1")
            .execute(&pool).await.unwrap();
        assert!(declined_guests(&pool, NOW).await.unwrap().is_empty());
    }
}
