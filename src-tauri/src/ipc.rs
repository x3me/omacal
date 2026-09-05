//! The write half of the CLI, server side (CLI-writes spec, 2026-08-27).
//!
//! A Unix socket the running app owns. One JSON request per connection, one
//! envelope reply — the same `{"ok":…}` shape `cli.rs` prints, so the CLI
//! relays bytes rather than translating them. Every write dispatches into
//! the same `*_body` functions the webview's commands call (`events.rs`),
//! which is the spec's §6 made structural: guards, sanitizer and widget
//! refresh included, one code path from any surface to any provider.
//!
//! Split as ever: [`parse_request`], [`default_calendar`]'s rule and the
//! envelope builders are pure or pool-only and tested; the accept loop and
//! the byte shuffling are the thin untested shell.
//!
//! **Same-user only, by filesystem permission.** The socket sits `0600` in a
//! `0700` directory — the same trust boundary the database file already
//! has. No network listener, ever.

use serde::Deserialize;
use tauri::Manager;

use crate::AppState;

/// Bumped when a request's meaning changes, never for additions — an old
/// CLI against a newer app must degrade into a legible refusal, not a
/// misread write.
pub(crate) const PROTOCOL_VERSION: u64 = 1;

/// Requests are one line; anything past this is refused unread. Nothing an
/// event can carry comes near it — the cap is against garbage, not use.
pub(crate) const MAX_REQUEST_BYTES: u64 = 64 * 1024;

/// Where the socket lives. **`cli.rs` connects through this same function**,
/// so the two ends cannot disagree about the address: the runtime dir where
/// the platform has one (Linux, Flatpak included — there it is app-private),
/// else beside the database, the one directory the app already owns.
pub(crate) fn socket_path() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        let dir = std::path::Path::new(&dir);
        if dir.is_absolute() {
            return Some(dir.join("omacal").join("ipc.sock"));
        }
    }
    Some(crate::cli::db_path()?.parent()?.join("ipc.sock"))
}

/// The wire vocabulary. `fields` is `write.rs`'s own [`crate::write::EventInput`]
/// — one vocabulary, zero translation layers, so a field added there is a
/// field this surface can carry the day it exists.
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd")]
pub(crate) enum Request {
    #[serde(rename = "events-create")]
    Create {
        /// Absent means the app's default-calendar rule ([`default_calendar`]).
        #[serde(default)]
        calendar_id: Option<i64>,
        fields: crate::write::EventInput,
        send_updates: String,
    },
    #[serde(rename = "events-update")]
    Update {
        id: i64,
        scope: String,
        occurrence_start_ms: i64,
        fields: crate::write::EventInput,
        send_updates: String,
    },
    #[serde(rename = "events-delete")]
    Delete { id: i64, scope: String, occurrence_start_ms: i64 },
    #[serde(rename = "events-respond")]
    Respond { id: i64, response: String, scope: String, occurrence_start_ms: i64 },
}

/// One request line to a [`Request`], or the usage message to refuse it
/// with. The version gate comes first and alone: a `v` this build does not
/// speak refuses *before* the shape is even looked at, so "unsupported
/// version" is never misreported as "bad request".
pub(crate) fn parse_request(line: &str) -> Result<Request, String> {
    let val: serde_json::Value =
        serde_json::from_str(line).map_err(|_| "the request is not JSON".to_string())?;
    match val.get("v").and_then(serde_json::Value::as_u64) {
        Some(PROTOCOL_VERSION) => {}
        Some(other) => {
            return Err(format!(
                "protocol v{other} is not spoken here — this OmaCal answers v{PROTOCOL_VERSION}"
            ));
        }
        None => return Err("the request names no protocol version".to_string()),
    }
    serde_json::from_value(val).map_err(|e| format!("bad request: {e}"))
}

/// The app's default-calendar rule, server-side: the stored choice when it
/// is still writable, else the primary, else the first writable calendar —
/// `offerableCalendarId`'s rule (calendars.ts), asked of the database
/// because the CLI has no form to seed. `None` means nothing is writable,
/// which is a refusal the caller words.
pub(crate) async fn default_calendar(pool: &sqlx::SqlitePool) -> Option<i64> {
    let stored = crate::settings::read_settings(pool).await.default_calendar_id;
    sqlx::query_scalar(
        "SELECT id FROM calendars WHERE access_role IN ('owner','writer')
         ORDER BY (id = ?1) DESC, is_primary DESC, id ASC LIMIT 1",
    )
    .bind(stored)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

fn ok_env(data: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "ok": true, "data": data })
}

/// `code` is `cli.rs`'s own error vocabulary. `refused` covers everything
/// the app itself answered — a guard, a conflict, a provider that said no —
/// already sanitized by the bodies (`errors::user_facing` is the single
/// gate); `usage` is a request this build could not even read.
pub(crate) fn fail_env(code: &str, message: &str) -> serde_json::Value {
    serde_json::json!({ "ok": false, "error": { "code": code, "message": message } })
}

/// One request to one envelope, through the same bodies the webview's
/// commands call.
pub(crate) async fn dispatch(state: &AppState, req: Request) -> serde_json::Value {
    match req {
        Request::Create { calendar_id, fields, send_updates } => {
            let calendar_id = match calendar_id {
                Some(id) => id,
                None => match default_calendar(&state.pool).await {
                    Some(id) => id,
                    None => {
                        return fail_env(
                            "refused",
                            "no writable calendar — connect an account in OmaCal first",
                        );
                    }
                },
            };
            match crate::events::create_event_body(state, calendar_id, fields, &send_updates)
                .await
            {
                Ok(detail) => ok_env(serde_json::json!(detail)),
                Err(m) => fail_env("refused", &m),
            }
        }
        Request::Update { id, scope, occurrence_start_ms, fields, send_updates } => {
            // `None`: the CLI has no verb for moving an event between
            // calendars, so its update always means "leave it where it is".
            // A socket request that could re-file an event would have to say
            // which calendar by name, and that is a CLI surface change with a
            // skill file to match — not something to smuggle in behind an
            // optional argument.
            match crate::events::update_event_body(
                state, id, &scope, occurrence_start_ms, fields, &send_updates, None,
            )
            .await
            {
                Ok(detail) => ok_env(serde_json::json!(detail)),
                Err(m) => fail_env("refused", &m),
            }
        }
        Request::Delete { id, scope, occurrence_start_ms } => {
            match crate::events::delete_event_body(state, id, &scope, occurrence_start_ms).await {
                Ok(()) => ok_env(serde_json::json!({ "deleted": true })),
                Err(m) => fail_env("refused", &m),
            }
        }
        Request::Respond { id, response, scope, occurrence_start_ms } => {
            match crate::events::respond_event_body(
                state, id, &response, &scope, occurrence_start_ms,
            )
            .await
            {
                Ok(detail) => ok_env(serde_json::json!(detail)),
                Err(m) => fail_env("refused", &m),
            }
        }
    }
}

/// Binds the socket and serves until the process ends. Called from setup;
/// failures are logged and give up quietly — an app that refuses to start
/// because a socket could not bind would be trading the calendar for the
/// CLI, which is the wrong trade in both directions.
#[cfg(unix)]
pub(crate) fn spawn(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let Some(path) = socket_path() else {
            tracing::warn!("no socket path could be derived; CLI writes stay off");
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            }
        }
        // A hard_restart runs no cleanups by design, so a stale socket from
        // the previous instance is the ordinary case, not a conflict: the
        // single-instance plugin has already guaranteed nobody is listening.
        let _ = std::fs::remove_file(&path);
        let listener = match tokio::net::UnixListener::bind(&path) {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(%e, "could not bind the CLI socket; CLI writes stay off");
                return;
            }
        };
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        tracing::info!(path = %path.display(), "CLI write socket up");

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = serve_one(app, stream).await {
                            tracing::debug!(%e, "a CLI connection ended early");
                        }
                    });
                }
                Err(e) => tracing::debug!(%e, "accept failed"),
            }
        }
    });
}

/// One request line to one envelope — the whole of the protocol above the
/// bytes, `serve_one` minus the I/O and the emit, so the e2e test can run
/// it behind a real socket without a Tauri app to hang state on.
pub(crate) async fn answer(state: &AppState, line: &str) -> serde_json::Value {
    match parse_request(line) {
        Ok(req) => dispatch(state, req).await,
        Err(m) => fail_env("usage", &m),
    }
}

/// One connection: read the line, answer the envelope, done. A successful
/// write also announces itself as `sync-finished` — the same "the store
/// changed underneath you" signal the sync loop sends, so the grid a user
/// is looking at repaints while the agent works, through the reload path
/// the webview already has.
#[cfg(unix)]
async fn serve_one(app: tauri::AppHandle, stream: tokio::net::UnixStream) -> std::io::Result<()> {
    use tauri::Emitter;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    let (read, mut write) = stream.into_split();
    let mut line = String::new();
    // The cap is on the reader itself: a request that exceeds it simply has
    // no newline inside the window we are willing to read, and answers as
    // unparseable rather than being buffered to whatever size it likes.
    BufReader::new(read).take(MAX_REQUEST_BYTES).read_line(&mut line).await?;

    let state = app.state::<AppState>();
    let reply = answer(&state, &line).await;
    if reply.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        let _ = app.emit("sync-finished", serde_json::json!({ "upserted": 1 }));
    }

    write.write_all(reply.to_string().as_bytes()).await?;
    write.write_all(b"\n").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The version gate answers before the shape is looked at, and both
    /// halves of "wrong" get their own words.
    #[test]
    fn the_version_gate_comes_first_and_alone() {
        assert!(parse_request("not json").unwrap_err().contains("not JSON"));
        assert!(parse_request(r#"{"cmd":"events-delete"}"#)
            .unwrap_err()
            .contains("names no protocol version"));
        assert!(parse_request(r#"{"v":2,"cmd":"events-delete"}"#)
            .unwrap_err()
            .contains("v2 is not spoken here"));
        // A good version with a bad shape is a *bad request*, not a version
        // complaint.
        assert!(parse_request(r#"{"v":1,"cmd":"events-delete"}"#)
            .unwrap_err()
            .contains("bad request"));
        assert!(parse_request(r#"{"v":1,"cmd":"no-such-thing"}"#)
            .unwrap_err()
            .contains("bad request"));
    }

    /// The wire vocabulary is `EventInput`'s own serde — the exact payload
    /// the webview sends deserializes here unchanged, which is what keeps
    /// the two surfaces from forking.
    #[test]
    fn a_request_carries_the_webviews_own_event_vocabulary() {
        let req = parse_request(
            r#"{"v":1,"cmd":"events-create","send_updates":"none","fields":{
                "summary":"Standup","location":null,"description":null,
                "when":{"kind":"timed","startMs":1786352400000,"endMs":1786356000000},
                "tz":"Europe/Sofia"}}"#,
        )
        .unwrap();
        match req {
            Request::Create { calendar_id, fields, send_updates } => {
                assert_eq!(calendar_id, None, "absent means the default rule");
                assert_eq!(send_updates, "none");
                assert_eq!(fields.summary.as_deref(), Some("Standup"));
                assert!(matches!(
                    fields.when,
                    crate::write::WhenInput::Timed { start_ms: 1_786_352_400_000, .. }
                ));
            }
            other => panic!("parsed as {other:?}"),
        }
    }

    /// All-day rides the same internally-tagged `when` the UI sends — and a
    /// payload naming neither shape refuses to parse rather than becoming a
    /// timed event at the epoch (write.rs's own doctrine, held here too).
    #[test]
    fn all_day_and_malformed_when_behave_as_the_forms_would() {
        let all_day = parse_request(
            r#"{"v":1,"cmd":"events-create","send_updates":"none","fields":{
                "summary":"Trip","when":{"kind":"allDay","startDate":"2026-09-01","endDate":"2026-09-03"},
                "tz":"Europe/Sofia"}}"#,
        );
        assert!(all_day.is_ok(), "{all_day:?}");

        let neither = parse_request(
            r#"{"v":1,"cmd":"events-create","send_updates":"none","fields":{
                "summary":"?","when":{},"tz":"UTC"}}"#,
        );
        assert!(neither.is_err());
    }

    /// The envelope speaks `cli.rs`'s own error vocabulary — `code` +
    /// `message` — so the CLI can relay it without translation.
    #[test]
    fn the_envelope_is_the_clis_own() {
        let e = fail_env("refused", "no");
        assert_eq!(e["ok"], false);
        assert_eq!(e["error"]["code"], "refused");
        let o = ok_env(serde_json::json!({ "deleted": true }));
        assert_eq!(o["ok"], true);
        assert_eq!(o["data"]["deleted"], true);
    }

    /// The spec's one end-to-end (§8): a real socket in a temp dir, the
    /// real `answer` path behind it, driven by the byte-for-byte requests
    /// `cli_write.rs` builds. Demo mode, deliberately: the demo gate is the
    /// first statement of every body, so the round trip proves the whole
    /// wire — parse, dispatch, guard, sanitized refusal, envelope — while
    /// touching no network and no calendar. And the refusal must arrive as
    /// `refused`, exit 6's code: the app answered, nothing broke.
    #[tokio::test]
    async fn a_write_travels_the_socket_and_the_demo_gate_answers_it() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let pool = omacal_store::connect_memory().await.unwrap();
        let state = AppState {
            pool,
            demo: true,
            tokens: Default::default(),
            reauth: Default::default(),
            update: Default::default(),
            update_checked_at: Default::default(),
            system_tz_change: Default::default(),
            quit_on_close: Default::default(),
            open_date: Default::default(),
        };

        let dir = std::env::temp_dir().join(format!("omacal-ipc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ipc.sock");
        let _ = std::fs::remove_file(&path);
        let listener = tokio::net::UnixListener::bind(&path).unwrap();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut line = String::new();
            BufReader::new(read).read_line(&mut line).await.unwrap();
            let reply = answer(&state, &line).await;
            write.write_all(reply.to_string().as_bytes()).await.unwrap();
            write.write_all(b"\n").await.unwrap();
        });

        // The client's own request builder — the same bytes `omacal events
        // create` would send.
        let request = crate::cli_write::create_request(
            &crate::cli_write::CreateArgs {
                title: "Standup".into(),
                date: "2026-09-01".into(),
                start: Some("09:00".into()),
                end: Some("09:30".into()),
                // Named, not defaulted: the default-calendar rule runs
                // before the demo gate, and an empty fixture pool would
                // answer "no writable calendar" — a true refusal, but not
                // the one this test is about.
                calendar: Some(1),
                ..Default::default()
            },
            serde_json::json!({ "kind": "timed", "startMs": 1_788_242_400_000i64, "endMs": 1_788_244_200_000i64 }),
            "none",
            "UTC",
        );

        let mut stream = tokio::net::UnixStream::connect(&path).await.unwrap();
        stream.write_all(request.to_string().as_bytes()).await.unwrap();
        stream.write_all(b"\n").await.unwrap();
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).await.unwrap();
        server.await.unwrap();
        let _ = std::fs::remove_file(&path);

        let envelope: serde_json::Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(envelope["ok"], false);
        assert_eq!(envelope["error"]["code"], "refused", "the app answered; nothing broke");
        assert!(
            envelope["error"]["message"].as_str().unwrap_or("").contains("demo"),
            "the demo gate's own words crossed the wire: {envelope}"
        );
    }
}
