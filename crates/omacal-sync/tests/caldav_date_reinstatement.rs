use omacal_caldav::CalDavClient;
use omacal_store::changes::changed_meetings;
use omacal_sync::caldav::sync_caldav_calendar;
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

async fn report(server: &MockServer, tag: &str, master: &str, exception: &str) {
    server.reset().await;
    let guest = "ORGANIZER:mailto:organizer@example.org\nATTENDEE;PARTSTAT=ACCEPTED:mailto:guest@example.org";
    let ics = format!("BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:s\nSUMMARY:Meeting\nDTSTART{master}\n{guest}\nEND:VEVENT\n{exception}END:VCALENDAR");
    for (verb, path, prop) in [
        ("PROPFIND", "/cal/", format!("<cs:getctag>{tag}</cs:getctag>")),
        ("REPORT", "/cal/s.ics", format!("<d:getetag>{tag}</d:getetag><c:calendar-data>{ics}</c:calendar-data>")),
    ] {
        let xml = format!(r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/"><d:response><d:href>{path}</d:href><d:propstat><d:prop>{prop}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#);
        Mock::given(method(verb)).respond_with(ResponseTemplate::new(207).set_body_string(xml)).mount(server).await;
    }
}

async fn guest_calendar(url: &str, timezone: &str) -> sqlx::SqlitePool {
    let pool = omacal_store::connect_memory().await.unwrap();
    sqlx::query("INSERT INTO accounts(google_sub,email,created_at,provider) VALUES('g','guest@example.org',0,'caldav')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO calendars(account_id,google_id,summary,timezone,access_role) VALUES(1,?,'Meetings',?,'owner')")
        .bind(url).bind(timezone).execute(&pool).await.unwrap();
    pool
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
    let url = format!("{}/cal/", server.uri());
    let pool = guest_calendar(&url, "Etc/GMT+4").await;
    let client = CalDavClient::new(&server.uri(), "guest@example.org", "pw").unwrap();
    let ms = |s: &str| s.parse::<jiff::Timestamp>().unwrap().as_millisecond();
    let now = ms("2026-11-24T12:00:00Z");
    let sync = || sync_caldav_calendar(&pool, &client, 1, &url, true, false, now, ms("2027-01-01T00:00:00Z"), now);
    let moved = "BEGIN:VEVENT\nUID:s\nRECURRENCE-ID;VALUE=DATE:20261202\nDTSTART;VALUE=DATE:20261203\nSUMMARY:Meeting\nORGANIZER:mailto:organizer@example.org\nATTENDEE;PARTSTAT=ACCEPTED:mailto:guest@example.org\nEND:VEVENT\n";
    let date_start = ";VALUE=DATE:20261201\nRRULE:FREQ=DAILY;COUNT=2";
    let timed_start = ":20261201T050000Z\nRRULE:FREQ=DAILY;COUNT=2";
    report(&server, "moved", date_start, moved).await;
    sync().await.unwrap();

    if legacy {
        // Recreate migration 0021's state for a row synced by an older app.
        sqlx::query("UPDATE caldav_recurrence_dates SET recurrence_date=NULL")
            .execute(&pool).await.unwrap();
    }

    // Pin both offsets so the regression is independent of the host tzdata.
    // At the new offset, the saved 04:00Z ID even falls on the previous date.
    sqlx::query("UPDATE calendars SET timezone='Etc/GMT+5' WHERE id=1").execute(&pool).await.unwrap();
    report(&server, "cancelled", &format!("{date_start}\nEXDATE;VALUE=DATE:20261202"), "").await;
    sync().await.unwrap();
    let cancellations = || async {
        changed_meetings(&pool, now).await.unwrap().into_iter().filter(|m| m.kind == "cancelled").count()
    };
    assert_eq!(cancellations().await, 1);

    report(&server, "restored", date_start, "").await;
    sync().await.unwrap();
    assert_eq!(cancellations().await, 0);

    // A timed override can reuse an old DATE key exactly. Restoring it
    // must consult the timed rule, even when its predecessor was all-day.
    report(&server, "all-day-move", date_start, moved).await;
    sync().await.unwrap();
    // The server can also omit the override in the very first timed REPORT.
    report(&server, "direct-timed-restore", timed_start, "").await;
    sync().await.unwrap();
    assert_eq!(cancellations().await, 0);
    report(&server, "all-day-move-again", date_start, moved).await;
    sync().await.unwrap();
    let timed = moved.replace(";VALUE=DATE:20261202", ":20261202T050000Z")
        .replace(";VALUE=DATE:20261203", ":20261203T050000Z");
    report(&server, "timed-move", timed_start, &timed).await;
    sync().await.unwrap();
    report(&server, "timed-restored", timed_start, "").await;
    sync().await.unwrap();
    assert_eq!(cancellations().await, 0);
}

#[tokio::test]
async fn guest_cancellations_follow_the_replacement_rule_across_a_gap() {
    let server = MockServer::start().await;
    let url = format!("{}/cal/", server.uri());
    let client = CalDavClient::new(&server.uri(), "guest@example.org", "pw").unwrap();
    let ms = |s: &str| s.parse::<jiff::Timestamp>().unwrap().as_millisecond();
    let now = ms("2026-03-01T12:00:00Z");
    let before_gap = ";TZID=America/New_York:20260307T023000";
    let in_gap = ";TZID=America/New_York:20260308T023000";
    let first_position = "FREQ=DAILY;BYHOUR=2,3;BYSETPOS=1;COUNT=3";
    let two_positions = "FREQ=DAILY;BYHOUR=2,3;BYSETPOS=1,2;COUNT=5";
    let reordered = "FREQ=DAILY;BYHOUR=2,3;BYMINUTE=0,30;BYSETPOS=2,3;COUNT=3";
    for (start, rule, slot, cancelled) in [
        (before_gap, "FREQ=DAILY;COUNT=2", "20260308T033000", false),
        (before_gap, "FREQ=DAILY;COUNT=2", "20260309T023000", true),
        (before_gap, "FREQ=DAILY", "20260308T033000", false),
        (before_gap, first_position, "20260308T033000", false),
        (before_gap, first_position, "20260309T023000", false),
        (before_gap, first_position, "20260310T023000", true),
        (before_gap, "FREQ=DAILY;COUNT=5;", "20260309T023000", false),
        (before_gap, "FREQ=DAILY;COUNT=5", "20260309T023000", false),
        (before_gap, two_positions, "20260309T033000", false),
        (before_gap, "FREQ=DAILY;BYHOUR=2,3;COUNT=5", "20260309T033000", true),
        (before_gap, reordered, "20260308T030000", false),
        (before_gap, reordered, "20260308T033000", true),
        (in_gap, "FREQ=HOURLY;COUNT=2", "20260308T043000", false),
        (in_gap, "FREQ=HOURLY;COUNT=2", "20260308T053000", true),
        (in_gap, "FREQ=DAILY;COUNT=2", "20260309T023000", false),
        (in_gap, "FREQ=DAILY;COUNT=2", "20260309T033000", true),
    ] {
        let pool = guest_calendar(&url, "America/New_York").await;
        let moved = format!("BEGIN:VEVENT\nUID:s\nRECURRENCE-ID;TZID=America/New_York:{slot}\n\
            DTSTART;TZID=America/New_York:{}T130000\nSUMMARY:Meeting\nORGANIZER:mailto:organizer@example.org\n\
            ATTENDEE;PARTSTAT=ACCEPTED:mailto:guest@example.org\nEND:VEVENT\n", &slot[..8]);
        let sync = || sync_caldav_calendar(&pool, &client, 1, &url, true, false, now, ms("2026-04-01T00:00:00Z"), now);
        report(&server, "moved", &format!("{before_gap}\nRRULE:FREQ=DAILY;BYHOUR=2,3,4,5;BYMINUTE=0,30"), &moved).await;
        sync().await.unwrap();
        let overrides = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events WHERE recurring_event_id='s'")
            .fetch_one(&pool);
        assert_eq!(overrides().await.unwrap(), 1);

        report(&server, "replacement", &format!("{start}\nRRULE:{rule}"), "").await;
        sync().await.unwrap();
        assert_eq!(overrides().await.unwrap(), 0);
        let tray = changed_meetings(&pool, now).await.unwrap();
        assert_eq!(tray.iter().any(|m| m.kind == "cancelled"), cancelled, "{rule} at {slot}");
    }
}
