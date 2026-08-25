// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

mod accounts;
mod browser;
mod caldav_account;
mod cli;
mod caldav_write;
mod calendars;
mod commands;
mod errors;
mod events;
mod fixtures;
/// Test-only: the golden-file mechanism the UI fixtures read. Gated so nothing
/// that reads or writes `ui/tests/` is compiled into the shipped app.
#[cfg(test)]
mod golden;
mod invites;
mod notify;
#[cfg(target_os = "macos")]
mod notify_mac;
mod notify_loop;
#[cfg(target_os = "linux")]
mod nvidia;
mod restart;
mod resume;
mod search;
mod settings;
mod status;
mod sync_loop;
mod tasks;
mod theme;
mod theme_watch;
mod tray;
mod tz_watch;
#[cfg(target_os = "linux")]
mod omarchy_plugin;
mod upcoming;
mod update;
mod weather;
mod write;
mod zoom;

use sqlx::SqlitePool;
use tauri::Manager;

/// Live credentials for one account, held for the process lifetime.
///
/// Deliberately has no `Debug`: a derived one would print both secrets, and a
/// leaked refresh token never expires.
pub struct CachedTokens {
    refresh_token: String,
    access_token: String,
    /// When the access token stops being usable, already carrying its safety margin.
    expires_at_ms: i64,
}

pub struct AppState {
    pub pool: SqlitePool,
    /// True when running on synthetic demo data; surfaced to the UI via
    /// `get_status` so it can show the `DEMO DATA` badge.
    pub demo: bool,
    /// Keyed by account email.
    ///
    /// Without this, every sync re-read the Keychain and re-exchanged a refresh
    /// token that was still valid for the best part of an hour. macOS prompts
    /// for Keychain access per binary, and an unsigned `cargo tauri dev` build
    /// changes hash on every rebuild — so "Always Allow" never stuck and the
    /// prompt returned every few minutes. One read per launch instead.
    pub tokens: tokio::sync::Mutex<std::collections::HashMap<String, CachedTokens>>,
    /// Accounts whose refresh token the token endpoint has pronounced dead —
    /// `invalid_grant` and its siblings, the answers that never change on
    /// retry. Sync skips these instead of re-asking Google every five minutes,
    /// and `get_status` surfaces them so the UI can offer the one thing that
    /// helps: signing in again. Cleared per account by a successful sign-in.
    ///
    /// In memory only, deliberately: on relaunch the first sync re-discovers a
    /// dead token in one request, which is cheaper than keeping a database
    /// column truthful.
    ///
    /// A `BTreeSet` so `get_status` reports a stable order; a blocking mutex
    /// because no holder crosses an await.
    pub reauth: std::sync::Mutex<std::collections::BTreeSet<String>>,
    /// A newer published release, once the daily check has found one —
    /// `None` until then. Set by `update::spawn`, carried to the UI on
    /// `get_status`, and read back by `open_latest_release` for the URL to
    /// open. In memory only: on relaunch the first check re-learns it in one
    /// request, same reasoning as `reauth`.
    pub update: std::sync::Mutex<Option<update::UpdateNotice>>,
    /// When the release endpoint was last asked, ms epoch; `0` for never.
    /// What `update::check_on_focus` rates its floor against, so an
    /// alt-tab is not a request to poll GitHub. Atomic rather than another
    /// mutex: one stamp, written after each check, read on each focus.
    pub update_checked_at: std::sync::atomic::AtomicI64,
    /// The system zone's new IANA name, once `tz_watch` has seen it move out
    /// from under this process — `None` until then. Carried to the UI on
    /// `get_status`, where it becomes the restart banner. In memory only,
    /// and deliberately never cleared while the process lives: the process
    /// zone is fixed at launch, so nothing short of the restart the banner
    /// offers can make the fact stop being true.
    pub system_tz_change: std::sync::Mutex<Option<String>>,
    /// The date a *fresh* launch was asked to open on (`omacal 2026-09-01`
    /// with no instance yet running — the running-instance case never lands
    /// here, it arrives over the single-instance channel while a webview
    /// already listens). Parked because the webview does not exist yet, and
    /// collected exactly once by `take_open_date`: a date is an instruction,
    /// not a state, and a later reader replaying it would yank the calendar
    /// back to a day the user has already navigated away from.
    pub open_date: std::sync::Mutex<Option<String>>,
}

/// The notification transport, managed on its own rather than inside
/// [`AppState`]: it exists only on desktop and only once `setup` has built
/// it, and `AppState` is constructed by tests that must not need one.
/// Readers use `try_state`, so its absence means "post nowhere" instead of a
/// panic.
pub(crate) struct NotifierHandle(pub(crate) std::sync::Arc<dyn notify::Notifier>);

/// The title bar style the shipped `tauri.conf.json` actually asked for.
///
/// Read back off the resolved config rather than restated here, so the window
/// and the header cannot disagree about whether the controls overlay the
/// content — see `status::controls_overlay_content`. `first()`, because this
/// app has exactly one window and the config's own list is what defines it;
/// a config with no window at all cannot have an overlaid title bar, and
/// `TitleBarStyle`'s default (`Visible`) says so.
fn configured_title_bar_style(app: &tauri::AppHandle) -> tauri::TitleBarStyle {
    app.config().app.windows.first().map(|w| w.title_bar_style).unwrap_or_default()
}

#[tauri::command]
async fn get_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<status::AppStatus, String> {
    // `cfg!`, not a runtime probe: this is decided when the binary is built,
    // and the Omarchy build and the macOS build are different binaries.
    let overlay = status::controls_overlay_content(
        configured_title_bar_style(&app),
        cfg!(target_os = "macos"),
    );
    let needs_reauth: Vec<String> =
        state.reauth.lock().expect("reauth mark poisoned").iter().cloned().collect();
    let update = state.update.lock().expect("update notice poisoned").clone();
    let tz_change = state.system_tz_change.lock().expect("tz change poisoned").clone();
    let version = app.package_info().version.to_string();
    let self_update = update::may_self_update(
        update::running_as_appimage(), cfg!(target_os = "macos"), state.demo,
    );
    status::read_status(
        &state.pool, state.demo, overlay, self_update, needs_reauth, update, tz_change, version,
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_palette() -> theme::Palette {
    theme::resolve(theme::omarchy_theme_dir().as_deref())
}

/// The date this process was launched to show, if any — and taking it *is*
/// the read, so a remount can never replay it. See `AppState::open_date`.
#[tauri::command]
fn take_open_date(state: tauri::State<'_, AppState>) -> Option<String> {
    state.open_date.lock().expect("open date poisoned").take()
}

/// The display zone: the user's setting if present, otherwise the system zone.
/// Every day boundary in the week grid is computed against this.
fn display_tz(pool: &SqlitePool) -> String {
    // `settings` is read on the sync task's runtime elsewhere; here we only
    // need a cheap default, so fall back to the system zone.
    let _ = pool;
    jiff::tz::TimeZone::system()
        .iana_name()
        .unwrap_or("UTC")
        .to_string()
}

#[tauri::command]
async fn get_week(
    state: tauri::State<'_, AppState>,
    week_start_ms: i64,
) -> Result<commands::WeekPayload, String> {
    let tz = display_tz(&state.pool);
    // Widen the fetch by a day either side so an event that begins just before
    // the week (or a DST-lengthened final day) is not missed.
    const DAY: i64 = 24 * 3_600_000;
    let events = omacal_store::events_in_window(
        &state.pool,
        week_start_ms - DAY,
        week_start_ms + 8 * DAY,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(commands::assemble_week(&events, week_start_ms, &tz))
}

#[tauri::command]
async fn get_day(
    state: tauri::State<'_, AppState>,
    day_start_ms: i64,
) -> Result<commands::WeekPayload, String> {
    let tz = display_tz(&state.pool);
    // Same widening as `get_week`, for the same reason: an event that begins
    // just before the day, or a DST-lengthened day, must not be missed.
    const DAY: i64 = 24 * 3_600_000;
    let events = omacal_store::events_in_window(
        &state.pool,
        day_start_ms - DAY,
        day_start_ms + 2 * DAY,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(commands::assemble_days(&events, day_start_ms, 1, &tz))
}

#[tauri::command]
async fn get_month(
    state: tauri::State<'_, AppState>,
    year: i32,
    month: u32,
) -> Result<commands::MonthPayload, String> {
    let tz = display_tz(&state.pool);
    // Read once and passed to both, deliberately: `month_grid_start_ms` sizes
    // the fetch window and `assemble_month` recomputes the same anchor, so two
    // separate reads could straddle a settings change and leave the window
    // short of the grid by up to six days.
    let week_start = settings::read_settings(&state.pool).await.week_start;
    let grid_start_ms = commands::month_grid_start_ms(year, month, &tz, week_start);
    // Same widening as `get_week`/`get_day`, for the same reason: an event
    // that begins just before the 42-day grid, or a DST-lengthened edge day,
    // must not be missed.
    const DAY: i64 = 24 * 3_600_000;
    let events = omacal_store::events_in_window(
        &state.pool,
        grid_start_ms - DAY,
        grid_start_ms + 43 * DAY,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(commands::assemble_month(&events, year, month, &tz, week_start))
}

#[tauri::command]
async fn get_year(
    state: tauri::State<'_, AppState>,
    year: i32,
) -> Result<commands::YearPayload, String> {
    let tz = display_tz(&state.pool);
    let week_start = settings::read_settings(&state.pool).await.week_start;
    let year_start_ms = commands::year_start_ms(year, &tz);
    let next_year_start_ms = commands::year_start_ms(year + 1, &tz);
    // Same widening as `get_week`/`get_day`/`get_month`, for the same reason:
    // an event that begins just before the year, or a DST-lengthened edge
    // day, must not be missed.
    const DAY: i64 = 24 * 3_600_000;
    let events = omacal_store::events_in_window(
        &state.pool,
        year_start_ms - DAY,
        next_year_start_ms + DAY,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(commands::assemble_year(&events, year, now_ms(), &tz, week_start))
}

#[tauri::command]
async fn get_big_year(
    state: tauri::State<'_, AppState>,
    year: i32,
) -> Result<commands::BigYearPayload, String> {
    let tz = display_tz(&state.pool);
    let week_start = settings::read_settings(&state.pool).await.week_start;
    get_big_year_impl(&state.pool, year, &tz, now_ms(), week_start).await
}

/// The body of `get_big_year`, minus the Tauri `State` wrapper so it is
/// reachable from a test — same reasoning as `sign_in_impl`.
async fn get_big_year_impl(
    pool: &SqlitePool,
    year: i32,
    tz: &str,
    now: i64,
    week_start: settings::WeekStart,
) -> Result<commands::BigYearPayload, String> {
    let ribbon_start_ms = commands::big_year_start_ms(year, tz, week_start);
    // Same widening as `get_week`/`get_day`/`get_month`/`get_year`, for the
    // same reason: an event that begins just before the 392-day ribbon, or a
    // DST-lengthened edge day, must not be missed.
    const DAY: i64 = 24 * 3_600_000;
    let events = omacal_store::events_in_window(
        pool,
        ribbon_start_ms - DAY,
        ribbon_start_ms + 393 * DAY,
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut payload = commands::assemble_big_year(&events, year, now, tz, week_start);

    // `assemble_big_year` takes only `&[StoredEvent]` — deliberately, so it
    // stays pure over the same shape every other assembler in `commands.rs`
    // does — and so has no calendar list to draw a real name from. Each
    // entry's `name` is a placeholder; this command, which does have the
    // pool, resolves it against `list_calendars` by the entry's own typed
    // `calendar_id` before the payload ever reaches the UI. A calendar that
    // has since been removed leaves the placeholder in place rather than
    // panicking or dropping the entry.
    let calendars = omacal_store::list_calendars(pool).await.map_err(|e| e.to_string())?;
    for entry in &mut payload.legend {
        if let Some(cal) = calendars.iter().find(|c| c.id == entry.calendar_id) {
            entry.name = cal.summary.clone();
        }
    }
    Ok(payload)
}

pub(crate) const KEYRING_SERVICE: &str = "omacal";

/// Google's Calendar API root. A constant so the one place that overrides it —
/// a test pointing `sync_accounts` at a local mock — is the only place a
/// different value can come from.
const GOOGLE_CALENDAR_API: &str = "https://www.googleapis.com/calendar/v3";

#[derive(serde::Deserialize)]
struct Config {
    client_id: String,
    client_secret: String,
}

/// The pair baked into official release builds, and only those: absent unless
/// the build ran with `OMACAL_CLIENT_ID`/`OMACAL_CLIENT_SECRET` set
/// (distribution.md §1). `option_env!` reads them at *compile* time, so a dev
/// build on a machine without the env vars carries `None` and behaves exactly
/// as it always has. The values arrive in CI as repository secrets; they are
/// extractable from any shipped binary regardless — Google's own position for
/// installed apps — and keeping them out of source is what keeps rotation
/// meaningful.
const EMBEDDED_CLIENT_ID: Option<&str> = option_env!("OMACAL_CLIENT_ID");
const EMBEDDED_CLIENT_SECRET: Option<&str> = option_env!("OMACAL_CLIENT_SECRET");
/// Zoom's native/public OAuth client id. Unlike Google's installed-app client,
/// a Zoom PKCE client has no secret; official builds may therefore carry this
/// one non-secret value independently of the Google pair above.
const EMBEDDED_ZOOM_PUBLIC_CLIENT_ID: Option<&str> = option_env!("OMACAL_ZOOM_PUBLIC_CLIENT_ID");

/// Reads `~/.config/omacal/config.toml`, which holds the Google Cloud client
/// credentials (spec §9), falling back to the embedded pair when the file is
/// absent.
fn load_config() -> anyhow::Result<Config> {
    let home = std::env::var("HOME")?;
    let path = std::path::Path::new(&home).join(".config/omacal/config.toml");
    load_config_from(&path, EMBEDDED_CLIENT_ID, EMBEDDED_CLIENT_SECRET)
}

/// The precedence, stated once: **a present `config.toml` always wins** —
/// including by failing loudly when it is malformed or unreadable, because
/// falling back to the embedded pair on a broken file would silently sign the
/// user into a client they did not choose. The embedded pair applies only
/// when the file is *absent* (and only when the build actually baked one in;
/// an empty env var counts as not baked). Only when neither exists does the
/// "no config at …" error appear — the same message as always, which
/// `errors.rs` safelists by that exact prefix.
///
/// Separate from `load_config` and parameterised on the embedded pair because
/// `option_env!` is decided when the binary compiles: a test cannot vary it,
/// but it can vary these arguments, and the precedence is the entire behaviour
/// worth pinning.
fn load_config_from(
    path: &std::path::Path,
    embedded_id: Option<&str>,
    embedded_secret: Option<&str>,
) -> anyhow::Result<Config> {
    match std::fs::read_to_string(path) {
        Ok(src) => Ok(toml::from_str(&src)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fn baked(v: Option<&str>) -> Option<&str> {
                v.filter(|s| !s.trim().is_empty())
            }
            match (baked(embedded_id), baked(embedded_secret)) {
                (Some(id), Some(secret)) => Ok(Config {
                    client_id: id.to_string(),
                    client_secret: secret.to_string(),
                }),
                _ => Err(anyhow::anyhow!(
                    "no config at {}: {e}. Create it with client_id and client_secret.",
                    path.display()
                )),
            }
        }
        Err(e) => Err(anyhow::anyhow!("config at {} is unreadable: {e}", path.display())),
    }
}

#[derive(Default, serde::Deserialize)]
struct ZoomConfig {
    zoom_public_client_id: Option<String>,
}

/// Reads the optional Zoom native-app client id without requiring Google to be
/// configured. This matters for a CalDAV-only user: Zoom conferencing is an
/// independent connection and must not fail merely because `client_id` and
/// `client_secret` are absent from the same file.
pub(crate) fn load_zoom_public_client_id() -> anyhow::Result<Option<String>> {
    let home = std::env::var("HOME")?;
    let path = std::path::Path::new(&home).join(".config/omacal/config.toml");
    load_zoom_public_client_id_from(&path, EMBEDDED_ZOOM_PUBLIC_CLIENT_ID)
}

fn load_zoom_public_client_id_from(
    path: &std::path::Path,
    embedded_id: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let clean = |value: Option<String>| value.filter(|s| !s.trim().is_empty());
    match std::fs::read_to_string(path) {
        Ok(src) => {
            let cfg: ZoomConfig = toml::from_str(&src)?;
            Ok(clean(cfg.zoom_public_client_id))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(clean(embedded_id.map(str::to_string)))
        }
        Err(e) => Err(anyhow::anyhow!("config at {} is unreadable: {e}", path.display())),
    }
}

fn store_refresh_token(email: &str, token: &str) -> anyhow::Result<()> {
    keyring::Entry::new(KEYRING_SERVICE, email)?.set_password(token)?;
    Ok(())
}

fn load_refresh_token(email: &str) -> anyhow::Result<String> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, email)?.get_password()?)
}

/// Runs the full interactive sign-in: loopback listener, browser, code
/// exchange, keyring write, then account and calendar bootstrap.
#[tauri::command]
async fn sign_in(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let email = sign_in_impl(&state.pool, state.demo).await?;
    forget_stale_credentials(
        &mut *state.tokens.lock().await,
        &mut state.reauth.lock().expect("reauth mark poisoned"),
        &email,
    );
    Ok(email)
}

/// What a successful (re-)sign-in resets for its account.
///
/// The cached tokens first: `access_token_for` prefers the cache's refresh
/// token over the keyring's, so after a re-sign-in the cache still holds the
/// dead pair that forced it — and would keep presenting it, turning the fresh
/// consent into no fix at all. The reauth mark second, so the account rejoins
/// the sync rotation. Other accounts' entries are none of this function's
/// business: reconnecting one account must not log out another's cache.
fn forget_stale_credentials(
    tokens: &mut std::collections::HashMap<String, CachedTokens>,
    reauth: &mut std::collections::BTreeSet<String>,
    email: &str,
) {
    tokens.remove(email);
    reauth.remove(email);
}

/// What the user reads when no browser could be started. Fixed literal, in
/// `errors::SAFE_EXACT`: a launcher failure carries no secret, and the
/// opaque fallback is what cost issue #1's reporter their afternoon — the
/// one fact that pointed at the cause was the one thing withheld.
pub(crate) const BROWSER_FAILED: &str =
    "Could not open a browser. Open your browser once and try again.";

/// The body of `sign_in`, minus the Tauri `State` wrapper so it is reachable
/// from a test. The demo gate is the first statement for the same reason it is
/// in `sync_now`: everything below it reads the config file, the keyring and
/// Google, and demo mode has no business touching any of the three.
async fn sign_in_impl(pool: &SqlitePool, demo: bool) -> Result<String, String> {
    demo_sync_guard(demo)?;

    async fn inner(pool: &SqlitePool) -> anyhow::Result<String> {
        let cfg = load_config()?;
        let pkce = omacal_google::auth::generate_pkce();
        let (listener, redirect_uri) = omacal_google::auth::bind_loopback()?;
        let csrf = omacal_google::auth::generate_pkce().verifier;

        let url = omacal_google::auth::authorize_url(
            &cfg.client_id, &redirect_uri, &pkce.challenge, &csrf,
        );
        // Through `browser`, not `open::that`: an AppImage's environment
        // crashes the browser it spawns (issue #1). And loudly on failure —
        // this exact spot failed silently on Arch for three releases, with
        // nothing logged and the message withheld by `errors::user_facing`.
        tracing::info!("sign-in: opening the consent page");
        crate::browser::open_external(&url).map_err(|e| {
            tracing::warn!(%e, "sign-in: could not open a browser");
            anyhow::anyhow!(BROWSER_FAILED)
        })?;

        // Deadline on the listener, not on this future: a `tokio::time::timeout`
        // here would return while the blocking thread stayed parked in
        // `accept()` for the life of the process.
        let redirect = tokio::task::spawn_blocking(move || {
            omacal_google::auth::wait_for_redirect(
                listener,
                omacal_google::auth::SIGN_IN_TIMEOUT,
            )
        })
        .await??;

        if redirect.state != csrf {
            anyhow::bail!("state mismatch — possible CSRF, sign-in aborted");
        }

        let tokens = omacal_google::auth::exchange_code(
            omacal_google::auth::TOKEN_ENDPOINT,
            &cfg.client_id, &cfg.client_secret,
            &redirect.code, &pkce.verifier, &redirect_uri,
        )
        .await?;

        let client =
            omacal_google::CalendarClient::new(GOOGLE_CALENDAR_API, &tokens.access_token);
        let calendars = client.list_calendars().await?;

        // The primary calendar's id is the account's email address, so we get
        // the identity without requesting a userinfo scope.
        let email = calendars
            .iter()
            .find(|c| c.primary)
            .map(|c| c.id.clone())
            .ok_or_else(|| anyhow::anyhow!("account has no primary calendar"))?;

        if let Some(rt) = &tokens.refresh_token {
            store_refresh_token(&email, rt)?;
        } else {
            anyhow::bail!("Google returned no refresh token — revoke the app's access and retry");
        }

        // `google_sub` keys the account. We use the email, which is stable for
        // our single-user case; Plan 5 may switch to the real `sub` from an
        // id_token when multiple accounts land.
        let account_id: i64 = sqlx::query_scalar(
            "INSERT INTO accounts (google_sub, email, created_at) VALUES (?1, ?1, ?2)
             ON CONFLICT (google_sub) DO UPDATE SET email = excluded.email
             RETURNING id",
        )
        .bind(&email)
        .bind(now_ms())
        .fetch_one(pool)
        .await?;

        for c in &calendars {
            upsert_calendar(pool, account_id, c).await?;
        }

        Ok(email)
    }

    inner(pool).await.map_err(|e| errors::user_facing(&e))
}

/// Records one calendar Google listed for an account.
///
/// The `ON CONFLICT` clause updates Google's own fields and *deliberately*
/// omits `selected` and `sync_enabled`. Those two belong to the user, not to
/// Google, and this statement runs on every sign-in — including the re-sign-in
/// that "Add account" plus `prompt=select_account` makes one click away. Adding
/// them here would silently undo every removal and every hide the moment the
/// user re-picked an account they had already connected, and nothing else in
/// the codebase would notice.
///
/// `default_reminders_json` *is* updated, on the other side of that same line:
/// it is Google's value, not the user's, and this app has no UI that sets it.
///
/// This is the only writer of that column, and it runs on sign-in alone —
/// nothing refreshes the calendar list on a timer. A calendar whose defaults
/// change in Google's own settings keeps the values from the last sign-in
/// until the account is re-connected.
async fn upsert_calendar(
    pool: &SqlitePool,
    account_id: i64,
    c: &omacal_google::model::Calendar,
) -> anyhow::Result<()> {
    // Mapped through `omacal_sync` rather than serialising the wire type
    // directly, so the stored shape is the same one `StoredEvent` reads back.
    let default_reminders =
        serde_json::to_string(&omacal_sync::from_google_reminders(&c.default_reminders))?;
    sqlx::query(
        "INSERT INTO calendars
             (account_id, google_id, summary, color_hex, timezone, access_role, is_primary,
              default_reminders_json)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT (account_id, google_id) DO UPDATE SET
             summary = excluded.summary, color_hex = excluded.color_hex,
             timezone = excluded.timezone, access_role = excluded.access_role,
             default_reminders_json = excluded.default_reminders_json",
    )
    .bind(account_id)
    .bind(&c.id)
    .bind(&c.summary)
    .bind(&c.background_color)
    .bind(c.time_zone.as_deref().unwrap_or("UTC"))
    .bind(&c.access_role)
    .bind(c.primary as i64)
    .bind(default_reminders)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The message every Google-reaching command returns in demo mode, before any
/// config or keyring I/O runs. The demo account (`fixtures::seed_demo`) is a
/// real `accounts` row but was never through OAuth, so without this gate
/// `load_config` or `load_refresh_token` would fail and surface a raw
/// technical string — `"No matching credential found"` — as the very first
/// thing a new user sees.
///
/// Worded for both actions it guards: Sync now, and Connect Google Calendar.
///
/// A second, separate demo refusal exists at `events::create_impl` ("demo
/// mode — there is nothing to create") rather than reusing this constant.
/// Two reasons, not one: this message's own wording ("nothing to connect or
/// sync") does not describe creating an event, and it starts with a capital
/// D — `create_impl`'s own guard test asserts `err.contains("demo")` on the
/// raw `anyhow` message (checked before that error ever reaches
/// `errors::user_facing`), which a leading-capital string cannot satisfy.
/// If a third demo-gated write command shows up, give it the same treatment
/// rather than growing this constant's wording to cover three unrelated
/// actions.
const DEMO_SYNC_MESSAGE: &str =
    "Demo mode — this is synthetic data, so there is nothing to connect or sync.";

/// `Err` when `demo` is true, `Ok` otherwise. A plain function of the flag —
/// no config or keyring I/O anywhere near it — so callers that check it first
/// (`sync_now`, `sign_in`, and the background loop) cannot reach that I/O in
/// demo mode.
pub(crate) fn demo_sync_guard(demo: bool) -> Result<(), String> {
    if demo {
        Err(DEMO_SYNC_MESSAGE.to_string())
    } else {
        Ok(())
    }
}

/// Whether a cached entry can be used as-is.
///
/// Pulled out so the decision is testable: an untested condition guarding a
/// Keychain read is what produced the prompt-every-few-minutes behaviour.
fn cached_is_usable(entry: Option<&CachedTokens>, now_ms: i64) -> bool {
    entry.is_some_and(|t| t.expires_at_ms > now_ms)
}

/// Returns a usable access token for `email`, reading the Keychain and
/// refreshing only when the cached one is absent or expired.
///
/// This is the whole reason the cache exists: the Keychain read is what raises
/// the macOS access prompt, and doing it on every sync made the app unusable.
async fn access_token_for(state: &AppState, cfg: &Config, email: &str) -> anyhow::Result<String> {
    {
        let cache = state.tokens.lock().await;
        if cached_is_usable(cache.get(email), now_ms()) {
            return Ok(cache[email].access_token.clone());
        }
    } // lock dropped before the network call

    // Prefer a refresh token we already hold; only touch the Keychain if we
    // have none for this account yet.
    let refresh_token = {
        let cache = state.tokens.lock().await;
        cache.get(email).map(|t| t.refresh_token.clone())
    };
    let refresh_token = match refresh_token {
        Some(rt) => rt,
        None => load_refresh_token(email)?,
    };

    let fresh = omacal_google::auth::refresh(
        omacal_google::auth::TOKEN_ENDPOINT,
        &cfg.client_id,
        &cfg.client_secret,
        &refresh_token,
    )
    .await?;

    let access_token = fresh.access_token.clone();
    let mut cache = state.tokens.lock().await;
    cache.insert(
        email.to_string(),
        CachedTokens {
            // Google omits refresh_token on refresh; keep the one we used.
            refresh_token: fresh.refresh_token.unwrap_or(refresh_token),
            access_token: fresh.access_token,
            expires_at_ms: fresh.expires_at_ms,
        },
    );
    Ok(access_token)
}

/// The calendars a sync should fetch for one account.
///
/// `sync_enabled`, deliberately — not `selected`. Hiding a calendar in the UI
/// must not stop it syncing, or re-showing it would reveal a gap until the
/// next full sync.
///
/// `ORDER BY id` for the same reason `sync_all` orders its accounts: without
/// it, which calendar is fetched first is whatever the query planner's chosen
/// index happens to yield, and that decided whose failure was visible.
pub(crate) async fn calendars_to_sync(
    pool: &SqlitePool,
    account_id: i64,
) -> anyhow::Result<Vec<(i64, String)>> {
    let cals = sqlx::query_as(
        "SELECT id, google_id FROM calendars
         WHERE account_id = ?1 AND sync_enabled = 1
         ORDER BY id",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    Ok(cals)
}

/// Syncs every calendar of every account, isolating failures.
///
/// The token source is injected so this is reachable from a test: the real one
/// reads the Keychain and talks to Google, neither of which belongs in a test,
/// and the property worth proving is precisely what happens when it fails.
///
/// Nothing here uses `?`. Before multi-account, "this account failed" and "the
/// sync failed" were the same sentence and a `?` was the honest spelling of
/// both. They are different sentences now: one account with revoked consent
/// answers `invalid_grant` on every refresh forever, and a shared calendar
/// unshared behind our back answers 403 or 404 forever — neither is a reason to
/// stop syncing anything else.
///
/// Returns the rows written, a label per failed account or calendar, and the
/// accounts whose refresh token is dead for good. The labels are for the log;
/// the caller turns their *count* into the user-facing failure, because an
/// email address has no business being handed to a user-facing string.
///
/// Dead tokens are their own list, not entries in `failed`: a transient
/// failure's story is "it will keep trying", and for a dead grant that story
/// is false — retrying is quota spent on an answer that cannot change. The
/// caller marks these accounts, sync stops touching them, and the UI offers
/// the only fix there is (sign in again). Their emails *are* shown to the
/// user, which is fine on this path: it is the user's own account list, in
/// the status payload — not an error string crossing `errors::user_facing`.
async fn sync_accounts<F, Fut>(
    pool: &SqlitePool,
    accounts: &[(i64, String)],
    api_base: &str,
    window_start_ms: i64,
    window_end_ms: i64,
    token_for: F,
) -> (u64, Vec<String>, Vec<String>)
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<String>>,
{
    let mut total = 0u64;
    let mut failed: Vec<String> = Vec::new();
    let mut dead: Vec<String> = Vec::new();

    for (account_id, email) in accounts {
        // `%e`, never the token: `Tokens` has a hand-written redacting `Debug`
        // and no credential is interpolated anywhere on this path.
        let access_token = match token_for(email.clone()).await {
            Ok(t) => t,
            Err(e) if omacal_google::auth::needs_reauth(&e) => {
                tracing::warn!(account = %email, %e,
                    "refresh token is dead; the account needs to be reconnected");
                dead.push(email.clone());
                continue;
            }
            Err(e) => {
                tracing::warn!(account = %email, %e, "no usable access token; skipping this account");
                failed.push(email.clone());
                continue;
            }
        };

        let client = omacal_google::CalendarClient::new(api_base, &access_token);

        let cals = match calendars_to_sync(pool, *account_id).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(account = %email, %e, "could not list calendars; skipping this account");
                failed.push(email.clone());
                continue;
            }
        };

        for (cal_id, google_id) in cals {
            match omacal_sync::sync_calendar(
                pool, &client, cal_id, &google_id, window_start_ms, window_end_ms,
            )
            .await
            {
                Ok(out) => total += (out.upserted + out.deleted) as u64,
                Err(e) => {
                    tracing::warn!(account = %email, calendar = %google_id, %e, "calendar sync failed");
                    failed.push(format!("{email} / {google_id}"));
                }
            }
        }
    }

    (total, failed, dead)
}

/// The accounts a sync should even attempt: everyone not marked as needing
/// re-consent. A marked account's refresh is a request whose answer is
/// already known — `invalid_grant` does not get better with retrying — spent
/// from a shared quota and logged as a fresh warning every five minutes. The
/// account rejoins the rotation when a sign-in clears its mark.
fn accounts_to_attempt(
    accounts: Vec<(i64, String)>,
    marked: &std::collections::BTreeSet<String>,
) -> Vec<(i64, String)> {
    accounts.into_iter().filter(|(_, email)| !marked.contains(email)).collect()
}

/// Turns what a sync managed to do into what it reports.
///
/// The whole point of isolating failures is that they still get *reported*.
/// Drop this decision and a sync in which every account failed returns `Ok`:
/// `record_sync` runs, the header reads "Synced just now", and no `sync-failed`
/// event ever fires. That is strictly worse than the `?` this replaced — the
/// `?` at least stopped loudly, whereas silent isolation fails forever without
/// telling anyone.
///
/// A count, never the labels. This string is what `errors::user_facing` sees,
/// and the labels are account email addresses; the detail is already in the
/// log, at the level the rest of this file uses. Partial success is still
/// failure: rows written by the accounts that worked do not make the account
/// that did not any less stale.
///
/// A separate function so the decision can be asserted directly — `sync_all`
/// itself is not reachable from a test, because it reads the real config file.
fn sync_result(total: u64, failed: &[String]) -> anyhow::Result<u64> {
    if !failed.is_empty() {
        anyhow::bail!("{} of this sync's targets failed; see the log", failed.len());
    }
    Ok(total)
}

/// The window the app keeps synced.
///
/// Extracted so the year views and the sync loop cannot disagree about where
/// the edge is: both render decisions ("is this date fetched?") and fetch
/// decisions ("what should I ask Google for?") must read one definition.
pub(crate) fn synced_window(now_ms: i64) -> (i64, i64) {
    const DAY: i64 = 24 * 3_600_000;
    (now_ms - 180 * DAY, now_ms + 365 * DAY)
}

/// Refreshes the access token and syncs every calendar of every account.
///
/// Pure sync work, with no demo check and no status bookkeeping of its own —
/// shared by the `sync_now` command and the background loop (Task 4), each of
/// which handles the demo gate and `record_sync` itself.
pub(crate) async fn sync_all(state: &AppState) -> anyhow::Result<u64> {
    let pool = &state.pool;
    // `ORDER BY id`, so which account goes first is a decision rather than
    // whatever rowid order happens to be — it used to decide whose failure
    // stopped the rest.
    // (id, email, provider, server_url, username)
    type AccountRow = (i64, String, String, Option<String>, Option<String>);
    let all: Vec<AccountRow> = sqlx::query_as(
        "SELECT id, email, provider, server_url, username FROM accounts ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let marked = state.reauth.lock().expect("reauth mark poisoned").clone();
    let google: Vec<(i64, String)> = all
        .iter()
        .filter(|(_, _, p, _, _)| p == "google")
        .map(|(id, email, _, _, _)| (*id, email.clone()))
        .collect();
    let google = accounts_to_attempt(google, &marked);

    let now = now_ms();
    let (window_start, window_end) = synced_window(now);

    let mut total = 0u64;
    let mut failed: Vec<String> = Vec::new();
    let mut dead: Vec<String> = Vec::new();

    // `load_config` moved inside the Google branch: a CalDAV-only install
    // has no Google client pair anywhere and must not need one to sync.
    if !google.is_empty() {
        let cfg = load_config()?;
        let cfg = &cfg;
        let (t, f, d) = sync_accounts(
            pool,
            &google,
            GOOGLE_CALENDAR_API,
            window_start,
            window_end,
            move |email| async move { access_token_for(state, cfg, &email).await },
        )
        .await;
        total += t;
        failed.extend(f);
        dead.extend(d);
    }

    for (account_id, email, _, server_url, username) in
        all.iter().filter(|(_, _, p, _, _)| p == "caldav")
    {
        if marked.contains(email) {
            continue; // dead credentials; the banner is already up
        }
        let client =
            match caldav_account::client_for(email, server_url.as_deref(), username.as_deref()) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(account = %email, %e, "no usable CalDAV credentials");
                    failed.push(email.clone());
                    continue;
                }
            };
        match caldav_account::sync_account(pool, *account_id, &client, window_start, window_end)
            .await
        {
            Ok(n) => total += n,
            Err(e) if e.needs_reauth() => {
                tracing::warn!(account = %email,
                    "the CalDAV password was rejected; the account needs to be reconnected");
                dead.push(email.clone());
            }
            Err(e) => {
                tracing::warn!(account = %email, %e, "CalDAV sync failed");
                failed.push(email.clone());
            }
        }
    }

    if !dead.is_empty() {
        state.reauth.lock().expect("reauth mark poisoned").extend(dead);
    }

    sync_result(total, &failed)
}

/// Refreshes the access token and syncs every calendar of every account.
#[tauri::command]
async fn sync_now(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<u64, String> {
    // Checked here, at the command boundary, rather than inside `sync_all` —
    // Task 4's background sync loop is a second caller that needs its own
    // demo check, and burying this one inside `sync_all` would leave that
    // caller with no way to see it.
    demo_sync_guard(state.demo)?;

    let n = sync_all(&state).await.map_err(|e| errors::user_facing(&e))?;
    status::record_sync(&state.pool, now_ms()).await.map_err(|e| e.to_string())?;
    invites::after_sync(&app).await;
    Ok(n)
}

/// Exports `TZ` from the display-tz sidecar, and **must run first thing in
/// `main`** — before GTK, the webview, or anything else resolves the local
/// zone, because they all capture it at process start and never ask again.
/// That constraint is also why this cannot use Tauri's path resolver or the
/// database: neither exists yet. The directory below is `app_data_dir` for
/// identifier `com.omacal.app`, spelled by hand; if the two ever diverge the
/// failure is the soft one — the setting silently stays on the system zone —
/// and `setup`'s sidecar re-sync writes to the real dir every launch, so a
/// divergence would also heal itself there.
pub fn apply_display_tz_early() {
    #[cfg(target_os = "linux")]
    let dir = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| std::path::PathBuf::from(h).join(".local/share"))
        })
        .map(|d| d.join("com.omacal.app"));
    #[cfg(target_os = "macos")]
    let dir = std::env::var_os("HOME")
        .map(|h| std::path::PathBuf::from(h).join("Library/Application Support/com.omacal.app"));
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let dir: Option<std::path::PathBuf> = None;

    let Some(dir) = dir else { return };
    let sidecar = std::fs::read_to_string(dir.join(settings::DISPLAY_TZ_SIDECAR))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    match tz_action(sidecar.as_deref(), std::env::var_os(TZ_MARKER).is_some()) {
        TzAction::Set(tz) => {
            std::env::set_var("TZ", tz);
            std::env::set_var(TZ_MARKER, "1");
        }
        TzAction::Clear => {
            std::env::remove_var("TZ");
            std::env::remove_var(TZ_MARKER);
        }
        TzAction::Leave => {}
    }
}

/// The marker that says *omacal* exported `TZ`, as opposed to the user's own
/// shell. It exists for the restart path: `app.restart()` re-execs with the
/// current environment inherited, `TZ` included — so returning to "System
/// default" found no sidecar, did nothing, and the inherited zone silently
/// survived (caught in the very first field test, 2026-08-19). Clearing must
/// undo our export and only ours; a `TZ` the user launched us under is
/// theirs to keep.
const TZ_MARKER: &str = "OMACAL_SET_TZ";

#[derive(Debug, PartialEq)]
enum TzAction {
    Set(String),
    Clear,
    Leave,
}

/// The whole decision, pure so it is testable without mutating the test
/// process's real environment.
fn tz_action(sidecar: Option<&str>, we_set_it: bool) -> TzAction {
    match (sidecar, we_set_it) {
        (Some(tz), _) => TzAction::Set(tz.to_string()),
        (None, true) => TzAction::Clear,
        (None, false) => TzAction::Leave,
    }
}

/// The single-instance plugin, with the one wrinkle a sandbox adds.
///
/// The bus name is `<dbus_id>.SingleInstance`, and `dbus_id` defaults to the
/// Tauri identifier — `com.omacal.app`, which is fine everywhere the app
/// owns its session bus outright. Inside a Flatpak it is not: the sandbox
/// grants only names under the Flatpak app id, and Flathub's linter refuses
/// `--own-name` for anything else as an exception it "never grants". So
/// where `FLATPAK_ID` is set — which is only ever inside a Flatpak — the
/// name becomes `<app id>.SingleInstance` and needs no permission at all.
/// Every other build keeps the name it has always had.
/// The one dispatcher for clicked notification actions, whichever transport
/// reported the click — D-Bus on Linux, the notification centre's delegate on
/// macOS. Everything it does beyond routing lives in tested functions; it is
/// a free function rather than a closure inside one platform's setup so the
/// other platform cannot grow a second, slightly different copy.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn dispatch_notification_action(handle: &tauri::AppHandle, action: notify::Action) {
    match action {
        notify::Action::AcceptInvite { event_id, start_ms } => {
            let app = handle.clone();
            tauri::async_runtime::spawn(async move {
                invites::accept_from_notification(app, event_id, start_ms).await;
            });
        }
        notify::Action::Join(uri) => {
            if let Err(e) = tauri_plugin_opener::open_url(&uri, None::<&str>) {
                tracing::warn!(%e, "could not open the meeting link");
            }
        }
        // Still display-only; re-queueing is §2.5's future.
        notify::Action::Snooze5m => {}
        // The click itself: bring the window up, then hand the webview the
        // occurrence — it lands and opens the popover exactly as a chosen
        // search hit does.
        notify::Action::OpenEvent { event_id, start_ms, end_ms } => {
            use tauri::Emitter;
            tray::show_main_window(handle);
            let _ = handle.emit(notify::OPEN_EVENT_EVENT, serde_json::json!({
                "id": event_id, "startMs": start_ms, "endMs": end_ms,
            }));
        }
    }
}

fn single_instance_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    let mut builder = tauri_plugin_single_instance::Builder::new().callback(
        |app: &tauri::AppHandle, argv: Vec<String>, _cwd: String| {
            match tray::instance_action(&argv) {
                tray::TrayAction::Quit => app.exit(0),
                tray::TrayAction::SyncNow => sync_loop::request_now(app),
                tray::TrayAction::OpenAt(ymd) => tray::open_at(app, &ymd),
                tray::TrayAction::Open => tray::show_main_window(app),
            }
        },
    );
    if let Ok(id) = std::env::var("FLATPAK_ID") {
        builder = builder.dbus_id(id);
    }
    builder.build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // The CLI door, before anything else exists — before tracing (a --json
    // stream must not carry log lines), before GTK, before the
    // single-instance plugin could forward a subcommand to the running GUI.
    // A recognised subcommand runs to an exit inside this call; everything
    // else falls straight through to the app omacal has always been.
    cli::maybe_run_and_exit();

    tracing_subscriber::fmt::init();

    // Before the builder is even assembled: GTK and WebKit read the
    // environment when they initialise, and this may need to change it.
    #[cfg(target_os = "linux")]
    nvidia::apply_if_needed();

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        // First, before every other plugin, per its own docs — a second
        // process must be turned away before anything else initialises.
        // Closing the window only hides omacal (see `tray`), so "start it
        // again" used to mean a second full instance: two schedulers, two
        // tray icons, and every reminder twice. Now it means the running
        // instance does what the argv asks — show the window on a bare
        // launch, or the tray menu's other two actions via `--sync-now` and
        // `--quit`, which is how a surface outside this process (the Omarchy
        // bar widget) drives the app. See `tray::instance_action`.
        .plugin(single_instance_plugin())
        .plugin(tauri_plugin_opener::init())
        // The self-updater behind the banner's "Update" button. Registration
        // is unconditional; whether the button exists at all is
        // `update::may_self_update`'s decision — AppImage only, never demo —
        // enforced again inside `update::install_update` for a webview that
        // did not take no for an answer.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Registered unconditionally; *enabling* it is what the demo guard
        // gates, in `setup` below. Registration alone adds no login item.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));

    // Registered so `app.notification()` resolves for `notify::MacNotifier`.
    // Registration alone posts nothing — it makes the transport available, and
    // Task 5 decides whether the loop that would use it ever starts.
    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_plugin_notification::init());
    }

    builder
        .setup(|app| {
            // The control flags are messages to a RUNNING instance (see
            // `tray::instance_action`), and the single-instance plugin is the
            // messenger: a second process delivers its argv during plugin
            // init and never reaches here. Reaching `setup` with `--quit`
            // therefore means there was nothing to quit — honour the intent
            // by leaving before anything is built, rather than starting a
            // whole calendar app in order to close it. Checked HERE and not
            // at the top of `run`, where it once was: an early return there
            // kills the messenger before it delivers, and `--quit` against a
            // running app silently does nothing (found live, the hard way).
            // `--sync-now` with nothing running falls through to a normal
            // launch, which syncs on startup anyway.
            if std::env::args().any(|a| a == "--quit") {
                std::process::exit(0);
            }

            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;

            // Owner-only on the whole directory, every launch: the store
            // tightens the database files themselves, but WebKit drops its
            // caches here too, and 0700 on the directory covers every file
            // anything adds later. Best-effort for the store's reason — a
            // filesystem without POSIX modes is no reason to refuse to start.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
            }

            // Demo mode writes to its own database file, never the real one, so
            // a user exploring the demo can never end up with synthetic events
            // mixed into their actual calendar store.
            let demo = fixtures::demo_mode();
            let db_name = if demo { "omacal-demo.db" } else { "omacal.db" };
            let url = format!("sqlite://{}", dir.join(db_name).display());

            // Block once at startup: nothing can render before migrations run.
            let pool = tauri::async_runtime::block_on(omacal_store::connect(&url))?;

            // Re-sync the display-tz sidecar from the database row — the row
            // is the truth the settings UI edits, the sidecar is what
            // `apply_display_tz_early` could reach before Tauri existed, and
            // a crash between the setter's two writes must cost one stale
            // restart, not a permanent disagreement.
            {
                let tz = tauri::async_runtime::block_on(settings::read_settings(&pool))
                    .display_timezone;
                if let Err(e) = settings::write_tz_sidecar(&dir, tz.as_deref()) {
                    tracing::warn!(%e, "could not re-sync the display-tz sidecar");
                }
            }

            if demo {
                let now = now_ms();
                let seeded = tauri::async_runtime::block_on(fixtures::seed_demo(&pool, now))?;
                tracing::warn!(seeded, db = db_name, "DEMO MODE — synthetic data, not your calendar");
            }

            // The GTK headerbar, off. `titleBarStyle: "Overlay"` in
            // tauri.conf.json is the macOS half of the same intent — traffic
            // lights over content, no bar — but Linux ignores that option and
            // draws its full client-side titlebar, which on a tiled Hyprland
            // desktop only duplicates the compositor (SUPER+W closes,
            // SUPER+drag moves) and costs a bar's height of calendar. Runtime
            // rather than a tauri.linux.conf.json, because the platform-merge
            // replaces the whole `windows` array and a second copy of the
            // window object is one that drifts.
            #[cfg(target_os = "linux")]
            if let Some(w) = tauri::Manager::get_webview_window(app, "main") {
                let _ = w.set_decorations(false);
            }

            // A dated fresh launch (`omacal 2026-09-01` with nothing running)
            // reaches setup rather than the single-instance channel. Read off
            // the same vocabulary that channel uses, so the two entrances
            // cannot drift.
            let open_date = match tray::instance_action(&std::env::args().collect::<Vec<_>>()) {
                tray::TrayAction::OpenAt(ymd) => Some(ymd),
                _ => None,
            };

            app.manage(AppState {
                pool, demo,
                tokens: Default::default(),
                reauth: Default::default(),
                update: Default::default(),
                update_checked_at: Default::default(),
                system_tz_change: Default::default(),
                open_date: std::sync::Mutex::new(open_date),
            });
            sync_loop::spawn(app.handle().clone());
            theme_watch::spawn(app.handle().clone());
            #[cfg(target_os = "linux")]
            tz_watch::spawn(app.handle().clone());
            // Seed the upcoming feed from whatever is already in the store,
            // so a bar widget has an answer before the first sync completes.
            {
                let state = app.state::<AppState>();
                upcoming::refresh_soon(state.pool.clone(), state.demo);
                // And on Omarchy, make sure that widget exists at all — the
                // app is its install medium (see `omarchy_plugin`).
                #[cfg(target_os = "linux")]
                omarchy_plugin::spawn(state.pool.clone(), state.demo);
            }
            update::spawn(app.handle().clone());
            // The forecast for the day headers — same demo gate as every
            // other loop, applied inside.
            weather::spawn(app.handle().clone());
            #[cfg(target_os = "linux")]
            resume::spawn(app.handle().clone());

            // The tray is the default way to quit, since closing the window
            // hides it. A failure here is logged rather than fatal: an app
            // that refuses to start because a system tray is unavailable is
            // worse than one running without a tray icon. Built even when the
            // setting hides it — hidden-then-shown is one toggle, while
            // never-built could not come back without a restart.
            if let Err(e) = tray::build(app.handle()) {
                tracing::warn!(%e, "could not build the tray icon");
            } else {
                let settings =
                    tauri::async_runtime::block_on(settings::read_settings(&app.state::<AppState>().pool));
                if !settings.tray_icon {
                    tray::set_visible(app.handle(), false);
                }
            }

            // Start on login (§2.6) — never in demo mode.
            if tray::may_autostart(demo) {
                use tauri_plugin_autostart::ManagerExt;
                if let Err(e) = app.autolaunch().enable() {
                    tracing::warn!(%e, "could not register start-on-login");
                }
            }

            // The scheduler. `run_once` refuses in demo mode on its own — see
            // `notify_loop::may_notify` — so this starts either way and the
            // guard stays in the one place a test can reach it.
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                #[cfg(target_os = "linux")]
                let notifier: std::sync::Arc<dyn notify::Notifier> = {
                    let dbus = std::sync::Arc::new(notify::DbusNotifier::default());
                    // Wired here because it is the only place that has the
                    // app handle the actions need.
                    let handle = app.handle().clone();
                    dbus.set_on_action(std::sync::Arc::new(move |action| {
                        dispatch_notification_action(&handle, action);
                    }));
                    dbus
                };
                #[cfg(target_os = "macos")]
                let notifier: std::sync::Arc<dyn notify::Notifier> = {
                    // The real notification centre, with the buttons and the
                    // click — possible at all only in a signed bundle, which
                    // v0.5.0 made the shipping reality (§2.4 as amended).
                    // `new` refuses when the process runs unbundled (`cargo
                    // tauri dev`), where `UNUserNotificationCenter` would
                    // throw; the legacy plugin path stays for exactly that
                    // run, failing quietly as it always has.
                    let handle = app.handle().clone();
                    match notify_mac::UnNotifier::new(std::sync::Arc::new(move |action| {
                        dispatch_notification_action(&handle, action);
                    })) {
                        Some(un) => std::sync::Arc::new(un),
                        None => std::sync::Arc::new(notify::MacNotifier { app: app.handle().clone() }),
                    }
                };
                // Managed so the invite pass (`invites::after_sync`) and the
                // click handler can post through the same transport the
                // reminder loop uses.
                app.manage(NotifierHandle(notifier.clone()));
                notify_loop::spawn(app.handle().clone(), notifier);
            }

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::Focused(true) => {
                sync_loop::request_now(window.app_handle());
                // The update notice rides the same "the user is back" signal,
                // behind its own four-hour floor: an instance launched 45
                // minutes before a release used to stay silent about it for
                // a day — the exact day the Update button matters most.
                update::check_on_focus(window.app_handle());
            }
            // §2.6: closing hides. The scheduler is the whole point of the
            // app, and a closed window that silently stopped firing reminders
            // would be a bug rather than a feature. Quit is explicit, from the
            // tray.
            tauri::WindowEvent::CloseRequested { api, .. } if tray::hide_instead_of_closing() => {
                api.prevent_close();
                let _ = window.hide();
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            invites::pending_invites,
            invites::declined_guests,
            invites::dismiss_decline_notice,
            invites::dismiss_all_decline_notices,
            invites::changed_meetings,
            invites::dismiss_change_notice,
            invites::dismiss_all_change_notices,
            get_palette,
            get_week,
            get_day,
            get_month,
            get_year,
            get_big_year,
            get_status,
            take_open_date,
            sign_in,
            sync_now,
            update::open_latest_release,
            update::install_update,
            commands::open_conference,
            weather::get_weather,
            calendars::get_calendars,
            calendars::set_calendar_selected,
            calendars::set_calendar_sync,
            calendars::set_calendar_color,
            search::search_events,
            events::known_guests,
            settings::get_settings,
            settings::set_sync_interval,
            settings::set_notifications_enabled,
            settings::set_tray_icon,
            settings::set_weather_enabled,
            settings::set_display_timezone,
            settings::set_second_timezone,
            settings::list_timezones,
            settings::restart_app,
            caldav_account::connect_caldav,
            accounts::list_accounts,
            accounts::sign_out,
            zoom::zoom_status,
            zoom::connect_zoom,
            zoom::disconnect_zoom,
            tasks::list_tasks,
            tasks::set_task_completed,
            tasks::create_task,
            tasks::delete_task_cmd,
            tasks::task_lists,
            settings::set_list_mode,
            settings::set_fallback_reminders,
            settings::set_default_calendar,
            settings::set_time_format,
            settings::set_week_start,
            events::event_detail,
            events::respond_to_event,
            events::refresh_event,
            events::create_event,
            events::update_event,
            events::delete_event_cmd
        ])
        .run(tauri::generate_context!())
        .expect("error while running omacal");
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// The restart-inheritance table, pinned. `app.restart()` re-execs with
    /// the environment inherited, so "System default" must *clear* a TZ this
    /// app exported — the first field test proved that leg (choose Sofia,
    /// return to system, stay in Sofia) — while a TZ the user launched us
    /// under, marker absent, is theirs and stays.
    #[test]
    fn the_tz_decision_sets_clears_ours_and_leaves_theirs() {
        assert_eq!(
            tz_action(Some("Europe/Sofia"), false),
            TzAction::Set("Europe/Sofia".into())
        );
        assert_eq!(
            tz_action(Some("Asia/Kolkata"), true),
            TzAction::Set("Asia/Kolkata".into()),
            "changing zones overwrites the inherited export"
        );
        assert_eq!(tz_action(None, true), TzAction::Clear, "back to system undoes OUR export");
        assert_eq!(tz_action(None, false), TzAction::Leave, "the user's own TZ is not ours to touch");
    }

    /// `sync_now` and `sign_in` both call this before doing anything else, so
    /// proving the guard itself never performs I/O — it is a pure function of
    /// the flag — proves neither command can reach `load_config` or the
    /// keyring in demo mode.
    #[test]
    fn the_demo_gate_blocks_sync_with_a_friendly_message_and_lets_real_accounts_through() {
        assert_eq!(demo_sync_guard(true), Err(DEMO_SYNC_MESSAGE.to_string()));
        assert!(!DEMO_SYNC_MESSAGE.to_lowercase().contains("credential"),
            "the demo-mode message must read as intentional, not as a leaked technical error");
        assert_eq!(demo_sync_guard(false), Ok(()));
    }

    /// A path in the OS temp dir unique to this test run, so parallel tests
    /// and stale files from an aborted run cannot collide.
    fn scratch_config(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("omacal-test-{}-{name}.toml", std::process::id()))
    }

    #[test]
    fn a_present_config_file_wins_over_the_embedded_pair() {
        let path = scratch_config("file-wins");
        std::fs::write(&path, "client_id = \"file-id\"\nclient_secret = \"file-secret\"\n").unwrap();
        let cfg = load_config_from(&path, Some("embedded-id"), Some("embedded-secret")).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(cfg.client_id, "file-id");
        assert_eq!(cfg.client_secret, "file-secret");
    }

    /// A corrupt file must fail, not fall back: signing the user into the
    /// embedded client because their own config broke would be a silent
    /// account-of-record change, which is worse than an error.
    #[test]
    fn a_malformed_config_file_fails_rather_than_falling_back() {
        let path = scratch_config("malformed");
        std::fs::write(&path, "client_id = ").unwrap();
        // `match` rather than `unwrap_err`, which would demand `Debug` on
        // `Config` — an impl a secret-bearing struct should not grow.
        let err = match load_config_from(&path, Some("embedded-id"), Some("embedded-secret")) {
            Ok(_) => panic!("a malformed file must not produce a Config"),
            Err(e) => e.to_string(),
        };
        std::fs::remove_file(&path).unwrap();
        assert!(
            !err.starts_with("no config at "),
            "a present-but-broken file must not read as absent: {err}"
        );
    }

    #[test]
    fn the_embedded_pair_applies_only_when_the_file_is_absent() {
        let path = scratch_config("absent-embedded");
        let cfg = load_config_from(&path, Some("embedded-id"), Some("embedded-secret")).unwrap();
        assert_eq!(cfg.client_id, "embedded-id");
        assert_eq!(cfg.client_secret, "embedded-secret");
    }

    /// The historical error, verbatim: `errors.rs` safelists the "no config
    /// at " prefix, and the message must keep naming the path it looked for.
    /// An empty env var at build time counts as not baked — `OMACAL_CLIENT_ID=""
    /// cargo tauri build` must not produce a binary that signs in with "".
    #[test]
    fn absent_file_and_no_embedded_pair_names_the_path_it_looked_for() {
        let path = scratch_config("absent-bare");
        for embedded in [None, Some("")] {
            let err = match load_config_from(&path, embedded, embedded) {
                Ok(_) => panic!("nothing to load from, yet a Config appeared"),
                Err(e) => e.to_string(),
            };
            assert!(err.starts_with("no config at "), "unexpected message: {err}");
            assert!(err.contains(path.to_str().unwrap()));
            assert!(err.contains("Create it with client_id and client_secret."));
        }
    }

    #[test]
    fn zoom_can_be_configured_without_google_credentials() {
        let path = scratch_config("zoom-only");
        std::fs::write(&path, "zoom_public_client_id = \"zoom-public-id\"\n").unwrap();
        let id = load_zoom_public_client_id_from(&path, Some("embedded-zoom")).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(id.as_deref(), Some("zoom-public-id"));
    }

    #[test]
    fn an_embedded_zoom_id_is_only_the_missing_file_fallback() {
        let absent = scratch_config("zoom-embedded");
        assert_eq!(
            load_zoom_public_client_id_from(&absent, Some("embedded-zoom")).unwrap().as_deref(),
            Some("embedded-zoom")
        );

        let present = scratch_config("zoom-present-without-key");
        std::fs::write(&present, "client_id = \"google\"\nclient_secret = \"secret\"\n").unwrap();
        let id = load_zoom_public_client_id_from(&present, Some("embedded-zoom")).unwrap();
        std::fs::remove_file(&present).unwrap();
        assert_eq!(id, None, "a present config file must win, including an omitted Zoom key");
    }

    fn cached(expires_at_ms: i64) -> CachedTokens {
        CachedTokens {
            refresh_token: "rt".into(),
            access_token: "at".into(),
            expires_at_ms,
        }
    }

    const NOW: i64 = 1_785_715_200_000;

    #[test]
    fn an_absent_entry_is_never_usable() {
        assert!(!cached_is_usable(None, NOW));
    }

    #[test]
    fn a_live_entry_is_usable_so_the_keychain_is_not_read() {
        assert!(cached_is_usable(Some(&cached(NOW + 60_000)), NOW));
    }

    #[test]
    fn an_expired_entry_is_not_usable() {
        assert!(!cached_is_usable(Some(&cached(NOW - 1)), NOW));
    }

    #[test]
    fn an_entry_expiring_exactly_now_is_not_usable() {
        // Strictly greater: a token expiring this millisecond is not worth a
        // request that will fail.
        assert!(!cached_is_usable(Some(&cached(NOW)), NOW));
    }

    #[test]
    fn the_synced_window_is_180_days_back_and_365_forward() {
        // Both year views render dates outside this, and must say "not fetched"
        // rather than draw them as free. One definition, so the views and the
        // sync loop can never disagree about where the edge is.
        const DAY: i64 = 24 * 3_600_000;
        let now = 1_786_341_600_000; // Mon 10 Aug 2026 09:00 Europe/Sofia
        let (from, to) = synced_window(now);
        assert_eq!(from, now - 180 * DAY);
        assert_eq!(to, now + 365 * DAY);
        assert!(from < now && now < to);
    }

    /// The message is shown for whichever button the user pressed, so it has
    /// to make sense for the sign-in path too — not just for syncing.
    #[test]
    fn the_demo_message_reads_correctly_for_sign_in_as_well_as_sync() {
        let m = DEMO_SYNC_MESSAGE.to_lowercase();
        assert!(m.contains("connect"), "must cover the Connect button: {DEMO_SYNC_MESSAGE}");
        assert!(m.contains("sync"), "must still cover Sync now: {DEMO_SYNC_MESSAGE}");
    }

    /// The behavioural half of X1: `sign_in` in demo mode returns the demo
    /// message and leaves the database untouched. Everything after the guard —
    /// `load_config`, `open::that`, the token exchange, the keyring write, the
    /// `accounts` insert — is unreachable, so this is safe to run anywhere: it
    /// opens no browser and makes no request.
    #[tokio::test]
    async fn sign_in_refuses_in_demo_mode_without_touching_config_keyring_or_google() {
        let pool = omacal_store::connect_memory().await.unwrap();

        assert_eq!(sign_in_impl(&pool, true).await, Err(DEMO_SYNC_MESSAGE.to_string()));

        // Had it run past the guard it would have inserted an account row (or
        // failed with a config/keyring error instead of the demo message).
        let accounts: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(accounts, 0, "demo sign-in wrote to the database");
    }

    async fn seed_account(pool: &SqlitePool, sub: &str, email: &str) -> i64 {
        sqlx::query("INSERT INTO accounts (google_sub, email, created_at) VALUES (?1, ?2, 0)")
            .bind(sub)
            .bind(email)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query_scalar("SELECT id FROM accounts WHERE google_sub = ?1")
            .bind(sub)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    async fn seed_calendar(
        pool: &SqlitePool,
        account_id: i64,
        google_id: &str,
        selected: i64,
        sync_enabled: i64,
    ) {
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role,
                 selected, sync_enabled)
             VALUES (?1, ?2, 'Cal', 'UTC', 'owner', ?3, ?4)",
        )
        .bind(account_id)
        .bind(google_id)
        .bind(selected)
        .bind(sync_enabled)
        .execute(pool)
        .await
        .unwrap();
    }

    /// `assemble_big_year` (Task 3) has no calendar list to draw a real name
    /// from, so it emits a `"Calendar {id}"` placeholder — `get_big_year_impl`
    /// is where that gets resolved against `list_calendars`, and this is the
    /// test that would fail if that join were ever dropped, leaving the
    /// placeholder to reach the UI.
    #[tokio::test]
    async fn the_legend_carries_a_real_calendar_name_not_a_placeholder() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let account_id = seed_account(&pool, "sub", "e@x").await;
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role,
                 selected, sync_enabled)
             VALUES (?1, 'cal-1', 'Excitel Team', 'UTC', 'owner', 1, 1)",
        )
        .bind(account_id)
        .execute(&pool)
        .await
        .unwrap();
        let calendar_id: i64 = sqlx::query_scalar("SELECT id FROM calendars WHERE google_id = 'cal-1'")
            .fetch_one(&pool)
            .await
            .unwrap();

        // An all-day span, 1-2 Jan 2026 UTC, so it lands squarely inside the
        // ribbon and is placed as a pill rather than dropped or overflowed.
        let leave = omacal_store::StoredEvent {
            id: 0, calendar_id, google_id: "leave".into(), summary: Some("Leave".into()),
            location: None, start_utc: 1_767_225_600_000, end_utc: 1_767_312_000_000,
            start_tz: "UTC".into(), end_tz: "UTC".into(),
            is_all_day: true, recurrence: None,
            recurring_event_id: None, original_start_utc: None,
            status: "confirmed".into(), self_response: Some("accepted".into()),
            conference_uri: None, color_hex: None, calendar_timezone: "UTC".into(),
            description: None, etag: None,
            sequence: 0, organizer_email: None, attendees: Vec::new(),
            reminders: Default::default(), calendar_default_reminders: Vec::new(),
        };
        omacal_store::upsert_event(&pool, &leave).await.unwrap();

        let payload =
            get_big_year_impl(&pool, 2026, "UTC", 1_786_341_600_000, settings::WeekStart::Monday)
                .await
                .unwrap();
        assert!(
            payload.legend.iter().any(|e| e.name == "Excitel Team"),
            "legend must carry the real calendar name, not a placeholder: {:?}",
            payload.legend
        );
    }

    /// The safety property the whole 1c plan rests on: a calendar hidden from
    /// the UI (`selected = 0`) must still be fetched, or re-showing it later
    /// would reveal a gap until the next full sync.
    #[tokio::test]
    async fn a_hidden_calendar_is_still_fetched() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let account_id = seed_account(&pool, "sub", "e@x").await;
        seed_calendar(&pool, account_id, "hidden-but-synced", 0, 1).await;

        let cals = calendars_to_sync(&pool, account_id).await.unwrap();
        assert_eq!(cals.len(), 1, "a hidden calendar must still be synced");
        assert_eq!(cals[0].1, "hidden-but-synced");
    }

    /// The other half of the split: a calendar the user removed from sync
    /// (`sync_enabled = 0`) must not be fetched, regardless of `selected`.
    #[tokio::test]
    async fn a_removed_calendar_is_not_fetched() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let account_id = seed_account(&pool, "sub", "e@x").await;
        seed_calendar(&pool, account_id, "shown-but-not-synced", 1, 0).await;

        let cals = calendars_to_sync(&pool, account_id).await.unwrap();
        assert!(cals.is_empty(), "sync_enabled = 0 must exclude the calendar even if selected");
    }

    #[tokio::test]
    async fn only_the_requested_account_is_returned() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let a1 = seed_account(&pool, "sub-1", "one@x").await;
        let a2 = seed_account(&pool, "sub-2", "two@x").await;
        seed_calendar(&pool, a1, "cal-1", 1, 1).await;
        seed_calendar(&pool, a2, "cal-2", 1, 1).await;

        let cals = calendars_to_sync(&pool, a1).await.unwrap();
        assert_eq!(cals.len(), 1);
        assert_eq!(cals[0].1, "cal-1", "must not bleed in the other account's calendar");
    }

    fn one_event_body() -> serde_json::Value {
        serde_json::json!({
            "items": [{
                "id": "e1", "status": "confirmed", "summary": "Standup",
                "start": {"dateTime": "2026-08-03T09:00:00Z"},
                "end":   {"dateTime": "2026-08-03T09:30:00Z"}
            }],
            "nextSyncToken": "tok-1"
        })
    }

    async fn accounts_in_order(pool: &SqlitePool) -> Vec<(i64, String)> {
        sqlx::query_as("SELECT id, email FROM accounts ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    /// Revoke the first account's access and every *other* account stopped
    /// syncing too: `access_token_for` returned `invalid_grant`, the `?` took
    /// all of `sync_all` with it, and `accounts` had no `ORDER BY`, so in rowid
    /// order the account that blocked everyone was the first one ever added.
    #[tokio::test]
    async fn an_account_that_cannot_get_a_token_does_not_stop_the_others() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(one_event_body()))
            .mount(&server)
            .await;

        let pool = omacal_store::connect_memory().await.unwrap();
        let revoked = seed_account(&pool, "sub-revoked", "revoked@x").await;
        let healthy = seed_account(&pool, "sub-healthy", "healthy@x").await;
        seed_calendar(&pool, revoked, "cal-revoked", 1, 1).await;
        seed_calendar(&pool, healthy, "cal-healthy", 1, 1).await;

        let accounts = accounts_in_order(&pool).await;
        assert_eq!(accounts[0].1, "revoked@x", "the broken account must go first");

        let (total, failed, _) = sync_accounts(
            &pool, &accounts, &server.uri(), 0, 9_999_999_999_999,
            |email| async move {
                if email == "revoked@x" {
                    anyhow::bail!("invalid_grant: token has been expired or revoked");
                }
                Ok("at-healthy".to_string())
            },
        )
        .await;

        assert_eq!(failed, vec!["revoked@x".to_string()]);
        assert_eq!(total, 1, "the healthy account still synced");

        let synced: Vec<String> = sqlx::query_scalar(
            "SELECT c.google_id FROM events e JOIN calendars c ON c.id = e.calendar_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(synced, vec!["cal-healthy".to_string()],
                   "the healthy account's calendar must have been fetched and stored");
    }

    /// The dead-token path: a rejection `needs_reauth` classifies must land in
    /// the third list, not among the transient failures — `failed` is what
    /// makes the sync report "it will keep trying", and for a dead grant that
    /// sentence is false. The healthy account still syncs, same isolation as
    /// above.
    #[tokio::test]
    async fn a_dead_token_is_reported_for_reconnection_not_as_a_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(one_event_body()))
            .mount(&server)
            .await;

        let pool = omacal_store::connect_memory().await.unwrap();
        seed_account(&pool, "sub-dead", "dead@x").await;
        let healthy = seed_account(&pool, "sub-healthy", "healthy@x").await;
        seed_calendar(&pool, healthy, "cal-healthy", 1, 1).await;

        let accounts = accounts_in_order(&pool).await;
        let (total, failed, dead) = sync_accounts(
            &pool, &accounts, &server.uri(), 0, 9_999_999_999_999,
            |email| async move {
                if email == "dead@x" {
                    return Err(omacal_google::auth::TokenRejected::new(
                        "invalid_grant", "Token has been expired or revoked.",
                    )
                    .into());
                }
                Ok("at-healthy".to_string())
            },
        )
        .await;

        assert_eq!(dead, vec!["dead@x".to_string()]);
        assert!(failed.is_empty(),
                "a dead grant is not a transient failure, and counting it as one \
                 re-toasts a message whose promise (\"it will keep trying\") is false: {failed:?}");
        assert_eq!(total, 1, "the healthy account still synced");
        assert!(sync_result(total, &failed).is_ok(),
                "the reconnect prompt is this state's surface; the generic failure is not");
    }

    /// The other half of going quiet: once marked, the account is not offered
    /// to the sync at all. Without the filter every tick would re-spend a
    /// token-endpoint request on an answer that cannot change.
    #[test]
    fn a_marked_account_is_left_out_of_the_attempt_list() {
        let accounts = vec![(1, "dead@x".to_string()), (2, "healthy@x".to_string())];
        let marked = std::collections::BTreeSet::from(["dead@x".to_string()]);

        let attempted = accounts_to_attempt(accounts.clone(), &marked);
        assert_eq!(attempted, vec![(2, "healthy@x".to_string())]);

        let nobody_marked = accounts_to_attempt(accounts.clone(), &Default::default());
        assert_eq!(nobody_marked, accounts, "an empty mark set must change nothing");
    }

    /// What a re-sign-in must reset, and the trap it exists for:
    /// `access_token_for` prefers the cache's refresh token over the
    /// keyring's, so a re-sign-in that leaves the cache alone keeps presenting
    /// the dead pair the user just replaced — fresh consent, no fix. The other
    /// account's entries stay: reconnecting one account is not a licence to
    /// forget another's.
    #[tokio::test]
    async fn a_sign_in_forgets_the_accounts_stale_credentials_and_mark() {
        let mut tokens = std::collections::HashMap::from([
            ("dead@x".to_string(), CachedTokens {
                refresh_token: "rt-dead".into(), access_token: "at-dead".into(),
                expires_at_ms: 0,
            }),
            ("other@x".to_string(), CachedTokens {
                refresh_token: "rt-other".into(), access_token: "at-other".into(),
                expires_at_ms: i64::MAX,
            }),
        ]);
        let mut reauth = std::collections::BTreeSet::from([
            "dead@x".to_string(), "other@x".to_string(),
        ]);

        forget_stale_credentials(&mut tokens, &mut reauth, "dead@x");

        assert!(!tokens.contains_key("dead@x"), "the dead pair would be presented again");
        assert!(!reauth.contains("dead@x"), "the account would never rejoin the sync rotation");
        assert!(tokens.contains_key("other@x"), "another account's cache was collateral");
        assert!(reauth.contains("other@x"), "another account's mark was collateral");
    }

    /// The same isolation one level down. A shared calendar unshared behind our
    /// back answers 403 forever, and it stays `sync_enabled = 1`, so it retries
    /// and re-fails on every tick — it must not take the account's other
    /// calendars with it.
    #[tokio::test]
    async fn one_calendar_returning_403_does_not_stop_the_others() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/calendars/cal-unshared/events"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/calendars/cal-ok/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(one_event_body()))
            .mount(&server)
            .await;

        let pool = omacal_store::connect_memory().await.unwrap();
        let account = seed_account(&pool, "sub", "me@x").await;
        seed_calendar(&pool, account, "cal-unshared", 1, 1).await;
        seed_calendar(&pool, account, "cal-ok", 1, 1).await;

        // Seeded first, and `calendars_to_sync` orders by id, so the failing
        // calendar is genuinely fetched before the healthy one. Without that
        // the healthy one might be fetched first and the assertions below
        // would hold even if the 403 aborted everything after it.
        assert_eq!(calendars_to_sync(&pool, account).await.unwrap()[0].1, "cal-unshared");

        let accounts = accounts_in_order(&pool).await;
        let (total, failed, _) = sync_accounts(
            &pool, &accounts, &server.uri(), 0, 9_999_999_999_999,
            |_| async { Ok("at".to_string()) },
        )
        .await;

        assert_eq!(failed, vec!["me@x / cal-unshared".to_string()]);
        assert_eq!(total, 1, "the account's other calendar still synced");

        let synced: Vec<String> = sqlx::query_scalar(
            "SELECT c.google_id FROM events e JOIN calendars c ON c.id = e.calendar_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(synced, vec!["cal-ok".to_string()]);
    }

    /// A sync where nothing went wrong must still say so, or the isolation
    /// above would be indistinguishable from swallowing every failure.
    #[tokio::test]
    async fn a_sync_with_no_failures_reports_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(one_event_body()))
            .mount(&server)
            .await;

        let pool = omacal_store::connect_memory().await.unwrap();
        let account = seed_account(&pool, "sub", "me@x").await;
        seed_calendar(&pool, account, "cal-1", 1, 1).await;

        let accounts = accounts_in_order(&pool).await;
        let (total, failed, _) = sync_accounts(
            &pool, &accounts, &server.uri(), 0, 9_999_999_999_999,
            |_| async { Ok("at".to_string()) },
        )
        .await;

        assert!(failed.is_empty(), "a clean sync reported a failure: {failed:?}");
        assert_eq!(total, 1);
    }

    #[test]
    fn a_clean_sync_reports_what_it_wrote() {
        assert_eq!(sync_result(7, &[]).unwrap(), 7);
        assert_eq!(sync_result(0, &[]).unwrap(), 0, "nothing to do is not a failure");
    }

    /// The one `if` that makes isolation reportable rather than silent.
    ///
    /// Without it, a sync in which *every* account failed returns `Ok`:
    /// `record_sync` runs, the header reads "Synced just now", and no
    /// `sync-failed` event ever fires. That is the failure mode isolating the
    /// errors exists to avoid creating — the `?` it replaced at least stopped
    /// loudly.
    #[test]
    fn a_sync_where_everything_failed_is_an_error() {
        assert!(sync_result(0, &["revoked@x".to_string()]).is_err());
    }

    /// The half that a "did we write anything?" check would get wrong. Rows
    /// were written, and it is still a failure: the account that could not sync
    /// is stale, and nothing about the accounts that did sync makes it less so.
    #[test]
    fn a_partial_sync_is_still_reported_as_a_failure() {
        assert!(sync_result(42, &["revoked@x".to_string()]).is_err());
    }

    /// The failure crosses `errors::user_facing` on its way to the app header,
    /// and the labels it is built from are account email addresses.
    #[test]
    fn the_reported_failure_carries_a_count_and_no_account() {
        let e = sync_result(0, &[
            "someone@example.com".to_string(),
            "someone@example.com / a-calendar".to_string(),
        ])
        .unwrap_err();
        let text = e.to_string();

        assert!(!text.contains("someone@example.com"),
                "an account email reached a user-facing string: {text}");
        assert!(!text.contains("a-calendar"), "a calendar id reached a user-facing string: {text}");
        assert!(text.contains('2'), "the count is the whole payload: {text}");

        // And it is not accidentally on the allowlist, so even this much is
        // withheld from the header.
        let shown = crate::errors::user_facing(&e);
        assert_ne!(shown, text, "the failure must not be shown verbatim");
        assert!(!shown.contains("someone@example.com"), "{shown}");
    }

    fn google_calendar(id: &str) -> omacal_google::model::Calendar {
        omacal_google::model::Calendar {
            id: id.to_string(),
            summary: "Work".into(),
            background_color: Some("#5b8def".into()),
            time_zone: Some("Europe/Sofia".into()),
            access_role: "owner".into(),
            primary: true,
            default_reminders: Vec::new(),
        }
    }

    /// What makes a removal survive the next sign-in: the calendar upsert's
    /// `ON CONFLICT` updates Google's fields and leaves `selected` and
    /// `sync_enabled` alone. "Add account" is a permanent button and
    /// `prompt=select_account` puts re-picking an already-connected account one
    /// click away, so this path is ordinary, not an edge case — and adding the
    /// two columns to that clause reverts every removal and hide with nothing
    /// else in the codebase noticing.
    #[tokio::test]
    async fn re_signing_in_preserves_removals_and_hides() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let account_id = seed_account(&pool, "sub", "me@x").await;

        upsert_calendar(&pool, account_id, &google_calendar("removed")).await.unwrap();
        upsert_calendar(&pool, account_id, &google_calendar("hidden")).await.unwrap();

        sqlx::query("UPDATE calendars SET sync_enabled = 0 WHERE google_id = 'removed'")
            .execute(&pool).await.unwrap();
        sqlx::query("UPDATE calendars SET selected = 0 WHERE google_id = 'hidden'")
            .execute(&pool).await.unwrap();

        // The user picks the same account again. Google lists the same
        // calendars, with a renamed one to prove the statement still updates
        // what it is supposed to update.
        let mut renamed = google_calendar("removed");
        renamed.summary = "Work (renamed)".into();
        upsert_calendar(&pool, account_id, &renamed).await.unwrap();
        upsert_calendar(&pool, account_id, &google_calendar("hidden")).await.unwrap();

        let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
            "SELECT google_id, summary, selected, sync_enabled FROM calendars ORDER BY google_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();

        assert_eq!(rows.len(), 2, "re-signing in must not duplicate calendars");

        let hidden = rows.iter().find(|r| r.0 == "hidden").unwrap();
        assert_eq!(hidden.2, 0, "re-signing in un-hid a hidden calendar");
        assert_eq!(hidden.3, 1);

        let removed = rows.iter().find(|r| r.0 == "removed").unwrap();
        assert_eq!(removed.3, 0, "re-signing in re-added a removed calendar");
        assert_eq!(removed.1, "Work (renamed)",
                   "Google's own fields must still be refreshed");
    }

    /// `defaultReminders` arrives on every `calendarList` entry and this is the
    /// only writer of the column that holds it. It is what an event saying
    /// `useDefault: true` resolves against, so a calendar that loses it makes
    /// every such event fire nothing at all.
    ///
    /// The second half is the refresh: unlike `selected` and `sync_enabled`,
    /// this value is Google's, not the user's, so re-signing in must *update*
    /// it rather than preserve what was there. A calendar whose defaults were
    /// changed in Google's own settings is otherwise stuck on the old list.
    #[tokio::test]
    async fn a_calendars_default_reminders_are_stored_and_refreshed() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let account_id = seed_account(&pool, "sub", "me@x").await;

        let mut cal = google_calendar("primary");
        cal.default_reminders = vec![
            omacal_google::model::Reminder { method: "popup".into(), minutes: 10 },
            omacal_google::model::Reminder { method: "email".into(), minutes: 1440 },
        ];
        upsert_calendar(&pool, account_id, &cal).await.unwrap();

        let stored: Option<String> = sqlx::query_scalar(
            "SELECT default_reminders_json FROM calendars WHERE google_id = 'primary'")
            .fetch_one(&pool).await.unwrap();
        let parsed: Vec<omacal_store::Reminder> =
            serde_json::from_str(&stored.expect("the column must be written, not left NULL"))
                .expect("stored as the shape StoredEvent reads back");
        assert_eq!(parsed, vec![
            omacal_store::Reminder { method: "popup".into(), minutes: 10 },
            omacal_store::Reminder { method: "email".into(), minutes: 1440 },
        ]);

        // Google's settings change; the next sign-in must carry that through.
        cal.default_reminders =
            vec![omacal_google::model::Reminder { method: "popup".into(), minutes: 5 }];
        upsert_calendar(&pool, account_id, &cal).await.unwrap();

        let stored: Option<String> = sqlx::query_scalar(
            "SELECT default_reminders_json FROM calendars WHERE google_id = 'primary'")
            .fetch_one(&pool).await.unwrap();
        let parsed: Vec<omacal_store::Reminder> =
            serde_json::from_str(&stored.unwrap()).unwrap();
        assert_eq!(
            parsed,
            vec![omacal_store::Reminder { method: "popup".into(), minutes: 5 }],
            "re-signing in must refresh Google's own value, not preserve the old one"
        );
    }
}
