//! The transport, behind a trait.
//!
//! Two halves, split so only one of them is untestable.
//!
//! [`notification_for_format`] decides *what a reminder says* — title, body, which
//! actions. It is a pure function of a [`Due`] and a zone, so every rule about
//! wording and about when a *Join* button appears is tested without anything
//! being posted anywhere.
//!
//! [`Notifier`] is *how it leaves the process*, and its implementations are the
//! one seam this feature cannot test. `notify-rust` talks to D-Bus, a signed
//! macOS bundle talks to `UNUserNotificationCenter` (`notify_mac.rs`), and an
//! unbundled macOS run falls back to `tauri-plugin-notification`; all of them
//! would put something in the user's notification centre if a test ran them.
//! **The only notifier the suite ever sees is [`RecordingNotifier`].** That is
//! stated here rather than hidden, per spec §3.

use omacal_core::remind::{Due, DueTask};
use crate::settings::TimeFormat;

/// What a posted notification says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notification {
    pub title: String,
    /// One line, read at a glance. The popover is where detail lives.
    pub body: String,
    pub actions: Vec<Action>,
    /// Stay on screen until acted on or dismissed, instead of expiring.
    ///
    /// For a notification whose click *does* something, expiry is loss: the
    /// invitation toast evaporated after a few seconds while the user read
    /// another window, and once a popup ages into Omarchy's history its
    /// actions are dead (2026-08-17, missed live during the first click
    /// test). Omarchy's shell clamps requested timeouts to 30s and never
    /// expires only `urgency=critical`, so that is what this maps to on the
    /// wire — `Timeout::Never` rides along for daemons that honour it.
    pub sticky: bool,
}

/// A button on a notification.
///
/// Serde, because macOS has no equivalent of the D-Bus action key riding back
/// with a click: `UNNotificationResponse` returns the *request identifier* and
/// the *action identifier*, and everything else the handler needs must have
/// been carried by the notification itself. The actions serialize into the
/// request identifier at post ([`un_payload`]) and deserialize out of it at
/// click ([`action_for_un`]), so the payload format is this enum's own derive
/// — one representation, no hand-rolled second one to drift.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Action {
    /// Open the occurrence's conferencing link. Carries the URI, so nothing
    /// downstream has to find it again — and **only exists when there is one**.
    Join(String),
    /// Post this reminder again five minutes out (§2.5), through
    /// [`crate::snooze`]. Carries the reminder's own [`FiredKey`] fields,
    /// because the click comes back with nothing else, and the loop needs them
    /// to derive the same reminder again from the calendar as it is by then.
    ///
    /// The missed rule does **not** bring a snoozed reminder back on its own:
    /// it was recorded as fired when it was posted, and a recorded reminder is
    /// never offered again. Until the snooze queue existed, this button
    /// dismissed the reminder and did nothing else.
    Snooze5m { event_id: i64, occurrence_ms: i64, minutes: i64 },
    /// Accept the invitation this notification announces, for the whole
    /// series (`respond_to_event`'s scope `all` — what answering the email
    /// would do).
    ///
    /// Registered under the D-Bus key `default`, which is the click itself:
    /// Omarchy's shell renders no action buttons and invokes exactly one
    /// action, `default`, on left-click (right-click dismisses). One action,
    /// not three — a Maybe or a No stays a considered answer in the popover,
    /// while a click on "Invitation: …" is the yes it says it is.
    ///
    /// `start_ms` is the master's own start, carried so the handler needs no
    /// re-read; scope `all` never resolves an instance, so any occurrence
    /// anchor satisfies `respond_to_event_impl`.
    AcceptInvite { event_id: i64, start_ms: i64 },
    /// Show the app on this occurrence — a reminder's `default`, the click
    /// itself. Until this existed, left-clicking a plain reminder on
    /// Omarchy did nothing at all: the shell invokes only `default`, and
    /// reminders registered Join and Snooze on keys it never sends. The
    /// click means "show me", never anything with side effects — a reminder
    /// is not a question the way an invitation is, so its click must not
    /// answer one. Carries the occurrence whole (id, start, end) for the
    /// same reason `Due` does: nothing downstream searches its way back.
    OpenEvent { event_id: i64, start_ms: i64, end_ms: i64 },
    /// Show the app with the task this announces (#137) — a task
    /// announcement's `default`, the click itself, and its only action. The
    /// click means "show me", never "done": completing a task is an answer,
    /// and an answer belongs where it can be seen and undone.
    OpenTask { task_id: i64 },
}

/// What a clicked reminder emits, carrying the occurrence (`id`, `startMs`,
/// `endMs`) for the webview to land on and open — the same three numbers a
/// chosen search hit navigates by.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
pub(crate) const OPEN_EVENT_EVENT: &str = "open-event";

/// What a clicked task announcement emits: the task to show (#137). The
/// window opens its Tasks pane on it.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
pub(crate) const OPEN_TASK_EVENT: &str = "open-task";

/// The D-Bus action key a clicked notification comes back with, resolved to
/// the [`Action`] it stood for — `None` for a dismissal (`__closed`) or a key
/// this notification never offered. Pure, because it is the one decision in
/// the click path and the transport around it cannot be tested at all.
/// macOS resolves through the same keys: [`action_for_un`] normalises Apple's
/// action identifiers to these before calling here, so both platforms answer
/// a click from one tested table.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
pub(crate) fn action_for_key(actions: &[Action], key: &str) -> Option<Action> {
    actions
        .iter()
        .find(|a| {
            matches!(
                (a, key),
                (Action::Join(_), "join")
                    | (Action::Snooze5m { .. }, "snooze")
                    | (Action::AcceptInvite { .. }, "default")
                    | (Action::OpenEvent { .. }, "default")
                    | (Action::OpenTask { .. }, "default")
            )
        })
        .cloned()
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum NotifyError {
    /// The transport would not take it. On macOS this is ordinary — see
    /// [`Notifier`] — and on Linux it means no D-Bus notification daemon.
    #[error("the notification transport is unavailable: {0}")]
    Unavailable(String),
}

/// How a notification leaves the process.
///
/// `Send + Sync` because the driver holds one across await points in a spawned
/// task.
///
/// **A failing `post` is not an error the user is shown.** The rule was
/// written when every macOS build was unsigned and `UNUserNotificationCenter`
/// refused it wholesale (§2.4); the signed bundle un-made that case, but the
/// rule stands — a user who declines the permission prompt, or an unbundled
/// dev run, is still a transport that refuses, and the driver's answer is the
/// same: log it, record the reminder as fired anyway, move on. See
/// `notify_loop::run_once` for why recording a failed post is the right trade.
pub(crate) trait Notifier: Send + Sync {
    fn post(&self, n: &Notification) -> Result<(), NotifyError>;
}

/// The fallback title. Google will hold an event with no summary, and the same
/// string is used by the grid (`commands.rs`) and the popover, so one untitled
/// event is described one way wherever it appears.
pub(crate) const NO_TITLE: &str = "(no title)";

/// What one reminder should say, read in `tz` — the **display** zone, the
/// user's own, not the calendar's. The calendar's zone decides *when* a
/// reminder fires; this decides what the user reads, and they read it where
/// they are.
///
/// The body is an absolute time and never "in 10 minutes". A missed reminder is
/// posted late by design (§2.3) — sometimes hours late, after a laptop has been
/// shut — and a relative phrase computed when the notification is built would
/// be a lie by the time it is read. An absolute time is true whenever it
/// arrives.
///
/// An all-day occurrence says so instead of showing a time. Its anchor is
/// midnight in the *calendar's* zone, which rendered in the display zone is
/// some arbitrary hour of the previous or next day — an actively misleading
/// thing to print.
pub(crate) fn notification_for_format(
    d: &Due,
    tz: &str,
    time_format: TimeFormat,
) -> Notification {
    let when = if d.is_all_day {
        "All day".to_string()
    } else {
        time_in_zone_with_format(d.key.occurrence_ms, tz, time_format)
    };

    let body = match &d.location {
        Some(loc) if !loc.is_empty() => format!("{when} · {loc}"),
        _ => when,
    };

    let mut actions = Vec::new();
    // Only when there is somewhere to join. A Join button on a meeting with no
    // conferencing link has nothing to open.
    if let Some(uri) = &d.conference_uri {
        if !uri.is_empty() {
            actions.push(Action::Join(uri.clone()));
        }
    }
    actions.push(Action::Snooze5m {
        event_id: d.key.event_id,
        occurrence_ms: d.key.occurrence_ms,
        minutes: d.key.minutes,
    });
    // The click itself, on every reminder: land the calendar on the
    // occurrence. Join keeps its own button where a daemon renders buttons —
    // a click must never join a meeting on the user's behalf.
    actions.push(Action::OpenEvent {
        event_id: d.key.event_id,
        start_ms: d.key.occurrence_ms,
        end_ms: d.occurrence_end_ms,
    });

    Notification {
        title: d.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| NO_TITLE.into()),
        body,
        actions,
        // A reminder is about a moment; once the moment is near, an expired
        // toast has done its job. Only actionable announcements stick.
        sticky: false,
    }
}

/// What a task announcement says (#137).
///
/// The title is the task; the line under it is the hour it is due, and the
/// list it is on where there is one — `notification_for_format`'s shape, with
/// the list where an event's location goes. An absolute time for the same
/// reason: an announcement may be read minutes after it arrived, and "in 5
/// minutes" would be a lie by then.
///
/// One action, the click: show the task. A "Done" button is deliberately
/// absent — Omarchy's shell invokes only `default`, so a single click would
/// have to be either "show me" or "completed", and completing something by
/// accident from a toast is not a mistake a notification should make possible.
pub(crate) fn task_notification_for_format(
    d: &DueTask,
    tz: &str,
    time_format: TimeFormat,
) -> Notification {
    let when = format!("Due {}", time_in_zone_with_format(d.key.due_ms, tz, time_format));
    let body = match &d.list {
        Some(list) if !list.is_empty() => format!("{when} · {list}"),
        _ => when,
    };
    Notification {
        title: d.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| NO_TITLE.into()),
        body,
        actions: vec![Action::OpenTask { task_id: d.key.task_id }],
        // A task is not a moment the way a meeting is: it waits. But a toast
        // that stays until dismissed is a nag, and the task is still in the
        // pane afterwards, so it behaves as a reminder does.
        sticky: false,
    }
}

/// A clock time for an instant, read in `tz`. Falls back to UTC for an unknown
/// zone rather than panicking, the same policy as
/// `omacal_core::zone::date_in_zone`.
pub(crate) fn time_in_zone_with_format(ms: i64, tz: &str, time_format: TimeFormat) -> String {
    let ts = jiff::Timestamp::from_millisecond(ms).unwrap_or(jiff::Timestamp::UNIX_EPOCH);
    let z = ts.in_tz(tz).unwrap_or_else(|_| ts.in_tz("UTC").expect("UTC always resolves"));
    match time_format {
        TimeFormat::H24 => format!("{:02}:{:02}", z.hour(), z.minute()),
        TimeFormat::H12 => {
            let hour = z.hour();
            let dial = if hour % 12 == 0 { 12 } else { hour % 12 };
            format!("{dial}:{:02} {}", z.minute(), if hour < 12 { "AM" } else { "PM" })
        }
    }
}

/// The pre-preference spelling retained only for the tests whose subject is
/// notification content other than the clock. Production has no default: it
/// must pass the stored preference to `notification_for_format`.
#[cfg(test)]
pub(crate) fn notification_for(d: &Due, tz: &str) -> Notification {
    notification_for_format(d, tz, TimeFormat::H24)
}

/// The category ids registered once with the macOS notification centre.
///
/// Categories are macOS's shape for buttons: a posted notification only
/// *names* a category, and the buttons were declared up front at registration
/// — there is no per-notification button list the way the D-Bus builder takes
/// one. So the button layouts this app can produce are enumerated here as
/// categories, and [`un_category`] picks one per notification.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) const UN_CATEGORY_JOIN: &str = "omacal.reminder.join";
/// A reminder with no conferencing link: Snooze alone.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) const UN_CATEGORY_REMINDER: &str = "omacal.reminder";

/// Which registered category a notification's actions call for — `""` for
/// none (the invitation toast and the plain announcements: their click is
/// their whole interface, and a considered Maybe/No stays in the popover,
/// exactly the Linux shape).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) fn un_category(actions: &[Action]) -> &'static str {
    if actions.iter().any(|a| matches!(a, Action::Join(_))) {
        UN_CATEGORY_JOIN
    } else if actions.iter().any(|a| matches!(a, Action::Snooze5m { .. })) {
        UN_CATEGORY_REMINDER
    } else {
        ""
    }
}

/// What rides in a UN request identifier: the actions, plus a nonce.
///
/// The nonce is not decoration. macOS treats the request identifier as a
/// replacement key — post twice under one identifier and the first
/// notification is silently swapped out. Actions alone are identical for,
/// say, two plain announcements in one sync pass ("3 guests declined",
/// "a meeting moved" — both `[]`), and the second would eat the first.
/// A per-process counter keeps same-session identifiers distinct, while a
/// re-fire of the *same occurrence* across restarts may still replace its
/// stale predecessor — which is the one replacement worth having.
#[derive(serde::Serialize, serde::Deserialize)]
struct UnPayload {
    n: u64,
    a: Vec<Action>,
}

/// The actions serialized for the UN request identifier. `n` is the caller's
/// nonce — the transport counts posts; tests pass what they like.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) fn un_payload(n: u64, actions: &[Action]) -> String {
    serde_json::to_string(&UnPayload { n, a: actions.to_vec() }).unwrap_or_default()
}

/// Apple's own identifier for "the notification body itself was clicked" —
/// a stable ABI string, matched by value because the constant the framework
/// exports is just this string with a symbol name on it.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) const UN_DEFAULT_ACTION: &str = "com.apple.UNNotificationDefaultActionIdentifier";
/// And for an explicit dismissal, which must resolve to no action at all.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) const UN_DISMISS_ACTION: &str = "com.apple.UNNotificationDismissActionIdentifier";

/// The macOS click, resolved to the [`Action`] it stood for — `None` for a
/// dismissal, an unparseable identifier (a notification from an older build,
/// left in Notification Center across an update), or a button this
/// notification never offered. Pure for the reason [`action_for_key`] is,
/// and resolving *through* it: Apple's identifiers are normalised to the
/// D-Bus keys and then both platforms answer from the one tested table.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) fn action_for_un(payload: &str, action_identifier: &str) -> Option<Action> {
    let key = match action_identifier {
        UN_DEFAULT_ACTION => "default",
        UN_DISMISS_ACTION => return None,
        other => other,
    };
    let p: UnPayload = serde_json::from_str(payload).ok()?;
    action_for_key(&p.a, key)
}

/// The only notifier any test uses. Records what it was handed and posts
/// nothing anywhere.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct RecordingNotifier {
    posted: std::sync::Mutex<Vec<Notification>>,
    /// When set, every `post` records the attempt and then fails — standing in
    /// for an unsigned macOS bundle whose notification centre refuses.
    fails: bool,
}

#[cfg(test)]
impl RecordingNotifier {
    pub(crate) fn failing() -> Self {
        Self { fails: true, ..Default::default() }
    }

    pub(crate) fn posted(&self) -> Vec<Notification> {
        self.posted.lock().expect("test notifier mutex").clone()
    }
}

#[cfg(test)]
impl Notifier for RecordingNotifier {
    fn post(&self, n: &Notification) -> Result<(), NotifyError> {
        // Recorded before the failure check, so a test can tell "never tried"
        // from "tried and was refused" — the distinction the macOS path turns
        // on.
        self.posted.lock().expect("test notifier mutex").push(n.clone());
        if self.fails {
            return Err(NotifyError::Unavailable("test notifier set to fail".into()));
        }
        Ok(())
    }
}

/// Omarchy's path: D-Bus `org.freedesktop.Notifications`, via `notify-rust`,
/// with the action buttons that are the reason this is not the Tauri plugin.
///
/// Kept as thin as a wrapper can be: every decision it could get wrong has
/// been moved into [`notification_for`]/[`action_for_key`], which are tested,
/// while this block only carries bytes to the bus and clicks back from it.
///
/// **Clicks flow through `on_action`.** The daemon reports an invoked action
/// as a D-Bus signal for the notification's id, and `notify-rust` surfaces it
/// only to a caller that keeps the handle and blocks on it — so each posted
/// notification that could be acted on parks one thread in
/// `wait_for_action`, which returns on click *or* dismissal. The callback is
/// injected once at startup (`set_on_action`) rather than passed per post,
/// so [`Notifier`]'s signature — and every tested caller of it — stays
/// untouched.
#[cfg(target_os = "linux")]
#[derive(Default)]
pub(crate) struct DbusNotifier {
    on_action: std::sync::OnceLock<std::sync::Arc<dyn Fn(Action) + Send + Sync>>,
}

#[cfg(target_os = "linux")]
impl DbusNotifier {
    /// Injects the click handler. Second and later calls are ignored — one
    /// dispatcher, wired once, in `run()`'s setup.
    pub(crate) fn set_on_action(&self, f: std::sync::Arc<dyn Fn(Action) + Send + Sync>) {
        let _ = self.on_action.set(f);
    }
}

#[cfg(target_os = "linux")]
impl Notifier for DbusNotifier {
    fn post(&self, n: &Notification) -> Result<(), NotifyError> {
        let mut builder = notify_rust::Notification::new();
        builder.summary(&n.title).body(&n.body).appname("OmaCal");
        if n.sticky {
            // See `Notification::sticky` for why this is urgency and not
            // just a timeout. It does not pierce do-not-disturb: Omarchy's
            // bypass admits criticals from `notify-send` alone.
            builder
                .urgency(notify_rust::Urgency::Critical)
                .timeout(notify_rust::Timeout::Never);
        }

        for action in &n.actions {
            match action {
                Action::Join(_) => builder.action("join", "Join"),
                Action::Snooze5m { .. } => builder.action("snooze", "Snooze 5m"),
                Action::AcceptInvite { .. } => builder.action("default", "Accept"),
                Action::OpenEvent { .. } => builder.action("default", "Open"),
                Action::OpenTask { .. } => builder.action("default", "Open"),
            };
        }

        let handle = builder.show().map_err(|e| NotifyError::Unavailable(e.to_string()))?;

        // Dropping the handle leaves the notification up; it only forfeits
        // the click. So the thread is parked only when a click could mean
        // something — actions to hear about and a dispatcher to tell.
        if let Some(cb) = self.on_action.get().cloned() {
            if !n.actions.is_empty() {
                let actions = n.actions.clone();
                std::thread::spawn(move || {
                    handle.wait_for_action(move |key| {
                        if let Some(action) = action_for_key(&actions, key) {
                            cb(action);
                        }
                    });
                });
            }
        }
        Ok(())
    }
}

/// macOS **unbundled** — `cargo tauri dev`, the raw binary — and **allowed
/// to fail** (§2.4).
///
/// The shipping transport is `notify_mac::UnNotifier`; this is what remains
/// for the run it cannot serve. `UNUserNotificationCenter` raises outside a
/// real bundle, so a dev run posts through the plugin instead and may simply
/// be refused. That is expected, not a fault: the error goes back to the
/// driver, which logs it and carries on. It is never surfaced as a banner
/// and never retried into a loop.
///
/// No actions. `tauri-plugin-notification` has no action buttons, which is why
/// Linux uses `notify-rust` and the bundled app uses the centre directly; here
/// the notification is informational and the app is one click away.
#[cfg(target_os = "macos")]
pub(crate) struct MacNotifier {
    pub app: tauri::AppHandle,
}

#[cfg(target_os = "macos")]
impl Notifier for MacNotifier {
    fn post(&self, n: &Notification) -> Result<(), NotifyError> {
        use tauri_plugin_notification::NotificationExt;
        self.app
            .notification()
            .builder()
            .title(&n.title)
            .body(&n.body)
            .show()
            .map_err(|e| NotifyError::Unavailable(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omacal_core::remind::{Due, FiredKey};

    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 60 * MINUTE;

    /// 2026-08-10T09:00:00Z. In `Europe/Sofia` (UTC+3 in August) that is 12:00.
    const T0900Z: i64 = 1_786_352_400_000;
    const SOFIA: &str = "Europe/Sofia";

    /// The Snooze action for the reminder `due` builds: event 1's 10-minute
    /// reminder for its 09:00 occurrence.
    const SNOOZE_THIS: Action = Action::Snooze5m { event_id: 1, occurrence_ms: T0900Z, minutes: 10 };

    fn due(title: Option<&str>, location: Option<&str>, conference: Option<&str>) -> Due {
        Due {
            key: FiredKey { event_id: 1, occurrence_ms: T0900Z, minutes: 10 },
            fire_at_ms: T0900Z - 10 * MINUTE,
            occurrence_end_ms: T0900Z + HOUR,
            title: title.map(Into::into),
            location: location.map(Into::into),
            conference_uri: conference.map(Into::into),
            is_all_day: false,
        }
    }

    #[test]
    fn the_title_is_the_events_own() {
        assert_eq!(notification_for(&due(Some("Standup"), None, None), SOFIA).title, "Standup");
    }

    /// Google will hold an event with no summary at all, and a notification
    /// headed with an empty string reads as broken. The same fallback the
    /// popover and the grid use, so one untitled event is described one way.
    #[test]
    fn an_untitled_event_still_has_a_title() {
        assert_eq!(notification_for(&due(None, None, None), SOFIA).title, "(no title)");
    }

    /// The body is the start time, read in the zone passed in — the user's own,
    /// not the calendar's. A reminder is read at a glance, and what it has to
    /// answer is "when".
    #[test]
    fn the_body_leads_with_the_start_time_in_the_given_zone() {
        let n = notification_for(&due(Some("Standup"), None, None), SOFIA);
        assert_eq!(n.body, "12:00", "09:00Z is 12:00 in Sofia");

        let utc = notification_for(&due(Some("Standup"), None, None), "UTC");
        assert_eq!(utc.body, "09:00", "the zone argument must actually be used");
    }

    #[test]
    fn the_body_uses_the_selected_twelve_hour_clock() {
        let noon = notification_for_format(
            &due(Some("Standup"), None, None),
            SOFIA,
            crate::settings::TimeFormat::H12,
        );
        assert_eq!(noon.body, "12:00 PM");

        let morning = notification_for_format(
            &due(Some("Standup"), None, None),
            "UTC",
            crate::settings::TimeFormat::H12,
        );
        assert_eq!(morning.body, "9:00 AM");
    }

    /// An absolute time rather than "in 10 minutes", and deliberately: a missed
    /// reminder is posted late (§2.3), sometimes much later, and a relative
    /// phrase computed at build time would be a lie by the time it is read.
    /// The absolute time is true whenever it arrives.
    #[test]
    fn an_all_day_occurrence_says_so_rather_than_showing_midnight() {
        let mut d = due(Some("Sofia trip"), None, None);
        d.is_all_day = true;
        assert_eq!(notification_for(&d, SOFIA).body, "All day");
    }

    #[test]
    fn a_location_follows_the_time() {
        let n = notification_for(&due(Some("Standup"), Some("Room 1"), None), SOFIA);
        assert_eq!(n.body, "12:00 · Room 1");
    }

    #[test]
    fn an_all_day_occurrence_with_a_location_still_reads_as_all_day() {
        let mut d = due(Some("Sofia trip"), Some("Bulgaria"), None);
        d.is_all_day = true;
        assert_eq!(notification_for(&d, SOFIA).body, "All day · Bulgaria");
    }

    /// §2.5. Snooze is offered on every reminder, and names that reminder:
    /// the click comes back with nothing but the action, so the action is
    /// what tells the snooze queue which reminder to post again.
    #[test]
    fn every_notification_offers_snooze_for_its_own_reminder() {
        assert!(notification_for(&due(Some("Standup"), None, None), SOFIA)
            .actions
            .contains(&SNOOZE_THIS));
    }

    /// A reminder announces a moment; once the moment is near, expiring is
    /// its job done. Sticky is the *invitation's* property (`invites.rs`),
    /// and a reminder that inherited it would pile up unread criticals.
    #[test]
    fn a_reminder_expires_like_an_ordinary_notification() {
        assert!(!notification_for(&due(Some("Standup"), None, None), SOFIA).sticky);
    }

    /// **The rule the plan calls out.** A meeting with nowhere to join must not
    /// offer a Join button — there is nothing for it to open.
    ///
    /// Both halves in one test on purpose: a fixture set where everything has a
    /// link cannot witness this, and one where nothing does is satisfied by
    /// never offering Join at all.
    #[test]
    fn join_is_offered_only_when_the_occurrence_has_a_conference_uri() {
        let with = notification_for(&due(Some("Standup"), None, Some("https://meet/x")), SOFIA);
        assert_eq!(
            with.actions.iter().filter_map(|a| match a {
                Action::Join(uri) => Some(uri.as_str()),
                _ => None,
            }).collect::<Vec<_>>(),
            vec!["https://meet/x"],
            "a meeting with a link must offer Join, carrying that link"
        );

        let without = notification_for(&due(Some("Coffee"), Some("Kitchen"), None), SOFIA);
        assert!(
            !without.actions.iter().any(|a| matches!(a, Action::Join(_))),
            "a meeting with nowhere to join must not offer Join"
        );
    }

    /// The recording fake is the only notifier the suite ever sees. Nothing in
    /// this crate's tests reaches D-Bus or the macOS notification centre.
    #[test]
    fn the_recording_notifier_keeps_what_it_was_given_and_posts_nothing() {
        let fake = RecordingNotifier::default();
        fake.post(&notification_for(&due(Some("Standup"), None, None), SOFIA)).unwrap();
        fake.post(&notification_for(&due(Some("Retro"), None, None), SOFIA)).unwrap();

        let posted = fake.posted();
        assert_eq!(posted.len(), 2);
        assert_eq!(posted[0].title, "Standup");
        assert_eq!(posted[1].title, "Retro");
    }

    /// The one decision in the click path: which offered action a returned
    /// D-Bus key stood for. `default` is the click itself — Omarchy's shell
    /// invokes only that key, on left-click — and a dismissal (`__closed`)
    /// must resolve to nothing, or closing an invitation would answer it.
    #[test]
    fn a_returned_key_resolves_only_to_an_action_the_notification_offered() {
        let accept = Action::AcceptInvite { event_id: 7, start_ms: T0900Z };
        let offered = vec![accept.clone()];
        assert_eq!(action_for_key(&offered, "default"), Some(accept));
        assert_eq!(action_for_key(&offered, "__closed"), None, "dismissal is not consent");
        assert_eq!(action_for_key(&offered, "join"), None, "never offered here");

        // A real reminder's own action list, not a hand-built one, so the
        // claim below is about what `notification_for` actually registers.
        let reminder =
            notification_for(&due(Some("Standup"), None, Some("https://meet/x")), SOFIA).actions;
        assert_eq!(action_for_key(&reminder, "join"), Some(Action::Join("https://meet/x".into())));
        assert_eq!(action_for_key(&reminder, "snooze"), Some(SNOOZE_THIS));
        // The click lands on the event — never on the meeting. Joining is a
        // side effect a click must not have; Join stays on its own button.
        assert_eq!(
            action_for_key(&reminder, "default"),
            Some(Action::OpenEvent { event_id: 1, start_ms: T0900Z, end_ms: T0900Z + HOUR }),
            "clicking a reminder opens the app on the occurrence"
        );
    }

    /// The click's payload is the `Due`'s own occurrence — id, anchor, end —
    /// so the webview lands on the instance that fired, not the master or a
    /// neighbour. All three numbers asserted, because a wrong `end_ms` opens
    /// the popover on a span the grid does not show.
    #[test]
    fn every_reminder_carries_its_occurrence_behind_the_click() {
        let n = notification_for(&due(Some("Coffee"), None, None), SOFIA);
        assert_eq!(
            n.actions.iter().filter_map(|a| match a {
                Action::OpenEvent { event_id, start_ms, end_ms } =>
                    Some((*event_id, *start_ms, *end_ms)),
                _ => None,
            }).collect::<Vec<_>>(),
            vec![(1, T0900Z, T0900Z + HOUR)],
            "one OpenEvent, carrying the Due's own numbers"
        );
    }

    /// A notifier that cannot post is the macOS case (§2.4), and the driver has
    /// to keep working around it. The fake can be told to fail so that is
    /// testable without an unsigned bundle.
    #[test]
    fn the_recording_notifier_can_be_told_to_fail() {
        let fake = RecordingNotifier::failing();
        let err = fake.post(&notification_for(&due(Some("Standup"), None, None), SOFIA));
        assert!(err.is_err());
        assert_eq!(
            fake.posted().len(),
            1,
            "a failed post is still an attempt, and the fake records the attempt"
        );
    }

    /// The macOS round trip: the actions a notification was posted with come
    /// back out of its request identifier, resolved through the same table
    /// the D-Bus path answers from. Real reminders' action lists, not
    /// hand-built ones, for the reason the D-Bus resolution test uses them.
    #[test]
    fn a_un_click_resolves_through_the_posted_payload() {
        let reminder =
            notification_for(&due(Some("Standup"), None, Some("https://meet/x")), SOFIA).actions;
        let payload = un_payload(3, &reminder);

        assert_eq!(
            action_for_un(&payload, "join"),
            Some(Action::Join("https://meet/x".into()))
        );
        assert_eq!(action_for_un(&payload, "snooze"), Some(SNOOZE_THIS));
        assert_eq!(
            action_for_un(&payload, UN_DEFAULT_ACTION),
            Some(Action::OpenEvent { event_id: 1, start_ms: T0900Z, end_ms: T0900Z + HOUR }),
            "the body click opens the app on the occurrence, exactly as on Linux"
        );
        assert_eq!(
            action_for_un(&payload, UN_DISMISS_ACTION),
            None,
            "dismissal is not consent — the invitation rule, on the other platform"
        );
        // A notification left in Notification Center across an update, whose
        // identifier an older build wrote in some other shape: silence, not a
        // panic and not a misread action.
        assert_eq!(action_for_un("not json at all", "join"), None);
    }

    /// Identifiers must not collide for notifications posted alive at once:
    /// macOS treats the request identifier as a replacement key, and two
    /// same-pass announcements with identical (empty) action lists would
    /// otherwise silently eat each other. The nonce is what keeps them apart.
    #[test]
    fn the_nonce_keeps_identical_action_lists_from_replacing_each_other() {
        assert_ne!(un_payload(1, &[]), un_payload(2, &[]));
        // …while the payload still resolves regardless of the nonce.
        let accept = Action::AcceptInvite { event_id: 7, start_ms: T0900Z };
        let p = un_payload(9, std::slice::from_ref(&accept));
        assert_eq!(action_for_un(&p, UN_DEFAULT_ACTION), Some(accept));
    }

    /// Which pre-registered category each action list names: buttons on
    /// macOS are declared up front, so this mapping is the whole of "which
    /// buttons does this notification get".
    #[test]
    fn the_category_follows_the_buttons_a_notification_would_carry() {
        let with_join =
            notification_for(&due(Some("Standup"), None, Some("https://meet/x")), SOFIA).actions;
        let without =
            notification_for(&due(Some("Coffee"), None, None), SOFIA).actions;
        assert_eq!(un_category(&with_join), UN_CATEGORY_JOIN);
        assert_eq!(un_category(&without), UN_CATEGORY_REMINDER);
        assert_eq!(
            un_category(&[Action::AcceptInvite { event_id: 7, start_ms: T0900Z }]),
            "",
            "the invitation toast keeps its click as its whole interface"
        );
    }
}
