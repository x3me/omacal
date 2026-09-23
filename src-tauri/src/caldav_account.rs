//! Connecting and syncing CalDAV accounts — iCloud and any standards server.
//!
//! The Google account flow's shape, deliberately: a connect command that
//! validates credentials before anything is stored, the secret in the
//! keyring, calendars upserted preserving the user's `selected` /
//! `sync_enabled` choices, and a dead credential marking the account for the
//! reconnect banner rather than failing silently forever.
//!
//! What is different is honest, not hidden: no OAuth (an app-specific
//! password over Basic), no tombstones (`omacal_sync::caldav` infers
//! deletions), and no guest *management* — inviting and uninviting is iTIP,
//! its own project. Answering an invitation does work: sync marks the
//! attendee matching the account's email as `self`
//! (`omacal_sync::caldav_to_stored`), which is what lets `can_respond` show
//! the RSVP strip, and `caldav_write::respond` writes the `PARTSTAT` back.
//! A resource that exposes no matching attendee stays unanswerable — the
//! capability gate falling out of data the server actually holds.

use omacal_caldav::{CalDavClient, CalDavError};
use sqlx::SqlitePool;

use crate::AppState;

/// The keyring key for a CalDAV secret. Prefixed so the same address used
/// as both a Google account and an Apple ID cannot collide on one entry.
fn keyring_key(email: &str) -> String {
    format!("caldav:{email}")
}

fn store_password(email: &str, password: &str) -> anyhow::Result<()> {
    keyring::Entry::new(crate::KEYRING_SERVICE, &keyring_key(email))?.set_password(password)?;
    Ok(())
}

fn password_for(email: &str) -> anyhow::Result<String> {
    Ok(keyring::Entry::new(crate::KEYRING_SERVICE, &keyring_key(email))?.get_password()?)
}

/// What `connect_caldav` needs to know about the kind of server it is
/// talking to. "icloud" is a UI nicety, not a protocol difference: it fixes
/// the URL and words the form ("Apple ID", "app-specific password").
fn base_url_for(kind: &str, server_url: Option<&str>) -> Result<String, String> {
    match kind {
        "icloud" => Ok(omacal_caldav::ICLOUD_BASE.to_string()),
        _ => server_url
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "A server address is required".to_string()),
    }
}

/// Connects (or reconnects) a CalDAV account: validates the credentials by
/// running discovery, then stores the password, the account row, and the
/// calendar list. Returns the account's display email.
#[tauri::command]
pub async fn connect_caldav(
    state: tauri::State<'_, AppState>,
    kind: String,
    server_url: Option<String>,
    email: String,
    username: Option<String>,
    password: String,
) -> Result<String, String> {
    crate::demo_sync_guard(state.demo)?;
    let email = email.trim().to_string();
    if email.is_empty() {
        return Err("An email (or account name) is required".to_string());
    }
    if password.trim().is_empty() {
        return Err("A password is required".to_string());
    }
    let base = base_url_for(&kind, server_url.as_deref())?;
    let login = username
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&email)
        .to_string();

    let client = CalDavClient::new(&base, &login, &password)
        .map_err(|e| crate::errors::user_facing(&e))?;
    // Discovery is the credential check: a wrong password dies here, before
    // anything at all is written.
    let calendars = client.discover().await.map_err(user_facing_caldav)?;
    if calendars.is_empty() {
        return Err("The server answered, but listed no calendars for this account".to_string());
    }

    store_password(&email, &password).map_err(|e| crate::errors::user_facing(&e))?;

    let pool = &state.pool;
    let account_id: i64 = sqlx::query_scalar(
        "INSERT INTO accounts (google_sub, email, created_at, provider, server_url, username)
         VALUES (?1, ?2, ?3, 'caldav', ?4, ?5)
         ON CONFLICT (google_sub) DO UPDATE SET
            email = excluded.email, server_url = excluded.server_url,
            username = excluded.username
         RETURNING id",
    )
    .bind(keyring_key(&email))
    .bind(&email)
    .bind(crate::now_ms())
    .bind(&base)
    .bind(&login)
    .fetch_one(pool)
    .await
    .map_err(|e| crate::errors::user_facing(&anyhow::Error::from(e)))?;

    // The machine's zone is the fallback for collections that publish none —
    // an all-day date has to resolve against something, and the zone the
    // user is sitting in is the only defensible guess.
    let fallback_tz = jiff::tz::TimeZone::system()
        .iana_name()
        .unwrap_or("UTC")
        .to_string();

    for cal in &calendars {
        // Like the Google upsert: the provider's fields update, the user's
        // (`selected`, `sync_enabled`) are never touched on reconnect.
        sqlx::query(
            "INSERT INTO calendars
                (account_id, google_id, summary, color_hex, timezone, access_role,
                 is_primary, supports_events, supports_tasks)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8)
             ON CONFLICT (account_id, google_id) DO UPDATE SET
                summary = excluded.summary,
                color_hex = excluded.color_hex,
                timezone = excluded.timezone,
                access_role = excluded.access_role,
                supports_events = excluded.supports_events,
                supports_tasks = excluded.supports_tasks",
        )
        .bind(account_id)
        .bind(&cal.url)
        .bind(&cal.display_name)
        .bind(&cal.color)
        .bind(cal.timezone.as_deref().unwrap_or(&fallback_tz))
        .bind(if cal.read_only { "reader" } else { "owner" })
        .bind(cal.supports_events as i64)
        .bind(cal.supports_tasks as i64)
        .execute(pool)
        .await
        .map_err(|e| crate::errors::user_facing(&anyhow::Error::from(e)))?;
    }

    // A reconnect clears the account's dead-credential mark, same as the
    // Google path.
    state.reauth.lock().expect("reauth mark poisoned").remove(&email);
    Ok(email)
}

/// Maps a CalDAV error to something the connect form can show.
fn user_facing_caldav(e: CalDavError) -> String {
    match e {
        CalDavError::Unauthorized => {
            "The server rejected the credentials. For iCloud, use an app-specific password from appleid.apple.com, not your Apple ID password.".to_string()
        }
        CalDavError::PreconditionFailed => e.to_string(),
        CalDavError::Http(status) => crate::errors::server_answered(status),
        CalDavError::Other(e) => crate::errors::user_facing(&e),
    }
}

/// Builds the client for a stored account.
pub(crate) fn client_for(
    email: &str,
    server_url: Option<&str>,
    username: Option<&str>,
) -> anyhow::Result<CalDavClient> {
    let base = server_url.ok_or_else(|| anyhow::anyhow!("account has no server url"))?;
    let login = username.filter(|s| !s.is_empty()).unwrap_or(email);
    let password = password_for(email)?;
    CalDavClient::new(base, login, &password)
}

/// The client (and collection URL) behind one CalDAV calendar, with the
/// read-only refusal every write path shares. Errors when the calendar does
/// not belong to a CalDAV account at all.
/// Every CalDAV create, update and delete passes through `client_for_calendar`,
/// so this is what a write to a subscribed or shared read-only collection
/// reports. A fixed literal, raised before anything leaves the machine.
pub(crate) const CALENDAR_IS_READ_ONLY: &str = "this calendar is read-only";

pub(crate) async fn client_for_calendar(
    state: &crate::AppState,
    calendar_id: i64,
) -> anyhow::Result<(CalDavClient, String)> {
    type Row = (String, Option<String>, Option<String>, String, String);
    let (email, server_url, username, access_role, collection_url): Row = sqlx::query_as(
        "SELECT a.email, a.server_url, a.username, c.access_role, c.google_id
         FROM calendars c JOIN accounts a ON a.id = c.account_id
         WHERE c.id = ?1 AND a.provider = 'caldav'",
    )
    .bind(calendar_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| anyhow::anyhow!("that calendar is not on a CalDAV account"))?;
    if access_role == "reader" {
        anyhow::bail!(CALENDAR_IS_READ_ONLY);
    }
    let client = client_for(&email, server_url.as_deref(), username.as_deref())?;
    Ok((client, collection_url))
}

/// Syncs every enabled calendar of one CalDAV account. Returns rows written;
/// an `Unauthorized` anywhere marks the whole account dead (one password
/// covers every collection).
pub(crate) async fn sync_account(
    pool: &SqlitePool,
    account_id: i64,
    client: &CalDavClient,
    window_start_ms: i64,
    window_end_ms: i64,
) -> Result<u64, CalDavError> {
    let cals: Vec<(i64, String, i64, i64)> = sqlx::query_as(
        "SELECT id, google_id, supports_events, supports_tasks FROM calendars
         WHERE account_id = ?1 AND sync_enabled = 1 ORDER BY id",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(anyhow::Error::from)?;

    sync_contacts(pool, account_id, client).await;

    let mut total = 0u64;
    for (cal_id, url, ev, tasks) in cals {
        let outcome = omacal_sync::caldav::sync_caldav_calendar(
            pool,
            client,
            cal_id,
            &url,
            ev != 0,
            tasks != 0,
            window_start_ms,
            window_end_ms,
            crate::now_ms(),
        )
        .await?;
        total += outcome.upserted as u64;
    }
    Ok(total)
}

/// How stale the stored contacts may be before an account's address books
/// are read again (#126). An hour: the suggestions are a convenience, the
/// books are fetched whole, and a name added on the phone showing up within
/// the hour is soon enough to be useful without making every five-minute
/// sync carry an address book.
const CONTACTS_MAX_AGE_MS: i64 = 60 * 60 * 1000;

/// Refreshes one account's contacts, for the attendee field's suggestions.
///
/// **Never fails the sync.** The calendars are what this account exists for,
/// and every path here — a server with no CardDAV, a REPORT it refuses, a
/// book that errors — leaves the previous contacts alone and says so in the
/// log. A partial read is not stored either: `replace_account_contacts`
/// replaces the whole set, so half an address book would read as the user
/// having deleted the other half.
async fn sync_contacts(pool: &SqlitePool, account_id: i64, client: &CalDavClient) {
    let last: Option<i64> =
        sqlx::query_scalar("SELECT MAX(updated_at) FROM contacts WHERE account_id = ?1")
            .bind(account_id)
            .fetch_one(pool)
            .await
            .unwrap_or(None);
    let now = crate::now_ms();
    if last.is_some_and(|t| now - t < CONTACTS_MAX_AGE_MS) {
        return;
    }

    let books = match client.discover_address_books().await {
        Ok(b) => b,
        Err(e) => {
            tracing::debug!(%e, account_id, "no address books read; contacts left as they were");
            return;
        }
    };
    if books.is_empty() {
        return;
    }

    let mut rows: Vec<omacal_store::StoredContact> = Vec::new();
    for book in &books {
        match client.cards(&book.url).await {
            Ok(cards) => {
                for card in cards {
                    for contact in omacal_caldav::parse_cards(&card.ics) {
                        for email in contact.emails {
                            rows.push(omacal_store::StoredContact {
                                href: card.url.clone(),
                                uid: contact.uid.clone(),
                                display_name: contact.name.clone(),
                                email,
                            });
                        }
                    }
                }
            }
            Err(e) => {
                // One unreadable book, and the stored set stays whole.
                tracing::debug!(%e, book = %book.url, "address book not read; contacts unchanged");
                return;
            }
        }
    }

    match omacal_store::replace_account_contacts(pool, account_id, &rows, now).await {
        Ok(n) => tracing::debug!(account_id, contacts = n, books = books.len(), "contacts refreshed"),
        Err(e) => tracing::warn!(%e, account_id, "contacts not stored"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keyring_key_cannot_collide_with_googles() {
        // Google stores under the bare email; the same address as an Apple ID
        // must land somewhere else or connecting one overwrites the other.
        assert_eq!(keyring_key("p@x.com"), "caldav:p@x.com");
        assert_ne!(keyring_key("p@x.com"), "p@x.com");
    }

    #[test]
    fn icloud_needs_no_url_and_generic_requires_one() {
        assert_eq!(base_url_for("icloud", None).unwrap(), omacal_caldav::ICLOUD_BASE);
        assert!(base_url_for("caldav", None).is_err());
        assert!(base_url_for("caldav", Some("  ")).is_err());
        assert_eq!(
            base_url_for("caldav", Some(" https://dav.example.com/ ")).unwrap(),
            "https://dav.example.com/"
        );
    }
}
