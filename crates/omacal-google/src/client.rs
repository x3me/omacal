use crate::model;
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// HTTP 410 — the stored sync token is stale. The caller must discard it
    /// and perform a full resync (spec §5).
    #[error("sync token is no longer valid")]
    SyncTokenInvalid,
    #[error("http error: {0}")]
    Http(String),
    #[error("transport error: {0}")]
    Transport(String),
    /// The provider accepted a write, but its success response could not be
    /// decoded. Callers must not retry blindly or compensate by deleting a
    /// related resource: the calendar mutation may already be durable.
    #[error("write committed but its response could not be read: {0}")]
    WriteCommitted(String),
    /// HTTP 412 — the caller's `If-Match` etag no longer matches; the event
    /// changed server-side since it was fetched.
    #[error("the event changed while you were editing it")]
    PreconditionFailed,
}

#[derive(Debug, Clone, Default)]
pub struct EventsRequest {
    pub sync_token: Option<String>,
    pub time_min: Option<String>,
    pub time_max: Option<String>,
    pub page_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EventsPage {
    pub events: Vec<model::Event>,
    pub next_page_token: Option<String>,
    pub next_sync_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventsResponse {
    #[serde(default)]
    items: Vec<model::Event>,
    next_page_token: Option<String>,
    next_sync_token: Option<String>,
}

#[derive(Deserialize)]
struct CalendarListResponse {
    #[serde(default)]
    items: Vec<model::Calendar>,
}

#[derive(Deserialize)]
struct EventInstancesResponse {
    #[serde(default)]
    items: Vec<model::Event>,
}

pub struct CalendarClient {
    base_url: String,
    access_token: String,
    http: reqwest::Client,
}

impl CalendarClient {
    /// `base_url` is `https://www.googleapis.com/calendar/v3` in production and
    /// a `wiremock` URI in tests.
    pub fn new(base_url: impl Into<String>, access_token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            access_token: access_token.into(),
            http: reqwest::Client::new(),
        }
    }

    pub async fn list_calendars(&self) -> anyhow::Result<Vec<model::Calendar>> {
        let resp = self
            .http
            .get(format!("{}/users/me/calendarList", self.base_url))
            .bearer_auth(&self.access_token)
            .send()
            .await?
            .error_for_status()?
            .json::<CalendarListResponse>()
            .await?;
        Ok(resp.items)
    }

    /// One page of events.
    ///
    /// `singleEvents=false` is deliberate (spec §5): we store recurring masters
    /// and expand locally. Every parameter here must stay byte-identical across
    /// incremental calls or Google invalidates the sync token.
    pub async fn list_events(
        &self,
        calendar_id: &str,
        req: &EventsRequest,
    ) -> Result<EventsPage, ApiError> {
        let mut params: Vec<(&str, String)> = vec![
            ("singleEvents", "false".into()),
            ("showDeleted", "true".into()),
            ("maxResults", "2500".into()),
        ];
        // timeMin/timeMax are illegal alongside a syncToken.
        if let Some(t) = &req.sync_token {
            params.push(("syncToken", t.clone()));
        } else {
            if let Some(t) = &req.time_min {
                params.push(("timeMin", t.clone()));
            }
            if let Some(t) = &req.time_max {
                params.push(("timeMax", t.clone()));
            }
        }
        if let Some(t) = &req.page_token {
            params.push(("pageToken", t.clone()));
        }

        let resp = self
            .http
            .get(format!(
                "{}/calendars/{}/events",
                self.base_url,
                urlencoding_path(calendar_id)
            ))
            .bearer_auth(&self.access_token)
            .query(&params)
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::GONE {
            return Err(ApiError::SyncTokenInvalid);
        }
        if !resp.status().is_success() {
            return Err(ApiError::Http(format!("{}", resp.status())));
        }

        let body: EventsResponse = resp
            .json()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;

        Ok(EventsPage {
            events: body.items,
            next_page_token: body.next_page_token,
            next_sync_token: body.next_sync_token,
        })
    }

    /// Fetch a single event by id.
    pub async fn get_event(&self, cal: &str, event_id: &str) -> Result<model::Event, ApiError> {
        let resp = self
            .http
            .get(format!(
                "{}/calendars/{}/events/{}",
                self.base_url,
                urlencoding_path(cal),
                urlencoding_path(event_id)
            ))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(ApiError::Http(format!("{}", resp.status())));
        }

        resp.json::<model::Event>()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))
    }

    /// Patch (partially update) an event.
    ///
    /// `send_updates` is Google's own vocabulary — `"all"`, `"externalOnly"` or
    /// `"none"` — and a parameter rather than a constant, the same shape as
    /// [`Self::insert_event`], because the callers want different answers.
    ///
    /// **The reasoning that made it `"all"` still holds, for the caller it was
    /// written about.** It read: *without it Google silently applies the change
    /// and nobody is told.* That is right for the **form** — a new time was
    /// typed on purpose and Save was pressed — but since the guest-list work,
    /// that rightness is honoured differently there: the form now asks and
    /// carries whatever answer it gets, the same choice guest-list spec §3
    /// gives a create. An RSVP has no dialog behind it and still passes
    /// `"all"` unconditionally, exactly as it always has.
    ///
    /// It is wrong for a **gesture**. A drag can happen by accident, and
    /// mailing a meeting's whole guest list is not something a slip of the
    /// mouse should do; spec §2 of the drag design is the ruling, and a drop
    /// chooses `"none"` unless the user is asked and says otherwise. Hardcoding
    /// `"all"` here is what would make that impossible to express — which is
    /// why the choice is the caller's, and why the default was considered
    /// rather than defaulted into.
    ///
    /// `etag`, when given, is sent as `If-Match` so a change made elsewhere
    /// since the caller last fetched the event surfaces as
    /// [`ApiError::PreconditionFailed`] instead of being clobbered.
    pub async fn patch_event(
        &self,
        cal: &str,
        event_id: &str,
        body: &serde_json::Value,
        send_updates: &str,
        etag: Option<&str>,
    ) -> Result<model::Event, ApiError> {
        let mut req = self
            .http
            .patch(format!(
                "{}/calendars/{}/events/{}",
                self.base_url,
                urlencoding_path(cal),
                urlencoding_path(event_id)
            ))
            .bearer_auth(&self.access_token)
            // Version 1 is required both to create conferenceData and to
            // preserve it correctly on later event modifications. It is sent
            // on every write once this client supports conferencing, as Google
            // recommends for an application that stores fully-synced events.
            .query(&[("sendUpdates", send_updates), ("conferenceDataVersion", "1")])
            .json(body);
        if let Some(etag) = etag {
            req = req.header("If-Match", etag);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::PRECONDITION_FAILED {
            return Err(ApiError::PreconditionFailed);
        }
        if !resp.status().is_success() {
            return Err(ApiError::Http(format!("{}", resp.status())));
        }

        resp.json::<model::Event>()
            .await
            .map_err(|e| ApiError::WriteCommitted(e.to_string()))
    }

    /// Expand a recurring event's instances within `[time_min, time_max)`.
    pub async fn event_instances(
        &self,
        cal: &str,
        event_id: &str,
        time_min: &str,
        time_max: &str,
    ) -> Result<Vec<model::Event>, ApiError> {
        let resp = self
            .http
            .get(format!(
                "{}/calendars/{}/events/{}/instances",
                self.base_url,
                urlencoding_path(cal),
                urlencoding_path(event_id)
            ))
            .bearer_auth(&self.access_token)
            .query(&[("timeMin", time_min), ("timeMax", time_max)])
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(ApiError::Http(format!("{}", resp.status())));
        }

        let body: EventInstancesResponse = resp
            .json()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        Ok(body.items)
    }

    /// Create an event. `send_updates` is Google's own vocabulary — `"all"`,
    /// `"externalOnly"` or `"none"` — and is a parameter rather than a
    /// constant.
    ///
    /// The new-event form's create carries whatever guest list the user typed
    /// and whatever `sendUpdates` answer they gave — the parameter exists
    /// because this app's two creates want different answers. The form's
    /// create asks the user; splitting a recurring series with "this and
    /// following" is the other case, and it is not the same one — it passes
    /// `"all"` because the event it creates carries the whole guest list of
    /// the series it continues. See `events::split_series` for why that one
    /// passes `"all"`.
    ///
    /// Hardcoding `"none"` here, as this method did when a create could only
    /// ever be guestless, is what makes that distinction invisible: a split
    /// would move every guest's meeting and tell none of them.
    pub async fn insert_event(
        &self,
        cal: &str,
        body: &serde_json::Value,
        send_updates: &str,
    ) -> Result<model::Event, ApiError> {
        let resp = self
            .http
            .post(format!(
                "{}/calendars/{}/events",
                self.base_url,
                urlencoding_path(cal)
            ))
            .bearer_auth(&self.access_token)
            .query(&[("sendUpdates", send_updates), ("conferenceDataVersion", "1")])
            .json(body)
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(ApiError::Http(format!("{}", resp.status())));
        }

        resp.json::<model::Event>()
            .await
            .map_err(|e| ApiError::WriteCommitted(e.to_string()))
    }

    /// Moves an event to another calendar **on the same account**, and returns
    /// it as it now is.
    ///
    /// Google's own verb, not a create-and-delete: the event keeps its id, its
    /// guest list, and every RSVP already answered on it. A copy would keep
    /// none of those — it would mail the whole room a fresh invitation to an
    /// event they had already accepted, which is exactly the outcome that made
    /// "delete it and make it again" an unacceptable workaround.
    ///
    /// **The whole series moves, or nothing does.** The endpoint takes an
    /// event id, and for a recurring event that means the master: there is no
    /// request that moves one occurrence and leaves its siblings behind. The
    /// caller refuses that scope before reaching here (`events::move_guard`)
    /// rather than moving more than the user asked for.
    ///
    /// `destination` is the target calendar's Google id. Cross-*account* moves
    /// are not this endpoint's business and are refused before it is called:
    /// the destination has to be visible to the same credentials.
    pub async fn move_event(
        &self,
        cal: &str,
        event_id: &str,
        destination: &str,
        send_updates: &str,
    ) -> Result<model::Event, ApiError> {
        let resp = self
            .http
            .post(format!(
                "{}/calendars/{}/events/{}/move",
                self.base_url,
                urlencoding_path(cal),
                urlencoding_path(event_id)
            ))
            .bearer_auth(&self.access_token)
            .query(&[("destination", destination), ("sendUpdates", send_updates)])
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::PRECONDITION_FAILED {
            return Err(ApiError::PreconditionFailed);
        }
        if !resp.status().is_success() {
            return Err(ApiError::Http(format!("{}", resp.status())));
        }

        resp.json::<model::Event>()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))
    }

    /// Delete an event. `sendUpdates=all` so a cancelled meeting reaches the
    /// guest list — a meeting that vanishes for the organiser only is worse
    /// than an email.
    ///
    /// `404` is success: the event is already gone, which is what the caller
    /// asked for. Returning an error there would make a double-click, or a
    /// retry after a dropped response, look like a failure.
    pub async fn delete_event(
        &self,
        cal: &str,
        event_id: &str,
        etag: Option<&str>,
    ) -> Result<(), ApiError> {
        let mut req = self
            .http
            .delete(format!(
                "{}/calendars/{}/events/{}",
                self.base_url,
                urlencoding_path(cal),
                urlencoding_path(event_id)
            ))
            .bearer_auth(&self.access_token)
            .query(&[("sendUpdates", "all")]);
        if let Some(etag) = etag {
            req = req.header("If-Match", etag);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::PRECONDITION_FAILED {
            return Err(ApiError::PreconditionFailed);
        }
        // 410 alongside 404, and 410 is the one Google actually answers for
        // an event that was deleted by somebody else: 404 means "no such
        // resource", 410 means "there was, and it is gone" — which is still
        // exactly what this caller asked for. Learned from a live ghost
        // (2026-08-23): an event deleted upstream during a stale-token gap
        // could not be deleted locally either, because its Google-side
        // delete answered 410 and this arm was missing.
        if resp.status() == reqwest::StatusCode::NOT_FOUND
            || resp.status() == reqwest::StatusCode::GONE
        {
            return Ok(());
        }
        if !resp.status().is_success() {
            return Err(ApiError::Http(format!("{}", resp.status())));
        }
        Ok(())
    }
}

/// Calendar ids are email-like and must be percent-encoded in the path.
///
/// `form_urlencoded` applies the *query string* rules, where a space becomes
/// `+`; in a path it would have to be `%20`. Nothing this builds a path from
/// can carry a space — calendar ids are email addresses, and event ids are
/// Google's own base32hex — so the difference has never been reachable. A
/// caller that could pass one would need a path-specific encoder rather than
/// this.
fn urlencoding_path(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json_string, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn list_calendars_parses_the_payload() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/me/calendarList"))
            .and(header("authorization", "Bearer at-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{
                    "id": "primary", "summary": "Work", "backgroundColor": "#5b8def",
                    "timeZone": "Europe/Sofia", "accessRole": "owner", "primary": true
                }]
            })))
            .mount(&server).await;

        let c = CalendarClient::new(server.uri(), "at-1");
        let cals = c.list_calendars().await.unwrap();
        assert_eq!(cals.len(), 1);
        assert_eq!(cals[0].id, "primary");
        assert_eq!(cals[0].time_zone.as_deref(), Some("Europe/Sofia"));
        assert!(cals[0].primary);
    }

    #[tokio::test]
    async fn a_full_sync_sends_single_events_false_and_returns_a_sync_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .and(query_param("singleEvents", "false"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{
                    "id": "e1", "status": "confirmed", "summary": "Standup",
                    "start": {"dateTime": "2026-08-03T09:00:00+03:00", "timeZone": "Europe/Sofia"},
                    "end":   {"dateTime": "2026-08-03T09:30:00+03:00", "timeZone": "Europe/Sofia"},
                    "recurrence": ["RRULE:FREQ=DAILY"]
                }],
                "nextSyncToken": "tok-1"
            })))
            .mount(&server).await;

        let c = CalendarClient::new(server.uri(), "at-1");
        let page = c.list_events("primary", &EventsRequest::default()).await.unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.next_sync_token.as_deref(), Some("tok-1"));
        assert_eq!(page.events[0].recurrence.as_ref().unwrap()[0], "RRULE:FREQ=DAILY");
    }

    #[tokio::test]
    async fn an_all_day_event_parses_its_date_form() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{
                    "id": "e2", "status": "confirmed", "summary": "Sofia trip",
                    "start": {"date": "2026-08-08"},
                    "end":   {"date": "2026-08-17"}
                }],
                "nextSyncToken": "tok-2"
            })))
            .mount(&server).await;

        let c = CalendarClient::new(server.uri(), "at-1");
        let page = c.list_events("primary", &EventsRequest::default()).await.unwrap();
        assert_eq!(page.events[0].start.date.as_deref(), Some("2026-08-08"));
        assert!(page.events[0].start.date_time.is_none());
    }

    #[tokio::test]
    async fn a_cancelled_instance_is_returned_not_dropped() {
        // Incremental syncs deliver deletions as status=cancelled tombstones.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"id": "e3", "status": "cancelled"}],
                "nextSyncToken": "tok-3"
            })))
            .mount(&server).await;

        let c = CalendarClient::new(server.uri(), "at-1");
        let page = c.list_events("primary", &EventsRequest::default()).await.unwrap();
        assert_eq!(page.events[0].status, "cancelled");
    }

    #[tokio::test]
    async fn a_410_becomes_sync_token_invalid() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .respond_with(ResponseTemplate::new(410).set_body_json(serde_json::json!({
                "error": {"code": 410, "message": "Sync token is no longer valid"}
            })))
            .mount(&server).await;

        let c = CalendarClient::new(server.uri(), "at-1");
        let req = EventsRequest { sync_token: Some("stale".into()), ..Default::default() };
        match c.list_events("primary", &req).await {
            Err(ApiError::SyncTokenInvalid) => {}
            other => panic!("expected SyncTokenInvalid, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_500_is_a_plain_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server).await;

        let c = CalendarClient::new(server.uri(), "at-1");
        match c.list_events("primary", &EventsRequest::default()).await {
            Err(ApiError::Http(_)) => {}
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_patch_asks_google_to_tell_the_organiser() {
        // events.patch notifies nobody by default. An RSVP the organiser never
        // receives is worse than none: the user believes they have declined and
        // the organiser is still expecting them.
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::query_param("sendUpdates", "all"))
            .respond_with(wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "id": "ev1", "status": "confirmed" })))
            .expect(1)
            .mount(&server).await;

        let c = CalendarClient::new(server.uri(), "at");
        c.patch_event("cal@x.com", "ev1", &serde_json::json!({}), "all", None).await.unwrap();
        // `.expect(1)` fails the test on drop if sendUpdates=all was absent.
    }

    /// **The choice, not the constant.** A patch sends exactly the
    /// `sendUpdates` it was handed, and both answers are asserted because they
    /// are opposite instructions to Google: `all` mails the guest list, `none`
    /// mails nobody.
    ///
    /// `query_param` **and** `.expect(1)`, for the reason
    /// `delete_sends_if_match_and_notifies_guests` already spells out: a
    /// request that omitted the parameter matches no mock, and wiremock's
    /// unmatched-request 404 would come back through a path that does not
    /// distinguish it from a transport failure. The matcher says what a
    /// matching request looks like; `.expect(1)` is what insists one happened.
    #[tokio::test]
    async fn a_patch_sends_the_send_updates_it_was_given() {
        for send_updates in ["all", "none"] {
            let server = MockServer::start().await;
            Mock::given(method("PATCH"))
                .and(path("/calendars/cal%40x.com/events/ev1"))
                .and(query_param("sendUpdates", send_updates))
                .and(query_param("conferenceDataVersion", "1"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "ev1", "status": "confirmed"
                })))
                .expect(1)
                .mount(&server)
                .await;

            let c = CalendarClient::new(server.uri(), "at-1");
            let ev = c
                .patch_event("cal@x.com", "ev1", &serde_json::json!({}), send_updates, None)
                .await
                .unwrap();
            assert_eq!(ev.id, "ev1");
        }
    }

    #[tokio::test]
    async fn a_successful_patch_with_an_unreadable_response_is_marked_committed() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let c = CalendarClient::new(server.uri(), "at");
        let error = c
            .patch_event("cal", "ev1", &serde_json::json!({"summary": "New"}), "none", None)
            .await
            .unwrap_err();
        assert!(matches!(error, ApiError::WriteCommitted(_)), "got {error:?}");
    }

    #[tokio::test]
    async fn a_stale_etag_surfaces_as_precondition_failed() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .respond_with(wiremock::ResponseTemplate::new(412))
            .mount(&server).await;

        let c = CalendarClient::new(server.uri(), "at");
        let err = c.patch_event("cal@x.com", "ev1", &serde_json::json!({}), "all", Some("\"old\""))
            .await.unwrap_err();
        assert!(matches!(err, ApiError::PreconditionFailed), "got {err:?}");
    }

    #[tokio::test]
    async fn an_if_match_header_carries_the_etag() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/calendars/cal%40x.com/events/ev1"))
            .and(header("if-match", "\"old\""))
            .and(query_param("sendUpdates", "all"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ev1", "status": "confirmed"
            })))
            .mount(&server).await;

        let c = CalendarClient::new(server.uri(), "at-1");
        let ev = c
            .patch_event("cal@x.com", "ev1", &serde_json::json!({"status": "cancelled"}), "all", Some("\"old\""))
            .await
            .unwrap();
        assert_eq!(ev.id, "ev1");
    }

    #[tokio::test]
    async fn get_event_url_encodes_the_calendar_and_event_ids() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/calendars/me%40x.com/events/abc_20260101T090000Z"))
            .and(header("authorization", "Bearer at-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "abc_20260101T090000Z", "status": "confirmed", "summary": "Standup"
            })))
            .mount(&server).await;

        let c = CalendarClient::new(server.uri(), "at-1");
        let ev = c.get_event("me@x.com", "abc_20260101T090000Z").await.unwrap();
        assert_eq!(ev.id, "abc_20260101T090000Z");
        assert_eq!(ev.summary.as_deref(), Some("Standup"));
    }

    /// The calendar id here is deliberately email-shaped (`cal@x.com`), the
    /// same way `get_event_url_encodes_the_calendar_and_event_ids` above
    /// proves its own encoding: `respond_to_event` is the first real caller
    /// of `event_instances`, and it always passes an email-shaped calendar
    /// id. A plain-ASCII id like `"primary"` round-trips through
    /// `urlencoding_path` unchanged, so a test using only that would not
    /// notice `urlencoding_path` being removed from this method — which is
    /// exactly what happened once already: a reviewer removed it here and
    /// every test in the crate still passed.
    #[tokio::test]
    async fn event_instances_url_encodes_the_calendar_id() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/calendars/cal%40x.com/events/master1/instances"))
            .and(query_param("timeMin", "2026-08-01T00:00:00Z"))
            .and(query_param("timeMax", "2026-08-31T00:00:00Z"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {"id": "master1_20260803T090000Z", "status": "confirmed"},
                    {"id": "master1_20260804T090000Z", "status": "confirmed"}
                ]
            })))
            .mount(&server).await;

        let c = CalendarClient::new(server.uri(), "at-1");
        let instances = c
            .event_instances("cal@x.com", "master1", "2026-08-01T00:00:00Z", "2026-08-31T00:00:00Z")
            .await
            .unwrap();
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].id, "master1_20260803T090000Z");
    }

    /// `.expect(1)` and the `query_param` matcher together are what make this
    /// test about notification at all: without the matcher a request carrying
    /// any `sendUpdates` would match, and without `.expect(1)` a request that
    /// missed the mock would come back as wiremock's bare 404 and fail for a
    /// reason that reads like a transport problem rather than a wrong query.
    #[tokio::test]
    async fn insert_posts_the_body_and_notifies_exactly_as_asked() {
        for send_updates in ["none", "all"] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/calendars/cal%40x.com/events"))
                .and(query_param("sendUpdates", send_updates))
                .and(query_param("conferenceDataVersion", "1"))
                .and(body_json_string(r#"{"summary":"Lunch"}"#))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "new1", "status": "confirmed", "etag": "\"e1\""
                })))
                .expect(1)
                .mount(&server)
                .await;

            let c = CalendarClient::new(server.uri(), "tok");
            let ev = c
                .insert_event("cal@x.com", &serde_json::json!({"summary": "Lunch"}), send_updates)
                .await
                .unwrap();
            assert_eq!(ev.id, "new1");
        }
    }

    #[tokio::test]
    async fn a_successful_insert_with_an_unreadable_response_is_marked_committed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let c = CalendarClient::new(server.uri(), "tok");
        let error = c
            .insert_event("cal", &serde_json::json!({"summary": "Lunch"}), "none")
            .await
            .unwrap_err();
        assert!(matches!(error, ApiError::WriteCommitted(_)), "got {error:?}");
    }

    #[tokio::test]
    async fn delete_sends_if_match_and_notifies_guests() {
        // `.expect(1)` is load-bearing, not decorative: `delete_event` treats
        // an unmatched-mock 404 as success (that is its own 404-means-gone
        // rule, working exactly as designed), so a request that misses this
        // mock — e.g. a wrong `sendUpdates` — would otherwise still return
        // `Ok(())` and this test would pass while asserting nothing.
        //
        // The ids are deliberately chosen so each encodes differently under
        // `urlencoding_path`: `cal@x.com` -> `cal%40x.com`, `ev#1` -> `ev%231`.
        // Plain-ASCII ids like the old `"c"`/`"e1"` round-trip unchanged, so
        // they would never notice `urlencoding_path` being silently removed
        // from either path segment — the same trap documented on
        // `event_instances_url_encodes_the_calendar_id` above.
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/calendars/cal%40x.com/events/ev%231"))
            .and(query_param("sendUpdates", "all"))
            .and(header("If-Match", "\"etag1\""))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let c = CalendarClient::new(server.uri(), "tok");
        c.delete_event("cal@x.com", "ev#1", Some("\"etag1\""))
            .await
            .unwrap();
    }

    /// Already gone is the caller's desired end state, not an error.
    #[tokio::test]
    async fn delete_treats_404_as_success() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let c = CalendarClient::new(server.uri(), "tok");
        c.delete_event("c", "gone", None).await.unwrap();
    }

    /// The sibling that reality actually sends: Google answers **410 Gone**,
    /// not 404, for an event somebody already deleted — the case a user hits
    /// when they try to remove a ghost of a meeting deleted elsewhere
    /// (2026-08-23, live). Same rule as 404: already gone is the desired end
    /// state.
    #[tokio::test]
    async fn delete_treats_410_as_success() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(410))
            .mount(&server)
            .await;

        let c = CalendarClient::new(server.uri(), "tok");
        c.delete_event("c", "gone", None).await.unwrap();
    }

    #[tokio::test]
    async fn delete_surfaces_a_conflict_as_precondition_failed() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(412))
            .mount(&server)
            .await;

        let c = CalendarClient::new(server.uri(), "tok");
        assert!(matches!(
            c.delete_event("c", "e1", Some("\"old\"")).await,
            Err(ApiError::PreconditionFailed)
        ));
    }
}
