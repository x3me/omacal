//! `omacal tasks` — the task half of the CLI.
//!
//! Same division as the events side: the **list is a read**, straight off
//! the synced database with no app running, and every **change goes over
//! the socket** into the app's own write path, so a task the CLI creates
//! passes exactly the guards a task the window creates does.
//!
//! The verbs are deliberately fewer than the window's. An agent's job here
//! is to answer "what do I still have to do" and to put something on the
//! list; the shapes a person wants a pointer and a calendar for — moving a
//! task between lists, priorities, recurrence — are not offered rather than
//! half-offered.

use crate::cli::{fail, EXIT_USAGE};

/// Which list `--list` names: its id, or its name as `omacal tasks lists`
/// prints it. A number is always an id.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ListRef {
    Id(i64),
    Name(String),
}

/// `--list`'s answer against the lists a task can go on: an id passes as it
/// is (the app refuses one that is not a task list), and a name must match
/// exactly one list, any case — never the nearest, and never one of two.
pub(crate) fn resolve_list(
    list: &Option<ListRef>,
    lists: &[crate::tasks::TaskListVm],
) -> Result<Option<i64>, String> {
    match list {
        None => Ok(None),
        Some(ListRef::Id(id)) => Ok(Some(*id)),
        Some(ListRef::Name(name)) => {
            let wanted = name.trim().to_lowercase();
            let hits: Vec<&crate::tasks::TaskListVm> =
                lists.iter().filter(|l| l.name.to_lowercase() == wanted).collect();
            match hits.as_slice() {
                [one] => Ok(Some(one.calendar_id)),
                [] => Err(format!("no task list is called “{name}” — `omacal tasks lists` names them")),
                _ => Err(format!(
                    "more than one list is called “{name}” — use its id from `omacal tasks lists`"
                )),
            }
        }
    }
}

/// `--priority`'s four words. Words, not numbers: the RFC's `1` is the
/// *highest* priority, which reads as "first/low" to most people, so a bare
/// integer is a trap. `None` is the word for clearing, not an absent flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PriorityArg {
    None,
    Low,
    Medium,
    High,
}

impl PriorityArg {
    /// The raw wire integer the app stores: the three levels' values from
    /// [`omacal_caldav::TaskPriority`], and `0` for no priority.
    fn wire(self) -> i64 {
        match self {
            PriorityArg::None => 0,
            PriorityArg::Low => omacal_caldav::TaskPriority::Low.wire(),
            PriorityArg::Medium => omacal_caldav::TaskPriority::Medium.wire(),
            PriorityArg::High => omacal_caldav::TaskPriority::High.wire(),
        }
    }
}

/// `--priority none|low|medium|high`, case-insensitive.
fn priority_of(word: &str) -> Result<PriorityArg, String> {
    match word.to_ascii_lowercase().as_str() {
        "none" => Ok(PriorityArg::None),
        "low" => Ok(PriorityArg::Low),
        "medium" => Ok(PriorityArg::Medium),
        "high" => Ok(PriorityArg::High),
        other => Err(format!("--priority takes none, low, medium or high, not \"{other}\"")),
    }
}

/// One change to a task, as the CLI parses it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TaskCmd {
    Add {
        summary: String,
        /// Absent means the first list a task can be created on.
        list: Option<ListRef>,
        due: Option<String>,
        at: Option<String>,
        priority: Option<PriorityArg>,
    },
    /// `done` and `reopen` are one verb with a flag on the wire; two words
    /// here because "mark it not-done" is not a thing anyone types.
    Complete { id: i64, done: bool },
    Edit {
        id: i64,
        title: Option<String>,
        /// `Some(None)` is `--due none`: the date goes. Absent leaves it.
        due: Option<Option<String>>,
        at: Option<Option<String>>,
        notes: Option<Option<String>>,
        /// Absent (`None`) leaves the task's priority alone; the word `none`
        /// is [`PriorityArg::None`], which clears it.
        priority: Option<PriorityArg>,
    },
}

/// `--due 2026-09-11` and `--at 18:00` into the instant they name, in the
/// machine's own zone, plus whether it is a whole day.
///
/// A time with no date is refused rather than assumed onto today: the
/// difference between "due at six" and "due today at six" is a day, and
/// guessing it is the kind of help nobody asked for.
pub(crate) fn due_at(
    due: Option<&str>,
    at: Option<&str>,
    tz: &jiff::tz::TimeZone,
) -> Result<(Option<i64>, bool), String> {
    let Some(date) = due else {
        return match at {
            Some(_) => Err("--at needs --due: a time with no day is not a due date".into()),
            None => Ok((None, true)),
        };
    };
    let date: jiff::civil::Date = date
        .parse()
        .map_err(|_| format!("--due takes YYYY-MM-DD, not \"{date}\""))?;
    let Some(time) = at else {
        let ms = date
            .to_datetime(jiff::civil::time(0, 0, 0, 0))
            .to_zoned(tz.clone())
            .map_err(|e| e.to_string())?
            .timestamp()
            .as_millisecond();
        return Ok((Some(ms), true));
    };
    let time: jiff::civil::Time = time
        .parse()
        .map_err(|_| format!("--at takes HH:MM, not \"{time}\""))?;
    let ms = date
        .to_datetime(time)
        .to_zoned(tz.clone())
        .map_err(|e| e.to_string())?
        .timestamp()
        .as_millisecond();
    Ok((Some(ms), false))
}

/// `omacal tasks add|done|reopen|edit …`, or `None` for the bare read.
pub(crate) fn parse(rest: &[&String]) -> Option<Result<TaskCmd, String>> {
    // The verb is the first word that is not a flag: `tasks --all --json`
    // is the read, and only `tasks add …` and friends are changes.
    let i = rest.iter().position(|a| !a.starts_with("--"))?;
    let verb = rest[i].as_str();
    let args = &rest[i + 1..];

    let take = |name: &str| -> Result<Option<String>, String> {
        let mut it = args.iter();
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
    /// `--flag none` clears; absent leaves alone; a value sets.
    fn clearable(v: Option<String>) -> Option<Option<String>> {
        v.map(|s| if s.eq_ignore_ascii_case("none") { None } else { Some(s) })
    }
    let positional = || -> Option<String> {
        args.iter().find(|a| !a.starts_with("--")).map(|s| (*s).clone())
    };
    let id_of = |what: &str| -> Result<i64, String> {
        positional()
            .and_then(|v| v.parse::<i64>().ok())
            .ok_or_else(|| format!("usage: omacal tasks {what} ID — `omacal tasks` prints ids"))
    };

    Some((|| {
        match verb {
            "add" => {
                let summary = positional()
                    .ok_or_else(|| "usage: omacal tasks add \"a title\" [--list ID|NAME] [--due YYYY-MM-DD] [--at HH:MM] [--priority none|low|medium|high]".to_string())?;
                let list = take("--list")?.map(|v| match v.parse::<i64>() {
                    Ok(id) => ListRef::Id(id),
                    Err(_) => ListRef::Name(v),
                });
                let priority = match take("--priority")? {
                    Some(w) => Some(priority_of(&w)?),
                    None => None,
                };
                Ok(TaskCmd::Add { summary, list, due: take("--due")?, at: take("--at")?, priority })
            }
            "done" => Ok(TaskCmd::Complete { id: id_of("done")?, done: true }),
            "reopen" => Ok(TaskCmd::Complete { id: id_of("reopen")?, done: false }),
            "edit" => {
                let id = id_of("edit")?;
                let priority = match take("--priority")? {
                    Some(w) => Some(priority_of(&w)?),
                    None => None,
                };
                let cmd = TaskCmd::Edit {
                    id,
                    title: take("--title")?,
                    due: clearable(take("--due")?),
                    at: clearable(take("--at")?),
                    notes: clearable(take("--notes")?),
                    priority,
                };
                let TaskCmd::Edit { title, due, at, notes, priority, .. } = &cmd else { unreachable!() };
                if title.is_none() && due.is_none() && at.is_none() && notes.is_none() && priority.is_none() {
                    return Err(
                        "omacal tasks edit ID needs something to change: --title, --due, --at, --notes or --priority \
                         (--due none clears the date)"
                            .into(),
                    );
                }
                Ok(cmd)
            }
            other => Err(format!(
                "usage: omacal tasks [add|done|reopen|edit] — not \"{other}\""
            )),
        }
    })())
}

/// Runs one task change: fills in whatever the edit did not say from the
/// task as it stands, then hands the whole state to the app.
///
/// Reading the current task first is what lets `edit` take one field at a
/// time while the app's own command takes the complete state — the CLI is
/// the layer that knows what "leave the rest alone" means, and it says so
/// by naming every field.
pub(crate) async fn execute(pool: &sqlx::SqlitePool, cmd: &TaskCmd, json: bool) -> i32 {
    let tz = jiff::tz::TimeZone::system();
    let refuse = |m: &str| fail(json, "usage", m, EXIT_USAGE);

    let (request, done_word) = match cmd {
        TaskCmd::Add { summary, list, due, at, priority } => {
            if summary.trim().is_empty() {
                return refuse("a task needs a title");
            }
            let (due_ms, all_day) = match due_at(due.as_deref(), at.as_deref(), &tz) {
                Ok(v) => v,
                Err(m) => return refuse(&m),
            };
            let list = match list {
                Some(ListRef::Name(_)) => match crate::tasks::writable_task_lists(pool).await {
                    Ok(lists) => match resolve_list(list, &lists) {
                        Ok(id) => id,
                        Err(m) => return refuse(&m),
                    },
                    Err(e) => return fail(json, "read_failed", &e.to_string(), crate::cli::EXIT_ERROR),
                },
                other => resolve_list(other, &[]).unwrap_or(None),
            };
            // 0 and absent both mean "no priority" on a create.
            (create_request(list, summary, due_ms, all_day, priority.map(PriorityArg::wire)), "Added")
        }
        TaskCmd::Complete { id, done } => {
            (complete_request(*id, *done), if *done { "Completed" } else { "Reopened" })
        }
        TaskCmd::Edit { id, title, due, at, notes, priority } => {
            let task = match omacal_store::task_by_id(pool, *id).await {
                Ok(Some(t)) => t,
                Ok(None) => return refuse("no task with that id — `omacal tasks` prints them"),
                Err(e) => return fail(json, "read_failed", &e.to_string(), crate::cli::EXIT_ERROR),
            };

            let summary = title.clone().unwrap_or_else(|| task.summary.clone().unwrap_or_default());
            if summary.trim().is_empty() {
                return refuse("a task needs a title");
            }
            let notes = match notes {
                Some(n) => n.clone(),
                None => task.description.clone(),
            };

            // The date and the time are one answer, so an edit naming only
            // one takes the other from the task as it stands.
            let (due_ms, all_day) = match resolve_edit_due(&task, due, at, &tz) {
                Ok(v) => v,
                Err(m) => return refuse(&m),
            };
            // Naming nothing leaves the value: the CLI sends the task's own
            // raw value through; a named word sets or clears.
            let priority = match priority {
                Some(word) => Some(word.wire()),
                None => Some(task.priority),
            };
            (update_request(*id, &summary, due_ms, all_day, notes.as_deref(), priority), "Saved")
        }
    };

    crate::cli_write::send(&request, json, done_word)
}

/// The three requests above, in the envelope every socket write speaks —
/// `crate::ipc::Request`'s own shape (`v` + `cmd`, snake_case fields), the
/// same one `cli_write.rs` builds for the events side. These used to carry
/// `kind` and camelCase field names instead, a leftover of the webview's own
/// vocabulary that the socket never spoke: every `omacal tasks add/edit`
/// refused with "the request names no protocol version", silently, since
/// the feature shipped.
pub(crate) fn create_request(
    list: Option<i64>,
    summary: &str,
    due_ms: Option<i64>,
    due_all_day: bool,
    priority: Option<i64>,
) -> serde_json::Value {
    serde_json::json!({
        "v": crate::ipc::PROTOCOL_VERSION,
        "cmd": "tasks-create",
        "calendar_id": list,
        "summary": summary,
        "due_ms": due_ms,
        "due_all_day": due_all_day,
        "priority": priority,
    })
}

pub(crate) fn complete_request(id: i64, done: bool) -> serde_json::Value {
    serde_json::json!({
        "v": crate::ipc::PROTOCOL_VERSION,
        "cmd": "tasks-complete",
        "id": id,
        "done": done,
    })
}

pub(crate) fn update_request(
    id: i64,
    summary: &str,
    due_ms: Option<i64>,
    due_all_day: bool,
    notes: Option<&str>,
    priority: Option<i64>,
) -> serde_json::Value {
    serde_json::json!({
        "v": crate::ipc::PROTOCOL_VERSION,
        "cmd": "tasks-update",
        "id": id,
        "summary": summary,
        "due_ms": due_ms,
        "due_all_day": due_all_day,
        "notes": notes,
        "priority": priority,
    })
}

/// The due date an `edit` means, given what it named and what the task
/// already had.
///
/// `--due none` clears the date, and clears any time with it: a time on no
/// day is not a due date. `--at none` keeps the day and drops the hour,
/// which is how a task goes from "by six" back to "some time that day".
pub(crate) fn resolve_edit_due(
    task: &omacal_store::StoredTask,
    due: &Option<Option<String>>,
    at: &Option<Option<String>>,
    tz: &jiff::tz::TimeZone,
) -> Result<(Option<i64>, bool), String> {
    if matches!(due, Some(None)) {
        return Ok((None, true));
    }
    let current = task.due_utc.map(|ms| {
        let z = jiff::Timestamp::from_millisecond(ms)
            .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
            .to_zoned(tz.clone());
        // An all-day due is its date in the list's zone, not its stored
        // midnight read here: a Sofia list's Thursday is Wednesday evening
        // in New York, and an edit naming only the title kept Wednesday.
        let date = match (task.due_all_day, task.due_tz.as_deref()) {
            (true, Some(zone)) => crate::tasks::due_date(ms, Some(zone)),
            _ => z.date(),
        };
        (date.to_string(), format!("{:02}:{:02}", z.hour(), z.minute()))
    });

    let date = match due {
        Some(Some(d)) => Some(d.clone()),
        _ => current.as_ref().map(|(d, _)| d.clone()),
    };
    let Some(date) = date else {
        // Nothing said a day and the task has none: `--at` alone cannot
        // invent one.
        return match at {
            Some(Some(_)) => Err("--at needs --due: a time with no day is not a due date".into()),
            _ => Ok((None, true)),
        };
    };
    let time = match at {
        Some(None) => None,
        Some(Some(t)) => Some(t.clone()),
        None => {
            if task.due_all_day { None } else { current.map(|(_, t)| t) }
        }
    };
    due_at(Some(&date), time.as_deref(), tz)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }
    fn p(s: &str) -> Result<TaskCmd, String> {
        let owned = argv(s);
        let refs: Vec<&String> = owned.iter().collect();
        parse(&refs).expect("a verb")
    }

    #[test]
    fn the_verbs_parse_and_the_mistakes_are_named() {
        assert_eq!(
            p("add Renew --due 2026-09-11 --at 18:00"),
            Ok(TaskCmd::Add {
                summary: "Renew".into(),
                list: None,
                due: Some("2026-09-11".into()),
                at: Some("18:00".into()),
                priority: None,
            })
        );
        assert_eq!(p("done 41"), Ok(TaskCmd::Complete { id: 41, done: true }));
        assert_eq!(p("reopen 41"), Ok(TaskCmd::Complete { id: 41, done: false }));

        // `none` clears; a value sets; an absent flag leaves alone.
        assert_eq!(
            p("edit 41 --due none --notes hello"),
            Ok(TaskCmd::Edit {
                id: 41,
                title: None,
                due: Some(None),
                at: None,
                notes: Some(Some("hello".into())),
                priority: None,
            })
        );

        // `--priority` is words, case-insensitive, and `none` clears.
        assert_eq!(
            p("add Thing --priority high"),
            Ok(TaskCmd::Add {
                summary: "Thing".into(),
                list: None,
                due: None,
                at: None,
                priority: Some(PriorityArg::High),
            })
        );
        assert_eq!(
            p("edit 41 --priority none"),
            Ok(TaskCmd::Edit {
                id: 41,
                title: None,
                due: None,
                at: None,
                notes: None,
                priority: Some(PriorityArg::None),
            })
        );
        // Every word maps to its level, in any case.
        for (word, level) in [
            ("low", PriorityArg::Low),
            ("medium", PriorityArg::Medium),
            ("HIGH", PriorityArg::High),
        ] {
            assert_eq!(
                p(&format!("edit 41 --priority {word}")),
                Ok(TaskCmd::Edit {
                    id: 41, title: None, due: None, at: None, notes: None, priority: Some(level),
                }),
                "--priority {word}"
            );
        }
        assert!(p("add Thing --priority urgent").unwrap_err().contains("none, low, medium or high"));

        assert!(p("add").is_err(), "a title is not optional");
        assert!(p("done").is_err(), "an id is not optional");
        assert!(p("edit 41").is_err(), "an edit that changes nothing is a usage error");
        assert!(p("wobble").unwrap_err().contains("add|done|reopen|edit"));
        // Flags are not verbs: `tasks --all --json` is the read.
        let flags = argv("--all --json");
        let refs: Vec<&String> = flags.iter().collect();
        assert!(parse(&refs).is_none(), "no verb means the read");
        // A number is a list id, and anything else a list's name.
        assert!(matches!(p("add Thing --list 9"), Ok(TaskCmd::Add { list: Some(ListRef::Id(9)), .. })));
        assert!(matches!(p("add Thing --list nine"), Ok(TaskCmd::Add { list: Some(ListRef::Name(ref n)), .. }) if n == "nine"));
    }

    /// A day alone is a whole day; a day and a time is an instant; a time
    /// with no day is refused rather than assumed onto today.
    #[test]
    fn a_due_date_needs_its_day() {
        let tz = jiff::tz::TimeZone::get("Asia/Kolkata").unwrap();
        let (ms, all_day) = due_at(Some("2026-09-11"), None, &tz).unwrap();
        assert!(all_day);
        // Midnight in Kolkata, not in UTC.
        assert_eq!(ms, Some("2026-09-10T18:30:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond()));

        let (ms, all_day) = due_at(Some("2026-09-11"), Some("18:00"), &tz).unwrap();
        assert!(!all_day);
        assert_eq!(ms, Some("2026-09-11T12:30:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond()));

        assert_eq!(due_at(None, None, &tz).unwrap(), (None, true));
        assert!(due_at(None, Some("18:00"), &tz).unwrap_err().contains("--at needs --due"));
        assert!(due_at(Some("11/09/2026"), None, &tz).unwrap_err().contains("YYYY-MM-DD"));
        assert!(due_at(Some("2026-09-11"), Some("six"), &tz).unwrap_err().contains("HH:MM"));
    }

    fn stored(due_utc: Option<i64>, all_day: bool) -> omacal_store::StoredTask {
        omacal_store::StoredTask {
            id: 1, calendar_id: 1, uid: "u".into(), etag: None, caldav_href: None,
            summary: Some("t".into()), description: None, due_utc, due_tz: None,
            due_all_day: all_day, status: "needs-action".into(), completed_utc: None,
            priority: 0, raw_ics: None, updated_at: 0,
        }
    }

    /// `--list Groceries`: exactly one list by that name, in any case, or a
    /// refusal that says where the names are — never the closest match.
    #[test]
    fn a_list_is_named_exactly_once_or_refused() {
        let list = |id: i64, name: &str| crate::tasks::TaskListVm {
            calendar_id: id, name: name.into(), color: None, local: true,
        };
        let lists = [list(3, "Groceries"), list(4, "Work"), list(5, "work")];
        let name = |n: &str| Some(ListRef::Name(n.into()));
        assert_eq!(resolve_list(&name("groceries"), &lists), Ok(Some(3)));
        assert_eq!(resolve_list(&name(" Groceries "), &lists), Ok(Some(3)));
        assert!(resolve_list(&name("Grocer"), &lists).unwrap_err().contains("no task list is called “Grocer”"));
        assert!(resolve_list(&name("WORK"), &lists).unwrap_err().contains("more than one list"));
        assert_eq!(resolve_list(&Some(ListRef::Id(42)), &lists), Ok(Some(42)), "an id passes; the app judges it");
        assert_eq!(resolve_list(&None, &lists), Ok(None));
    }

    /// The audit's zone drift, in the CLI's edit: an all-day task on a Sofia
    /// list read from New York keeps its Thursday when only the title changes.
    #[test]
    fn an_edit_keeps_an_all_day_date_from_another_zone() {
        let ny = jiff::tz::TimeZone::get("America/New_York").unwrap();
        // DUE;VALUE=DATE:20260917 on a Sofia list: midnight there.
        let mut sofia_thursday = stored(Some("2026-09-16T21:00:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond()), true);
        sofia_thursday.due_tz = Some("Europe/Sofia".into());
        let (ms, all_day) = resolve_edit_due(&sofia_thursday, &None, &None, &ny).unwrap();
        assert!(all_day);
        let thursday_ny = "2026-09-17T04:00:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond();
        assert_eq!(ms, Some(thursday_ny), "Thursday, sent as the CLI's own midnight for the app to date");
    }

    /// An edit naming one half of the date takes the other from the task,
    /// and the two clears mean different things.
    #[test]
    fn an_edit_fills_the_half_it_was_not_given() {
        let tz = jiff::tz::TimeZone::get("Asia/Kolkata").unwrap();
        let at_six = "2026-09-11T12:30:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond();
        let timed = stored(Some(at_six), false);

        // A new day keeps the hour the task already had.
        let (ms, all_day) =
            resolve_edit_due(&timed, &Some(Some("2026-09-12".into())), &None, &tz).unwrap();
        assert!(!all_day);
        assert_eq!(ms, Some("2026-09-12T12:30:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond()));

        // `--at none` keeps the day and drops the hour.
        let (_, all_day) = resolve_edit_due(&timed, &None, &Some(None), &tz).unwrap();
        assert!(all_day);

        // `--due none` clears both.
        assert_eq!(resolve_edit_due(&timed, &Some(None), &None, &tz).unwrap(), (None, true));

        // An all-day task given a time becomes timed on its own day.
        let day = stored(Some("2026-09-10T18:30:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond()), true);
        let (ms, all_day) = resolve_edit_due(&day, &None, &Some(Some("09:15".into())), &tz).unwrap();
        assert!(!all_day);
        assert_eq!(ms, Some("2026-09-11T03:45:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond()));

        // A task with no date, given only a time, is refused.
        let none = stored(None, true);
        assert!(resolve_edit_due(&none, &None, &Some(Some("09:15".into())), &tz)
            .unwrap_err()
            .contains("--at needs --due"));
    }

    /// The bug this module shipped with: every request here used to carry
    /// `kind` and camelCase fields — the webview's own vocabulary, not the
    /// socket's. `crate::ipc::parse_request` is the actual, only parser on
    /// the other end, and it speaks `v` + `cmd` + snake_case
    /// (`crate::ipc::Request`'s own derive). A request this module cannot
    /// get past that parser is not a "no task list" or "no such task"
    /// refusal — it never reaches those checks — it is silently the wrong
    /// shape, and `omacal tasks add/edit/done` answered "the request names
    /// no protocol version" for every caller since the feature shipped.
    /// Proving these three round-trip through the real parser is the
    /// regression test: a return to `kind`/camelCase fails here before it
    /// ever reaches a user's terminal.
    #[test]
    fn every_request_this_module_builds_parses_on_the_sockets_own_terms() {
        match crate::ipc::parse_request(&create_request(Some(3), "Water plants", Some(1_000), false, Some(1)).to_string()) {
            Ok(crate::ipc::Request::TaskCreate { calendar_id, summary, due_ms, due_all_day, priority }) => {
                assert_eq!(calendar_id, Some(3));
                assert_eq!(summary, "Water plants");
                assert_eq!(due_ms, Some(1_000));
                assert!(!due_all_day);
                assert_eq!(priority, Some(1), "the priority rides the create");
            }
            other => panic!("create_request did not parse as TaskCreate: {other:?}"),
        }

        match crate::ipc::parse_request(&complete_request(41, true).to_string()) {
            Ok(crate::ipc::Request::TaskComplete { id, done }) => {
                assert_eq!(id, 41);
                assert!(done);
            }
            other => panic!("complete_request did not parse as TaskComplete: {other:?}"),
        }

        match crate::ipc::parse_request(
            &update_request(41, "Water plants twice", None, true, Some("weekly"), Some(5)).to_string(),
        ) {
            Ok(crate::ipc::Request::TaskUpdate { id, summary, due_ms, due_all_day, notes, priority }) => {
                assert_eq!(id, 41);
                assert_eq!(summary, "Water plants twice");
                assert_eq!(due_ms, None);
                assert!(due_all_day);
                assert_eq!(notes.as_deref(), Some("weekly"));
                assert_eq!(priority, Some(5), "the priority rides the update");
            }
            other => panic!("update_request did not parse as TaskUpdate: {other:?}"),
        }

        // The tri-state, on the wire: absent leaves, an explicit 0 clears.
        let absent = serde_json::json!({
            "v": crate::ipc::PROTOCOL_VERSION, "cmd": "tasks-update",
            "id": 41, "summary": "x", "due_all_day": true,
        }).to_string();
        match crate::ipc::parse_request(&absent) {
            Ok(crate::ipc::Request::TaskUpdate { priority, .. }) => {
                assert_eq!(priority, None, "absent leaves the priority alone");
            }
            other => panic!("an absent priority must still parse: {other:?}"),
        }
        let clear = serde_json::json!({
            "v": crate::ipc::PROTOCOL_VERSION, "cmd": "tasks-update",
            "id": 41, "summary": "x", "due_all_day": true, "priority": 0,
        }).to_string();
        match crate::ipc::parse_request(&clear) {
            Ok(crate::ipc::Request::TaskUpdate { priority, .. }) => {
                assert_eq!(priority, Some(0), "an explicit 0 is a clear, not an absence");
            }
            other => panic!("an explicit 0 must parse: {other:?}"),
        }
        // A create with no priority at all parses too, as no priority.
        let create_absent = serde_json::json!({
            "v": crate::ipc::PROTOCOL_VERSION, "cmd": "tasks-create", "summary": "x",
        }).to_string();
        match crate::ipc::parse_request(&create_absent) {
            Ok(crate::ipc::Request::TaskCreate { priority, .. }) => assert_eq!(priority, None),
            other => panic!("a create with no priority must parse: {other:?}"),
        }
    }
}
