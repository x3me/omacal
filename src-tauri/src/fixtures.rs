use omacal_store::StoredEvent;
use sqlx::SqlitePool;

const DAY_MS: i64 = 24 * 3_600_000;
const MIN: i64 = 60_000;

/// The demo account's marker. Seeding removes and rewrites only this account,
/// so it can never disturb real synced data.
const DEMO_SUB: &str = "demo@omacal.local";

/// Demo mode is opt-in and explicit. It also uses a separate database file
/// (see `lib.rs`), so fake events can never reach the real store.
pub fn demo_mode() -> bool {
    std::env::var("OMACAL_SEED_DEMO").map(|v| v == "1").unwrap_or(false)
}

/// "Excitel weekly"'s description — the only field in this file exercising
/// `descriptionSegments`' link detection in demo mode, alongside a bare
/// `<b>` tag proving the popover shows it as text, not markup.
const EXCITEL_WEEKLY_DESCRIPTION: &str =
    "Weekly sync on delivery risk and blockers. <b>Bring updates.</b>\n\nJoin: https://meet.google.com/abc-defg-hij";

/// The guest list for "Excitel weekly" — the popover's own demo fixture.
/// One of each RSVP state the week view itself already covers
/// (`the_fixture_exercises_every_rsvp_state`), so the guest list and the
/// event's own colour agree, plus a `self` row so the guest list carries a
/// "(you)" marker. The RSVP controls stay hidden regardless — `can_respond`
/// withholds them in demo mode.
fn excitel_weekly_guests() -> Vec<omacal_store::Attendee> {
    vec![
        omacal_store::Attendee {
            email: "ivan@excitel.com".into(),
            display_name: Some("Ivan".into()),
            response_status: "accepted".into(),
            optional: false,
            is_self: false,
            comment: None,
            additional_guests: 0,
        },
        omacal_store::Attendee {
            email: DEMO_SUB.into(),
            display_name: Some("Demo".into()),
            response_status: "needsAction".into(),
            optional: false,
            is_self: true,
            comment: None,
            additional_guests: 0,
        },
        omacal_store::Attendee {
            email: "petya@excitel.com".into(),
            display_name: Some("Petya".into()),
            response_status: "declined".into(),
            optional: false,
            is_self: false,
            comment: None,
            additional_guests: 0,
        },
    ]
}

struct Spec {
    cal: usize,
    title: &'static str,
    location: Option<&'static str>,
    /// Day offset from Monday of the seeded week.
    day: i64,
    /// Minutes from local midnight; ignored when `all_day`.
    start_min: i64,
    dur_min: i64,
    all_day: bool,
    /// Inclusive extra days for an all-day span.
    extra_days: i64,
    response: &'static str,
    recurrence: Option<&'static str>,
}

/// Deliberately covers every visual case the week view can render:
/// all four RSVP states, each rung of the duration ladder, a 2-way exact
/// overlap, a 3-way pile, a partial overlap, multi-day all-day spans
/// including one that begins before the week, and a recurring series.
fn specs() -> Vec<Spec> {
    let s = |cal, title, location, day, start_min, dur_min, response, recurrence| Spec {
        cal, title, location, day, start_min, dur_min,
        all_day: false, extra_days: 0, response, recurrence,
    };
    vec![
        // Recurring daily standup — 30 min, title-only rung.
        s(0, "Standup", Some("Meet"), 0, 9 * 60, 30, "accepted", Some("RRULE:FREQ=DAILY;COUNT=5")),
        // Monday
        s(0, "Excitel weekly", Some("Meet"), 0, 11 * 60, 60, "accepted", None),
        s(1, "1:1 Rahul", None, 0, 15 * 60, 30, "accepted", None),
        // Tuesday — 90 min rung (time gets its own line)
        s(0, "NetSense demo", Some("Zoom"), 1, 13 * 60, 90, "accepted", None),
        // Wednesday — three-way pile
        s(0, "Board prep", Some("Room 4A"), 2, 10 * 60, 120, "accepted", None),
        s(1, "Vendor sync", Some("Meet"), 2, 11 * 60, 60, "accepted", None),
        s(1, "Legal review", None, 2, 11 * 60 + 30, 60, "tentative", None),
        s(0, "Deep work", Some("Focus"), 2, 14 * 60, 180, "accepted", None),
        // Thursday — exact overlap, 50/50 split
        s(0, "Ops review", Some("Meet"), 3, 10 * 60, 60, "accepted", None),
        s(1, "Investors", Some("Zoom"), 3, 10 * 60, 60, "needsAction", None),
        s(0, "All hands", Some("Meet"), 3, 16 * 60, 60, "declined", None),
        // Friday — partial overlap
        s(0, "Retro", Some("Meet"), 4, 11 * 60, 60, "accepted", None),
        s(1, "Interview", Some("Room 2"), 4, 11 * 60 + 30, 60, "needsAction", None),
        s(1, "Gym", None, 4, 17 * 60, 60, "accepted", None),
        // Short event — 15 min, proves fill-based RSVP survives at minimum height
        s(0, "Sync w/ Ivan", None, 1, 16 * 60, 15, "needsAction", None),
        // All-day spans
        Spec { cal: 1, title: "Rahul on leave", location: None, day: 0, start_min: 0,
               dur_min: 0, all_day: true, extra_days: 2, response: "accepted", recurrence: None },
        Spec { cal: 1, title: "Sofia trip", location: None, day: 5, start_min: 0,
               dur_min: 0, all_day: true, extra_days: 1, response: "accepted", recurrence: None },
        // Begins before the week — exercises the `cont_left` flat/dashed edge
        Spec { cal: 0, title: "Q3 planning", location: None, day: -2, start_min: 0,
               dur_min: 0, all_day: true, extra_days: 3, response: "accepted", recurrence: None },
    ]
}

/// The year around the seeded week, because the week specs alone leave every
/// other zoom level lying about the app: Month draws near-empty cells and Big
/// Year draws a blank ribbon, when Big Year's whole argument is that
/// multi-week spans become visible shapes. Two kinds of entry, deliberately:
/// long all-day spans (2–3 weeks, the bars Big Year exists for) spread across
/// the year on both sides of today, and ordinary timed events scattered over
/// the surrounding weeks so Month reads as a lived-in calendar rather than a
/// demo of one good week. Offsets are days from the seeded week's Monday,
/// exactly as in `specs()` — nothing here introduces a second clock.
fn year_specs() -> Vec<Spec> {
    let s = |cal, title, location, day, start_min, dur_min, response| Spec {
        cal, title, location, day, start_min, dur_min,
        all_day: false, extra_days: 0, response, recurrence: None,
    };
    let span = |cal, title, day, extra_days| Spec {
        cal, title, location: None, day, start_min: 0, dur_min: 0,
        all_day: true, extra_days, response: "accepted", recurrence: None,
    };
    vec![
        // ---- the long spans Big Year is for: 2–3 weeks each, spread so the
        // ribbon carries bars in every quarter of the year ----
        span(1, "Ski week, then some", -140, 9),
        span(1, "India trip", -70, 20),
        span(1, "Berlin → Lisbon", -35, 13),
        span(0, "Launch runway", 21, 18),
        span(1, "Niki visiting", 42, 12),
        span(0, "Conference circuit", 100, 15),
        span(1, "Kitchen renovation", 70, 20),
        // Single all-days between the bars, so the quieter weeks still dot.
        span(1, "Marathon", 56, 0),
        span(0, "Server migration", 35, 0),
        span(1, "Annual review", -49, 0),
        span(0, "Public holiday", -10, 0),
        span(0, "Payroll", 8, 0),
        // ---- the surrounding weeks, so Month has more than one good row ----
        // Two weeks back
        s(0, "Security audit kickoff", Some("Meet"), -11, 10 * 60, 60, "accepted"),
        s(1, "Car service", None, -9, 8 * 60 + 30, 90, "accepted"),
        // One week back
        s(0, "Roadmap review", Some("Room 4A"), -6, 11 * 60, 90, "accepted"),
        s(1, "Dentist", None, -5, 9 * 60, 45, "accepted"),
        s(0, "Sprint review", Some("Meet"), -4, 14 * 60, 60, "accepted"),
        s(1, "Dinner w/ Maya", None, -2, 19 * 60, 120, "accepted"),
        // One week ahead
        s(0, "Quarterly close", None, 8, 10 * 60, 120, "accepted"),
        s(0, "NetSense onboarding", Some("Zoom"), 9, 13 * 60, 60, "needsAction"),
        s(1, "Flat viewing", None, 10, 17 * 60 + 30, 45, "tentative"),
        s(0, "Offsite planning", Some("Meet"), 11, 15 * 60, 60, "accepted"),
        s(1, "Padel", None, 12, 18 * 60, 90, "accepted"),
        // Two weeks ahead
        s(0, "Board meeting", Some("Room 4A"), 16, 10 * 60, 180, "accepted"),
        s(0, "Excitel QBR", Some("Zoom"), 17, 12 * 60, 120, "accepted"),
        s(1, "Theatre", None, 19, 19 * 60, 150, "accepted"),
    ]
}

/// Seeds a realistic week around `now_ms`. Idempotent: the demo account and
/// everything cascading from it is removed first.
pub async fn seed_demo(pool: &SqlitePool, now_ms: i64) -> anyhow::Result<usize> {
    sqlx::query("DELETE FROM accounts WHERE google_sub = ?1")
        .bind(DEMO_SUB)
        .execute(pool)
        .await?;

    let account_id: i64 = sqlx::query_scalar(
        "INSERT INTO accounts (google_sub, email, display_name, created_at)
         VALUES (?1, ?1, 'Demo', ?2) RETURNING id",
    )
    .bind(DEMO_SUB)
    .bind(now_ms)
    .fetch_one(pool)
    .await?;

    // Two calendars with distinct colours, so per-calendar colour is visible.
    let mut cal_ids = Vec::new();
    for (gid, summary, colour, primary) in [
        ("demo-work", "Work", "#5b8def", 1),
        ("demo-personal", "Personal", "#4cc38a", 0),
    ] {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO calendars
                 (account_id, google_id, summary, color_hex, timezone, access_role, is_primary)
             VALUES (?1, ?2, ?3, ?4, 'UTC', 'owner', ?5) RETURNING id",
        )
        .bind(account_id)
        .bind(gid)
        .bind(summary)
        .bind(colour)
        .bind(primary)
        .fetch_one(pool)
        .await?;
        cal_ids.push(id);
    }

    // Monday 00:00 of the week containing `now_ms` — in the *system* zone,
    // not UTC. The specs speak in minutes-from-midnight, and a 9:00 standup
    // seeded against UTC midnight lands at 14:30 for a viewer in Calcutta:
    // the demo's whole morning looked empty on any machine east of Greenwich.
    // The zone's offset at `now` is enough — a demo does not need to survive
    // a DST transition mid-week, and the shift keeps the arithmetic below
    // exactly what it was.
    let offset_ms = jiff::Timestamp::from_millisecond(now_ms)?
        .to_zoned(jiff::tz::TimeZone::system())
        .offset()
        .seconds() as i64
        * 1000;
    let week_start = {
        let days = (now_ms + offset_ms).div_euclid(DAY_MS);
        // 1970-01-01 was a Thursday; shift so 0 == Monday.
        let dow = (days + 3).rem_euclid(7);
        (days - dow) * DAY_MS - offset_ms
    };

    let mut all = specs();
    all.extend(year_specs());

    let mut written = 0usize;
    for (i, sp) in all.iter().enumerate() {
        let day_start = week_start + sp.day * DAY_MS;
        let (start, end) = if sp.all_day {
            (day_start, day_start + (sp.extra_days + 1) * DAY_MS)
        } else {
            let s = day_start + sp.start_min * MIN;
            (s, s + sp.dur_min * MIN)
        };

        // Only "Excitel weekly" carries a description and a guest list: the
        // popover has to be exercisable in demo mode (`OMACAL_SEED_DEMO=1`,
        // no Google account anywhere near it), which needs at least one demo
        // event with something in both fields.
        let (description, attendees) = if sp.title == "Excitel weekly" {
            (Some(EXCITEL_WEEKLY_DESCRIPTION.to_string()), excitel_weekly_guests())
        } else {
            (None, Vec::new())
        };

        omacal_store::upsert_event(
            pool,
            &StoredEvent {
                id: 0,
                calendar_id: cal_ids[sp.cal],
                google_id: format!("demo-{i}"),
                summary: Some(sp.title.to_string()),
                location: sp.location.map(str::to_string),
                start_utc: start,
                end_utc: end,
                start_tz: "UTC".into(),
                end_tz: "UTC".into(),
                is_all_day: sp.all_day,
                recurrence: sp.recurrence.map(str::to_string),
                recurring_event_id: None,
                original_start_utc: None,
                status: "confirmed".into(),
                self_response: Some(sp.response.to_string()),
                // The same event carries the popover's Join button.
                conference_uri: (sp.title == "Excitel weekly")
                    .then(|| "https://meet.google.com/abc-defg-hij".to_string()),
                color_hex: None,
                // Joined in from `calendars` on read, so this write is inert;
                // every demo calendar above is seeded with 'UTC' anyway.
                calendar_timezone: "UTC".into(),
                description,
                etag: None,
                sequence: 0,
                organizer_email: None,
                guests_can_modify: false,
                attendees,
                reminders: Default::default(),
                calendar_default_reminders: Vec::new(),
            },
        )
        .await?;
        written += 1;
    }

    seed_demo_tasks(pool, account_id, week_start, offset_ms, now_ms).await?;
    Ok(written)
}

/// A task in the demo week. `day`/`at_min` as in [`Spec`]; `at_min` of
/// `None` is a due *date* (all-day), and `day` of `None` is no due date.
struct TaskSpec {
    list: usize,
    title: &'static str,
    day: Option<i64>,
    at_min: Option<i64>,
    done: bool,
}

/// Two task lists and a week of tasks, because the pane and the grid are half
/// of what OmaCal draws and the demo showed neither. Chosen to reach every
/// state a task can be seen in: due at an hour (drawn on the grid among the
/// meetings), due on a day, overdue, done this week, and undated.
fn task_specs() -> Vec<TaskSpec> {
    let t = |list, title, day, at_min, done| TaskSpec { list, title, day, at_min, done };
    vec![
        t(0, "Reply to Niki about the offsite", Some(0), Some(10 * 60), false),
        t(0, "Renew the domain", Some(1), Some(12 * 60), true),
        t(0, "Send the board deck", Some(2), Some(13 * 60), false),
        t(0, "Book flights to Sofia", Some(3), Some(12 * 60), false),
        t(0, "Submit expenses", Some(4), Some(16 * 60), false),
        t(1, "Pay the electricity bill", Some(2), None, false),
        t(1, "Pick up dry cleaning", Some(4), None, false),
        t(1, "Call the accountant", None, None, false),
        t(1, "Order a birthday gift for Maya", None, None, false),
    ]
}

async fn seed_demo_tasks(
    pool: &SqlitePool,
    account_id: i64,
    week_start: i64,
    offset_ms: i64,
    now_ms: i64,
) -> anyhow::Result<()> {
    let mut list_ids = Vec::new();
    for (gid, summary, colour) in [("demo-tasks", "Tasks", "#e2a03f"), ("demo-home", "Home", "#c678dd")] {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO calendars
                 (account_id, google_id, summary, color_hex, timezone, access_role, is_primary,
                  supports_events, supports_tasks)
             VALUES (?1, ?2, ?3, ?4, 'UTC', 'owner', 0, 0, 1) RETURNING id",
        )
        .bind(account_id)
        .bind(gid)
        .bind(summary)
        .bind(colour)
        .fetch_one(pool)
        .await?;
        list_ids.push(id);
    }
    for (i, t) in task_specs().iter().enumerate() {
        // A due *date* is stored as that date's UTC midnight, the way a
        // `DUE;VALUE=DATE` is read back; a due *time* is the local instant.
        let due = t.day.map(|d| match t.at_min {
            Some(at) => week_start + d * DAY_MS + at * MIN,
            None => week_start + offset_ms + d * DAY_MS,
        });
        omacal_store::upsert_task(
            pool,
            &omacal_store::StoredTask {
                id: 0,
                calendar_id: list_ids[t.list],
                uid: format!("demo-task-{i}"),
                etag: None,
                caldav_href: None,
                summary: Some(t.title.to_string()),
                description: None,
                due_utc: due,
                due_tz: due.map(|_| "UTC".to_string()),
                due_all_day: due.is_some() && t.at_min.is_none(),
                status: if t.done { "completed" } else { "needs-action" }.to_string(),
                completed_utc: t.done.then_some(now_ms - DAY_MS),
                priority: 0,
                updated_at: now_ms,
                raw_ics: None,
            },
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monday 2026-08-03 00:00:00 UTC
    const MON: i64 = 1_785_715_200_000;
    const DAY: i64 = 24 * 3_600_000;

    #[tokio::test]
    async fn seeding_creates_an_account_calendars_and_events() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let n = seed_demo(&pool, MON + 3 * DAY).await.unwrap();
        assert!(n >= 15, "expected a rich fixture, got {n} events");

        let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(accounts, 1);

        let cals: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM calendars")
            .fetch_one(&pool).await.unwrap();
        assert!(cals >= 2, "need multiple calendars to exercise per-calendar colour");
    }

    /// The pane and the grid both have something to draw, in every state.
    #[tokio::test]
    async fn seeding_includes_tasks_in_each_state() {
        let pool = omacal_store::connect_memory().await.unwrap();
        seed_demo(&pool, MON + 3 * DAY).await.unwrap();
        seed_demo(&pool, MON + 3 * DAY).await.unwrap();
        let rows = omacal_store::tasks_for_ui(&pool, 0).await.unwrap();
        assert_eq!(rows.len(), task_specs().len(), "a reseed duplicated tasks");
        assert!(rows.iter().any(|r| r.task.status == "completed"));
        assert!(rows.iter().any(|r| r.task.due_utc.is_none()));
        assert!(rows.iter().any(|r| r.task.due_all_day));
        assert!(rows.iter().any(|r| r.task.due_utc.is_some() && !r.task.due_all_day));
    }

    #[tokio::test]
    async fn seeding_twice_does_not_duplicate() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let first = seed_demo(&pool, MON + 3 * DAY).await.unwrap();
        let second = seed_demo(&pool, MON + 3 * DAY).await.unwrap();
        assert_eq!(first, second);

        let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(events as usize, first);
    }

    #[tokio::test]
    async fn the_fixture_exercises_every_rsvp_state() {
        let pool = omacal_store::connect_memory().await.unwrap();
        seed_demo(&pool, MON + 3 * DAY).await.unwrap();
        for state in ["accepted", "needsAction", "tentative", "declined"] {
            let n: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM events WHERE self_response = ?1")
                .bind(state).fetch_one(&pool).await.unwrap();
            assert!(n > 0, "fixture has no event in state {state}");
        }
    }

    #[tokio::test]
    async fn the_fixture_exercises_overlaps_and_all_day_spans() {
        let pool = omacal_store::connect_memory().await.unwrap();
        seed_demo(&pool, MON + 3 * DAY).await.unwrap();

        let all_day: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events WHERE is_all_day = 1")
            .fetch_one(&pool).await.unwrap();
        assert!(all_day >= 2, "need multi-day spans for the all-day band");

        let recurring: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events WHERE recurrence IS NOT NULL")
            .fetch_one(&pool).await.unwrap();
        assert!(recurring >= 1, "need a recurring series");

        // Two events at identical times on the same day => 50/50 column split.
        let exact_overlap: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM (
               SELECT start_utc, end_utc FROM events WHERE is_all_day = 0
               GROUP BY start_utc, end_utc HAVING COUNT(*) >= 2)")
            .fetch_one(&pool).await.unwrap();
        assert!(exact_overlap >= 1, "need an exact overlap to exercise column splitting");
    }

    /// Task 8's own requirement: the popover has to be exercisable with
    /// `OMACAL_SEED_DEMO=1` and no Google account, which needs at least one
    /// demo event with a description (for `descriptionSegments`) and a full
    /// guest list — one of each of the three RSVP states plus a `self` row,
    /// which is what renders the "(you)" marker the guest list is built
    /// around.
    ///
    /// The RSVP controls themselves stay hidden here whatever this fixture
    /// says: `can_respond` withholds them in demo mode outright, since the
    /// only thing pressing one could do is reach `demo_sync_guard` and be
    /// refused. `access_role` is still asserted below because the *shape* of
    /// the demo data should match a real writable calendar — a `reader` row
    /// would be a different fixture, silently.
    #[tokio::test]
    async fn the_popover_fixture_has_a_description_and_a_full_guest_list() {
        let pool = omacal_store::connect_memory().await.unwrap();
        seed_demo(&pool, MON + 3 * DAY).await.unwrap();

        let id: i64 = sqlx::query_scalar("SELECT id FROM events WHERE summary = 'Excitel weekly'")
            .fetch_one(&pool)
            .await
            .unwrap();
        let (ev, access_role, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();

        assert!(ev.description.is_some(), "nothing for descriptionSegments to render");
        assert_eq!(access_role, "owner", "the demo data must match a real writable calendar");

        assert!(
            ev.attendees.iter().any(|a| a.is_self),
            "without a self row there is no \"(you)\" marker in the guest list"
        );
        for state in ["accepted", "declined", "needsAction"] {
            assert!(
                ev.attendees.iter().any(|a| a.response_status == state),
                "no demo guest is in state {state}"
            );
        }
    }

    #[test]
    fn demo_mode_is_off_unless_explicitly_enabled() {
        // Guard against a stray env var making a real launch load fake data.
        std::env::remove_var("OMACAL_SEED_DEMO");
        assert!(!demo_mode());
        std::env::set_var("OMACAL_SEED_DEMO", "0");
        assert!(!demo_mode());
        std::env::set_var("OMACAL_SEED_DEMO", "1");
        assert!(demo_mode());
        std::env::remove_var("OMACAL_SEED_DEMO");
    }
}
