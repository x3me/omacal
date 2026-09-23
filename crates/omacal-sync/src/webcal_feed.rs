//! Read-only WebCal subscriptions: one public `webcal://` / `https://` feed URL
//! synced into one `reader` calendar.
//!
//! The shape deliberately mirrors [`crate::caldav::sync_caldav_calendar`]:
//! a cheap change probe (conditional GET on the stored ETag / Last-Modified,
//! kept in `sync_state.sync_token` — the same row CalDAV's ctag rides in),
//! then a full fetch, `BEGIN IMMEDIATE` plus a `sync_enabled` re-check,
//! upserts and window-bounded inferred deletions.
//!
//! Differences are honest, not hidden. No ctag, REPORT or per-resource etag
//! exists here, so the whole file is parsed at once (`omacal_caldav::parse`
//! once, then `events_in`) and the window filter is client-side. No
//! credentials travel, so redirects may cross hosts (feeds commonly
//! redirect); cleartext `http://` outside the user's own network is still
//! refused, exactly like CalDAV.
//!
//! Tasks (VTODO) are out of scope: subscribed calendars are events-only
//! (`supports_tasks = 0`).

use sqlx::SqlitePool;
use url::Url;

use crate::SyncOutcome;

/// Upper bound for one feed fetch — the same 32 MiB `import.rs` allows for a
/// file import. A public feed bigger than that is not a calendar subscription
/// but a denial-of-service with a URL.
pub const MAX_FEED_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WebcalFeedError {
    /// Spelled as `CalDavError::Http` and `errors::SERVER_ANSWERED` are. Not on
    /// a user-facing path today — `user_facing_feed` builds its own sentence
    /// and the sync loop only counts failures — but four spellings of one
    /// sentence is how a later path picks the wrong one.
    #[error("The server answered {0}")]
    Http(reqwest::StatusCode),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Typed nothing. Allow-listed in `errors.rs` beside `NOT_PRIVATE_HTTP`,
/// for the same reason: it names what to do, and an opaque stand-in would
/// send the user looking for a fault instead of reading a decision.
pub const FEED_URL_REQUIRED: &str = "A calendar URL is required";

/// Typed something that is not an address at all.
pub const NOT_A_FEED_URL: &str =
    "That is not a calendar address. It should look like https://example.com/calendar.ics";

/// Normalizes a user-typed feed address: trims, rewrites `webcal://` (and
/// `webcals://`) to `https://`, and validates the scheme/host.
///
/// The returned string is what is stored (`accounts.server_url` and the
/// calendar's `google_id`) and what is fetched — one spelling, so
/// re-subscribing the same feed lands on the same rows.
pub fn normalize_feed_url(raw: &str) -> anyhow::Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!(FEED_URL_REQUIRED);
    }
    let https = if let Some(rest) = trimmed.strip_prefix("webcals://") {
        format!("https://{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("webcal://") {
        format!("https://{rest}")
    } else {
        trimmed.to_string()
    };
    // **A fixed sentence, not the parser's.** `url::ParseError` says things
    // like "relative URL without a base", which names nothing the user can
    // act on — and every variant of it would have to be allow-listed one by
    // one to reach them at all, or it reads as "Sync failed. See the
    // application log", naming an operation they did not ask for.
    let url = Url::parse(&https).map_err(|_| anyhow::anyhow!(NOT_A_FEED_URL))?;
    omacal_caldav::https_or_private(&url)?;
    Ok(url.to_string())
}

/// A display name when the user did not give one: the feed's host, which is
/// what distinguishes one subscription from another in the account list.
pub fn default_name_for(feed_url: &str) -> String {
    Url::parse(feed_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "Subscribed calendar".to_string())
}

/// FNV-1a over the body: stable across processes (unlike `DefaultHasher`,
/// whose SipHash keys are random per run), dependency-free, and good enough
/// for a change probe — collisions only skip a sync, never corrupt one.
fn content_hash(body: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in body.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("hash:{h:016x}")
}

/// The stored change probe, **tagged with which validator it is**.
///
/// An ETag and a `Last-Modified` date go back to the server in different
/// headers, and an untagged cursor cannot say which it is. Sending one as
/// both put an HTTP date in `If-None-Match`, where it is not a valid
/// entity-tag: RFC 9110 has a recipient ignore `If-Modified-Since` whenever
/// `If-None-Match` is present, so the server read the malformed tag, matched
/// nothing, and answered 200 with the whole file every time. A feed that
/// offers only `Last-Modified` therefore never produced a 304 — the branch
/// existed and could not fire.
fn cursor_for(etag: Option<&str>, last_modified: Option<&str>, body: &str) -> String {
    match (etag, last_modified) {
        (Some(etag), _) => format!("etag:{etag}"),
        (None, Some(lm)) => format!("lm:{lm}"),
        (None, None) => content_hash(body),
    }
}

/// The conditional header a stored cursor travels in, or `None` for a
/// `hash:` cursor — content-derived, never server-issued, so there is
/// nothing to validate against.
fn conditional_header(cursor: &str) -> Option<(&'static str, &str)> {
    if let Some(etag) = cursor.strip_prefix("etag:") {
        Some(("If-None-Match", etag))
    } else {
        cursor.strip_prefix("lm:").map(|lm| ("If-Modified-Since", lm))
    }
}

pub struct FetchedFeed {
    pub body: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

fn feed_client() -> Result<reqwest::Client, WebcalFeedError> {
    // Shared builder from `omacal-caldav`: no credentials travel, so feeds
    // may redirect cross-host; the cleartext rule stays identical.
    omacal_caldav::anonymous_feed_client()
        .map_err(WebcalFeedError::Other)
}

/// What a failed request becomes, with the address taken out of it.
///
/// **A redirect the cleartext rule refused says so** — `NOT_PRIVATE_HTTP`, the
/// sentence a typed `http://` address gets — found by walking the source chain
/// for `RedirectRefused` rather than trusting reqwest's own wording.
///
/// **Everything else loses its URL before it becomes an error value at all.**
/// A `reqwest::Error` displays as "error sending request for url
/// (https://…)", and for a subscription that address is frequently the
/// credential: a private calendar's "secret address in iCal format" grants
/// read access to anyone holding it. Stripping it here, at the one place feed
/// errors are made, means no log line and no message downstream can carry it —
/// rather than relying on every caller to remember not to print `%e`.
fn feed_transport_error(e: reqwest::Error) -> anyhow::Error {
    let refused = std::iter::successors(
        std::error::Error::source(&e),
        |err| err.source(),
    )
    .any(|err| err.is::<omacal_caldav::RedirectRefused>());
    if refused {
        anyhow::anyhow!(omacal_caldav::NOT_PRIVATE_HTTP)
    } else {
        anyhow::Error::from(e.without_url())
    }
}

fn too_large() -> WebcalFeedError {
    WebcalFeedError::Other(anyhow::anyhow!("the feed is larger than {MAX_FEED_BYTES} bytes"))
}

/// GETs the feed, sending the cached cursor as conditional headers.
/// `Ok(None)` is "not modified" (HTTP 304): nothing changed, sync nothing.
pub async fn fetch_feed(
    feed_url: &str,
    cached_token: Option<&str>,
) -> Result<Option<FetchedFeed>, WebcalFeedError> {
    let url = Url::parse(feed_url).map_err(anyhow::Error::from)?;
    omacal_caldav::https_or_private(&url).map_err(WebcalFeedError::Other)?;
    let mut req = feed_client()?
        .get(url)
        .header("Accept", "text/calendar");
    // One cursor, one header — see `cursor_for` for what sending both cost.
    if let Some((name, value)) = cached_token.and_then(conditional_header) {
        req = req.header(name, value);
    }
    let mut resp = req.send().await.map_err(feed_transport_error)?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(WebcalFeedError::Http(status));
    }
    // A declared length refuses before a byte of body is read; the loop below
    // is what holds when nothing was declared.
    if resp.content_length().is_some_and(|len| len > MAX_FEED_BYTES as u64) {
        return Err(too_large());
    }
    let etag = resp
        .headers()
        .get("ETag")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let last_modified = resp
        .headers()
        .get("Last-Modified")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    // **Counted as it arrives**, so a feed that never declared its length is
    // refused at the cap rather than after it has been held whole in memory.
    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(feed_transport_error)? {
        if bytes.len() + chunk.len() > MAX_FEED_BYTES {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Some(FetchedFeed {
        body: String::from_utf8_lossy(&bytes).into_owned(),
        etag,
        last_modified,
    }))
}

/// Syncs one subscribed feed into its calendar: conditional GET, full parse,
/// then a transactional upsert and whole-collection inferred deletion.
/// `feed_url` is the calendar row's `google_id`.
///
/// **No window, unlike CalDAV and Google.** Those two ask the server for a
/// slice and so must bound what they store and what they reap. A feed arrives
/// whole or not at all, so there is nothing to bound — and bounding it was a
/// bug: an event past the horizon was parsed, discarded, and then never seen
/// again, because the next sync answered 304 and never re-parsed the file.
pub async fn sync_webcal_calendar(
    pool: &SqlitePool,
    calendar_id: i64,
    feed_url: &str,
) -> Result<SyncOutcome, WebcalFeedError> {
    let stored: Option<String> =
        sqlx::query_scalar("SELECT sync_token FROM sync_state WHERE calendar_id = ?1")
            .bind(calendar_id)
            .fetch_optional(pool)
            .await
            .map_err(anyhow::Error::from)?
            .flatten();

    // HTTP 304: the file has not changed, and without a window there is
    // nothing else that could have — the rows are current, full stop.
    let Some(fetched) = fetch_feed(feed_url, stored.as_deref()).await? else {
        return Ok(SyncOutcome::default());
    };
    let cursor = cursor_for(fetched.etag.as_deref(), fetched.last_modified.as_deref(), &fetched.body);
    // Content-identical but validator-less (the server issues neither ETag
    // nor Last-Modified): the rows already match the file, so skip the churn.
    if stored.as_deref() == Some(cursor.as_str()) {
        return Ok(SyncOutcome::default());
    }

    let applied = apply_feed_body(pool, calendar_id, &fetched.body, &cursor)
        .await
        .map_err(WebcalFeedError::Other)?;
    Ok(applied)
}

/// Parses one feed body and reconciles it with the store. Split from
/// [`sync_webcal_calendar`] so tests drive the whole write path without HTTP.
pub async fn apply_feed_body(
    pool: &SqlitePool,
    calendar_id: i64,
    body: &str,
    cursor: &str,
) -> anyhow::Result<SyncOutcome> {
    let cal_tz: String = sqlx::query_scalar("SELECT timezone FROM calendars WHERE id = ?1")
        .bind(calendar_id)
        .fetch_one(pool)
        .await?;

    let root = omacal_caldav::parse(body).ok_or_else(|| anyhow::anyhow!("the URL did not return a calendar"))?;
    // No self mailbox exists on a subscription: every attendee match would be
    // a false RSVP strip, so nobody is marked.
    let mut rows = Vec::new();
    for ev in omacal_caldav::events_in(&root) {
        let Some(stored) = crate::caldav::caldav_to_stored(&ev, calendar_id, &cal_tz, None, "") else {
            tracing::warn!("VEVENT with unusable times; skipping");
            continue;
        };
        // **No window.** The whole file was fetched and parsed either way — the
        // window was a client-side discard of rows already in hand, and
        // discarding them is what made an event beyond the horizon invisible
        // for good once the feed stopped changing: the next sync answered 304
        // and never re-parsed. A feed costs the same to store whole.
        rows.push(stored);
    }
    let seen: Vec<String> = rows.iter().map(|r| r.google_id.clone()).collect();

    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let enabled: Option<i64> = sqlx::query_scalar("SELECT sync_enabled FROM calendars WHERE id = ?1")
        .bind(calendar_id)
        .fetch_optional(&mut *tx)
        .await?;
    if enabled != Some(1) {
        tx.rollback().await?;
        return Ok(SyncOutcome::default());
    }

    let mut outcome = SyncOutcome::default();
    for row in &rows {
        omacal_store::upsert_event(&mut *tx, row).await?;
        outcome.upserted += 1;
    }

    // **Deletions need no window here.** The CalDAV and Google paths bound
    // theirs because they only ever looked at a slice of the collection, and
    // may not delete what they did not examine. A feed is the whole truth on
    // every fetch, so anything on this calendar the file does not name is
    // gone: `Reach::Whole`.
    outcome.deleted += crate::reap::delete_unnamed(
        &mut tx,
        calendar_id,
        &seen,
        crate::reap::Reach::Whole,
    )
    .await? as usize;

    // The window columns belong to the providers that still have one.
    sqlx::query(
        "INSERT INTO sync_state (calendar_id, sync_token)
         VALUES (?1, ?2)
         ON CONFLICT (calendar_id) DO UPDATE SET sync_token = excluded.sync_token",
    )
    .bind(calendar_id)
    .bind(cursor)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const FEED: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\n\
        BEGIN:VEVENT\r\nUID:one\r\nDTSTART:20260817T090000Z\r\nDTEND:20260817T100000Z\r\nSUMMARY:Standup\r\nEND:VEVENT\r\n\
        BEGIN:VEVENT\r\nUID:two\r\nDTSTART:20260818T090000Z\r\nDTEND:20260818T100000Z\r\nSUMMARY:Retro\r\nEND:VEVENT\r\n\
        END:VCALENDAR\r\n";

    async fn seeded_pool() -> SqlitePool {
        let pool = omacal_store::connect_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (google_sub, email, created_at, provider, server_url) VALUES ('webcal:x','x',0,'webcal','x')")
            .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role, supports_events, supports_tasks)
             VALUES (1, 'x', 'Feed', 'UTC', 'reader', 1, 0)",
        ).execute(&pool).await.unwrap();
        pool
    }

    #[test]
    fn webcal_addresses_become_https_and_empty_is_refused() {
        assert_eq!(
            normalize_feed_url("webcal://example.com/cal.ics").unwrap(),
            "https://example.com/cal.ics"
        );
        assert_eq!(
            normalize_feed_url("webcals://example.com/c").unwrap(),
            "https://example.com/c"
        );
        assert!(normalize_feed_url("  ").is_err());
        assert!(normalize_feed_url("https://example.com/c").is_ok());
    }

    #[test]
    fn plain_http_follows_the_caldav_rule() {
        assert_eq!(
            normalize_feed_url("http://localhost:5232/x").unwrap(),
            "http://localhost:5232/x"
        );
        let err = normalize_feed_url("http://cal.example.com/x").unwrap_err().to_string();
        assert_eq!(err, omacal_caldav::NOT_PRIVATE_HTTP);
    }

    #[test]
    fn the_content_hash_is_stable_and_sensitive() {
        assert_eq!(content_hash("a"), content_hash("a"));
        assert_ne!(content_hash("a"), content_hash("b"));
    }

    /// **The cap holds while the body is arriving, not only once it has.** A
    /// server that declares no length skips the `Content-Length` check, and
    /// the old `resp.bytes()` buffered the whole body before the size was
    /// looked at — so an endless feed ran the app out of memory before it
    /// could be refused. wiremock always declares a length, hence a raw
    /// socket: 64 MiB, no `Content-Length`, closed when done. A capped read
    /// hangs up near the 32 MiB cap, so the server manages to write barely
    /// more than that; an uncapped one takes all 64.
    #[tokio::test]
    async fn an_undeclared_oversized_feed_is_refused_before_it_is_all_read() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut req = [0u8; 1024];
            let _ = sock.read(&mut req);
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/calendar\r\nConnection: close\r\n\r\n").unwrap();
            let chunk = vec![b'x'; 64 * 1024];
            let mut sent = 0usize;
            while sent < 64 * 1024 * 1024 {
                if sock.write_all(&chunk).is_err() { break; }
                sent += chunk.len();
            }
            sent
        });

        let Err(err) = fetch_feed(&format!("http://{addr}/feed.ics"), None).await else {
            panic!("an oversized feed was accepted");
        };
        assert!(err.to_string().contains("larger than"), "{err}");
        // Joined off the runtime's one thread: blocking it would stop hyper's
        // connection task from ever closing the socket the server writes to.
        let sent = tokio::task::spawn_blocking(move || server.join().unwrap()).await.unwrap();
        assert!(
            sent < 48 * 1024 * 1024,
            "the whole body was read before the cap was checked ({sent} bytes sent)"
        );
    }

    #[tokio::test]
    async fn a_feed_sync_stores_its_events_and_records_the_etag() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/feed.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(FEED).insert_header("ETag", "\"v1\""))
            .mount(&server).await;
        let pool = seeded_pool().await;
        let url = format!("{}/feed.ics", server.uri());
        // Point the seeded calendar at the mock feed.
        sqlx::query("UPDATE calendars SET google_id = ?1 WHERE id = 1").bind(&url)
            .execute(&pool).await.unwrap();

        let out = sync_webcal_calendar(&pool, 1, &url).await.unwrap();
        assert_eq!(out.upserted, 2);
        let tok: Option<String> = sqlx::query_scalar("SELECT sync_token FROM sync_state WHERE calendar_id = 1")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(tok.as_deref(), Some("etag:\"v1\""), "stored tagged, so the refetch knows which header to use");
    }

    #[tokio::test]
    async fn a_304_answer_syncs_nothing() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/feed.ics"))
            .and(header("If-None-Match", "\"v1\""))
            .respond_with(ResponseTemplate::new(304))
            .mount(&server).await;
        let pool = seeded_pool().await;
        let url = format!("{}/feed.ics", server.uri());
        sqlx::query("INSERT INTO sync_state (calendar_id, sync_token, window_start, window_end) VALUES (1, 'etag:\"v1\"', 0, 0)")
            .execute(&pool).await.unwrap();

        let out = sync_webcal_calendar(&pool, 1, &url).await.unwrap();
        assert_eq!(out, SyncOutcome::default());
        // Nothing else to record: without a window, an unchanged file means
        // unchanged rows, and the cursor it was probed with still stands.
        let tok: Option<String> =
            sqlx::query_scalar("SELECT sync_token FROM sync_state WHERE calendar_id = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(tok.as_deref(), Some("etag:\"v1\""));
    }

    /// The mirror of the test above for a feed that offers `Last-Modified`
    /// and no `ETag`. The date must travel in `If-Modified-Since` and
    /// nowhere else: as an `If-None-Match` it is not a valid entity-tag, the
    /// server matches nothing, and the whole file comes back on every sync.
    #[tokio::test]
    async fn a_last_modified_feed_sends_the_date_as_if_modified_since_and_gets_its_304() {
        const LM: &str = "Wed, 17 Sep 2026 07:00:00 GMT";
        let server = MockServer::start().await;
        // Matched on the path alone: an HTTP date is comma-bearing, which
        // wiremock's header matcher reads as a value list. What the request
        // carried is asserted below, off the recording.
        Mock::given(method("GET")).and(path("/feed.ics"))
            .respond_with(ResponseTemplate::new(304))
            .mount(&server).await;
        let pool = seeded_pool().await;
        let url = format!("{}/feed.ics", server.uri());
        sqlx::query("INSERT INTO sync_state (calendar_id, sync_token, window_start, window_end) VALUES (1, ?1, 0, 0)")
            .bind(format!("lm:{LM}"))
            .execute(&pool).await.unwrap();

        let out = sync_webcal_calendar(&pool, 1, &url).await.unwrap();
        assert_eq!(out, SyncOutcome::default(), "304 writes nothing");

        let sent = server.received_requests().await.unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].headers.get("If-Modified-Since").map(|v| v.to_str().unwrap()),
            Some(LM),
            "the date is a modification time, and travels as one",
        );
        assert!(
            sent[0].headers.get("If-None-Match").is_none(),
            "a date is not an entity-tag: as one it matches nothing and the feed refetches in full",
        );

    }

    /// **A redirect to cleartext on the open internet is refused**, and the
    /// hop is never sent. The feed client followed any redirect until
    /// 2026-09-23, so an `https://` feed could send the next request — and for
    /// a private calendar the address *is* the credential — in the clear.
    ///
    /// The mock is `http://127.0.0.1`, which the rule allows as loopback; the
    /// target is a public host, which it does not. The policy decides before
    /// the request leaves, so nothing reaches example.com.
    #[tokio::test]
    async fn a_redirect_to_public_cleartext_is_refused_before_it_is_sent() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/feed.ics"))
            .respond_with(ResponseTemplate::new(302)
                .insert_header("Location", "http://example.com/secret-address-4f9a.ics"))
            .mount(&server).await;

        let err = fetch_feed(&format!("{}/feed.ics", server.uri()), None)
            .await
            .err()
            .expect("a hop to public http must be refused");
        let shown = err.to_string();
        assert_eq!(shown, omacal_caldav::NOT_PRIVATE_HTTP, "says why, as a typed http:// does");
        assert!(!shown.contains("secret-address"), "the refused address leaked: {shown}");
    }

    /// The guard must not break the redirects feeds actually rely on.
    #[tokio::test]
    async fn a_redirect_the_rule_allows_is_still_followed() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/old.ics"))
            .respond_with(ResponseTemplate::new(301)
                .insert_header("Location", format!("{}/feed.ics", server.uri())))
            .mount(&server).await;
        Mock::given(method("GET")).and(path("/feed.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(FEED))
            .mount(&server).await;

        let got = fetch_feed(&format!("{}/old.ics", server.uri()), None)
            .await
            .unwrap()
            .expect("a 200 at the end of the redirect");
        assert!(got.body.contains("Standup"));
    }

    /// **No feed address inside a transport error.** A `reqwest::Error` prints
    /// "error sending request for url (…)", and the sync loop logs `%e` beside
    /// a failure the user is told to look up in the log.
    #[tokio::test]
    async fn a_transport_error_carries_no_part_of_the_address() {
        // Port 1 on loopback: allowed by the cleartext rule, refused by the
        // kernel, so the request fails at the transport.
        let err = fetch_feed("http://127.0.0.1:1/secret-address-77b2.ics", None)
            .await
            .err()
            .expect("nothing listens on port 1");
        let shown = format!("{err} / {err:?}");
        assert!(!shown.contains("secret-address"), "the address reached the error: {shown}");
    }

    /// Which header a cursor travels in, and that a content hash travels in
    /// none — the rule `fetch_feed` reads.
    #[test]
    fn a_cursor_names_its_own_validator() {
        assert_eq!(cursor_for(Some("\"v1\""), Some("ignored"), ""), "etag:\"v1\"");
        assert_eq!(cursor_for(None, Some("a date"), ""), "lm:a date");
        assert!(cursor_for(None, None, "body").starts_with("hash:"));
        assert_eq!(conditional_header("etag:\"v1\""), Some(("If-None-Match", "\"v1\"")));
        assert_eq!(conditional_header("lm:a date"), Some(("If-Modified-Since", "a date")));
        assert_eq!(conditional_header("hash:0123"), None);
    }

    #[tokio::test]
    async fn a_validator_less_refetch_with_identical_content_skips_the_writes() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/feed.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(FEED))
            .mount(&server).await;
        let pool = seeded_pool().await;
        let url = format!("{}/feed.ics", server.uri());
        sqlx::query("UPDATE calendars SET google_id = ?1 WHERE id = 1").bind(&url)
            .execute(&pool).await.unwrap();
        let cursor = cursor_for(None, None, FEED);
        sqlx::query("INSERT INTO sync_state (calendar_id, sync_token, window_start, window_end) VALUES (1, ?1, 0, 0)")
            .bind(&cursor)
            .execute(&pool).await.unwrap();

        let out = sync_webcal_calendar(&pool, 1, &url).await.unwrap();
        assert_eq!(out, SyncOutcome::default(), "no write churn on identical content");
        let tok: Option<String> =
            sqlx::query_scalar("SELECT sync_token FROM sync_state WHERE calendar_id = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(tok.as_deref(), Some(cursor.as_str()));
    }

    /// A feed names its collection in full, so anything it does not name is
    /// gone — including rows a window would once have shielded. That shielding
    /// was the bug: the file is the whole truth on every fetch.
    #[tokio::test]
    async fn an_event_missing_from_the_refetch_is_deleted_however_far_out() {
        let pool = seeded_pool().await;
        // Two ghosts the feed does not name: one recent, one far in the past
        // that the old window-bounded delete would have left behind for ever.
        for (gid, start, end) in [("ghost", 3_000_000, 3_100_000), ("relic", 100, 200)] {
            sqlx::query(
                "INSERT INTO events (calendar_id, google_id, start_utc, end_utc, start_tz, end_tz, status, updated_at)
                 VALUES (1, ?1, ?2, ?3, 'UTC', 'UTC', 'confirmed', 0)",
            ).bind(gid).bind(start).bind(end).execute(&pool).await.unwrap();
        }
        let one = FEED.replace("UID:two", "UID:kept")
            .replace("20260818", "20260819");
        // Feed holds one event overlapping the window; ghost is absent.
        let out = apply_feed_body(&pool, 1, &one, "hash:x")
            .await.unwrap();
        assert_eq!(out.deleted, 2, "both ghosts go; the feed named neither");
        let left: Vec<String> =
            sqlx::query_scalar("SELECT google_id FROM events WHERE calendar_id = 1 ORDER BY google_id")
                .fetch_all(&pool).await.unwrap();
        assert!(!left.contains(&"relic".to_string()));
        assert!(!left.contains(&"ghost".to_string()));
        assert!(left.iter().any(|g| g == "one"), "what the feed names stays");
    }

    /// **The bug this replaced.** Every event in the file is stored, however
    /// far from today it sits. The window used to discard them after parsing,
    /// so an event beyond the horizon was invisible until the feed changed
    /// again — and a holidays feed may not change for a year.
    #[tokio::test]
    async fn every_event_in_the_feed_is_stored_however_far_out() {
        let pool = seeded_pool().await;
        let far = FEED
            .replace("20260817", "20991231")
            .replace("20260818", "20991231");
        let out = apply_feed_body(&pool, 1, &far, "hash:y").await.unwrap();
        assert_eq!(out.upserted, 2, "a 2099 event is still the feed's to tell us about");
        let stored: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE calendar_id = 1")
                .fetch_one(&pool).await.unwrap();
        assert_eq!(stored, 2);
    }
}
