//! Display grouping only: every source event keeps its own identity and writes.
use crate::commands::UiEvent;
use omacal_core::Segment;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize)]
pub struct EventCopy {
    pub id: i64,
    pub calendar_id: i64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub color: String,
}

#[derive(Hash, PartialEq, Eq)]
enum Span {
    Timed(i64, i64),
    AllDay(String, String),
}

fn copy(event: &UiEvent) -> EventCopy {
    EventCopy { id: event.id, calendar_id: event.calendar_id,
        start_ms: event.start_ms, end_ms: event.end_ms, color: event.color.clone() }
}

// A duplicated conference is stored as a manual URL in Location. Compare its
// physical place with the source's place, not with the appended call label.
fn place(raw: Option<&str>) -> &str {
    let raw = raw.unwrap_or("").trim();
    for label in ["Google Meet: ", "Zoom: ", "Video call: "] {
        let (location, link) = if let Some(link) = raw.strip_prefix(label) {
            ("", link)
        } else if let Some(pair) = raw.rsplit_once(&format!(" · {label}")) {
            pair
        } else { continue; };
        if crate::upcoming::location_meeting_url(Some(link)).is_some() { return location.trim(); }
    }
    raw
}

/// Return the original-index -> displayed-index mapping for lane repacking.
pub fn combine(events: &mut Vec<UiEvent>) -> Vec<usize> {
    let mut groups: HashMap<(String, String, Span), Vec<usize>> = HashMap::new();
    let mut displayed: Vec<UiEvent> = Vec::new();
    let mut mapping = Vec::with_capacity(events.len());
    for event in std::mem::take(events) {
        let span = match &event.all_day_dates {
            Some((first, last)) => Span::AllDay(first.clone(), last.clone()),
            None => Span::Timed(event.start_ms, event.end_ms),
        };
        let candidates = groups.entry((event.title.trim().to_owned(), place(event.location.as_deref()).to_owned(), span)).or_default();
        // Two identical events on the same calendar remain independently
        // visible. A third calendar can join one group, never both.
        let target = candidates.iter().copied().find(|&i| {
            let current = &displayed[i];
            current.calendar_id != event.calendar_id
                && !current.copies.iter().any(|c| c.calendar_id == event.calendar_id)
        });
        if let Some(i) = target {
            let current = &mut displayed[i];
            if current.copies.is_empty() { current.copies.push(copy(current)); }
            current.copies.push(copy(&event));
            mapping.push(i);
        } else {
            let i = displayed.len();
            candidates.push(i);
            mapping.push(i);
            displayed.push(event);
        }
    }
    *events = displayed;
    mapping
}

pub fn combine_lanes(events: &mut Vec<UiEvent>, segments: &mut Vec<Segment>) {
    let mapping = combine(events);
    let mut seen = HashSet::new();
    segments.retain_mut(|segment| {
        segment.idx = mapping[segment.idx];
        seen.insert(segment.idx)
    });
}
