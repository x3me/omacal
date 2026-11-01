use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, PartialEq)]
pub struct StoredEvent {
    pub id: i64,
    pub calendar_id: i64,
    pub google_id: String,
    pub summary: Option<String>,
    pub location: Option<String>,
    pub start_utc: i64,
    pub end_utc: i64,
    /// IANA zone the start was authored in.
    pub start_tz: String,
    /// IANA zone the end was authored in. Usually equal to `start_tz`, but a
    /// flight departs in one zone and lands in another — storing both is what
    /// lets the UI later render "09:00 IST – 13:00 EET".
    pub end_tz: String,
    pub is_all_day: bool,
    pub recurrence: Option<String>,
    /// Set on an *exception*: the `google_id` of the series this row overrides.
    /// The pair `(recurring_event_id, original_start_utc)` names exactly one
    /// occurrence of that series, which the renderer must then suppress — the
    /// master keeps expanding into the slot otherwise, and you get a ghost.
    pub recurring_event_id: Option<String>,
    /// The instant the overridden occurrence *would* have started at. Not the
    /// same as `start_utc` once the instance has been moved.
    pub original_start_utc: Option<i64>,
    pub status: String,
    pub self_response: Option<String>,
    pub conference_uri: Option<String>,
    /// The owning calendar's colour, joined in by `events_in_window`. It lives
    /// on `calendars`, not `events`, so `upsert_event` neither reads nor writes
    /// it; a hand-built `StoredEvent` on the write path leaves it `None`.
    /// **The colour to draw this event in** — the calendar's override if it has
    /// one, and Google's own colour otherwise. The `COALESCE` in `SELECT_COLS`
    /// is where those two become one answer, which is why nothing downstream
    /// of this field knows an override exists: the grid, the month bars, the
    /// Big Year pills and their legend all read it and are all correct without
    /// a line of their own. See `0006_calendar_colour.sql` for why the two are
    /// stored separately.
    pub color_hex: Option<String>,
    /// The owning calendar's own `timezone`, joined in by `events_in_window`
    /// alongside `color_hex`. **Not** `start_tz`/`end_tz` above — those are the
    /// zone a *timed* event was authored in, which for an all-day event is
    /// whatever sync happened to fall back to. This is the zone Google's bare
    /// `date` was resolved against for an all-day event, and the only one that
    /// buckets it back onto the right calendar day.
    pub calendar_timezone: String,
    pub description: Option<String>,
    pub etag: Option<String>,
    pub sequence: i64,
    pub organizer_email: Option<String>,
    /// Google's `guestsCanModify`: whether attendees other than the organizer
    /// may change the event for everyone. False is the default and the
    /// ordinary case, and it is what decides whether a guest's move or save
    /// reaches anybody but themselves (see `EventDetail::guests_can_modify`).
    pub guests_can_modify: bool,
    pub attendees: Vec<Attendee>,
    /// What this event asks for: the calendar's defaults, or its own overrides.
    pub reminders: Reminders,
    /// The owning calendar's `default_reminders_json`, joined in by
    /// [`events_in_window`] alongside `color_hex` and `calendar_timezone`. It
    /// lives on `calendars`, not `events`, so [`upsert_event`] neither reads
    /// nor writes it; a hand-built `StoredEvent` on the write path leaves it
    /// empty.
    ///
    /// Carried on the event rather than looked up per calendar because it is
    /// only ever wanted *with* one: `reminders.use_default` is the question and
    /// this is the answer, and reading one without the other says nothing.
    pub calendar_default_reminders: Vec<Reminder>,
}

/// One reminder: fire `minutes` before the event starts, by `method`.
///
/// `method` is Google's own vocabulary — `popup` or `email`. Both are stored;
/// only `popup` may ever be fired locally, since `email` is Google's to send
/// and firing it here would double it. Keeping `email` rows rather than
/// filtering them at the store boundary means the rule lives in one place, and
/// a stored row still describes what Google actually holds.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Reminder {
    pub method: String,
    pub minutes: i64,
}

/// An event's reminder settings. The two fields are alternatives, not additive:
/// either `use_default` and the owning calendar's list applies, or `overrides`
/// replaces it entirely.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Reminders {
    pub use_default: bool,
    pub overrides: Vec<Reminder>,
}

/// One invitee. Mirrors Google's `attendees[]` entry, kept in a JSON column
/// rather than a table — see 0003_attendees.sql for why.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Attendee {
    pub email: String,
    pub display_name: Option<String>,
    /// `accepted` | `declined` | `tentative` | `needsAction`.
    pub response_status: String,
    pub optional: bool,
    /// True for the signed-in user's own row. This is the entry an RSVP edits,
    /// and the only one it may edit.
    pub is_self: bool,
    /// A free-text note the attendee left on their RSVP. Not surfaced by this
    /// app's own UI, but writable on Google's side — carried through
    /// unchanged rather than dropped, since an RSVP patch replaces the whole
    /// attendee array and any field this struct doesn't round-trip is erased
    /// for real, for every attendee, on every response.
    pub comment: Option<String>,
    /// How many extra guests this attendee is bringing. Also writable and
    /// otherwise unmodelled; same reason as `comment`.
    pub additional_guests: i64,
}

const SELECT_COLS: &str = "e.id, e.calendar_id, e.google_id, e.summary, e.location,
     e.start_utc, e.end_utc, e.start_tz, e.end_tz, e.is_all_day, e.recurrence,
     e.recurring_event_id, e.original_start_utc,
     e.status, e.self_response, e.conference_uri,
     COALESCE(c.color_override, c.color_hex) AS color_hex, c.timezone,
     e.description, e.etag, e.sequence, e.organizer_email, e.guests_can_modify,
     e.attendees_json, e.reminders_json, c.default_reminders_json";

fn row_to_event(row: &sqlx::sqlite::SqliteRow) -> StoredEvent {
    StoredEvent {
        id: row.get("id"),
        calendar_id: row.get("calendar_id"),
        google_id: row.get("google_id"),
        summary: row.get("summary"),
        location: row.get("location"),
        start_utc: row.get("start_utc"),
        end_utc: row.get("end_utc"),
        start_tz: row.get("start_tz"),
        end_tz: row.get("end_tz"),
        is_all_day: row.get::<i64, _>("is_all_day") != 0,
        recurrence: row.get("recurrence"),
        recurring_event_id: row.get("recurring_event_id"),
        original_start_utc: row.get("original_start_utc"),
        status: row.get("status"),
        self_response: row.get("self_response"),
        conference_uri: row.get("conference_uri"),
        color_hex: row.get("color_hex"),
        calendar_timezone: row.get("timezone"),
        description: row.get("description"),
        etag: row.get("etag"),
        sequence: row.get("sequence"),
        organizer_email: row.get("organizer_email"),
        guests_can_modify: row.get::<i64, _>("guests_can_modify") != 0,
        // A malformed or absent JSON column must not fail the whole window
        // query — most personal events have no guests at all, so `NULL` here
        // is the common path, not an edge case.
        attendees: row
            .get::<Option<String>, _>("attendees_json")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        // NULL on both of these is the ordinary case, not an edge one: every
        // row written before 0004 has a NULL `reminders_json`, and a calendar
        // has a NULL `default_reminders_json` until the account is next
        // signed in. Both read as "fires nothing", which is what the absence
        // of a reminder means.
        reminders: row
            .get::<Option<String>, _>("reminders_json")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        calendar_default_reminders: row
            .get::<Option<String>, _>("default_reminders_json")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Generic over the executor rather than taking `&SqlitePool`, so the sync path
/// can run this inside the same transaction as its `sync_enabled` re-check. A
/// `&SqlitePool` still satisfies the bound, so every other call site is
/// unchanged.
pub async fn upsert_event<'e, E>(exec: E, ev: &StoredEvent) -> anyhow::Result<i64>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    // 23 columns, 23 placeholders, 23 binds, all in the same order. Keep them
    // that way: a mismatch here writes a value into the wrong column silently.
    let attendees_json = serde_json::to_string(&ev.attendees)?;
    let reminders_json = serde_json::to_string(&ev.reminders)?;
    let id: i64 = sqlx::query(
        "INSERT INTO events (calendar_id, google_id, summary, location, start_utc, end_utc,
             start_tz, end_tz, is_all_day, recurrence, recurring_event_id,
             original_start_utc, status, self_response, conference_uri, updated_at,
             description, etag, sequence, organizer_email, attendees_json, reminders_json,
             guests_can_modify)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)
         ON CONFLICT (calendar_id, google_id) DO UPDATE SET
             summary = excluded.summary, location = excluded.location,
             start_utc = excluded.start_utc, end_utc = excluded.end_utc,
             start_tz = excluded.start_tz, end_tz = excluded.end_tz,
             is_all_day = excluded.is_all_day, recurrence = excluded.recurrence,
             recurring_event_id = excluded.recurring_event_id,
             original_start_utc = excluded.original_start_utc,
             status = excluded.status, self_response = excluded.self_response,
             conference_uri = excluded.conference_uri, updated_at = excluded.updated_at,
             description = excluded.description, etag = excluded.etag,
             sequence = excluded.sequence, organizer_email = excluded.organizer_email,
             attendees_json = excluded.attendees_json,
             reminders_json = excluded.reminders_json,
             guests_can_modify = excluded.guests_can_modify
         RETURNING id",
    )
    .bind(ev.calendar_id)          // ?1  calendar_id
    .bind(&ev.google_id)           // ?2  google_id
    .bind(&ev.summary)             // ?3  summary
    .bind(&ev.location)            // ?4  location
    .bind(ev.start_utc)            // ?5  start_utc
    .bind(ev.end_utc)              // ?6  end_utc
    .bind(&ev.start_tz)            // ?7  start_tz
    .bind(&ev.end_tz)              // ?8  end_tz
    .bind(ev.is_all_day as i64)    // ?9  is_all_day
    .bind(&ev.recurrence)          // ?10 recurrence
    .bind(&ev.recurring_event_id)  // ?11 recurring_event_id
    .bind(ev.original_start_utc)   // ?12 original_start_utc
    .bind(&ev.status)              // ?13 status
    .bind(&ev.self_response)       // ?14 self_response
    .bind(&ev.conference_uri)      // ?15 conference_uri
    .bind(now_ms())                // ?16 updated_at
    .bind(&ev.description)         // ?17 description
    .bind(&ev.etag)                // ?18 etag
    .bind(ev.sequence)             // ?19 sequence
    .bind(&ev.organizer_email)     // ?20 organizer_email
    .bind(attendees_json)          // ?21 attendees_json
    .bind(reminders_json)          // ?22 reminders_json
    .bind(ev.guests_can_modify as i64) // ?23 guests_can_modify
    .fetch_one(exec)
    .await?
    .get("id");
    Ok(id)
}

/// Generic over the executor for the same reason as [`upsert_event`].
pub async fn delete_event<'e, E>(
    exec: E,
    calendar_id: i64,
    google_id: &str,
) -> anyhow::Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query("DELETE FROM events WHERE calendar_id = ?1 AND google_id = ?2")
        .bind(calendar_id)
        .bind(google_id)
        .execute(exec)
        .await?;
    Ok(())
}

/// Every local row of one series: the master, and every exception pointing
/// back at it.
///
/// For "delete all events", where Google removes the master *and* every
/// materialised exception of it — an occurrence somebody moved or deleted on
/// its own is a separate event whose only reason to exist is the series that is
/// going. [`delete_event`] on the master alone is not enough: an exception left
/// behind carries no `recurrence` of its own, so [`events_in_window`] goes on
/// returning it and the grid goes on rendering a meeting that is no longer on
/// anybody's calendar — with the series it belonged to gone from the screen, so
/// there is nothing left to delete it from either.
///
/// `master_google_id` is matched against both columns deliberately: by
/// `recurring_event_id` alone this would leave the master, and by `google_id`
/// alone it is [`delete_event`], which is what a single tombstone from sync
/// still wants.
///
/// Deliberately **not** used for "this and following". Which materialised
/// exceptions Google drops when a rule is truncated is an inference about
/// `UNTIL`, not something this app observes, and rows are not deleted here on an
/// inference — see `events::truncate_series`.
pub async fn delete_series(
    pool: &SqlitePool,
    calendar_id: i64,
    master_google_id: &str,
) -> anyhow::Result<()> {
    // The first clause hits the `(calendar_id, google_id)` unique index, the
    // second `idx_events_recurring (calendar_id, recurring_event_id)`.
    sqlx::query(
        "DELETE FROM events
          WHERE calendar_id = ?1 AND (google_id = ?2 OR recurring_event_id = ?2)",
    )
    .bind(calendar_id)
    .bind(master_google_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Re-files one event — and, when it is a series master, every exception of
/// it — under another calendar.
///
/// The counterpart to Google's own `events.move`, applied to the local copy so
/// the grid redraws in the new calendar's colour without waiting for two
/// syncs. Deliberately an `UPDATE` rather than a delete-and-reinsert: the row
/// **id** is what the open form, the popover and the caller's own
/// `event_detail` are all holding, and a move that renumbered it would leave
/// every one of them pointing at nothing.
///
/// The exception clause is [`delete_series`]'s, for its reason in reverse: a
/// materialised occurrence is its own row, and one left behind in the old
/// calendar carries no `recurrence` to be expanded from — so it would go on
/// rendering there, in the old colour, as a meeting that has moved.
///
/// Addressed by `master_google_id` rather than by row id, and matched against
/// both columns exactly as [`delete_series`] does — which also keeps every
/// value this statement reads out of the table it is writing to. A `WHERE`
/// clause that asked the events table for the row's own calendar mid-`UPDATE`
/// could see the master already re-filed and stop matching its own exceptions.
///
/// Returns the number of rows re-filed, which a caller can read as "the master
/// and its N exceptions". Zero means nothing matched.
pub async fn move_series_to_calendar(
    pool: &SqlitePool,
    source_calendar_id: i64,
    master_google_id: &str,
    target_calendar_id: i64,
) -> anyhow::Result<u64> {
    // The first clause hits the `(calendar_id, google_id)` unique index, the
    // second `idx_events_recurring (calendar_id, recurring_event_id)`.
    //
    // `OR REPLACE` for a race that is narrow but real: the sync loop runs in
    // the background, and a sync of the *destination* landing between the
    // provider's move and this statement brings the very same event in under
    // its own new row. Without it the move fails on `UNIQUE (calendar_id,
    // google_id)` — after the provider has already moved the event, so the
    // user is told a move failed that in fact happened. A conflicting row here
    // is by definition the same event on the same account, so replacing it
    // with the copy carrying this session's edits is not a guess.
    let moved = sqlx::query(
        "UPDATE OR REPLACE events SET calendar_id = ?3
          WHERE calendar_id = ?1 AND (google_id = ?2 OR recurring_event_id = ?2)",
    )
    .bind(source_calendar_id)
    .bind(master_google_id)
    .bind(target_calendar_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(moved)
}

/// The query behind both [`event_by_id`] and [`event_for_write`]: one event
/// row, its calendar's `access_role`, the calendar's own `timezone` and
/// `google_id`, and the email of the account that owns it. The last needs a
/// second join beyond `calendars` — it lives on `accounts` — because an RSVP
/// has to know which account's access token to use, not just whether the
/// calendar is writable.
///
/// `c.google_id` is aliased: `SELECT_COLS` already selects the *event's* own
/// `google_id` as `e.google_id`, and an unaliased second column of the same
/// name would collide when read back by name. `c.timezone` needs no such
/// alias and no separate selection at all any more — `SELECT_COLS` already
/// carries it as `StoredEvent::calendar_timezone`, so this query only adds
/// the two columns that field doesn't cover.
async fn event_row_for_write(
    pool: &SqlitePool,
    id: i64,
) -> anyhow::Result<Option<(StoredEvent, String, String, String, String)>> {
    let sql = format!(
        "SELECT {SELECT_COLS}, c.access_role, c.google_id AS cal_google_id,
                a.email AS account_email
         FROM events e
         JOIN calendars c ON c.id = e.calendar_id
         JOIN accounts a ON a.id = c.account_id
         WHERE e.id = ?1"
    );
    let row = sqlx::query(&sql).bind(id).fetch_optional(pool).await?;
    Ok(row.map(|r| {
        let access_role: String = r.get("access_role");
        let cal_google_id: String = r.get("cal_google_id");
        let account_email: String = r.get("account_email");
        let ev = row_to_event(&r);
        let cal_timezone = ev.calendar_timezone.clone();
        (ev, access_role, cal_google_id, account_email, cal_timezone)
    }))
}

/// One event plus its calendar's `access_role` and its calendar's own
/// `timezone`, by row id.
///
/// Both travel alongside the event rather than being a second query the caller
/// makes itself, because they are only ever needed together with it: an
/// `EventDetail` cannot decide whether to show RSVP controls from the event row
/// alone, and it cannot say what *date* an all-day event falls on from it
/// either. The store holds an instant for one, and which date that instant is
/// depends entirely on the zone it is read in — the calendar's, because that is
/// the one sync resolved Google's bare `date` against. Read in any other zone
/// it is a day out on one side of midnight or the other.
///
/// The zone costs nothing to carry: the query behind this already joins
/// `calendars` for the role, so `c.timezone` is one more column on a row
/// already being fetched. A second `calendar_for_write` lookup would be a
/// second round trip on a path — the event popover opening — that runs on every
/// click.
///
/// **Not** the event's own `start_tz`/`end_tz`. Those are the zones a *timed*
/// event was authored in, which for an all-day event is whatever sync happened
/// to fall back to; the calendar's zone is the one Google's `date` was resolved
/// against and the only one that reads it back unchanged.
pub async fn event_by_id(
    pool: &SqlitePool,
    id: i64,
) -> anyhow::Result<Option<(StoredEvent, String, String)>> {
    Ok(event_row_for_write(pool, id).await?.map(|(ev, role, _, _, tz)| (ev, role, tz)))
}

/// One event plus what the popover's detail needs to say whose it is: the
/// calendar's `access_role`, its own `google_id` and the owning account's
/// email — the two addresses `events::is_organizer` compares the organizer
/// against — and the calendar's zone. The same row [`event_for_write`]
/// reads, with nothing dropped; a detail and a write must agree on whose
/// event it is.
pub async fn event_for_detail(
    pool: &SqlitePool,
    id: i64,
) -> anyhow::Result<Option<(StoredEvent, String, String, String, String)>> {
    event_row_for_write(pool, id).await
}

/// One event plus everything an RSVP write needs beyond it: the calendar's
/// `access_role` (can this calendar be answered at all), the calendar's own
/// `google_id` (which calendar to patch), and the owning account's email
/// (which account's access token to use).
pub async fn event_for_write(
    pool: &SqlitePool,
    id: i64,
) -> anyhow::Result<Option<(StoredEvent, String, String, String)>> {
    Ok(event_row_for_write(pool, id)
        .await?
        .map(|(ev, role, cal_google_id, account_email, _)| (ev, role, cal_google_id, account_email)))
}

/// How many materialised exceptions of `master_google_id` override an
/// occurrence at or after `from_ms`.
///
/// An exception is one occurrence of a series that has been changed on its own
/// — moved, retitled, or deleted. It is a separate Google event pointing back
/// at the master, and `original_start_utc` is the slot it overrides, which is
/// what has to be compared against a split point: an occurrence dragged into
/// next month still overrides the slot it left behind.
///
/// Counted, rather than fetched, because the one caller
/// (`events::split_series`) needs only to know whether splitting there would
/// strand any — and how many, to say so.
///
/// **Cancelled exceptions are counted too, and that is the point of the
/// `status` column being absent from this query.** A cancelled exception is the
/// only record that a particular occurrence was deleted; losing it does not
/// leave a gap, it brings a cancelled meeting back to life in the new series.
///
/// `except_google_id` is excluded from the count, and it is load-bearing rather
/// than a convenience. The occurrence a split happens *at* is very often an
/// exception itself — dragging one occurrence and then splitting from it is an
/// ordinary thing to do — and that one is not stranded by the split: it becomes
/// the first occurrence of the new series, carrying the form's values. Counting
/// it would refuse exactly the case the caller handles best. Pass the
/// `google_id` of the row being edited; when that row is the master it names no
/// exception and the exclusion does nothing.
///
/// Only ever a lower bound: it sees what this store has synced. An exception
/// somebody created seconds ago, in a window this app has not fetched, is not
/// in here to be counted. The caller's refusal is therefore best-effort, and
/// says so.
pub async fn exceptions_from(
    pool: &SqlitePool,
    calendar_id: i64,
    master_google_id: &str,
    from_ms: i64,
    except_google_id: &str,
) -> anyhow::Result<i64> {
    // Hits `idx_events_recurring (calendar_id, recurring_event_id)`.
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM events
          WHERE calendar_id = ?1 AND recurring_event_id = ?2 AND original_start_utc >= ?3
            AND google_id <> ?4",
    )
    .bind(calendar_id)
    .bind(master_google_id)
    .bind(from_ms)
    .bind(except_google_id)
    .fetch_one(pool)
    .await?)
}

/// Events overlapping `[from_ms, to_ms)` on selected calendars, plus every
/// recurring master on a selected calendar. Masters are returned unconditionally
/// because their stored `start_utc` is the series start, which may be years
/// before the requested window; expansion happens in `omacal-core::expand`.
///
/// Cancelled rows are excluded — *except* cancelled exceptions. A cancelled
/// exception is the only record that a particular occurrence of a series was
/// deleted; drop it and the master expands into that slot forever. It is
/// returned so the renderer can suppress that occurrence, and renders nothing
/// itself.
///
/// An exception is also matched on `original_start_utc`, not only on its own
/// times: an instance dragged out of the window still has to suppress the slot
/// it left behind.
/// Every event whose **title** contains `query`, on a calendar the user
/// displays, newest sync state and all.
///
/// **`c.selected = 1`, exactly as [`events_in_window`] does it**, and for a
/// reason search shares with notifications rather than one of its own: a result
/// on a hidden calendar is one the user cannot land on. Click it, the view
/// jumps to its date, and nothing is drawn — because hiding it is what they
/// asked for. A result you cannot land on is worse than no result.
///
/// **Titles only** (search spec §2): `summary` and nothing else. Narrower than
/// it could be, and chosen so every result is explicable from what was typed.
/// Widening is additive — title matches are a strict subset of any broader
/// rule — so location or guests can be added later without invalidating this.
///
/// **`%` and `_` in the query are escaped**, which is not pedantry: `LIKE`
/// reads them as wildcards, so searching for a literal `50%` would otherwise
/// match every event in the database and searching `a_b` would match `axb`.
/// A user typing a percent sign means a percent sign; there is no query
/// language here (§2) and this is what keeps that true.
///
/// **Case and accents are folded** — the query and the column both through
/// [`omacal_core::fold`], the column via the `fold_ci` SQL function — so
/// `usa intrare` finds `ușă intrare` and `BOARD` finds `Board`. See
/// [`omacal_core::fold`] for what is deliberately left alone (Turkish `ı`,
/// `ß`, `ł`, `ø`). The pool must come from [`crate::connect`],
/// [`crate::connect_readonly`] or [`crate::connect_memory`], which register
/// `fold_ci`; a pool built some other way would answer `no such function`.
///
/// Recurring masters are returned as the single row they are. Resolving which
/// *occurrence* to show is [`omacal_core::search`]'s, against a clock the
/// caller supplies.
/// One person the user has met with, distilled from every synced guest list.
#[derive(Debug, Clone, PartialEq)]
pub struct KnownGuest {
    /// Lowercased — the identity the dedup groups on.
    pub email: String,
    /// The richest name any event carried for this address.
    pub display_name: Option<String>,
    /// How many stored events they appear on.
    pub met: i64,
    /// The start of the most recent of those events.
    pub last_met_utc: i64,
}

/// Everyone the user has ever met with, most-met first — the guest field's
/// autocomplete corpus (2026-08-23, by request: "right now I have to write
/// them from memory or find them in mail"). Google's own UI completes from
/// the People API; this completes from the history already sitting in
/// `attendees_json`, which needs no new OAuth scope, no network, and no
/// re-verification — and "people I have already met with" is almost exactly
/// the set worth completing to.
///
/// Rooms and projected calendars (`*.calendar.google.com`) are not people,
/// and the user's own account addresses are not guests to suggest.
pub async fn known_guests(pool: &SqlitePool) -> anyhow::Result<Vec<KnownGuest>> {
    let rows = sqlx::query(
        "SELECT LOWER(json_extract(j.value, '$.email')) AS email,
                MAX(NULLIF(json_extract(j.value, '$.display_name'), '')) AS display_name,
                COUNT(*) AS met,
                MAX(e.start_utc) AS last_met_utc
           FROM events e, json_each(e.attendees_json) j
          WHERE json_extract(j.value, '$.email') IS NOT NULL
            AND LOWER(json_extract(j.value, '$.email'))
                    NOT LIKE '%.calendar.google.com'
            AND LOWER(json_extract(j.value, '$.email'))
                    NOT IN (SELECT LOWER(email) FROM accounts)
          GROUP BY LOWER(json_extract(j.value, '$.email'))
          ORDER BY met DESC, last_met_utc DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| KnownGuest {
            email: r.get("email"),
            display_name: r.get("display_name"),
            met: r.get("met"),
            last_met_utc: r.get("last_met_utc"),
        })
        .collect())
}

/// One distinct location string the user has already saved — the Location
/// field's local autocomplete corpus, most-recent first.
#[derive(Debug, Clone, PartialEq)]
pub struct KnownLocation {
    pub label: String,
    pub used: i64,
    pub last_utc: i64,
}

/// Distinct non-blank `events.location` values, most recently used first.
/// Cancelled one-offs are dropped the same way `events_in_window` drops them
/// — they are not places the user still has; a cancelled exception stays
/// because it is the record of a slot, and its location is still a place
/// they went.
pub async fn known_locations(pool: &SqlitePool) -> anyhow::Result<Vec<KnownLocation>> {
    let rows = sqlx::query(
        "SELECT location AS label, MAX(start_utc) AS last_utc, COUNT(*) AS used
           FROM events
          WHERE location IS NOT NULL
            AND TRIM(location) != ''
            AND (status != 'cancelled' OR recurring_event_id IS NOT NULL)
          GROUP BY location
          ORDER BY last_utc DESC
          LIMIT 50",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| KnownLocation {
            label: r.get("label"),
            used: r.get("used"),
            last_utc: r.get("last_utc"),
        })
        .collect())
}

pub async fn search_events(pool: &SqlitePool, query: &str) -> anyhow::Result<Vec<StoredEvent>> {
    // Fold the query exactly as `fold_ci` folds the column, so `usa` finds
    // `ușă`, and lowercase is not a separate concern. The wildcards are escaped
    // *after* folding: `fold` leaves `%`, `_` and `\` alone, so the two
    // operations do not interact.
    //
    // `\` as the escape character, applied to the two wildcards and to itself
    // — escaping `%` and `_` while leaving a literal backslash unescaped would
    // make `\%` mean "escaped percent" when the user typed two characters.
    let escaped = omacal_core::fold(query)
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{escaped}%");

    let sql = format!(
        "SELECT {SELECT_COLS}
         FROM events e
         JOIN calendars c ON c.id = e.calendar_id
         WHERE c.selected = 1
           AND (e.status != 'cancelled' OR e.recurring_event_id IS NOT NULL)
           AND e.summary IS NOT NULL
           AND fold_ci(e.summary) LIKE ?1 ESCAPE '\\'"
    );

    let rows = sqlx::query(&sql).bind(pattern).fetch_all(pool).await?;
    Ok(rows.iter().map(row_to_event).collect())
}

/// Masters are not date-bounded here. A series' own start says nothing about
/// when it still occurs, and a long history keeps every series that ever
/// did. `omacal_core::recur::expand` skips a rule that has already ended, so
/// a view of today does not re-expand those; an open-ended series still
/// expands, because it still occurs.
pub async fn events_in_window(
    pool: &SqlitePool,
    from_ms: i64,
    to_ms: i64,
) -> anyhow::Result<Vec<StoredEvent>> {
    let sql = format!(
        "SELECT {SELECT_COLS}
         FROM events e
         JOIN calendars c ON c.id = e.calendar_id
         WHERE c.selected = 1
           AND (e.status != 'cancelled' OR e.recurring_event_id IS NOT NULL)
           AND (e.recurrence IS NOT NULL
                OR (e.start_utc < ?2 AND e.end_utc > ?1)
                OR (e.recurring_event_id IS NOT NULL
                    AND e.original_start_utc >= ?1
                    AND e.original_start_utc < ?2))
         ORDER BY e.start_utc"
    );
    let rows = sqlx::query(&sql).bind(from_ms).bind(to_ms).fetch_all(pool).await?;
    Ok(rows.iter().map(row_to_event).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect_memory;

    async fn seed(pool: &SqlitePool) -> i64 {
        sqlx::query("INSERT INTO accounts (google_sub, email, created_at) VALUES ('s','e@x',0)")
            .execute(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
             VALUES (1, 'primary', 'Work', 'Europe/Sofia', 'owner')",
        ).execute(pool).await.unwrap();
        1
    }

    /// Two accounts, one calendar each, with the calendar ids and account ids
    /// deliberately crossed so no calendar's `id` equals its own
    /// `account_id`. `seed`'s single account/calendar fixture makes that
    /// coincidence unavoidable — both ids come out `1` — which means a join
    /// on the wrong column can still return the right row by accident. Kept
    /// general enough to seed other cross-account tests, not just this one.
    ///
    /// Returns `(cal_on_account_b, cal_on_account_a)`: the first calendar
    /// inserted (id 1) belongs to the *second* account, and vice versa.
    async fn seed_two_accounts(pool: &SqlitePool) -> (i64, i64) {
        sqlx::query("INSERT INTO accounts (google_sub, email, created_at) VALUES ('a','a@x',0)")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO accounts (google_sub, email, created_at) VALUES ('b','b@x',0)")
            .execute(pool).await.unwrap();
        // Calendar id 1 belongs to account 2, calendar id 2 belongs to
        // account 1 — crossed on purpose.
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
             VALUES (2, 'cal-on-b', 'On B', 'UTC', 'reader')",
        ).execute(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
             VALUES (1, 'cal-on-a', 'On A', 'UTC', 'owner')",
        ).execute(pool).await.unwrap();

        // The crossing is the whole fixture, and nothing else checks it: an
        // edit that renumbered these rows — an extra account inserted first, a
        // reordering — would leave both join tests below passing for the
        // coincidental reason they exist to rule out, saying nothing while
        // looking like they still do.
        for google_id in ["cal-on-b", "cal-on-a"] {
            let (id, account_id): (i64, i64) =
                sqlx::query_as("SELECT id, account_id FROM calendars WHERE google_id = ?1")
                    .bind(google_id)
                    .fetch_one(pool)
                    .await
                    .unwrap();
            debug_assert_ne!(
                id, account_id,
                "{google_id}: no calendar's id may equal its own account_id, or a join on the \
                 wrong column still returns the right row"
            );
        }

        (1, 2)
    }

    fn ev(cal: i64, gid: &str, start: i64, end: i64) -> StoredEvent {
        StoredEvent {
            id: 0, calendar_id: cal, google_id: gid.into(),
            summary: Some("Standup".into()), location: None,
            start_utc: start, end_utc: end,
            start_tz: "Europe/Sofia".into(), end_tz: "Europe/Sofia".into(),
            is_all_day: false, recurrence: None,
            recurring_event_id: None, original_start_utc: None,
            status: "confirmed".into(),
            self_response: Some("accepted".into()), conference_uri: None,
            color_hex: None, calendar_timezone: "Europe/Sofia".into(),
            description: None, etag: None, sequence: 0, organizer_email: None,
            guests_can_modify: false,
            attendees: Vec::new(),
            reminders: Reminders::default(), calendar_default_reminders: Vec::new(),
        }
    }

    /// **Case and accents are folded on both sides** — the column through
    /// `fold_ci`, the query through `omacal_core::fold` — so an unaccented
    /// query finds an accented title and the title is still found by its own
    /// spelling. `connect_memory` registers the function exactly as the app's
    /// pool and the CLI's read-only pool do, which is what makes this the same
    /// answer those two would give.
    #[tokio::test]
    async fn search_folds_case_and_accents_on_both_sides() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        let mut e = ev(cal, "accents", 0, 3_600_000);
        e.summary = Some("Garnituri ușă intrare".into());
        upsert_event(&pool, &e).await.unwrap();

        for q in ["usa", "USA intrare", "ușă", "garnituri usa"] {
            let hits = search_events(&pool, q).await.unwrap();
            assert_eq!(hits.len(), 1, "query {q:?} should find the accented title");
        }
        assert!(search_events(&pool, "garage").await.unwrap().is_empty());
        // The percent sign is still a character, not a wildcard, now that the
        // query is folded before it is escaped.
        assert!(search_events(&pool, "%").await.unwrap().is_empty());
    }

    fn att(email: &str, name: Option<&str>) -> Attendee {
        Attendee {
            email: email.into(),
            display_name: name.map(Into::into),
            response_status: "accepted".into(),
            optional: false,
            is_self: false,
            comment: None,
            additional_guests: 0,
        }
    }

    /// A move takes the series and only the series: the master, its
    /// materialised exceptions, and nothing else sharing the calendar.
    #[tokio::test]
    async fn moving_a_series_takes_its_exceptions_and_leaves_the_rest() {
        let pool = connect_memory().await.unwrap();
        let (source, destination) = seed_two_accounts(&pool).await;

        upsert_event(&pool, &ev(source, "master", 1_000, 2_000)).await.unwrap();
        let mut exception = ev(source, "master_20260803", 3_000, 4_000);
        exception.recurring_event_id = Some("master".into());
        upsert_event(&pool, &exception).await.unwrap();
        // Same calendar, nothing to do with that series.
        upsert_event(&pool, &ev(source, "other", 5_000, 6_000)).await.unwrap();

        let moved = move_series_to_calendar(&pool, source, "master", destination).await.unwrap();
        assert_eq!(moved, 2, "the master and its one exception");

        let left: Vec<String> =
            sqlx::query_scalar("SELECT google_id FROM events WHERE calendar_id = ?1 ORDER BY google_id")
                .bind(source)
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(left, vec!["other".to_string()], "only the unrelated event stays");
        let arrived: Vec<String> =
            sqlx::query_scalar("SELECT google_id FROM events WHERE calendar_id = ?1 ORDER BY google_id")
                .bind(destination)
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(arrived, vec!["master".to_string(), "master_20260803".to_string()]);
    }

    /// The race `UPDATE OR REPLACE` exists for: a background sync of the
    /// destination pulls the moved event in *before* the local re-file runs,
    /// so the row this statement is about to move already exists there. The
    /// plain `UPDATE` fails on the unique index — after the provider has
    /// already moved the event, telling the user a move failed that happened.
    #[tokio::test]
    async fn a_destination_that_already_holds_the_event_does_not_block_the_move() {
        let pool = connect_memory().await.unwrap();
        let (source, destination) = seed_two_accounts(&pool).await;

        let mut ours = ev(source, "master", 1_000, 2_000);
        ours.summary = Some("Renamed in this very save".into());
        upsert_event(&pool, &ours).await.unwrap();
        upsert_event(&pool, &ev(destination, "master", 1_000, 2_000)).await.unwrap();

        move_series_to_calendar(&pool, source, "master", destination).await.unwrap();

        let rows: Vec<(i64, Option<String>)> =
            sqlx::query_as("SELECT calendar_id, summary FROM events")
                .fetch_all(&pool)
                .await
                .unwrap();
        // One row, on the destination, carrying *this* session's edit rather
        // than what the sync happened to bring in.
        assert_eq!(rows.len(), 1, "the duplicate should be gone, not doubled");
        assert_eq!(rows[0].0, destination);
        assert_eq!(rows[0].1.as_deref(), Some("Renamed in this very save"));
    }

    /// The autocomplete corpus: people deduped across events (case folded),
    /// the richest name kept, rooms and the user's own address left out,
    /// most-met first.
    #[tokio::test]
    async fn known_guests_distills_people_from_history() {
        let pool = connect_memory().await.unwrap();
        seed(&pool).await;

        let mut a = ev(1, "g1", 1_000, 2_000);
        a.attendees = vec![
            att("Ana@x.com", None),
            att("room-4@resource.calendar.google.com", Some("Room 4")),
            att("e@x", Some("Myself")), // the account's own address
        ];
        upsert_event(&pool, &a).await.unwrap();
        let mut b = ev(1, "g2", 5_000, 6_000);
        b.attendees = vec![att("ana@x.com", Some("Ana K")), att("bo@x.com", None)];
        upsert_event(&pool, &b).await.unwrap();

        let people = known_guests(&pool).await.unwrap();
        assert_eq!(
            people,
            vec![
                KnownGuest {
                    email: "ana@x.com".into(),
                    display_name: Some("Ana K".into()),
                    met: 2,
                    last_met_utc: 5_000,
                },
                KnownGuest {
                    email: "bo@x.com".into(),
                    display_name: None,
                    met: 1,
                    last_met_utc: 5_000,
                },
            ],
            "deduped and case-folded, name backfilled, room and self excluded",
        );
    }

    /// Distinct strings, most recently used first: two Banbury rows collapse
    /// to one, a blank is not a place, and a cancelled one-off is gone.
    #[tokio::test]
    async fn known_locations_are_distinct_most_recent_first() {
        let pool = connect_memory().await.unwrap();
        seed(&pool).await;

        const BANBURY: &str = "Banbury Golf Course, 2626 South Marypost Place, Eagle, Idaho 83616, United States";
        let mut a = ev(1, "loc1", 1_000, 2_000);
        a.location = Some(BANBURY.into());
        upsert_event(&pool, &a).await.unwrap();
        let mut b = ev(1, "loc2", 9_000, 10_000);
        b.location = Some(BANBURY.into());
        upsert_event(&pool, &b).await.unwrap();
        let mut c = ev(1, "loc3", 3_000, 4_000);
        c.location = Some("Room 4A".into());
        upsert_event(&pool, &c).await.unwrap();
        let mut blank = ev(1, "loc4", 8_000, 9_000);
        blank.location = Some("  ".into());
        upsert_event(&pool, &blank).await.unwrap();
        let mut cancelled = ev(1, "loc5", 20_000, 21_000);
        cancelled.location = Some("Ghost hall".into());
        cancelled.status = "cancelled".into();
        upsert_event(&pool, &cancelled).await.unwrap();

        let hits = known_locations(&pool).await.unwrap();
        assert_eq!(
            hits.iter().map(|h| h.label.as_str()).collect::<Vec<_>>(),
            vec![BANBURY, "Room 4A"],
        );
        assert_eq!(hits[0].used, 2);
        assert_eq!(hits[0].last_utc, 9_000);
        assert!(hits.iter().all(|h| !h.label.trim().is_empty()));
        assert_eq!(hits.iter().filter(|h| h.label == BANBURY).count(), 1);
    }

    /// Every clause of `exceptions_from` in one fixture, because each of them
    /// is a way for a series split to be refused when it is safe, or allowed
    /// when it would strand somebody's changed occurrences.
    ///
    /// The `<> except_google_id` clause is the one worth staring at: the
    /// occurrence a split happens at is very often an exception itself, and it
    /// is not stranded — it becomes the new series' first occurrence. Without
    /// that clause, splitting from an occurrence you had previously dragged is
    /// refused every time.
    ///
    /// Cancelled exceptions count. A cancelled exception is the only record
    /// that an occurrence was deleted, so losing it does not leave a gap — the
    /// cancelled meeting comes back in the new series.
    #[tokio::test]
    async fn exceptions_from_counts_only_the_ones_a_split_would_strand() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        let other_cal = {
            sqlx::query(
                "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
                 VALUES (1, 'second', 'Home', 'Europe/Sofia', 'owner')",
            )
            .execute(&pool)
            .await
            .unwrap();
            2
        };

        let exception = |cal: i64, gid: &str, master: &str, slot: i64, status: &str| {
            let mut e = ev(cal, gid, slot, slot + 1000);
            e.recurring_event_id = Some(master.into());
            e.original_start_utc = Some(slot);
            e.status = status.into();
            e
        };

        for row in [
            // Before the split: stays with the original series.
            exception(cal, "m1_a", "m1", 1_000, "confirmed"),
            // The occurrence being split at, dragged elsewhere. Not stranded.
            exception(cal, "m1_at", "m1", 2_000, "confirmed"),
            // After the split: both stranded, cancelled included.
            exception(cal, "m1_b", "m1", 3_000, "confirmed"),
            exception(cal, "m1_c", "m1", 4_000, "cancelled"),
            // A different series, and a different calendar. Neither counts.
            exception(cal, "m2_a", "m2", 5_000, "confirmed"),
            exception(other_cal, "m1_elsewhere", "m1", 5_000, "confirmed"),
            // An ordinary event of the series' own master, which is not an
            // exception at all and has no `original_start_utc`.
            ev(cal, "m1", 0, 1_000),
        ] {
            upsert_event(&pool, &row).await.unwrap();
        }

        assert_eq!(
            exceptions_from(&pool, cal, "m1", 2_000, "m1_at").await.unwrap(),
            2,
            "expected only the two occupied slots after the split point"
        );
        // The same call from the master's own row: nothing is excluded, so the
        // occurrence at the split point counts too.
        assert_eq!(exceptions_from(&pool, cal, "m1", 2_000, "m1").await.unwrap(), 3);
        // A split with nothing after it strands nothing.
        assert_eq!(exceptions_from(&pool, cal, "m1", 9_000, "m1").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn an_event_round_trips() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        upsert_event(&pool, &ev(cal, "a", 1000, 2000)).await.unwrap();
        let out = events_in_window(&pool, 0, 5000).await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].summary.as_deref(), Some("Standup"));
        assert_eq!(out[0].start_utc, 1000);
    }

    #[tokio::test]
    async fn upsert_updates_rather_than_duplicates() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        upsert_event(&pool, &ev(cal, "a", 1000, 2000)).await.unwrap();
        let mut changed = ev(cal, "a", 1000, 2000);
        changed.summary = Some("Standup (moved)".into());
        upsert_event(&pool, &changed).await.unwrap();
        let out = events_in_window(&pool, 0, 5000).await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].summary.as_deref(), Some("Standup (moved)"));
    }

    #[tokio::test]
    async fn events_outside_the_window_are_excluded() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        upsert_event(&pool, &ev(cal, "a", 10_000, 11_000)).await.unwrap();
        assert!(events_in_window(&pool, 0, 5_000).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_event_straddling_the_window_edge_is_included() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        upsert_event(&pool, &ev(cal, "a", 4_000, 9_000)).await.unwrap();
        assert_eq!(events_in_window(&pool, 5_000, 6_000).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn recurring_masters_are_returned_regardless_of_window() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        let mut master = ev(cal, "r", 0, 1_800_000);
        master.recurrence = Some("RRULE:FREQ=DAILY".into());
        upsert_event(&pool, &master).await.unwrap();
        // Window is far in the future; the master must still come back so the
        // caller can expand it.
        let out = events_in_window(&pool, 10_000_000_000, 10_000_100_000).await.unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].recurrence.is_some());
    }

    #[tokio::test]
    async fn events_on_deselected_calendars_are_excluded() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        upsert_event(&pool, &ev(cal, "a", 1000, 2000)).await.unwrap();
        sqlx::query("UPDATE calendars SET selected = 0").execute(&pool).await.unwrap();
        assert!(events_in_window(&pool, 0, 5000).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn deleting_an_event_removes_it() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        upsert_event(&pool, &ev(cal, "a", 1000, 2000)).await.unwrap();
        delete_event(&pool, cal, "a").await.unwrap();
        assert!(events_in_window(&pool, 0, 5000).await.unwrap().is_empty());
    }

    /// Both clauses of `delete_series`, and both ways of getting it wrong.
    ///
    /// Matching only `recurring_event_id` leaves the master, so the whole series
    /// goes on expanding; matching only `google_id` leaves every exception, and
    /// an exception carries no rule of its own, so it renders as a standalone
    /// meeting that no longer exists anywhere.
    ///
    /// The negative half is what stops it reaching too far: another series on
    /// the same calendar, the same series on another calendar, and a one-off
    /// that merely sorts nearby all have to survive.
    #[tokio::test]
    async fn deleting_a_series_takes_its_master_and_its_exceptions_and_nothing_else() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        let other_cal = {
            sqlx::query(
                "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
                 VALUES (1, 'second', 'Home', 'Europe/Sofia', 'owner')",
            )
            .execute(&pool)
            .await
            .unwrap();
            2
        };

        let exception = |cal: i64, gid: &str, master: &str, slot: i64, status: &str| {
            let mut e = ev(cal, gid, slot, slot + 1000);
            e.recurring_event_id = Some(master.into());
            e.original_start_utc = Some(slot);
            e.status = status.into();
            e
        };

        let mut master = ev(cal, "m1", 1_000, 2_000);
        master.recurrence = Some("RRULE:FREQ=WEEKLY".into());
        for row in [
            master,
            exception(cal, "m1_moved", "m1", 3_000, "confirmed"),
            exception(cal, "m1_gone", "m1", 4_000, "cancelled"),
            // Must survive: another series, the same series elsewhere, and an
            // event that belongs to no series at all.
            exception(cal, "m2_moved", "m2", 3_000, "confirmed"),
            exception(other_cal, "m1_elsewhere", "m1", 3_000, "confirmed"),
            ev(cal, "one-off", 1_000, 2_000),
        ] {
            upsert_event(&pool, &row).await.unwrap();
        }

        delete_series(&pool, cal, "m1").await.unwrap();

        // Read straight off the table rather than through `events_in_window`:
        // that query hides cancelled rows unless they are exceptions, and the
        // cancelled exception above is one of the rows this must remove.
        let left: Vec<String> = sqlx::query_scalar("SELECT google_id FROM events ORDER BY google_id")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(left, vec!["m1_elsewhere", "m2_moved", "one-off"]);
    }

    /// Every column written is read back with the value it went in with. This is
    /// the guard against the upsert's bind order drifting out of step with its
    /// column list — a mismatch there is silent, and lands data in the wrong
    /// column rather than failing.
    #[tokio::test]
    async fn every_column_round_trips_in_its_own_field() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        let full = StoredEvent {
            id: 0, calendar_id: cal, google_id: "gid".into(),
            summary: Some("Summary".into()), location: Some("Room 1".into()),
            start_utc: 1_000, end_utc: 2_000,
            start_tz: "Asia/Kolkata".into(), end_tz: "Europe/Sofia".into(),
            is_all_day: true, recurrence: Some("RRULE:FREQ=DAILY".into()),
            recurring_event_id: Some("master".into()), original_start_utc: Some(1_500),
            status: "tentative".into(), self_response: Some("needsAction".into()),
            conference_uri: Some("https://meet/x".into()), color_hex: None,
            calendar_timezone: "Europe/Sofia".into(),
            description: None, etag: None, sequence: 0, organizer_email: None,
            guests_can_modify: false,
            attendees: Vec::new(),
            reminders: Reminders::default(), calendar_default_reminders: Vec::new(),
        };
        upsert_event(&pool, &full).await.unwrap();

        let out = events_in_window(&pool, 0, 5_000).await.unwrap();
        assert_eq!(out.len(), 1);
        let got = &out[0];
        assert_eq!(got.google_id, "gid");
        assert_eq!(got.summary.as_deref(), Some("Summary"));
        assert_eq!(got.location.as_deref(), Some("Room 1"));
        assert_eq!(got.start_utc, 1_000);
        assert_eq!(got.end_utc, 2_000);
        assert_eq!(got.start_tz, "Asia/Kolkata");
        assert_eq!(got.end_tz, "Europe/Sofia");
        assert!(got.is_all_day);
        assert_eq!(got.recurrence.as_deref(), Some("RRULE:FREQ=DAILY"));
        assert_eq!(got.recurring_event_id.as_deref(), Some("master"));
        assert_eq!(got.original_start_utc, Some(1_500));
        assert_eq!(got.status, "tentative");
        assert_eq!(got.self_response.as_deref(), Some("needsAction"));
        assert_eq!(got.conference_uri.as_deref(), Some("https://meet/x"));
    }

    #[tokio::test]
    async fn an_updated_exception_keeps_its_recurrence_link() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        let mut x = ev(cal, "x", 1_000, 2_000);
        x.recurring_event_id = Some("master".into());
        x.original_start_utc = Some(1_000);
        upsert_event(&pool, &x).await.unwrap();

        // The instance is dragged later; the original slot must not change.
        x.start_utc = 3_000;
        x.end_utc = 4_000;
        upsert_event(&pool, &x).await.unwrap();

        let out = events_in_window(&pool, 0, 5_000).await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].start_utc, 3_000);
        assert_eq!(out[0].original_start_utc, Some(1_000));
        assert_eq!(out[0].recurring_event_id.as_deref(), Some("master"));
    }

    #[tokio::test]
    async fn an_ordinary_cancelled_event_is_excluded() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        let mut e = ev(cal, "a", 1_000, 2_000);
        e.status = "cancelled".into();
        upsert_event(&pool, &e).await.unwrap();
        assert!(events_in_window(&pool, 0, 5_000).await.unwrap().is_empty());
    }

    /// The one cancelled row that must come back: without it the master keeps
    /// expanding into the slot the user deleted.
    #[tokio::test]
    async fn a_cancelled_exception_is_returned_so_it_can_suppress_its_slot() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        let mut e = ev(cal, "x", 1_000, 2_000);
        e.status = "cancelled".into();
        e.recurring_event_id = Some("master".into());
        e.original_start_utc = Some(1_000);
        upsert_event(&pool, &e).await.unwrap();

        let out = events_in_window(&pool, 0, 5_000).await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, "cancelled");
        assert_eq!(out[0].recurring_event_id.as_deref(), Some("master"));
    }

    /// An instance moved clean out of the window still has to be returned: the
    /// slot it vacated is inside the window and must stay empty.
    #[tokio::test]
    async fn an_exception_moved_out_of_the_window_is_still_returned() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        let mut e = ev(cal, "x", 900_000, 901_000);
        e.recurring_event_id = Some("master".into());
        e.original_start_utc = Some(3_000);
        upsert_event(&pool, &e).await.unwrap();

        let out = events_in_window(&pool, 0, 5_000).await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].original_start_utc, Some(3_000));
    }

    #[tokio::test]
    async fn the_calendar_colour_is_joined_onto_each_event() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        sqlx::query("UPDATE calendars SET color_hex = '#b58900'")
            .execute(&pool).await.unwrap();
        upsert_event(&pool, &ev(cal, "a", 1000, 2000)).await.unwrap();
        let out = events_in_window(&pool, 0, 5000).await.unwrap();
        assert_eq!(out[0].color_hex.as_deref(), Some("#b58900"));
    }

    /**
     * **The seam the whole per-calendar-colour design rests on.**
     *
     * An override joins onto every event through the same `COALESCE` the
     * calendar list uses, which is why nothing downstream knows overrides
     * exist: the week grid, the month bars, the Big Year pills and their
     * legend are all correct without a line of their own. Delete the
     * `COALESCE` here and the settings row still shows the chosen colour while
     * every event stays Google's — the exact half-applied state a reader would
     * report as "the colour did not take".
     */
    #[tokio::test]
    async fn an_overridden_calendar_colours_its_events() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        sqlx::query("UPDATE calendars SET color_hex = '#b58900', color_override = '#e2a03f'")
            .execute(&pool).await.unwrap();
        upsert_event(&pool, &ev(cal, "a", 1000, 2000)).await.unwrap();

        let out = events_in_window(&pool, 0, 5000).await.unwrap();
        assert_eq!(
            out[0].color_hex.as_deref(),
            Some("#e2a03f"),
            "the event must take the override, not Google's colour",
        );
    }

    /// And with the override cleared it goes back to Google's — so the rule
    /// above cannot be satisfied by a version that ignores `color_hex`
    /// entirely.
    #[tokio::test]
    async fn a_cleared_override_leaves_events_on_googles_colour() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        sqlx::query("UPDATE calendars SET color_hex = '#b58900', color_override = NULL")
            .execute(&pool).await.unwrap();
        upsert_event(&pool, &ev(cal, "a", 1000, 2000)).await.unwrap();

        let out = events_in_window(&pool, 0, 5000).await.unwrap();
        assert_eq!(out[0].color_hex.as_deref(), Some("#b58900"));
    }

    #[tokio::test]
    async fn a_calendar_without_a_colour_yields_none() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        upsert_event(&pool, &ev(cal, "a", 1000, 2000)).await.unwrap();
        let out = events_in_window(&pool, 0, 5000).await.unwrap();
        assert!(out[0].color_hex.is_none());
    }

    /// The zone `assemble_days` needs to bucket an all-day event by the right
    /// day: the stored instant is midnight in the *calendar's* zone, and
    /// bucketing it against day boundaries in any other zone lands the chip
    /// under the wrong date.
    ///
    /// `seed`'s calendar and `ev`'s event both say `Europe/Sofia`, which would
    /// let either column pass as the other, so this fixture moves the
    /// *calendar* to `Pacific/Auckland` and gives the *event* its own
    /// `start_tz` of `UTC` — reading `e.start_tz` by mistake then answers
    /// `UTC` and fails here rather than shipping.
    #[tokio::test]
    async fn events_in_window_returns_the_calendars_own_timezone_not_the_events() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        sqlx::query("UPDATE calendars SET timezone = 'Pacific/Auckland' WHERE id = ?1")
            .bind(cal)
            .execute(&pool)
            .await
            .unwrap();
        let mut e = ev(cal, "a", 1000, 2000);
        e.start_tz = "UTC".into();
        upsert_event(&pool, &e).await.unwrap();

        let out = events_in_window(&pool, 0, 5000).await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].start_tz, "UTC",
            "fixture check: the event's own zone must differ from its calendar's, or this \
             test cannot tell the two columns apart"
        );
        assert_eq!(out[0].calendar_timezone, "Pacific/Auckland");
    }

    #[tokio::test]
    async fn hiding_a_calendar_does_not_stop_it_syncing() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        upsert_event(&pool, &ev(cal, "a", 1000, 2000)).await.unwrap();

        // Hidden, but still synced: the whole point of the split.
        sqlx::query("UPDATE calendars SET selected = 0").execute(&pool).await.unwrap();

        assert!(events_in_window(&pool, 0, 5000).await.unwrap().is_empty(),
                "a hidden calendar must not render");

        let syncing: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM calendars WHERE sync_enabled = 1")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(syncing, 1, "hiding must not disable syncing");
    }

    // Note: this only checks the DEFAULT applied to a newly-inserted row, not
    // the migration's backfill `UPDATE` — `connect_memory` always starts from
    // an empty database, so there is no pre-existing row for that `UPDATE` to
    // act on. The backfill itself is verified against a copy of the real
    // production database (see task-1-report.md), not by a unit test here.
    #[tokio::test]
    async fn a_new_calendar_defaults_to_synced_and_shown() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        let _ = cal;
        let (selected, sync_enabled): (i64, i64) = sqlx::query_as(
            "SELECT selected, sync_enabled FROM calendars LIMIT 1")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(selected, 1);
        assert_eq!(sync_enabled, 1, "a fresh calendar syncs by default");
    }

    #[tokio::test]
    async fn attendees_round_trip_through_the_store() {
        let pool = crate::connect_memory().await.unwrap();
        let cal = seed(&pool).await;

        let ev = StoredEvent {
            id: 0,
            calendar_id: cal,
            google_id: "ev1".into(),
            summary: Some("Weekly Standup".into()),
            location: None,
            start_utc: 1786341600000,
            end_utc: 1786343400000,
            start_tz: "Europe/Sofia".into(),
            end_tz: "Europe/Sofia".into(),
            is_all_day: false,
            recurrence: None,
            recurring_event_id: None,
            original_start_utc: None,
            status: "confirmed".into(),
            self_response: Some("needsAction".into()),
            conference_uri: None,
            color_hex: None,
            calendar_timezone: "Europe/Sofia".into(),
            description: Some("Sprint sync.".into()),
            etag: Some("\"etag-1\"".into()),
            sequence: 3,
            organizer_email: Some("ana@x.com".into()),
            guests_can_modify: true,
            reminders: Reminders::default(),
            calendar_default_reminders: Vec::new(),
            attendees: vec![
                Attendee { email: "ana@x.com".into(), display_name: Some("Ana".into()),
                           response_status: "accepted".into(), optional: false, is_self: false,
                           comment: Some("running 5 late".into()), additional_guests: 1 },
                Attendee { email: "me@x.com".into(), display_name: None,
                           response_status: "needsAction".into(), optional: true, is_self: true,
                           comment: None, additional_guests: 0 },
            ],
        };
        upsert_event(&pool, &ev).await.unwrap();

        let back = events_in_window(&pool, 1786300000000, 1786400000000).await.unwrap();
        let got = back.iter().find(|e| e.google_id == "ev1").expect("event stored");

        assert_eq!(got.description.as_deref(), Some("Sprint sync."));
        assert_eq!(got.etag.as_deref(), Some("\"etag-1\""));
        assert_eq!(got.sequence, 3);
        assert_eq!(got.organizer_email.as_deref(), Some("ana@x.com"));
        assert!(got.guests_can_modify, "guestsCanModify lost in the round trip");
        assert_eq!(got.attendees.len(), 2, "attendees lost in the round trip");
        assert_eq!(got.attendees[1].email, "me@x.com");
        assert!(got.attendees[1].is_self, "the self flag must survive");
        assert!(got.attendees[1].optional, "the optional flag must survive");
        assert_eq!(got.attendees[0].display_name.as_deref(), Some("Ana"));
        assert_eq!(got.attendees[0].comment.as_deref(), Some("running 5 late"),
                   "comment lost in the round trip");
        assert_eq!(got.attendees[0].additional_guests, 1,
                   "additional_guests lost in the round trip");

        // The update path: a re-sync of the same google_id must overwrite the
        // stored attendee list, not merge into it or leave it alone. Dropping
        // "ana@x.com" entirely (rather than only editing "me@x.com" in place)
        // also proves the list can shrink, not just mutate an existing entry.
        let mut changed = ev.clone();
        changed.attendees = vec![
            Attendee { email: "me@x.com".into(), display_name: None,
                       response_status: "accepted".into(), optional: true, is_self: true,
                       comment: None, additional_guests: 0 },
        ];
        upsert_event(&pool, &changed).await.unwrap();

        let back2 = events_in_window(&pool, 1786300000000, 1786400000000).await.unwrap();
        let got2 = back2.iter().find(|e| e.google_id == "ev1").expect("event still stored");
        assert_eq!(got2.attendees.len(), 1,
                   "the second upsert must replace the attendee list, not merge it");
        assert_eq!(got2.attendees[0].email, "me@x.com");
        assert_eq!(got2.attendees[0].response_status, "accepted",
                   "the second write's response status did not take effect");
    }

    /// Both halves of a reminder answer, and the two must not be mistaken for
    /// each other: an event's own overrides come off `events.reminders_json`,
    /// the fallback for `use_default` off the calendar's
    /// `default_reminders_json`. The two fixtures below share no method and no
    /// minute value, so a query reading either column in place of the other
    /// fails here rather than looking right.
    ///
    /// `email` overrides are stored, not filtered — only `popup` may ever be
    /// *fired* locally, and that is a decision for whatever reads these rows,
    /// not for the store. Dropping them here would also make the stored row
    /// stop describing what Google actually holds.
    #[tokio::test]
    async fn an_events_own_reminders_and_its_calendars_defaults_both_round_trip() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        sqlx::query(
            "UPDATE calendars
                SET default_reminders_json = '[{\"method\":\"popup\",\"minutes\":30}]'
              WHERE id = ?1",
        )
        .bind(cal)
        .execute(&pool)
        .await
        .unwrap();

        let mut overriding = ev(cal, "own", 1_000, 2_000);
        overriding.reminders = Reminders {
            use_default: false,
            overrides: vec![
                Reminder { method: "popup".into(), minutes: 10 },
                Reminder { method: "email".into(), minutes: 1_440 },
            ],
        };
        let mut deferring = ev(cal, "defers", 1_000, 2_000);
        deferring.reminders = Reminders { use_default: true, overrides: Vec::new() };

        upsert_event(&pool, &overriding).await.unwrap();
        upsert_event(&pool, &deferring).await.unwrap();

        let out = events_in_window(&pool, 0, 5_000).await.unwrap();
        let got = |gid: &str| {
            out.iter()
                .find(|e| e.google_id == gid)
                .unwrap_or_else(|| panic!("no event {gid}"))
                .clone()
        };

        let own = got("own");
        assert!(!own.reminders.use_default);
        assert_eq!(own.reminders.overrides.len(), 2, "an override was lost in the round trip");
        assert_eq!(own.reminders.overrides[0], Reminder { method: "popup".into(), minutes: 10 });
        assert_eq!(
            own.reminders.overrides[1],
            Reminder { method: "email".into(), minutes: 1_440 },
            "an email reminder must be stored, not filtered out at the store boundary"
        );

        let defers = got("defers");
        assert!(defers.reminders.use_default, "the defer-to-calendar flag was lost");
        assert!(defers.reminders.overrides.is_empty());

        // The calendar's own list, joined in like `color_hex`, on both rows —
        // it is the answer to `use_default` and is useless without it.
        for e in [&own, &defers] {
            assert_eq!(
                e.calendar_default_reminders,
                vec![Reminder { method: "popup".into(), minutes: 30 }],
                "{}: the calendar's defaults did not join in",
                e.google_id
            );
        }
    }

    /// The update path: a re-sync of the same event must replace its reminder
    /// list rather than leave the old one standing. Deleting a reminder in
    /// Google is delivered as an event whose `overrides` is simply shorter.
    #[tokio::test]
    async fn re_upserting_an_event_replaces_its_reminders() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;

        let mut e = ev(cal, "a", 1_000, 2_000);
        e.reminders = Reminders {
            use_default: false,
            overrides: vec![
                Reminder { method: "popup".into(), minutes: 10 },
                Reminder { method: "popup".into(), minutes: 60 },
            ],
        };
        upsert_event(&pool, &e).await.unwrap();

        e.reminders = Reminders { use_default: true, overrides: Vec::new() };
        upsert_event(&pool, &e).await.unwrap();

        let out = events_in_window(&pool, 0, 5_000).await.unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].reminders.use_default);
        assert!(
            out[0].reminders.overrides.is_empty(),
            "the second upsert must replace the override list, not merge into it"
        );
    }

    /// Every row written before 0004 has a NULL `reminders_json`, and every
    /// calendar has a NULL `default_reminders_json` until the account is next
    /// signed in. Neither may become a parse failure that takes the whole
    /// window query down with it.
    #[tokio::test]
    async fn null_reminder_columns_read_back_as_nothing_rather_than_failing() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        sqlx::query(
            "INSERT INTO events (calendar_id, google_id, start_utc, end_utc,
                 start_tz, end_tz, status, updated_at)
             VALUES (?1, 'bare', 1000, 2000, 'Europe/Sofia', 'Europe/Sofia', 'confirmed', 0)",
        )
        .bind(cal)
        .execute(&pool)
        .await
        .unwrap();

        let out = events_in_window(&pool, 0, 5_000).await.unwrap();
        let got = out.iter().find(|e| e.google_id == "bare").expect("the bare row must come back");
        assert_eq!(got.reminders, Reminders::default());
        assert!(got.calendar_default_reminders.is_empty());
    }

    #[tokio::test]
    async fn an_event_with_no_attendees_reads_back_as_an_empty_list() {
        // A NULL column must not become a parse error. Most personal events have
        // no guests at all, so this is the common path, not an edge case.
        let pool = crate::connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        sqlx::query(
            "INSERT INTO events (calendar_id, google_id, start_utc, end_utc,
                 start_tz, end_tz, status, updated_at)
             VALUES (?1, 'bare', 1786341600000, 1786343400000,
                     'Europe/Sofia', 'Europe/Sofia', 'confirmed', 0)")
            .bind(cal).execute(&pool).await.unwrap();

        let back = events_in_window(&pool, 1786300000000, 1786400000000).await.unwrap();
        let got = back.iter().find(|e| e.google_id == "bare").unwrap();
        assert!(got.attendees.is_empty());
        assert_eq!(got.sequence, 0);
    }

    /// A fixed list of already-resolved migrations, so a slice of the real
    /// migration set can be run on its own.
    #[derive(Debug)]
    struct Subset(Vec<sqlx::migrate::Migration>);
    impl sqlx::migrate::MigrationSource<'static> for Subset {
        #[allow(clippy::type_complexity)]
        fn resolve(
            self,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            Vec<sqlx::migrate::Migration>,
                            Box<dyn std::error::Error + Sync + Send>,
                        >,
                    > + Send,
            >,
        > {
            Box::pin(async move { Ok(self.0) })
        }
    }

    /// A database with every migration *below* `version` applied and nothing
    /// above it, so a test can plant rows that predate the one it is about.
    ///
    /// Migrations normally all run together at `connect_memory` time, which
    /// would make a backfill assertion observe only the shape of the finished
    /// schema rather than the migration running against rows that were already
    /// there. `expected` pins how many migrations that is, so a renumbering
    /// cannot silently turn this into "run nothing, then assert".
    async fn pool_migrated_below(version: i64, expected: usize) -> SqlitePool {
        use sqlx::migrate::{Migration, Migrator};
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;

        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(true);
        // `max_connections(1)`, same reason as `connect_memory`: every other
        // connection to `:memory:` would be its own empty database.
        let pool = SqlitePoolOptions::new().max_connections(1).connect_with(opts).await.unwrap();

        let earlier: Vec<Migration> = sqlx::migrate!("./migrations")
            .iter()
            .filter(|m| m.version < version)
            .cloned()
            .collect();
        assert_eq!(earlier.len(), expected, "expected {expected} migrations before {version} lands");
        Migrator::new(Subset(earlier)).await.unwrap().run(&pool).await.unwrap();
        pool
    }

    /// Applies exactly the migration at `version` on top of a pool already
    /// carrying the earlier ones.
    async fn apply_migration(pool: &SqlitePool, version: i64) {
        use sqlx::migrate::{Migration, Migrator};

        let just_this: Vec<Migration> = sqlx::migrate!("./migrations")
            .iter()
            .filter(|m| m.version == version)
            .cloned()
            .collect();
        assert_eq!(just_this.len(), 1, "expected exactly one migration at version {version}");
        let mut only_this = Migrator::new(Subset(just_this)).await.unwrap();
        // This source lists one version; without this, `run` sees the earlier
        // ones already applied but absent from its own list and refuses to
        // proceed (`VersionMissing`) rather than treating that as fine.
        only_this.set_ignore_missing(true);
        only_this.run(pool).await.unwrap();
    }

    /// 0015's backfill: a Google event cached before v0.17.0 imported
    /// conferenceData, and unchanged since, has no Join link and is never
    /// re-delivered by an incremental sync. Dropping Google's cursors is what
    /// makes the next sync a full window fetch; CalDAV's ctags live in the
    /// same table and must survive, and so must the cached events.
    #[tokio::test]
    async fn conference_backfill_resets_google_cursors_but_preserves_caldav_and_events() {
        let pool = pool_migrated_below(15, 14).await;
        let (google_cal, dav_cal) = seed_two_accounts(&pool).await;
        sqlx::query("UPDATE accounts SET provider = 'caldav' WHERE id = 1")
            .execute(&pool).await.unwrap();
        for cal in [google_cal, dav_cal] {
            sqlx::query("INSERT INTO sync_state (calendar_id, sync_token, window_start, window_end)
                         VALUES (?1, 'old-cursor', 0, 0)")
                .bind(cal).execute(&pool).await.unwrap();
        }
        let mut ev = ev(google_cal, "old-series", 0, 60_000);
        ev.conference_uri = None;
        let id = upsert_event(&pool, &ev).await.unwrap();
        apply_migration(&pool, 15).await;

        let remaining: Vec<i64> = sqlx::query_scalar("SELECT calendar_id FROM sync_state")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(remaining, vec![dav_cal]);
        assert!(event_by_id(&pool, id).await.unwrap().is_some(),
            "cached events must remain available until the refetch succeeds");
    }

    /// 0003's backfill: an event stored before it landed is missing the guest
    /// list the popover needs, and an unchanged event is never re-delivered by
    /// an incremental sync. Dropping every cursor is what makes the next sync
    /// a full window fetch.
    #[tokio::test]
    async fn the_migration_drops_every_sync_cursor_so_old_rows_get_backfilled() {
        let pool = pool_migrated_below(3, 2).await;

        let cal = seed(&pool).await;
        sqlx::query("INSERT INTO sync_state (calendar_id, sync_token, window_start, window_end)
                     VALUES (?1, 'tok-from-before-the-upgrade', 0, 0)")
            .bind(cal).execute(&pool).await.unwrap();

        apply_migration(&pool, 3).await;

        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_state")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(left, 0, "a surviving cursor means old rows never get their attendees");
    }

    /// 0004 makes the same trade for the same reason. `reminders_json` has
    /// existed unwritten since 0001, so every row stored before this branch
    /// reads as "no reminders" — and would keep reading that way forever,
    /// because an unchanged event is never re-delivered incrementally. A
    /// notification feature reading those rows would simply never fire for
    /// anything already in the store.
    ///
    /// The column half is asserted too: without it there is nowhere for a
    /// calendar's defaults to go, and `SELECT_COLS` fails to compile a query
    /// at all.
    #[tokio::test]
    async fn the_reminders_migration_adds_the_column_and_drops_every_sync_cursor() {
        let pool = pool_migrated_below(4, 3).await;

        let cal = seed(&pool).await;
        sqlx::query("INSERT INTO sync_state (calendar_id, sync_token, window_start, window_end)
                     VALUES (?1, 'tok-from-before-the-upgrade', 0, 0)")
            .bind(cal).execute(&pool).await.unwrap();

        // The column cannot be there yet, or this test would pass against a
        // migration that only ran the `DELETE`.
        assert!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(default_reminders_json) FROM calendars")
                .fetch_one(&pool)
                .await
                .is_err(),
            "fixture check: default_reminders_json must not exist before 0004 runs"
        );

        apply_migration(&pool, 4).await;

        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_state")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(left, 0, "a surviving cursor means old rows never get their reminders");

        // Present, and NULL until the account is next signed in.
        let defaults: Option<String> =
            sqlx::query_scalar("SELECT default_reminders_json FROM calendars WHERE id = ?1")
                .bind(cal)
                .fetch_one(&pool)
                .await
                .expect("0004 must add default_reminders_json to calendars");
        assert!(defaults.is_none(), "an existing calendar has no defaults until it is re-fetched");
    }

    #[tokio::test]
    async fn event_by_id_returns_the_event_and_its_calendars_access_role() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        let id = upsert_event(&pool, &ev(cal, "a", 1000, 2000)).await.unwrap();

        let (got, access_role, _) = event_by_id(&pool, id).await.unwrap().expect("event exists");
        assert_eq!(got.google_id, "a");
        assert_eq!(access_role, "owner", "seed()'s calendar is owned");
    }

    /// The zone an all-day event's *date* is read in, all the way at the far
    /// end of this: `event_detail_impl` derives `EventDetail::start_date` in
    /// it, and read in any other zone that date is a day out on one side of
    /// midnight or the other.
    ///
    /// The calendar's own `timezone` column, never the event's `start_tz`.
    /// `seed`'s calendar and `ev`'s event both say `Europe/Sofia`, which would
    /// let either column pass as the other, so this fixture moves the
    /// *calendar* somewhere else — reading `e.start_tz` by mistake then answers
    /// Sofia and fails here rather than shipping.
    #[tokio::test]
    async fn event_by_id_returns_the_calendars_own_timezone_not_the_events() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        sqlx::query("UPDATE calendars SET timezone = 'America/New_York' WHERE id = ?1")
            .bind(cal).execute(&pool).await.unwrap();
        let id = upsert_event(&pool, &ev(cal, "a", 1000, 2000)).await.unwrap();

        let (got, _, cal_tz) = event_by_id(&pool, id).await.unwrap().expect("event exists");
        assert_eq!(got.start_tz, "Europe/Sofia",
            "fixture check: the event's own zone must differ from its calendar's, or this \
             test cannot tell the two columns apart");
        assert_eq!(cal_tz, "America/New_York");
    }

    #[tokio::test]
    async fn event_by_id_reports_a_read_only_calendars_role() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        sqlx::query("UPDATE calendars SET access_role = 'reader' WHERE id = ?1")
            .bind(cal).execute(&pool).await.unwrap();
        let id = upsert_event(&pool, &ev(cal, "a", 1000, 2000)).await.unwrap();

        let (_, access_role, _) = event_by_id(&pool, id).await.unwrap().expect("event exists");
        assert_eq!(access_role, "reader");
    }

    #[tokio::test]
    async fn event_by_id_returns_none_for_an_unknown_id() {
        let pool = connect_memory().await.unwrap();
        assert!(event_by_id(&pool, 999).await.unwrap().is_none());
    }

    /// Guards the join column itself, not just the value it happens to
    /// produce. With `seed`'s single account/calendar, `c.id` and
    /// `c.account_id` are both 1, so joining on the wrong column returns the
    /// right row by coincidence. `seed_two_accounts` crosses the ids so the
    /// two joins disagree: the event lives on the calendar owned by account
    /// 1, whose *own* `access_role` is `"owner"`, while the calendar that
    /// happens to share its `account_id` with the event's `calendar_id` is a
    /// different row entirely, with `access_role` `"reader"`.
    #[tokio::test]
    async fn event_by_id_returns_the_events_own_calendar_not_one_sharing_its_id() {
        let pool = connect_memory().await.unwrap();
        let (_cal_on_b, cal_on_a) = seed_two_accounts(&pool).await;
        let id = upsert_event(&pool, &ev(cal_on_a, "a", 1000, 2000)).await.unwrap();

        let (got, access_role, _) = event_by_id(&pool, id).await.unwrap().expect("event exists");
        assert_eq!(got.calendar_id, cal_on_a);
        assert_eq!(access_role, "owner",
            "must be cal_on_a's own role, not a calendar that merely shares an id with it");
    }

    #[tokio::test]
    async fn event_for_write_returns_the_calendars_google_id_and_the_owning_accounts_email() {
        let pool = connect_memory().await.unwrap();
        let cal = seed(&pool).await;
        let id = upsert_event(&pool, &ev(cal, "a", 1000, 2000)).await.unwrap();

        let (got, access_role, cal_google_id, account_email) =
            event_for_write(&pool, id).await.unwrap().expect("event exists");
        assert_eq!(got.google_id, "a", "the event's own google_id must not be shadowed");
        assert_eq!(access_role, "owner");
        assert_eq!(cal_google_id, "primary", "seed()'s calendar google_id");
        assert_eq!(account_email, "e@x", "seed()'s account email");
    }

    #[tokio::test]
    async fn event_for_write_returns_none_for_an_unknown_id() {
        let pool = connect_memory().await.unwrap();
        assert!(event_for_write(&pool, 999).await.unwrap().is_none());
    }

    /// The same guard as `event_by_id_returns_the_events_own_calendar_not_one_sharing_its_id`,
    /// extended to the account join: `seed_two_accounts` crosses calendar and
    /// account ids so joining `accounts` on the wrong column (e.g. `a.id =
    /// c.id` instead of `a.id = c.account_id`) still returns *an* account, just
    /// the wrong one.
    #[tokio::test]
    async fn event_for_write_resolves_the_owning_account_not_one_sharing_an_id() {
        let pool = connect_memory().await.unwrap();
        let (_cal_on_b, cal_on_a) = seed_two_accounts(&pool).await;
        let id = upsert_event(&pool, &ev(cal_on_a, "a", 1000, 2000)).await.unwrap();

        let (_, _, cal_google_id, account_email) =
            event_for_write(&pool, id).await.unwrap().expect("event exists");
        assert_eq!(cal_google_id, "cal-on-a");
        assert_eq!(account_email, "a@x",
            "must be account a's own email, not account b's merely sharing an id with cal_on_a");
    }
}
