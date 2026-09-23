//! Read-only WebCal subscriptions: one public feed URL syncing into one `reader`
//! calendar.
//!
//! The CalDAV connect flow's shape, minus everything a feed does not have:
//! no credentials (nothing in the keyring, nothing to revoke), no discovery
//! walk (a GET that parses is the whole validation), one account row per URL
//! holding one calendar. What is shared is honest: the same `reader` role
//! every write path already refuses, the same `sync_enabled` unsubscribe
//! semantics, the same `https_or_private` refusal for cleartext off-LAN.

use sqlx::SqlitePool;

use crate::AppState;

/// Subscribes to a public WebCal feed: normalizes + validates the URL, proves it
/// parses with a first fetch, then stores the account/calendar rows and syncs.
///
/// `name` is the display name; blank falls back to the feed's host. Returns
/// the calendar id the feed syncs into.
#[tauri::command]
pub async fn subscribe_webcal(
    state: tauri::State<'_, AppState>,
    url: String,
    name: Option<String>,
) -> Result<i64, String> {
    crate::demo_sync_guard(state.demo)?;
    let feed_url = omacal_sync::webcal_feed::normalize_feed_url(&url)
        .map_err(|e| crate::errors::user_facing(&e))?;
    let name = webcal_subscription_fallback_name(&feed_url, name.as_deref());

    // The fetch is the credential check: a URL that does not answer with a
    // parseable calendar dies here, before anything at all is written.
    let fetched = omacal_sync::webcal_feed::fetch_feed(&feed_url, None)
        .await
        .map_err(user_facing_feed)?
        .ok_or_else(|| "The server answered with no calendar".to_string())?;
    if omacal_caldav::parse(&fetched.body).is_none() {
        return Err("The URL did not return a calendar".to_string());
    }

    let fallback_tz = jiff::tz::TimeZone::system()
        .iana_name()
        .unwrap_or("UTC")
        .to_string();
    let (_account_id, calendar_id) = omacal_store::ensure_webcal_subscription(
        &state.pool,
        &feed_url,
        &name,
        &fallback_tz,
        crate::now_ms(),
    )
    .await
    .map_err(|e| crate::errors::user_facing(&e))?;

    // Immediate first sync so the calendar is not empty until the ticker.
    // A failure here is not fatal: the rows exist and the loop will retry.
    if let Err(e) = omacal_sync::webcal_feed::sync_webcal_calendar(&state.pool, calendar_id, &feed_url)
        .await
    {
        tracing::warn!(%e, calendar_id, "subscribed feed did not sync on its first try");
    }
    crate::upcoming::refresh_soon(state.pool.clone(), state.demo);
    Ok(calendar_id)
}

fn user_facing_feed(e: omacal_sync::webcal_feed::WebcalFeedError) -> String {
    match e {
        omacal_sync::webcal_feed::WebcalFeedError::Http(status) => crate::errors::server_answered(status),
        omacal_sync::webcal_feed::WebcalFeedError::Other(e) => crate::errors::user_facing(&e),
    }
}

/// Syncs every enabled calendar of one subscribed-feeds account. One bad feed
/// never stops the others — the same per-calendar isolation Google gets.
///
/// Takes no window: a feed arrives whole, so there is no slice to ask for.
/// No `dead`/reauth concept: feeds hold no credentials, so every failure is
/// a plain `failed` label, never a reconnect banner.
pub(crate) async fn sync_account(pool: &SqlitePool, account_id: i64) -> (u64, Vec<String>) {
    let account_name: String = sqlx::query_scalar("SELECT email FROM accounts WHERE id = ?1")
        .bind(account_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten()
        .unwrap_or_else(|| format!("webcal:{account_id}"));
    let cals: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, google_id FROM calendars WHERE account_id = ?1 AND sync_enabled = 1 ORDER BY id",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let mut total = 0u64;
    let mut failed = Vec::new();
    for (cal_id, feed_url) in cals {
        match omacal_sync::webcal_feed::sync_webcal_calendar(pool, cal_id, &feed_url)
            .await
        {
            Ok(out) => total += (out.upserted + out.deleted) as u64,
            Err(e) => {
                // **The host and the row id, never the address.** For a
                // subscription the URL is frequently the credential itself —
                // a private calendar's "secret address in iCal format" grants
                // read access to anyone holding it — and the failure message
                // the user reads says "see the application log", which is how
                // that address would end up pasted into a bug report. Google
                // logs a calendar id and CalDAV an email; neither is a secret.
                // The host still says which subscription it was. `%e` is safe:
                // `fetch_feed` strips the URL out of every transport error.
                let host = omacal_sync::webcal_feed::default_name_for(&feed_url);
                tracing::warn!(account = %account_name, calendar_id = cal_id, %host, %e,
                               "subscribed feed sync failed");
                // Counted, not shown, today (`sync_result` reads `.len()`) — but
                // a label that is the credential would become a leak the day
                // these are displayed, so it carries the host as well.
                failed.push(format!("{account_name} / {host}"));
            }
        }
    }
    (total, failed)
}

/// Test seam for the name fallback (kept free of Tauri state).
pub(crate) fn webcal_subscription_fallback_name(feed_url: &str, name: Option<&str>) -> String {
    name.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| omacal_sync::webcal_feed::default_name_for(feed_url))
}

#[cfg(test)]
mod tests {
    use super::webcal_subscription_fallback_name;

    #[test]
    fn a_blank_name_falls_back_to_the_host() {
        assert_eq!(
            webcal_subscription_fallback_name("https://cal.example.com/x.ics", Some("  ")),
            "cal.example.com"
        );
        assert_eq!(
            webcal_subscription_fallback_name("https://cal.example.com/x.ics", Some("Team")),
            "Team"
        );
    }
}

