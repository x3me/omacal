//! The scheduler driver — **the only thing in this feature that reads a clock.**
//!
//! Every rule about *which* reminders fire lives in `omacal_core::remind`,
//! where `now` is a parameter and no test has to sleep to observe anything.
//! What is left here is the part that genuinely cannot be pure: read the world,
//! call that function, post what is ready, record it, forget what is dead, and
//! work out when to wake. A rule that creeps into this file becomes untestable
//! without waiting, which is the whole reason the split exists.
//!
//! Note the shape of [`run_once`]: `now_ms` is still a parameter *here* too, so
//! the driver's own behaviour — what it posts, what it records, what it prunes,
//! when it would next wake — is tested at a fixed clock. Only [`spawn`] reads
//! the real one, and it is deliberately too thin to be wrong.

use omacal_core::remind::{
    due_reminders, due_tasks, Due, DueTask, FiredKey, FiredTask, Reminder, ScheduledEvent,
    ScheduledTask, MISSED_TASK_MS,
};
use sqlx::SqlitePool;

/// How far ahead the scheduler looks (spec §4). Comfortably longer than the
/// sync interval, so a reminder cannot fall between two recomputations, and
/// short enough that expanding recurring series stays cheap.
pub(crate) const HORIZON_MS: i64 = 48 * 3_600_000;

/// The longest lead time Google accepts on a reminder: 40320 minutes, four
/// weeks.
///
/// The fetch window has to reach this far past the horizon, because the horizon
/// bounds *fire times* and a fire time is up to this long before the occurrence
/// it belongs to. An event four weeks out with a four-week reminder is due
/// right now, and a window that stopped at the horizon would not see it.
const MAX_REMINDER_LEAD_MS: i64 = 40_320 * 60_000;

/// The longest lead a task's own alarm can ask for before the fetch below
/// stops looking for it. Four weeks, as for events and for the same reason:
/// the window bounds *dues*, while a fire time is up to a lead earlier.
const MAX_TASK_LEAD_MS: i64 = MAX_REMINDER_LEAD_MS;

/// One pass's worth of outcome, so a test can see everything the pass decided
/// without watching a clock.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Pass {
    /// Handed to the notifier on this pass, and recorded. A reminder the
    /// notifier *refused* is still in here: the attempt was made and the record
    /// written, which is what stops it being attempted again forever.
    pub posted: Vec<Due>,
    /// Task announcements posted on this pass (#137), on the same terms as
    /// `posted` above: a refused one is still here, and still recorded.
    pub posted_tasks: Vec<DueTask>,
    /// The earliest fire-time still ahead of `now_ms`, if any. What [`spawn`]
    /// sleeps until.
    pub next_fire_ms: Option<i64>,
    /// Rows forgotten because their occurrence has ended.
    pub pruned: u64,
    /// Task records forgotten because nothing answers to them any more.
    pub pruned_tasks: u64,
}

/// Maps one stored reminder into the shape the pure function takes.
fn to_core(r: &omacal_store::Reminder) -> Reminder {
    Reminder { method: r.method.clone(), minutes: r.minutes }
}

/// The scheduler's view of the world at `now_ms`: every occurrence that could
/// carry a reminder worth posting, expanded and carrying its calendar's zone.
///
/// **Only calendars with `selected = true` are represented, and that is
/// enforced by `events_in_window`'s own `WHERE c.selected = 1`** — the same
/// query the grid draws from. Deliberately the same one: §2.2 says a calendar
/// not worth drawing is not worth interrupting you, and reading notifications
/// off a second, parallel query is how the two would come to disagree.
/// `calendar_selected` below is therefore always `true` here; the field is the
/// pure function's own statement of the rule, and it is what a caller
/// constructing `ScheduledEvent`s by hand has to get right.
///
/// Cancelled rows and suppressed slots are skipped exactly as the four grid
/// assemblers skip them — a deleted occurrence of a series must not notify any
/// more than it should draw.
///
/// **A declined invitation notifies nothing.** Both Google Calendar and Apple
/// Calendar treat declining as "off my schedule" and go quiet; a reminder for
/// a meeting the user said no to is noise wearing punctuality's clothes. The
/// row-level skip handles both shapes: a declined event is one row, and a
/// declined occurrence of a series is an exception row that has already
/// suppressed its parent's slot. `tentative` and `needsAction` still fire —
/// a maybe and an unanswered invitation are both still claims on the hour.
async fn scheduled_events(
    pool: &SqlitePool,
    now_ms: i64,
    horizon_ms: i64,
) -> anyhow::Result<Vec<ScheduledEvent>> {
    let to_ms = now_ms.saturating_add(horizon_ms).saturating_add(MAX_REMINDER_LEAD_MS);
    // From `now_ms`, not earlier: an occurrence that began in the past is still
    // returned while it is running, since both the store's window query and
    // `occurrences` keep anything whose end is still ahead — which is exactly
    // the set the missed-reminder rule needs and no more.
    let stored = omacal_store::events_in_window(pool, now_ms, to_ms).await?;
    let suppressed = crate::commands::suppressed_slots(&stored);

    let mut out = Vec::new();
    for src in &stored {
        // A cancelled exception exists only to record that an occurrence was
        // deleted. It has been counted into `suppressed`; it notifies nothing.
        if src.status == "cancelled" {
            continue;
        }
        // Declined means off the schedule (§ doc comment above): skip the row,
        // whether it is a whole event or one occurrence's exception.
        if src.self_response.as_deref() == Some("declined") {
            continue;
        }
        for iv in crate::commands::occurrences(src, now_ms, to_ms) {
            if suppressed.contains(&(src.calendar_id, src.google_id.as_str(), iv.start_ms)) {
                continue;
            }
            out.push(ScheduledEvent {
                event_id: src.id,
                calendar_selected: true,
                calendar_tz: src.calendar_timezone.clone(),
                occurrence_start_ms: iv.start_ms,
                occurrence_end_ms: iv.end_ms,
                is_all_day: src.is_all_day,
                use_default_reminders: src.reminders.use_default,
                overrides: src.reminders.overrides.iter().map(to_core).collect(),
                calendar_defaults: src.calendar_default_reminders.iter().map(to_core).collect(),
                title: src.summary.clone(),
                location: src.location.clone(),
                conference_uri: src.conference_uri.clone(),
            });
        }
    }
    Ok(out)
}

/// The scheduler's view of the tasks worth announcing at `now_ms` (#137):
/// open, timed, on a selected list, each carrying the lead its own resource
/// asks for.
///
/// Which tasks those are is the store query's decision
/// (`tasks_to_announce`), exactly as `events_in_window` decides it for
/// events — one query, the same `selected` gate the pane draws through, and
/// so nothing to drift. The lead comes from the task's own `VALARM`: a
/// reminder made on the phone with "15 minutes before" speaks when the phone
/// speaks, and a task with no alarm speaks at its due.
async fn scheduled_tasks(
    pool: &SqlitePool,
    now_ms: i64,
    horizon_ms: i64,
) -> anyhow::Result<Vec<ScheduledTask>> {
    let from_ms = now_ms.saturating_sub(MISSED_TASK_MS);
    let to_ms = now_ms.saturating_add(horizon_ms).saturating_add(MAX_TASK_LEAD_MS);
    Ok(omacal_store::tasks_to_announce(pool, from_ms, to_ms)
        .await?
        .into_iter()
        .map(|row| ScheduledTask {
            task_id: row.task.id,
            // `IS NOT NULL` in the query, so the default is unreachable.
            due_ms: row.task.due_utc.unwrap_or_default(),
            lead_minutes: row
                .task
                .raw_ics
                .as_deref()
                .and_then(|raw| omacal_caldav::todo_alarm_lead_minutes(raw, &row.task.uid))
                .unwrap_or(0),
            title: row.task.summary.clone(),
            list: Some(row.calendar_summary.clone()),
        })
        .collect())
}

/// Whether this run is allowed to post at all.
///
/// **Demo mode posts no notifications** (§2.7) — the fourth enforcement point
/// beside the separate database, `demo_sync_guard`, and
/// `sync_loop::may_sync`/`should_sync`. Demo seeds synthetic events in the
/// present so the views look alive, which is precisely what would make an
/// unguarded scheduler buzz about meetings that do not exist.
///
/// **And the user's own switch** (settings spec §3): reminders can be turned
/// off entirely, which is a different thing from having none due. Read here
/// rather than at `spawn` for the same reason demo mode is — a guard on a path
/// no test can reach is a guard nobody has checked.
///
/// Named, like `may_sync`, so the decision is one thing every caller routes
/// through rather than an untested `if demo` copied about.
pub(crate) fn may_notify(demo: bool, enabled: bool) -> bool {
    !demo && enabled
}

/// One pass of the scheduler, at a clock the caller supplies.
///
/// **Demo mode returns immediately**, having posted nothing and recorded
/// nothing — see [`may_notify`].
///
/// `tz` is the **display** zone — what the user reads the time in. It decides
/// only wording; every firing decision was made against the calendar's zone
/// before this point.
///
/// **A notifier that refuses is not an error, and the reminder is recorded
/// anyway.** On macOS an unsigned bundle may simply be refused (§2.4), and that
/// is the expected case rather than a fault. The alternative — leaving it
/// unrecorded — re-offers the same reminder on every pass for as long as the
/// occurrence runs, so a transport that is down turns into an unbounded retry
/// loop and a log full of the same failure. Recording it means one attempt per
/// reminder, one log line, and no banner. The cost is a reminder genuinely lost
/// when the transport was briefly unavailable, which is the cheaper mistake:
/// the user missed one notification rather than being unable to use the app.
///
/// **Ready means `fire_at_ms <= now_ms`, not "in the returned list".**
/// `due_reminders` hands back the whole schedule out to the horizon, most of
/// which is still in the future; posting all of it would fire two days of
/// reminders at once. Everything past that cut is what the driver sleeps until.
///
/// A reminder whose time has already gone by is posted immediately, which is
/// the missed rule (§2.3) arriving here: `fire_at_ms` may be well behind
/// `now_ms`, and the comparison above treats that as ready rather than as a
/// negative delay to wait out.
///
/// Recording follows posting, never precedes it. A crash in between re-posts
/// one reminder on the next pass; the other order silently swallows it forever.
pub(crate) async fn run_once(
    pool: &SqlitePool,
    demo: bool,
    now_ms: i64,
    horizon_ms: i64,
    tz: &str,
    notifier: &dyn crate::notify::Notifier,
) -> anyhow::Result<Pass> {
    // Before any I/O, and the only place this is enforced. Guarding at `spawn`
    // instead would keep the loop from starting, which sounds stronger and is
    // weaker: the guard would then sit on a path no test can reach, and the
    // test that matters is the one running through this function exactly as a
    // real pass does. A pointless loop in demo mode costs a comparison every
    // few minutes and returns here before touching the database.
    let settings = crate::settings::read_settings(pool).await;
    // Demo mode, and the two switches: nothing to do when neither half may
    // speak. Each half then asks for its own switch below, so meeting
    // reminders and task announcements are turned off apart (#137).
    if !may_notify(demo, settings.notifications_enabled || settings.task_notifications_enabled) {
        return Ok(Pass::default());
    }

    let mut pass = Pass::default();
    if may_notify(demo, settings.notifications_enabled) {
        run_events(pool, now_ms, horizon_ms, tz, notifier, &settings, &mut pass).await?;
    }
    if may_notify(demo, settings.task_notifications_enabled) {
        run_tasks(pool, now_ms, horizon_ms, tz, notifier, &settings, &mut pass).await?;
    }
    Ok(pass)
}

/// The event half of a pass, split out when tasks became the second half
/// (#137). Its rules are unchanged, and are `due_reminders`'.
async fn run_events(
    pool: &SqlitePool,
    now_ms: i64,
    horizon_ms: i64,
    tz: &str,
    notifier: &dyn crate::notify::Notifier,
    settings: &crate::settings::AppSettings,
    pass: &mut Pass,
) -> anyhow::Result<()> {
    // Popup by construction (fallback spec §3): the setting stores minutes
    // alone, and this is the one place they become reminders.
    let fallback: Vec<Reminder> = settings
        .fallback_reminder_minutes
        .iter()
        .map(|&minutes| Reminder { method: "popup".into(), minutes })
        .collect();

    let events = scheduled_events(pool, now_ms, horizon_ms).await?;

    let fired: std::collections::HashSet<FiredKey> = omacal_store::fired_keys(pool)
        .await?
        .into_iter()
        .map(|(event_id, occurrence_ms, minutes)| FiredKey { event_id, occurrence_ms, minutes })
        .collect();

    let due = due_reminders(&events, &fired, now_ms, horizon_ms, &fallback);

    for d in due {
        if d.fire_at_ms > now_ms {
            // Still ahead. `due_reminders` returns its results in fire-time
            // order, so the first one past the cut is the earliest.
            pass.next_fire_ms = Some(pass.next_fire_ms.map_or(d.fire_at_ms, |n: i64| n.min(d.fire_at_ms)));
            continue;
        }

        if let Err(e) = notifier.post(&crate::notify::notification_for_format(
            &d,
            tz,
            settings.time_format,
        )) {
            // Logged and dropped. Never a banner, never a retry — see this
            // function's own doc comment for why the reminder is still
            // recorded below.
            tracing::warn!(%e, event_id = d.key.event_id, "could not post a reminder");
        }

        // `d.occurrence_end_ms` rather than a lookup: the `Due` carries the end
        // of its own occurrence, so there is no search to come up empty and no
        // fallback to be wrong. Searching `events` would have to match the
        // anchor against a raw start, which for an all-day occurrence on a
        // moved calendar zone are different instants.
        omacal_store::record_fired(
            pool,
            d.key.event_id,
            d.key.occurrence_ms,
            d.key.minutes,
            d.occurrence_end_ms,
            now_ms,
        )
        .await?;

        pass.posted.push(d);
    }

    pass.pruned = omacal_store::prune_fired(pool, now_ms).await?;
    Ok(())
}

/// The task half of a pass (#137): announce a task at its due, or at its own
/// alarm's lead, once.
///
/// The same order as the event half, for the same reasons: post, then record,
/// so a crash in between repeats one announcement rather than swallowing it;
/// a notifier that refuses is not an error and the announcement is still
/// recorded; and anything still ahead only moves the next wake.
async fn run_tasks(
    pool: &SqlitePool,
    now_ms: i64,
    horizon_ms: i64,
    tz: &str,
    notifier: &dyn crate::notify::Notifier,
    settings: &crate::settings::AppSettings,
    pass: &mut Pass,
) -> anyhow::Result<()> {
    let tasks = scheduled_tasks(pool, now_ms, horizon_ms).await?;
    let fired: std::collections::HashSet<FiredTask> = omacal_store::fired_task_keys(pool)
        .await?
        .into_iter()
        .map(|(task_id, due_ms)| FiredTask { task_id, due_ms })
        .collect();

    for d in due_tasks(&tasks, &fired, now_ms, horizon_ms) {
        if d.fire_at_ms > now_ms {
            pass.next_fire_ms =
                Some(pass.next_fire_ms.map_or(d.fire_at_ms, |n: i64| n.min(d.fire_at_ms)));
            continue;
        }

        if let Err(e) = notifier.post(&crate::notify::task_notification_for_format(
            &d,
            tz,
            settings.time_format,
        )) {
            tracing::warn!(%e, task_id = d.key.task_id, "could not announce a task");
        }

        omacal_store::record_fired_task(pool, d.key.task_id, d.key.due_ms, now_ms).await?;
        pass.posted_tasks.push(d);
    }

    pass.pruned_tasks = omacal_store::prune_fired_tasks(pool).await?;
    Ok(())
}

/// How long to sleep: until the next fire-time or the next sync, whichever is
/// sooner, and never a negative duration.
///
/// Clamped at zero because `next_fire_ms` can be behind `now_ms` in principle —
/// a pass that failed to post, a clock moved backwards — and a driver that
/// subtracted and slept would wait for hours or panic converting to a
/// `Duration`. Zero means "go round again now", which re-posts nothing, because
/// the record written on the last pass excludes it.
pub(crate) fn next_wake_ms(next_fire_ms: Option<i64>, now_ms: i64, sync_interval_ms: i64) -> i64 {
    let until_sync = sync_interval_ms.max(0);
    match next_fire_ms {
        Some(fire) => (fire - now_ms).clamp(0, until_sync),
        None => until_sync,
    }
}

/// The loop, and the only clock read in the feature.
///
/// Deliberately thin to the point of having nothing in it worth testing: every
/// decision it makes was made by [`run_once`] and [`next_wake_ms`], both of
/// which take their clock as an argument.
///
/// Started from `run()`'s `setup`, once the transport and the demo guard both
/// exist. Deliberately not before: a pass with no working notifier would record
/// every due reminder as fired while posting nothing, and a recorded reminder
/// is never offered again — it would have silently consumed the first day of
/// notifications.
pub(crate) fn spawn(app: tauri::AppHandle, notifier: std::sync::Arc<dyn crate::notify::Notifier>) {
    tauri::async_runtime::spawn(async move {
        loop {
            let (pool, demo, interval) = {
                let state = tauri::Manager::state::<crate::AppState>(&app);
                (
                    state.pool.clone(),
                    state.demo,
                    crate::sync_loop::interval_ms(&state.pool).await,
                )
            };

            let now = crate::now_ms();
            let tz = crate::display_tz(&pool);
            let next_fire =
                match run_once(&pool, demo, now, HORIZON_MS, &tz, notifier.as_ref()).await {
                Ok(pass) => pass.next_fire_ms,
                Err(e) => {
                    // Offline, a locked database, a malformed rule — all normal
                    // and all transient. Never panic the app over one pass.
                    tracing::warn!(%e, "notification pass failed");
                    None
                }
            };

            let delay = next_wake_ms(next_fire, crate::now_ms(), interval);
            tokio::time::sleep(std::time::Duration::from_millis(delay as u64)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use omacal_store::{Reminders, StoredEvent};

    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;

    /// 2026-08-10T09:00:00Z.
    const T0900Z: i64 = 1_786_352_400_000;
    /// Midnight 2026-08-10 in `Europe/Sofia` — 2026-08-09T21:00Z.
    const SOFIA_MIDNIGHT_AUG10: i64 = 1_786_309_200_000;
    /// Midnight 2026-08-10 in `Pacific/Auckland` — 2026-08-09T12:00Z.
    const AUCKLAND_MIDNIGHT_AUG10: i64 = 1_786_276_800_000;

    async fn seeded(cal_tz: &str, defaults: &str) -> SqlitePool {
        let pool = omacal_store::connect_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (google_sub, email, created_at) VALUES ('s','e@x',0)")
            .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO calendars
                 (account_id, google_id, summary, timezone, access_role, default_reminders_json)
             VALUES (1, 'primary', 'Work', ?1, 'owner', ?2)",
        )
        .bind(cal_tz)
        .bind(defaults)
        .execute(&pool).await.unwrap();
        pool
    }

    fn popup(minutes: i64) -> omacal_store::Reminder {
        omacal_store::Reminder { method: "popup".into(), minutes }
    }

    fn event(google_id: &str, start: i64, end: i64, reminders: Reminders) -> StoredEvent {
        StoredEvent {
            id: 0,
            calendar_id: 1,
            google_id: google_id.into(),
            summary: Some("Standup".into()),
            location: None,
            start_utc: start,
            end_utc: end,
            start_tz: "UTC".into(),
            end_tz: "UTC".into(),
            is_all_day: false,
            recurrence: None,
            recurring_event_id: None,
            original_start_utc: None,
            status: "confirmed".into(),
            self_response: None,
            conference_uri: None,
            color_hex: None,
            calendar_timezone: "UTC".into(),
            description: None,
            etag: None,
            sequence: 0,
            organizer_email: None,
            guests_can_modify: false,
            attendees: Vec::new(),
            reminders,
            calendar_default_reminders: Vec::new(),
        }
    }

    fn own(minutes: i64) -> Reminders {
        Reminders { use_default: false, overrides: vec![popup(minutes)] }
    }

    /// The display zone every test reads times in. Deliberately not UTC, so a
    /// body that ignored the zone argument would read differently.
    const SOFIA: &str = "Europe/Sofia";

    /// One pass against a recording fake. **No test in this file reaches a real
    /// transport** — `RecordingNotifier` is the only notifier the suite has.
    async fn pass_with(
        pool: &SqlitePool,
        now_ms: i64,
        notifier: &crate::notify::RecordingNotifier,
    ) -> (Pass, Vec<(i64, i64)>) {
        let pass = run_once(pool, false, now_ms, HORIZON_MS, SOFIA, notifier).await.unwrap();
        let handed = pass.posted.iter().map(|d| (d.key.event_id, d.key.minutes)).collect();
        (pass, handed)
    }

    /// `pass_with` against a fresh fake, for the tests that only care which
    /// reminders came out.
    async fn pass_at(pool: &SqlitePool, now_ms: i64) -> (Pass, Vec<(i64, i64)>) {
        pass_with(pool, now_ms, &crate::notify::RecordingNotifier::default()).await
    }

    #[tokio::test]
    async fn a_reminder_that_is_due_is_posted_and_recorded() {
        let pool = seeded("UTC", "[]").await;
        omacal_store::upsert_event(&pool, &event("e1", T0900Z, T0900Z + HOUR, own(10)))
            .await.unwrap();

        // Ten minutes before the meeting: exactly when the reminder is due.
        let (pass, posted) = pass_at(&pool, T0900Z - 10 * MINUTE).await;

        assert_eq!(posted, vec![(1, 10)]);
        assert_eq!(pass.posted.len(), 1);
        assert_eq!(
            omacal_store::fired_keys(&pool).await.unwrap(),
            vec![(1, T0900Z, 10)],
            "posting without recording re-posts on the next pass"
        );
    }

    #[tokio::test]
    async fn the_pass_applies_the_stored_clock_format_to_notifications() {
        let pool = seeded("UTC", "[]").await;
        sqlx::query("INSERT INTO settings (key, value) VALUES ('time_format', '12h')")
            .execute(&pool).await.unwrap();
        omacal_store::upsert_event(&pool, &event("e1", T0900Z, T0900Z + HOUR, own(10)))
            .await.unwrap();

        let fake = crate::notify::RecordingNotifier::default();
        pass_with(&pool, T0900Z - 10 * MINUTE, &fake).await;

        assert_eq!(fake.posted()[0].body, "12:00 PM");
    }

    /// A fire-time still ahead is scheduled, not posted. `due_reminders` hands
    /// back the whole 48-hour schedule, and posting all of it would fire two
    /// days of reminders at once.
    #[tokio::test]
    async fn a_reminder_still_in_the_future_is_scheduled_rather_than_posted() {
        let pool = seeded("UTC", "[]").await;
        omacal_store::upsert_event(&pool, &event("e1", T0900Z, T0900Z + HOUR, own(10)))
            .await.unwrap();

        let (pass, posted) = pass_at(&pool, T0900Z - 3 * HOUR).await;

        assert!(posted.is_empty(), "a reminder due in hours must not fire now");
        assert_eq!(pass.next_fire_ms, Some(T0900Z - 10 * MINUTE));
        assert!(
            omacal_store::fired_keys(&pool).await.unwrap().is_empty(),
            "nothing may be recorded that was not posted, or it never fires at all"
        );
    }

    /// **The restart case, and the reason the table exists.**
    ///
    /// The second pass rebuilds everything from the database — events, fired
    /// state, the lot — exactly as a fresh process would. The clock is
    /// deliberately still *inside* the meeting: past its end the missed rule
    /// would decline to fire for its own reasons and this would pass without
    /// witnessing the record at all.
    #[tokio::test]
    async fn a_recorded_reminder_is_not_posted_again_after_a_restart() {
        let pool = seeded("UTC", "[]").await;
        omacal_store::upsert_event(&pool, &event("e1", T0900Z, T0900Z + HOUR, own(10)))
            .await.unwrap();

        let (_, first) = pass_at(&pool, T0900Z - 10 * MINUTE).await;
        assert_eq!(first, vec![(1, 10)], "the first pass must actually post something");

        // Five minutes in: the meeting is running, so the missed rule would
        // gladly fire this again if nothing had been recorded.
        let now = T0900Z + 5 * MINUTE;
        assert!(now < T0900Z + HOUR, "fixture check: the meeting must still be running");

        let (_, second) = pass_at(&pool, now).await;
        assert!(second.is_empty(), "a restart mid-meeting must not re-post the reminder");
    }

    /// The other half of that: with the record cleared, the same clock *does*
    /// post. Without this, the test above would pass against a driver that
    /// simply stopped posting after the first pass.
    #[tokio::test]
    async fn the_same_clock_posts_again_once_the_record_is_gone() {
        let pool = seeded("UTC", "[]").await;
        omacal_store::upsert_event(&pool, &event("e1", T0900Z, T0900Z + HOUR, own(10)))
            .await.unwrap();

        pass_at(&pool, T0900Z - 10 * MINUTE).await;
        sqlx::query("DELETE FROM fired_reminders").execute(&pool).await.unwrap();

        let (_, again) = pass_at(&pool, T0900Z + 5 * MINUTE).await;
        assert_eq!(again, vec![(1, 10)], "the record is the only thing suppressing this");
    }

    /// §2.3: a reminder missed while omacal was not running fires at launch,
    /// while the meeting is still on.
    #[tokio::test]
    async fn a_reminder_missed_while_the_app_was_shut_fires_at_the_next_pass() {
        let pool = seeded("UTC", "[]").await;
        omacal_store::upsert_event(&pool, &event("e1", T0900Z, T0900Z + HOUR, own(10)))
            .await.unwrap();

        // First run of the day is five minutes into the meeting.
        let (pass, posted) = pass_at(&pool, T0900Z + 5 * MINUTE).await;

        assert_eq!(posted, vec![(1, 10)]);
        assert!(
            pass.posted[0].fire_at_ms < T0900Z + 5 * MINUTE,
            "its fire time is in the past, which is the whole point of the rule"
        );
    }

    /// §2.2, end to end: a hidden calendar interrupts nobody. Enforced by
    /// `events_in_window`'s own filter, which is why this is asserted here and
    /// not only as a rule in the pure function.
    #[tokio::test]
    async fn a_hidden_calendars_events_notify_nothing() {
        let pool = seeded("UTC", "[]").await;
        omacal_store::upsert_event(&pool, &event("e1", T0900Z, T0900Z + HOUR, own(10)))
            .await.unwrap();

        let (_, shown) = pass_at(&pool, T0900Z - 10 * MINUTE).await;
        assert_eq!(shown, vec![(1, 10)], "fixture check: it fires while the calendar is shown");

        sqlx::query("DELETE FROM fired_reminders").execute(&pool).await.unwrap();
        sqlx::query("UPDATE calendars SET selected = 0").execute(&pool).await.unwrap();

        let (_, hidden) = pass_at(&pool, T0900Z - 10 * MINUTE).await;
        assert!(hidden.is_empty(), "deselecting a calendar cancels its pending reminders");
    }

    /// Declining an invitation is saying "off my schedule", and both Google
    /// and Apple go quiet from that moment. A `tentative` reply, by contrast,
    /// is still a claim on the hour and still fires — the pair below is one
    /// test so the contrast is witnessed by the same clock.
    #[tokio::test]
    async fn a_declined_events_reminder_notifies_nothing_and_a_tentative_ones_still_fires() {
        let pool = seeded("UTC", "[]").await;

        let mut declined = event("e1", T0900Z, T0900Z + HOUR, own(10));
        declined.self_response = Some("declined".into());
        omacal_store::upsert_event(&pool, &declined).await.unwrap();

        let mut tentative = event("e2", T0900Z, T0900Z + HOUR, own(10));
        tentative.self_response = Some("tentative".into());
        omacal_store::upsert_event(&pool, &tentative).await.unwrap();

        let (_, posted) = pass_at(&pool, T0900Z - 10 * MINUTE).await;
        assert_eq!(posted, vec![(2, 10)], "declined stays silent; a maybe still rings");
    }

    /// Declining one occurrence of a series silences that occurrence alone.
    /// Google records the decline on an exception row; the exception suppresses
    /// its parent's slot and is itself skipped, while the rest of the series
    /// keeps its reminders.
    #[tokio::test]
    async fn a_declined_occurrence_of_a_series_notifies_nothing_while_the_series_still_fires() {
        let pool = seeded("UTC", "[]").await;

        let mut master = event("m1", T0900Z, T0900Z + HOUR, own(10));
        master.recurrence = Some("RRULE:FREQ=DAILY".into());
        omacal_store::upsert_event(&pool, &master).await.unwrap();

        // Tomorrow's occurrence is declined — an exception row, still confirmed.
        let mut declined = event("m1_20260811", T0900Z + DAY, T0900Z + DAY + HOUR, own(10));
        declined.self_response = Some("declined".into());
        declined.recurring_event_id = Some("m1".into());
        declined.original_start_utc = Some(T0900Z + DAY);
        omacal_store::upsert_event(&pool, &declined).await.unwrap();

        // Ten minutes before the declined occurrence: silence.
        let (_, posted) = pass_at(&pool, T0900Z + DAY - 10 * MINUTE).await;
        assert!(posted.is_empty(), "a declined occurrence must not notify");

        // Ten minutes before the day after's occurrence: the series is intact.
        let (_, posted) = pass_at(&pool, T0900Z + 2 * DAY - 10 * MINUTE).await;
        assert_eq!(posted, vec![(1, 10)], "declining one occurrence must not silence the series");
    }

    /// A deleted occurrence of a series must not notify, any more than it
    /// should draw. The cancelled exception is what records the deletion.
    #[tokio::test]
    async fn a_deleted_occurrence_of_a_series_notifies_nothing() {
        let pool = seeded("UTC", "[]").await;

        let mut master = event("m1", T0900Z, T0900Z + HOUR, own(10));
        master.recurrence = Some("RRULE:FREQ=DAILY".into());
        omacal_store::upsert_event(&pool, &master).await.unwrap();

        // Tomorrow's occurrence is deleted.
        let mut gone = event("m1_20260811", T0900Z + DAY, T0900Z + DAY + HOUR, own(10));
        gone.status = "cancelled".into();
        gone.recurring_event_id = Some("m1".into());
        gone.original_start_utc = Some(T0900Z + DAY);
        omacal_store::upsert_event(&pool, &gone).await.unwrap();

        // Ten minutes before the *deleted* occurrence would have started.
        let (_, posted) = pass_at(&pool, T0900Z + DAY - 10 * MINUTE).await;
        assert!(posted.is_empty(), "a cancelled occurrence must not notify");
    }

    /// The all-day case, keyed by the anchor rather than the raw start, and the
    /// fixture is the divergence: the event was stored as midnight in
    /// `Europe/Sofia` while its calendar now says `Pacific/Auckland`. Recording
    /// the raw start would write a key that the next pass fails to match, and
    /// every all-day reminder would re-post after every restart.
    #[tokio::test]
    async fn an_all_day_reminder_is_recorded_under_the_anchor_the_pure_function_computes() {
        let pool = seeded("Pacific/Auckland", "[]").await;

        let mut ev = event("d1", SOFIA_MIDNIGHT_AUG10, SOFIA_MIDNIGHT_AUG10 + DAY, own(30));
        ev.is_all_day = true;
        ev.start_tz = "Europe/Sofia".into();
        omacal_store::upsert_event(&pool, &ev).await.unwrap();

        assert_ne!(
            SOFIA_MIDNIGHT_AUG10, AUCKLAND_MIDNIGHT_AUG10,
            "fixture check: the stored start must not already be the anchor"
        );

        let (_, posted) = pass_at(&pool, AUCKLAND_MIDNIGHT_AUG10 - 30 * MINUTE).await;
        assert_eq!(posted, vec![(1, 30)]);
        assert_eq!(
            omacal_store::fired_keys(&pool).await.unwrap(),
            vec![(1, AUCKLAND_MIDNIGHT_AUG10, 30)],
            "the recorded key must be the anchor, not the instant the store held"
        );

        // The recorded *end* is the occurrence's own, not one derived from the
        // anchor. Asserted directly, because its only visible effect is when
        // the row is pruned — and an end that is wrong by the offset between
        // the two zones prunes the row early and re-posts the reminder, which
        // takes three passes to show up. This states it in one.
        let end_ms: i64 = sqlx::query_scalar("SELECT occurrence_end_ms FROM fired_reminders")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(
            end_ms,
            SOFIA_MIDNIGHT_AUG10 + DAY,
            "the occurrence's real end must be recorded, or the row is pruned early"
        );

        // And the record must actually suppress it on a rebuild.
        let (_, again) = pass_at(&pool, AUCKLAND_MIDNIGHT_AUG10 + HOUR).await;
        assert!(again.is_empty(), "an all-day reminder re-posted after a restart");

        // A third pass, and it is the one that matters. `run_once` prunes
        // *after* it consults the record, so a row pruned wrongly on pass two
        // is still present when pass two needs it — the damage only surfaces on
        // the pass after that. Two passes stopped one short of the bug.
        let (_, third) = pass_at(&pool, AUCKLAND_MIDNIGHT_AUG10 + 2 * HOUR).await;
        assert!(third.is_empty(), "the record was pruned early and the reminder came back");
    }

    /// The row for a long occurrence survives while it is still running.
    /// Pruned on the anchor plus a horizon, a week-long trip loses its record
    /// on day three and the missed rule posts it all over again.
    #[tokio::test]
    async fn a_still_running_occurrence_keeps_its_record_past_the_horizon() {
        let pool = seeded("UTC", "[]").await;
        omacal_store::upsert_event(&pool, &event("trip", T0900Z, T0900Z + 7 * DAY, own(30)))
            .await.unwrap();

        let (_, posted) = pass_at(&pool, T0900Z - 30 * MINUTE).await;
        assert_eq!(posted, vec![(1, 30)]);

        // Three days in — well past the 48-hour horizon, still mid-trip.
        let (pass, later) = pass_at(&pool, T0900Z + 3 * DAY).await;
        assert_eq!(pass.pruned, 0, "the record must outlive the horizon while the event runs");
        assert!(later.is_empty(), "and so the reminder must not fire a second time");
    }

    #[tokio::test]
    async fn a_finished_occurrences_record_is_forgotten() {
        let pool = seeded("UTC", "[]").await;
        omacal_store::upsert_event(&pool, &event("e1", T0900Z, T0900Z + HOUR, own(10)))
            .await.unwrap();

        pass_at(&pool, T0900Z - 10 * MINUTE).await;
        let (pass, _) = pass_at(&pool, T0900Z + HOUR).await;

        assert_eq!(pass.pruned, 1);
        assert!(omacal_store::fired_keys(&pool).await.unwrap().is_empty());
    }

    /// The calendar's defaults reach the driver, so an event that defers fires
    /// what the calendar says. Without the join this is silently nothing.
    #[tokio::test]
    async fn an_event_deferring_to_its_calendar_fires_the_calendars_defaults() {
        let pool = seeded("UTC", r#"[{"method":"popup","minutes":45}]"#).await;
        let ev = event(
            "e1",
            T0900Z,
            T0900Z + HOUR,
            Reminders { use_default: true, overrides: Vec::new() },
        );
        omacal_store::upsert_event(&pool, &ev).await.unwrap();

        let (_, posted) = pass_at(&pool, T0900Z - 45 * MINUTE).await;
        assert_eq!(posted, vec![(1, 45)], "the calendar's default reminder did not reach here");
    }

    /// What a given clock actually puts in front of the user — the whole point
    /// of the feature, asserted end to end.
    #[tokio::test]
    async fn what_is_posted_for_a_given_clock_carries_the_events_title_and_time() {
        let pool = seeded("UTC", "[]").await;
        let mut ev = event("e1", T0900Z, T0900Z + HOUR, own(10));
        ev.summary = Some("Weekly Standup".into());
        ev.location = Some("Room 1".into());
        omacal_store::upsert_event(&pool, &ev).await.unwrap();

        let fake = crate::notify::RecordingNotifier::default();
        pass_with(&pool, T0900Z - 10 * MINUTE, &fake).await;

        let posted = fake.posted();
        assert_eq!(posted.len(), 1, "exactly one notification for one due reminder");
        assert_eq!(posted[0].title, "Weekly Standup");
        assert_eq!(
            posted[0].body, "12:00 · Room 1",
            "09:00Z read in Europe/Sofia is 12:00, and the location follows it"
        );
    }

    /// **The Join rule, through the driver.** A meeting with a conferencing
    /// link offers Join; one without must not — there would be nothing for the
    /// button to open.
    ///
    /// Both events in one pass, so the fixture set contains a link and an
    /// absence. A pass where everything had a link could not witness this.
    #[tokio::test]
    async fn join_is_offered_only_for_the_occurrence_that_has_a_conference_link() {
        let pool = seeded("UTC", "[]").await;

        let mut online = event("e1", T0900Z, T0900Z + HOUR, own(10));
        online.summary = Some("Design review".into());
        online.conference_uri = Some("https://meet.google.com/abc".into());
        omacal_store::upsert_event(&pool, &online).await.unwrap();

        let mut in_person = event("e2", T0900Z, T0900Z + HOUR, own(10));
        in_person.summary = Some("Coffee".into());
        in_person.conference_uri = None;
        omacal_store::upsert_event(&pool, &in_person).await.unwrap();

        let fake = crate::notify::RecordingNotifier::default();
        pass_with(&pool, T0900Z - 10 * MINUTE, &fake).await;

        let posted = fake.posted();
        assert_eq!(posted.len(), 2, "fixture check: both meetings must notify");

        let join_uri = |title: &str| {
            posted
                .iter()
                .find(|n| n.title == title)
                .unwrap_or_else(|| panic!("nothing posted for {title}"))
                .actions
                .iter()
                .find_map(|a| match a {
                    crate::notify::Action::Join(u) => Some(u.clone()),
                    _ => None,
                })
        };

        assert_eq!(join_uri("Design review").as_deref(), Some("https://meet.google.com/abc"));
        assert_eq!(join_uri("Coffee"), None, "a meeting with nowhere to join must not offer Join");
    }

    /// §2.4: on an unsigned macOS bundle the notification centre may simply
    /// refuse. That is expected, so the pass must not fail — and the reminder
    /// must still be recorded, or every later pass re-attempts it for as long
    /// as the meeting runs and the log fills with the same failure.
    #[tokio::test]
    async fn a_notifier_that_refuses_is_tolerated_and_the_reminder_is_still_recorded() {
        let pool = seeded("UTC", "[]").await;
        omacal_store::upsert_event(&pool, &event("e1", T0900Z, T0900Z + HOUR, own(10)))
            .await.unwrap();

        let failing = crate::notify::RecordingNotifier::failing();
        let pass = run_once(&pool, false, T0900Z - 10 * MINUTE, HORIZON_MS, SOFIA, &failing)
            .await
            .expect("a refusing transport must not fail the pass");

        assert_eq!(failing.posted().len(), 1, "it must still have been attempted");
        assert_eq!(pass.posted.len(), 1);
        assert_eq!(
            omacal_store::fired_keys(&pool).await.unwrap(),
            vec![(1, T0900Z, 10)],
            "a refused reminder must still be recorded, or it is retried forever"
        );

        // And the next pass, mid-meeting, does not attempt it again.
        let second = crate::notify::RecordingNotifier::default();
        pass_with(&pool, T0900Z + 5 * MINUTE, &second).await;
        assert!(second.posted().is_empty(), "the record must suppress the retry");
    }

    /// **The fourth demo enforcement point** (§2.7), beside the separate
    /// database, `demo_sync_guard`, and `should_sync`/`may_sync`.
    ///
    /// Demo mode seeds synthetic events *in the present* precisely so the views
    /// look alive, which is exactly what makes an unguarded scheduler buzz
    /// about meetings that do not exist.
    ///
    /// Both halves in one test, and that is the point of it. The demo half
    /// alone would pass against a build where notifications were simply broken
    /// — the same fixture, the same clock and the same call must post when
    /// `demo` is false, or this proves nothing about the guard. It runs through
    /// `run_once`, the path a real pass takes, rather than asserting about a
    /// flag somewhere upstream.
    #[tokio::test]
    async fn demo_mode_posts_nothing_through_the_same_path_a_real_run_posts_on() {
        let pool = seeded("UTC", "[]").await;
        omacal_store::upsert_event(&pool, &event("e1", T0900Z, T0900Z + HOUR, own(10)))
            .await.unwrap();
        let now = T0900Z - 10 * MINUTE;

        let in_demo = crate::notify::RecordingNotifier::default();
        let pass = run_once(&pool, true, now, HORIZON_MS, SOFIA, &in_demo).await.unwrap();

        assert!(in_demo.posted().is_empty(), "demo mode must post nothing at all");
        assert!(pass.posted.is_empty());
        assert!(
            omacal_store::fired_keys(&pool).await.unwrap().is_empty(),
            "demo mode must not record either: a reminder recorded but never posted is \
             swallowed for good the moment demo mode is turned off"
        );

        // The same event, the same clock, the same call — with the guard off.
        let for_real = crate::notify::RecordingNotifier::default();
        run_once(&pool, false, now, HORIZON_MS, SOFIA, &for_real).await.unwrap();

        assert_eq!(
            for_real.posted().len(),
            1,
            "the guard must block demo mode, not disable notifications altogether"
        );
    }

    /// Named and separate for the same reason `may_sync` is: the decision is
    /// worth being able to point at, and every caller routes through it rather
    /// than carrying its own untested `if demo`.
    #[test]
    fn only_a_real_run_may_notify() {
        assert!(may_notify(false, true));
        assert!(!may_notify(true, true), "demo mode posts no notifications at all");
    }

    /// The user's own switch, and it is **not** the same question as demo mode:
    /// both have to be able to stop reminders on their own, or turning them off
    /// in a real build would depend on a flag the user cannot see.
    #[test]
    fn reminders_turned_off_post_nothing_even_in_a_real_run() {
        assert!(!may_notify(false, false), "the switch must work on its own");
        assert!(!may_notify(true, false));
    }

    #[test]
    fn the_next_wake_is_the_sooner_of_the_next_reminder_and_the_next_sync() {
        let sync = 5 * MINUTE;
        assert_eq!(
            next_wake_ms(Some(T0900Z + MINUTE), T0900Z, sync),
            MINUTE,
            "a reminder before the next sync wins"
        );
        assert_eq!(
            next_wake_ms(Some(T0900Z + HOUR), T0900Z, sync),
            sync,
            "a reminder after the next sync must not delay the sync"
        );
        assert_eq!(next_wake_ms(None, T0900Z, sync), sync, "nothing scheduled: wait for the sync");
    }

    /// A fire-time already behind the clock must never become a negative sleep.
    #[test]
    fn a_fire_time_already_past_wakes_immediately_rather_than_sleeping_backwards() {
        assert_eq!(next_wake_ms(Some(T0900Z - HOUR), T0900Z, 5 * MINUTE), 0);
        assert_eq!(next_wake_ms(Some(T0900Z), T0900Z, 5 * MINUTE), 0);
    }

    /// The fallback (fallback spec §1), driven through a full pass: a timed
    /// event that follows its calendar's defaults, on a calendar that has
    /// none, fires the *shipped* fallback — 60 and 10 — with no settings row
    /// written at all. This is exactly the meeting that was silent before.
    #[tokio::test]
    async fn a_defaults_following_event_on_a_bare_calendar_fires_the_fallback() {
        let pool = seeded("UTC", "[]").await;
        let follows = Reminders { use_default: true, overrides: Vec::new() };
        omacal_store::upsert_event(&pool, &event("e1", T0900Z, T0900Z + HOUR, follows))
            .await.unwrap();

        let (_, posted) = pass_at(&pool, T0900Z - 10 * MINUTE).await;
        assert_eq!(
            posted,
            vec![(1, 60), (1, 10)],
            "both shipped rows are due at T-10: the 60 was missed, the 10 is now"
        );

        // And the setting turned off is the end of it — `[]` stored must not
        // read back as the shipped default.
        let pool = seeded("UTC", "[]").await;
        crate::settings::store(&pool, &crate::settings::Setting::FallbackReminderMinutes(vec![])).await.unwrap();
        let follows = Reminders { use_default: true, overrides: Vec::new() };
        omacal_store::upsert_event(&pool, &event("e2", T0900Z, T0900Z + HOUR, follows))
            .await.unwrap();
        let (_, posted) = pass_at(&pool, T0900Z - 10 * MINUTE).await;
        assert!(posted.is_empty(), "an emptied fallback list is the feature off");
    }

    // --- tasks (#137) ------------------------------------------------------

    /// A task on the seeded calendar, due at `due_ms`, with `alarm` written
    /// into its resource verbatim (empty for none).
    async fn seed_task(pool: &SqlitePool, id: &str, summary: &str, due_ms: i64, alarm: &str) -> i64 {
        let ics = format!(
            "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:{id}\r\nSUMMARY:{summary}\r\n\
             DUE:20260810T090000Z\r\nSTATUS:NEEDS-ACTION\r\n{alarm}END:VTODO\r\nEND:VCALENDAR\r\n"
        );
        omacal_store::upsert_task(
            pool,
            &omacal_store::StoredTask {
                id: 0, calendar_id: 1, uid: id.into(), etag: None, caldav_href: None,
                summary: Some(summary.into()), description: None,
                due_utc: Some(due_ms), due_tz: Some("Europe/Sofia".into()), due_all_day: false,
                status: "needs-action".into(), completed_utc: None, priority: 0,
                updated_at: 0, raw_ics: Some(ics),
            },
        )
        .await
        .unwrap()
    }

    /// The whole of a task's announcement: it speaks at its due, says what it
    /// is and when, is recorded, and never speaks for that due again.
    #[tokio::test]
    async fn a_task_due_now_is_announced_once_and_recorded() {
        let pool = seeded(SOFIA, "[]").await;
        let id = seed_task(&pool, "t-1", "Call the bank", T0900Z, "").await;
        let fake = crate::notify::RecordingNotifier::default();

        let pass = run_once(&pool, false, T0900Z, HORIZON_MS, SOFIA, &fake).await.unwrap();
        assert_eq!(
            pass.posted_tasks.iter().map(|d| d.key.task_id).collect::<Vec<_>>(),
            vec![id],
        );
        let posted = fake.posted();
        assert_eq!(posted.len(), 1);
        assert_eq!(posted[0].title, "Call the bank");
        assert_eq!(posted[0].body, "Due 12:00 · Work", "the hour it is due, and its list");
        assert_eq!(
            posted[0].actions,
            vec![crate::notify::Action::OpenTask { task_id: id }],
            "one action: show me",
        );
        assert_eq!(omacal_store::fired_task_keys(&pool).await.unwrap(), vec![(id, T0900Z)]);

        // A second pass, even a restart, says nothing more.
        let again = run_once(&pool, false, T0900Z + MINUTE, HORIZON_MS, SOFIA, &fake).await.unwrap();
        assert!(again.posted_tasks.is_empty());
        assert_eq!(fake.posted().len(), 1);
    }

    /// A task carrying an alarm of its own speaks when the alarm does, so the
    /// desktop and the phone say the same thing at the same moment.
    #[tokio::test]
    async fn a_task_with_its_own_alarm_speaks_at_that_lead() {
        let pool = seeded(SOFIA, "[]").await;
        let alarm = "BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Reminder\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n";
        seed_task(&pool, "t-2", "Leave for the airport", T0900Z, alarm).await;

        // Twenty minutes before the due: not yet, and the wake is its lead.
        let early = run_once(&pool, false, T0900Z - 20 * MINUTE, HORIZON_MS, SOFIA, &crate::notify::RecordingNotifier::default()).await.unwrap();
        assert!(early.posted_tasks.is_empty());
        assert_eq!(early.next_fire_ms, Some(T0900Z - 15 * MINUTE));

        let fake = crate::notify::RecordingNotifier::default();
        let at = run_once(&pool, false, T0900Z - 15 * MINUTE, HORIZON_MS, SOFIA, &fake).await.unwrap();
        assert_eq!(at.posted_tasks.len(), 1);
        assert_eq!(fake.posted()[0].body, "Due 12:00 · Work", "still the due, not the alarm");
    }

    /// The missed rule, and its bound: a due that passed while OmaCal was shut
    /// speaks once at the next launch, and one older than a day says nothing —
    /// otherwise the first launch after this shipped would announce every
    /// timed task anyone had left undone.
    #[tokio::test]
    async fn a_missed_task_speaks_once_while_it_is_still_news() {
        let pool = seeded(SOFIA, "[]").await;
        seed_task(&pool, "recent", "Three hours ago", T0900Z - 3 * HOUR, "").await;
        seed_task(&pool, "stale", "Two days ago", T0900Z - 2 * DAY, "").await;

        let fake = crate::notify::RecordingNotifier::default();
        let pass = run_once(&pool, false, T0900Z, HORIZON_MS, SOFIA, &fake).await.unwrap();
        assert_eq!(
            pass.posted_tasks.iter().map(|d| d.title.clone().unwrap()).collect::<Vec<_>>(),
            vec!["Three hours ago".to_string()],
        );
    }

    /// Completing a task calls its announcement off, and its record is
    /// forgotten with it — a task reopened later with the same due is news
    /// again.
    #[tokio::test]
    async fn completing_a_task_calls_off_its_announcement() {
        let pool = seeded(SOFIA, "[]").await;
        let id = seed_task(&pool, "t-3", "Pay the rent", T0900Z + HOUR, "").await;
        omacal_store::mark_task_status(&pool, id, "completed", Some(T0900Z), None, None, T0900Z)
            .await
            .unwrap();

        let fake = crate::notify::RecordingNotifier::default();
        let pass = run_once(&pool, false, T0900Z + HOUR, HORIZON_MS, SOFIA, &fake).await.unwrap();
        assert!(pass.posted_tasks.is_empty(), "a done task has nothing to announce");
        assert!(fake.posted().is_empty());
    }

    /// The two switches are apart (#137): either half can be silent while the
    /// other speaks.
    #[tokio::test]
    async fn each_half_is_switched_on_its_own() {
        let pool = seeded(SOFIA, "[]").await;
        seed_task(&pool, "t-4", "Call the bank", T0900Z, "").await;
        omacal_store::upsert_event(&pool, &event("ev", T0900Z + HOUR, T0900Z + 2 * HOUR, own(60)))
            .await
            .unwrap();

        crate::settings::write(&pool, "task_notifications_enabled", "0").await.unwrap();
        let fake = crate::notify::RecordingNotifier::default();
        let pass = run_once(&pool, false, T0900Z, HORIZON_MS, SOFIA, &fake).await.unwrap();
        assert!(pass.posted_tasks.is_empty(), "tasks are quiet");
        assert_eq!(pass.posted.len(), 1, "and the meeting still speaks");

        crate::settings::write(&pool, "task_notifications_enabled", "1").await.unwrap();
        crate::settings::write(&pool, "notifications_enabled", "0").await.unwrap();
        let fake = crate::notify::RecordingNotifier::default();
        let pass = run_once(&pool, false, T0900Z, HORIZON_MS, SOFIA, &fake).await.unwrap();
        assert_eq!(pass.posted_tasks.len(), 1, "and now the other way round");
        assert!(pass.posted.is_empty());
    }
}
