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
//!   weekdays, weekly, monthly, yearly — rather than an RRULE, deliberately
//!   (see its doc comment), so a rule outside that set has no honest
//!   representation here. Importing the first occurrence and calling it the
//!   series would be a lie about the user's data.
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
        /// The repeat the form's vocabulary can express, when the file
        /// carried one at all.
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

/// The RRULE shapes `write::rrule_for` can produce, inverted.
///
/// Deliberately exact rather than clever: the point is to recognise only
/// what the app can store *and* re-emit unchanged, so a rule that merely
/// looks close (`FREQ=WEEKLY;INTERVAL=2`, a `BYDAY` that is not the working
/// week, a `COUNT`) is not one of these and is skipped. The inverse of a
/// five-entry table is a five-entry table.
pub fn repeat_for_rrule(lines: &[String]) -> Option<Option<&'static str>> {
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
    let body = rule.split_once(':').map(|(_, v)| v).unwrap_or("").to_ascii_uppercase();
    let normalised = body.replace(' ', "");
    Some(Some(match normalised.as_str() {
        "FREQ=DAILY" => "daily",
        "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR" => "weekdays",
        "FREQ=WEEKLY" => "weekly",
        "FREQ=MONTHLY" => "monthly",
        "FREQ=YEARLY" => "yearly",
        _ => return None,
    }))
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
                let Some(repeat) = repeat_for_rrule(&ev.recurrence) else {
                    return Planned::Skip { summary, reason: SKIP_UNEXPRESSIBLE_REPEAT.into() };
                };
                let Some((start_ms, _tz, all_day)) = ics::resolve(&ev.start, cal_tz) else {
                    return Planned::Skip { summary, reason: SKIP_NO_START.into() };
                };
                Planned::Import {
                    summary,
                    start_ms,
                    all_day,
                    repeat: repeat.map(str::to_string),
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
pub(crate) fn input_for(
    ev: &ics::CalEvent,
    cal_tz: &str,
    repeat: Option<&str>,
) -> Option<crate::write::EventInput> {
    let (start_ms, _tz, all_day) = ics::resolve(&ev.start, cal_tz)?;
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
        repeat: repeat.map(str::to_string),
        weekly_days: None,
        repeat_end: None,
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
                repeat: None,
                dropped_guests: 0,
            }
        );
    }

    /// The five shapes the form can store come through as repeats; anything
    /// else is skipped rather than quietly imported as a single event, which
    /// would be this module telling the user something their file did not
    /// say.
    #[test]
    fn only_the_repeats_the_form_can_store_are_carried_and_the_rest_are_refused() {
        for (rule, expected) in [
            ("FREQ=DAILY", Some("daily")),
            ("FREQ=WEEKLY", Some("weekly")),
            ("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", Some("weekdays")),
            ("FREQ=MONTHLY", Some("monthly")),
            ("FREQ=YEARLY", Some("yearly")),
        ] {
            assert_eq!(
                repeat_for_rrule(&[format!("RRULE:{rule}")]),
                Some(expected),
                "{rule}"
            );
        }
        for rule in [
            "FREQ=WEEKLY;INTERVAL=2",
            "FREQ=DAILY;COUNT=10",
            "FREQ=MONTHLY;BYDAY=3TH",
            "FREQ=WEEKLY;BYDAY=MO,WE",
            "FREQ=HOURLY",
        ] {
            assert_eq!(repeat_for_rrule(&[format!("RRULE:{rule}")]), None, "{rule}");
        }
        // A rule the app could store, next to dates the form cannot hold.
        assert_eq!(
            repeat_for_rrule(&["RRULE:FREQ=DAILY".into(), "EXDATE:20260909T113000Z".into()]),
            None,
            "an exception list is part of the rule",
        );
        assert_eq!(repeat_for_rrule(&[]), Some(None), "no rule is a single event");
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
        let Planned::Import { all_day, summary, .. } = &plan[0] else { panic!() };
        assert!(all_day);
        assert_eq!(summary, "(no title)");
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
            input_for(&with_end, "UTC", None).unwrap().when
        else { panic!("timed") };
        assert_eq!(end_ms - start_ms, 3_600_000);

        let dur = only_event(&ics_with(
            "BEGIN:VEVENT\r\nUID:f@x\r\nDTSTART:20260907T113000Z\r\n\
             DURATION:PT45M\r\nEND:VEVENT\r\n",
        ));
        let crate::write::WhenInput::Timed { start_ms, end_ms } =
            input_for(&dur, "UTC", None).unwrap().when
        else { panic!("timed") };
        assert_eq!(end_ms - start_ms, 45 * 60_000);

        let bare = only_event(&ics_with(
            "BEGIN:VEVENT\r\nUID:g@x\r\nDTSTART:20260907T113000Z\r\nEND:VEVENT\r\n",
        ));
        let crate::write::WhenInput::Timed { start_ms, end_ms } =
            input_for(&bare, "UTC", None).unwrap().when
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
            input_for(&two, "UTC", None).unwrap().when
        else { panic!("all-day") };
        assert_eq!((start_date.as_str(), end_date.as_str()), ("2026-09-10", "2026-09-12"));
        let one = only_event(&ics_with(
            "BEGIN:VEVENT\r\nUID:i@x\r\nDTSTART;VALUE=DATE:20260910\r\nEND:VEVENT\r\n",
        ));
        let crate::write::WhenInput::AllDay { start_date, end_date } =
            input_for(&one, "UTC", None).unwrap().when
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
             ATTENDEE;CN=A:mailto:a@x.com\r\nEND:VEVENT\r\n",
        ));
        assert_eq!(ev.attendees.len(), 1, "the file really does carry one");
        let input = input_for(&ev, "UTC", Some("weekly")).unwrap();
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
        let r = input_for(&ev, "UTC", None).unwrap().reminders.expect("reminders");
        assert!(!r.use_default);
        assert_eq!(r.overrides.len(), 1, "the email alarm is not carried");
        assert_eq!(r.overrides[0].minutes, 15);
        assert_eq!(r.overrides[0].method, "popup");

        let none = only_event(ONE);
        assert!(input_for(&none, "UTC", None).unwrap().reminders.is_none(),
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
/// On a CalDAV calendar these *are* the files' UIDs (`omacal-sync`'s
/// `caldav` module stores `ev.uid` as the provider identifier), so importing
/// the same file twice is a no-op there. On a Google calendar the column
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
        let Planned::Import { ref repeat, .. } = p else {
            report.skipped.push(p);
            continue;
        };
        let Some(ev) = events.get(i) else {
            report.failed.push(p);
            continue;
        };
        let Some(input) = input_for(ev, &tz, repeat.as_deref()) else {
            report.failed.push(p);
            continue;
        };
        match crate::events::create_event_body(state, calendar_id, input, "none").await {
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
