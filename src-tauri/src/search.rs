//! Search: the query half. The overlay is Task 2.
//!
//! Three layers, and the split is the same one `notify_loop` uses: the store
//! answers *which events match* (`omacal_store::search_events` — titles, and
//! only calendars the user displays), `omacal_core::search` answers *which
//! occurrence and in what order* against a clock it is handed, and this file
//! joins them. **Tasks are the fourth piece**: an open task has no
//! occurrence, so it is matched here in Rust and ordered by due date, and the
//! two kinds travel as two arrays — see [`SearchResults`].
//!
//! **No write path, no network** (spec §7). This is a `SELECT` and some
//! arithmetic. A search that synced first would be slow and surprising, and a
//! search that could change an event would be a second way to do what the
//! popover already does with every guard it has.

use omacal_core::search::{by_distance, nearest, Hit};
use omacal_core::layout::Interval;
use omacal_store::StoredEvent;
use sqlx::SqlitePool;

use crate::cli::{task_json, TaskHit};
use crate::AppState;

/// What a search answers with: events and tasks, kept apart.
///
/// **Two arrays, not one list**. An event resolves to an
/// occurrence and is ordered by its distance from today; a task has no
/// occurrence and is ordered by its due date. A single list would need one
/// sort key that means both, and one pick handler that branches anyway — so
/// the kinds stay separate all the way out, and the CLI and the overlay render
/// them as two sections.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResults {
    pub events: Vec<Hit>,
    pub tasks: Vec<TaskHit>,
}


/// How wide a net to cast when resolving a recurring match to one occurrence.
///
/// A year either side of the clock. It is a policy rather than a fact, and it
/// lives here — in the impure half — rather than in `omacal_core::search`,
/// which is handed occurrences and does not care where the window came from.
///
/// A year is enough for everything a person searches for by name: a weekly
/// standup, a monthly review, an annual trip all have an occurrence inside it.
/// What it misses is a series that stopped more than a year ago or starts more
/// than a year out, and [`resolve`]'s fallback is what covers those.
const NEAR_WINDOW_MS: i64 = 365 * 24 * 3_600_000;

/// How many occurrences the fallback expansion may produce before giving up.
///
/// Only the *far* path uses it, and only to bound a daily series expanded over
/// decades. `u16` because that is what `expand` takes.
const FAR_LIMIT: u16 = 2_000;

/// The occurrence of `src` nearest `now_ms`.
///
/// **Not the master's own start**, which for a series running since 2019 is
/// four years from anything anybody is looking for — and which is the answer a
/// naive implementation gives, because for a series beginning today the two
/// coincide and every simple test passes.
///
/// Two passes, and the second is what makes the first's window a convenience
/// rather than a correctness claim:
///
/// 1. A year either side. Covers everything a person searches by name.
/// 2. Failing that, from the series' own `DTSTART` to a year out. A series that
///    ended in 2021 resolves to its **last** occurrence; one that starts in
///    2030 resolves to its first. Bounded by [`FAR_LIMIT`], which for a daily
///    series is about five and a half years of it — enough to reach the end of
///    anything that has one.
///
/// Falls back to the row's own span only when both expansions come back empty,
/// which means a rule that generates nothing at all. Something is on screen for
/// the user either way; a result with no occurrence would be a row that cannot
/// be clicked.
fn resolve(src: &StoredEvent, now_ms: i64) -> Interval {
    let own = Interval { start_ms: src.start_utc, end_ms: src.end_utc };
    if src.recurrence.is_none() {
        return own;
    }

    let near = crate::commands::occurrences(src, now_ms - NEAR_WINDOW_MS, now_ms + NEAR_WINDOW_MS);
    if let Some(iv) = nearest(&near, now_ms) {
        return iv;
    }

    let far = crate::commands::occurrences_limited(
        src,
        src.start_utc,
        now_ms + NEAR_WINDOW_MS,
        FAR_LIMIT,
    );
    nearest(&far, now_ms).unwrap_or(own)
}

/// Everything whose **title** contains `query`: events nearest first, open
/// tasks by due date.
///
/// `now_ms` is a parameter for the reason `due_reminders` takes one: "nearest
/// today" is the whole of the event ordering, and a function that read a clock
/// could not be tested against a fixed one.
///
/// **Titles only, both kinds** (spec §2): an event's `summary`, and a
/// task's `summary` with `None` handed to `matches_query` as the note — so a
/// word in a task's note is not a match, exactly as a word in an event's
/// description is not. Tasks appear only while they are **open** and on a
/// `selected` list (`tasks_for_ui` already carries both, and filters the rest
/// out), because "what do I still have to do" is the question a task search is
/// for.
pub(crate) async fn search_impl(
    pool: &SqlitePool,
    query: &str,
    now_ms: i64,
) -> anyhow::Result<SearchResults> {
    // An empty query is not "match everything" — `LIKE '%%'` would return the
    // entire database, and the overlay asks on every keystroke including the
    // one that empties the field.
    let q = query.trim();
    if q.is_empty() {
        return Ok(SearchResults { events: Vec::new(), tasks: Vec::new() });
    }

    let rows = omacal_store::search_events(pool, q).await?;
    let mut events: Vec<Hit> = rows
        .iter()
        .map(|src| {
            let iv = resolve(src, now_ms);
            Hit {
                event_id: src.id,
                title: src.summary.clone().unwrap_or_else(|| "(no title)".into()),
                start_ms: iv.start_ms,
                end_ms: iv.end_ms,
            }
        })
        .collect();
    by_distance(&mut events, now_ms);

    // The zone a task's "late" is read in; the same `TimeZone::system()` the
    // CLI's own `tasks` read uses, and the window ignores the formatted `due`
    // in favour of `due_ms`.
    let tz = jiff::tz::TimeZone::system();
    let rows = omacal_store::tasks_for_ui(pool, now_ms).await?;
    let mut tasks: Vec<TaskHit> = rows
        .iter()
        .filter(|r| r.task.status != "completed")
        .filter(|r| {
            crate::tasks::matches_query(r.task.summary.as_deref().unwrap_or(""), None, q)
        })
        .map(|r| task_json(r, now_ms, &tz))
        .collect();
    // Due date first, undated last, title as the tiebreak — the sidebar's own
    // "By when" order, flattened.
    tasks.sort_by(|a, b| {
        (a.due_ms.is_none(), a.due_ms.unwrap_or(0), a.summary.as_str())
            .cmp(&(b.due_ms.is_none(), b.due_ms.unwrap_or(0), b.summary.as_str()))
    });

    Ok(SearchResults { events, tasks })
}

#[tauri::command]
pub async fn search(
    state: tauri::State<'_, AppState>,
    query: String,
) -> Result<SearchResults, String> {
    // The one clock read in this feature, and it is here rather than any
    // deeper: `search_impl` takes `now_ms` so it can be driven against a fixed
    // one, exactly as `due_reminders` does.
    search_impl(&state.pool, &query, crate::now_ms())
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 24 * HOUR;
    /// Monday 10 Aug 2026, 12:00 UTC. Fixed, because "nearest today" is the
    /// thing under test.
    const NOW: i64 = 1_786_017_600_000;

    async fn pool_with_two_calendars() -> SqlitePool {
        let pool = omacal_store::connect_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (google_sub, email, created_at) VALUES ('s','me@x.com',0)")
            .execute(&pool).await.unwrap();
        // 1 shown, 2 hidden. Both synced — `selected` and `sync_enabled` are
        // separate switches, and it is `selected` search follows.
        for (gid, name, selected) in [("shown", "Shown", 1), ("hidden", "Hidden", 0)] {
            sqlx::query(
                "INSERT INTO calendars
                     (account_id, google_id, summary, color_hex, timezone, access_role,
                      is_primary, selected, sync_enabled)
                 VALUES (1, ?1, ?2, '#5b8def', 'UTC', 'owner', 0, ?3, 1)")
                .bind(gid).bind(name).bind(selected)
                .execute(&pool).await.unwrap();
        }
        pool
    }

    fn row(cal: i64, gid: &str, title: &str, start_ms: i64) -> StoredEvent {
        StoredEvent {
            id: 0, calendar_id: cal, google_id: gid.into(), summary: Some(title.into()),
            location: None, start_utc: start_ms, end_utc: start_ms + HOUR,
            start_tz: "UTC".into(), end_tz: "UTC".into(), is_all_day: false,
            recurrence: None, recurring_event_id: None, original_start_utc: None,
            status: "confirmed".into(), self_response: None, conference_uri: None,
            color_hex: None, calendar_timezone: "UTC".into(),
            description: None, etag: None, sequence: 0, organizer_email: None,
            guests_can_modify: false,
            transparency: "opaque".into(), visibility: "default".into(),
            attendees: Vec::new(),
            reminders: Default::default(), calendar_default_reminders: Vec::new(),
        }
    }

    async fn insert(pool: &SqlitePool, e: &StoredEvent) {
        omacal_store::upsert_event(pool, e).await.unwrap();
    }

    /// The matched **events** for `q` at the fixed clock. Tasks are read
    /// through [`tasks`]: the two kinds are separate answers and the tests keep
    /// them separate, which is the shape under test.
    async fn events(pool: &SqlitePool, q: &str) -> Vec<Hit> {
        search_impl(pool, q, NOW).await.unwrap().events
    }

    async fn tasks(pool: &SqlitePool, q: &str) -> Vec<TaskHit> {
        search_impl(pool, q, NOW).await.unwrap().tasks
    }

    fn task(cal: i64, uid: &str, title: &str, status: &str, due_ms: Option<i64>) -> omacal_store::StoredTask {
        omacal_store::StoredTask {
            id: 0, calendar_id: cal, uid: uid.into(), etag: None, caldav_href: None,
            summary: Some(title.into()), description: None, due_utc: due_ms,
            due_tz: Some("UTC".into()), due_all_day: true, status: status.into(),
            completed_utc: None, priority: 0, updated_at: 0, raw_ics: None,
        }
    }

    async fn insert_task(pool: &SqlitePool, t: &omacal_store::StoredTask) {
        omacal_store::upsert_task(pool, t).await.unwrap();
    }

    #[tokio::test]
    async fn a_title_is_matched_case_insensitively_anywhere_in_it() {
        let pool = pool_with_two_calendars().await;
        insert(&pool, &row(1, "a", "Quarterly Board Review", NOW + DAY)).await;

        for q in ["board", "BOARD", "Board Review", "quarterly"] {
            assert_eq!(events(&pool, q).await.len(), 1, "query {q:?} should match");
        }
        assert!(events(&pool, "budget").await.is_empty());
    }

    /// §2: titles and nothing else. A word in the location must not pull an
    /// event in, or results stop being explicable from what was typed.
    #[tokio::test]
    async fn a_match_in_the_location_is_not_a_match() {
        let pool = pool_with_two_calendars().await;
        let mut e = row(1, "a", "Standup", NOW + DAY);
        e.location = Some("Board room".into());
        insert(&pool, &e).await;

        assert!(events(&pool, "board").await.is_empty());
    }

    /// **Spec §5, both halves in one query.** The absence alone would pass
    /// against a search that returns nothing at all, which is why the visible
    /// calendar's match is asserted in the same call.
    #[tokio::test]
    async fn a_hidden_calendars_events_are_not_found_while_a_shown_calendars_are() {
        let pool = pool_with_two_calendars().await;
        insert(&pool, &row(1, "shown-ev", "Team lunch", NOW + DAY)).await;
        insert(&pool, &row(2, "hidden-ev", "Team lunch", NOW + 2 * DAY)).await;

        let hits = events(&pool, "team lunch").await;

        assert_eq!(hits.len(), 1, "the hidden calendar's copy must not appear");
        assert_eq!(hits[0].start_ms, NOW + DAY, "and the one that did is the shown one");
    }

    /// §4, with fixtures on **both sides of today** — without the past ones,
    /// nearest-first and soonest-first are the same order.
    #[tokio::test]
    async fn results_come_back_nearest_first_in_either_direction() {
        let pool = pool_with_two_calendars().await;
        insert(&pool, &row(1, "a", "Trip to Rome", NOW + 300 * DAY)).await;
        insert(&pool, &row(1, "b", "Trip to Lisbon", NOW - 2 * DAY)).await;
        insert(&pool, &row(1, "c", "Trip to Oslo", NOW + DAY)).await;
        insert(&pool, &row(1, "d", "Trip to Cairo", NOW - 400 * DAY)).await;

        let hits = events(&pool, "trip to").await;

        assert_eq!(
            hits.iter().map(|h| h.title.as_str()).collect::<Vec<_>>(),
            vec!["Trip to Oslo", "Trip to Lisbon", "Trip to Rome", "Trip to Cairo"],
        );
    }

    /// **Spec §3, and the fixture is built to witness both mistakes.**
    ///
    /// A weekly standup running for two years is one row and about a hundred
    /// occurrences. One result — and the occurrence it resolves to is the
    /// nearest, which is neither the series' own `DTSTART` (two years ago) nor
    /// the first occurrence of any window.
    #[tokio::test]
    async fn a_long_running_series_is_one_result_at_its_nearest_occurrence() {
        let pool = pool_with_two_calendars().await;
        // DTSTART two years before the clock, on the same weekday, so the
        // occurrences land on NOW - 2y + 7n days.
        let dtstart = NOW - 728 * DAY;
        let mut e = row(1, "series", "Standup", dtstart);
        e.recurrence = Some("RRULE:FREQ=WEEKLY".into());
        insert(&pool, &e).await;

        let hits = events(&pool, "standup").await;

        assert_eq!(hits.len(), 1, "a series is one result, not one per occurrence");
        assert_ne!(hits[0].start_ms, dtstart, "not the master's own start");
        // 728 is a multiple of 7, so an occurrence falls exactly on the clock.
        assert_eq!(hits[0].start_ms, NOW, "the nearest occurrence");
    }

    /// The other half of §3's rule: a series that **ended** long ago resolves
    /// to its last occurrence rather than its first. This is the case the
    /// near-window alone cannot answer, and the one where "the master's own
    /// start" is most obviously wrong.
    #[tokio::test]
    async fn a_series_that_ended_years_ago_resolves_to_its_last_occurrence() {
        let pool = pool_with_two_calendars().await;
        let dtstart = NOW - 1_400 * DAY;
        let mut e = row(1, "old", "Retro", dtstart);
        // Four weekly occurrences, then it stops.
        e.recurrence = Some("RRULE:FREQ=WEEKLY;COUNT=4".into());
        insert(&pool, &e).await;

        let hits = events(&pool, "retro").await;

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].start_ms, dtstart + 21 * DAY, "the last of the four, not the first");
    }

    /// §2 again, from the direction a query language would break it: `%` is a
    /// character somebody typed, not a wildcard.
    ///
    /// **The fixture is built so an unescaped `%` gives a different answer**,
    /// which took a mutation to get right. A first version searched `80%`
    /// against "Q3 at 80% capacity" and a "Standup": unescaped that is
    /// `%80%%`, which still needs the literal "80" and so still matched one
    /// row — the test passed either way and witnessed nothing. Here the query
    /// spans the percent sign, so unescaped it matches the spelled-out title
    /// too and the count changes.
    #[tokio::test]
    async fn a_percent_sign_matches_a_percent_sign_and_not_anything() {
        let pool = pool_with_two_calendars().await;
        insert(&pool, &row(1, "a", "50% progress", NOW + DAY)).await;
        insert(&pool, &row(1, "b", "50 percent progress", NOW + 2 * DAY)).await;

        // `50% p` — a substring of the first title and, unescaped, a wildcard
        // match for the second as well ("50", anything, " p").
        let hits = events(&pool, "50% p").await;

        assert_eq!(hits.len(), 1, "an unescaped % would match the spelled-out one too");
        assert_eq!(hits[0].title, "50% progress");
    }

    #[tokio::test]
    async fn an_underscore_is_a_character_too() {
        let pool = pool_with_two_calendars().await;
        insert(&pool, &row(1, "a", "sync_now", NOW + DAY)).await;
        insert(&pool, &row(1, "b", "syncXnow", NOW + 2 * DAY)).await;

        let hits = events(&pool, "sync_now").await;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "sync_now");
    }

    /// The overlay asks on every keystroke, including the one that empties the
    /// field. `LIKE '%%'` would answer with the whole database, and an empty
    /// query must not be "every open task" either.
    #[tokio::test]
    async fn an_empty_query_matches_nothing_rather_than_everything() {
        let pool = pool_with_two_calendars().await;
        insert(&pool, &row(1, "a", "Standup", NOW + DAY)).await;
        insert_task(&pool, &task(1, "t1", "Call the bank", "needs-action", None)).await;

        assert!(events(&pool, "").await.is_empty());
        assert!(events(&pool, "   ").await.is_empty());
        assert!(tasks(&pool, "").await.is_empty());
        assert!(tasks(&pool, "   ").await.is_empty());
    }

    /// **A task is found by its title**, case-insensitively and
    /// anywhere in it, the way an event is.
    #[tokio::test]
    async fn an_open_task_is_found_by_its_title() {
        let pool = pool_with_two_calendars().await;
        insert_task(&pool, &task(1, "t1", "Call the bank", "needs-action", None)).await;

        for q in ["bank", "BANK", "call the", "the bank"] {
            assert_eq!(tasks(&pool, q).await.len(), 1, "query {q:?} should match");
        }
        assert!(tasks(&pool, "garage").await.is_empty());
    }

    /// **A task's title folds case and accents**, the same `fold` the event
    /// search applies: #153 routes task search through `matches_query`, so an
    /// unaccented query finds an accented title, in either spelling.
    #[tokio::test]
    async fn an_open_tasks_title_folds_case_and_accents() {
        let pool = pool_with_two_calendars().await;
        insert_task(&pool, &task(1, "t1", "Garnituri ușă intrare", "needs-action", None)).await;

        for q in ["usa", "ușă", "GARNITURI USA", "garnituri usa intrare"] {
            assert_eq!(tasks(&pool, q).await.len(), 1, "query {q:?} should match");
        }
        assert!(tasks(&pool, "iesire").await.is_empty(), "folding is not fuzzing");
    }

    /// The task half of the title-only rule: a word in the **note** is not a
    /// match, exactly as a word in an event's description is not.
    #[tokio::test]
    async fn a_match_in_a_tasks_note_is_not_a_match() {
        let pool = pool_with_two_calendars().await;
        let mut t = task(1, "t1", "Call the bank", "needs-action", None);
        t.description = Some("about the landlord".into());
        insert_task(&pool, &t).await;

        assert_eq!(tasks(&pool, "bank").await.len(), 1);
        assert!(tasks(&pool, "landlord").await.is_empty());
    }

    /// **Open only.** A completed task is not a search result, even one the
    /// list still carries: a `completed_utc IS NULL` completion slips past
    /// `tasks_for_ui`'s own cutoff, which is exactly the row the `status`
    /// filter here has to catch.
    #[tokio::test]
    async fn a_completed_task_is_not_a_search_hit() {
        let pool = pool_with_two_calendars().await;
        insert_task(&pool, &task(1, "open", "Call the bank", "needs-action", None)).await;
        insert_task(&pool, &task(1, "done", "Call the bank too", "completed", None)).await;

        let hits = tasks(&pool, "bank").await;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].summary, "Call the bank");
    }

    /// The list rule, for tasks as for events: a hit on a hidden list is one
    /// the user cannot act on, so it is not a hit.
    #[tokio::test]
    async fn a_task_on_a_hidden_list_is_not_found_while_a_shown_lists_is() {
        let pool = pool_with_two_calendars().await;
        insert_task(&pool, &task(1, "shown", "Pack the van", "needs-action", None)).await;
        insert_task(&pool, &task(2, "hidden", "Pack the van", "needs-action", None)).await;

        let hits = tasks(&pool, "pack the van").await;
        assert_eq!(hits.len(), 1, "the hidden list's copy must not appear");
    }

    /// Due date first (earliest first), undated last, title as the tiebreak —
    /// **not priority**. The store's own order puts priority ahead of summary,
    /// so a due tie between a high-priority "Zulu" and a low-priority "Delta"
    /// comes back Zulu first; the search re-sorts to Delta first, and that is
    /// the half this fixture exists to reach.
    #[tokio::test]
    async fn tasks_come_back_due_first_undated_last_and_ignore_priority_for_the_tie() {
        let pool = pool_with_two_calendars().await;
        insert_task(&pool, &task(1, "z", "Task Zebra", "needs-action", None)).await;
        insert_task(&pool, &task(1, "a", "Task Alpha", "needs-action", None)).await;
        insert_task(&pool, &task(1, "b", "Task Beta", "needs-action", None)).await;
        insert_task(&pool, &task(1, "m", "Task Mango", "needs-action", Some(NOW + DAY))).await;
        insert_task(&pool, &task(1, "p", "Task Apple", "needs-action", Some(NOW - DAY))).await;
        // The same due date, and priority disagrees with the title rule.
        let mut zulu = task(1, "z2", "Task Zulu", "needs-action", Some(NOW + 2 * DAY));
        zulu.priority = 1;
        let mut delta = task(1, "d", "Task Delta", "needs-action", Some(NOW + 2 * DAY));
        delta.priority = 9;
        insert_task(&pool, &zulu).await;
        insert_task(&pool, &delta).await;

        let hits = tasks(&pool, "task").await;
        assert_eq!(
            hits.iter().map(|t| t.summary.as_str()).collect::<Vec<_>>(),
            vec![
                "Task Apple", "Task Mango", "Task Delta", "Task Zulu",
                "Task Alpha", "Task Beta", "Task Zebra",
            ],
            "earliest due first, undated last, ties by title rather than priority",
        );
    }

    /// The shape itself: one query can answer with both kinds at once, and
    /// each stays in its own array with its own ordering.
    #[tokio::test]
    async fn one_query_can_answer_with_an_event_and_a_task() {
        let pool = pool_with_two_calendars().await;
        insert(&pool, &row(1, "ev", "Board prep", NOW + DAY)).await;
        insert_task(&pool, &task(1, "tk", "Board pack", "needs-action", None)).await;

        let results = search_impl(&pool, "board", NOW).await.unwrap();
        assert_eq!(results.events.len(), 1);
        assert_eq!(results.tasks.len(), 1);
        assert_eq!(results.events[0].title, "Board prep");
        assert_eq!(results.tasks[0].summary, "Board pack");
    }

    /// **The wire shape**: two arrays under `events`/`tasks`, and a task row
    /// that is `task_json`'s whole contract rather than a slimmer hit. The
    /// `{ok, data}` envelope above it is `print_json`'s, shared by every
    /// command and not search's to change — what search owns is what sits under
    /// `data`, and that is what this pins.
    #[tokio::test]
    async fn the_wire_shape_is_two_arrays_and_a_task_row_is_the_whole_contract() {
        let pool = pool_with_two_calendars().await;
        insert(&pool, &row(1, "ev", "Board prep", NOW + DAY)).await;
        insert_task(&pool, &task(1, "tk", "Board pack", "needs-action", Some(NOW))).await;

        let results = search_impl(&pool, "board", NOW).await.unwrap();
        let v = serde_json::to_value(&results).unwrap();

        assert!(v["events"].is_array(), "events under its own key");
        assert!(v["tasks"].is_array(), "tasks under its own key");
        for key in [
            "id", "summary", "notes", "due", "dueMs", "dueAllDay",
            "overdue", "completed", "list", "listId", "canWrite",
        ] {
            assert!(v["tasks"][0].get(key).is_some(), "task row is missing {key}");
        }
    }
}
