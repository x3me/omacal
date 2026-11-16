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
//! - **following**: a brand-new resource (new UID) carrying the rest, written
//!   first, then `UNTIL` on the master's rule. Refused, as a Google split is,
//!   when the master's rule ends after a `COUNT` (the rest would need what is
//!   left of it, which only a full expansion knows) and when later
//!   occurrences were changed on their own (shortening the master takes those
//!   changes with it).
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

/// Whether anyone hears about a write on this calendar: only Google mails.
/// CalDAV has no guest management and subscribed feeds are read-only, so both
/// read as silent — the same answer the form's notify question is gated on.
pub(crate) async fn calendar_mails_guests(
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
    Ok(provider == "google")
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
        _ => anyhow::bail!(EVENT_NOT_SYNCED_YET),
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
/// The instant that names one occurrence of a series: where the series put
/// it, not where it sits now. For an occurrence that was moved on its own
/// the two differ, and only the first matches its exception's
/// `RECURRENCE-ID` — rendering the second would address a slot the series
/// never had, so an edit would add a second exception and a delete would
/// exclude the wrong slot and leave the moved occurrence on screen.
fn occurrence_identity(viewed: &StoredEvent, occurrence_start_ms: i64) -> i64 {
    viewed.original_start_utc.unwrap_or(occurrence_start_ms)
}

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

/// The recurrence lines a write carries: the form's rule when it set one,
/// none when it chose Never, and the master's own when Repeat was left alone.
fn recurrence_for(input: &Option<Option<String>>, master: Option<&StoredEvent>) -> Vec<String> {
    match input {
        Some(Some(rule)) => vec![rule.clone()],
        Some(None) => Vec::new(),
        None => master
            .and_then(|m| m.recurrence.as_deref())
            .map(|r| r.lines().map(str::to_string).collect())
            .unwrap_or_default(),
    }
}

fn now_ts() -> jiff::Timestamp {
    jiff::Timestamp::from_millisecond(crate::now_ms()).unwrap_or(jiff::Timestamp::UNIX_EPOCH)
}

/// An RSVP on a CalDAV event whose copy on the server has no ATTENDEE line
/// for this account: nothing to answer. Raised before any write;
/// allowlisted in `errors.rs`.
pub(crate) const NOT_A_GUEST_HERE: &str =
    "this event's copy on the server does not list you as a guest, so there is no invitation here to answer";

/// The 412 an etag-guarded write answers with, worded as its task twin
/// `tasks::TASK_CHANGED_ON_SERVER` is. Both name the one fix there is —
/// sync, then save again — and until 2026-09-23 only the task one was
/// allow-listed, so the same conflict told a task user what to do and left
/// an event user reading "Sync failed. See the application log for details."
pub(crate) const EVENT_CHANGED_ON_SERVER: &str =
    "That event changed on the server since it was loaded — sync and try again";

/// The write paths' two refusals before anything leaves the machine.
pub(crate) const EVENT_NOT_SYNCED_YET: &str =
    "this event has not finished syncing; try again in a moment";

fn friendly(e: CalDavError) -> anyhow::Error {
    match e {
        CalDavError::PreconditionFailed => anyhow::anyhow!(EVENT_CHANGED_ON_SERVER),
        // In the allowlisted words any other status gets: the user learns the
        // server said no, rather than reading "Sync failed".
        CalDavError::Forbidden => {
            anyhow::anyhow!(crate::errors::server_answered(reqwest::StatusCode::FORBIDDEN))
        }
        other => anyhow::Error::from(other),
    }
}

/// Post-write reconciliation: one collection resynced through the read path,
/// and the bar's feed refreshed.
pub(crate) async fn resync(state: &AppState, calendar_id: i64) -> anyhow::Result<()> {
    let (client, collection_url) =
        crate::caldav_account::client_for_calendar(state, calendar_id).await?;
    let (supports_events, supports_tasks): (i64, i64) = sqlx::query_as(
        "SELECT supports_events, supports_tasks FROM calendars WHERE id = ?1",
    )
    .bind(calendar_id)
    .fetch_one(&state.pool)
    .await?;
    let now = crate::now_ms();
    let (ws, we) = crate::synced_window(state, now);
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
///
/// `uid` is the identity to keep, when the event already has one: an `.ics`
/// import passes the file's own UID (#180), so importing the same file again
/// is recognised (`import::existing_uids` reads this calendar's UIDs) and a
/// later update from the same sender can address the same event. `None` is a
/// new event, which gets a fresh UUID. A kept UID names the event, not the
/// file: the resource still gets a fresh UUID name, since a UID may hold
/// characters a URL path cannot (`/`, spaces) and is not ours to escape.
pub(crate) async fn create(
    state: &AppState,
    calendar_id: i64,
    fields: EventFields,
    uid: Option<&str>,
) -> anyhow::Result<crate::events::EventDetail> {
    let (client, collection_url) =
        crate::caldav_account::client_for_calendar(state, calendar_id).await?;
    let cal_tz: String = sqlx::query_scalar("SELECT timezone FROM calendars WHERE id = ?1")
        .bind(calendar_id)
        .fetch_one(&state.pool)
        .await?;

    let (start, end, _) = endpoints(&fields.when, &fields.tz, &cal_tz)?;
    let (resource, uid) = identity(uid);
    let ev = EventWrite {
        uid: uid.clone(),
        summary: fields.summary,
        location: fields.location,
        description: fields.description,
        start,
        end,
        recurrence: recurrence_for(&fields.recurrence, None),
        recurrence_id: None,
        alarms: alarms_for(&fields.reminders, None),
        sequence: 0,
        attendees: attendees_for(&fields.guests),
        conference: None,
        // Only what differs from a server's own assumption, the rule Google's
        // create keeps: no `TRANSP:OPAQUE` or empty claim nobody made.
        transp: fields
            .transparency
            .filter(|t| *t != crate::write::Transparency::Opaque)
            .map(crate::write::Transparency::transp),
        class: fields
            .visibility
            .filter(|v| *v != crate::write::Visibility::Default)
            .map(crate::write::Visibility::class),
    };
    let ics = omacal_caldav::new_event_ics(&ev, now_ts());
    let href = format!("{}/{resource}.ics", collection_url.trim_end_matches('/'));
    client.put(&href, &ics, None).await.map_err(friendly)?;

    created_but(
        async {
            resync(state, calendar_id).await?;
            let id = row_id_by_uid(state, calendar_id, &uid).await?;
            crate::events::event_detail_impl(state, id).await
        }
        .await,
    )
}

/// The resource name and the UID a create writes, given the UID the event
/// already has, if any (see [`create`]). A new event is named by its own fresh
/// UID, as it always was; a kept UID gets a fresh name beside it.
fn identity(kept: Option<&str>) -> (String, String) {
    let fresh = uuid::Uuid::new_v4().to_string();
    match kept.map(str::trim).filter(|u| !u.is_empty()) {
        Some(uid) => (fresh, uid.to_string()),
        None => (fresh.clone(), fresh),
    }
}

/// **Past the PUT, no error may read as "the create failed."** The event
/// exists on the server; a failure in the local half reported like the ones
/// before it invites the user to create it again (the app's Reopen even offers
/// to), and the second attempt duplicates it. The same rule
/// `create_via_client` keeps for Google: collapse into the safelisted
/// [`crate::events::CREATED_NOT_STORED`], which the UI keeps drawn until the
/// next sync heals it, with the cause kept for the log. (Its wording names
/// Google; the promise it makes holds for a CalDAV server too.)
fn created_but<T>(r: anyhow::Result<T>) -> anyhow::Result<T> {
    r.map_err(|e| {
        tracing::error!(%e, "created on the CalDAV server, but the local half failed");
        anyhow::anyhow!(crate::events::CREATED_NOT_STORED)
    })
}

/// [`created_but`]'s rule for an edit, a calendar move or a delete: past the
/// server's write, a failure in the local half collapses into the safelisted
/// [`crate::events::SAVED_NOT_STORED`], so the UI keeps the change drawn until
/// the next sync instead of undoing what happened. The cause goes to the log.
fn saved_but<T>(r: anyhow::Result<T>) -> anyhow::Result<T> {
    r.map_err(|e| {
        tracing::error!(%e, "changed on the CalDAV server, but the local half failed");
        anyhow::anyhow!(crate::events::SAVED_NOT_STORED)
    })
}

/// A stored "Show as" as the `TRANSP` a fresh resource should carry. Busy is
/// `None`, since an absent `TRANSP` already means `OPAQUE`.
pub(crate) fn transp_of_stored(transparency: &str) -> Option<omacal_caldav::Transp> {
    (transparency == "transparent").then_some(omacal_caldav::Transp::Transparent)
}

/// A stored visibility as the `CLASS` a fresh resource should carry. Default
/// is `None`: no line, which is what an event that never chose looks like.
pub(crate) fn class_of_stored(visibility: &str) -> Option<omacal_caldav::Class> {
    match visibility {
        "public" => Some(omacal_caldav::Class::Public),
        "private" => Some(omacal_caldav::Class::Private),
        "confidential" => Some(omacal_caldav::Class::Confidential),
        _ => None,
    }
}

/// "Show as" and visibility for the tail of a "this and following" split.
///
/// The tail is a brand-new resource with no lines of its own to keep, so a
/// field the form did not touch takes the value the dialog showed: the clicked
/// occurrence's own. That is the series' for an ordinary occurrence, and the
/// exception's for one that was moved and changed on its own, so a free
/// occurrence of a busy series splits into a free series.
fn tail_show_as(
    fields: &EventFields,
    occurrence: &StoredEvent,
) -> (Option<omacal_caldav::Transp>, Option<omacal_caldav::Class>) {
    let transp = fields
        .transparency
        .map(crate::write::Transparency::transp)
        .or_else(|| transp_of_stored(&occurrence.transparency));
    let class = fields
        .visibility
        .map(crate::write::Visibility::class)
        .or_else(|| class_of_stored(&occurrence.visibility));
    (transp, class)
}

/// The attendee list a write owns, out of the guest list the form sends.
///
/// `None` all the way through: a path with no guest editor (a drag, the CLI's
/// time edit) leaves every `ATTENDEE` line on the resource alone, which is the
/// rule [`omacal_caldav::EventWrite::attendees`] is built on. Nothing is
/// validated here beyond what the form allows, because an address this app
/// cannot parse is still an address the server may know.
fn attendees_for(guests: &Option<Vec<crate::write::Guest>>) -> Option<Vec<omacal_caldav::AttendeeWrite>> {
    guests.as_ref().map(|list| {
        list.iter()
            .map(|g| omacal_caldav::AttendeeWrite {
                address: g.email.clone(),
                display_name: g.display_name.clone(),
                optional: g.optional,
            })
            .collect()
    })
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
                recurrence: recurrence_for(&fields.recurrence, Some(&master)),
                recurrence_id: None,
                alarms: alarms_for(&fields.reminders, Some(&master)),
                sequence: master.sequence + 1,
                attendees: attendees_for(&fields.guests),
                conference: None,
                // `None` keeps the resource's own lines; see `rewrite_master`.
                transp: fields.transparency.map(crate::write::Transparency::transp),
                class: fields.visibility.map(crate::write::Visibility::class),
            };
            let out = omacal_caldav::rewrite_master(&raw, &uid, &ev, now, moved)
                .ok_or_else(|| anyhow::anyhow!("could not rewrite the event's resource"))?;
            (out, None, uid.clone())
        } else if scope == "following" {
            // Cut at the slot the series put the occurrence in, as a Google
            // split does: cut where a moved occurrence sits now and the old
            // series keeps it while the new one starts on it too.
            let split_at_ms = occurrence_identity(&viewed, occurrence_start_ms);
            if split_at_ms <= master.start_utc {
                // "This and following" from the first occurrence is the
                // whole series; recurse as an "all" edit.
                return Box::pin(update(state, id, "all", occurrence_start_ms, fields)).await;
            }
            // The Google split's two refusals, in its words and before
            // anything is sent. A counted rule cannot be carried: the rest
            // would need what is left of the count, and dropping it made the
            // rest repeat forever. And later occurrences changed on their own
            // live under this master, so shortening it takes them away.
            if master.recurrence.as_deref().is_some_and(|r| r.lines().any(crate::write::has_count)) {
                anyhow::bail!(crate::events::SPLIT_REFUSES_COUNT);
            }
            let stranded = omacal_store::exceptions_from(
                &state.pool,
                master.calendar_id,
                &uid,
                split_at_ms,
                &viewed.google_id,
            )
            .await?;
            if stranded > 0 {
                anyhow::bail!("{}{stranded}", crate::events::SPLIT_STRANDS_PREFIX);
            }
            let cut = occurrence_time(&master, split_at_ms)?;
            let until = crate::write::until_value(
                split_at_ms,
                master.is_all_day,
                &master.calendar_timezone,
            );
            let truncated = omacal_caldav::truncate_series(&raw, &uid, &until, &cut)
                .ok_or_else(|| anyhow::anyhow!("could not truncate the series"))?;

            let new_uid = uuid::Uuid::new_v4().to_string();
            let (transp, class) = tail_show_as(&fields, &viewed);
            let ev = EventWrite {
                uid: new_uid.clone(),
                summary: fields.summary,
                location: fields.location,
                description: fields.description,
                start,
                end,
                recurrence: recurrence_for(&fields.recurrence, Some(&master)),
                recurrence_id: None,
                // Untouched reminders are the ones the dialog showed: the
                // clicked occurrence's, which a moved one may have of its own.
                alarms: alarms_for(&fields.reminders, Some(&viewed)),
                sequence: 0,
                attendees: attendees_for(&fields.guests),
                conference: None,
                transp,
                class,
            };
            let ics = omacal_caldav::new_event_ics(&ev, now);
            let collection = href.rsplit_once('/').map(|(c, _)| c).unwrap_or("");
            let new_href = format!("{collection}/{new_uid}.ics");
            (truncated, Some((new_href, ics)), new_uid)
        } else {
            // Scope "this": one occurrence becomes (or updates) an exception.
            let rid_ms = occurrence_identity(&viewed, occurrence_start_ms);
            let rid = occurrence_time(&master, rid_ms)?;
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
                attendees: attendees_for(&fields.guests),
                conference: None,
                // `None` carries the exception's own lines, or the master's
                // for a new exception; see `upsert_exception`.
                transp: fields.transparency.map(crate::write::Transparency::transp),
                class: fields.visibility.map(crate::write::Visibility::class),
            };
            let out = omacal_caldav::upsert_exception(&raw, &ev, now)
                .ok_or_else(|| anyhow::anyhow!("could not write the occurrence's exception"))?;
            (out, None, format!("{uid}#{rid_ms}"))
        };

    // A split writes the rest of the series first and shortens the original
    // second. In the other order a failure between the two loses every future
    // occurrence; in this one it leaves a duplicate, which the user can see
    // and is told about. Google's split keeps the same order for the same
    // reason.
    match follow_up {
        None => {
            client.put(&href, &new_raw, master.etag.as_deref()).await.map_err(friendly)?;
        }
        Some((new_href, ics)) => {
            client.put(&new_href, &ics, None).await.map_err(friendly)?;
            if let Err(e) = client.put(&href, &new_raw, master.etag.as_deref()).await {
                tracing::error!(%e, %uid, "split wrote the new series but could not shorten the original");
                anyhow::bail!(crate::events::SPLIT_LEFT_TWO_SERIES);
            }
        }
    }

    saved_but(
        async {
            resync(state, master.calendar_id).await?;
            let row = row_id_by_uid(state, master.calendar_id, &detail_uid).await;
            // An exception the resync collapsed away (or a row the server
            // renamed) falls back to the master — the popover shows
            // *something* true.
            let row = match row {
                Ok(r) => r,
                Err(_) => row_id_by_uid(state, master.calendar_id, &uid).await?,
            };
            crate::events::event_detail_impl(state, row).await
        }
        .await,
    )
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
        let rid_ms = occurrence_identity(&viewed, occurrence_start_ms);
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
    let new_raw = new_raw.ok_or_else(|| anyhow::anyhow!(NOT_A_GUEST_HERE))?;

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
    // A name of this app's own making, as `create` mints one: a UID is the
    // author's text and may hold `/`, `..`, `?` or `#`, none of which may
    // steer where the copy lands — or which resource the DELETE below is
    // then aimed past.
    let destination = resource_href(&collection_url, &uuid::Uuid::new_v4().to_string());

    // `None` rather than an etag: this is a create in the destination, so
    // `If-None-Match: *` — a name that somehow already exists there fails
    // loudly instead of overwriting a stranger's event.
    client.put(&destination, &raw, None).await.map_err(friendly)?;

    if let Err(e) = client.delete(&href, master.etag.as_deref()).await {
        tracing::error!(%e, %uid, "moved the event but could not remove the original");
        anyhow::bail!(crate::events::MOVED_NOT_REMOVED);
    }

    // Both collections, because both changed. The destination is resynced
    // last so the row this returns is the one it just brought in.
    saved_but(
        async {
            resync(state, master.calendar_id).await?;
            resync(state, target_calendar_id).await?;
            let row = row_id_by_uid(state, target_calendar_id, &uid).await?;
            crate::events::event_detail_impl(state, row).await
        }
        .await,
    )
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
        // The slot the series put the occurrence in, as for an edit.
        let split_at_ms = occurrence_identity(&viewed, occurrence_start_ms);
        if split_at_ms <= master.start_utc {
            client.delete(&href, master.etag.as_deref()).await.map_err(friendly)?;
        } else {
            let cut = occurrence_time(&master, split_at_ms)?;
            let until = crate::write::until_value(
                split_at_ms,
                master.is_all_day,
                &master.calendar_timezone,
            );
            let truncated = omacal_caldav::truncate_series(&raw, &uid, &until, &cut)
                .ok_or_else(|| anyhow::anyhow!("could not truncate the series"))?;
            client.put(&href, &truncated, master.etag.as_deref()).await.map_err(friendly)?;
        }
    } else {
        let occurrence = occurrence_time(&master, occurrence_identity(&viewed, occurrence_start_ms))?;
        let excluded = omacal_caldav::exclude_occurrence(&raw, &uid, &occurrence)
            .ok_or_else(|| anyhow::anyhow!("could not remove the occurrence"))?;
        client.put(&href, &excluded, master.etag.as_deref()).await.map_err(friendly)?;
    }

    saved_but(resync(state, master.calendar_id).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Past the PUT the event exists on the server, so a failure in the local
    /// half must say so rather than read as a failed create (part 3 review):
    /// the UI keeps it drawn, offers no Reopen, and the next sync heals it.
    /// An edit, move or delete whose server write landed: a failure in the
    /// local half keeps the change (the UI holds it drawn until the sync).
    #[test]
    fn a_failure_after_a_change_landed_reads_as_saved_not_stored() {
        let failed: anyhow::Result<()> = saved_but(Err(anyhow::anyhow!("resync timed out")));
        assert_eq!(failed.unwrap_err().to_string(), crate::events::SAVED_NOT_STORED);
        assert_eq!(saved_but(Ok(3)).unwrap(), 3);
    }

    /// An imported event keeps the file's UID (#180): it is what lets a second
    /// import of the same file be recognised, and a later update from the same
    /// sender address the same event. Before, every create minted a UUID, so
    /// the same file imported twice made every event twice.
    #[test]
    fn a_create_keeps_an_identity_the_event_already_has() {
        let (resource, uid) = identity(Some("nvbpels-ethics-20261110@omamail"));
        assert_eq!(uid, "nvbpels-ethics-20261110@omamail");
        assert!(uuid::Uuid::parse_str(&resource).is_ok(), "the resource still gets a safe name: {resource}");

        // A UID is not ours to escape into a path.
        let (resource, uid) = identity(Some("a/b c@x"));
        assert_eq!(uid, "a/b c@x");
        assert!(!resource.contains('/') && !resource.contains(' '), "{resource}");

        // A new event, or a blank UID, is a fresh UUID naming its own resource.
        for kept in [None, Some(""), Some("   ")] {
            let (resource, uid) = identity(kept);
            assert!(uuid::Uuid::parse_str(&uid).is_ok(), "{kept:?} → {uid}");
            assert_eq!(resource, uid, "{kept:?}");
        }
        assert_ne!(identity(None).1, identity(None).1, "fresh every time");
    }

    #[test]
    fn a_failure_after_the_put_reads_as_created_not_stored() {
        let failed: anyhow::Result<i64> =
            created_but(Err(anyhow::anyhow!("the write landed but the resync did not find it")));
        assert_eq!(failed.unwrap_err().to_string(), crate::events::CREATED_NOT_STORED);
        assert_eq!(created_but(Ok(7)).unwrap(), 7);
    }

    /// The three-state that keeps every guest-free write harmless (#114): a
    /// path with no attendee editor sends `None`, and `None` is what tells
    /// `rewrite_master` to leave every ATTENDEE line where the server put it.
    /// `Some(vec![])` is a different thing entirely and has to survive as one.
    #[test]
    fn only_an_edit_that_owns_the_list_carries_attendees() {
        assert_eq!(attendees_for(&None), None);
        assert_eq!(attendees_for(&Some(Vec::new())), Some(Vec::new()));
        let list = Some(vec![
            crate::write::Guest { email: "ada@example.com".into(), optional: false, display_name: Some("Ada".into()) },
            crate::write::Guest { email: "urn:uuid:12345678".into(), optional: true, display_name: None },
        ]);
        let out = attendees_for(&list).unwrap();
        assert_eq!(out[0].address, "ada@example.com");
        assert_eq!(out[0].display_name.as_deref(), Some("Ada"));
        assert!(!out[0].optional);
        // The URI is carried as typed. What becomes of it is `cal_address`'s
        // business, and it is tested where it lives.
        assert_eq!(out[1].address, "urn:uuid:12345678");
        assert!(out[1].optional);
    }

    /// A form that touched nothing but "Show as" and visibility.
    fn form_with(
        transparency: Option<crate::write::Transparency>,
        visibility: Option<crate::write::Visibility>,
    ) -> EventFields {
        EventFields {
            summary: None,
            location: None,
            description: None,
            when: When::Timed { start_ms: 0, end_ms: 0 },
            tz: "Europe/Sofia".into(),
            recurrence: None,
            guests: None,
            reminders: None,
            conference: None,
            transparency,
            visibility,
        }
    }

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
            transparency: "opaque".into(), visibility: "default".into(),
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

    /// What a fresh resource (an export, the tail of a split) says about a
    /// stored "Show as" and visibility: free and every chosen class as lines,
    /// confidential included since a split must not quietly drop it, and busy
    /// and default as no line at all, which is what they mean.
    #[test]
    fn a_fresh_resource_carries_free_and_a_class_but_writes_nothing_for_the_defaults() {
        assert_eq!(transp_of_stored("transparent"), Some(omacal_caldav::Transp::Transparent));
        assert_eq!(transp_of_stored("opaque"), None);
        assert_eq!(class_of_stored("public"), Some(omacal_caldav::Class::Public));
        assert_eq!(class_of_stored("private"), Some(omacal_caldav::Class::Private));
        assert_eq!(class_of_stored("confidential"), Some(omacal_caldav::Class::Confidential));
        assert_eq!(class_of_stored("default"), None);
    }

    /// The tail of a split takes "Show as" and visibility from the occurrence
    /// the dialog was opened on, not from the series: a moved occurrence set
    /// to Free and Private on a busy series splits into a free, private tail
    /// when the user left both alone. A value the form did set still wins.
    #[test]
    fn a_split_tail_keeps_the_clicked_occurrences_show_as_and_visibility() {
        let mut moved = master(None);
        moved.transparency = "transparent".into();
        moved.visibility = "private".into();
        let untouched = form_with(None, None);
        assert_eq!(
            tail_show_as(&untouched, &moved),
            (Some(omacal_caldav::Transp::Transparent), Some(omacal_caldav::Class::Private)),
        );

        let chosen = form_with(
            Some(crate::write::Transparency::Opaque),
            Some(crate::write::Visibility::Public),
        );
        assert_eq!(
            tail_show_as(&chosen, &moved),
            (Some(omacal_caldav::Transp::Opaque), Some(omacal_caldav::Class::Public)),
        );
    }

    /// Inherited verbatim, `COUNT` included: a split never inherits a counted
    /// rule, because it refuses one before it gets here.
    #[test]
    fn recurrence_inherits_verbatim_and_the_forms_rule_replaces_it() {
        let m = master(Some("RRULE:FREQ=DAILY;COUNT=10"));
        assert_eq!(recurrence_for(&None, Some(&m)), vec!["RRULE:FREQ=DAILY;COUNT=10"]);
        assert_eq!(
            recurrence_for(&Some(Some("RRULE:FREQ=WEEKLY".into())), Some(&m)),
            vec!["RRULE:FREQ=WEEKLY"]
        );
        assert!(recurrence_for(&Some(None), Some(&m)).is_empty(), "cleared is cleared");
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

    /// The write path driven end to end against a wiremock CalDAV server:
    /// rows seeded from a real resource the way a sync stores them, the
    /// public `update`/`delete` called the way the commands call them, and
    /// the bytes that reach the server read back off the mock.
    mod against_a_server {
        use super::super::*;
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        /// A weekly 10:00 standup in Sofia, its 9 November occurrence moved
        /// to Tuesday 10 November. The move crosses a day, so the moved start
        /// and the original slot cannot render to the same RECURRENCE-ID.
        const MOVED: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\n\
            BEGIN:VEVENT\r\nUID:series-1\r\nDTSTAMP:20261101T000000Z\r\n\
            DTSTART;TZID=Europe/Sofia:20261102T100000\r\nDTEND;TZID=Europe/Sofia:20261102T110000\r\n\
            RRULE:FREQ=WEEKLY\r\nSUMMARY:Standup\r\nEND:VEVENT\r\n\
            BEGIN:VEVENT\r\nUID:series-1\r\nDTSTAMP:20261101T000000Z\r\n\
            RECURRENCE-ID;TZID=Europe/Sofia:20261109T100000\r\n\
            DTSTART;TZID=Europe/Sofia:20261110T100000\r\nDTEND;TZID=Europe/Sofia:20261110T110000\r\n\
            SUMMARY:Standup\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

        pub(super) fn ms(s: &str) -> i64 {
            s.parse::<jiff::Timestamp>().unwrap().as_millisecond()
        }

        /// A pool holding one CalDAV account on `server`, one writable
        /// collection, and `raw` stored as that collection's resource.
        /// Returns the state and the row id of every component, keyed by the
        /// store's google_id (`series-1`, `series-1#<orig ms>`).
        pub(super) async fn seeded(
            server: &MockServer,
            email: &str,
            raw: &str,
        ) -> (AppState, std::collections::HashMap<String, i64>) {
            crate::caldav_account::test_passwords::set(email, "p");
            let pool = omacal_store::connect_memory().await.unwrap();
            let account: i64 = sqlx::query_scalar(
                "INSERT INTO accounts (google_sub, email, created_at, provider, server_url, username)
                 VALUES (?1, ?2, 0, 'caldav', ?3, ?2) RETURNING id",
            )
            .bind(format!("caldav:{email}"))
            .bind(email)
            .bind(server.uri())
            .fetch_one(&pool)
            .await
            .unwrap();
            let calendar: i64 = sqlx::query_scalar(
                "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
                 VALUES (?1, ?2, 'Work', 'Europe/Sofia', 'owner') RETURNING id",
            )
            .bind(account)
            .bind(format!("{}/cal/", server.uri()))
            .fetch_one(&pool)
            .await
            .unwrap();
            let href = format!("{}/cal/series-1.ics", server.uri());
            let root = omacal_caldav::parse(raw).unwrap();
            let mut ids = std::collections::HashMap::new();
            for ev in omacal_caldav::events_in(&root) {
                let row = omacal_sync::caldav::caldav_to_stored(&ev, calendar, "Europe/Sofia", Some("\"v1\""), email)
                    .unwrap();
                let id = omacal_store::upsert_event(&pool, &row).await.unwrap();
                sqlx::query("UPDATE events SET caldav_href = ?1, raw_ics = ?2 WHERE id = ?3")
                    .bind(&href)
                    .bind(raw)
                    .bind(id)
                    .execute(&pool)
                    .await
                    .unwrap();
                ids.insert(row.google_id.clone(), id);
            }
            let state = AppState {
                pool,
                demo: false,
                tokens: Default::default(),
                reauth: Default::default(),
                update: Default::default(),
                update_checked_at: Default::default(),
                system_tz_change: Default::default(),
                quit_on_close: Default::default(),
                open_date: Default::default(),
                history_months: std::sync::atomic::AtomicI64::new(crate::settings::DEFAULT_HISTORY_MONTHS),
            };
            (state, ids)
        }

        /// Every PUT body the server received, in order.
        pub(super) async fn puts(server: &MockServer) -> Vec<String> {
            server
                .received_requests()
                .await
                .unwrap()
                .into_iter()
                .filter(|r| r.method.as_str() == "PUT")
                .map(|r| String::from_utf8(r.body).unwrap())
                .collect()
        }

        pub(super) async fn accepting(server: &MockServer) {
            Mock::given(method("PUT"))
                .respond_with(ResponseTemplate::new(204).insert_header("ETag", "\"v2\""))
                .mount(server)
                .await;
        }

        fn retitled(start_ms: i64, end_ms: i64) -> EventFields {
            EventFields {
                summary: Some("Standup (moved)".into()),
                location: None,
                description: None,
                when: When::Timed { start_ms, end_ms },
                tz: "Europe/Sofia".into(),
                recurrence: None,
                guests: None,
                reminders: None,
                conference: None,
                transparency: None,
                visibility: None,
            }
        }

        fn lines_starting<'a>(ics: &'a str, prefix: &str) -> Vec<&'a str> {
            ics.lines().map(str::trim_end).filter(|l| l.starts_with(prefix)).collect()
        }

        /// Editing an occurrence that was already moved rewrites *its*
        /// exception, named by the slot it was moved from. Naming it by where
        /// it sits now would add a second exception for a slot the series
        /// never had, and the occurrence would show twice.
        #[tokio::test]
        async fn editing_a_moved_occurrence_rewrites_its_own_exception() {
            let server = MockServer::start().await;
            accepting(&server).await;
            let (state, ids) = seeded(&server, "edit-moved@x.test", MOVED).await;
            let original = ms("2026-11-09T08:00:00Z");
            let exception = ids[&format!("series-1#{original}")];
            let moved = ms("2026-11-10T08:00:00Z");

            let _ = update(&state, exception, "this", moved, retitled(moved, moved + 3_600_000)).await;

            let sent = puts(&server).await;
            assert_eq!(sent.len(), 1, "one resource rewritten: {sent:?}");
            assert_eq!(
                lines_starting(&sent[0], "RECURRENCE-ID"),
                vec!["RECURRENCE-ID;TZID=Europe/Sofia:20261109T100000"],
                "one exception, still named by its original slot:\n{}",
                sent[0]
            );
            assert!(sent[0].contains("SUMMARY:Standup (moved)"), "and it carries the edit:\n{}", sent[0]);
        }

        /// Deleting an occurrence that was already moved excludes the slot it
        /// was moved from and takes its exception with it. Excluding where it
        /// sits now would leave the exception standing, and the occurrence on
        /// screen.
        #[tokio::test]
        async fn deleting_a_moved_occurrence_removes_its_exception() {
            let server = MockServer::start().await;
            accepting(&server).await;
            let (state, ids) = seeded(&server, "delete-moved@x.test", MOVED).await;
            let original = ms("2026-11-09T08:00:00Z");
            let exception = ids[&format!("series-1#{original}")];
            let moved = ms("2026-11-10T08:00:00Z");

            let _ = delete(&state, exception, "this", moved).await;

            let sent = puts(&server).await;
            assert_eq!(sent.len(), 1, "one resource rewritten: {sent:?}");
            assert!(lines_starting(&sent[0], "RECURRENCE-ID").is_empty(), "the exception is gone:\n{}", sent[0]);
            assert_eq!(
                lines_starting(&sent[0], "EXDATE"),
                vec!["EXDATE;TZID=Europe/Sofia:20261109T100000"],
                "and its original slot is excluded:\n{}",
                sent[0]
            );
        }

        /// A weekly series in Sofia with nothing moved, `rule` its RRULE line.
        fn plain(rule: &str) -> String {
            format!(
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\n\
                 BEGIN:VEVENT\r\nUID:series-1\r\nDTSTAMP:20261101T000000Z\r\n\
                 DTSTART;TZID=Europe/Sofia:20261102T100000\r\nDTEND;TZID=Europe/Sofia:20261102T110000\r\n\
                 {rule}\r\nSUMMARY:Standup\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
            )
        }

        /// PUT paths in the order the server received them.
        async fn put_paths(server: &MockServer) -> Vec<String> {
            server
                .received_requests()
                .await
                .unwrap()
                .into_iter()
                .filter(|r| r.method.as_str() == "PUT")
                .map(|r| r.url.path().to_string())
                .collect()
        }

        /// "This and following" from the third Monday, 16 November.
        async fn split_third(state: &AppState, ids: &std::collections::HashMap<String, i64>, recurrence: Option<Option<String>>) -> anyhow::Result<crate::events::EventDetail> {
            let at = ms("2026-11-16T08:00:00Z");
            let mut f = retitled(at, at + 3_600_000);
            f.recurrence = recurrence;
            update(state, ids["series-1"], "following", at, f).await
        }

        /// The new series is written before the old one is cut short. In the
        /// other order a failure between the two writes loses every future
        /// occurrence; in this one it leaves a duplicate, which is visible.
        #[tokio::test]
        async fn a_split_writes_the_new_series_before_it_shortens_the_old() {
            let server = MockServer::start().await;
            accepting(&server).await;
            let (state, ids) = seeded(&server, "split-order@x.test", &plain("RRULE:FREQ=WEEKLY")).await;

            let _ = split_third(&state, &ids, None).await;

            let paths = put_paths(&server).await;
            assert_eq!(paths.len(), 2, "{paths:?}");
            assert_ne!(paths[0], "/cal/series-1.ics", "the tail first: {paths:?}");
            assert_eq!(paths[1], "/cal/series-1.ics", "the original second: {paths:?}");
        }

        /// And when the old series will not shorten after the new one landed,
        /// the user is told there are now two, the same words a Google split
        /// uses.
        #[tokio::test]
        async fn a_split_whose_original_will_not_shorten_says_there_are_two_series() {
            let server = MockServer::start().await;
            Mock::given(method("PUT"))
                .and(wiremock::matchers::path("/cal/series-1.ics"))
                .respond_with(ResponseTemplate::new(500))
                .with_priority(1)
                .mount(&server)
                .await;
            accepting(&server).await;
            let (state, ids) = seeded(&server, "split-half@x.test", &plain("RRULE:FREQ=WEEKLY")).await;

            let err = split_third(&state, &ids, None).await.unwrap_err();

            assert_eq!(crate::errors::user_facing(&err), crate::events::SPLIT_LEFT_TWO_SERIES);
            assert_eq!(put_paths(&server).await.len(), 2, "the tail landed, which is why it says so");
        }

        /// A series that ends after a set number of times is refused before
        /// anything is sent, as a Google one is: the tail's count would have to
        /// be what is left of the original's, a number only a full expansion
        /// knows. Dropping it, as this once did, made the rest repeat forever.
        #[tokio::test]
        async fn a_series_that_ends_after_a_count_is_not_split() {
            let server = MockServer::start().await;
            accepting(&server).await;
            let (state, ids) = seeded(&server, "split-count@x.test", &plain("RRULE:FREQ=WEEKLY;COUNT=10")).await;

            for recurrence in [None, Some(Some("RRULE:FREQ=WEEKLY".to_string()))] {
                let err = split_third(&state, &ids, recurrence).await.unwrap_err();
                assert_eq!(crate::errors::user_facing(&err), crate::events::SPLIT_REFUSES_COUNT);
            }
            assert!(server.received_requests().await.unwrap().is_empty(), "nothing sent at all");
        }

        /// The user's own "ends after N" on the rest of a series is theirs to
        /// choose, and is written as chosen.
        #[tokio::test]
        async fn a_count_the_user_chose_for_the_rest_is_kept() {
            let server = MockServer::start().await;
            accepting(&server).await;
            let (state, ids) = seeded(&server, "split-chosen@x.test", &plain("RRULE:FREQ=WEEKLY")).await;

            let _ = split_third(&state, &ids, Some(Some("RRULE:FREQ=WEEKLY;COUNT=3".into()))).await;

            let tail = server.received_requests().await.unwrap().into_iter()
                .find(|r| r.method.as_str() == "PUT" && r.url.path() != "/cal/series-1.ics")
                .map(|r| String::from_utf8(r.body).unwrap())
                .expect("the rest of the series was written");
            assert_eq!(lines_starting(&tail, "RRULE"), vec!["RRULE:FREQ=WEEKLY;COUNT=3"], "the tail:\n{tail}");
        }

        /// A later occurrence changed on its own lives under the old series,
        /// and shortening it takes that change away. Refused before anything
        /// is sent, naming how many, as a Google split is.
        #[tokio::test]
        async fn a_split_that_would_strand_a_moved_occurrence_is_refused() {
            let server = MockServer::start().await;
            accepting(&server).await;
            let (state, ids) = seeded(&server, "split-stranded@x.test", MOVED).await;
            // From 9 November, the slot the moved occurrence came from, and
            // from the master's row: the moved occurrence is not the one being
            // edited, so it would be stranded rather than carried.
            let at = ms("2026-11-09T08:00:00Z");
            let err = update(&state, ids["series-1"], "following", at, retitled(at, at + 3_600_000))
                .await
                .unwrap_err();

            assert_eq!(
                crate::errors::user_facing(&err),
                format!("{}1", crate::events::SPLIT_STRANDS_PREFIX)
            );
            assert!(server.received_requests().await.unwrap().is_empty(), "nothing sent at all");
        }

        /// The UNTIL a shortened series ends at, read off the PUT to its own
        /// resource.
        async fn master_until(server: &MockServer) -> String {
            let body = server.received_requests().await.unwrap().into_iter()
                .filter(|r| r.method.as_str() == "PUT" && r.url.path() == "/cal/series-1.ics")
                .map(|r| String::from_utf8(r.body).unwrap())
                .next_back()
                .expect("the original was shortened");
            assert!(lines_starting(&body, "RECURRENCE-ID").is_empty(), "the moved occurrence left with the cut:\n{body}");
            let rule = lines_starting(&body, "RRULE").pop().expect("still a series").to_string();
            rule.split(';').find_map(|p| p.strip_prefix("UNTIL=")).expect("an UNTIL").to_string()
        }

        /// "This and following" from an occurrence that was moved on its own
        /// cuts the series at the slot the series put it in, as a Google split
        /// does. Cut where it sits now, the old series would keep it and the
        /// new one would start on it too: the occurrence twice.
        #[tokio::test]
        async fn splitting_from_a_moved_occurrence_cuts_at_its_original_slot() {
            let server = MockServer::start().await;
            accepting(&server).await;
            let (state, ids) = seeded(&server, "split-moved@x.test", MOVED).await;
            let exception = ids[&format!("series-1#{}", ms("2026-11-09T08:00:00Z"))];
            let moved = ms("2026-11-10T08:00:00Z");

            update(&state, exception, "following", moved, retitled(moved, moved + 3_600_000)).await.ok();

            let until = master_until(&server).await;
            assert!(until.as_str() < "20261109T080000Z", "ends before 9 November's slot: {until}");
            assert!(until.as_str() >= "20261102T080000Z", "and keeps 2 November: {until}");
        }

        /// And deleting "this and following" from it removes the slot it was
        /// moved from, rather than ending the series after it and leaving the
        /// moved occurrence standing.
        #[tokio::test]
        async fn deleting_following_from_a_moved_occurrence_cuts_at_its_original_slot() {
            let server = MockServer::start().await;
            accepting(&server).await;
            let (state, ids) = seeded(&server, "delete-following-moved@x.test", MOVED).await;
            let exception = ids[&format!("series-1#{}", ms("2026-11-09T08:00:00Z"))];

            delete(&state, exception, "following", ms("2026-11-10T08:00:00Z")).await.ok();

            let until = master_until(&server).await;
            assert!(until.as_str() < "20261109T080000Z", "ends before 9 November's slot: {until}");
        }

        /// A server that refuses a change (a calendar shared read-only that
        /// still lists write access, say) says so in the words every other
        /// status gets, rather than "Sync failed".
        #[tokio::test]
        async fn a_refused_change_says_the_server_refused_it() {
            let server = MockServer::start().await;
            Mock::given(method("PUT")).respond_with(ResponseTemplate::new(403)).mount(&server).await;
            let (state, ids) = seeded(&server, "refused-put@x.test", &plain("RRULE:FREQ=WEEKLY")).await;
            let at = ms("2026-11-02T08:00:00Z");

            let err = update(&state, ids["series-1"], "all", at, retitled(at, at + 3_600_000)).await.unwrap_err();

            assert_eq!(crate::errors::user_facing(&err), "The server answered 403 Forbidden");
        }

        /// A moved event gets a file name of its own making. A UID is the
        /// author's text and may hold `/`, `..` or `?`, none of which may
        /// steer where the copy lands.
        #[tokio::test]
        async fn a_moved_event_is_filed_under_a_fresh_name_not_its_uid() {
            let server = MockServer::start().await;
            accepting(&server).await;
            Mock::given(method("DELETE")).respond_with(ResponseTemplate::new(204)).mount(&server).await;
            let raw = plain("RRULE:FREQ=WEEKLY").replace("UID:series-1", "UID:../../other/evil?x");
            let (state, _) = seeded(&server, "move-name@x.test", &raw).await;
            let (account, master): (i64, i64) = sqlx::query_as(
                "SELECT c.account_id, e.id FROM events e JOIN calendars c ON c.id = e.calendar_id",
            )
            .fetch_one(&state.pool)
            .await
            .unwrap();
            let target: i64 = sqlx::query_scalar(
                "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
                 VALUES (?1, ?2, 'Home', 'Europe/Sofia', 'owner') RETURNING id",
            )
            .bind(account)
            .bind(format!("{}/home/", server.uri()))
            .fetch_one(&state.pool)
            .await
            .unwrap();

            let _ = move_to(&state, master, target).await;

            let paths = put_paths(&server).await;
            assert_eq!(paths.len(), 1, "{paths:?}");
            let name = paths[0].strip_prefix("/home/").expect("into the destination collection");
            let stem = name.strip_suffix(".ics").expect("an .ics resource");
            assert!(uuid::Uuid::parse_str(stem).is_ok(), "a fresh name, not the UID: {}", paths[0]);
        }
    }
}
