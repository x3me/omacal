//! The write half of the CLI, client side (CLI-writes spec §3–§5).
//!
//! `omacal events create|update|delete|respond` — parsed here, prechecked
//! here, and then **executed by the running app** over the socket `ipc.rs`
//! owns. This module never opens the database for writing; its read-only
//! pool answers only what the refusals below need (does this event repeat,
//! who is on it, whose is it), and everything that could change a
//! calendar happens on the other end of the socket, behind the app's own
//! guards.
//!
//! The two refusals are the spec's §3/§4 verbatim, and they are refusals,
//! never defaults: a recurring event without `--scope` and a guest-touching
//! write without `--notify` are usage errors with the options named. One
//! flag is a cheap price; a meeting-cancelled email nobody meant to send is
//! not. A third joined them later, the other way round: a No on the user's
//! own meeting, which sends no email at all when one was expected
//! ([`own_no_rule`]).
//!
//! Split as ever: parsing, the rules, the time building and the request
//! shapes are pure and tested — the request builders are tested through
//! `ipc::parse_request` itself, so the two ends of the wire provably speak
//! one vocabulary — while the socket call is the thin untested shell.

use sqlx::SqlitePool;

use crate::cli::{fail, EXIT_ERROR, EXIT_OK, EXIT_USAGE};

pub(crate) const EXIT_NOT_RUNNING: i32 = 5;
pub(crate) const EXIT_REFUSED: i32 = 6;

/// How long the CLI waits for the app's answer. A write travels to Google
/// or a CalDAV server before it is answered, so this is generous — and past
/// it the truth is that the write's fate is unknown, which the timeout
/// message says instead of inviting the retry that mints duplicates.
const REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, PartialEq)]
pub(crate) enum WriteCmd {
    Create(CreateArgs),
    /// The task verbs, parsed and executed by `cli_tasks`.
    Task(crate::cli_tasks::TaskCmd),
    Update(UpdateArgs),
    Delete(DeleteArgs),
    Respond(RespondArgs),
}

#[derive(Debug, PartialEq, Default)]
pub(crate) struct CreateArgs {
    pub title: String,
    pub date: String,
    pub start: Option<String>,
    pub end: Option<String>,
    /// Timed events crossing midnight: the end's own date.
    pub end_date: Option<String>,
    pub all_day: bool,
    /// All-day: the *inclusive* last day, the form's own vocabulary. The
    /// wire's exclusive end is built in [`all_day_when`], nowhere else.
    pub last_day: Option<String>,
    pub calendar: Option<i64>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub guests: Vec<String>,
    pub notify: Option<String>,
    /// The repeat, in the app form's own vocabulary rather than iCalendar.
    /// Raw here and validated in one place — [`repeat_for`] — which is also
    /// what refuses the three flags below when this one is absent.
    pub repeat: Option<String>,
    pub days: Option<String>,
    pub until: Option<String>,
    pub count: Option<String>,
    /// `--meet`: ask Google to mint a Meet conference with the event — the
    /// form's "Google Meet" choice (`EventInput.conference = "googleMeet"`).
    /// Refused before the socket on any calendar but Google's ([`meet_rule`]).
    pub meet: bool,
}

#[derive(Debug, PartialEq)]
pub(crate) struct UpdateArgs {
    pub id: i64,
    pub occurrence: i64,
    pub scope: Option<String>,
    pub title: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub date: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub notify: Option<String>,
    /// `--meet`: add a Meet conference to an event that has no meeting link
    /// yet. Add only — removing one is the app's form, not a flag.
    pub meet: bool,
}

#[derive(Debug, PartialEq)]
pub(crate) struct DeleteArgs {
    pub id: i64,
    pub occurrence: i64,
    pub scope: Option<String>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct RespondArgs {
    pub id: i64,
    pub answer: String,
    pub scope: Option<String>,
    pub occurrence: Option<i64>,
    /// `--keep-meeting`: a No on the user's own meeting is meant as "I'm
    /// out", not "it's off" — see [`own_no_rule`].
    pub keep_meeting: bool,
}

/// `omacal events <verb> …` for the write verbs — `list` stays in `cli.rs`,
/// whose read-only doctrine this module deliberately does not touch.
pub(crate) fn parse_events(verb: &str, rest: &[&String]) -> Result<WriteCmd, String> {
    let take = |name: &str| -> Result<Option<String>, String> {
        let mut it = rest.iter();
        while let Some(a) = it.next() {
            if a.as_str() == name {
                return match it.next() {
                    Some(v) if !v.starts_with("--") => Ok(Some((*v).clone())),
                    _ => Err(format!("{name} needs a value")),
                };
            }
        }
        Ok(None)
    };
    let take_all = |name: &str| -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        let mut it = rest.iter();
        while let Some(a) = it.next() {
            if a.as_str() == name {
                match it.next() {
                    Some(v) if !v.starts_with("--") => out.push((*v).clone()),
                    _ => return Err(format!("{name} needs a value")),
                }
            }
        }
        Ok(out)
    };
    let flag = |name: &str| rest.iter().any(|a| a.as_str() == name);
    let required = |name: &str| -> Result<String, String> {
        take(name)?.ok_or_else(|| format!("{name} is required"))
    };
    let id_of = |v: &str| -> Result<i64, String> {
        v.parse().map_err(|_| "the event id is a number — `omacal events list` prints them".into())
    };
    let ms_of = |name: &str, v: &str| -> Result<i64, String> {
        v.parse().map_err(|_| {
            format!("{name} takes the occurrence's startMs — `omacal events list --json` prints it")
        })
    };
    // The verbs that address an occurrence all lead with the id, bare —
    // `omacal events delete 41 --occurrence …` — matching how phase 1
    // prints them back.
    let bare_id = || -> Result<i64, String> {
        rest.first()
            .filter(|a| !a.starts_with("--"))
            .ok_or_else(|| "the event id comes first — `omacal events list` prints them".to_string())
            .and_then(|v| id_of(v))
    };

    match verb {
        "create" => {
            let all_day = flag("--all-day");
            let args = CreateArgs {
                title: required("--title")?,
                date: required("--date")?,
                start: take("--start")?,
                end: take("--end")?,
                end_date: take("--end-date")?,
                all_day,
                last_day: take("--last-day")?,
                calendar: take("--calendar")?.map(|v| id_of(&v)).transpose()?,
                location: take("--location")?,
                description: take("--description")?,
                guests: take_all("--guest")?,
                notify: take("--notify")?,
                repeat: take("--repeat")?,
                days: take("--days")?,
                until: take("--until")?,
                count: take("--count")?,
                meet: flag("--meet"),
            };
            if all_day {
                if args.start.is_some() || args.end.is_some() {
                    return Err("--all-day and --start/--end do not mix".into());
                }
            } else {
                if args.start.is_none() || args.end.is_none() {
                    return Err("a timed event needs --start HH:MM and --end HH:MM (or --all-day)".into());
                }
                if args.last_day.is_some() {
                    return Err("--last-day is for --all-day events; timed ones take --end-date".into());
                }
            }
            Ok(WriteCmd::Create(args))
        }
        "update" => {
            let args = UpdateArgs {
                id: bare_id()?,
                occurrence: ms_of("--occurrence", &required("--occurrence")?)?,
                scope: take("--scope")?,
                title: take("--title")?,
                location: take("--location")?,
                description: take("--description")?,
                date: take("--date")?,
                start: take("--start")?,
                end: take("--end")?,
                notify: take("--notify")?,
                meet: flag("--meet"),
            };
            if args.title.is_none()
                && args.location.is_none()
                && args.description.is_none()
                && args.date.is_none()
                && args.start.is_none()
                && args.end.is_none()
                && !args.meet
            {
                return Err("nothing to change — pass --title, --location, --description, \
                            --date, --start, --end or --meet"
                    .into());
            }
            Ok(WriteCmd::Update(args))
        }
        "delete" => Ok(WriteCmd::Delete(DeleteArgs {
            id: bare_id()?,
            occurrence: ms_of("--occurrence", &required("--occurrence")?)?,
            scope: take("--scope")?,
        })),
        "respond" => {
            let answer = rest
                .iter()
                .skip(1)
                .find(|a| !a.starts_with("--"))
                .map(|a| a.to_string())
                .ok_or_else(|| "usage: omacal events respond ID yes|maybe|no".to_string())?;
            Ok(WriteCmd::Respond(RespondArgs {
                id: bare_id()?,
                answer,
                scope: take("--scope")?,
                occurrence: take("--occurrence")?.map(|v| ms_of("--occurrence", &v)).transpose()?,
                keep_meeting: flag("--keep-meeting"),
            }))
        }
        other => Err(format!(
            "unknown events subcommand \"{other}\" — list, create, update, delete or respond"
        )),
    }
}

// ---- The rules that refuse rather than guess (spec §3, §4) ---------------

/// Which recurrence scope a write carries. Recurring without an answer is
/// the refusal; non-recurring quietly means "this", and a scope handed to a
/// non-recurring event is refused too — a script must not carry a
/// meaningless flag into a future where it means something.
pub(crate) fn scope_for(recurring: bool, asked: Option<&str>) -> Result<&'static str, String> {
    match (recurring, asked) {
        (true, None) => Err(
            "this event repeats — say which occurrences you mean: --scope this|following|all".into(),
        ),
        (true, Some("this")) => Ok("this"),
        (true, Some("following")) => Ok("following"),
        (true, Some("all")) => Ok("all"),
        (true, Some(other)) => Err(format!("--scope takes this|following|all, not \"{other}\"")),
        (false, None) => Ok("this"),
        (false, Some(_)) => Err("--scope is for repeating events; this one does not repeat".into()),
    }
}

/// [`notify_for`], read through who the change reaches. A guest's own copy
/// (`Reach::OwnCopy`) has nobody to notify — Google keeps it apart from the
/// organizer's, so an update moves this calendar alone and no mail goes
/// anywhere — which makes the flag unnecessary rather than required, and
/// `--notify all` a promise the CLI refuses to make. The other two reaches
/// keep the ordinary rule.
pub(crate) fn notify_for_reach(
    reach: crate::events::Reach,
    has_guests: bool,
    asked: Option<&str>,
) -> Result<&'static str, String> {
    match (reach, asked) {
        (crate::events::Reach::OwnCopy, None | Some("none")) => Ok("none"),
        (crate::events::Reach::OwnCopy, Some("all")) => Err(
            "this is your copy of somebody else's event — a change moves it on your calendar \
             alone and nobody else's copy changes, so there is nobody to notify; drop --notify"
                .into(),
        ),
        _ => notify_for(has_guests, asked),
    }
}

/// The notify rule on a calendar whose provider mails nobody (anything but
/// Google: CalDAV, WebCal, local — see `EventDetail::mails_guests`). There is nobody to notify however
/// many attendees the event has, so the flag is unnecessary, `none` is
/// accepted as what happens anyway, and `all` is refused with the reason —
/// the own-copy shape above, for a different reason. Read *instead of*
/// [`notify_for_reach`] and [`notify_for`], never after them: an answer
/// they would demand is one nothing on this calendar can act on.
pub(crate) fn notify_without_mail(asked: Option<&str>) -> Result<&'static str, String> {
    match asked {
        None | Some("none") => Ok("none"),
        Some("all") => Err(
            "OmaCal emails nobody on this calendar, so there is nobody to notify; drop --notify"
                .into(),
        ),
        Some(other) => Err(format!("--notify takes all|none, not \"{other}\"")),
    }
}

/// Whether `--meet` can do what it says. Only Google mints a Meet
/// conference, and Google is also the one provider that mails guests, so
/// `is_google` is [`calendar_mails_guests`]'s answer — one question, asked
/// once. The app refuses the same thing behind the socket (`events.rs`);
/// this is the CLI's usage error first, with the way out named.
///
/// An update also carries `reach` and the event's current meeting link,
/// whichever way it is held — structured conference data, or a URL in
/// Location or Description (`upcoming::conference_join_url`, the same
/// fallback `events list` prints as `conference`). A guest's own copy has
/// no conference of its own to set: the organizer owns it. And an event
/// that already has a link keeps it: `--meet` there would put a second
/// meeting beside, or in place of, the one the guests were sent.
pub(crate) fn meet_rule(
    is_google: bool,
    reach: Option<crate::events::Reach>,
    existing_link: Option<&str>,
) -> Result<(), String> {
    if !is_google {
        return Err("only Google calendars can create Google Meet links; drop --meet or \
                    pick a Google calendar with --calendar"
            .into());
    }
    if reach == Some(crate::events::Reach::OwnCopy) {
        return Err("this is your copy of somebody else's event — its organizer owns the \
                    meeting link, so --meet cannot add one; drop --meet"
            .into());
    }
    if let Some(url) = existing_link {
        return Err(format!(
            "this event already has a meeting link ({url}); --meet would add a second one, \
             so drop --meet"
        ));
    }
    Ok(())
}

/// Whether a calendar's provider mails the people on an event: the rule
/// `EventDetail::mails_guests` answers for the app, asked of the CLI's
/// read-only pool. A calendar that cannot be read reads as mailing, so the
/// notify question is asked rather than skipped — the side a wrong answer
/// should fall on.
async fn calendar_mails_guests(pool: &SqlitePool, calendar_id: Option<i64>) -> bool {
    let Some(id) = calendar_id else { return true };
    !matches!(crate::caldav_write::calendar_mails_guests(pool, id).await, Ok(false))
}

/// [`calendar_mails_guests`] for a create: the calendar named, or the one
/// the app's own default rule (`ipc::default_calendar`) will pick when none
/// is, asked of the same database.
async fn create_mails_guests(pool: &SqlitePool, calendar: Option<i64>) -> bool {
    let calendar = match calendar {
        Some(id) => Some(id),
        None => crate::ipc::default_calendar(pool).await,
    };
    calendar_mails_guests(pool, calendar).await
}

/// Whether guests hear about a write. Touching a guest list without an
/// answer is the refusal; a guestless write quietly mails nobody, which is
/// the only thing it could honestly do.
pub(crate) fn notify_for(has_guests: bool, asked: Option<&str>) -> Result<&'static str, String> {
    match (has_guests, asked) {
        (true, None) => Err(
            "this event has guests — say who hears about the change: --notify all|none".into(),
        ),
        (_, Some("all")) => Ok("all"),
        (_, Some("none")) => Ok("none"),
        (_, Some(other)) => Err(format!("--notify takes all|none, not \"{other}\"")),
        (false, None) => Ok("none"),
    }
}

/// A validated repeat, carried in the app's vocabulary rather than
/// iCalendar: [`crate::write::rrule_for`] on the other side of the socket
/// owns the RRULE, exactly as it does for the form. Absent is the one-off
/// create this command has always made.
#[derive(Debug, PartialEq)]
pub(crate) struct RepeatPlan {
    pub repeat: String,
    /// Canonical weekday codes; empty unless a weekly cadence named days.
    pub weekly_days: Vec<String>,
    pub end: crate::write::RepeatEnd,
}

/// The repeat a create carries. Neither vocabulary is re-listed here: a
/// word is a repeat if [`crate::write::rrule_for`] can build a rule from it,
/// and a weekday is a weekday if [`crate::write::WeekdayCode`] — the command
/// boundary's own type — deserializes it. Spelling either out a second time
/// is how the CLI and the app would drift.
///
/// Every combination that means nothing is refused rather than dropped: a
/// day pattern without a weekly cadence, an ending without a repeat, both
/// endings at once, a count of zero. `never` is refused too — leaving the
/// flag off is how that is said, and accepting the word would give "does
/// not repeat" two spellings that could disagree.
pub(crate) fn repeat_for(
    asked: Option<&str>,
    days: Option<&str>,
    until: Option<&str>,
    count: Option<&str>,
) -> Result<Option<RepeatPlan>, String> {
    let Some(word) = asked else {
        return match (days, until, count) {
            (None, None, None) => Ok(None),
            (Some(_), _, _) => {
                Err("--days is for --repeat weekly; this event does not repeat".into())
            }
            _ => Err("a repeat ending needs a repeating schedule: pass --repeat".into()),
        };
    };
    if word == "never" {
        return Err("leave --repeat off for an event that does not repeat".into());
    }
    if crate::write::rrule_for(word).is_none() {
        return Err(format!(
            "--repeat takes daily|weekdays|weekly|monthly|yearly, not \"{word}\""
        ));
    }
    let weekly_days = match days {
        None => Vec::new(),
        Some(_) if word != "weekly" => {
            return Err(format!(
                "--days is for --repeat weekly; \"{word}\" already says which days"
            ));
        }
        Some(list) => weekday_codes(list)?,
    };
    let end = match (until, count) {
        (None, None) => crate::write::RepeatEnd::Never,
        (Some(_), Some(_)) => {
            return Err("--until and --count are two ways to end a repeat; pass one".into());
        }
        (Some(date), None) => {
            date.parse::<jiff::civil::Date>()
                .map_err(|_| "--until takes YYYY-MM-DD".to_string())?;
            crate::write::RepeatEnd::On { date: date.to_string() }
        }
        (None, Some(n)) => match n.parse::<u32>() {
            Ok(0) => return Err("a repeating event must occur at least once".into()),
            Ok(count) => crate::write::RepeatEnd::After { count },
            Err(_) => return Err(format!("--count takes a number of occurrences, not \"{n}\"")),
        },
    };
    Ok(Some(RepeatPlan { repeat: word.to_string(), weekly_days, end }))
}

/// `MO,we,FR` to the canonical codes. Case is forgiven — a terminal is not
/// a form — but nothing else is: the codes are checked by deserializing
/// them as [`crate::write::WeekdayCode`], so a typo is refused here with a
/// readable message instead of reaching Google inside a broken RRULE.
fn weekday_codes(list: &str) -> Result<Vec<String>, String> {
    list.split(',')
        .map(|code| {
            let code = code.trim().to_ascii_uppercase();
            serde_json::from_value::<crate::write::WeekdayCode>(serde_json::Value::String(
                code.clone(),
            ))
            .map(|_| code.clone())
            .map_err(|_| format!("--days takes SU,MO,TU,WE,TH,FR,SA — \"{code}\" is not one"))
        })
        .collect()
}

/// `yes|maybe|no` to the protocol's own words. The CLI speaks the human's
/// vocabulary and the wire speaks Google's; this is the whole translation.
pub(crate) fn answer_word(answer: &str) -> Result<&'static str, String> {
    match answer {
        "yes" => Ok("accepted"),
        "maybe" => Ok("tentative"),
        "no" => Ok("declined"),
        other => Err(format!("respond takes yes|maybe|no, not \"{other}\"")),
    }
}

/// Respond's scope: `all` unless narrowed — what answering the email would
/// do, and `respond_to_event`'s own semantics for any event, repeating or
/// not (scope `all` never resolves an instance).
pub(crate) fn respond_scope_for(asked: Option<&str>) -> Result<&'static str, String> {
    match asked {
        None | Some("all") => Ok("all"),
        Some("this") => Ok("this"),
        Some(other) => Err(format!("--scope takes this|all here, not \"{other}\"")),
    }
}

/// An address Google minted for a room or a calendar rather than a person —
/// the UI's `isMachineAddress`, matched on the exact domain for the same
/// reason it is there: a suffix match accepts look-alikes.
fn is_machine_address(email: &str) -> bool {
    email.rsplit_once('@').is_some_and(|(_, domain)| {
        matches!(
            domain.trim().to_ascii_lowercase().as_str(),
            "group.calendar.google.com" | "resource.calendar.google.com"
        )
    })
}

/// Who a No on the user's own meeting leaves expecting them, by name: the
/// other people on it, minus anyone who already declined and the rooms. The
/// UI's `leftOutByYourNo` (`owndecline.ts`) counts the same way.
pub(crate) fn left_out_by_your_no(attendees: &[omacal_store::Attendee]) -> Vec<String> {
    attendees
        .iter()
        .filter(|a| !a.is_self && a.response_status != "declined" && !is_machine_address(&a.email))
        .map(|a| a.display_name.clone().unwrap_or_else(|| a.email.clone()))
        .collect()
}

/// The third never-guess refusal (2026-10-26): a No on the user's own
/// Google meeting. Google mails a guest's No to the organizer, so when the
/// organizer says it nobody is told and the meeting stays on everyone
/// else's calendar — Plamen declined his own 1:1 that way and Teodor still
/// expected it. "It's off" and "I'm out" are different requests, so the
/// caller names one: `events delete` (Google mails the cancellation) or
/// `--keep-meeting`. A guest's No, a calendar that mails nobody, and a
/// meeting with no one else left to expect it pass untouched.
pub(crate) fn own_no_rule(
    response: &str,
    reach: crate::events::Reach,
    mails_guests: bool,
    left_out: &[String],
    keep_meeting: bool,
    cancel_command: &str,
) -> Result<(), String> {
    if response != "declined"
        || reach != crate::events::Reach::Organizer
        || !mails_guests
        || left_out.is_empty()
        || keep_meeting
    {
        return Ok(());
    }
    let whose = match left_out {
        [one] => format!("{one}'s calendar"),
        many => format!("{} other people's calendars", many.len()),
    };
    Err(format!(
        "you organise this meeting, so a No only changes your own answer: Google tells nobody \
         and it stays on {whose}. To call it off for them, `{cancel_command}` — Google emails \
         them the cancellation. To mark yourself out and leave it on, add --keep-meeting"
    ))
}

// ---- Time building, in an explicit zone so a test can pin one ------------

/// `--date` + `--start`/`--end` (+ optional `--end-date`) to the wire's
/// instants, read in `tz` — the display zone at runtime, because that is
/// the zone every time in omacal means. Strict `HH:MM`: a CLI is where
/// scripts speak, not where people mistype, and a guess stored is a guess
/// on somebody's calendar.
pub(crate) fn timed_when(
    date: &str,
    start: &str,
    end: &str,
    end_date: Option<&str>,
    tz: &jiff::tz::TimeZone,
) -> Result<(i64, i64), String> {
    let day: jiff::civil::Date =
        date.parse().map_err(|_| "--date takes YYYY-MM-DD".to_string())?;
    let end_day: jiff::civil::Date = match end_date {
        Some(d) => d.parse().map_err(|_| "--end-date takes YYYY-MM-DD".to_string())?,
        None => day,
    };
    let clock = |name: &str, v: &str| -> Result<jiff::civil::Time, String> {
        v.parse().map_err(|_| format!("{name} takes HH:MM"))
    };
    let ms = |d: jiff::civil::Date, t: jiff::civil::Time| -> Result<i64, String> {
        Ok(d.to_datetime(t)
            .to_zoned(tz.clone())
            .map_err(|e| e.to_string())?
            .timestamp()
            .as_millisecond())
    };
    let start_ms = ms(day, clock("--start", start)?)?;
    let end_ms = ms(end_day, clock("--end", end)?)?;
    if end_ms <= start_ms {
        return Err("the end is not after the start".into());
    }
    Ok((start_ms, end_ms))
}

/// `--date` + `--last-day` (inclusive, the human's word) to the wire's
/// exclusive `endDate` — the exact `addDays(endDate, 1)` the form performs
/// in `toEventInput`, done here and nowhere else.
pub(crate) fn all_day_when(date: &str, last_day: &str) -> Result<(String, String), String> {
    let first: jiff::civil::Date =
        date.parse().map_err(|_| "--date takes YYYY-MM-DD".to_string())?;
    let last: jiff::civil::Date =
        last_day.parse().map_err(|_| "--last-day takes YYYY-MM-DD".to_string())?;
    if last < first {
        return Err("--last-day is before --date".into());
    }
    let exclusive = last
        .checked_add(jiff::Span::new().days(1))
        .map_err(|e| e.to_string())?;
    Ok((first.to_string(), exclusive.to_string()))
}

// ---- Requests, in the server's own vocabulary ----------------------------

fn base_fields(
    summary: Option<&str>,
    location: Option<&str>,
    description: Option<&str>,
    when: serde_json::Value,
    tz_name: &str,
) -> serde_json::Value {
    serde_json::json!({
        "summary": summary,
        "location": location,
        "description": description,
        "when": when,
        "tz": tz_name,
    })
}

pub(crate) fn create_request(
    args: &CreateArgs,
    when: serde_json::Value,
    notify: &str,
    tz_name: &str,
    repeat: Option<&RepeatPlan>,
) -> serde_json::Value {
    let mut fields = base_fields(
        Some(&args.title),
        args.location.as_deref(),
        args.description.as_deref(),
        when,
        tz_name,
    );
    // `repeat`, `weeklyDays` and `repeatEnd` are `EventInput`'s own fields:
    // the form has always sent them, so the app needs no change to read them
    // from here. Absent stays absent — an omitted `repeat` is what has always
    // meant "one-off", and `never` would mean "clear the rule" instead.
    if let Some(plan) = repeat {
        fields["repeat"] = serde_json::json!(plan.repeat);
        if !plan.weekly_days.is_empty() {
            fields["weeklyDays"] = serde_json::json!(plan.weekly_days);
        }
        if plan.end != crate::write::RepeatEnd::Never {
            fields["repeatEnd"] = serde_json::json!(plan.end);
        }
    }
    if !args.guests.is_empty() {
        fields["guests"] = serde_json::json!(args
            .guests
            .iter()
            .map(|email| serde_json::json!({ "email": email }))
            .collect::<Vec<_>>());
    }
    // `EventInput.conference`, which the form has always sent. Only ever
    // `googleMeet`: absent means "no conference" on a create and "leave it
    // alone" on an update, so the CLI never needs to say `none`.
    if args.meet {
        fields["conference"] = serde_json::json!("googleMeet");
    }
    serde_json::json!({
        "v": crate::ipc::PROTOCOL_VERSION,
        "cmd": "events-create",
        "calendar_id": args.calendar,
        "fields": fields,
        "send_updates": notify,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_request(
    args: &UpdateArgs,
    scope: &str,
    when: serde_json::Value,
    summary: Option<&str>,
    location: Option<&str>,
    description: Option<&str>,
    notify: &str,
    tz_name: &str,
) -> serde_json::Value {
    let mut fields = base_fields(summary, location, description, when, tz_name);
    // See `create_request`: `googleMeet` or absent, never `none`.
    if args.meet {
        fields["conference"] = serde_json::json!("googleMeet");
    }
    serde_json::json!({
        "v": crate::ipc::PROTOCOL_VERSION,
        "cmd": "events-update",
        "id": args.id,
        "scope": scope,
        "occurrence_start_ms": args.occurrence,
        "fields": fields,
        "send_updates": notify,
    })
}

pub(crate) fn delete_request(id: i64, scope: &str, occurrence: i64) -> serde_json::Value {
    serde_json::json!({
        "v": crate::ipc::PROTOCOL_VERSION,
        "cmd": "events-delete",
        "id": id,
        "scope": scope,
        "occurrence_start_ms": occurrence,
    })
}

pub(crate) fn respond_request(
    id: i64,
    response: &str,
    scope: &str,
    occurrence: i64,
) -> serde_json::Value {
    serde_json::json!({
        "v": crate::ipc::PROTOCOL_VERSION,
        "cmd": "events-respond",
        "id": id,
        "response": response,
        "scope": scope,
        "occurrence_start_ms": occurrence,
    })
}

// ---- The socket call, and the exits its answers map to -------------------

enum CallError {
    NotRunning,
    Timeout,
    Io(String),
}

/// One request, one envelope. Blocking I/O on purpose: this is a CLI with
/// exactly one thing to wait for.
fn call(request: &serde_json::Value) -> Result<serde_json::Value, CallError> {
    use std::io::{BufRead, Write};

    let Some(path) = crate::ipc::socket_path() else {
        return Err(CallError::NotRunning);
    };
    // A connect refusal and a missing socket answer the same: nobody is
    // listening. A stale file from a hard_restart connect-fails too, which
    // is exactly the honest reading.
    let mut stream =
        std::os::unix::net::UnixStream::connect(&path).map_err(|_| CallError::NotRunning)?;
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(5)));
    let _ = stream.set_read_timeout(Some(REPLY_TIMEOUT));

    let mut line = request.to_string();
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|e| CallError::Io(e.to_string()))?;

    let mut reply = String::new();
    let mut reader = std::io::BufReader::new(stream);
    match reader.read_line(&mut reply) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Err(CallError::Timeout),
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => return Err(CallError::Timeout),
        Err(e) => return Err(CallError::Io(e.to_string())),
    }
    serde_json::from_str(&reply).map_err(|e| CallError::Io(format!("unreadable reply: {e}")))
}

/// The envelope to the terminal, in both registers, with the exit the spec
/// names: the app's own refusal is 6, and transport trouble stays 4/5 —
/// the one distinction an agent acts on (retry nothing on 6).
fn finish(json: bool, envelope: serde_json::Value, done_word: &str) -> i32 {
    if envelope.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        if json {
            // Relayed, not rebuilt: the bytes the app answered are the bytes
            // the agent reads.
            println!("{envelope}");
        } else {
            let title = envelope["data"]["summary"].as_str().unwrap_or("");
            if title.is_empty() {
                println!("{done_word}.");
            } else {
                println!("{done_word}: {title}");
            }
        }
        return EXIT_OK;
    }
    let code = envelope["error"]["code"].as_str().unwrap_or("error");
    let message =
        envelope["error"]["message"].as_str().unwrap_or("the app answered something unreadable");
    let exit = match code {
        "usage" => EXIT_USAGE,
        "refused" => EXIT_REFUSED,
        _ => EXIT_ERROR,
    };
    fail(json, code, message, exit)
}

/// The row the prechecks read — one query through the store's own
/// `event_by_id`, so "does it repeat" and "does it have guests" are the
/// app's own answers, not a second derivation.
struct Target {
    recurring: bool,
    has_other_guests: bool,
    /// See [`left_out_by_your_no`]; read by [`own_no_rule`] alone.
    left_out: Vec<String>,
    /// Who a change reaches (`events::Reach`): for a guest's own copy the
    /// notify rule has nobody to apply to, and says so instead of demanding
    /// an answer.
    reach: crate::events::Reach,
    /// See [`calendar_mails_guests`]. `false` replaces the notify rule with
    /// [`notify_without_mail`].
    mails_guests: bool,
    all_day: bool,
    start_ms: i64,
    end_ms: i64,
    summary: Option<String>,
    location: Option<String>,
    description: Option<String>,
    /// The event's meeting link, however it is held — see [`meet_rule`].
    conference: Option<String>,
}

/// An update's descriptive fields: the flag when given, the event's own
/// value otherwise — **never absent**, because the wire treats the whole
/// `EventInput` as the caller's complete statement, exactly as the form
/// sends it. The first field run proved what an absent field means: a
/// time-only update went out with `summary: null` and came back titled
/// "(no title)", the title honestly cleared by a caller that never meant
/// to mention it.
fn merged_field(flag: &Option<String>, current: &Option<String>) -> Option<String> {
    flag.clone().or_else(|| current.clone())
}

async fn target(pool: &SqlitePool, id: i64) -> Result<Target, String> {
    let (event, _role, cal_google_id, account_email, _tz) =
        omacal_store::event_for_detail(pool, id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no event {id} — `omacal events list` prints real ids"))?;
    let is_organizer = crate::events::owns_event(
        event.organizer_email.as_deref(),
        &account_email,
        &cal_google_id,
    );
    Ok(Target {
        recurring: event.recurrence.is_some() || event.recurring_event_id.is_some(),
        reach: crate::events::Reach::of(is_organizer, event.guests_can_modify),
        mails_guests: calendar_mails_guests(pool, Some(event.calendar_id)).await,
        // The guest-list rule counts *other people*: a solo event mails
        // nobody whatever the flag says, and `mailableGuests` upstream
        // counts the same way.
        has_other_guests: event.attendees.iter().any(|a| !a.is_self),
        left_out: left_out_by_your_no(&event.attendees),
        all_day: event.is_all_day,
        start_ms: event.start_utc,
        end_ms: event.end_utc,
        summary: event.summary.clone(),
        location: event.location.clone(),
        description: event.description.clone(),
        conference: event.conference_uri.clone().or_else(|| {
            crate::upcoming::conference_join_url(
                event.location.as_deref(),
                event.description.as_deref(),
            )
        }),
    })
}

/// Runs one write command to an exit code. The pool is phase 1's read-only
/// one; every refusal below is decided before the socket is even touched,
/// so a request that reaches the app is one the app can only judge on its
/// own guards.
pub(crate) async fn execute(pool: &SqlitePool, cmd: &WriteCmd, json: bool) -> i32 {
    if let WriteCmd::Task(task) = cmd {
        return crate::cli_tasks::execute(pool, task, json).await;
    }
    let tz = jiff::tz::TimeZone::system();
    let tz_name = tz.iana_name().unwrap_or("UTC").to_string();

    let refuse = |m: &str| fail(json, "usage", m, EXIT_USAGE);
    let meet = matches!(cmd, WriteCmd::Create(CreateArgs { meet: true, .. })
        | WriteCmd::Update(UpdateArgs { meet: true, .. }));

    let (request, done_word) = match cmd {
        // Taken above; the compiler wants it named all the same.
        WriteCmd::Task(_) => unreachable!("handled at the top of execute"),
        WriteCmd::Create(args) => {
            let mails_guests = create_mails_guests(pool, args.calendar).await;
            if args.meet {
                if let Err(m) = meet_rule(mails_guests, None, None) {
                    return refuse(&m);
                }
            }
            let notify = if mails_guests {
                notify_for(!args.guests.is_empty(), args.notify.as_deref())
            } else {
                notify_without_mail(args.notify.as_deref())
            };
            let notify = match notify {
                Ok(n) => n,
                Err(m) => return refuse(&m),
            };
            let repeat = match repeat_for(
                args.repeat.as_deref(),
                args.days.as_deref(),
                args.until.as_deref(),
                args.count.as_deref(),
            ) {
                Ok(r) => r,
                Err(m) => return refuse(&m),
            };
            let when = if args.all_day {
                let last = args.last_day.as_deref().unwrap_or(&args.date);
                match all_day_when(&args.date, last) {
                    Ok((s, e)) => {
                        serde_json::json!({ "kind": "allDay", "startDate": s, "endDate": e })
                    }
                    Err(m) => return refuse(&m),
                }
            } else {
                // Parse guaranteed start/end present for timed creates.
                let (s, e) = (args.start.as_deref().unwrap_or(""), args.end.as_deref().unwrap_or(""));
                match timed_when(&args.date, s, e, args.end_date.as_deref(), &tz) {
                    Ok((s, e)) => serde_json::json!({ "kind": "timed", "startMs": s, "endMs": e }),
                    Err(m) => return refuse(&m),
                }
            };
            (create_request(args, when, notify, &tz_name, repeat.as_ref()), "Created")
        }
        WriteCmd::Update(args) => {
            let t = match target(pool, args.id).await {
                Ok(t) => t,
                Err(m) => return refuse(&m),
            };
            if t.all_day {
                return refuse(
                    "editing an all-day event from the CLI is not built yet — the app's form is",
                );
            }
            if args.meet {
                if let Err(m) = meet_rule(t.mails_guests, Some(t.reach), t.conference.as_deref()) {
                    return refuse(&m);
                }
            }
            let scope = match scope_for(t.recurring, args.scope.as_deref()) {
                Ok(s) => s,
                Err(m) => return refuse(&m),
            };
            let notify = if t.mails_guests {
                notify_for_reach(t.reach, t.has_other_guests, args.notify.as_deref())
            } else {
                notify_without_mail(args.notify.as_deref())
            };
            let notify = match notify {
                Ok(n) => n,
                Err(m) => return refuse(&m),
            };
            // The unchanged half of `when` comes from the occurrence the
            // flags address — its own start, plus the event's duration —
            // exactly what the form seeds before the user types.
            let occ_start = args.occurrence;
            let occ_end = occ_start + (t.end_ms - t.start_ms);
            let stamp = |ms: i64| -> Result<(String, String), String> {
                let z = jiff::Timestamp::from_millisecond(ms)
                    .map_err(|e| e.to_string())?
                    .to_zoned(tz.clone());
                Ok((z.date().to_string(), z.strftime("%H:%M").to_string()))
            };
            let ((cur_date, cur_start), (_, cur_end)) =
                match (stamp(occ_start), stamp(occ_end)) {
                    (Ok(a), Ok(b)) => (a, b),
                    (Err(m), _) | (_, Err(m)) => return refuse(&m),
                };
            let when = match timed_when(
                args.date.as_deref().unwrap_or(&cur_date),
                args.start.as_deref().unwrap_or(&cur_start),
                args.end.as_deref().unwrap_or(&cur_end),
                None,
                &tz,
            ) {
                Ok((s, e)) => serde_json::json!({ "kind": "timed", "startMs": s, "endMs": e }),
                Err(m) => return refuse(&m),
            };
            // Flags override, the event's own values carry — an update is
            // the whole statement (`merged_field`'s doc has the field
            // story of what an absent title really does).
            let summary = merged_field(&args.title, &t.summary);
            let location = merged_field(&args.location, &t.location);
            let description = merged_field(&args.description, &t.description);
            (
                update_request(
                    args,
                    scope,
                    when,
                    summary.as_deref(),
                    location.as_deref(),
                    description.as_deref(),
                    notify,
                    &tz_name,
                ),
                "Saved",
            )
        }
        WriteCmd::Delete(args) => {
            let t = match target(pool, args.id).await {
                Ok(t) => t,
                Err(m) => return refuse(&m),
            };
            let scope = match scope_for(t.recurring, args.scope.as_deref()) {
                Ok(s) => s,
                Err(m) => return refuse(&m),
            };
            (delete_request(args.id, scope, args.occurrence), "Deleted")
        }
        WriteCmd::Respond(args) => {
            let t = match target(pool, args.id).await {
                Ok(t) => t,
                Err(m) => return refuse(&m),
            };
            let response = match answer_word(&args.answer) {
                Ok(r) => r,
                Err(m) => return refuse(&m),
            };
            let scope = match respond_scope_for(args.scope.as_deref()) {
                Ok(s) => s,
                Err(m) => return refuse(&m),
            };
            if scope == "this" && args.occurrence.is_none() {
                return refuse("--scope this needs --occurrence — which time do you mean?");
            }
            let occurrence = args.occurrence.unwrap_or(t.start_ms);
            // The delete that calls it off. A series names its scope and, with
            // no --occurrence given, leaves MS to fill in: the series' own
            // start is the first occurrence, never a safe guess for "this one".
            let cancel = if t.recurring {
                let ms = args.occurrence.map_or("MS".to_string(), |o| o.to_string());
                format!("omacal events delete {} --occurrence {ms} --scope this|following|all", args.id)
            } else {
                format!("omacal events delete {} --occurrence {occurrence}", args.id)
            };
            if let Err(m) =
                own_no_rule(response, t.reach, t.mails_guests, &t.left_out, args.keep_meeting, &cancel)
            {
                return refuse(&m);
            }
            (respond_request(args.id, response, scope, occurrence), "Answered")
        }
    };

    send_reporting(&request, json, done_word, meet)
}

/// Sends one request to the running app and reports what came back.
///
/// Shared with `cli_tasks`: the socket, the three failure shapes and the
/// "the write's fate is unknown" wording are the same contract whichever
/// verb asked, and two copies of it would be two chances to drift.
pub(crate) fn send(request: &serde_json::Value, json: bool, done_word: &str) -> i32 {
    send_reporting(request, json, done_word, false)
}

/// [`send`], and for a `--meet` write the link Google minted. `--json`
/// already carries it as `data.conference_uri` (the saved event, as the
/// app answers it), relayed untouched; text mode
/// prints it on its own line. A saved event that came back without one is
/// still saved — exit 0 and a warning, never an error, because an error
/// is what invites the retry that creates the event twice.
/// The Meet link in the app's answer to a write: the saved event's
/// `conference_uri` (`events::EventDetail`, snake_case on this wire, unlike
/// `events show`'s `conference`).
fn minted_link(envelope: &serde_json::Value) -> Option<&str> {
    envelope["data"]["conference_uri"].as_str()
}

fn send_reporting(request: &serde_json::Value, json: bool, done_word: &str, meet: bool) -> i32 {
    match call(request) {
        Ok(envelope) => {
            let link = minted_link(&envelope).map(str::to_string);
            let code = finish(json, envelope, done_word);
            if meet && !json && code == EXIT_OK {
                match link {
                    Some(url) => println!("Meet: {url}"),
                    None => eprintln!(
                        "warning: saved, but Google returned no Meet link yet — check the event \
                         with `omacal events show` before trying again"
                    ),
                }
            }
            code
        }
        Err(CallError::NotRunning) => fail(
            json,
            "not_running",
            "omacal is not running — launch it first (writes go through the app's own guards)",
            EXIT_NOT_RUNNING,
        ),
        Err(CallError::Timeout) => fail(
            json,
            "timeout",
            "omacal did not answer within 60s — the write's fate is unknown; check the calendar \
             before retrying",
            EXIT_ERROR,
        ),
        Err(CallError::Io(m)) => fail(json, "io", &m, EXIT_ERROR),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(rest: &str) -> Vec<String> {
        rest.split_whitespace().map(String::from).collect()
    }
    fn parse(verb: &str, rest: &str) -> Result<WriteCmd, String> {
        let owned = argv(rest);
        let refs: Vec<&String> = owned.iter().collect();
        parse_events(verb, &refs)
    }

    /// The spec's §3 surface, as the parser reads it — the shapes that must
    /// land and the mistakes that must be named.
    #[test]
    fn create_parses_both_shapes_and_names_the_mistakes() {
        let timed = parse(
            "create",
            "--title Standup --date 2026-09-01 --start 09:00 --end 09:30 --guest a@b --guest c@d",
        )
        .unwrap();
        match timed {
            WriteCmd::Create(a) => {
                assert_eq!(a.title, "Standup");
                assert_eq!(a.guests, vec!["a@b", "c@d"], "--guest repeats");
                assert!(!a.all_day);
            }
            other => panic!("{other:?}"),
        }

        let repeating = parse(
            "create",
            "--title Gym --date 2026-09-14 --start 20:00 --end 21:30 \
             --repeat weekly --days MO,TU --until 2026-12-31",
        )
        .unwrap();
        match repeating {
            WriteCmd::Create(a) => {
                assert_eq!(a.repeat.as_deref(), Some("weekly"));
                assert_eq!(a.days.as_deref(), Some("MO,TU"));
                assert_eq!(a.until.as_deref(), Some("2026-12-31"));
                assert_eq!(a.count, None, "the parser only carries what was passed");
            }
            other => panic!("{other:?}"),
        }

        assert!(parse("create", "--title Trip --date 2026-09-01 --all-day").is_ok());
        assert!(parse("create", "--title X --date 2026-09-01")
            .unwrap_err()
            .contains("--start"));
        assert!(parse("create", "--title X --date 2026-09-01 --all-day --start 09:00")
            .unwrap_err()
            .contains("do not mix"));
        assert!(parse("create", "--date 2026-09-01 --start 09:00 --end 10:00")
            .unwrap_err()
            .contains("--title is required"));
    }

    /// Every combination that means nothing is named rather than dropped,
    /// and the two vocabularies come from the app's own types.
    #[test]
    fn the_repeat_rules_refuse_rather_than_guess() {
        assert_eq!(repeat_for(None, None, None, None).unwrap(), None, "no flags, no repeat");

        let weekly = repeat_for(Some("weekly"), Some("MO,we, FR"), None, None).unwrap().unwrap();
        assert_eq!(weekly.weekly_days, ["MO", "WE", "FR"], "case is forgiven, order is kept");
        assert_eq!(weekly.end, crate::write::RepeatEnd::Never);

        assert_eq!(
            repeat_for(Some("daily"), None, Some("2026-12-31"), None).unwrap().unwrap().end,
            crate::write::RepeatEnd::On { date: "2026-12-31".into() }
        );
        assert_eq!(
            repeat_for(Some("monthly"), None, None, Some("3")).unwrap().unwrap().end,
            crate::write::RepeatEnd::After { count: 3 }
        );

        // The refusals, each naming the flag that is wrong.
        assert!(repeat_for(Some("fortnightly"), None, None, None)
            .unwrap_err()
            .contains("daily|weekdays|weekly|monthly|yearly"));
        assert!(repeat_for(Some("never"), None, None, None)
            .unwrap_err()
            .contains("leave --repeat off"));
        assert!(repeat_for(Some("weekdays"), Some("MO"), None, None)
            .unwrap_err()
            .contains("--days is for --repeat weekly"));
        assert!(repeat_for(None, Some("MO"), None, None).unwrap_err().contains("--days"));
        assert!(repeat_for(None, None, Some("2026-12-31"), None)
            .unwrap_err()
            .contains("needs a repeating schedule"));
        assert!(repeat_for(Some("weekly"), Some("MO,XX"), None, None)
            .unwrap_err()
            .contains("\"XX\" is not one"));
        assert!(repeat_for(Some("weekly"), None, Some("2026-12-31"), Some("4"))
            .unwrap_err()
            .contains("pass one"));
        assert!(repeat_for(Some("daily"), None, Some("31/12/2026"), None)
            .unwrap_err()
            .contains("YYYY-MM-DD"));
        assert!(repeat_for(Some("daily"), None, None, Some("0"))
            .unwrap_err()
            .contains("at least once"));
        assert!(repeat_for(Some("daily"), None, None, Some("many"))
            .unwrap_err()
            .contains("a number"));
    }

    #[test]
    fn the_occurrence_verbs_lead_with_the_id_phase_one_printed() {
        assert!(matches!(
            parse("delete", "41 --occurrence 1786352400000").unwrap(),
            WriteCmd::Delete(DeleteArgs { id: 41, occurrence: 1_786_352_400_000, scope: None })
        ));
        assert!(parse("delete", "--occurrence 1786352400000")
            .unwrap_err()
            .contains("id comes first"));
        assert!(parse("update", "41 --occurrence 1786352400000")
            .unwrap_err()
            .contains("nothing to change"));
        assert!(matches!(
            parse("respond", "41 yes").unwrap(),
            WriteCmd::Respond(RespondArgs { id: 41, .. })
        ));
        assert!(parse("bogus", "").unwrap_err().contains("unknown events subcommand"));
    }

    /// §3's scope rule, the whole matrix: recurring refuses silence,
    /// non-recurring refuses noise.
    #[test]
    fn scope_is_never_guessed() {
        assert!(scope_for(true, None).unwrap_err().contains("--scope this|following|all"));
        assert_eq!(scope_for(true, Some("this")).unwrap(), "this");
        assert_eq!(scope_for(true, Some("following")).unwrap(), "following");
        assert_eq!(scope_for(true, Some("all")).unwrap(), "all");
        assert!(scope_for(true, Some("weekly")).is_err());
        assert_eq!(scope_for(false, None).unwrap(), "this");
        assert!(scope_for(false, Some("all")).unwrap_err().contains("does not repeat"));
    }

    #[test]
    fn respond_takes_keep_meeting_as_a_bare_flag() {
        match parse("respond", "41 no --keep-meeting").unwrap() {
            WriteCmd::Respond(a) => {
                assert_eq!(a.answer, "no", "the flag is not mistaken for the answer");
                assert!(a.keep_meeting);
            }
            other => panic!("{other:?}"),
        }
        match parse("respond", "41 no").unwrap() {
            WriteCmd::Respond(a) => assert!(!a.keep_meeting),
            other => panic!("{other:?}"),
        }
    }

    fn person(email: &str, name: Option<&str>, response: &str, is_self: bool) -> omacal_store::Attendee {
        omacal_store::Attendee {
            email: email.into(),
            display_name: name.map(String::from),
            response_status: response.into(),
            optional: false,
            is_self,
            comment: None,
            additional_guests: 0,
        }
    }

    /// The people a No on your own meeting leaves expecting you — the UI's
    /// `leftOutByYourNo` table, minus the two gates `own_no_rule` reads off
    /// the event itself.
    #[test]
    fn left_out_counts_the_people_still_expecting_the_meeting() {
        let attendees = [
            person("me@x.com", None, "accepted", true),
            person("teodor@x.com", Some("Teodor"), "accepted", false),
            person("ivan@x.com", None, "needsAction", false),
            person("zoe@x.com", Some("Zoe"), "declined", false),
            person("c_4a@resource.calendar.google.com", Some("Room 4A"), "accepted", false),
            person("c_e1@group.calendar.google.com", None, "accepted", false),
        ];
        assert_eq!(left_out_by_your_no(&attendees), ["Teodor", "ivan@x.com"]);
    }

    /// The organizer's No (2026-10-26, Plamen's 1:1 with Teodor): Google
    /// mails a guest's No to the organizer, and an organizer's own No to
    /// nobody. So it is refused until the caller says which they meant —
    /// the meeting off for everyone (delete) or only for them.
    #[test]
    fn your_own_no_is_refused_until_the_meeting_is_said_to_stay() {
        use crate::events::Reach;
        let teodor = ["Teodor".to_string()];
        let hint = "omacal events delete 41 --occurrence 1793019600000";
        let err = own_no_rule("declined", Reach::Organizer, true, &teodor, false, hint).unwrap_err();
        assert!(err.contains("you organise this"), "{err}");
        assert!(err.contains("Teodor's calendar"), "{err}");
        assert!(err.contains(hint), "names the cancel: {err}");
        assert!(err.contains("--keep-meeting"), "names the way through: {err}");

        let three = ["Ana".to_string(), "Ivan".to_string(), "Petya".to_string()];
        let err = own_no_rule("declined", Reach::Organizer, true, &three, false, hint).unwrap_err();
        assert!(err.contains("3 other people's calendars"), "{err}");

        assert!(own_no_rule("declined", Reach::Organizer, true, &teodor, true, hint).is_ok(), "said");
        assert!(own_no_rule("accepted", Reach::Organizer, true, &teodor, false, hint).is_ok());
        assert!(own_no_rule("tentative", Reach::Organizer, true, &teodor, false, hint).is_ok());
        assert!(own_no_rule("declined", Reach::OwnCopy, true, &teodor, false, hint).is_ok(), "a guest");
        assert!(own_no_rule("declined", Reach::Shared, true, &teodor, false, hint).is_ok(), "a guest");
        assert!(own_no_rule("declined", Reach::Organizer, false, &teodor, false, hint).is_ok(), "CalDAV");
        assert!(own_no_rule("declined", Reach::Organizer, true, &[], false, hint).is_ok(), "nobody else");
    }

    /// A guest's own copy: the flag is unnecessary, `none` is accepted as
    /// what happens anyway, and `all` is refused with the reason. The other
    /// two reaches are the ordinary rule, unchanged.
    #[test]
    fn a_guests_own_copy_has_nobody_to_notify() {
        use crate::events::Reach;
        assert_eq!(notify_for_reach(Reach::OwnCopy, true, None).unwrap(), "none");
        assert_eq!(notify_for_reach(Reach::OwnCopy, true, Some("none")).unwrap(), "none");
        let err = notify_for_reach(Reach::OwnCopy, true, Some("all")).unwrap_err();
        assert!(err.contains("nobody to notify"), "{err}");
        assert!(notify_for_reach(Reach::Organizer, true, None).is_err(), "still asked");
        assert!(notify_for_reach(Reach::Shared, true, None).is_err(), "still asked");
        assert_eq!(notify_for_reach(Reach::Shared, true, Some("all")).unwrap(), "all");
        assert_eq!(notify_for_reach(Reach::Organizer, false, None).unwrap(), "none");
    }

    /// CalDAV mails nobody: no flag needed whatever the guest list, `none`
    /// accepted, `all` refused with the reason, a bad word still a bad word.
    #[test]
    fn a_calendar_that_mails_nobody_has_nobody_to_notify() {
        assert_eq!(notify_without_mail(None).unwrap(), "none");
        assert_eq!(notify_without_mail(Some("none")).unwrap(), "none");
        let err = notify_without_mail(Some("all")).unwrap_err();
        assert!(err.contains("nobody to notify") && err.contains("this calendar"), "{err}");
        assert!(notify_without_mail(Some("everyone")).unwrap_err().contains("all|none"));
    }

    /// The wiring behind the rule above, against a real store: an update's
    /// target and a create (named calendar or the default rule) both read
    /// the owning account's provider, and a calendar nothing can be read
    /// for falls on the asking side.
    #[tokio::test]
    async fn the_write_path_reads_the_provider_off_the_calendar() {
        let pool = omacal_store::connect_memory().await.unwrap();
        for q in [
            "INSERT INTO accounts (google_sub, email, created_at) VALUES ('s','me@x.test',0)",
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role, is_primary)
             VALUES (1, 'me@x.test', 'Work', 'UTC', 'owner', 1)",
            "INSERT INTO events (calendar_id, google_id, summary, start_utc, end_utc,
                                 start_tz, end_tz, is_all_day, status, updated_at)
             VALUES (1, 'g1', 'Book club', 1786352400000, 1786356000000,
                     'UTC', 'UTC', 0, 'confirmed', 0)",
        ] {
            sqlx::query(q).execute(&pool).await.unwrap();
        }
        assert!(target(&pool, 1).await.unwrap().mails_guests, "Google mails");
        assert!(create_mails_guests(&pool, None).await, "the default calendar is Google");

        sqlx::query("UPDATE accounts SET provider = 'caldav'").execute(&pool).await.unwrap();
        assert!(!target(&pool, 1).await.unwrap().mails_guests, "CalDAV does not");
        assert!(!create_mails_guests(&pool, Some(1)).await, "named");
        assert!(!create_mails_guests(&pool, None).await, "by the default rule");
        assert!(create_mails_guests(&pool, Some(99)).await, "unknown calendar asks");

        // WebCal feeds and on-device calendars are read as silent too — only
        // Google mails. The `== "google"` rule (not `!= "caldav"`) is what
        // keeps a new provider from accidentally mailing.
        for provider in ["webcal", "local"] {
            sqlx::query("UPDATE accounts SET provider = ?1")
                .bind(provider)
                .execute(&pool)
                .await
                .unwrap();
            assert!(
                !create_mails_guests(&pool, Some(1)).await,
                "{provider} does not mail"
            );
        }
    }

    /// §4's notify rule: guests demand an answer, solitude implies none.
    #[test]
    fn notify_is_never_guessed_where_it_matters() {
        assert!(notify_for(true, None).unwrap_err().contains("--notify all|none"));
        assert_eq!(notify_for(true, Some("all")).unwrap(), "all");
        assert_eq!(notify_for(true, Some("none")).unwrap(), "none");
        assert!(notify_for(true, Some("everyone")).is_err());
        assert_eq!(notify_for(false, None).unwrap(), "none");
    }

    /// The first field run's own bug, pinned: a time-only update must carry
    /// the title (and its siblings) it never mentioned, because the wire
    /// reads the whole `EventInput` as the caller's statement and an absent
    /// summary is honestly a cleared one.
    #[test]
    fn an_untouched_field_rides_along_instead_of_clearing() {
        let kept = merged_field(&None, &Some("CLI field test".into()));
        assert_eq!(kept.as_deref(), Some("CLI field test"));
        let changed = merged_field(&Some("Renamed".into()), &Some("Old".into()));
        assert_eq!(changed.as_deref(), Some("Renamed"));
        assert_eq!(merged_field(&None, &None), None);
    }

    #[test]
    fn answers_speak_the_humans_words_and_the_wire_speaks_googles() {
        assert_eq!(answer_word("yes").unwrap(), "accepted");
        assert_eq!(answer_word("maybe").unwrap(), "tentative");
        assert_eq!(answer_word("no").unwrap(), "declined");
        assert!(answer_word("nope").is_err());
        assert_eq!(respond_scope_for(None).unwrap(), "all");
        assert_eq!(respond_scope_for(Some("this")).unwrap(), "this");
        assert!(respond_scope_for(Some("following")).is_err());
    }

    /// Times are read in the zone the caller names — pinned here to Sofia,
    /// +03 in September — and the inclusive `--last-day` becomes the wire's
    /// exclusive `endDate`, the form's own `addDays(…, 1)`.
    #[test]
    fn time_building_is_civil_in_and_instants_out() {
        let tz = jiff::tz::TimeZone::get("Europe/Sofia").unwrap();
        let (s, e) = timed_when("2026-09-01", "09:00", "10:30", None, &tz).unwrap();
        assert_eq!(e - s, 90 * 60_000);
        // 2026-09-01 09:00 +03:00 == 2026-09-01T06:00Z.
        assert_eq!(s, 1_788_242_400_000);

        let (s2, e2) =
            timed_when("2026-09-01", "23:00", "01:00", Some("2026-09-02"), &tz).unwrap();
        assert_eq!(e2 - s2, 2 * 3_600_000, "--end-date carries a night shift across midnight");

        assert!(timed_when("2026-09-01", "10:00", "09:00", None, &tz)
            .unwrap_err()
            .contains("not after"));
        assert!(timed_when("2026-09-01", "9am", "10:00", None, &tz)
            .unwrap_err()
            .contains("HH:MM"));

        assert_eq!(
            all_day_when("2026-09-01", "2026-09-03").unwrap(),
            ("2026-09-01".to_string(), "2026-09-04".to_string()),
        );
        assert!(all_day_when("2026-09-03", "2026-09-01").is_err());
    }

    /// **The fork-proof property**: every request this module builds is
    /// accepted by `ipc::parse_request` — the server's own reader — so the
    /// two ends of the wire cannot drift apart without a test going red.
    #[test]
    fn every_built_request_parses_on_the_servers_side() {
        let tz = jiff::tz::TimeZone::get("UTC").unwrap();
        let args = CreateArgs {
            title: "Standup".into(),
            date: "2026-09-01".into(),
            start: Some("09:00".into()),
            end: Some("09:30".into()),
            guests: vec!["a@b".into()],
            ..Default::default()
        };
        let (s, e) = timed_when("2026-09-01", "09:00", "09:30", None, &tz).unwrap();
        let when = serde_json::json!({ "kind": "timed", "startMs": s, "endMs": e });
        let create = create_request(&args, when.clone(), "all", "UTC", None);
        assert!(crate::ipc::parse_request(&create.to_string()).is_ok(), "{create}");

        // The repeating shape, whose whole point is that the app already
        // reads these three fields: `parse_request` deserializes them into
        // `write::EventInput` or this goes red.
        let plan = repeat_for(Some("weekly"), Some("mo,WE"), None, Some("10")).unwrap().unwrap();
        let repeating = create_request(&args, when.clone(), "all", "UTC", Some(&plan));
        assert_eq!(repeating["fields"]["repeat"], "weekly");
        assert_eq!(repeating["fields"]["weeklyDays"], serde_json::json!(["MO", "WE"]));
        assert_eq!(
            repeating["fields"]["repeatEnd"],
            serde_json::json!({ "kind": "after", "count": 10 })
        );
        assert!(crate::ipc::parse_request(&repeating.to_string()).is_ok(), "{repeating}");

        // An unbounded repeat sends no ending at all: absent and `never`
        // mean the same thing there, and the smaller payload is the honest one.
        let plain = repeat_for(Some("weekdays"), None, None, None).unwrap().unwrap();
        let weekdays = create_request(&args, when.clone(), "all", "UTC", Some(&plain));
        assert_eq!(weekdays["fields"]["repeat"], "weekdays");
        assert!(weekdays["fields"].get("repeatEnd").is_none(), "{weekdays}");
        assert!(weekdays["fields"].get("weeklyDays").is_none(), "{weekdays}");
        assert!(crate::ipc::parse_request(&weekdays.to_string()).is_ok(), "{weekdays}");

        let update = update_request(
            &UpdateArgs {
                id: 41,
                occurrence: s,
                scope: None,
                title: Some("Standup, moved".into()),
                location: None,
                description: None,
                date: None,
                start: None,
                end: None,
                notify: None,
                meet: false,
            },
            "this",
            when,
            Some("Standup, moved"),
            None,
            None,
            "none",
            "UTC",
        );
        assert!(crate::ipc::parse_request(&update.to_string()).is_ok(), "{update}");
        assert!(update["fields"].get("conference").is_none(), "{update}");

        let delete = delete_request(41, "all", s);
        assert!(crate::ipc::parse_request(&delete.to_string()).is_ok(), "{delete}");

        let respond = respond_request(41, "accepted", "all", s);
        assert!(crate::ipc::parse_request(&respond.to_string()).is_ok(), "{respond}");
    }

    /// `--meet` is a bare flag on both verbs: it takes no value, swallows no
    /// neighbour, and counts as a change on its own for an update.
    #[test]
    fn meet_is_a_bare_flag_on_create_and_update() {
        match parse("create", "--meet --title Standup --date 2026-09-01 --start 09:00 --end 09:30")
            .unwrap()
        {
            WriteCmd::Create(a) => {
                assert!(a.meet);
                assert_eq!(a.title, "Standup", "the flag is not mistaken for a value");
            }
            other => panic!("{other:?}"),
        }
        match parse("create", "--title Standup --date 2026-09-01 --start 09:00 --end 09:30").unwrap() {
            WriteCmd::Create(a) => assert!(!a.meet),
            other => panic!("{other:?}"),
        }
        match parse("update", "41 --occurrence 1786352400000 --meet").unwrap() {
            WriteCmd::Update(a) => assert!(a.meet, "--meet alone is a change"),
            other => panic!("{other:?}"),
        }
        let err = parse("update", "41 --occurrence 1786352400000").unwrap_err();
        assert!(err.contains("--meet"), "the refusal names the new way out: {err}");
    }

    /// Where `--meet` is refused before the socket: not Google, a guest's
    /// own copy, or an event that already has a link. Everything else passes.
    #[test]
    fn meet_is_refused_where_google_cannot_or_should_not_mint_one() {
        use crate::events::Reach;
        let err = meet_rule(false, None, None).unwrap_err();
        assert!(err.contains("only Google calendars"), "{err}");
        let err = meet_rule(false, Some(Reach::Organizer), None).unwrap_err();
        assert!(err.contains("only Google calendars"), "update too: {err}");

        let err = meet_rule(true, Some(Reach::OwnCopy), None).unwrap_err();
        assert!(err.contains("organizer owns"), "{err}");

        let link = "https://meet.google.com/abc-defg-hij";
        let err = meet_rule(true, Some(Reach::Organizer), Some(link)).unwrap_err();
        assert!(err.contains(link), "the refusal names the link it found: {err}");

        assert!(meet_rule(true, None, None).is_ok(), "a create on Google");
        assert!(meet_rule(true, Some(Reach::Organizer), None).is_ok());
        assert!(meet_rule(true, Some(Reach::Shared), None).is_ok(), "guests may modify");
    }

    /// The link `meet_rule` refuses over is read the way `events list`
    /// prints it: structured conference data first, then a meeting URL
    /// already sitting in Location or Description.
    #[tokio::test]
    async fn an_events_meeting_link_is_found_wherever_it_is_held() {
        let pool = omacal_store::connect_memory().await.unwrap();
        for q in [
            "INSERT INTO accounts (google_sub, email, created_at) VALUES ('s','me@x.test',0)",
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role, is_primary)
             VALUES (1, 'me@x.test', 'Work', 'UTC', 'owner', 1)",
            "INSERT INTO events (calendar_id, google_id, summary, start_utc, end_utc,
                                 start_tz, end_tz, is_all_day, status, updated_at)
             VALUES (1, 'g1', 'Book club', 1786352400000, 1786356000000,
                     'UTC', 'UTC', 0, 'confirmed', 0)",
        ] {
            sqlx::query(q).execute(&pool).await.unwrap();
        }
        assert_eq!(target(&pool, 1).await.unwrap().conference, None, "no link anywhere");

        sqlx::query("UPDATE events SET location = 'Room 4 · Zoom: https://zoom.us/j/123456789'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            target(&pool, 1).await.unwrap().conference.as_deref(),
            Some("https://zoom.us/j/123456789"),
            "a pasted link in Location counts"
        );

        sqlx::query("UPDATE events SET conference_uri = 'https://meet.google.com/abc-defg-hij'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            target(&pool, 1).await.unwrap().conference.as_deref(),
            Some("https://meet.google.com/abc-defg-hij"),
            "structured conference data comes first"
        );
    }

    /// `--meet` speaks `EventInput.conference` exactly as the form does —
    /// proven through `ipc::parse_request`, the app's own reader — and an
    /// untouched flag sends no `conference` key at all, because on an update
    /// `"none"` would remove the meeting everyone was sent.
    #[test]
    fn meet_travels_as_the_forms_own_conference_field() {
        let tz = jiff::tz::TimeZone::get("UTC").unwrap();
        let (s, e) = timed_when("2026-09-01", "09:00", "09:30", None, &tz).unwrap();
        let when = serde_json::json!({ "kind": "timed", "startMs": s, "endMs": e });

        let mut args = CreateArgs {
            title: "Standup".into(),
            date: "2026-09-01".into(),
            start: Some("09:00".into()),
            end: Some("09:30".into()),
            meet: true,
            ..Default::default()
        };
        let create = create_request(&args, when.clone(), "none", "UTC", None);
        assert_eq!(create["fields"]["conference"], "googleMeet", "{create}");
        match crate::ipc::parse_request(&create.to_string()).unwrap() {
            crate::ipc::Request::Create { fields, .. } => assert!(
                matches!(fields.conference, Some(crate::write::ConferenceInput::GoogleMeet)),
                "{create}"
            ),
            other => panic!("{other:?}"),
        }
        args.meet = false;
        let plain = create_request(&args, when.clone(), "none", "UTC", None);
        assert!(plain["fields"].get("conference").is_none(), "{plain}");

        let mut update_args = UpdateArgs {
            id: 41,
            occurrence: s,
            scope: None,
            title: None,
            location: None,
            description: None,
            date: None,
            start: None,
            end: None,
            notify: None,
            meet: true,
        };
        let update =
            update_request(&update_args, "this", when.clone(), Some("Standup"), None, None, "none", "UTC");
        assert_eq!(update["fields"]["conference"], "googleMeet", "{update}");
        match crate::ipc::parse_request(&update.to_string()).unwrap() {
            crate::ipc::Request::Update { fields, .. } => assert!(
                matches!(fields.conference, Some(crate::write::ConferenceInput::GoogleMeet)),
                "{update}"
            ),
            other => panic!("{other:?}"),
        }
        update_args.meet = false;
        let plain = update_request(&update_args, "this", when, Some("Standup"), None, None, "none", "UTC");
        assert!(plain["fields"].get("conference").is_none(), "{plain}");
    }

    /// The link is read from the app's real answer — this envelope is the
    /// shape a `--meet` create came back with against Google, trimmed.
    #[test]
    fn the_minted_link_is_read_from_the_apps_own_answer() {
        let answer = serde_json::json!({
            "ok": true,
            "data": {
                "id": 975, "calendar_id": 5, "title": "Meet test",
                "conference_uri": "https://meet.google.com/tpr-mvez-srw",
                "is_organizer": true, "mails_guests": true, "attendees": []
            }
        });
        assert_eq!(minted_link(&answer), Some("https://meet.google.com/tpr-mvez-srw"));
        let none = serde_json::json!({ "ok": true, "data": { "id": 976, "conference_uri": null } });
        assert_eq!(minted_link(&none), None, "a save with no link reads as none, not an error");
    }
}
