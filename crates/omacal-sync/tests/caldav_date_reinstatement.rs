use omacal_caldav::CalDavClient;
use omacal_store::changes::changed_meetings;
use omacal_sync::caldav::sync_caldav_calendar;
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

async fn report(server: &MockServer, tag: &str, extra: &str, exception: &str) {
    server.reset().await;
    let guest = "ORGANIZER:mailto:organizer@example.org\nATTENDEE;PARTSTAT=ACCEPTED:mailto:guest@example.org";
    let ics = format!("BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:s\nSUMMARY:Meeting\nDTSTART;VALUE=DATE:20261201\nRRULE:FREQ=DAILY;COUNT=2\n{extra}{guest}\nEND:VEVENT\n{exception}END:VCALENDAR");
    for (verb, path, prop) in [
        ("PROPFIND", "/cal/", format!("<cs:getctag>{tag}</cs:getctag>")),
        ("REPORT", "/cal/s.ics", format!("<d:getetag>{tag}</d:getetag><c:calendar-data>{ics}</c:calendar-data>")),
    ] {
        let xml = format!(r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/"><d:response><d:href>{path}</d:href><d:propstat><d:prop>{prop}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#);
        Mock::given(method(verb)).respond_with(ResponseTemplate::new(207).set_body_string(xml)).mount(server).await;
    }
}

#[tokio::test]
async fn a_restored_date_survives_a_change_in_midnight_offset() {
    check_reinstatement(false).await;
}

#[tokio::test]
async fn a_legacy_exception_recovers_its_date_from_the_previous_report() {
    check_reinstatement(true).await;
}

async fn check_reinstatement(legacy: bool) {
    let server = MockServer::start().await;
    let pool = omacal_store::connect_memory().await.unwrap();
    let url = format!("{}/cal/", server.uri());
    sqlx::query("INSERT INTO accounts(google_sub,email,created_at,provider) VALUES('g','guest@example.org',0,'caldav')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO calendars(account_id,google_id,summary,timezone,access_role) VALUES(1,?,'Meetings','Etc/GMT+4','owner')")
        .bind(&url).execute(&pool).await.unwrap();
    let client = CalDavClient::new(&server.uri(), "guest@example.org", "pw").unwrap();
    let ms = |s: &str| s.parse::<jiff::Timestamp>().unwrap().as_millisecond();
    let now = ms("2026-11-24T12:00:00Z");
    let sync = || sync_caldav_calendar(&pool, &client, 1, &url, true, false, now, ms("2027-01-01T00:00:00Z"), now);
    let moved = "BEGIN:VEVENT\nUID:s\nRECURRENCE-ID;VALUE=DATE:20261202\nDTSTART;VALUE=DATE:20261203\nSUMMARY:Meeting\nORGANIZER:mailto:organizer@example.org\nATTENDEE;PARTSTAT=ACCEPTED:mailto:guest@example.org\nEND:VEVENT\n";
    report(&server, "moved", "", moved).await;
    sync().await.unwrap();

    if legacy {
        // Recreate migration 0021's state for a row synced by an older app.
        sqlx::query("UPDATE caldav_recurrence_dates SET recurrence_date=NULL")
            .execute(&pool).await.unwrap();
    }

    // Pin both offsets so the regression is independent of the host tzdata.
    // At the new offset, the saved 04:00Z ID even falls on the previous date.
    sqlx::query("UPDATE calendars SET timezone='Etc/GMT+5' WHERE id=1").execute(&pool).await.unwrap();
    report(&server, "cancelled", "EXDATE;VALUE=DATE:20261202\n", "").await;
    sync().await.unwrap();
    let cancellations = || async {
        changed_meetings(&pool, now).await.unwrap().into_iter().filter(|m| m.kind == "cancelled").count()
    };
    assert_eq!(cancellations().await, 1);

    report(&server, "restored", "", "").await;
    sync().await.unwrap();
    assert_eq!(cancellations().await, 0);
}
