//! "Snooze 5m": a reminder posted again five minutes after the click.
//!
//! In memory only, deliberately. A snooze is a five-minute promise, and one
//! that does not survive the app quitting inside those five minutes is the
//! same case as the app having been shut when a reminder fell due.
//!
//! The queue holds **which** reminder and **when**, never what it said. When a
//! snooze falls due, the reminder loop derives that reminder again from the
//! calendar as it is now (`notify_loop::run_events`). So an event deleted,
//! declined, hidden or already over in the meantime posts nothing, and one
//! that was retitled says its new title. `fired_reminders` is not touched: the
//! reminder was posted and recorded once, and that record is still what keeps
//! the ordinary pass from posting it again.
//!
//! The deadline is a wall-clock instant, compared against the loop's own
//! `now_ms`. The loop's sleep is a monotonic timer, and on macOS that timer
//! stops while the machine sleeps. So while a snooze waits, the loop never
//! sleeps longer than [`MAX_SLEEP_WHILE_SNOOZED_MS`]; after a lid opens, the
//! snooze is at most that late.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use omacal_core::remind::FiredKey;

/// How long a snooze lasts.
pub(crate) const SNOOZE_MS: i64 = 5 * 60_000;

/// The longest the reminder loop sleeps while a snooze is waiting.
pub(crate) const MAX_SLEEP_WHILE_SNOOZED_MS: i64 = 60_000;

/// Reminders waiting to be posted again, each with the wall-clock instant it
/// is due.
#[derive(Default)]
pub(crate) struct SnoozeQueue {
    due: Mutex<HashMap<FiredKey, i64>>,
    wake: tokio::sync::Notify,
}

impl SnoozeQueue {
    /// Posts `key` again [`SNOOZE_MS`] after `now_ms`. Snoozing it again
    /// moves the deadline rather than adding a second one. Wakes the loop, so
    /// a long sleep towards the next sync does not swallow the snooze.
    pub(crate) fn snooze(&self, key: FiredKey, now_ms: i64) {
        self.due.lock().expect("snooze queue poisoned").insert(key, now_ms + SNOOZE_MS);
        self.wake.notify_one();
    }

    /// The snoozes due by `now_ms`, removed from the queue.
    pub(crate) fn take_due(&self, now_ms: i64) -> Vec<FiredKey> {
        let mut due = self.due.lock().expect("snooze queue poisoned");
        let ready: Vec<FiredKey> = due.iter().filter(|(_, &at)| at <= now_ms).map(|(k, _)| *k).collect();
        for k in &ready {
            due.remove(k);
        }
        ready
    }

    /// The earliest deadline still waiting.
    pub(crate) fn next_at(&self) -> Option<i64> {
        self.due.lock().expect("snooze queue poisoned").values().copied().min()
    }

    /// Resolves when a snooze is added.
    pub(crate) async fn woken(&self) {
        self.wake.notified().await;
    }
}

/// The queue the running app's reminder loop reads.
pub(crate) fn queue() -> &'static SnoozeQueue {
    static QUEUE: OnceLock<SnoozeQueue> = OnceLock::new();
    QUEUE.get_or_init(SnoozeQueue::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(minutes: i64) -> FiredKey {
        FiredKey { event_id: 1, occurrence_ms: 1_000_000, minutes }
    }

    #[test]
    fn a_snooze_is_due_five_minutes_on_and_taken_once() {
        let q = SnoozeQueue::default();
        q.snooze(key(10), 0);
        assert_eq!(q.next_at(), Some(SNOOZE_MS));
        assert!(q.take_due(SNOOZE_MS - 1).is_empty());
        assert_eq!(q.take_due(SNOOZE_MS), vec![key(10)]);
        assert!(q.take_due(SNOOZE_MS + 1).is_empty(), "taken means gone");
        assert_eq!(q.next_at(), None);
    }

    #[test]
    fn snoozing_again_moves_the_deadline_rather_than_adding_one() {
        let q = SnoozeQueue::default();
        q.snooze(key(10), 0);
        q.snooze(key(10), 60_000);
        assert_eq!(q.next_at(), Some(60_000 + SNOOZE_MS));
        assert_eq!(q.take_due(i64::MAX).len(), 1);
    }
}
