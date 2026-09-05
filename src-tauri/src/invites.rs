//! Announcing new invitations, and answering one from its notification.
//!
//! The pass runs after every successful sync: whatever `unanswered_invites`
//! returns that survived the ledger is, by construction, news — the store
//! seeds each calendar's pre-existing backlog silently on its first pass
//! (`omacal-store/src/invites.rs`), so an announcement here means an
//! invitation that arrived while omacal was watching.
//!
//! The notification's click is the acceptance — [`crate::notify::Action`]
//! documents why it is one action and not three — and the click lands back
//! here in [`accept_from_notification`], which runs the same write path as
//! the popover's Yes button and then says out loud whether it worked: the
//! notification is gone by then, and silence would leave "did that count?"
//! hanging on a write to somebody's real calendar.

use omacal_store::InviteCandidate;
use sqlx::SqlitePool;
use tauri::{Emitter, Manager};

use crate::notify::{Action, Notification, Notifier};

/// What one pass did — how many notifications went out, how many backlog
/// rows were swallowed silently.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct InvitePass {
    pub posted: Vec<i64>,
    pub seeded: usize,
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `Wed, Aug 19` for an instant read in `tz` — a reminder fires minutes ahead
/// and says only the hour, but an invitation may be for weeks away, so the
/// day is the headline. Unknown zones fall back to UTC, the same policy as
/// `notify::time_in_zone_with_format`.
fn date_in_words(ms: i64, tz: &str) -> String {
    let ts = jiff::Timestamp::from_millisecond(ms).unwrap_or(jiff::Timestamp::UNIX_EPOCH);
    let z = ts.in_tz(tz).unwrap_or_else(|_| ts.in_tz("UTC").expect("UTC always resolves"));
    let weekday = WEEKDAYS[z.weekday().to_monday_zero_offset() as usize];
    let month = MONTHS[z.month() as usize - 1];
    format!("{weekday}, {month} {}", z.day())
}

/// Whether the notification may carry the accepting click at all: an RSVP
/// write exists only for Google, and past the provider it is the popover's
/// own rule — [`crate::events::can_respond`] — with `demo` answered `false`
/// because [`run_pass`] never reaches here in demo mode.
pub(crate) fn offers_accept(c: &InviteCandidate) -> bool {
    c.provider == "google" && crate::events::can_respond(false, &c.access_role, &c.attendees)
}

/// What one invitation's announcement says.
///
/// The date is read where the user is (`display_tz`) for a timed event, and
/// in the **calendar's** zone for an all-day one — the store anchors an
/// all-day date at midnight in that zone, and reading it anywhere else shifts
/// the day for any user east or west of the calendar.
///
/// The "Click to accept" line appears only when the click actually does that:
/// an action-less announcement (CalDAV, a read-only calendar, macOS's
/// button-less transport) must not instruct anyone to click.
pub(crate) fn invite_notification_with_format(
    c: &InviteCandidate,
    display_tz: &str,
    time_format: crate::settings::TimeFormat,
) -> Notification {
    let when = if c.is_all_day {
        format!("{} · All day", date_in_words(c.start_utc, &c.calendar_timezone))
    } else {
        format!(
            "{} · {}",
            date_in_words(c.start_utc, display_tz),
            crate::notify::time_in_zone_with_format(c.start_utc, display_tz, time_format)
        )
    };

    let mut body = when;
    if let Some(org) = c.organizer_email.as_deref().filter(|o| !o.is_empty()) {
        body.push_str(" · from ");
        body.push_str(org);
    }

    let actions = if offers_accept(c) && cfg!(target_os = "linux") {
        vec![Action::AcceptInvite { event_id: c.event_id, start_ms: c.start_utc }]
    } else {
        Vec::new()
    };
    // A sticky card with no visible control needs its ways out spelled on
    // the card itself — "how do you dismiss it?" was the first question the
    // sticky toast earned in the field (2026-08-17). Omarchy's shell draws
    // no buttons and no close cross; the body line is the one surface this
    // app owns there.
    if !actions.is_empty() {
        body.push_str("\nClick to accept · right-click to dismiss");
    } else if cfg!(target_os = "linux") {
        body.push_str("\nRight-click to dismiss");
    }

    Notification {
        title: format!(
            "Invitation: {}",
            c.summary.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| crate::notify::NO_TITLE.into())
        ),
        body,
        actions,
        // An invitation waits for an answer, so its announcement waits for
        // one too — the first live click test failed precisely because this
        // expired into history, click and all, while the user read another
        // window. Sticky whether or not the click is offered: a CalDAV
        // invite still deserves to be seen, and dismissal is one right-click.
        sticky: true,
    }
}

/// A 24-hour wrapper for tests whose subject is invitation wording or actions
/// rather than the clock. The live pass has no default and always supplies
/// the stored preference.
#[cfg(test)]
pub(crate) fn invite_notification(c: &InviteCandidate, display_tz: &str) -> Notification {
    invite_notification_with_format(c, display_tz, crate::settings::TimeFormat::H24)
}

/// One pass over the ledger: seed what predates the watch, announce what is
/// new, skip what is hidden.
///
/// Gated exactly as reminders are ([`crate::notify_loop::may_notify`]): demo
/// mode and the notifications switch silence this too — an invitation
/// announcement is a notification, and "notifications off" that still buzzed
/// about invites would be a switch that lies.
///
/// Posting precedes recording, the order `run_once` documents: a crash in
/// between re-announces one invitation on the next pass, where the other
/// order swallows it forever. A notifier that refuses is logged and the
/// invitation recorded anyway, also for `run_once`'s reason — a transport
/// that is down must not turn one invitation into a retry loop.
///
/// A hidden calendar's invitation is skipped *without being recorded*: hiding
/// a calendar mutes it (the reminder rule), but un-hiding it should surface
/// an invitation that is still unanswered, not reveal that it was silently
/// consumed weeks ago.
pub(crate) async fn run_pass(
    pool: &SqlitePool,
    demo: bool,
    now_ms: i64,
    display_tz: &str,
    notifier: &dyn Notifier,
) -> anyhow::Result<InvitePass> {
    let settings = crate::settings::read_settings(pool).await;
    if !crate::notify_loop::may_notify(demo, settings.notifications_enabled) {
        return Ok(InvitePass::default());
    }

    let unseeded: std::collections::HashSet<i64> =
        omacal_store::unseeded_calendars(pool).await?.into_iter().collect();
    let candidates = omacal_store::unanswered_invites(pool, now_ms).await?;

    let mut pass = InvitePass::default();
    for c in candidates {
        if unseeded.contains(&c.calendar_id) {
            omacal_store::record_invite_notice(pool, c.event_id, false, now_ms).await?;
            pass.seeded += 1;
            continue;
        }
        if !c.calendar_selected {
            continue;
        }
        if let Err(e) = notifier.post(&invite_notification_with_format(
            &c,
            display_tz,
            settings.time_format,
        )) {
            tracing::warn!(%e, event_id = c.event_id, "could not announce an invitation");
        }
        omacal_store::record_invite_notice(pool, c.event_id, true, now_ms).await?;
        pass.posted.push(c.event_id);
    }

    // Only after their rows are in the ledger — the other order, interrupted,
    // marks a calendar seeded with its backlog still eligible.
    for calendar_id in unseeded {
        omacal_store::mark_invites_seeded(pool, calendar_id, now_ms).await?;
    }
    Ok(pass)
}

/// One row of the app's invitation tray — the in-app answer to a missed
/// notification (a toast can evaporate; this list cannot). The UI's shape,
/// ready to render: times as instants, the all-day days as calendar-zone
/// dates (the store's all-day instants are foreign-zone midnights the
/// browser must never re-derive — `EventDetail::start_date` documents that
/// trap), and `can_respond` already decided so the buttons appear exactly
/// where the popover's would.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct PendingInvite {
    pub id: i64,
    pub title: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub is_all_day: bool,
    /// The first and last day an all-day invite covers (`yyyy-mm-dd`,
    /// calendar zone, last day inclusive); `None` for a timed one.
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub organizer_email: Option<String>,
    pub color: Option<String>,
    pub can_respond: bool,
}

fn to_pending(c: &InviteCandidate, demo: bool) -> PendingInvite {
    let (start_date, end_date) = if c.is_all_day {
        let (s, e) =
            crate::write::all_day_span_dates(c.start_utc, c.end_utc, &c.calendar_timezone);
        (Some(s), Some(e))
    } else {
        (None, None)
    };
    PendingInvite {
        id: c.event_id,
        title: c.summary.clone(),
        start_ms: c.start_utc,
        end_ms: c.end_utc,
        is_all_day: c.is_all_day,
        start_date,
        end_date,
        organizer_email: c.organizer_email.clone(),
        color: c.color_hex.clone(),
        // The popover's own gate, provider included — a CalDAV invitation
        // lists (it is real, and unanswered) but carries no buttons, since
        // no RSVP write exists for it.
        can_respond: c.provider == "google"
            && crate::events::can_respond(demo, &c.access_role, &c.attendees),
    }
}

pub(crate) async fn pending_invites_impl(
    pool: &SqlitePool,
    demo: bool,
    now_ms: i64,
) -> anyhow::Result<Vec<PendingInvite>> {
    Ok(omacal_store::pending_invites(pool, now_ms)
        .await?
        .iter()
        .map(|c| to_pending(c, demo))
        .collect())
}

/// What the header's invitation badge and tray render — see [`PendingInvite`].
#[tauri::command]
pub(crate) async fn pending_invites(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<PendingInvite>, String> {
    pending_invites_impl(&state.pool, state.demo, crate::now_ms())
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

/// One guest who declined one of the user's own meetings — the organizer's
/// side of the tray, in-app only by request (2026-08-18): no toast, no
/// widget, just the row and its ×. Shape mirrors [`PendingInvite`], plus the
/// stable ids the dismissal is recorded under.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct DeclineNotice {
    pub calendar_id: i64,
    pub gid: String,
    pub email: String,
    pub display_name: Option<String>,
    pub title: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub is_all_day: bool,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub color: Option<String>,
}

pub(crate) async fn declined_guests_impl(
    pool: &SqlitePool,
    now_ms: i64,
) -> anyhow::Result<Vec<DeclineNotice>> {
    Ok(omacal_store::declined_guests(pool, now_ms)
        .await?
        .into_iter()
        .map(|d| {
            let (start_date, end_date) = if d.is_all_day {
                let (s, e) =
                    crate::write::all_day_span_dates(d.start_utc, d.end_utc, &d.calendar_timezone);
                (Some(s), Some(e))
            } else {
                (None, None)
            };
            DeclineNotice {
                calendar_id: d.calendar_id,
                gid: d.gid,
                email: d.email,
                display_name: d.display_name,
                title: d.summary,
                start_ms: d.start_utc,
                end_ms: d.end_utc,
                is_all_day: d.is_all_day,
                start_date,
                end_date,
                color: d.color_hex,
            }
        })
        .collect())
}

/// What the tray's "declined" section renders — see [`DeclineNotice`].
#[tauri::command]
pub(crate) async fn declined_guests(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<DeclineNotice>, String> {
    declined_guests_impl(&state.pool, crate::now_ms())
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

/// The tray's "Dismiss all": every currently listed decline acknowledged in
/// one stroke. Returns the count, purely for the log.
#[tauri::command]
pub(crate) async fn dismiss_all_decline_notices(
    state: tauri::State<'_, crate::AppState>,
) -> Result<usize, String> {
    omacal_store::dismiss_all_declines(&state.pool, crate::now_ms())
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

/// The ×: acknowledges one guest's decline of one meeting.
#[tauri::command]
pub(crate) async fn dismiss_decline_notice(
    state: tauri::State<'_, crate::AppState>,
    calendar_id: i64,
    gid: String,
    email: String,
) -> Result<(), String> {
    omacal_store::dismiss_decline(&state.pool, calendar_id, &gid, &email, crate::now_ms())
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

/// One meeting that moved or was cancelled under the user — the tray's
/// Rescheduled/Cancelled sections, in-app only like the declines. The
/// all-day date strings follow the same calendar-zone rule as everything
/// else in this file.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct ChangeNotice {
    pub calendar_id: i64,
    pub gid: String,
    /// `moved` | `cancelled`.
    pub kind: String,
    pub title: Option<String>,
    pub is_all_day: bool,
    pub old_start_ms: i64,
    pub old_end_ms: Option<i64>,
    pub new_start_ms: Option<i64>,
    pub new_end_ms: Option<i64>,
    /// First covered day (`yyyy-mm-dd`, calendar zone) of the old and new
    /// slots for an all-day meeting; `None` for timed ones.
    pub old_start_date: Option<String>,
    pub new_start_date: Option<String>,
    pub color: Option<String>,
    /// The live row to answer, for a move — "can you still make the new
    /// time?" is an RSVP like any other (2026-08-21, by request). `None`
    /// for a cancellation, which has nothing left to answer.
    pub event_id: Option<i64>,
    /// `this` | `all`: a moved exception answers one occurrence, a moved
    /// master answers the series — the popover's own scope vocabulary.
    pub respond_scope: String,
    /// What to hand `respond_to_event` as the occurrence start: the slot
    /// the meeting now occupies.
    pub respond_start_ms: Option<i64>,
    /// The popover's own gate, decided here like `PendingInvite`'s: buttons
    /// appear exactly where an answer can actually be sent.
    pub can_respond: bool,
}

const DAY_MS: i64 = 24 * 3_600_000;

pub(crate) async fn changed_meetings_impl(
    pool: &SqlitePool,
    demo: bool,
    now_ms: i64,
) -> anyhow::Result<Vec<ChangeNotice>> {
    Ok(omacal_store::changed_meetings(pool, now_ms)
        .await?
        .into_iter()
        .map(|m| {
            let day_of = |start: i64, end: Option<i64>| {
                crate::write::all_day_span_dates(
                    start,
                    end.unwrap_or(start + DAY_MS),
                    &m.calendar_timezone,
                )
                .0
            };
            let (old_start_date, new_start_date) = if m.is_all_day {
                (
                    Some(day_of(m.old_start_utc, m.old_end_utc)),
                    m.new_start_utc.map(|s| day_of(s, m.new_end_utc)),
                )
            } else {
                (None, None)
            };
            let can_respond = m.event_id.is_some()
                && m.provider == "google"
                && crate::events::can_respond(demo, &m.access_role, &m.attendees);
            ChangeNotice {
                calendar_id: m.calendar_id,
                gid: m.gid,
                kind: m.kind,
                title: m.summary,
                is_all_day: m.is_all_day,
                old_start_ms: m.old_start_utc,
                old_end_ms: m.old_end_utc,
                new_start_ms: m.new_start_utc,
                new_end_ms: m.new_end_utc,
                old_start_date,
                new_start_date,
                color: m.color_hex,
                event_id: m.event_id,
                respond_scope: if m.respond_all { "all".into() } else { "this".into() },
                respond_start_ms: m.new_start_utc,
                can_respond,
            }
        })
        .collect())
}

/// What the tray's Rescheduled and Cancelled sections render.
#[tauri::command]
pub(crate) async fn changed_meetings(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<ChangeNotice>, String> {
    changed_meetings_impl(&state.pool, state.demo, crate::now_ms())
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

/// The ×: acknowledges one change.
#[tauri::command]
pub(crate) async fn dismiss_change_notice(
    state: tauri::State<'_, crate::AppState>,
    calendar_id: i64,
    gid: String,
) -> Result<(), String> {
    omacal_store::dismiss_change(&state.pool, calendar_id, &gid)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

/// One section's Dismiss all — `kind` is `moved` or `cancelled`.
#[tauri::command]
pub(crate) async fn dismiss_all_change_notices(
    state: tauri::State<'_, crate::AppState>,
    kind: String,
) -> Result<usize, String> {
    omacal_store::dismiss_all_changes(&state.pool, &kind, crate::now_ms())
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

/// Runs the invite pass with the app's own state, after a sync. Failures are
/// logged and dropped — an announcement is never worth failing a sync over.
pub(crate) async fn after_sync(app: &tauri::AppHandle) {
    let Some(notifier) = app.try_state::<crate::NotifierHandle>() else {
        return; // setup has not wired a transport (tests, early startup)
    };
    let (pool, demo) = {
        let state = app.state::<crate::AppState>();
        (state.pool.clone(), state.demo)
    };
    let tz = crate::display_tz(&pool);
    match run_pass(&pool, demo, crate::now_ms(), &tz, notifier.0.as_ref()).await {
        Ok(pass) if !pass.posted.is_empty() => {
            tracing::info!(announced = pass.posted.len(), "new invitations announced");
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(%e, "invite pass failed"),
    }
}

/// The clicked notification, landing: accept the invitation for the whole
/// series, then say how it went — as another notification, because the app
/// may well not be on screen (that is what notifications are for).
///
/// The write is the popover's own path, `respond_to_event_impl`, demo gate
/// and all. On success the UI is nudged the same two ways a popover answer
/// nudges it (`sync-finished` reloads the grid; the widget feed refreshes),
/// so an open window shows the ring change without waiting a sync interval.
pub(crate) async fn accept_from_notification(app: tauri::AppHandle, event_id: i64, start_ms: i64) {
    let outcome = {
        let state = app.state::<crate::AppState>();
        crate::events::respond_to_event_impl(&state, event_id, "accepted", "all", start_ms).await
    };

    let Some(notifier) = app.try_state::<crate::NotifierHandle>() else { return };
    match outcome {
        Ok(detail) => {
            let _ = notifier.0.post(&Notification {
                title: detail.title.filter(|t| !t.is_empty()).unwrap_or_else(|| crate::notify::NO_TITLE.into()),
                body: "Invitation accepted".into(),
                actions: Vec::new(),
                // A confirmation is read once; expiring is its job done.
                sticky: false,
            });
            let state = app.state::<crate::AppState>();
            let _ = app.emit("sync-finished", serde_json::json!({ "upserted": 1 }));
            crate::upcoming::refresh_soon(state.pool.clone(), state.demo);
        }
        Err(e) => {
            tracing::warn!(event_id, error = %e, "accepting from a notification failed");
            let _ = notifier.0.post(&Notification {
                title: "Could not accept the invitation".into(),
                // `respond_to_event_impl` errors are already user-facing text.
                body: format!("{e} Open OmaCal to answer."),
                actions: Vec::new(),
                // A failure asks the user to do something; it must not
                // evaporate before it is read.
                sticky: true,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notify::RecordingNotifier;
    use omacal_store::{upsert_event, Attendee, StoredEvent};

    const NOW: i64 = 1_786_352_400_000; // 2026-08-10T09:00:00Z
    const HOUR: i64 = 3_600_000;
    const SOFIA: &str = "Europe/Sofia";

    async fn seeded_pool() -> SqlitePool {
        let pool = omacal_store::connect_memory().await.unwrap();
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

    fn invite(gid: &str, start: i64) -> StoredEvent {
        StoredEvent {
            id: 0,
            calendar_id: 1,
            google_id: gid.into(),
            summary: Some("NVP sync meeting".into()),
            location: None,
            start_utc: start,
            end_utc: start + HOUR,
            start_tz: SOFIA.into(),
            end_tz: SOFIA.into(),
            is_all_day: false,
            recurrence: None,
            recurring_event_id: None,
            original_start_utc: None,
            status: "confirmed".into(),
            self_response: Some("needsAction".into()),
            conference_uri: None,
            color_hex: None,
            calendar_timezone: SOFIA.into(),
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

    fn candidate() -> InviteCandidate {
        InviteCandidate {
            event_id: 7,
            calendar_id: 1,
            summary: Some("NVP sync meeting".into()),
            start_utc: NOW + HOUR, // 10:00Z = 13:00 Sofia
            end_utc: NOW + 2 * HOUR,
            is_all_day: false,
            organizer_email: Some("ana@x.com".into()),
            calendar_timezone: SOFIA.into(),
            calendar_selected: true,
            provider: "google".into(),
            access_role: "owner".into(),
            attendees: invite("x", NOW).attendees,
            color_hex: Some("#5b8def".into()),
        }
    }

    /// Seeding first, so every pass test below means what it says: a pass on
    /// a fresh store announces nothing, however many invitations exist.
    async fn seeded_and_swallowed(pool: &SqlitePool) {
        let fake = RecordingNotifier::default();
        let pass = run_pass(pool, false, NOW, SOFIA, &fake).await.unwrap();
        assert!(pass.posted.is_empty(), "a first pass must announce nothing");
        assert!(fake.posted().is_empty());
    }

    #[tokio::test]
    async fn the_first_pass_swallows_the_backlog_silently() {
        let pool = seeded_pool().await;
        upsert_event(&pool, &invite("old-1", NOW + HOUR)).await.unwrap();
        upsert_event(&pool, &invite("old-2", NOW + 2 * HOUR)).await.unwrap();

        let fake = RecordingNotifier::default();
        let pass = run_pass(&pool, false, NOW, SOFIA, &fake).await.unwrap();
        assert_eq!(pass.seeded, 2);
        assert!(pass.posted.is_empty());
        assert!(fake.posted().is_empty(), "backlog is not news");
    }

    #[tokio::test]
    async fn an_invitation_arriving_after_the_seed_is_announced_once() {
        let pool = seeded_pool().await;
        seeded_and_swallowed(&pool).await;

        let id = upsert_event(&pool, &invite("new-1", NOW + HOUR)).await.unwrap();

        let fake = RecordingNotifier::default();
        let pass = run_pass(&pool, false, NOW, SOFIA, &fake).await.unwrap();
        assert_eq!(pass.posted, vec![id]);
        let posted = fake.posted();
        assert_eq!(posted.len(), 1);
        assert_eq!(posted[0].title, "Invitation: NVP sync meeting");

        // The next pass — and every one after — has nothing to say about it.
        let again = run_pass(&pool, false, NOW, SOFIA, &fake).await.unwrap();
        assert_eq!(again, InvitePass::default());
        assert_eq!(fake.posted().len(), 1);
    }

    #[tokio::test]
    async fn the_pass_applies_the_stored_clock_format_to_invitation_notifications() {
        let pool = seeded_pool().await;
        seeded_and_swallowed(&pool).await;
        sqlx::query("INSERT INTO settings (key, value) VALUES ('time_format', '12h')")
            .execute(&pool).await.unwrap();
        upsert_event(&pool, &invite("new-1", NOW + HOUR)).await.unwrap();

        let fake = RecordingNotifier::default();
        run_pass(&pool, false, NOW, SOFIA, &fake).await.unwrap();

        assert!(fake.posted()[0].body.starts_with("Mon, Aug 10 · 1:00 PM"));
    }

    /// `run_once`'s trade, inherited deliberately: a refusing transport logs
    /// and records rather than retrying the same announcement forever.
    #[tokio::test]
    async fn a_refused_post_still_records_the_invitation() {
        let pool = seeded_pool().await;
        seeded_and_swallowed(&pool).await;
        upsert_event(&pool, &invite("new-1", NOW + HOUR)).await.unwrap();

        let failing = RecordingNotifier::failing();
        let pass = run_pass(&pool, false, NOW, SOFIA, &failing).await.unwrap();
        assert_eq!(pass.posted.len(), 1, "recorded despite the refusal");

        let fake = RecordingNotifier::default();
        let again = run_pass(&pool, false, NOW, SOFIA, &fake).await.unwrap();
        assert_eq!(again, InvitePass::default(), "no retry loop");
    }

    #[tokio::test]
    async fn demo_mode_and_the_notifications_switch_both_silence_the_pass() {
        let pool = seeded_pool().await;
        upsert_event(&pool, &invite("new-1", NOW + HOUR)).await.unwrap();

        let fake = RecordingNotifier::default();
        let demo = run_pass(&pool, true, NOW, SOFIA, &fake).await.unwrap();
        assert_eq!(demo, InvitePass::default(), "demo posts nothing and seeds nothing");

        sqlx::query("INSERT INTO settings (key, value) VALUES ('notifications_enabled', '0')")
            .execute(&pool).await.unwrap();
        let off = run_pass(&pool, false, NOW, SOFIA, &fake).await.unwrap();
        assert_eq!(off, InvitePass::default());
        assert!(fake.posted().is_empty());
    }

    /// The reminder rule, with the invite twist: hidden is muted, but *not*
    /// consumed — showing the calendar again lets a still-unanswered
    /// invitation surface then.
    #[tokio::test]
    async fn a_hidden_calendars_invitation_waits_rather_than_vanishes() {
        let pool = seeded_pool().await;
        seeded_and_swallowed(&pool).await;
        let id = upsert_event(&pool, &invite("new-1", NOW + HOUR)).await.unwrap();
        sqlx::query("UPDATE calendars SET selected = 0 WHERE id = 1")
            .execute(&pool).await.unwrap();

        let fake = RecordingNotifier::default();
        let hidden = run_pass(&pool, false, NOW, SOFIA, &fake).await.unwrap();
        assert_eq!(hidden, InvitePass::default());

        sqlx::query("UPDATE calendars SET selected = 1 WHERE id = 1")
            .execute(&pool).await.unwrap();
        let shown = run_pass(&pool, false, NOW, SOFIA, &fake).await.unwrap();
        assert_eq!(shown.posted, vec![id], "still unanswered, so still news");
    }

    // --- the tray's rows -------------------------------------------------

    /// The tray exists for the invitation whose toast was missed — so the
    /// ledger that silences the announcer must not silence it.
    #[tokio::test]
    async fn the_tray_lists_an_already_announced_invitation_with_buttons() {
        let pool = seeded_pool().await;
        seeded_and_swallowed(&pool).await;
        let id = upsert_event(&pool, &invite("new-1", NOW + HOUR)).await.unwrap();
        run_pass(&pool, false, NOW, SOFIA, &RecordingNotifier::default()).await.unwrap();

        let rows = pending_invites_impl(&pool, false, NOW).await.unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.id, id);
        assert_eq!(r.title.as_deref(), Some("NVP sync meeting"));
        assert!(r.can_respond, "a writable Google invite carries the buttons");
        assert_eq!(r.start_date, None, "timed events carry no dates");
    }

    /// The gates the popover applies, applied here too: demo mode and
    /// providers without an RSVP write list without buttons.
    #[test]
    fn rows_without_a_working_rsvp_carry_no_buttons() {
        assert!(to_pending(&candidate(), false).can_respond);
        assert!(!to_pending(&candidate(), true).can_respond, "demo answers nothing");

        let mut caldav = candidate();
        caldav.provider = "caldav".into();
        assert!(!to_pending(&caldav, false).can_respond);
    }

    /// An all-day invite carries its calendar-zone days, worked out here —
    /// the browser must never re-derive them from the instant
    /// (`EventDetail::start_date`'s trap).
    #[test]
    fn an_all_day_invite_carries_its_days_not_just_instants() {
        let mut c = candidate();
        c.is_all_day = true;
        // Midnight Aug 11 Sofia (2026-08-10T21:00Z) through the 12th,
        // exclusive: a one-day event on the 11th.
        c.start_utc = 1_786_395_600_000;
        c.end_utc = c.start_utc + 24 * HOUR;
        let r = to_pending(&c, false);
        assert_eq!(r.start_date.as_deref(), Some("2026-08-11"));
        assert_eq!(r.end_date.as_deref(), Some("2026-08-11"), "inclusive last day");
    }

    // --- what the announcement says -------------------------------------

    #[test]
    fn a_timed_invitation_reads_in_the_display_zone_with_its_day() {
        let n = invite_notification(&candidate(), SOFIA);
        assert_eq!(n.title, "Invitation: NVP sync meeting");
        // 2026-08-10T10:00Z is Monday 13:00 in Sofia.
        assert!(n.body.starts_with("Mon, Aug 10 · 13:00 · from ana@x.com"), "{}", n.body);
    }

    #[test]
    fn a_timed_invitation_uses_the_selected_twelve_hour_clock() {
        let n = invite_notification_with_format(
            &candidate(),
            SOFIA,
            crate::settings::TimeFormat::H12,
        );
        assert!(n.body.starts_with("Mon, Aug 10 · 1:00 PM · from ana@x.com"), "{}", n.body);
    }

    #[test]
    fn an_all_day_invitation_reads_its_date_in_the_calendars_zone() {
        let mut c = candidate();
        c.is_all_day = true;
        // Midnight Aug 11 in Sofia, stored as 2026-08-10T21:00Z.
        c.start_utc = 1_786_395_600_000;
        // Read in a display zone west of the calendar, where that instant is
        // still Aug 10 — the date must come from the calendar's zone anyway.
        let n = invite_notification(&c, "UTC");
        assert!(n.body.starts_with("Tue, Aug 11 · All day"), "{}", n.body);
    }

    #[test]
    fn an_untitled_invitation_still_has_a_title() {
        let mut c = candidate();
        c.summary = None;
        assert_eq!(invite_notification(&c, SOFIA).title, "Invitation: (no title)");
    }

    /// The lesson of the first live click test: the toast expired into
    /// history — click and all — while the user read another window. An
    /// announcement that waits for an answer must wait to be answered,
    /// clickable or not.
    #[test]
    fn an_invitation_announcement_stays_until_dealt_with() {
        assert!(invite_notification(&candidate(), SOFIA).sticky);

        let mut caldav = candidate();
        caldav.provider = "caldav".into();
        assert!(invite_notification(&caldav, SOFIA).sticky, "no click, still worth seeing");
    }

    /// The click is offered — and instructed — only where it can act.
    #[test]
    fn the_accepting_click_exists_only_for_an_answerable_google_event() {
        let n = invite_notification(&candidate(), SOFIA);
        assert_eq!(
            n.actions,
            vec![Action::AcceptInvite { event_id: 7, start_ms: NOW + HOUR }]
        );
        assert!(
            n.body.ends_with("Click to accept · right-click to dismiss"),
            "{}", n.body
        );

        let mut caldav = candidate();
        caldav.provider = "caldav".into();
        let n = invite_notification(&caldav, SOFIA);
        assert!(n.actions.is_empty(), "no RSVP write exists for CalDAV");
        assert!(
            !n.body.contains("Click to accept"),
            "must not instruct a click that does nothing"
        );
        // Sticky with no click of its own still spells its way out.
        assert!(n.body.ends_with("Right-click to dismiss"), "{}", n.body);

        let mut reader = candidate();
        reader.access_role = "reader".into();
        assert!(invite_notification(&reader, SOFIA).actions.is_empty());

        let mut not_a_guest = candidate();
        not_a_guest.attendees.clear();
        assert!(invite_notification(&not_a_guest, SOFIA).actions.is_empty());
    }
}
