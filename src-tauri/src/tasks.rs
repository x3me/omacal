//! The tasks commands — VTODO's face toward the UI.
//!
//! Reads come straight off the store; writes follow the calendar rule this
//! codebase lives by: **the server first, the local row after**, so the app
//! never shows a state the server refused. A completion toggle is a
//! line-surgery rewrite of the task's own resource (`ics::patch_todo_status`)
//! guarded by its etag; a create is a fresh single-VTODO resource guarded by
//! `If-None-Match: *`. Both go through the same client the sync loop uses.

use serde::Serialize;

use crate::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskVm {
    pub id: i64,
    pub calendar_id: i64,
    pub summary: String,
    pub notes: Option<String>,
    pub due_ms: Option<i64>,
    pub due_all_day: bool,
    pub completed: bool,
    /// When it was completed, if the resource says. The Done list's "today"
    /// is a question about this, and a server that set STATUS without
    /// COMPLETED leaves it empty.
    pub completed_ms: Option<i64>,
    pub calendar: String,
    pub color: Option<String>,
    pub priority: i64,
    /// False on read-only lists and in demo mode; the checkbox renders
    /// disabled rather than pretending.
    pub can_write: bool,
}

/// How far back completed tasks stay visible: a week, matching the feed's
/// notion of "recent enough to still matter".
const DONE_WINDOW_MS: i64 = 7 * 24 * 3_600_000;

/// The date an all-day due names: its stored midnight read back in the zone
/// it was resolved in, which is the list's own (`omacal_caldav::resolve`).
/// Unknown zones read as UTC rather than failing, as `due_for` does.
pub(crate) fn due_date(due_utc: i64, due_tz: Option<&str>) -> jiff::civil::Date {
    let tz = due_tz
        .and_then(|z| jiff::tz::TimeZone::get(z).ok())
        .unwrap_or(jiff::tz::TimeZone::UTC);
    jiff::Timestamp::from_millisecond(due_utc)
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
        .to_zoned(tz)
        .date()
}

/// A task's due as the window and the CLI mean it, in `display`.
///
/// A timed due is its instant. An **all-day** due is its *date*, at midnight
/// in the display zone — not the stored instant, which is midnight in the
/// list's zone. Handing that instant over as it was put a task due Thursday
/// on a New York list under Wednesday's column in Sofia, and a save sent
/// Wednesday back to the server: a day lost per edit (found 2026-09-17).
pub(crate) fn display_due_ms(task: &omacal_store::StoredTask, display: &jiff::tz::TimeZone) -> Option<i64> {
    let ms = task.due_utc?;
    if !task.due_all_day {
        return Some(ms);
    }
    let date = due_date(ms, task.due_tz.as_deref());
    Some(
        date.to_zoned(display.clone())
            .map(|z| z.timestamp().as_millisecond())
            .unwrap_or(ms),
    )
}

/// What the store keeps for a due: an instant as itself, a date as midnight
/// in the list's zone — the shape a sync writes, so an edit and the next
/// sync store the same row.
fn stored_due_ms(due: &omacal_caldav::TodoDue, cal_tz: &str) -> Option<i64> {
    match due {
        omacal_caldav::TodoDue::At(ts) => Some(ts.as_millisecond()),
        omacal_caldav::TodoDue::Date(d) => {
            omacal_caldav::resolve(&omacal_caldav::IcsTime::Date(*d), cal_tz).map(|(ms, _, _)| ms)
        }
    }
}

fn to_vm(row: &omacal_store::TaskRow, demo: bool) -> TaskVm {
    TaskVm {
        id: row.task.id,
        calendar_id: row.task.calendar_id,
        summary: row.task.summary.clone().unwrap_or_else(|| "(untitled)".into()),
        notes: row.task.description.clone(),
        due_ms: display_due_ms(&row.task, &jiff::tz::TimeZone::system()),
        due_all_day: row.task.due_all_day,
        completed: row.task.status == "completed",
        completed_ms: row.task.completed_utc,
        calendar: row.calendar_summary.clone(),
        color: row.color_hex.clone(),
        priority: row.task.priority,
        can_write: !demo && row.access_role != "reader",
    }
}

#[tauri::command]
pub async fn list_tasks(state: tauri::State<'_, AppState>) -> Result<Vec<TaskVm>, String> {
    let since = crate::now_ms() - DONE_WINDOW_MS;
    let rows = omacal_store::tasks_for_ui(&state.pool, since)
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    Ok(rows.iter().map(|r| to_vm(r, state.demo)).collect())
}

/// The account credentials behind one task's calendar — the shared
/// per-calendar helper, with a task-flavoured wrapper name kept for the
/// call sites below.
async fn client_for_task_calendar(
    state: &AppState,
    calendar_id: i64,
) -> anyhow::Result<(omacal_caldav::CalDavClient, String, String)> {
    let (client, collection_url) =
        crate::caldav_account::client_for_calendar(state, calendar_id).await?;
    Ok((client, collection_url, String::new()))
}

/// Where a task list lives.
///
/// A Google account brings no tasks, so an install with only Google had a
/// pane that could never hold one (Plamen, 2026-09-16). An **on-this-device**
/// list is the answer: the same rows, the same pane, the same CLI, with
/// nothing on the other end. Every write below asks this first, and the only
/// difference it makes is whether a `PUT` happens — the iCalendar text is
/// written either way, so a list that later moves to a server moves as a
/// copy rather than a rewrite.
enum TaskHome {
    /// A CalDAV collection: its client and the collection's URL.
    Server(Box<omacal_caldav::CalDavClient>, String),
    /// This machine.
    Device,
}

async fn task_home(state: &AppState, calendar_id: i64) -> anyhow::Result<TaskHome> {
    if omacal_store::is_local_calendar(&state.pool, calendar_id).await? {
        return Ok(TaskHome::Device);
    }
    let (client, collection_url, _) = client_for_task_calendar(state, calendar_id).await?;
    Ok(TaskHome::Server(Box::new(client), collection_url))
}

pub(crate) const TASK_CHANGED_ON_SERVER: &str =
    "That task changed on the server since it was loaded — sync and try again";

#[tauri::command]
pub async fn set_task_completed(
    state: tauri::State<'_, AppState>,
    id: i64,
    on: bool,
) -> Result<Vec<TaskVm>, String> {
    crate::demo_sync_guard(state.demo)?;
    set_completed_impl(&state, id, on).await.map_err(|e| crate::errors::user_facing(&e))?;
    list_tasks(state).await
}

async fn set_completed_impl(state: &AppState, id: i64, on: bool) -> anyhow::Result<()> {
    let task = omacal_store::task_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!(TASK_GONE))?;
    let raw = task.raw_ics.as_deref().ok_or_else(|| anyhow::anyhow!("task has no resource"))?;
    let home = task_home(state, task.calendar_id).await?;

    let now = jiff::Timestamp::from_millisecond(crate::now_ms())?;
    let patched = omacal_caldav::patch_todo_status(raw, &task.uid, on, now)
        .ok_or_else(|| anyhow::anyhow!("could not rewrite the task's resource"))?;

    let new_etag = match &home {
        TaskHome::Device => None,
        TaskHome::Server(client, _) => {
            let href = task.caldav_href.as_deref()
                .ok_or_else(|| anyhow::anyhow!("task has no href"))?;
            put_task(client, href, &patched, Written::Existing(task.etag.as_deref())).await?
        }
    };

    let now_ms = crate::now_ms();
    omacal_store::mark_task_status(
        &state.pool,
        id,
        if on { "completed" } else { "needs-action" },
        on.then_some(now_ms),
        new_etag.as_deref(),
        Some(&patched),
        now_ms,
    )
    .await?;
    crate::upcoming::refresh_soon(state.pool.clone(), state.demo);
    Ok(())
}

/// Edits a task: its title, its due date and its note, in one write.
///
/// Every field is the whole answer rather than a change to apply — the
/// editor knows the complete state, so there is no "leave this alone" to
/// get wrong, and clearing a due date is saying `None` rather than
/// omitting it.
///
/// `due_all_day` is the difference between "by Thursday" and "by Thursday
/// at 18:00", and it is the user's distinction, not a storage detail: a
/// date-only due goes on the wire as `VALUE=DATE` and an instant as a UTC
/// stamp. A due date resolves against the *calendar's* zone, the same one
/// the sync reads it back in, so a task does not move a day when the
/// display zone differs.
///
/// `calendar_id` moves the task to another list in the same save; `None`
/// (or its own list) leaves it where it is. See [`move_resource`].
#[tauri::command]
pub async fn update_task(
    state: tauri::State<'_, AppState>,
    id: i64,
    summary: String,
    due_ms: Option<i64>,
    due_all_day: bool,
    notes: Option<String>,
    calendar_id: Option<i64>,
) -> Result<Vec<TaskVm>, String> {
    crate::demo_sync_guard(state.demo)?;
    update_impl(&state, id, &summary, due_ms, due_all_day, notes.as_deref(), calendar_id)
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    list_tasks(state).await
}

#[allow(clippy::too_many_arguments)]
async fn update_impl(
    state: &AppState,
    id: i64,
    summary: &str,
    due_ms: Option<i64>,
    due_all_day: bool,
    notes: Option<&str>,
    to_list: Option<i64>,
) -> anyhow::Result<()> {
    let summary = summary.trim();
    if summary.is_empty() {
        anyhow::bail!(TASK_NEEDS_A_TITLE);
    }
    let notes = notes.map(str::trim).filter(|n| !n.is_empty());

    let task = omacal_store::task_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!(TASK_GONE))?;
    let raw = task.raw_ics.as_deref().ok_or_else(|| anyhow::anyhow!(TASK_GONE))?;
    let to_list = to_list.filter(|&to| to != task.calendar_id);
    // The same refusal a create gets: only a list the window would offer.
    if let Some(to) = to_list {
        if !writable_task_lists(&state.pool).await?.iter().any(|l| l.calendar_id == to) {
            anyhow::bail!(NOT_A_TASK_LIST);
        }
    }
    // The list the task will live on. Its zone is the one a due date is
    // written and read back in, so a moved task is written in its new list's.
    let list = to_list.unwrap_or(task.calendar_id);
    let cal_tz: String = sqlx::query_scalar("SELECT timezone FROM calendars WHERE id = ?1")
        .bind(list)
        .fetch_one(&state.pool)
        .await?;

    let due = due_for(due_ms, due_all_day, &jiff::tz::TimeZone::system())?;
    let now = jiff::Timestamp::from_millisecond(crate::now_ms())?;
    let edit = omacal_caldav::TodoEdit { summary, due, description: notes };
    let patched = omacal_caldav::patch_todo_fields(raw, &task.uid, &edit, &cal_tz, now)
        .ok_or_else(|| anyhow::anyhow!("could not rewrite the task's resource"))?;
    let due_utc = due.as_ref().and_then(|d| stored_due_ms(d, &cal_tz));
    let due_tz = due.is_some().then_some(cal_tz.as_str());

    if let Some(to) = to_list {
        let from = task_home(state, task.calendar_id).await?;
        let dest = task_home(state, to).await?;
        let (href, etag) = move_resource(&from, &dest, &task, &patched).await?;
        omacal_store::move_task(
            &state.pool,
            &omacal_store::StoredTask {
                calendar_id: to,
                etag,
                caldav_href: href,
                summary: Some(summary.to_string()),
                description: notes.map(str::to_string),
                due_utc,
                due_tz: due_tz.map(str::to_string),
                due_all_day,
                updated_at: crate::now_ms(),
                raw_ics: Some(patched),
                ..task
            },
        )
        .await?;
        crate::upcoming::refresh_soon(state.pool.clone(), state.demo);
        return Ok(());
    }

    let new_etag = match task_home(state, task.calendar_id).await? {
        TaskHome::Device => None,
        TaskHome::Server(client, _) => {
            let href = task.caldav_href.as_deref().ok_or_else(|| anyhow::anyhow!(TASK_GONE))?;
            put_task(&client, href, &patched, Written::Existing(task.etag.as_deref())).await?
        }
    };

    omacal_store::update_task_fields(
        &state.pool,
        id,
        summary,
        notes,
        due_utc,
        due_tz,
        due_all_day,
        new_etag.as_deref(),
        &patched,
        crate::now_ms(),
    )
    .await?;
    crate::upcoming::refresh_soon(state.pool.clone(), state.demo);
    Ok(())
}

/// Puts a task's resource on another list and takes it off its own, and
/// answers with where it now lives: its href and etag on the new list, both
/// `None` on this device.
///
/// The copy first, the original after, so a failure part-way never loses the
/// task. A copy that is refused leaves everything as it was. An original that
/// will not come off gets its copy taken back, and the task stays where it
/// was; only when that fails too is the task on both lists, and the message
/// says so rather than letting a second copy turn up on the next sync
/// unexplained. The UID stays the same: a UID only has to be unique within
/// one collection (RFC 4791 §5.3.2.1), and the resource is the same task.
async fn move_resource(
    from: &TaskHome,
    to: &TaskHome,
    task: &omacal_store::StoredTask,
    ics: &str,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    let old_href = match from {
        TaskHome::Device => None,
        TaskHome::Server(..) => {
            Some(task.caldav_href.as_deref().ok_or_else(|| anyhow::anyhow!(TASK_GONE))?)
        }
    };

    let (href, etag) = match to {
        TaskHome::Device => (None, None),
        TaskHome::Server(client, collection_url) => {
            // Named afresh rather than after the UID, which another client
            // may have written with characters a path cannot carry.
            let href = format!(
                "{}/{}.ics",
                collection_url.trim_end_matches('/'),
                uuid::Uuid::new_v4()
            );
            match put_task(client, &href, ics, Written::New).await {
                Ok(etag) => (Some(href), etag),
                Err(e) => {
                    tracing::warn!(error = %format!("{e:#}"), "the new list refused the task's copy");
                    anyhow::bail!(TASK_NOT_MOVED);
                }
            }
        }
    };

    if let (TaskHome::Server(client, _), Some(old)) = (from, old_href) {
        if let Err(e) = client.delete(old, task.etag.as_deref()).await {
            let changed = matches!(e, omacal_caldav::CalDavError::PreconditionFailed);
            tracing::warn!(error = %e, "the old list kept the task; taking the copy back");
            if let (TaskHome::Server(dest, _), Some(copy)) = (to, href.as_deref()) {
                if let Err(undo) = dest.delete(copy, etag.as_deref()).await {
                    tracing::warn!(error = %undo, "could not take the copy back either");
                    anyhow::bail!(TASK_ON_BOTH_LISTS);
                }
            }
            anyhow::bail!(if changed { TASK_CHANGED_ON_SERVER } else { TASK_NOT_MOVED });
        }
    }
    Ok((href, etag))
}

/// Whether a PUT makes a resource or replaces one, and what the row knows.
pub(crate) enum Written<'a> {
    New,
    /// A resource that exists, with the etag the row holds, if any.
    Existing(Option<&'a str>),
}

/// PUTs a task's resource and answers with the etag it now has.
///
/// iCloud often answers a PUT without an ETag. The task row used to keep
/// nothing (a create, an edit) or the *old* etag (a completion), and the
/// second change before the next sync went out as a create or behind a
/// stale `If-Match` — 412, "that task changed on the server", on a task
/// nobody else had touched (found 2026-09-17). So an answer without one is
/// followed by a PROPFIND for it, and a resource whose etag is still unknown
/// is replaced without a guard rather than created. Asking is best-effort:
/// the write itself has already succeeded.
pub(crate) async fn put_task(
    client: &omacal_caldav::CalDavClient,
    href: &str,
    ics: &str,
    written: Written<'_>,
) -> anyhow::Result<Option<String>> {
    let answered = match written {
        Written::New => client.put(href, ics, None).await,
        Written::Existing(Some(etag)) => client.put(href, ics, Some(etag)).await,
        Written::Existing(None) => client.put_unguarded(href, ics).await,
    }
    .map_err(|e| match (e, &written) {
        (omacal_caldav::CalDavError::PreconditionFailed, Written::Existing(_)) => {
            anyhow::anyhow!(TASK_CHANGED_ON_SERVER)
        }
        (other, _) => anyhow::Error::from(other),
    })?;
    if answered.is_some() {
        return Ok(answered);
    }
    Ok(client.etag_of(href).await.ok().flatten())
}

/// The wire's `(ms, all_day)` pair as iCalendar spells it.
///
/// Split out and pure so the one thing worth checking — which zone an
/// all-day due takes its date in — is checkable without a server. It is the
/// **display** zone: the window and the CLI both send a date as midnight
/// there, so that is where the instant names the day the user picked.
/// Reading it in UTC filed a Kolkata task a day early (the class of bug as
/// #44); reading it in the *list's* zone did the same whenever the two
/// zones differ, one day per save (found 2026-09-17).
pub(crate) fn due_for(
    due_ms: Option<i64>,
    all_day: bool,
    display: &jiff::tz::TimeZone,
) -> anyhow::Result<Option<omacal_caldav::TodoDue>> {
    let Some(ms) = due_ms else { return Ok(None) };
    let ts = jiff::Timestamp::from_millisecond(ms)?;
    if !all_day {
        return Ok(Some(omacal_caldav::TodoDue::At(ts)));
    }
    Ok(Some(omacal_caldav::TodoDue::Date(ts.to_zoned(display.clone()).date())))
}

/// The list a task lands on when nobody said: the first one a task can be
/// created on, which is what `task_lists` already means.
pub(crate) async fn first_writable_list(pool: &sqlx::SqlitePool) -> Option<i64> {
    writable_task_lists(pool).await.ok()?.first().map(|l| l.calendar_id)
}

/// The socket's three task verbs, sharing the window's own write path —
/// one code path, the same guards, as the events side does.
pub(crate) async fn create_body(
    state: &AppState,
    calendar_id: i64,
    summary: &str,
    due_ms: Option<i64>,
    due_all_day: bool,
) -> Result<Vec<TaskVm>, String> {
    crate::demo_sync_guard(state.demo)?;
    create_impl(state, calendar_id, summary, due_ms, due_all_day)
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    Ok(list_body(state).await)
}

pub(crate) async fn complete_body(state: &AppState, id: i64, done: bool) -> Result<Vec<TaskVm>, String> {
    crate::demo_sync_guard(state.demo)?;
    set_completed_impl(state, id, done)
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    Ok(list_body(state).await)
}

pub(crate) async fn update_body(
    state: &AppState,
    id: i64,
    summary: &str,
    due_ms: Option<i64>,
    due_all_day: bool,
    notes: Option<&str>,
) -> Result<Vec<TaskVm>, String> {
    crate::demo_sync_guard(state.demo)?;
    update_impl(state, id, summary, due_ms, due_all_day, notes, None)
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    Ok(list_body(state).await)
}

/// `list_tasks` without the Tauri wrapper, for the socket.
pub(crate) async fn list_body(state: &AppState) -> Vec<TaskVm> {
    let since = crate::now_ms() - DONE_WINDOW_MS;
    omacal_store::tasks_for_ui(&state.pool, since)
        .await
        .map(|rows| rows.iter().map(|r| to_vm(r, state.demo)).collect())
        .unwrap_or_default()
}

pub(crate) const TASK_NEEDS_A_TITLE: &str = "a task needs a title";
/// A move that did not happen: the copy was refused, or the original would
/// not come off its list and the copy was taken back.
pub(crate) const TASK_NOT_MOVED: &str =
    "The task could not be moved to that list — it is still on the old one.";
/// A move that half happened, which the next sync would otherwise show as a
/// second copy with no word of why. Worded as `events::MOVED_NOT_REMOVED` is,
/// for the same situation.
pub(crate) const TASK_ON_BOTH_LISTS: &str =
    "The task is now on the list you chose, but OmaCal could not take it off the \
     old one — it is on both. Delete the copy on the old list.";
pub(crate) const TASK_GONE: &str = "that task is no longer here";
/// `list_name`'s three refusals. Named so they can be allow-listed: each
/// says what to do about a name the user just typed, and OPAQUE for a
/// 61-character list name would report a sync fault for a typo.
///
/// **The collision one names the list**, and so is a `SAFE_PREFIXES` entry
/// rather than an exact one: the trailing detail is a list name the user or
/// their server chose, which is the "variable, benign" case that list is for.
/// Naming it was deliberate — `lists_on_this_device_are_created_renamed_and_
/// deleted` asserts the name appears — and two lists whose collision is with
/// one the user cannot see is exactly when the name earns its place.
pub(crate) const LIST_NEEDS_A_NAME: &str = "a list needs a name";
pub(crate) const LIST_NAME_TOO_LONG: &str = "a list name can be up to 60 characters";
pub(crate) const LIST_NAME_TAKEN: &str = "there is already a list called ";
pub(crate) const NOT_A_TASK_LIST: &str =
    "that is not a task list you can add to — `omacal tasks` names the lists in each row";

/// `due_all_day` is `update_task`'s distinction: a date, or an hour on it.
/// Left out, a due is a date — the add row's "by Friday" — and a list's own
/// new line sends `false` when it was given a time.
#[tauri::command]
pub async fn create_task(
    state: tauri::State<'_, AppState>,
    calendar_id: i64,
    summary: String,
    due_ms: Option<i64>,
    due_all_day: Option<bool>,
) -> Result<Vec<TaskVm>, String> {
    crate::demo_sync_guard(state.demo)?;
    create_impl(&state, calendar_id, &summary, due_ms, due_all_day.unwrap_or(true))
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    list_tasks(state).await
}

async fn create_impl(
    state: &AppState,
    calendar_id: i64,
    summary: &str,
    due_ms: Option<i64>,
    all_day: bool,
) -> anyhow::Result<()> {
    // Only a list the window would offer: the socket names a list by id, and
    // an id that is an events-only collection, a hidden list or a read-only
    // one would take a task nobody can see or keep (found 2026-09-17).
    if !writable_task_lists(&state.pool).await?.iter().any(|l| l.calendar_id == calendar_id) {
        anyhow::bail!(NOT_A_TASK_LIST);
    }
    let summary = summary.trim();
    if summary.is_empty() {
        anyhow::bail!(TASK_NEEDS_A_TITLE);
    }
    let home = task_home(state, calendar_id).await?;
    let cal_tz: String = sqlx::query_scalar("SELECT timezone FROM calendars WHERE id = ?1")
        .bind(calendar_id)
        .fetch_one(&state.pool)
        .await?;

    let uid = uuid::Uuid::new_v4().to_string();
    let now = jiff::Timestamp::from_millisecond(crate::now_ms())?;
    // The window's quick-add says dates, not instants: "by Friday", not
    // "by 16:23:07". The CLI can say an hour, and then it means one.
    let due_time = due_ms.and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok()).map(|ts| {
        let tz = jiff::tz::TimeZone::get(&cal_tz).unwrap_or(jiff::tz::TimeZone::UTC);
        let z = ts.to_zoned(tz);
        if all_day {
            // The date in the display zone, for `due_for`'s reason.
            omacal_caldav::IcsTime::Date(ts.to_zoned(jiff::tz::TimeZone::system()).date())
        } else {
            // In the calendar's zone, not as a bare UTC instant (issue
            // #102): `cal_tz` was already being handed to `new_todo_ics`
            // and ignored there, which is the shape of a wire that was
            // meant to be connected and never was.
            omacal_caldav::IcsTime::Zoned { dt: z.datetime(), tzid: cal_tz.clone() }
        }
    });
    let ics = omacal_caldav::new_todo_ics(&uid, summary, due_time.as_ref(), now);

    // On this device there is no resource to address, so the row carries no
    // href — which is also what every write above reads it as.
    let (href, new_etag) = match &home {
        TaskHome::Device => (None, None),
        TaskHome::Server(client, collection_url) => {
            let href = format!("{}/{uid}.ics", collection_url.trim_end_matches('/'));
            let etag = put_task(client, &href, &ics, Written::New).await?;
            (Some(href), etag)
        }
    };

    let now_ms = crate::now_ms();
    let due = due_time.as_ref().and_then(|t| omacal_caldav::resolve(t, &cal_tz));
    omacal_store::upsert_task(
        &state.pool,
        &omacal_store::StoredTask {
            id: 0,
            calendar_id,
            uid,
            etag: new_etag,
            caldav_href: href,
            summary: Some(summary.to_string()),
            description: None,
            due_utc: due.as_ref().map(|(ms, _, _)| *ms),
            due_tz: due.as_ref().map(|(_, tz, _)| tz.clone()),
            // Whether the resource says a date or an instant — not whether
            // there is a due at all, which stored every CLI `--at` on this
            // device as all-day (found 2026-09-17).
            due_all_day: due.as_ref().is_some_and(|(_, _, date)| *date),
            status: "needs-action".into(),
            completed_utc: None,
            priority: 0,
            updated_at: now_ms,
            raw_ics: Some(ics),
        },
    )
    .await?;
    crate::upcoming::refresh_soon(state.pool.clone(), state.demo);
    Ok(())
}

#[tauri::command]
pub async fn delete_task_cmd(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<Vec<TaskVm>, String> {
    crate::demo_sync_guard(state.demo)?;
    delete_impl(&state, id).await.map_err(|e| crate::errors::user_facing(&e))?;
    list_tasks(state).await
}

async fn delete_impl(state: &AppState, id: i64) -> anyhow::Result<()> {
    let task = omacal_store::task_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!(TASK_GONE))?;
    if let TaskHome::Server(client, _) = task_home(state, task.calendar_id).await? {
        let href = task.caldav_href.as_deref()
            .ok_or_else(|| anyhow::anyhow!("task has no href"))?;
        client.delete(href, task.etag.as_deref()).await.map_err(anyhow::Error::from)?;
    }
    omacal_store::delete_task(&state.pool, id).await?;
    crate::upcoming::refresh_soon(state.pool.clone(), state.demo);
    Ok(())
}

/// One page of the Done list's history.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DonePage {
    pub tasks: Vec<TaskVm>,
    /// Whether another page follows this one.
    pub more: bool,
}

/// Whether a task matches what was typed into the Done list's search.
///
/// Every word must appear, in the title or the note, in any order and any
/// case — "bank call" finds "Call the bank". Case is folded by Rust, not by
/// SQLite, whose `LOWER` knows ASCII only: a list kept in Bulgarian has to
/// search like one kept in English. An empty query matches everything.
pub(crate) fn matches_query(summary: &str, notes: Option<&str>, query: &str) -> bool {
    let hay = format!("{}\n{}", summary, notes.unwrap_or("")).to_lowercase();
    query.split_whitespace().all(|word| hay.contains(&word.to_lowercase()))
}

/// Completed tasks older than `before_ms`, newest first, filtered by `query`
/// and cut into pages — the Done list's "earlier", which the window asks for
/// only when somebody opens it.
#[tauri::command]
pub async fn search_done_tasks(
    state: tauri::State<'_, AppState>,
    query: String,
    before_ms: Option<i64>,
    offset: usize,
    limit: usize,
) -> Result<DonePage, String> {
    done_page(&state, &query, before_ms, offset, limit)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

pub(crate) async fn done_page(
    state: &AppState,
    query: &str,
    before_ms: Option<i64>,
    offset: usize,
    limit: usize,
) -> anyhow::Result<DonePage> {
    let rows = omacal_store::completed_tasks_for_ui(&state.pool, before_ms).await?;
    let mut hits = rows.iter().filter(|r| {
        matches_query(
            r.task.summary.as_deref().unwrap_or(""),
            r.task.description.as_deref(),
            query,
        )
    });
    let limit = limit.clamp(1, 200);
    let tasks: Vec<TaskVm> = hits.by_ref().skip(offset).take(limit).map(|r| to_vm(r, state.demo)).collect();
    let more = hits.next().is_some();
    Ok(DonePage { tasks, more })
}

/// The task-capable, writable lists the quick-add can land on.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskListVm {
    pub calendar_id: i64,
    pub name: String,
    pub color: Option<String>,
    /// Kept on this machine rather than on a server: the lists the Tasks
    /// pane can rename and delete.
    pub local: bool,
}

/// The task-capable, writable lists, in the order the picker offers them.
/// One query, two callers: the window's command and the socket's "no list
/// was named" default, so the CLI can never land a task somewhere the
/// window would not offer.
///
/// WebCal feeds are events-only (`supports_tasks = 0`) and `reader`, so the
/// provider list needs no `webcal` entry — the two guards below already
/// exclude them. Google has no task lists at all.
pub(crate) async fn writable_task_lists(
    pool: &sqlx::SqlitePool,
) -> anyhow::Result<Vec<TaskListVm>> {
    let rows: Vec<(i64, String, Option<String>, String)> = sqlx::query_as(
        "SELECT c.id, COALESCE(c.label_override, c.summary), COALESCE(c.color_override, c.color_hex),
                a.provider
         FROM calendars c JOIN accounts a ON a.id = c.account_id
         WHERE a.provider IN ('caldav', 'local') AND c.supports_tasks = 1
           AND c.selected = 1 AND c.access_role != 'reader'
         ORDER BY COALESCE(c.label_override, c.summary) COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(calendar_id, name, color, provider)| TaskListVm {
            calendar_id,
            name,
            color,
            local: provider == omacal_store::LOCAL_PROVIDER,
        })
        .collect())
}

/// Creates the on-this-device task list, or finds the one already there,
/// and answers with the lists as the pickers see them.
///
/// The button behind this exists because a Google-only install has no task
/// list at all and no way to make one: Google keeps tasks in another product
/// with another API, and asking for a CalDAV account to write a shopping
/// list is asking for a server nobody wanted (Plamen, 2026-09-16).
///
/// The list's zone is the display zone, which is what a due date means here:
/// "by Thursday" is Thursday where the user is.
#[tauri::command]
pub async fn create_local_task_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<TaskListVm>, String> {
    crate::demo_sync_guard(state.demo)?;
    let tz = crate::settings::read_settings(&state.pool)
        .await
        .display_timezone
        .unwrap_or_else(|| jiff::tz::TimeZone::system().iana_name().unwrap_or("UTC").to_string());
    omacal_store::ensure_local_task_list(&state.pool, LOCAL_TASK_LIST_NAME, &tz, crate::now_ms())
        .await
        .map_err(|e| crate::errors::user_facing(&e))?;
    writable_task_lists(&state.pool)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

/// The colours a new list is given, in order: `theme.ts`'s
/// `CALENDAR_COLOURS`, the swatches Settings offers for any calendar.
const LIST_COLOURS: &[&str] = &[
    "#5b8def", "#2aa198", "#5aa84f", "#8a9a3b", "#e2a03f",
    "#e07b39", "#e2564a", "#e06c9f", "#9a7bd0", "#7b8a9a",
];

/// The first swatch no list is wearing yet, so two lists made in a row can be
/// told apart on the grid; past ten lists, round again.
fn next_list_colour(lists: &[TaskListVm]) -> &'static str {
    let used: Vec<String> = lists.iter().filter_map(|l| l.color.as_deref().map(str::to_ascii_lowercase)).collect();
    LIST_COLOURS
        .iter()
        .find(|c| !used.contains(&c.to_string()))
        .copied()
        .unwrap_or(LIST_COLOURS[lists.len() % LIST_COLOURS.len()])
}

/// A list name as given, or the reason it cannot be one: blank, too long, or
/// the name of another list already — two "Groceries" in the pickers would
/// leave nobody sure which one a task lands on.
fn list_name(name: &str, lists: &[TaskListVm], except: Option<i64>) -> anyhow::Result<String> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!(LIST_NEEDS_A_NAME);
    }
    if name.chars().count() > 60 {
        anyhow::bail!(LIST_NAME_TOO_LONG);
    }
    let lower = name.to_lowercase();
    if lists.iter().any(|l| Some(l.calendar_id) != except && l.name.to_lowercase() == lower) {
        anyhow::bail!("{LIST_NAME_TAKEN}{name}");
    }
    Ok(name.to_string())
}

/// Makes another task list on this device (2026-09-17, Plamen: lists for
/// the tasks that sync with nothing). Answers with the lists as the pickers
/// see them, the new one among them.
#[tauri::command]
pub async fn create_task_list(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<Vec<TaskListVm>, String> {
    crate::demo_sync_guard(state.demo)?;
    create_list_impl(&state, &name).await.map_err(|e| crate::errors::user_facing(&e))
}

pub(crate) async fn create_list_impl(state: &AppState, name: &str) -> anyhow::Result<Vec<TaskListVm>> {
    let lists = writable_task_lists(&state.pool).await?;
    let name = list_name(name, &lists, None)?;
    let tz = crate::settings::read_settings(&state.pool)
        .await
        .display_timezone
        .unwrap_or_else(|| jiff::tz::TimeZone::system().iana_name().unwrap_or("UTC").to_string());
    omacal_store::create_local_list(&state.pool, &name, &tz, next_list_colour(&lists), crate::now_ms()).await?;
    writable_task_lists(&state.pool).await
}

/// Renames a list on this device. A server's list keeps the name its server
/// gives it; Settings → Calendars can still label it locally.
#[tauri::command]
pub async fn rename_task_list(
    state: tauri::State<'_, AppState>,
    id: i64,
    name: String,
) -> Result<Vec<TaskListVm>, String> {
    crate::demo_sync_guard(state.demo)?;
    rename_list_impl(&state, id, &name).await.map_err(|e| crate::errors::user_facing(&e))
}

pub(crate) async fn rename_list_impl(state: &AppState, id: i64, name: &str) -> anyhow::Result<Vec<TaskListVm>> {
    let lists = writable_task_lists(&state.pool).await?;
    let name = list_name(name, &lists, Some(id))?;
    omacal_store::rename_local_list(&state.pool, id, &name).await?;
    crate::upcoming::refresh_soon(state.pool.clone(), state.demo);
    writable_task_lists(&state.pool).await
}

/// Deletes a list on this device and its tasks. The pane asks first, naming
/// how many tasks go with it.
#[tauri::command]
pub async fn delete_task_list(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<Vec<TaskListVm>, String> {
    crate::demo_sync_guard(state.demo)?;
    delete_list_impl(&state, id).await.map_err(|e| crate::errors::user_facing(&e))
}

pub(crate) async fn delete_list_impl(state: &AppState, id: i64) -> anyhow::Result<Vec<TaskListVm>> {
    omacal_store::delete_local_list(&state.pool, id).await?;
    crate::upcoming::refresh_soon(state.pool.clone(), state.demo);
    writable_task_lists(&state.pool).await
}

/// What the on-this-device list is called. Plain, because the pane already
/// says these are tasks and the account row beside it says where they live.
pub(crate) const LOCAL_TASK_LIST_NAME: &str = "Tasks on this device";

#[tauri::command]
pub async fn task_lists(state: tauri::State<'_, AppState>) -> Result<Vec<TaskListVm>, String> {
    writable_task_lists(&state.pool)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_state(pool: sqlx::SqlitePool) -> AppState {
        AppState {
            pool,
            demo: false,
            tokens: Default::default(),
            reauth: Default::default(),
            update: Default::default(),
            update_checked_at: Default::default(),
            system_tz_change: Default::default(),
            quit_on_close: Default::default(),
            open_date: Default::default(),
        }
    }

    /// **A task list with no server behind it** (Plamen, 2026-09-16): the
    /// whole life of one, through the same commands a CalDAV list uses. If
    /// any of them reached for a client this would fail here rather than in
    /// somebody's pane — there is no server, no credential and no keyring
    /// entry anywhere in this test.
    #[tokio::test]
    async fn a_task_on_this_device_is_created_edited_completed_and_deleted_without_a_server() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let list =
            omacal_store::ensure_local_task_list(&pool, "Tasks on this device", "Europe/Sofia", 0)
                .await
                .unwrap();
        let state = local_state(pool.clone());

        create_impl(&state, list, "Water the plants", None, true).await.unwrap();
        let rows = omacal_store::tasks_for_ui(&pool, 0).await.unwrap();
        assert_eq!(rows.len(), 1, "the pane shows it, like any other list's task");
        let task = &rows[0].task;
        let id = task.id;
        assert_eq!(task.summary.as_deref(), Some("Water the plants"));
        // No resource to address, and the iCalendar text kept anyway — which
        // is what lets this list move to a server later as a copy.
        assert!(task.caldav_href.is_none(), "nothing to address on this device");
        assert!(task.etag.is_none());
        assert!(task.raw_ics.as_deref().is_some_and(|r| r.contains("BEGIN:VTODO")));

        // A due date, a note and a new title, in one write.
        let due: i64 = "2026-09-18T00:00:00+03:00".parse::<jiff::Timestamp>().unwrap().as_millisecond();
        update_impl(&state, id, "Water the plants twice", Some(due), true, Some("the big one"), None)
            .await
            .unwrap();
        let t = omacal_store::task_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(t.summary.as_deref(), Some("Water the plants twice"));
        assert_eq!(t.description.as_deref(), Some("the big one"));
        assert!(t.due_utc.is_some() && t.due_all_day);

        set_completed_impl(&state, id, true).await.unwrap();
        let t = omacal_store::task_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(t.status, "completed");
        assert!(t.completed_utc.is_some());
        set_completed_impl(&state, id, false).await.unwrap();
        assert_eq!(
            omacal_store::task_by_id(&pool, id).await.unwrap().unwrap().status,
            "needs-action"
        );

        delete_impl(&state, id).await.unwrap();
        assert!(omacal_store::task_by_id(&pool, id).await.unwrap().is_none());
    }

    /// iCloud's shape: PUTs answered without an ETag. Each write reads the
    /// etag back and the next one is guarded by it; a row that still knows
    /// none replaces the resource rather than trying to create it again.
    #[tokio::test]
    async fn a_server_that_answers_puts_without_an_etag_takes_the_second_change_too() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let etag_is = |e: &str| {
            ResponseTemplate::new(207).set_body_string(format!(
                r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"><d:response><d:href>/l/t.ics</d:href><d:propstat><d:prop><d:getetag>{e}</d:getetag></d:prop></d:propstat></d:response></d:multistatus>"#
            ))
        };
        Mock::given(method("PROPFIND")).and(path("/l/t.ics")).respond_with(etag_is("\"v1\"")).mount(&server).await;
        Mock::given(method("PUT")).and(path("/l/t.ics")).and(header("If-None-Match", "*"))
            .respond_with(ResponseTemplate::new(201)).expect(1).mount(&server).await;
        Mock::given(method("PUT")).and(path("/l/t.ics")).and(header("If-Match", "\"v1\""))
            .respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;
        Mock::given(method("PUT")).and(path("/l/t.ics")).respond_with(ResponseTemplate::new(204))
            .with_priority(10).mount(&server).await;

        let client = omacal_caldav::CalDavClient::new(&server.uri(), "u", "p").unwrap();
        let href = format!("{}/l/t.ics", server.uri());
        let created = put_task(&client, &href, "a", Written::New).await.unwrap();
        assert_eq!(created.as_deref(), Some("\"v1\""), "asked for, since the PUT did not say");
        let completed = put_task(&client, &href, "b", Written::Existing(created.as_deref())).await.unwrap();
        assert_eq!(completed.as_deref(), Some("\"v1\""));

        // A row that knows no etag at all: replaced, never re-created.
        put_task(&client, &href, "c", Written::Existing(None)).await.unwrap();
        let unguarded = server.received_requests().await.unwrap().into_iter()
            .filter(|r| r.method.as_str() == "PUT")
            .filter(|r| r.headers.get("If-Match").is_none() && r.headers.get("If-None-Match").is_none())
            .count();
        assert_eq!(unguarded, 1);
    }

    /// The Done list's search: every word, anywhere, any case — including
    /// the cases SQLite's `LOWER` would miss.
    #[test]
    fn done_search_matches_every_word_in_the_title_or_the_note_in_any_case() {
        assert!(matches_query("Call the bank", None, ""), "an empty search is everything");
        assert!(matches_query("Call the bank", None, "  "), "and so is a blank one");
        assert!(matches_query("Call the bank", None, "BANK call"), "any order, any case");
        assert!(matches_query("Pay rent", Some("to the landlord"), "rent landlord"), "the note counts");
        assert!(!matches_query("Call the bank", None, "bank rent"), "every word, not any word");
        // Cyrillic: `LOWER('Обади')` in SQLite is still 'Обади'.
        assert!(matches_query("Обади се на банката", None, "БАНКАТА"));
        assert!(matches_query("Élise's birthday", None, "élise"));
    }

    /// Earlier done tasks come in pages, newest first, never repeating what
    /// the window already shows as today's.
    #[tokio::test]
    async fn done_pages_are_newest_first_filtered_and_say_whether_more_follow() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let list =
            omacal_store::ensure_local_task_list(&pool, LOCAL_TASK_LIST_NAME, "UTC", 0).await.unwrap();
        let state = local_state(pool.clone());
        for title in ["Pay rent", "Call the bank", "Book flights", "Call mum", "Today's one"] {
            create_impl(&state, list, title, None, true).await.unwrap();
        }
        // Completed a day apart, in the order created; the last one "today".
        let rows = omacal_store::tasks_for_ui(&pool, 0).await.unwrap();
        for r in &rows {
            let day = match r.task.summary.as_deref() {
                Some("Pay rent") => 1,
                Some("Call the bank") => 2,
                Some("Book flights") => 3,
                Some("Call mum") => 4,
                _ => 10,
            };
            omacal_store::mark_task_status(&pool, r.task.id, "completed", Some(day * 86_400_000), None, None, 0)
                .await
                .unwrap();
        }
        let today = 10 * 86_400_000;
        let titles = |p: &DonePage| p.tasks.iter().map(|t| t.summary.clone()).collect::<Vec<_>>();

        let first = done_page(&state, "", Some(today), 0, 3).await.unwrap();
        assert_eq!(titles(&first), vec!["Call mum", "Book flights", "Call the bank"]);
        assert!(first.more);
        assert!(first.tasks.iter().all(|t| t.completed && t.completed_ms.is_some()));
        let second = done_page(&state, "", Some(today), 3, 3).await.unwrap();
        assert_eq!(titles(&second), vec!["Pay rent"]);
        assert!(!second.more, "the last page says so");

        let calls = done_page(&state, "call", Some(today), 0, 10).await.unwrap();
        assert_eq!(titles(&calls), vec!["Call mum", "Call the bank"]);
        assert!(!calls.more);
        let everything = done_page(&state, "", None, 0, 10).await.unwrap();
        assert_eq!(everything.tasks.len(), 5, "no cutoff includes today's");
    }

    /// A task goes only where the window would offer to put it.
    #[tokio::test]
    async fn a_task_is_refused_on_a_list_that_is_not_one() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let list = omacal_store::ensure_local_task_list(&pool, LOCAL_TASK_LIST_NAME, "UTC", 0).await.unwrap();
        let state = local_state(pool.clone());
        let refused = |e: anyhow::Error| e.to_string() == NOT_A_TASK_LIST;

        assert!(create_impl(&state, 9_999, "Nowhere", None, true).await.is_err_and(refused), "no such list");
        for (setting, undo) in [
            ("UPDATE calendars SET supports_tasks = 0 WHERE id = ?1", "UPDATE calendars SET supports_tasks = 1 WHERE id = ?1"),
            ("UPDATE calendars SET selected = 0 WHERE id = ?1", "UPDATE calendars SET selected = 1 WHERE id = ?1"),
            ("UPDATE calendars SET access_role = 'reader' WHERE id = ?1", "UPDATE calendars SET access_role = 'owner' WHERE id = ?1"),
        ] {
            sqlx::query(setting).bind(list).execute(&pool).await.unwrap();
            assert!(create_impl(&state, list, "Hidden", None, true).await.is_err_and(refused), "{setting}");
            sqlx::query(undo).bind(list).execute(&pool).await.unwrap();
        }
        assert!(omacal_store::tasks_for_ui(&pool, 0).await.unwrap().is_empty(), "nothing was written");
        create_impl(&state, list, "Here", None, true).await.unwrap();
    }

    /// "New list" on this device: named, coloured apart, renamed and deleted
    /// — and refused where a name would be confusing or the list is a
    /// server's.
    #[tokio::test]
    async fn lists_on_this_device_are_created_renamed_and_deleted() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let state = local_state(pool.clone());
        let default = omacal_store::ensure_local_task_list(&pool, LOCAL_TASK_LIST_NAME, "UTC", 0).await.unwrap();

        let lists = create_list_impl(&state, "  Groceries ").await.unwrap();
        let groceries = lists.iter().find(|l| l.name == "Groceries").expect("trimmed and offered");
        assert!(groceries.local);
        let lists = create_list_impl(&state, "Books").await.unwrap();
        let colours: Vec<_> = lists.iter().map(|l| l.color.clone()).collect();
        assert_eq!(lists.len(), 3);
        assert_eq!(
            colours.iter().filter(|c| c.is_some()).count(),
            2,
            "the default list has no colour of its own; the new ones each get one"
        );
        assert_ne!(colours[0], colours[1], "two new lists are told apart: {colours:?}");

        let refused = |r: anyhow::Result<Vec<TaskListVm>>| r.unwrap_err().to_string();
        assert_eq!(refused(create_list_impl(&state, "   ").await), "a list needs a name");
        assert_eq!(refused(create_list_impl(&state, "groceries").await), "there is already a list called groceries");
        assert!(refused(create_list_impl(&state, &"x".repeat(61)).await).contains("60"));

        let id = groceries.calendar_id;
        // A rename may keep its own name in another case, not another list's.
        let lists = rename_list_impl(&state, id, "GROCERIES").await.unwrap();
        assert!(lists.iter().any(|l| l.calendar_id == id && l.name == "GROCERIES"));
        assert!(rename_list_impl(&state, id, "Books").await.is_err());

        // A task on it goes with it.
        create_impl(&state, id, "Milk", None, true).await.unwrap();
        let lists = delete_list_impl(&state, id).await.unwrap();
        assert!(!lists.iter().any(|l| l.calendar_id == id));
        assert!(omacal_store::tasks_for_ui(&pool, 0).await.unwrap().is_empty());

        // The default list is a list like the others.
        delete_list_impl(&state, default).await.unwrap();
        assert_eq!(writable_task_lists(&pool).await.unwrap().len(), 1);
    }

    /// The list is offered to the pickers, and making it twice makes one.
    #[tokio::test]
    async fn the_on_device_list_is_offered_once_however_often_it_is_asked_for() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let first =
            omacal_store::ensure_local_task_list(&pool, LOCAL_TASK_LIST_NAME, "UTC", 0).await.unwrap();
        let again =
            omacal_store::ensure_local_task_list(&pool, LOCAL_TASK_LIST_NAME, "UTC", 1).await.unwrap();
        assert_eq!(first, again, "one list, not two");

        let lists = writable_task_lists(&pool).await.unwrap();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].calendar_id, first);
        assert_eq!(lists[0].name, LOCAL_TASK_LIST_NAME);
        assert!(omacal_store::is_local_calendar(&pool, first).await.unwrap());
    }

    /// An all-day due date takes its date in the **display** zone, where the
    /// window and the CLI put its midnight.
    ///
    /// The case that bit #44 still holds: in Kolkata a task due at 00:30
    /// local is 19:00 UTC the day before, so reading the UTC date would file
    /// it a day early. A timed due is the instant itself and has no such
    /// question.
    #[test]
    fn an_all_day_due_takes_its_date_in_the_display_zone() {
        // 2026-09-11T00:30 in Asia/Kolkata is 2026-09-10T19:00Z.
        let ms: i64 = "2026-09-10T19:00:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond();
        let zone = |z: &str| jiff::tz::TimeZone::get(z).unwrap();

        let kolkata = due_for(Some(ms), true, &zone("Asia/Kolkata")).unwrap();
        assert_eq!(kolkata, Some(omacal_caldav::TodoDue::Date(jiff::civil::date(2026, 9, 11))));
        let utc = due_for(Some(ms), true, &jiff::tz::TimeZone::UTC).unwrap();
        assert_eq!(utc, Some(omacal_caldav::TodoDue::Date(jiff::civil::date(2026, 9, 10))));

        // A timed due is the instant, whatever the zone.
        let at = due_for(Some(ms), false, &zone("Asia/Kolkata")).unwrap();
        assert_eq!(at, Some(omacal_caldav::TodoDue::At(jiff::Timestamp::from_millisecond(ms).unwrap())));

        // No date is no date, not an instant at zero.
        assert_eq!(due_for(None, true, &zone("Asia/Kolkata")).unwrap(), None);
        assert_eq!(due_for(None, false, &jiff::tz::TimeZone::UTC).unwrap(), None);
    }

    /// The audit's scenario, both ways round: the display zone and the
    /// list's zone differ, and an all-day due must name the same date read
    /// back, and written back, however often it is saved.
    #[test]
    fn an_all_day_due_keeps_its_date_across_differing_zones() {
        let zone = |z: &str| jiff::tz::TimeZone::get(z).unwrap();
        for (display, list) in [("Europe/Sofia", "America/New_York"), ("America/New_York", "Europe/Sofia")] {
            let thursday = jiff::civil::date(2026, 9, 17);
            // What a sync stores for `DUE;VALUE=DATE:20260917` on that list.
            let (stored, tz, _) =
                omacal_caldav::resolve(&omacal_caldav::IcsTime::Date(thursday), list).unwrap();
            let mut task = omacal_store::StoredTask {
                id: 1, calendar_id: 1, uid: "u".into(), etag: None, caldav_href: None,
                summary: Some("x".into()), description: None, due_utc: Some(stored),
                due_tz: Some(tz), due_all_day: true, status: "needs-action".into(),
                completed_utc: None, priority: 0, updated_at: 0, raw_ics: None,
            };
            for round in 0..3 {
                // What the window is handed: Thursday's midnight where it is.
                let shown = display_due_ms(&task, &zone(display)).unwrap();
                let midnight = thursday.to_zoned(zone(display)).unwrap().timestamp().as_millisecond();
                assert_eq!(shown, midnight, "{display} over a {list} list, round {round}: shown");
                // And what a save of that, unchanged, sends and stores.
                let due = due_for(Some(shown), true, &zone(display)).unwrap().unwrap();
                assert_eq!(due, omacal_caldav::TodoDue::Date(thursday), "{display} over {list}, round {round}: written");
                task.due_utc = stored_due_ms(&due, list);
            }
            assert_eq!(task.due_utc, Some(stored), "a save stores what a sync would");
        }
    }

    /// Found 2026-09-17: a timed due created on this device came back all-day.
    #[tokio::test]
    async fn a_timed_task_created_on_this_device_keeps_its_hour() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let list = omacal_store::ensure_local_task_list(&pool, LOCAL_TASK_LIST_NAME, "Europe/Sofia", 0)
            .await
            .unwrap();
        let state = local_state(pool.clone());
        let at: i64 = "2026-09-18T07:00:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond();
        create_impl(&state, list, "Call the bank", Some(at), false).await.unwrap();
        let row = &omacal_store::tasks_for_ui(&pool, 0).await.unwrap()[0].task;
        assert!(!row.due_all_day, "an hour was given, so it is not all-day");
        assert_eq!(row.due_utc, Some(at));
        assert!(row.raw_ics.as_deref().is_some_and(|r| r.contains("DUE;TZID=Europe/Sofia:20260918T100000")));
    }

    /// **A task moves to another list in the save that edits it** (Plamen,
    /// 2026-09-18: "how to change it from one list to another?"). The row
    /// keeps its id, and an all-day due is written in the new list's zone, so
    /// it names the same day there as it did before.
    #[tokio::test]
    async fn a_task_moves_between_lists_on_this_device_and_keeps_its_day() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let home = omacal_store::ensure_local_task_list(&pool, LOCAL_TASK_LIST_NAME, "Europe/Sofia", 0)
            .await
            .unwrap();
        let errands = omacal_store::create_local_list(&pool, "Errands", "America/New_York", "#5aa84f", 0)
            .await
            .unwrap();
        let state = local_state(pool.clone());
        create_impl(&state, home, "Buy stamps", None, true).await.unwrap();
        let id = omacal_store::tasks_for_ui(&pool, 0).await.unwrap()[0].task.id;

        let due: i64 = "2026-09-24T12:00:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond();
        let day = jiff::Timestamp::from_millisecond(due).unwrap().to_zoned(jiff::tz::TimeZone::system()).date();
        update_impl(&state, id, "Buy stamps", Some(due), true, Some("two books"), Some(errands))
            .await
            .unwrap();
        let t = omacal_store::task_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(t.calendar_id, errands, "on the new list, as the same row");
        assert_eq!(t.description.as_deref(), Some("two books"), "and the edit came with it");
        assert_eq!(t.due_tz.as_deref(), Some("America/New_York"));
        assert_eq!(due_date(t.due_utc.unwrap(), t.due_tz.as_deref()), day, "the same day, in its new zone");
        assert!(t.caldav_href.is_none() && t.etag.is_none(), "still nothing to address");
        assert_eq!(omacal_store::tasks_for_ui(&pool, 0).await.unwrap().len(), 1, "moved, not copied");

        // A list that is not one is refused, and the task stays put.
        let refused = update_impl(&state, id, "Buy stamps", None, true, None, Some(9_999)).await.unwrap_err();
        assert_eq!(refused.to_string(), NOT_A_TASK_LIST);
        assert_eq!(omacal_store::task_by_id(&pool, id).await.unwrap().unwrap().calendar_id, errands);
    }

    /// `move_resource` against a CalDAV server, one per way it can go. The
    /// server is a wiremock with two collections, `/a/` and `/b/`.
    mod moving {
        use super::super::*;
        use wiremock::matchers::{header, method, path, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        fn on_server(server: &MockServer, collection: &str) -> TaskHome {
            let client = omacal_caldav::CalDavClient::new(&server.uri(), "u", "p").unwrap();
            TaskHome::Server(Box::new(client), format!("{}/{collection}/", server.uri()))
        }

        fn task_at(server: &MockServer) -> omacal_store::StoredTask {
            omacal_store::StoredTask {
                id: 7, calendar_id: 1, uid: "task-1@example.com".into(),
                etag: Some("\"old\"".into()), caldav_href: Some(format!("{}/a/t1.ics", server.uri())),
                summary: Some("Buy stamps".into()), description: None, due_utc: None, due_tz: None,
                due_all_day: false, status: "needs-action".into(), completed_utc: None, priority: 0,
                updated_at: 0, raw_ics: None,
            }
        }

        async fn methods(server: &MockServer) -> Vec<(String, String)> {
            server.received_requests().await.unwrap().into_iter()
                .filter(|r| r.method.as_str() != "PROPFIND")
                .map(|r| (r.method.to_string(), r.url.path().to_string()))
                .collect()
        }

        /// The copy lands on the new list before the original leaves the old
        /// one, guarded both ways: a create there, the known etag here.
        #[tokio::test]
        async fn a_move_between_two_server_lists_copies_first_and_removes_second() {
            let server = MockServer::start().await;
            Mock::given(method("PUT")).and(path_regex(r"^/b/[0-9a-f-]+\.ics$")).and(header("If-None-Match", "*"))
                .respond_with(ResponseTemplate::new(201).insert_header("ETag", "\"new\""))
                .expect(1).mount(&server).await;
            Mock::given(method("DELETE")).and(path("/a/t1.ics")).and(header("If-Match", "\"old\""))
                .respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;

            let (href, etag) = move_resource(&on_server(&server, "a"), &on_server(&server, "b"),
                                             &task_at(&server), "BEGIN:VCALENDAR").await.unwrap();
            assert!(href.as_deref().is_some_and(|h| h.starts_with(&format!("{}/b/", server.uri()))));
            assert_eq!(etag.as_deref(), Some("\"new\""));
            let seen = methods(&server).await;
            assert_eq!(seen[0].0, "PUT", "the copy first: {seen:?}");
            assert_eq!(seen[1], ("DELETE".into(), "/a/t1.ics".into()), "the original second");
        }

        /// From this device to a server is a create there and nothing else;
        /// from a server to this device is a delete there and nothing else.
        #[tokio::test]
        async fn a_move_to_or_from_this_device_touches_only_the_server_end() {
            let server = MockServer::start().await;
            Mock::given(method("PUT")).and(path_regex(r"^/b/")).and(header("If-None-Match", "*"))
                .respond_with(ResponseTemplate::new(201).insert_header("ETag", "\"new\""))
                .expect(1).mount(&server).await;
            Mock::given(method("DELETE")).and(path("/a/t1.ics"))
                .respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;

            let mut local = task_at(&server);
            local.caldav_href = None;
            local.etag = None;
            let (href, _) = move_resource(&TaskHome::Device, &on_server(&server, "b"), &local, "X")
                .await.unwrap();
            assert!(href.is_some());
            let (href, etag) = move_resource(&on_server(&server, "a"), &TaskHome::Device,
                                             &task_at(&server), "X").await.unwrap();
            assert_eq!((href, etag), (None, None), "nothing to address on this device");
        }

        /// A new list that refuses the copy: nothing else is sent, and the
        /// task is where it was.
        #[tokio::test]
        async fn a_refused_copy_leaves_the_original_alone() {
            let server = MockServer::start().await;
            Mock::given(method("PUT")).respond_with(ResponseTemplate::new(403)).mount(&server).await;
            Mock::given(method("DELETE")).respond_with(ResponseTemplate::new(204)).expect(0).mount(&server).await;

            let err = move_resource(&on_server(&server, "a"), &on_server(&server, "b"),
                                    &task_at(&server), "X").await.unwrap_err();
            assert_eq!(err.to_string(), TASK_NOT_MOVED);
        }

        /// An original that will not come off its list: the copy is taken
        /// back, so the task is on one list, not two. A 412 says why.
        #[tokio::test]
        async fn an_original_that_will_not_leave_gets_its_copy_taken_back() {
            for (status, says) in [(500, TASK_NOT_MOVED), (412, TASK_CHANGED_ON_SERVER)] {
                let server = MockServer::start().await;
                Mock::given(method("PUT")).and(path_regex(r"^/b/"))
                    .respond_with(ResponseTemplate::new(201).insert_header("ETag", "\"new\""))
                    .mount(&server).await;
                Mock::given(method("DELETE")).and(path("/a/t1.ics"))
                    .respond_with(ResponseTemplate::new(status)).mount(&server).await;
                Mock::given(method("DELETE")).and(path_regex(r"^/b/")).and(header("If-Match", "\"new\""))
                    .respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;

                let err = move_resource(&on_server(&server, "a"), &on_server(&server, "b"),
                                        &task_at(&server), "X").await.unwrap_err();
                assert_eq!(err.to_string(), says, "{status}");
            }
        }

        /// And when the copy will not come back either, the task is on both
        /// lists, and the message says so.
        #[tokio::test]
        async fn a_copy_that_cannot_be_taken_back_is_reported_on_both_lists() {
            let server = MockServer::start().await;
            Mock::given(method("PUT")).respond_with(ResponseTemplate::new(201)).mount(&server).await;
            Mock::given(method("DELETE")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
            let err = move_resource(&on_server(&server, "a"), &on_server(&server, "b"),
                                    &task_at(&server), "X").await.unwrap_err();
            assert_eq!(err.to_string(), TASK_ON_BOTH_LISTS);
        }
    }
}
