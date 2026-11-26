//! Account listing and sign-out — the "not built yet" that finally is.
//!
//! Signing out is three removals in strictness order: the provider's server
//! side first where one exists (Google's token revocation — best-effort,
//! because a token we can no longer revoke must not hold the account
//! hostage), then the keyring entry, then the local rows through
//! `omacal_store::delete_account`'s tested cascade. Local data goes because
//! it is a *cache* of the provider's truth: signing back in re-syncs all of
//! it, and keeping another account's events on disk after the user said
//! "sign out" would betray what the button says.
//!
//! For CalDAV there is no server side to call: the app-specific password (or
//! server password) simply stops being used, and revoking it for real is the
//! user's move at their provider (the UI says so).

use serde::Serialize;

use crate::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountVm {
    pub id: i64,
    pub email: String,
    pub provider: String,
    /// Where a CalDAV account's server is, and the name it logs in with when
    /// that differs from the email: neither is secret, and the Reconnect form
    /// fills them in so only the password has to be typed again. `None` for
    /// Google and feeds.
    pub server_url: Option<String>,
    pub username: Option<String>,
    /// The server stopped accepting this account's password (the reconnect
    /// banner's account): its row offers Reconnect first.
    pub needs_reconnect: bool,
}

const GOOGLE_REVOKE: &str = "https://oauth2.googleapis.com/revoke";

#[tauri::command]
pub async fn list_accounts(state: tauri::State<'_, AppState>) -> Result<Vec<AccountVm>, String> {
    accounts_of(&state).await.map_err(|e| crate::errors::user_facing(&e))
}

/// `id, email, provider, server_url, username`, as `accounts_of` selects them.
type AccountRow = (i64, String, String, Option<String>, Option<String>);

async fn accounts_of(state: &AppState) -> anyhow::Result<Vec<AccountVm>> {
    let rows: Vec<AccountRow> = sqlx::query_as(
        "SELECT id, email, provider, server_url, username FROM accounts ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await?;
    let marked = state.reauth.lock().expect("reauth mark poisoned").clone();
    Ok(rows
        .into_iter()
        .map(|(id, email, provider, server_url, username)| {
            let caldav = provider == "caldav";
            AccountVm {
                needs_reconnect: caldav && marked.contains(&reauth_mark(&provider, &email)),
                server_url: server_url.filter(|_| caldav),
                username: username.filter(|_| caldav),
                id,
                email,
                provider,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn sign_out(
    state: tauri::State<'_, AppState>,
    account_id: i64,
) -> Result<Vec<AccountVm>, String> {
    crate::demo_sync_guard(state.demo)?;
    sign_out_impl(&state, account_id).await.map_err(|e| e.to_string())?;
    accounts_of(&state).await.map_err(|e| crate::errors::user_facing(&e))
}

/// The dead-credential mark a sync sets for this account in `AppState.reauth`:
/// a Google account's is its address, a CalDAV account's is
/// `caldav_account::reauth_key` (`caldav:` and the address). Signing out
/// cleared the bare address for both, so a CalDAV account signed out while its
/// password was rejected left the reconnect banner up for the rest of the
/// session, about an account that no longer existed (found in the 2026-11-22
/// triage of #191).
fn reauth_mark(provider: &str, email: &str) -> String {
    match provider {
        "caldav" => crate::caldav_account::reauth_key(email),
        _ => email.to_string(),
    }
}

async fn sign_out_impl(state: &AppState, account_id: i64) -> anyhow::Result<()> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT email, provider FROM accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((email, provider)) = row else {
        return Ok(()); // Already gone — which is what signing out wanted.
    };

    // The keyring key this account's secret lives under. Subscribed feeds
    // hold no secret at all — there is nothing to delete.
    let keyring_key = match provider.as_str() {
        "caldav" => Some(format!("caldav:{email}")),
        "google" => Some(email.clone()),
        _ => None,
    };

    if provider == "google" {
        // Best-effort server-side revocation, before the token is deleted
        // locally — afterwards there would be nothing left to revoke with.
        match crate::load_refresh_token(&email) {
            Ok(token) => {
                if let Err(e) = omacal_google::auth::revoke(GOOGLE_REVOKE, &token).await {
                    tracing::warn!(account = %email, %e,
                        "could not revoke the token; sign-out proceeds — \
                         the grant can be removed at myaccount.google.com/permissions");
                }
            }
            Err(e) => {
                tracing::warn!(account = %email, %e, "no refresh token to revoke");
            }
        }
    }

    if let Some(keyring_key) = keyring_key {
        if let Ok(entry) = keyring::Entry::new(crate::KEYRING_SERVICE, &keyring_key) {
            if let Err(e) = entry.delete_credential() {
                // A missing entry is fine — the goal is its absence.
                tracing::debug!(account = %email, %e, "keyring entry not deleted");
            }
        }
    }

    omacal_store::delete_account(&state.pool, account_id).await?;

    // The account's cached access token and its dead-credential mark both
    // describe an account that no longer exists.
    state.tokens.lock().await.remove(&email);
    state.reauth.lock().expect("reauth mark poisoned").remove(&reauth_mark(&provider, &email));

    crate::upcoming::refresh_soon(state.pool.clone(), state.demo);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mark signing out clears is the one the sync set (`lib.rs`'s
    /// `sync_accounts`): `caldav_account::reauth_key` for a rejected CalDAV
    /// password, and the bare address for a Google account whose grant died.
    /// A CalDAV row carries what the Reconnect form fills in, its server and
    /// login name, and says when the server stopped taking its password. A
    /// Google account and a feed carry neither, even with a mark under the
    /// same address: a Google mark is the Google banner's business.
    #[tokio::test]
    async fn a_caldav_row_carries_its_server_and_whether_it_needs_reconnecting() {
        let pool = omacal_store::connect_memory().await.unwrap();
        for (sub, email, provider, url, user) in [
            ("g1", "me@gmail.com", "google", None, None),
            ("caldav:me@icloud.com", "me@icloud.com", "caldav", Some(omacal_caldav::ICLOUD_BASE), None),
            ("caldav:ana", "ana", "caldav", Some("https://dav.example.com/"), Some("ana-login")),
            ("webcal:x", "https://feeds.example.com/h.ics", "webcal", Some("https://feeds.example.com/h.ics"), None),
        ] {
            sqlx::query("INSERT INTO accounts (google_sub, email, created_at, provider, server_url, username)
                         VALUES (?1, ?2, 0, ?3, ?4, ?5)")
                .bind(sub).bind(email).bind(provider).bind(url).bind(user)
                .execute(&pool).await.unwrap();
        }
        let state = AppState {
            pool, demo: false, tokens: Default::default(), reauth: Default::default(),
            update: Default::default(), update_checked_at: Default::default(),
            system_tz_change: Default::default(), quit_on_close: Default::default(),
            open_date: Default::default(),
            history_months: tokio::sync::Mutex::new(crate::settings::DEFAULT_HISTORY_MONTHS),
        };
        {
            let mut marked = state.reauth.lock().unwrap();
            marked.insert(crate::caldav_account::reauth_key("me@icloud.com"));
            marked.insert("me@gmail.com".to_string());
        }
        let rows = accounts_of(&state).await.unwrap();
        let by = |email: &str| rows.iter().find(|r| r.email == email).unwrap();

        let icloud = by("me@icloud.com");
        assert!(icloud.needs_reconnect, "its password was rejected");
        assert_eq!(icloud.server_url.as_deref(), Some(omacal_caldav::ICLOUD_BASE));

        let ana = by("ana");
        assert!(!ana.needs_reconnect, "nothing marked for it");
        assert_eq!(ana.server_url.as_deref(), Some("https://dav.example.com/"));
        assert_eq!(ana.username.as_deref(), Some("ana-login"));

        let google = by("me@gmail.com");
        assert!(!google.needs_reconnect, "the Google banner handles a Google mark");
        assert_eq!((google.server_url.as_deref(), google.username.as_deref()), (None, None));
        let feed = by("https://feeds.example.com/h.ics");
        assert_eq!((feed.server_url.as_deref(), feed.needs_reconnect), (None, false));
    }

    #[test]
    fn signing_out_clears_the_mark_the_sync_set() {
        let email = "user@icloud.com";
        assert_eq!(reauth_mark("caldav", email), crate::caldav_account::reauth_key(email));
        assert_ne!(reauth_mark("caldav", email), email, "a CalDAV mark is not the bare address");
        assert_eq!(reauth_mark("google", "me@gmail.com"), "me@gmail.com");
    }
}
