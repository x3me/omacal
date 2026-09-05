//! CalDAV event writes — the provider decision doc's phase 3.
//!
//! The shape every operation shares: **rewrite the resource, PUT it behind
//! its etag, then resync the collection through the read path.** The store is
//! never hand-edited after a write; the same `sync_caldav_calendar` that
//! passes the Radicale end-to-end reconciles it from what the server now
//! holds. One code path decides what rows exist, whichever direction the
//! change came from — and a write that raced another client either wins the
//! etag or surfaces `PreconditionFailed`, never both.
//!
//! Scope vocabulary maps onto ICS structure rather than onto Google calls:
//! - **all** (and any edit of a one-off): rewrite the master component in
//!   place, preserving properties this app does not model. A time change
//!   drops the series' exceptions — their `RECURRENCE-ID`s name occurrence
//!   starts that no longer exist.
//! - **this**: upsert an exception component under the same UID.
//! - **following**: `UNTIL` on the master's rule, plus a brand-new resource
//!   (new UID) carrying the rest. `COUNT` does not survive a split — it
//!   counts from a start that no longer exists — so the remainder's rule is
//!   carried without it.
//!
//! Guests are deliberately not written: CalDAV scheduling (iTIP) is its own
//! project, the form hides the editor on these calendars, and the rewrite
//! passes `ATTENDEE` lines through untouched — nothing a payload carries can
//! erase somebody's RSVP.
//!
//! [`respond`] is the one measured exception to that rule: it rewrites
//! exactly one thing on exactly one `ATTENDEE` line — the account's own
//! `PARTSTAT` — and only when the resource already carries that line.
//! Answering an invitation is the attendee's own field by definition, and
//! the iTIP REPLY to the organizer is the server's job (RFC 6638), not
//! omacal's.

use omacal_caldav::{CalDavError, EventWrite, WriteTime};
use omacal_store::StoredEvent;

use crate::write::{EventFields, When};
use crate::AppState;

pub(crate) async fn is_caldav_calendar(
    pool: &sqlx::SqlitePool,
    calendar_id: i64,
) -> anyhow::Result<bool> {
    let provider: String = sqlx::query_scalar(
        "SELECT a.provider FROM calendars c JOIN accounts a ON a.id = c.account_id
         WHERE c.id = ?1",
    )
    .bind(calendar_id)
    .fetch_one(pool)
    .await?;
    Ok(provider == "caldav")
}

/// The master row of whatever the user is looking at: the row itself, unless
/// it is an exception, in which case the series master it belongs to — the
/// component that owns the resource.
async fn master_of(state: &AppState, viewed: &StoredEvent) -> anyhow::Result<StoredEvent> {
    let Some(master_uid) = viewed.recurring_event_id.as_deref() else {
        return Ok(viewed.clone());
    };
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM events WHERE calendar_id = ?1 AND google_id = ?2",
    )
    .bind(viewed.calendar_id)
    .bind(master_uid)
    .fetch_optional(&state.pool)
    .await?;
    let (id,) = row.ok_or_else(|| anyhow::anyhow!("the series this occurrence belongs to is no longer here"))?;
    let (event, _, _) = omacal_store::event_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the series this occurrence belongs to is no longer here"))?;
    Ok(event)
}

/// The resource pointer a write needs. Deliberately not fields on
/// `StoredEvent` — only this path reads them, and the struct's dozens of
/// constructors stay untouched.
async fn resource_of(state: &AppState, master_id: i64) -> anyhow::Result<(String, String)> {
    let row: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT caldav_href, raw_ics FROM events WHERE id = ?1")
            .bind(master_id)
            .fetch_optional(&state.pool)
            .await?;
    let (href, raw) = row.ok_or_else(|| anyhow::anyhow!("that event is no longer here"))?;
    match (href, raw) {
        (Some(h), Some(r)) => Ok((h, r)),
        _ => anyhow::bail!("this event has not finished syncing; try again in a moment"),
    }
}

/// `When` + the authoring zone → the two serializable endpoints, plus the
/// new start instant (what decides whether a series edit moved its times).
fn endpoints(when: &When, tz: &str, cal_tz: &str) -> anyhow::Result<(WriteTime, WriteTime, i64)> {
    match when {
        When::Timed { start_ms, end_ms } => {
            let zone = jiff::tz::TimeZone::get(tz)
                .or_else(|_| jiff::tz::TimeZone::get(cal_tz))
                .map_err(|_| anyhow::anyhow!("unknown time zone {tz}"))?;
            let name = zone.iana_name().unwrap_or("UTC").to_string();
            let civil = |ms: i64| -> anyhow::Result<jiff::civil::DateTime> {
                Ok(jiff::Timestamp::from_millisecond(ms)?.to_zoned(zone.clone()).datetime())
            };
            Ok((
                WriteTime::Zoned { dt: civil(*start_ms)?, tzid: name.clone() },
                WriteTime::Zoned { dt: civil(*end_ms)?, tzid: name },
                *start_ms,
            ))
        }
        When::AllDay { start_date, end_date } => {
            let start: jiff::civil::Date = start_date.parse()?;
            let end: jiff::civil::Date = end_date.parse()?;
            let start_ms = start
                .to_datetime(jiff::civil::Time::midnight())
                .in_tz(cal_tz)?
                .timestamp()
                .as_millisecond();
            Ok((WriteTime::Date(start), WriteTime::Date(end), start_ms))
        }
    }
}

/// One occurrence's identity, rendered the way the resource's own DTSTART is
/// — a bare date for all-day series, the master's zone otherwise.
fn occurrence_time(master: &StoredEvent, occurrence_start_ms: i64) -> anyhow::Result<WriteTime> {
    let ts = jiff::Timestamp::from_millisecond(occurrence_start_ms)?;
    if master.is_all_day {
        Ok(WriteTime::Date(ts.in_tz(&master.calendar_timezone)?.date()))
    } else {
        let z = ts.in_tz(&master.start_tz)?;
        Ok(WriteTime::Zoned { dt: z.datetime(), tzid: master.start_tz.clone() })
    }
}

/// The VALARM list a write should carry: the form's answer when it touched
/// reminders, the master's existing alarms when it did not, nothing on a
/// create. `use_default: true` renders as no alarms at all — symmetric with
/// the read side, where no VALARM maps back to "follow the defaults".
fn alarms_for(
    input: &Option<crate::write::RemindersInput>,
    existing: Option<&StoredEvent>,
) -> Vec<(String, i64)> {
    match input {
        Some(r) if r.use_default => Vec::new(),
        Some(r) => r.overrides.iter().map(|o| (o.method.clone(), o.minutes)).collect(),
        None => existing
            .filter(|e| !e.reminders.use_default)
            .map(|e| {
                e.reminders.overrides.iter().map(|o| (o.method.clone(), o.minutes)).collect()
            })
            .unwrap_or_default(),
    }
}

/// The recurrence lines a rewrite should carry. `COUNT` is stripped when the
/// lines are inherited across a split (`strip_count`), never otherwise.
fn recurrence_for(
    input: &Option<Option<String>>,
    master: Option<&StoredEvent>,
    strip_count: bool,
) -> Vec<String> {
    let inherited = || -> Vec<String> {
        master
            .and_then(|m| m.recurrence.as_deref())
            .map(|r| r.lines().map(str::to_string).collect())
            .unwrap_or_default()
    };
    let mut lines = match input {
        Some(Some(rule)) => vec![rule.clone()],
        Some(None) => Vec::new(),
        None => inherited(),
    };
    if strip_count {
        lines = lines
            .into_iter()
            .map(|l| {
                if !l.to_ascii_uppercase().starts_with("RRULE") {
                    return l;
                }
                let Some(colon) = l.find(':') else { return l };
                let (head, params) = l.split_at(colon + 1);
                let kept: Vec<&str> = params
                    .split(';')
                    .filter(|p| !p.trim().to_ascii_uppercase().starts_with("COUNT="))
                    .collect();
                format!("{head}{}", kept.join(";"))
            })
            .collect();
    }
    lines
}

fn now_ts() -> jiff::Timestamp {
    jiff::Timestamp::from_millisecond(crate::now_ms()).unwrap_or(jiff::Timestamp::UNIX_EPOCH)
}

fn friendly(e: CalDavError) -> anyhow::Error {
    match e {
        CalDavError::PreconditionFailed => {
            anyhow::anyhow!("That event changed on the server since it was loaded — sync and try again")
        }
        other => anyhow::Error::from(other),
    }
}

/// Post-write reconciliation: one collection resynced through the read path,
/// and the bar's feed refreshed.
async fn resync(state: &AppState, calendar_id: i64) -> anyhow::Result<()> {
    let (client, collection_url) =
        crate::caldav_account::client_for_calendar(state, calendar_id).await?;
    let (supports_events, supports_tasks): (i64, i64) = sqlx::query_as(
        "SELECT supports_events, supports_tasks FROM calendars WHERE id = ?1",
    )
    .bind(calendar_id)
    .fetch_one(&state.pool)
    .await?;
    let now = crate::now_ms();
    let (ws, we) = crate::synced_window(now);
    omacal_sync::caldav::sync_caldav_calendar(
        &state.pool,
        &client,
        calendar_id,
        &collection_url,
        supports_events != 0,
        supports_tasks != 0,
        ws,
        we,
        now,
    )
    .await
    .map_err(friendly)?;
    crate::upcoming::refresh_soon(state.pool.clone(), state.demo);
    Ok(())
}

async fn row_id_by_uid(state: &AppState, calendar_id: i64, uid: &str) -> anyhow::Result<i64> {
    sqlx::query_scalar("SELECT id FROM events WHERE calendar_id = ?1 AND google_id = ?2")
        .bind(calendar_id)
        .bind(uid)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the write landed but the resync did not find it"))
}

/// Creates an event on a CalDAV calendar and answers with its detail.
pub(crate) async fn create(
    state: &AppState,
    calendar_id: i64,
    fields: EventFields,
) -> anyhow::Result<crate::events::EventDetail> {
    let (client, collection_url) =
        crate::caldav_account::client_for_calendar(state, calendar_id).await?;
    let cal_tz: String = sqlx::query_scalar("SELECT timezone FROM calendars WHERE id = ?1")
        .bind(calendar_id)
        .fetch_one(&state.pool)
        .await?;

    let (start, end, _) = endpoints(&fields.when, &fields.tz, &cal_tz)?;
    let uid = uuid::Uuid::new_v4().to_string();
    let ev = EventWrite {
        uid: uid.clone(),
        summary: fields.summary,
        location: fields.location,
        description: fields.description,
        start,
        end,
        recurrence: recurrence_for(&fields.recurrence, None, false),
        recurrence_id: None,
        alarms: alarms_for(&fields.reminders, None),
        sequence: 0,
    };
    let ics = omacal_caldav::new_event_ics(&ev, now_ts());
    let href = format!("{}/{uid}.ics", collection_url.trim_end_matches('/'));
    client.put(&href, &ics, None).await.map_err(friendly)?;

    resync(state, calendar_id).await?;
    let id = row_id_by_uid(state, calendar_id, &uid).await?;
    crate::events::event_detail_impl(state, id).await
}

/// Edits an event (or one occurrence, or the rest of a series).
pub(crate) async fn update(
    state: &AppState,
    id: i64,
    scope: &str,
    occurrence_start_ms: i64,
    fields: EventFields,
) -> anyhow::Result<crate::events::EventDetail> {
    let (viewed, _, cal_tz) = omacal_store::event_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("that event is no longer here"))?;
    let master = master_of(state, &viewed).await?;
    let (client, _) = crate::caldav_account::client_for_calendar(state, master.calendar_id).await?;
    let (href, raw) = resource_of(state, master.id).await?;
    let uid = master.google_id.clone();
    let is_series = master.recurrence.is_some();

    let (start, end, new_start_ms) = endpoints(&fields.when, &fields.tz, &cal_tz)?;
    let now = now_ts();

    let (new_raw, follow_up, detail_uid): (String, Option<(String, String)>, String) =
        if !is_series || scope == "all" {
            let moved = is_series && new_start_ms != master.start_utc;
            let ev = EventWrite {
                uid: uid.clone(),
                summary: fields.summary,
                location: fields.location,
                description: fields.description,
                start,
                end,
                recurrence: recurrence_for(&fields.recurrence, Some(&master), false),
                recurrence_id: None,
                alarms: alarms_for(&fields.reminders, Some(&master)),
                sequence: master.sequence + 1,
            };
            let out = omacal_caldav::rewrite_master(&raw, &uid, &ev, now, moved)
                .ok_or_else(|| anyhow::anyhow!("could not rewrite the event's resource"))?;
            (out, None, uid.clone())
        } else if scope == "following" {
            if occurrence_start_ms <= master.start_utc {
                // "This and following" from the first occurrence is the
                // whole series; recurse as an "all" edit.
                return Box::pin(update(state, id, "all", occurrence_start_ms, fields)).await;
            }
            let cut = occurrence_time(&master, occurrence_start_ms)?;
            let until = crate::write::until_value(
                occurrence_start_ms,
                master.is_all_day,
                &master.calendar_timezone,
            );
            let truncated = omacal_caldav::truncate_series(&raw, &uid, &until, &cut)
                .ok_or_else(|| anyhow::anyhow!("could not truncate the series"))?;

            let new_uid = uuid::Uuid::new_v4().to_string();
            let ev = EventWrite {
                uid: new_uid.clone(),
                summary: fields.summary,
                location: fields.location,
                description: fields.description,
                start,
                end,
                recurrence: recurrence_for(&fields.recurrence, Some(&master), true),
                recurrence_id: None,
                alarms: alarms_for(&fields.reminders, Some(&master)),
                sequence: 0,
            };
            let ics = omacal_caldav::new_event_ics(&ev, now);
            let collection = href.rsplit_once('/').map(|(c, _)| c).unwrap_or("");
            let new_href = format!("{collection}/{new_uid}.ics");
            (truncated, Some((new_href, ics)), new_uid)
        } else {
            // Scope "this": one occurrence becomes (or updates) an exception.
            let rid = occurrence_time(&master, occurrence_start_ms)?;
            let ev = EventWrite {
                uid: uid.clone(),
                summary: fields.summary,
                location: fields.location,
                description: fields.description,
                start,
                end,
                recurrence: Vec::new(),
                recurrence_id: Some(rid),
                alarms: alarms_for(&fields.reminders, Some(&viewed)),
                sequence: 0,
            };
            let out = omacal_caldav::upsert_exception(&raw, &ev, now)
                .ok_or_else(|| anyhow::anyhow!("could not write the occurrence's exception"))?;
            (out, None, format!("{uid}#{occurrence_start_ms}"))
        };

    client.put(&href, &new_raw, master.etag.as_deref()).await.map_err(friendly)?;
    if let Some((new_href, ics)) = follow_up {
        client.put(&new_href, &ics, None).await.map_err(friendly)?;
    }

    resync(state, master.calendar_id).await?;
    let row = row_id_by_uid(state, master.calendar_id, &detail_uid).await;
    // An exception the resync collapsed away (or a row the server renamed)
    // falls back to the master — the popover shows *something* true.
    let row = match row {
        Ok(r) => r,
        Err(_) => row_id_by_uid(state, master.calendar_id, &uid).await?,
    };
    crate::events::event_detail_impl(state, row).await
}

/// Google's response vocabulary rendered as `PARTSTAT` — two spellings of
/// one answer. `None` for anything else, refused before any bytes move: an
/// unknown word PUT into a resource would be a corrupt `PARTSTAT` on a
/// server that may or may not validate it.
fn partstat_of(response: &str) -> Option<&'static str> {
    match response {
        "accepted" => Some("ACCEPTED"),
        "declined" => Some("DECLINED"),
        "tentative" => Some("TENTATIVE"),
        _ => None,
    }
}

/// Answers an invitation on a CalDAV event: the account's own `ATTENDEE`
/// line's `PARTSTAT`, rewritten inside the resource and PUT back behind its
/// etag — see the module doc for why this is the one attendee write that
/// exists. Scope maps onto structure exactly as [`update`]'s does: "all"
/// answers every component of the uid, "this" answers one occurrence's
/// exception, materialising it from the master if the server holds none.
///
/// The occurrence's identity is `original_start_utc` when the viewed row is
/// an already-moved exception — its `RECURRENCE-ID` names the slot it left,
/// not the slot it sits in — and the clicked start otherwise.
pub(crate) async fn respond(
    state: &AppState,
    id: i64,
    response: &str,
    scope: &str,
    occurrence_start_ms: i64,
    self_email: &str,
) -> anyhow::Result<crate::events::EventDetail> {
    let partstat = partstat_of(response)
        .ok_or_else(|| anyhow::anyhow!("\"{response}\" is not an answer OmaCal knows"))?;
    let (viewed, _, _) = omacal_store::event_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("that event is no longer here"))?;
    let master = master_of(state, &viewed).await?;
    let (client, _) = crate::caldav_account::client_for_calendar(state, master.calendar_id).await?;
    let (href, raw) = resource_of(state, master.id).await?;
    let uid = master.google_id.clone();
    let is_series = master.recurrence.is_some();

    let (new_raw, detail_uid) = if !is_series || scope == "all" {
        (omacal_caldav::respond_all(&raw, &uid, self_email, partstat), uid.clone())
    } else {
        let rid_ms = viewed.original_start_utc.unwrap_or(occurrence_start_ms);
        let rid = occurrence_time(&master, rid_ms)?;
        // The viewed row's own span: for a materialised exception the end is
        // unused (its block is answered in place), and for a bare occurrence
        // the viewed row is the master, whose duration is the occurrence's.
        let end = occurrence_time(&master, rid_ms + (viewed.end_utc - viewed.start_utc))?;
        (
            omacal_caldav::respond_occurrence(
                &raw, &uid, self_email, partstat, &rid, &end, now_ts(),
            ),
            format!("{uid}#{rid_ms}"),
        )
    };
    let new_raw = new_raw.ok_or_else(|| {
        anyhow::anyhow!(
            "this event's copy on the server does not list you as a guest, \
             so there is no invitation here to answer"
        )
    })?;

    client.put(&href, &new_raw, master.etag.as_deref()).await.map_err(friendly)?;
    resync(state, master.calendar_id).await?;
    let row = match row_id_by_uid(state, master.calendar_id, &detail_uid).await {
        Ok(r) => r,
        Err(_) => row_id_by_uid(state, master.calendar_id, &uid).await?,
    };
    crate::events::event_detail_impl(state, row).await
}

/// Where one UID's resource lives inside a collection.
///
/// A collection URL may or may not carry its trailing slash — servers publish
/// both, and Radicale's discovery answers with one — so joining without
/// trimming produces `…/calendar//uid.ics`. Some servers accept that and store
/// the event at a path the next sync does not recognise, which is the worst
/// available outcome: a move that looks like it worked and loses the event.
fn resource_href(collection_url: &str, uid: &str) -> String {
    format!("{}/{uid}.ics", collection_url.trim_end_matches('/'))
}

/// Re-files an event onto another collection of the **same account**.
///
/// CalDAV has no move verb: a collection is a directory and an event is a file
/// in it, so this is a PUT into the destination followed by a DELETE from the
/// source. The whole `VCALENDAR` resource travels — master and every
/// materialised exception, which live in one file under one UID — so a series
/// arrives whole or not at all.
///
/// **PUT first, DELETE second, and never the other way round.** Both orders
/// can fail in the middle; only one of them can lose the event. A failed
/// DELETE leaves a copy in the old calendar, which is visible, fixable by
/// hand, and reported as [`crate::events::MOVED_NOT_REMOVED`]; a failed PUT
/// after a DELETE leaves nothing anywhere.
///
/// The etag on the DELETE is the row's, re-read here rather than carried in:
/// the field edit that precedes a move has already rewritten and resynced this
/// resource, so the version in hand at the start of the save is one write out
/// of date by the time this runs.
pub(crate) async fn move_to(
    state: &AppState,
    id: i64,
    target_calendar_id: i64,
) -> anyhow::Result<crate::events::EventDetail> {
    let (viewed, _, _) = omacal_store::event_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("that event is no longer here"))?;
    let master = master_of(state, &viewed).await?;
    let (client, _) = crate::caldav_account::client_for_calendar(state, master.calendar_id).await?;
    // The destination's collection URL, and its read-only refusal: one call
    // does both, so a move onto a calendar that cannot be written to is turned
    // down by the same rule an edit there would be.
    let (_, collection_url) =
        crate::caldav_account::client_for_calendar(state, target_calendar_id).await?;
    let (href, raw) = resource_of(state, master.id).await?;
    let uid = master.google_id.clone();
    let destination = resource_href(&collection_url, &uid);

    // `None` rather than an etag: this is a create in the destination, so
    // `If-None-Match: *` — a UID that somehow already exists there fails
    // loudly instead of overwriting a stranger's event.
    client.put(&destination, &raw, None).await.map_err(friendly)?;

    if let Err(e) = client.delete(&href, master.etag.as_deref()).await {
        tracing::error!(%e, %uid, "moved the event but could not remove the original");
        anyhow::bail!(crate::events::MOVED_NOT_REMOVED);
    }

    // Both collections, because both changed. The destination is resynced
    // last so the row this returns is the one it just brought in.
    resync(state, master.calendar_id).await?;
    resync(state, target_calendar_id).await?;
    let row = row_id_by_uid(state, target_calendar_id, &uid).await?;
    crate::events::event_detail_impl(state, row).await
}

/// Deletes an event, one occurrence, or the rest of a series.
pub(crate) async fn delete(
    state: &AppState,
    id: i64,
    scope: &str,
    occurrence_start_ms: i64,
) -> anyhow::Result<()> {
    let (viewed, _, _) = omacal_store::event_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("that event is no longer here"))?;
    let master = master_of(state, &viewed).await?;
    let (client, _) = crate::caldav_account::client_for_calendar(state, master.calendar_id).await?;
    let (href, raw) = resource_of(state, master.id).await?;
    let uid = master.google_id.clone();
    let is_series = master.recurrence.is_some();

    if !is_series || scope == "all" {
        client.delete(&href, master.etag.as_deref()).await.map_err(friendly)?;
    } else if scope == "following" {
        if occurrence_start_ms <= master.start_utc {
            client.delete(&href, master.etag.as_deref()).await.map_err(friendly)?;
        } else {
            let cut = occurrence_time(&master, occurrence_start_ms)?;
            let until = crate::write::until_value(
                occurrence_start_ms,
                master.is_all_day,
                &master.calendar_timezone,
            );
            let truncated = omacal_caldav::truncate_series(&raw, &uid, &until, &cut)
                .ok_or_else(|| anyhow::anyhow!("could not truncate the series"))?;
            client.put(&href, &truncated, master.etag.as_deref()).await.map_err(friendly)?;
        }
    } else {
        let occurrence = occurrence_time(&master, occurrence_start_ms)?;
        let excluded = omacal_caldav::exclude_occurrence(&raw, &uid, &occurrence)
            .ok_or_else(|| anyhow::anyhow!("could not remove the occurrence"))?;
        client.put(&href, &excluded, master.etag.as_deref()).await.map_err(friendly)?;
    }

    resync(state, master.calendar_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn master(recurrence: Option<&str>) -> StoredEvent {
        StoredEvent {
            id: 1,
            calendar_id: 1,
            google_id: "u".into(),
            summary: None,
            location: None,
            start_utc: 1_786_352_400_000, // 2026-08-10T09:00Z
            end_utc: 1_786_356_000_000,
            start_tz: "Europe/Sofia".into(),
            end_tz: "Europe/Sofia".into(),
            is_all_day: false,
            recurrence: recurrence.map(str::to_string),
            recurring_event_id: None,
            original_start_utc: None,
            status: "confirmed".into(),
            self_response: None,
            conference_uri: None,
            color_hex: None,
            calendar_timezone: "Europe/Sofia".into(),
            description: None,
            etag: None,
            sequence: 3,
            organizer_email: None,
            guests_can_modify: false,
            attendees: Vec::new(),
            reminders: omacal_store::Reminders {
                use_default: false,
                overrides: vec![omacal_store::Reminder { method: "popup".into(), minutes: 25 }],
            },
            calendar_default_reminders: Vec::new(),
        }
    }

    /// A move's destination is built from a collection URL the server chose
    /// the spelling of, so both spellings have to land on the same path.
    #[test]
    fn a_collection_url_joins_the_same_way_with_or_without_its_slash() {
        assert_eq!(
            resource_href("https://dav.example.com/u/calendar/", "abc"),
            "https://dav.example.com/u/calendar/abc.ics",
        );
        assert_eq!(
            resource_href("https://dav.example.com/u/calendar", "abc"),
            "https://dav.example.com/u/calendar/abc.ics",
        );
    }

    /// The answer vocabulary, both directions: the three words the popover
    /// can send map to their PARTSTAT spellings, and anything else — a typo,
    /// a future word, `needsAction` itself — is refused before a byte moves.
    #[test]
    fn only_the_three_answers_render_as_partstat() {
        assert_eq!(partstat_of("accepted"), Some("ACCEPTED"));
        assert_eq!(partstat_of("declined"), Some("DECLINED"));
        assert_eq!(partstat_of("tentative"), Some("TENTATIVE"));
        assert_eq!(partstat_of("needsAction"), None, "un-answering is not an answer");
        assert_eq!(partstat_of("ACCEPTED"), None, "the wire vocabulary is lowercase");
        assert_eq!(partstat_of(""), None);
    }

    #[test]
    fn untouched_reminders_inherit_and_touched_ones_replace() {
        let m = master(None);
        assert_eq!(alarms_for(&None, Some(&m)), vec![("popup".to_string(), 25)]);
        let cleared = crate::write::RemindersInput { use_default: true, overrides: vec![] };
        assert!(alarms_for(&Some(cleared), Some(&m)).is_empty());
        assert!(alarms_for(&None, None).is_empty(), "a create starts silent");
    }

    #[test]
    fn recurrence_inherits_verbatim_but_count_never_survives_a_split() {
        let m = master(Some("RRULE:FREQ=DAILY;COUNT=10"));
        assert_eq!(recurrence_for(&None, Some(&m), false), vec!["RRULE:FREQ=DAILY;COUNT=10"]);
        assert_eq!(recurrence_for(&None, Some(&m), true), vec!["RRULE:FREQ=DAILY"]);
        assert_eq!(
            recurrence_for(&Some(Some("RRULE:FREQ=WEEKLY".into())), Some(&m), false),
            vec!["RRULE:FREQ=WEEKLY"]
        );
        assert!(recurrence_for(&Some(None), Some(&m), false).is_empty(), "cleared is cleared");
    }

    #[test]
    fn an_occurrence_renders_like_its_series_dtstart() {
        let timed = occurrence_time(&master(None), 1_786_352_400_000).unwrap();
        match timed {
            WriteTime::Zoned { dt, tzid } => {
                assert_eq!(tzid, "Europe/Sofia");
                assert_eq!((dt.hour(), dt.minute()), (12, 0), "09:00Z is 12:00 Sofia in August");
            }
            other => panic!("expected zoned, got {other:?}"),
        }
        let mut all_day = master(None);
        all_day.is_all_day = true;
        // Midnight Sofia on Aug 10 = Aug 9, 21:00Z.
        let d = occurrence_time(&all_day, 1_786_309_200_000).unwrap();
        assert_eq!(d, WriteTime::Date(jiff::civil::date(2026, 8, 10)));
    }

    #[test]
    fn endpoints_resolve_both_shapes() {
        let (s, _, ms) = endpoints(
            &When::Timed { start_ms: 1_786_352_400_000, end_ms: 1_786_356_000_000 },
            "Europe/Sofia",
            "UTC",
        )
        .unwrap();
        assert_eq!(ms, 1_786_352_400_000);
        assert!(matches!(s, WriteTime::Zoned { ref tzid, .. } if tzid == "Europe/Sofia"));

        let (s, e, ms) = endpoints(
            &When::AllDay { start_date: "2026-08-20".into(), end_date: "2026-08-21".into() },
            "Europe/Sofia",
            "Europe/Sofia",
        )
        .unwrap();
        assert_eq!(s, WriteTime::Date(jiff::civil::date(2026, 8, 20)));
        assert_eq!(e, WriteTime::Date(jiff::civil::date(2026, 8, 21)));
        // Midnight Sofia = 21:00Z the day before.
        assert_eq!(ms % 86_400_000, 75_600_000);
    }
}
