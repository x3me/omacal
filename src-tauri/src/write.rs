//! Pure builders for event write bodies.
//!
//! Everything here is a function of its arguments: no pool, no client, no
//! clock. The write commands stay thin wrappers around these so the rules
//! that matter — "never send a field the user did not touch", "all-day means
//! `date` not `dateTime`" — are testable without a server.

use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EventFields {
    pub summary: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    /// When the event happens. One field rather than three, so that a date and
    /// an all-day flag cannot disagree — see [`When`].
    pub when: When,
    /// IANA zone the times are authored in. Read by the timed arm alone; an
    /// all-day event has no zone, which is what [`When`] enforces.
    pub tz: String,
    /// Three-state, and the distinction is the point:
    /// `None` — the user did not touch Repeat; omit `recurrence` entirely.
    /// `Some(None)` — the user chose Never; send `null`.
    /// `Some(Some(rule))` — send `[rule]`.
    pub recurrence: Option<Option<String>>,
    /// **The guest list the user wants the event to end up with**, or `None`
    /// for "the guest list was not touched".
    ///
    /// The same absent/present distinction the rest of this struct runs on, and
    /// here it is load bearing rather than tidy. Google's `attendees` is a
    /// whole-list replace (guest-list spec §2), so a save that resent the list
    /// every time would rewrite every attendee on the event from whatever
    /// omacal last read — quietly un-inviting anyone added elsewhere since. A
    /// path that does not offer guest editing (a drag) sends `None` and cannot
    /// touch the list at all.
    ///
    /// `Some(vec![])` is not the same as `None`: it means *remove everyone*,
    /// which is a thing a user can ask for. There is no "remove" call.
    ///
    /// (`reminders` below runs on the same distinction, for the same reason:
    /// Google's `reminders` is a whole-object replace.)
    ///
    /// Deliberately not a diff. The form knows the list it is showing, not the
    /// operations that produced it, and turning a target list back into a delta
    /// would be guesswork — see [`crate::events::attendees_for_edit`], which is
    /// where the target is reconciled against what is actually stored.
    ///
    /// **Read by both write paths.** The edit path reconciles this target list
    /// against what is stored ([`crate::events::attendees_for_edit`]); the
    /// create path runs the same builder against an empty list, since a
    /// brand-new event has nobody on it. The absent/present distinction above
    /// still does different work on each: on an edit, absent is the only way to
    /// say "leave the list alone", while on a create there is no list to leave
    /// alone and absent simply means nobody was invited.
    pub guests: Option<Vec<Guest>>,
    /// **The reminder settings the event should end up with**, or `None` for
    /// "reminders were not touched" — `guests`' own three-state, minus the
    /// empty-list subtlety: `overrides: []` with `use_default: false` is a
    /// meaningful value (no reminders at all), and absent is the only way to
    /// say leave-it-alone, because Google's `reminders` is a whole-object
    /// replace (reminders spec §2).
    ///
    /// Unlike `guests`, the create path reads this too: a reminder invites
    /// nobody and mails nobody, so there is no notify question for a create to
    /// defer.
    pub reminders: Option<RemindersInput>,
}

/// One guest, as the form names them.
///
/// Two fields, because two are all the user can author: an address, and whether
/// the person is optional. Everything else an attendee carries —
/// `responseStatus`, `displayName`, `comment`, `additionalGuests` — belongs to
/// that person rather than to whoever is editing, and is echoed back from what
/// is stored rather than sent from here. That asymmetry *is* guest-list spec
/// §2, expressed as a type: there is no field on this struct through which a
/// form could overwrite somebody's answer.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Guest {
    pub email: String,
    /// Google's `optional`. Absent reads as false, which is what an ordinary
    /// invitation is.
    #[serde(default)]
    pub optional: bool,
}

/// An event's reminder settings, as the form sends them (reminders spec §2).
///
/// The wire twin of `omacal_store::Reminders` rather than the type itself, for
/// the layering rule `Guest` states: the store's types do not enter this
/// file's pure builders. The two fields are alternatives — `use_default` and
/// the calendar's list applies, or `overrides` replaces it entirely.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemindersInput {
    pub use_default: bool,
    #[serde(default)]
    pub overrides: Vec<ReminderInput>,
}

/// One reminder: fire `minutes` before the start, by `method` — Google's own
/// vocabulary, `popup` or `email`. The form authors `popup` rows only and
/// echoes stored `email` ones back verbatim (spec §1); [`validate_reminders`]
/// is what refuses anything else the wire could carry.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub(crate) struct ReminderInput {
    pub method: String,
    pub minutes: i64,
}

/// Google's `reminders` object, whole — the only shape it accepts (spec §2).
pub(crate) fn reminders_json(r: &RemindersInput) -> Value {
    json!({
        "useDefault": r.use_default,
        "overrides": r.overrides.iter()
            .map(|o| json!({ "method": o.method, "minutes": o.minutes }))
            .collect::<Vec<_>>(),
    })
}

/// Google's own limits, refused with the limit in the message rather than
/// clamped (spec §4). Checked before anything is sent — and before any row is
/// read, per `create_impl`'s ordering rule, which is why this is a pure
/// function and not a step inside a command.
pub(crate) fn validate_reminders(r: &RemindersInput) -> Result<(), String> {
    if r.overrides.len() > 5 {
        return Err("an event can carry at most 5 reminders".into());
    }
    for o in &r.overrides {
        if o.method != "popup" && o.method != "email" {
            return Err(format!("'{}' is not a reminder method Google knows", o.method));
        }
        if !(0..=40_320).contains(&o.minutes) {
            return Err("a reminder must be 0 to 40320 minutes (four weeks) ahead".into());
        }
    }
    Ok(())
}

/// When an event happens.
///
/// An all-day event has **no instant and no zone** — Google models it as a bare
/// `date`, and so does the store once `omacal_sync::resolve` has read it. The
/// enum exists so nobody can supply a zone for one: the previous shape took
/// `(ms, is_all_day, tz)` and the two sides of the boundary converted that date
/// to an instant in *different* zones, which moved events nobody moved.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum When {
    Timed { start_ms: i64, end_ms: i64 },
    /// Both `yyyy-mm-dd`. `end_date` is **exclusive** — the day after the last
    /// one — matching Google's wire format and the store's `end_utc`.
    AllDay { start_date: String, end_date: String },
}

impl When {
    /// Which of Google's two time shapes this is.
    ///
    /// Derived, never stored — that is the point of the enum, and it is why
    /// this is a method rather than the field it replaced: the flag cannot
    /// disagree with the times it describes. It exists for
    /// [`crate::events::edit_zone`], which answers the same question for a
    /// `StoredEvent` too and so still takes a bare `bool`.
    pub(crate) fn is_all_day(&self) -> bool {
        matches!(self, When::AllDay { .. })
    }
}

/// The calendar date `ms` falls on, read in `tz`, as `yyyy-mm-dd`.
///
/// Re-exported, not defined here. It moved down to [`omacal_core::zone`] so
/// that crate's `midnight_in_zone` — the inverse derivation, and the one that
/// turns a date back into an instant — could sit beside it, and so
/// `due_reminders` could reach it without a second copy being written in the
/// crate below. Every call site here and in `commands.rs`/`events.rs` still
/// says `crate::write::date_in_zone`, and still gets the same function.
pub(crate) use omacal_core::zone::date_in_zone;

/// The two dates an all-day span covers, read in the **calendar's** zone
/// `cal_tz`: the first day it covers and the **inclusive** last one, both
/// `yyyy-mm-dd`.
///
/// The last day is one civil day back from `end_ms`'s date, because the stored
/// end is exclusive: midnight *after* the last day, as on Google's wire.
/// Deliberately not the date of `end_ms` minus some slack — `valueFromDetail`
/// steps back half a day, and half a day of slack silently absorbs a wrong
/// zone, which is how the end came out right by accident while the start was a
/// day early. A civil day back from the exclusive date is the same answer in
/// every zone and across every daylight-saving transition, since a date carries
/// no time of day for a transition to move.
///
/// The one derivation, shared by everything that needs an all-day event's
/// dates: [`crate::events`] for the popover and the edit form, and
/// [`crate::commands`] for where the chip is drawn in the grid. Two derivations
/// of this on this project once disagreed silently, so there is only one.
///
/// A date that will not parse or has no yesterday falls through to `start`
/// rather than panicking, the same fallback philosophy as [`date_in_zone`],
/// which cannot produce one anyway: it answers a real date for every input.
pub(crate) fn all_day_span_dates(start_ms: i64, end_ms: i64, cal_tz: &str) -> (String, String) {
    let start = date_in_zone(start_ms, cal_tz);
    let last = date_in_zone(end_ms, cal_tz)
        .parse::<jiff::civil::Date>()
        .ok()
        .and_then(|exclusive| exclusive.yesterday().ok())
        .map_or_else(|| start.clone(), |d| d.to_string());
    (start, last)
}

/// Where `target` lands after the same movement that takes `from` to `to`,
/// counted in whole days. All three are `yyyy-mm-dd`.
///
/// The date analogue of [`shifted_like`], and far shorter for the reason the
/// plan exists: a date has no zone and no time of day, so there is no
/// daylight-saving transition for a movement to drag across and no civil round
/// trip to lose anything in.
///
/// The movement must be a count of **days** and never a count of months: "one
/// month on", added to a target in a different month, is a different number of
/// days and would move a series by one to three of them. `jiff`'s default
/// largest unit for a date difference is already `Day`, so naming
/// [`jiff::Unit::Day`] changes nothing today and no mutation of it fails a test
/// — it is here to say what this depends on, and
/// `a_date_moves_by_the_days_the_user_moved_it` pins the behaviour itself from
/// the outside, whichever way it is reached.
///
/// `shifted_like`'s **first** short circuit is here and is a guarantee rather
/// than an optimisation, exactly as it is there: nothing moved means the target
/// does not move. It holds by string equality, before anything is parsed, which
/// is what makes "an untouched date sends no `start`/`end`" exact rather than
/// approximate.
///
/// Its **second** (`target == from` returns `to` unchanged) is deliberately
/// absent: it exists there to keep a repeated hour out of a civil round trip,
/// and `target + (to - from)` is already exactly `to` when `target == from`
/// with nothing in between that could disagree.
///
/// A date that will not parse can only have come from the form, and it falls
/// through to `to` — the user's own value — rather than to `target`. Both are
/// wrong; this one is wrong *loudly*, since Google rejects a `date` that is not
/// one. Falling back to `target` would produce an empty body instead and report
/// a save that silently did nothing.
pub(crate) fn shifted_date(target: &str, from: &str, to: &str) -> String {
    if from == to {
        return target.to_string();
    }
    let parse = |s: &str| s.parse::<jiff::civil::Date>().ok();
    (|| -> Option<String> {
        let movement = parse(to)?.since((jiff::Unit::Day, parse(from)?)).ok()?;
        Some(parse(target)?.checked_add(movement).ok()?.to_string())
    })()
    .unwrap_or_else(|| to.to_string())
}

/// Google's `start` and `end` objects. `tz` is read only by the timed arm.
pub(crate) fn when_json(when: &When, tz: &str) -> (Value, Value) {
    match when {
        When::AllDay { start_date, end_date } => (
            json!({ "date": start_date }),
            json!({ "date": end_date }),
        ),
        When::Timed { start_ms, end_ms } => (
            json!({ "dateTime": omacal_sync::to_rfc3339(*start_ms), "timeZone": tz }),
            json!({ "dateTime": omacal_sync::to_rfc3339(*end_ms),   "timeZone": tz }),
        ),
    }
}

/// Where `target_ms` lands after the same *calendar* movement that takes
/// `from_ms` to `to_ms`, read in `tz`.
///
/// Deliberately not `target_ms + (to_ms - from_ms)`. An edit is applied to a
/// resource that may be anchored a long way from the occurrence the user was
/// looking at — a series master months earlier — and a daylight-saving
/// transition can sit between the two. A millisecond delta then carries the
/// transition across with it: moving an occurrence from Saturday to Sunday
/// over a spring-forward is 23 hours, and 23 hours added to a master on the
/// other side of it arrives an hour early. For an all-day event that is worse
/// than untidy — 23 hours from midnight is 23:00 the same day, so the whole
/// move is silently discarded, while a PATCH still goes out with
/// `sendUpdates=all` and mails every guest about a change that did not
/// happen.
///
/// So the movement is measured *civilly*: the span between two wall clocks,
/// which `jiff` balances into days plus a time of day, added to the target's
/// own wall clock and only then resolved back to an instant. A day stays a
/// day across a transition; an hour stays an hour.
///
/// The two short circuits are not optimisations, they are guarantees. Nothing
/// moved means the target does not move, whatever `tz` says — that is what
/// makes "an untouched time sends no `start`/`end`" exact rather than
/// approximate. And when the target *is* the thing that moved (every one-off,
/// and every resolved occurrence) the answer is the new instant itself, with
/// no civil round trip that a repeated hour could shift.
///
/// An unresolvable zone falls back to the plain delta rather than failing, the
/// same fallback philosophy as [`date_in_zone`].
///
/// This is the *instant* half. An all-day event moves through
/// [`shifted_date`], where none of the above applies because a date carries no
/// zone at all.
pub(crate) fn shifted_like(target_ms: i64, from_ms: i64, to_ms: i64, tz: &str) -> i64 {
    if to_ms == from_ms {
        return target_ms;
    }
    if target_ms == from_ms {
        return to_ms;
    }

    let civil = |ms: i64| -> Option<jiff::civil::DateTime> {
        jiff::Timestamp::from_millisecond(ms).ok()?.in_tz(tz).ok().map(|z| z.datetime())
    };
    let moved = (|| -> Option<i64> {
        let movement = civil(to_ms)? - civil(from_ms)?;
        Some(
            civil(target_ms)?
                .checked_add(movement)
                .ok()?
                .in_tz(tz)
                .ok()?
                .timestamp()
                .as_millisecond(),
        )
    })();
    moved.unwrap_or_else(|| target_ms.saturating_add(to_ms.saturating_sub(from_ms)))
}

/// A PATCH body carrying only what actually changed.
///
/// A field absent from a PATCH body means "leave it alone"; a field present
/// and null means "clear it". Both are needed, and conflating them makes
/// clearing a location impossible.
///
/// `create_event` builds its insert body from `EventFields` directly instead —
/// a create has no "before" to diff against. The edit command is this
/// function's consumer: `events::edit_patch_body` builds both sides and calls
/// it, and its doc comment is where the rules for *how* each side is built
/// live.
pub(crate) fn changed_fields(before: &EventFields, after: &EventFields) -> Value {
    let mut body = serde_json::Map::new();

    let mut text = |key: &str, b: &Option<String>, a: &Option<String>| {
        if b != a {
            body.insert(
                key.to_string(),
                match a {
                    Some(s) => Value::String(s.clone()),
                    None => Value::Null,
                },
            );
        }
    };
    text("summary", &before.summary, &after.summary);
    text("location", &before.location, &after.location);
    text("description", &before.description, &after.description);

    // Times move as a pair. Google rejects a body that redefines one end of
    // an event without the other when the two ends disagree about their value
    // type, and half a move is not a thing a user can mean.
    //
    // **One comparison**, because [`When`] holds both ends and the shape
    // together. An all-day date compares as a *string*, so a date nobody
    // touched is equal on both sides with no instant and no zone anywhere near
    // it — which is the whole reason the type exists. The three fields this
    // replaced compared instants that the two sides of the boundary had built
    // from the same date in *different* zones, so they could not be equal, and
    // a title-only save moved the event a day with `sendUpdates=all`.
    //
    // `tz` is in this trigger too: it never appears in the body by itself, but
    // it changes what a timed `dateTime` serializes to, so a zone-only edit
    // must still resend both ends. It can say nothing about an all-day event —
    // `when_json`'s date arm ignores `tz` — and in practice cannot fire alone
    // for one either, since `events::edit_zone` puts both sides through the
    // same rule.
    if before.when != after.when || before.tz != after.tz {
        let (start, end) = when_json(&after.when, &after.tz);
        body.insert("start".into(), start);
        body.insert("end".into(), end);
    }

    match &after.recurrence {
        None => {}
        Some(None) => {
            body.insert("recurrence".into(), Value::Null);
        }
        Some(Some(rule)) => {
            body.insert("recurrence".into(), json!([rule]));
        }
    }

    Value::Object(body)
}

/// What the UI actually sends. Distinct from [`EventFields`] because the
/// three-state above needs two levels of `Option` and JSON has one `null`.
///
/// `repeat` carries the dropdown's own vocabulary rather than an RRULE: the UI
/// has no business authoring iCalendar, and keeping the mapping in one place
/// ([`rrule_for`]) is what makes "a rule we cannot express is never
/// overwritten" checkable.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventInput {
    pub summary: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub when: WhenInput,
    pub tz: String,
    /// Absent when the user did not touch Repeat.
    #[serde(default)]
    pub repeat: Option<String>,
    /// The selected days for a custom weekly cadence, as iCalendar's
    /// two-letter weekday codes. Meaningful only when `repeat == "weekly"`.
    ///
    /// This is structured rather than an RRULE fragment on purpose: serde
    /// rejects an unknown code at the command boundary, and the builder below
    /// owns ordering, de-duplication and iCalendar syntax. An empty list falls
    /// back to ordinary weekly rather than clearing recurrence.
    #[serde(default)]
    pub weekly_days: Option<Vec<WeekdayCode>>,
    /// How a repeating series stops. Absent and `never` both mean an
    /// unbounded rule; the explicit variant is useful on the read side and
    /// keeps the TypeScript union symmetric with [`RepeatEnd`].
    #[serde(default)]
    pub repeat_end: Option<RepeatEnd>,
    /// Absent when the user did not touch the guest list — see
    /// [`EventFields::guests`], which this becomes unchanged. `#[serde(default)]`
    /// so every payload written before this field existed still deserializes,
    /// and so a caller that has no guest editing (a drag) simply omits it.
    #[serde(default)]
    pub guests: Option<Vec<Guest>>,
    /// Absent when the user did not touch reminders — see
    /// [`EventFields::reminders`]. `#[serde(default)]` for the same two
    /// reasons as `guests`: older payloads, and callers with no reminder
    /// editing (a drag).
    #[serde(default)]
    pub reminders: Option<RemindersInput>,
}
/// A weekday as the UI names it and iCalendar writes it. The explicit serde
/// names are the command-boundary contract; accepting a free-form `String`
/// here would let a typo become an invalid RRULE sent to Google.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub(crate) enum WeekdayCode {
    #[serde(rename = "SU")]
    Su,
    #[serde(rename = "MO")]
    Mo,
    #[serde(rename = "TU")]
    Tu,
    #[serde(rename = "WE")]
    We,
    #[serde(rename = "TH")]
    Th,
    #[serde(rename = "FR")]
    Fr,
    #[serde(rename = "SA")]
    Sa,
}

/// The two RFC 5545 termination forms omacal can author, plus the ordinary
/// unbounded case. Internally tagged so malformed combinations (a date on an
/// `after`, a count on an `on`) fail at the command boundary rather than being
/// guessed at while building an RRULE.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RepeatEnd {
    Never,
    On { date: String },
    After { count: u32 },
}

impl Default for RepeatEnd {
    fn default() -> Self {
        Self::Never
    }
}

impl WeekdayCode {
    const ALL: [Self; 7] = [Self::Su, Self::Mo, Self::Tu, Self::We, Self::Th, Self::Fr, Self::Sa];

    fn text(self) -> &'static str {
        match self {
            Self::Su => "SU",
            Self::Mo => "MO",
            Self::Tu => "TU",
            Self::We => "WE",
            Self::Th => "TH",
            Self::Fr => "FR",
            Self::Sa => "SA",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|candidate| *candidate == self).unwrap_or(0)
    }

    fn from_text(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|day| day.text() == text)
    }
}

/// [`When`] as the UI sends it. A separate type for the same reason
/// [`EventInput`] is separate from [`EventFields`]: the wire vocabulary is
/// `serde`'s business and the domain type's is not. [`When`] deliberately
/// derives no `Deserialize` at all, so there is exactly one way for a payload
/// to become one and it goes through [`fields_from_input`].
///
/// **Internally tagged, with no `default` and no `untagged` fallback
/// anywhere.** A payload that names neither shape — or names `allDay` and then
/// sends instants — fails to deserialize and the command answers with an
/// error, rather than quietly becoming a timed event at the Unix epoch and
/// PATCHing it onto somebody's calendar.
///
/// Both `rename_all` attributes are load bearing and they do different jobs:
/// `rename_all` camel-cases the *variant* names (`AllDay` → `allDay`),
/// `rename_all_fields` the fields inside them (`start_date` → `startDate`).
/// With only the first, `{"kind":"allDay","startDate":…}` — what the UI
/// actually sends — does not deserialize at all. `ui/src/lib/eventdetail.ts`
/// mirrors this shape, and `the_payload_the_ui_sends_deserializes_as_written`
/// pins the exact strings on this side of it.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub(crate) enum WhenInput {
    Timed { start_ms: i64, end_ms: i64 },
    AllDay { start_date: String, end_date: String },
}

/// `"never"` maps to `Some(None)` — clear the rule — because [`rrule_for`]
/// returns `None` for it. That is the one case worth staring at: an absent
/// `repeat` and a `repeat` of `"never"` must not collapse together.
pub(crate) fn fields_from_input(input: EventInput) -> Result<EventFields, String> {
    let when = match input.when {
        WhenInput::Timed { start_ms, end_ms } => When::Timed { start_ms, end_ms },
        WhenInput::AllDay { start_date, end_date } => When::AllDay { start_date, end_date },
    };
    let recurrence = input
        .repeat
        .as_deref()
        .map(|repeat| {
            rrule_for_input(
                repeat,
                input.weekly_days.as_deref(),
                input.repeat_end.as_ref(),
                &when,
                &input.tz,
            )
        })
        .transpose()?;
    Ok(EventFields {
        summary: input.summary,
        location: input.location,
        description: input.description,
        when,
        tz: input.tz,
        recurrence,
        guests: input.guests,
        reminders: input.reminders,
    })
}

/// The rule omacal writes for each Repeat option. `never` is `None`.
pub(crate) fn rrule_for(repeat: &str) -> Option<String> {
    Some(
        match repeat {
            "daily" => "RRULE:FREQ=DAILY",
            "weekdays" => "RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR",
            "weekly" => "RRULE:FREQ=WEEKLY",
            "monthly" => "RRULE:FREQ=MONTHLY",
            "yearly" => "RRULE:FREQ=YEARLY",
            _ => return None,
        }
        .to_string(),
    )
}

/// The RRULE for a command payload. `rrule_for` remains the dropdown's base
/// vocabulary; this adds the two structured refinements the form can author:
/// a weekly day pattern and an optional COUNT/UNTIL termination.
fn rrule_for_input(
    repeat: &str,
    weekly_days: Option<&[WeekdayCode]>,
    repeat_end: Option<&RepeatEnd>,
    when: &When,
    tz: &str,
) -> Result<Option<String>, String> {
    let Some(mut rule) = rrule_for(repeat) else {
        if matches!(repeat_end, Some(end) if end != &RepeatEnd::Never) {
            return Err("a repeat ending needs a repeating schedule".into());
        }
        return Ok(None);
    };

    if repeat == "weekly" {
        if let Some(days) = weekly_days {
            // Canonical Sunday-first order and no duplicates, independent of
            // click or NLP order. Besides producing stable rules, this gives
            // the read side one finite, fully representable grammar to recognise.
            let mut selected = [false; 7];
            for day in days {
                selected[day.index()] = true;
            }
            let days = WeekdayCode::ALL
                .into_iter()
                .filter(|day| selected[day.index()])
                .map(WeekdayCode::text)
                .collect::<Vec<_>>();
            if !days.is_empty() {
                rule.push_str(";BYDAY=");
                rule.push_str(&days.join(","));
            }
        }
    }

    match repeat_end.unwrap_or(&RepeatEnd::Never) {
        RepeatEnd::Never => {}
        RepeatEnd::After { count } => {
            if *count == 0 {
                return Err("a repeating event must occur at least once".into());
            }
            rule.push_str(&format!(";COUNT={count}"));
        }
        RepeatEnd::On { date } => {
            let end: jiff::civil::Date = date
                .parse()
                .map_err(|_| format!("{date} is not a valid repeat end date"))?;
            let start_text = match when {
                When::Timed { start_ms, .. } => date_in_zone(*start_ms, tz),
                When::AllDay { start_date, .. } => start_date.clone(),
            };
            let start: jiff::civil::Date = start_text
                .parse()
                .map_err(|_| format!("{start_text} is not a valid event start date"))?;
            if end < start {
                return Err("the repeat end date cannot be before the event starts".into());
            }

            let until = match when {
                When::AllDay { .. } => end.strftime("%Y%m%d").to_string(),
                When::Timed { .. } => end
                    .at(23, 59, 59, 0)
                    .in_tz(tz)
                    .map_err(|_| format!("unknown time zone {tz}"))?
                    .timestamp()
                    .in_tz("UTC")
                    .expect("UTC always resolves")
                    .strftime("%Y%m%dT%H%M%SZ")
                    .to_string(),
            };
            rule.push_str(";UNTIL=");
            rule.push_str(&until);
        }
    }
    Ok(Some(rule))
}

fn parse_weekdays(list: &str) -> Option<Vec<WeekdayCode>> {
    if list.is_empty() { return None; }
    let mut selected = [false; 7];
    for raw in list.split(',') {
        let day = WeekdayCode::from_text(raw)?;
        if selected[day.index()] {
            return None;
        }
        selected[day.index()] = true;
    }
    Some(
        WeekdayCode::ALL
            .into_iter()
            .filter(|day| selected[day.index()])
            .collect(),
    )
}

fn basic_date(value: &str) -> Option<jiff::civil::Date> {
    if value.len() != 8 || !value.bytes().all(|b| b.is_ascii_digit()) { return None; }
    format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..8]).parse().ok()
}

fn basic_utc_timestamp(value: &str) -> Option<jiff::Timestamp> {
    let b = value.as_bytes();
    if b.len() != 16 || b[8] != b'T' || b[15] != b'Z'
        || !b[..8].iter().chain(&b[9..15]).all(|c| c.is_ascii_digit())
    {
        return None;
    }
    format!(
        "{}-{}-{}T{}:{}:{}Z",
        &value[..4], &value[4..6], &value[6..8],
        &value[9..11], &value[11..13], &value[13..15],
    )
    .parse()
    .ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecurrenceControls {
    pub repeat: String,
    pub weekly_days: Vec<String>,
    pub repeat_end: RepeatEnd,
}

/// Reads exactly the recurrence grammar the editor can write: one of its five
/// cadences, an optional weekly BYDAY list, and at most one COUNT or UNTIL.
/// Parts may arrive in any order because other calendar clients reorder them;
/// unknown, duplicate, contradictory or value-type-mismatched parts make the
/// whole rule custom. That all-or-nothing rule is what prevents a title-only
/// save from simplifying somebody else's recurrence behind their back.
pub(crate) fn recurrence_controls_from_rrule(
    rule: Option<&str>,
    is_all_day: bool,
    tz: &str,
) -> RecurrenceControls {
    let custom = || RecurrenceControls {
        repeat: "custom".into(), weekly_days: vec![], repeat_end: RepeatEnd::Never,
    };
    let Some(rule) = rule else {
        return RecurrenceControls {
            repeat: "never".into(), weekly_days: vec![], repeat_end: RepeatEnd::Never,
        };
    };
    let Some(body) = rule.strip_prefix("RRULE:") else { return custom(); };

    let mut freq: Option<&str> = None;
    let mut byday: Option<&str> = None;
    let mut count: Option<&str> = None;
    let mut until: Option<&str> = None;
    for part in body.split(';') {
        let Some((key, value)) = part.split_once('=') else { return custom(); };
        if value.is_empty() { return custom(); }
        let slot = match key {
            "FREQ" => &mut freq,
            "BYDAY" => &mut byday,
            "COUNT" => &mut count,
            "UNTIL" => &mut until,
            _ => return custom(),
        };
        if slot.replace(value).is_some() { return custom(); }
    }
    if count.is_some() && until.is_some() { return custom(); }

    let days = match byday {
        Some(list) => match parse_weekdays(list) {
            Some(days) => days,
            None => return custom(),
        },
        None => vec![],
    };
    let repeat = match (freq, byday) {
        (Some("DAILY"), None) => "daily",
        (Some("WEEKLY"), None) => "weekly",
        (Some("WEEKLY"), Some(_)) => {
            if days == [WeekdayCode::Mo, WeekdayCode::Tu, WeekdayCode::We,
                        WeekdayCode::Th, WeekdayCode::Fr] {
                "weekdays"
            } else {
                "weekly"
            }
        }
        (Some("MONTHLY"), None) => "monthly",
        (Some("YEARLY"), None) => "yearly",
        _ => return custom(),
    };

    let repeat_end = if let Some(raw) = count {
        match raw.parse::<u32>().ok().filter(|n| *n > 0) {
            Some(count) => RepeatEnd::After { count },
            None => return custom(),
        }
    } else if let Some(raw) = until {
        if is_all_day {
            match basic_date(raw) {
                Some(date) => RepeatEnd::On { date: date.to_string() },
                None => return custom(),
            }
        } else {
            let Some(ts) = basic_utc_timestamp(raw) else { return custom(); };
            let Ok(local) = ts.in_tz(tz) else { return custom(); };
            RepeatEnd::On { date: local.date().to_string() }
        }
    } else {
        RepeatEnd::Never
    };

    RecurrenceControls {
        repeat: repeat.into(),
        weekly_days: days.into_iter().map(|day| day.text().to_string()).collect(),
        repeat_end,
    }
}

/// The `UNTIL` value that ends a series immediately before `before_ms`.
///
/// Two forms, because RFC 5545 §3.3.10 requires `UNTIL` to carry the same
/// *value type* as `DTSTART`, and getting that wrong produces a rule other
/// clients reject even where Google is lenient about it.
///
/// A timed series has a `DTSTART` with a zone, so `UNTIL` must be a UTC
/// date-time: `before_ms` less one second, rendered `%Y%m%dT%H%M%SZ`. One
/// second, because `UNTIL` is *inclusive* — landing it on the occurrence
/// itself keeps that occurrence in both series, and a whole minute earlier
/// would drop any occurrence in between.
///
/// An all-day series has a `DTSTART` that is a bare date, so `UNTIL` must be a
/// bare date too — and "the moment before" is then the *previous day*, not the
/// previous second, since a date-valued `UNTIL` is inclusive of the whole day
/// it names. The date is read in `tz`.
///
/// **`is_all_day` is the value type of the series being truncated, and of
/// nothing else.** Worth stating flatly, because the plausible-sounding wrong
/// answer is right next to it: the caller (`events::split_series`) creates a
/// *second* event in the same breath, and that one has an all-day flag of its
/// own — the form's. They are unrelated. A truncation patches `recurrence` and
/// nothing else, so the event it lands on keeps whatever `start` it already
/// had; the rule has to agree with *that*. A user splitting an all-day series
/// into a timed remainder is an ordinary thing to do, and taking the new
/// event's flag there writes a date-time `UNTIL` onto a series whose `DTSTART`
/// is still a bare date. An earlier version of this comment said the date was
/// read in the same zone the sibling `start` is rendered in — true of the
/// fixtures, false as a rule, and an instruction to introduce exactly that bug.
///
/// **No daylight-saving hazard, unlike [`shifted_like`].** The timed form is an
/// absolute UTC instant on both sides: one second is subtracted from epoch
/// milliseconds and rendered in UTC, with no wall clock and no zone in
/// between, so no transition can move it. The all-day form does read a wall
/// clock, but only to name a calendar date — and a date is what a transition
/// leaves alone. `shifted_like` needed civil arithmetic precisely because it
/// carries a *span* across a transition; nothing here carries a span.
pub(crate) fn until_value(before_ms: i64, is_all_day: bool, tz: &str) -> String {
    let at = |ms: i64| {
        let ts = jiff::Timestamp::from_millisecond(ms).unwrap_or(jiff::Timestamp::UNIX_EPOCH);
        ts.in_tz(tz).unwrap_or_else(|_| ts.in_tz("UTC").expect("UTC always resolves"))
    };
    if is_all_day {
        let date = at(before_ms).date();
        // `yesterday` fails only at the very start of the supported range.
        date.yesterday().unwrap_or(date).strftime("%Y%m%d").to_string()
    } else {
        jiff::Timestamp::from_millisecond(before_ms.saturating_sub(1000))
            .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
            .in_tz("UTC")
            .expect("UTC always resolves")
            .strftime("%Y%m%dT%H%M%SZ")
            .to_string()
    }
}

/// One `RRULE` line, ending strictly before `before_ms`.
///
/// This is half of "this and following": the original series is shortened to
/// stop just before the occurrence the user split at, and a new series carries
/// the rest. `events::split_series` is the other half.
///
/// Three rewrites, each of which produces an invalid or wrong rule if skipped:
///
/// * **`UNTIL` is added** at [`until_value`]'s instant — see there for why it
///   is one second (or one day) before rather than on the occurrence.
/// * **An existing `UNTIL` is replaced, not appended.** Two `UNTIL` parts in
///   one rule is not valid iCalendar, and which one a parser honours is
///   anyone's guess.
/// * **`COUNT` is dropped.** RFC 5545 §3.3.10 makes `COUNT` and `UNTIL`
///   mutually exclusive: "the UNTIL and COUNT rule parts are OPTIONAL, but
///   they MUST NOT occur in the same 'recur'". A rule carrying both is
///   rejected outright by some parsers and silently resolved by others.
///
/// Everything else is carried through untouched and in its original order —
/// `INTERVAL`, `BYDAY`, `WKST`, and any part this app does not model. The
/// `RRULE:` prefix is split off first rather than matched as text, so a rule
/// whose first part happens to be `UNTIL` is still rewritten rather than
/// having it hide behind the prefix.
///
/// Parts are matched on their key, case-insensitively: RFC 5545's ABNF spells
/// them uppercase and Google always emits them that way, but a rule that
/// arrived from another client is not this function's to trust.
pub(crate) fn truncated_rule(rule: &str, before_ms: i64, is_all_day: bool, tz: &str) -> String {
    let (prefix, parts) = match rule.split_once(':') {
        Some((name, parts)) => (format!("{name}:"), parts),
        None => (String::new(), rule),
    };

    let mut kept: Vec<String> = parts
        .split(';')
        .filter(|part| {
            let key = part.split('=').next().unwrap_or_default();
            !key.eq_ignore_ascii_case("UNTIL") && !key.eq_ignore_ascii_case("COUNT")
        })
        .map(str::to_string)
        .collect();
    // Not `parts.split(';')` straight into `format!`: a rule that was nothing
    // but a `COUNT` would leave an empty first part and produce `RRULE:;UNTIL=`.
    kept.retain(|p| !p.is_empty());
    kept.push(format!("UNTIL={}", until_value(before_ms, is_all_day, tz)));

    format!("{prefix}{}", kept.join(";"))
}

/// Whether `line` is the `RRULE` of a `recurrence` array.
///
/// Google's `recurrence` is a list of iCalendar *lines*, only one of which is
/// the repeat rule — the others are `EXDATE`/`RDATE` lists naming individual
/// occurrences that were removed from or added to the series. Only the `RRULE`
/// is truncated; the rest are carried across a split verbatim, since an
/// `EXDATE` in the tail is an occurrence somebody deleted and re-creating it
/// would resurrect a meeting that was cancelled.
pub(crate) fn is_rrule(line: &str) -> bool {
    line.trim_start().len() >= 5 && line.trim_start()[..5].eq_ignore_ascii_case("RRULE")
}

/// Whether `rule` ends after a fixed number of occurrences rather than on a
/// date. Splitting one of these correctly means working out how many
/// occurrences the first half consumed, which `events::split_series` refuses
/// to guess at — see its doc comment.
pub(crate) fn has_count(rule: &str) -> bool {
    let parts = rule.split_once(':').map_or(rule, |(_, p)| p);
    parts
        .split(';')
        .any(|part| part.split('=').next().unwrap_or_default().eq_ignore_ascii_case("COUNT"))
}

/// Which Repeat option represents `rule` completely. Kept as a small wrapper
/// for callers/tests interested in only that one field; the full read side
/// uses [`recurrence_controls_from_rrule`] once and takes all three controls.
#[cfg(test)]
pub(crate) fn repeat_from_rrule(rule: Option<&str>, is_all_day: bool, tz: &str) -> String {
    recurrence_controls_from_rrule(rule, is_all_day, tz).repeat
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> EventFields {
        EventFields {
            summary: Some("Standup".into()),
            location: None,
            description: None,
            when: When::Timed { start_ms: 1_785_398_400_000, end_ms: 1_785_400_200_000 },
            tz: "Europe/Sofia".into(),
            recurrence: None,
            guests: None,
            reminders: None,
        }
    }

    /// [`base`] as an all-day event on `start`..`end` — `end` **exclusive**,
    /// the day after the last one, as everywhere else. The `tz` it inherits is
    /// deliberately left in place: an all-day event has no zone, and every
    /// assertion below that passes while one is present is one more thing
    /// proving the date never consults it.
    fn all_day_fields(start: &str, end: &str) -> EventFields {
        EventFields {
            when: When::AllDay { start_date: start.into(), end_date: end.into() },
            ..base()
        }
    }

    /// The property the whole plan exists for: an all-day date that nobody
    /// edited compares equal on both sides, so no `start`/`end` is sent at all.
    ///
    /// Under the old shape the two sides held *instants* built from that same
    /// date in different zones — the store's calendar zone and the form's
    /// browser zone — so they could not compare equal, and a title-only save
    /// moved the event a day with `sendUpdates=all`.
    #[test]
    fn an_untouched_all_day_date_produces_an_empty_body() {
        let before = all_day_fields("2026-08-10", "2026-08-11");
        let after = all_day_fields("2026-08-10", "2026-08-11");
        assert_eq!(changed_fields(&before, &after), serde_json::json!({}));
    }

    #[test]
    fn a_changed_all_day_date_sends_both_dates() {
        let before = all_day_fields("2026-08-10", "2026-08-11");
        let after = all_day_fields("2026-08-12", "2026-08-13");
        let body = changed_fields(&before, &after);
        assert_eq!(body["start"], serde_json::json!({ "date": "2026-08-12" }));
        assert_eq!(body["end"], serde_json::json!({ "date": "2026-08-13" }));
    }

    /// Switching an event between all-day and timed is a real change even when
    /// the day is the same, and the two variants can never compare equal.
    #[test]
    fn changing_between_all_day_and_timed_always_sends_times() {
        let before = all_day_fields("2026-08-10", "2026-08-11");
        let mut after = before.clone();
        after.when = When::Timed { start_ms: 1_785_398_400_000, end_ms: 1_785_402_000_000 };
        let body = changed_fields(&before, &after);
        assert!(body.get("start").is_some(), "body was {body}");
        assert!(body["start"]["dateTime"].is_string());
    }

    /// The property the whole module exists for. A fortnightly meeting whose
    /// title changes must not carry `recurrence` in its body — the Repeat
    /// dropdown cannot express "every 2nd Tuesday", so sending it would
    /// silently rewrite the real rule to something simpler.
    #[test]
    fn an_untouched_recurrence_is_never_sent() {
        let mut after = base();
        after.summary = Some("Standup (moved)".into());
        let body = changed_fields(&base(), &after);
        assert_eq!(body["summary"], "Standup (moved)");
        assert!(body.get("recurrence").is_none(), "body was {body}");
    }

    #[test]
    fn nothing_changed_produces_an_empty_body() {
        assert_eq!(changed_fields(&base(), &base()), serde_json::json!({}));
    }

    /// Clearing a field must send explicit null, not omit it — omitting means
    /// "leave alone" to a PATCH, so a cleared location would silently persist.
    #[test]
    fn clearing_a_field_sends_null_rather_than_omitting_it() {
        let mut before = base();
        before.location = Some("Room 4A".into());
        let body = changed_fields(&before, &base());
        assert!(body.get("location").is_some(), "body was {body}");
        assert!(body["location"].is_null());
    }

    /// Google rejects a body with start but not end when only one moved, and
    /// a half-moved event is meaningless anyway.
    #[test]
    fn moving_either_end_sends_both_times() {
        let mut after = base();
        after.when = When::Timed { start_ms: 1_785_398_400_000, end_ms: 1_785_401_100_000 };
        let body = changed_fields(&base(), &after);
        assert!(body.get("start").is_some(), "body was {body}");
        assert!(body.get("end").is_some(), "body was {body}");
    }

    #[test]
    fn a_touched_repeat_is_sent_as_an_array() {
        let mut after = base();
        after.recurrence = Some(Some("RRULE:FREQ=WEEKLY".into()));
        let body = changed_fields(&base(), &after);
        assert_eq!(body["recurrence"], serde_json::json!(["RRULE:FREQ=WEEKLY"]));
    }

    /// Turning repetition off is `recurrence: null`, which Google reads as
    /// "make this a single event". `Value`'s `Index` returns `Null` for a
    /// *missing* key too, so the presence check is load-bearing: without it
    /// this test cannot tell "sent null" from "never mentioned recurrence".
    #[test]
    fn repeat_set_to_never_sends_null() {
        let mut after = base();
        after.recurrence = Some(None);
        let body = changed_fields(&base(), &after);
        assert!(body.get("recurrence").is_some(), "body was {body}");
        assert!(body["recurrence"].is_null());
    }

    /// A zone-only edit (same wall clock, different `tz`) still changes what a
    /// timed `dateTime` serializes to, so it must not be dropped just because
    /// `when` is unchanged.
    #[test]
    fn a_timezone_only_change_still_sends_both_times() {
        let mut after = base();
        after.tz = "America/New_York".into();
        let body = changed_fields(&base(), &after);
        assert!(body.get("start").is_some(), "body was {body}");
        assert!(body.get("end").is_some(), "body was {body}");
        assert_eq!(body["start"]["timeZone"], "America/New_York");
    }

    /// The instant→date derivation reads the zone it is handed, and that is
    /// the whole of what makes the store's row and the body it produces agree:
    /// the row holds midnight in the *calendar's* zone, so only that zone
    /// returns the date sync put in. The two zones here are seven hours apart
    /// in August, either side of the instant.
    #[test]
    fn a_date_is_read_in_the_zone_it_is_asked_for() {
        // 2026-08-10T22:00:00-04:00 — still the 10th in New York, already the
        // 11th in Sofia.
        let ms = "2026-08-10T22:00:00[America/New_York]"
            .parse::<jiff::Zoned>()
            .unwrap()
            .timestamp()
            .as_millisecond();
        assert_eq!(date_in_zone(ms, "America/New_York"), "2026-08-10");
        assert_eq!(date_in_zone(ms, "Europe/Sofia"), "2026-08-11");
    }

    /// An unresolvable zone must fall back to UTC rather than panic — the
    /// fallback chain in `date_in_zone` still has to hold this.
    #[test]
    fn an_unknown_timezone_falls_back_to_a_date_instead_of_panicking() {
        let d = date_in_zone(1_785_398_400_000, "Not/AZone");
        assert_eq!(d.len(), 10, "got {d}");
        assert_eq!(d, date_in_zone(1_785_398_400_000, "UTC"));
    }

    /// A date movement is whole days and nothing else, so a series master
    /// months away from the occurrence the user clicked moves by exactly what
    /// they did. `shifted_like`'s daylight-saving hazard has no analogue here —
    /// there is no zone to have a transition in.
    #[test]
    fn a_date_moves_by_the_days_the_user_moved_it() {
        // Forwards, backwards, and across both a month and a year boundary.
        assert_eq!(shifted_date("2026-01-05", "2026-08-10", "2026-08-11"), "2026-01-06");
        assert_eq!(shifted_date("2026-01-05", "2026-08-10", "2026-08-09"), "2026-01-04");
        assert_eq!(shifted_date("2026-01-31", "2026-08-10", "2026-08-11"), "2026-02-01");
        assert_eq!(shifted_date("2025-12-31", "2026-08-10", "2026-08-11"), "2026-01-01");
        // A movement of a calendar month is *days*, not "the same day next
        // month": 31 days from 10 August, applied to a target in February,
        // must be 31 days there too.
        assert_eq!(shifted_date("2026-02-01", "2026-08-10", "2026-09-10"), "2026-03-04");
    }

    /// The short circuit, which is a guarantee: nothing moved moves nothing,
    /// and it holds before anything is parsed. This is what makes "an untouched
    /// date sends no `start`/`end`" exact.
    #[test]
    fn a_date_that_did_not_move_moves_nothing() {
        assert_eq!(shifted_date("2026-01-05", "2026-08-10", "2026-08-10"), "2026-01-05");
        // Even for input no parser would accept — the target is returned
        // untouched rather than the comparison failing open.
        assert_eq!(shifted_date("2026-01-05", "not a date", "not a date"), "2026-01-05");
    }

    /// A date that will not parse belongs to the form, and it is passed
    /// through rather than swallowed: Google rejects it and the user is told.
    /// Returning the target instead would build an empty body and report a save
    /// that did nothing.
    #[test]
    fn an_unparseable_date_is_sent_on_rather_than_silently_dropped() {
        assert_eq!(shifted_date("2026-01-05", "2026-08-10", "not a date"), "not a date");
    }

    #[test]
    fn an_all_day_when_needs_no_zone_and_sends_bare_dates() {
        let w = When::AllDay { start_date: "2026-08-10".into(), end_date: "2026-08-11".into() };
        // The zone is deliberately absurd: an all-day event must not consult it.
        let (start, end) = when_json(&w, "Not/AZone");
        assert_eq!(start, serde_json::json!({ "date": "2026-08-10" }));
        assert_eq!(end, serde_json::json!({ "date": "2026-08-11" }));
        assert!(start.get("dateTime").is_none());
        assert!(start.get("timeZone").is_none());
    }

    #[test]
    fn a_timed_when_sends_datetime_and_zone() {
        let w = When::Timed { start_ms: 1_785_398_400_000, end_ms: 1_785_402_000_000 };
        let (start, end) = when_json(&w, "Europe/Sofia");
        assert!(start["dateTime"].is_string());
        assert_eq!(start["timeZone"], "Europe/Sofia");
        assert!(start.get("date").is_none());
        assert!(end["dateTime"].is_string());
    }

    /// A wall clock in New York as an instant. `2026-03-08T02:00` is that
    /// zone's spring-forward for 2026, which is what the tests below sit
    /// either side of.
    fn ny(wall: &str) -> i64 {
        wall.parse::<jiff::civil::DateTime>()
            .unwrap()
            .in_tz("America/New_York")
            .unwrap()
            .timestamp()
            .as_millisecond()
    }

    /// The property `shifted_like` exists for. Moving an occurrence from the
    /// Saturday before a spring-forward to the Sunday after it is 23 hours of
    /// elapsed time but one day of calendar time. Applied to a master a month
    /// earlier — on the winter side of the transition — the plain delta
    /// arrives an hour early and the meeting silently moves to 08:00.
    #[test]
    fn a_day_stays_a_day_when_the_shift_crosses_a_daylight_saving_transition() {
        let master = ny("2026-02-07T09:00:00");
        let occurrence = ny("2026-03-07T09:00:00");
        let moved = ny("2026-03-08T09:00:00");

        assert_eq!(
            moved - occurrence,
            23 * 3_600_000,
            "fixture check: this move must actually cross the transition, or the \
             assertion below proves nothing"
        );
        assert_eq!(
            shifted_like(master, occurrence, moved, "America/New_York"),
            ny("2026-02-08T09:00:00"),
            "a one-day move became 23 hours: the series would keep its time of day \
             only on the side of the transition it was edited from"
        );
    }

    /// The same shift measured from midnight, where a plain delta is worse
    /// than untidy: 23 hours from midnight is 23:00 the *same* day, so the date
    /// derived from it is the one the event already had and the move vanishes.
    ///
    /// **Still a live path, in a place worth naming.** An all-day event's own
    /// dates no longer travel through `shifted_like` — they go through
    /// [`shifted_date`], which has no zone to have a transition in. But
    /// `events::edit_patch_body` derives the clicked occurrence's *end* with
    /// `shifted_like`, from a pair of midnights, and then reads a date off it
    /// with [`date_in_zone`]. A millisecond delta there lands at 23:00 the
    /// previous day and the whole comparison shifts by one.
    #[test]
    fn a_shift_across_a_transition_lands_on_the_next_date_not_the_same_one() {
        let master = ny("2026-02-07T00:00:00");
        let occurrence = ny("2026-03-08T00:00:00");
        let moved = ny("2026-03-09T00:00:00");

        let shifted = shifted_like(master, occurrence, moved, "America/New_York");
        assert_eq!(shifted, ny("2026-02-08T00:00:00"));
        assert_eq!(
            date_in_zone(shifted, "America/New_York"),
            "2026-02-08",
            "the move was dropped: the date derived from this instant is the one the \
             event already has"
        );
    }

    /// The control. Without it the two tests above could pass for a reason
    /// that has nothing to do with transitions — a function that always
    /// returned "the same wall clock, one day on" would satisfy them both.
    #[test]
    fn an_ordinary_shift_with_no_transition_in_it_still_moves_by_what_the_user_did() {
        let master = ny("2026-06-06T09:00:00");
        let occurrence = ny("2026-07-04T09:00:00");

        // A pure time-of-day change.
        assert_eq!(
            shifted_like(master, occurrence, occurrence + 90 * 60_000, "America/New_York"),
            ny("2026-06-06T10:30:00")
        );
        // A day *and* a time-of-day change together.
        assert_eq!(
            shifted_like(master, occurrence, ny("2026-07-06T08:00:00"), "America/New_York"),
            ny("2026-06-08T08:00:00")
        );
    }

    /// Both short circuits, which are guarantees rather than optimisations —
    /// see the doc comment. The first is what makes "an untouched time sends
    /// nothing" exact; the second keeps every one-off and every resolved
    /// occurrence away from a civil round trip that a repeated hour could
    /// shift.
    #[test]
    fn nothing_moved_moves_nothing_and_a_target_that_is_itself_the_move_takes_the_new_instant() {
        let target = ny("2026-02-07T09:00:00");
        let from = ny("2026-03-07T09:00:00");
        assert_eq!(shifted_like(target, from, from, "America/New_York"), target);
        let to = ny("2026-03-08T09:00:00");
        assert_eq!(shifted_like(from, from, to, "America/New_York"), to);
    }

    /// An unresolvable zone must not panic or swallow the movement; it falls
    /// back to the plain delta, exactly as `date_in_zone` falls back to UTC.
    #[test]
    fn an_unknown_timezone_falls_back_to_the_plain_delta() {
        assert_eq!(shifted_like(1_000, 5_000, 8_000, "Not/AZone"), 4_000);
    }

    fn sample_input() -> EventInput {
        EventInput {
            summary: Some("Standup".into()),
            location: None,
            description: None,
            when: WhenInput::Timed {
                start_ms: 1_785_398_400_000,
                end_ms: 1_785_400_200_000,
            },
            tz: "Europe/Sofia".into(),
            repeat: None,
            weekly_days: None,
            repeat_end: None,
            guests: None,
            reminders: None,
        }
    }

    #[test]
    fn each_offered_repeat_maps_to_a_rule() {
        assert_eq!(rrule_for("never"), None);
        assert_eq!(rrule_for("daily").as_deref(), Some("RRULE:FREQ=DAILY"));
        assert_eq!(
            rrule_for("weekdays").as_deref(),
            Some("RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR")
        );
        assert_eq!(rrule_for("weekly").as_deref(), Some("RRULE:FREQ=WEEKLY"));
        assert_eq!(rrule_for("monthly").as_deref(), Some("RRULE:FREQ=MONTHLY"));
        assert_eq!(rrule_for("yearly").as_deref(), Some("RRULE:FREQ=YEARLY"));
    }

    #[test]
    fn every_rule_we_author_reads_back_as_itself() {
        for r in ["daily", "weekdays", "weekly", "monthly", "yearly"] {
            let rule = rrule_for(r).unwrap();
            assert_eq!(repeat_from_rrule(Some(&rule), false, "UTC"), r, "round trip failed for {r}");
        }
        assert_eq!(repeat_from_rrule(None, false, "UTC"), "never");
    }

    #[test]
    fn every_nonempty_weekday_pattern_round_trips_without_losing_a_day() {
        for mask in 1_u8..128 {
            let days = WeekdayCode::ALL
                .into_iter()
                .enumerate()
                .filter_map(|(i, day)| (mask & (1 << i) != 0).then_some(day))
                .collect::<Vec<_>>();
            let rule = rrule_for_input(
                "weekly", Some(&days), None, &base().when, "UTC",
            ).unwrap().unwrap();
            let expected = days.iter().map(|day| day.text().to_string()).collect::<Vec<_>>();
            let controls = recurrence_controls_from_rrule(Some(&rule), false, "UTC");
            assert_eq!(controls.weekly_days, expected, "{rule}");
            // Monday–Friday is also the existing "Every weekday" preset.
            // The labels may alias, but the selected days must never change.
            let expected_repeat = if rule == "RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR" {
                "weekdays"
            } else {
                "weekly"
            };
            assert_eq!(controls.repeat, expected_repeat, "{rule}");
        }
    }

    #[test]
    fn weekly_days_are_canonical_and_duplicate_clicks_cannot_duplicate_byday() {
        let rule = rrule_for_input(
            "weekly",
            Some(&[WeekdayCode::Fr, WeekdayCode::Mo, WeekdayCode::Fr, WeekdayCode::We]),
            None,
            &base().when,
            "UTC",
        ).unwrap();
        assert_eq!(rule.as_deref(), Some("RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR"));
    }

    #[test]
    fn the_weekly_pattern_wire_shape_is_typed_and_builds_byday() {
        let input: EventInput = serde_json::from_value(serde_json::json!({
            "when": { "kind": "timed", "startMs": 1_785_398_400_000_i64,
                      "endMs": 1_785_400_200_000_i64 },
            "tz": "UTC",
            "repeat": "weekly",
            "weeklyDays": ["FR", "MO", "WE"]
        }))
        .unwrap();
        assert_eq!(
            fields_from_input(input).unwrap().recurrence,
            Some(Some("RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR".into()))
        );

        let invalid = serde_json::from_value::<EventInput>(serde_json::json!({
            "when": { "kind": "timed", "startMs": 1_i64, "endMs": 2_i64 },
            "tz": "UTC", "repeat": "weekly", "weeklyDays": ["XX"]
        }));
        assert!(invalid.is_err(), "an unknown weekday must fail before a write");
    }

    #[test]
    fn repeat_endings_build_valid_rules_and_read_back_without_loss() {
        let counted: EventInput = serde_json::from_value(serde_json::json!({
            "when": { "kind": "timed", "startMs": 1_785_398_400_000_i64,
                      "endMs": 1_785_400_200_000_i64 },
            "tz": "Europe/Sofia", "repeat": "weekly",
            "weeklyDays": ["MO", "WE", "FR"],
            "repeatEnd": { "kind": "after", "count": 8 }
        })).unwrap();
        let counted_rule = fields_from_input(counted).unwrap().recurrence.unwrap().unwrap();
        assert_eq!(counted_rule, "RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR;COUNT=8");
        let controls = recurrence_controls_from_rrule(
            Some(&counted_rule), false, "Europe/Sofia",
        );
        assert_eq!(controls.repeat, "weekly");
        assert_eq!(controls.weekly_days, ["MO", "WE", "FR"]);
        assert_eq!(controls.repeat_end, RepeatEnd::After { count: 8 });

        let timed: EventInput = serde_json::from_value(serde_json::json!({
            "when": { "kind": "timed", "startMs": 1_785_398_400_000_i64,
                      "endMs": 1_785_400_200_000_i64 },
            "tz": "Europe/Sofia", "repeat": "daily",
            "repeatEnd": { "kind": "on", "date": "2026-08-10" }
        })).unwrap();
        let timed_rule = fields_from_input(timed).unwrap().recurrence.unwrap().unwrap();
        assert_eq!(timed_rule, "RRULE:FREQ=DAILY;UNTIL=20260810T205959Z");
        assert_eq!(
            recurrence_controls_from_rrule(Some(&timed_rule), false, "Europe/Sofia").repeat_end,
            RepeatEnd::On { date: "2026-08-10".into() },
        );

        let all_day: EventInput = serde_json::from_value(serde_json::json!({
            "when": { "kind": "allDay", "startDate": "2026-08-03",
                      "endDate": "2026-08-04" },
            "tz": "Not/AZone", "repeat": "monthly",
            "repeatEnd": { "kind": "on", "date": "2026-12-31" }
        })).unwrap();
        let all_day_rule = fields_from_input(all_day).unwrap().recurrence.unwrap().unwrap();
        assert_eq!(all_day_rule, "RRULE:FREQ=MONTHLY;UNTIL=20261231");
        assert_eq!(
            recurrence_controls_from_rrule(Some(&all_day_rule), true, "Not/AZone").repeat_end,
            RepeatEnd::On { date: "2026-12-31".into() },
        );
    }

    #[test]
    fn invalid_repeat_endings_are_refused_before_any_write() {
        for end in [
            serde_json::json!({ "kind": "after", "count": 0 }),
            serde_json::json!({ "kind": "on", "date": "2026-02-31" }),
            serde_json::json!({ "kind": "on", "date": "2026-01-01" }),
        ] {
            let input: EventInput = serde_json::from_value(serde_json::json!({
                "when": { "kind": "allDay", "startDate": "2026-08-03",
                          "endDate": "2026-08-04" },
                "tz": "UTC", "repeat": "daily", "repeatEnd": end,
            })).unwrap();
            assert!(fields_from_input(input).is_err(), "accepted {end}");
        }
    }

    /// The property that stops a silent overwrite: a rule the dropdown cannot
    /// express must be reported as `custom`, so the UI can disable the control
    /// rather than offering to replace it with something simpler.
    #[test]
    fn a_rule_we_cannot_express_is_custom() {
        for exotic in [
            "RRULE:FREQ=MONTHLY;BYDAY=-1FR",
            "RRULE:FREQ=WEEKLY;INTERVAL=2",
            "RRULE:FREQ=WEEKLY;BYDAY=MO,MO",
            "RRULE:FREQ=WEEKLY;BYDAY=1MO",
            "RRULE:FREQ=WEEKLY;BYDAY=MO,WE;WKST=SU",
            "RRULE:FREQ=WEEKLY;BYDAY=XX",
            "RRULE:FREQ=DAILY;COUNT=0",
            "RRULE:FREQ=DAILY;COUNT=5;UNTIL=20261231T000000Z",
            "RRULE:FREQ=DAILY;UNTIL=20261231",
        ] {
            assert_eq!(repeat_from_rrule(Some(exotic), false, "UTC"), "custom", "{exotic}");
        }
    }

    /// 2026-07-30T08:00:00Z. Verify with:
    /// `python3 -c "import datetime as d; print(d.datetime.fromtimestamp(1785398400, d.timezone.utc))"`
    /// — one second earlier is 07:59:59Z, which is what every `UNTIL` below
    /// expects. An earlier draft of the plan said `20260731` and was a day out.
    const SPLIT: i64 = 1_785_398_400_000;

    /// UNTIL is inclusive in RFC 5545, so it must land strictly before the
    /// occurrence that moves to the new series — one second earlier. Getting
    /// this wrong duplicates that occurrence in both series or drops it from
    /// both.
    #[test]
    fn until_lands_one_second_before_the_split() {
        let r = truncated_rule("RRULE:FREQ=WEEKLY", SPLIT, false, "UTC");
        assert_eq!(r, "RRULE:FREQ=WEEKLY;UNTIL=20260730T075959Z");
    }

    /// An existing UNTIL is replaced, not appended — two UNTILs is invalid.
    #[test]
    fn an_existing_until_is_replaced() {
        let r = truncated_rule("RRULE:FREQ=WEEKLY;UNTIL=20271231T000000Z", SPLIT, false, "UTC");
        assert_eq!(r.matches("UNTIL").count(), 1);
        assert!(r.ends_with("UNTIL=20260730T075959Z"), "got {r}");
    }

    /// COUNT and UNTIL are mutually exclusive in RFC 5545; a rule carrying
    /// COUNT must lose it when truncated.
    #[test]
    fn count_is_dropped_when_until_is_added() {
        let r = truncated_rule("RRULE:FREQ=DAILY;COUNT=10", SPLIT, false, "UTC");
        assert!(!r.contains("COUNT"), "got {r}");
        assert!(r.contains("UNTIL="));
    }

    /// The parts this app cannot author are exactly the ones a truncation must
    /// not disturb: "every second Tuesday and Thursday" is carried through in
    /// its original order, with only the ending rewritten. Rebuilding the rule
    /// from a parse, or reordering it, is how a fortnightly meeting quietly
    /// becomes a weekly one.
    #[test]
    fn every_other_part_survives_truncation_in_its_original_order() {
        let r = truncated_rule(
            "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=TU,TH;WKST=SU;COUNT=8",
            SPLIT,
            false,
            "UTC",
        );
        assert_eq!(r, "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=TU,TH;WKST=SU;UNTIL=20260730T075959Z");
    }

    /// The timed form is an absolute UTC instant on both sides, so no zone the
    /// caller passes can move it — which is what makes "no DST hazard" a
    /// property rather than a hope. `Pacific/Kiritimati` (UTC+14) and
    /// `Pacific/Niue` (UTC-11) are 25 hours apart and would land on different
    /// *dates* if the zone were consulted at all.
    #[test]
    fn a_timed_until_is_the_same_instant_whatever_zone_it_is_asked_for() {
        let utc = truncated_rule("RRULE:FREQ=WEEKLY", SPLIT, false, "UTC");
        for tz in ["Pacific/Kiritimati", "Pacific/Niue", "America/New_York", "Not/AZone"] {
            assert_eq!(truncated_rule("RRULE:FREQ=WEEKLY", SPLIT, false, tz), utc, "{tz}");
        }
    }

    /// RFC 5545 §3.3.10: UNTIL must carry the same value type as DTSTART. An
    /// all-day series' DTSTART is a bare date, so its UNTIL must be one too —
    /// and a date-valued UNTIL is inclusive of the whole day it names, so
    /// "before this occurrence" is the *previous day*, not the previous
    /// second. Emitting `20260730T075959Z` here produces a rule other clients
    /// reject, and one whose last day is ambiguous even where they don't.
    #[test]
    fn an_all_day_until_is_a_bare_date_on_the_previous_day() {
        // Midnight on 2026-07-30 in Sofia, the instant an all-day occurrence
        // on that date is stored at.
        let midnight = "2026-07-30T00:00:00"
            .parse::<jiff::civil::DateTime>()
            .unwrap()
            .in_tz("Europe/Sofia")
            .unwrap()
            .timestamp()
            .as_millisecond();
        let r = truncated_rule("RRULE:FREQ=WEEKLY", midnight, true, "Europe/Sofia");
        assert_eq!(r, "RRULE:FREQ=WEEKLY;UNTIL=20260729");
    }

    /// The all-day date is read in the zone it is *stored* against, the same
    /// one [`date_in_zone`] derives the `start` of the very same write from.
    /// Reading it in UTC instead is a day out for every zone whose midnight
    /// falls on the other side of it — `Pacific/Auckland` (UTC+12) is midday
    /// the *previous* day in UTC, so the series would keep an occurrence the
    /// user split away.
    #[test]
    fn an_all_day_until_is_read_in_the_events_own_zone_not_utc() {
        let midnight = "2026-07-30T00:00:00"
            .parse::<jiff::civil::DateTime>()
            .unwrap()
            .in_tz("Pacific/Auckland")
            .unwrap()
            .timestamp()
            .as_millisecond();
        assert_eq!(
            jiff::Timestamp::from_millisecond(midnight).unwrap().to_string(),
            "2026-07-29T12:00:00Z",
            "fixture check: this instant must fall on the previous date in UTC, or the \
             assertion below proves nothing"
        );
        let r = truncated_rule("RRULE:FREQ=WEEKLY", midnight, true, "Pacific/Auckland");
        assert_eq!(r, "RRULE:FREQ=WEEKLY;UNTIL=20260729");
    }

    /// Google always emits uppercase, but a rule that arrived through another
    /// client is not ours to trust: a lowercase `count=` left in place is a
    /// rule carrying both COUNT and UNTIL, which RFC 5545 forbids outright.
    #[test]
    fn a_lowercase_count_or_until_is_still_replaced() {
        let r = truncated_rule("RRULE:freq=DAILY;count=10;until=20271231T000000Z", SPLIT, false, "UTC");
        assert_eq!(r, "RRULE:freq=DAILY;UNTIL=20260730T075959Z");
    }

    #[test]
    fn an_rrule_is_told_from_the_other_recurrence_lines() {
        assert!(is_rrule("RRULE:FREQ=WEEKLY"));
        assert!(is_rrule("rrule:FREQ=WEEKLY"));
        assert!(!is_rrule("EXDATE;TZID=Europe/Sofia:20260810T090000"));
        assert!(!is_rrule("RDATE:20260810T090000Z"));
        assert!(!is_rrule(""));
    }

    #[test]
    fn a_count_rule_is_told_from_a_dated_one() {
        assert!(has_count("RRULE:FREQ=DAILY;COUNT=10"));
        assert!(has_count("RRULE:FREQ=DAILY;count=10"));
        assert!(!has_count("RRULE:FREQ=DAILY"));
        assert!(!has_count("RRULE:FREQ=DAILY;UNTIL=20271231T000000Z"));
        // Not a substring match: `BYSETPOS` and friends must not be mistaken
        // for a `COUNT`, and a value that happens to contain the word is not a
        // key. Both would refuse a split that is perfectly safe.
        assert!(!has_count("RRULE:FREQ=MONTHLY;BYDAY=MO;BYSETPOS=-1"));
    }

    /// The two states JSON cannot tell apart on its own, and the reason
    /// `EventInput` exists. An absent `repeat` must leave the rule alone; an
    /// explicit `"never"` must clear it. Collapsing them makes every title edit
    /// on a recurring event either impossible or destructive.
    #[test]
    fn an_absent_repeat_and_an_explicit_never_are_different_things() {
        let mut input = sample_input();
        input.repeat = None;
        assert_eq!(fields_from_input(input).unwrap().recurrence, None);

        let mut input = sample_input();
        input.repeat = Some("never".into());
        assert_eq!(fields_from_input(input).unwrap().recurrence, Some(None));

        let mut input = sample_input();
        input.repeat = Some("weekly".into());
        assert_eq!(
            fields_from_input(input).unwrap().recurrence,
            Some(Some("RRULE:FREQ=WEEKLY".into()))
        );
    }

    /// The exact JSON `ui/src/lib/eventdetail.ts` sends, written out as a
    /// string rather than built from the type — so this fails if either
    /// `rename_all` attribute is dropped, which is the whole failure mode worth
    /// pinning. `rename_all` alone leaves the fields snake_case and the all-day
    /// payload below stops deserializing; `rename_all_fields` alone leaves the
    /// variants `Timed`/`AllDay` and neither does.
    ///
    /// Both arms, because the two names differ in different places.
    #[test]
    fn the_payload_the_ui_sends_deserializes_as_written() {
        let timed: EventInput = serde_json::from_str(
            r#"{"summary":"Standup","location":null,"description":null,
                "when":{"kind":"timed","startMs":1785398400000,"endMs":1785400200000},
                "tz":"Europe/Sofia"}"#,
        )
        .expect("the timed payload the UI sends must deserialize");
        assert_eq!(
            fields_from_input(timed).unwrap().when,
            When::Timed { start_ms: 1_785_398_400_000, end_ms: 1_785_400_200_000 }
        );

        let all_day: EventInput = serde_json::from_str(
            r#"{"summary":"Berlin trip","location":null,"description":null,
                "when":{"kind":"allDay","startDate":"2026-08-10","endDate":"2026-08-11"},
                "tz":"Europe/Sofia"}"#,
        )
        .expect("the all-day payload the UI sends must deserialize");
        assert_eq!(
            fields_from_input(all_day).unwrap().when,
            When::AllDay { start_date: "2026-08-10".into(), end_date: "2026-08-11".into() }
        );
    }

    #[test]
    fn the_reminders_payload_the_ui_sends_deserializes_as_written() {
        let input: EventInput = serde_json::from_str(
            r#"{"summary":"Standup","location":null,"description":null,
                "when":{"kind":"timed","startMs":1785398400000,"endMs":1785400200000},
                "tz":"Europe/Sofia",
                "reminders":{"useDefault":false,
                             "overrides":[{"method":"popup","minutes":15}]}}"#,
        )
        .expect("the reminders payload the UI sends must deserialize");
        assert_eq!(
            fields_from_input(input).unwrap().reminders,
            Some(RemindersInput {
                use_default: false,
                overrides: vec![ReminderInput { method: "popup".into(), minutes: 15 }],
            })
        );
    }

    #[test]
    fn reminders_render_as_the_whole_object_google_replaces() {
        let r = RemindersInput {
            use_default: false,
            overrides: vec![
                ReminderInput { method: "popup".into(), minutes: 15 },
                ReminderInput { method: "email".into(), minutes: 1440 },
            ],
        };
        assert_eq!(
            reminders_json(&r),
            json!({ "useDefault": false,
                    "overrides": [ { "method": "popup", "minutes": 15 },
                                   { "method": "email", "minutes": 1440 } ] })
        );
    }

    /// Spec §4: refused with the limit in the message, never clamped. The
    /// boundary values pass; one past each fails.
    #[test]
    fn reminder_bounds_are_googles_own() {
        let ok = |overrides: Vec<ReminderInput>| RemindersInput { use_default: false, overrides };
        let one = |minutes| vec![ReminderInput { method: "popup".into(), minutes }];

        assert!(validate_reminders(&ok(one(0))).is_ok());
        assert!(validate_reminders(&ok(one(40_320))).is_ok());
        assert!(validate_reminders(&ok(one(40_321))).is_err());
        assert!(validate_reminders(&ok(one(-1))).is_err());

        let five = (0..5).map(|i| ReminderInput { method: "popup".into(), minutes: i }).collect();
        assert!(validate_reminders(&ok(five)).is_ok());
        let six = (0..6).map(|i| ReminderInput { method: "popup".into(), minutes: i }).collect();
        assert!(validate_reminders(&ok(six)).is_err());

        let sms = vec![ReminderInput { method: "sms".into(), minutes: 5 }];
        assert!(validate_reminders(&ok(sms)).is_err(), "an unknown method is refused");
    }

    /// The reason `when` is tagged and carries no `default` anywhere: a payload
    /// that does not say which shape it is, or says one and sends the other,
    /// must fail loudly at the boundary. Defaulting would make it a timed event
    /// at the Unix epoch and PATCH that onto somebody's calendar with
    /// `sendUpdates=all`.
    #[test]
    fn a_payload_that_names_no_shape_or_the_wrong_one_is_refused() {
        let cases = [
            // No `kind` at all — the shape the old `EventInput` accepted.
            r#"{"summary":null,"location":null,"description":null,
                "when":{"startMs":1785398400000,"endMs":1785400200000},"tz":"UTC"}"#,
            // A `kind` nothing implements.
            r#"{"summary":null,"location":null,"description":null,
                "when":{"kind":"floating","startMs":1,"endMs":2},"tz":"UTC"}"#,
            // `allDay`, with a timed payload underneath it.
            r#"{"summary":null,"location":null,"description":null,
                "when":{"kind":"allDay","startMs":1785398400000,"endMs":1785400200000},
                "tz":"UTC"}"#,
            // `when` missing entirely.
            r#"{"summary":null,"location":null,"description":null,"tz":"UTC"}"#,
        ];
        for case in cases {
            assert!(
                serde_json::from_str::<EventInput>(case).is_err(),
                "this deserialized instead of failing: {case}"
            );
        }
    }
}
