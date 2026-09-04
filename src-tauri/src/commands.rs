use omacal_core::{expand, lay_out_day, pack_lanes, Interval, Lane, Placed, Segment, Series};

/// How many rows of all-day spans the week's payload *positions*.
///
/// **Not the band's height.** The band draws four rows and folds the rest
/// behind "+N more", which expands — and that is a UI decision, made in
/// `weekwindow.ts` against the days actually on screen. Reported
/// 2026-09-04 (Michael Brennan, by email, on 1.2.0): a week with many
/// all-day events showed "+70 more" and clicking it did nothing. It could
/// not do anything: the events were in the payload, but everything past the
/// fourth row had been dropped by the packer here, so they had no columns
/// to be drawn at.
///
/// Generous rather than unbounded, because the packer is O(rows) per
/// segment and the input is somebody else's calendar; anything past this is
/// still reported in `overflow`, which the UI adds to its own count.
const ALL_DAY_LANES_MAX: u8 = 64;
use omacal_store::StoredEvent;
use serde::Serialize;
use std::collections::HashSet;

const DAY_MS: i64 = 24 * 3_600_000;
/// Used when a calendar carries no colour of its own — Google omits
/// `backgroundColor` on some calendars, and a missing colour must not be a
/// missing event.
const DEFAULT_EVENT_COLOR: &str = "#5b8def";
/// Expansion guard for one week of any single series. Sized for the realistic
/// worst case — a 30-minute block recurring through every working hour is ~336
/// occurrences a week — so that `Expansion::truncated` stays false in practice.
const EXPAND_LIMIT: u16 = 512;

#[derive(Debug, Clone, Serialize)]
pub struct UiEvent {
    pub id: i64,
    pub title: String,
    pub location: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub color: String,
    /// `accepted` | `needsAction` | `tentative` | `declined`
    pub response: String,
    pub is_all_day: bool,
    /// Invitees on the event, the organizer's row included — the same count
    /// the widget's feed publishes. `0` means a solo event, not unknown.
    pub attendees: u32,
    /// Part of a series: the master's own expansion, or an exception row
    /// that overrides one occurrence of it. Either way the row on screen is
    /// one instance of something that repeats.
    pub recurring: bool,
    /// Google's structured conference link, when the event carries one. The
    /// list row's Join reads this first and falls back to a recognised
    /// meeting URL in `location` — the popover's exact derivation.
    pub conference: Option<String>,
    /// **Every invitee other than you has said no**, and there was at least
    /// one to say it. A meeting nobody is coming to, which until now was
    /// visible only by opening the event and reading the guest list
    /// (2026-09-04, Plamen: a 1:1 he organised, declined, looked exactly
    /// like one that was going ahead).
    ///
    /// Deliberately not "the organizer's guests declined": whether you own
    /// the event or were invited to it, an event whose every other invitee
    /// has declined is not happening, and `to_ui` has no account email to
    /// decide ownership with anyway (`events::is_organizer` needs one). The
    /// signed-in user's own row is excluded through `is_self`, so your own
    /// "no" — which already strikes the block through — never sets this.
    pub all_guests_declined: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DayColumn {
    pub start_ms: i64,
    /// Midnight on the *next* day, in the display zone. Carried explicitly so
    /// the UI can draw against the column's true span: a DST day is 23 or 25
    /// hours long, and `start_ms + 24h` puts every hour rule an hour out.
    pub end_ms: i64,
    pub events: Vec<UiEvent>,
    pub placed: Vec<Placed>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WeekPayload {
    pub days: Vec<DayColumn>,
    pub all_day: Vec<Lane>,
    pub all_day_events: Vec<UiEvent>,
    pub overflow: Vec<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonthCell {
    pub start_ms: i64,
    pub end_ms: i64,
    /// False for the leading and trailing days that belong to a neighbouring
    /// month. Drawn dimmed rather than blank so the grid stays rectangular.
    pub in_month: bool,
    /// Timed events for this day, sorted by start. The UI decides how many
    /// fit and renders `+N more` from what it drops — cell height is a
    /// layout question and the backend has no business guessing it.
    pub timed: Vec<UiEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonthRow {
    pub cells: Vec<MonthCell>, // always 7
    /// Multi-day and all-day events spanning this row, lane-packed at
    /// row_len 7. Indices point into `bar_events`.
    pub bars: Vec<Lane>,
    pub bar_events: Vec<UiEvent>,
    pub bar_overflow: Vec<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonthPayload {
    pub rows: Vec<MonthRow>, // always 6
    pub year: i32,
    pub month: u32, // 1-12
}

fn to_ui(src: &StoredEvent, start_ms: i64, end_ms: i64) -> UiEvent {
    UiEvent {
        id: src.id,
        title: src.summary.clone().unwrap_or_else(|| "(no title)".into()),
        location: src.location.clone(),
        start_ms,
        end_ms,
        color: src
            .color_hex
            .clone()
            .unwrap_or_else(|| DEFAULT_EVENT_COLOR.into()),
        response: src.self_response.clone().unwrap_or_else(|| "accepted".into()),
        is_all_day: src.is_all_day,
        attendees: src.attendees.len() as u32,
        recurring: src.recurrence.is_some() || src.recurring_event_id.is_some(),
        conference: src.conference_uri.clone(),
        all_guests_declined: {
            let mut others = src.attendees.iter().filter(|a| !a.is_self).peekable();
            others.peek().is_some() && others.all(|a| a.response_status == "declined")
        },
    }
}

/// The occurrences that an exception has taken over from its master.
///
/// An exception is stored as a standalone row carrying the id of the series it
/// overrides and the instant the overridden occurrence started at. Both a moved
/// instance and a deleted one produce such a row, and in both cases the master
/// must stop expanding into that slot — otherwise a moved instance renders
/// twice (once at its new time, once as a ghost at the old one) and a deleted
/// one never disappears at all.
pub(crate) fn suppressed_slots(events: &[StoredEvent]) -> HashSet<(i64, &str, i64)> {
    events
        .iter()
        .filter_map(|e| {
            let master = e.recurring_event_id.as_deref()?;
            Some((e.calendar_id, master, e.original_start_utc?))
        })
        .collect()
}

/// Expands one stored row into the concrete occurrences overlapping the window.
pub(crate) fn occurrences(src: &StoredEvent, from_ms: i64, to_ms: i64) -> Vec<Interval> {
    occurrences_limited(src, from_ms, to_ms, EXPAND_LIMIT)
}

/// [`occurrences`] with the expansion cap named by the caller.
///
/// Every grid asks for a week, a month or a year and `EXPAND_LIMIT` is sized
/// for those. Search's fallback asks for *everything since a series began*, to
/// find the last occurrence of one that ended years ago, and needs a larger
/// bound — one it states rather than inherits.
pub(crate) fn occurrences_limited(
    src: &StoredEvent,
    from_ms: i64,
    to_ms: i64,
    limit: u16,
) -> Vec<Interval> {
    let Some(rule) = &src.recurrence else {
        let iv = Interval { start_ms: src.start_utc, end_ms: src.end_utc };
        return if iv.start_ms < to_ms && iv.end_ms > from_ms { vec![iv] } else { vec![] };
    };
    let lines: Vec<String> = rule.lines().map(|s| s.to_string()).collect();
    let series = Series {
        dtstart_ms: src.start_utc,
        dtstart_tz: &src.start_tz,
        duration_ms: src.end_utc - src.start_utc,
        is_all_day: src.is_all_day,
        recurrence: &lines,
    };
    match expand(&series, from_ms, to_ms, limit) {
        Ok(e) => {
            if e.truncated {
                // Surfaced rather than swallowed: a series this dense means the
                // window is showing an incomplete picture.
                tracing::warn!(
                    google_id = %src.google_id,
                    limit,
                    "recurrence expansion truncated"
                );
            }
            e.intervals
        }
        Err(err) => {
            tracing::warn!(google_id = %src.google_id, %err, "recurrence expansion failed");
            Vec::new()
        }
    }
}

/// The `n + 1` local-midnight boundaries starting at `start_ms`, computed
/// **in `tz`**.
///
/// `n + 1` and not `n`: every consumer needs the *end* of the last day, and a
/// DST day is not 24 hours, so it cannot be derived by addition. Never
/// `start_ms + n * DAY_MS` either: on a DST transition day that arithmetic is
/// off by an hour, which both misplaces events and squashes or stretches the
/// day's geometry. `Zoned::checked_add(1.day())` does calendar-day arithmetic,
/// so a 23- or 25-hour day comes out at its true length.
///
/// Falls back to fixed 24-hour days only if the zone is unknown — the grid must
/// still render.
fn n_day_boundaries(start_ms: i64, n: usize, tz: &str) -> Vec<i64> {
    use jiff::{Timestamp, ToSpan};

    let fallback = || (0..=n as i64).map(|i| start_ms + i * DAY_MS).collect::<Vec<_>>();

    let Ok(start) = Timestamp::from_millisecond(start_ms) else {
        return fallback();
    };
    let Ok(mut z) = start.in_tz(tz) else {
        return fallback();
    };

    let mut out = Vec::with_capacity(n + 1);
    out.push(z.timestamp().as_millisecond());
    for _ in 0..n {
        match z.checked_add(1.day()) {
            Ok(next) => {
                z = next;
                out.push(z.timestamp().as_millisecond());
            }
            Err(_) => {
                let last = *out.last().unwrap();
                out.push(last + DAY_MS);
            }
        }
    }
    out
}

/// The day boundary `days` days away from `start_ms` — negative for earlier —
/// walked in `tz` so a DST day counts as its real 23 or 25 hours, the same
/// way `n_day_boundaries` walks forward. The padded window the views fetch
/// (2026-09-03) begins `pad` days before the day the user is looking at,
/// which is the one place this walks backwards. Same fallback as its
/// sibling: an unknown zone still yields a boundary.
pub fn day_start_shifted(start_ms: i64, days: i64, tz: &str) -> i64 {
    use jiff::{Timestamp, ToSpan};
    Timestamp::from_millisecond(start_ms)
        .ok()
        .and_then(|t| t.in_tz(tz).ok())
        .and_then(|z| z.checked_add(days.days()).ok())
        .map(|z| z.timestamp().as_millisecond())
        .unwrap_or(start_ms + days * DAY_MS)
}

/// Local midnight for a civil date, in `tz`, as epoch milliseconds. Falls
/// back to UTC if the zone is unknown — the grid must still render, the same
/// fallback philosophy as `n_day_boundaries`.
fn local_midnight_ms(d: jiff::civil::Date, tz: &str) -> i64 {
    let midnight = d.at(0, 0, 0, 0);
    match midnight.in_tz(tz) {
        Ok(z) => z.timestamp().as_millisecond(),
        Err(_) => midnight
            .to_zoned(jiff::tz::TimeZone::UTC)
            .expect("UTC is always a valid zone")
            .timestamp()
            .as_millisecond(),
    }
}

/// The first day of the week on or before `year`-`month`-01, at local
/// midnight in `tz` — the anchor for the 42-cell month grid.
///
/// `pub(crate)`: `get_month` needs this same anchor to size its fetch window
/// *before* calling `assemble_month`, which recomputes it internally — so
/// both must be handed the same `week_start`, or the window and the grid
/// disagree by up to six days.
pub(crate) fn month_grid_start_ms(
    year: i32,
    month: u32,
    tz: &str,
    week_start: crate::settings::WeekStart,
) -> i64 {
    use jiff::civil::date;

    let mut grid_start = date(year as i16, month as i8, 1);
    while grid_start.weekday() != week_start.weekday() {
        grid_start = grid_start.yesterday().expect("civil date underflow");
    }
    local_midnight_ms(grid_start, tz)
}

/// Column `0..bounds.len() - 1` for an instant inside the window, or `None`
/// outside it.
fn column_for(bounds: &[i64], ms: i64) -> Option<usize> {
    if ms < bounds[0] || ms >= *bounds.last().unwrap() {
        return None;
    }
    Some(bounds.partition_point(|&b| b <= ms) - 1)
}

/// The column a timed occurrence should be drawn in, or `None` if it does not
/// touch the week at all.
///
/// Normally that is the column containing its start. An event that began before
/// the week and runs into it — Sunday 23:00 to Monday 01:00, on the Monday the
/// week begins — has no such column, and dropping it made the event vanish
/// entirely. It is clamped into column 0 instead, where `lay_out_day` clips the
/// geometry to the column's own bounds.
fn timed_column(bounds: &[i64], iv: &Interval) -> Option<usize> {
    column_for(bounds, iv.start_ms)
        .or_else(|| (iv.start_ms < bounds[0] && iv.end_ms > bounds[0]).then_some(0))
}

/// Each column's own civil date in the **display** zone, `yyyy-mm-dd` — the
/// left-hand side of every all-day comparison. One entry per column, so
/// `bounds` contributes its first `n` entries and not its trailing end.
fn column_dates(bounds: &[i64], tz: &str) -> Vec<String> {
    bounds[..bounds.len() - 1]
        .iter()
        .map(|&ms| crate::write::date_in_zone(ms, tz))
        .collect()
}

/// The column a *date* belongs in: the one whose own date is the same string.
///
/// This is the whole of all-day placement, and it is a string comparison
/// deliberately. An all-day event has a date, not an instant — the store holds
/// midnight in the **calendar's** zone, which for a foreign calendar is a
/// different instant from midnight in the display zone and often a different
/// day. Bucketing that instant against display-zone boundaries drew a
/// Pacific/Auckland event under the previous day of a Europe/Sofia week, while
/// the event's own popover and edit form named the right one. Comparing
/// instants against day boundaries is what went wrong in three separate defects
/// on this project; comparing a date to a date cannot. This is now the only
/// column rule for an all-day event in any of the four grids — the
/// instant-bucketing `signed_column` it replaced is gone rather than left
/// beside it, so there is nothing to copy by mistake.
///
/// Out-of-window dates return `-1` or `n`. Only the *sign* is read downstream —
/// `pack_lanes` turns it into a continuation flag and clips the value to the row
/// — so those two say everything a caller may act on, and no test pins the
/// magnitude, because a test would be asserting a number nothing consumes. Any
/// caller that starts reading *how far* outside the window a column falls — a
/// continuation marker that counts days, say — needs its own coverage here
/// first. `yyyy-mm-dd` orders lexicographically exactly as dates order, so the
/// comparison needs no parsing.
fn date_column(col_dates: &[String], date: &str) -> i32 {
    if let Some(i) = col_dates.iter().position(|d| d == date) {
        return i as i32;
    }
    match col_dates.first() {
        Some(first) if date < first.as_str() => -1,
        // Past the window — and also the answer for a window with no columns
        // at all, where `pack_lanes` discards every segment and the value is
        // inert.
        _ => col_dates.len() as i32,
    }
}

/// The `Segment` columns for one all-day occurrence: its own two dates, read in
/// its **calendar's** zone, matched against the display zone's column dates.
///
/// The dates come from `write::all_day_span_dates`, the same derivation the
/// popover and the edit form read, so the day a chip is drawn under and the day
/// the event says it is on cannot drift apart.
fn all_day_columns(src: &StoredEvent, iv: &Interval, col_dates: &[String]) -> (i32, i32) {
    let (start_date, last_date) =
        crate::write::all_day_span_dates(iv.start_ms, iv.end_ms, &src.calendar_timezone);
    (date_column(col_dates, &start_date), date_column(col_dates, &last_date))
}

/// Turns stored events into `n` laid-out day columns plus the all-day band.
///
/// `start_ms` is midnight in `tz` on the window's first day; `tz` is the
/// display zone. All day-boundary maths flows through `n_day_boundaries`, so
/// a window containing a DST transition lays out correctly.
///
/// The two kinds of event are bucketed by two different rules, deliberately. A
/// timed event is an instant, so it belongs to the column that contains it in
/// the display zone (`timed_column`). An all-day event is a *date*, so it goes
/// where its own calendar's date matches a column's (`all_day_columns`) — see
/// `date_column` for why anything else misplaces it.
pub fn assemble_days(events: &[StoredEvent], start_ms: i64, n: usize, tz: &str) -> WeekPayload {
    let bounds = n_day_boundaries(start_ms, n, tz);
    let end_ms = bounds[n];
    let col_dates = column_dates(&bounds, tz);

    let mut day_events: Vec<Vec<UiEvent>> = vec![Vec::new(); n];
    let mut all_day_events: Vec<UiEvent> = Vec::new();
    let mut segments: Vec<Segment> = Vec::new();

    let suppressed = suppressed_slots(events);

    for src in events {
        // A cancelled exception exists only to record that an occurrence was
        // deleted. It has already been counted into `suppressed`; it draws
        // nothing itself.
        if src.status == "cancelled" {
            continue;
        }
        for iv in occurrences(src, bounds[0], end_ms) {
            // Only a master can match: the keys are master ids, and an
            // exception never carries its own id as its master.
            if suppressed.contains(&(src.calendar_id, src.google_id.as_str(), iv.start_ms)) {
                continue;
            }
            if src.is_all_day {
                let (start_col, end_col) = all_day_columns(src, &iv, &col_dates);
                segments.push(Segment { idx: all_day_events.len(), start_col, end_col });
                all_day_events.push(to_ui(src, iv.start_ms, iv.end_ms));
            } else if let Some(col) = timed_column(&bounds, &iv) {
                day_events[col].push(to_ui(src, iv.start_ms, iv.end_ms));
            }
        }
    }

    // Every span that can be positioned is positioned; how many rows to
    // *show* is the band's own decision (see `ALL_DAY_LANES_MAX`). It was
    // four here until 2026-09-04, and four is still what the band draws
    // unexpanded — the difference is that the fifth row now exists to be
    // expanded into, rather than being discarded before the UI sees it.
    let (all_day, overflow) = pack_lanes(&segments, n as u16, ALL_DAY_LANES_MAX);

    let days = (0..n)
        .map(|d| {
            let evs = std::mem::take(&mut day_events[d]);
            let intervals: Vec<Interval> = evs
                .iter()
                .map(|e| Interval { start_ms: e.start_ms, end_ms: e.end_ms })
                .collect();
            // The window is the day's *true* length, so a 25-hour day is not
            // compressed into 24 hours' worth of geometry.
            let placed = lay_out_day(&intervals, bounds[d], bounds[d + 1]);
            DayColumn { start_ms: bounds[d], end_ms: bounds[d + 1], events: evs, placed }
        })
        .collect();

    WeekPayload { days, all_day, all_day_events, overflow }
}

/// The week view. A thin wrapper so `assemble_days` has one caller shape and
/// Day view is provably the same engine — see
/// `one_day_and_seven_days_agree_about_the_day_they_share`.
pub fn assemble_week(events: &[StoredEvent], week_start_ms: i64, tz: &str) -> WeekPayload {
    assemble_days(events, week_start_ms, 7, tz)
}

/// Six week-rows of a calendar month, always 42 cells regardless of how many
/// weeks the month actually spans — a grid that changes height as you page
/// through the year is worse than one dimmed row.
///
/// Unlike `assemble_days`, Month needs no time positioning: each row is
/// lane-packed independently at `row_len = 7` for its own spanning bars, and
/// each cell gets a flat, sorted list of the day's timed events for the UI to
/// lay out.
pub fn assemble_month(
    events: &[StoredEvent],
    year: i32,
    month: u32,
    tz: &str,
    week_start: crate::settings::WeekStart,
) -> MonthPayload {
    use jiff::civil::date;

    let grid_start_ms = month_grid_start_ms(year, month, tz, week_start);
    let bounds = n_day_boundaries(grid_start_ms, 42, tz);
    // All 42 at once, then sliced per row — the same relationship `row_bounds`
    // has with `bounds`, and a row's seven column dates are its own seven.
    let grid_dates = column_dates(&bounds, tz);

    let month_start_ms = local_midnight_ms(date(year as i16, month as i8, 1), tz);
    let (next_year, next_month) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    let next_month_start_ms = local_midnight_ms(date(next_year as i16, next_month as i8, 1), tz);

    let suppressed = suppressed_slots(events);

    let rows = (0..6)
        .map(|r| {
            let row_bounds = &bounds[r * 7..=r * 7 + 7];
            let row_dates = &grid_dates[r * 7..r * 7 + 7];
            let row_start = row_bounds[0];
            let row_end = row_bounds[7];

            let mut day_events: Vec<Vec<UiEvent>> = vec![Vec::new(); 7];
            let mut bar_events: Vec<UiEvent> = Vec::new();
            let mut segments: Vec<Segment> = Vec::new();

            for src in events {
                // A cancelled exception exists only to record that an
                // occurrence was deleted; see `assemble_days`.
                if src.status == "cancelled" {
                    continue;
                }
                for iv in occurrences(src, row_start, row_end) {
                    if suppressed.contains(&(src.calendar_id, src.google_id.as_str(), iv.start_ms)) {
                        continue;
                    }
                    if src.is_all_day {
                        let (start_col, end_col) = all_day_columns(src, &iv, row_dates);
                        segments.push(Segment { idx: bar_events.len(), start_col, end_col });
                        bar_events.push(to_ui(src, iv.start_ms, iv.end_ms));
                    } else if let Some(col) = timed_column(row_bounds, &iv) {
                        day_events[col].push(to_ui(src, iv.start_ms, iv.end_ms));
                    }
                }
            }

            // Three lanes, matching the spec's month rows.
            let (bars, bar_overflow) = pack_lanes(&segments, 7, 3);

            let cells = (0..7)
                .map(|c| {
                    let mut timed = std::mem::take(&mut day_events[c]);
                    timed.sort_by_key(|e| e.start_ms);
                    let start_ms = row_bounds[c];
                    MonthCell {
                        start_ms,
                        end_ms: row_bounds[c + 1],
                        in_month: start_ms >= month_start_ms && start_ms < next_month_start_ms,
                        timed,
                    }
                })
                .collect();

            MonthRow { cells, bars, bar_events, bar_overflow }
        })
        .collect();

    MonthPayload { rows, year, month }
}

#[derive(Debug, Clone, Serialize)]
pub struct YearDay {
    pub start_ms: i64,
    pub day: u32, // 1-31
    /// At least one all-day event landed on this day. A timed event does not
    /// dot the year grid — this view answers "what is blocked out", not "how
    /// busy am I".
    pub has_all_day: bool,
    /// Outside the synced window (`synced_window`). Drawn distinctly from an
    /// in-window day with nothing on it — absence of a dot must never be
    /// confused with "nothing is fetched here".
    pub unsynced: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct YearMonth {
    pub month: u32, // 1-12
    /// Leading blank cells before the 1st, so every month's weekday columns
    /// line up. Monday-first, so 0..=6.
    pub lead_blanks: usize,
    pub days: Vec<YearDay>,
}

#[derive(Debug, Clone, Serialize)]
pub struct YearPayload {
    pub year: i32,
    pub months: Vec<YearMonth>,
}

/// Local midnight of `year`-01-01, in `tz`.
///
/// `pub(crate)`: `get_year` calls it twice — once for this year's start and
/// once for next year's — to widen its fetch window a day either side, the
/// same way `get_month` widens its own around `month_grid_start_ms`.
pub(crate) fn year_start_ms(year: i32, tz: &str) -> i64 {
    local_midnight_ms(jiff::civil::date(year as i16, 1, 1), tz)
}

/// The 12-up year grid: every day of `year`, marked with whether it carries
/// an all-day event and whether it falls outside the window the app actually
/// keeps synced (`synced_window`).
///
/// Each month's days come from that month's own local-midnight boundaries
/// (`n_day_boundaries`), exactly as `assemble_month` finds a row's — so a
/// DST transition inside any month still lands every day on its own local
/// midnight rather than sliding by an hour.
///
/// A day is dotted when **its own date falls inside the event's inclusive date
/// span**, both sides read the way `date_column` reads them: the day's date in
/// the display zone, the span's two dates in the **calendar's** zone. It used
/// to dot on an *instant* overlap — `start_ms < bounds[d + 1] && end_ms >
/// bounds[d]` — which for a foreign calendar is worse than the shift the other
/// grids had. An Auckland one-day event is stored across noon of a UTC day, so
/// it overlapped two of them and dotted **both**: one day rendered as two.
/// A range and not `date_column`'s two endpoints because every day in between
/// is dotted too, and because a span lying entirely outside the month must dot
/// nothing — which comparing dates gives for free and clamping two
/// out-of-window sentinels does not.
pub fn assemble_year(
    events: &[StoredEvent],
    year: i32,
    now_ms: i64,
    tz: &str,
    week_start: crate::settings::WeekStart,
) -> YearPayload {
    use jiff::civil::date;

    let suppressed = suppressed_slots(events);
    let (synced_from, synced_to) = crate::synced_window(now_ms);

    let months = (1..=12u32)
        .map(|month| {
            let first = date(year as i16, month as i8, 1);
            let days_in_month = first.days_in_month() as usize;
            let month_start_ms = local_midnight_ms(first, tz);
            let bounds = n_day_boundaries(month_start_ms, days_in_month, tz);
            // One entry per day, so it lines up with `has_all_day` exactly.
            let day_dates = column_dates(&bounds, tz);
            let lead_blanks = week_start.lead_blanks(first.weekday());

            let mut has_all_day = vec![false; days_in_month];

            for src in events {
                // A cancelled exception exists only to record that an
                // occurrence was deleted; see `assemble_days`.
                if src.status == "cancelled" {
                    continue;
                }
                // Only all-day events dot the year grid — a timed meeting is
                // not "blocked out"; this view answers what *is*.
                if !src.is_all_day {
                    continue;
                }
                for iv in occurrences(src, bounds[0], bounds[days_in_month]) {
                    if suppressed.contains(&(src.calendar_id, src.google_id.as_str(), iv.start_ms)) {
                        continue;
                    }
                    let (first_date, last_date) = crate::write::all_day_span_dates(
                        iv.start_ms,
                        iv.end_ms,
                        &src.calendar_timezone,
                    );
                    for (dotted, day_date) in has_all_day.iter_mut().zip(&day_dates) {
                        if day_date.as_str() >= first_date.as_str()
                            && day_date.as_str() <= last_date.as_str()
                        {
                            *dotted = true;
                        }
                    }
                }
            }

            let days = (0..days_in_month)
                .map(|d| {
                    let start_ms = bounds[d];
                    YearDay {
                        start_ms,
                        day: (d + 1) as u32,
                        has_all_day: has_all_day[d],
                        unsynced: start_ms < synced_from || start_ms >= synced_to,
                    }
                })
                .collect();

            YearMonth { month, lead_blanks, days }
        })
        .collect();

    YearPayload { year, months }
}

#[derive(Debug, Clone, Serialize)]
pub struct RibbonDay {
    pub start_ms: i64,
    /// False for days that spill into the year before or after — the ribbon
    /// is a Monday-aligned 14x28 grid, which never lines up exactly on
    /// 1 Jan or 31 Dec.
    pub in_year: bool,
    /// Outside the synced window (`synced_window`), same meaning as
    /// `YearDay::unsynced`.
    pub unsynced: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RibbonRow {
    pub days: Vec<RibbonDay>, // always 28
    /// All-day/multi-day spans, lane-packed at row_len 28, 3 lanes.
    pub pills: Vec<Lane>,
    pub pill_events: Vec<UiEvent>, // `Lane.idx` indexes into this
    pub overflow: Vec<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BigYearPayload {
    pub year: i32,
    pub rows: Vec<RibbonRow>, // always 14
}


/// The Monday on or before `year`-01-01, at local midnight in `tz` — the
/// anchor for the 392-day Big Year ribbon.
///
/// `pub(crate)`: `get_big_year` needs this same anchor to size its fetch
/// window *before* calling `assemble_big_year`, which recomputes it
/// internally — the same relationship `month_grid_start_ms` has with
/// `get_month`/`assemble_month`.
pub(crate) fn big_year_start_ms(
    year: i32,
    tz: &str,
    week_start: crate::settings::WeekStart,
) -> i64 {
    use jiff::civil::date;

    let mut d = date(year as i16, 1, 1);
    while d.weekday() != week_start.weekday() {
        d = d.yesterday().expect("civil date underflow");
    }
    local_midnight_ms(d, tz)
}

/// The Big Year ribbon: fourteen rows of 28 days (392 days total), anchored
/// on the Monday on or before `year`-01-01 and running with overhang on both
/// ends. Only all-day and multi-day events are shown — this view answers
/// "does this leave request swallow a weekend", not "how busy is this day".
///
/// Rows are 28 days, not 29, even though 392 already covers the year with
/// room to spare either width would give: 28 is a multiple of 7, so the
/// weekend columns (`[5,6,12,13,19,20,26,27]`) fall in the same place in
/// every row, reading as straight vertical stripes down the page. A 29-day
/// row drifts the weekends diagonally instead, which quietly defeats the
/// view's entire purpose. See
/// `every_row_puts_its_weekends_in_the_same_columns`.
///
/// Pills are placed by date (`all_day_columns`), the same rule as the week and
/// month grids — see `date_column`. Bucketing the stored instant against the
/// display zone's day boundaries, which this did, drew a foreign calendar's
/// one-day event as two pills: the end of one row and the start of the next.
pub fn assemble_big_year(
    events: &[StoredEvent],
    year: i32,
    now_ms: i64,
    tz: &str,
    week_start: crate::settings::WeekStart,
) -> BigYearPayload {
    use jiff::civil::date;

    let ribbon_start_ms = big_year_start_ms(year, tz, week_start);
    let bounds = n_day_boundaries(ribbon_start_ms, 392, tz);
    // All 392 at once, then sliced per row — the same relationship `row_bounds`
    // has with `bounds`, and exactly how `assemble_month` feeds its rows.
    let ribbon_dates = column_dates(&bounds, tz);

    let year_start_ms = local_midnight_ms(date(year as i16, 1, 1), tz);
    let next_year_start_ms = local_midnight_ms(date((year + 1) as i16, 1, 1), tz);
    let (synced_from, synced_to) = crate::synced_window(now_ms);
    let suppressed = suppressed_slots(events);

    let rows = (0..14)
        .map(|r| {
            let row_bounds = &bounds[r * 28..=r * 28 + 28];
            let row_dates = &ribbon_dates[r * 28..r * 28 + 28];
            let row_start = row_bounds[0];
            let row_end = row_bounds[28];

            let mut pill_events: Vec<UiEvent> = Vec::new();
            let mut segments: Vec<Segment> = Vec::new();

            for src in events {
                // A cancelled exception exists only to record that an
                // occurrence was deleted; see `assemble_days`.
                if src.status == "cancelled" {
                    continue;
                }
                for iv in occurrences(src, row_start, row_end) {
                    if suppressed.contains(&(src.calendar_id, src.google_id.as_str(), iv.start_ms)) {
                        continue;
                    }
                    if src.is_all_day {
                        let (start_col, end_col) = all_day_columns(src, &iv, row_dates);
                        segments.push(Segment { idx: pill_events.len(), start_col, end_col });
                        pill_events.push(to_ui(src, iv.start_ms, iv.end_ms));
                    }
                }
            }

            // Three lanes, matching the spec's row height.
            let (pills, overflow) = pack_lanes(&segments, 28, 3);

            let days = (0..28)
                .map(|c| {
                    let start_ms = row_bounds[c];
                    RibbonDay {
                        start_ms,
                        in_year: start_ms >= year_start_ms && start_ms < next_year_start_ms,
                        unsynced: start_ms < synced_from || start_ms >= synced_to,
                    }
                })
                .collect();

            RibbonRow { days, pills, pill_events, overflow }
        })
        .collect();

    BigYearPayload { year, rows }
}

/// Opens an event's meeting link with its preferred application — the Join
/// control, moved backend-side.
///
/// The webview's `<a target="_blank">` used to carry this click, which
/// routed it through the opener plugin's raw spawn — inside an AppImage,
/// the environment-poisoning path of issue #1, and a browser that crashes
/// on launch. Now the UI sends the event *id* and this resolves the URL
/// itself: `open_latest_release`'s rule, for the same reason — the webview
/// never chooses what the browser is pointed at, it can only name an event
/// the user can already see.
///
/// The URL comes from the same places the popover's own derivation reads,
/// in the same order: structured conference data first, then a recognised
/// meeting link in `location`, then one in `description`
/// (`conference_join_url` — the widget feed's helper, so this, the feed and
/// the CLI all agree on what is joinable).
#[tauri::command]
pub(crate) async fn open_conference(
    state: tauri::State<'_, crate::AppState>,
    id: i64,
) -> Result<(), String> {
    let row: Option<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT conference_uri, location, description FROM events WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| crate::errors::user_facing(&e.into()))?;
    let Some((conference, location, description)) = row else {
        return Err("This event is no longer on the calendar.".into());
    };
    let url = conference
        .or_else(|| crate::upcoming::conference_join_url(location.as_deref(), description.as_deref()))
        .ok_or_else(|| "This event has no meeting link.".to_string())?;
    crate::browser::open_external(&url).map_err(|e| {
        tracing::warn!(%e, "could not open the meeting link");
        crate::BROWSER_FAILED.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(gid: &str, start: i64, end: i64, all_day: bool) -> omacal_store::StoredEvent {
        omacal_store::StoredEvent {
            id: 0, calendar_id: 1, google_id: gid.into(), summary: Some(gid.into()),
            location: None, start_utc: start, end_utc: end,
            start_tz: "UTC".into(), end_tz: "UTC".into(),
            is_all_day: all_day, recurrence: None,
            recurring_event_id: None, original_start_utc: None,
            status: "confirmed".into(),
            self_response: Some("accepted".into()), conference_uri: None,
            color_hex: None, calendar_timezone: "UTC".into(),
            description: None, etag: None, sequence: 0, organizer_email: None,
            guests_can_modify: false,
            attendees: Vec::new(),
            reminders: Default::default(), calendar_default_reminders: Vec::new(),
        }
    }

    /// A daily 09:00–09:30 series starting on the week's Monday.
    fn daily_master() -> omacal_store::StoredEvent {
        let mut m = ev("standup", MON + 9 * 3_600_000, MON + 9 * 3_600_000 + 1_800_000, false);
        m.recurrence = Some("RRULE:FREQ=DAILY".into());
        m
    }

    const DAY: i64 = 24 * 3_600_000;
    /// Monday 2026-08-03 00:00:00 UTC
    const MON: i64 = 1_785_715_200_000;

    #[test]
    fn a_timed_event_lands_in_its_own_day_column() {
        let evs = vec![ev("a", MON + 9 * 3_600_000, MON + 10 * 3_600_000, false)];
        let w = assemble_week(&evs, MON, "UTC");
        assert_eq!(w.days[0].events.len(), 1);
        assert!(w.days[1].events.is_empty());
    }

    #[test]
    fn an_event_on_wednesday_lands_in_column_two() {
        let evs = vec![ev("a", MON + 2 * DAY + 9 * 3_600_000, MON + 2 * DAY + 10 * 3_600_000, false)];
        let w = assemble_week(&evs, MON, "UTC");
        assert_eq!(w.days[2].events.len(), 1);
    }

    #[test]
    fn overlapping_events_get_two_columns() {
        let evs = vec![
            ev("a", MON + 10 * 3_600_000, MON + 11 * 3_600_000, false),
            ev("b", MON + 10 * 3_600_000, MON + 11 * 3_600_000, false),
        ];
        let w = assemble_week(&evs, MON, "UTC");
        assert_eq!(w.days[0].placed[0].columns, 2);
    }

    #[test]
    fn all_day_events_go_to_the_band_not_the_grid() {
        let evs = vec![ev("trip", MON, MON + 3 * DAY, true)];
        let w = assemble_week(&evs, MON, "UTC");
        assert!(w.days[0].events.is_empty());
        assert_eq!(w.all_day_events.len(), 1);
        assert_eq!(w.all_day[0].start_col, 0);
        assert_eq!(w.all_day[0].end_col, 2);
    }

    #[test]
    fn an_all_day_span_entering_the_week_is_flagged_as_continuing() {
        let evs = vec![ev("trip", MON - 3 * DAY, MON + 2 * DAY, true)];
        let w = assemble_week(&evs, MON, "UTC");
        assert!(w.all_day[0].cont_left);
        assert!(!w.all_day[0].cont_right);
    }

    #[test]
    fn events_outside_the_week_are_dropped() {
        let evs = vec![ev("a", MON + 30 * DAY, MON + 30 * DAY + 3_600_000, false)];
        let w = assemble_week(&evs, MON, "UTC");
        assert!(w.days.iter().all(|d| d.events.is_empty()));
    }

    #[test]
    fn a_recurring_master_is_expanded_across_the_week() {
        let mut master = ev("standup", MON + 9 * 3_600_000, MON + 9 * 3_600_000 + 1_800_000, false);
        master.recurrence = Some("RRULE:FREQ=DAILY".into());
        let w = assemble_week(&[master], MON, "UTC");
        let total: usize = w.days.iter().map(|d| d.events.len()).sum();
        assert_eq!(total, 7);
    }

    /// Monday 2026-10-19 00:00 in Europe/Sofia. That week contains the
    /// end of DST on Sunday 2026-10-25, making that Sunday 25 hours long.
    fn dst_week_start() -> i64 {
        jiff::civil::date(2026, 10, 19)
            .at(0, 0, 0, 0)
            .in_tz("Europe/Sofia")
            .unwrap()
            .timestamp()
            .as_millisecond()
    }

    #[test]
    fn a_dst_week_contains_a_twenty_five_hour_day() {
        let bounds = n_day_boundaries(dst_week_start(), 7, "Europe/Sofia");
        let lengths: Vec<i64> = bounds.windows(2).map(|w| w[1] - w[0]).collect();
        assert_eq!(lengths.len(), 7);
        assert!(
            lengths.contains(&(25 * 3_600_000)),
            "expected one 25-hour day, got {lengths:?}"
        );
    }

    #[test]
    fn a_normal_week_has_seven_equal_days() {
        let bounds = n_day_boundaries(dst_week_start() - 7 * DAY, 7, "Europe/Sofia");
        let lengths: Vec<i64> = bounds.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(lengths.iter().all(|&l| l == DAY), "got {lengths:?}");
    }

    #[test]
    fn an_event_late_on_a_long_day_stays_inside_its_column() {
        let start = dst_week_start();
        let bounds = n_day_boundaries(start, 7, "Europe/Sofia");
        // 24h30m into the 25-hour Sunday: valid, and only representable
        // because the day window is its true length.
        let late = bounds[6] + 24 * 3_600_000 + 1_800_000;
        assert_eq!(column_for(&bounds, late), Some(6));

        let w = assemble_week(&[ev("night", late, late + 1_800_000, false)], start, "Europe/Sofia");
        assert_eq!(w.days[6].events.len(), 1);
        let p = w.days[6].placed[0];
        assert!(p.top < 1.0, "top {} should stay within the day", p.top);
        assert!(p.top + p.height <= 1.0001, "block overflows the column");
    }

    #[test]
    fn an_unknown_timezone_still_produces_seven_days() {
        let bounds = n_day_boundaries(MON, 7, "Mars/Olympus_Mons");
        assert_eq!(bounds.len(), 8);
        assert_eq!(bounds[7] - bounds[0], 7 * DAY);
    }

    /// A moved instance: the master must stop expanding into the slot the
    /// instance came from. Without suppression Tuesday shows two events — the
    /// real one at 14:00 and a ghost at 09:00.
    #[test]
    fn a_moved_instance_replaces_its_original_occurrence() {
        let mut moved = ev("standup_20260804", MON + DAY + 14 * 3_600_000,
                           MON + DAY + 14 * 3_600_000 + 1_800_000, false);
        moved.recurring_event_id = Some("standup".into());
        moved.original_start_utc = Some(MON + DAY + 9 * 3_600_000);

        let w = assemble_week(&[daily_master(), moved], MON, "UTC");

        assert_eq!(w.days[1].events.len(), 1, "Tuesday must show one event, not two");
        assert_eq!(w.days[1].events[0].start_ms, MON + DAY + 14 * 3_600_000);
        // Every other day keeps its ordinary occurrence.
        let total: usize = w.days.iter().map(|d| d.events.len()).sum();
        assert_eq!(total, 7);
    }

    /// A deleted instance: Google sends a cancelled exception. It is stored,
    /// renders nothing itself, and silences the master for that one day.
    #[test]
    fn a_cancelled_instance_empties_its_day_and_no_other() {
        let mut cancelled = ev("standup_20260805", MON + 2 * DAY + 9 * 3_600_000,
                               MON + 2 * DAY + 9 * 3_600_000, false);
        cancelled.status = "cancelled".into();
        cancelled.recurring_event_id = Some("standup".into());
        cancelled.original_start_utc = Some(MON + 2 * DAY + 9 * 3_600_000);

        let w = assemble_week(&[daily_master(), cancelled], MON, "UTC");

        assert!(w.days[2].events.is_empty(), "the deleted occurrence must be gone");
        assert_eq!(w.days[1].events.len(), 1, "the day before is unaffected");
        assert_eq!(w.days[3].events.len(), 1, "the day after is unaffected");
        let total: usize = w.days.iter().map(|d| d.events.len()).sum();
        assert_eq!(total, 6);
    }

    /// An instance dragged clean out of the week still has to silence the slot
    /// it left behind, so the store returns it and nothing renders on that day.
    #[test]
    fn an_instance_moved_out_of_the_week_leaves_no_ghost() {
        let mut moved = ev("standup_20260806", MON + 40 * DAY, MON + 40 * DAY + 1_800_000, false);
        moved.recurring_event_id = Some("standup".into());
        moved.original_start_utc = Some(MON + 3 * DAY + 9 * 3_600_000);

        let w = assemble_week(&[daily_master(), moved], MON, "UTC");
        assert!(w.days[3].events.is_empty());
        let total: usize = w.days.iter().map(|d| d.events.len()).sum();
        assert_eq!(total, 6);
    }

    /// An exception only silences its own master, and only on its own calendar.
    #[test]
    fn an_exception_does_not_silence_an_unrelated_series() {
        let mut other = ev("other_ex", MON + DAY + 14 * 3_600_000,
                           MON + DAY + 15 * 3_600_000, false);
        other.recurring_event_id = Some("some-other-series".into());
        other.original_start_utc = Some(MON + DAY + 9 * 3_600_000);

        let w = assemble_week(&[daily_master(), other], MON, "UTC");
        // Tuesday keeps its standup *and* gains the unrelated exception.
        assert_eq!(w.days[1].events.len(), 2);
    }

    /// A meeting that runs Sunday 23:00 into Monday 01:00 overlaps the week but
    /// starts before it. Dropping it made it vanish; it belongs in column 0,
    /// clipped to the column.
    #[test]
    fn a_timed_event_starting_before_the_week_is_clamped_into_the_first_column() {
        let evs = vec![ev("night", MON - 3_600_000, MON + 3_600_000, false)];
        let w = assemble_week(&evs, MON, "UTC");
        assert_eq!(w.days[0].events.len(), 1);
        let p = w.days[0].placed[0];
        assert!((p.top - 0.0).abs() < 1e-6, "top {} should clamp to the column start", p.top);
        assert!(p.height > 0.0);
    }

    /// The same shape at the other edge: it starts inside the week and runs out
    /// of it. The geometry must stay inside the last column.
    #[test]
    fn a_timed_event_running_past_the_week_stays_in_the_last_column() {
        let start = MON + 6 * DAY + 23 * 3_600_000;
        let evs = vec![ev("night", start, start + 2 * 3_600_000, false)];
        let w = assemble_week(&evs, MON, "UTC");
        assert_eq!(w.days[6].events.len(), 1);
        let p = w.days[6].placed[0];
        assert!(p.top + p.height <= 1.0001, "block overflows the column");
    }

    /// An event that ends before the week begins still has no column.
    #[test]
    fn an_event_entirely_before_the_week_is_still_dropped() {
        let evs = vec![ev("old", MON - 5 * 3_600_000, MON - 4 * 3_600_000, false)];
        let w = assemble_week(&evs, MON, "UTC");
        assert!(w.days.iter().all(|d| d.events.is_empty()));
    }

    #[test]
    fn an_event_carries_its_calendars_colour() {
        let mut e = ev("a", MON + 9 * 3_600_000, MON + 10 * 3_600_000, false);
        e.color_hex = Some("#b58900".into());
        let w = assemble_week(&[e], MON, "UTC");
        assert_eq!(w.days[0].events[0].color, "#b58900");
    }

    #[test]
    fn a_calendar_without_a_colour_falls_back_to_the_default() {
        let evs = vec![ev("a", MON + 9 * 3_600_000, MON + 10 * 3_600_000, false)];
        let w = assemble_week(&evs, MON, "UTC");
        assert_eq!(w.days[0].events[0].color, DEFAULT_EVENT_COLOR);
    }

    /// The UI draws hour rules and the now-line against `end_ms - start_ms`, so
    /// a long day has to report its true length rather than a nominal 24 hours.
    #[test]
    fn each_column_reports_its_true_span() {
        let w = assemble_week(&[], dst_week_start(), "Europe/Sofia");
        let spans: Vec<i64> = w.days.iter().map(|d| d.end_ms - d.start_ms).collect();
        assert!(spans.contains(&(25 * 3_600_000)), "expected a 25-hour day, got {spans:?}");
        // Each column ends exactly where the next begins; no gaps, no overlaps.
        for pair in w.days.windows(2) {
            assert_eq!(pair[0].end_ms, pair[1].start_ms);
        }
    }

    #[test]
    fn one_day_and_seven_days_agree_about_the_day_they_share() {
        // This is the guard that Day is genuinely the week engine at n=1 rather
        // than a parallel implementation that will drift. If someone later
        // "optimises" assemble_days for the n=1 case, this fails.
        //
        // It carries all-day events as well as the timed one because
        // `assemble_days` runs **two** placement rules, not one: a timed event
        // is an instant and goes through `timed_column`, an all-day event is a
        // date and goes through `all_day_columns`. A timed-only fixture covered
        // both while they were the same rule; it leaves the second unguarded
        // now that they are not, and an `n == 1` special case putting every
        // all-day chip in column 0 passed the whole committed suite.
        //
        // **The pair is what makes it bite, not the all-day event on its own.**
        // One Auckland event really is on the shared day, so a rule that always
        // answers column 0 gets it right by accident. The other is on the
        // *next* day, yet its stored instant still falls inside the one-day
        // window — so it reaches placement and has to be rejected *there*, on
        // its date. Drop it and the mutation survives.
        let standup = ev("standup", MON + 9 * 3_600_000, MON + 9 * 3_600_000 + 30 * 60_000, false);
        let on_the_shared_day = all_day_event(
            AUCKLAND,
            midnight_ms(AUCKLAND, 2026, 8, 3),
            midnight_ms(AUCKLAND, 2026, 8, 4),
        );
        let on_the_next_day = all_day_event(
            AUCKLAND,
            midnight_ms(AUCKLAND, 2026, 8, 4),
            midnight_ms(AUCKLAND, 2026, 8, 5),
        );

        // The premise, without which the two rules cannot be told apart: the
        // second event's stored instant lands on the *shared* day in the
        // display zone, while its own calendar calls it the day after.
        assert_eq!(
            crate::write::date_in_zone(on_the_next_day.start_utc, "Europe/Sofia"),
            "2026-08-03",
            "the display zone reads it as the shared day"
        );
        assert_eq!(
            crate::write::date_in_zone(on_the_next_day.start_utc, AUCKLAND),
            "2026-08-04",
            "while its own calendar calls it the next one"
        );

        let evs = vec![standup, on_the_shared_day, on_the_next_day];

        let week = assemble_days(&evs, MON, 7, "Europe/Sofia");
        let day = assemble_days(&evs, week.days[0].start_ms, 1, "Europe/Sofia");

        assert_eq!(day.days.len(), 1);
        assert_eq!(day.days[0].start_ms, week.days[0].start_ms);
        assert_eq!(day.days[0].end_ms, week.days[0].end_ms);
        assert_eq!(
            day.days[0].events.len(),
            week.days[0].events.len(),
            "the same day assembled alone and as part of a week disagreed"
        );

        // The all-day band — the half a timed fixture cannot see. A lane that
        // covers column 0 is clipped to start there, so `start_col == 0` is
        // exactly "covers the shared day".
        //
        // Counted from `all_day` and never from `all_day_events`: the latter
        // collects every occurrence that reached placement, *including* those
        // placed outside the window, so it is 2 either way and would assert
        // nothing at all.
        let week_chips_on_the_shared_day = week.all_day.iter().filter(|l| l.start_col == 0).count();
        assert_eq!(
            week_chips_on_the_shared_day, 1,
            "exactly one of the two Auckland events belongs to the shared day"
        );
        assert_eq!(
            day.all_day.len(),
            week_chips_on_the_shared_day,
            "the all-day band disagreed about the day they share"
        );
    }

    #[test]
    fn a_single_day_window_still_bounds_its_all_day_lane() {
        // pack_lanes is called with `n` as row_len; passing 7 for a one-day view
        // would let an all-day event claim columns that do not exist.
        let evs = vec![ev("trip", MON, MON + 3 * DAY, true)];
        let day = assemble_days(&evs, MON, 1, "Europe/Sofia");
        for lane in &day.all_day {
            assert!(lane.end_col < 1, "a 1-day view produced column {}", lane.end_col);
        }
    }

    /// An all-day event on a calendar whose own zone is `cal_tz`.
    ///
    /// The zone is not decoration. An all-day event's ends are stored as local
    /// midnight **in the calendar's zone** — Google sends a bare `date` and
    /// `omacal_sync::resolve` resolves it against `calendars.timezone` — and
    /// that is the only zone the date reads back correctly in. A fixture whose
    /// instants are one zone's midnights while its `calendar_timezone` names
    /// another describes an event that cannot exist, and would let a placement
    /// bug hide behind it.
    fn all_day_event(cal_tz: &str, start: i64, end: i64) -> omacal_store::StoredEvent {
        let mut e = ev(&format!("allday_{start}"), start, end, true);
        e.calendar_timezone = cal_tz.into();
        e
    }

    fn timed_event(start: i64, end: i64) -> omacal_store::StoredEvent {
        ev(&format!("timed_{start}"), start, end, false)
    }

    /// Local midnight on a civil date in `zone`, as epoch milliseconds — how
    /// sync stores an all-day event's ends, and the only shape a fixture for
    /// one may take.
    fn midnight_ms(zone: &str, y: i16, mo: i8, d: i8) -> i64 {
        jiff::civil::date(y, mo, d)
            .at(0, 0, 0, 0)
            .in_tz(zone)
            .unwrap()
            .timestamp()
            .as_millisecond()
    }

    /// UTC+12 through August, so its midnight falls on the *previous* UTC date.
    /// That is what lets a fixture see a placement reading the stored instant
    /// instead of the date. A calendar zone equal to the display zone cannot.
    const AUCKLAND: &str = "Pacific/Auckland";
    /// UTC-4 through August — the mirror image, and needed for a reason the
    /// arithmetic gives rather than symmetry: see
    /// `an_all_day_events_last_day_comes_from_its_own_calendars_zone`.
    const NEW_YORK: &str = "America/New_York";

    /// The defect this plan exists to close, in the week grid.
    ///
    /// An all-day event has a *date*, not an instant. The store holds midnight
    /// in the **calendar's** zone; bucketing that instant against the
    /// **display** zone's day boundaries draws the chip on whichever day
    /// happens to contain it there. Auckland is UTC+12 in August, so 5 Aug is
    /// stored as 2026-08-04T12:00Z and the chip was drawn under Tue 4 — while
    /// the event's own popover said Wed 5 and the edit form opened on
    /// 2026-08-05.
    #[test]
    fn an_all_day_event_lands_on_its_own_calendars_date_not_the_displays() {
        let start = midnight_ms(AUCKLAND, 2026, 8, 5);
        let end = midnight_ms(AUCKLAND, 2026, 8, 6); // Google's end is exclusive

        // The fixture's own premise, asserted rather than assumed: a calendar
        // zone that agrees with the display about the date cannot see this
        // defect at all, so a fixture that stopped separating would pass
        // vacuously.
        assert_eq!(start, 1_785_844_800_000, "2026-08-05 00:00 Auckland is 2026-08-04T12:00Z");
        assert_eq!(crate::write::date_in_zone(start, AUCKLAND), "2026-08-05");
        assert_eq!(
            crate::write::date_in_zone(start, "UTC"),
            "2026-08-04",
            "the display zone must read a different date, or this test proves nothing"
        );

        let w = assemble_week(&[all_day_event(AUCKLAND, start, end)], MON, "UTC");

        assert_eq!(w.all_day.len(), 1, "one event, one chip");
        assert_eq!(w.all_day[0].start_col, 2, "Wed 5 Aug is column 2 of the week opening Mon 3");
        assert_eq!(w.all_day[0].end_col, 2, "a one-day event covers one column");
        assert!(!w.all_day[0].cont_left, "it does not begin before the week");
        assert!(!w.all_day[0].cont_right, "nor run past it");
        assert_eq!(w.days[2].start_ms, MON + 2 * DAY, "and column 2 really is Wed 5 Aug");
    }

    /// The same rule over a span: it runs from its start date's column to its
    /// **inclusive** end date's column, both read in the calendar's zone.
    #[test]
    fn a_multi_day_all_day_event_spans_its_own_calendars_dates() {
        // Wed 5 - Fri 7 Aug inclusive on the Auckland calendar; the end Google
        // sends is the exclusive 8th.
        let start = midnight_ms(AUCKLAND, 2026, 8, 5);
        let end = midnight_ms(AUCKLAND, 2026, 8, 8);
        assert_eq!(
            crate::write::date_in_zone(start, "UTC"),
            "2026-08-04",
            "premise: the display zone reads the start a day early"
        );

        let w = assemble_week(&[all_day_event(AUCKLAND, start, end)], MON, "UTC");

        assert_eq!(w.all_day.len(), 1);
        assert_eq!(w.all_day[0].start_col, 2, "Wed 5 Aug");
        assert_eq!(w.all_day[0].end_col, 4, "Fri 7 Aug, inclusive");
        assert!(!w.all_day[0].cont_left);
        assert!(!w.all_day[0].cont_right);
    }

    /// `AUCKLAND` cannot witness the *end* of a span, so this fixture is not
    /// symmetry — it closes a survivor the arithmetic leaves open.
    ///
    /// At UTC+12 the stored exclusive end is 12:00 on the previous UTC day, and
    /// one millisecond back from it is still that same UTC day: exactly the
    /// date the calendar's own zone gives. The old `end_ms - 1` bucketing and
    /// the date derivation agree there, so an Auckland-only fixture would leave
    /// the end unguarded. A calendar *west* of the display separates them: New
    /// York is UTC-4 in August, so a span through Fri 7 Aug ends at
    /// 2026-08-08T04:00Z, and a millisecond back from that is Saturday.
    #[test]
    fn an_all_day_events_last_day_comes_from_its_own_calendars_zone() {
        let start = midnight_ms(NEW_YORK, 2026, 8, 5);
        let end = midnight_ms(NEW_YORK, 2026, 8, 8);

        // The premise, both halves of it.
        assert_eq!(
            crate::write::date_in_zone(end - 1, "UTC"),
            "2026-08-08",
            "a millisecond before the stored end is Saturday in the display zone"
        );
        assert_eq!(
            crate::write::date_in_zone(end, NEW_YORK),
            "2026-08-08",
            "while the calendar's own exclusive end is the 8th, so its last covered day is the 7th"
        );

        let w = assemble_week(&[all_day_event(NEW_YORK, start, end)], MON, "UTC");

        assert_eq!(w.all_day.len(), 1);
        assert_eq!(w.all_day[0].start_col, 2, "Wed 5 Aug");
        assert_eq!(
            w.all_day[0].end_col, 4,
            "Fri 7 Aug — not Sat 8, which the stored instant suggests"
        );
        assert!(!w.all_day[0].cont_right);
    }

    /// The other half of the fix, and it has to be witnessed on its own rather
    /// than inferred from the absence of a failure.
    ///
    /// A timed event genuinely *is* an instant, so it belongs to the day the
    /// display zone puts that instant on. Same calendar and the very same
    /// instant as `an_all_day_event_lands_on_its_own_calendars_date_not_the_displays`
    /// — Auckland's 5 Aug midnight, which UTC calls Tue 4 Aug 12:00. It must
    /// stay in Tuesday's column while the all-day chip moves to Wednesday's.
    #[test]
    fn a_timed_event_on_the_same_calendar_still_follows_the_display_zone() {
        let start = midnight_ms(AUCKLAND, 2026, 8, 5);
        let mut e = timed_event(start, start + 3_600_000);
        e.calendar_timezone = AUCKLAND.into();

        let w = assemble_week(std::slice::from_ref(&e), MON, "UTC");
        assert_eq!(w.days[1].events.len(), 1, "Tue 4 Aug UTC is where that instant falls");
        assert!(w.days[2].events.is_empty(), "it must not follow the all-day rule to Wednesday");
        assert_eq!(w.days.iter().map(|d| d.events.len()).sum::<usize>(), 1);
        assert!(w.all_day.is_empty(), "and it is not an all-day chip");

        // The month grid's timed path is the same claim and takes the same
        // mutation, so it is asserted here rather than left to inference.
        let m = assemble_month(&[e], 2026, 8, "UTC", crate::settings::WeekStart::Monday);
        // Row 1 of the August 2026 UTC grid is Mon 3 - Sun 9 Aug.
        assert_eq!(m.rows[1].cells[1].timed.len(), 1, "Tue 4 Aug is column 1");
        assert!(m.rows[1].cells[2].timed.is_empty());
    }

    /// The two grids must name the same day for the same event — the month
    /// buckets all-day events through its own copy of the placement, so a fix
    /// applied to one and not the other is a grid that disagrees with itself.
    #[test]
    fn the_month_grid_places_an_all_day_event_where_the_week_grid_does() {
        let start = midnight_ms(AUCKLAND, 2026, 8, 5);
        let end = midnight_ms(AUCKLAND, 2026, 8, 6);
        let evs = vec![all_day_event(AUCKLAND, start, end)];

        let w = assemble_week(&evs, MON, "UTC");
        let m = assemble_month(&evs, 2026, 8, "UTC", crate::settings::WeekStart::Monday);

        // August 2026 opens on a Saturday, so the UTC grid runs Mon 27 Jul -
        // Sun 6 Sep and its row 1 is exactly the week under test.
        assert_eq!(m.rows[1].cells[0].start_ms, w.days[0].start_ms, "row 1 is the Mon 3 Aug week");

        assert_eq!(m.rows[1].bars.len(), 1, "the month draws the chip once");
        assert_eq!(m.rows[1].bars[0].start_col, 2, "Wed 5 Aug is column 2");
        assert_eq!(m.rows[1].bars[0].end_col, 2);
        // Then that the two agree. The absolute columns above are what stops
        // two wrong-but-equal answers passing this.
        assert_eq!(m.rows[1].bars[0].start_col, w.all_day[0].start_col);
        assert_eq!(m.rows[1].bars[0].end_col, w.all_day[0].end_col);
        // "And in no other row" belongs to
        // `an_all_day_event_on_a_row_boundary_appears_in_one_row_only`, whose
        // fixture can actually reach a second row. Asserted here it would be a
        // loop no mutation can fail: this event's instants never enter another
        // row's occurrence window at all.
    }

    /// The defect exactly as reported, in the zones it was reported in: a
    /// `Pacific/Auckland` calendar rendered against a `Europe/Sofia` display,
    /// where the chip appeared under Sun 9 Aug while its own popover said
    /// Mon, Aug 10 and the edit form opened on 2026-08-10.
    ///
    /// It is also the one test here that witnesses the **display** side of the
    /// comparison — every other uses a UTC display, where a column's date is
    /// the same whichever zone it is read in, so a `column_dates` that stopped
    /// reading `tz` would go unnoticed. Sofia's Monday midnight is
    /// 2026-08-09T21:00Z: still Sunday in UTC.
    #[test]
    fn an_auckland_event_rendered_in_sofia_lands_on_the_monday_both_of_them_name() {
        const SOFIA: &str = "Europe/Sofia";
        let start = midnight_ms(AUCKLAND, 2026, 8, 10);
        let end = midnight_ms(AUCKLAND, 2026, 8, 11);
        let week_start = midnight_ms(SOFIA, 2026, 8, 10);

        // The premise, all three legs of it.
        assert_eq!(crate::write::date_in_zone(start, AUCKLAND), "2026-08-10");
        assert_eq!(
            crate::write::date_in_zone(start, SOFIA),
            "2026-08-09",
            "the display zone reads the stored instant as the day before"
        );
        assert_eq!(
            crate::write::date_in_zone(week_start, "UTC"),
            "2026-08-09",
            "and Sofia's Monday midnight is still Sunday in UTC, so the column dates must be read in Sofia"
        );

        let evs = vec![all_day_event(AUCKLAND, start, end)];

        let w = assemble_week(&evs, week_start, SOFIA);
        assert_eq!(w.all_day.len(), 1);
        assert_eq!(w.all_day[0].start_col, 0, "Mon 10 Aug");
        assert_eq!(w.all_day[0].end_col, 0);
        assert!(!w.all_day[0].cont_left, "a one-day event began nowhere earlier");

        // And the week before — the one the stored instant really does fall
        // inside, in Sofia — must not draw it. That column is the bug report.
        let before = assemble_week(&evs, midnight_ms(SOFIA, 2026, 8, 3), SOFIA);
        assert!(
            before.all_day.is_empty(),
            "Sofia's Sun 9 Aug drew a chip for Auckland's Mon 10 Aug"
        );
    }

    /// The payload behind the UI suite's `crossZoneWeek` fixture — the same
    /// zone pair as the test above, one week's Wednesday instead of its Monday,
    /// because that is the week `ui/tests/app.spec.ts`'s end-to-end agreement
    /// spec drives.
    ///
    /// That fixture used to be a hand-written TypeScript literal, and the only
    /// thing that ever established it was right was a person reading real
    /// output off a temporary probe test and pasting it in. Nothing then held
    /// the two together: an edit here could not fail a Playwright spec, and a
    /// fixture describing a payload this function no longer returns would have
    /// gone unreported. Now the UI imports `ui/tests/generated/cross-zone-week.json`
    /// and this test writes it — see `crate::golden` for the division of labour.
    ///
    /// Everything in the file comes from here, so the fixture's numbers are the
    /// backend's own: `days[0].start_ms` is Sofia's Monday midnight
    /// (`XZONE_WEEK_START` on the UI side), `all_day_events[0]`'s instants are
    /// the stored Auckland midnights, and its `color` is `DEFAULT_EVENT_COLOR`
    /// because this row carries no colour of its own.
    #[test]
    fn the_cross_zone_week_golden_file_is_what_assemble_week_produces() {
        const SOFIA: &str = "Europe/Sofia";
        // The one value in the file chosen rather than derived. A real row id
        // is whatever SQLite handed out; the UI fixture keys the chip's popover
        // detail by it, and reads it back out of the file (`XZONE_ID`) rather
        // than restating it, so the two cannot disagree.
        const GOLDEN_ROW_ID: i64 = 4245;

        let start = midnight_ms(AUCKLAND, 2026, 8, 12);
        let end = midnight_ms(AUCKLAND, 2026, 8, 13); // Google's end is exclusive
        let week_start = midnight_ms(SOFIA, 2026, 8, 10);

        // The premise, both legs. Without them "column 2" is arithmetic; with
        // them it is the claim that a chip goes where its own calendar puts it.
        assert_eq!(crate::write::date_in_zone(start, AUCKLAND), "2026-08-12");
        assert_eq!(
            crate::write::date_in_zone(start, SOFIA),
            "2026-08-11",
            "the display zone reads the stored instant as the day before"
        );

        let mut src = all_day_event(AUCKLAND, start, end);
        src.id = GOLDEN_ROW_ID;
        src.summary = Some("Berlin trip".into()); // the title the UI spec reads off the chip

        let w = assemble_week(&[src], week_start, SOFIA);

        // The golden comparison first, deliberately: a changed assembler should
        // be reported as what it is — the committed fixture no longer describing
        // this backend — and not as a column assertion that happens to be next
        // to one.
        crate::golden::assert_golden("cross-zone-week", &w);

        // Then the claims the UI spec leans on, against the freshly computed
        // payload rather than the file. These are the backstop against a
        // regeneration that absorbed a real defect: `assert_golden` above would
        // be satisfied by any payload at all once the file had been rewritten.
        //
        // `start_col` is the discriminating one. A count of chips is not: the
        // instant-bucketing placement this replaced produced *one* lane too —
        // it just ran from column 1 to column 2, a two-day bar for a one-day
        // event — so `all_day.len() == 1` reads the same either way.
        assert_eq!(w.all_day.len(), 1);
        assert_eq!(w.all_day[0].start_col, 2, "Wed 12 Aug is column 2 of a Monday week");
        assert_eq!(w.all_day[0].end_col, 2, "a one-day event ends in the column it starts in");
        assert_eq!(w.days[0].start_ms, week_start, "the file's columns are Sofia midnights");
    }

    /// The shape the defect was reported in, and the one that makes "drawn in
    /// exactly one place" a claim with teeth.
    ///
    /// Auckland's Mon 10 Aug is stored as 2026-08-09T12:00Z, which falls inside
    /// the *previous* UTC week's bounds — so the old placement drew the chip
    /// under Sun 9 **and** under Mon 10, rendering one day as a two-row span,
    /// while the event's own popover said Mon, Aug 10 throughout.
    #[test]
    fn an_all_day_event_on_a_row_boundary_appears_in_one_row_only() {
        let start = midnight_ms(AUCKLAND, 2026, 8, 10);
        let end = midnight_ms(AUCKLAND, 2026, 8, 11);
        assert_eq!(
            crate::write::date_in_zone(start, "UTC"),
            "2026-08-09",
            "premise: the display zone reads the stored instant as the Sunday before"
        );
        let evs = vec![all_day_event(AUCKLAND, start, end)];

        // The week that contains it: Mon 10 - Sun 16 Aug UTC.
        let w = assemble_week(&evs, MON + 7 * DAY, "UTC");
        assert_eq!(w.all_day.len(), 1);
        assert_eq!(w.all_day[0].start_col, 0, "Mon 10 Aug");
        assert_eq!(w.all_day[0].end_col, 0);
        assert!(!w.all_day[0].cont_left, "a one-day event began nowhere earlier");

        // The week *before* must not draw it at all, even though the stored
        // instant sits squarely inside that week's UTC bounds.
        let before = assemble_week(&evs, MON, "UTC");
        assert!(before.all_day.is_empty(), "Sun 9 Aug's week drew a chip for Mon 10 Aug");

        // The same claim in the month grid, where those two weeks are adjacent
        // rows and the stray is visible side by side.
        let m = assemble_month(&evs, 2026, 8, "UTC", crate::settings::WeekStart::Monday);
        assert_eq!(m.rows[2].cells[0].start_ms, w.days[0].start_ms, "row 2 is the Mon 10 Aug week");
        assert_eq!(m.rows[2].bars.len(), 1);
        assert_eq!(m.rows[2].bars[0].start_col, 0, "Mon 10 Aug is column 0");
        assert!(!m.rows[2].bars[0].cont_left);
        for (r, row) in m.rows.iter().enumerate() {
            if r != 2 {
                assert!(row.bars.is_empty(), "row {r} drew a chip for a day it does not contain");
            }
        }
    }

    /// The month's own witness for the end of a span, for the reason
    /// `an_all_day_events_last_day_comes_from_its_own_calendars_zone` gives: an
    /// Auckland fixture cannot separate it.
    #[test]
    fn the_month_grid_reads_the_last_day_from_the_calendars_zone_too() {
        let evs = vec![all_day_event(
            NEW_YORK,
            midnight_ms(NEW_YORK, 2026, 8, 5),
            midnight_ms(NEW_YORK, 2026, 8, 8),
        )];
        let m = assemble_month(&evs, 2026, 8, "UTC", crate::settings::WeekStart::Monday);

        assert_eq!(m.rows[1].bars.len(), 1);
        assert_eq!(m.rows[1].bars[0].start_col, 2, "Wed 5 Aug");
        assert_eq!(m.rows[1].bars[0].end_col, 4, "Fri 7 Aug, not Sat 8");
        assert!(!m.rows[1].bars[0].cont_right);
    }

    #[test]
    fn august_2026_starts_on_a_saturday_and_needs_six_rows() {
        // August 2026 begins Sat 1 Aug, so the grid runs Mon 27 Jul - Sun 6 Sep.
        // It exercises leading out-of-month days, trailing ones, and six rows.
        let m = assemble_month(&[], 2026, 8, "Europe/Sofia", crate::settings::WeekStart::Monday);
        assert_eq!(m.rows.len(), 6);
        assert_eq!(m.rows[0].cells.len(), 7);
        assert_eq!(m.rows[0].cells[0].start_ms, 1785099600000, "grid must start Mon 27 Jul");
        assert!(!m.rows[0].cells[0].in_month, "27 Jul belongs to July");
        assert!(m.rows[0].cells[5].in_month, "1 Aug is a Saturday, column 5");
        assert!(!m.rows[5].cells[6].in_month, "the last cell belongs to September");
    }

    #[test]
    fn a_month_that_fits_in_five_rows_still_renders_six() {
        // Otherwise the grid changes height as you page through the year.
        let m = assemble_month(&[], 2026, 2, "Europe/Sofia", crate::settings::WeekStart::Monday);
        assert_eq!(m.rows.len(), 6);
    }

    /// June 2026's 1st is itself a Monday — the one shape every other weekday
    /// absorbs. A backward search that steps one day too far before checking
    /// converges on the same Monday for every other starting weekday, and
    /// only misfires here, landing a week early. Value computed independently
    /// (Europe/Sofia local midnight, cross-checked against the plan's
    /// `AUG_GRID_START` derivation method): 2026-06-01 00:00 Sofia =
    /// 1780261200000.
    #[test]
    fn a_month_that_starts_on_a_monday_has_no_leading_days() {
        let m = assemble_month(&[], 2026, 6, "Europe/Sofia", crate::settings::WeekStart::Monday);
        assert_eq!(m.rows[0].cells[0].start_ms, 1780261200000, "grid must start Mon 1 Jun");
        assert!(m.rows[0].cells[0].in_month, "1 Jun is itself the Monday the grid starts on");
    }

    /// January 2026 opens on a Thursday, so the Monday on or before it falls
    /// in the *previous* calendar year: 2025-12-29, a boundary the naive "same
    /// year" assumption would get wrong. Value computed independently
    /// (Europe/Sofia local midnight): 2025-12-29 00:00 Sofia = 1766959200000.
    #[test]
    fn a_grid_start_can_fall_in_the_previous_year() {
        let m = assemble_month(&[], 2026, 1, "Europe/Sofia", crate::settings::WeekStart::Monday);
        assert_eq!(m.rows[0].cells[0].start_ms, 1766959200000, "grid must start Mon 29 Dec 2025");
        assert!(!m.rows[0].cells[0].in_month, "29 Dec 2025 belongs to December, not January");
    }

    /// Every cell of the grid, row by row — what an assertion about the month
    /// as a whole (how many days are in it, how long each one is) reads.
    fn month_cells(m: &MonthPayload) -> Vec<&MonthCell> {
        m.rows.iter().flat_map(|r| &r.cells).collect()
    }

    /// Local midnight-relative instant in Europe/Sofia, the zone every month
    /// test here uses. Computed rather than pasted so a boundary case reads as
    /// the date it is about.
    fn sofia_ms(y: i16, mo: i8, d: i8, hour: i8) -> i64 {
        jiff::civil::date(y, mo, d)
            .at(hour, 0, 0, 0)
            .in_tz("Europe/Sofia")
            .unwrap()
            .timestamp()
            .as_millisecond()
    }

    /// December is the only month whose next month is in another year, and
    /// `assemble_month` computes that boundary itself. Get the rollover wrong
    /// — `(year, 1)` instead of `(year + 1, 1)` — and `next_month_start_ms`
    /// lands *before* `month_start_ms`, so no cell satisfies `in_month` at
    /// all and the whole of December renders dimmed as out-of-month.
    /// `a_grid_start_can_fall_in_the_previous_year` covers a grid *start* in
    /// the previous year; nothing else here reaches `month == 12`.
    #[test]
    fn december_rolls_over_into_the_next_year() {
        // Tue 1 Dec 2026, so the grid runs Mon 30 Nov - Sun 10 Jan.
        let m = assemble_month(&[], 2026, 12, "Europe/Sofia", crate::settings::WeekStart::Monday);
        let cells = month_cells(&m);
        assert_eq!(
            cells.iter().filter(|c| c.in_month).count(),
            31,
            "December has 31 days, and all of them belong to it"
        );
        assert!(cells[31].in_month, "31 Dec 2026 is still December");
        assert!(!cells[32].in_month, "1 Jan 2027 is not");
    }

    /// The month grid's own DST safety. October 2026 in Europe/Sofia contains
    /// the 25 Oct fall-back, so one of its 42 cells is 25 hours long. Derive
    /// the cells with fixed 24-hour arithmetic instead and every cell after
    /// the transition starts at 23:00 the previous day: `in_month` reaches 32,
    /// and a Mon 26 Oct 18:00 meeting lands in the cell the UI labels 25.
    ///
    /// The week has `a_dst_week_contains_a_twenty_five_hour_day`; a 42-day
    /// grid is six times likelier to contain a transition than a 7-day one.
    #[test]
    fn a_dst_month_keeps_every_cell_on_its_own_local_midnight() {
        let mon_26 = sofia_ms(2026, 10, 26, 0);
        let meeting = sofia_ms(2026, 10, 26, 18);
        let m = assemble_month(
            &[timed_event(meeting, meeting + 3_600_000)],
            2026,
            10,
            "Europe/Sofia",
            crate::settings::WeekStart::Monday,
        );
        let cells = month_cells(&m);

        let lengths: Vec<i64> = cells.iter().map(|c| c.end_ms - c.start_ms).collect();
        assert!(
            lengths.contains(&(25 * 3_600_000)),
            "Sun 25 Oct is 25 hours long, got {lengths:?}"
        );
        assert_eq!(
            cells.iter().filter(|c| c.in_month).count(),
            31,
            "October has 31 days; a cell starting at 23:00 on the 31st would make 32"
        );

        let holding: Vec<&&MonthCell> = cells.iter().filter(|c| !c.timed.is_empty()).collect();
        assert_eq!(holding.len(), 1, "one meeting belongs to exactly one cell");
        assert_eq!(
            holding[0].start_ms, mon_26,
            "an 18:00 meeting on Mon 26 Oct belongs to Monday's cell, not to Sunday's"
        );
    }

    #[test]
    fn a_multi_day_event_crossing_a_row_boundary_appears_in_both_rows_clipped() {
        // Sun 2 Aug -> Tue 4 Aug straddles the first row's end. It must appear
        // in both rows, clipped to each, and never as one event counted
        // twice.
        //
        // The brief's epoch values (1785963600000, 1786222800000) land on
        // Thu 6 Aug and Sun 9 Aug in Europe/Sofia, not Sun 2 / Tue 4 as
        // named — verified against the plan's fixture table
        // (`docs/superpowers/plans/2026-08-06-omacal-day-month-views.md`,
        // where `AUG_1` = Sat 2026-08-01 00:00 Sofia = 1785531600000).
        // Recomputed here instead of adjusting the assertions to match
        // whatever the code produces: start = Sun 2 Aug 00:00 Sofia
        // (1785618000000), end = Wed 5 Aug 00:00 Sofia (1785877200000,
        // exclusive per Google's all-day convention), covering Sun 2 - Tue
        // 4 Aug inclusive.
        let evs = vec![all_day_event("Europe/Sofia", 1785618000000, 1785877200000)];
        let m = assemble_month(&evs, 2026, 8, "Europe/Sofia", crate::settings::WeekStart::Monday);
        // `bars: Vec<Lane>` — each `Lane` is already one placed, row-clipped
        // segment (see `omacal_core::lanes::Lane`), not a container of
        // segments, so the count of bars *is* the count of segments placed
        // in the row.
        assert_eq!(m.rows[0].bars.len(), 1, "row 0 should carry the Sunday tail");
        assert_eq!(m.rows[1].bars.len(), 1, "row 1 should carry the Mon-Tue head");
        for lane in &m.rows[0].bars {
            assert!(lane.end_col <= 6, "a segment escaped its row: {}", lane.end_col);
        }
    }

    #[test]
    fn timed_events_land_in_their_own_day_sorted() {
        let evs = vec![
            timed_event(1786341600000 + 3 * 3_600_000, 1786341600000 + 4 * 3_600_000),
            timed_event(1786341600000, 1786341600000 + 30 * 60_000),
        ];
        let m = assemble_month(&evs, 2026, 8, "Europe/Sofia", crate::settings::WeekStart::Monday);
        // Mon 10 Aug is row 2, column 0.
        let cell = &m.rows[2].cells[0];
        assert_eq!(cell.timed.len(), 2);
        assert!(cell.timed[0].start_ms < cell.timed[1].start_ms, "not sorted by start");
    }

    #[test]
    fn a_year_has_twelve_months_with_the_right_day_counts() {
        let y = assemble_year(&[], 2026, 1_786_341_600_000, "Europe/Sofia", crate::settings::WeekStart::Monday);
        assert_eq!(y.months.len(), 12);
        assert_eq!(y.months[0].days.len(), 31, "January");
        assert_eq!(y.months[1].days.len(), 28, "February 2026 is not a leap year");
        assert_eq!(y.months[10].days.len(), 30, "November");
    }

    #[test]
    fn a_leap_february_has_twenty_nine_days() {
        let y = assemble_year(&[], 2028, 1_786_341_600_000, "Europe/Sofia", crate::settings::WeekStart::Monday);
        assert_eq!(y.months[1].days.len(), 29);
    }

    #[test]
    fn lead_blanks_line_the_first_up_under_its_weekday() {
        // 1 Jan 2026 is a Thursday, so Monday-first means three blanks.
        let y = assemble_year(&[], 2026, 1_786_341_600_000, "Europe/Sofia", crate::settings::WeekStart::Monday);
        assert_eq!(y.months[0].lead_blanks, 3);
        // 1 Jun 2026 is a Monday — no blanks at all.
        assert_eq!(y.months[5].lead_blanks, 0);
    }

    #[test]
    fn only_all_day_events_dot_the_year_grid() {
        // A timed meeting is not "blocked out"; this view answers what is.
        //
        // The one-hour meeting alone no longer witnesses the `is_all_day` guard
        // and cannot be left to stand for it. It did while days were dotted by
        // *instant overlap* — an hour overlaps a day. Now they are dotted by
        // date range, and an hour-long event read as a span gives an
        // **inverted** one (the exclusive end steps back a whole day), which
        // matches nothing whether the guard is there or not. A timed event
        // longer than a day gives a range that is the right way round, so it is
        // the shape that actually fails if the guard goes.
        let timed = vec![
            timed_event(1_786_341_600_000, 1_786_341_600_000 + 3_600_000),
            timed_event(
                midnight_ms("UTC", 2026, 8, 5) + 9 * 3_600_000,
                midnight_ms("UTC", 2026, 8, 8) + 17 * 3_600_000,
            ),
        ];
        let y = assemble_year(&timed, 2026, 1_786_341_600_000, "Europe/Sofia", crate::settings::WeekStart::Monday);
        assert_eq!(dotted_days(&y), Vec::new(), "a timed event dotted the year grid");
    }

    #[test]
    fn an_all_day_event_dots_exactly_the_days_it_covers() {
        // `only_all_day_events_dot_the_year_grid` above asserts that *no* day
        // is dotted, so `has_all_day[d] = false` makes it pass harder rather
        // than fail: it satisfies its own wording while being structurally
        // incapable of noticing a year grid that has stopped dotting
        // altogether. This is the positive half it needs to be read against.
        //
        // 15-17 March 2026, as Google sends an all-day span — the end is
        // exclusive, so it is local midnight on the 18th. Well clear of the
        // synced window's edge (which opens in February for this `now`), so
        // this property and `unsynced` stay independent.
        let tz = "Europe/Sofia";
        let start = local_midnight_ms(jiff::civil::date(2026, 3, 15), tz);
        let end = local_midnight_ms(jiff::civil::date(2026, 3, 18), tz);
        let y = assemble_year(&[all_day_event(tz, start, end)], 2026, 1_786_341_600_000, tz, crate::settings::WeekStart::Monday);

        let march = &y.months[2];
        assert_eq!(march.days[14].start_ms, start, "days[14] is the 15th");
        let dotted: Vec<u32> =
            march.days.iter().filter(|d| d.has_all_day).map(|d| d.day).collect();
        assert_eq!(dotted, vec![15, 16, 17], "exactly the days the span covers");

        for (i, m) in y.months.iter().enumerate() {
            if i == 2 {
                continue;
            }
            assert!(
                m.days.iter().all(|d| !d.has_all_day),
                "month {} dotted a day nothing covers",
                i + 1
            );
        }
    }

    #[test]
    fn days_outside_the_synced_window_are_marked_unsynced() {
        // From Aug 2026 the window starts in Feb, so January of the *current*
        // year is already outside it — an empty January must not read as free.
        let y = assemble_year(&[], 2026, 1_786_341_600_000, "Europe/Sofia", crate::settings::WeekStart::Monday);
        assert!(y.months[0].days[0].unsynced, "1 Jan 2026 is before now-180d");
        assert!(!y.months[7].days[0].unsynced, "1 Aug 2026 is inside the window");
    }

    #[test]
    fn the_ribbon_starts_on_the_monday_before_new_year_and_runs_fourteen_rows() {
        let b = assemble_big_year(&[], 2026, 1_786_341_600_000, "Europe/Sofia", crate::settings::WeekStart::Monday);
        assert_eq!(b.rows.len(), 14);
        assert_eq!(b.rows[0].days.len(), 28);
        // 1 Jan 2026 is a Thursday, so the ribbon opens Mon 29 Dec 2025.
        assert_eq!(b.rows[0].days[0].start_ms, 1766959200000);
        assert_eq!(b.rows[1].days[0].start_ms, 1769378400000);
        assert_eq!(b.rows[13].days[0].start_ms, 1798408800000);
        assert!(!b.rows[0].days[0].in_year, "29 Dec 2025 belongs to the year before");
    }

    /// 1 Jan 2024 is itself a Monday — the one starting weekday where the
    /// "Monday on or before" search must stop immediately rather than
    /// stepping back a further week, the same edge
    /// `a_month_that_starts_on_a_monday_has_no_leading_days` guards for the
    /// 42-day month grid. Probed during review against a real
    /// `assemble_big_year(&[], 2024, ..)` call and then deleted; made
    /// permanent here since it guards a real, previously-untested edge.
    /// 1 Jan 2024 00:00:00 UTC = 1704067200000.
    #[test]
    fn a_year_that_opens_on_a_monday_does_not_skip_back_a_further_week() {
        let b = assemble_big_year(&[], 2024, 1_704_067_200_000, "UTC", crate::settings::WeekStart::Monday);
        assert_eq!(b.rows[0].days[0].start_ms, 1_704_067_200_000, "the ribbon must open on 1 Jan itself");
        assert!(b.rows[0].days[0].in_year, "1 Jan 2024 belongs to the year it opens");
    }

    /// The ribbon's own §6 coverage. Nothing else asserted `RibbonDay.unsynced`
    /// at all, so hardcoding it to `false` left all 267 tests green — and an
    /// unsynced stretch with no hatch reads as "free", which is precisely the
    /// reading this flag exists to prevent.
    ///
    /// A single ribbon can never be outside the window at both ends (392 days
    /// against a 545-day window), so both edges take a ribbon each, from the
    /// same "now" — Aug 2026, putting the window at roughly Feb 2026 to Aug
    /// 2027. Both directions are asserted at both edges, so neither a
    /// hardcoded `false` nor a hardcoded `true` survives.
    #[test]
    fn ribbon_days_outside_the_synced_window_are_marked_unsynced() {
        const NOW: i64 = 1_786_341_600_000; // Aug 2026, as the tests above use

        // Near edge. A 2026 ribbon anchors on Mon 29 Dec 2025, so it always
        // opens before a window that only reaches back to February.
        let b = assemble_big_year(&[], 2026, NOW, "Europe/Sofia", crate::settings::WeekStart::Monday);
        assert!(b.rows[0].days[0].unsynced, "Mon 29 Dec 2025 is before now-180d");
        assert!(!b.rows[13].days[27].unsynced, "Jan 2027 is still inside now+365d");

        // Far edge. The next year's ribbon runs into Jan 2028, past a window
        // that ends in Aug 2027.
        let b = assemble_big_year(&[], 2027, NOW, "Europe/Sofia", crate::settings::WeekStart::Monday);
        assert!(!b.rows[0].days[0].unsynced, "Mon 28 Dec 2026 is inside the window");
        assert!(b.rows[13].days[27].unsynced, "Jan 2028 is past now+365d");
    }

    /// The anchors move with the setting, on the month that separates all
    /// three: **1 Aug 2026 is a Saturday**, so each start walks back a
    /// different distance — six days, seven, or none at all. A month opening
    /// mid-week would agree under two of the three and hide a wrong walk.
    #[test]
    fn the_month_grid_anchors_on_the_chosen_first_day() {
        use crate::settings::WeekStart;
        use jiff::{civil::Weekday, Timestamp};

        let weekday_of = |ms: i64| {
            Timestamp::from_millisecond(ms).unwrap().in_tz("UTC").unwrap().weekday()
        };
        const DAY: i64 = 24 * 3_600_000;

        let monday = month_grid_start_ms(2026, 8, "UTC", WeekStart::Monday);
        let sunday = month_grid_start_ms(2026, 8, "UTC", WeekStart::Sunday);
        let saturday = month_grid_start_ms(2026, 8, "UTC", WeekStart::Saturday);

        assert_eq!(weekday_of(monday), Weekday::Monday);
        assert_eq!(weekday_of(sunday), Weekday::Sunday);
        assert_eq!(weekday_of(saturday), Weekday::Saturday);

        // Absolute distances, not just "the right weekday": walking back a
        // whole extra week would still land on the right day name.
        //
        // Note the order, which is the counter-intuitive part and the reason
        // this test is worth its length: from a Saturday the *Sunday* start
        // walks back furthest (six days, to Sun 26 Jul) and Monday's is
        // nearer (five, to Mon 27 Jul). "Sunday is earlier in the week" is
        // the intuition that gets this backwards.
        assert_eq!(saturday - sunday, 6 * DAY, "Sat 1 Aug is six days after Sun 26 Jul");
        assert_eq!(saturday - monday, 5 * DAY, "and five after Mon 27 Jul");
        assert!(sunday < monday, "the Sunday grid opens a day earlier than the Monday one here");
    }

    /// The same for the 392-day ribbon, whose anchor is the week containing
    /// 1 Jan. 1 Jan 2026 is a Thursday.
    #[test]
    fn the_big_year_ribbon_anchors_on_the_chosen_first_day() {
        use crate::settings::WeekStart;
        use jiff::{civil::Weekday, Timestamp};

        let weekday_of = |ms: i64| {
            Timestamp::from_millisecond(ms).unwrap().in_tz("UTC").unwrap().weekday()
        };
        for (start, expected) in [
            (WeekStart::Monday, Weekday::Monday),
            (WeekStart::Sunday, Weekday::Sunday),
            (WeekStart::Saturday, Weekday::Saturday),
        ] {
            assert_eq!(weekday_of(big_year_start_ms(2026, "UTC", start)), expected, "{start:?}");
        }
    }

    /// **The 28-day row's invariant holds under all three starts.**
    ///
    /// The row length is 28 because that is a multiple of 7, so whichever day
    /// a row opens on, each column holds the same weekday in every row and the
    /// weekend reads as straight vertical stripes. Only *where* the stripes
    /// fall changes: Monday keeps the pair together at 5 and 6, Sunday splits
    /// it to the ends, Saturday puts it together at the front. This is the
    /// generalisation of `every_row_puts_its_weekends_in_the_same_columns`,
    /// which pins the Monday case exactly.
    #[test]
    fn the_ribbons_weekend_stripes_stay_straight_under_every_start() {
        use crate::settings::WeekStart;
        use jiff::{civil::Weekday, Timestamp};

        for (start, expected) in [
            (WeekStart::Monday, [5usize, 6, 12, 13, 19, 20, 26, 27]),
            (WeekStart::Sunday, [0, 6, 7, 13, 14, 20, 21, 27]),
            (WeekStart::Saturday, [0, 1, 7, 8, 14, 15, 21, 22]),
        ] {
            let b = assemble_big_year(&[], 2026, 1_786_341_600_000, "Europe/Sofia", start);
            for (r, row) in b.rows.iter().enumerate() {
                let weekend: Vec<usize> = row
                    .days
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| {
                        let wd = Timestamp::from_millisecond(d.start_ms)
                            .unwrap()
                            .in_tz("Europe/Sofia")
                            .unwrap()
                            .weekday();
                        wd == Weekday::Saturday || wd == Weekday::Sunday
                    })
                    .map(|(i, _)| i)
                    .collect();
                assert_eq!(weekend, expected, "{start:?} row {r} weekend columns drifted");
            }
            // And the UI reads the same columns off the index alone, without
            // consulting a date — the two must agree or the shading paints
            // the wrong cells.
            let from_setting: Vec<usize> = (0..28).filter(|&c| start.is_weekend_column(c)).collect();
            assert_eq!(from_setting, expected, "{start:?}: index rule disagrees with the dates");
        }
    }

    #[test]
    fn every_row_puts_its_weekends_in_the_same_columns() {
        // This is the entire reason rows are 28 days and not the reference
        // image's 29: at 28 the weekend columns are constant, so the shading
        // reads as straight vertical stripes instead of drifting diagonally.
        // A later "tidy-up" to 29 would break exactly this.
        use jiff::{civil::Weekday, Timestamp};
        let b = assemble_big_year(&[], 2026, 1_786_341_600_000, "Europe/Sofia", crate::settings::WeekStart::Monday);
        let expected = [5usize, 6, 12, 13, 19, 20, 26, 27];
        for (r, row) in b.rows.iter().enumerate() {
            let weekend: Vec<usize> = row
                .days
                .iter()
                .enumerate()
                .filter(|(_, d)| {
                    let wd = Timestamp::from_millisecond(d.start_ms)
                        .unwrap()
                        .in_tz("Europe/Sofia")
                        .unwrap()
                        .weekday();
                    wd == Weekday::Saturday || wd == Weekday::Sunday
                })
                .map(|(i, _)| i)
                .collect();
            assert_eq!(weekend, expected, "row {r} weekend columns drifted");
        }
    }

    #[test]
    fn a_span_crossing_a_row_boundary_splits_and_both_halves_know_it() {
        // 28-day rows guarantee this happens; `pack_lanes` already sets
        // `cont_left`/`cont_right` when it clips, so the renderer's `‹`
        // marker is a flag being read, not recomputed.
        // Sun 25 Jan .. Tue 27 Jan 2026 inclusive, so the end is Wed 28 at
        // 00:00 — Google's all-day end is exclusive. Row 0 ends on Sun 25
        // Jan, so this straddles the row 0/1 boundary by construction.
        let ev = vec![all_day_event("Europe/Sofia", 1769292000000, 1769551200000)];
        let b = assemble_big_year(&ev, 2026, 1_786_341_600_000, "Europe/Sofia", crate::settings::WeekStart::Monday);
        let r0: Vec<_> = b.rows[0].pills.iter().collect();
        let r1: Vec<_> = b.rows[1].pills.iter().collect();
        assert_eq!(r0.len(), 1, "row 0 carries the Sunday tail");
        assert_eq!(r1.len(), 1, "row 1 carries the Mon-Tue head");
        assert!(r0[0].cont_right, "row 0's half continues past the row");
        assert!(r1[0].cont_left, "row 1's half began before the row");
    }

    #[test]
    fn only_all_day_and_multi_day_events_reach_the_ribbon() {
        // The multi-day one is here for the reason
        // `only_all_day_events_dot_the_year_grid` spells out: read as an
        // all-day span, an hour-long event yields an inverted column range that
        // `pack_lanes` discards anyway, so it stopped witnessing the
        // `is_all_day` guard once pills were placed by date.
        let timed = vec![
            timed_event(1_786_341_600_000, 1_786_341_600_000 + 3_600_000),
            timed_event(
                midnight_ms("UTC", 2026, 8, 5) + 9 * 3_600_000,
                midnight_ms("UTC", 2026, 8, 8) + 17 * 3_600_000,
            ),
        ];
        let b = assemble_big_year(&timed, 2026, 1_786_341_600_000, "Europe/Sofia", crate::settings::WeekStart::Monday);
        assert!(b.rows.iter().all(|r| r.pills.is_empty()));
    }

    /// August 2026, the "now" every year and ribbon test here shares.
    const NOW_AUG_2026: i64 = 1_786_341_600_000;
    const SOFIA: &str = "Europe/Sofia";

    /// Every dotted day in the whole year, as `(month, day)`. The *whole* year
    /// deliberately: the year grid's defect was an extra dot, so a claim about
    /// it has to be a claim about how many there are and nowhere else, not
    /// about one month in isolation.
    fn dotted_days(y: &YearPayload) -> Vec<(u32, u32)> {
        y.months
            .iter()
            .flat_map(|m| m.days.iter().filter(|d| d.has_all_day).map(|d| (m.month, d.day)))
            .collect()
    }

    /// Every placed pill in the ribbon, as `(row, lane)`. Same reasoning as
    /// `dotted_days`: the ribbon drew a one-day event as two pills in adjacent
    /// rows, so the count across all fourteen is the claim.
    fn ribbon_pills(b: &BigYearPayload) -> Vec<(usize, Lane)> {
        b.rows
            .iter()
            .enumerate()
            .flat_map(|(r, row)| row.pills.iter().map(move |l| (r, *l)))
            .collect()
    }

    /// Where the ribbon puts the day beginning at `start_ms`, as `(row, col)`.
    /// Found from the day cells themselves — which carry their own instants and
    /// know nothing about pills — so a pill assertion built on it is not
    /// checking placement against itself.
    fn ribbon_day_at(b: &BigYearPayload, start_ms: i64) -> (usize, usize) {
        for (r, row) in b.rows.iter().enumerate() {
            if let Some(c) = row.days.iter().position(|d| d.start_ms == start_ms) {
                return (r, c);
            }
        }
        panic!("no ribbon day begins at {start_ms}");
    }

    /// The year grid's half of this plan's defect, and the worst-behaved half.
    ///
    /// It dotted a day when the event's stored *instant range* overlapped that
    /// day's bounds in the display zone. Auckland is UTC+12 in August, so a
    /// one-day event on 10 Aug is stored 2026-08-09T12:00Z .. 2026-08-10T12:00Z
    /// and overlaps **two** UTC days — 9 Aug from noon and 10 Aug until noon —
    /// so a single day came out as two dots rather than one dot a day late.
    /// Hence the assertion is on the dotted days of the entire year.
    #[test]
    fn the_year_grid_dots_one_day_for_a_one_day_event_from_a_foreign_zone() {
        let start = midnight_ms(AUCKLAND, 2026, 8, 10);
        let end = midnight_ms(AUCKLAND, 2026, 8, 11); // Google's end is exclusive

        // The fixture's premise: the stored span must straddle a display-zone
        // midnight, or there is no second dot to be rid of.
        assert_eq!(start, 1_786_276_800_000, "2026-08-10 00:00 Auckland is 2026-08-09T12:00Z");
        assert_eq!(crate::write::date_in_zone(start, AUCKLAND), "2026-08-10");
        assert_eq!(
            crate::write::date_in_zone(start, "UTC"),
            "2026-08-09",
            "the display zone must read the stored instant on the previous day"
        );
        assert_eq!(
            crate::write::date_in_zone(end - 1, "UTC"),
            "2026-08-10",
            "and the span must reach into the next display day, which is the second dot"
        );

        let y = assemble_year(&[all_day_event(AUCKLAND, start, end)], 2026, NOW_AUG_2026, "UTC", crate::settings::WeekStart::Monday);

        assert_eq!(dotted_days(&y), vec![(8, 10)], "one day covered, one dot in the year");
        assert_eq!(
            y.months[7].days[9].start_ms,
            midnight_ms("UTC", 2026, 8, 10),
            "and days[9] of August really is the 10th"
        );
    }

    /// The display side of the year grid's comparison, which a UTC display
    /// cannot witness: with a UTC display a day's date is the same string
    /// whichever zone it is read in, so a grid that stopped reading the display
    /// zone at all would go unnoticed. Sofia's Monday midnight is
    /// 2026-08-09T21:00Z — still Sunday in UTC.
    ///
    /// It is also the defect in the pair of zones it was reported in.
    #[test]
    fn the_year_grid_reads_its_own_days_dates_in_the_display_zone() {
        let start = midnight_ms(AUCKLAND, 2026, 8, 10);
        let end = midnight_ms(AUCKLAND, 2026, 8, 11);

        assert_eq!(crate::write::date_in_zone(start, AUCKLAND), "2026-08-10");
        assert_eq!(
            crate::write::date_in_zone(start, SOFIA),
            "2026-08-09",
            "the display zone reads the stored instant as the day before"
        );
        assert_eq!(
            crate::write::date_in_zone(midnight_ms(SOFIA, 2026, 8, 10), "UTC"),
            "2026-08-09",
            "and Sofia's Monday midnight is still Sunday in UTC, so a day's date must be read in Sofia"
        );

        let y = assemble_year(&[all_day_event(AUCKLAND, start, end)], 2026, NOW_AUG_2026, SOFIA, crate::settings::WeekStart::Monday);

        assert_eq!(dotted_days(&y), vec![(8, 10)]);
        assert_eq!(y.months[7].days[9].start_ms, midnight_ms(SOFIA, 2026, 8, 10));
    }

    /// The end of a span in the year grid. `AUCKLAND` cannot witness it — at
    /// UTC+12 a millisecond before the stored exclusive end is still the same
    /// UTC day the calendar's own zone names, so the old arithmetic and the
    /// date derivation agree there. A calendar *west* of the display separates
    /// them: a New York span through Fri 7 Aug is stored to 2026-08-08T04:00Z,
    /// which overlaps Saturday and dotted it.
    #[test]
    fn the_year_grid_stops_dotting_after_a_spans_last_day() {
        let start = midnight_ms(NEW_YORK, 2026, 8, 5);
        let end = midnight_ms(NEW_YORK, 2026, 8, 8);

        assert_eq!(
            crate::write::date_in_zone(end - 1, "UTC"),
            "2026-08-08",
            "a millisecond before the stored end is Saturday in the display zone"
        );
        assert_eq!(
            crate::write::date_in_zone(end, NEW_YORK),
            "2026-08-08",
            "while the calendar's exclusive end is the 8th, so its last covered day is the 7th"
        );

        let y = assemble_year(&[all_day_event(NEW_YORK, start, end)], 2026, NOW_AUG_2026, "UTC", crate::settings::WeekStart::Monday);

        assert_eq!(
            dotted_days(&y),
            vec![(8, 5), (8, 6), (8, 7)],
            "Wed 5 to Fri 7 Aug inclusive — not Sat 8, which the stored instant overlaps"
        );
    }

    /// The ribbon's half of this plan's defect. Same shape as the month grid's:
    /// an Auckland event on Mon 10 Aug is stored 2026-08-09T12:00Z, which falls
    /// inside the *previous* 28-day row, so the pill was drawn once at the end
    /// of that row and again — flagged as continuing — at the start of the next.
    ///
    /// This is also the ribbon's only witness for the **display** side of the
    /// comparison, which is why the display is Sofia and not UTC: with a UTC
    /// display a column's date is the same in every zone.
    #[test]
    fn the_ribbon_places_an_all_day_event_on_its_own_calendars_date() {
        let start = midnight_ms(AUCKLAND, 2026, 8, 10);
        let end = midnight_ms(AUCKLAND, 2026, 8, 11);

        // All three legs of the premise.
        assert_eq!(crate::write::date_in_zone(start, AUCKLAND), "2026-08-10");
        assert_eq!(
            crate::write::date_in_zone(start, SOFIA),
            "2026-08-09",
            "the display zone reads the stored instant as the day before"
        );
        assert_eq!(
            crate::write::date_in_zone(midnight_ms(SOFIA, 2026, 8, 10), "UTC"),
            "2026-08-09",
            "and Sofia's Monday midnight is still Sunday in UTC, so column dates must be read in Sofia"
        );

        let b = assemble_big_year(
            &[all_day_event(AUCKLAND, start, end)],
            2026,
            NOW_AUG_2026,
            SOFIA,
            crate::settings::WeekStart::Monday,
        );

        // The ribbon opens Mon 29 Dec 2025, so Mon 10 Aug 2026 is day 224 —
        // row 8, column 0, the first cell of a row. Pinned absolutely as well
        // as located, so the search cannot drift.
        assert_eq!(ribbon_day_at(&b, midnight_ms(SOFIA, 2026, 8, 10)), (8, 0));

        let pills = ribbon_pills(&b);
        assert_eq!(pills.len(), 1, "one day covered, one pill in the whole ribbon");
        let (row, pill) = pills[0];
        assert_eq!(row, 8, "the row containing Mon 10 Aug");
        assert_eq!(pill.start_col, 0, "Mon 10 Aug is that row's first column");
        assert_eq!(pill.end_col, 0, "a one-day event covers one column");
        assert!(!pill.cont_left, "it did not begin in the row before");
        assert!(!pill.cont_right);
        assert!(b.rows[7].pills.is_empty(), "the row the stored instant falls in drew a pill");
    }

    /// The ribbon's own witness for the end of a span, for the reason
    /// `the_year_grid_stops_dotting_after_a_spans_last_day` gives: an Auckland
    /// fixture cannot separate the two derivations there.
    #[test]
    fn the_ribbon_reads_the_last_day_of_a_span_from_the_calendars_zone() {
        let start = midnight_ms(NEW_YORK, 2026, 8, 5);
        let end = midnight_ms(NEW_YORK, 2026, 8, 8);

        assert_eq!(
            crate::write::date_in_zone(end - 1, "UTC"),
            "2026-08-08",
            "a millisecond before the stored end is Saturday in the display zone"
        );
        assert_eq!(crate::write::date_in_zone(end, NEW_YORK), "2026-08-08");

        let b =
            assemble_big_year(&[all_day_event(NEW_YORK, start, end)], 2026, NOW_AUG_2026, "UTC", crate::settings::WeekStart::Monday);

        // Wed 5 Aug is day 219 of a ribbon opening Mon 29 Dec 2025 — row 7,
        // column 23 — and Fri 7 Aug is column 25 of that same row.
        assert_eq!(ribbon_day_at(&b, midnight_ms("UTC", 2026, 8, 5)), (7, 23));
        assert_eq!(ribbon_day_at(&b, midnight_ms("UTC", 2026, 8, 7)), (7, 25));

        let pills = ribbon_pills(&b);
        assert_eq!(pills.len(), 1, "one span, one pill");
        let (row, pill) = pills[0];
        assert_eq!(row, 7);
        assert_eq!(pill.start_col, 23, "Wed 5 Aug");
        assert_eq!(pill.end_col, 25, "Fri 7 Aug — not Sat 8, which the stored instant reaches");
        assert!(!pill.cont_right);
    }
}

#[cfg(test)]
mod shifted_day_tests {
    use super::*;
    use jiff::civil::date;

    /// Sofia springs forward on 29 Mar 2026, so the day before 30 Mar is 23
    /// hours long: a walk by wall-clock days lands on its real midnight, a
    /// walk by 86 400 000 would land an hour into it.
    #[test]
    fn a_day_back_across_a_dst_change_is_the_real_midnight() {
        let tz = "Europe/Sofia";
        let mon = local_midnight_ms(date(2026, 3, 30), tz);
        let sun = local_midnight_ms(date(2026, 3, 29), tz);
        assert_eq!(day_start_shifted(mon, -1, tz), sun);
        assert_eq!(mon - sun, 23 * 3_600_000, "the DST day is 23 hours, not 24");
        assert_eq!(day_start_shifted(sun, 1, tz), mon);
        // A week back, straight through it.
        assert_eq!(day_start_shifted(mon, -7, tz), local_midnight_ms(date(2026, 3, 23), tz));
    }

    #[test]
    fn an_unknown_zone_still_yields_a_boundary() {
        assert_eq!(day_start_shifted(1_000 * DAY_MS, -3, "Mars/Olympus"), 997 * DAY_MS);
    }
}

#[cfg(test)]
mod all_guests_declined_tests {
    use super::*;
    use omacal_store::Attendee;

    fn guest(response: &str, is_self: bool) -> Attendee {
        Attendee {
            email: format!("{response}-{is_self}@x.com"),
            display_name: None,
            response_status: response.into(),
            optional: false,
            is_self,
            comment: None,
            additional_guests: 0,
        }
    }

    fn with(attendees: Vec<Attendee>) -> bool {
        let mut src = omacal_store::StoredEvent {
            id: 1, calendar_id: 1, google_id: "g".into(), summary: Some("Meeting".into()),
            location: None, start_utc: 0, end_utc: 3_600_000,
            start_tz: "UTC".into(), end_tz: "UTC".into(),
            is_all_day: false, recurrence: None,
            recurring_event_id: None, original_start_utc: None,
            status: "confirmed".into(), self_response: Some("accepted".into()),
            conference_uri: None, color_hex: None, calendar_timezone: "UTC".into(),
            description: None, etag: None, sequence: 0, organizer_email: None,
            guests_can_modify: false, attendees: Vec::new(),
            reminders: Default::default(), calendar_default_reminders: Vec::new(),
        };
        src.attendees = attendees;
        to_ui(&src, src.start_utc, src.end_utc).all_guests_declined
    }

    /// The reported case: a 1:1 the user organised and accepted, and the one
    /// guest said no. Nothing on the block said so before this flag.
    #[test]
    fn one_guest_who_declined_is_everyone() {
        assert!(with(vec![guest("accepted", true), guest("declined", false)]));
    }

    /// One "no" among several is not this. Partial declines live in the guest
    /// list, where each is named; marking the block for them would put a
    /// strike through half a busy week.
    #[test]
    fn one_no_among_others_is_not() {
        assert!(!with(vec![
            guest("accepted", true),
            guest("declined", false),
            guest("accepted", false),
        ]));
        assert!(!with(vec![guest("accepted", true), guest("needsAction", false)]));
    }

    /// **Your own no is not everyone's.** It already hollows the block and
    /// strikes the title; setting this as well would mark every event you
    /// have ever declined as abandoned by its guests.
    #[test]
    fn the_users_own_decline_is_excluded() {
        assert!(!with(vec![guest("declined", true)]));
        assert!(!with(vec![guest("declined", true), guest("accepted", false)]));
    }

    /// A solo event has nobody to decline it. `attendees` is empty for one,
    /// and an empty list must not read as "all of them said no".
    #[test]
    fn a_solo_event_is_never_marked() {
        assert!(!with(Vec::new()));
    }
}
