//! Syncing one CalDAV collection into the same store the Google path fills.
//!
//! The shape differs from Google's in exactly two ways, and everything else
//! deliberately matches:
//!
//! - **Change detection is a ctag, not a sync token.** One cheap PROPFIND
//!   answers "did anything change?"; only a changed (or unknown) ctag pays
//!   for a windowed refetch. The ctag rides in `sync_state.sync_token` —
//!   same row, same cursor semantics, no second table.
//! - **Deletions are inferred, not delivered.** CalDAV has no tombstones, so
//!   after a refetch the rows this collection previously held and the server
//!   no longer returned are deleted — bounded to rows the window's query
//!   would have returned, so an old finished series outside the window is
//!   left alone rather than wrongly reaped (the same staleness the Google
//!   window has).
//!
//! Tasks (VTODO) sync in the same pass under the same ctag: one collection,
//! one answer to "anything changed?".

use omacal_caldav::{CalDavClient, CalDavError, CalEvent, DiscoveredCalendar, IcsTime};
use omacal_store::StoredEvent;
use sqlx::SqlitePool;

use crate::SyncOutcome;

const DAY_MS: i64 = 86_400_000;

/// Resolves an event's end: `DTEND`, else `DURATION`, else the RFC defaults
/// (one day for all-day, zero-length for timed).
fn resolve_end(
    ev: &CalEvent,
    start_ms: i64,
    all_day: bool,
    cal_tz: &str,
) -> (i64, String) {
    if let Some(end) = &ev.end {
        if let Some((ms, tz, _)) = omacal_caldav::resolve(end, cal_tz) {
            return (ms, tz);
        }
    }
    if let Some(d) = ev.duration_ms {
        return (start_ms + d.max(0), cal_tz.to_string());
    }
    if all_day {
        (start_ms + DAY_MS, cal_tz.to_string())
    } else {
        (start_ms, cal_tz.to_string())
    }
}

/// Converts one parsed VEVENT into a storable row.
///
/// The synthetic id for an exception is `uid#<original-start-ms>` — CalDAV
/// keeps a whole series in one resource under one UID, and the store needs
/// each occurrence-override to be its own row with its own identity, exactly
/// as Google's instance ids provide.
///
/// **No VALARM means "follow the defaults"** (`use_default: true` over an
/// empty calendar list): CalDAV calendars have no default-reminder list, so
/// this lands on the app's own fallback-reminders setting — which exists
/// precisely for calendars that arrive silent. A VALARM present is the
/// author's word and maps to overrides.
///
/// `self_email` is the account's own address, and it is what stands in for
/// Google's `self` flag: CalDAV has no such marker, so the ATTENDEE whose
/// mailbox matches it (case-insensitively — mailboxes are compared, not
/// spelled) is the user's row, `is_self` and `self_response` both. An
/// invitation delivered to an alias the account was not connected under
/// finds no match and stays unanswerable — the same guard the write side
/// applies, made visible at read time.
pub fn caldav_to_stored(
    ev: &CalEvent,
    calendar_id: i64,
    cal_tz: &str,
    resource_etag: Option<&str>,
    self_email: &str,
) -> Option<StoredEvent> {
    let (start_utc, start_tz, is_all_day) = omacal_caldav::resolve(&ev.start, cal_tz)?;
    let (end_utc, end_tz) = resolve_end(ev, start_utc, is_all_day, cal_tz);

    let (google_id, recurring_event_id, original_start_utc) = match &ev.recurrence_id {
        Some(rid) => {
            let (orig_ms, _, _) = omacal_caldav::resolve(rid, cal_tz)?;
            (format!("{}#{}", ev.uid, orig_ms), Some(ev.uid.clone()), Some(orig_ms))
        }
        None => (ev.uid.clone(), None, None),
    };

    let reminders = if ev.alarms.is_empty() {
        omacal_store::Reminders { use_default: true, overrides: Vec::new() }
    } else {
        omacal_store::Reminders {
            use_default: false,
            overrides: ev
                .alarms
                .iter()
                .map(|(method, minutes)| omacal_store::Reminder {
                    method: method.clone(),
                    minutes: *minutes,
                })
                .collect(),
        }
    };

    Some(StoredEvent {
        id: 0,
        calendar_id,
        google_id,
        summary: ev.summary.clone(),
        location: ev.location.clone(),
        start_utc,
        end_utc: end_utc.max(start_utc),
        start_tz,
        end_tz,
        is_all_day,
        recurrence: if ev.recurrence.is_empty() { None } else { Some(ev.recurrence.join("\n")) },
        recurring_event_id,
        original_start_utc,
        status: ev.status.clone(),
        // Derived from the self attendee the same way `merge_patched` does
        // for Google — one derivation rule, whoever the provider is.
        self_response: ev
            .attendees
            .iter()
            .find(|a| a.email.eq_ignore_ascii_case(self_email))
            .map(|a| a.response_status.clone()),
        conference_uri: ev.conference_uri.clone(),
        color_hex: None,
        calendar_timezone: cal_tz.to_string(),
        description: ev.description.clone(),
        etag: resource_etag.map(str::to_string),
        sequence: ev.sequence,
        organizer_email: ev.organizer_email.clone(),
        guests_can_modify: false,
        attendees: ev
            .attendees
            .iter()
            .map(|a| omacal_store::Attendee {
                email: a.email.clone(),
                display_name: a.display_name.clone(),
                response_status: a.response_status.clone(),
                optional: a.optional,
                is_self: a.email.eq_ignore_ascii_case(self_email),
                comment: None,
                additional_guests: 0,
            })
            .collect(),
        reminders,
        calendar_default_reminders: Vec::new(),
    })
}

/// Converts one parsed VTODO into a storable task row.
pub fn caldav_todo_to_stored(
    todo: &omacal_caldav::CalTodo,
    calendar_id: i64,
    cal_tz: &str,
    href: &str,
    etag: Option<&str>,
    raw_ics: &str,
    now_ms: i64,
) -> omacal_store::StoredTask {
    let due = todo.due.as_ref().and_then(|d| omacal_caldav::resolve(d, cal_tz));
    let completed_utc = todo
        .completed
        .as_ref()
        .and_then(|c| omacal_caldav::resolve(c, cal_tz))
        .map(|(ms, _, _)| ms);
    let due_all_day = matches!(todo.due, Some(IcsTime::Date(_)));
    omacal_store::StoredTask {
        id: 0,
        calendar_id,
        uid: todo.uid.clone(),
        etag: etag.map(str::to_string),
        caldav_href: Some(href.to_string()),
        summary: todo.summary.clone(),
        description: todo.description.clone(),
        due_utc: due.as_ref().map(|(ms, _, _)| *ms),
        due_tz: due.as_ref().map(|(_, tz, _)| tz.clone()),
        due_all_day,
        status: todo.status.clone(),
        completed_utc,
        priority: todo.priority,
        updated_at: now_ms,
        raw_ics: Some(raw_ics.to_string()),
    }
}

/// Syncs one collection: events when it holds them, tasks when it holds
/// them, both behind one ctag probe. `collection_url` is the calendar row's
/// `google_id` — the column holds "the provider's identifier", and for
/// CalDAV that is the collection URL.
#[allow(clippy::too_many_arguments)]
pub async fn sync_caldav_calendar(
    pool: &SqlitePool,
    client: &CalDavClient,
    calendar_id: i64,
    collection_url: &str,
    supports_events: bool,
    supports_tasks: bool,
    window_start_ms: i64,
    window_end_ms: i64,
    now_ms: i64,
) -> Result<SyncOutcome, CalDavError> {
    let cal_tz: String = sqlx::query_scalar("SELECT timezone FROM calendars WHERE id = ?1")
        .bind(calendar_id)
        .fetch_one(pool)
        .await
        .map_err(anyhow::Error::from)?;

    // The account's own address, which is what identifies the user's row on
    // an ATTENDEE list — see `caldav_to_stored`'s own comment.
    let self_email: String = sqlx::query_scalar(
        "SELECT a.email FROM accounts a JOIN calendars c ON c.account_id = a.id WHERE c.id = ?1",
    )
    .bind(calendar_id)
    .fetch_one(pool)
    .await
    .map_err(anyhow::Error::from)?;

    let stored_ctag: Option<String> =
        sqlx::query_scalar("SELECT sync_token FROM sync_state WHERE calendar_id = ?1")
            .bind(calendar_id)
            .fetch_optional(pool)
            .await
            .map_err(anyhow::Error::from)?
            .flatten();

    // An unchanged ctag means nothing was edited — but it says nothing about
    // what the window has newly reached, so the day-advance check gets a say
    // before the short-circuit. See `crate::window_advanced_a_day`.
    let ctag = client.ctag(collection_url).await?;
    // `supports_events` gates the check, because the window write below is
    // inside that branch: a tasks-only collection never records one, so an
    // ungated check would read "no window" for ever and refetch its tasks on
    // every tick. Tasks have no window to advance past in any case.
    if ctag.is_some()
        && ctag == stored_ctag
        && !(supports_events
            && crate::window_advanced_a_day(pool, calendar_id, window_end_ms).await)
    {
        return Ok(SyncOutcome::default());
    }

    let mut outcome = SyncOutcome::default();

    if supports_events {
        let resources = client
            .events_in_window(collection_url, window_start_ms, window_end_ms)
            .await?;
        let mut seen: Vec<String> = Vec::new();
        let mut rows: Vec<(StoredEvent, Option<(String, String)>)> = Vec::new();
        for res in &resources {
            let Some(root) = omacal_caldav::parse(&res.ics) else {
                tracing::warn!(url = %res.url, "unparseable ICS resource; skipping");
                continue;
            };
            for (i, ev) in omacal_caldav::events_in(&root).into_iter().enumerate() {
                let Some(stored) =
                    caldav_to_stored(&ev, calendar_id, &cal_tz, res.etag.as_deref(), &self_email)
                else {
                    tracing::warn!(url = %res.url, "VEVENT with unusable times; skipping");
                    continue;
                };
                seen.push(stored.google_id.clone());
                // The master (first, by events_in's ordering) carries the
                // resource pointer and raw bytes for the write path.
                let src = (i == 0).then(|| (res.url.clone(), res.ics.clone()));
                rows.push((stored, src));
            }
        }

        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(anyhow::Error::from)?;
        let enabled: Option<i64> =
            sqlx::query_scalar("SELECT sync_enabled FROM calendars WHERE id = ?1")
                .bind(calendar_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(anyhow::Error::from)?;
        if enabled != Some(1) {
            tx.rollback().await.map_err(anyhow::Error::from)?;
            return Ok(SyncOutcome::default());
        }

        for (row, src) in &rows {
            let id = omacal_store::upsert_event(&mut *tx, row).await?;
            if let Some((href, ics)) = src {
                sqlx::query("UPDATE events SET caldav_href = ?2, raw_ics = ?3 WHERE id = ?1")
                    .bind(id)
                    .bind(href)
                    .bind(ics)
                    .execute(&mut *tx)
                    .await
                    .map_err(anyhow::Error::from)?;
            }
            outcome.upserted += 1;
        }

        // Inferred deletions, bounded to what the query would have returned:
        // anything alive in (or recurring into) the window that the server
        // did not mention no longer exists.
        //
        // **Bounded at both ends, unlike the Google sweep** — see
        // `reap::Reach` for why each provider reaches exactly as far as its
        // own fetch asked, and no further.
        let deleted = crate::reap::delete_unnamed(
            &mut tx,
            calendar_id,
            &seen,
            crate::reap::Reach::Overlapping { start_ms: window_start_ms, end_ms: window_end_ms },
        )
        .await
        .map_err(anyhow::Error::from)?;
        outcome.deleted += deleted as usize;

        sqlx::query(
            "INSERT INTO sync_state (calendar_id, sync_token, window_start, window_end)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (calendar_id) DO UPDATE SET
                 sync_token = excluded.sync_token,
                 window_start = excluded.window_start,
                 window_end = excluded.window_end",
        )
        .bind(calendar_id)
        .bind(&ctag)
        .bind(window_start_ms)
        .bind(window_end_ms)
        .execute(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?;
        tx.commit().await.map_err(anyhow::Error::from)?;
    }

    if supports_tasks {
        let resources = client.todos(collection_url).await?;
        let mut keep: Vec<String> = Vec::new();
        let mut skipped = 0usize;
        for res in &resources {
            // Skipped, and said so — the event branch's rule. Silently, a task
            // missing from the pane could not be told apart from one the
            // server never sent (#145).
            let Some(root) = omacal_caldav::parse(&res.ics) else {
                tracing::warn!(url = %res.url, "unparseable task resource; skipping");
                skipped += 1;
                continue;
            };
            let todos = omacal_caldav::todos_in(&root);
            // `todos_in` drops a VTODO without a UID, since nothing could
            // address it again. Counted for the same reason.
            let without_uid = root.components("VTODO").count() - todos.len();
            if without_uid > 0 {
                tracing::warn!(url = %res.url, count = without_uid, "task without a UID; skipping");
                skipped += without_uid;
            }
            for todo in todos {
                let row = caldav_todo_to_stored(
                    &todo,
                    calendar_id,
                    &cal_tz,
                    &res.url,
                    res.etag.as_deref(),
                    &res.ics,
                    now_ms,
                );
                keep.push(row.uid.clone());
                omacal_store::upsert_task(pool, &row).await?;
                outcome.upserted += 1;
            }
        }
        let deleted = omacal_store::delete_tasks_not_in(pool, calendar_id, &keep).await? as usize;
        outcome.deleted += deleted;
        outcome.skipped += skipped;
        // One line per list actually fetched, which the ctag check above
        // limits to when the server says something changed. It is what tells
        // "the server sent nothing" from "OmaCal dropped it" when a task is
        // missing. Ids and counts only: the log is what a user pastes into an
        // issue, and a list's name or address is theirs.
        tracing::info!(
            calendar_id,
            resources = resources.len(),
            tasks = keep.len(),
            skipped,
            deleted,
            "task list fetched"
        );

        // A tasks-only collection still needs its ctag recorded — the event
        // branch above did it for mixed collections.
        if !supports_events {
            sqlx::query(
                "INSERT INTO sync_state (calendar_id, sync_token, window_start, window_end)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (calendar_id) DO UPDATE SET sync_token = excluded.sync_token,
                     window_start = excluded.window_start, window_end = excluded.window_end",
            )
            .bind(calendar_id)
            .bind(&ctag)
            .bind(window_start_ms)
            .bind(window_end_ms)
            .execute(pool)
            .await
            .map_err(anyhow::Error::from)?;
        }
    }

    Ok(outcome)
}

/// Records the collections a discovery listed for one account.
///
/// Like the Google upsert: the provider's fields update, the user's
/// (`selected`, `sync_enabled`, the label and colour overrides) are never in
/// the statement, so neither a reconnect nor a sync can undo a hide, a
/// removal or a rename. A collection new to the account gets the schema's
/// defaults: shown, synced.
///
/// `fallback_tz` is for a new collection that publishes no zone — an all-day
/// date has to resolve against something, and the zone the user is sitting
/// in is the only defensible guess. A known row that publishes none keeps the
/// zone it has: this runs on every sync now, and re-guessing would move a
/// traveller's all-day dates to wherever the laptop was last opened.
pub async fn upsert_collections(
    pool: &SqlitePool,
    account_id: i64,
    calendars: &[DiscoveredCalendar],
    fallback_tz: &str,
) -> anyhow::Result<()> {
    for cal in calendars {
        sqlx::query(
            "INSERT INTO calendars
                (account_id, google_id, summary, color_hex, timezone, access_role,
                 is_primary, supports_events, supports_tasks)
             VALUES (?1, ?2, ?3, ?4, COALESCE(?5, ?9), ?6, 0, ?7, ?8)
             ON CONFLICT (account_id, google_id) DO UPDATE SET
                summary = excluded.summary,
                color_hex = excluded.color_hex,
                timezone = COALESCE(?5, calendars.timezone),
                access_role = excluded.access_role,
                supports_events = excluded.supports_events,
                supports_tasks = excluded.supports_tasks",
        )
        .bind(account_id)
        .bind(&cal.url)
        .bind(&cal.display_name)
        .bind(&cal.color)
        .bind(cal.timezone.as_deref())
        .bind(if cal.read_only { "reader" } else { "owner" })
        .bind(cal.supports_events as i64)
        .bind(cal.supports_tasks as i64)
        .bind(fallback_tz)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// An error as the log may carry it: the status or the kind, never the
/// message, which for a failed request is reqwest's and names the URL — and a
/// Fastmail collection URL holds the account's address.
fn log_kind(e: &CalDavError) -> String {
    match e {
        CalDavError::Http(status) => status.as_u16().to_string(),
        CalDavError::Unauthorized => "unauthorized".into(),
        CalDavError::PreconditionFailed => "precondition failed".into(),
        CalDavError::Other(_) => "request failed".into(),
    }
}

/// What one [`refresh_collections`] changed, for the log.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CollectionsRefresh {
    /// Collections the listing named that the account had no row for.
    pub added: usize,
    /// Rows deleted because their collection is gone from the server.
    pub removed: usize,
}

/// Brings one account's calendar list up to date with the server: a
/// collection made on another device since the account was connected gets a
/// row, and one deleted there loses its row.
///
/// Discovery used to run once, when the account was connected, so a task
/// list made later on the phone never reached OmaCal at all, and a deleted
/// one stayed listed, its ctag probe answering 404 on every sync.
///
/// **Nothing is deleted on the listing's word alone.** A failed discovery
/// changes nothing. An empty listing changes nothing either: connecting
/// refuses an account with no calendars, so "none" is a broken answer rather
/// than a user who deleted everything. And a row the listing left out is
/// deleted only once its own collection answers 404 or 410 — a listing cut
/// short by a proxy, or a collection that still answers but no longer sits
/// in the home set, keeps its row and its events.
///
/// Errors only with `Unauthorized`, so the caller can mark the account for
/// reconnecting. Anything else is logged and the sync goes on with the rows
/// it has: a calendar list a few minutes stale is not worth failing the
/// sync of the calendars that are fine.
pub async fn refresh_collections(
    pool: &SqlitePool,
    account_id: i64,
    client: &CalDavClient,
    fallback_tz: &str,
) -> Result<CollectionsRefresh, CalDavError> {
    let listed = match client.discover().await {
        Ok(l) => l,
        Err(e) if e.needs_reauth() => return Err(e),
        Err(e) => {
            tracing::warn!(account_id, error = %log_kind(&e), "collection discovery failed; calendar list unchanged");
            return Ok(CollectionsRefresh::default());
        }
    };
    if listed.is_empty() {
        tracing::warn!(account_id, "discovery listed no collections; calendar list unchanged");
        return Ok(CollectionsRefresh::default());
    }

    let stored: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, google_id FROM calendars WHERE account_id = ?1 ORDER BY id")
            .bind(account_id)
            .fetch_all(pool)
            .await
            .map_err(anyhow::Error::from)?;

    let mut out = CollectionsRefresh {
        added: listed.iter().filter(|c| !stored.iter().any(|(_, url)| *url == c.url)).count(),
        ..Default::default()
    };
    upsert_collections(pool, account_id, &listed, fallback_tz).await?;

    for (calendar_id, url) in stored {
        if listed.iter().any(|c| c.url == url) {
            continue;
        }
        match client.ctag(&url).await {
            Err(CalDavError::Http(status))
                if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE =>
            {
                omacal_store::delete_calendar(pool, calendar_id).await?;
                out.removed += 1;
            }
            // Kept, and on purpose not raised: the listing just succeeded
            // with these credentials, so a refusal on one collection is not
            // a dead password.
            Ok(_) => {
                tracing::debug!(calendar_id, "collection not listed but still answers; kept");
            }
            Err(e) => {
                tracing::warn!(calendar_id, error = %log_kind(&e), "collection not listed and its probe failed; kept");
            }
        }
    }

    if out != CollectionsRefresh::default() {
        // Ids and counts only, like the task-list line above.
        tracing::info!(account_id, added = out.added, removed = out.removed, "calendar list refreshed");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(uid: &str) -> CalEvent {
        CalEvent {
            uid: uid.into(),
            summary: Some("Board sync".into()),
            description: None,
            location: Some("HQ".into()),
            status: "confirmed".into(),
            start: IcsTime::Zoned {
                dt: jiff::civil::date(2026, 8, 17).at(9, 30, 0, 0),
                tzid: "Europe/Sofia".into(),
            },
            end: None,
            duration_ms: Some(45 * 60_000),
            recurrence: vec!["RRULE:FREQ=WEEKLY".into()],
            recurrence_id: None,
            sequence: 2,
            alarms: vec![("popup".into(), 10)],
            organizer_email: Some("boss@x".into()),
            conference_uri: None,
            last_modified_ms: None,
            attendees: Vec::new(),
        }
    }

    /// The self mark, which is what `can_respond` gates the RSVP strip on:
    /// the account's own mailbox — however either side spells its case — is
    /// `is_self`, and it names `self_response` too. The empty address every
    /// other test passes marks nobody, so a fixture cannot conjure an
    /// answerable invitation by accident.
    #[test]
    fn the_accounts_own_attendee_is_marked_self_and_names_the_response() {
        let mut ev = sample("u1");
        ev.attendees = vec![
            omacal_caldav::CalAttendee {
                email: "P@X.COM".into(),
                display_name: None,
                response_status: "tentative".into(),
                optional: false,
            },
            omacal_caldav::CalAttendee {
                email: "ana@x.com".into(),
                display_name: None,
                response_status: "accepted".into(),
                optional: false,
            },
        ];

        let s = caldav_to_stored(&ev, 7, "Europe/Sofia", None, "p@x.com").unwrap();
        let mine = s.attendees.iter().find(|a| a.email == "P@X.COM").unwrap();
        assert!(mine.is_self, "mailboxes compare case-insensitively");
        assert!(!s.attendees.iter().find(|a| a.email == "ana@x.com").unwrap().is_self);
        assert_eq!(s.self_response.as_deref(), Some("tentative"));

        let nobody = caldav_to_stored(&ev, 7, "Europe/Sofia", None, "").unwrap();
        assert!(nobody.attendees.iter().all(|a| !a.is_self));
        assert!(nobody.self_response.is_none());
    }

    #[test]
    fn a_master_maps_with_duration_and_verbatim_recurrence() {
        let s = caldav_to_stored(&sample("u1"), 7, "Europe/Sofia", Some("\"e\""), "").unwrap();
        assert_eq!(s.google_id, "u1");
        assert_eq!(s.calendar_id, 7);
        assert_eq!(s.end_utc - s.start_utc, 45 * 60_000, "DURATION resolved");
        assert_eq!(s.recurrence.as_deref(), Some("RRULE:FREQ=WEEKLY"));
        assert_eq!(s.etag.as_deref(), Some("\"e\""));
        assert!(!s.reminders.use_default, "a VALARM is the author's word");
        assert_eq!(s.reminders.overrides[0].minutes, 10);
    }

    #[test]
    fn an_exception_gets_a_synthetic_identity() {
        let mut ev = sample("u1");
        ev.recurrence = Vec::new();
        ev.recurrence_id = Some(IcsTime::Zoned {
            dt: jiff::civil::date(2026, 8, 24).at(9, 30, 0, 0),
            tzid: "Europe/Sofia".into(),
        });
        let s = caldav_to_stored(&ev, 7, "Europe/Sofia", None, "").unwrap();
        let orig = s.original_start_utc.unwrap();
        assert_eq!(s.google_id, format!("u1#{orig}"));
        assert_eq!(s.recurring_event_id.as_deref(), Some("u1"));
    }

    #[test]
    fn no_alarms_means_follow_the_fallback() {
        let mut ev = sample("u2");
        ev.alarms = Vec::new();
        let s = caldav_to_stored(&ev, 1, "UTC", None, "").unwrap();
        assert!(s.reminders.use_default);
        assert!(s.reminders.overrides.is_empty());
    }

    #[test]
    fn an_all_day_event_defaults_to_one_day() {
        let mut ev = sample("u3");
        ev.start = IcsTime::Date(jiff::civil::date(2026, 8, 20));
        ev.end = None;
        ev.duration_ms = None;
        let s = caldav_to_stored(&ev, 1, "Europe/Sofia", None, "").unwrap();
        assert!(s.is_all_day);
        assert_eq!(s.end_utc - s.start_utc, DAY_MS);
    }

    /// #145: what a task list's server sends and OmaCal cannot store is
    /// counted, not dropped without a word. One good VTODO, one resource that
    /// is not iCalendar, one VTODO with no UID: one stored, two skipped.
    #[tokio::test]
    async fn a_task_resource_that_cannot_be_read_is_counted_not_silently_dropped() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let ok = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t-1\r\nSUMMARY:Call the bank\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let no_uid = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nSUMMARY:Nobody can address me\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let response = |href: &str, data: &str| {
            format!(
                "<d:response><d:href>{href}</d:href><d:propstat><d:prop><d:getetag>\"1\"</d:getetag>\
                 <c:calendar-data>{data}</c:calendar-data></d:prop>\
                 <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
            )
        };
        let multistatus = |body: String| {
            format!(r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/">{body}</d:multistatus>"#)
        };
        Mock::given(method("PROPFIND"))
            .respond_with(ResponseTemplate::new(207).set_body_string(multistatus(
                "<d:response><d:href>/tasks/</d:href><d:propstat><d:prop><cs:getctag>c1</cs:getctag></d:prop>\
                 <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>".into(),
            )))
            .mount(&server)
            .await;
        Mock::given(method("REPORT"))
            .respond_with(ResponseTemplate::new(207).set_body_string(multistatus(
                response("/tasks/a.ics", ok)
                    + &response("/tasks/b.ics", "this is not a calendar")
                    + &response("/tasks/c.ics", no_uid),
            )))
            .mount(&server)
            .await;

        let pool = omacal_store::connect_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (google_sub, email, created_at) VALUES ('s','e@x',0)")
            .execute(&pool).await.unwrap();
        let list = format!("{}/tasks/", server.uri());
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role,
                                    supports_events, supports_tasks)
             VALUES (1, ?1, 'Chores', 'UTC', 'owner', 0, 1)",
        )
        .bind(&list)
        .execute(&pool).await.unwrap();

        let client = omacal_caldav::CalDavClient::new(&server.uri(), "u", "p").unwrap();
        let out = sync_caldav_calendar(&pool, &client, 1, &list, false, true, 0, 0, 0).await.unwrap();

        assert_eq!(out.upserted, 1, "the readable task is stored");
        assert_eq!(out.skipped, 2, "the unreadable resource and the UID-less task are counted");
        let rows = omacal_store::tasks_for_ui(&pool, 0).await.unwrap();
        assert_eq!(rows.iter().map(|r| r.task.uid.as_str()).collect::<Vec<_>>(), vec!["t-1"]);
    }

    /// The collection-refresh tests' server: the discovery walk of
    /// `omacal-caldav`'s own test, with the home's listing up to each test.
    mod refresh {
        use super::super::*;
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        const PRINCIPAL: &str = r#"<?xml version="1.0"?>
          <d:multistatus xmlns:d="DAV:"><d:response><d:href>/</d:href><d:propstat><d:prop>
          <d:current-user-principal><d:href>/principals/p/</d:href></d:current-user-principal>
          </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;

        const HOME: &str = r#"<?xml version="1.0"?>
          <d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response>
          <d:href>/principals/p/</d:href><d:propstat><d:prop>
          <c:calendar-home-set><d:href>/cals/p/</d:href></c:calendar-home-set>
          </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;

        /// One `<d:response>` for a calendar collection in the home.
        fn collection(name: &str, display: &str, component: &str) -> String {
            format!(
                r#"<d:response><d:href>/cals/p/{name}/</d:href><d:propstat><d:prop>
                <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>
                <d:displayname>{display}</d:displayname>
                <c:supported-calendar-component-set><c:comp name="{component}"/></c:supported-calendar-component-set>
                <d:current-user-privilege-set><d:privilege><d:write/></d:privilege></d:current-user-privilege-set>
                </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"#
            )
        }

        /// The home's Depth-1 listing: the home itself (not a calendar), then
        /// `entries`.
        fn listing(entries: &[String]) -> String {
            format!(
                r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"
                     xmlns:cs="http://calendarserver.org/ns/" xmlns:ic="http://apple.com/ns/ical/">
                <d:response><d:href>/cals/p/</d:href><d:propstat><d:prop>
                <d:resourcetype><d:collection/></d:resourcetype></d:prop>
                <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>{}</d:multistatus>"#,
                entries.concat()
            )
        }

        /// A server whose principal and home answer, and whose home listing
        /// answers `home`.
        async fn server(home: ResponseTemplate) -> MockServer {
            let server = MockServer::start().await;
            Mock::given(method("PROPFIND")).and(path("/"))
                .respond_with(ResponseTemplate::new(207).set_body_raw(PRINCIPAL, "application/xml"))
                .mount(&server).await;
            Mock::given(method("PROPFIND")).and(path("/principals/p/"))
                .respond_with(ResponseTemplate::new(207).set_body_raw(HOME, "application/xml"))
                .mount(&server).await;
            Mock::given(method("PROPFIND")).and(path("/cals/p/")).and(header("Depth", "1"))
                .respond_with(home)
                .mount(&server).await;
            server
        }

        /// A probe of one collection, as `ctag` asks it.
        async fn probe(server: &MockServer, name: &str, answer: ResponseTemplate) {
            Mock::given(method("PROPFIND")).and(path(format!("/cals/p/{name}/"))).and(header("Depth", "0"))
                .respond_with(answer)
                .mount(server).await;
        }

        /// An account holding `work` with every user choice made: hidden,
        /// removed from syncing, its label and colour overridden, and the zone
        /// it was given when connected. Returns the pool and the row's id.
        async fn connected(server: &MockServer) -> (SqlitePool, i64) {
            let pool = omacal_store::connect_memory().await.unwrap();
            sqlx::query(
                "INSERT INTO accounts (google_sub, email, created_at, provider)
                 VALUES ('caldav:e@x', 'e@x', 0, 'caldav')",
            )
            .execute(&pool).await.unwrap();
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role,
                                        selected, sync_enabled, label_override, color_override)
                 VALUES (1, ?1, 'Work', 'America/Chicago', 'owner', 0, 0, 'My work', '#123456')
                 RETURNING id",
            )
            .bind(format!("{}/cals/p/work/", server.uri()))
            .fetch_one(&pool).await.unwrap();
            (pool, id)
        }

        async fn add_row(pool: &SqlitePool, server: &MockServer, name: &str) -> i64 {
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
                 VALUES (1, ?1, ?2, 'UTC', 'owner') RETURNING id",
            )
            .bind(format!("{}/cals/p/{name}/", server.uri()))
            .bind(name)
            .fetch_one(pool).await.unwrap();
            sqlx::query(
                "INSERT INTO events (calendar_id, google_id, start_utc, end_utc, start_tz, end_tz, updated_at)
                 VALUES (?1, 'ev', 0, 1, 'UTC', 'UTC', 0)",
            )
            .bind(id)
            .execute(pool).await.unwrap();
            id
        }

        async fn ids(pool: &SqlitePool) -> Vec<i64> {
            sqlx::query_scalar("SELECT id FROM calendars ORDER BY id").fetch_all(pool).await.unwrap()
        }

        /// A task list made on the phone after the account was connected is
        /// listed on the next refresh, as a tasks-only list, shown and synced;
        /// and the row that was already there keeps every choice the user made
        /// on it, and the zone it already had (the listing publishes none).
        #[tokio::test]
        async fn a_collection_made_elsewhere_is_added_and_the_known_one_keeps_its_choices() {
            let server = server(ResponseTemplate::new(207).set_body_string(listing(&[
                collection("work", "Work (renamed)", "VEVENT"),
                collection("bernard", "Bernard", "VTODO"),
            ])))
            .await;
            let (pool, work) = connected(&server).await;
            let client = CalDavClient::new(&server.uri(), "e@x", "pw").unwrap();

            let out = refresh_collections(&pool, 1, &client, "Europe/Sofia").await.unwrap();
            assert_eq!(out, CollectionsRefresh { added: 1, removed: 0 });

            type Row = (String, i64, i64, i64, i64, String);
            let bernard: Row = sqlx::query_as(
                "SELECT summary, supports_events, supports_tasks, selected, sync_enabled, timezone
                 FROM calendars WHERE google_id = ?1",
            )
            .bind(format!("{}/cals/p/bernard/", server.uri()))
            .fetch_one(&pool).await.unwrap();
            assert_eq!(bernard, ("Bernard".into(), 0, 1, 1, 1, "Europe/Sofia".into()));

            type Kept = (String, i64, i64, Option<String>, Option<String>, String);
            let kept: Kept = sqlx::query_as(
                "SELECT summary, selected, sync_enabled, label_override, color_override, timezone
                 FROM calendars WHERE id = ?1",
            )
            .bind(work)
            .fetch_one(&pool).await.unwrap();
            assert_eq!(kept, ("Work (renamed)".into(), 0, 0, Some("My work".into()), Some("#123456".into()),
                              "America/Chicago".into()),
                       "the server's name updates; hidden, removed, label, colour and zone stay");

            let again = refresh_collections(&pool, 1, &client, "Europe/Sofia").await.unwrap();
            assert_eq!(again, CollectionsRefresh::default(), "a second refresh finds nothing new");
            assert_eq!(ids(&pool).await.len(), 2);
        }

        /// A collection the listing leaves out goes only on its own 404 or
        /// 410. One that still answers, one whose probe fails, and one whose
        /// probe is refused all stay with their events — and the refusal does
        /// not fail the refresh, since the listing just succeeded.
        #[tokio::test]
        async fn an_unlisted_collection_goes_only_when_it_answers_not_found() {
            let server = server(ResponseTemplate::new(207).set_body_string(listing(&[
                collection("work", "Work", "VEVENT"),
            ])))
            .await;
            let (pool, work) = connected(&server).await;
            let deleted = add_row(&pool, &server, "deleted").await;
            let gone = add_row(&pool, &server, "gone").await;
            let alive = add_row(&pool, &server, "alive").await;
            let flaky = add_row(&pool, &server, "flaky").await;
            let refused = add_row(&pool, &server, "refused").await;
            probe(&server, "deleted", ResponseTemplate::new(404)).await;
            probe(&server, "gone", ResponseTemplate::new(410)).await;
            probe(&server, "alive", ResponseTemplate::new(207).set_body_string(
                r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/">
                <d:response><d:href>/cals/p/alive/</d:href><d:propstat><d:prop><cs:getctag>c</cs:getctag></d:prop>
                <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#,
            )).await;
            probe(&server, "flaky", ResponseTemplate::new(502)).await;
            probe(&server, "refused", ResponseTemplate::new(403)).await;
            let client = CalDavClient::new(&server.uri(), "e@x", "pw").unwrap();

            let out = refresh_collections(&pool, 1, &client, "UTC").await.unwrap();
            assert_eq!(out, CollectionsRefresh { added: 0, removed: 2 });
            assert_eq!(ids(&pool).await, vec![work, alive, flaky, refused]);
            assert!(!ids(&pool).await.contains(&deleted) && !ids(&pool).await.contains(&gone));
            let events: Vec<i64> = sqlx::query_scalar("SELECT calendar_id FROM events ORDER BY calendar_id")
                .fetch_all(&pool).await.unwrap();
            assert_eq!(events, vec![alive, flaky, refused], "the kept rows keep their events");
        }

        /// A listing that fails, or names nothing, changes nothing — not even
        /// for a collection that would answer 404 — and does not fail the
        /// sync. A refused password does, so the account can be marked.
        #[tokio::test]
        async fn a_failed_or_empty_listing_touches_nothing() {
            for home in [ResponseTemplate::new(502), ResponseTemplate::new(207).set_body_string(listing(&[]))] {
                let server = server(home).await;
                let (pool, work) = connected(&server).await;
                let deleted = add_row(&pool, &server, "deleted").await;
                probe(&server, "work", ResponseTemplate::new(404)).await;
                probe(&server, "deleted", ResponseTemplate::new(404)).await;
                let client = CalDavClient::new(&server.uri(), "e@x", "pw").unwrap();

                let out = refresh_collections(&pool, 1, &client, "UTC").await.unwrap();
                assert_eq!(out, CollectionsRefresh::default());
                assert_eq!(ids(&pool).await, vec![work, deleted]);
            }

            let server = server(ResponseTemplate::new(401)).await;
            let (pool, _) = connected(&server).await;
            let client = CalDavClient::new(&server.uri(), "e@x", "pw").unwrap();
            assert!(matches!(
                refresh_collections(&pool, 1, &client, "UTC").await,
                Err(CalDavError::Unauthorized)
            ));
        }
    }
}
