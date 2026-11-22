//! Importing events from an `.ics` file (#67).
//!
//! **Planning is separate from doing, and the plan is the product.** An
//! import is the one operation here that writes many events at once, from a
//! file the app did not write, so the thing that matters is knowing what it
//! will do before it does it: this module reads a file and answers with one
//! [`Planned`] per VEVENT — imported with these fields, or skipped with this
//! reason. The surfaces show that plan (`--dry-run`, and the dialog's
//! preview) and then hand the same plan back to be executed.
//!
//! Three rules decide what is skipped, and all three are the same rule:
//! **never silently alter what the file said.**
//!
//! - A **repeat rule the app cannot express** is not downgraded to a single
//!   event. `write::EventInput` carries the form's vocabulary — daily,
//!   weekdays, weekly on chosen days, monthly, yearly, ending never, on a
//!   date or after N times — rather than an RRULE, deliberately (see its doc
//!   comment), so a rule outside that set (an INTERVAL, a monthly BYDAY, an
//!   EXDATE) has no honest representation here. Importing the first
//!   occurrence and calling it the series would be a lie about the user's
//!   data.
//! - **Guests are never imported.** A create carries its guest list to the
//!   server, and a server invites the people on it. Importing a colleague's
//!   exported year would email everyone they ever met, about meetings that
//!   already happened. The events come in; the guest lists stay out, and the
//!   plan says so per event.
//! - An event whose **UID is already in the database** is skipped, so the
//!   same file imported twice does not double anything.
//!
//! Everything the plan does import goes through `events::create_event_body`,
//! the path the window and the CLI already write through. No second write
//! path, and no raw resource PUT: a lossless CalDAV import would be one, and
//! it would be the only way into the database that none of this repo's
//! guards cover.

use omacal_caldav::ics;
use sqlx::Row;

/// What an import will do with one VEVENT.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Planned {
    /// Will be created, with these fields.
    Import {
        summary: String,
        /// When it starts, as the row will read it: ms epoch.
        start_ms: i64,
        all_day: bool,
        /// For an all-day event, its date (`yyyy-mm-dd`) as the file states
        /// it, which the preview names. `None` when timed.
        date: Option<String>,
        /// The repeat as the preview words it ("weekly on Mon, Wed, 10
        /// times"), when the file carried one at all: the whole rule, end
        /// included, so the preview shows what will be stored.
        repeat: Option<String>,
        /// How many guests the file listed and this import will not carry.
        /// Zero for an ordinary event; the count is what the preview shows,
        /// so the omission is stated rather than discovered.
        dropped_guests: usize,
    },
    /// Will not be created, and why — in the words the surfaces print.
    Skip { summary: String, reason: String },
}

impl Planned {
    pub fn summary(&self) -> &str {
        match self {
            Planned::Import { summary, .. } | Planned::Skip { summary, .. } => summary,
        }
    }
}

/// The reasons a plan gives, as constants so a surface and a test cannot
/// disagree about the wording.
pub const SKIP_ALREADY_HERE: &str = "already in this calendar";
pub const SKIP_UNEXPRESSIBLE_REPEAT: &str =
    "repeats in a way this version cannot store; import it from the app that wrote it";
pub const SKIP_NO_START: &str = "no usable start time";
pub const SKIP_AN_OCCURRENCE: &str = "an exception to a series, which cannot be imported on its own";

/// The form's recurrence controls for an event's recurrence lines, when the
/// form can hold the rule **exactly**: `Some(None)` for no rule at all,
/// `Some(Some(_))` for a rule the app can store and re-emit with the same
/// occurrences, and `None` for anything else, which the plan skips.
///
/// The grammar is the form's own reader's (`write::recurrence_controls_from_
/// rrule`): one of its cadences, weekly days, and one COUNT or UNTIL, parts
/// in any order. Until #181 this was a five-entry table of bare cadences, so
/// a file with "ends after 10 times" or "Mondays and Wednesdays" was skipped
/// although the form makes exactly those rules.
///
/// **An UNTIL is moved, never stretched.** The form writes "ends on D" as the
/// end of day D in the calendar's zone, while a file's UNTIL can stop earlier
/// that day, before D's occurrence (Google writes `T235959Z`, which in Sofia
/// is 01:59 the next morning). Taken as it reads, that would add an
/// occurrence the file did not have. So when D's occurrence falls after the
/// file's UNTIL, the end becomes D's eve, which keeps every occurrence the
/// file had and adds none. `start` is the event's own start, as the plan
/// resolved it; a rule that needs it and has none is skipped.
fn recurrence_for(
    lines: &[String],
    start: Option<(i64, bool)>,
    cal_tz: &str,
) -> Option<Option<crate::write::RecurrenceControls>> {
    use crate::write::RepeatEnd;
    // An event with EXDATE or RDATE has a rule the form cannot carry either,
    // whatever its RRULE says.
    if lines.iter().any(|l| {
        let name = l.split([':', ';']).next().unwrap_or("").to_ascii_uppercase();
        name == "EXDATE" || name == "RDATE"
    }) {
        return None;
    }
    let mut rules = lines.iter().filter(|l| {
        l.split([':', ';']).next().unwrap_or("").eq_ignore_ascii_case("RRULE")
    });
    let Some(rule) = rules.next() else {
        return Some(None); // no rule at all: a single event, importable
    };
    if rules.next().is_some() {
        return None; // two rules is not a shape the form has
    }
    let body = rule.split_once(':').map(|(_, v)| v).unwrap_or("").to_ascii_uppercase().replace(' ', "");
    let (start_ms, all_day) = start?;
    let mut controls =
        crate::write::recurrence_controls_from_rrule(Some(&format!("RRULE:{body}")), all_day, cal_tz);
    if controls.repeat == "custom" || controls.repeat == "never" {
        return None;
    }
    if let (RepeatEnd::On { date }, false) = (&controls.repeat_end, all_day) {
        let tz = jiff::tz::TimeZone::get(cal_tz).ok()?;
        let until = body
            .split(';')
            .find_map(|p| p.strip_prefix("UNTIL="))
            .and_then(crate::write::basic_utc_timestamp)?;
        let start = jiff::Timestamp::from_millisecond(start_ms).ok()?.to_zoned(tz.clone());
        let mut last: jiff::civil::Date = date.parse().ok()?;
        let on_last = last.to_datetime(start.time()).to_zoned(tz).ok()?;
        if on_last.timestamp() > until {
            last = last.yesterday().ok()?;
        }
        if last < start.date() {
            return None; // a rule that ends before it starts is not a series
        }
        controls.repeat_end = RepeatEnd::On { date: last.to_string() };
    }
    Some(Some(controls))
}

/// The preview's words for a rule [`recurrence_for`] accepted: the cadence,
/// the days a weekly rule names, and how it ends.
fn repeat_words(c: &crate::write::RecurrenceControls) -> String {
    use crate::write::RepeatEnd;
    let mut words = c.repeat.clone();
    if c.repeat == "weekly" && !c.weekly_days.is_empty() {
        let names: Vec<&str> = c
            .weekly_days
            .iter()
            .map(|d| match d.as_str() {
                "MO" => "Mon", "TU" => "Tue", "WE" => "Wed", "TH" => "Thu",
                "FR" => "Fri", "SA" => "Sat", "SU" => "Sun", other => other,
            })
            .collect();
        words.push_str(" on ");
        words.push_str(&names.join(", "));
    }
    match &c.repeat_end {
        RepeatEnd::Never => {}
        RepeatEnd::After { count } => words.push_str(&format!(", {count} times")),
        RepeatEnd::On { date } => words.push_str(&format!(", until {date}")),
    }
    words
}

/// Plans an import of `src` into a calendar whose zone is `cal_tz`, given
/// the UIDs that calendar already holds.
///
/// A file that is not iCalendar at all answers `None`; a file with no
/// events answers an empty plan, which the surfaces report as such rather
/// than as a failure.
pub fn plan(src: &str, cal_tz: &str, existing_uids: &[String]) -> Option<Vec<Planned>> {
    let root = ics::parse(src)?;
    let events = ics::events_in(&root);
    Some(
        events
            .into_iter()
            .map(|ev| {
                let summary = ev.summary.clone().unwrap_or_else(|| "(no title)".into());
                if ev.recurrence_id.is_some() {
                    return Planned::Skip { summary, reason: SKIP_AN_OCCURRENCE.into() };
                }
                if existing_uids.contains(&ev.uid) {
                    return Planned::Skip { summary, reason: SKIP_ALREADY_HERE.into() };
                }
                let start = ics::resolve(&ev.start, cal_tz);
                let Some(repeat) =
                    recurrence_for(&ev.recurrence, start.as_ref().map(|(ms, _, all_day)| (*ms, *all_day)), cal_tz)
                else {
                    return Planned::Skip { summary, reason: SKIP_UNEXPRESSIBLE_REPEAT.into() };
                };
                let Some((start_ms, _tz, all_day)) = start else {
                    return Planned::Skip { summary, reason: SKIP_NO_START.into() };
                };
                Planned::Import {
                    summary,
                    start_ms,
                    all_day,
                    date: if all_day { all_day_date(&ev.start) } else { None },
                    repeat: repeat.as_ref().map(repeat_words),
                    dropped_guests: ev.attendees.len(),
                }
            })
            .collect(),
    )
}

/// One planned event, as the create path takes it.
///
/// Built only for an event the plan already accepted, so every refusal has
/// happened before this point: this function maps, it does not judge.
/// Guests are absent by construction rather than by omission — see the
/// module doc for why importing them would email people.
pub(crate) fn input_for(ev: &ics::CalEvent, cal_tz: &str) -> Option<crate::write::EventInput> {
    let (start_ms, _tz, all_day) = ics::resolve(&ev.start, cal_tz)?;
    // The same reading the plan made, so what is written is what was shown.
    let controls = recurrence_for(&ev.recurrence, Some((start_ms, all_day)), cal_tz)?;
    let when = if all_day {
        // The dates as the form holds them: an inclusive start and the
        // exclusive end iCalendar already uses. A missing DTEND is the
        // RFC's one day.
        let start_date = all_day_date(&ev.start)?;
        let end_date = ev
            .end
            .as_ref()
            .and_then(all_day_date)
            .or_else(|| next_day(&start_date))?;
        crate::write::WhenInput::AllDay { start_date, end_date }
    } else {
        // DTEND, else DTSTART plus DURATION, else the RFC's instant event.
        let end_ms = ev
            .end
            .as_ref()
            .and_then(|e| ics::resolve(e, cal_tz))
            .map(|(ms, _, _)| ms)
            .or_else(|| ev.duration_ms.map(|d| start_ms + d))
            .unwrap_or(start_ms);
        crate::write::WhenInput::Timed { start_ms, end_ms }
    };
    Some(crate::write::EventInput {
        summary: ev.summary.clone(),
        location: ev.location.clone(),
        description: ev.description.clone(),
        when,
        tz: cal_tz.to_string(),
        repeat: controls.as_ref().map(|c| c.repeat.clone()),
        // Only a weekly rule names its days; "weekdays" is a cadence of its own.
        weekly_days: controls
            .as_ref()
            .filter(|c| c.repeat == "weekly" && !c.weekly_days.is_empty())
            .map(|c| c.weekly_days.iter().filter_map(|d| crate::write::WeekdayCode::from_text(d)).collect()),
        repeat_end: controls.as_ref().map(|c| c.repeat_end.clone()),
        // Never. The module doc has the whole of the reason.
        guests: None,
        reminders: reminders_for(ev),
        conference: None,
        // Not carried, by decision for now: an imported event takes the
        // server's defaults (busy, no class). Reading the file's `TRANSP` and
        // `CLASS` here is a possible follow-up, not a missing line.
        transparency: None,
        visibility: None,
    })
}

/// The popup reminders an event carries, or `None` to leave the calendar's
/// own defaults alone.
///
/// **Popup only.** A VALARM with `EMAIL` asks a server to send mail, and an
/// import is the last place to start doing that on somebody's behalf; those
/// alarms are dropped with the guests. A trigger after the start (a positive
/// offset here) is not something the form can hold, so it is dropped too
/// rather than flipped into a reminder before.
fn reminders_for(ev: &ics::CalEvent) -> Option<crate::write::RemindersInput> {
    let overrides: Vec<crate::write::ReminderInput> = ev
        .alarms
        .iter()
        .filter(|(method, minutes)| method == "popup" && *minutes >= 0)
        .map(|(_, minutes)| crate::write::ReminderInput {
            method: "popup".into(),
            minutes: *minutes,
        })
        .collect();
    if overrides.is_empty() {
        None
    } else {
        Some(crate::write::RemindersInput { use_default: false, overrides })
    }
}

/// `YYYY-MM-DD` for a `VALUE=DATE` property, and nothing for any other
/// shape — an all-day event whose DTEND is a date-time is not one this maps.
fn all_day_date(t: &ics::IcsTime) -> Option<String> {
    match t {
        ics::IcsTime::Date(d) => Some(d.to_string()),
        _ => None,
    }
}

/// The day after `date`, for an all-day event with no DTEND: the RFC says
/// one day, and the form's end is exclusive.
fn next_day(date: &str) -> Option<String> {
    let d: jiff::civil::Date = date.parse().ok()?;
    Some(d.tomorrow().ok()?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n\
        BEGIN:VEVENT\r\nUID:a@x\r\nSUMMARY:Lunch\r\nDTSTART:20260907T113000Z\r\n\
        DTEND:20260907T123000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    fn ics_with(body: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{body}END:VCALENDAR\r\n")
    }

    #[test]
    fn a_plain_event_is_planned_for_import() {
        let plan = plan(ONE, "UTC", &[]).unwrap();
        assert_eq!(plan.len(), 1);
        // The instant DTSTART names, derived rather than written out: a
        // magic number here would be checking my arithmetic, not the parse.
        let expected: jiff::Timestamp = "2026-09-07T11:30:00Z".parse().unwrap();
        assert_eq!(
            plan[0],
            Planned::Import {
                summary: "Lunch".into(),
                start_ms: expected.as_millisecond(),
                all_day: false,
                date: None,
                repeat: None,
                dropped_guests: 0,
            }
        );
    }

    /// The rule as an event in a file, and what the form will be told.
    fn controls(rule: &str, start: &str, cal_tz: &str) -> Option<Option<crate::write::RecurrenceControls>> {
        let start: jiff::Timestamp = start.parse().unwrap();
        recurrence_for(&[format!("RRULE:{rule}")], Some((start.as_millisecond(), false)), cal_tz)
    }

    /// **What the form can make, import takes** (#181), and nothing else.
    /// The cadences, a weekly rule's days and one COUNT or UNTIL come through,
    /// parts in any order; anything the form cannot make is still skipped
    /// rather than quietly imported as something simpler, which would be
    /// this module telling the user something their file did not say.
    #[test]
    fn the_repeats_the_form_can_make_are_carried_and_the_rest_are_refused() {
        use crate::write::RepeatEnd;
        let at = "2026-09-07T09:00:00Z"; // a Monday
        for (rule, repeat, days, end, words) in [
            ("FREQ=DAILY", "daily", vec![], RepeatEnd::Never, "daily"),
            ("FREQ=WEEKLY", "weekly", vec![], RepeatEnd::Never, "weekly"),
            ("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", "weekdays", vec!["MO", "TU", "WE", "TH", "FR"], RepeatEnd::Never, "weekdays"),
            ("FREQ=MONTHLY", "monthly", vec![], RepeatEnd::Never, "monthly"),
            ("FREQ=YEARLY", "yearly", vec![], RepeatEnd::Never, "yearly"),
            ("FREQ=DAILY;COUNT=10", "daily", vec![], RepeatEnd::After { count: 10 }, "daily, 10 times"),
            ("BYDAY=WE,MO;FREQ=WEEKLY", "weekly", vec!["MO", "WE"], RepeatEnd::Never, "weekly on Mon, Wed"),
            ("FREQ=WEEKLY;BYDAY=MO,WE;COUNT=6", "weekly", vec!["MO", "WE"], RepeatEnd::After { count: 6 }, "weekly on Mon, Wed, 6 times"),
            ("freq=monthly;until=20261231T235959Z", "monthly", vec![],
             RepeatEnd::On { date: "2026-12-31".into() }, "monthly, until 2026-12-31"),
        ] {
            let c = controls(rule, at, "UTC").unwrap_or_else(|| panic!("{rule} refused")).expect("a rule");
            assert_eq!(c.repeat, repeat, "{rule}");
            assert_eq!(c.weekly_days, days.iter().map(|d| d.to_string()).collect::<Vec<_>>(), "{rule}");
            assert_eq!(c.repeat_end, end, "{rule}");
            assert_eq!(repeat_words(&c), words, "{rule}");
        }
        for rule in [
            "FREQ=WEEKLY;INTERVAL=2",
            "FREQ=MONTHLY;BYDAY=3TH",
            "FREQ=HOURLY",
            "FREQ=DAILY;COUNT=10;UNTIL=20261231T000000Z",
            "FREQ=DAILY;COUNT=0",
            "FREQ=DAILY;BYMONTH=1",
        ] {
            assert_eq!(controls(rule, at, "UTC"), None, "{rule}");
        }
        let start = Some(("2026-09-07T09:00:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond(), false));
        assert_eq!(
            recurrence_for(&["RRULE:FREQ=DAILY".into(), "EXDATE:20260909T090000Z".into()], start, "UTC"),
            None,
            "an exception list is part of the rule",
        );
        assert_eq!(
            recurrence_for(&["RRULE:FREQ=DAILY".into(), "RRULE:FREQ=WEEKLY".into()], start, "UTC"),
            None,
            "two rules are not a shape the form has",
        );
        assert_eq!(recurrence_for(&[], start, "UTC"), Some(None), "no rule is a single event");
        assert_eq!(recurrence_for(&["RRULE:FREQ=DAILY".into()], None, "UTC"), None, "a rule with no start");
    }

    /// **An UNTIL is moved, never stretched.** The form ends a series at the
    /// end of a day in the calendar's zone; a file's UNTIL can stop earlier
    /// that day, before the day's occurrence. Taken as it reads, the series
    /// would gain one. Each case checks the rule the form will actually
    /// write, through `fields_from_input`, not just the date.
    #[test]
    fn an_until_keeps_exactly_the_occurrences_the_file_had() {
        use crate::write::RepeatEnd;
        let written = |rule: &str, start: &str, cal_tz: &str| {
            let src = ics_with(&format!(
                "BEGIN:VEVENT\r\nUID:u@x\r\nSUMMARY:Standup\r\nDTSTART:{start}\r\n\
                 DTEND:{start}\r\nRRULE:{rule}\r\nEND:VEVENT\r\n"
            ));
            let input = input_for(&only_event(&src), cal_tz).expect("imported");
            let end = input.repeat_end.clone();
            let fields = crate::write::fields_from_input(input).expect("a rule the form writes");
            (end, fields.recurrence.flatten().expect("a rule"))
        };

        // Ends after the last day's 09:00: the date stands.
        let (end, rule) = written("FREQ=DAILY;UNTIL=20261231T235959Z", "20260907T090000Z", "UTC");
        assert_eq!(end, Some(RepeatEnd::On { date: "2026-12-31".into() }));
        assert!(rule.contains("UNTIL=20261231T235959Z"), "{rule}");

        // Ends at 08:00 on the 31st, before that day's 09:00: the 31st was
        // never an occurrence, so the series ends on the 30th.
        let (end, rule) = written("FREQ=DAILY;UNTIL=20261231T080000Z", "20260907T090000Z", "UTC");
        assert_eq!(end, Some(RepeatEnd::On { date: "2026-12-30".into() }));
        assert!(rule.contains("UNTIL=20261230T235959Z"), "{rule}");

        // Google's end-of-day UTC, read in Sofia: 01:59 on 1 January there,
        // before that morning's 10:00. Read as written it would end on the
        // 1st and add an occurrence; it ends on the 31st.
        let (end, rule) = written("FREQ=DAILY;UNTIL=20261231T235959Z", "20260907T070000Z", "Europe/Sofia");
        assert_eq!(end, Some(RepeatEnd::On { date: "2026-12-31".into() }));
        assert!(rule.contains("UNTIL=20261231T215959Z"), "end of the 31st in Sofia: {rule}");

        // Weekly on chosen days with a count is carried whole.
        let (end, rule) = written("FREQ=WEEKLY;BYDAY=MO,WE;COUNT=6", "20260907T090000Z", "UTC");
        assert_eq!(end, Some(RepeatEnd::After { count: 6 }));
        assert_eq!(rule, "RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=6");
    }

    #[test]
    fn an_unstorable_repeat_is_skipped_by_name() {
        let src = ics_with(
            "BEGIN:VEVENT\r\nUID:b@x\r\nSUMMARY:Third Thursday\r\n\
             DTSTART:20260903T090000Z\r\nRRULE:FREQ=MONTHLY;BYDAY=3TH\r\nEND:VEVENT\r\n",
        );
        let plan = plan(&src, "UTC", &[]).unwrap();
        assert_eq!(
            plan[0],
            Planned::Skip {
                summary: "Third Thursday".into(),
                reason: SKIP_UNEXPRESSIBLE_REPEAT.into(),
            }
        );
    }

    /// Guests are counted and left behind, and the count is in the plan so
    /// the surfaces can say so before anything is written. Importing them
    /// would invite the people on them.
    #[test]
    fn guests_are_counted_and_not_carried() {
        let src = ics_with(
            "BEGIN:VEVENT\r\nUID:c@x\r\nSUMMARY:Retro\r\nDTSTART:20260907T090000Z\r\n\
             ATTENDEE;CN=A:mailto:a@x.com\r\nATTENDEE;CN=B:mailto:b@x.com\r\nEND:VEVENT\r\n",
        );
        let plan = plan(&src, "UTC", &[]).unwrap();
        let Planned::Import { dropped_guests, .. } = &plan[0] else { panic!("{:?}", plan[0]) };
        assert_eq!(*dropped_guests, 2);
    }

    /// The same file twice does not double anything.
    #[test]
    fn an_event_already_here_is_skipped() {
        let plan = plan(ONE, "UTC", &["a@x".to_string()]).unwrap();
        assert_eq!(
            plan[0],
            Planned::Skip { summary: "Lunch".into(), reason: SKIP_ALREADY_HERE.into() }
        );
    }

    /// One occurrence of a series, exported on its own, has no series here
    /// to attach to.
    #[test]
    fn a_lone_exception_is_skipped() {
        let src = ics_with(
            "BEGIN:VEVENT\r\nUID:d@x\r\nSUMMARY:Moved standup\r\n\
             RECURRENCE-ID:20260908T090000Z\r\nDTSTART:20260908T100000Z\r\nEND:VEVENT\r\n",
        );
        let plan = plan(&src, "UTC", &[]).unwrap();
        assert_eq!(plan[0], Planned::Skip {
            summary: "Moved standup".into(),
            reason: SKIP_AN_OCCURRENCE.into(),
        });
    }

    /// An all-day event keeps its all-day shape, and a titleless one is
    /// still named in the plan rather than showing as a blank row.
    #[test]
    fn all_day_and_untitled_events_still_plan() {
        let src = ics_with(
            "BEGIN:VEVENT\r\nUID:e@x\r\nDTSTART;VALUE=DATE:20260910\r\n\
             DTEND;VALUE=DATE:20260911\r\nEND:VEVENT\r\n",
        );
        let plan = plan(&src, "UTC", &[]).unwrap();
        let Planned::Import { all_day, summary, date, .. } = &plan[0] else { panic!() };
        assert!(all_day);
        assert_eq!(summary, "(no title)");
        // The file's own day, which the preview names: read from the instant
        // in the webview's zone, a calendar ahead of it showed the day before.
        assert_eq!(date.as_deref(), Some("2026-09-10"));
    }

    fn only_event(src: &str) -> ics::CalEvent {
        ics::events_in(&ics::parse(src).unwrap()).remove(0)
    }

    /// A timed event takes its own end; without one it takes DURATION; with
    /// neither it is the RFC's instant, not a guess at a length.
    #[test]
    fn a_timed_events_end_comes_from_dtend_then_duration_then_the_rfc() {
        let with_end = only_event(ONE);
        let crate::write::WhenInput::Timed { start_ms, end_ms } =
            input_for(&with_end, "UTC").unwrap().when
        else { panic!("timed") };
        assert_eq!(end_ms - start_ms, 3_600_000);

        let dur = only_event(&ics_with(
            "BEGIN:VEVENT\r\nUID:f@x\r\nDTSTART:20260907T113000Z\r\n\
             DURATION:PT45M\r\nEND:VEVENT\r\n",
        ));
        let crate::write::WhenInput::Timed { start_ms, end_ms } =
            input_for(&dur, "UTC").unwrap().when
        else { panic!("timed") };
        assert_eq!(end_ms - start_ms, 45 * 60_000);

        let bare = only_event(&ics_with(
            "BEGIN:VEVENT\r\nUID:g@x\r\nDTSTART:20260907T113000Z\r\nEND:VEVENT\r\n",
        ));
        let crate::write::WhenInput::Timed { start_ms, end_ms } =
            input_for(&bare, "UTC").unwrap().when
        else { panic!("timed") };
        assert_eq!(start_ms, end_ms, "an instant, not an invented hour");
    }

    /// An all-day event keeps iCalendar's exclusive end, and one without a
    /// DTEND gets the RFC's single day rather than a zero-length span.
    #[test]
    fn an_all_day_event_keeps_its_dates() {
        let two = only_event(&ics_with(
            "BEGIN:VEVENT\r\nUID:h@x\r\nDTSTART;VALUE=DATE:20260910\r\n\
             DTEND;VALUE=DATE:20260912\r\nEND:VEVENT\r\n",
        ));
        let crate::write::WhenInput::AllDay { start_date, end_date } =
            input_for(&two, "UTC").unwrap().when
        else { panic!("all-day") };
        assert_eq!((start_date.as_str(), end_date.as_str()), ("2026-09-10", "2026-09-12"));
        let one = only_event(&ics_with(
            "BEGIN:VEVENT\r\nUID:i@x\r\nDTSTART;VALUE=DATE:20260910\r\nEND:VEVENT\r\n",
        ));
        let crate::write::WhenInput::AllDay { start_date, end_date } =
            input_for(&one, "UTC").unwrap().when
        else { panic!("all-day") };
        assert_eq!((start_date.as_str(), end_date.as_str()), ("2026-09-10", "2026-09-11"),
            "no DTEND is the RFC's one day, not a zero-length span");
    }

    /// Guests never reach the create path, whatever the file said, and the
    /// text fields that are safe to carry do.
    #[test]
    fn the_mapped_input_carries_the_text_and_never_the_guests() {
        let ev = only_event(&ics_with(
            "BEGIN:VEVENT\r\nUID:j@x\r\nSUMMARY:Retro\r\nLOCATION:Room 3\r\n\
             DESCRIPTION:Bring notes\r\nDTSTART:20260907T090000Z\r\n\
             RRULE:FREQ=WEEKLY\r\nATTENDEE;CN=A:mailto:a@x.com\r\nEND:VEVENT\r\n",
        ));
        assert_eq!(ev.attendees.len(), 1, "the file really does carry one");
        let input = input_for(&ev, "UTC").unwrap();
        assert_eq!(input.summary.as_deref(), Some("Retro"));
        assert_eq!(input.location.as_deref(), Some("Room 3"));
        assert_eq!(input.description.as_deref(), Some("Bring notes"));
        assert_eq!(input.repeat.as_deref(), Some("weekly"));
        assert!(input.guests.is_none(), "a create with guests would invite them");
        assert!(input.conference.is_none());
    }

    /// Popup alarms come through as reminders; an email alarm does not,
    /// because honouring it would have a server send mail on an import.
    #[test]
    fn popup_alarms_become_reminders_and_email_alarms_do_not() {
        let ev = only_event(&ics_with(
            "BEGIN:VEVENT\r\nUID:k@x\r\nSUMMARY:Call\r\nDTSTART:20260907T090000Z\r\n\
             BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n\
             BEGIN:VALARM\r\nACTION:EMAIL\r\nTRIGGER:-PT60M\r\nEND:VALARM\r\n\
             END:VEVENT\r\n",
        ));
        let r = input_for(&ev, "UTC").unwrap().reminders.expect("reminders");
        assert!(!r.use_default);
        assert_eq!(r.overrides.len(), 1, "the email alarm is not carried");
        assert_eq!(r.overrides[0].minutes, 15);
        assert_eq!(r.overrides[0].method, "popup");

        let none = only_event(ONE);
        assert!(input_for(&none, "UTC").unwrap().reminders.is_none(),
            "no alarms leaves the calendar's own defaults alone");
    }

    /// Not iCalendar at all is nothing to plan; iCalendar with no events is
    /// an empty plan, which is a different answer and reads differently.
    #[test]
    fn rubbish_is_no_plan_and_an_empty_calendar_is_an_empty_one() {
        assert!(plan("this is not a calendar", "UTC", &[]).is_none());
        assert_eq!(plan(&ics_with(""), "UTC", &[]).unwrap(), vec![]);
    }
}

/// What an import did, once it had done it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub imported: usize,
    /// Planned but refused, with the reason — the same list the preview
    /// showed, repeated here so the result stands on its own.
    pub skipped: Vec<Planned>,
    /// Planned, attempted, and turned down by the server. Named one by one:
    /// a count would leave the user guessing which of their events is
    /// missing.
    pub failed: Vec<Planned>,
}

/// The identifiers the target calendar already holds.
///
/// On a CalDAV calendar these are the events' UIDs (`omacal-sync`'s `caldav`
/// module stores `ev.uid` as the provider identifier), and an import writes
/// each event under the file's own UID (`run_file` hands it to the create),
/// so importing the same file twice is a no-op there. That second half was
/// missing until #180: the create minted a fresh UUID, and a re-import
/// duplicated everything. On a Google calendar the column
/// holds Google's own event id instead, which an `.ics` file does not carry,
/// so a re-import cannot be recognised and will create the events again.
/// That is a real limit and the reason this returns what it can see rather
/// than claiming to answer the question in general.
async fn existing_uids(pool: &sqlx::SqlitePool, calendar_id: i64) -> Vec<String> {
    sqlx::query("SELECT google_id FROM events WHERE calendar_id = ?1")
        .bind(calendar_id)
        .fetch_all(pool)
        .await
        .map(|rows| rows.iter().map(|r| r.get::<String, _>("google_id")).collect())
        .unwrap_or_default()
}

/// Reads `path` and says what importing it into `calendar_id` would do.
pub(crate) async fn plan_file(
    state: &crate::AppState,
    calendar_id: i64,
    path: &std::path::Path,
) -> Result<Vec<Planned>, String> {
    let src = read_ics(path)?;
    let tz = calendar_zone(state, calendar_id).await?;
    plan(&src, &tz, &existing_uids(&state.pool, calendar_id).await)
        .ok_or_else(|| NOT_A_CALENDAR.to_string())
}

/// Imports `path` into `calendar_id`, and reports what happened.
///
/// The plan is taken again here rather than passed in from the preview:
/// what gets written is decided by the file as it is now, and the report
/// says what that was. Every create goes through `events::create_event_body`
/// — the window's own path — with `send_updates` of `none`, which together
/// with the absent guest list is the second of the two locks on an import
/// mailing anybody.
pub(crate) async fn run_file(
    state: &crate::AppState,
    calendar_id: i64,
    path: &std::path::Path,
) -> Result<Report, String> {
    let src = read_ics(path)?;
    let tz = calendar_zone(state, calendar_id).await?;
    let uids = existing_uids(&state.pool, calendar_id).await;
    let planned = plan(&src, &tz, &uids).ok_or_else(|| NOT_A_CALENDAR.to_string())?;

    // Keyed by summary and start, which is what a `Planned::Import` carries;
    // the events are re-read from the file so the full fields are to hand.
    let root = ics::parse(&src).ok_or_else(|| NOT_A_CALENDAR.to_string())?;
    let events = ics::events_in(&root);

    let mut report = Report { imported: 0, skipped: Vec::new(), failed: Vec::new() };
    for (i, p) in planned.into_iter().enumerate() {
        let Planned::Import { .. } = p else {
            report.skipped.push(p);
            continue;
        };
        let Some(ev) = events.get(i) else {
            report.failed.push(p);
            continue;
        };
        let Some(input) = input_for(ev, &tz) else {
            report.failed.push(p);
            continue;
        };
        match crate::events::create_event_body(state, calendar_id, input, "none", Some(&ev.uid)).await {
            Ok(_) => report.imported += 1,
            Err(e) => {
                tracing::warn!(%e, summary = p.summary(), "import: one event refused");
                report.failed.push(p);
            }
        }
    }
    Ok(report)
}

pub const NOT_A_CALENDAR: &str = "that file is not an iCalendar file";
pub const NO_SUCH_CALENDAR: &str = "that calendar is not here any more";
/// A guard, not a judgement about real files: an `.ics` is text, and a
/// hundred megabytes of it is a mistake rather than a calendar.
const MAX_BYTES: u64 = 32 * 1024 * 1024;

fn read_ics(path: &std::path::Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|_| "that file could not be read".to_string())?;
    if meta.len() > MAX_BYTES {
        return Err("that file is too large to import".into());
    }
    std::fs::read_to_string(path).map_err(|_| "that file could not be read".to_string())
}

async fn calendar_zone(state: &crate::AppState, calendar_id: i64) -> Result<String, String> {
    omacal_store::calendar_for_write(&state.pool, calendar_id)
        .await
        .map_err(|e| e.to_string())?
        .map(|(_, _, _, tz)| tz)
        .ok_or_else(|| NO_SUCH_CALENDAR.to_string())
}

/// The form's "Import .ics…" (#180): the platform's file chooser, filtered to
/// calendar files, and the path picked, or `None` when it was dismissed. Only
/// the path comes back: what to do with the file is the same panel a drop
/// opens ([`plan_ics_import`], then [`run_ics_import`]), so a picked file and
/// a dropped one cannot be imported two different ways. On Linux the chooser
/// is the desktop's portal picker when there is one (#182,
/// `gtk_use_portal`).
#[tauri::command]
pub(crate) async fn pick_ics_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    // The chooser answers on its own thread; a channel turns the callback
    // into something to await, as `export_event` does.
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Calendar file", &["ics", "ICS"])
        .pick_file(move |chosen| {
            let _ = tx.send(chosen);
        });
    let Some(chosen) = rx.await.map_err(|_| crate::export::EXPORT_DISMISSED.to_string())? else {
        return Ok(None);
    };
    let path = chosen.into_path().map_err(|e| crate::errors::user_facing(&anyhow::anyhow!("{e}")))?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// What dropping a file on the window asks: what would this do?
#[tauri::command]
pub(crate) async fn plan_ics_import(
    state: tauri::State<'_, crate::AppState>,
    calendar_id: i64,
    path: String,
) -> Result<Vec<Planned>, String> {
    plan_file(&state, calendar_id, std::path::Path::new(&path)).await
}

/// And what confirming it does.
#[tauri::command]
pub(crate) async fn run_ics_import(
    state: tauri::State<'_, crate::AppState>,
    calendar_id: i64,
    path: String,
) -> Result<Report, String> {
    run_file(&state, calendar_id, std::path::Path::new(&path)).await
}
