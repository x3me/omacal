use serde::Serialize;
use sqlx::SqlitePool;
use tauri::TitleBarStyle;

const LAST_SYNC_KEY: &str = "last_sync_ms";

#[derive(Debug, Clone, Serialize)]
pub struct AppStatus {
    /// Email addresses of connected accounts; empty means "not signed in".
    pub accounts: Vec<String>,
    /// The subset of `accounts` whose refresh token is dead for good
    /// (`AppState::reauth`): sync has stopped trying them, and the only fix
    /// is signing in again. The UI turns a non-empty list into a reconnect
    /// prompt — which is why this is emails rather than a count: "an account
    /// needs attention" with two accounts connected is a guessing game.
    pub needs_reauth: Vec<String>,
    /// A newer published release, when the daily check has found one — the
    /// header turns it into a quiet notice. `None` means current, unchecked,
    /// or demo; the UI has no reason to tell those apart.
    pub update: Option<crate::update::UpdateNotice>,
    /// The running build's own version, for the Settings footer. The same
    /// source the update check compares against (`package_info`), so the app
    /// can never claim one version and check another.
    pub version: String,
    /// The system zone's new IANA name, when `tz_watch` has seen it move out
    /// from under this process — whose own zone is fixed at launch, so every
    /// time on screen is still drawn in the old one. The UI turns this into
    /// a banner whose one action is the restart that catches the app up.
    /// `None` means the system zone is (still) the one this process started
    /// in, or the display zone is pinned by setting and cannot go stale.
    pub system_tz_change: Option<String>,
    pub last_sync_ms: Option<i64>,
    /// True when the app is running on synthetic data, so the UI can say so.
    pub demo: bool,
    /// True when the window's own controls are drawn *over* the webview rather
    /// than in a strip above it, so the header has to leave room for them.
    ///
    /// This rides on `get_status` for one reason: `ui/src` has no other way to
    /// learn what it is running on. Nothing in the frontend is platform-aware,
    /// and the alternative — `@tauri-apps/plugin-os` — is a whole dependency
    /// bought for a single boolean the app already fetches a payload for on
    /// mount. It sits beside `demo` rather than in `get_palette` because it is
    /// the same kind of fact: not a colour, but something about *this* running
    /// instance that the UI has to render differently for.
    pub overlay_titlebar: bool,
    /// True when the update banner's action can be "Update" rather than
    /// "re-run the install command": this process is an AppImage — one
    /// user-writable file the updater can replace — and not demo. Decided
    /// by [`crate::update::may_self_update`]; this is the same kind of fact
    /// as `overlay_titlebar`, about *this* running instance rather than the
    /// calendar, riding on `get_status` for the same reason.
    pub self_update: bool,
}

/// Whether the window controls overlay the webview's own content.
///
/// Both arguments, rather than reading `cfg!` here: the config half is what the
/// shipped `tauri.conf.json` actually asks for (so removing `titleBarStyle`
/// removes the inset with it, instead of leaving the UI reserving space for
/// controls that moved back into their own strip), and the platform half is what
/// keeps Omarchy out of it. `titleBarStyle` is a macOS-only key — Linux parses
/// it and ignores it — so on Linux the config says `Overlay` and the controls
/// are still in a strip of their own. Reserving 60px there buys a dead gap at
/// the left of the header and nothing else.
pub fn controls_overlay_content(style: TitleBarStyle, macos: bool) -> bool {
    macos && matches!(style, TitleBarStyle::Overlay)
}

// Eight, and honestly: every one is a distinct fact `get_status` owns and
// this function only assembles. The hazard eight arguments actually carry
// here — three adjacent bools — is pinned by the crossed-flags test below.
#[allow(clippy::too_many_arguments)]
pub async fn read_status(
    pool: &SqlitePool,
    demo: bool,
    overlay_titlebar: bool,
    self_update: bool,
    needs_reauth: Vec<String>,
    update: Option<crate::update::UpdateNotice>,
    system_tz_change: Option<String>,
    version: String,
) -> anyhow::Result<AppStatus> {
    let accounts: Vec<String> =
        sqlx::query_scalar("SELECT email FROM accounts ORDER BY id")
            .fetch_all(pool)
            .await?;

    Ok(AppStatus {
        accounts,
        needs_reauth,
        update,
        version,
        system_tz_change,
        last_sync_ms: last_sync_ms(pool).await?,
        demo,
        overlay_titlebar,
        self_update,
    })
}

/// When the last successful sync was recorded, and nothing else.
///
/// The background loop wants this one number to decide whether a sync is due.
/// It has no use for the account list, and — more to the point — no honest
/// value for either of `read_status`'s flags: it is not the UI, so whether the
/// window controls overlay the content is none of its business, and passing a
/// placeholder for a field somebody later reads is how a placeholder becomes a
/// claim.
pub async fn last_sync_ms(pool: &SqlitePool) -> anyhow::Result<Option<i64>> {
    let raw: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
            .bind(LAST_SYNC_KEY)
            .fetch_optional(pool)
            .await?;
    Ok(raw.and_then(|v| v.parse().ok()))
}

pub async fn record_sync(pool: &SqlitePool, at_ms: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )
    .bind(LAST_SYNC_KEY)
    .bind(at_ms.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool_with_account() -> sqlx::SqlitePool {
        let pool = omacal_store::connect_memory().await.unwrap();
        sqlx::query(
            "INSERT INTO accounts (google_sub, email, created_at) VALUES ('s','me@x.com',0)")
            .execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn status_reports_no_accounts_on_a_fresh_database() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let s = read_status(&pool, false, false, false, vec![], None, None, "1.2.3".into()).await.unwrap();
        assert!(s.accounts.is_empty());
        assert_eq!(s.last_sync_ms, None);
        assert!(!s.demo);
    }

    #[tokio::test]
    async fn status_lists_connected_accounts_by_email() {
        let pool = pool_with_account().await;
        let s = read_status(&pool, false, false, false, vec![], None, None, "1.2.3".into()).await.unwrap();
        assert_eq!(s.accounts, vec!["me@x.com".to_string()]);
    }

    #[tokio::test]
    async fn recording_a_sync_round_trips() {
        let pool = pool_with_account().await;
        record_sync(&pool, 1_785_715_200_000).await.unwrap();
        let s = read_status(&pool, false, false, false, vec![], None, None, "1.2.3".into()).await.unwrap();
        assert_eq!(s.last_sync_ms, Some(1_785_715_200_000));
    }

    #[tokio::test]
    async fn recording_a_sync_twice_keeps_the_latest() {
        let pool = pool_with_account().await;
        record_sync(&pool, 1_000).await.unwrap();
        record_sync(&pool, 2_000).await.unwrap();
        let s = read_status(&pool, false, false, false, vec![], None, None, "1.2.3".into()).await.unwrap();
        assert_eq!(s.last_sync_ms, Some(2_000));
    }

    /// The reconnect list rides through untouched — and as emails, because
    /// "an account needs attention" with two accounts connected is a guessing
    /// game the UI cannot resolve on the user's behalf.
    #[tokio::test]
    async fn status_surfaces_the_accounts_needing_reconnection() {
        let pool = pool_with_account().await;

        let s = read_status(&pool, false, false, false, vec!["me@x.com".into()], None, None, "1.2.3".into()).await.unwrap();
        assert_eq!(s.needs_reauth, vec!["me@x.com".to_string()]);

        let none = read_status(&pool, false, false, false, vec![], None, None, "1.2.3".into()).await.unwrap();
        assert!(none.needs_reauth.is_empty(), "a healthy account was told to reconnect");
    }

    /// The Settings footer's whole claim. Pinned as a ride-through so a
    /// refactor that hardcodes it can't survive: "1.2.3" is nothing this
    /// crate would ever invent on its own.
    #[tokio::test]
    async fn status_carries_the_running_version() {
        let pool = pool_with_account().await;
        let s = read_status(&pool, false, false, false, vec![], None, None, "1.2.3".into()).await.unwrap();
        assert_eq!(s.version, "1.2.3");
    }

    /// The update notice rides through whole — version and URL — because the
    /// header renders one and the open command needs the other.
    #[tokio::test]
    async fn status_surfaces_a_newer_release() {
        let pool = pool_with_account().await;

        let n = crate::update::UpdateNotice {
            version: "0.2.0".into(),
            url: "https://github.com/x3me/omacal/releases/tag/v0.2.0".into(),
        };
        let s = read_status(&pool, false, false, false, vec![], Some(n), None, "1.2.3".into()).await.unwrap();
        let got = s.update.expect("the notice was dropped on the way to the UI");
        assert_eq!(got.version, "0.2.0");
        assert!(got.url.ends_with("v0.2.0"));

        let current = read_status(&pool, false, false, false, vec![], None, None, "1.2.3".into()).await.unwrap();
        assert!(current.update.is_none(), "a current install was told to update");
    }

    /// The moved-zone notice rides through as the zone's own name: the banner
    /// has to say where the system went, or "your time zone changed" reads as
    /// a riddle on a machine whose clock looks fine.
    #[tokio::test]
    async fn status_surfaces_a_system_zone_that_moved() {
        let pool = pool_with_account().await;

        let s = read_status(
            &pool, false, false, false, vec![], None, Some("Asia/Kolkata".into()), "1.2.3".into(),
        ).await.unwrap();
        assert_eq!(s.system_tz_change.as_deref(), Some("Asia/Kolkata"));

        let still = read_status(&pool, false, false, false, vec![], None, None, "1.2.3".into())
            .await.unwrap();
        assert!(still.system_tz_change.is_none(), "a zone that never moved grew a banner");
    }

    /// All three flags, each in a run where the other two are false: `demo`,
    /// `overlay_titlebar` and `self_update` are adjacent `bool` parameters,
    /// so the one mistake this signature invites is passing them the wrong
    /// way round. Asserting each alone is what makes any swap fail rather
    /// than cancel out.
    #[tokio::test]
    async fn the_demo_overlay_and_self_update_flags_are_surfaced_independently() {
        let pool = omacal_store::connect_memory().await.unwrap();

        let demo_only = read_status(&pool, true, false, false, vec![], None, None, "1.2.3".into()).await.unwrap();
        assert!(demo_only.demo, "the demo badge would never appear");
        assert!(!demo_only.overlay_titlebar, "demo leaked into the titlebar flag");
        assert!(!demo_only.self_update, "demo leaked into the self-update flag");

        let overlay_only = read_status(&pool, false, true, false, vec![], None, None, "1.2.3".into()).await.unwrap();
        assert!(overlay_only.overlay_titlebar, "the header would never clear the traffic lights");
        assert!(!overlay_only.demo, "the titlebar flag leaked into the demo badge");
        assert!(!overlay_only.self_update, "the titlebar flag leaked into the self-update flag");

        let self_update_only = read_status(&pool, false, false, true, vec![], None, None, "1.2.3".into()).await.unwrap();
        assert!(self_update_only.self_update, "the Update button would never appear");
        assert!(!self_update_only.demo, "the self-update flag leaked into the demo badge");
        assert!(!self_update_only.overlay_titlebar, "the self-update flag leaked into the titlebar flag");
    }

    /// The whole of the Linux half of this feature, and the half no macOS run
    /// can observe. `titleBarStyle` is macOS-only, so `tauri.conf.json` says
    /// `Overlay` on Omarchy too — and there the controls still sit in a strip of
    /// their own, so an inset is a dead ~60px gap at the left of the header.
    #[test]
    fn overlay_reserves_room_only_on_macos() {
        assert!(controls_overlay_content(TitleBarStyle::Overlay, true));
        assert!(
            !controls_overlay_content(TitleBarStyle::Overlay, false),
            "Linux would get a dead gap where macOS has traffic lights"
        );
    }

    /// The other direction: macOS alone is not the reason to inset. A window
    /// whose controls are back in their own strip needs no room made for them,
    /// and reserving it would put the title 60px adrift of nothing.
    #[test]
    fn a_normal_title_bar_reserves_no_room_on_either_platform() {
        for macos in [true, false] {
            assert!(!controls_overlay_content(TitleBarStyle::Visible, macos), "macos={macos}");
            assert!(!controls_overlay_content(TitleBarStyle::Transparent, macos), "macos={macos}");
        }
    }

    /// What makes the two bands one on macOS, and the one thing here no other
    /// test can see: `get_status` derives `overlay_titlebar` from this file, so
    /// a config that stopped asking for `Overlay` would take the inset with it
    /// and nothing would be wrong — except that the title strip would be back.
    ///
    /// `include_str!`, so the path is checked when this compiles rather than
    /// when it runs, and no test needs a working directory.
    #[test]
    fn the_shipped_window_config_asks_for_a_unified_title_bar() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let w = &conf["app"]["windows"][0];

        assert_eq!(w["titleBarStyle"], "Overlay", "the title bar is back in its own strip");
        assert_eq!(w["hiddenTitle"], true, "\"omacal\" is drawn over the header again");
        // Not a round number for its own sake: 800x600 cannot show a working
        // week and a month grid without crushing both.
        assert_eq!(w["width"], 1280, "the default window is back to a size no calendar fits");
        assert_eq!(w["height"], 800, "the default window is back to a size no calendar fits");
    }

    /// Platform config uses JSON Merge Patch, so the Linux `windows` array
    /// replaces the base array rather than extending its one object. Pin the
    /// duplication: Linux may add transparent backing, but it must not quietly
    /// lose a base-window choice when that object changes later.
    #[test]
    fn linux_adds_transparent_backing_without_changing_the_window() {
        let base: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let linux: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.linux.conf.json")).unwrap();
        let mut linux_window = linux["app"]["windows"][0].clone();

        assert_eq!(
            linux_window["transparent"], true,
            "CSS transparency would stop at an opaque webview backing store",
        );
        linux_window.as_object_mut().unwrap().remove("transparent");
        assert_eq!(
            linux_window, base["app"]["windows"][0],
            "the platform window drifted from the base config",
        );
    }
}
