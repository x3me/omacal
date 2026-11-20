//! The update notice — knowing a newer release exists, saying so once — and,
//! on exactly one channel, the button that acts on it.
//!
//! The notice is the foundation and it is universal: an installed copy
//! otherwise runs unchanged forever with no way to learn that a newer
//! version exists, which matters doubly here because a rotated OAuth secret
//! strands old binaries (distribution runbook) and the only cure is the
//! update the user never heard about.
//!
//! Acting on the notice is narrower. The AppImage — the installer
//! one-liner's channel, and most installs — is one user-owned file, so
//! [`install_update`] can replace it in place: a signed download, verified
//! against the pubkey in `tauri.conf.json`, then the same restart the
//! moved-zone banner uses. Every packaged install (.deb, .rpm, AUR,
//! Flatpak) keeps the notice-only banner, because there the files belong to
//! a package manager and the update mechanism is that manager, as the
//! user's own act. [`may_self_update`] is that decision, stated once.
//!
//! Split the way `sync_loop` and `tray` are: the decisions — is a tag newer,
//! does a response announce a notice, may demo check at all, may this build
//! replace itself — are pure and tested; the ticker, the browser-open and
//! the plugin's download-and-install are OS integration and are the
//! untested half.

use tauri::{AppHandle, Manager};

/// GitHub's public "latest published release" endpoint — the same one the
/// install script queries, so the app and the installer can never disagree
/// about what "latest" means. Unauthenticated, rate-limited per IP at a
/// level one request a day does not approach.
pub(crate) const LATEST_RELEASE_ENDPOINT: &str =
    "https://api.github.com/repos/x3me/omacal/releases/latest";

/// Once a day. A release is a rare event and the endpoint is a shared,
/// unauthenticated courtesy; polling it faster buys nothing but rate-limit
/// exposure.
const CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(24 * 3600);
/// The first check waits out the launch: the sync loop and the webview are
/// both starting, and "is there a newer version" is the least urgent fact in
/// the room.
const FIRST_CHECK_DELAY: std::time::Duration = std::time::Duration::from_secs(30);

/// What the UI is told when a newer release exists. Rides on `get_status`
/// beside `needs_reauth`, for the same reason: it is a fact about this
/// running instance that the header renders differently for.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UpdateNotice {
    /// The newer version, without the tag's `v` — it is displayed inside a
    /// sentence, and "v0.1.2 is available" reads like a git tag escaped.
    pub version: String,
    /// The release page, opened by `open_latest_release`. Kept backend-side
    /// so the webview never chooses what the browser opens.
    pub url: String,
}

/// Whether the loop may reach the network at all. Demo mode's promise is
/// broader than "no Google": it produces no network traffic, full stop —
/// same rule, same shape, same reason as [`crate::sync_loop::may_sync`] and
/// [`crate::notify_loop::may_notify`].
///
/// And the user's own switch (`settings::update_check`): the check asks a
/// third party, so it is theirs to turn off.
pub(crate) fn may_check(demo: bool, enabled: bool) -> bool {
    !demo && enabled
}

/// Whether this process may replace itself — the gate on the banner's
/// "Update" button and on [`install_update`], decided from three facts.
///
/// The AppImage qualifies because it is one file the user owns, so
/// replacing it is an ordinary write. macOS qualifies since 2026-08-25,
/// when the first notarization cleared: the .app is the user's own too,
/// and the *signed* guarantee lives in the pipeline, not here — the
/// updater manifest only ever carries a `darwin-aarch64` entry when Apple
/// signing actually ran (release.yml's join job), so an unsigned build
/// cannot be offered as an update whatever this gate says. Everything
/// else is somebody else's property — dpkg's, rpm's, pacman's, flatpak's —
/// and a binary that overwrites files a package manager believes it owns
/// corrupts exactly the record that manager exists to keep. Those
/// channels keep the notice-only banner. And demo stays out for
/// `may_check`'s reason: a download is network traffic.
pub(crate) fn may_self_update(is_appimage: bool, is_macos: bool, demo: bool) -> bool {
    (is_appimage || is_macos) && !demo
}

/// Whether this process is running out of an AppImage. The AppImage runtime
/// exports `APPIMAGE` — the image's own path — before launching the payload,
/// and nothing else sets it. The *filename* proves nothing: the installer
/// lands the image at `~/.local/bin/omacal`, extensionless, indistinguishable
/// by name from the .deb's binary.
pub(crate) fn running_as_appimage() -> bool {
    std::env::var_os("APPIMAGE").is_some()
}

/// `Some((major, minor, patch))` for `"0.1.2"` or `"v0.1.2"`, `None` for
/// anything else. Numeric, not lexical — `"0.1.10"` is newer than `"0.1.9"`
/// and a string comparison says otherwise.
fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim();
    let s = s.strip_prefix('v').unwrap_or(s);
    let mut parts = s.split('.');
    let mut next = || parts.next()?.parse::<u64>().ok();
    let v = (next()?, next()?, next()?);
    // A fourth component is not a version this app ever tagged; refusing it
    // beats guessing what it means.
    if parts.next().is_some() {
        return None;
    }
    Some(v)
}

/// Whether `latest_tag` names a strictly newer version than `current`.
///
/// Strictly: equal is not an update, and an *older* published latest (a
/// yanked release, a rollback) must not prompt anyone to "upgrade" backwards.
/// Unparseable input on either side is `false` — a notice is an interruption,
/// and an interruption built on a guess is spam.
pub(crate) fn newer_than(current: &str, latest_tag: &str) -> bool {
    match (parse_version(current), parse_version(latest_tag)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

/// Turns a fetched release into what the UI should be told: a notice when it
/// is genuinely newer, nothing otherwise.
pub(crate) fn notice_for(current: &str, tag: &str, url: &str) -> Option<UpdateNotice> {
    newer_than(current, tag).then(|| UpdateNotice {
        version: tag.trim().trim_start_matches('v').to_string(),
        url: url.to_string(),
    })
}

#[derive(serde::Deserialize)]
struct LatestRelease {
    tag_name: String,
    html_url: String,
}

/// Asks the endpoint what the latest published release is.
///
/// The `User-Agent` is not decoration: GitHub's API rejects requests without
/// one outright, so an "optimisation" that drops it turns every check into a
/// 403 and the notice silently never appears.
pub(crate) async fn fetch_latest(endpoint: &str) -> anyhow::Result<(String, String)> {
    use anyhow::Context;
    let resp = reqwest::Client::builder()
        .user_agent(concat!("omacal/", env!("CARGO_PKG_VERSION")))
        // A background notice; a GitHub that hangs must not hold a task.
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(30))
        .build()?
        .get(endpoint)
        .send()
        .await
        .context("release endpoint unreachable")?;

    if !resp.status().is_success() {
        anyhow::bail!("release endpoint answered {}", resp.status());
    }

    let r: LatestRelease =
        resp.json().await.context("release response was not JSON")?;
    Ok((r.tag_name, r.html_url))
}

/// How long a focus is answered from the last check rather than a new one.
/// A release is a rare event; four hours means an instance that missed one
/// by minutes — launched just before a publish, the exact case that
/// otherwise stays silent all day — still hears within the afternoon,
/// while an alt-tab is never a request to poll GitHub.
const FOCUS_CHECK_FLOOR_MS: i64 = 4 * 3600 * 1000;

/// Whether a focus at `now_ms` should re-ask, given the last check at
/// `last_ms` (`0` for never). A window, not a comparison — a stamp from the
/// future (a clock that jumped back) reads as "check", the same posture as
/// `weather::cache_is_fresh`, because the alternative is a floor that never
/// opens again.
pub(crate) fn focus_should_check(last_ms: i64, now_ms: i64) -> bool {
    last_ms == 0 || !(last_ms <= now_ms && now_ms - last_ms < FOCUS_CHECK_FLOOR_MS)
}

/// One check: ask, compare, store, and say so. The stamp is written on
/// every attempt — success or not — so a failing endpoint is retried on the
/// ticker's schedule, not hammered on every focus. A *new* notice also
/// emits `update-notice`, which is what lets the banner appear while the
/// window sits open: `get_status` is only re-read when something tells the
/// webview to, and the daily ticker's discovery used to wait for whatever
/// refetch happened along next.
async fn check_once(app: &AppHandle, current: &str) {
    app.state::<crate::AppState>()
        .update_checked_at
        .store(crate::now_ms(), std::sync::atomic::Ordering::Relaxed);
    match fetch_latest(LATEST_RELEASE_ENDPOINT).await {
        Ok((tag, url)) => {
            if let Some(n) = notice_for(current, &tag, &url) {
                tracing::info!(version = %n.version, "a newer release exists");
                let state = app.state::<crate::AppState>();
                let mut slot = state.update.lock().expect("update notice poisoned");
                let is_news = slot.as_ref() != Some(&n);
                *slot = Some(n);
                drop(slot);
                if is_news {
                    use tauri::Emitter;
                    let _ = app.emit("update-notice", ());
                }
            }
        }
        Err(e) => tracing::debug!(%e, "update check failed; retrying later"),
    }
}

/// Starts the daily check. **Untested**, like `sync_loop::spawn`: everything
/// it decides is decided by the pure functions above; what is left is a
/// ticker and a mutex write.
///
/// A failed check is `debug!`, not `warn!` — being offline is a normal state
/// for a laptop and this is the one loop with nothing at stake; retrying
/// later is the whole plan.
pub(crate) fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let (demo, current) = {
            let state = app.state::<crate::AppState>();
            (state.demo, app.package_info().version.to_string())
        };
        if !may_check(demo, true) {
            return;
        }

        tokio::time::sleep(FIRST_CHECK_DELAY).await;
        let mut ticker = tokio::time::interval(CHECK_INTERVAL);
        loop {
            ticker.tick().await; // the first tick resolves immediately
            // Read each tick, so the switch takes effect without a restart.
            let pool = app.state::<crate::AppState>().pool.clone();
            if may_check(demo, crate::settings::update_check(&pool).await) {
                check_once(&app, &current).await;
            }
        }
    });
}

/// The focus edge: the user is back, and the notice may be a day stale —
/// re-ask behind [`focus_should_check`]'s floor. Rides the same signal the
/// focus-sync does (`lib.rs`'s `Focused(true)` handler), gated the same way
/// demo gates everything.
pub(crate) fn check_on_focus(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let (demo, current, last) = {
            let state = app.state::<crate::AppState>();
            (
                state.demo,
                app.package_info().version.to_string(),
                state.update_checked_at.load(std::sync::atomic::Ordering::Relaxed),
            )
        };
        let pool = app.state::<crate::AppState>().pool.clone();
        if !may_check(demo, crate::settings::update_check(&pool).await)
            || !focus_should_check(last, crate::now_ms())
        {
            return;
        }
        check_once(&app, &current).await;
    });
}

/// Opens the latest release's page in the browser — the URL the *backend*
/// fetched, deliberately not one the webview supplies: a compromised webview
/// asking the OS to open an attacker's URL is the whole class of bug this
/// signature refuses.
#[tauri::command]
pub(crate) fn open_latest_release(state: tauri::State<'_, crate::AppState>) -> Result<(), String> {
    let url = state
        .update
        .lock()
        .expect("update notice poisoned")
        .as_ref()
        .map(|n| n.url.clone());
    if let Some(url) = url {
        // `browser`, not `open::that`: inside an AppImage the raw spawn
        // hands the browser an environment that crashes it (issue #1).
        crate::browser::open_external(&url).map_err(|e| {
            tracing::warn!(%e, "could not open the release page");
            crate::BROWSER_FAILED.to_string()
        })?;
    }
    Ok(())
}

/// Replaces this AppImage with the latest release and restarts — the banner's
/// "Update" button, on the one channel where that is an ordinary file write
/// (see [`may_self_update`]).
///
/// The plugin re-checks the endpoint rather than trusting the daily notice:
/// the notice is up to a day old, and what must never happen is installing a
/// version other than the one whose signature was just verified. Verification
/// is the plugin's, against the pubkey baked into `tauri.conf.json` — a
/// mirror or a compromised release page cannot hand this function an
/// arbitrary binary, only fail it.
///
/// Errors return as the sentence the banner shows, detail at `warn!` — the
/// same split as everywhere else a command reports failure. Every failure
/// leaves the running image untouched: the plugin writes a temp file and
/// renames, so there is no half-updated state to fall into.
///
/// On success the restart is the delayed shape of
/// [`crate::settings::restart_app`], for its reason: the reply must reach
/// the webview first, so the button can say "Updating…" instead of dying
/// mid-await.
#[tauri::command]
pub(crate) async fn install_update(
    app: AppHandle,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    if !may_self_update(running_as_appimage(), cfg!(target_os = "macos"), state.demo) {
        // Unreachable through the UI — the button only renders when
        // `get_status` said yes — so this is the backend refusing to trust
        // the webview's gating, same as `open_latest_release` refusing its
        // URL.
        return Err("This build updates through its package manager.".into());
    }

    use tauri_plugin_updater::UpdaterExt;
    let update = app
        .updater()
        .map_err(|e| {
            tracing::warn!(%e, "updater unavailable");
            "The updater is not available in this build.".to_string()
        })?
        .check()
        .await
        .map_err(|e| {
            tracing::warn!(%e, "update check failed");
            "Could not reach the release server. Try again later.".to_string()
        })?
        .ok_or_else(|| "Already up to date.".to_string())?;

    tracing::info!(version = %update.version, "downloading and installing an update");
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| {
            tracing::warn!(%e, "self-update failed");
            "The update could not be installed. Re-run the install command instead."
                .to_string()
        })?;

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        // Not `app.restart()`: its exit-time teardown is what left a
        // windowless zombie of the old instance after every update
        // (field-diagnosed 2026-08-26; see `restart`'s module doc).
        drop(app);
        crate::restart::hard_restart();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Numeric on every component — the lexical comparison this replaces
    /// says "0.1.9" > "0.1.10" and would sit silent across exactly the
    /// releases that follow a .9.
    #[test]
    fn a_newer_tag_is_newer_numerically_not_lexically() {
        assert!(newer_than("0.1.1", "v0.1.2"));
        assert!(newer_than("0.1.9", "v0.1.10"));
        assert!(newer_than("0.9.9", "v0.10.0"));
        assert!(newer_than("0.1.1", "v1.0.0"));
    }

    /// Equal is not an update, and an older "latest" — a yanked release — is
    /// not an invitation to downgrade.
    #[test]
    fn the_current_or_an_older_release_is_not_an_update() {
        assert!(!newer_than("0.1.1", "v0.1.1"));
        assert!(!newer_than("0.1.1", "v0.1.0"));
        assert!(!newer_than("1.0.0", "v0.9.9"));
    }

    /// A notice is an interruption; an interruption built on a guess is spam.
    /// Whatever shape the endpoint's answer drifts to, unparseable means
    /// silent.
    #[test]
    fn garbage_on_either_side_never_announces_an_update() {
        for tag in ["", "latest", "v0.2", "v0.2.0.1", "0.2.x", "nightly-2026"] {
            assert!(!newer_than("0.1.1", tag), "tag {tag:?} produced a notice");
        }
        assert!(!newer_than("not-a-version", "v99.0.0"));
    }

    /// The sentence in the header says "0.1.2", not "v0.1.2" — the tag's
    /// prefix is git's business.
    #[test]
    fn a_notice_carries_the_version_without_the_tags_v() {
        let n = notice_for("0.1.1", "v0.1.2", "https://example.com/rel").unwrap();
        assert_eq!(n.version, "0.1.2");
        assert_eq!(n.url, "https://example.com/rel");
        assert!(notice_for("0.1.2", "v0.1.2", "u").is_none(), "equal announced an update");
    }

    /// The focus floor, all corners: never-checked asks, a recent check
    /// answers from memory, an old one re-asks — and a stamp from the
    /// future (a clock that jumped back) re-asks rather than sealing the
    /// floor shut forever.
    #[test]
    fn a_focus_reasks_only_past_the_floor_and_a_future_stamp_reasks() {
        assert!(focus_should_check(0, 1_000), "a fresh launch's focus never re-asked");
        assert!(!focus_should_check(1_000, 1_000 + FOCUS_CHECK_FLOOR_MS - 1),
                "an alt-tab polled GitHub");
        assert!(focus_should_check(1_000, 1_000 + FOCUS_CHECK_FLOOR_MS));
        assert!(focus_should_check(10_000, 1_000), "a future stamp sealed the floor shut");
    }

    /// Demo mode's promise is no network traffic at all, and the check is
    /// network traffic. Same rule as `may_sync` and `may_notify`.
    #[test]
    fn demo_mode_never_checks_for_updates() {
        assert!(!may_check(true, true));
        assert!(may_check(false, true));
    }

    /// The user's switch: off is off, demo or not.
    #[test]
    fn a_check_turned_off_never_runs() {
        assert!(!may_check(false, false));
        assert!(!may_check(true, false));
    }

    /// On by default, except inside a Flatpak, whose package manager brings
    /// the updates; a stored choice wins either way.
    #[tokio::test]
    async fn the_update_check_is_on_by_default_except_in_flatpak_and_a_choice_wins() {
        assert!(crate::settings::update_check_default(false));
        assert!(!crate::settings::update_check_default(true));

        let pool = omacal_store::connect_memory().await.unwrap();
        assert!(crate::settings::update_check(&pool).await, "no FLATPAK_ID under test: on");
        crate::settings::store(&pool, &crate::settings::Setting::UpdateCheck(false)).await.ok();
        assert!(!crate::settings::update_check(&pool).await, "turned off, it stays off");
    }

    /// The self-update gate, every corner. The packaged cases are the ones
    /// that matter: a .deb or AUR build offering to overwrite files its
    /// package manager owns corrupts the manager's record of the system.
    /// Parameters rather than `cfg!`, so every platform's answer is pinned
    /// from any platform's test run.
    #[test]
    fn only_a_non_demo_appimage_or_macos_build_may_self_update() {
        assert!(may_self_update(true, false, false));
        assert!(may_self_update(false, true, false), "signed macOS lost its button");
        assert!(!may_self_update(false, false, false), "a packaged build offered to replace itself");
        assert!(!may_self_update(true, false, true), "demo offered to make network traffic");
        assert!(!may_self_update(false, true, true), "demo on macOS offered network traffic");
        assert!(!may_self_update(false, false, true));
    }

    /// Through the real fetch path, with the one header the whole feature
    /// hangs on: GitHub answers 403 to requests without a `User-Agent`, so a
    /// client that stopped sending one fails on every check, forever, at
    /// `debug!` level — invisible. The matcher makes that a red test instead.
    #[tokio::test]
    async fn the_latest_release_is_fetched_with_a_user_agent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/latest"))
            .and(header_exists("user-agent"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tag_name": "v0.1.2",
                "html_url": "https://github.com/x3me/omacal/releases/tag/v0.1.2"
            })))
            .mount(&server)
            .await;

        let (tag, url) = fetch_latest(&format!("{}/latest", server.uri())).await.unwrap();
        assert_eq!(tag, "v0.1.2");
        assert_eq!(url, "https://github.com/x3me/omacal/releases/tag/v0.1.2");
    }

    /// A rate-limited or erroring endpoint is an `Err`, not a fabricated
    /// answer — the caller's whole handling is "log and retry tomorrow".
    #[tokio::test]
    async fn an_unhappy_endpoint_is_an_error_not_a_notice() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let err = fetch_latest(&server.uri()).await.unwrap_err().to_string();
        assert!(err.contains("403"), "the status is the diagnosis: {err}");
    }
}
