//! The read-only CLI: the calendar omacal already syncs, answerable from a
//! terminal — and so from a script, a bar module, or an agent.
//!
//! `omacal agenda`, `events list`, `search`, `calendars` — reads, and reads
//! only. Phase 1 stops deliberately short of writes: an agent creating an
//! event must inherit the same guards the form has (who gets mailed, etag
//! conflicts, recurrence scopes), and those live in the running app —
//! writes arrive later over the single-instance IPC the bar widget already
//! drives. Until then the sharpest promise this surface can make is that it
//! cannot damage anything.
//!
//! It runs *before* Tauri exists: no window, no tray, no single-instance
//! forwarding — `omacal agenda` in a terminal must never wake a GUI or be
//! swallowed by the running one. The database is opened read-only against
//! the file the app maintains (WAL keeps that safe beside the app's own
//! writes), and a missing database is an answer, not a crash: exit 3,
//! "launch omacal first".
//!
//! Two output registers, hey-cli's discipline: `--json` prints an envelope
//! (`{"ok":true,"data":…}` / `{"ok":false,"error":…}`) and never prompts,
//! never decorates; without it, output is for a person. Exit codes are
//! stable and documented on `omacal cli-help`:
//! 0 ok · 2 usage · 3 no database · 4 error.
//!
//! Split as ever: parsing, date windows and row assembly are pure and
//! tested; the runtime, the file open and the printing are the thin
//! untested shell.

use serde::Serialize;
use sqlx::SqlitePool;

pub(crate) const EXIT_OK: i32 = 0;
pub(crate) const EXIT_USAGE: i32 = 2;
const EXIT_NO_DB: i32 = 3;
pub(crate) const EXIT_ERROR: i32 = 4;

const USAGE: &str = "\
omacal CLI — your calendar from the terminal: reads answer offline from
the synced database; writes execute through the running app's own guards.

USAGE
  omacal agenda [--days N] [--json]        the next N days (default 7)
  omacal events list --from YYYY-MM-DD --to YYYY-MM-DD [--json]
  omacal events show ID [--json]           one event whole: guest list with
                                           each person's answer, join link,
                                           organizer, description
  omacal search <query> [--json]           titles, nearest to today first
  omacal calendars [--json]                every calendar, with ids
  omacal doctor [--json]                   diagnose this install
  omacal skill                             print the agent skill this binary carries
  omacal skill install                     install it for your agents (Claude Code linked
                                           automatically; refreshed on every update)
  omacal commands [--json]                 every command and flag, machine-readable
  omacal cli-help                          this text

WRITES (the running app executes them, through its own guards)
  omacal events create --title T --date D --start HH:MM --end HH:MM
         [--end-date D] [--all-day --last-day D] [--calendar ID]
         [--location L] [--description TEXT] [--guest a@b]…
         [--notify all|none] [--json]
  omacal events update ID --occurrence MS [--title T] [--date D]
         [--start HH:MM] [--end HH:MM] [--location L] [--description TEXT]
         [--scope this|following|all] [--notify all|none] [--json]
  omacal events delete ID --occurrence MS [--scope this|following|all] [--json]
  omacal events respond ID yes|maybe|no [--scope this|all] [--occurrence MS] [--json]

  ID and MS are `events list --json`'s own eventId and startMs. Times read
  in omacal's display zone. A repeating event needs --scope said out loud;
  an event with guests needs --notify said out loud — neither is guessed.

UPDATING
  The app updates itself: when a release exists its header grows an Update
  button. `omacal doctor` prints the version you are on. An AppImage
  installed by the install script is also replaced by re-running that line;
  a .deb or .rpm is replaced by the newer package.

OUTPUT
  --json prints {\"ok\":true,\"data\":…} on success and
  {\"ok\":false,\"error\":{\"code\",\"message\"}} on failure; nothing prompts.

EXIT CODES
  0 ok · 2 usage error · 3 no database (launch omacal and connect an
  account first) · 4 internal error · 5 omacal is not running (writes
  need it) · 6 the app refused the write (a conflict, a guard — read the
  message, change the request; retrying as-is changes nothing)";

#[derive(Debug, PartialEq)]
pub(crate) enum Command {
    Agenda { days: u32 },
    Events { from: jiff::civil::Date, to: jiff::civil::Date },
    /// One event, whole — the guest list with each person's answer, which
    /// the window rows deliberately compress to a count. Read-only like
    /// every read here, straight off the local database.
    Show { id: i64 },
    Search { query: String },
    Calendars,
    Doctor,
    Help,
    /// The write verbs, whole — parsed, prechecked and executed by
    /// `cli_write.rs`, over the running app's socket. This module's
    /// read-only doctrine holds: nothing behind this variant opens the
    /// database for writing either. Boxed for clippy's variant-size rule:
    /// a create carries a form's worth of fields and every other variant
    /// carries a date or two.
    Write(Box<crate::cli_write::WriteCmd>),
    /// Print (`skill`) or install (`skill install`) the agent skill the
    /// binary carries — `cli_skill.rs`, the 37signals pattern. Needs no
    /// database: handled beside Help, before anything is opened.
    Skill { install: bool },
    /// The machine-readable catalog of every command and flag — agent
    /// discovery beyond what the skill narrates (basecamp-cli's
    /// `commands --json`, adopted). No database either.
    Commands,
}

/// One catalog row. `writes` means *calendar* writes — the two-tier trust
/// question an agent asks first — not "touches any file" (`skill install`
/// writes skill files and is still `false` here; its description says so).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandInfo {
    pub name: &'static str,
    pub usage: &'static str,
    pub description: &'static str,
    pub writes: bool,
    pub flags: &'static [&'static str],
}

/// Every command this parser answers to, as data. Hand-kept beside the
/// parser it describes; `the_catalog_matches_the_parser` holds the two
/// together, which is this file's version of basecamp-cli's drift check.
pub(crate) fn command_catalog() -> Vec<CommandInfo> {
    vec![
        CommandInfo { name: "agenda", usage: "agenda [--days N]", writes: false,
            description: "the next N days (default 7), all accounts, offline",
            flags: &["--days", "--json"] },
        CommandInfo { name: "events list", usage: "events list --from D --to D", writes: false,
            description: "expanded occurrences in a date window, ids included",
            flags: &["--from", "--to", "--json"] },
        CommandInfo { name: "events show", usage: "events show ID", writes: false,
            description: "one event whole: guests with answers, organizer, join link",
            flags: &["--json"] },
        CommandInfo { name: "events create", usage: "events create --title T --date D …", writes: true,
            description: "create through the running app's guards",
            flags: &["--title", "--date", "--start", "--end", "--end-date", "--all-day",
                     "--last-day", "--calendar", "--location", "--description", "--guest",
                     "--notify", "--json"] },
        CommandInfo { name: "events update", usage: "events update ID --occurrence MS …", writes: true,
            description: "reschedule or retitle; scope and notify never guessed",
            flags: &["--occurrence", "--scope", "--title", "--date", "--start", "--end",
                     "--location", "--description", "--notify", "--json"] },
        CommandInfo { name: "events delete", usage: "events delete ID --occurrence MS", writes: true,
            description: "delete at a scope, through the app",
            flags: &["--occurrence", "--scope", "--json"] },
        CommandInfo { name: "events respond", usage: "events respond ID yes|maybe|no", writes: true,
            description: "answer an invitation",
            flags: &["--scope", "--occurrence", "--json"] },
        CommandInfo { name: "search", usage: "search <query>", writes: false,
            description: "titles, nearest to today first",
            flags: &["--json"] },
        CommandInfo { name: "calendars", usage: "calendars", writes: false,
            description: "every calendar with ids and accounts",
            flags: &["--json"] },
        CommandInfo { name: "doctor", usage: "doctor", writes: false,
            description: "diagnose this install",
            flags: &["--json"] },
        CommandInfo { name: "skill", usage: "skill [install]", writes: false,
            description: "print the embedded agent skill, or install it for your agents (writes skill files only, never the calendar)",
            flags: &["--json"] },
        CommandInfo { name: "commands", usage: "commands", writes: false,
            description: "this catalog",
            flags: &["--json"] },
        CommandInfo { name: "cli-help", usage: "cli-help", writes: false,
            description: "usage, exit codes, the whole contract",
            flags: &[] },
    ]
}

#[derive(Debug, PartialEq)]
pub(crate) struct Invocation {
    pub command: Command,
    pub json: bool,
}

/// What of `argv` is the CLI's. `None` means "not ours" — a bare launch, a
/// date, the tray flags — and the GUI path proceeds untouched, which is the
/// property that keeps this module unable to break anything that exists.
/// `Some(Err(...))` is a recognised subcommand used wrongly: usage text and
/// exit 2, never a fall-through into a GUI the user did not ask for.
pub(crate) fn parse(argv: &[String]) -> Option<Result<Invocation, String>> {
    let mut args = argv.iter().skip(1); // argv[0] is the binary
    let sub = args.next()?;
    let rest: Vec<&String> = args.collect();

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
    let json = rest.iter().any(|a| a.as_str() == "--json");

    let build = |command: Command| Some(Ok(Invocation { command, json }));

    match sub.as_str() {
        "cli-help" => build(Command::Help),
        "commands" => build(Command::Commands),
        "skill" => match rest.iter().find(|a| !a.starts_with("--")).map(|s| s.as_str()) {
            None => build(Command::Skill { install: false }),
            Some("install") => build(Command::Skill { install: true }),
            Some(other) => Some(Err(format!(
                "usage: omacal skill [install] — not \"{other}\""
            ))),
        },
        "calendars" => build(Command::Calendars),
        "doctor" => build(Command::Doctor),
        "agenda" => {
            let days = match take("--days") {
                Err(e) => return Some(Err(e)),
                Ok(None) => 7,
                Ok(Some(v)) => match v.parse::<u32>() {
                    Ok(n) if (1..=366).contains(&n) => n,
                    _ => return Some(Err("--days takes 1..=366".into())),
                },
            };
            build(Command::Agenda { days })
        }
        "events" => {
            // The write verbs live in `cli_write.rs`; `list` stays here.
            match rest.first().map(|s| s.as_str()) {
                Some(verb @ ("create" | "update" | "delete" | "respond")) => {
                    return Some(crate::cli_write::parse_events(verb, &rest[1..]).map(|cmd| {
                        Invocation { command: Command::Write(Box::new(cmd)), json }
                    }));
                }
                Some("show") => {
                    let id = rest
                        .get(1)
                        .filter(|a| !a.starts_with("--"))
                        .and_then(|v| v.parse::<i64>().ok());
                    return Some(match id {
                        Some(id) => Ok(Invocation { command: Command::Show { id }, json }),
                        None => Err(
                            "usage: omacal events show ID — `omacal events list` prints ids"
                                .into(),
                        ),
                    });
                }
                Some("list") => {}
                _ => {
                    return Some(Err(
                        "usage: omacal events list|create|update|delete|respond — \
                         see omacal cli-help"
                            .into(),
                    ));
                }
            }
            let date = |name: &str| -> Result<jiff::civil::Date, String> {
                match take(name)? {
                    Some(v) => v.parse().map_err(|_| format!("{name} takes YYYY-MM-DD")),
                    None => Err(format!("{name} is required")),
                }
            };
            let (from, to) = match (date("--from"), date("--to")) {
                (Ok(f), Ok(t)) => (f, t),
                (Err(e), _) | (_, Err(e)) => return Some(Err(e)),
            };
            if to < from {
                return Some(Err("--to is before --from".into()));
            }
            build(Command::Events { from, to })
        }
        "search" => {
            let query: Vec<&str> = rest
                .iter()
                .filter(|a| !a.starts_with("--"))
                .map(|a| a.as_str())
                .collect();
            if query.is_empty() {
                return Some(Err("usage: omacal search <query>".into()));
            }
            build(Command::Search { query: query.join(" ") })
        }
        _ => None,
    }
}

/// One expanded occurrence, as both registers print it. `camelCase` like
/// every payload the app serialises; `start`/`end` are the display zone's
/// own RFC 3339 readings of the same instants `startMs`/`endMs` carry, so a
/// script gets numbers and an agent gets something it can read aloud.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Row {
    pub event_id: i64,
    pub title: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub location: Option<String>,
    pub calendar: String,
    pub calendar_id: i64,
    pub attendees: u32,
    pub recurring: bool,
    pub response: Option<String>,
    /// Whether this is the user's own event — the organizer's address
    /// matching the owning account's. Explicit so an agent never has to
    /// infer it: `response: null` on 2026-08-27 was read as "unanswered
    /// invitation" and an organizer was told to RSVP to their own meeting.
    /// `false` also covers "organizer unstated", which is not a claim the
    /// user is a guest — `response` still carries that story.
    pub organizer: bool,
    pub conference: Option<String>,
}

/// The comparison behind [`Row::organizer`] — `events::is_organizer`, which
/// the popover's detail reads through as well, so the CLI and the app never
/// disagree about whose event it is.
pub(crate) use crate::events::is_organizer;

/// The window's occurrences, expanded and honest: the same suppression,
/// cancellation and declined rules the app's own views apply
/// (`upcoming::assemble`'s trio), the same `selected = 1` filter
/// `events_in_window` already carries — an agent sees exactly what the user
/// sees, hidden calendars included in their absence.
pub(crate) async fn rows_in_window(
    pool: &SqlitePool,
    from_ms: i64,
    to_ms: i64,
) -> anyhow::Result<Vec<Row>> {
    let stored = omacal_store::events_in_window(pool, from_ms, to_ms).await?;
    // Per calendar: its name for the row, and its account's address for the
    // organizer comparison — one map, one walk.
    let names: std::collections::HashMap<i64, (String, String)> =
        omacal_store::list_calendars(pool)
            .await?
            .into_iter()
            .map(|c| (c.id, (c.summary, c.account_email)))
            .collect();
    // The calendars' own ids, for `organizer.self`'s second half — a shared
    // calendar's events are organized as the *calendar*, not the account.
    let cal_gids: std::collections::HashMap<i64, String> =
        sqlx::query_as::<_, (i64, String)>("SELECT id, google_id FROM calendars")
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();
    let suppressed = crate::commands::suppressed_slots(&stored);

    let tz = jiff::tz::TimeZone::system();
    let stamp = |ms: i64| -> String {
        jiff::Timestamp::from_millisecond(ms)
            .map(|t| t.to_zoned(tz.clone()).strftime("%Y-%m-%dT%H:%M:%S%:z").to_string())
            .unwrap_or_default()
    };

    let mut rows = Vec::new();
    for src in &stored {
        if src.status == "cancelled" || src.self_response.as_deref() == Some("declined") {
            continue;
        }
        for iv in crate::commands::occurrences(src, from_ms, to_ms) {
            if suppressed.contains(&(src.calendar_id, src.google_id.as_str(), iv.start_ms)) {
                continue;
            }
            rows.push(Row {
                event_id: src.id,
                title: src.summary.clone().unwrap_or_else(|| "(no title)".into()),
                start_ms: iv.start_ms,
                end_ms: iv.end_ms,
                start: stamp(iv.start_ms),
                end: stamp(iv.end_ms),
                all_day: src.is_all_day,
                location: src.location.clone(),
                calendar: names
                    .get(&src.calendar_id)
                    .map(|(summary, _)| summary.clone())
                    .unwrap_or_default(),
                calendar_id: src.calendar_id,
                attendees: src.attendees.len() as u32,
                recurring: src.recurrence.is_some() || src.recurring_event_id.is_some(),
                response: src.self_response.clone(),
                organizer: is_organizer(
                    src.organizer_email.as_deref(),
                    names.get(&src.calendar_id).map(|(_, email)| email.as_str()).unwrap_or(""),
                    cal_gids.get(&src.calendar_id).map(String::as_str).unwrap_or(""),
                ),
                conference: src.conference_uri.clone().or_else(|| {
                    crate::upcoming::conference_join_url(
                        src.location.as_deref(),
                        src.description.as_deref(),
                    )
                }),
            });
        }
    }
    rows.sort_by_key(|r| (r.start_ms, r.end_ms, r.event_id));
    Ok(rows)
}

/// One guest on [`Detail`], answer included — the row the window listing
/// compresses to a count, uncompressed.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DetailGuest {
    pub email: String,
    pub name: Option<String>,
    /// `accepted` | `declined` | `tentative` | `needsAction` — the store's
    /// own vocabulary, unedited.
    pub response: String,
    pub optional: bool,
    /// The signed-in user's own row, when they are on the list at all.
    pub is_self: bool,
}

/// `omacal events show` — one event whole. The same field names [`Row`]
/// uses wherever the two overlap, so a script that learned the listing
/// reads this for free.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Detail {
    pub event_id: i64,
    pub title: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub start: String,
    pub end: String,
    pub all_day: bool,
    /// True for a series — and then the times above are the series anchor,
    /// not any particular occurrence; the window commands expand those.
    pub recurring: bool,
    pub location: Option<String>,
    pub description: Option<String>,
    pub calendar: String,
    pub calendar_id: i64,
    pub organizer: bool,
    pub organizer_email: Option<String>,
    /// Google's `guestsCanModify`: the organizer let guests change the
    /// event for everyone. Only meaningful when `organizer` is false.
    pub guests_can_modify: bool,
    /// `organizer` | `own-copy` | `shared` — who a change to this event
    /// reaches (`events::Reach`). `own-copy` is the one that changes what a
    /// write means: an update lands on the user's calendar alone, nobody
    /// else's copy moves, and `--notify` has nobody to reach.
    pub reach: &'static str,
    pub response: Option<String>,
    pub conference: Option<String>,
    pub guests: Vec<DetailGuest>,
}

/// The event, assembled from the same store read the write path trusts.
/// `None` when the id names nothing — a usage answer, not an error.
pub(crate) async fn detail_by_id(pool: &SqlitePool, id: i64) -> anyhow::Result<Option<Detail>> {
    let Some((ev, _role, _tz)) = omacal_store::event_by_id(pool, id).await? else {
        return Ok(None);
    };
    let (cal_name, account_email, cal_gid): (String, String, String) = sqlx::query_as(
        "SELECT c.summary, a.email, c.google_id FROM calendars c
         JOIN accounts a ON a.id = c.account_id WHERE c.id = ?1",
    )
    .bind(ev.calendar_id)
    .fetch_optional(pool)
    .await
    .map(|r: Option<(String, String, String)>| r.unwrap_or_default())
    .map_err(anyhow::Error::from)?;

    let tz = jiff::tz::TimeZone::system();
    let stamp = |ms: i64| -> String {
        jiff::Timestamp::from_millisecond(ms)
            .map(|t| t.to_zoned(tz.clone()).strftime("%Y-%m-%dT%H:%M:%S%:z").to_string())
            .unwrap_or_default()
    };

    Ok(Some(Detail {
        event_id: ev.id,
        title: ev.summary.clone().unwrap_or_else(|| "(no title)".into()),
        start_ms: ev.start_utc,
        end_ms: ev.end_utc,
        start: stamp(ev.start_utc),
        end: stamp(ev.end_utc),
        all_day: ev.is_all_day,
        recurring: ev.recurrence.is_some() || ev.recurring_event_id.is_some(),
        location: ev.location.clone(),
        description: ev.description.clone(),
        calendar: cal_name,
        calendar_id: ev.calendar_id,
        organizer: is_organizer(ev.organizer_email.as_deref(), &account_email, &cal_gid),
        organizer_email: ev.organizer_email.clone(),
        guests_can_modify: ev.guests_can_modify,
        reach: crate::events::Reach::of(
            is_organizer(ev.organizer_email.as_deref(), &account_email, &cal_gid),
            ev.guests_can_modify,
        )
        .as_str(),
        response: ev.self_response.clone(),
        conference: ev.conference_uri.clone().or_else(|| {
            crate::upcoming::conference_join_url(ev.location.as_deref(), ev.description.as_deref())
        }),
        guests: ev
            .attendees
            .iter()
            .map(|a| DetailGuest {
                email: a.email.clone(),
                name: a.display_name.clone(),
                response: a.response_status.clone(),
                optional: a.optional,
                is_self: a.is_self,
            })
            .collect(),
    }))
}

fn print_detail_human(d: &Detail) {
    println!("{}", d.title);
    let tz = jiff::tz::TimeZone::system();
    let when = jiff::Timestamp::from_millisecond(d.start_ms)
        .map(|t| t.to_zoned(tz.clone()))
        .ok();
    let day = when.as_ref().map(|z| z.strftime("%a, %b %-d").to_string()).unwrap_or_default();
    if d.all_day {
        println!("{day}  All day");
    } else {
        let start = when.map(|z| z.strftime("%H:%M").to_string()).unwrap_or_default();
        let end = jiff::Timestamp::from_millisecond(d.end_ms)
            .map(|t| t.to_zoned(tz).strftime("%H:%M").to_string())
            .unwrap_or_default();
        println!("{day}  {start}–{end}{}", if d.recurring { "  (repeats)" } else { "" });
    }
    if let Some(loc) = d.location.as_deref().filter(|l| !l.is_empty()) {
        println!("Where: {loc}");
    }
    if let Some(uri) = d.conference.as_deref() {
        println!("Join: {uri}");
    }
    println!(
        "Calendar: {}{}",
        d.calendar,
        if d.organizer { "  · your event" } else { "" }
    );
    match d.reach {
        "own-copy" => println!("You are a guest: a change here moves only your copy, and nobody is told"),
        "shared" => println!("You are a guest, and the organizer lets guests change this for everyone"),
        _ => {}
    }
    if d.guests.is_empty() {
        println!("Guests: none");
    } else {
        println!("Guests:");
        for g in &d.guests {
            println!(
                "  {:<12} {}{}{}",
                g.response,
                g.email,
                if g.optional { "  [optional]" } else { "" },
                if g.is_self { "  (you)" } else { "" },
            );
        }
    }
}

/// Where the app keeps its database — `app_data_dir` reproduced without an
/// app, because this path runs before Tauri exists. The identifier is
/// `tauri.conf.json`'s and moves only if that does, which `lib.rs` already
/// promises never to do (it would move every user's data).
pub(crate) fn db_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let home = std::path::Path::new(&home);
    let dir = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/com.omacal.app")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("com.omacal.app")
    };
    Some(dir.join("omacal.db"))
}

/// Local midnight of `date`, ms — the CLI's windows are civil days in the
/// display zone, the same zone the app draws.
pub(crate) fn day_start_ms(date: jiff::civil::Date) -> anyhow::Result<i64> {
    Ok(date
        .to_zoned(jiff::tz::TimeZone::system())?
        .timestamp()
        .as_millisecond())
}

fn print_rows_human(rows: &[Row]) {
    if rows.is_empty() {
        println!("Nothing scheduled.");
        return;
    }
    let tz = jiff::tz::TimeZone::system();
    let mut last_day = String::new();
    for r in rows {
        let z = jiff::Timestamp::from_millisecond(r.start_ms)
            .map(|t| t.to_zoned(tz.clone()))
            .ok();
        let day = z
            .as_ref()
            .map(|z| z.strftime("%a, %b %-d").to_string())
            .unwrap_or_default();
        if day != last_day {
            println!("{day}");
            last_day = day;
        }
        let when = if r.all_day {
            "All day     ".to_string()
        } else {
            let end = jiff::Timestamp::from_millisecond(r.end_ms)
                .map(|t| t.to_zoned(tz.clone()).strftime("%H:%M").to_string())
                .unwrap_or_default();
            format!("{}–{end}", z.map(|z| z.strftime("%H:%M").to_string()).unwrap_or_default())
        };
        let mut line = format!("  {when}  {}", r.title);
        if let Some(loc) = r.location.as_deref().filter(|l| !l.is_empty()) {
            line.push_str(&format!("  · {loc}"));
        }
        if r.attendees > 1 {
            line.push_str(&format!("  · {} people", r.attendees));
        }
        println!("{line}");
    }
}

fn print_json<T: Serialize>(data: &T) {
    println!(
        "{}",
        serde_json::json!({ "ok": true, "data": data })
    );
}

pub(crate) fn fail(json: bool, code: &str, message: &str, exit: i32) -> i32 {
    if json {
        println!("{}", serde_json::json!({ "ok": false, "error": { "code": code, "message": message } }));
    } else {
        eprintln!("omacal: {message}");
    }
    exit
}

/// Runs one invocation to completion and answers with the process exit
/// code. Its own tokio runtime, because the app's has not been built — and
/// never will be on this path.
/// The mark and the wordmark as truecolor half-blocks — drawn for the
/// terminal's grid by `scripts/generate-cli-logo.py`, never hand-edited.
/// Foreground colours only, so it sits on any terminal background.
/// basecamp-cli's welcome wears the same trick, and it earns its bytes:
/// a CLI that greets a person differently from a pipe is a CLI that knows
/// which one it is talking to.
const LOGO: &str = include_str!("cli_logo.ans");

/// The greeting, person-only: the logo and tagline print exactly when
/// stdout is a real terminal, colour is welcome (`NO_COLOR` unset), and a
/// dumb terminal is not pretending otherwise. A pipe gets `USAGE` bare —
/// escape codes in a script's captured help are noise someone greps around.
fn print_help() {
    use std::io::IsTerminal;
    let dumb = std::env::var_os("TERM").is_some_and(|t| t == "dumb");
    if std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() && !dumb {
        println!("\n{LOGO}");
        println!("  \x1b[1mYour calendar, at your command (line).\x1b[0m\n");
    }
    println!("{USAGE}");
}

pub(crate) fn run(inv: Invocation) -> i32 {
    if matches!(inv.command, Command::Help) {
        print_help();
        return EXIT_OK;
    }
    if matches!(inv.command, Command::Commands) {
        let catalog = command_catalog();
        if inv.json {
            print_json(&catalog);
        } else {
            for c in &catalog {
                println!("{:<28} {}", c.usage, c.description);
            }
        }
        return EXIT_OK;
    }

    // Every CLI run keeps the installed skill matched to this binary —
    // silent, marker-guarded, and a no-op within one version (cli_skill.rs).
    // Before anything else so even a failing command leaves agents current.
    if let Some(home) = std::env::var_os("HOME") {
        crate::cli_skill::refresh_if_version_changed(std::path::Path::new(&home));
    }

    // The skill commands need a $HOME and nothing else — no database, no
    // runtime: a fresh machine can wire its agent before ever launching
    // the app.
    if let Command::Skill { install } = inv.command {
        let Some(home) = std::env::var_os("HOME") else {
            return fail(inv.json, "no_home", "HOME is not set", EXIT_ERROR);
        };
        let home = std::path::PathBuf::from(home);
        if !install {
            // Raw markdown, both registers: this output exists to be piped
            // into whatever an agent framework wants, and an envelope
            // around a document would just be bytes to strip.
            print!("{}", crate::cli_skill::SKILL_MD);
            return EXIT_OK;
        }
        return match crate::cli_skill::install(&home) {
            Ok(done) => {
                if inv.json {
                    print_json(&done);
                } else {
                    println!("Skill installed: {}", done.canonical);
                    if let Some(link) = &done.claude_link {
                        println!("Claude Code linked: {link}");
                    }
                    for old in &done.legacy {
                        println!("Note: {old} is the old hand-copied skill — this replaces it; remove it when convenient.");
                    }
                    println!("It stays current by itself: every omacal run refreshes it after an update.");
                }
                EXIT_OK
            }
            Err(m) => fail(inv.json, "refused", &m, EXIT_ERROR),
        };
    }

    let Some(path) = db_path() else {
        return fail(inv.json, "no_home", "HOME is not set", EXIT_ERROR);
    };
    if matches!(inv.command, Command::Doctor) {
        // Doctor's whole job is diagnosing a broken install, so a missing
        // database is a finding for it, never a refusal.
        return doctor::run(inv.json, &path);
    }
    if !path.exists() {
        return fail(
            inv.json,
            "no_database",
            "no omacal database yet — launch omacal and connect an account first",
            EXIT_NO_DB,
        );
    }

    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => return fail(inv.json, "runtime", &e.to_string(), EXIT_ERROR),
    };

    rt.block_on(async {
        let url = format!("sqlite://{}?mode=ro", path.display());
        let pool = match omacal_store::connect_readonly(&url).await {
            Ok(p) => p,
            Err(e) => return fail(inv.json, "open_failed", &e.to_string(), EXIT_ERROR),
        };

        // The write verbs: prechecked against this same read-only pool —
        // does it repeat, does it have guests — then executed by the
        // running app over its socket (`cli_write.rs`). The read-only mode
        // above is not incidental: it is the module's whole claim.
        if let Command::Write(cmd) = &inv.command {
            return crate::cli_write::execute(&pool, cmd, inv.json).await;
        }

        let result: anyhow::Result<i32> = async {
            match &inv.command {
                Command::Help | Command::Doctor => unreachable!("handled above"),
                Command::Skill { .. } | Command::Commands => {
                    unreachable!("handled above, before the database")
                }
                Command::Write(_) => unreachable!("taken by value above"),
                Command::Calendars => {
                    let cals = omacal_store::list_calendars(&pool).await?;
                    if inv.json {
                        print_json(&cals);
                    } else if cals.is_empty() {
                        println!("No calendars — connect an account in omacal first.");
                    } else {
                        for c in &cals {
                            println!(
                                "{:>5}  {}  ({}, {}){}{}",
                                c.id,
                                c.summary,
                                c.account_email,
                                c.provider,
                                if c.selected { "" } else { "  [hidden]" },
                                if c.sync_enabled { "" } else { "  [not fetched]" },
                            );
                        }
                    }
                    Ok(EXIT_OK)
                }
                Command::Agenda { days } => {
                    let today = jiff::Zoned::now().date();
                    let from = day_start_ms(today)?;
                    let to = day_start_ms(today.saturating_add(jiff::Span::new().days(i64::from(*days))))?;
                    let rows = rows_in_window(&pool, from, to).await?;
                    if inv.json { print_json(&rows) } else { print_rows_human(&rows) }
                    Ok(EXIT_OK)
                }
                Command::Events { from, to } => {
                    let from_ms = day_start_ms(*from)?;
                    // Inclusive last day, exclusive instant — the CLI's dates
                    // read like the form's ("to Friday" includes Friday).
                    let to_ms = day_start_ms(to.saturating_add(jiff::Span::new().days(1)))?;
                    let rows = rows_in_window(&pool, from_ms, to_ms).await?;
                    if inv.json { print_json(&rows) } else { print_rows_human(&rows) }
                    Ok(EXIT_OK)
                }
                Command::Show { id } => {
                    match detail_by_id(&pool, *id).await? {
                        Some(d) => {
                            if inv.json { print_json(&d) } else { print_detail_human(&d) }
                            Ok(EXIT_OK)
                        }
                        None => Ok(fail(
                            inv.json,
                            "usage",
                            &format!("no event {id} — `omacal events list` prints real ids"),
                            EXIT_USAGE,
                        )),
                    }
                }
                Command::Search { query } => {
                    let hits = crate::search::search(&pool, query, crate::now_ms()).await?;
                    if inv.json {
                        print_json(&hits);
                    } else if hits.is_empty() {
                        println!("No matches.");
                    } else {
                        let tz = jiff::tz::TimeZone::system();
                        for h in &hits {
                            let when = jiff::Timestamp::from_millisecond(h.start_ms)
                                .map(|t| t.to_zoned(tz.clone()).strftime("%Y-%m-%d %H:%M").to_string())
                                .unwrap_or_default();
                            println!("{when}  {}  (event {})", h.title, h.event_id);
                        }
                    }
                    Ok(EXIT_OK)
                }
            }
        }
        .await;

        match result {
            Ok(code) => code,
            Err(e) => fail(inv.json, "query_failed", &e.to_string(), EXIT_ERROR),
        }
    })
}

/// The whole CLI entry: parse, and either run to an exit or hand back to
/// the GUI path. Called first thing in `lib::run`, before tracing installs
/// (a JSON stream must not carry log lines) and before Tauri is built.
pub(crate) fn maybe_run_and_exit() {
    let argv: Vec<String> = std::env::args().collect();
    // Rust ships with SIGPIPE ignored, which turns `omacal agenda | head`
    // into a panic the moment head closes the pipe. Restore the default —
    // die quietly, the way every Unix filter does — but only once this is
    // known to be a CLI run: the GUI's webview and sockets want the ignore.
    #[cfg(unix)]
    fn allow_sigpipe() {
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    }
    match parse(&argv) {
        None => {}
        Some(Ok(inv)) => {
            #[cfg(unix)]
            allow_sigpipe();
            std::process::exit(run(inv))
        }
        Some(Err(usage)) => {
            eprintln!("omacal: {usage}\n\n{USAGE}");
            std::process::exit(EXIT_USAGE);
        }
    }
}

/// `omacal doctor`: every fact a bug report needs, in one paste.
///
/// Born from issue #1, where the reporter spent an afternoon establishing
/// facts this prints in two seconds — which binary, which channel, whether
/// the keyring answers, whether the network does. Checks that can fail do
/// so as findings, never as crashes: a doctor that dies on the disease it
/// exists to diagnose is the one outcome not allowed.
mod doctor {
    use serde::Serialize;

    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct Check {
        pub name: &'static str,
        /// `None` is "informational", not pass/fail — the version row is
        /// nobody's failure.
        pub ok: Option<bool>,
        pub detail: String,
    }

    /// Which door this binary came through. The same probes the updater
    /// gates on (`update::running_as_appimage`, the flatpak marker file),
    /// pure over their answers so the mapping is testable.
    pub(super) fn channel(is_appimage: bool, is_flatpak: bool) -> &'static str {
        match (is_appimage, is_flatpak) {
            (true, _) => "appimage",
            (_, true) => "flatpak",
            _ => "package or dev build",
        }
    }

    fn push(checks: &mut Vec<Check>, name: &'static str, ok: Option<bool>, detail: String) {
        checks.push(Check { name, ok, detail });
    }

    pub(super) fn run(json: bool, db: &std::path::Path) -> i32 {
        let mut checks = Vec::new();

        push(&mut checks, "version", None, env!("CARGO_PKG_VERSION").into());
        let is_flatpak = std::path::Path::new("/.flatpak-info").exists();
        push(
            &mut checks,
            "channel",
            None,
            channel(crate::update::running_as_appimage(), is_flatpak).into(),
        );

        push(
            &mut checks,
            "database",
            Some(db.exists()),
            if db.exists() {
                format!("{}", db.display())
            } else {
                format!("missing — launch omacal and connect an account ({})", db.display())
            },
        );

        // The keyring: ask for an entry that never exists. "No such entry"
        // is the healthy answer — the Secret Service picked up and said no —
        // while a platform error means no gnome-keyring/KeePassXC/kwallet is
        // running, which is issue-#1-adjacent territory: sign-in appears to
        // work and nothing persists.
        let keyring = match keyring::Entry::new(crate::KEYRING_SERVICE, "__doctor_probe__")
            .and_then(|e| e.get_password().map(|_| ()))
        {
            Err(keyring::Error::NoEntry) | Ok(()) => (true, "Secret Service reachable".to_string()),
            Err(e) => (false, format!("unreachable — start gnome-keyring, KeePassXC or kwallet ({e})")),
        };
        push(&mut checks, "keyring", Some(keyring.0), keyring.1);

        // The agent skill: informational, like version — a machine with no
        // agent has nothing failing. Installed means the canonical copy is
        // this module's own (the marker is the claim, cli_skill.rs).
        let skill = std::env::var_os("HOME").map(|h| {
            std::path::Path::new(&h).join(".agents/skills/omacal/.managed-by-omacal").exists()
        });
        push(
            &mut checks,
            "agent skill",
            None,
            match skill {
                Some(true) => "installed (~/.agents/skills/omacal, auto-refreshed)".into(),
                _ => "not installed — `omacal skill install` wires your agents".into(),
            },
        );

        push(
            &mut checks,
            "custom credentials",
            None,
            if std::env::var_os("HOME")
                .map(|h| std::path::Path::new(&h).join(".config/omacal/config.toml").exists())
                .unwrap_or(false)
            {
                "config.toml present (own Google client in use)".into()
            } else {
                "none (the official client)".into()
            },
        );

        // Network, with the update endpoint doubling as the reachability
        // probe: one request answers both "is there internet" and "is a
        // newer omacal out".
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build();
        if let Ok(rt) = rt {
            let latest = rt.block_on(async {
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    crate::update::fetch_latest(crate::update::LATEST_RELEASE_ENDPOINT),
                )
                .await
            });
            match latest {
                Ok(Ok((tag, _))) => {
                    let tag_v = tag.trim_start_matches('v');
                    let current = env!("CARGO_PKG_VERSION");
                    let newer = crate::update::newer_than(current, &tag);
                    push(&mut checks, "network", Some(true), "release endpoint reachable".into());
                    push(
                        &mut checks,
                        "update",
                        Some(!newer),
                        if newer {
                            format!("{tag_v} is available (this is {current})")
                        } else {
                            "up to date".into()
                        },
                    );
                }
                Ok(Err(e)) => push(&mut checks, "network", Some(false), format!("release endpoint: {e}")),
                Err(_) => push(&mut checks, "network", Some(false), "release endpoint: timed out".into()),
            }
        }

        if json {
            println!("{}", serde_json::json!({ "ok": true, "data": checks }));
        } else {
            for c in &checks {
                let mark = match c.ok {
                    Some(true) => "✓",
                    Some(false) => "✗",
                    None => "·",
                };
                println!("{mark} {:<20} {}", c.name, c.detail);
            }
        }
        // Exit 0 even with red rows: doctor reports, the reader decides.
        // A script that wants a verdict reads the JSON.
        super::EXIT_OK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &str) -> Vec<String> {
        std::iter::once("omacal".to_string())
            .chain(s.split_whitespace().map(String::from))
            .collect()
    }

    /// The property everything else stands on: nothing the GUI already
    /// answers is ever the CLI's. A bare launch, the tray flags, a date —
    /// all fall through, or `omacal --sync-now` in a script would print
    /// usage instead of syncing.
    #[test]
    fn everything_the_gui_owns_falls_through() {
        for s in ["", "--sync-now", "--quit", "2026-09-01"] {
            assert_eq!(parse(&argv(s)), None, "{s:?} was claimed by the CLI");
        }
    }

    /// `events show` parses with a numeric id and refuses everything else
    /// by name — never a fall-through to a window.
    #[test]
    fn show_takes_an_id_and_nothing_else() {
        assert_eq!(
            parse(&argv("events show 41 --json")),
            Some(Ok(Invocation { command: Command::Show { id: 41 }, json: true }))
        );
        assert!(matches!(parse(&argv("events show")), Some(Err(_))));
        assert!(matches!(parse(&argv("events show denis")), Some(Err(_))));
    }

    /// The detail, assembled against a real (in-memory) store: the guest
    /// list arrives whole with each answer — the fact the window rows
    /// compress to a count, and the reason this command exists (three
    /// field sessions in a row asked "who accepted?" and the CLI could
    /// not say) — and the organizer flag speaks `organizer.self` here
    /// exactly as it does on the rows.
    #[tokio::test]
    async fn show_uncompresses_the_guest_list() {
        let pool = omacal_store::connect_memory().await.unwrap();
        sqlx::query(
            "INSERT INTO accounts (google_sub, email, created_at) VALUES ('s','me@x.test',0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
             VALUES (1, 'me@x.test', 'Work', 'UTC', 'owner')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO events (calendar_id, google_id, summary, start_utc, end_utc,
                                 start_tz, end_tz, is_all_day, status, updated_at,
                                 organizer_email, attendees_json)
             VALUES (1, 'g1', 'Feedback session', 1786352400000, 1786356000000,
                     'UTC', 'UTC', 0, 'confirmed', 0, 'me@x.test',
                     '[{\"email\":\"iskren@x.test\",\"display_name\":null,\"response_status\":\"accepted\",\"optional\":false,\"is_self\":false,\"comment\":null,\"additional_guests\":0},
                       {\"email\":\"denis@x.test\",\"display_name\":null,\"response_status\":\"needsAction\",\"optional\":true,\"is_self\":false,\"comment\":null,\"additional_guests\":0}]')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let d = detail_by_id(&pool, 1).await.unwrap().expect("the row exists");
        assert_eq!(d.title, "Feedback session");
        assert!(d.organizer, "organizer.self: the account's own address");
        assert_eq!(d.guests.len(), 2);
        assert_eq!(
            (d.guests[0].email.as_str(), d.guests[0].response.as_str()),
            ("iskren@x.test", "accepted"),
        );
        assert_eq!(
            (d.guests[1].response.as_str(), d.guests[1].optional),
            ("needsAction", true),
            "the unanswered guest arrives as needsAction, the store's own word",
        );

        assert!(
            detail_by_id(&pool, 99).await.unwrap().is_none(),
            "an unknown id is an answer, not an error",
        );
    }

    /// The comparison behind `Row::organizer`, as a table — Google's
    /// `organizer.self`: the account's address OR the calendar's own id,
    /// case-insensitive; absences match nothing, ever.
    #[test]
    fn the_organizer_flag_speaks_organizer_self() {
        let is = is_organizer;
        assert!(is(Some("plamen@excitel.com"), "plamen@excitel.com", "plamen@excitel.com"));
        assert!(is(Some("Plamen@Excitel.COM"), "plamen@excitel.com", ""));
        // The live row that disproved the account-only rule: an event on a
        // calendar shared into another account is organized as the calendar.
        assert!(is(Some("plamen@x3me.net"), "marlowbg@gmail.com", "plamen@x3me.net"));
        assert!(!is(Some("denis@x.com"), "plamen@excitel.com", "plamen@excitel.com"));
        assert!(!is(None, "plamen@excitel.com", "x"), "unstated is not a claim");
        assert!(!is(Some("a@b.c"), "", ""), "an orphaned calendar organizes nothing");
        // A CalDAV calendar's id is a URL; the comparison fails harmlessly
        // and the account address carries the answer.
        assert!(is(Some("ana@fastmail.com"), "ana@fastmail.com", "https://dav.example/cal/"));
    }

    /// The catalog matches the parser: every top-level word `parse`
    /// answers to appears exactly once as a catalog name prefix, the four
    /// write verbs are marked as writes, and nothing else is. Hand-kept on
    /// both sides — this test is the seam that keeps them one.
    #[test]
    fn the_catalog_matches_the_parser() {
        let catalog = command_catalog();
        let names: Vec<&str> = catalog.iter().map(|c| c.name).collect();
        for required in [
            "agenda", "events list", "events show", "events create", "events update",
            "events delete", "events respond", "search", "calendars", "doctor",
            "skill", "commands", "cli-help",
        ] {
            assert_eq!(
                names.iter().filter(|n| **n == required).count(),
                1,
                "{required} appears exactly once"
            );
        }
        assert_eq!(names.len(), 13, "nothing in the catalog the parser does not answer");
        let writers: Vec<&str> =
            catalog.iter().filter(|c| c.writes).map(|c| c.name).collect();
        assert_eq!(
            writers,
            ["events create", "events update", "events delete", "events respond"],
            "exactly the four socket verbs write"
        );
        assert!(serde_json::to_string(&catalog).is_ok());

        assert_eq!(
            parse(&argv("commands --json")),
            Some(Ok(Invocation { command: Command::Commands, json: true }))
        );
    }

    /// The embedded logo is the generator's output: truecolor half-blocks,
    /// nothing else — and present, so a TTY greeting can never be blank.
    #[test]
    fn the_logo_is_real_ansi_art() {
        assert!(LOGO.contains('\u{2580}'), "half-blocks are the medium");
        assert!(LOGO.contains("\u{1b}[38;2;"), "24-bit colour escapes");
        assert!(LOGO.lines().count() >= 10, "a logo, not a smudge");
    }

    /// The skill commands parse bare and with `install`, and a stray word
    /// is usage — never a fall-through to a window.
    #[test]
    fn the_skill_commands_are_claimed() {
        assert_eq!(
            parse(&argv("skill")),
            Some(Ok(Invocation { command: Command::Skill { install: false }, json: false }))
        );
        assert_eq!(
            parse(&argv("skill install --json")),
            Some(Ok(Invocation { command: Command::Skill { install: true }, json: true }))
        );
        assert!(matches!(parse(&argv("skill uninstall")), Some(Err(_))));
    }

    /// The write verbs route to `cli_write` and stay inside the CLI's
    /// claimed territory — a good one parses to `Write`, a bad one is
    /// usage, and neither can boot a GUI.
    #[test]
    fn the_write_verbs_are_claimed_and_routed() {
        assert!(matches!(
            parse(&argv(
                "events create --title Standup --date 2026-09-01 --start 09:00 --end 09:30"
            )),
            Some(Ok(Invocation { command: Command::Write(_), json: false }))
        ));
        assert!(matches!(
            parse(&argv("events delete 41 --occurrence 1786352400000 --json")),
            Some(Ok(Invocation { command: Command::Write(_), json: true }))
        ));
        // Recognised verb, used wrongly: usage, never a window.
        assert!(matches!(parse(&argv("events create")), Some(Err(_))));
        assert!(matches!(parse(&argv("events respond")), Some(Err(_))));
    }

    /// The channel mapping, pinned: AppImage wins over a stray flatpak
    /// marker (it cannot happen, but the arm order should not matter by
    /// accident), and neither means a package.
    #[test]
    fn the_doctor_names_the_channel_it_actually_is() {
        assert_eq!(super::doctor::channel(true, false), "appimage");
        assert_eq!(super::doctor::channel(false, true), "flatpak");
        assert_eq!(super::doctor::channel(false, false), "package or dev build");
        assert_eq!(
            parse(&argv("doctor --json")),
            Some(Ok(Invocation { command: Command::Doctor, json: true }))
        );
    }

    #[test]
    fn agenda_defaults_to_a_week_and_bounds_its_days() {
        assert_eq!(
            parse(&argv("agenda")),
            Some(Ok(Invocation { command: Command::Agenda { days: 7 }, json: false }))
        );
        assert_eq!(
            parse(&argv("agenda --days 30 --json")),
            Some(Ok(Invocation { command: Command::Agenda { days: 30 }, json: true }))
        );
        assert!(matches!(parse(&argv("agenda --days 0")), Some(Err(_))));
        assert!(matches!(parse(&argv("agenda --days 400")), Some(Err(_))));
        assert!(matches!(parse(&argv("agenda --days")), Some(Err(_))));
    }

    /// A recognised subcommand used wrongly errs — it must never fall
    /// through and boot a GUI the user did not ask for.
    #[test]
    fn a_wrong_events_invocation_is_usage_not_a_window() {
        assert!(matches!(parse(&argv("events")), Some(Err(_))));
        assert!(matches!(parse(&argv("events list")), Some(Err(_))));
        assert!(matches!(parse(&argv("events list --from 2026-09-01")), Some(Err(_))));
        assert!(matches!(
            parse(&argv("events list --from 2026-09-02 --to 2026-09-01")),
            Some(Err(_))
        ));
        assert!(matches!(parse(&argv("events list --from sept --to 2026-09-01")), Some(Err(_))));
    }

    #[test]
    fn events_dates_parse_and_search_joins_its_words() {
        let inv = parse(&argv("events list --from 2026-09-01 --to 2026-09-05 --json"))
            .unwrap()
            .unwrap();
        assert_eq!(
            inv.command,
            Command::Events {
                from: jiff::civil::date(2026, 9, 1),
                to: jiff::civil::date(2026, 9, 5),
            }
        );
        assert!(inv.json);

        let inv = parse(&argv("search weekly ops review")).unwrap().unwrap();
        assert_eq!(inv.command, Command::Search { query: "weekly ops review".into() });
        assert!(matches!(parse(&argv("search --json")), Some(Err(_))));
    }

    /// The expansion path against a real store: a one-off, a daily series
    /// (expanded, not one row), a declined row (absent), and a hidden
    /// calendar's event (absent) — the same visibility the app's own views
    /// have, which is the whole contract with an agent reading this.
    #[tokio::test]
    async fn rows_expand_series_and_hide_what_the_app_hides() {
        let pool = omacal_store::connect_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (google_sub, email, created_at) VALUES ('s','me@x.com',0)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO calendars (id, account_id, google_id, summary, color_hex, timezone,
                                    access_role, is_primary, selected, sync_enabled)
             VALUES (1, 1, 'g1', 'Work',   '#5b8def', 'UTC', 'owner', 1, 1, 1),
                    (2, 1, 'g2', 'Hidden', '#5b8def', 'UTC', 'owner', 0, 0, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();

        const DAY: i64 = 24 * 3_600_000;
        let base = 1_788_000_000_000; // some instant; the window is relative
        let ev = |cal: i64, gid: &str, start: i64, rrule: Option<&str>, resp: &str| {
            let rrule = rrule.map(str::to_string);
            let resp = resp.to_string();
            let gid = gid.to_string();
            let pool = pool.clone();
            async move {
                sqlx::query(
                    "INSERT INTO events (calendar_id, google_id, summary, start_utc, end_utc,
                                         start_tz, end_tz, recurrence, status, self_response, updated_at)
                     VALUES (?1, ?2, ?2, ?3, ?4, 'UTC', 'UTC', ?5, 'confirmed', ?6, 0)",
                )
                .bind(cal)
                .bind(gid)
                .bind(start)
                .bind(start + 3_600_000)
                .bind(rrule)
                .bind(resp)
                .execute(&pool)
                .await
                .unwrap();
            }
        };
        ev(1, "solo", base + DAY, None, "accepted").await;
        ev(1, "daily", base, Some("RRULE:FREQ=DAILY"), "accepted").await;
        ev(1, "nope", base + DAY, None, "declined").await;
        ev(2, "ghost", base + DAY, None, "accepted").await;

        let rows = rows_in_window(&pool, base, base + 3 * DAY).await.unwrap();
        let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(
            titles.iter().filter(|t| **t == "daily").count(),
            3,
            "a daily series across three days expands to three rows"
        );
        assert!(titles.contains(&"solo"));
        assert!(!titles.contains(&"nope"), "a declined event reached the agenda");
        assert!(!titles.contains(&"ghost"), "a hidden calendar's event reached the agenda");
        assert!(rows.windows(2).all(|w| w[0].start_ms <= w[1].start_ms), "unsorted");
        assert!(rows.iter().find(|r| r.title == "daily").unwrap().recurring);
        assert_eq!(rows[0].calendar, "Work");
        assert!(!rows[0].start.is_empty(), "the readable stamp is missing");
    }
}
