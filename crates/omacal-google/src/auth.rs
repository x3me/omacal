use anyhow::{bail, Context};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::Rng;
use sha2::{Digest, Sha256};

pub const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
pub const SCOPE: &str = "https://www.googleapis.com/auth/calendar";

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

/// RFC 7636 S256 pair. The verifier is 32 random bytes, base64url-unpadded.
pub fn generate_pkce() -> Pkce {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Pkce { verifier, challenge }
}

/// `prompt` carries two space-delimited values (OIDC allows a list here, and
/// `form_urlencoded` encodes the space as the browser expects), and both must
/// stay:
///
/// - `consent`, together with `access_type=offline` below, is what makes
///   Google issue a refresh token. Without it, re-authorising an
///   already-consented account silently returns an access token only, and
///   background sync breaks the moment that access token expires.
/// - `select_account` forces Google's account chooser to appear. Without it,
///   a browser signed into exactly one Google account — the common case —
///   walks straight through consent for the account already connected: "Add
///   account" looks like it works and adds nothing.
///
/// Neither value can do the other's job, so resist "simplifying" this back
/// down to one.
pub fn authorize_url(client_id: &str, redirect_uri: &str, challenge: &str, state: &str) -> String {
    let q = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPE)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent select_account")
        .finish();
    format!("{AUTH_ENDPOINT}?{q}")
}

#[derive(Clone)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at_ms: i64,
}

/// Hand-written so that neither token can reach a log line, a `dbg!`, or an
/// error report through `{:?}`. A refresh token in particular is a long-lived
/// credential; a derived `Debug` puts it one careless format string away from
/// the terminal. Only the expiry, which is not a secret, prints for real.
impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
}

/// A rejection from the token endpoint, carrying the OAuth error code as a
/// field so a caller can classify it without parsing the message back out of
/// a string. Its `Display` is exactly the sentence `post_token` always
/// emitted — our own text leading, for the reason given at the `Err` site —
/// so nothing downstream of `anyhow` sees a different message.
#[derive(Debug)]
pub struct TokenRejected {
    /// The endpoint's `error` field: `invalid_grant`, `unauthorized_client`, …
    pub code: String,
    description: String,
}

impl TokenRejected {
    /// For tests in downstream crates that need the typed rejection without a
    /// mocked endpoint — production code only ever gets one from `post_token`.
    pub fn new(code: impl Into<String>, description: impl Into<String>) -> Self {
        Self { code: code.into(), description: description.into() }
    }
}

impl std::fmt::Display for TokenRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the token endpoint rejected the request: {}: {}",
            self.code, self.description
        )
    }
}

impl std::error::Error for TokenRejected {}

/// Whether this error means the stored refresh token is dead for good, so
/// re-consent is the only fix and retrying is pure quota noise.
///
/// The three codes are the ways a grant stops being honoured (RFC 6749 §5.2):
/// `invalid_grant` is a token expired or revoked by the user, and
/// `unauthorized_client`/`invalid_client` are a token minted by one OAuth
/// client presented by another — which is exactly what happens when a machine
/// that signed in under a personal dev client runs a release build carrying
/// the official pair. Everything else (a 5xx dressed as
/// `temporarily_unavailable`, transport failures, non-JSON bodies) stays
/// unclassified and keeps being retried.
pub fn needs_reauth(e: &anyhow::Error) -> bool {
    e.downcast_ref::<TokenRejected>().is_some_and(|r| {
        matches!(r.code.as_str(), "invalid_grant" | "unauthorized_client" | "invalid_client")
    })
}

async fn post_token(endpoint: &str, form: &[(&str, &str)]) -> anyhow::Result<Tokens> {
    let resp = crate::client::http_client()
        .post(endpoint)
        .form(form)
        .send()
        .await
        .context("token endpoint unreachable")?;

    let body: TokenResponse = resp.json().await.context("token response was not JSON")?;

    if let Some(err) = body.error {
        // Our own text leads, deliberately. Both halves that follow are written
        // by the token endpoint, and `src-tauri/src/errors.rs` decides whether
        // to show an error verbatim by matching its *start* against a list of
        // strings this app emits. Let the endpoint's `error` field lead and it
        // can reproduce any of those strings exactly and have its
        // `error_description` rendered in the app header.
        //
        // A typed error rather than `bail!`, so `needs_reauth` can read the
        // code as a field instead of grepping the message.
        return Err(TokenRejected {
            code: err,
            description: body.error_description.unwrap_or_default(),
        }
        .into());
    }

    let access_token = body.access_token.context("token response had no access_token")?;
    let expires_in = body.expires_in.unwrap_or(3600);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;

    Ok(Tokens {
        access_token,
        refresh_token: body.refresh_token,
        // 60s of slack so a request in flight does not expire mid-call.
        expires_at_ms: now_ms + (expires_in - 60).max(0) * 1000,
    })
}

pub async fn exchange_code(
    token_endpoint: &str,
    client_id: &str,
    client_secret: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> anyhow::Result<Tokens> {
    post_token(
        token_endpoint,
        &[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
            ("code_verifier", verifier),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri),
        ],
    )
    .await
}

/// Tells Google to invalidate a refresh token — the server half of signing
/// out. Takes the endpoint as a parameter like everything here, so a test
/// can point it at wiremock; production callers pass
/// `https://oauth2.googleapis.com/revoke`.
///
/// Callers treat failure as advisory: the local sign-out proceeds either
/// way, because a token we can no longer revoke (already expired, network
/// down) must not hold the account hostage. The user can always finish the
/// job at myaccount.google.com/permissions.
pub async fn revoke(revoke_endpoint: &str, token: &str) -> anyhow::Result<()> {
    let resp = crate::client::http_client()
        .post(revoke_endpoint)
        .form(&[("token", token)])
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("revoke answered {}", resp.status());
    }
    Ok(())
}

pub async fn refresh(
    token_endpoint: &str,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> anyhow::Result<Tokens> {
    post_token(
        token_endpoint,
        &[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ],
    )
    .await
}

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Redirect {
    pub code: String,
    pub state: String,
}

pub const CANCELLED: &str = "Sign-in cancelled.";

/// How long the loopback listener waits for the browser before giving up.
///
/// Long enough for a human to read a consent screen, pick an account and type
/// a password; short enough that abandoning the flow is not permanent. An
/// explicit *deny* comes back as `?error=` and returns immediately — this
/// deadline is for the user who simply closes the tab, in which case nothing
/// ever reaches us. Explicit cancellation can end the wait sooner.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(180);

/// A connection that opens but never sends a request line would hang the read
/// just as `accept` used to hang. Port scanners do this by accident.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// How often the accept loop wakes to check the deadline. Idle cost only.
const POLL: Duration = Duration::from_millis(50);

/// What the user sees when the deadline passes. Plain language, and it names
/// the action to take — this is the string that reaches the header.
pub const TIMED_OUT: &str = "Sign-in timed out — no response from the browser. Try again.";

/// Binds an ephemeral loopback port for the OAuth redirect.
///
/// Google's installed-app flow allows any `http://127.0.0.1:<port>` redirect
/// without pre-registering the port, so we take whatever the OS gives us.
pub fn bind_loopback() -> anyhow::Result<(TcpListener, String)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    Ok((listener, format!("http://127.0.0.1:{port}")))
}

/// Accepts one connection, or gives up once `deadline` has elapsed.
///
/// The deadline lives on the listener rather than on the caller's future
/// deliberately. Wrapping the blocking call in `tokio::time::timeout` would
/// free the UI but abandon the thread inside `accept()`, which would then sit
/// blocked for the life of the process; polling a non-blocking listener means
/// the thread actually ends.
fn accept_before(listener: &TcpListener, deadline: Duration, cancelled: &AtomicBool) -> anyhow::Result<TcpStream> {
    listener.set_nonblocking(true)?;
    let start = Instant::now();
    loop {
        if cancelled.load(Ordering::Relaxed) { bail!(CANCELLED); }
        match listener.accept() {
            Ok((stream, _)) => {
                // Back to blocking for the request line and the reply, with a
                // read timeout so a silent client cannot reintroduce the hang.
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(READ_TIMEOUT))?;
                stream.set_write_timeout(Some(READ_TIMEOUT))?;
                return Ok(stream);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                let elapsed = start.elapsed();
                if elapsed >= deadline {
                    bail!("{TIMED_OUT}");
                }
                std::thread::sleep(POLL.min(deadline - elapsed));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Blocks until the browser hits the redirect URI, then returns the code.
/// Gives up after `deadline` — a user who opens the consent screen and then
/// closes the tab sends nothing at all, and without this the sign-in future
/// never resolves and the UI's `busy` flag never clears.
///
/// Blocking is deliberate: call it from `spawn_blocking`. Writing an async
/// HTTP server for a single one-shot request would be more machinery than the
/// problem deserves.
pub fn wait_for_redirect(listener: TcpListener, deadline: Duration) -> anyhow::Result<Redirect> {
    wait_for_redirect_cancellable(listener, deadline, &AtomicBool::new(false))
}

/// Ends the listener as well as the caller when consent is abandoned.
pub fn wait_for_redirect_cancellable(listener: TcpListener, deadline: Duration, cancelled: &AtomicBool) -> anyhow::Result<Redirect> {
    let mut stream = accept_before(&listener, deadline, cancelled)?;
    let mut line = String::new();
    BufReader::new(stream.try_clone()?).read_line(&mut line)?;

    // "GET /?code=...&state=... HTTP/1.1"
    let target = line.split_whitespace().nth(1).unwrap_or("/");
    let url = url::Url::parse(&format!("http://127.0.0.1{target}"))?;
    let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();

    let body = if params.contains_key("code") {
        "<html><body style=\"font:14px system-ui;padding:3rem\">\
         Signed in. You can close this tab.</body></html>"
    } else {
        "<html><body style=\"font:14px system-ui;padding:3rem\">\
         Sign-in failed. You can close this tab.</body></html>"
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )?;
    stream.flush()?;

    match (params.get("code"), params.get("state")) {
        (Some(code), Some(state)) => Ok(Redirect { code: code.clone(), state: state.clone() }),
        _ => anyhow::bail!(
            "authorisation failed: {}",
            params.get("error").map(String::as_str).unwrap_or("no code returned")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// The token refresh runs before every Google sync, so a token endpoint
    /// that hangs would hang them all.
    ///
    /// The server hangs for a minute and the client (two seconds under test)
    /// must give up within half of it. A five-second hang left three seconds
    /// of slack, which a loaded machine ate: the timer fired late and the test
    /// failed now and then (2026-11-22, load average 15 to 55). The minute
    /// costs nothing: the server is dropped once the client gives up.
    #[tokio::test]
    async fn a_token_endpoint_that_hangs_is_given_up_on() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(60)))
            .mount(&server).await;
        let started = std::time::Instant::now();
        assert!(refresh(&format!("{}/token", server.uri()), "id", "secret", "rt").await.is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(30), "gave up before the server answered");
    }

    #[test]
    fn pkce_challenge_is_url_safe_and_unpadded() {
        let p = generate_pkce();
        assert!(p.challenge.len() >= 43);
        assert!(!p.challenge.contains('='), "challenge must be unpadded");
        assert!(!p.challenge.contains('+') && !p.challenge.contains('/'),
                "challenge must be URL-safe base64");
    }

    #[test]
    fn pkce_verifiers_differ_between_calls() {
        assert_ne!(generate_pkce().verifier, generate_pkce().verifier);
    }

    #[test]
    fn the_authorize_url_carries_everything_google_requires() {
        let url = authorize_url("cid", "http://127.0.0.1:9999", "chal", "st");
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert!(url.contains("client_id=cid"));
        assert!(url.contains("code_challenge=chal"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=st"));
        // Both are required to receive a refresh token at all.
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        // ...and select_account, or "Add account" is a no-op whenever exactly
        // one Google account is signed into the browser: prompt=consent alone
        // reauthorises the account already connected instead of offering a
        // chooser. `prompt=consent` above is a prefix match and stays true
        // either way, so this needs its own assertion.
        assert!(url.contains("select_account"), "prompt must also request the account chooser: {url}");
        assert!(url.contains("calendar"));
    }

    #[tokio::test]
    async fn exchanging_a_code_returns_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "at-1",
                "refresh_token": "rt-1",
                "expires_in": 3599,
                "token_type": "Bearer"
            })))
            .mount(&server)
            .await;

        let t = exchange_code(&format!("{}/token", server.uri()),
                              "cid", "secret", "code", "verifier", "http://127.0.0.1:9999")
            .await.unwrap();
        assert_eq!(t.access_token, "at-1");
        assert_eq!(t.refresh_token.as_deref(), Some("rt-1"));
        assert!(t.expires_at_ms > 0);
    }

    #[tokio::test]
    async fn a_refresh_response_without_a_refresh_token_is_accepted() {
        // Google omits refresh_token on refresh; the caller keeps the old one.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "at-2", "expires_in": 3599, "token_type": "Bearer"
            })))
            .mount(&server)
            .await;

        let t = refresh(&format!("{}/token", server.uri()), "cid", "secret", "rt-1")
            .await.unwrap();
        assert_eq!(t.access_token, "at-2");
        assert!(t.refresh_token.is_none());
    }

    #[tokio::test]
    async fn an_oauth_error_response_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant",
                "error_description": "Token has been expired or revoked."
            })))
            .mount(&server)
            .await;

        let err = refresh(&format!("{}/token", server.uri()), "cid", "secret", "rt-old")
            .await.unwrap_err();
        assert!(err.to_string().contains("invalid_grant"));
    }

    /// The classifier the sync path uses to stop retrying a dead grant —
    /// exercised through the real `refresh` path, so what is classified is
    /// what the endpoint actually produces rather than a hand-built error.
    /// All three codes, because each is a distinct way a grant dies:
    /// revocation, and a client-pair mismatch spelled two ways.
    #[tokio::test]
    async fn a_dead_grant_is_classified_as_needing_reauth() {
        for code in ["invalid_grant", "unauthorized_client", "invalid_client"] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/token"))
                .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                    "error": code,
                    "error_description": "endpoint detail"
                })))
                .mount(&server)
                .await;

            let err = refresh(&format!("{}/token", server.uri()), "cid", "secret", "rt")
                .await.unwrap_err();
            assert!(needs_reauth(&err), "{code} means re-consent is the only fix");
        }
    }

    /// The other half, without which the classifier could be `|_| true`: a
    /// retryable failure must stay unclassified, or one flaky answer would
    /// park an account behind a reconnect prompt it does not need.
    #[tokio::test]
    async fn a_transient_failure_is_not_classified_as_needing_reauth() {
        // An OAuth-shaped error that is not a dead grant.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(503).set_body_json(serde_json::json!({
                "error": "temporarily_unavailable",
                "error_description": "try again"
            })))
            .mount(&server)
            .await;

        let err = refresh(&format!("{}/token", server.uri()), "cid", "secret", "rt")
            .await.unwrap_err();
        assert!(!needs_reauth(&err), "a 503 is a retry, not a reconnect");

        // And an error with no token-endpoint shape at all — offline, bad
        // JSON — is never a reason to demand re-consent.
        assert!(!needs_reauth(&anyhow::anyhow!("token endpoint unreachable")));
    }

    /// The token endpoint writes both `error` and `error_description`, and
    /// `src-tauri/src/errors.rs` decides whether to show an error verbatim by
    /// matching the *start* of the string against messages this app emits. Let
    /// the endpoint's text lead and it can name any of them and have its
    /// description rendered in the app header.
    ///
    /// The impersonated string below is a copy of one of those messages; the
    /// allowlist itself is guarded on its own side of the crate boundary.
    #[tokio::test]
    async fn a_token_endpoint_error_never_leads_the_message() {
        const IMPERSONATED: &str = "state mismatch — possible CSRF, sign-in aborted";

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": IMPERSONATED,
                "error_description": "endpoint-controlled text"
            })))
            .mount(&server)
            .await;

        let err = refresh(&format!("{}/token", server.uri()), "cid", "secret", "rt")
            .await.unwrap_err().to_string();

        assert!(!err.starts_with(IMPERSONATED),
                "external text leads the message, so a prefix allowlist will pass it through: {err}");
        assert!(err.starts_with("the token endpoint rejected the request"),
                "the message must lead with our own text: {err}");
        // The endpoint's detail is still carried — it belongs in the log.
        assert!(err.contains("endpoint-controlled text"), "{err}");
    }

    #[test]
    fn debug_output_never_carries_a_token_value() {
        let t = Tokens {
            access_token: "ya29-secret-access".into(),
            refresh_token: Some("1//secret-refresh".into()),
            expires_at_ms: 1_785_736_800_000,
        };
        let printed = format!("{t:?}");
        assert!(!printed.contains("ya29-secret-access"), "access token leaked: {printed}");
        assert!(!printed.contains("1//secret-refresh"), "refresh token leaked: {printed}");
        assert!(printed.contains("<redacted>"));
        // The expiry is not a secret and stays useful in a log line.
        assert!(printed.contains("1785736800000"), "expiry should still print: {printed}");
    }

    #[test]
    fn debug_output_distinguishes_an_absent_refresh_token() {
        let t = Tokens {
            access_token: "at".into(),
            refresh_token: None,
            expires_at_ms: 0,
        };
        assert!(format!("{t:?}").contains("refresh_token: None"));
    }

    /// Generous enough that a loaded machine cannot make the happy-path tests
    /// flaky, while still proving the deadline plumbing is wired up.
    const TEST_DEADLINE: Duration = Duration::from_secs(30);

    #[tokio::test]
    async fn the_loopback_listener_captures_code_and_state() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let handle =
            tokio::task::spawn_blocking(move || wait_for_redirect(listener, TEST_DEADLINE));

        // Simulate the browser hitting the redirect URI.
        let _ = reqwest::get(format!("http://127.0.0.1:{port}/?code=abc123&state=xyz")).await;

        let got = handle.await.unwrap().unwrap();
        assert_eq!(got.code, "abc123");
        assert_eq!(got.state, "xyz");
    }

    #[tokio::test]
    async fn the_loopback_listener_reports_a_denied_consent() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle =
            tokio::task::spawn_blocking(move || wait_for_redirect(listener, TEST_DEADLINE));
        let _ = reqwest::get(format!("http://127.0.0.1:{port}/?error=access_denied")).await;
        assert!(handle.await.unwrap().is_err());
    }

    /// The abandoned-consent case: the user opens the tab and closes it, so
    /// nothing ever connects. Before the deadline this call blocked forever,
    /// `sign_in` never returned, and the UI's only button stayed disabled
    /// until the app was restarted. A tiny deadline keeps the test fast.
    #[tokio::test]
    async fn a_listener_nobody_ever_hits_times_out_instead_of_blocking_forever() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let handle = tokio::task::spawn_blocking(move || {
            wait_for_redirect(listener, Duration::from_millis(120))
        });

        // The whole point is that this returns. Bound the test so a regression
        // fails in seconds rather than hanging the suite.
        let out = tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("wait_for_redirect never returned — the deadline is not being honoured")
            .unwrap();

        // Matched rather than `unwrap_err`ed: `Redirect` holds an
        // authorization code and deliberately has no `Debug`.
        let err = match out {
            Ok(_) => panic!("a listener nobody connected to returned a redirect"),
            Err(e) => e.to_string(),
        };
        assert_eq!(err, TIMED_OUT);
        // The user has to be able to act on it, so it must not read as a
        // socket error escaping from the plumbing.
        assert!(!err.to_lowercase().contains("os error"), "raw io error leaked: {err}");
    }

    /// The deadline is a ceiling, not a fixed wait: a browser that arrives
    /// promptly must not be made to wait it out.
    ///
    /// A two-minute ceiling, and back within half of it. Thirty seconds against
    /// five left too little room for a loaded machine (2026-11-22); the gap
    /// between the two is what the test is about, not either number.
    #[tokio::test]
    async fn a_redirect_that_arrives_in_time_does_not_wait_out_the_deadline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let started = Instant::now();
        let handle = tokio::task::spawn_blocking(move || {
            wait_for_redirect(listener, Duration::from_secs(120))
        });
        let _ = reqwest::get(format!("http://127.0.0.1:{port}/?code=c&state=s")).await;
        assert!(handle.await.unwrap().is_ok());
        assert!(started.elapsed() < Duration::from_secs(60), "returned only after the deadline");
    }

    #[test]
    fn cancelling_abandoned_consent_closes_listener_and_allows_a_fresh_attempt() {
        let (listener, _) = bind_loopback().unwrap();
        let address = listener.local_addr().unwrap();
        let cancelled = std::sync::Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let worker = std::thread::spawn(move || {
            match wait_for_redirect_cancellable(listener, Duration::from_millis(500), &worker_cancelled) {
                Ok(_) => panic!("cancelled listener returned a code"),
                Err(e) => e.to_string(),
            }
        });
        std::thread::sleep(Duration::from_millis(60));
        cancelled.store(true, Ordering::Relaxed);
        assert_eq!(worker.join().unwrap(), CANCELLED);
        assert!(TcpStream::connect(address).is_err(), "cancel must close the old listener");
        let (fresh, _) = bind_loopback().unwrap();
        let address = fresh.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            wait_for_redirect_cancellable(fresh, Duration::from_secs(2), &AtomicBool::new(false)).is_ok()
        });
        let mut browser = TcpStream::connect(address).unwrap();
        browser.write_all(b"GET /?code=fresh&state=fresh HTTP/1.1\r\n\r\n").unwrap();
        assert!(worker.join().unwrap(), "a new attempt must accept its own redirect");
    }

    #[test]
    fn the_sign_in_deadline_is_long_enough_for_a_human_consent_flow() {
        assert!(SIGN_IN_TIMEOUT >= Duration::from_secs(60));
    }
}
