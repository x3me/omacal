use jiff::civil::Date;
use jiff::Timestamp;
use omacal_google::model::{Event, EventDateTime};
use omacal_store::StoredEvent;

/// Incremental sync delivers deletions as `status: "cancelled"` rows that carry
/// little more than an id.
pub fn is_tombstone(ev: &Event) -> bool {
    ev.status == "cancelled"
}

/// Maps one wire attendee to its stored shape. `pub` beyond this crate: the
/// RSVP command in `src-tauri` reaches this same mapping a third time, on the
/// conflict-retry path that re-reads an event fresh from Google, and a second
/// hand-written copy of this seven-field struct literal is exactly the kind of
/// duplication that drifts when Google adds a field to one side and not the
/// other. `comment` and `additional_guests` are carried through unchanged even
/// though nothing in this app reads them yet: an RSVP patch replaces Google's
/// whole attendee array, so a field this mapping drops is erased for real.
pub fn from_google_attendee(a: &omacal_google::model::Attendee) -> omacal_store::Attendee {
    omacal_store::Attendee {
        email: a.email.clone(),
        display_name: a.display_name.clone(),
        response_status: a.response_status.clone(),
        optional: a.optional,
        is_self: a.is_self,
        comment: a.comment.clone(),
        additional_guests: a.additional_guests,
    }
}

/// Maps one wire reminder to its stored shape.
///
/// `pub` for the same reason as [`from_google_attendee`]: the calendar-list
/// upsert in `src-tauri` maps `defaultReminders` through this too, and the
/// event path maps `reminders.overrides` through it, so the two halves of the
/// same pair cannot drift apart.
pub fn from_google_reminder(r: &omacal_google::model::Reminder) -> omacal_store::Reminder {
    omacal_store::Reminder { method: r.method.clone(), minutes: r.minutes }
}

/// Maps a list of wire reminders — an event's `overrides`, or a calendar's
/// `defaultReminders`, which are the same shape.
pub fn from_google_reminders(rs: &[omacal_google::model::Reminder]) -> Vec<omacal_store::Reminder> {
    rs.iter().map(from_google_reminder).collect()
}

/// Maps an event's whole `reminders` object.
fn event_reminders(r: &omacal_google::model::Reminders) -> omacal_store::Reminders {
    omacal_store::Reminders {
        use_default: r.use_default,
        overrides: from_google_reminders(&r.overrides),
    }
}

/// Resolves one endpoint to an epoch-millisecond instant.
///
/// Timed events carry RFC 3339 with an offset. All-day events carry a bare
/// date, which must be interpreted in the calendar's zone — midnight in Sofia
/// is not midnight UTC.
fn resolve(dt: &EventDateTime, cal_tz: &str) -> Option<i64> {
    if let Some(s) = &dt.date_time {
        return s.parse::<Timestamp>().ok().map(|t| t.as_millisecond());
    }
    let d = dt.date.as_ref()?;
    let date: Date = d.parse().ok()?;
    let tz = dt.time_zone.as_deref().unwrap_or(cal_tz);
    date.to_datetime(jiff::civil::Time::midnight())
        .in_tz(tz)
        .ok()
        .map(|z| z.timestamp().as_millisecond())
}

/// Converts a wire event into a storable row. Returns `None` for tombstones and
/// for rows whose times cannot be parsed — a malformed event must not abort a
/// whole sync page.
pub fn to_stored(ev: &Event, calendar_id: i64, cal_tz: &str) -> Option<StoredEvent> {
    if is_tombstone(ev) {
        return None;
    }
    let start_utc = resolve(&ev.start, cal_tz)?;
    let end_utc = resolve(&ev.end, cal_tz)?;
    let is_all_day = ev.start.date.is_some();

    Some(StoredEvent {
        id: 0,
        calendar_id,
        google_id: ev.id.clone(),
        summary: ev.summary.clone(),
        location: ev.location.clone(),
        start_utc,
        end_utc,
        start_tz: ev
            .start
            .time_zone
            .clone()
            .unwrap_or_else(|| cal_tz.to_string()),
        // Kept separately from `start_tz`: a flight departs in one zone and
        // lands in another, and collapsing the two loses that.
        end_tz: ev
            .end
            .time_zone
            .clone()
            .or_else(|| ev.start.time_zone.clone())
            .unwrap_or_else(|| cal_tz.to_string()),
        is_all_day,
        recurrence: ev.recurrence.as_ref().map(|r| r.join("\n")),
        recurring_event_id: ev.recurring_event_id.clone(),
        // Resolved through the same helper as start/end, so an all-day
        // exception's original slot lands on the same instant the master's
        // expansion produces for that day rather than on UTC midnight.
        original_start_utc: ev
            .original_start_time
            .as_ref()
            .and_then(|d| resolve(d, cal_tz)),
        status: ev.status.clone(),
        self_response: ev
            .attendees
            .iter()
            .find(|a| a.is_self)
            .map(|a| a.response_status.clone()),
        conference_uri: ev.hangout_link.clone(),
        // Joined in from `calendars` on read; nothing to write here.
        color_hex: None,
        // Also joined in on read, so this write is inert — but `cal_tz` is
        // already on hand, so there is no reason to leave it wrong.
        calendar_timezone: cal_tz.to_string(),
        description: ev.description.clone(),
        etag: ev.etag.clone(),
        sequence: ev.sequence,
        organizer_email: (!ev.organizer.email.is_empty()).then(|| ev.organizer.email.clone()),
        attendees: ev.attendees.iter().map(from_google_attendee).collect(),
        reminders: event_reminders(&ev.reminders),
        // Joined in from `calendars` on read, like `color_hex`; nothing to
        // write here.
        calendar_default_reminders: Vec::new(),
    })
}

/// Builds the row for a cancelled *exception* — a tombstone that carries a
/// `recurringEventId`.
///
/// Deleting one occurrence of a series is delivered as a cancelled event, and
/// deleting the local row for it is exactly wrong: there is no local row (the
/// occurrence only ever existed as an expansion of the master), and the master
/// goes on producing it. The deletion has to be *stored* so the renderer can
/// suppress that slot.
///
/// Returns `None` when the wire event is not an exception, or when it carries no
/// resolvable `originalStartTime` — without that instant there is no occurrence
/// to point at, and the caller should fall back to deleting by id.
pub fn to_cancelled_exception(ev: &Event, calendar_id: i64, cal_tz: &str) -> Option<StoredEvent> {
    let recurring_event_id = ev.recurring_event_id.clone()?;
    let original = ev.original_start_time.as_ref()?;
    let original_start_utc = resolve(original, cal_tz)?;

    // A tombstone carries little more than its id, so the real times are
    // usually absent; the vacated slot is the only instant we can rely on.
    let start_utc = resolve(&ev.start, cal_tz).unwrap_or(original_start_utc);
    let end_utc = resolve(&ev.end, cal_tz).unwrap_or(start_utc);

    Some(StoredEvent {
        id: 0,
        calendar_id,
        google_id: ev.id.clone(),
        summary: ev.summary.clone(),
        location: None,
        start_utc,
        end_utc,
        start_tz: ev
            .start
            .time_zone
            .clone()
            .or_else(|| original.time_zone.clone())
            .unwrap_or_else(|| cal_tz.to_string()),
        end_tz: ev
            .end
            .time_zone
            .clone()
            .or_else(|| ev.start.time_zone.clone())
            .unwrap_or_else(|| cal_tz.to_string()),
        is_all_day: ev.start.date.is_some() || original.date.is_some(),
        recurrence: None,
        recurring_event_id: Some(recurring_event_id),
        original_start_utc: Some(original_start_utc),
        status: ev.status.clone(),
        self_response: None,
        conference_uri: None,
        color_hex: None,
        calendar_timezone: cal_tz.to_string(),
        // A tombstone carries little more than its id, so these are usually
        // absent — mapped the same way as `to_stored` rather than hardcoded,
        // so nothing is silently dropped if Google ever sends more.
        description: ev.description.clone(),
        etag: ev.etag.clone(),
        sequence: ev.sequence,
        organizer_email: (!ev.organizer.email.is_empty()).then(|| ev.organizer.email.clone()),
        attendees: ev.attendees.iter().map(from_google_attendee).collect(),
        // Usually absent on a tombstone, and nothing fires for a cancelled
        // occurrence anyway — mapped rather than hardcoded for the same reason
        // as the four fields above, so nothing is silently dropped if Google
        // ever sends more.
        reminders: event_reminders(&ev.reminders),
        calendar_default_reminders: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use omacal_google::model::{Event, EventDateTime, Organizer};

    fn timed(start: &str, end: &str) -> Event {
        Event {
            id: "e1".into(), status: "confirmed".into(), etag: None, ical_uid: None,
            summary: Some("Standup".into()), description: None, location: Some("Meet".into()),
            start: EventDateTime { date_time: Some(start.into()), date: None,
                                   time_zone: Some("Europe/Sofia".into()) },
            end: EventDateTime { date_time: Some(end.into()), date: None,
                                 time_zone: Some("Europe/Sofia".into()) },
            recurrence: None, recurring_event_id: None, original_start_time: None,
            hangout_link: None, conference_data: None, attendees: vec![], sequence: 0,
            organizer: Organizer::default(),
            reminders: Default::default(),
        }
    }

    /// The mapping used by every call site — `to_stored`, `to_cancelled_exception`,
    /// and the RSVP conflict-retry path in `src-tauri` — must carry every
    /// field, not just the ones `to_stored`'s own tests happen to exercise.
    #[test]
    fn from_google_attendee_maps_every_field() {
        let a = omacal_google::model::Attendee {
            email: "x@y.com".into(),
            display_name: Some("X".into()),
            response_status: "tentative".into(),
            optional: true,
            is_self: true,
            comment: Some("running late".into()),
            additional_guests: 2,
        };
        let s = from_google_attendee(&a);
        assert_eq!(s.email, "x@y.com");
        assert_eq!(s.display_name.as_deref(), Some("X"));
        assert_eq!(s.response_status, "tentative");
        assert!(s.optional);
        assert!(s.is_self);
        assert_eq!(s.comment.as_deref(), Some("running late"), "comment dropped");
        assert_eq!(s.additional_guests, 2, "additional_guests dropped");
    }

    #[test]
    fn a_timed_event_converts_to_utc_millis() {
        let ev = timed("2026-08-03T09:00:00+03:00", "2026-08-03T09:30:00+03:00");
        let s = to_stored(&ev, 1, "Europe/Sofia").unwrap();
        assert_eq!(s.start_utc, 1_785_736_800_000);
        assert_eq!(s.end_utc - s.start_utc, 30 * 60_000);
        assert_eq!(s.start_tz, "Europe/Sofia");
        assert!(!s.is_all_day);
    }

    /// A flight departs in one zone and lands in another. Both must survive.
    #[test]
    fn a_cross_timezone_event_keeps_both_zones() {
        let mut ev = timed("2026-08-09T09:00:00+05:30", "2026-08-09T13:00:00+03:00");
        ev.start.time_zone = Some("Asia/Kolkata".into());
        ev.end.time_zone = Some("Europe/Sofia".into());
        let s = to_stored(&ev, 1, "Europe/Sofia").unwrap();
        assert_eq!(s.start_tz, "Asia/Kolkata");
        assert_eq!(s.end_tz, "Europe/Sofia");
        // 09:00 IST is 03:30Z; 13:00 EEST is 10:00Z.
        assert_eq!(s.end_utc - s.start_utc, 6 * 3_600_000 + 1_800_000);
    }

    #[test]
    fn end_zone_defaults_to_the_start_zone_when_absent() {
        let mut ev = timed("2026-08-03T09:00:00+03:00", "2026-08-03T09:30:00+03:00");
        ev.end.time_zone = None;
        let s = to_stored(&ev, 1, "UTC").unwrap();
        assert_eq!(s.end_tz, "Europe/Sofia");
    }

    #[test]
    fn an_all_day_event_uses_the_calendar_timezone() {
        let mut ev = timed("", "");
        ev.start = EventDateTime { date: Some("2026-08-08".into()), ..Default::default() };
        ev.end = EventDateTime { date: Some("2026-08-09".into()), ..Default::default() };
        let s = to_stored(&ev, 1, "Europe/Sofia").unwrap();
        assert!(s.is_all_day);
        // Google's all-day end date is exclusive; one calendar day must remain
        // exactly one day long.
        assert_eq!(s.end_utc - s.start_utc, 24 * 3_600_000);
    }

    #[test]
    fn a_cancelled_event_is_a_tombstone() {
        let mut ev = timed("2026-08-03T09:00:00+03:00", "2026-08-03T09:30:00+03:00");
        ev.status = "cancelled".into();
        assert!(is_tombstone(&ev));
        assert!(to_stored(&ev, 1, "Europe/Sofia").is_none());
    }

    #[test]
    fn recurrence_lines_are_joined_with_newlines() {
        let mut ev = timed("2026-08-03T09:00:00+03:00", "2026-08-03T09:30:00+03:00");
        ev.recurrence = Some(vec!["RRULE:FREQ=DAILY".into(), "EXDATE:20260804T060000Z".into()]);
        let s = to_stored(&ev, 1, "Europe/Sofia").unwrap();
        assert_eq!(s.recurrence.unwrap(), "RRULE:FREQ=DAILY\nEXDATE:20260804T060000Z");
    }

    #[test]
    fn the_self_attendee_response_is_captured() {
        let mut ev = timed("2026-08-03T09:00:00+03:00", "2026-08-03T09:30:00+03:00");
        ev.attendees = vec![
            omacal_google::model::Attendee {
                email: "other@x".into(), display_name: None,
                response_status: "accepted".into(), optional: false, is_self: false,
                comment: None, additional_guests: 0 },
            omacal_google::model::Attendee {
                email: "me@x".into(), display_name: None,
                response_status: "needsAction".into(), optional: false, is_self: true,
                comment: None, additional_guests: 0 },
        ];
        let s = to_stored(&ev, 1, "Europe/Sofia").unwrap();
        assert_eq!(s.self_response.as_deref(), Some("needsAction"));
    }

    #[test]
    fn an_unparseable_start_is_skipped_rather_than_panicking() {
        let ev = timed("not-a-date", "also-not-a-date");
        assert!(to_stored(&ev, 1, "Europe/Sofia").is_none());
    }

    #[test]
    fn an_exception_keeps_its_master_id_and_original_slot() {
        let mut ev = timed("2026-08-03T14:00:00+03:00", "2026-08-03T14:30:00+03:00");
        ev.id = "master_20260803T060000Z".into();
        ev.recurring_event_id = Some("master".into());
        ev.original_start_time = Some(EventDateTime {
            date_time: Some("2026-08-03T09:00:00+03:00".into()), date: None,
            time_zone: Some("Europe/Sofia".into()),
        });
        let s = to_stored(&ev, 1, "Europe/Sofia").unwrap();
        assert_eq!(s.recurring_event_id.as_deref(), Some("master"));
        // The slot it vacated, not the slot it moved to.
        assert_eq!(s.original_start_utc, Some(1_785_736_800_000));
        assert_eq!(s.start_utc, 1_785_754_800_000);
    }

    /// An all-day `originalStartTime` is a bare date; read in UTC it would land
    /// on the wrong instant for any zone east or west of Greenwich.
    #[test]
    fn an_all_day_original_start_resolves_in_the_calendar_zone() {
        let mut ev = timed("", "");
        ev.start = EventDateTime { date: Some("2026-08-09".into()), ..Default::default() };
        ev.end = EventDateTime { date: Some("2026-08-10".into()), ..Default::default() };
        ev.recurring_event_id = Some("master".into());
        ev.original_start_time =
            Some(EventDateTime { date: Some("2026-08-08".into()), ..Default::default() });
        let s = to_stored(&ev, 1, "Europe/Sofia").unwrap();
        let midnight_sofia = jiff::civil::date(2026, 8, 8)
            .at(0, 0, 0, 0).in_tz("Europe/Sofia").unwrap()
            .timestamp().as_millisecond();
        assert_eq!(s.original_start_utc, Some(midnight_sofia));
    }

    #[test]
    fn an_ordinary_event_has_no_recurrence_link() {
        let ev = timed("2026-08-03T09:00:00+03:00", "2026-08-03T09:30:00+03:00");
        let s = to_stored(&ev, 1, "Europe/Sofia").unwrap();
        assert!(s.recurring_event_id.is_none());
        assert!(s.original_start_utc.is_none());
    }

    /// The shape Google actually sends when one occurrence is deleted: a
    /// cancelled row with a `recurringEventId`, an `originalStartTime`, and no
    /// start or end of its own.
    #[test]
    fn a_cancelled_exception_becomes_a_storable_row() {
        let mut ev = timed("", "");
        ev.id = "master_20260804T060000Z".into();
        ev.status = "cancelled".into();
        ev.summary = None;
        ev.start = EventDateTime::default();
        ev.end = EventDateTime::default();
        ev.recurring_event_id = Some("master".into());
        ev.original_start_time = Some(EventDateTime {
            date_time: Some("2026-08-04T09:00:00+03:00".into()), date: None,
            time_zone: Some("Europe/Sofia".into()),
        });

        let s = to_cancelled_exception(&ev, 1, "Europe/Sofia").unwrap();
        assert_eq!(s.status, "cancelled");
        assert_eq!(s.recurring_event_id.as_deref(), Some("master"));
        assert_eq!(s.original_start_utc, Some(1_785_823_200_000));
        // No times of its own: both fall back to the vacated slot.
        assert_eq!(s.start_utc, 1_785_823_200_000);
        assert_eq!(s.end_utc, 1_785_823_200_000);
    }

    #[test]
    fn a_tombstone_without_a_master_is_not_an_exception() {
        let mut ev = timed("", "");
        ev.status = "cancelled".into();
        ev.start = EventDateTime::default();
        ev.end = EventDateTime::default();
        assert!(to_cancelled_exception(&ev, 1, "Europe/Sofia").is_none());
    }

    /// Without a resolvable original slot there is no occurrence to point at,
    /// so the caller has to fall back to deleting by id.
    #[test]
    fn an_exception_without_an_original_start_is_not_storable() {
        let mut ev = timed("", "");
        ev.status = "cancelled".into();
        ev.start = EventDateTime::default();
        ev.end = EventDateTime::default();
        ev.recurring_event_id = Some("master".into());
        ev.original_start_time = None;
        assert!(to_cancelled_exception(&ev, 1, "Europe/Sofia").is_none());
    }
}
