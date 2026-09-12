//! Exporting one event as an `.ics` file (#113).
//!
//! The inverse of `import.rs`, and deliberately narrower than it: an import
//! reads a file somebody else wrote and has to survive anything, while an
//! export writes one this app authored and can say exactly what it contains.
//!
//! **Through `omacal_caldav`'s serializer, not a second one.** The CalDAV
//! write path already turns an event into a VEVENT, and a file that came out
//! of a different spelling of the same idea would drift from it silently —
//! the mistake `omacal_core::zone` exists to prevent, one layer up. What is
//! here is the mapping from a stored row to an [`EventWrite`], which is the
//! only part that is this feature's own.
//!
//! Split so the whole answer is testable without a file dialog: [`ics_for`]
//! is pure, and the command below it is the dialog and the write.

use omacal_caldav::{EventWrite, WriteTime};
use omacal_store::StoredEvent;

/// What an exported file is called, before the user renames it.
///
/// The title, with everything a path separator or a shell would argue about
/// removed rather than escaped — a suggestion the file chooser will show and
/// the user can overwrite is not the place to be clever. An untitled event
/// still gets a name, because a chooser opening on an empty field is a worse
/// answer than a dull one.
pub(crate) fn file_name(summary: Option<&str>) -> String {
    let stem: String = summary
        .unwrap_or("")
        .chars()
        .map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { ' ' } else { c })
        .collect();
    let stem = stem.trim();
    // A leading dot would hide the file on every Unix; a chooser that appears
    // to have saved nothing is the report this avoids.
    let stem = stem.trim_start_matches('.').trim();
    if stem.is_empty() { "event.ics".to_string() } else { format!("{stem}.ics") }
}

/// The `.ics` for **one occurrence**, as a standalone one-off event.
///
/// The occurrence rather than the series, because that is what the details
/// card was showing when Export was pressed and what "export this event"
/// means to the person who asked (#113 quotes GNOME Calendar, which does the
/// same). A recipient wants the meeting they were told about, not a rule.
///
/// **An occurrence of a series is given a fresh UID.** Keeping the master's
/// would hand another client a single VEVENT claiming to be the series, and
/// the honest reading of that file is "the whole series is now this one
/// event" — a data loss disguised as an import. A non-recurring event keeps
/// its own id: it *is* that event, and a client that already holds it should
/// recognise it rather than acquire a twin.
pub(crate) fn ics_for(
    event: &StoredEvent,
    start_ms: i64,
    end_ms: i64,
    now: jiff::Timestamp,
) -> anyhow::Result<String> {
    let (start, end) = endpoints(event, start_ms, end_ms)?;
    let ev = EventWrite {
        uid: if event.recurrence.is_some() || event.recurring_event_id.is_some() {
            uuid::Uuid::new_v4().to_string()
        } else {
            event.google_id.clone()
        },
        summary: event.summary.clone(),
        location: event.location.clone(),
        description: event.description.clone(),
        // No guest list, as before attendees were writable. `AttendeeWrite`
        // carries no `PARTSTAT` by design, so an export through it would name
        // everybody and answer for nobody — worse than the honest omission,
        // and an exported file is a thing other people's calendars read.
        attendees: None,
        start,
        end,
        // Empty: a single occurrence does not repeat. See the UID note above
        // — the two decisions are the same decision.
        recurrence: Vec::new(),
        recurrence_id: None,
        // Reminders are the exporter's own, not the recipient's business:
        // "remind me 10 minutes before" is a fact about this user's client.
        alarms: Vec::new(),
        sequence: 0,
        conference: event.conference_uri.clone(),
    };
    Ok(omacal_caldav::new_event_ics(&ev, now))
}

/// The occurrence's endpoints in the shape the serializer writes.
///
/// All-day events take `VALUE=DATE`, read in the **calendar's** zone — the
/// store holds midnight there for them, which is the convention
/// `omacal_core::zone` documents and issue #44 was a violation of. A timed
/// event keeps the zone it was authored in, the same rule
/// `caldav_write::endpoints` follows.
fn endpoints(
    event: &StoredEvent,
    start_ms: i64,
    end_ms: i64,
) -> anyhow::Result<(WriteTime, WriteTime)> {
    if event.is_all_day {
        let tz = &event.start_tz;
        let date = |ms: i64| -> anyhow::Result<jiff::civil::Date> {
            let zone = jiff::tz::TimeZone::get(tz).unwrap_or(jiff::tz::TimeZone::UTC);
            Ok(jiff::Timestamp::from_millisecond(ms)?.to_zoned(zone).date())
        };
        return Ok((WriteTime::Date(date(start_ms)?), WriteTime::Date(date(end_ms)?)));
    }
    let zone = jiff::tz::TimeZone::get(&event.start_tz)
        .map_err(|_| anyhow::anyhow!("unknown time zone {}", event.start_tz))?;
    let name = zone.iana_name().unwrap_or("UTC").to_string();
    let civil = |ms: i64| -> anyhow::Result<jiff::civil::DateTime> {
        Ok(jiff::Timestamp::from_millisecond(ms)?.to_zoned(zone.clone()).datetime())
    };
    Ok((
        WriteTime::Zoned { dt: civil(start_ms)?, tzid: name.clone() },
        WriteTime::Zoned { dt: civil(end_ms)?, tzid: name },
    ))
}

/// Saves one occurrence as an `.ics` the user names.
///
/// Returns the path written, or `None` when the chooser was dismissed —
/// a cancel is an answer, not a failure, and the UI says nothing for it.
///
/// **The dialog is the platform's, through `tauri-plugin-dialog`.** An
/// "Export" that silently picks a directory is the kind of control people
/// then have to go looking for the output of; the chooser is what makes the
/// destination the user's. Everything worth testing happens above it in
/// [`ics_for`], which is why this function has no test of its own — it is a
/// file dialog and a `write`, and neither is ours.
#[tauri::command]
pub(crate) async fn export_event(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    id: i64,
    start_ms: i64,
    end_ms: i64,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (event, _, _) = omacal_store::event_by_id(&state.pool, id)
        .await
        .map_err(|e| crate::errors::user_facing(&e))?
        .ok_or_else(|| EXPORT_GONE.to_string())?;

    let now = jiff::Timestamp::from_millisecond(crate::now_ms())
        .map_err(|e| crate::errors::user_facing(&anyhow::Error::from(e)))?;
    let ics = ics_for(&event, start_ms, end_ms, now).map_err(|e| crate::errors::user_facing(&e))?;
    let suggested = file_name(event.summary.as_deref());

    // The chooser answers on its own thread; a channel is how a callback API
    // becomes an awaited one without blocking the runtime.
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&suggested)
        .add_filter("Calendar file", &["ics"])
        .save_file(move |chosen| {
            let _ = tx.send(chosen);
        });
    let Some(path) = rx.await.map_err(|_| EXPORT_DISMISSED.to_string())? else {
        return Ok(None);
    };
    let path = path
        .into_path()
        .map_err(|e| crate::errors::user_facing(&anyhow::anyhow!("{e}")))?;

    std::fs::write(&path, ics).map_err(|e| {
        tracing::warn!(%e, "export: could not write the file");
        EXPORT_FAILED.to_string()
    })?;
    tracing::info!(?path, "export: wrote an event");
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// The chooser went away without answering — a shape that should not happen,
/// and one the user can act on if it does.
pub(crate) const EXPORT_DISMISSED: &str = "The file chooser closed without an answer.";

/// The row went out from under the click — a sync landed between opening the
/// details card and pressing Export.
pub(crate) const EXPORT_GONE: &str = "That event is no longer here.";

/// A fixed literal, in `errors::SAFE_EXACT`: a failed write must not put a
/// filesystem path or an OS error into a message the user reads.
pub(crate) const EXPORT_FAILED: &str =
    "Could not write that file. Check the folder and try again.";

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_786_352_400_000;
    fn now() -> jiff::Timestamp {
        jiff::Timestamp::from_millisecond(NOW).unwrap()
    }

    fn stored() -> StoredEvent {
        StoredEvent {
            id: 1,
            calendar_id: 1,
            google_id: "abc123".into(),
            summary: Some("Design sync".into()),
            location: Some("Room 2".into()),
            start_utc: 1_786_352_400_000,
            end_utc: 1_786_356_000_000,
            start_tz: "Europe/Sofia".into(),
            end_tz: "Europe/Sofia".into(),
            is_all_day: false,
            recurrence: None,
            recurring_event_id: None,
            original_start_utc: None,
            status: "confirmed".into(),
            self_response: None,
            conference_uri: None,
            color_hex: None,
            calendar_timezone: "Europe/Sofia".into(),
            description: Some("Bring the notes".into()),
            etag: None,
            sequence: 0,
            organizer_email: None,
            guests_can_modify: false,
            attendees: Vec::new(),
            reminders: Default::default(),
            calendar_default_reminders: Default::default(),
        }
    }

    #[test]
    fn a_one_off_exports_as_itself() {
        let out = ics_for(&stored(), 1_786_352_400_000, 1_786_356_000_000, now()).unwrap();
        assert!(out.contains("BEGIN:VEVENT"), "{out}");
        assert!(out.contains("UID:abc123"), "a one-off keeps its own id: {out}");
        assert!(out.contains("SUMMARY:Design sync"), "{out}");
        assert!(out.contains("LOCATION:Room 2"), "{out}");
        assert!(out.contains("DTSTART;TZID=Europe/Sofia:20260810T120000"), "{out}");
        assert!(!out.contains("RRULE"), "a one-off carries no rule: {out}");
    }

    /// The occurrence, not the series — and not the series' identity either.
    /// A single VEVENT wearing the master's UID reads to an importing client
    /// as "the series is now this one event".
    #[test]
    fn an_occurrence_of_a_series_never_carries_the_series_uid() {
        let mut ev = stored();
        ev.recurrence = Some("RRULE:FREQ=DAILY".into());
        let out = ics_for(&ev, 1_786_438_800_000, 1_786_442_400_000, now()).unwrap();
        assert!(!out.contains("UID:abc123"), "the master's id must not travel: {out}");
        assert!(!out.contains("RRULE"), "{out}");
        // The occurrence's own day, a day after the master's.
        assert!(out.contains("DTSTART;TZID=Europe/Sofia:20260811T120000"), "{out}");
    }

    /// The zone the event was authored in, not the exporting machine's — the
    /// same rule the CalDAV writer keeps, and the one #44 was a breach of on
    /// the reading side.
    #[test]
    fn an_all_day_event_takes_its_date_in_the_calendars_zone() {
        let mut ev = stored();
        ev.is_all_day = true;
        ev.start_tz = "Australia/Brisbane".into();
        ev.end_tz = "Australia/Brisbane".into();
        // 2026-08-12 00:00 Brisbane — 14:00 the previous day in UTC.
        let out = ics_for(&ev, 1_786_456_800_000, 1_786_543_200_000, now()).unwrap();
        assert!(out.contains("DTSTART;VALUE=DATE:20260812"), "{out}");
        assert!(out.contains("DTEND;VALUE=DATE:20260813"), "{out}");
    }

    /// Dropping the join link would make the exported meeting unjoinable,
    /// which is the one thing the recipient opens it for.
    #[test]
    fn a_video_call_link_travels_with_the_event() {
        let mut ev = stored();
        ev.conference_uri = Some("https://meet.google.com/abc-defg-hij".into());
        let out = ics_for(&ev, 1_786_352_400_000, 1_786_356_000_000, now()).unwrap();
        assert!(
            out.contains("CONFERENCE;VALUE=URI;FEATURE=VIDEO:https://meet.google.com/abc-defg-hij"),
            "{out}",
        );
    }

    #[test]
    fn a_file_name_is_suggested_from_the_title_and_never_empty() {
        assert_eq!(file_name(Some("Design sync")), "Design sync.ics");
        assert_eq!(file_name(Some("Q3/Q4 planning")), "Q3 Q4 planning.ics");
        assert_eq!(file_name(Some("   ")), "event.ics");
        assert_eq!(file_name(None), "event.ics");
        // A leading dot hides the file on every Unix, which reads as a save
        // that did nothing.
        assert_eq!(file_name(Some(".hidden")), "hidden.ics");
    }
}
