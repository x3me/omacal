//! Background sync ticker (spec §5): syncs on a timer, on window focus, and
//! implicitly on wake-from-sleep (a long-asleep clock makes `due` fire on the
//! next tick after waking). Demo mode never reaches Google here — both
//! `spawn` and `request_now` route their decision through `may_sync`/
//! `should_sync` rather than carrying their own untested `if demo` check.

use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager};

pub const DEFAULT_INTERVAL_MS: i64 = 5 * 60 * 1_000;
/// Floor on the poll interval. Google's quota is finite and a desktop app has
/// no business polling faster than this.
pub const MIN_INTERVAL_MS: i64 = 60 * 1_000;
/// The floor `request_now` applies. Focusing the window is an explicit "give
/// me fresh data" signal, so it gets a far shorter interval than the ticker —
/// but an interval all the same: twenty alt-tabs used to mean twenty token
/// refreshes and twenty per-calendar syncs against Google's quota.
pub const FOCUS_MIN_INTERVAL_MS: i64 = 30 * 1_000;
const SETTING_KEY: &str = "sync_interval_ms";
/// How often the ticker wakes to *consider* syncing. Short enough that a
/// wake-from-sleep is noticed promptly, cheap because `due` is pure.
const TICK: std::time::Duration = std::time::Duration::from_secs(30);

/// Reads the configured interval, clamped to something sane. Any unparseable
/// or absent value yields the default.
pub async fn interval_ms(pool: &SqlitePool) -> i64 {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
        .bind(SETTING_KEY)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

    raw.and_then(|v| v.parse::<i64>().ok())
        .map(|v| v.max(MIN_INTERVAL_MS))
        .unwrap_or(DEFAULT_INTERVAL_MS)
}

/// Whether a sync should run now.
///
/// Uses absolute elapsed distance, so both a long sleep (clock far ahead of
/// the last sync) and a clock moved backwards (last sync in the future) read
/// as due rather than wedging sync off.
pub fn due(last_sync_ms: Option<i64>, now_ms: i64, interval_ms: i64) -> bool {
    match last_sync_ms {
        None => true,
        Some(last) => (now_ms - last).abs() >= interval_ms,
    }
}

/// Whether the loop is allowed to reach Google at all. Demo mode has no
/// credentials and must produce no network traffic — every sync decision,
/// ticked or on-demand, is gated on this first.
pub fn may_sync(demo: bool) -> bool {
    !demo
}

/// Whether the ticker should sync right now: allowed (not demo) and due.
pub fn should_sync(demo: bool, last_sync_ms: Option<i64>, now_ms: i64, interval_ms: i64) -> bool {
    may_sync(demo) && due(last_sync_ms, now_ms, interval_ms)
}

/// Whether a window-focus event should sync. Same rule as the ticker, against
/// [`FOCUS_MIN_INTERVAL_MS`] instead of the configured interval — named so
/// that the constant `request_now` actually applies is the one under test.
pub fn focus_should_sync(demo: bool, last_sync_ms: Option<i64>, now_ms: i64) -> bool {
    should_sync(demo, last_sync_ms, now_ms, FOCUS_MIN_INTERVAL_MS)
}

/// What the UI is told when a sync fails.
///
/// Fixed text, deliberately: the underlying error is not safe to forward. A
/// malformed `config.toml` makes `load_config` return a `toml` parse error
/// that quotes the offending line back verbatim — and that line may be
/// `client_secret = "…"`. `ApiError::Transport` likewise carries a reqwest
/// error that can include a request URL with a sync token in it. Neither
/// belongs in a payload that crosses into the webview, so the detail stays in
/// the local log and the user gets a sentence they can act on.
pub const SYNC_FAILED_MESSAGE: &str =
    "Sync failed — OmaCal could not reach Google. It will keep trying.";

/// The `sync-failed` payload. Takes the error so that the omission is visible
/// at the call site and deliberate here, rather than looking like an oversight.
fn sync_failed_payload(_e: &anyhow::Error) -> serde_json::Value {
    serde_json::json!({ "message": SYNC_FAILED_MESSAGE })
}

/// One sync at a time. A second request that arrives while one is running
/// waits for it and does not start another: the ticker and the focus event
/// both fire at launch, a few dozen milliseconds apart, and each used to
/// download every calendar on its own.
pub(crate) struct SyncGate {
    running: std::sync::Mutex<Running>,
    done: tokio::sync::Notify,
}

struct Running {
    on: bool,
    /// Bumped when a run finishes, so a waiter can tell "still the run I
    /// saw" from "that run ended before I subscribed".
    gen: u64,
}

impl SyncGate {
    pub(crate) fn new() -> Self {
        Self {
            running: std::sync::Mutex::new(Running { on: false, gen: 0 }),
            done: tokio::sync::Notify::new(),
        }
    }

    /// Runs `work` unless a sync is already in progress, in which case this
    /// waits for that one and returns `None`. The future is lazy, so a
    /// coalesced caller never polls it.
    pub(crate) async fn run<T>(&self, work: impl std::future::Future<Output = T>) -> Option<T> {
        // The guard must not live across an await: it is not `Send`, and a
        // sync runs on Tauri's thread pool.
        let waiting_on = {
            let mut slot = self.running.lock().expect("sync gate");
            if slot.on {
                Some(slot.gen)
            } else {
                slot.on = true;
                None
            }
        };
        if let Some(gen) = waiting_on {
            loop {
                // Subscribe first, then look: a finish that lands between the
                // two must not be missed, and a finish that already happened
                // shows up as a moved generation.
                let notified = self.done.notified();
                let still = {
                    let slot = self.running.lock().expect("sync gate");
                    slot.on && slot.gen == gen
                };
                if !still {
                    return None;
                }
                notified.await;
            }
        }
        let result = work.await;
        {
            let mut slot = self.running.lock().expect("sync gate");
            slot.on = false;
            slot.gen = slot.gen.wrapping_add(1);
        }
        self.done.notify_waiters();
        Some(result)
    }
}

fn shared_gate() -> &'static SyncGate {
    static GATE: std::sync::OnceLock<SyncGate> = std::sync::OnceLock::new();
    GATE.get_or_init(SyncGate::new)
}

/// [`SyncGate::run`] on the process-wide gate `sync_all` uses.
pub(crate) async fn coalesce<T>(work: impl std::future::Future<Output = T>) -> Option<T> {
    shared_gate().run(work).await
}

/// Runs one sync and reports the outcome to the UI. Shared by the ticker and
/// the focus path so both record the timestamp and, crucially, both *say* when
/// a sync fails — silence on this path is what let the header read "Synced 4
/// min ago" all night with the network down.
async fn sync_and_report(app: &AppHandle) {
    let state = app.state::<crate::AppState>();
    match crate::sync_all(&state, false).await {
        Ok(n) => {
            if let Err(e) = crate::status::record_sync(&state.pool, crate::now_ms()).await {
                tracing::warn!(%e, "sync succeeded but recording it failed");
            }
            let _ = app.emit("sync-finished", serde_json::json!({ "upserted": n }));
            crate::upcoming::refresh(&state.pool, state.demo).await;
            // The bar widget re-reads the file above by itself; the tray is
            // in this process and has to be told (spec 2026-08-29 §3).
            crate::tray::refresh(app);
            crate::invites::after_sync(app).await;
        }
        // No token yet, offline, revoked consent — all normal. Retry next tick.
        Err(e) => {
            tracing::warn!(%e, "background sync failed");
            let _ = app.emit("sync-failed", sync_failed_payload(&e));
        }
    }
}

/// Starts the background ticker. Never panics the app: a failed sync is logged
/// and retried on the next tick.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(TICK);
        loop {
            ticker.tick().await;
            let (pool, demo) = {
                let state = app.state::<crate::AppState>();
                (state.pool.clone(), state.demo)
            };

            let now = crate::now_ms();
            let last = crate::status::last_sync_ms(&pool).await.ok().flatten();

            if !should_sync(demo, last, now, interval_ms(&pool).await) {
                continue;
            }

            sync_and_report(&app).await;
        }
    });
}

/// Nudges the ticker to reconsider immediately — used on window focus.
///
/// Goes through the same due-check as the ticker, on a much shorter interval:
/// a focus is a request for fresh data, but it is not a licence to sync once
/// per alt-tab.
pub fn request_now(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let (pool, demo) = {
            let state = app.state::<crate::AppState>();
            (state.pool.clone(), state.demo)
        };
        if !may_sync(demo) {
            return;
        }

        let last = crate::status::last_sync_ms(&pool).await.ok().flatten();
        if !focus_should_sync(demo, last, crate::now_ms()) {
            return;
        }

        sync_and_report(&app).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_785_715_200_000;

    #[test]
    fn a_never_synced_store_is_due_immediately() {
        assert!(due(None, NOW, DEFAULT_INTERVAL_MS));
    }

    #[test]
    fn a_recent_sync_is_not_due() {
        assert!(!due(Some(NOW - 60_000), NOW, DEFAULT_INTERVAL_MS));
    }

    #[test]
    fn a_stale_sync_is_due() {
        assert!(due(Some(NOW - 6 * 60_000), NOW, DEFAULT_INTERVAL_MS));
    }

    #[test]
    fn exactly_at_the_interval_is_due() {
        assert!(due(Some(NOW - DEFAULT_INTERVAL_MS), NOW, DEFAULT_INTERVAL_MS));
    }

    /// After the laptop sleeps, the clock jumps far forward. That must read as
    /// due, not as a negative elapsed time.
    #[test]
    fn a_long_sleep_reads_as_due() {
        assert!(due(Some(NOW - 8 * 3_600_000), NOW, DEFAULT_INTERVAL_MS));
    }

    /// A clock moved backwards must not wedge sync off forever.
    #[test]
    fn a_future_timestamp_is_treated_as_due() {
        assert!(due(Some(NOW + 3_600_000), NOW, DEFAULT_INTERVAL_MS));
    }

    /// `due()` alone would return true for both cases here — this only passes
    /// because `should_sync` also carries the demo term.
    #[test]
    fn demo_mode_never_syncs_however_overdue() {
        assert!(!should_sync(true, None, NOW, DEFAULT_INTERVAL_MS));
        assert!(!should_sync(true, Some(NOW - 8 * 3_600_000), NOW, DEFAULT_INTERVAL_MS));
    }

    #[test]
    fn a_real_account_still_syncs_when_due() {
        assert_eq!(
            should_sync(false, None, NOW, DEFAULT_INTERVAL_MS),
            due(None, NOW, DEFAULT_INTERVAL_MS),
        );
        assert_eq!(
            should_sync(false, Some(NOW - 60_000), NOW, DEFAULT_INTERVAL_MS),
            due(Some(NOW - 60_000), NOW, DEFAULT_INTERVAL_MS),
        );
        assert_eq!(
            should_sync(false, Some(NOW - 6 * 60_000), NOW, DEFAULT_INTERVAL_MS),
            due(Some(NOW - 6 * 60_000), NOW, DEFAULT_INTERVAL_MS),
        );
    }

    #[test]
    fn may_sync_is_false_in_demo_mode_and_true_otherwise() {
        assert!(!may_sync(true));
        assert!(may_sync(false));
    }

    /// Alt-tabbing back into the window a few seconds after a sync must not
    /// spend another round of Google's quota on data that cannot have changed.
    #[test]
    fn a_focus_immediately_after_a_sync_does_not_sync_again() {
        assert!(!focus_should_sync(false, Some(NOW - 1_000), NOW));
        assert!(!focus_should_sync(false, Some(NOW - (FOCUS_MIN_INTERVAL_MS - 1)), NOW));
    }

    /// But focus is still the "give me fresh data" gesture, so the wait is
    /// seconds rather than the ticker's minutes.
    #[test]
    fn a_focus_after_the_focus_interval_syncs() {
        assert!(focus_should_sync(false, Some(NOW - FOCUS_MIN_INTERVAL_MS), NOW));
        assert!(focus_should_sync(false, Some(NOW - 10 * 60_000), NOW));
        // A store that has never synced syncs on the first focus.
        assert!(focus_should_sync(false, None, NOW));
    }

    /// A focus one minute after a sync would be refused by the ticker's
    /// interval and allowed by the focus one — that difference is the point.
    #[test]
    fn the_focus_floor_is_shorter_than_the_tick_interval() {
        const { assert!(FOCUS_MIN_INTERVAL_MS < DEFAULT_INTERVAL_MS) };
        let a_minute_ago = Some(NOW - 60_000);
        assert!(focus_should_sync(false, a_minute_ago, NOW));
        assert!(!should_sync(false, a_minute_ago, NOW, DEFAULT_INTERVAL_MS));
    }

    /// Demo mode is refused on the focus path too, however overdue.
    #[test]
    fn a_focus_in_demo_mode_never_syncs() {
        assert!(!focus_should_sync(true, None, NOW));
        assert!(!focus_should_sync(true, Some(NOW - 8 * 3_600_000), NOW));
    }

    /// The failure the user sees must be a sentence they can act on, and must
    /// not carry the underlying error: a `toml` parse failure in `config.toml`
    /// quotes the offending line, which may be the client secret.
    #[test]
    fn the_sync_failed_payload_carries_no_error_detail() {
        let secret = "GOCSPX-pretend-client-secret";
        let e = anyhow::anyhow!(
            "TOML parse error at line 2\n  |\n2 | client_secret = \"{secret}\"\n  | ^"
        );
        let printed = sync_failed_payload(&e).to_string();

        assert!(!printed.contains(secret), "the error leaked into the UI payload: {printed}");
        assert!(!printed.contains("TOML"), "the raw error leaked into the UI payload: {printed}");
        assert!(printed.contains("Sync failed"));
        assert_eq!(sync_failed_payload(&e), sync_failed_payload(&anyhow::anyhow!("anything else")));
    }

    #[tokio::test]
    async fn the_interval_defaults_when_unset() {
        let pool = omacal_store::connect_memory().await.unwrap();
        assert_eq!(interval_ms(&pool).await, DEFAULT_INTERVAL_MS);
    }

    #[tokio::test]
    async fn the_interval_is_configurable_from_settings() {
        let pool = omacal_store::connect_memory().await.unwrap();
        sqlx::query("INSERT INTO settings (key, value) VALUES ('sync_interval_ms', '120000')")
            .execute(&pool).await.unwrap();
        assert_eq!(interval_ms(&pool).await, 120_000);
    }

    #[tokio::test]
    async fn an_absurd_interval_is_clamped_not_obeyed() {
        let pool = omacal_store::connect_memory().await.unwrap();
        sqlx::query("INSERT INTO settings (key, value) VALUES ('sync_interval_ms', '100')")
            .execute(&pool).await.unwrap();
        // A 100 ms poll would hammer Google and burn quota.
        assert_eq!(interval_ms(&pool).await, MIN_INTERVAL_MS);
    }

    #[tokio::test]
    async fn a_garbage_interval_falls_back_to_the_default() {
        let pool = omacal_store::connect_memory().await.unwrap();
        sqlx::query("INSERT INTO settings (key, value) VALUES ('sync_interval_ms', 'soon')")
            .execute(&pool).await.unwrap();
        assert_eq!(interval_ms(&pool).await, DEFAULT_INTERVAL_MS);
    }

    /// Launch fires the ticker and a focus within a few dozen milliseconds.
    /// The second must wait, not download every calendar beside the first.
    #[tokio::test]
    async fn a_sync_already_running_is_not_started_again() {
        let gate = std::sync::Arc::new(SyncGate::new());
        let runs = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let entered = std::sync::Arc::new(tokio::sync::Notify::new());
        let release = std::sync::Arc::new(tokio::sync::Notify::new());

        let wait_entered = entered.notified();
        let (g1, r1, e1, rel1) = (gate.clone(), runs.clone(), entered.clone(), release.clone());
        let first = tokio::spawn(async move {
            g1.run(async {
                r1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                e1.notify_waiters();
                rel1.notified().await;
            })
            .await
        });
        wait_entered.await;

        let (g2, r2) = (gate.clone(), runs.clone());
        let second = tokio::spawn(async move {
            g2.run(async {
                r2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            })
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        release.notify_waiters();
        assert!(first.await.unwrap().is_some(), "the first request is the one that runs");
        assert!(second.await.unwrap().is_none(), "the one that arrived during it does not");
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
