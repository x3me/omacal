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

use omacal_caldav::{CalDavClient, CalDavError, CalEvent, IcsTime};
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
        // **Bounded at both ends, unlike the Google sweep** in `lib.rs`, which
        // has no `start_utc <` term. The two are not a copy that drifted and
        // should not be merged into one helper: this runs on every successful
        // CalDAV sync, so it must not reach past what this REPORT asked for,
        // while the Google one runs only after a *full* resync, where the
        // fetch covered the window entire. The third copy that did duplicate
        // this one — the WebCal path — is gone: a feed arrives whole, so it
        // deletes by "not named in the file" and needs no window at all.
        let placeholders: Vec<String> =
            (0..seen.len()).map(|i| format!("?{}", i + 4)).collect();
        let sql = format!(
            "DELETE FROM events WHERE calendar_id = ?1
               AND start_utc < ?2 AND (end_utc > ?3 OR recurrence IS NOT NULL)
               {}",
            if seen.is_empty() {
                String::new()
            } else {
                format!("AND google_id NOT IN ({})", placeholders.join(", "))
            }
        );
        let mut q = sqlx::query(&sql).bind(calendar_id).bind(window_end_ms).bind(window_start_ms);
        for id in &seen {
            q = q.bind(id);
        }
        let deleted = q.execute(&mut *tx).await.map_err(anyhow::Error::from)?.rows_affected();
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
        for res in &resources {
            let Some(root) = omacal_caldav::parse(&res.ics) else {
                continue;
            };
            for todo in omacal_caldav::todos_in(&root) {
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
        outcome.deleted +=
            omacal_store::delete_tasks_not_in(pool, calendar_id, &keep).await? as usize;

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
}
