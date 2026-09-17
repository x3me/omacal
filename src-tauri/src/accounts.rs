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
}

const GOOGLE_REVOKE: &str = "https://oauth2.googleapis.com/revoke";

#[tauri::command]
pub async fn list_accounts(state: tauri::State<'_, AppState>) -> Result<Vec<AccountVm>, String> {
    accounts_of(&state).await.map_err(|e| crate::errors::user_facing(&e))
}

async fn accounts_of(state: &AppState) -> anyhow::Result<Vec<AccountVm>> {
    let rows: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id, email, provider FROM accounts ORDER BY id")
            .fetch_all(&state.pool)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(id, email, provider)| AccountVm { id, email, provider })
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
    state.reauth.lock().expect("reauth mark poisoned").remove(&email);

    crate::upcoming::refresh_soon(state.pool.clone(), state.demo);
    Ok(())
}
