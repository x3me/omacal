//! A deliberately small ICS reader and writer.
//!
//! Small because omacal already speaks the hard half of RFC 5545: recurrence
//! lines (`RRULE`/`EXDATE`/`RDATE`) pass through **verbatim** to
//! `omacal-core`'s expander, which builds its own ICS internally. What this
//! module owns is the container: unfolding, components, parameters, the four
//! date-time shapes, text escaping, VALARM triggers — and, on the write side,
//! serializing new components and patching a VTODO's status inside an
//! otherwise-untouched resource. Round-tripping bytes we do not model is the
//! design constraint throughout: a CalDAV write replaces a whole resource,
//! so anything dropped in parsing would be deleted for real on the server.

use jiff::civil::{Date, DateTime};
use jiff::Timestamp;

/// One content line, unfolded: `NAME;PARAM=V;PARAM2=V2:value`.
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    /// Uppercased.
    pub name: String,
    /// Parameter names uppercased; values as written (quotes stripped).
    pub params: Vec<(String, String)>,
    /// Raw value — unescape with [`unescape`] where the property is TEXT.
    pub value: String,
}

impl Property {
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// One `BEGIN:`…`END:` block.
#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    /// Uppercased: `VCALENDAR`, `VEVENT`, `VTODO`, `VALARM`, …
    pub name: String,
    pub props: Vec<Property>,
    pub children: Vec<Component>,
}

impl Component {
    pub fn prop(&self, name: &str) -> Option<&Property> {
        self.props.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }
    pub fn prop_value(&self, name: &str) -> Option<&str> {
        self.prop(name).map(|p| p.value.as_str())
    }
    pub fn components<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Component> {
        self.children.iter().filter(move |c| c.name.eq_ignore_ascii_case(name))
    }
}

/// RFC 5545 line unfolding: a CRLF (or bare LF — real servers emit both)
/// followed by one space or tab continues the previous line.
fn unfold(src: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in src.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix(' ').or_else(|| line.strip_prefix('\t')) {
            if let Some(last) = out.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        if !line.is_empty() {
            out.push(line.to_string());
        }
    }
    out
}

/// Splits `NAME;PARAMS:value`, honouring quoted parameter values (a TZID may
/// legally contain a colon inside quotes).
/// Splits on a separator that is outside quotes, keeping the quotes in place
/// for the caller to trim.
fn split_unquoted(text: &str, sep: char) -> std::vec::IntoIter<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for ch in text.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                cur.push(ch);
            }
            c if c == sep && !in_quotes => out.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    out.push(cur);
    out.into_iter()
}

fn parse_line(line: &str) -> Option<Property> {
    let mut in_quotes = false;
    let mut colon = None;
    for (i, ch) in line.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    // The same quote rule as the colon scan above, and for the same reason: a
    // quoted parameter value may legally hold the `;` this splits on. `CN="Ada;
    // the second"` is one parameter, not two, and splitting it blindly loses
    // half the name — which is exactly what a name this app now *writes* would
    // have hit.
    let mut segs = split_unquoted(head, ';');
    let name = segs.next()?.trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let params = segs
        .filter_map(|s| {
            let (n, v) = s.split_once('=')?;
            Some((n.trim().to_ascii_uppercase(), v.trim().trim_matches('"').to_string()))
        })
        .collect();
    Some(Property { name, params, value: value.to_string() })
}

/// Parses a whole ICS document into its root component tree. Tolerant by
/// intent: unknown properties are kept, stray lines outside any component are
/// dropped, and an unterminated component closes at end of input — a broken
/// resource should yield what it can, not abort a sync.
pub fn parse(src: &str) -> Option<Component> {
    let mut stack: Vec<Component> = Vec::new();
    let mut root: Option<Component> = None;
    for line in unfold(src) {
        let prop = parse_line(&line)?;
        match prop.name.as_str() {
            "BEGIN" => stack.push(Component {
                name: prop.value.trim().to_ascii_uppercase(),
                props: Vec::new(),
                children: Vec::new(),
            }),
            "END" => {
                let done = stack.pop()?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(done),
                    None => root = Some(done),
                }
            }
            _ => {
                if let Some(top) = stack.last_mut() {
                    top.props.push(prop);
                }
            }
        }
    }
    // Unterminated nesting: fold whatever is open into the outermost block.
    while let Some(done) = stack.pop() {
        match stack.last_mut() {
            Some(parent) => parent.children.push(done),
            None => root = Some(done),
        }
    }
    root
}

/// RFC 5545 TEXT unescaping.
pub fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') | Some('N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// RFC 5545 TEXT escaping, for serialization.
pub fn escape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            _ => out.push(c),
        }
    }
    out
}

/// The four shapes an ICS date-time property takes.
#[derive(Debug, Clone, PartialEq)]
pub enum IcsTime {
    /// `VALUE=DATE` — an all-day date, no zone of its own.
    Date(Date),
    /// `…Z` — an instant.
    Utc(Timestamp),
    /// `TZID=…` — a wall time in a named zone.
    Zoned { dt: DateTime, tzid: String },
    /// Neither `Z` nor `TZID` — a floating wall time, resolved against the
    /// calendar's zone at conversion.
    Floating(DateTime),
}

fn parse_basic_date(v: &str) -> Option<Date> {
    if v.len() != 8 || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Date::new(v[0..4].parse().ok()?, v[4..6].parse().ok()?, v[6..8].parse().ok()?).ok()
}

fn parse_basic_datetime(v: &str) -> Option<DateTime> {
    // 20260815T093000
    let (d, t) = v.split_once('T')?;
    let date = parse_basic_date(d)?;
    if t.len() < 6 || !t.as_bytes()[..6].iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let time = jiff::civil::Time::new(
        t[0..2].parse().ok()?,
        t[2..4].parse().ok()?,
        t[4..6].parse().ok()?,
        0,
    )
    .ok()?;
    Some(date.to_datetime(time))
}

/// Reads a DTSTART/DTEND/DUE/COMPLETED/RECURRENCE-ID property into its shape.
pub fn parse_time(p: &Property) -> Option<IcsTime> {
    let v = p.value.trim();
    if p.param("VALUE").is_some_and(|x| x.eq_ignore_ascii_case("DATE")) || (v.len() == 8 && !v.contains('T')) {
        return parse_basic_date(v).map(IcsTime::Date);
    }
    if let Some(stripped) = v.strip_suffix('Z') {
        let dt = parse_basic_datetime(stripped)?;
        return dt.to_zoned(jiff::tz::TimeZone::UTC).ok().map(|z| IcsTime::Utc(z.timestamp()));
    }
    let dt = parse_basic_datetime(v)?;
    match p.param("TZID") {
        Some(tzid) => Some(IcsTime::Zoned { dt, tzid: tzid.to_string() }),
        None => Some(IcsTime::Floating(dt)),
    }
}

/// Resolves a shape to `(epoch_ms, tz_name, is_all_day)` against the
/// calendar's zone. A TZID jiff does not know (a Windows zone name, a private
/// Apple alias) falls back to the calendar's zone rather than dropping the
/// event — a meeting an hour off beats a meeting missing.
pub fn resolve(t: &IcsTime, cal_tz: &str) -> Option<(i64, String, bool)> {
    match t {
        IcsTime::Date(d) => {
            let z = d.to_datetime(jiff::civil::Time::midnight()).in_tz(cal_tz).ok()?;
            Some((z.timestamp().as_millisecond(), cal_tz.to_string(), true))
        }
        IcsTime::Utc(ts) => Some((ts.as_millisecond(), "UTC".to_string(), false)),
        IcsTime::Zoned { dt, tzid } => match dt.in_tz(tzid) {
            Ok(z) => Some((z.timestamp().as_millisecond(), tzid.clone(), false)),
            Err(_) => {
                tracing::warn!(%tzid, "unknown TZID; resolving in the calendar's zone");
                let z = dt.in_tz(cal_tz).ok()?;
                Some((z.timestamp().as_millisecond(), cal_tz.to_string(), false))
            }
        },
        IcsTime::Floating(dt) => {
            let z = dt.in_tz(cal_tz).ok()?;
            Some((z.timestamp().as_millisecond(), cal_tz.to_string(), false))
        }
    }
}

/// An ISO 8601 duration (`P1D`, `PT1H30M`, `-PT15M`) in milliseconds.
pub fn parse_duration_ms(v: &str) -> Option<i64> {
    let (neg, rest) = match v.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, v.strip_prefix('+').unwrap_or(v)),
    };
    let rest = rest.strip_prefix('P')?;
    let (date_part, time_part) = match rest.split_once('T') {
        Some((d, t)) => (d, t),
        None => (rest, ""),
    };
    let mut ms: i64 = 0;
    let mut num = String::new();
    for c in date_part.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            let n: i64 = num.parse().ok()?;
            num.clear();
            ms += n * match c {
                'W' => 7 * 86_400_000,
                'D' => 86_400_000,
                _ => return None,
            };
        }
    }
    for c in time_part.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            let n: i64 = num.parse().ok()?;
            num.clear();
            ms += n * match c {
                'H' => 3_600_000,
                'M' => 60_000,
                'S' => 1_000,
                _ => return None,
            };
        }
    }
    if !num.is_empty() {
        return None;
    }
    Some(if neg { -ms } else { ms })
}

/// One VEVENT, lifted out of a resource. Times stay in their ICS shapes;
/// the store conversion resolves them against the calendar zone.
#[derive(Debug, Clone, PartialEq)]
pub struct CalEvent {
    pub uid: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    /// Lowercased; `confirmed` when absent, as Google's mapping assumes.
    pub status: String,
    pub start: IcsTime,
    /// Exactly one of `end`/`duration_ms` is normally present; neither means
    /// the RFC's defaults (instant event, or one day for all-day).
    pub end: Option<IcsTime>,
    pub duration_ms: Option<i64>,
    /// `RRULE:`/`EXDATE…`/`RDATE…` lines verbatim — omacal-core's expander
    /// takes exactly these.
    pub recurrence: Vec<String>,
    /// Set on an exception component overriding one occurrence.
    pub recurrence_id: Option<IcsTime>,
    pub sequence: i64,
    /// Minutes-before, from relative VALARM TRIGGERs. Display/audio → popup;
    /// email → email.
    pub alarms: Vec<(String, i64)>,
    pub organizer_email: Option<String>,
    pub conference_uri: Option<String>,
    pub last_modified_ms: Option<i64>,
    pub attendees: Vec<CalAttendee>,
}

/// One invitee off an `ATTENDEE` line.
#[derive(Debug, Clone, PartialEq)]
pub struct CalAttendee {
    pub email: String,
    pub display_name: Option<String>,
    /// Google's spelling of PARTSTAT: `accepted` | `declined` | `tentative`
    /// | `needsAction` — normalised here so the store sees one vocabulary.
    pub response_status: String,
    pub optional: bool,
}

fn read_attendee(p: &Property) -> Option<CalAttendee> {
    let v = p.value.trim();
    // The scheme case-insensitively, like `is_attendee_of`: `MAILTO:` is
    // legal and real, and a scheme left on the mailbox poisons every
    // comparison downstream — the RSVP self-match most of all.
    let email = match v.get(..7) {
        Some(scheme) if scheme.eq_ignore_ascii_case("mailto:") => &v[7..],
        _ => v,
    }
    .to_string();
    if email.is_empty() {
        return None;
    }
    let response_status = match p
        .param("PARTSTAT")
        .map(str::to_ascii_uppercase)
        .as_deref()
    {
        Some("ACCEPTED") => "accepted",
        Some("DECLINED") => "declined",
        Some("TENTATIVE") => "tentative",
        _ => "needsAction",
    }
    .to_string();
    Some(CalAttendee {
        email,
        display_name: p.param("CN").map(str::to_string),
        response_status,
        optional: p.param("ROLE").is_some_and(|r| r.eq_ignore_ascii_case("OPT-PARTICIPANT")),
    })
}

/// One VTODO.
#[derive(Debug, Clone, PartialEq)]
pub struct CalTodo {
    pub uid: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub due: Option<IcsTime>,
    /// Lowercased; `needs-action` when absent.
    pub status: String,
    pub completed: Option<IcsTime>,
    pub priority: i64,
}

fn text(c: &Component, name: &str) -> Option<String> {
    c.prop_value(name).map(unescape).filter(|s| !s.trim().is_empty())
}

fn alarm_minutes(alarm: &Component) -> Option<(String, i64)> {
    let trigger = alarm.prop("TRIGGER")?;
    // Absolute triggers (VALUE=DATE-TIME) do not map to minutes-before.
    if trigger.param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE-TIME")) {
        return None;
    }
    // RELATED=END alarms are rare and mean something else; skip rather than lie.
    if trigger.param("RELATED").is_some_and(|v| v.eq_ignore_ascii_case("END")) {
        return None;
    }
    let ms = parse_duration_ms(trigger.value.trim())?;
    let minutes = (-ms) / 60_000;
    if minutes < 0 {
        return None; // an alarm after the start is not a reminder
    }
    let method = match alarm.prop_value("ACTION").map(str::to_ascii_uppercase).as_deref() {
        Some("EMAIL") => "email",
        _ => "popup",
    };
    Some((method.to_string(), minutes))
}

fn read_event(c: &Component) -> Option<CalEvent> {
    let uid = c.prop_value("UID")?.trim().to_string();
    let start = parse_time(c.prop("DTSTART")?)?;
    let recurrence = c
        .props
        .iter()
        .filter(|p| matches!(p.name.as_str(), "RRULE" | "EXDATE" | "RDATE"))
        .map(raw_line)
        .collect();
    Some(CalEvent {
        uid,
        summary: text(c, "SUMMARY"),
        description: text(c, "DESCRIPTION"),
        location: text(c, "LOCATION"),
        status: c
            .prop_value("STATUS")
            .map(|s| s.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "confirmed".into()),
        end: c.prop("DTEND").and_then(parse_time),
        duration_ms: c.prop_value("DURATION").and_then(parse_duration_ms),
        recurrence,
        recurrence_id: c.prop("RECURRENCE-ID").and_then(parse_time),
        sequence: c.prop_value("SEQUENCE").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
        alarms: c.components("VALARM").filter_map(alarm_minutes).collect(),
        organizer_email: c.prop_value("ORGANIZER").map(|v| {
            v.trim().strip_prefix("mailto:").unwrap_or(v.trim()).to_string()
        }),
        conference_uri: c
            .prop_value("URL")
            .map(str::trim)
            .filter(|v| v.starts_with("http"))
            .map(str::to_string),
        last_modified_ms: c
            .prop("LAST-MODIFIED")
            .and_then(parse_time)
            .and_then(|t| resolve(&t, "UTC"))
            .map(|(ms, _, _)| ms),
        attendees: c
            .props
            .iter()
            .filter(|p| p.name == "ATTENDEE")
            .filter_map(read_attendee)
            .collect(),
        start,
    })
}

/// Re-renders a parsed property as the ICS line the expander expects.
fn raw_line(p: &Property) -> String {
    let mut s = p.name.clone();
    for (n, v) in &p.params {
        s.push(';');
        s.push_str(n);
        s.push('=');
        s.push_str(v);
    }
    s.push(':');
    s.push_str(&p.value);
    s
}

fn read_todo(c: &Component) -> Option<CalTodo> {
    Some(CalTodo {
        uid: c.prop_value("UID")?.trim().to_string(),
        summary: text(c, "SUMMARY"),
        description: text(c, "DESCRIPTION"),
        due: c.prop("DUE").and_then(parse_time),
        status: c
            .prop_value("STATUS")
            .map(|s| s.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "needs-action".into()),
        completed: c.prop("COMPLETED").and_then(parse_time),
        priority: c.prop_value("PRIORITY").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
    })
}

/// Every VEVENT in a resource — the master first when both master and
/// exceptions are present, which is the order the store conversion wants.
pub fn events_in(root: &Component) -> Vec<CalEvent> {
    let mut out: Vec<CalEvent> = root.components("VEVENT").filter_map(read_event).collect();
    out.sort_by_key(|e| e.recurrence_id.is_some());
    out
}

/// Every VTODO in a resource.
pub fn todos_in(root: &Component) -> Vec<CalTodo> {
    root.components("VTODO").filter_map(read_todo).collect()
}

fn fmt_utc(ts: Timestamp) -> String {
    let z = ts.to_zoned(jiff::tz::TimeZone::UTC);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        z.year(),
        z.month(),
        z.day(),
        z.hour(),
        z.minute(),
        z.second()
    )
}

/// One content line as the resource spells it: the unfolded text a decision
/// reads, and the physical lines it arrived as.
struct Logical {
    text: String,
    physical: Vec<String>,
}

/// `raw` as content lines, RFC 5545 folding undone for reading but kept for
/// writing back. A line that begins with a space or a tab continues the one
/// before it; blank lines are dropped, as `unfold` drops them.
fn logical_lines(raw: &str) -> Vec<Logical> {
    let mut out: Vec<Logical> = Vec::new();
    for raw_line in raw.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if let Some(rest) = line.strip_prefix(' ').or_else(|| line.strip_prefix('\t')) {
            if let Some(last) = out.last_mut() {
                last.text.push_str(rest);
                last.physical.push(line.to_string());
                continue;
            }
        }
        if !line.is_empty() {
            out.push(Logical { text: line.to_string(), physical: vec![line.to_string()] });
        }
    }
    out
}

/// What an edit does to the task's own alarms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AlarmEdit {
    /// A status change touches nothing.
    Leave,
    /// A timed due: the task needs an alarm, and gets one if it has none.
    Ensure,
    /// A due that is a date, or none at all: an alarm with a *relative*
    /// trigger hangs on a `DTSTART` that goes with the hour, so it is dropped
    /// rather than left pointing at nothing (#139's follow-up, found by
    /// @joshhattan). One with an absolute trigger names its own instant and
    /// is the user's; it stays.
    DropRelative,
}

/// Whether a `TRIGGER` line is relative — a duration from the task's start —
/// rather than an instant of its own.
fn trigger_is_relative(text: &str) -> bool {
    let upper = text.to_ascii_uppercase();
    upper.starts_with("TRIGGER") && !upper.contains("VALUE=DATE-TIME")
}

/// Rewrites the one VTODO in `raw` whose UID matches, leaving every other
/// component and every line this does not name exactly as it found them.
///
/// Line surgery rather than a re-serialise, for the reason the whole CalDAV
/// side works this way: a resource carries properties omacal does not model
/// — categories, RELATED-TO, an organiser's X- lines, another client's
/// alarms — and a round trip through our own struct would drop every one of
/// them.
///
/// `rewrite` is handed the VTODO's **own** properties, unfolded, and returns
/// the properties to put back. Two things about that are load-bearing
/// (found 2026-09-17 by an audit, and reproduced):
///
/// - **Unfolded.** Servers fold long lines. Deciding on physical lines
///   dropped a folded DESCRIPTION's first line and kept its continuation,
///   which then joined the property above it — a task's UID read back as
///   `abcd tempor incididunt`. A property kept unchanged goes back as the
///   physical lines it came in as.
/// - **Own.** A nested VALARM has its own DESCRIPTION, and a filter reading
///   every line of the block removed Apple's `DESCRIPTION:Reminder` from the
///   alarm. Nested components pass through whole, after the properties,
///   which is also the order RFC 5545 requires.
fn patch_todo<F>(raw: &str, uid: &str, alarms: AlarmEdit, rewrite: F) -> Option<String>
where
    F: Fn(&[String]) -> Vec<String>,
{
    let lines = logical_lines(raw);
    let mut out: Vec<String> = Vec::new();
    let mut touched = false;
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].text.eq_ignore_ascii_case("BEGIN:VTODO") {
            out.extend(lines[i].physical.iter().cloned());
            i += 1;
            continue;
        }
        // The VTODO's extent, its own properties and its nested components.
        let begin = i;
        let mut depth = 0usize;
        let mut own: Vec<usize> = Vec::new();
        let mut nested: Vec<usize> = Vec::new();
        let mut end = None;
        let mut j = i + 1;
        while j < lines.len() {
            let u = lines[j].text.to_ascii_uppercase();
            if depth == 0 && u == "END:VTODO" {
                end = Some(j);
                break;
            }
            if u.starts_with("BEGIN:") {
                depth += 1;
            }
            if depth > 0 {
                nested.push(j);
            } else {
                own.push(j);
            }
            if u.starts_with("END:") && depth > 0 {
                depth -= 1;
            }
            j += 1;
        }
        let Some(end) = end else {
            // An unterminated VTODO is not something to guess at.
            out.extend(lines[begin..].iter().flat_map(|l| l.physical.iter().cloned()));
            break;
        };
        let block: Vec<&str> = lines[begin..=end].iter().map(|l| l.text.as_str()).collect();
        let this_uid = parse(&format!("BEGIN:VCALENDAR\n{}\nEND:VCALENDAR", block.join("\n")))
            .and_then(|r| r.components("VTODO").next().and_then(|t| t.prop_value("UID").map(|u| u.trim().to_string())));
        if this_uid.as_deref() == Some(uid) {
            touched = true;
            let texts: Vec<String> = own.iter().map(|&k| lines[k].text.clone()).collect();
            // A property handed back unchanged keeps the folding it came with.
            let mut spelled: std::collections::HashMap<&str, std::collections::VecDeque<&[String]>> =
                std::collections::HashMap::new();
            for &k in &own {
                spelled.entry(lines[k].text.as_str()).or_default().push_back(&lines[k].physical);
            }
            out.extend(lines[begin].physical.iter().cloned());
            for line in rewrite(&texts) {
                match spelled.get_mut(line.as_str()).and_then(|q| q.pop_front()) {
                    Some(physical) => out.extend(physical.iter().cloned()),
                    None => out.push(line),
                }
            }
            // Nested components pass through whole, one at a time so an
            // alarm can be weighed on its own (see [`AlarmEdit`]).
            let mut kept_alarm = false;
            let mut depth = 0usize;
            let mut block: Vec<usize> = Vec::new();
            for &k in &nested {
                let upper = lines[k].text.to_ascii_uppercase();
                if upper.starts_with("BEGIN:") {
                    depth += 1;
                }
                block.push(k);
                if upper.starts_with("END:") {
                    depth -= 1;
                }
                if depth > 0 {
                    continue;
                }
                let is_alarm = lines[block[0]].text.eq_ignore_ascii_case("BEGIN:VALARM");
                let relative = is_alarm
                    && block.iter().any(|&j| trigger_is_relative(&lines[j].text));
                if !(alarms == AlarmEdit::DropRelative && relative) {
                    kept_alarm |= is_alarm;
                    out.extend(block.iter().flat_map(|&j| lines[j].physical.iter().cloned()));
                }
                block.clear();
            }
            // Only when there is none: an alarm another client set, with its
            // own offset, is the user's and is never doubled or replaced.
            if alarms == AlarmEdit::Ensure && !kept_alarm {
                out.extend(alarm_at_due_lines());
            }
            out.extend(lines[end].physical.iter().cloned());
        } else {
            out.extend(lines[begin..=end].iter().flat_map(|l| l.physical.iter().cloned()));
        }
        i = end + 1;
    }
    // A resource that never held this UID is not something to write back.
    (touched && !out.is_empty()).then(|| out.join("\r\n"))
}

/// What an edit sets on a task. Every field is the whole answer rather than
/// a change to apply: the editor always knows the complete state, and a
/// "leave this alone" third state would be one more thing for a caller to
/// get wrong. `None` on an optional field means the property goes.
/// The priority levels OmaCal authors, with their RFC 5545 §3.8.1.9 wire
/// values. "No priority" is `None` at the call site, never a variant, so it
/// cannot be mistaken for the lowest level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskPriority {
    High = 1,
    Medium = 5,
    Low = 9,
}

impl TaskPriority {
    pub fn wire(self) -> i64 {
        self as i64
    }

    /// The level a stored integer reads as, by the RFC's bands: 1–4 high,
    /// 5 medium, 6–9 low. Anything else — `0`, or out of range — is no level.
    pub fn from_wire(n: i64) -> Option<TaskPriority> {
        match n {
            1..=4 => Some(TaskPriority::High),
            5 => Some(TaskPriority::Medium),
            6..=9 => Some(TaskPriority::Low),
            _ => None,
        }
    }
}

/// The `PRIORITY` value to write for an integer the caller named: `1..=9`
/// as-is, and everything else — `0`, or out of range — means the property is
/// omitted. The **one** place an integer becomes a writable priority, so the
/// create and edit paths cannot drift apart on what "no priority" is. It
/// keeps a valid non-canonical value (a server's `7`) exactly as given; only
/// the *authored* values come from [`TaskPriority`], at 1/5/9.
pub fn priority_to_write(raw: i64) -> Option<i64> {
    (1..=9).contains(&raw).then_some(raw)
}

#[derive(Debug, Clone, PartialEq)]
pub struct TodoEdit<'a> {
    pub summary: &'a str,
    pub due: Option<TodoDue>,
    pub description: Option<&'a str>,
    /// `None` clears the `PRIORITY` property; `Some(1..=9)` writes it. Held as
    /// the raw wire integer rather than a [`TaskPriority`] so a server's
    /// non-canonical value — a `7` — passes through an unrelated edit
    /// untouched; only a deliberate choice writes 1/5/9.
    pub priority: Option<i64>,
}

/// When a task is due: a bare date, or an instant.
///
/// The distinction is the user's, not a detail — a task due "Thursday" and
/// one due "Thursday at 18:00" are different promises, and iCalendar spells
/// them differently (`VALUE=DATE` against a UTC stamp).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoDue {
    Date(Date),
    At(Timestamp),
}

/// A `DTSTART`/`DUE`-shaped wall-time line: `TZID` in the calendar's own
/// zone, or a bare UTC stamp when jiff cannot resolve it. Shared so a
/// mirrored `DTSTART` is spelled exactly the way its `DUE` is.
fn zoned_line(name: &str, t: Timestamp, cal_tz: &str) -> String {
    match jiff::tz::TimeZone::get(cal_tz) {
        Ok(tz) => {
            let z = t.to_zoned(tz);
            format!(
                "{name};TZID={cal_tz}:{:04}{:02}{:02}T{:02}{:02}{:02}",
                z.year(), z.month(), z.day(), z.hour(), z.minute(), z.second()
            )
        }
        Err(_) => format!("{name}:{}", fmt_utc(t)),
    }
}

impl TodoDue {
    /// The `DUE` line, with a timed due rendered as a wall time in the
    /// calendar's own zone.
    ///
    /// **Not a bare UTC stamp** (issue #102). Every other client writes a
    /// task's due time the way it was authored — this crate's own event
    /// writer says so where [`WriteTime`] is defined, "authored events keep
    /// their author's zone, like every client" — and the task writer was the
    /// one place rendering an instant straight to `Z`. A zone that jiff
    /// cannot resolve falls back to UTC's spelling rather than dropping the
    /// property: a due an hour off beats a due that vanished, the same trade
    /// [`resolve`] makes on the way in.
    fn line(&self, cal_tz: &str) -> String {
        match self {
            TodoDue::Date(d) => {
                format!("DUE;VALUE=DATE:{:04}{:02}{:02}", d.year(), d.month(), d.day())
            }
            TodoDue::At(t) => zoned_line("DUE", *t, cal_tz),
        }
    }

    /// A `DTSTART` at the same instant as this due — `None` for a bare date,
    /// since OmaCal still has no notion of a task's start to fabricate one
    /// for.
    ///
    /// **Confirmed against a real iCloud account** (issue #102, the sequel):
    /// a `DUE` sent alone — or beside a `DTSTART` that no longer matches it —
    /// round-trips through iCloud's Reminders backend as a date-only due,
    /// the time silently dropped, whether or not the property carries a
    /// `TZID`. A task iOS itself created always pairs a timed due with a
    /// `DTSTART` at the identical instant, and mirroring that pairing is what
    /// this method is for. Discarding another client's own `DTSTART` this
    /// way is still the cost of last resort — see [`TodoDue::admits_start`]
    /// for the narrower case, a bare-date due, where an existing start is
    /// kept rather than replaced.
    fn mirrored_start(&self, cal_tz: &str) -> Option<String> {
        match self {
            TodoDue::Date(_) => None,
            TodoDue::At(t) => Some(zoned_line("DTSTART", *t, cal_tz)),
        }
    }

    /// Whether an *existing* `DTSTART` may stay beside a bare-date due.
    ///
    /// RFC 5545 §3.6.2 asks two things of the pair: the same value type, and
    /// a `DTSTART` strictly earlier than the `DUE`. A resource that breaks
    /// either is one a strict server is entitled to reject or rewrite.
    ///
    /// Only reached for [`TodoDue::Date`] — a timed due always replaces
    /// `DTSTART` outright via [`TodoDue::mirrored_start`], never merely
    /// keeps or drops what was there.
    fn admits_start(&self, start: &IcsTime, cal_tz: &str) -> bool {
        match (self, start) {
            (TodoDue::Date(due), IcsTime::Date(from)) => from < due,
            // A bare date beside a timestamp, either way round.
            (TodoDue::Date(_), _) | (_, IcsTime::Date(_)) => false,
            // Both are DATE-TIME; the ordering is the remaining question, and
            // `resolve` is what already turns each spelling of one into an
            // instant.
            (TodoDue::At(due), other) => resolve(other, cal_tz)
                .and_then(|(ms, _, _)| Timestamp::from_millisecond(ms).ok())
                .is_some_and(|from| from < *due),
        }
    }
}

/// Applies an edit to the task with this UID.
///
/// SEQUENCE goes up and LAST-MODIFIED/DTSTAMP are restamped, which is what
/// tells every other client that this version supersedes the one they hold.
/// A task with no SEQUENCE is treated as 0, per the RFC.
///
/// `cal_tz` is the calendar's zone: it is the zone a timed due (and its
/// mirrored `DTSTART`) goes out in, and the one an existing `DTSTART` is read
/// back in to decide whether it may stay beside a bare-date due. See
/// [`TodoDue::line`], [`TodoDue::mirrored_start`] and
/// [`TodoDue::admits_start`] — a task list created from Apple Reminders
/// carries a `DTSTART`, and a timed `DUE` needs one at the same instant or
/// iCloud drops the time, the invalid-pair shape issue #102 first described.
pub fn patch_todo_fields(raw: &str, uid: &str, edit: &TodoEdit, cal_tz: &str, now: Timestamp) -> Option<String> {
    // A timed due needs an alarm; a date or no due at all leaves nothing for
    // a relative one to hang on.
    let alarms = if matches!(edit.due, Some(TodoDue::At(_))) {
        AlarmEdit::Ensure
    } else {
        AlarmEdit::DropRelative
    };
    patch_todo(raw, uid, alarms, |inner| {
        let sequence = inner
            .iter()
            .find(|l| l.to_ascii_uppercase().starts_with("SEQUENCE:"))
            .and_then(|l| l.split_once(':'))
            .and_then(|(_, v)| v.trim().parse::<i64>().ok())
            .unwrap_or(0);
        // A timed due always gets its own mirrored `DTSTART`, replacing
        // whatever was there; a bare-date due keeps an existing `DTSTART`
        // only while it still agrees with the new `DUE` — the RFC wants the
        // same value type and an earlier instant, and neither is ours to
        // fake.
        let mirrored_start = edit.due.and_then(|due| due.mirrored_start(cal_tz));
        let drop_start = mirrored_start.is_some()
            || edit.due.is_some_and(|due| {
                inner
                    .iter()
                    .filter(|l| {
                        let u = l.to_ascii_uppercase();
                        u.starts_with("DTSTART:") || u.starts_with("DTSTART;")
                    })
                    .filter_map(|l| parse_line(l))
                    .any(|p| parse_time(&p).is_none_or(|t| !due.admits_start(&t, cal_tz)))
            });

        let mut kept: Vec<String> = inner
            .iter()
            .filter(|l| {
                let u = l.to_ascii_uppercase();
                // Unfolded, and the VTODO's own lines only: `patch_todo`
                // hands over no continuation lines and nothing of a VALARM.
                !(u.starts_with("SUMMARY:")
                    || u.starts_with("SUMMARY;")
                    || u.starts_with("DUE:")
                    || u.starts_with("DUE;")
                    || u.starts_with("DESCRIPTION:")
                    || u.starts_with("DESCRIPTION;")
                    || u.starts_with("PRIORITY:")
                    || u.starts_with("PRIORITY;")
                    || u.starts_with("SEQUENCE:")
                    || u.starts_with("LAST-MODIFIED:")
                    || u.starts_with("DTSTAMP:")
                    || (drop_start && (u.starts_with("DTSTART:") || u.starts_with("DTSTART;"))))
            })
            .cloned()
            .collect();
        kept.push(format!("SUMMARY:{}", escape(edit.summary)));
        if let Some(start) = &mirrored_start {
            kept.push(start.clone());
        }
        if let Some(due) = edit.due {
            kept.push(due.line(cal_tz));
        }
        if let Some(text) = edit.description {
            kept.push(format!("DESCRIPTION:{}", escape(text)));
        }
        if let Some(priority) = edit.priority {
            kept.push(format!("PRIORITY:{priority}"));
        }
        kept.push(format!("SEQUENCE:{}", sequence + 1));
        kept.push(format!("DTSTAMP:{}", fmt_utc(now)));
        kept.push(format!("LAST-MODIFIED:{}", fmt_utc(now)));
        kept
    })
}

/// Completes or reopens the task with this UID, touching nothing else in the
/// resource — [`patch_todo`]'s surgery, on the status properties alone.
pub fn patch_todo_status(raw: &str, uid: &str, completed: bool, now: Timestamp) -> Option<String> {
    patch_todo(raw, uid, AlarmEdit::Leave, |own| {
        let sequence = own
            .iter()
            .find(|l| l.to_ascii_uppercase().starts_with("SEQUENCE:"))
            .and_then(|l| l.split_once(':'))
            .and_then(|(_, v)| v.trim().parse::<i64>().ok())
            .unwrap_or(0);
        let mut kept: Vec<String> = own
            .iter()
            .filter(|l| {
                let u = l.to_ascii_uppercase();
                !(u.starts_with("STATUS:")
                    || u.starts_with("STATUS;")
                    || u.starts_with("COMPLETED:")
                    || u.starts_with("COMPLETED;")
                    || u.starts_with("PERCENT-COMPLETE:")
                    || u.starts_with("PERCENT-COMPLETE;")
                    || u.starts_with("SEQUENCE:")
                    || u.starts_with("DTSTAMP:")
                    || u.starts_with("LAST-MODIFIED:"))
            })
            .cloned()
            .collect();
        // iCloudBridge compares modification times, so a status-only change
        // needs the same revision stamps as a title/date edit.
        kept.push(format!("SEQUENCE:{}", sequence + 1));
        kept.push(format!("DTSTAMP:{}", fmt_utc(now)));
        kept.push(format!("LAST-MODIFIED:{}", fmt_utc(now)));
        if completed {
            kept.push("STATUS:COMPLETED".to_string());
            kept.push(format!("COMPLETED:{}", fmt_utc(now)));
            kept.push("PERCENT-COMPLETE:100".to_string());
        } else {
            kept.push("STATUS:NEEDS-ACTION".to_string());
        }
        kept
    })
}

/// The alarm a timed task carries: at the due instant, which is what a
/// reminder made in Reminders itself carries too.
fn alarm_at_due_lines() -> [String; 5] {
    [
        "BEGIN:VALARM".to_string(),
        "ACTION:DISPLAY".to_string(),
        "DESCRIPTION:Reminder".to_string(),
        "TRIGGER:PT0M".to_string(),
        "END:VALARM".to_string(),
    ]
}

/// How long before its due a task's own alarm asks to speak, in minutes
/// (#137), or `None` for a task that carries none.
///
/// A reminder made on the phone with "15 minutes before" carries `-PT15M`; one
/// OmaCal writes carries `PT0M`, which is zero minutes before — the due
/// itself. Honouring the task's own alarm is what keeps the desktop and the
/// phone saying the same thing at the same moment rather than twice, minutes
/// apart.
///
/// Ignored: an absolute `TRIGGER` (`VALUE=DATE-TIME`), which names an instant
/// rather than a lead, and one hung on the task's end (`RELATED=END`), which
/// has nothing to hang on — a VTODO OmaCal writes has no `DURATION`. Both fall
/// back to the due itself. Where a task carries several alarms the earliest
/// wins: it is the one that speaks first.
pub fn todo_alarm_lead_minutes(raw: &str, uid: &str) -> Option<i64> {
    let cal = parse(raw)?;
    let todo = cal
        .components("VTODO")
        .find(|t| t.prop_value("UID").map(str::trim) == Some(uid))?;
    todo.components("VALARM")
        .filter_map(|alarm| {
            let trigger = alarm.prop("TRIGGER")?;
            let param = |name: &str, want: &str| {
                trigger
                    .params
                    .iter()
                    .any(|(k, v)| k.eq_ignore_ascii_case(name) && v.eq_ignore_ascii_case(want))
            };
            if param("VALUE", "DATE-TIME") || param("RELATED", "END") {
                return None;
            }
            // `-PT15M` is a quarter of an hour *before*, so the lead is the
            // negated offset.
            Some(-parse_duration_ms(&trigger.value)? / 60_000)
        })
        .max()
}

/// A brand-new single-VTODO resource.
///
/// No `DTSTART` for a bare-date due: OmaCal has no notion of when a task
/// *starts*, and the RFC only constrains the property when it is there. A
/// *timed* due gets one anyway, at the identical instant — see
/// [`TodoDue::mirrored_start`] for why: a `DUE` iCloud is handed without a
/// matching `DTSTART` round-trips with its time silently dropped, and a
/// brand-new task is exactly as exposed to that as an edited one.
pub fn new_todo_ics(
    uid: &str,
    summary: &str,
    due: Option<&IcsTime>,
    priority: Option<i64>,
    now: Timestamp,
) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//omacal//EN".to_string(),
        "BEGIN:VTODO".to_string(),
        format!("UID:{uid}"),
        format!("DTSTAMP:{}", fmt_utc(now)),
        format!("SUMMARY:{}", escape(summary)),
        "STATUS:NEEDS-ACTION".to_string(),
    ];
    // Omitted when there is no priority, never `PRIORITY:0`: the two are
    // equivalent to the RFC, but `0` churns against the clients that omit it.
    if let Some(priority) = priority {
        lines.push(format!("PRIORITY:{priority}"));
    }
    if let Some(due) = due {
        match due {
            IcsTime::Date(d) => lines.push(format!(
                "DUE;VALUE=DATE:{:04}{:02}{:02}",
                d.year(),
                d.month(),
                d.day()
            )),
            IcsTime::Utc(ts) => {
                lines.push(format!("DTSTART:{}", fmt_utc(*ts)));
                lines.push(format!("DUE:{}", fmt_utc(*ts)));
            }
            IcsTime::Zoned { dt, tzid } => {
                lines.push(format!(
                    "DTSTART;TZID={tzid}:{:04}{:02}{:02}T{:02}{:02}{:02}",
                    dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), dt.second()
                ));
                lines.push(format!(
                    "DUE;TZID={tzid}:{:04}{:02}{:02}T{:02}{:02}{:02}",
                    dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), dt.second()
                ));
            }
            IcsTime::Floating(dt) => {
                lines.push(format!(
                    "DTSTART:{:04}{:02}{:02}T{:02}{:02}{:02}",
                    dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), dt.second()
                ));
                lines.push(format!(
                    "DUE:{:04}{:02}{:02}T{:02}{:02}{:02}",
                    dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), dt.second()
                ));
            }
        }
        // A bare date has no time to alarm at. A timed due needs one: unlike
        // an event, a task has no reminders UI yet, and Reminders itself
        // stamps a VALARM on every timed reminder it creates — without this,
        // an omacal-created task carries nothing for the phone (or Omarchy)
        // to fire on.
        if !matches!(due, IcsTime::Date(_)) {
            lines.extend(alarm_at_due_lines());
        }
    }
    lines.push("END:VTODO".to_string());
    lines.push("END:VCALENDAR".to_string());
    lines.join("\r\n")
}

// --- event write-side -------------------------------------------------------

/// An endpoint the serializer can render: a bare date (all-day) or a wall
/// time in a named zone. UTC instants are rendered through a zone by the
/// caller — authored events keep their author's zone, like every client.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteTime {
    Date(Date),
    Zoned { dt: DateTime, tzid: String },
}

impl WriteTime {
    fn prop(&self, name: &str) -> String {
        match self {
            WriteTime::Date(d) => {
                format!("{name};VALUE=DATE:{:04}{:02}{:02}", d.year(), d.month(), d.day())
            }
            WriteTime::Zoned { dt, tzid } => format!(
                "{name};TZID={tzid}:{:04}{:02}{:02}T{:02}{:02}{:02}",
                dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), dt.second()
            ),
        }
    }
}

/// Everything a written VEVENT carries. One shape serves creates, full
/// rewrites, and exception components — presence of `recurrence_id` is what
/// makes it an exception.
/// One person to write onto an event, as the *editor* can author them.
///
/// Two things the user types and one they tick, which is all an attendee line
/// this app composes ever says. `PARTSTAT` is absent on purpose: an answer
/// belongs to the person who gave it, and nothing here may overwrite one
/// (guest-list spec §2, in the vocabulary of iCalendar).
#[derive(Debug, Clone, PartialEq)]
pub struct AttendeeWrite {
    /// A calendar user address. **Not necessarily an email** (RFC 5545 §3.3.3
    /// is a URI): `urn:uuid:…` is as legal as `mailto:…`, and a bare mailbox
    /// is written as `mailto:` by [`cal_address`] rather than guessed at.
    pub address: String,
    /// `CN`, kept apart from the address it names.
    pub display_name: Option<String>,
    /// `ROLE=OPT-PARTICIPANT`.
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventWrite {
    pub uid: String,
    pub summary: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub start: WriteTime,
    pub end: WriteTime,
    /// Recurrence lines verbatim (`RRULE:...`); empty for a one-off.
    pub recurrence: Vec<String>,
    pub recurrence_id: Option<WriteTime>,
    /// `(method, minutes)` — `popup` renders DISPLAY, `email` EMAIL.
    pub alarms: Vec<(String, i64)>,
    pub sequence: i64,
    /// **The attendee list this edit owns**, or `None` for "the attendees were
    /// not touched" — [`crate::EventFields::guests`]' own three-state, and
    /// load bearing for the same reason. `None` leaves every `ATTENDEE` line
    /// on the resource exactly as the server has it, which is what every
    /// write did before attendees were editable and what a drag still does.
    /// `Some(vec![])` removes everyone.
    ///
    /// A `Some` is reconciled against the lines already there rather than
    /// replacing them: an attendee who stays keeps their own line, and with it
    /// their `PARTSTAT` and anything else the server put on it. Retyping a
    /// title must not reset everybody's reply to needs-action.
    pub attendees: Option<Vec<AttendeeWrite>>,
    /// A video-call link, written as RFC 7986 `CONFERENCE`.
    ///
    /// `None` on every CalDAV write, and deliberately: this app does not
    /// *create* conferences over CalDAV, and a property invented on a write
    /// would be a claim the server never agreed to. It exists for the export
    /// (#113), where dropping the join link would make the exported meeting
    /// unjoinable — the one thing an attendee opens it for. Zoom and Teams
    /// links usually ride in `location` and survive either way; Meet's does
    /// not, which is what makes this the difference between a whole event
    /// and most of one.
    pub conference: Option<String>,
}

/// The CAL-ADDRESS to write for an authored address.
///
/// A value that already names a scheme is written as it stands, which is the
/// whole of issue #114's second half: `urn:uuid:12345678` is a real attendee
/// and must survive a round trip rather than be "fixed" into a mailbox. A bare
/// `ada@example.com` gets `mailto:`, because that is what it means.
///
/// Control characters are dropped rather than escaped: a newline in a property
/// value is a second line, and a second line the user wrote is an injected
/// property. URI values take no `\` escaping (RFC 5545 §3.3.13), so there is
/// nothing else to encode.
fn cal_address(address: &str) -> String {
    let clean: String = address.trim().chars().filter(|c| !c.is_control()).collect();
    let scheme = clean
        .split_once(':')
        .map(|(s, _)| !s.is_empty()
            && s.starts_with(|c: char| c.is_ascii_alphabetic())
            && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
        .unwrap_or(false);
    if scheme { clean } else { format!("mailto:{clean}") }
}

/// The address an attendee is *matched* by: the CAL-ADDRESS with a `mailto:`
/// scheme taken off, which is the form [`read_attendee`] stores and
/// [`is_attendee_of`] compares.
fn match_key(address: &str) -> String {
    let v = cal_address(address);
    match v.get(..7) {
        Some(scheme) if scheme.eq_ignore_ascii_case("mailto:") => v[7..].to_string(),
        _ => v,
    }
}

/// A parameter value, quoted when it has to be.
///
/// RFC 5545 §3.1: a param value carrying `;`, `:` or `,` must be a quoted
/// string, and a quoted string cannot itself contain `"`. Quotes are dropped
/// rather than escaped, because there is no escape for them to use.
fn param_value(v: &str) -> String {
    let clean: String = v.chars().filter(|c| !c.is_control() && *c != '"').collect();
    if clean.contains([';', ':', ',']) { format!("\"{clean}\"") } else { clean }
}

/// A fresh `ATTENDEE` line for somebody who is not on the resource yet.
///
/// `PARTSTAT=NEEDS-ACTION` is written rather than left implied: it is the
/// default either way, and a reader that shows a list is friendlier when every
/// row carries the same property. No `RSVP=TRUE` — that asks a server to chase
/// an answer, which is scheduling, which this does not do.
fn attendee_line(a: &AttendeeWrite) -> String {
    let mut line = "ATTENDEE".to_string();
    if let Some(cn) = a.display_name.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        line.push_str(&format!(";CN={}", param_value(cn)));
    }
    if a.optional {
        line.push_str(";ROLE=OPT-PARTICIPANT");
    }
    line.push_str(";PARTSTAT=NEEDS-ACTION:");
    line.push_str(&cal_address(&a.address));
    line
}

/// The list `want` describes, expressed over the lines already on the block.
///
/// Somebody who stays keeps their own line byte for byte, save for the two
/// parameters the editor owns (`CN`, and `ROLE` when it is the optional flag
/// being turned off). Everything else the server put there — `PARTSTAT`,
/// `RSVP`, `CUTYPE`, `SENT-BY`, `DIR`, `X-`… — rides along, which is the only
/// way an unrelated edit can be non-destructive. Somebody dropped loses their
/// line; somebody new gets [`attendee_line`]. Order follows `want`, so the
/// list reads the way the form showed it.
fn merge_attendee_lines(existing: &[String], want: &[AttendeeWrite]) -> Vec<String> {
    want
        .iter()
        .filter(|a| !match_key(&a.address).is_empty())
        .map(|a| {
            let key = match_key(&a.address);
            let Some(line) = existing.iter().find(|l| is_attendee_of(l, &key)) else {
                return attendee_line(a);
            };
            let named = match a.display_name.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                Some(cn) => with_param(line, "CN", Some(&param_value(cn))),
                None => with_param(line, "CN", None),
            };
            if a.optional {
                with_param(&named, "ROLE", Some("OPT-PARTICIPANT"))
            } else if parse_line(&named)
                .and_then(|p| p.param("ROLE").map(|r| r.eq_ignore_ascii_case("OPT-PARTICIPANT")))
                .unwrap_or(false)
            {
                // Only the flag this app sets is cleared. A `ROLE=CHAIR` the
                // server wrote is not ours to take off.
                with_param(&named, "ROLE", None)
            } else {
                named
            }
        })
        .collect()
}

/// Every `ATTENDEE` line of one block, in the order it has them.
fn attendee_lines_of(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| l.to_ascii_uppercase().starts_with("ATTENDEE"))
        .cloned()
        .collect()
}

fn vevent_lines(ev: &EventWrite, now: Timestamp) -> Vec<String> {
    let mut lines = vec![
        "BEGIN:VEVENT".to_string(),
        format!("UID:{}", ev.uid),
        format!("DTSTAMP:{}", fmt_utc(now)),
    ];
    if let Some(rid) = &ev.recurrence_id {
        lines.push(rid.prop("RECURRENCE-ID"));
    }
    lines.push(ev.start.prop("DTSTART"));
    lines.push(ev.end.prop("DTEND"));
    if let Some(s) = &ev.summary {
        lines.push(format!("SUMMARY:{}", escape(s)));
    }
    if let Some(l) = &ev.location {
        lines.push(format!("LOCATION:{}", escape(l)));
    }
    if let Some(d) = &ev.description {
        lines.push(format!("DESCRIPTION:{}", escape(d)));
    }
    if let Some(uri) = &ev.conference {
        // RFC 7986 §5.11, the property every modern client reads for this —
        // rather than a line appended to the description, which would put a
        // link in prose and call it data.
        lines.push(format!("CONFERENCE;VALUE=URI;FEATURE=VIDEO:{}", escape(uri)));
    }
    for r in &ev.recurrence {
        lines.push(r.clone());
    }
    if ev.sequence > 0 {
        lines.push(format!("SEQUENCE:{}", ev.sequence));
    }
    for (method, minutes) in &ev.alarms {
        lines.push("BEGIN:VALARM".to_string());
        lines.push(format!(
            "ACTION:{}",
            if method == "email" { "EMAIL" } else { "DISPLAY" }
        ));
        lines.push("DESCRIPTION:Reminder".to_string());
        lines.push(format!("TRIGGER:-PT{minutes}M"));
        lines.push("END:VALARM".to_string());
    }
    if let Some(want) = &ev.attendees {
        // Nothing to reconcile against on a render: every line is a new one.
        // The rewriting paths merge with what the resource already has.
        lines.extend(merge_attendee_lines(&[], want));
    }
    lines.push("END:VEVENT".to_string());
    lines
}

/// A brand-new single-event resource.
pub fn new_event_ics(ev: &EventWrite, now: Timestamp) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//omacal//EN".to_string(),
    ];
    lines.extend(vevent_lines(ev, now));
    lines.push("END:VCALENDAR".to_string());
    lines.join("\r\n")
}

/// The physical lines of one VEVENT block, with its position, plus whether it
/// is an exception (carries RECURRENCE-ID) — the unit every resource edit
/// below works in.
struct EventBlock {
    start: usize,
    end: usize, // inclusive, the END:VEVENT line
    is_exception: bool,
    uid: Option<String>,
}

fn split_lines(raw: &str) -> Vec<String> {
    raw.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect()
}

fn event_blocks(lines: &[String]) -> Vec<EventBlock> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        if line.eq_ignore_ascii_case("BEGIN:VEVENT") {
            start = Some(i);
        } else if line.eq_ignore_ascii_case("END:VEVENT") {
            if let Some(s) = start.take() {
                let body = lines[s..=i].join("\n");
                let parsed = parse(&format!("BEGIN:VCALENDAR\n{body}\nEND:VCALENDAR"));
                let (uid, is_exception) = parsed
                    .as_ref()
                    .and_then(|r| r.components("VEVENT").next())
                    .map(|c| {
                        (
                            c.prop_value("UID").map(|u| u.trim().to_string()),
                            c.prop("RECURRENCE-ID").is_some(),
                        )
                    })
                    .unwrap_or((None, false));
                out.push(EventBlock { start: s, end: i, is_exception, uid });
            }
        }
    }
    out
}

/// Rewrites the *master* VEVENT of `uid` inside `raw` with `ev`'s fields,
/// preserving every property this module does not model. When
/// `drop_exceptions` is set (a series edit that moved its times), exception
/// blocks of that uid are removed too — their RECURRENCE-IDs name occurrence
/// starts that no longer exist.
pub fn rewrite_master(
    raw: &str,
    uid: &str,
    ev: &EventWrite,
    now: Timestamp,
    drop_exceptions: bool,
) -> Option<String> {
    let lines = split_lines(raw);
    let blocks = event_blocks(&lines);
    let master = blocks
        .iter()
        .find(|b| !b.is_exception && b.uid.as_deref() == Some(uid))?;

    // The master block, with the properties the edit owns removed and the
    // fresh ones inserted. Unknown lines (X-…, ORGANIZER, ATTENDEE, CLASS…)
    // pass through untouched.
    let owned = |l: &str| {
        let u = l.to_ascii_uppercase();
        u.starts_with("SUMMARY") || u.starts_with("LOCATION") || u.starts_with("DESCRIPTION:")
            || u.starts_with("DESCRIPTION;") || u.starts_with("DTSTART") || u.starts_with("DTEND")
            || u.starts_with("DURATION") || u.starts_with("RRULE") || u.starts_with("RDATE")
            || u.starts_with("SEQUENCE") || u.starts_with("DTSTAMP")
            // Only when this edit owns the list. An edit that does not touch
            // attendees leaves every line where it is, exactly as before.
            || (ev.attendees.is_some() && u.starts_with("ATTENDEE"))
    };

    // Reconciled against the block's own lines, so an attendee who stays keeps
    // their answer. Rendered here rather than by `vevent_lines`, which has
    // nothing to reconcile against.
    let merged = ev
        .attendees
        .as_ref()
        .map(|want| merge_attendee_lines(&attendee_lines_of(&lines[master.start..=master.end]), want));
    // A time change invalidates EXDATEs too (they name occurrence starts).
    let drop_exdates = drop_exceptions;

    let mut patched: Vec<String> = Vec::new();
    let mut in_alarm = false;
    for l in &lines[master.start..=master.end] {
        let u = l.to_ascii_uppercase();
        if u == "BEGIN:VALARM" {
            in_alarm = true;
            continue;
        }
        if u == "END:VALARM" {
            in_alarm = false;
            continue;
        }
        if in_alarm || owned(l) || (drop_exdates && u.starts_with("EXDATE")) {
            continue;
        }
        if u == "END:VEVENT" {
            // Insert the owned properties just before the close. Attendees are
            // the merged lines below, not the render's fresh ones.
            let fresh = vevent_lines(&EventWrite { attendees: None, ..ev.clone() }, now);
            // Skip BEGIN:VEVENT/UID from the fresh render — the block keeps
            // its own — and take everything else.
            for f in &fresh[2..fresh.len() - 1] {
                patched.push(f.clone());
            }
            if let Some(m) = &merged {
                patched.extend(m.iter().cloned());
            }
        }
        patched.push(l.clone());
    }

    let mut out: Vec<String> = Vec::new();
    let mut skip: Vec<(usize, usize)> = vec![(master.start, master.end)];
    if drop_exceptions {
        for b in blocks.iter().filter(|b| b.is_exception && b.uid.as_deref() == Some(uid)) {
            skip.push((b.start, b.end));
        }
    }
    for (i, l) in lines.iter().enumerate() {
        if skip.iter().any(|(s, e)| i >= *s && i <= *e) {
            if i == master.start {
                out.extend(patched.iter().cloned());
            }
            continue;
        }
        out.push(l.clone());
    }
    Some(out.join("\r\n"))
}

/// Inserts (or replaces) the exception component for one occurrence — scope
/// "this" on a series. An existing exception with the same RECURRENCE-ID
/// value is replaced; the master and its siblings stay byte-identical.
pub fn upsert_exception(raw: &str, ev: &EventWrite, now: Timestamp) -> Option<String> {
    let rid = ev.recurrence_id.as_ref()?;
    let rid_value = rid.prop("RECURRENCE-ID");
    let lines = split_lines(raw);
    let blocks = event_blocks(&lines);
    // An existing exception for the same occurrence: match on the rendered
    // RECURRENCE-ID value's date-time part, ignoring parameter order.
    let same_occurrence = |b: &EventBlock| {
        b.is_exception
            && b.uid.as_deref() == Some(ev.uid.as_str())
            && lines[b.start..=b.end].iter().any(|l| {
                l.to_ascii_uppercase().starts_with("RECURRENCE-ID")
                    && l.rsplit(':').next() == rid_value.rsplit(':').next()
            })
    };
    let replaced: Option<(usize, usize)> =
        blocks.iter().find(|b| same_occurrence(b)).map(|b| (b.start, b.end));

    // The exception block is rewritten wholesale, so its `ATTENDEE` lines have
    // to be carried across by hand or editing one occurrence would empty its
    // guest list. `None` keeps them verbatim, which is the promise every other
    // path already makes; `Some` reconciles against them.
    let existing = replaced
        .map(|(s, e)| attendee_lines_of(&lines[s..=e]))
        .unwrap_or_default();
    let merged = match &ev.attendees {
        Some(want) => merge_attendee_lines(&existing, want),
        None => existing,
    };

    let close = lines
        .iter()
        .rposition(|l| l.eq_ignore_ascii_case("END:VCALENDAR"))?;

    let mut out: Vec<String> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if let Some((s, e)) = replaced {
            if i >= s && i <= e {
                continue;
            }
        }
        if i == close {
            let fresh = vevent_lines(&EventWrite { attendees: None, ..ev.clone() }, now);
            // Before END:VEVENT, where every other property of the block sits.
            out.extend(fresh[..fresh.len() - 1].iter().cloned());
            out.extend(merged.iter().cloned());
            out.push("END:VEVENT".to_string());
        }
        out.push(l.clone());
    }
    Some(out.join("\r\n"))
}

/// Removes one occurrence from a series: an EXDATE on the master (matching
/// DTSTART's form) plus the removal of any exception component that named
/// that occurrence.
pub fn exclude_occurrence(raw: &str, uid: &str, occurrence: &WriteTime) -> Option<String> {
    let lines = split_lines(raw);
    let blocks = event_blocks(&lines);
    let master = blocks
        .iter()
        .find(|b| !b.is_exception && b.uid.as_deref() == Some(uid))?;
    let exdate = occurrence.prop("EXDATE");
    let rid_tail = occurrence.prop("RECURRENCE-ID");
    let doomed: Vec<(usize, usize)> = blocks
        .iter()
        .filter(|b| {
            b.is_exception
                && b.uid.as_deref() == Some(uid)
                && lines[b.start..=b.end].iter().any(|l| {
                    l.to_ascii_uppercase().starts_with("RECURRENCE-ID")
                        && l.rsplit(':').next() == rid_tail.rsplit(':').next()
                })
        })
        .map(|b| (b.start, b.end))
        .collect();

    let mut out: Vec<String> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if doomed.iter().any(|(s, e)| i >= *s && i <= *e) {
            continue;
        }
        if i == master.end {
            out.push(exdate.clone()); // just before the master's END:VEVENT
        }
        out.push(l.clone());
    }
    Some(out.join("\r\n"))
}

/// Ends a series before the `cut` occurrence — scope "following": the RRULE
/// gets `UNTIL` (replacing any COUNT), and exception components at or past
/// the cut die with the occurrences they overrode. `until_render` must match
/// DTSTART's value type — a bare date for all-day series, UTC basic time
/// otherwise — and `cut` must be rendered in the master's own zone, which is
/// what makes the exception comparison below chronological: two basic
/// date-times in the same zone order lexicographically.
pub fn truncate_series(raw: &str, uid: &str, until_render: &str, cut: &WriteTime) -> Option<String> {
    let lines = split_lines(raw);
    let blocks = event_blocks(&lines);
    let master = blocks
        .iter()
        .find(|b| !b.is_exception && b.uid.as_deref() == Some(uid))?;
    let rrule_idx = (master.start..=master.end)
        .find(|&i| lines[i].to_ascii_uppercase().starts_with("RRULE"))?;

    let rule = lines[rrule_idx].clone();
    // Drop COUNT and any existing UNTIL, then append ours.
    let (head, params) = rule.split_at(rule.find(':')? + 1);
    let kept: Vec<&str> = params
        .split(';')
        .filter(|p| {
            let u = p.trim().to_ascii_uppercase();
            !u.starts_with("COUNT=") && !u.starts_with("UNTIL=")
        })
        .collect();
    let rule = format!("{head}{};UNTIL={until_render}", kept.join(";"));

    let cut_tail = cut.prop("RECURRENCE-ID");
    let cut_tail = cut_tail.rsplit(':').next().unwrap_or("").to_string();
    let doomed: Vec<(usize, usize)> = blocks
        .iter()
        .filter(|b| {
            b.is_exception
                && b.uid.as_deref() == Some(uid)
                && lines[b.start..=b.end].iter().any(|l| {
                    l.to_ascii_uppercase().starts_with("RECURRENCE-ID")
                        && l.rsplit(':').next().unwrap_or("") >= cut_tail.as_str()
                })
        })
        .map(|b| (b.start, b.end))
        .collect();

    let mut out: Vec<String> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if doomed.iter().any(|(s, e)| i >= *s && i <= *e) {
            continue;
        }
        if i == rrule_idx {
            out.push(rule.clone());
        } else {
            out.push(l.clone());
        }
    }
    Some(out.join("\r\n"))
}

/// Whether an unfolded content line is this attendee's own `ATTENDEE`
/// property. The mailbox is compared case-insensitively and so is the
/// `mailto:` scheme — servers spell both however they feel like.
fn is_attendee_of(line: &str, email: &str) -> bool {
    let Some(p) = parse_line(line) else { return false };
    if p.name != "ATTENDEE" {
        return false;
    }
    let v = p.value.trim();
    let v = match v.get(..7) {
        Some(scheme) if scheme.eq_ignore_ascii_case("mailto:") => &v[7..],
        _ => v,
    };
    v.eq_ignore_ascii_case(email)
}

/// The line with one parameter replaced, added or removed, and every other
/// byte kept. Its own scanner rather than a `parse_line` round trip, because a
/// reparse would strip parameter quotes — and a quoted `CN` may legally
/// contain the `;` and `:` this splits on, so both splits honour quotes.
/// Only called on lines [`is_attendee_of`] already parsed, so the colon is
/// known to exist.
///
/// `None` removes the parameter, which is how a display name is cleared and an
/// optional attendee made required. The value is written as given: a caller
/// with user text runs it through [`param_value`] first.
fn with_param(line: &str, name: &str, value: Option<&str>) -> String {
    let mut in_quotes = false;
    let mut colon = line.len();
    for (i, ch) in line.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon = i;
                break;
            }
            _ => {}
        }
    }
    let (head, value_str) = line.split_at(colon);

    let mut segs: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut q = false;
    for ch in head.chars() {
        match ch {
            '"' => {
                q = !q;
                cur.push(ch);
            }
            ';' if !q => segs.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    segs.push(cur);

    let prefix = format!("{}=", name.to_ascii_uppercase());
    let mut replaced = false;
    let mut out: Vec<String> = Vec::with_capacity(segs.len());
    for (i, s) in segs.iter().enumerate() {
        if i > 0 && s.trim().to_ascii_uppercase().starts_with(&prefix) {
            match value {
                Some(v) if !replaced => out.push(format!("{name}={v}")),
                // A duplicate of the same parameter is not legal, and keeping
                // the first is the reading every parser gives it.
                _ => {}
            }
            replaced = true;
            continue;
        }
        out.push(s.clone());
    }
    if let (Some(v), false) = (value, replaced) {
        out.push(format!("{name}={v}"));
    }
    format!("{}{value_part}", out.join(";"), value_part = value_str)
}

/// Sets `email`'s `PARTSTAT` across every VEVENT of `uid` — master and
/// exceptions alike, which is what answering "all of them" means: an
/// exception carries its own `ATTENDEE` lines, and a master-only rewrite
/// would leave last week's moved occurrence still saying needs-action.
///
/// The input is unfolded first — folded `ATTENDEE` lines are how iCloud
/// actually ships a long `CN` + mailto pair, and the fold can land inside
/// the mailbox itself — so the output is the same resource on single lines:
/// a byte change, never a data change, and the liberty `new_event_ics`
/// already takes with long lines.
///
/// `None` when no VEVENT of `uid` exposes this attendee. That is the guard,
/// not a shrug: a resource that does not carry your invitation cannot be
/// answered, and inventing an `ATTENDEE` line would fabricate one the
/// organizer never sent.
pub fn respond_all(raw: &str, uid: &str, email: &str, partstat: &str) -> Option<String> {
    let lines = unfold(raw);
    let blocks = event_blocks(&lines);
    let mut out = lines.clone();
    let mut touched = false;
    for b in blocks.iter().filter(|b| b.uid.as_deref() == Some(uid)) {
        for i in b.start..=b.end {
            if is_attendee_of(&lines[i], email) {
                out[i] = with_param(&lines[i], "PARTSTAT", Some(partstat));
                touched = true;
            }
        }
    }
    touched.then(|| out.join("\r\n"))
}

/// Sets `email`'s `PARTSTAT` for one occurrence of a series — scope "this".
/// An exception already materialised for the occurrence is answered in
/// place; otherwise one is cloned from the master — unmodeled lines
/// verbatim, the series machinery (`RRULE`/`RDATE`/`EXDATE`) dropped, the
/// occurrence's own `RECURRENCE-ID`/`DTSTART`/`DTEND` and a fresh `DTSTAMP`
/// added — and answered there. The master's own `ATTENDEE` lines stay as
/// they were: that is the entire difference between "this one" and "all of
/// them". Unfolds like [`respond_all`], refuses like it too.
pub fn respond_occurrence(
    raw: &str,
    uid: &str,
    email: &str,
    partstat: &str,
    rid: &WriteTime,
    dtend: &WriteTime,
    now: Timestamp,
) -> Option<String> {
    let lines = unfold(raw);
    let blocks = event_blocks(&lines);
    let rid_line = rid.prop("RECURRENCE-ID");
    let rid_tail = rid_line.rsplit(':').next();

    let existing = blocks.iter().find(|b| {
        b.is_exception
            && b.uid.as_deref() == Some(uid)
            && lines[b.start..=b.end].iter().any(|l| {
                l.to_ascii_uppercase().starts_with("RECURRENCE-ID")
                    && l.rsplit(':').next() == rid_tail
            })
    });
    if let Some(b) = existing {
        let mut out = lines.clone();
        let mut touched = false;
        for i in b.start..=b.end {
            if is_attendee_of(&lines[i], email) {
                out[i] = with_param(&lines[i], "PARTSTAT", Some(partstat));
                touched = true;
            }
        }
        return touched.then(|| out.join("\r\n"));
    }

    let master = blocks.iter().find(|b| !b.is_exception && b.uid.as_deref() == Some(uid))?;
    let dropped = |l: &str| {
        let u = l.to_ascii_uppercase();
        u.starts_with("RRULE")
            || u.starts_with("RDATE")
            || u.starts_with("EXDATE")
            || u.starts_with("DTSTART")
            || u.starts_with("DTEND")
            || u.starts_with("DURATION")
            || u.starts_with("DTSTAMP")
    };
    let mut clone: Vec<String> = Vec::new();
    let mut touched = false;
    for l in &lines[master.start..=master.end] {
        if dropped(l) {
            continue;
        }
        let line = if is_attendee_of(l, email) {
            touched = true;
            with_param(l, "PARTSTAT", Some(partstat))
        } else {
            l.clone()
        };
        clone.push(line);
        if l.eq_ignore_ascii_case("BEGIN:VEVENT") {
            clone.push(rid_line.clone());
            clone.push(rid.prop("DTSTART"));
            clone.push(dtend.prop("DTEND"));
            clone.push(format!("DTSTAMP:{}", fmt_utc(now)));
        }
    }
    if !touched {
        return None;
    }

    let close = lines.iter().rposition(|l| l.eq_ignore_ascii_case("END:VCALENDAR"))?;
    let mut out: Vec<String> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if i == close {
            out.extend(clone.iter().cloned());
        }
        out.push(l.clone());
    }
    Some(out.join("\r\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:abc-1\r\nSUMMARY:Standup\\, daily\r\nDTSTART;TZID=Europe/Sofia:20260817T093000\r\nDTEND;TZID=Europe/Sofia:20260817T094500\r\nSTATUS:CONFIRMED\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn a_simple_vevent_parses_completely() {
        let root = parse(SIMPLE).unwrap();
        let evs = events_in(&root);
        assert_eq!(evs.len(), 1);
        let e = &evs[0];
        assert_eq!(e.uid, "abc-1");
        assert_eq!(e.summary.as_deref(), Some("Standup, daily"), "escaped comma unescapes");
        assert_eq!(e.alarms, vec![("popup".to_string(), 15)]);
        let (start_ms, tz, all_day) = resolve(&e.start, "UTC").unwrap();
        assert_eq!(tz, "Europe/Sofia");
        assert!(!all_day);
        // 09:30 Sofia (UTC+3 in August) = 06:30Z.
        let z = Timestamp::from_millisecond(start_ms).unwrap().to_zoned(jiff::tz::TimeZone::UTC);
        assert_eq!((z.hour(), z.minute()), (6, 30));
    }

    #[test]
    fn folded_lines_unfold() {
        let src = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:f1\r\nSUMMARY:A very long tit\r\n le that was folded\r\nDTSTART:20260817T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR";
        let root = parse(src).unwrap();
        let evs = events_in(&root);
        assert_eq!(evs[0].summary.as_deref(), Some("A very long title that was folded"));
    }

    #[test]
    fn all_day_dates_resolve_in_the_calendar_zone() {
        let src = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:d1\nDTSTART;VALUE=DATE:20260820\nDTEND;VALUE=DATE:20260821\nEND:VEVENT\nEND:VCALENDAR";
        let root = parse(src).unwrap();
        let e = &events_in(&root)[0];
        let (ms, tz, all_day) = resolve(&e.start, "Europe/Sofia").unwrap();
        assert!(all_day);
        assert_eq!(tz, "Europe/Sofia");
        // Midnight Sofia on Aug 20 = Aug 19 21:00Z.
        let z = Timestamp::from_millisecond(ms).unwrap().to_zoned(jiff::tz::TimeZone::UTC);
        assert_eq!((z.day(), z.hour()), (19, 21));
    }

    #[test]
    fn recurrence_lines_pass_through_verbatim() {
        let src = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:r1\nDTSTART;TZID=Europe/Sofia:20260817T090000\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE\nEXDATE;TZID=Europe/Sofia:20260824T090000\nEND:VEVENT\nEND:VCALENDAR";
        let e = &events_in(&parse(src).unwrap())[0];
        assert_eq!(
            e.recurrence,
            vec![
                "RRULE:FREQ=WEEKLY;BYDAY=MO,WE".to_string(),
                "EXDATE;TZID=Europe/Sofia:20260824T090000".to_string(),
            ]
        );
    }

    #[test]
    fn a_master_sorts_before_its_exception() {
        let src = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:s1\nRECURRENCE-ID;TZID=Europe/Sofia:20260818T090000\nDTSTART;TZID=Europe/Sofia:20260818T100000\nEND:VEVENT\nBEGIN:VEVENT\nUID:s1\nDTSTART;TZID=Europe/Sofia:20260817T090000\nRRULE:FREQ=DAILY\nEND:VEVENT\nEND:VCALENDAR";
        let evs = events_in(&parse(src).unwrap());
        assert_eq!(evs.len(), 2);
        assert!(evs[0].recurrence_id.is_none(), "master first");
        assert!(evs[1].recurrence_id.is_some());
    }

    #[test]
    fn unknown_tzids_fall_back_to_the_calendar_zone() {
        let p = Property {
            name: "DTSTART".into(),
            params: vec![("TZID".into(), "Cupertino Standard Time".into())],
            value: "20260817T090000".into(),
        };
        let t = parse_time(&p).unwrap();
        let (_, tz, _) = resolve(&t, "Europe/Sofia").unwrap();
        assert_eq!(tz, "Europe/Sofia");
    }

    #[test]
    fn durations_parse() {
        assert_eq!(parse_duration_ms("PT1H30M"), Some(5_400_000));
        assert_eq!(parse_duration_ms("-PT15M"), Some(-900_000));
        assert_eq!(parse_duration_ms("P1D"), Some(86_400_000));
        assert_eq!(parse_duration_ms("P1W"), Some(604_800_000));
        assert_eq!(parse_duration_ms("nonsense"), None);
    }

    const TODO: &str = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t-9\r\nSUMMARY:Buy stamps\r\nDUE;VALUE=DATE:20260820\r\nPRIORITY:1\r\nX-APPLE-SPECIAL:kept\r\nEND:VTODO\r\nEND:VCALENDAR";

    #[test]
    fn a_vtodo_parses() {
        let t = &todos_in(&parse(TODO).unwrap())[0];
        assert_eq!(t.uid, "t-9");
        assert_eq!(t.summary.as_deref(), Some("Buy stamps"));
        assert_eq!(t.status, "needs-action");
        assert_eq!(t.priority, 1);
        assert!(matches!(t.due, Some(IcsTime::Date(_))));
    }

    /// The whole point of line-surgery: complete a task and the vendor
    /// extension line survives untouched.
    /// An edit rewrites what it names and nothing else. The categories and
    /// the alarm here are the point: a resource carries properties omacal
    /// does not model, and a round trip through our own struct would drop
    /// every one of them.
    #[test]
    fn an_edit_rewrites_its_own_fields_and_leaves_the_rest_alone() {
        let raw = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:t-9\r\n\
            SUMMARY:Old title\r\nDUE;VALUE=DATE:20260910\r\nDESCRIPTION:old note\r\n\
            CATEGORIES:home\r\nSEQUENCE:3\r\nSTATUS:NEEDS-ACTION\r\nPRIORITY:5\r\n\
            BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT30M\r\nEND:VALARM\r\n\
            END:VTODO\r\nEND:VCALENDAR\r\n";
        let now: Timestamp = "2026-09-07T09:00:00Z".parse().unwrap();
        let edit = TodoEdit {
            summary: "New title",
            due: Some(TodoDue::At("2026-09-11T12:30:00Z".parse().unwrap())),
            description: Some("new note"),
            // Named explicitly, because priority is a modelled field now: the
            // caller passes the stored value through to keep it. The other
            // lines below — CATEGORIES, VALARM, STATUS — stay unmodelled and
            // are preserved by the surgery alone.
            priority: Some(5),
        };
        let out = patch_todo_fields(raw, "t-9", &edit, "Europe/Sofia", now).unwrap();

        assert!(out.contains("SUMMARY:New title"), "{out}");
        assert!(!out.contains("Old title"));
        assert!(out.contains("DUE;TZID=Europe/Sofia:20260911T153000"), "{out}");
        assert!(
            out.contains("DTSTART;TZID=Europe/Sofia:20260911T153000"),
            "iCloud only honours a timed DUE when DTSTART mirrors it: {out}"
        );
        assert!(!out.contains("VALUE=DATE"), "the old date-only DUE is gone");
        assert!(out.contains("DESCRIPTION:new note"));
        // SEQUENCE is what tells other clients this supersedes their copy.
        assert!(out.contains("SEQUENCE:4"), "{out}");
        assert!(out.contains("LAST-MODIFIED:20260907T090000Z"));
        // Untouched, all of it.
        assert!(out.contains("CATEGORIES:home"));
        assert!(out.contains("PRIORITY:5"));
        assert!(out.contains("STATUS:NEEDS-ACTION"));
        assert!(out.contains("BEGIN:VALARM") && out.contains("TRIGGER:-PT30M"));
        assert_eq!(out.matches("BEGIN:VALARM").count(), 1, "another client's alarm is not doubled: {out}");
    }

    /// The window's quick-add creates a date-only task, and its time arrives
    /// in a later edit — so an edit that gives a task its first time is the
    /// path most tasks take, and it has to leave the alarm a timed task needs
    /// (found 2026-09-18: "9/18 Test 2" carried DTSTART/DUE and no VALARM, and
    /// the phone never fired).
    #[test]
    fn an_edit_that_gives_a_task_a_time_gives_it_an_alarm() {
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t-8\r\nSUMMARY:Call\r\n\
            DUE;VALUE=DATE:20260918\r\nSTATUS:NEEDS-ACTION\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let now: Timestamp = "2026-09-18T09:00:00Z".parse().unwrap();
        let edit = TodoEdit {
            summary: "Call",
            due: Some(TodoDue::At("2026-09-18T21:55:00Z".parse().unwrap())),
            description: None,
            priority: None,
        };
        let out = patch_todo_fields(raw, "t-8", &edit, "America/Denver", now).unwrap();
        assert_eq!(out.matches("BEGIN:VALARM").count(), 1, "{out}");
        assert!(out.contains("ACTION:DISPLAY") && out.contains("TRIGGER:PT0M"), "{out}");
        assert!(
            out.find("BEGIN:VALARM").unwrap() < out.find("END:VTODO").unwrap(),
            "the alarm sits inside the VTODO: {out}"
        );
        assert!(
            out.find("LAST-MODIFIED").unwrap() < out.find("BEGIN:VALARM").unwrap(),
            "properties before nested components, as RFC 5545 wants: {out}"
        );
    }

    /// A bare date has no time to alarm at, whatever the edit.
    #[test]
    fn an_edit_that_leaves_a_task_date_only_gives_it_no_alarm() {
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t-7\r\nSUMMARY:Call\r\n\
            DUE;VALUE=DATE:20260918\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let now: Timestamp = "2026-09-18T09:00:00Z".parse().unwrap();
        let edit = TodoEdit {
            summary: "Call it",
            due: Some(TodoDue::Date(Date::new(2026, 9, 19).unwrap())),
            description: None,
            priority: None,
        };
        let out = patch_todo_fields(raw, "t-7", &edit, "America/Denver", now).unwrap();
        assert!(!out.contains("VALARM"), "{out}");
    }

    /// Completing a task must not conjure an alarm on one that never had it.
    #[test]
    fn completing_a_task_adds_no_alarm() {
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t-6\r\nSUMMARY:Call\r\n\
            DUE;TZID=America/Denver:20260918T155500\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let now: Timestamp = "2026-09-18T09:00:00Z".parse().unwrap();
        let out = patch_todo_status(raw, "t-6", true, now).unwrap();
        assert!(!out.contains("VALARM"), "{out}");
    }

    /// Clearing is a real answer: no due date and no note means the
    /// properties go, not that they keep their old values.
    #[test]
    fn an_edit_can_clear_the_due_date_and_the_note() {
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t-1\r\nSUMMARY:Thing\r\n\
            DUE;VALUE=DATE:20260910\r\nDESCRIPTION:note\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let now: Timestamp = "2026-09-07T09:00:00Z".parse().unwrap();
        let out = patch_todo_fields(
            raw, "t-1",
            &TodoEdit { summary: "Thing", due: None, description: None, priority: None },
            "Europe/Sofia", now,
        ).unwrap();
        assert!(!out.contains("DUE"), "{out}");
        assert!(!out.contains("DESCRIPTION"), "{out}");
        assert!(out.contains("SEQUENCE:1"), "an absent SEQUENCE counts as 0");
    }

    /// A date-only due date keeps its shape: "Thursday" and "Thursday at
    /// 18:00" are different promises and iCalendar spells them differently.
    #[test]
    fn a_date_only_due_stays_date_only() {
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t-1\r\nSUMMARY:Thing\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let now: Timestamp = "2026-09-07T09:00:00Z".parse().unwrap();
        let out = patch_todo_fields(
            raw, "t-1",
            &TodoEdit {
                summary: "Thing",
                due: Some(TodoDue::Date(jiff::civil::date(2026, 9, 10))),
                description: None,
                priority: None,
            },
            "Europe/Sofia", now,
        ).unwrap();
        assert!(out.contains("DUE;VALUE=DATE:20260910"), "{out}");
    }

    /// Issue #102, confirmed against a real iCloud account: a list added to
    /// Apple Reminders through CalDAV, whose tasks Reminders wrote with a
    /// `DTSTART`. A `DUE` sent without a matching `DTSTART` — or with a
    /// `DTSTART;VALUE=DATE` beside a now DATE-TIME `DUE`, which RFC 5545
    /// §3.6.2 already forbids — round-trips through iCloud's Reminders
    /// backend as a date-only due, the time silently dropped. Apple's own
    /// client never sends a timed due without an identical `DTSTART`
    /// (verified from a task iOS itself created), so this mirrors that: the
    /// old bare-date start is replaced with one at the same instant as the
    /// new due, not merely dropped.
    #[test]
    fn a_timed_due_takes_a_matching_dtstart_with_it() {
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t-2\r\nSUMMARY:Buy stamps\r\n\
            DTSTART;VALUE=DATE:20260910\r\nDUE;VALUE=DATE:20260911\r\n\
            X-APPLE-SORT-ORDER:12\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let now: Timestamp = "2026-09-07T09:00:00Z".parse().unwrap();
        let edit = TodoEdit {
            summary: "Buy stamps",
            due: Some(TodoDue::At("2026-09-11T12:30:00Z".parse().unwrap())),
            description: None,
            priority: None,
        };
        let out = patch_todo_fields(raw, "t-2", &edit, "Europe/Sofia", now).unwrap();

        assert!(!out.contains("VALUE=DATE"), "the bare-date start cannot stand beside a timed due: {out}");
        assert!(out.contains("DUE;TZID=Europe/Sofia:20260911T153000"), "{out}");
        assert!(
            out.contains("DTSTART;TZID=Europe/Sofia:20260911T153000"),
            "the new start mirrors the new due, the way iCloud's own writes do: {out}"
        );
        assert!(out.contains("X-APPLE-SORT-ORDER:12"), "everything else we do not model survives");
    }

    /// The other half of the same rule: a start that still agrees with the
    /// due stays. Discarding another client's data is the cost of last
    /// resort, not the first move.
    #[test]
    fn a_dtstart_that_still_agrees_with_the_due_is_kept() {
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t-3\r\nSUMMARY:Trip\r\n\
            DTSTART;VALUE=DATE:20260910\r\nDUE;VALUE=DATE:20260911\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let now: Timestamp = "2026-09-07T09:00:00Z".parse().unwrap();
        let edit = TodoEdit {
            summary: "Trip",
            due: Some(TodoDue::Date(jiff::civil::date(2026, 9, 12))),
            description: None,
            priority: None,
        };
        let out = patch_todo_fields(raw, "t-3", &edit, "Europe/Sofia", now).unwrap();
        assert!(out.contains("DTSTART;VALUE=DATE:20260910"), "{out}");
        assert!(out.contains("DUE;VALUE=DATE:20260912"), "{out}");
    }

    /// A start that is the right value type but no longer earlier than the
    /// due is just as invalid, and is replaced by the mirrored start for the
    /// same reason a bare-date one is.
    #[test]
    fn a_dtstart_later_than_the_due_is_replaced_too() {
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t-4\r\nSUMMARY:Thing\r\n\
            DTSTART;TZID=Europe/Sofia:20260911T180000\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let now: Timestamp = "2026-09-07T09:00:00Z".parse().unwrap();
        let edit = TodoEdit {
            summary: "Thing",
            // 15:30 Sofia, before the 18:00 start.
            due: Some(TodoDue::At("2026-09-11T12:30:00Z".parse().unwrap())),
            description: None,
            priority: None,
        };
        let out = patch_todo_fields(raw, "t-4", &edit, "Europe/Sofia", now).unwrap();
        assert!(!out.contains("180000"), "the stale 18:00 start is gone: {out}");
        assert!(out.contains("DTSTART;TZID=Europe/Sofia:20260911T153000"), "{out}");
    }

    /// The due a user set is the due that comes back, whichever spelling it
    /// went out in — the round trip is what the edit is for.
    #[test]
    fn a_timed_due_round_trips_through_the_calendars_zone() {
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t-5\r\nSUMMARY:Call\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let now: Timestamp = "2026-09-07T09:00:00Z".parse().unwrap();
        let due: Timestamp = "2026-09-11T12:30:00Z".parse().unwrap();
        let out = patch_todo_fields(
            raw, "t-5",
            &TodoEdit { summary: "Call", due: Some(TodoDue::At(due)), description: None, priority: None },
            "Australia/Brisbane", now,
        ).unwrap();

        let back = &todos_in(&parse(&out).unwrap())[0];
        let (ms, _, all_day) = resolve(back.due.as_ref().unwrap(), "Australia/Brisbane").unwrap();
        assert_eq!(ms, due.as_millisecond(), "the instant survived the zone it was written in");
        assert!(!all_day, "a timed due must not read back as a date");
    }

    /// Only the matching task is touched, and a UID that is not in the
    /// resource writes nothing at all rather than a copy with no edit.
    #[test]
    fn a_second_task_in_the_same_resource_is_untouched_and_a_stranger_is_refused() {
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:a\r\nSUMMARY:First\r\nEND:VTODO\r\n\
            BEGIN:VTODO\r\nUID:b\r\nSUMMARY:Second\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let now: Timestamp = "2026-09-07T09:00:00Z".parse().unwrap();
        let edit = TodoEdit { summary: "Renamed", due: None, description: None, priority: None };
        let out = patch_todo_fields(raw, "b", &edit, "Europe/Sofia", now).unwrap();
        assert!(out.contains("SUMMARY:First"), "{out}");
        assert!(out.contains("SUMMARY:Renamed"));
        assert!(!out.contains("SUMMARY:Second"));
        assert_eq!(patch_todo_fields(raw, "nobody", &edit, "Europe/Sofia", now), None);
    }

    #[test]
    fn completing_a_todo_preserves_what_we_do_not_model() {
        let now = Timestamp::from_millisecond(1_786_352_400_000).unwrap();
        let done = patch_todo_status(TODO, "t-9", true, now).unwrap();
        assert!(done.contains("X-APPLE-SPECIAL:kept"), "vendor line survives");
        assert!(done.contains("STATUS:COMPLETED"));
        assert!(done.contains("PERCENT-COMPLETE:100"));
        assert!(done.contains("COMPLETED:20260810T090000Z"));

        let back = patch_todo_status(&done, "t-9", false, now).unwrap();
        assert!(back.contains("STATUS:NEEDS-ACTION"));
        assert!(!back.contains("STATUS:COMPLETED"));
        assert!(!back.contains("PERCENT-COMPLETE"));
        assert!(back.contains("X-APPLE-SPECIAL:kept"));
    }

    /// Timestamp-based CalDAV clients (including iCloudBridge) must see
    /// both completion and reopening as a new revision of the task.
    #[test]
    fn changing_task_completion_advances_its_revision() {
        for (metadata, sequence) in [("", 0), ("SEQUENCE:7\r\nDTSTAMP:20260101T000000Z\r\nLAST-MODIFIED:20260101T000000Z\r\n", 7)] {
            let raw = format!("BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:task\r\n{metadata}STATUS:NEEDS-ACTION\r\nEND:VTODO\r\nEND:VCALENDAR\r\n");
            let now: Timestamp = "2026-09-07T18:00:00Z".parse().unwrap();
            let done = patch_todo_status(&raw, "task", true, now).unwrap();
            assert!(done.contains("LAST-MODIFIED:20260907T180000Z"), "completion must be discoverable");
            assert!(done.contains("DTSTAMP:20260907T180000Z"));
            assert!(done.contains(&format!("SEQUENCE:{}\r\n", sequence + 1)));
            let later: Timestamp = "2026-09-07T18:01:00Z".parse().unwrap();
            let reopened = patch_todo_status(&done, "task", false, later).unwrap();
            assert!(reopened.contains("LAST-MODIFIED:20260907T180100Z"), "reopening must be discoverable");
            assert!(reopened.contains("DTSTAMP:20260907T180100Z"));
            assert!(reopened.contains(&format!("SEQUENCE:{}\r\n", sequence + 2)));
            assert_eq!(reopened.matches("LAST-MODIFIED:").count(), 1);
            assert_eq!(reopened.matches("DTSTAMP:").count(), 1);
            assert_eq!(reopened.matches("SEQUENCE:").count(), 1);
        }
    }

    /// Issue #114's own example, both halves of it: the common case gets the
    /// `mailto:` it means, and a `urn:uuid:` attendee is written as the URI it
    /// already is rather than mangled into something mailbox-shaped.
    #[test]
    fn a_new_attendee_is_a_mailto_and_a_urn_address_survives_verbatim() {
        let now = Timestamp::from_millisecond(NOW).unwrap();
        let mut ev = write_sample("n1");
        ev.attendees = Some(vec![
            AttendeeWrite { address: "ada@example.com".into(), display_name: Some("Ada Lovelace".into()), optional: false },
            AttendeeWrite { address: "urn:uuid:12345678".into(), display_name: None, optional: true },
        ]);
        let out = new_event_ics(&ev, now);
        assert!(out.contains("ATTENDEE;CN=Ada Lovelace;PARTSTAT=NEEDS-ACTION:mailto:ada@example.com\r\n"), "{out}");
        assert!(out.contains("ATTENDEE;ROLE=OPT-PARTICIPANT;PARTSTAT=NEEDS-ACTION:urn:uuid:12345678\r\n"), "{out}");
        // And it reads back as what was written, addresses included.
        let parsed = events_in(&parse(&out).unwrap());
        let names: Vec<&str> = parsed[0].attendees.iter().map(|a| a.email.as_str()).collect();
        assert_eq!(names, ["ada@example.com", "urn:uuid:12345678"]);
        assert!(parsed[0].attendees[1].optional);
    }

    /// The reason a `Some` is reconciled rather than rendered: an unrelated
    /// edit must not reset anybody's reply. The one who stays keeps their
    /// `PARTSTAT` and the `X-` line the server hung off them; the one dropped
    /// loses their line; the new one arrives needing action.
    #[test]
    fn editing_a_guest_list_keeps_the_answers_of_the_guests_who_stay() {
        let now = Timestamp::from_millisecond(NOW).unwrap();
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:g1\r\nSUMMARY:Review\r\n\
DTSTART;TZID=Europe/Sofia:20260817T091500\r\nDTEND;TZID=Europe/Sofia:20260817T093000\r\n\
ATTENDEE;CN=Ada;PARTSTAT=ACCEPTED;X-NUM-GUESTS=0:mailto:ada@example.com\r\n\
ATTENDEE;PARTSTAT=DECLINED:mailto:bob@example.com\r\n\
END:VEVENT\r\nEND:VCALENDAR";
        let mut ev = write_sample("g1");
        ev.summary = Some("Review, renamed".into());
        ev.attendees = Some(vec![
            AttendeeWrite { address: "ADA@example.com".into(), display_name: Some("Ada".into()), optional: false },
            AttendeeWrite { address: "cleo@example.com".into(), display_name: None, optional: false },
        ]);
        let out = rewrite_master(raw, "g1", &ev, now, false).unwrap();
        assert!(out.contains("ATTENDEE;CN=Ada;PARTSTAT=ACCEPTED;X-NUM-GUESTS=0:mailto:ada@example.com"),
            "the mailbox matched case-insensitively and the line came through whole: {out}");
        assert!(!out.contains("bob@example.com"), "removed: {out}");
        assert!(out.contains("ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:cleo@example.com"), "{out}");
        assert_eq!(out.matches("ATTENDEE").count(), 2);
    }

    /// `None` is the three-state's whole point, and the behaviour every write
    /// had before attendees were editable: a drag, a resize, a title change
    /// from a path with no guest editor leaves the list exactly as found.
    #[test]
    fn an_edit_that_does_not_own_the_list_leaves_every_attendee_line() {
        let now = Timestamp::from_millisecond(NOW).unwrap();
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:g2\r\nSUMMARY:Review\r\n\
DTSTART;TZID=Europe/Sofia:20260817T091500\r\nDTEND;TZID=Europe/Sofia:20260817T093000\r\n\
ORGANIZER;CN=Ivan:mailto:ivan@example.com\r\n\
ATTENDEE;CN=Ada;PARTSTAT=ACCEPTED:mailto:ada@example.com\r\n\
END:VEVENT\r\nEND:VCALENDAR";
        let mut ev = write_sample("g2");
        ev.summary = Some("Review, renamed".into());
        assert_eq!(ev.attendees, None);
        let out = rewrite_master(raw, "g2", &ev, now, false).unwrap();
        assert!(out.contains("ATTENDEE;CN=Ada;PARTSTAT=ACCEPTED:mailto:ada@example.com"), "{out}");
        assert!(out.contains("ORGANIZER;CN=Ivan:mailto:ivan@example.com"), "{out}");
    }

    /// A name is the editor's to set and to clear, and a `;` in one is why
    /// `param_value` exists: unquoted it would read as the end of the
    /// parameter and the start of another.
    #[test]
    fn a_display_name_is_quoted_when_it_has_to_be_and_can_be_cleared() {
        let now = Timestamp::from_millisecond(NOW).unwrap();
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:g3\r\nSUMMARY:Review\r\n\
DTSTART;TZID=Europe/Sofia:20260817T091500\r\nDTEND;TZID=Europe/Sofia:20260817T093000\r\n\
ATTENDEE;CN=Ada;ROLE=OPT-PARTICIPANT;PARTSTAT=ACCEPTED:mailto:ada@example.com\r\n\
END:VEVENT\r\nEND:VCALENDAR";
        let mut ev = write_sample("g3");
        ev.attendees = Some(vec![
            AttendeeWrite { address: "ada@example.com".into(), display_name: None, optional: false },
            AttendeeWrite { address: "cleo@example.com".into(), display_name: Some("Cleo; the second".into()), optional: false },
        ]);
        let out = rewrite_master(raw, "g3", &ev, now, false).unwrap();
        assert!(out.contains("ATTENDEE;PARTSTAT=ACCEPTED:mailto:ada@example.com"),
            "the name and the optional flag both came off, the answer did not: {out}");
        assert!(out.contains("ATTENDEE;CN=\"Cleo; the second\";PARTSTAT=NEEDS-ACTION:mailto:cleo@example.com"), "{out}");
        // And the quoted name survives the reader that has to split on that `;`.
        let parsed = events_in(&parse(&out).unwrap());
        let cleo = parsed[0].attendees.iter().find(|a| a.email == "cleo@example.com").unwrap();
        assert_eq!(cleo.display_name.as_deref(), Some("Cleo; the second"));
    }

    /// A quoted parameter holds the `;` the parameter list splits on, and
    /// iCloud quotes display names as a matter of course. Reading it wrong
    /// cost half a name and a mis-read `ROLE`; writing one made it ours to get
    /// right. Pinned at the parser rather than only through an attendee.
    #[test]
    fn a_quoted_parameter_may_hold_the_separators_around_it() {
        let p = parse_line("ATTENDEE;CN=\"Ada; of: Lovelace\";ROLE=CHAIR:mailto:ada@example.com").unwrap();
        assert_eq!(p.param("CN"), Some("Ada; of: Lovelace"));
        assert_eq!(p.param("ROLE"), Some("CHAIR"));
        assert_eq!(p.value, "mailto:ada@example.com");
    }

    /// An exception block is rewritten whole, so its own `ATTENDEE` lines have
    /// to be carried across by hand. Before this they were dropped: editing one
    /// occurrence of a series emptied that occurrence's guest list, silently,
    /// on a resource nobody was looking at.
    #[test]
    fn editing_one_occurrence_keeps_that_occurrence_s_own_attendees() {
        let now = Timestamp::from_millisecond(NOW).unwrap();
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:s2\r\nSUMMARY:Standup\r\n\
DTSTART;TZID=Europe/Sofia:20260817T091500\r\nDTEND;TZID=Europe/Sofia:20260817T093000\r\n\
RRULE:FREQ=DAILY;COUNT=10\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:s2\r\nRECURRENCE-ID;TZID=Europe/Sofia:20260819T091500\r\n\
SUMMARY:Standup (moved)\r\nDTSTART;TZID=Europe/Sofia:20260819T140000\r\n\
DTEND;TZID=Europe/Sofia:20260819T141500\r\n\
ATTENDEE;CN=Ada;PARTSTAT=ACCEPTED:mailto:ada@example.com\r\n\
END:VEVENT\r\nEND:VCALENDAR";
        let mut ev = write_sample("s2");
        ev.summary = Some("Standup (moved again)".into());
        ev.recurrence_id = Some(WriteTime::Zoned {
            dt: jiff::civil::date(2026, 8, 19).at(9, 15, 0, 0),
            tzid: "Europe/Sofia".into(),
        });
        let out = upsert_exception(raw, &ev, now).unwrap();
        assert!(out.contains("ATTENDEE;CN=Ada;PARTSTAT=ACCEPTED:mailto:ada@example.com"), "{out}");
        assert!(out.contains("SUMMARY:Standup (moved again)"), "{out}");
        assert_eq!(out.matches("BEGIN:VEVENT").count(), 2, "still one master and one exception: {out}");
    }

    /// A server's copy: a DESCRIPTION folded across three physical lines
    /// *before* the UID, and an Apple alarm with a DESCRIPTION of its own.
    const FOLDED: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nDTSTAMP:20260901T000000Z\r\nDESCRIPTION:Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusm\r\n od tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim ve\r\n niam, quis nostrud\r\nUID:abc\r\nSUMMARY:Pay rent\r\nX-APPLE-SORT-ORDER:7\r\nSEQUENCE:3\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Reminder\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";

    fn read_back(raw: &str) -> (CalTodo, Component) {
        let root = parse(raw).expect("the patched resource still parses");
        let todo = todos_in(&root).into_iter().next().expect("one VTODO");
        let t = root.components("VTODO").next().unwrap().clone();
        (todo, t)
    }

    /// Found 2026-09-17 by an audit and reproduced: a folded property's
    /// continuation lines outlived the property they belonged to and joined
    /// the line above, and the alarm lost its text.
    #[test]
    fn editing_a_folded_task_keeps_its_uid_its_alarm_and_the_rfc_order() {
        let now = Timestamp::from_millisecond(1_790_000_000_000).unwrap();
        // A timed due, so the task keeps the alarm this test then reads: an
        // edit that takes the time away takes a relative alarm with it, which
        // is `clearing_a_tasks_time_takes_the_alarm_that_hung_on_it`'s subject
        // rather than this one's (folding).
        let edit = TodoEdit {
            summary: "Pay the rent",
            due: Some(TodoDue::At("2026-09-19T12:00:00Z".parse().unwrap())),
            description: Some("to the landlord"),
            priority: None,
        };
        let out = patch_todo_fields(FOLDED, "abc", &edit, "Europe/Sofia", now).unwrap();
        let (todo, vtodo) = read_back(&out);
        assert_eq!(todo.uid, "abc", "no continuation line joined the UID");
        assert_eq!(todo.summary.as_deref(), Some("Pay the rent"));
        assert_eq!(todo.description.as_deref(), Some("to the landlord"));
        assert!(!out.contains(" od tempor") && !out.contains(" niam, quis"), "the old folds went with it");
        let alarm = vtodo.components("VALARM").next().expect("the alarm survives");
        assert_eq!(alarm.prop_value("DESCRIPTION"), Some("Reminder"), "the alarm keeps its own text");
        assert_eq!(vtodo.prop_value("X-APPLE-SORT-ORDER"), Some("7"), "what omacal does not model stays");
        assert_eq!(vtodo.prop_value("SEQUENCE"), Some("4"));
        // Properties first, then the alarm: RFC 5545's `todoc`.
        let summary_at = out.find("SUMMARY:Pay the rent").unwrap();
        assert!(summary_at < out.find("BEGIN:VALARM").unwrap(), "no property after the alarm");
    }

    #[test]
    fn completing_a_folded_task_keeps_its_uid_its_note_and_its_alarm() {
        let now = Timestamp::from_millisecond(1_790_000_000_000).unwrap();
        let out = patch_todo_status(FOLDED, "abc", true, now).unwrap();
        let (todo, vtodo) = read_back(&out);
        assert_eq!(todo.uid, "abc");
        assert_eq!(todo.status.to_ascii_uppercase(), "COMPLETED");
        // Untouched, so it goes back folded exactly as the server folded it.
        assert!(out.contains("DESCRIPTION:Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusm\r\n od tempor"));
        assert!(todo.description.as_deref().is_some_and(|d| d.ends_with("quis nostrud")));
        assert_eq!(vtodo.components("VALARM").next().unwrap().prop_value("DESCRIPTION"), Some("Reminder"));
        assert!(out.find("STATUS:COMPLETED").unwrap() < out.find("BEGIN:VALARM").unwrap());
        // And a resource that never held the UID is not written back at all.
        assert_eq!(patch_todo_status(FOLDED, "nobody", true, now), None);
    }

    #[test]
    fn patching_one_uid_leaves_its_siblings_alone() {
        let two = "BEGIN:VCALENDAR\nBEGIN:VTODO\nUID:a\nSTATUS:NEEDS-ACTION\nEND:VTODO\nBEGIN:VTODO\nUID:b\nSTATUS:NEEDS-ACTION\nEND:VTODO\nEND:VCALENDAR";
        let now = Timestamp::from_millisecond(0).unwrap();
        let done = patch_todo_status(two, "b", true, now).unwrap();
        assert_eq!(done.matches("STATUS:NEEDS-ACTION").count(), 1, "a untouched");
        assert_eq!(done.matches("STATUS:COMPLETED").count(), 1, "b completed");
    }

    fn write_sample(uid: &str) -> EventWrite {
        EventWrite {
            uid: uid.into(),
            // The default every existing case wants: this edit does not own
            // the guest list, so every ATTENDEE line stays where it is.
            attendees: None,
            summary: Some("Planning".into()),
            location: Some("Room 2; annex".into()),
            description: None,
            start: WriteTime::Zoned {
                dt: jiff::civil::date(2026, 8, 20).at(10, 0, 0, 0),
                tzid: "Europe/Sofia".into(),
            },
            end: WriteTime::Zoned {
                dt: jiff::civil::date(2026, 8, 20).at(11, 0, 0, 0),
                tzid: "Europe/Sofia".into(),
            },
            recurrence: Vec::new(),
            recurrence_id: None,
            alarms: vec![("popup".into(), 10)],
            sequence: 0,
            conference: None,
        }
    }

    const NOW: i64 = 1_786_352_400_000;

    #[test]
    fn a_new_event_serializes_and_reparses() {
        let now = Timestamp::from_millisecond(NOW).unwrap();
        let ics = new_event_ics(&write_sample("w1"), now);
        let evs = events_in(&parse(&ics).unwrap());
        assert_eq!(evs.len(), 1);
        let e = &evs[0];
        assert_eq!(e.uid, "w1");
        assert_eq!(e.location.as_deref(), Some("Room 2; annex"), "escaping round-trips");
        assert_eq!(e.alarms, vec![("popup".to_string(), 10)]);
        let (start_ms, tz, all_day) = resolve(&e.start, "UTC").unwrap();
        assert_eq!(tz, "Europe/Sofia");
        assert!(!all_day);
        let z = Timestamp::from_millisecond(start_ms).unwrap().to_zoned(jiff::tz::TimeZone::UTC);
        assert_eq!((z.hour(), z.minute()), (7, 0), "10:00 Sofia in August is 07:00Z");
    }

    const SERIES_RAW: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-CALNAME:kept\r\nBEGIN:VEVENT\r\nUID:s1\r\nSUMMARY:Standup\r\nDTSTART;TZID=Europe/Sofia:20260817T091500\r\nDTEND;TZID=Europe/Sofia:20260817T093000\r\nRRULE:FREQ=DAILY;COUNT=10\r\nX-PRIVATE:survives\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:s1\r\nRECURRENCE-ID;TZID=Europe/Sofia:20260819T091500\r\nSUMMARY:Standup (moved)\r\nDTSTART;TZID=Europe/Sofia:20260819T140000\r\nDTEND;TZID=Europe/Sofia:20260819T141500\r\nEND:VEVENT\r\nEND:VCALENDAR";

    #[test]
    fn rewriting_a_master_keeps_unknown_lines_and_its_exception() {
        let now = Timestamp::from_millisecond(NOW).unwrap();
        let mut ev = write_sample("s1");
        ev.summary = Some("Standup, renamed".into());
        ev.start = WriteTime::Zoned {
            dt: jiff::civil::date(2026, 8, 17).at(9, 15, 0, 0),
            tzid: "Europe/Sofia".into(),
        };
        ev.end = WriteTime::Zoned {
            dt: jiff::civil::date(2026, 8, 17).at(9, 30, 0, 0),
            tzid: "Europe/Sofia".into(),
        };
        ev.recurrence = vec!["RRULE:FREQ=DAILY;COUNT=10".into()];
        ev.sequence = 1;

        let out = rewrite_master(SERIES_RAW, "s1", &ev, now, false).unwrap();
        assert!(out.contains("X-PRIVATE:survives"), "unknown line survives");
        assert!(out.contains("X-WR-CALNAME:kept"), "calendar-level line survives");
        assert!(out.contains("SUMMARY:Standup\\, renamed"));
        assert!(out.contains("SUMMARY:Standup (moved)"), "the exception is untouched");
        assert_eq!(out.matches("BEGIN:VEVENT").count(), 2);

        let evs = events_in(&parse(&out).unwrap());
        assert_eq!(evs[0].summary.as_deref(), Some("Standup, renamed"));
        assert_eq!(evs[0].sequence, 1);
    }

    #[test]
    fn a_time_change_on_the_series_drops_stale_exceptions() {
        let now = Timestamp::from_millisecond(NOW).unwrap();
        let mut ev = write_sample("s1");
        ev.recurrence = vec!["RRULE:FREQ=DAILY;COUNT=10".into()];
        let out = rewrite_master(SERIES_RAW, "s1", &ev, now, true).unwrap();
        assert_eq!(out.matches("BEGIN:VEVENT").count(), 1, "the moved occurrence dies");
        assert!(!out.contains("RECURRENCE-ID"));
    }

    #[test]
    fn editing_one_occurrence_upserts_its_exception() {
        let now = Timestamp::from_millisecond(NOW).unwrap();
        let mut ev = write_sample("s1");
        ev.summary = Some("One-off change".into());
        ev.recurrence_id = Some(WriteTime::Zoned {
            dt: jiff::civil::date(2026, 8, 18).at(9, 15, 0, 0),
            tzid: "Europe/Sofia".into(),
        });
        let out = upsert_exception(SERIES_RAW, &ev, now).unwrap();
        assert_eq!(out.matches("BEGIN:VEVENT").count(), 3, "a new exception joined");
        assert!(out.contains("SUMMARY:One-off change"));
        assert!(out.contains("SUMMARY:Standup (moved)"), "the other exception stays");

        // Editing the SAME occurrence again replaces, not duplicates.
        ev.summary = Some("Changed twice".into());
        let out2 = upsert_exception(&out, &ev, now).unwrap();
        assert_eq!(out2.matches("BEGIN:VEVENT").count(), 3);
        assert!(out2.contains("SUMMARY:Changed twice"));
        assert!(!out2.contains("SUMMARY:One-off change"));
    }

    #[test]
    fn excluding_an_occurrence_exdates_it_and_reaps_its_exception() {
        let cut = WriteTime::Zoned {
            dt: jiff::civil::date(2026, 8, 19).at(9, 15, 0, 0),
            tzid: "Europe/Sofia".into(),
        };
        let out = exclude_occurrence(SERIES_RAW, "s1", &cut).unwrap();
        assert!(out.contains("EXDATE;TZID=Europe/Sofia:20260819T091500"));
        assert_eq!(out.matches("BEGIN:VEVENT").count(), 1, "its exception died with it");
        // The EXDATE landed inside the master block, before its END:VEVENT.
        let master_end = out.find("END:VEVENT").unwrap();
        assert!(out.find("EXDATE").unwrap() < master_end);
    }

    #[test]
    fn truncating_a_series_sets_until_and_reaps_later_exceptions() {
        let cut = WriteTime::Zoned {
            dt: jiff::civil::date(2026, 8, 19).at(9, 15, 0, 0),
            tzid: "Europe/Sofia".into(),
        };
        let out = truncate_series(SERIES_RAW, "s1", "20260819T061459Z", &cut).unwrap();
        assert!(out.contains("RRULE:FREQ=DAILY;UNTIL=20260819T061459Z"), "COUNT replaced");
        assert_eq!(out.matches("BEGIN:VEVENT").count(), 1, "the moved (later) occurrence died");
    }

    #[test]
    fn a_new_todo_serializes_and_reparses() {
        let now = Timestamp::from_millisecond(1_786_352_400_000).unwrap();
        let due = IcsTime::Date(Date::new(2026, 8, 20).unwrap());
        let ics = new_todo_ics("new-1", "Fix; the, thing", Some(&due), None, now);
        let t = &todos_in(&parse(&ics).unwrap())[0];
        assert_eq!(t.uid, "new-1");
        assert_eq!(t.summary.as_deref(), Some("Fix; the, thing"), "escaping round-trips");
        assert!(matches!(t.due, Some(IcsTime::Date(_))));
    }

    #[test]
    fn a_new_todo_carries_the_priority_it_was_given() {
        let now = Timestamp::from_millisecond(1_786_352_400_000).unwrap();
        let ics = new_todo_ics("p-1", "File the return", None, Some(TaskPriority::High.wire()), now);
        assert!(ics.contains("PRIORITY:1"), "{ics}");
        assert_eq!(todos_in(&parse(&ics).unwrap())[0].priority, 1);

        let none = new_todo_ics("p-2", "Someday", None, None, now);
        assert!(!none.contains("PRIORITY"), "no priority writes no line: {none}");
    }

    #[test]
    fn an_edit_sets_clears_and_preserves_priority() {
        let now: Timestamp = "2026-09-07T09:00:00Z".parse().unwrap();
        let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t-p\r\nSUMMARY:Task\r\n\
                   PRIORITY:5\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let edit = |priority| TodoEdit { summary: "Task", due: None, description: None, priority };

        // A deliberate choice rewrites it, without doubling the line.
        let set = patch_todo_fields(raw, "t-p", &edit(Some(TaskPriority::Low.wire())), "UTC", now).unwrap();
        assert!(set.contains("PRIORITY:9"), "{set}");
        assert_eq!(set.matches("PRIORITY").count(), 1, "never doubled: {set}");

        // Clearing removes the line entirely.
        let cleared = patch_todo_fields(raw, "t-p", &edit(None), "UTC", now).unwrap();
        assert!(!cleared.contains("PRIORITY"), "{cleared}");

        // A value the caller passes through is kept as-is: a server's
        // non-canonical 7 is not normalised to the 9 that no-one asked for.
        let raw7 = raw.replace("PRIORITY:5", "PRIORITY:7");
        let kept = patch_todo_fields(&raw7, "t-p", &edit(Some(7)), "UTC", now).unwrap();
        assert!(kept.contains("PRIORITY:7"), "{kept}");
    }

    #[test]
    fn task_priority_reads_the_rfc_bands() {
        assert_eq!(TaskPriority::from_wire(0), None);
        assert_eq!(TaskPriority::from_wire(1), Some(TaskPriority::High));
        assert_eq!(TaskPriority::from_wire(4), Some(TaskPriority::High));
        assert_eq!(TaskPriority::from_wire(5), Some(TaskPriority::Medium));
        assert_eq!(TaskPriority::from_wire(6), Some(TaskPriority::Low));
        assert_eq!(TaskPriority::from_wire(9), Some(TaskPriority::Low));
        assert_eq!(TaskPriority::from_wire(10), None);
        assert_eq!(TaskPriority::High.wire(), 1);
        assert_eq!(TaskPriority::Medium.wire(), 5);
        assert_eq!(TaskPriority::Low.wire(), 9);
    }

    /// A task created with a time, not just a date, needs the same mirrored
    /// `DTSTART` a later edit gets — a brand-new task is exactly as likely
    /// to have its time silently dropped by iCloud as an edited one.
    #[test]
    fn a_new_todo_with_a_time_gets_a_matching_dtstart() {
        let now = Timestamp::from_millisecond(1_786_352_400_000).unwrap();
        let due = IcsTime::Zoned {
            dt: jiff::civil::date(2026, 9, 11).at(15, 30, 0, 0),
            tzid: "Europe/Sofia".to_string(),
        };
        let ics = new_todo_ics("new-2", "Call", Some(&due), None, now);
        assert!(ics.contains("DUE;TZID=Europe/Sofia:20260911T153000"), "{ics}");
        assert!(ics.contains("DTSTART;TZID=Europe/Sofia:20260911T153000"), "{ics}");

        let t = &todos_in(&parse(&ics).unwrap())[0];
        let (ms, _, all_day) = resolve(t.due.as_ref().unwrap(), "Europe/Sofia").unwrap();
        assert!(!all_day, "a timed due must not read back as a date");
        assert_eq!(
            ms,
            "2026-09-11T15:30:00+03:00[Europe/Sofia]".parse::<jiff::Zoned>().unwrap().timestamp().as_millisecond()
        );
    }

    /// A task due at a specific time needs a `VALARM` of its own: unlike
    /// Reminders on the phone, which stamps one onto every timed reminder it
    /// creates, omacal wrote a bare `DUE` and nothing fired anywhere —
    /// neither the phone (nothing in the resource to alert on) nor Omarchy.
    #[test]
    fn a_new_todo_with_a_time_gets_a_valarm_at_its_due() {
        let now = Timestamp::from_millisecond(1_786_352_400_000).unwrap();
        let due = IcsTime::Zoned {
            dt: jiff::civil::date(2026, 9, 11).at(15, 30, 0, 0),
            tzid: "Europe/Sofia".to_string(),
        };
        let ics = new_todo_ics("new-3", "Call", Some(&due), None, now);
        assert!(ics.contains("BEGIN:VALARM"), "{ics}");
        assert!(ics.contains("ACTION:DISPLAY"), "{ics}");
        assert!(ics.contains("TRIGGER:PT0M"), "{ics}");
        assert!(ics.contains("END:VALARM"), "{ics}");
    }

    /// A bare-date due has no time to alarm at — an all-day task must not
    /// grow a `VALARM` it can't honor.
    #[test]
    fn a_new_todo_with_only_a_date_gets_no_valarm() {
        let now = Timestamp::from_millisecond(1_786_352_400_000).unwrap();
        let due = IcsTime::Date(Date::new(2026, 8, 20).unwrap());
        let ics = new_todo_ics("new-4", "Fix the thing", Some(&due), None, now);
        assert!(!ics.contains("VALARM"), "{ics}");
    }

    /// An invitation resource the way iCloud actually ships one: the user's
    /// own ATTENDEE folded mid-mailbox, a quoted CN carrying the `;` and `:`
    /// both rewrites split on, the scheme in caps, and a second guest whose
    /// answer must survive anything done to ours.
    const INVITE_RAW: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:inv-1\r\nSUMMARY:Quarterly review\r\nDTSTART;TZID=Europe/Sofia:20260901T100000\r\nDTEND;TZID=Europe/Sofia:20260901T110000\r\nORGANIZER;CN=Boss:mailto:boss@x.com\r\nATTENDEE;CN=\"Petkov; who: asks\";RSVP=TRUE;PARTSTAT=NEEDS-ACTION:MAILTO\r\n :P@X.COM\r\nATTENDEE;CN=Ana;PARTSTAT=ACCEPTED:mailto:ana@x.com\r\nEND:VEVENT\r\nEND:VCALENDAR";

    /// The whole rewrite in one place: the fold is crossed, the quoted CN
    /// and the RSVP param survive byte-for-byte, the old PARTSTAT is
    /// replaced rather than joined by a second one, the other guest's answer
    /// and the ORGANIZER line are untouched — and the result still parses,
    /// with the store-side vocabulary agreeing.
    #[test]
    fn answering_rewrites_your_partstat_and_nothing_else() {
        let out = respond_all(INVITE_RAW, "inv-1", "p@x.com", "ACCEPTED").unwrap();
        assert!(
            out.contains("ATTENDEE;CN=\"Petkov; who: asks\";RSVP=TRUE;PARTSTAT=ACCEPTED:MAILTO:P@X.COM"),
            "params kept, PARTSTAT swapped, folded mailbox rejoined: {out}"
        );
        assert_eq!(out.matches("PARTSTAT=NEEDS-ACTION").count(), 0, "the old answer is gone");
        assert!(out.contains("ATTENDEE;CN=Ana;PARTSTAT=ACCEPTED:mailto:ana@x.com"));
        assert!(out.contains("ORGANIZER;CN=Boss:mailto:boss@x.com"));

        let ev = &events_in(&parse(&out).unwrap())[0];
        let mine = ev.attendees.iter().find(|a| a.email.eq_ignore_ascii_case("p@x.com")).unwrap();
        assert_eq!(mine.response_status, "accepted");
    }

    /// The guard: a resource that does not carry your invitation cannot be
    /// answered. Inventing an ATTENDEE line would fabricate an invitation
    /// the organizer never sent — `None`, not a quiet no-op PUT.
    #[test]
    fn a_resource_without_your_invitation_refuses_the_answer() {
        assert!(respond_all(INVITE_RAW, "inv-1", "stranger@x.com", "ACCEPTED").is_none());
        // The organizer's own mailbox is on ORGANIZER, not ATTENDEE — being
        // the organizer is not an invitation either.
        assert!(respond_all(INVITE_RAW, "inv-1", "boss@x.com", "ACCEPTED").is_none());
        // A matching attendee on a *different* uid in the file helps nothing.
        assert!(respond_all(INVITE_RAW, "other-uid", "p@x.com", "ACCEPTED").is_none());
    }

    /// A series answered whole: the exception carries its own ATTENDEE line,
    /// and "all of them" that skipped it would leave the moved occurrence
    /// still saying needs-action.
    const SERIES_INVITE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:s2\r\nSUMMARY:Standup\r\nDTSTART;TZID=Europe/Sofia:20260817T091500\r\nDTEND;TZID=Europe/Sofia:20260817T093000\r\nRRULE:FREQ=DAILY;COUNT=10\r\nX-PRIVATE:survives\r\nATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:p@x.com\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:s2\r\nRECURRENCE-ID;TZID=Europe/Sofia:20260819T091500\r\nSUMMARY:Standup (moved)\r\nDTSTART;TZID=Europe/Sofia:20260819T140000\r\nDTEND;TZID=Europe/Sofia:20260819T141500\r\nATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:p@x.com\r\nEND:VEVENT\r\nEND:VCALENDAR";

    #[test]
    fn answering_all_reaches_the_exception_too() {
        let out = respond_all(SERIES_INVITE, "s2", "p@x.com", "DECLINED").unwrap();
        assert_eq!(out.matches("PARTSTAT=DECLINED").count(), 2, "master and exception both");
        assert_eq!(out.matches("PARTSTAT=NEEDS-ACTION").count(), 0);
    }

    /// Scope "this" on an occurrence that already has its exception: that
    /// one component is answered, and the master's own line — the whole
    /// series' answer — stays exactly as it was.
    #[test]
    fn answering_one_materialised_occurrence_leaves_the_series_alone() {
        let rid = WriteTime::Zoned {
            dt: jiff::civil::date(2026, 8, 19).at(9, 15, 0, 0),
            tzid: "Europe/Sofia".into(),
        };
        let end = WriteTime::Zoned {
            dt: jiff::civil::date(2026, 8, 19).at(9, 30, 0, 0),
            tzid: "Europe/Sofia".into(),
        };
        let now = Timestamp::from_millisecond(NOW).unwrap();
        let out =
            respond_occurrence(SERIES_INVITE, "s2", "p@x.com", "ACCEPTED", &rid, &end, now)
                .unwrap();
        assert_eq!(out.matches("PARTSTAT=ACCEPTED").count(), 1, "one occurrence, one answer");
        assert_eq!(out.matches("PARTSTAT=NEEDS-ACTION").count(), 1, "the master still asks");
        assert_eq!(out.matches("BEGIN:VEVENT").count(), 2, "no third component appeared");
        let accepted_at = out.find("PARTSTAT=ACCEPTED").unwrap();
        assert!(
            accepted_at > out.find("RECURRENCE-ID").unwrap(),
            "the answer landed in the exception, not the master"
        );
    }

    /// Scope "this" on an occurrence with no exception yet: one is cloned
    /// from the master — unmodeled lines verbatim, the series machinery and
    /// the master's own times dropped for the occurrence's — and the answer
    /// lives only there.
    #[test]
    fn answering_an_unmaterialised_occurrence_clones_the_master() {
        let rid = WriteTime::Zoned {
            dt: jiff::civil::date(2026, 8, 18).at(9, 15, 0, 0),
            tzid: "Europe/Sofia".into(),
        };
        let end = WriteTime::Zoned {
            dt: jiff::civil::date(2026, 8, 18).at(9, 30, 0, 0),
            tzid: "Europe/Sofia".into(),
        };
        let now = Timestamp::from_millisecond(NOW).unwrap();
        let out =
            respond_occurrence(SERIES_INVITE, "s2", "p@x.com", "TENTATIVE", &rid, &end, now)
                .unwrap();
        assert_eq!(out.matches("BEGIN:VEVENT").count(), 3, "a fresh exception appeared");
        assert_eq!(out.matches("RRULE").count(), 1, "the clone did not become a second series");
        assert_eq!(out.matches("X-PRIVATE:survives").count(), 2, "unmodeled lines cloned whole");
        assert!(out.contains("RECURRENCE-ID;TZID=Europe/Sofia:20260818T091500"));
        assert_eq!(out.matches("PARTSTAT=TENTATIVE").count(), 1);

        // And the round trip agrees: three components, exactly one of which
        // answers tentative, anchored on the cloned occurrence.
        let evs = events_in(&parse(&out).unwrap());
        assert_eq!(evs.len(), 3);
        let cloned = evs
            .iter()
            .find(|e| e.attendees.iter().any(|a| a.response_status == "tentative"))
            .unwrap();
        let (ms, _, _) = resolve(&cloned.start, "UTC").unwrap();
        let (rid_ms, _, _) =
            resolve(cloned.recurrence_id.as_ref().unwrap(), "UTC").unwrap();
        assert_eq!(ms, rid_ms, "the clone's DTSTART is the occurrence it overrides");
    }

    /// #137: the desktop speaks when the task's own alarm does. A reminder
    /// made on the phone carries its lead; one OmaCal wrote carries `PT0M`,
    /// the due itself; and a trigger this cannot honour reads as none.
    #[test]
    fn a_tasks_own_alarm_says_how_long_before_its_due_it_speaks() {
        let with = |alarm: &str| {
            format!(
                "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t-1\r\nSUMMARY:Call\r\n\
                 DUE;TZID=Europe/Sofia:20260918T155500\r\n{alarm}END:VTODO\r\nEND:VCALENDAR\r\n"
            )
        };
        let alarm = |trigger: &str| {
            format!("BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Reminder\r\nTRIGGER{trigger}\r\nEND:VALARM\r\n")
        };

        assert_eq!(todo_alarm_lead_minutes(&with(""), "t-1"), None, "no alarm, no lead");
        assert_eq!(todo_alarm_lead_minutes(&with(&alarm(":PT0M")), "t-1"), Some(0), "OmaCal's own");
        assert_eq!(todo_alarm_lead_minutes(&with(&alarm(":-PT15M")), "t-1"), Some(15), "Reminders'");
        assert_eq!(todo_alarm_lead_minutes(&with(&alarm(":-P1D")), "t-1"), Some(24 * 60));
        assert_eq!(todo_alarm_lead_minutes(&with(&alarm(":PT30M")), "t-1"), Some(-30), "after the due");
        assert_eq!(
            todo_alarm_lead_minutes(&with(&alarm(";VALUE=DATE-TIME:20260918T150000Z")), "t-1"),
            None,
            "an instant is not a lead",
        );
        assert_eq!(
            todo_alarm_lead_minutes(&with(&alarm(";RELATED=END:-PT15M")), "t-1"),
            None,
            "a VTODO OmaCal writes has no end to hang on",
        );
        // Several: the earliest speaks.
        let two = with(&format!("{}{}", alarm(":-PT5M"), alarm(":-PT45M")));
        assert_eq!(todo_alarm_lead_minutes(&two, "t-1"), Some(45));
        // Another task's alarm is not this one's.
        assert_eq!(todo_alarm_lead_minutes(&with(&alarm(":-PT15M")), "t-2"), None);
    }

    /// #139's follow-up: taking a task's time away leaves its at-due alarm
    /// pointing at a `DTSTART` that went with the hour, and the phone fired at
    /// whatever the server made of it. A relative alarm goes with the time; an
    /// absolute one names its own instant and stays.
    #[test]
    fn clearing_a_tasks_time_takes_the_alarm_that_hung_on_it() {
        let timed = |alarm: &str| {
            format!(
                "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t-9\r\nSUMMARY:Call\r\n\
                 DTSTART;TZID=Europe/Sofia:20260918T155500\r\n\
                 DUE;TZID=Europe/Sofia:20260918T155500\r\n{alarm}END:VTODO\r\nEND:VCALENDAR\r\n"
            )
        };
        let relative = "BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Reminder\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n";
        let absolute = "BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Reminder\r\nTRIGGER;VALUE=DATE-TIME:20260918T120000Z\r\nEND:VALARM\r\n";
        let now: Timestamp = "2026-09-18T09:00:00Z".parse().unwrap();
        let to_date = TodoEdit {
            summary: "Call",
            due: Some(TodoDue::Date(Date::new(2026, 9, 19).unwrap())),
            description: None,
            priority: None,
        };

        let out = patch_todo_fields(&timed(relative), "t-9", &to_date, "Europe/Sofia", now).unwrap();
        assert!(!out.contains("VALARM"), "nothing left for it to hang on: {out}");
        assert!(out.contains("DUE;VALUE=DATE:20260919"), "{out}");

        let out = patch_todo_fields(&timed(absolute), "t-9", &to_date, "Europe/Sofia", now).unwrap();
        assert!(out.contains("TRIGGER;VALUE=DATE-TIME:20260918T120000Z"), "the user's own: {out}");

        // Clearing the due entirely does the same.
        let cleared = TodoEdit { summary: "Call", due: None, description: None, priority: None };
        let out = patch_todo_fields(&timed(relative), "t-9", &cleared, "Europe/Sofia", now).unwrap();
        assert!(!out.contains("VALARM"), "{out}");

        // And a task that keeps its time keeps its alarm, exactly one.
        let keep = TodoEdit {
            summary: "Call",
            due: Some(TodoDue::At("2026-09-18T12:55:00Z".parse().unwrap())),
            description: None,
            priority: None,
        };
        let out = patch_todo_fields(&timed(relative), "t-9", &keep, "Europe/Sofia", now).unwrap();
        assert_eq!(out.matches("BEGIN:VALARM").count(), 1);
        assert!(out.contains("TRIGGER:-PT15M"), "the user's lead, not ours: {out}");
    }
}
