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

    omacal_sync::caldav::upsert_collections(pool, account_id, &calendars, &fallback_tz())
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;

    // A reconnect clears the account's dead-credential mark, same as the
    // Google path.
    state.reauth.lock().expect("reauth mark poisoned").remove(&email);
    Ok(email)
}

/// The zone for collections that publish none — an all-day date has to
/// resolve against something, and the zone the user is sitting in is the
/// only defensible guess.
fn fallback_tz() -> String {
    jiff::tz::TimeZone::system()
        .iana_name()
        .unwrap_or("UTC")
        .to_string()
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
///
/// The calendar list is refreshed first, so a collection made on another
/// device is synced in the same pass that finds it — see
/// `omacal_sync::caldav::refresh_collections` for what it may and may not
/// delete.
pub(crate) async fn sync_account(
    pool: &SqlitePool,
    account_id: i64,
    client: &CalDavClient,
    window_start_ms: i64,
    window_end_ms: i64,
) -> Result<u64, CalDavError> {
    omacal_sync::caldav::refresh_collections(pool, account_id, client, &fallback_tz()).await?;

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

    /// The wiring: a sync finds a list made since the account was connected
    /// and fetches its tasks in the same pass, rather than only on the sync
    /// after. The listing's own rules are `omacal_sync::caldav`'s tests.
    #[tokio::test]
    async fn a_sync_picks_up_a_task_list_made_after_the_account_was_connected() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let ns = r#"xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/""#;
        let server = MockServer::start().await;
        Mock::given(method("PROPFIND")).and(path("/"))
            .respond_with(ResponseTemplate::new(207).set_body_string(format!(
                r#"<?xml version="1.0"?><d:multistatus {ns}><d:response><d:href>/</d:href><d:propstat><d:prop>
                <d:current-user-principal><d:href>/principals/p/</d:href></d:current-user-principal>
                </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#)))
            .mount(&server).await;
        Mock::given(method("PROPFIND")).and(path("/principals/p/"))
            .respond_with(ResponseTemplate::new(207).set_body_string(format!(
                r#"<?xml version="1.0"?><d:multistatus {ns}><d:response><d:href>/principals/p/</d:href><d:propstat><d:prop>
                <c:calendar-home-set><d:href>/cals/p/</d:href></c:calendar-home-set>
                </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#)))
            .mount(&server).await;
        Mock::given(method("PROPFIND")).and(path("/cals/p/")).and(header("Depth", "1"))
            .respond_with(ResponseTemplate::new(207).set_body_string(format!(
                r#"<?xml version="1.0"?><d:multistatus {ns}>
                <d:response><d:href>/cals/p/bernard/</d:href><d:propstat><d:prop>
                <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>
                <d:displayname>Bernard</d:displayname>
                <c:supported-calendar-component-set><c:comp name="VTODO"/></c:supported-calendar-component-set>
                </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#)))
            .mount(&server).await;
        Mock::given(method("PROPFIND")).and(path("/cals/p/bernard/")).and(header("Depth", "0"))
            .respond_with(ResponseTemplate::new(207).set_body_string(format!(
                r#"<?xml version="1.0"?><d:multistatus {ns}><d:response><d:href>/cals/p/bernard/</d:href>
                <d:propstat><d:prop><cs:getctag>c1</cs:getctag></d:prop>
                <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#)))
            .mount(&server).await;
        Mock::given(method("REPORT")).and(path("/cals/p/bernard/"))
            .respond_with(ResponseTemplate::new(207).set_body_string(format!(
                r#"<?xml version="1.0"?><d:multistatus {ns}><d:response><d:href>/cals/p/bernard/t.ics</d:href>
                <d:propstat><d:prop><d:getetag>"1"</d:getetag><c:calendar-data>BEGIN:VCALENDAR&#13;
BEGIN:VTODO&#13;
UID:t-1&#13;
SUMMARY:Take the bins out&#13;
END:VTODO&#13;
END:VCALENDAR&#13;
</c:calendar-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#)))
            .mount(&server).await;

        let pool = omacal_store::connect_memory().await.unwrap();
        sqlx::query(
            "INSERT INTO accounts (google_sub, email, created_at, provider)
             VALUES ('caldav:e@x', 'e@x', 0, 'caldav')",
        )
        .execute(&pool).await.unwrap();
        let client = CalDavClient::new(&server.uri(), "e@x", "pw").unwrap();

        sync_account(&pool, 1, &client, 0, 1).await.unwrap();

        let lists: Vec<(String, i64, i64)> =
            sqlx::query_as("SELECT summary, supports_events, supports_tasks FROM calendars")
                .fetch_all(&pool).await.unwrap();
        assert_eq!(lists, vec![("Bernard".to_string(), 0, 1)]);
        let tasks: Vec<String> = sqlx::query_scalar("SELECT summary FROM tasks")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(tasks, vec!["Take the bins out".to_string()], "fetched in the same pass");
    }
}
