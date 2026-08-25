use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Calendar {
    pub id: String,
    #[serde(default)]
    pub summary: String,
    pub background_color: Option<String>,
    pub time_zone: Option<String>,
    #[serde(default)]
    pub access_role: String,
    #[serde(default)]
    pub primary: bool,
    /// What this calendar fires for an event that says `useDefault: true`.
    ///
    /// Arrives on every `calendarList` entry already — that call carries no
    /// `fields=` mask either — and is empty for a calendar with no defaults
    /// set, which is a real answer rather than a missing one.
    #[serde(default)]
    pub default_reminders: Vec<Reminder>,
}

/// One reminder. `method` is Google's own vocabulary — `popup` or `email` —
/// and `minutes` is how long *before* the event it fires.
///
/// Both fields default rather than being required: a reminder missing one of
/// them is Google sending a shape this app has not seen, and defaulting a
/// single field beats failing the whole event's parse over it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub minutes: i64,
}

/// An event's own reminder settings: either the owning calendar's defaults, or
/// a list of overrides that replaces them entirely.
///
/// The two are alternatives, not additive — Google sends `useDefault: true`
/// with no `overrides`, or `useDefault: false` with the whole override list.
///
/// The whole struct defaults, so an event carrying no `reminders` key at all
/// parses. That is not a hypothetical: a cancelled-event tombstone carries
/// little more than its id, and requiring the key would fail the parse of
/// every deletion an incremental sync delivers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reminders {
    #[serde(default)]
    pub use_default: bool,
    #[serde(default)]
    pub overrides: Vec<Reminder>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDateTime {
    /// Present for timed events, RFC 3339.
    pub date_time: Option<String>,
    /// Present for all-day events, `YYYY-MM-DD`. The `end` date is exclusive.
    pub date: Option<String>,
    pub time_zone: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attendee {
    #[serde(default)]
    pub email: String,
    pub display_name: Option<String>,
    #[serde(default)]
    pub response_status: String,
    #[serde(default)]
    pub optional: bool,
    #[serde(rename = "self", default)]
    pub is_self: bool,
    /// A free-text note the attendee left on their RSVP. Writable, and not
    /// modelled anywhere else in this app — carried through unchanged rather
    /// than dropped, since an RSVP patch replaces Google's whole attendee
    /// array and anything this struct doesn't round-trip is erased for real.
    pub comment: Option<String>,
    /// How many extra guests this attendee is bringing. Also writable and
    /// otherwise unmodelled; same reason as `comment`.
    #[serde(default)]
    pub additional_guests: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Organizer {
    #[serde(default)]
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    /// `confirmed` | `tentative` | `cancelled`. Cancelled rows are tombstones
    /// delivered by incremental sync and carry almost no other fields.
    #[serde(default)]
    pub status: String,
    pub etag: Option<String>,
    #[serde(rename = "iCalUID")]
    pub ical_uid: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    #[serde(default)]
    pub start: EventDateTime,
    #[serde(default)]
    pub end: EventDateTime,
    pub recurrence: Option<Vec<String>>,
    pub recurring_event_id: Option<String>,
    pub original_start_time: Option<EventDateTime>,
    pub hangout_link: Option<String>,
    /// The complete conference object, retained for operations that create a
    /// second event from this one (a “this and following” series split).
    /// `hangout_link` is enough to render Join, but not enough to copy the
    /// conference without losing its entry points and solution metadata.
    #[serde(default)]
    pub conference_data: Option<serde_json::Value>,
    #[serde(default)]
    pub attendees: Vec<Attendee>,
    #[serde(default)]
    pub sequence: i64,
    #[serde(default)]
    pub organizer: Organizer,
    /// Already on the wire. `list_events` sends no `fields=` mask, so Google
    /// has been returning this on every event all along — it was parsed into
    /// nothing and discarded. Defaulted because a tombstone omits it.
    #[serde(default)]
    pub reminders: Reminders,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The full payload shape `events.list` sends, trimmed to the fields these
    /// tests are about. `reminders` is spliced in by the caller so a *missing*
    /// key is expressible, which is the case a tombstone actually sends.
    fn event_json(reminders: Option<serde_json::Value>) -> serde_json::Value {
        let mut ev = serde_json::json!({
            "id": "ev1",
            "status": "confirmed",
            "summary": "Standup",
            "start": {"dateTime": "2026-08-10T09:00:00+03:00", "timeZone": "Europe/Sofia"},
            "end":   {"dateTime": "2026-08-10T09:30:00+03:00", "timeZone": "Europe/Sofia"}
        });
        if let Some(r) = reminders {
            ev["reminders"] = r;
        }
        ev
    }

    fn parse_event(v: serde_json::Value) -> Event {
        serde_json::from_value(v).expect("event should parse")
    }

    #[test]
    fn an_event_keeps_complete_conference_data_for_copying() {
        let mut wire = event_json(None);
        wire["conferenceData"] = serde_json::json!({
            "conferenceId": "abc-defg-hij",
            "conferenceSolution": { "key": { "type": "hangoutsMeet" } },
            "entryPoints": [{
                "entryPointType": "video",
                "uri": "https://meet.google.com/abc-defg-hij"
            }]
        });
        let event = parse_event(wire);
        assert_eq!(
            event.conference_data.as_ref().and_then(|c| c["conferenceId"].as_str()),
            Some("abc-defg-hij")
        );
    }

    /// The override list replaces the calendar's defaults entirely, so every
    /// entry in it has to survive — `method` included. Only `popup` may ever be
    /// fired locally (`email` is Google's own to send), and that decision needs
    /// the method to still be here to make.
    #[test]
    fn an_events_reminder_overrides_keep_their_method_and_minutes() {
        let ev = parse_event(event_json(Some(serde_json::json!({
            "useDefault": false,
            "overrides": [
                {"method": "popup", "minutes": 10},
                {"method": "email", "minutes": 1440}
            ]
        }))));

        assert!(!ev.reminders.use_default);
        assert_eq!(ev.reminders.overrides.len(), 2, "an override was dropped");
        assert_eq!(ev.reminders.overrides[0].method, "popup");
        assert_eq!(ev.reminders.overrides[0].minutes, 10);
        assert_eq!(ev.reminders.overrides[1].method, "email");
        assert_eq!(ev.reminders.overrides[1].minutes, 1440);
    }

    /// The far commoner shape: no overrides at all, defer to the calendar.
    /// Google omits `overrides` entirely here rather than sending an empty
    /// array, so the absent key must read as an empty list.
    #[test]
    fn an_event_deferring_to_the_calendar_says_use_default_with_no_overrides() {
        let ev = parse_event(event_json(Some(serde_json::json!({"useDefault": true}))));
        assert!(ev.reminders.use_default);
        assert!(ev.reminders.overrides.is_empty());
    }

    /// A cancelled-event tombstone carries almost nothing — no start, no end,
    /// and no `reminders` key. Making the key required would fail the parse of
    /// every deletion an incremental sync delivers.
    #[test]
    fn an_event_with_no_reminders_key_at_all_still_parses() {
        let ev: Event = serde_json::from_value(serde_json::json!({
            "id": "ev1", "status": "cancelled"
        }))
        .expect("a tombstone carries no reminders and must still parse");

        assert!(!ev.reminders.use_default);
        assert!(ev.reminders.overrides.is_empty());
    }

    /// What `useDefault: true` above actually resolves to. It arrives on the
    /// `calendarList` entry, not on the event.
    #[test]
    fn a_calendar_carries_its_own_default_reminders() {
        let cal: Calendar = serde_json::from_value(serde_json::json!({
            "id": "primary", "summary": "Work", "accessRole": "owner", "primary": true,
            "defaultReminders": [
                {"method": "popup", "minutes": 30},
                {"method": "email", "minutes": 60}
            ]
        }))
        .expect("calendar should parse");

        assert_eq!(cal.default_reminders.len(), 2, "a default reminder was dropped");
        assert_eq!(cal.default_reminders[0].method, "popup");
        assert_eq!(cal.default_reminders[0].minutes, 30);
        assert_eq!(cal.default_reminders[1].method, "email");
        assert_eq!(cal.default_reminders[1].minutes, 60);
    }

    /// A calendar with no defaults set omits the key rather than sending an
    /// empty array. That is "this calendar fires nothing", not a parse failure.
    #[test]
    fn a_calendar_with_no_default_reminders_parses_as_an_empty_list() {
        let cal: Calendar = serde_json::from_value(serde_json::json!({
            "id": "hols", "summary": "Holidays", "accessRole": "reader"
        }))
        .expect("calendar should parse");
        assert!(cal.default_reminders.is_empty());
    }
}
