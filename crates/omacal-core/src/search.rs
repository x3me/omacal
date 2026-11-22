//! Which occurrence a search result resolves to, and what order results come
//! back in.
//!
//! Pure, and the clock is a parameter — exactly as [`crate::remind`] takes
//! `now_ms`. Nothing here reads a clock, touches a database or reaches the
//! network; search is a lookup against data that is already local (spec §7),
//! and the part with judgement in it is the arithmetic below.

use crate::layout::Interval;

/// How far `interval` starts from `now_ms`, in either direction.
///
/// **Absolute distance, not a signed difference**, which is the whole of spec
/// §4: a trip last month and a trip next month are both more likely to be what
/// you meant than one from four years ago. Ordering by the raw difference
/// gives soonest-first and buries everything in the past.
fn distance(interval: &Interval, now_ms: i64) -> i64 {
    (interval.start_ms - now_ms).abs()
}

/// The occurrence of a series nearest `now_ms`, or `None` when there are none.
///
/// **Spec §3: one result per event, not per occurrence.** A weekly standup is
/// one row and hundreds of occurrences; returning fifty-two identical results
/// would bury everything else. So a recurring event appears once, resolved to
/// the occurrence you almost certainly mean — the nearest one, whether that is
/// last Tuesday's or tomorrow's.
///
/// Two answers this deliberately is not. Not the **first** occurrence in the
/// window, which is a different thing whenever the window opens before today.
/// And not the **master's own `DTSTART`**, which for a series running since
/// 2019 is four years from anything the user is looking for — that one passes
/// a naive test, because for a series that starts today the two coincide.
///
/// Ties go to the **earlier** occurrence. A tie needs two occurrences exactly
/// equidistant either side of the clock, which a real calendar reaches only by
/// coincidence; picking deliberately means the answer does not depend on the
/// order the expansion happened to produce.
pub fn nearest(occurrences: &[Interval], now_ms: i64) -> Option<Interval> {
    occurrences
        .iter()
        .copied()
        .reduce(|best, iv| {
            let (d_best, d_iv) = (distance(&best, now_ms), distance(&iv, now_ms));
            if d_iv < d_best || (d_iv == d_best && iv.start_ms < best.start_ms) {
                iv
            } else {
                best
            }
        })
}

/// One search result: an event, and the occurrence of it being offered.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    /// The store row's id — the event, not the occurrence. Every result is one
    /// of these, however many times the event happens.
    pub event_id: i64,
    pub title: String,
    /// The occurrence this result resolves to, which for a recurring event is
    /// [`nearest`]'s answer and for a one-off is its own span.
    pub start_ms: i64,
    pub end_ms: i64,
    /// For an all-day occurrence, its civil date (`2026-08-05`) in its own
    /// calendar's zone, which is what the result names and where a jump
    /// lands. Read from the instant in the machine's zone instead, a calendar
    /// ahead of it showed the day before. `None` for a timed occurrence.
    pub all_day_date: Option<String>,
}

/// `hits`, ordered nearest to `now_ms` first (spec §4).
///
/// **By distance in either direction**, not by date: a stable sort on the
/// absolute gap. Where two results are equidistant the earlier one comes first,
/// for the same reason [`nearest`] breaks its tie that way — an order that
/// depends on which row the database happened to return is not an order.
pub fn by_distance(hits: &mut [Hit], now_ms: i64) {
    hits.sort_by_key(|h| ((h.start_ms - now_ms).abs(), h.start_ms));
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 24 * HOUR;
    /// Monday 10 Aug 2026, midday UTC. A fixed clock, because "nearest today"
    /// is the thing under test and a real one would make these pass or fail by
    /// the date they were run.
    const NOW: i64 = 1_786_017_600_000;

    fn at(offset: i64) -> Interval {
        Interval { start_ms: NOW + offset, end_ms: NOW + offset + HOUR }
    }

    #[test]
    fn no_occurrences_resolve_to_nothing() {
        assert_eq!(nearest(&[], NOW), None);
    }

    /// The rule, from the past side — and this is the case that catches
    /// "return the first one in the window".
    #[test]
    fn the_nearest_occurrence_wins_over_the_first() {
        let occurrences = [at(-30 * DAY), at(-7 * DAY), at(-DAY), at(21 * DAY)];
        assert_eq!(nearest(&occurrences, NOW), Some(at(-DAY)));
        assert_ne!(
            nearest(&occurrences, NOW),
            Some(at(-30 * DAY)),
            "the first occurrence in the window is a different answer",
        );
    }

    /// And from the future side, so "nearest" cannot be satisfied by "the last
    /// one before now".
    #[test]
    fn an_occurrence_after_now_can_be_the_nearest() {
        let occurrences = [at(-9 * DAY), at(2 * DAY), at(40 * DAY)];
        assert_eq!(nearest(&occurrences, NOW), Some(at(2 * DAY)));
    }

    /// **Either direction, and the past can win.** Without a case where the
    /// past is nearer, an implementation that always prefers the future passes
    /// everything above.
    #[test]
    fn the_past_wins_when_it_is_nearer() {
        assert_eq!(nearest(&[at(-HOUR), at(5 * DAY)], NOW), Some(at(-HOUR)));
    }

    #[test]
    fn an_exact_tie_takes_the_earlier_one() {
        // Deliberate rather than incidental: otherwise the answer depends on
        // the order the expansion happened to produce.
        assert_eq!(nearest(&[at(3 * DAY), at(-3 * DAY)], NOW), Some(at(-3 * DAY)));
    }

    fn hit(id: i64, offset: i64) -> Hit {
        Hit {
            event_id: id,
            title: format!("event {id}"),
            start_ms: NOW + offset,
            end_ms: NOW + offset + HOUR,
            all_day_date: None,
        }
    }

    /// **Spec §4, and the fixture is on both sides of today on purpose**:
    /// with everything in the future, nearest-first and soonest-first are the
    /// same order and neither is being tested.
    #[test]
    fn results_are_ordered_by_distance_in_either_direction() {
        let mut hits = vec![
            hit(1, 400 * DAY),
            hit(2, -2 * DAY),
            hit(3, 30 * DAY),
            hit(4, -365 * DAY),
            hit(5, DAY),
        ];
        by_distance(&mut hits, NOW);

        assert_eq!(
            hits.iter().map(|h| h.event_id).collect::<Vec<_>>(),
            vec![5, 2, 3, 4, 1],
            "nearest first, and a day ago beats a month away",
        );
    }

    /// The same list ordered by *date* comes out differently, which is what
    /// makes the assertion above about distance rather than about sorting at
    /// all.
    #[test]
    fn distance_order_is_not_date_order() {
        let mut hits = vec![hit(1, -2 * DAY), hit(2, DAY)];
        by_distance(&mut hits, NOW);
        assert_eq!(hits[0].event_id, 2, "tomorrow is nearer than the day before yesterday");

        let mut by_date = [hit(1, -2 * DAY), hit(2, DAY)];
        by_date.sort_by_key(|h| h.start_ms);
        assert_eq!(by_date[0].event_id, 1, "date order would answer the other way");
    }

    #[test]
    fn an_empty_result_set_orders_without_complaint() {
        let mut hits: Vec<Hit> = Vec::new();
        by_distance(&mut hits, NOW);
        assert!(hits.is_empty());
    }
}
