//! Zoom OAuth and meeting creation.
//!
//! Zoom is deliberately a connection of its own rather than another calendar
//! account. A Google or CalDAV event can carry the link, while the meeting
//! itself is created through the user's Zoom account. Native-app OAuth uses
//! Authorization Code + PKCE, so no Zoom client secret exists in this app.

use serde::{Deserialize, Serialize};

use crate::write::{EventFields, When};
use crate::AppState;

const AUTH_ENDPOINT: &str = "https://zoom.us/oauth/authorize";
const TOKEN_ENDPOINT: &str = "https://zoom.us/oauth/token";
const API_ROOT: &str = "https://api.zoom.us/v2";
const SCOPE: &str = "meeting:write:meeting meeting:delete:meeting";
const KEYRING_ACCOUNT: &str = "zoom:oauth";

pub(crate) const NOT_CONFIGURED: &str =
    "Zoom meeting creation is not configured. Add zoom_public_client_id to config.toml and restart omacal.";
pub(crate) const RECONNECT: &str =
    "Zoom needs to be connected again. Open Settings → Accounts and reconnect Zoom.";
pub(crate) const AUTH_FAILED: &str =
    "Zoom could not be connected. Check the Zoom app configuration and try again.";
pub(crate) const CREATE_FAILED: &str =
    "Zoom could not create the meeting. The calendar change was not saved.";
pub(crate) const ALL_DAY_UNSUPPORTED: &str =
    "Automatic Zoom meetings need a start time. Paste an existing Zoom link for an all-day event.";
pub(crate) const TOO_LONG: &str = "Zoom meetings created by omacal must be 24 hours or shorter.";

/// The small bit of connection state the UI needs. `configured` and
/// `connected` are separate on purpose: the first tells the user whether a
/// Connect button can work, while the second says whether meeting creation can
/// happen now.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZoomStatus {
    pub configured: bool,
    pub connected: bool,
}

/// Stored as one JSON value under one keyring entry. Zoom rotates refresh
/// tokens, so replacing the whole pair atomically is safer than maintaining
/// two entries that can get out of step.
#[derive(Serialize, Deserialize)]
struct StoredTokens {
    access_token: String,
    refresh_token: String,
    expires_at_ms: i64,
}

/// No derived `Debug`: both fields are credentials. The only diagnostic we
/// ever need is the non-secret expiry.
impl std::fmt::Debug for StoredTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredTokens")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct MeetingResponse {
    id: Option<u64>,
    join_url: Option<String>,
}

/// The durable identity returned by Zoom. The URL is what calendar guests
/// use; the numeric id is retained so a calendar write that fails before
/// attaching that URL can remove the otherwise-orphaned Zoom resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CreatedZoomMeeting {
    pub id: u64,
    pub join_url: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct MeetingRequest {
    topic: String,
    #[serde(rename = "type")]
    kind: u8,
    start_time: String,
    duration: i64,
    timezone: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    agenda: Option<String>,
}

#[derive(Debug)]
struct TokenFailure {
    reconnect: bool,
}

impl std::fmt::Display for TokenFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.reconnect {
            RECONNECT
        } else {
            AUTH_FAILED
        })
    }
}

impl std::error::Error for TokenFailure {}

struct TokenCache {
    loaded: bool,
    tokens: Option<StoredTokens>,
}

/// One keyring read per launch, as with Google credentials in `AppState`.
/// Besides avoiding repeated macOS Keychain prompts, the lock serialises
/// refresh-token rotation: a second refresh can never present the token the
/// first request just replaced. It is never held during meeting creation.
static TOKENS: tokio::sync::Mutex<TokenCache> = tokio::sync::Mutex::const_new(TokenCache {
    loaded: false,
    tokens: None,
});

fn entry() -> anyhow::Result<keyring::Entry> {
    Ok(keyring::Entry::new(
        crate::KEYRING_SERVICE,
        KEYRING_ACCOUNT,
    )?)
}

fn load_tokens() -> anyhow::Result<Option<StoredTokens>> {
    match entry()?.get_password() {
        Ok(raw) => Ok(Some(serde_json::from_str(&raw)?)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn ensure_loaded(cache: &mut TokenCache) -> anyhow::Result<()> {
    if !cache.loaded {
        cache.tokens = load_tokens()?;
        cache.loaded = true;
    }
    Ok(())
}

fn store_tokens(tokens: &StoredTokens) -> anyhow::Result<()> {
    entry()?.set_password(&serde_json::to_string(tokens)?)?;
    Ok(())
}

fn forget_tokens() -> anyhow::Result<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn public_client_id() -> anyhow::Result<String> {
    crate::load_zoom_public_client_id()?.ok_or_else(|| anyhow::anyhow!(NOT_CONFIGURED))
}

pub(crate) fn authorize_url(
    endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state)
        .append_pair("scope", SCOPE)
        .finish();
    format!("{endpoint}?{query}")
}

async fn token_request(endpoint: &str, form: &[(&str, &str)]) -> anyhow::Result<StoredTokens> {
    let response = reqwest::Client::new()
        .post(endpoint)
        .form(form)
        .send()
        .await
        .map_err(|e| {
            tracing::warn!(%e, "Zoom token endpoint was unreachable");
            TokenFailure { reconnect: false }
        })?;
    let status = response.status();
    let body: TokenResponse = response.json().await.map_err(|e| {
        tracing::warn!(%e, %status, "Zoom token response was not JSON");
        TokenFailure { reconnect: false }
    })?;

    if !status.is_success() || body.error.is_some() {
        let code = body.error.as_deref().unwrap_or("http_error");
        tracing::warn!(%status, error = code, "Zoom rejected an OAuth token request");
        return Err(TokenFailure {
            reconnect: matches!(
                code,
                "invalid_grant" | "invalid_client" | "unauthorized_client"
            ),
        }
        .into());
    }

    let access_token = body.access_token.ok_or_else(|| {
        tracing::warn!("Zoom token response omitted access_token");
        TokenFailure { reconnect: false }
    })?;
    let refresh_token = body.refresh_token.ok_or_else(|| {
        tracing::warn!("Zoom token response omitted refresh_token");
        TokenFailure { reconnect: false }
    })?;
    let expires_in = body.expires_in.unwrap_or(3600);
    Ok(StoredTokens {
        access_token,
        refresh_token,
        // One minute of slack keeps a request in flight from crossing expiry.
        expires_at_ms: crate::now_ms() + (expires_in - 60).max(0) * 1000,
    })
}

async fn exchange_code(
    endpoint: &str,
    client_id: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> anyhow::Result<StoredTokens> {
    token_request(
        endpoint,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
        ],
    )
    .await
}

async fn refresh_tokens(
    endpoint: &str,
    client_id: &str,
    refresh_token: &str,
) -> anyhow::Result<StoredTokens> {
    token_request(
        endpoint,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
        ],
    )
    .await
}

async fn access_token(force_refresh: bool) -> anyhow::Result<String> {
    let mut cache = TOKENS.lock().await;
    let client_id = public_client_id()?;
    ensure_loaded(&mut cache)?;
    let stored = cache
        .tokens
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!(RECONNECT))?;
    if !force_refresh && stored.expires_at_ms > crate::now_ms() {
        return Ok(stored.access_token.clone());
    }
    let refresh_token = stored.refresh_token.clone();

    match refresh_tokens(TOKEN_ENDPOINT, &client_id, &refresh_token).await {
        Ok(next) => {
            let token = next.access_token.clone();
            store_tokens(&next)?;
            cache.tokens = Some(next);
            Ok(token)
        }
        Err(e) => {
            if e.downcast_ref::<TokenFailure>()
                .is_some_and(|f| f.reconnect)
            {
                let _ = forget_tokens();
                cache.tokens = None;
            }
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn zoom_status() -> Result<ZoomStatus, String> {
    async fn inner() -> anyhow::Result<ZoomStatus> {
        let configured = crate::load_zoom_public_client_id()?.is_some();
        let connected = if configured {
            let mut cache = TOKENS.lock().await;
            ensure_loaded(&mut cache)?;
            cache.tokens.is_some()
        } else {
            false
        };
        Ok(ZoomStatus {
            configured,
            connected,
        })
    }
    inner().await.map_err(|e| crate::errors::user_facing(&e))
}

#[tauri::command]
pub async fn connect_zoom(state: tauri::State<'_, AppState>) -> Result<ZoomStatus, String> {
    crate::demo_sync_guard(state.demo)?;
    connect_zoom_impl()
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

async fn connect_zoom_impl() -> anyhow::Result<ZoomStatus> {
    let client_id = public_client_id()?;
    let pkce = omacal_google::auth::generate_pkce();
    let csrf = omacal_google::auth::generate_pkce().verifier;
    let (listener, redirect_uri) = omacal_google::auth::bind_loopback()?;
    let url = authorize_url(
        AUTH_ENDPOINT,
        &client_id,
        &redirect_uri,
        &pkce.challenge,
        &csrf,
    );

    crate::browser::open_external(&url).map_err(|e| {
        tracing::warn!(%e, "Zoom sign-in could not open a browser");
        anyhow::anyhow!(crate::BROWSER_FAILED)
    })?;
    let redirect = tokio::task::spawn_blocking(move || {
        omacal_google::auth::wait_for_redirect(listener, omacal_google::auth::SIGN_IN_TIMEOUT)
    })
    .await??;
    if redirect.state != csrf {
        anyhow::bail!("state mismatch — possible CSRF, sign-in aborted");
    }

    let tokens = exchange_code(
        TOKEN_ENDPOINT,
        &client_id,
        &redirect.code,
        &pkce.verifier,
        &redirect_uri,
    )
    .await?;
    let mut cache = TOKENS.lock().await;
    store_tokens(&tokens)?;
    cache.loaded = true;
    cache.tokens = Some(tokens);
    Ok(ZoomStatus {
        configured: true,
        connected: true,
    })
}

#[tauri::command]
pub async fn disconnect_zoom(state: tauri::State<'_, AppState>) -> Result<ZoomStatus, String> {
    crate::demo_sync_guard(state.demo)?;
    let mut cache = TOKENS.lock().await;
    forget_tokens().map_err(|e| crate::errors::user_facing(&e))?;
    cache.loaded = true;
    cache.tokens = None;
    Ok(ZoomStatus {
        configured: crate::load_zoom_public_client_id()
            .map_err(|e| crate::errors::user_facing(&e))?
            .is_some(),
        connected: false,
    })
}

fn truncate_chars(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn request_for(fields: &EventFields) -> anyhow::Result<MeetingRequest> {
    let (start_ms, end_ms) = match &fields.when {
        When::Timed { start_ms, end_ms } => (*start_ms, *end_ms),
        When::AllDay { .. } => anyhow::bail!(ALL_DAY_UNSUPPORTED),
    };
    let span = end_ms - start_ms;
    if span <= 0 {
        anyhow::bail!(CREATE_FAILED);
    }
    let duration = (span + 59_999) / 60_000;
    if duration > 1_440 {
        anyhow::bail!(TOO_LONG);
    }
    let start = jiff::Timestamp::from_millisecond(start_ms).map_err(|e| {
        tracing::warn!(%e, "event start was outside Zoom's timestamp range");
        anyhow::anyhow!(CREATE_FAILED)
    })?;
    let topic = fields
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Untitled event");
    let agenda = fields
        .description
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| truncate_chars(s, 2_000));
    Ok(MeetingRequest {
        topic: truncate_chars(topic, 200),
        kind: 2,
        start_time: start.to_string(),
        duration,
        timezone: "UTC",
        agenda,
    })
}

async fn send_create(
    api_root: &str,
    token: &str,
    request: &MeetingRequest,
) -> anyhow::Result<(reqwest::StatusCode, Option<MeetingResponse>)> {
    let url = format!("{}/users/me/meetings", api_root.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(token)
        .json(request)
        .send()
        .await
        .map_err(|e| {
            tracing::warn!(%e, "Zoom meeting endpoint was unreachable");
            anyhow::anyhow!(CREATE_FAILED)
        })?;
    let status = response.status();
    if !status.is_success() {
        tracing::warn!(%status, "Zoom rejected meeting creation");
        return Ok((status, None));
    }
    let body: MeetingResponse = response.json().await.map_err(|e| {
        tracing::warn!(%e, "Zoom meeting response was not JSON");
        anyhow::anyhow!(CREATE_FAILED)
    })?;
    Ok((status, Some(body)))
}

async fn send_delete(
    api_root: &str,
    token: &str,
    meeting_id: u64,
) -> anyhow::Result<reqwest::StatusCode> {
    let url = format!("{}/meetings/{meeting_id}", api_root.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .delete(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| {
            tracing::warn!(%e, meeting_id, "Zoom meeting cleanup endpoint was unreachable");
            anyhow::anyhow!("Zoom meeting cleanup failed")
        })?;
    Ok(response.status())
}

fn valid_join_url(raw: &str) -> bool {
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    url.scheme() == "https" && (host == "zoom.us" || host.ends_with(".zoom.us"))
}

/// Creates the Zoom resource and returns both its durable id and its
/// attendee-facing URL. The caller owns attaching that URL to the calendar
/// event. A 401 receives one forced refresh and one retry; every other failure
/// is final, so a click never creates two meetings merely because the server
/// returned an error.
pub(crate) async fn create_meeting(fields: &EventFields) -> anyhow::Result<CreatedZoomMeeting> {
    let request = request_for(fields)?;
    let mut token = access_token(false).await?;
    let (mut status, mut response) = send_create(API_ROOT, &token, &request).await?;
    if status == reqwest::StatusCode::UNAUTHORIZED {
        token = access_token(true).await?;
        (status, response) = send_create(API_ROOT, &token, &request).await?;
    }
    if !status.is_success() {
        anyhow::bail!(CREATE_FAILED);
    }
    let response = response.ok_or_else(|| anyhow::anyhow!(CREATE_FAILED))?;
    let id = response.id.filter(|id| *id > 0).ok_or_else(|| {
        tracing::warn!("Zoom meeting response omitted a valid id");
        anyhow::anyhow!(CREATE_FAILED)
    })?;
    let Some(join_url) = response.join_url.filter(|u| valid_join_url(u)) else {
        tracing::warn!(
            meeting_id = id,
            "Zoom meeting response omitted a valid join_url"
        );
        // The meeting exists and its id is usable even though its attendee URL
        // is not. Do not knowingly leave that resource behind.
        if let Err(e) = delete_meeting(id).await {
            tracing::warn!(%e, meeting_id = id,
                "could not clean up a Zoom meeting with an invalid response");
        }
        anyhow::bail!(CREATE_FAILED);
    };
    Ok(CreatedZoomMeeting { id, join_url })
}

/// Deletes a Zoom meeting that could not be attached to a calendar event.
/// `404` is success: it is already absent. As with creation, one 401 refreshes
/// the rotating OAuth credentials and retries exactly once.
pub(crate) async fn delete_meeting(meeting_id: u64) -> anyhow::Result<()> {
    let mut token = access_token(false).await?;
    let mut status = send_delete(API_ROOT, &token, meeting_id).await?;
    if status == reqwest::StatusCode::UNAUTHORIZED {
        token = access_token(true).await?;
        status = send_delete(API_ROOT, &token, meeting_id).await?;
    }
    if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    tracing::warn!(%status, meeting_id, "Zoom rejected meeting cleanup");
    anyhow::bail!("Zoom meeting cleanup failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::{ConferenceAction, When};
    use wiremock::matchers::{body_json, body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fields() -> EventFields {
        EventFields {
            summary: Some("Meet with Tim".into()),
            location: None,
            description: Some("Quarterly planning".into()),
            when: When::Timed {
                start_ms: 1_788_895_800_000,
                end_ms: 1_788_897_630_001,
            },
            tz: "America/Chicago".into(),
            recurrence: None,
            guests: None,
            reminders: None,
            conference: Some(ConferenceAction::Remove),
            create_zoom: false,
        }
    }

    #[test]
    fn authorize_url_is_pkce_and_requests_only_meeting_lifecycle_access() {
        let raw = authorize_url(
            "https://zoom.example/authorize",
            "public client",
            "http://127.0.0.1:43125",
            "challenge",
            "csrf",
        );
        let url = url::Url::parse(&raw).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.get("response_type").map(String::as_str), Some("code"));
        assert_eq!(
            query.get("client_id").map(String::as_str),
            Some("public client")
        );
        assert_eq!(
            query.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(
            query.get("scope").map(String::as_str),
            Some("meeting:write:meeting meeting:delete:meeting")
        );
        assert_eq!(query.get("state").map(String::as_str), Some("csrf"));
    }

    #[tokio::test]
    async fn code_exchange_uses_public_pkce_form_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("client_id=public-id"))
            .and(body_string_contains("code_verifier=verifier"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "access",
                "refresh_token": "refresh",
                "expires_in": 3600
            })))
            .mount(&server)
            .await;

        let got = exchange_code(
            &format!("{}/token", server.uri()),
            "public-id",
            "code",
            "verifier",
            "http://127.0.0.1:1234",
        )
        .await
        .unwrap();
        assert_eq!(got.access_token, "access");
        assert_eq!(got.refresh_token, "refresh");
    }

    #[tokio::test]
    async fn refresh_uses_and_replaces_the_rotating_refresh_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=old-refresh"))
            .and(body_string_contains("client_id=public-id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "new-access",
                "refresh_token": "new-refresh",
                "expires_in": 3600
            })))
            .mount(&server)
            .await;

        let got = refresh_tokens(
            &format!("{}/token", server.uri()),
            "public-id",
            "old-refresh",
        )
        .await
        .unwrap();
        assert_eq!(got.access_token, "new-access");
        assert_eq!(got.refresh_token, "new-refresh");
    }

    #[test]
    fn meeting_request_uses_utc_and_rounds_partial_minutes_up() {
        let request = request_for(&fields()).unwrap();
        assert_eq!(request.topic, "Meet with Tim");
        assert_eq!(request.kind, 2);
        assert_eq!(request.duration, 31);
        assert_eq!(request.timezone, "UTC");
        assert!(request.start_time.ends_with('Z'));
        assert_eq!(request.agenda.as_deref(), Some("Quarterly planning"));
    }

    #[test]
    fn all_day_and_over_day_meetings_are_refused_before_the_api() {
        let mut all_day = fields();
        all_day.when = When::AllDay {
            start_date: "2026-08-25".into(),
            end_date: "2026-08-26".into(),
        };
        assert_eq!(
            request_for(&all_day).unwrap_err().to_string(),
            ALL_DAY_UNSUPPORTED
        );

        let mut long = fields();
        long.when = When::Timed {
            start_ms: 0,
            end_ms: 86_400_001,
        };
        assert_eq!(request_for(&long).unwrap_err().to_string(), TOO_LONG);
    }

    #[tokio::test]
    async fn meeting_create_sends_the_bearer_and_retains_the_cleanup_id() {
        let server = MockServer::start().await;
        let request = request_for(&fields()).unwrap();
        Mock::given(method("POST"))
            .and(path("/users/me/meetings"))
            .and(header("authorization", "Bearer access-token"))
            .and(body_json(serde_json::to_value(&request).unwrap()))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": 123456789,
                "join_url": "https://us02web.zoom.us/j/123456789?pwd=x",
                "start_url": "https://zoom.us/s/secret-host-token"
            })))
            .mount(&server)
            .await;
        let (status, response) = send_create(&server.uri(), "access-token", &request)
            .await
            .unwrap();
        assert_eq!(status, reqwest::StatusCode::CREATED);
        let response = response.unwrap();
        assert_eq!(response.id, Some(123456789));
        assert_eq!(
            response.join_url.as_deref(),
            Some("https://us02web.zoom.us/j/123456789?pwd=x")
        );
    }

    #[tokio::test]
    async fn meeting_delete_sends_the_bearer_to_the_retained_id() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/meetings/123456789"))
            .and(header("authorization", "Bearer access-token"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let status = send_delete(&server.uri(), "access-token", 123456789)
            .await
            .unwrap();
        assert_eq!(status, reqwest::StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn meeting_delete_can_treat_an_already_absent_meeting_as_success() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/meetings/123456789"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let status = send_delete(&server.uri(), "access-token", 123456789)
            .await
            .unwrap();
        assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
    }

    /// Manual release evidence, deliberately ignored by ordinary CI. This is
    /// the one test wiremock cannot replace: Zoom must accept the ephemeral
    /// loopback redirect for the Marketplace app, mint real tokens, create a
    /// meeting, and delete that same resource again. It never prints tokens.
    #[tokio::test]
    #[ignore = "requires OMACAL_LIVE_ZOOM=1, a Marketplace public client id, and browser consent"]
    async fn live_marketplace_pkce_loopback_and_meeting_round_trip() {
        assert_eq!(
            std::env::var("OMACAL_LIVE_ZOOM").as_deref(),
            Ok("1"),
            "set OMACAL_LIVE_ZOOM=1 only for an intentional live test"
        );
        let status = connect_zoom_impl().await.unwrap();
        assert!(status.configured && status.connected);

        let mut live = fields();
        let start_ms = crate::now_ms() + 10 * 60_000;
        live.summary = Some("omacal Zoom Marketplace live E2E".into());
        live.description = None;
        live.when = When::Timed {
            start_ms,
            end_ms: start_ms + 30 * 60_000,
        };

        let created = create_meeting(&live).await.unwrap();
        let valid = created.id > 0 && valid_join_url(&created.join_url);
        let cleanup = delete_meeting(created.id).await;
        cleanup.expect("live test created a meeting but could not delete it");
        assert!(valid, "Zoom returned an invalid meeting id or join URL");
    }

    #[test]
    fn only_https_zoom_hosts_are_accepted_as_join_links() {
        assert!(valid_join_url("https://us02web.zoom.us/j/123"));
        assert!(valid_join_url("https://zoom.us/j/123"));
        assert!(!valid_join_url("http://zoom.us/j/123"));
        assert!(!valid_join_url("https://zoom.us.evil.example/j/123"));
    }
}
