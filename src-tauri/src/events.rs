use crate::AppState;
use sqlx::SqlitePool;

#[derive(Debug, serde::Serialize)]
pub struct EventDetail {
    pub id: i64,
    pub calendar_id: i64,
    pub title: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub conference_uri: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    /// The first day an all-day event covers, `yyyy-mm-dd`, read in the
    /// **calendar's** zone. `None` for a timed event, which has no date of its
    /// own — it has an instant, and which day that falls on is a question about
    /// the reader, not about the event.
    ///
    /// Here rather than derived in the UI because the UI cannot derive it. The
    /// store holds an instant for an all-day event too, and it is midnight in
    /// the calendar's zone — Google sends a bare `date` and `omacal_sync`
    /// resolves it against `calendars.timezone`. Read back in that zone it is
    /// the date sync put in; read in the browser's it is the previous day for
    /// any user east of the calendar, and the form then shows a trip on the
    /// 10th as starting the 9th before anybody presses Save.
    pub start_date: Option<String>,
    /// The **last** day an all-day event covers — the day a user would point at
    /// — `yyyy-mm-dd`, in the same zone as [`Self::start_date`], and `None` on
    /// the same condition.
    ///
    /// Inclusive, unlike [`Self::end_ms`] and unlike the `endDate` a write
    /// sends, both of which are the exclusive midnight *after* the last day.
    /// The one conversion between the two happens here, so a single-day event
    /// reports the same date twice rather than two different ones the form
    /// would have to reconcile.
    pub end_date: Option<String>,
    pub is_all_day: bool,
    pub is_recurring: bool,
    /// The raw `RRULE`, carried through unchanged so the UI can show a rule it
    /// cannot represent back to the user in words.
    pub recurrence: Option<String>,
    /// Which Repeat option represents [`Self::recurrence`] exactly, or
    /// `"custom"` for a rule this app cannot express — the answer
    /// [`crate::write::repeat_from_rrule`] gives, computed here rather than in
    /// the UI.
    ///
    /// Deliberately not derivable from `recurrence` on the TypeScript side.
    /// The Rust authority matches base rules exactly and admits only one
    /// strictly parsed extension: plain weekly `BYDAY`, whose every fact is
    /// represented by `weekly_days`, plus COUNT/UNTIL endings represented by
    /// `repeat_end`. A second copy in the UI could drift and
    /// silently turn "every 2nd Tuesday" into "weekly" on the next save.
    pub repeat: String,
    /// The Sunday-first weekday codes in a plain weekly `BYDAY` rule. Empty
    /// for bare weekly (whose day is DTSTART) and for every other cadence.
    pub weekly_days: Vec<String>,
    /// Whether the representable recurrence is unbounded, ends on a calendar
    /// date, or stops after a fixed number of occurrences.
    pub repeat_end: crate::write::RepeatEnd,
    pub color: Option<String>,
    pub organizer_email: Option<String>,
    pub self_response: Option<String>,
    pub can_respond: bool,
    pub can_edit: bool,
    pub attendees: Vec<omacal_store::Attendee>,
    /// What this event asks for: the calendar's defaults, or its own
    /// overrides — the store's shape, unconverted (reminders spec §3).
    pub reminders: omacal_store::Reminders,
    /// What "the calendar's defaults" means for this event, so the form can
    /// show the effective rows when `reminders.use_default` — carried with the
    /// event for the reason `StoredEvent` documents: one is the question, the
    /// other the answer, and neither reads alone.
    pub calendar_default_reminders: Vec<omacal_store::Reminder>,
}

/// Whether the RSVP controls are shown at all.
///
/// Three independent reasons to withhold them: the app is in demo mode, the
/// calendar is not writable, or there is no attendee row of yours to change.
/// The last matters as much as the others — an RSVP patch rewrites the whole
/// attendee array, so without a `self` row there is nothing to edit and
/// everything to damage.
///
/// Demo mode is checked here rather than only at the write, because the demo
/// calendars are seeded `owner` with a `self` attendee — everything the other
/// two conditions ask for — so without this the popover offers three buttons
/// that `demo_sync_guard` can only refuse. Plan 1c settled that convention
/// for the same situation: "Sync now" and "Connect" are *hidden* in demo
/// mode, not left to error. The demo popover keeps its guest list, its
/// description and its links; it just does not pretend there is something to
/// answer.
pub(crate) fn can_respond(demo: bool, access_role: &str, attendees: &[omacal_store::Attendee]) -> bool {
    !demo && matches!(access_role, "owner" | "writer") && attendees.iter().any(|a| a.is_self)
}

/// Whether the edit and delete controls are shown at all.
///
/// Deliberately *not* `can_respond` minus its attendee clause: responding
/// needs a `self` attendee row to change, editing does not — you can edit an
/// event nobody else is on. Sharing an implementation would couple two rules
/// that only look alike.
pub(crate) fn can_edit(demo: bool, access_role: &str) -> bool {
    !demo && matches!(access_role, "owner" | "writer")
}

/// Whether an event belongs to a recurring series: either the series master
/// itself (`recurrence` set) or a materialised exception overriding one
/// occurrence of a series (`recurring_event_id` set, with no `recurrence` of
/// its own). A later task shows the "This one / All of them" edit choice
/// from this field, so misreporting either arm either hides that choice on a
/// repeating meeting or offers it on a one-off.
pub(crate) fn is_recurring(recurrence: &Option<String>, recurring_event_id: &Option<String>) -> bool {
    recurrence.is_some() || recurring_event_id.is_some()
}

/// The two dates an all-day event covers, read in the **calendar's** zone
/// `cal_tz`. `None` for a timed event, which has no dates — see
/// [`EventDetail::start_date`].
///
/// The derivation itself is [`crate::write::all_day_span_dates`], shared with
/// the grid so the day the chip is drawn under and the day the popover names
/// cannot drift apart. All this adds is the "is it all-day at all" gate, which
/// is the only part the grid does not want.
fn all_day_dates(event: &omacal_store::StoredEvent, cal_tz: &str) -> Option<(String, String)> {
    if !event.is_all_day {
        return None;
    }
    Some(crate::write::all_day_span_dates(event.start_utc, event.end_utc, cal_tz))
}

#[tauri::command]
pub async fn event_detail(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<EventDetail, String> {
    event_detail_impl(&state, id).await.map_err(|e| crate::errors::user_facing(&e))
}

/// The body of `event_detail`, minus the Tauri `State` wrapper — also the tail
/// end of `respond_to_event` and `refresh_event`, both of which return the
/// freshly-written row through the same shape rather than re-deriving it
/// themselves.
///
/// Takes the whole `&AppState`, not `(&pool, demo)`, for the same reason
/// [`respond_to_event_impl`] and [`refresh_event_impl`] do: the wrapper above
/// then has no argument left to get wrong. Spelled out as two parameters, the
/// wrapper could pass `false` for `demo` and the entire workspace stayed green
/// at 240 passing tests — while the demo popover started offering three RSVP
/// buttons again, the exact thing the gate inside exists to prevent.
pub(crate) async fn event_detail_impl(state: &AppState, id: i64) -> anyhow::Result<EventDetail> {
    let (event, access_role, cal_tz) = omacal_store::event_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("that event is no longer here"))?;

    let can_respond = can_respond(state.demo, &access_role, &event.attendees);
    let is_recurring = is_recurring(&event.recurrence, &event.recurring_event_id);
    let (start_date, end_date) = match all_day_dates(&event, &cal_tz) {
        Some((start, end)) => (Some(start), Some(end)),
        None => (None, None),
    };

    let recurrence_controls = crate::write::recurrence_controls_from_rrule(
        event.recurrence.as_deref(), event.is_all_day, &event.start_tz,
    );
    Ok(EventDetail {
        id: event.id,
        calendar_id: event.calendar_id,
        title: event.summary,
        description: event.description,
        location: event.location,
        conference_uri: event.conference_uri,
        start_ms: event.start_utc,
        end_ms: event.end_utc,
        start_date,
        end_date,
        is_all_day: event.is_all_day,
        is_recurring,
        repeat: recurrence_controls.repeat,
        weekly_days: recurrence_controls.weekly_days,
        repeat_end: recurrence_controls.repeat_end,
        recurrence: event.recurrence,
        color: event.color_hex,
        organizer_email: event.organizer_email,
        self_response: event.self_response,
        can_respond,
        can_edit: can_edit(state.demo, &access_role),
        attendees: event.attendees,
        reminders: event.reminders,
        calendar_default_reminders: event.calendar_default_reminders,
    })
}

/// One attendee as Google's wire shape, with `response_status` overridden.
///
/// **The single field list every write in this app sends an attendee through**,
/// and deliberately only one. Google replaces the attendee array wholesale on
/// both `insert` and `patch`, so a field missing from this function is not
/// merely unrecorded — it is *erased from the real event*, for every guest, on
/// every write. `comment` and `additionalGuests` are here for exactly that
/// reason: nothing in this app reads or writes them, but they are writable
/// per-attendee state that a guest set on Google's own client, and a second
/// copy of this mapping that forgot them would delete other people's notes.
///
/// `is_self` is not sent. It is Google's own annotation of which row is the
/// authenticated user's, computed per request from the credentials, and
/// sending it back is meaningless at best.
fn attendee_json(a: &omacal_store::Attendee, response_status: &str) -> serde_json::Value {
    let mut v = serde_json::json!({
        "email": a.email,
        "responseStatus": response_status,
        "optional": a.optional,
        "additionalGuests": a.additional_guests,
    });
    if let Some(n) = &a.display_name {
        v["displayName"] = serde_json::Value::String(n.clone());
    }
    if let Some(c) = &a.comment {
        v["comment"] = serde_json::Value::String(c.clone());
    }
    v
}

/// Rebuilds the attendee array with only the `self` row's response changed.
/// Every other attendee is copied through by [`attendee_json`], unchanged.
///
/// `None` when no attendee is marked `self`: there is no row of ours to edit,
/// and sending the list anyway would rewrite other people's for no reason.
pub(crate) fn attendees_with_self_response(
    attendees: &[omacal_store::Attendee],
    response: &str,
) -> Option<Vec<serde_json::Value>> {
    if !attendees.iter().any(|a| a.is_self) {
        return None;
    }
    Some(
        attendees
            .iter()
            .map(|a| {
                let status = if a.is_self { response } else { a.response_status.as_str() };
                attendee_json(a, status)
            })
            .collect(),
    )
}

/// The attendee array unchanged — everybody, with the answer they already gave.
///
/// For copying a guest list onto a *different* event, which is what splitting
/// a series with "this and following" does. Nobody's response is touched: the
/// new series is the same meeting continuing, so a guest who accepted it has
/// accepted the second half too, and resetting them all to `needsAction` would
/// ask an entire team to answer an invitation they already answered.
///
/// Unlike [`attendees_with_self_response`] this does not answer `None` for a
/// list with no `self` row. That function returns `None` because there is
/// nothing of *ours* to change; here there is nothing of ours to change either
/// way, and a list without us on it still has to be carried across — dropping
/// it because the signed-in user is not a guest is precisely how the second
/// half of a series loses its entire guest list.
pub(crate) fn attendees_verbatim(attendees: &[omacal_store::Attendee]) -> Vec<serde_json::Value> {
    attendees.iter().map(|a| attendee_json(a, &a.response_status)).collect()
}

/// An address as it is compared. Case-insensitive, trimmed — `Ana@X.com` and
/// `ana@x.com` are one person, and a stored list that spells an address one way
/// must not gain a duplicate because the form spelled it another.
fn same_address(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// **The guest list Google should be sent, given what is stored and what the
/// user asked for. This function is what the whole guest-list design exists
/// for.**
///
/// `attendees` is a **whole-list replace** (spec §2): the array in a PATCH is
/// what the event ends up with, and anything left out is gone. Two things
/// follow, and the second is the dangerous one.
///
/// Removing somebody means sending the list without them — there is no remove
/// call, which is why `wanted` is a target list rather than a set of
/// operations.
///
/// And **every attendee who stays must go out carrying the fields they already
/// had.** Send `{"email": …}` alone for somebody who had accepted and their
/// answer can come back as `needsAction` — on their calendar as well as this
/// one. The popover this app is built around exists to show those answers;
/// wiping them would destroy the exact data it displays, and it would look
/// perfectly fine locally until the next sync brought back a room full of
/// un-answered guests. So an attendee who is merely *kept* is echoed back
/// through [`attendee_json`] — `responseStatus`, `displayName`, `comment`,
/// `additionalGuests` and all — rather than reconstructed from an address.
///
/// That is deliberately safe under either reading of Google's merge semantics.
/// It does not depend on being right about what a partial entry would do.
///
/// **`optional` is the one field `wanted` may overrule**, because it is the one
/// the form lets a user change. Nothing else on this struct can reach an
/// attendee's own data — see [`crate::write::Guest`], which has no field for it.
///
/// The order is stored-first, additions after, so a list whose *membership* is
/// unchanged produces the same array however the form happened to order it.
/// Without that, a re-render that shuffled rows would read as a change and send
/// a whole-list replace nobody asked for.
///
/// A duplicate address is a no-op rather than an error (spec §5): the second
/// mention adds nothing, so it produces no second row. Refusing instead would
/// make the write path answer a question the form should have answered.
pub(crate) fn attendees_for_edit(
    attendees: &[omacal_store::Attendee],
    wanted: &[crate::write::Guest],
) -> Vec<serde_json::Value> {
    let mut out = Vec::new();

    // Everyone who was already on the event and is still wanted, in the order
    // the event holds them, echoed whole.
    for a in attendees {
        let Some(g) = wanted.iter().find(|g| same_address(&g.email, &a.email)) else {
            continue; // removed
        };
        // `optional` is taken from the form; everything else from the store.
        // Cloning the row to overrule one field keeps the echo-back in
        // `attendee_json` rather than spreading a second copy of the field list
        // through here, which is the duplication that drifts.
        let kept = omacal_store::Attendee { optional: g.optional, ..a.clone() };
        out.push(attendee_json(&kept, &a.response_status));
    }

    // Then the newly invited, in the order the form gave them. `responseStatus`
    // is `needsAction` because that is what an un-answered invitation is; it is
    // sent rather than omitted so that every entry in this array has the same
    // shape, and so a reader of the request cannot mistake a new guest for one
    // whose answer was dropped.
    for g in wanted {
        let already_on_event = attendees.iter().any(|a| same_address(&g.email, &a.email));
        let already_added = out
            .iter()
            .any(|v| v["email"].as_str().is_some_and(|e| same_address(e, &g.email)));
        if already_on_event || already_added {
            continue;
        }
        out.push(serde_json::json!({
            "email": g.email.trim(),
            "responseStatus": "needsAction",
            "optional": g.optional,
            "additionalGuests": 0,
        }));
    }

    out
}

/// What the user is told when a guest-list change loses a race.
///
/// A named constant because it is asserted in two places that must not drift:
/// the test that pins it, and `errors.rs`'s `SAFE_EXACT` allowlist, which shows
/// a message verbatim only if it matches one there exactly.
pub(crate) const CONFLICT_GUESTS: &str =
    "somebody else changed this event while you were editing it, so the guest list was not \
     saved — close the form, let it refresh, and make the change again";

/// Which Google event id an RSVP write targets.
#[derive(Debug, PartialEq)]
pub(crate) enum Target {
    /// Patch this id directly.
    Master(String),
    /// Resolve the occurrence through `events.instances` first; `fallback` is
    /// the stored row's own id, used when the row is already a materialised
    /// exception and the lookup finds nothing.
    Instance { master: String, fallback: String },
}

/// Which Google event id an RSVP should patch.
///
/// A one-off event whose row *is* the master but which carries `recurrence`
/// (a series master rendered directly) also has to take the `Instance` path
/// when scope is `"this"`; the caller handles that by passing
/// `recurring_event_id.or(Some(own_id))` for rows carrying `recurrence`, not
/// this function — it stays a pure mapping of the two ids it is given.
pub(crate) fn target_event_id(
    scope: &str,
    recurring_event_id: Option<&str>,
    own_id: &str,
) -> Target {
    match (scope, recurring_event_id) {
        ("all", Some(master)) => Target::Master(master.to_string()),
        ("all", None) => Target::Master(own_id.to_string()),
        (_, Some(master)) => {
            Target::Instance { master: master.to_string(), fallback: own_id.to_string() }
        }
        // Not recurring at all: one event, one id, no lookup.
        (_, None) => Target::Master(own_id.to_string()),
    }
}

/// The `[timeMin, timeMax)` window `events.instances` is bracketed with when
/// resolving "this occurrence" to a concrete Google event id: the *clicked*
/// occurrence's own start, to one second after it.
///
/// `timeMin` is that start exactly, with nothing subtracted. Google documents
/// `timeMin` as an exclusive lower bound on an instance's *end* time, not on
/// its start — an instance comes back when `end > timeMin` and `start <
/// timeMax`. Backing `timeMin` off by a second therefore also admits the
/// *previous* occurrence of a contiguous series: one ending exactly when this
/// one begins clears `end > start - 1s`, and its own start clears `timeMax`.
/// Google orders instances by start, so that predecessor arrives first and
/// [`resolve_instance_id`] takes it — patching the wrong day, with
/// `sendUpdates=all` telling the whole guest list about it. At `timeMin =
/// start` the predecessor fails `end > timeMin` and drops out, while the
/// clicked occurrence (whose end is strictly after its own start) stays.
///
/// Deliberately a function of `occurrence_start_ms` alone, never of the
/// stored row's `start_utc`: every expanded occurrence of a recurring master
/// shares that same database row (`commands::to_ui` gives them all the
/// master's own id), and hence shares its `start_utc` — the series' own
/// DTSTART. Bracketing by that would always resolve to the *first*
/// occurrence of the series, regardless of which day was actually clicked.
pub(crate) fn instance_lookup_window(occurrence_start_ms: i64) -> (String, String) {
    (
        omacal_sync::to_rfc3339(occurrence_start_ms),
        omacal_sync::to_rfc3339(occurrence_start_ms + 1000),
    )
}

/// Chooses which id to patch once `events.instances` has answered.
///
/// `found.first()` is Google's own id for the occurrence — never built by
/// string-formatting the master id and a timestamp, since an all-day event
/// and an already-moved occurrence both format differently.
///
/// *First*, specifically, and not any other member of the list: Google
/// returns instances ordered by start time, and [`instance_lookup_window`]
/// brackets the lookup from the clicked occurrence's own start, so the
/// earliest instance the window can contain is the one that was clicked.
/// Anything after it in the list started later and is a different occurrence.
///
/// When the lookup finds nothing, `fallback` is a safe stand-in *only* when
/// the row was already a materialised exception: there, `master != fallback`,
/// and `fallback` is that exception's own distinct id. When `master ==
/// fallback` the clicked row *is* the series master (the call site offers its
/// own id as both master and fallback for that shape), and falling back to it
/// would silently widen "this occurrence" into "the whole series" — an empty
/// lookup on a bare master has to fail loudly instead of guessing.
pub(crate) fn resolve_instance_id(
    found: &[omacal_google::model::Event],
    master: &str,
    fallback: &str,
) -> anyhow::Result<String> {
    match found.first() {
        Some(i) => Ok(i.id.clone()),
        None if master != fallback => Ok(fallback.to_string()),
        None => anyhow::bail!("could not find that occurrence on the calendar"),
    }
}

/// Copies onto `row` the fields a patch (or a refresh) response actually
/// carries: `etag` and `sequence`, so the next write's conflict check is
/// against the new version; `attendees`, so the guest list reflects what
/// Google now has; and `self_response`, derived the same way sync derives it
/// — Google does not return it as a field of its own — so the week grid's
/// block styling updates immediately instead of waiting for the next sync.
pub(crate) fn merge_patched(row: &mut omacal_store::StoredEvent, patched: &omacal_google::model::Event) {
    row.etag = patched.etag.clone();
    row.sequence = patched.sequence;
    row.attendees = patched.attendees.iter().map(omacal_sync::from_google_attendee).collect();
    row.self_response = row.attendees.iter().find(|a| a.is_self).map(|a| a.response_status.clone());
}

/// `occurrence_start_ms` is the `start_ms` of the block the user actually
/// clicked — see [`instance_lookup_window`] for why this cannot be derived
/// from the stored row instead.
#[tauri::command]
pub async fn respond_to_event(
    state: tauri::State<'_, AppState>,
    id: i64,
    response: String,
    scope: String,
    occurrence_start_ms: i64,
) -> Result<EventDetail, String> {
    respond_to_event_impl(&state, id, &response, &scope, occurrence_start_ms)
        .await
        .inspect(|_| crate::upcoming::refresh_soon(state.pool.clone(), state.demo))
}

/// The body of `respond_to_event`, minus the Tauri `State` wrapper so the
/// demo gate is reachable from a test — the same split `sign_in_impl` uses,
/// and for the same reason: a gate that exists only inside a
/// `#[tauri::command]` cannot be exercised without a running app, and an
/// unexercised gate is one a future edit deletes in silence.
///
/// The gate is the first statement. Everything past it reads the config file,
/// the Keychain and Google, and then *writes to somebody's real calendar* —
/// the first thing in this app that does.
pub(crate) async fn respond_to_event_impl(
    state: &AppState,
    id: i64,
    response: &str,
    scope: &str,
    occurrence_start_ms: i64,
) -> Result<EventDetail, String> {
    crate::demo_sync_guard(state.demo)?;
    respond_impl(state, id, response, scope, occurrence_start_ms)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
}

/// Sends an RSVP to Google and folds the result back into the local store.
///
/// `scope` is `"this"` (just the occurrence being viewed) or `"all"` (the
/// whole series); which Google event id that resolves to is
/// [`target_event_id`]. Everything past building the `CalendarClient` lives
/// in [`respond_via_client`], split out purely so a test can hand it a
/// client pointed at a `wiremock` server instead of this function touching
/// `load_config` or the Keychain — the same split `sync_accounts` (in
/// `lib.rs`) uses for its access-token source.
async fn respond_impl(
    state: &AppState,
    id: i64,
    response: &str,
    scope: &str,
    occurrence_start_ms: i64,
) -> anyhow::Result<EventDetail> {
    let (ev, access_role, cal_google_id, account_email) = omacal_store::event_for_write(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("that event is no longer here"))?;

    if !can_respond(state.demo, &access_role, &ev.attendees) {
        anyhow::bail!("this calendar cannot be answered from omacal");
    }

    // The same answer on a CalDAV account is a PARTSTAT rewrite inside the
    // event's own resource — no Google client, no attendee body. The write
    // path owns everything from here, guards included.
    if crate::caldav_write::is_caldav_calendar(&state.pool, ev.calendar_id).await? {
        return crate::caldav_write::respond(
            state, id, response, scope, occurrence_start_ms, &account_email,
        )
        .await;
    }

    let body_attendees = attendees_with_self_response(&ev.attendees, response)
        .ok_or_else(|| anyhow::anyhow!("you are not a guest on this event"))?;

    let cfg = crate::load_config()?;
    let token = crate::access_token_for(state, &cfg, &account_email).await?;
    let client = omacal_google::CalendarClient::new(crate::GOOGLE_CALENDAR_API, &token);

    respond_via_client(
        &state.pool,
        response,
        scope,
        occurrence_start_ms,
        ev,
        &cal_google_id,
        body_attendees,
        &client,
    )
    .await?;

    event_detail_impl(state, id).await
}

/// The network exchange and local write-back half of [`respond_impl`], with
/// the `CalendarClient` already built.
///
/// Returns nothing: reading the freshly-written row back is the caller's job,
/// since only it holds the `AppState` [`event_detail_impl`] needs — this
/// function is handed a bare pool precisely so a test can drive it without
/// one.
#[allow(clippy::too_many_arguments)]
async fn respond_via_client(
    pool: &SqlitePool,
    response: &str,
    scope: &str,
    occurrence_start_ms: i64,
    ev: omacal_store::StoredEvent,
    cal_google_id: &str,
    body_attendees: Vec<serde_json::Value>,
    client: &omacal_google::CalendarClient,
) -> anyhow::Result<()> {
    // A row carrying `recurrence` is a series master; scope "this" must still
    // go through instance resolution for it, which is why `own_id` is offered
    // as the master when there is no `recurring_event_id`.
    let series = ev
        .recurring_event_id
        .as_deref()
        .or_else(|| ev.recurrence.as_ref().map(|_| ev.google_id.as_str()));
    let target = target_event_id(scope, series, &ev.google_id);

    // The resolved instance is kept, not just its id: `event_instances` asks
    // for no `fields` mask, so each item is a full event — `etag` and
    // `attendees` included — and the rule below needs both. Re-fetching it
    // would spend a request on data already in hand.
    let (event_id, instance) = match &target {
        Target::Master(master_id) => (master_id.clone(), None),
        Target::Instance { master, fallback } => {
            let (time_min, time_max) = instance_lookup_window(occurrence_start_ms);
            let found = client.event_instances(cal_google_id, master, &time_min, &time_max).await?;
            let id = resolve_instance_id(&found, master, fallback)?;
            let inst = found.iter().find(|i| i.id == id).cloned();
            (id, inst)
        }
    };

    // ---------------------------------------------------------------------
    // The provenance rule, which the rest of this function is one expression
    // of: THE BODY AND THE ETAG MUST BOTH COME FROM THE RESOURCE BEING
    // PATCHED — never from the row that happened to be on screen.
    //
    // `ev` is that row. Its `attendees` and its `etag` describe
    // `ev.google_id` and nothing else, and by this point `event_id` is not
    // always `ev.google_id`. Google replaces the attendee array wholesale on
    // patch, and every patch here goes out with `sendUpdates=all`, so sending
    // one resource's guest list to another does not merely mis-record
    // something: it overwrites other people's answers and then emails them
    // about it.
    //
    // Three separate bugs on this branch have been that one sentence:
    //
    //   * scope "this" on a series master resolves to an instance id. In
    //     Google Calendar a guest answering "this event" is itself what
    //     materialises that instance, and this store does not see the
    //     resulting exception until the next sync — up to one interval, with
    //     `suppressed_slots` rendering the master meanwhile. Patching with
    //     the master's array in that window reverts their answer.
    //
    //   * scope "all" from an exception row targets the *master*, which is
    //     again not `ev`. The exception is where a per-occurrence answer
    //     lives, so sending its array to the master applies one occurrence's
    //     answers to the entire series.
    //
    //   * the 412 arm below, which has always re-read before retrying — the
    //     one place that got this right from the start, and the shape the two
    //     above are now brought into line with.
    //
    // So: same resource, use the row. Different resource, describe *it* —
    // from the instance already in hand, or by fetching it.
    //
    // The fetch happens on exactly one branch, scope "all" from an exception
    // row, and it is that branch's *first* request, not an extra one on top
    // of others: nothing precedes it, because a `Target::Master` never does
    // an instances lookup. It takes that path from one request to two. Every
    // other path is unchanged — one for a one-off, one for the whole series
    // from a master, two for "this occurrence".
    let (body_attendees, if_match) = if event_id == ev.google_id {
        (body_attendees, ev.etag.clone())
    } else {
        let target_event = match instance {
            Some(inst) => inst,
            None => client.get_event(cal_google_id, &event_id).await?,
        };
        let target_attendees: Vec<omacal_store::Attendee> =
            target_event.attendees.iter().map(omacal_sync::from_google_attendee).collect();
        // Not a guest on the resource being patched means there is nothing of
        // ours to change on it. Falling back to `ev`'s array here would be
        // the very write this rule exists to prevent, so it fails instead.
        let from_target = attendees_with_self_response(&target_attendees, response)
            .ok_or_else(|| anyhow::anyhow!("you are not a guest on this event"))?;
        (from_target, target_event.etag)
    };

    let body = serde_json::json!({ "attendees": body_attendees });
    let patched = match client
        .patch_event(cal_google_id, &event_id, &body, "all", if_match.as_deref())
        .await
    {
        Ok(p) => p,
        Err(omacal_google::ApiError::PreconditionFailed) => {
            // Someone edited the event while the popover was open. Re-read,
            // re-apply our answer to the list as it is now, and try once more —
            // retrying with the same stale list would overwrite their change.
            let fresh = client.get_event(cal_google_id, &event_id).await?;
            let fresh_attendees: Vec<omacal_store::Attendee> =
                fresh.attendees.iter().map(omacal_sync::from_google_attendee).collect();
            let retry = attendees_with_self_response(&fresh_attendees, response)
                .ok_or_else(|| anyhow::anyhow!("you are not a guest on this event"))?;
            client
                .patch_event(
                    cal_google_id,
                    &event_id,
                    &serde_json::json!({ "attendees": retry }),
                    "all",
                    fresh.etag.as_deref(),
                )
                .await?
        }
        Err(e) => return Err(e.into()),
    };

    // Close the loop locally — but only when the patch actually targeted the
    // row we loaded. When scope "this" resolved to a *different* Google event
    // id (a series master rendered directly, or an exception this store has
    // no local row for yet), `ev.google_id` names a row that is not the one
    // Google just changed: stamping the instance's etag/attendees onto it
    // would corrupt that row outright, since `upsert_event` is keyed on
    // `(calendar_id, google_id)` and would write straight onto it. Leave it
    // for the next sync to materialise correctly instead of guessing.
    if event_id == ev.google_id {
        let mut row = ev;
        merge_patched(&mut row, &patched);
        omacal_store::upsert_event(pool, &row).await?;
    }

    Ok(())
}

#[tauri::command]
pub async fn refresh_event(state: tauri::State<'_, AppState>, id: i64) -> Result<EventDetail, String> {
    refresh_event_impl(&state, id).await
}

/// The body of `refresh_event`, split for the same reason as
/// [`respond_to_event_impl`]: the demo gate is the first statement, and it
/// has to be reachable without a running app or nothing proves it is there.
/// This one only reads from Google, but it reads with a real account's access
/// token, and demo mode has no account to read as.
async fn refresh_event_impl(state: &AppState, id: i64) -> Result<EventDetail, String> {
    crate::demo_sync_guard(state.demo)?;
    refresh_impl(state, id).await.map_err(|e| crate::errors::user_facing(&e))
}

/// Re-pulls one event from Google and folds it back in, the same shape as
/// `respond_via_client` minus the patch: `get_event`, [`merge_patched`],
/// `upsert_event`, then the fresh detail. Used to pick up a change made
/// elsewhere — another attendee's answer, a moved time — while the popover was
/// open. Its failures are the caller's to ignore: whatever `EventDetail` is
/// already on screen is still valid if this does not succeed.
async fn refresh_impl(state: &AppState, id: i64) -> anyhow::Result<EventDetail> {
    let (ev, _access_role, cal_google_id, account_email) = omacal_store::event_for_write(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("that event is no longer here"))?;

    let cfg = crate::load_config()?;
    let token = crate::access_token_for(state, &cfg, &account_email).await?;
    let client = omacal_google::CalendarClient::new(crate::GOOGLE_CALENDAR_API, &token);

    let fresh = client.get_event(&cal_google_id, &ev.google_id).await?;

    let mut row = ev;
    merge_patched(&mut row, &fresh);
    omacal_store::upsert_event(&state.pool, &row).await?;

    event_detail_impl(state, id).await
}

#[tauri::command]
pub async fn create_event(
    state: tauri::State<'_, AppState>,
    calendar_id: i64,
    fields: crate::write::EventInput,
    send_updates: String,
) -> Result<EventDetail, String> {
    let fields = crate::write::fields_from_input(fields)?;
    create_impl(&state, calendar_id, fields, &send_updates)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
        .inspect(|_| crate::upcoming::refresh_soon(state.pool.clone(), state.demo))
}

/// The body of `create_event`, minus the Tauri `State` wrapper — the same
/// split `respond_impl` gets, and for the same reason: a test can hand
/// [`create_via_client`] a client pointed at `wiremock` without this function
/// touching `load_config` or the Keychain.
///
/// Unlike `respond_to_event`/`respond_impl`, there is only one layer here
/// rather than two: `respond_impl` needs its own inner demo/writability check
/// (`can_respond`) because `respond_to_event_impl`'s outer one guards a
/// *different* command (`refresh_event` shares no gate with it), and because
/// `can_respond` folds in a third condition — a `self` attendee row — that
/// has nothing to do with demo mode or access role. Creating has no second
/// caller and no third condition, so both checks live here, in the order
/// that matters: demo first, before the calendar is even looked up, so a
/// demo run never touches the database at all; writability second, so a
/// reader calendar is refused before `load_config`, the Keychain, or Google
/// ever see the request.
async fn create_impl(
    state: &AppState,
    calendar_id: i64,
    fields: crate::write::EventFields,
    send_updates: &str,
) -> anyhow::Result<EventDetail> {
    if state.demo {
        anyhow::bail!("demo mode — there is nothing to create");
    }

    // Same ordering rule: a pure check of the argument, decided before any
    // row is read (reminders spec §4).
    if let Some(r) = &fields.reminders {
        crate::write::validate_reminders(r).map_err(|m| anyhow::anyhow!(m))?;
    }

    // A CalDAV calendar takes the resource path: rewrite, etag-guarded PUT,
    // resync. `send_updates` does not travel — CalDAV has no notify question,
    // and the form never asks one on these calendars.
    if crate::caldav_write::is_caldav_calendar(&state.pool, calendar_id).await? {
        return crate::caldav_write::create(state, calendar_id, fields).await;
    }

    let (cal_google_id, access_role, account_email, cal_tz) =
        omacal_store::calendar_for_write(&state.pool, calendar_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("that calendar is no longer here"))?;

    // Reuses `can_edit`'s own rule for "owner or writer" rather than
    // repeating the match here — the same rule `EventDetail::can_edit` is
    // built from, so a calendar that shows an Edit button cannot silently
    // refuse the create it implies. `demo: false` because that half of
    // `can_edit` was already handled above, with its own message and before
    // any database access at all.
    if !can_edit(false, &access_role) {
        anyhow::bail!("this calendar is not writable from omacal");
    }

    let cfg = crate::load_config()?;
    let token = crate::access_token_for(state, &cfg, &account_email).await?;
    let client = omacal_google::CalendarClient::new(crate::GOOGLE_CALENDAR_API, &token);

    let id = create_via_client(
        &state.pool, calendar_id, &cal_google_id, &cal_tz, fields, send_updates, &client,
    )
    .await?;

    // The same guard one level up: the row is created AND stored by now, so a
    // failed read-back must not read as a failed create either. (The stored
    // half makes the sentence's "next sync" a mild overstatement — a mere
    // reload would show it — but "already saved, do not create it again" is
    // the part that matters, and one sentence for one situation beats two the
    // safelist has to carry.)
    event_detail_impl(state, id).await.map_err(|e| {
        tracing::error!(%e, id, "created and stored, but the read-back failed");
        anyhow::anyhow!(CREATED_NOT_STORED)
    })
}

/// The request-building and local write-back half of [`create_impl`], with
/// the `CalendarClient` already built — parallel to `respond_via_client`,
/// and handed a bare pool for the same reason: so a test can drive it
/// without an `AppState`.
///
/// Built from `fields` directly, not [`crate::write::changed_fields`]: that
/// function's whole point is "only send what changed from a *before*", and a
/// create has no before — every field on it is new.
///
/// `cal_tz` is the *calendar's own* stored timezone (`calendars.timezone`,
/// via `calendar_for_write`), deliberately not `fields.tz` — the zone the
/// event happens to be authored in. For a timed event the two never diverge
/// in practice: `when_json` sends `dateTime` with an explicit offset,
/// `resolve` (in `omacal-sync`) parses that offset directly and never
/// consults `cal_tz` at all. An all-day event is the case that makes them
/// diverge: Google's wire format for `start`/`end` there is a bare `date`
/// with no zone of its own, so `resolve` *always* falls back to `cal_tz` to
/// turn "2026-08-10" into an instant — and sync always resolves every other
/// all-day row on this calendar against `calendars.timezone`. Passing
/// `fields.tz` here instead would store this one row at a different instant
/// than the very next sync recomputes it at, until sync corrects it.
///
/// `fields.tz` is still handed to `when_json`, and for an all-day create it is
/// now inert rather than merely unused: [`crate::write::When::AllDay`] carries
/// the date the user picked and `when_json`'s date arm never looks at a zone.
/// The date that goes out is the date the form sent, and `cal_tz` alone decides
/// which instant it is stored at.
///
/// Returns the local row id rather than an `EventDetail`: reading the
/// freshly-written row back needs the `AppState` [`event_detail_impl`] takes,
/// which this function is deliberately not handed — the same reason
/// `respond_via_client` returns nothing and leaves that step to its caller.
async fn create_via_client(
    pool: &SqlitePool,
    calendar_id: i64,
    cal_google_id: &str,
    cal_tz: &str,
    fields: crate::write::EventFields,
    send_updates: &str,
    client: &omacal_google::CalendarClient,
) -> anyhow::Result<i64> {
    let f = &fields;
    let (start, end) = crate::write::when_json(&f.when, &f.tz);
    let mut body = serde_json::json!({ "start": start, "end": end });
    if let Some(s) = &f.summary     { body["summary"]     = s.clone().into(); }
    if let Some(s) = &f.location    { body["location"]    = s.clone().into(); }
    if let Some(s) = &f.description { body["description"] = s.clone().into(); }
    if let Some(Some(rule)) = &f.recurrence {
        body["recurrence"] = serde_json::json!([rule]);
    }
    // Unlike `guests`, read on the create path too: a reminder invites nobody
    // and mails nobody, so there is no notify question to defer (reminders
    // spec §2). Absent means Google applies the calendar's defaults, which is
    // what an untouched form asked for.
    if let Some(r) = &f.reminders {
        body["reminders"] = crate::write::reminders_json(r);
    }

    // **The guest list, through the edit path's own builder.**
    //
    // `attendees_for_edit(&[], wanted)` — an empty "already on the event" list,
    // because a brand-new event has nobody on it — so a guest invited here has
    // exactly the shape a guest invited by an edit has: `needsAction`, with an
    // explicit `optional` and `additionalGuests`. A second array written out
    // here would be a second authority on that shape, and the one it would
    // drift towards is the flat `{email}` that resets an RSVP (guest-list spec
    // §2).
    //
    // Absent for an empty list rather than `attendees: []`. On a create there
    // is nobody to remove, so the two produce the same event and absent is the
    // smaller claim. (On an *edit* they differ absolutely — see
    // `EventFields::guests` — which is why this reads `is_some_and(!is_empty)`
    // and not a bare `if let`.)
    if let Some(wanted) = f.guests.as_ref().filter(|g| !g.is_empty()) {
        body["attendees"] = serde_json::Value::Array(attendees_for_edit(&[], wanted));
    }

    // **The caller's answer, never a constant.** This was `"none"` while a
    // create could not invite anybody, which was correct for an event with no
    // attendees and is exactly what stops being true above. Guest-list spec §3
    // makes it a choice; the form asks and this carries the answer. The other
    // caller of `insert_event` — the series split in [`split_series`] — passes
    // `"all"` for its own reasons. See `insert_event`'s own doc comment.
    let created = client.insert_event(cal_google_id, &body, send_updates).await?;

    // **Past this line, no error may read as "the create failed."** The event
    // exists on Google and any invited guest has already been mailed; a
    // failure below reported like the ones above invites the user to create
    // it again, and the second attempt mails the whole list twice. Both
    // remaining steps therefore collapse into [`CREATED_NOT_STORED`] — a
    // fixed, safelisted sentence that says what actually happened and what
    // will heal it (the next sync fetches the event like any other) — with
    // the underlying cause kept for the log.
    //
    // The mapping is the same Google -> StoredEvent conversion `omacal-sync`
    // uses for every event it writes locally, so a row created here is shaped
    // identically to one that arrived through an ordinary sync.
    let Some(row) = omacal_sync::to_stored(&created, calendar_id, cal_tz) else {
        tracing::error!(google_id = %created.id,
            "created on Google, but the response could not be mapped for storage");
        anyhow::bail!(CREATED_NOT_STORED);
    };

    match omacal_store::upsert_event(pool, &row).await {
        Ok(id) => Ok(id),
        Err(e) => {
            tracing::error!(%e, google_id = %created.id,
                "created on Google, but the local write failed");
            anyhow::bail!(CREATED_NOT_STORED)
        }
    }
}

/// What a create reports when Google succeeded and the local half did not.
///
/// A fixed literal, safelisted verbatim in `errors.rs`: the one thing this
/// message must never do is look like a failed create, because the natural
/// answer to a failed create is to try again — and trying again mails every
/// guest a second invitation. `App.svelte` recognises the sentence and runs
/// its ordinary post-write sync instead of stopping, which is the heal the
/// sentence promises.
pub(crate) const CREATED_NOT_STORED: &str =
    "The event was created on Google, but omacal could not record it locally. \
     The next sync will bring it in — do not create it again.";

/// Which timezone an edit puts on both sides of its diff.
///
/// A timed event keeps the zone it is *stored* in, never the zone of the
/// machine doing the editing: the instant travels in the epoch milliseconds,
/// and `timeZone` only says which zone the event is displayed in. Taking the
/// authoring zone would re-zone a New York meeting the moment somebody in
/// Sofia touched its title.
///
/// An all-day event takes the *calendar's* own zone instead, for the reason
/// [`create_via_client`] spells out: Google's wire format for an all-day
/// `start`/`end` is a bare `date` with no zone of its own, so `resolve` (in
/// `omacal-sync`) always falls back to `calendars.timezone` — and an event
/// written against a different zone lands at a different instant than the very
/// next sync recomputes it at.
///
/// Both sides of the diff go through here, so the `tz` term in
/// [`crate::write::changed_fields`]' times trigger cannot fire on its own: a
/// zone only changes when the all-day flag does, which already triggers it.
pub(crate) fn edit_zone<'a>(is_all_day: bool, cal_tz: &'a str, event_tz: &'a str) -> &'a str {
    if is_all_day {
        cal_tz
    } else {
        event_tz
    }
}

/// The PATCH body for an edit: [`crate::write::changed_fields`] with both
/// sides built here, because how each side is built *is* the safety argument.
///
/// **The text fields come from `ev`** — the row the user was looking at — so
/// "changed" means "changed by the user", not "differs from whatever Google
/// holds right now". Diffing against a freshly-read copy instead would turn
/// somebody else's concurrent rename into an apparent edit and send the stale
/// title back over it. Fields the user did not touch are simply absent, which
/// is a PATCH's way of saying "leave it alone", so the other change survives.
///
/// **The times come from the resource being patched.** `after`'s instants are
/// in the *clicked occurrence's* coordinates: the form was pre-filled from the
/// block on screen, which for a series is one expansion of a master anchored
/// somewhere else entirely. Sending those instants to the master verbatim
/// would move the series' DTSTART onto the edited occurrence's date and drop
/// every occurrence before it. So a time change reaches the target as the
/// *movement* the user made, applied to the target's own start and end
/// through [`crate::write::shifted_like`] — a calendar movement rather than a
/// millisecond delta, because master and occurrence can sit on opposite sides
/// of a daylight-saving transition. An untouched time is a movement of zero,
/// and the body carries no `start`/`end` at all.
///
/// **The anchor is a constant across the 412 retry, and that is load
/// bearing.** `target_start_ms` is re-read on the retry, so anchoring on it
/// would make the movement absolute rather than relative: for a one-off whose
/// time somebody else had just changed, the *absence* of a user edit would
/// come back as the *presence* of a revert, rescheduling their move and
/// mailing the guest list. Both arms below are values the retry cannot move —
/// the clicked occurrence, or the row as it was loaded.
///
/// `occurrence_start_ms` is the anchor only when the row actually has
/// occurrences. For a one-off it is redundant — the target *is* the event —
/// and using it anyway would let a wrong value from the caller move an event
/// nobody asked to move. `is_recurring` rather than `recurrence.is_some()`:
/// a materialised exception carries no rule of its own but is still one
/// occurrence of a series, and `"all"` from that row patches a master anchored
/// somewhere else entirely.
///
/// `anchor_end` is derived rather than passed: the occurrence's own end is the
/// target's end moved by the same span that separates the anchor from the
/// target's start, which is precisely what an expansion of a series is. That
/// keeps a *duration* change (the user lengthened the meeting) distinguishable
/// from a *move*, and both correct across a transition.
///
/// `before.recurrence` is `None` and must stay that way: `changed_fields`
/// never reads it, because the touched/untouched signal for Repeat lives
/// entirely in `after` (`None` = the user did not touch it, `Some(None)` =
/// they chose Never). Setting it to the event's real rule here would not
/// "improve" the diff — it would do nothing at all, while reading as though
/// the rule were being compared.
pub(crate) fn edit_patch_body(
    ev: &omacal_store::StoredEvent,
    target_start_ms: i64,
    target_end_ms: i64,
    occurrence_start_ms: i64,
    cal_tz: &str,
    after: &crate::write::EventFields,
) -> serde_json::Value {
    // The zone the *movement* is read in, and the one the before side is
    // described in: the event as it stands, not as the form would leave it. A
    // user toggling all-day is already resending both ends anyway, since the
    // two `When` variants can never compare equal.
    let zone = edit_zone(ev.is_all_day, cal_tz, &ev.start_tz);

    let before = crate::write::EventFields {
        summary: ev.summary.clone(),
        location: ev.location.clone(),
        description: ev.description.clone(),
        when: when_of(ev.is_all_day, target_start_ms, target_end_ms, zone),
        tz: zone.to_string(),
        // `None`, always — `changed_fields` never reads this side. See above.
        recurrence: None,
        // Likewise: the guest list is not diffed by `changed_fields` at all. It
        // is compared below against the row's own attendees, which carry fields
        // `EventFields` has no room for.
        guests: None,
        // And likewise again: compared below against `ev.reminders`, the row's
        // own settings, not against a before-side this struct would carry.
        reminders: None,
    };

    let anchor = if is_recurring(&ev.recurrence, &ev.recurring_event_id) {
        occurrence_start_ms
    } else {
        ev.start_utc
    };
    let anchor_end = crate::write::shifted_like(anchor, target_start_ms, target_end_ms, zone);
    let after = crate::write::EventFields {
        when: shifted_when(
            &after.when,
            target_start_ms,
            target_end_ms,
            anchor,
            anchor_end,
            zone,
        ),
        tz: edit_zone(after.when.is_all_day(), cal_tz, &ev.start_tz).to_string(),
        ..after.clone()
    };

    let mut body = crate::write::changed_fields(&before, &after);

    // **The guest list, and only when it actually changed.**
    //
    // Here rather than in `changed_fields`, and that is a decision rather than
    // an accident. `changed_fields` compares two `EventFields`, and the *before*
    // side of a guest list is not one: it is `ev.attendees`, which carries
    // `responseStatus`, `displayName`, `comment` and `additionalGuests` — the
    // fields §2 is about, and the ones `EventFields` deliberately has no room
    // for. Putting the rule there would mean dragging a store type into
    // `write.rs`'s pure builders, or thinning the attendee down to what
    // `EventFields` can hold, which is precisely the loss the rule exists to
    // prevent.
    //
    // Here it is also **structural**: this is the one function that builds an
    // edit's PATCH body, it is called twice (once for the request, once for the
    // 412 retry), and `attendee_json` — the field list being echoed — already
    // lives beside it. A caller that forgot the rule is not a shape this file
    // admits, because there is no other place for a caller to build a body.
    //
    // Compared against [`attendees_verbatim`] rather than against a second
    // notion of "the same list": that function is what an *unchanged* list
    // serializes to, so equality here means exactly "the user changed nothing",
    // by construction and with no rule to keep in step. An absent `attendees`
    // is a PATCH saying leave the list alone, which is the only safe thing to
    // send for an event whose guests nobody touched.
    if let Some(wanted) = &after.guests {
        let sending = attendees_for_edit(&ev.attendees, wanted);
        if sending != attendees_verbatim(&ev.attendees) {
            body["attendees"] = serde_json::json!(sending);
        }
    }

    // **Reminders, and only when they actually changed** — the guest rule one
    // paragraph up, applied to a simpler object: both sides carry exactly
    // `method` and `minutes`, so the stored row converts losslessly into the
    // wire shape and equality means "the user changed nothing" with no echo
    // subtleties. An absent `reminders` is a PATCH saying leave them alone
    // (reminders spec §2), which matters doubly here because the object is a
    // whole replace: a form that always resent it would rewrite the event's
    // settings from whatever omacal last read.
    if let Some(wanted) = &after.reminders {
        if *wanted != reminders_as_input(&ev.reminders) {
            body["reminders"] = crate::write::reminders_json(wanted);
        }
    }

    body
}

/// The stored settings in the wire shape, so `edit_patch_body` can ask "is
/// this what the form sent back?" with plain equality.
pub(crate) fn reminders_as_input(r: &omacal_store::Reminders) -> crate::write::RemindersInput {
    crate::write::RemindersInput {
        use_default: r.use_default,
        overrides: r
            .overrides
            .iter()
            .map(|o| crate::write::ReminderInput { method: o.method.clone(), minutes: o.minutes })
            .collect(),
    }
}

/// The resource being patched, as a [`crate::write::When`].
///
/// An all-day row is described by the **date** it falls on rather than the
/// instant the store holds, and that derivation is lossless for one reason
/// only: `zone` here is [`edit_zone`]'s answer, which for an all-day event is
/// the *calendar's* zone — the same one `omacal_sync::resolve` built the
/// instant with. Read in any other zone the date comes back a day out on one
/// side of midnight, which is the whole defect this plan closes.
fn when_of(is_all_day: bool, start_ms: i64, end_ms: i64, zone: &str) -> crate::write::When {
    if is_all_day {
        crate::write::When::AllDay {
            start_date: crate::write::date_in_zone(start_ms, zone),
            end_date: crate::write::date_in_zone(end_ms, zone),
        }
    } else {
        crate::write::When::Timed { start_ms, end_ms }
    }
}

/// The form's `when`, re-expressed in the target resource's own coordinates.
///
/// This is Plan 5's anchoring rule, and it is the reason a title-only save on a
/// series sends no times at all: what reaches the target is the *movement* the
/// user made, applied to the target's own start and end — never the form's
/// values verbatim. Sent verbatim to a series master anchored months earlier,
/// they would drag its DTSTART onto the clicked occurrence's date and drop
/// every occurrence before it. See [`edit_patch_body`]'s doc comment for the
/// anchor's own provenance and why it must survive the 412 retry unchanged.
///
/// Both arms measure the same movement; only the units differ, and the
/// difference is the point of this plan.
///
/// * **Timed** — instants, through [`crate::write::shifted_like`], which
///   measures the movement *civilly* so a daylight-saving transition between
///   the anchor and the target cannot turn a day into 23 hours.
/// * **All-day** — dates, through [`crate::write::shifted_date`], which is
///   whole days and no zone at all. `zone` reaches this arm only to name the
///   dates the target and the anchor already sit on, never to build one.
///
/// The arm is chosen by the **form's** variant, not the row's, so the two
/// mixed cases fall out rather than needing a case of their own: turning
/// all-day off leaves a real pair of instants to shift, and turning it on
/// shifts the dates those instants fall on. Either way the variant differs
/// from the before side, so both ends are resent — which is correct, since the
/// event is being redefined.
///
/// One residual, worth stating rather than discovering. For a *timed* row the
/// form's date is still the browser's reading of the row's instant, so toggling
/// all-day on from a zone east or west of the event's own can name the
/// neighbouring day. That is the display half of the same boundary, closed for
/// all-day rows by this plan's later tasks and out of scope for timed ones —
/// §3 of the design keeps instants for those deliberately. It is bounded: the
/// result is still a shift relative to the anchor, so a series' start can move
/// by a day but can never land on the clicked occurrence.
fn shifted_when(
    form: &crate::write::When,
    target_start_ms: i64,
    target_end_ms: i64,
    anchor: i64,
    anchor_end: i64,
    zone: &str,
) -> crate::write::When {
    match form {
        crate::write::When::Timed { start_ms, end_ms } => crate::write::When::Timed {
            start_ms: crate::write::shifted_like(target_start_ms, anchor, *start_ms, zone),
            end_ms: crate::write::shifted_like(target_end_ms, anchor_end, *end_ms, zone),
        },
        crate::write::When::AllDay { start_date, end_date } => crate::write::When::AllDay {
            start_date: crate::write::shifted_date(
                &crate::write::date_in_zone(target_start_ms, zone),
                &crate::write::date_in_zone(anchor, zone),
                start_date,
            ),
            end_date: crate::write::shifted_date(
                &crate::write::date_in_zone(target_end_ms, zone),
                &crate::write::date_in_zone(anchor_end, zone),
                end_date,
            ),
        },
    }
}

/// One wire event as a store row, through the same mapping every sync uses.
///
/// `to_stored` answers `None` for two unrelated reasons, and they get separate
/// messages because only one of them is worth showing. A tombstone is the
/// ordinary case — somebody deleted the occurrence between the popover opening
/// and the save — and it is allowlisted in `errors.rs`, since a user who is
/// told that knows exactly what happened. Times that will not parse are a
/// shape nobody has seen; that one stays opaque rather than claiming something
/// specific and being wrong about it.
///
/// Either way this stops instead of guessing: the times it would have to
/// invent are the ones the request is built against.
fn row_from_wire(
    wire: &omacal_google::model::Event,
    calendar_id: i64,
    cal_tz: &str,
) -> anyhow::Result<omacal_store::StoredEvent> {
    if omacal_sync::is_tombstone(wire) {
        anyhow::bail!("that occurrence is no longer on the calendar");
    }
    omacal_sync::to_stored(wire, calendar_id, cal_tz)
        .ok_or_else(|| anyhow::anyhow!("Google returned an event omacal could not read"))
}

/// `occurrence_start_ms` is the `start_ms` of the block the user actually
/// clicked — see [`instance_lookup_window`] for why this cannot be derived
/// from the stored row instead. `scope` is `"this"` or `"all"`.
#[tauri::command]
pub async fn update_event(
    state: tauri::State<'_, AppState>,
    id: i64,
    scope: String,
    occurrence_start_ms: i64,
    fields: crate::write::EventInput,
    send_updates: String,
) -> Result<EventDetail, String> {
    let fields = crate::write::fields_from_input(fields)?;
    update_impl(
        &state,
        id,
        &scope,
        occurrence_start_ms,
        fields,
        &send_updates,
    )
    .await
    .map_err(|e| crate::errors::user_facing(&e))
    .inspect(|_| crate::upcoming::refresh_soon(state.pool.clone(), state.demo))
}

/// The body of `update_event`, minus the Tauri `State` wrapper — the same
/// split, and for the same reason, as [`create_impl`]: a test can drive
/// [`update_via_client`] against `wiremock` without this function touching
/// `load_config` or the Keychain, and the guards above it stay reachable
/// without a running app.
///
/// `send_updates` is Google's own vocabulary, carried from the caller rather
/// than chosen here — see `CalendarClient::patch_event`. The form sends
/// `"all"`, because a time typed on purpose and saved is exactly the change
/// guests need to hear about; a drag sends `"none"`, because a gesture can
/// happen by accident and a slip of the mouse must not mail a guest list
/// (drag spec §2).
///
/// The order of the four checks is the point of the function. Demo mode
/// first, before any database access at all. Then `scope`, because it is a
/// pure function of an argument and the two scopes this task implements are
/// not the only two the UI will eventually send. Then the row, then
/// writability — refused before `load_config`, the Keychain or Google ever
/// see the request.
#[allow(clippy::too_many_arguments)]
async fn update_impl(
    state: &AppState,
    id: i64,
    scope: &str,
    occurrence_start_ms: i64,
    fields: crate::write::EventFields,
    send_updates: &str,
) -> anyhow::Result<EventDetail> {
    if state.demo {
        anyhow::bail!("demo mode — there is nothing to save");
    }

    // Every scope this command implements, named exhaustively. The check is
    // not redundant with the `match` further down: `target_event_id` reads
    // every scope that is not `"all"` as "this one", so an unrecognised scope
    // arriving from a future UI would quietly edit a single occurrence of a
    // series the user asked to do something else with. Deliberately not in
    // `errors.rs`'s allowlist — a scope the shipped UI cannot send is a bug,
    // not something to explain to a user.
    if !matches!(scope, "this" | "all" | "following") {
        anyhow::bail!("that edit scope is not available yet");
    }

    // A pure check of an argument, so it sits with `scope` — before the row
    // (reminders spec §4).
    if let Some(r) = &fields.reminders {
        crate::write::validate_reminders(r).map_err(|m| anyhow::anyhow!(m))?;
    }

    // The CalDAV path, decided off the event's own calendar — see
    // `create_impl` for the shape.
    {
        let cal_id: Option<i64> =
            sqlx::query_scalar("SELECT calendar_id FROM events WHERE id = ?1")
                .bind(id)
                .fetch_optional(&state.pool)
                .await?;
        if let Some(cal_id) = cal_id {
            if crate::caldav_write::is_caldav_calendar(&state.pool, cal_id).await? {
                return crate::caldav_write::update(state, id, scope, occurrence_start_ms, fields)
                    .await;
            }
        }
    }

    let (ev, access_role, cal_google_id, account_email) =
        omacal_store::event_for_write(&state.pool, id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("that event is no longer here"))?;

    // The same rule `EventDetail::can_edit` is built from, so a calendar that
    // shows an Edit button cannot silently refuse the save it implies.
    // `demo: false` because that half is already handled above, with its own
    // message and before any database access.
    if !can_edit(false, &access_role) {
        anyhow::bail!("this calendar is not writable from omacal");
    }

    // The calendar's own zone, for the all-day half of [`edit_zone`]. Read
    // through `calendar_for_write` rather than added to `event_for_write`'s
    // tuple: that query is shared with `event_detail`, which has no use for
    // it, and this is one indexed lookup on a path that is about to make a
    // network request anyway.
    let (_, _, _, cal_tz) = omacal_store::calendar_for_write(&state.pool, ev.calendar_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("that calendar is no longer here"))?;

    let cfg = crate::load_config()?;
    let token = crate::access_token_for(state, &cfg, &account_email).await?;
    let client = omacal_google::CalendarClient::new(crate::GOOGLE_CALENDAR_API, &token);

    update_via_client(
        &state.pool,
        scope,
        occurrence_start_ms,
        ev,
        &cal_google_id,
        &cal_tz,
        fields,
        send_updates,
        &client,
    )
    .await?;

    event_detail_impl(state, id).await
}

/// The network exchange and local write-back half of [`update_impl`], with the
/// `CalendarClient` already built — handed a bare pool for the same reason
/// [`respond_via_client`] and [`create_via_client`] are.
///
/// The call sequence is `respond_via_client`'s, deliberately: series id →
/// [`target_event_id`] → for `"this"`, [`instance_lookup_window`] +
/// [`resolve_instance_id`] → patch → 412 retry → fold back only when the patch
/// landed on the row that was loaded. That machinery is not re-derived here.
///
/// What differs is the body, which is [`edit_patch_body`]'s, and the
/// provenance rule it forces: **the etag and the times must come from the
/// resource being patched**, never from the row that happened to be on screen.
/// By this point `event_id` is not always `ev.google_id` — an occurrence
/// resolves to its own instance, and `"all"` from an exception row resolves to
/// the master — and each of those is a different resource with its own version
/// and its own start. Conditioning on `ev.etag` there could only ever be
/// rejected, and anchoring times on `ev.start_utc` would move an event.
///
/// The one fetch this adds over `respond_via_client` is the same one it makes:
/// scope `"all"` from an exception row has neither the master's version nor
/// its times in hand, and that branch does no instances lookup, so it is that
/// branch's first request rather than an extra one.
#[allow(clippy::too_many_arguments)]
async fn update_via_client(
    pool: &SqlitePool,
    scope: &str,
    occurrence_start_ms: i64,
    ev: omacal_store::StoredEvent,
    cal_google_id: &str,
    cal_tz: &str,
    after: crate::write::EventFields,
    send_updates: &str,
    client: &omacal_google::CalendarClient,
) -> anyhow::Result<()> {
    // A row carrying `recurrence` is a series master; scope "this" must still
    // go through instance resolution for it, which is why `own_id` is offered
    // as the master when there is no `recurring_event_id`.
    let series = ev
        .recurring_event_id
        .as_deref()
        .or_else(|| ev.recurrence.as_ref().map(|_| ev.google_id.as_str()));

    // "This and following" is not a patch at all — it is two writes, and it
    // leaves through [`split_series`] rather than falling into the machinery
    // below. The two ways it does *not* leave here are the point of this
    // block. (A third — a save that changes nothing — is refused outright by
    // the guard between this comment and that block, before either write and
    // before the master is even read.)
    //
    // A row that belongs to no series has no "following": there is one event,
    // and editing it and everything after it is editing it. That reads as
    // `"all"`, which for a one-off is a patch of the event itself — the same
    // answer `target_event_id` documents for `(_, None)`.
    //
    // The clicked occurrence being the series' *own first* is the case that
    // has to be caught rather than split. Truncating a master to end before
    // its own DTSTART leaves a rule that expands to nothing: an event Google
    // still holds, that renders in no grid, that the user cannot see and so
    // cannot delete. There is nothing before the first occurrence to keep, so
    // "this and following" there means the whole series, and `"all"` is what
    // the user meant. The master is fetched to decide it — not read off `ev`,
    // whose `start_utc` is the *exception's* start when the row is one.
    //
    // Both of those, and the truncation itself, are aimed at the occurrence's
    // slot **on the rule's own grid** rather than at where it is rendered. For
    // a master row the two are the same instant. For an exception — an
    // occurrence somebody has already moved — they are not, and `UNTIL` is
    // compared against the grid: aimed an hour late, the rule still generates
    // the original slot and the shortened series keeps an occurrence it was
    // meant to lose; aimed early, it loses one it was meant to keep. It is
    // also what makes "the first occurrence, dragged later, is still the first
    // occurrence" come out right.
    let split_at_ms = ev.original_start_utc.unwrap_or(occurrence_start_ms);

    // Nothing changed — the same refusal as the empty-PATCH guard further
    // down, moved in front of the split because the split never reaches it.
    //
    // **A split's payload is not a diff**, which is why an empty one cannot be
    // read off the request it is about to send: [`split_series`] POSTs the
    // whole tail (times, text, rule and guest list, `sendUpdates=all`) and then
    // PATCHes the master, so a save that changed nothing still leaves two
    // Google resources where there was one and mails every guest twice — about
    // an edit nobody made, with nothing in omacal to undo it. The emptiness
    // has to be decided from the form against the row, before any of it.
    //
    // "Nothing changed" is narrower here than for a patch, and the extra term
    // is `original_start_utc`:
    //
    // * `None` — the clicked block is a plain expansion of the master, so it
    //   sits exactly on the slot the truncation is aimed at, with the master's
    //   own duration and text. The tail the POST would create is then the
    //   master's rule anchored on that same slot, with those same values: the
    //   series it already expands to. Two writes, and afterwards the calendar
    //   reads precisely as it did. That is a no-op however many resources it
    //   takes to produce, and it is the case the UI hands over almost every
    //   time.
    //
    // * `Some` — the row is a materialised exception, and an exception is by
    //   construction *not* what the rule expands to at that slot: something
    //   about it (its time, its length, its title) already differs. Splitting
    //   from it carries that difference onto the whole tail — a series that
    //   repeats from where one occurrence was moved to, rather than one moved
    //   occurrence — so the calendar afterwards is genuinely different even
    //   with every form field untouched. Refusing there would swallow an edit
    //   the UI gives the user no other way to express, so this never fires for
    //   an exception. The carve-out is held in place by its own test:
    //   `a_following_save_from_a_moved_occurrence_still_splits_with_the_form_untouched`.
    //
    // The diff itself is [`edit_patch_body`]'s rather than a comparison
    // written out again here: for this row it is empty exactly when the user
    // left every field alone, and it is where the civil-movement rules
    // (`shifted_like`, and the anchor for a recurring row) are already
    // reasoned through. Only its *emptiness* is used — the body it builds is a
    // PATCH of the master, and is not what a split would send.
    //
    // Placed before the master GET rather than after it, so a no-op costs no
    // request at all: it also short-circuits the two arms below that would
    // have fetched the master only to discover the same emptiness at the PATCH
    // guard. The refusals inside `split_series` (`COUNT`, stranded exceptions)
    // are skipped along with them, which is right — there is nothing to tell
    // the user about a split that was never going to change anything.
    if scope == "following"
        && ev.original_start_utc.is_none()
        && edit_patch_body(&ev, ev.start_utc, ev.end_utc, occurrence_start_ms, cal_tz, &after)
            == serde_json::json!({})
    {
        return Ok(());
    }

    // The master this block fetched, when it decided not to split after all.
    // It is the very resource the `"all"` path below is about to ask for, so
    // it is handed on rather than fetched twice.
    let mut prefetched = None;
    let scope = if scope == "following" {
        match series {
            None => "all",
            Some(master_id) => {
                let master = client.get_event(cal_google_id, master_id).await?;
                let master_row = row_from_wire(&master, ev.calendar_id, cal_tz)?;
                if split_at_ms > master_row.start_utc {
                    return split_series(
                        pool,
                        split_at_ms,
                        &ev,
                        &master,
                        &master_row,
                        cal_google_id,
                        cal_tz,
                        &after,
                        send_updates,
                        client,
                    )
                    .await;
                }
                prefetched = Some(master);
                "all"
            }
        }
    } else {
        scope
    };

    let target = target_event_id(scope, series, &ev.google_id);

    // The resolved instance is kept, not just its id: `event_instances` asks
    // for no `fields` mask, so each item is a full event — `etag` and its own
    // times included — and both are needed below. Re-fetching it would spend a
    // request on data already in hand. `prefetched` is the same idea one step
    // earlier: `Some` only on the `"following"`-became-`"all"` path, where it
    // is by construction the master `target_event_id` has just named.
    let (event_id, instance) = match &target {
        Target::Master(master_id) => (master_id.clone(), prefetched),
        Target::Instance { master, fallback } => {
            let (time_min, time_max) = instance_lookup_window(occurrence_start_ms);
            let found = client.event_instances(cal_google_id, master, &time_min, &time_max).await?;
            let id = resolve_instance_id(&found, master, fallback)?;
            let inst = found.iter().find(|i| i.id == id).cloned();
            (id, inst)
        }
    };

    let (target_start, target_end, if_match) = if event_id == ev.google_id {
        (ev.start_utc, ev.end_utc, ev.etag.clone())
    } else {
        let target_event = match instance {
            Some(inst) => inst,
            None => client.get_event(cal_google_id, &event_id).await?,
        };
        let row = row_from_wire(&target_event, ev.calendar_id, cal_tz)?;
        (row.start_utc, row.end_utc, row.etag)
    };

    let body =
        edit_patch_body(&ev, target_start, target_end, occurrence_start_ms, cal_tz, &after);
    // Nothing changed. A PATCH with an empty body is not harmless: on the
    // form's path it carries `sendUpdates=all`, so it would mail the guest
    // list about an edit nobody made. A drag sends `"none"` and reaches here
    // only if a drop landed back on its own slot — which the grid already
    // declines to send at all, so this is the second of two guards rather
    // than the only one.
    if body == serde_json::json!({}) {
        return Ok(());
    }

    let patched = match client
        .patch_event(cal_google_id, &event_id, &body, send_updates, if_match.as_deref())
        .await
    {
        Ok(p) => p,
        Err(omacal_google::ApiError::PreconditionFailed) => {
            // **A guest-list change is not retried. It is reported.**
            //
            // Every other field in this body is a diff — only what the user
            // touched — so rebuilding it against a freshly-read event leaves
            // somebody else's concurrent change intact. `attendees` is not a
            // diff. It is a whole-list replace built from `ev.attendees`, the
            // list as omacal last read it, and a 412 is Google saying that
            // reading is out of date. Retrying would send that stale list over
            // the current one: anyone invited elsewhere since the form opened
            // is silently un-invited, and with `sendUpdates=all` they are
            // mailed a cancellation for a meeting nobody meant to remove them
            // from.
            //
            // Re-reading and merging is not an option either, because a target
            // list cannot say what the user *did*. If Dan is on the fresh copy
            // and not on the form's list, "the user removed Dan" and "the user
            // never saw Dan" are the same list, and the two want opposite
            // writes. The form is the only thing that knows, so the conflict
            // goes back to it. Guest-list spec §2 is explicit that `If-Match`
            // matters more here than anywhere, and this is what it is for.
            if body.get("attendees").is_some() {
                anyhow::bail!(CONFLICT_GUESTS);
            }

            // Somebody changed the event while the form was open. Re-read for
            // the current version, rebuild against where the event now *is*
            // (a time shift the user made applies to its new position), and
            // try once more. The user's own diff is unchanged: it is still
            // only the fields they touched, so the other edit survives.
            let fresh = client.get_event(cal_google_id, &event_id).await?;
            let row = row_from_wire(&fresh, ev.calendar_id, cal_tz)?;
            let retry = edit_patch_body(
                &ev,
                row.start_utc,
                row.end_utc,
                occurrence_start_ms,
                cal_tz,
                &after,
            );
            client
                .patch_event(cal_google_id, &event_id, &retry, send_updates, row.etag.as_deref())
                .await?
        }
        Err(e) => return Err(e.into()),
    };

    // Close the loop locally — but only when the patch actually targeted the
    // row that was loaded, exactly as `respond_via_client` does: `upsert_event`
    // is keyed on `(calendar_id, google_id)`, so folding another resource's
    // state in would write straight onto this row and corrupt it. The
    // occurrence Google has just materialised is left for the next sync.
    //
    // Folded in through `to_stored` rather than [`merge_patched`]: an edit
    // changes precisely the fields `merge_patched` does not carry, so the
    // popover would go on showing the old title until the next sync. That
    // mapping is a superset of it — etag, sequence, attendees and a re-derived
    // `self_response` included — and is the same one every synced row is built
    // by, so a row edited here stays shaped like one that arrived normally.
    // `merge_patched` is still the fallback for a response `to_stored` cannot
    // read: Google has accepted the write by then, and reporting a failure
    // over an unreadable *response* would be a lie about what happened.
    if event_id == ev.google_id {
        let row = match omacal_sync::to_stored(&patched, ev.calendar_id, cal_tz) {
            Some(row) => row,
            None => {
                let mut row = ev;
                merge_patched(&mut row, &patched);
                row
            }
        };
        omacal_store::upsert_event(pool, &row).await?;
    }

    Ok(())
}

/// "This and following": the same meeting, continuing — as two series where
/// there was one.
///
/// # The order is the whole safety argument
///
/// **Create the tail first, shorten the original second.** Google has no
/// transaction across two events, so one of the two writes can land without
/// the other, and the two orders fail in opposite directions:
///
/// * create-then-truncate, failing at the truncate, leaves an overlapping
///   *duplicate*. Every occurrence still exists; two of them sit on top of
///   each other in the grid, and the user can delete one. Nothing is lost, and
///   the error below says exactly that.
/// * truncate-then-create, failing at the create, leaves **nothing at all**
///   after the split point. The tail of the series is gone, silently and
///   unrecoverably — the rule that generated it has already been rewritten, so
///   there is no record of what those occurrences were.
///
/// Every error past the `insert` therefore says the same thing: the duplicate
/// exists and needs a human. Reporting a bare transport failure there would
/// leave the user with two series and no idea why.
///
/// # The new series carries the master's guest list, verbatim
///
/// A split is one meeting continuing, so the second half has the same people
/// on it. Omitting `attendees` from the `insert` does not merely leave the
/// list blank: it *removes every guest from the second half of their own
/// series*, which is worse than any scheduling error this function could make
/// and is invisible to the person who did it. They go through
/// [`attendees_verbatim`], which is the same field-for-field mapping every
/// other write uses — see [`attendee_json`] for why a second copy of that
/// list is not an option.
///
/// That is also why the `insert` notifies (`sendUpdates=all`) where the
/// new-event form does not. The truncation half already mails every guest that
/// the series changed; creating the replacement silently would tell them half
/// the story, and leave external guests with a meeting nobody invited them to.
///
/// # What the two bodies are built from
///
/// The new series takes its times from `after` **absolutely**, with no shift.
/// It is anchored *at* the clicked occurrence, so the form's instants are
/// already in the resource's own coordinates —
/// [`crate::write::shifted_like`]'s `target == from` short circuit would
/// return them unchanged, and going through it would only obscure that. This
/// is the one write in this file where the form's numbers are used as they
/// arrived; [`edit_patch_body`]'s doc comment covers why every other one
/// cannot.
///
/// Its recurrence follows the same three-state as everywhere else: Repeat
/// untouched carries the series' own lines across, Never makes the remainder a
/// single event, and a chosen rule replaces the `RRULE` while keeping the rest.
/// `EXDATE`/`RDATE` lines are carried in every case — an `EXDATE` in the tail
/// names an occurrence somebody deleted, and dropping it would resurrect a
/// cancelled meeting.
///
/// The truncation is `{"recurrence": [...]}` and nothing else. The user's edits
/// belong to the tail; the original keeps the past exactly as it was.
///
/// It is aimed at `split_at_ms`, which is **not** the instant the tail starts
/// at and need not be the instant the user clicked either. `UNTIL` is compared
/// against the instants the *rule* generates, so the truncation has to name the
/// occurrence's slot on that grid — its `originalStartTime` when the clicked
/// occurrence is one somebody had already dragged elsewhere. The tail's own
/// start, by contrast, is wherever the user put it. The caller separates the
/// two; see [`update_via_client`].
///
/// # Two shapes this refuses rather than guesses at
///
/// A rule ending in `COUNT` cannot be split by rewriting text. The original
/// converts to an `UNTIL` (see [`crate::write::truncated_rule`]), but the tail
/// would need `COUNT` minus however many occurrences the first half consumed —
/// a number only a full expansion of the rule knows, and one that an
/// off-by-one in either direction turns into a meeting the user never
/// scheduled or one that quietly disappears. Carrying `COUNT` across unchanged
/// is the same bug with a bigger number. It stops and says so.
///
/// The `412` from a concurrent edit is not retried, unlike every other write
/// here. The master was read moments earlier, so a `412` means somebody
/// changed the series *during* the split; a retry would have to re-derive the
/// truncation from a rule this function has not checked — one that may now end
/// in `COUNT` — and the tail already exists by then. The duplicate is the safe
/// place to stop.
#[allow(clippy::too_many_arguments)]
async fn split_series(
    pool: &SqlitePool,
    split_at_ms: i64,
    ev: &omacal_store::StoredEvent,
    master: &omacal_google::model::Event,
    master_row: &omacal_store::StoredEvent,
    cal_google_id: &str,
    cal_tz: &str,
    after: &crate::write::EventFields,
    send_updates: &str,
    client: &omacal_google::CalendarClient,
) -> anyhow::Result<()> {
    let lines: Vec<String> = master.recurrence.clone().unwrap_or_default();
    let rules: Vec<&String> = lines.iter().filter(|l| crate::write::is_rrule(l)).collect();
    // Somebody removed the rule between the popover opening and the save.
    // Deliberately not allowlisted in `errors.rs`: it is a state nobody has
    // seen, and this stops rather than inventing a series to split.
    if rules.is_empty() {
        anyhow::bail!("that event no longer repeats, so there is nothing to split");
    }
    // Every rule line, not just the first: two `RRULE`s in one `recurrence` is
    // not valid iCalendar and Google does not emit it, but checking one and
    // truncating both is the shape a real bug hides in.
    if rules.iter().any(|r| crate::write::has_count(r)) {
        anyhow::bail!(
            "omacal cannot split a series that ends after a set number of times — \
             edit all events instead"
        );
    }

    // Occurrences in the tail that somebody has already changed on their own.
    //
    // They are separate Google events pointing back at *this* master, and
    // truncating it takes them with it — Google drops the instances past
    // `UNTIL`, materialised ones included. The new series is generated fresh
    // from the rule and knows nothing about them, so a move is silently undone
    // and a deletion silently reversed: the cancelled occurrence comes back.
    //
    // That is the same failure the create-before-truncate ordering exists to
    // prevent, and the same reason it cannot be shipped quietly — the user
    // cannot see what went, so they cannot put it back. Carrying them across
    // means re-creating each against a master that did not exist a moment ago,
    // which is a larger piece of work than this; until then it refuses, before
    // either write, and names the number so the user knows the scale of what
    // they are being asked to redo.
    //
    // The clicked occurrence is excluded, and that is not a rounding-off: when
    // the row being edited *is* an exception — dragging one occurrence and then
    // splitting from it is an ordinary thing to do — it is not stranded by the
    // split. It becomes the first occurrence of the new series, carrying the
    // form's values. Counting it would refuse precisely the case this handles
    // best.
    //
    // A lower bound, and the message is worded to be true as one: this counts
    // what the store has synced, so an exception created seconds ago in a
    // window this app has not fetched is not in it. Better than proceeding
    // regardless, and honest about which direction it can be wrong in.
    let stranded = omacal_store::exceptions_from(
        pool,
        ev.calendar_id,
        &master.id,
        split_at_ms,
        &ev.google_id,
    )
    .await?;
    if stranded > 0 {
        anyhow::bail!(
            "some later occurrences of this series were moved or deleted on their own, and a \
             split cannot carry them across — edit all events instead, or re-create them \
             afterwards. Occurrences affected: {stranded}"
        );
    }

    // ---- 1. The tail. -----------------------------------------------------
    let zone = edit_zone(after.when.is_all_day(), cal_tz, &ev.start_tz);
    let (start, end) = crate::write::when_json(&after.when, zone);
    let mut body = serde_json::json!({ "start": start, "end": end });
    if let Some(s) = &after.summary     { body["summary"]     = s.clone().into(); }
    if let Some(s) = &after.location    { body["location"]    = s.clone().into(); }
    if let Some(s) = &after.description { body["description"] = s.clone().into(); }

    match &after.recurrence {
        // Repeat untouched: the tail repeats exactly as the series did.
        None => body["recurrence"] = serde_json::json!(lines),
        // Repeat set to Never: what is left of the series is one event.
        Some(None) => {}
        // A rule the user chose replaces the `RRULE` line only. The rest
        // travel with it — see [`crate::write::is_rrule`] for why an `EXDATE`
        // dropped here brings a cancelled occurrence back to life.
        Some(Some(chosen)) => {
            let mut kept = vec![chosen.clone()];
            kept.extend(lines.iter().filter(|l| !crate::write::is_rrule(l)).cloned());
            body["recurrence"] = serde_json::json!(kept);
        }
    }

    // **The tail carries the guest list the user asked for.**
    //
    // "This and following" means *from here on, it is like this*, so a guest
    // change made on this save belongs to the tail — the half the user is
    // authoring — exactly as their new title and their new time do. Copying
    // the series' own list across regardless would drop the change silently:
    // the truncation below carries `recurrence` and nothing else, so there is
    // no second write for it to land in, and the user would see the guest they
    // removed still invited to every occurrence from here on.
    //
    // The master keeps its own list untouched, which is the same rule the rest
    // of this function follows: the user's edits belong to the tail, and
    // shortening a series must not rewrite who was invited to the part that
    // already happened.
    //
    // **Reconciled against `master_row`**, which is the row this edit was based
    // on and the one whose attendees carry the answers worth preserving —
    // `attendees_for_edit` is the same echo-back the patch path uses, so
    // everyone carried across keeps their `responseStatus`, `displayName`,
    // `comment` and `additionalGuests` rather than being reduced to an address.
    //
    // One approximation, deliberately, and it is worth knowing about. When the
    // clicked occurrence is a *materialised exception* its attendee list can
    // differ from the master's. Somebody the master has and the exception does
    // not is dropped, which is right: the user was looking at a list without
    // that person and did not add them. But somebody carried across whose
    // **answer** differs between the two — accepted on the exception, still
    // `needsAction` on the master — goes out with the master's. That is a
    // cosmetic divergence in one field, corrected by the next sync of the tail,
    // and not a lost invitation or a cancellation. The alternative is to read
    // the exception's own list as well, which costs a request on every split to
    // fix a field Google is about to restate anyway.
    let attendees = match &after.guests {
        Some(wanted) => attendees_for_edit(&master_row.attendees, wanted),
        None => attendees_verbatim(&master_row.attendees),
    };
    // An empty array is omitted rather than sent, which is also the right
    // answer for a user who removed every guest: an insert with no `attendees`
    // key creates an event with no attendees, which is what they asked for.
    if !attendees.is_empty() {
        body["attendees"] = serde_json::json!(attendees);
    }

    // **The caller's choice, not a constant.** Both writes below carried
    // `"all"` while a save was the only way to change an event and always
    // notified. Guest-list spec §3 makes notifying a choice, and a split that
    // mailed regardless would make that choice a lie on one scope in three:
    // the user would press "Save without notifying" and every guest would be
    // told twice — once by the tail's creation, once by the truncation.
    let created = client.insert_event(cal_google_id, &body, send_updates).await?;

    // ---- 2. The original, shortened. --------------------------------------
    // Past this point the tail exists on Google, so every failure below is the
    // duplicate, whatever its cause.
    //
    // Every argument here describes the **master**, and `master_row.is_all_day`
    // is the one that has a plausible-looking wrong answer sitting next to it.
    // RFC 5545 requires `UNTIL` to carry the same value type as the `DTSTART`
    // of the rule it belongs to, and this body is `recurrence` alone — the
    // master keeps whatever `start` it already had. The tail's shape
    // (`after.when`, twenty lines up) is a different event's, and the two
    // genuinely diverge: splitting an all-day series into a timed remainder is
    // an ordinary thing for a user to do, and taking the tail's shape there
    // writes a date-time `UNTIL` onto a series whose `DTSTART` is a bare date.
    //
    // The zone is inert and is passed for shape only: `edit_zone`'s all-day arm
    // returns `cal_tz` whatever it is handed, and `until_value`'s timed form
    // never reads a zone at all. Only the flag is load-bearing.
    let shortened: Vec<String> = lines
        .iter()
        .map(|l| {
            if crate::write::is_rrule(l) {
                crate::write::truncated_rule(
                    l,
                    split_at_ms,
                    master_row.is_all_day,
                    edit_zone(master_row.is_all_day, cal_tz, &master_row.start_tz),
                )
            } else {
                l.clone()
            }
        })
        .collect();

    let patched = client
        .patch_event(
            cal_google_id,
            &master.id,
            &serde_json::json!({ "recurrence": shortened }),
            send_updates,
            master.etag.as_deref(),
        )
        .await
        .map_err(|e| {
            tracing::error!(master = %master.id, created = %created.id, %e,
                "series split created the new series but could not shorten the original");
            anyhow::anyhow!(
                "the new series was created but the original could not be shortened — \
                 you now have two overlapping series and should delete one"
            )
        })?;

    // ---- 3. Only now, the local store. ------------------------------------
    // Deliberately after both writes rather than between them. A local row is
    // a cache the next sync repairs, so writing one buys nothing on the
    // failure path — while writing it *between* the two would put a database
    // error in front of the message above, leaving the user with two series
    // and an error that never mentions them.
    //
    // A created event Google describes in terms this app cannot store is left
    // for the next sync rather than reported: both writes have landed by then,
    // and failing here would claim the split did not happen when it did.
    match omacal_sync::to_stored(&created, ev.calendar_id, cal_tz) {
        Some(row) => {
            omacal_store::upsert_event(pool, &row).await?;
        }
        None => tracing::warn!(created = %created.id,
            "the new series was created but could not be stored locally; sync will pick it up"),
    }

    // Folded in only when the resource that was patched *is* the row that was
    // loaded — the same conservatism as [`update_via_client`] and
    // `respond_via_client`. It is conservatism rather than a safety property
    // here: `to_stored` names its own `google_id`, and `upsert_event` is keyed
    // on `(calendar_id, google_id)`, so a master folded back from an exception
    // row would land on the master's row rather than over the exception's.
    // Leaving it to sync keeps all three functions reading the same way.
    //
    // **No `merge_patched` fallback, unlike those two.** They fall back to it
    // because it carries what *their* write changed — etag, sequence,
    // attendees. This write changes `recurrence`, and `merge_patched` does not
    // carry `recurrence` at all: using it here would stamp the new version onto
    // a row still holding the *untruncated* rule, so the grid would go on
    // expanding the series past the split while the store claimed to be up to
    // date, and the next edit would condition on an etag whose rule it does not
    // have. An unreadable response is left for sync instead — both writes have
    // landed by then, so reporting a failure would be a lie about what
    // happened, exactly as for the create above.
    if master.id == ev.google_id {
        match omacal_sync::to_stored(&patched, ev.calendar_id, cal_tz) {
            Some(row) => {
                omacal_store::upsert_event(pool, &row).await?;
            }
            None => tracing::warn!(master = %master.id,
                "the series was split but the shortened original could not be stored locally; \
                 sync will pick it up"),
        }
    }

    Ok(())
}

/// `occurrence_start_ms` is the `start_ms` of the block the user actually
/// clicked — see [`instance_lookup_window`] for why this cannot be derived from
/// the stored row instead. `scope` is `"this"`, `"all"` or `"following"`.
///
/// Named `delete_event_cmd` and not `delete_event` because this file already
/// calls [`omacal_store::delete_event`], and two functions of that name in one
/// module — one removing a database row, one removing everybody's meeting and
/// mailing them about it — is a mistake waiting to be made by autocomplete.
///
/// Returns nothing. The other three write commands answer with the freshly
/// written [`EventDetail`], which is what the popover re-renders from; here the
/// event the popover was showing is gone, and reading it back would fail on
/// exactly the runs that succeeded.
#[tauri::command]
pub async fn delete_event_cmd(
    state: tauri::State<'_, AppState>,
    id: i64,
    scope: String,
    occurrence_start_ms: i64,
) -> Result<(), String> {
    // Captured before the delete — afterwards there may be no row to ask.
    // A departure the user chose must not come back through the change
    // ledger as "cancelled" news, so a successful delete erases the
    // meeting's ledger memory (series root, exceptions included).
    let target: Option<(i64, String, Option<String>)> = sqlx::query_as(
        "SELECT calendar_id, google_id, recurring_event_id FROM events WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten();

    delete_impl(&state, id, &scope, occurrence_start_ms)
        .await
        .map_err(|e| crate::errors::user_facing(&e))
        .inspect(|_| {
            crate::upcoming::refresh_soon(state.pool.clone(), state.demo);
            if let Some((calendar_id, gid, series)) = target {
                let pool = state.pool.clone();
                tauri::async_runtime::spawn(async move {
                    let root = series.unwrap_or(gid);
                    if let Err(e) = omacal_store::forget_changes(&pool, calendar_id, &root).await {
                        tracing::warn!(%e, "could not clear the change ledger after a delete");
                    }
                });
            }
        })
}

/// The body of `delete_event_cmd`, minus the Tauri `State` wrapper — the same
/// split, and for the same reason, as [`update_impl`]: a test can drive
/// [`delete_via_client`] against `wiremock` without this function touching
/// `load_config` or the Keychain, and the guards above it stay reachable
/// without a running app.
///
/// The order of the four checks is [`update_impl`]'s exactly, and matters more
/// here than anywhere else in this file: every request below goes out with
/// `sendUpdates=all` and the resource it names stops existing. Demo mode first,
/// before any database access at all. Then `scope`, because it is a pure
/// function of an argument and [`target_event_id`] reads *every* scope that is
/// not `"all"` as "this one" — an unrecognised scope arriving from a future UI
/// would delete one occurrence of a series the user asked to do something else
/// with. Then the row, then writability, so a reader calendar is refused before
/// `load_config`, the Keychain or Google ever see the request.
async fn delete_impl(
    state: &AppState,
    id: i64,
    scope: &str,
    occurrence_start_ms: i64,
) -> anyhow::Result<()> {
    if state.demo {
        anyhow::bail!("demo mode — there is nothing to delete");
    }

    // Deliberately not in `errors.rs`'s allowlist, exactly as `update_impl`'s
    // is not: a scope the shipped UI cannot send is a bug, not something to
    // explain to a user.
    if !matches!(scope, "this" | "all" | "following") {
        anyhow::bail!("that delete scope is not available yet");
    }

    // The CalDAV path — see `create_impl` for the shape.
    {
        let cal_id: Option<i64> =
            sqlx::query_scalar("SELECT calendar_id FROM events WHERE id = ?1")
                .bind(id)
                .fetch_optional(&state.pool)
                .await?;
        if let Some(cal_id) = cal_id {
            if crate::caldav_write::is_caldav_calendar(&state.pool, cal_id).await? {
                return crate::caldav_write::delete(state, id, scope, occurrence_start_ms).await;
            }
        }
    }

    let (ev, access_role, cal_google_id, account_email) =
        omacal_store::event_for_write(&state.pool, id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("that event is no longer here"))?;

    // The same rule `EventDetail::can_edit` is built from, so a calendar that
    // shows a Delete control cannot silently refuse the delete it implies.
    // `demo: false` because that half is already handled above, with its own
    // message and before any database access.
    if !can_edit(false, &access_role) {
        anyhow::bail!("this calendar is not writable from omacal");
    }

    // The calendar's own zone, for the all-day half of [`edit_zone`] and for
    // `row_from_wire`. Only the `"following"` arm reads it, but it is read here
    // rather than inside so that a missing calendar is reported by the same
    // message on every scope.
    let (_, _, _, cal_tz) = omacal_store::calendar_for_write(&state.pool, ev.calendar_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("that calendar is no longer here"))?;

    let cfg = crate::load_config()?;
    let token = crate::access_token_for(state, &cfg, &account_email).await?;
    let client = omacal_google::CalendarClient::new(crate::GOOGLE_CALENDAR_API, &token);

    delete_via_client(
        &state.pool,
        scope,
        occurrence_start_ms,
        ev,
        &cal_google_id,
        &cal_tz,
        &client,
    )
    .await
}

/// The network exchange and local write-back half of [`delete_impl`], with the
/// `CalendarClient` already built — handed a bare pool for the same reason
/// [`respond_via_client`], [`create_via_client`] and [`update_via_client`] are.
///
/// The resolution machinery is [`update_via_client`]'s and is not re-derived:
/// series id → [`target_event_id`] → for `"this"`, [`instance_lookup_window`] +
/// [`resolve_instance_id`]. Two things about it are worth restating here,
/// because a delete is where getting them wrong is unrecoverable.
///
/// **`"this"` on a series master must resolve to the instance.** Deleting
/// `ev.google_id` there is deleting the *series*: every occurrence, past ones
/// included, with a cancellation mailed to the whole guest list. That is not a
/// worse version of what the user asked for, it is a different operation.
///
/// **An empty lookup on a bare master fails rather than falling back**, which
/// is [`resolve_instance_id`]'s own rule, and the reason it exists. `404` is
/// success to [`omacal_google::CalendarClient::delete_event`] — an event that
/// is already gone is the caller's desired end state — so a delete aimed at the
/// wrong id has no failure of its own to report.
///
/// # `"following"` is a truncation and never a delete
///
/// Deleting the master would take the *past* occurrences with it, which is the
/// one thing the user chose this scope in order not to do. It leaves through
/// [`truncate_series`] instead, and the two ways it does not leave here are
/// [`update_via_client`]'s: a row belonging to no series has no "following", and
/// the series' own first occurrence means the whole series. Both read as
/// `"all"`, which for a delete is what the user meant either way — an empty rule
/// would otherwise leave a master that expands to nothing, which renders in no
/// grid and so cannot be deleted from one.
///
/// # No `If-Match` on the DELETE, and that is a decision rather than an omission
///
/// Every other write in this file conditions on an etag, because every other
/// write *replaces* something: a stale version means somebody else's change is
/// about to be overwritten, and the retry re-reads and re-applies. A delete
/// replaces nothing. The resource ends up gone, which is what was asked, and it
/// is asked of an id — an id a concurrent rename or reschedule does not change.
///
/// A `412` here would also have no answer. Retrying unconditionally is "delete
/// anyway", which is what sending no `If-Match` already does in one request
/// instead of two; failing tells the user their delete was refused because
/// somebody edited the title, which they can do nothing about but try again.
/// And the freshness that actually matters on the dangerous scope is not an
/// etag at all: `"this"` resolves through a *live* `events.instances` lookup, so
/// an occurrence somebody moved out from under the click is not found and the
/// delete is refused by [`resolve_instance_id`] rather than landing on the wrong
/// day.
///
/// The truncation in [`truncate_series`] does send one, for the opposite reason:
/// it is a `PATCH` that replaces a rule.
///
/// # What is repaired locally, and what is left to sync
///
/// The rule is that this repairs only what it can name with certainty — the
/// resources it has itself just deleted — and leaves everything that would need
/// an inference to the next sync, which is minutes away.
///
/// * `"all"`: the master's row *and* every exception of it
///   ([`omacal_store::delete_series`]). Google deletes the series entire; a
///   local exception left behind carries no rule of its own and renders as a
///   meeting nobody can find any more.
/// * `"this"` on a one-off: the row goes. There was one event and it is gone.
/// * `"this"` on an exception row: the row is marked **cancelled**, not
///   removed. A cancelled exception is the only record that a slot of the series
///   is empty (`commands::suppressed_slots`), so deleting the row instead would
///   let the master expand straight back into it and the occurrence the user
///   just deleted would reappear. It is also exactly what sync writes for this
///   event — see `omacal_sync::to_cancelled_exception`.
/// * `"this"` resolving to an instance this store has no row for: nothing. The
///   master row stays, because only Google knows that occurrence is gone, and
///   the cancelled exception it has just materialised is left for sync exactly
///   as `respond_via_client` leaves the one *it* causes. Until then the grid
///   still draws the occurrence, so the UI wants a sync after this returns.
async fn delete_via_client(
    pool: &SqlitePool,
    scope: &str,
    occurrence_start_ms: i64,
    ev: omacal_store::StoredEvent,
    cal_google_id: &str,
    cal_tz: &str,
    client: &omacal_google::CalendarClient,
) -> anyhow::Result<()> {
    // A row carrying `recurrence` is a series master; scope "this" must still
    // go through instance resolution for it, which is why `own_id` is offered
    // as the master when there is no `recurring_event_id`.
    let series = ev
        .recurring_event_id
        .as_deref()
        .or_else(|| ev.recurrence.as_ref().map(|_| ev.google_id.as_str()));

    // The clicked occurrence's slot **on the rule's own grid**, which is what a
    // truncation has to be aimed at: `UNTIL` is compared against the instants
    // the rule generates, not against where a block is drawn. For a master row
    // the two are the same instant; for an exception — an occurrence somebody
    // has already dragged — they are not, and aiming at the rendered position
    // either keeps an occurrence the user meant to delete or deletes one they
    // meant to keep. It is also what makes "the first occurrence, dragged
    // later, is still the first occurrence" come out right.
    let split_at_ms = ev.original_start_utc.unwrap_or(occurrence_start_ms);

    let scope = if scope == "following" {
        match series {
            None => "all",
            Some(master_id) => {
                let master = client.get_event(cal_google_id, master_id).await?;
                let master_row = row_from_wire(&master, ev.calendar_id, cal_tz)?;
                if split_at_ms > master_row.start_utc {
                    return truncate_series(
                        pool,
                        split_at_ms,
                        &master,
                        &master_row,
                        cal_google_id,
                        cal_tz,
                        client,
                    )
                    .await;
                }
                // Deliberately not carried forward the way `update_via_client`
                // carries its `prefetched` master: the `"all"` arm below is a
                // DELETE, which needs no version and no body, so there is
                // nothing left for this response to save.
                "all"
            }
        }
    } else {
        scope
    };

    let target = target_event_id(scope, series, &ev.google_id);
    let event_id = match &target {
        Target::Master(master_id) => master_id.clone(),
        Target::Instance { master, fallback } => {
            let (time_min, time_max) = instance_lookup_window(occurrence_start_ms);
            let found = client.event_instances(cal_google_id, master, &time_min, &time_max).await?;
            resolve_instance_id(&found, master, fallback)?
        }
    };

    client.delete_event(cal_google_id, &event_id, None).await?;

    // See the doc comment for why each arm does what it does. The one that
    // looks wrong and is not is `status = "cancelled"`: a deleted occurrence of
    // a series is *stored*, not removed, because that row is the only record
    // that the slot is empty.
    if scope == "all" {
        omacal_store::delete_series(pool, ev.calendar_id, &event_id).await?;
    } else if event_id == ev.google_id {
        if ev.recurring_event_id.is_some() {
            let mut row = ev;
            row.status = "cancelled".into();
            omacal_store::upsert_event(pool, &row).await?;
        } else if ev.recurrence.is_none() {
            omacal_store::delete_event(pool, ev.calendar_id, &ev.google_id).await?;
        }
        // A bare series master cannot reach here — `resolve_instance_id` bails
        // rather than answering with `ev.google_id` — and if it ever did, its
        // row is the whole series and must not be removed for one occurrence.
    }

    Ok(())
}

/// "Delete this and following": the series stops just before the clicked
/// occurrence, and nothing else about it changes.
///
/// One `PATCH` of `{"recurrence": [...]}`, and no `DELETE` anywhere — the
/// distinction the whole scope turns on. The master is the event the *past*
/// occurrences live in, so deleting it deletes them too, silently and with a
/// cancellation mailed to everybody; truncating its rule ends the series where
/// the user pointed and leaves everything before that untouched.
///
/// **It is not [`split_series`] minus the insert.** Two of that function's rules
/// invert here, and reusing it would have carried both across:
///
/// * A rule ending in `COUNT` is **fine to truncate and is not refused.** The
///   split refuses it because the *tail* would need `COUNT` less however many
///   occurrences the first half consumed, which only a full expansion knows.
///   There is no tail. [`crate::write::truncated_rule`] drops `COUNT` and adds
///   `UNTIL`, which is the complete and correct answer for a series that now
///   ends on a date.
/// * Materialised exceptions in the tail are **not refused either**, and that
///   is argued rather than overlooked. The split refuses them because it
///   re-creates the tail as a new series that cannot carry them, so a move
///   would be silently undone and a deletion silently reversed — information
///   destroyed with nothing put in its place. Here the user has asked for those
///   occurrences to go. Refusing would leave them only "delete all events",
///   which also takes the past occurrences the scope exists to protect: a
///   refusal that pushes the user onto the more destructive button is worse than
///   the imprecision it prevents.
///
///   **The exact cost of not refusing, which is not zero.** `UNTIL` is compared
///   against `originalStartTime`, not against where a block is drawn, so the two
///   disagree for any occurrence somebody has dragged *across* the cut. One
///   dragged **backwards** renders before the cut — inside the half the user is
///   keeping — while its slot is after `UNTIL`, so Google drops it: a meeting
///   they meant to keep disappears, silently, and they cannot see what went. One
///   dragged forwards is the mirror and survives a delete it was inside. A
///   refusal cannot fix either; it can only decline the whole operation, which
///   is why the common case is not made to pay for the rare one. It is a real
///   limitation and belongs in front of anybody changing this.
///
/// `master_row` is the only place the value type comes from, and the clicked row
/// is deliberately **not** a parameter of this function so that it cannot be
/// read from by accident. RFC 5545 requires `UNTIL` to carry the same value type
/// as the `DTSTART` of the rule it belongs to, and this body is `recurrence`
/// alone — the master keeps whatever `start` it already had, so the rule has to
/// agree with *that* and with nothing else. The zone is inert and passed for
/// shape: [`edit_zone`]'s all-day arm returns `cal_tz` whatever it is handed and
/// the timed form of `UNTIL` never reads a zone. Only the flag is load-bearing.
///
/// **That absence is the whole of the protection, and no test can stand in for
/// it.** Every fixture here has the master and the clicked row agreeing on
/// `is_all_day`, so nothing in the suite distinguishes `master_row.is_all_day`
/// from `ev.is_all_day` — the two are the same value in all of them. Harmless
/// while the clicked row is not reachable from this function; a silent hole the
/// moment somebody adds it as a parameter. If you add one, add the fixture that
/// tells them apart first: an all-day master with a timed exception, on the
/// model of `the_until_follows_the_masters_value_type_not_the_new_series`.
///
/// The `412` is not retried, for [`split_series`]' reason minus its
/// complication: the master was read moments ago, so a `412` means somebody
/// changed the series *during* this, and a retry would re-derive a truncation
/// from a rule this function has not looked at. Nothing has been written by
/// then, so unlike a split there is no leftover state — the operation simply did
/// not happen, and repeating it after a sync is safe.
async fn truncate_series(
    pool: &SqlitePool,
    split_at_ms: i64,
    master: &omacal_google::model::Event,
    master_row: &omacal_store::StoredEvent,
    cal_google_id: &str,
    cal_tz: &str,
    client: &omacal_google::CalendarClient,
) -> anyhow::Result<()> {
    let lines: Vec<String> = master.recurrence.clone().unwrap_or_default();
    // Somebody removed the rule between the popover opening and the delete.
    // Deliberately not allowlisted in `errors.rs`: it is a state nobody has
    // seen, and this stops rather than guessing that "all events" was meant —
    // which on this verb would delete the very occurrences the user chose this
    // scope to keep.
    if !lines.iter().any(|l| crate::write::is_rrule(l)) {
        anyhow::bail!("that event no longer repeats, so there is nothing to delete from here");
    }

    let shortened: Vec<String> = lines
        .iter()
        .map(|l| {
            if crate::write::is_rrule(l) {
                crate::write::truncated_rule(
                    l,
                    split_at_ms,
                    master_row.is_all_day,
                    edit_zone(master_row.is_all_day, cal_tz, &master_row.start_tz),
                )
            } else {
                // `EXDATE`/`RDATE` travel untouched. An `EXDATE` before the cut
                // names an occurrence somebody deleted, and dropping it here
                // would bring a cancelled meeting back into the half the user
                // is keeping.
                l.clone()
            }
        })
        .collect();

    let patched = client
        .patch_event(
            cal_google_id,
            &master.id,
            &serde_json::json!({ "recurrence": shortened }),
            "all",
            master.etag.as_deref(),
        )
        .await?;

    // Written back unconditionally, unlike [`split_series`]' guarded fold-back,
    // and the divergence is deliberate. `to_stored` names the *patched* event's
    // own `google_id` and `upsert_event` is keyed on `(calendar_id, google_id)`,
    // so this lands on the master's row however the popover was opened —
    // including from an exception row, where `split_series` conservatively left
    // it to sync. It is worth doing here because the truncated rule is the
    // entire visible effect of the command: without it the grid goes on drawing
    // the occurrences the user has just deleted, which for a delete is the whole
    // thing they asked for.
    //
    // No [`merge_patched`] fallback, for `split_series`' reason: it does not
    // carry `recurrence`, which is this write's only payload, so it would stamp
    // the new version onto a row still holding the untruncated rule — the grid
    // expanding past the cut while the store claimed to be current.
    //
    // **What this does not repair, and the one case a user will notice.** Local
    // rows for materialised exceptions in the tail are left alone, because
    // whether Google drops them is an inference about `UNTIL` rather than
    // something this app observes — and leaving them is correct if it keeps
    // them, merely stale for a sync interval if it does not.
    //
    // The clicked row is the exception to that, in the literal sense: when the
    // popover was opened from an exception, `split_at_ms` *is* that row's own
    // `original_start_utc`, so it is inside the deleted range by construction
    // and needs no inference at all. It is still left alone, because the same
    // uncertainty about what Google does to it applies — but the visible result
    // is the worst-placed one available: the very block the user clicked goes on
    // drawing while the rest of the tail disappears around it. Task 10 wants a
    // `sync_now` after this scope for that reason, not a local re-read.
    match omacal_sync::to_stored(&patched, master_row.calendar_id, cal_tz) {
        Some(row) => {
            omacal_store::upsert_event(pool, &row).await?;
        }
        None => tracing::warn!(master = %master.id,
            "the series was shortened but the result could not be stored locally; \
             sync will pick it up"),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use omacal_store::Attendee;

    fn guest(is_self: bool) -> Attendee {
        Attendee {
            email: "me@x.com".into(),
            display_name: None,
            response_status: "needsAction".into(),
            optional: false,
            is_self,
            comment: None,
            additional_guests: 0,
        }
    }

    #[test]
    fn a_writable_calendar_where_you_are_a_guest_can_respond() {
        assert!(can_respond(false, "owner", &[guest(true)]));
        assert!(can_respond(false, "writer", &[guest(true)]));
    }

    #[test]
    fn a_read_only_calendar_cannot_respond_however_many_guests() {
        // A subscribed holiday calendar, or one shared with you read-only. The
        // buttons are hidden rather than disabled: a disabled control invites a
        // click and explains nothing.
        assert!(!can_respond(false, "reader", &[guest(true)]));
        assert!(!can_respond(false, "freeBusyReader", &[guest(true)]));
    }

    #[test]
    fn an_event_you_are_not_invited_to_cannot_be_answered() {
        // Watching someone else's calendar you have write access to. There is no
        // attendee row of yours to change, and patching would rewrite theirs.
        let others = vec![Attendee {
            email: "ana@x.com".into(),
            display_name: None,
            response_status: "accepted".into(),
            optional: false,
            is_self: false,
            comment: None,
            additional_guests: 0,
        }];
        assert!(!can_respond(false, "owner", &others));
        assert!(!can_respond(false, "owner", &[]));
    }

    /// Demo mode looks answerable from every other angle: the demo calendars
    /// are seeded `owner` and the demo event carries a `self` attendee, so
    /// both other conditions pass and the popover offered three buttons that
    /// `demo_sync_guard` could only refuse. Plan 1c settled this — "Sync now"
    /// and "Connect" are hidden in demo mode rather than left to error.
    #[test]
    fn demo_mode_offers_no_rsvp_however_writable_the_calendar_looks() {
        assert!(!can_respond(true, "owner", &[guest(true)]));
        assert!(!can_respond(true, "writer", &[guest(true)]));
    }

    #[test]
    fn only_writable_calendars_are_editable() {
        assert!(can_edit(false, "owner"));
        assert!(can_edit(false, "writer"));
        assert!(!can_edit(false, "reader"));
        assert!(!can_edit(false, "freeBusyReader"));
    }

    /// Demo mode may not write, exactly as `can_respond` refuses it — the demo
    /// calendars are seeded `owner`, so without this the form would offer a Save
    /// that the write guard can only refuse.
    #[test]
    fn demo_mode_is_never_editable() {
        assert!(!can_edit(true, "owner"));
        assert!(!can_edit(true, "writer"));
    }

    #[test]
    fn a_series_master_is_recurring() {
        assert!(is_recurring(&Some("RRULE:FREQ=DAILY".into()), &None));
    }

    /// A materialised exception carries no `recurrence` of its own — that
    /// field belongs to the master it overrides — so `is_recurring` has to
    /// catch this arm through `recurring_event_id` alone.
    #[test]
    fn a_materialised_exception_is_recurring() {
        assert!(is_recurring(&None, &Some("master-google-id".into())));
    }

    #[test]
    fn a_one_off_event_is_not_recurring() {
        assert!(!is_recurring(&None, &None));
    }

    fn three() -> Vec<Attendee> {
        vec![
            Attendee { email: "ana@x.com".into(), display_name: Some("Ana".into()),
                       response_status: "accepted".into(), optional: false, is_self: false,
                       comment: Some("running 5 late".into()), additional_guests: 1 },
            Attendee { email: "me@x.com".into(), display_name: None,
                       response_status: "needsAction".into(), optional: false, is_self: true,
                       comment: None, additional_guests: 0 },
            Attendee { email: "petya@x.com".into(), display_name: None,
                       response_status: "declined".into(), optional: true, is_self: false,
                       comment: None, additional_guests: 2 },
        ]
    }

    #[test]
    fn responding_changes_only_your_own_row() {
        // Google replaces the attendee array wholesale on patch. Sending a list
        // that has quietly reset someone else's answer is the worst thing this
        // feature could do to a real calendar, so this is the load-bearing test.
        let out = attendees_with_self_response(&three(), "declined").unwrap();
        assert_eq!(out.len(), 3, "an attendee was dropped");
        assert_eq!(out[0]["email"], "ana@x.com");
        assert_eq!(out[0]["responseStatus"], "accepted", "Ana's answer was overwritten");
        assert_eq!(out[0]["displayName"], "Ana", "Ana's display name was dropped");
        assert_eq!(out[0]["comment"], "running 5 late", "Ana's comment was dropped");
        assert_eq!(out[0]["additionalGuests"], 1, "Ana's additional guests were dropped");
        assert_eq!(out[1]["email"], "me@x.com");
        assert_eq!(out[1]["responseStatus"], "declined");
        assert_eq!(out[2]["email"], "petya@x.com");
        assert_eq!(out[2]["responseStatus"], "declined", "Petya's answer was overwritten");
        assert_eq!(out[2]["optional"], true, "the optional flag was lost");
        assert_eq!(out[2]["additionalGuests"], 2, "Petya's additional guests were dropped");
    }

    #[test]
    fn without_a_self_row_there_is_nothing_to_answer() {
        let others: Vec<Attendee> = three().into_iter().filter(|a| !a.is_self).collect();
        assert!(attendees_with_self_response(&others, "accepted").is_none());
        assert!(attendees_with_self_response(&[], "accepted").is_none());
    }

    /// **The guest list a form shows for `attendees`, with nothing touched**:
    /// every stored address carrying the optional flag it already has.
    ///
    /// Built from the stored list rather than typed out as addresses, and that
    /// is not tidiness. A helper that spelled the addresses alone would have to
    /// invent an `optional` for each, and the obvious invention — `false` for
    /// all of them — silently *changes* Petya, who is stored optional. Every
    /// test below that says "nothing changed" would then have been asserting
    /// something else, and the premise test is what caught it.
    fn guests_of(attendees: &[Attendee]) -> Vec<crate::write::Guest> {
        attendees
            .iter()
            .map(|a| crate::write::Guest { email: a.email.clone(), optional: a.optional })
            .collect()
    }

    /// [`guests_of`] with one address typed in at the end — somebody the user
    /// has just invited.
    fn guests_plus(attendees: &[Attendee], email: &str) -> Vec<crate::write::Guest> {
        let mut w = guests_of(attendees);
        w.push(crate::write::Guest { email: email.into(), optional: false });
        w
    }

    /// [`guests_of`] with one address taken out — somebody the user has just
    /// removed.
    fn guests_without(attendees: &[Attendee], email: &str) -> Vec<crate::write::Guest> {
        guests_of(attendees).into_iter().filter(|g| g.email != email).collect()
    }

    /// **The assertion the whole guest-list design exists for** (spec §2, §7).
    ///
    /// `attendees` is a whole-list replace, so adding one person means resending
    /// everyone. Send `{"email": …}` for the three who were already there and
    /// Ana's `accepted` can come back as `needsAction` — on her calendar, not
    /// just this one — and the popover this app is built around would be showing
    /// a room full of un-answered guests that nobody un-answered.
    ///
    /// Every field is named individually rather than compared as a blob,
    /// because each is a separate way to lose somebody's data and a blob
    /// comparison says only that *something* moved.
    #[test]
    fn adding_a_guest_sends_everyone_elses_fields_back_untouched() {
        let out = attendees_for_edit(&three(), &guests_plus(&three(), "dan@x.com"));

        assert_eq!(out.len(), 4, "the list must carry everyone, not just the new one");

        assert_eq!(out[0]["email"], "ana@x.com");
        assert_eq!(out[0]["responseStatus"], "accepted", "Ana's answer was reset");
        assert_eq!(out[0]["displayName"], "Ana", "Ana's display name was dropped");
        assert_eq!(out[0]["comment"], "running 5 late", "Ana's comment was dropped");
        assert_eq!(out[0]["additionalGuests"], 1, "Ana's additional guests were dropped");

        assert_eq!(out[1]["email"], "me@x.com");
        assert_eq!(out[1]["responseStatus"], "needsAction");

        assert_eq!(out[2]["email"], "petya@x.com");
        assert_eq!(out[2]["responseStatus"], "declined", "Petya's answer was reset");
        assert_eq!(out[2]["optional"], true, "Petya's optional flag was lost");
        assert_eq!(out[2]["additionalGuests"], 2, "Petya's additional guests were dropped");

        // And the newcomer, who genuinely has no answer yet.
        assert_eq!(out[3]["email"], "dan@x.com");
        assert_eq!(out[3]["responseStatus"], "needsAction");
        assert_eq!(out[3]["optional"], false);
    }

    /// Removal is "send the list without them" — there is no remove call — and
    /// it must not disturb anybody else on the way.
    #[test]
    fn removing_a_guest_leaves_everyone_elses_answer_alone() {
        let out = attendees_for_edit(&three(), &guests_without(&three(), "petya@x.com"));

        assert_eq!(out.len(), 2, "Petya should be the only one gone");
        assert!(
            !out.iter().any(|a| a["email"] == "petya@x.com"),
            "the removed guest is still on the list"
        );
        assert_eq!(out[0]["responseStatus"], "accepted", "Ana's answer moved during a removal");
        assert_eq!(out[0]["comment"], "running 5 late");
    }

    /// The one field the form may overrule, and it must overrule *only* that
    /// one: a version that rebuilt the attendee from the form would pass an
    /// assertion on `optional` while quietly resetting the answer beside it.
    #[test]
    fn marking_a_guest_optional_changes_that_flag_and_nothing_else() {
        // Ana becomes optional, Petya stops being: both directions, so the
        // flag cannot be satisfied by a version that always sends `true`.
        let mut wanted = guests_of(&three());
        wanted[0].optional = true;
        wanted[2].optional = false;
        let out = attendees_for_edit(&three(), &wanted);

        assert_eq!(out[0]["optional"], true, "Ana was not made optional");
        assert_eq!(out[0]["responseStatus"], "accepted", "Ana's answer moved with her flag");
        assert_eq!(out[0]["displayName"], "Ana");
        assert_eq!(out[2]["optional"], false, "Petya was not made required");
        assert_eq!(out[2]["responseStatus"], "declined");
    }

    /// §5: a duplicate address is a no-op, not an error and not a second row.
    #[test]
    fn a_duplicate_address_adds_no_second_row() {
        let out = attendees_for_edit(&three(), &guests_plus(&three(), "ana@x.com"));
        assert_eq!(out.len(), 3, "Ana was invited twice");

        // And the same for two mentions of somebody genuinely new.
        let twice = attendees_for_edit(&[], &guests_plus(&[], "dan@x.com")
            .into_iter()
            .chain(guests_plus(&[], "dan@x.com"))
            .collect::<Vec<_>>());
        assert_eq!(twice.len(), 1, "a new address was added twice");
    }

    /// Addresses are compared case-insensitively and trimmed. Typing `Ana@X.com`
    /// beside a stored `ana@x.com` is the same person: treated otherwise, the
    /// list goes out with Ana twice — the second entry carrying no answer, which
    /// is the reset this design is about, wearing a duplicate's clothes.
    #[test]
    fn an_address_that_differs_only_in_case_or_spacing_is_the_same_person() {
        let mut wanted = guests_of(&three());
        wanted[0].email = " Ana@X.com ".into();
        let out = attendees_for_edit(&three(), &wanted);
        assert_eq!(out.len(), 3, "Ana was treated as a second person");
        assert_eq!(out[0]["email"], "ana@x.com", "the stored spelling must win");
        assert_eq!(out[0]["responseStatus"], "accepted");
    }

    /// The premise the "send nothing when nothing changed" rule rests on: an
    /// untouched list must serialize **exactly** as [`attendees_verbatim`] does,
    /// so equality between the two means "the user changed nothing" and needs no
    /// second definition of sameness to keep in step.
    #[test]
    fn an_unchanged_list_serializes_exactly_as_the_stored_one() {
        let unchanged = attendees_for_edit(&three(), &guests_of(&three()));
        assert_eq!(unchanged, attendees_verbatim(&three()));

        // And order is not part of it: the form renders from a list it may
        // reorder, and a reshuffle nobody asked for must not read as a change.
        let mut shuffled_in = guests_of(&three());
        shuffled_in.rotate_right(1);
        let shuffled = attendees_for_edit(&three(), &shuffled_in);
        assert_eq!(shuffled, attendees_verbatim(&three()), "a reordered list read as a change");
    }

    /// Removing everybody is a thing a user can ask for, and it is not the same
    /// as not touching the list — see [`crate::write::EventFields::guests`].
    #[test]
    fn removing_everyone_sends_an_empty_list_rather_than_the_old_one() {
        assert!(attendees_for_edit(&three(), &[]).is_empty());
    }

    #[test]
    fn answering_the_whole_series_targets_the_master() {
        // An exception row carries the series id; the master carries its own.
        assert_eq!(target_event_id("all", Some("master-1"), "instance-9"), Target::Master("master-1".into()));
        assert_eq!(target_event_id("all", None, "master-1"), Target::Master("master-1".into()));
    }

    #[test]
    fn answering_one_occurrence_asks_google_which_instance_it_is() {
        // Instance ids look like `{master}_{20260813T060000Z}`, and formatting that
        // by hand works until an all-day event or an already-moved occurrence
        // breaks it silently. The caller must resolve it against the API instead.
        assert_eq!(
            target_event_id("this", Some("master-1"), "instance-9"),
            Target::Instance { master: "master-1".into(), fallback: "instance-9".into() }
        );
    }

    #[test]
    fn a_one_off_event_is_patched_directly_whatever_the_scope() {
        // No recurrence anywhere: both scopes mean the same single event, and no
        // instance lookup should happen.
        assert_eq!(target_event_id("this", None, "ev1"), Target::Master("ev1".into()));
    }

    #[test]
    fn the_instance_lookup_window_is_bracketed_by_the_clicked_occurrence_not_the_series_start() {
        // Mirrors a real bug: every expanded occurrence of a recurring master
        // shares the same database row (`commands::to_ui`), and hence the same
        // `start_utc` — the series' own DTSTART. Bracketing the lookup by that
        // would always resolve occurrence #0, no matter which day was actually
        // clicked; the window has to come from the clicked occurrence itself.
        const DAY: i64 = 24 * 3_600_000;
        let series_dtstart = 1_785_715_200_000; // Monday, occurrence #0
        let occurrence_4 = series_dtstart + 4 * DAY; // Friday, occurrence #4

        let window_0 = instance_lookup_window(series_dtstart);
        let window_4 = instance_lookup_window(occurrence_4);

        assert_ne!(window_0, window_4, "the window must move with the clicked occurrence");
        assert_eq!(window_4.0, omacal_sync::to_rfc3339(occurrence_4));
        assert_eq!(window_4.1, omacal_sync::to_rfc3339(occurrence_4 + 1000));
    }

    /// `timeMin` bounds an instance's *end*, exclusively — not its start. A
    /// window that starts even a moment before the clicked occurrence sweeps
    /// in the occurrence *before* it whenever the series is contiguous
    /// (back-to-back 30-minute standups, an all-day event repeating daily):
    /// that predecessor's end is exactly this occurrence's start, so it
    /// clears an exclusive bound placed any earlier, and Google returns it
    /// first because it starts first. The RSVP then lands on the wrong day
    /// and `sendUpdates=all` mails it to everyone.
    #[test]
    fn the_window_starts_at_the_occurrence_so_a_contiguous_predecessor_cannot_match() {
        let clicked = 1_785_715_200_000;
        let predecessor_end = clicked; // back-to-back: it ends as this one starts
        let (time_min, _) = instance_lookup_window(clicked);

        assert_eq!(
            time_min,
            omacal_sync::to_rfc3339(predecessor_end),
            "timeMin is exclusive on an instance's *end*: set any earlier than the clicked \
             start and the predecessor ending there clears it and is returned first"
        );
    }

    fn wire_instance(id: &str) -> omacal_google::model::Event {
        omacal_google::model::Event {
            id: id.into(), status: "confirmed".into(), etag: None, ical_uid: None,
            summary: None, description: None, location: None,
            start: Default::default(), end: Default::default(),
            recurrence: None, recurring_event_id: None, original_start_time: None,
            hangout_link: None, attendees: vec![], sequence: 0, organizer: Default::default(),
            reminders: Default::default(),
        }
    }

    #[test]
    fn a_found_instance_id_is_used_verbatim() {
        let found = vec![wire_instance("master_20260804T060000Z")];
        assert_eq!(
            resolve_instance_id(&found, "master", "instance-9").unwrap(),
            "master_20260804T060000Z"
        );
    }

    /// Which element is taken is not a free choice, and until this test
    /// nothing said so: no other test ever handed `resolve_instance_id` more
    /// than one instance, so `first()` could be swapped for `last()` — or any
    /// other index — without a single failure anywhere in the workspace.
    ///
    /// Google orders instances by start time and the window starts at the
    /// clicked occurrence, so the earliest is the one that was clicked; a
    /// later entry is a different occurrence, and patching it would answer the
    /// wrong day with `sendUpdates=all`.
    #[test]
    fn the_earliest_instance_returned_is_the_one_that_was_clicked() {
        let found = vec![
            wire_instance("master_20260807T090000Z"), // the clicked occurrence
            wire_instance("master_20260808T090000Z"), // a later one, ordered after it
        ];
        assert_eq!(
            resolve_instance_id(&found, "master", "master").unwrap(),
            "master_20260807T090000Z",
            "the RSVP must land on the earliest instance in the window, not a later one"
        );
    }

    #[test]
    fn an_empty_lookup_falls_back_to_the_exceptions_own_id() {
        // master != fallback: the row was already a materialised exception,
        // and its own id is a safe stand-in.
        assert_eq!(resolve_instance_id(&[], "master", "instance-9").unwrap(), "instance-9");
    }

    #[test]
    fn an_empty_lookup_on_a_bare_master_errors_instead_of_widening_to_the_whole_series() {
        // master == fallback is exactly the shape produced when the clicked
        // row *is* the series master. Falling back here would patch every
        // occurrence in the series instead of the one the user answered.
        assert!(resolve_instance_id(&[], "master-1", "master-1").is_err());
    }

    fn stored(attendees: Vec<Attendee>) -> omacal_store::StoredEvent {
        omacal_store::StoredEvent {
            id: 1, calendar_id: 1, google_id: "ev1".into(),
            summary: None, location: None, start_utc: 0, end_utc: 0,
            start_tz: "UTC".into(), end_tz: "UTC".into(), is_all_day: false,
            recurrence: None, recurring_event_id: None, original_start_utc: None,
            status: "confirmed".into(), self_response: Some("needsAction".into()),
            conference_uri: None, color_hex: None, calendar_timezone: "UTC".into(),
            description: None,
            etag: Some("\"old\"".into()), sequence: 1, organizer_email: None,
            attendees,
            reminders: Default::default(), calendar_default_reminders: Vec::new(),
        }
    }

    /// `merge_patched` is what makes the week grid's colouring reflect an RSVP
    /// immediately, without waiting for the next sync — it must actually
    /// re-derive `self_response` from the patched attendees, not carry the
    /// stale value on `row` through untouched.
    #[test]
    fn merge_patched_updates_etag_sequence_attendees_and_derives_self_response() {
        let mut row = stored(vec![guest(true)]);
        let patched = omacal_google::model::Event {
            id: "ev1".into(), status: "confirmed".into(), etag: Some("\"new\"".into()),
            ical_uid: None, summary: None, description: None, location: None,
            start: Default::default(), end: Default::default(),
            recurrence: None, recurring_event_id: None, original_start_time: None,
            hangout_link: None,
            attendees: vec![omacal_google::model::Attendee {
                email: "me@x.com".into(), display_name: None,
                response_status: "declined".into(), optional: false, is_self: true,
                comment: None, additional_guests: 0,
            }],
            sequence: 5,
            organizer: Default::default(),
            reminders: Default::default(),
        };
        merge_patched(&mut row, &patched);
        assert_eq!(row.etag.as_deref(), Some("\"new\""));
        assert_eq!(row.sequence, 5);
        assert_eq!(row.attendees.len(), 1);
        assert_eq!(
            row.self_response.as_deref(), Some("declined"),
            "self_response must be re-derived from the patched attendees, not left stale"
        );
    }

    // --- respond_via_client: reachable without touching load_config or the
    // Keychain, since the CalendarClient is a parameter rather than built
    // inside. Points it at a wiremock server and a `connect_memory` pool.

    /// One account, one calendar, and `ev` upserted onto it — enough for
    /// `respond_via_client`'s own reads (`event_by_id` inside
    /// `event_detail_impl`) to succeed afterward. Returns the store row id.
    async fn seeded_pool_with(ev: &omacal_store::StoredEvent) -> (SqlitePool, i64) {
        let pool = omacal_store::connect_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (google_sub, email, created_at) VALUES ('s','e@x',0)")
            .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
             VALUES (1, 'primary', 'Work', 'UTC', 'owner')",
        ).execute(&pool).await.unwrap();
        let id = omacal_store::upsert_event(&pool, ev).await.unwrap();
        (pool, id)
    }

    /// Guards the local write-back on its own: deleting `merge_patched` +
    /// `upsert_event` from `respond_via_client` entirely does not fail
    /// `cargo test --workspace` anywhere else, because nothing else calls
    /// this function.
    #[tokio::test]
    async fn a_successful_patch_folds_its_response_back_into_the_local_row() {
        let ev = stored(vec![guest(true)]);
        let (pool, id) = seeded_pool_with(&ev).await;

        let server = wiremock::MockServer::start().await;
        // The other half of the `if_match` decision: the patch is going to
        // this row's *own* id, so the row's etag is the right precondition
        // and must still be sent. Dropping it unconditionally would make
        // every RSVP a last-writer-wins overwrite.
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/primary/events/ev1"))
            .and(wiremock::matchers::header("if-match", "\"old\""))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ev1", "status": "confirmed", "etag": "\"new\"", "sequence": 4,
                "attendees": [{"email": "me@x.com", "responseStatus": "declined",
                               "optional": false, "self": true}]
            })))
            .mount(&server).await;

        let client = omacal_google::CalendarClient::new(server.uri(), "at-1");
        let body_attendees = attendees_with_self_response(&ev.attendees, "declined").unwrap();
        respond_via_client(&pool, "declined", "all", 0, ev, "primary", body_attendees, &client)
            .await
            .unwrap();

        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.etag.as_deref(), Some("\"new\""), "the write-back did not happen");
        assert_eq!(row.sequence, 4);
        assert_eq!(row.self_response.as_deref(), Some("declined"));
    }

    /// The single most dangerous regression this feature can have: retrying
    /// with the *stale* body would silently overwrite whatever change caused
    /// the 412 in the first place. The mock event has gained a second
    /// attendee (`ana@x.com`) between the first attempt and the retry — the
    /// retry's body must include her, not just re-send the original
    /// one-attendee payload.
    #[tokio::test]
    async fn a_stale_etag_retries_with_the_freshly_fetched_attendees_not_the_stale_ones() {
        let ev = stored(vec![guest(true)]);
        let (pool, _id) = seeded_pool_with(&ev).await;

        let server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/primary/events/ev1"))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "attendees": [{"email": "me@x.com", "responseStatus": "declined",
                               "optional": false, "additionalGuests": 0}]
            })))
            .respond_with(wiremock::ResponseTemplate::new(412))
            .expect(1)
            .mount(&server).await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/primary/events/ev1"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ev1", "status": "confirmed", "etag": "\"fresh\"",
                "attendees": [
                    {"email": "me@x.com", "responseStatus": "needsAction",
                     "optional": false, "self": true},
                    {"email": "ana@x.com", "responseStatus": "tentative", "optional": false}
                ]
            })))
            .mount(&server).await;

        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/primary/events/ev1"))
            .and(wiremock::matchers::header("if-match", "\"fresh\""))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "attendees": [
                    {"email": "me@x.com", "responseStatus": "declined",
                     "optional": false, "additionalGuests": 0},
                    {"email": "ana@x.com", "responseStatus": "tentative",
                     "optional": false, "additionalGuests": 0}
                ]
            })))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ev1", "status": "confirmed", "etag": "\"new\""
            })))
            .expect(1)
            .mount(&server).await;

        let client = omacal_google::CalendarClient::new(server.uri(), "at-1");
        let body_attendees = attendees_with_self_response(&ev.attendees, "declined").unwrap();
        respond_via_client(&pool, "declined", "all", 0, ev, "primary", body_attendees, &client)
            .await
            .unwrap();
        // `.expect(1)` on the second PATCH mock fails on drop if the retry
        // never sent that body — including if it resent the stale one, which
        // would either 404 (no mock matches it a second time) or panic via
        // `unwrap()` on the resulting `PreconditionFailed`.
    }

    /// Combines the fix for Critical 1 (the lookup window must come from the
    /// *clicked* occurrence, not the master's own `start_utc`) with the fix
    /// for Important 3 (a patch that landed on a different Google id than the
    /// row loaded must not be folded back onto that row).
    #[tokio::test]
    async fn answering_a_non_first_occurrence_targets_that_occurrence_and_leaves_the_local_master_row_alone() {
        const DAY: i64 = 24 * 3_600_000;
        const SERIES_DTSTART: i64 = 1_785_715_200_000; // Monday
        let occurrence_4 = SERIES_DTSTART + 4 * DAY; // Friday, occurrence #4

        let mut ev = stored(vec![guest(true)]);
        ev.google_id = "master1".into();
        ev.recurrence = Some("RRULE:FREQ=DAILY".into());
        ev.start_utc = SERIES_DTSTART; // every occurrence shares this row's own start
        ev.etag = Some("\"master-etag\"".into());
        let (pool, id) = seeded_pool_with(&ev).await;

        let server = wiremock::MockServer::start().await;

        // Bracketed by the clicked occurrence: a lookup bracketed by
        // SERIES_DTSTART instead would not match this mock at all, and the
        // call below would 404. `timeMin` is the occurrence's own start with
        // nothing subtracted — see `instance_lookup_window` for why a second
        // either side is not a harmless margin.
        //
        // Two items, not one: Google orders instances by start, and taking
        // anything but the first patches a different occurrence of the same
        // series. With a single item in the response that choice is invisible
        // — `found.first()` and `found.last()` agree — and nothing else in
        // the suite ever returns more than one.
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/primary/events/master1/instances"))
            .and(wiremock::matchers::query_param(
                "timeMin", omacal_sync::to_rfc3339(occurrence_4),
            ))
            .and(wiremock::matchers::query_param(
                "timeMax", omacal_sync::to_rfc3339(occurrence_4 + 1000),
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {"id": "master1_20260807T000000Z", "status": "confirmed",
                     "etag": "\"occ-4-etag\"",
                     "attendees": [{"email": "me@x.com", "responseStatus": "needsAction",
                                    "optional": false, "self": true}]},
                    {"id": "master1_20260808T000000Z", "status": "confirmed",
                     "etag": "\"occ-5-etag\"",
                     "attendees": [{"email": "me@x.com", "responseStatus": "needsAction",
                                    "optional": false, "self": true}]}
                ]
            })))
            .mount(&server).await;

        // `If-Match` is the *instance's* etag, taken from the lookup above —
        // never `master1`'s, which is the version of a different resource and
        // could only ever be rejected. Matching on the header pins which of
        // the two items was used as well: `"occ-5-etag"` here would mean the
        // second instance, i.e. the wrong day.
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/primary/events/master1_20260807T000000Z"))
            .and(wiremock::matchers::header("if-match", "\"occ-4-etag\""))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1_20260807T000000Z", "status": "confirmed", "etag": "\"occ-etag\"",
                "attendees": [{"email": "me@x.com", "responseStatus": "declined",
                               "optional": false, "self": true}]
            })))
            .expect(1)
            .mount(&server).await;

        let client = omacal_google::CalendarClient::new(server.uri(), "at-1");
        let body_attendees = attendees_with_self_response(&ev.attendees, "declined").unwrap();
        respond_via_client(
            &pool, "declined", "this", occurrence_4, ev, "primary", body_attendees, &client,
        )
        .await
        .unwrap();

        // The occurrence was patched — proved above by `.expect(1)` and by
        // `unwrap()` not panicking (a wrongly-bracketed lookup 404s). The
        // *local master row* must be untouched: `master1_20260807T000000Z` is
        // a different Google id than `master1`, and stamping the instance's
        // response onto the master's row would corrupt it.
        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.etag.as_deref(), Some("\"master-etag\""),
            "the instance's etag must not be stamped onto the master's own row");
        assert_eq!(row.self_response.as_deref(), Some("needsAction"),
            "the master's local self_response must not change from one occurrence's answer");
    }

    /// Provenance. The patch body for a resolved occurrence must be built
    /// from *that occurrence's* guest list, as Google just reported it — not
    /// from the master row this store happens to hold.
    ///
    /// The scenario is ordinary, not contrived. A colleague answering "this
    /// event" on one occurrence is itself what materialises that instance on
    /// Google's side; until the next sync (five minutes at most) this store
    /// still has only the master, and `suppressed_slots` renders the master
    /// for that slot. Answering the same occurrence in that window with the
    /// master's array would push Ana's stale `accepted` back over her
    /// `declined` — and `sendUpdates=all` would tell the whole guest list
    /// about it.
    #[tokio::test]
    async fn an_occurrence_rsvp_carries_that_occurrences_guest_list_not_the_masters() {
        const OCCURRENCE: i64 = 1_785_715_200_000;

        // The stored master: Ana still reads `accepted` here, because this
        // store has not seen her exception yet.
        let mut ev = stored(vec![
            Attendee {
                email: "ana@x.com".into(), display_name: None,
                response_status: "accepted".into(), optional: false, is_self: false,
                comment: None, additional_guests: 0,
            },
            guest(true),
        ]);
        ev.google_id = "master1".into();
        ev.recurrence = Some("RRULE:FREQ=DAILY".into());
        ev.start_utc = OCCURRENCE;
        ev.etag = Some("\"master-etag\"".into());
        let (pool, _id) = seeded_pool_with(&ev).await;

        let server = wiremock::MockServer::start().await;

        // What Google actually has for this occurrence: Ana declined it.
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/primary/events/master1/instances"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{
                    "id": "master1_20260804T060000Z", "status": "confirmed",
                    "etag": "\"occ-etag\"",
                    "attendees": [
                        {"email": "ana@x.com", "responseStatus": "declined", "optional": false},
                        {"email": "me@x.com", "responseStatus": "needsAction",
                         "optional": false, "self": true}
                    ]
                }]
            })))
            .mount(&server).await;

        // The only body this may send. Built from the master's array instead,
        // Ana would read `accepted` and nothing here would match — the call
        // then 404s and the `unwrap()` below panics.
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/primary/events/master1_20260804T060000Z"))
            .and(wiremock::matchers::header("if-match", "\"occ-etag\""))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "attendees": [
                    {"email": "ana@x.com", "responseStatus": "declined",
                     "optional": false, "additionalGuests": 0},
                    {"email": "me@x.com", "responseStatus": "accepted",
                     "optional": false, "additionalGuests": 0}
                ]
            })))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1_20260804T060000Z", "status": "confirmed", "etag": "\"occ-2\""
            })))
            .expect(1)
            .mount(&server).await;

        let client = omacal_google::CalendarClient::new(server.uri(), "at-1");
        // Deliberately the master's own attendees, exactly as `respond_impl`
        // builds them: the fix is that `respond_via_client` replaces this
        // once it knows the patch is going somewhere else.
        let body_attendees = attendees_with_self_response(&ev.attendees, "accepted").unwrap();
        respond_via_client(
            &pool, "accepted", "this", OCCURRENCE, ev, "primary", body_attendees, &client,
        )
        .await
        .unwrap();
    }

    /// The other half of taking the instance as authoritative: if *its* list
    /// has no row of ours, there is nothing to answer, and the master's list
    /// is not a stand-in for one — sending it is the write this whole fix
    /// exists to stop. No PATCH mock is mounted at all, so any attempt to
    /// send one 404s rather than passing quietly.
    #[tokio::test]
    async fn an_occurrence_you_are_no_longer_a_guest_on_is_not_answered_from_the_masters_list() {
        const OCCURRENCE: i64 = 1_785_715_200_000;

        let mut ev = stored(vec![guest(true)]);
        ev.google_id = "master1".into();
        ev.recurrence = Some("RRULE:FREQ=DAILY".into());
        ev.start_utc = OCCURRENCE;
        let (pool, _id) = seeded_pool_with(&ev).await;

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/primary/events/master1/instances"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{
                    "id": "master1_20260804T060000Z", "status": "confirmed",
                    "etag": "\"occ-etag\"",
                    "attendees": [
                        {"email": "ana@x.com", "responseStatus": "accepted", "optional": false}
                    ]
                }]
            })))
            .mount(&server).await;

        let client = omacal_google::CalendarClient::new(server.uri(), "at-1");
        let body_attendees = attendees_with_self_response(&ev.attendees, "declined").unwrap();
        let err = respond_via_client(
            &pool, "declined", "this", OCCURRENCE, ev, "primary", body_attendees, &client,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("not a guest on this event"), "{err}");
    }

    /// The one combination nothing else here covers end to end: an exception
    /// row answered with `scope: "this"`. It is the only path that does an
    /// instances lookup, comes back to the *same* resource it started from,
    /// and then writes back locally — the other three tests each cover at
    /// most two of those.
    ///
    /// The pieces are individually guarded (`resolve_instance_id` picks the
    /// id, a one-off covers the same-resource arm, another covers the
    /// write-back), so no mutation of today's code slips past unnoticed
    /// without this. It earns its place against a *future* change: if
    /// `resolve_instance_id`'s contract moves, this is the shape that
    /// silently starts patching the master instead.
    #[tokio::test]
    async fn answering_one_occurrence_from_an_exception_row_patches_that_row_and_folds_it_back() {
        const OCCURRENCE: i64 = 1_785_715_200_000;

        let mut ev = stored(vec![guest(true)]);
        ev.google_id = "exception1".into();
        ev.recurring_event_id = Some("master1".into());
        ev.original_start_utc = Some(OCCURRENCE);
        ev.start_utc = OCCURRENCE;
        ev.etag = Some("\"exception-etag\"".into());
        let (pool, id) = seeded_pool_with(&ev).await;

        let server = wiremock::MockServer::start().await;

        // The lookup goes to the *master* — an exception has no instances of
        // its own — and Google answers with the exception itself, since that
        // is what now occupies the slot. So `event_id` comes back equal to
        // `ev.google_id`, and the row is its own target.
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/primary/events/master1/instances"))
            .and(wiremock::matchers::query_param(
                "timeMin", omacal_sync::to_rfc3339(OCCURRENCE),
            ))
            .and(wiremock::matchers::query_param(
                "timeMax", omacal_sync::to_rfc3339(OCCURRENCE + 1000),
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{
                    "id": "exception1", "status": "confirmed",
                    "etag": "\"exception-etag\"",
                    "attendees": [{"email": "me@x.com", "responseStatus": "needsAction",
                                   "optional": false, "self": true}]
                }]
            })))
            .expect(1)
            .mount(&server).await;

        // `exception1`, not `master1` and not a hand-formatted
        // `master1_<timestamp>`; and with a precondition, since this row does
        // hold a version of the resource being patched.
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/primary/events/exception1"))
            .and(wiremock::matchers::header("if-match", "\"exception-etag\""))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "exception1", "status": "confirmed", "etag": "\"exception-2\"",
                "sequence": 3,
                "attendees": [{"email": "me@x.com", "responseStatus": "declined",
                               "optional": false, "self": true}]
            })))
            .expect(1)
            .mount(&server).await;

        let client = omacal_google::CalendarClient::new(server.uri(), "at-1");
        let body_attendees = attendees_with_self_response(&ev.attendees, "declined").unwrap();
        respond_via_client(
            &pool, "declined", "this", OCCURRENCE, ev, "primary", body_attendees, &client,
        )
        .await
        .unwrap();

        // Patch landed on this row's own id, so the write-back is not only
        // allowed but required — the complement of
        // `answering_a_non_first_occurrence_...`, which asserts the opposite
        // for a patch that landed elsewhere.
        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.etag.as_deref(), Some("\"exception-2\""), "the write-back did not happen");
        assert_eq!(row.sequence, 3);
        assert_eq!(row.self_response.as_deref(), Some("declined"));
    }

    /// The provenance rule's other arm: `scope: "all"` from a *materialised
    /// exception* row targets the series master, which is again not the row
    /// that was loaded — and this one is not a race, it is unconditional.
    ///
    /// An exception is exactly where a per-occurrence answer lives. Ana
    /// declined one occurrence, so the exception row says `declined` while
    /// the master still says `accepted`. Answering "all of them" from that
    /// row used to send the exception's array to the master, declining Ana
    /// for the entire series and, with `sendUpdates=all`, telling everyone.
    ///
    /// This is the one branch that pays a third round trip: nothing here has
    /// the master in hand, and there is no version of it to condition on
    /// without asking.
    #[tokio::test]
    async fn answering_the_whole_series_from_an_exception_sends_the_masters_guest_list() {
        // The exception row: Ana declined *this* occurrence.
        let mut ev = stored(vec![
            Attendee {
                email: "ana@x.com".into(), display_name: None,
                response_status: "declined".into(), optional: false, is_self: false,
                comment: None, additional_guests: 0,
            },
            guest(true),
        ]);
        ev.google_id = "exception1".into();
        ev.recurring_event_id = Some("master1".into());
        ev.etag = Some("\"exception-etag\"".into());
        let (pool, id) = seeded_pool_with(&ev).await;

        let server = wiremock::MockServer::start().await;

        // The series master, where Ana is still `accepted` — she declined one
        // occurrence, not the series.
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/primary/events/master1"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"master-etag\"",
                "attendees": [
                    {"email": "ana@x.com", "responseStatus": "accepted", "optional": false},
                    {"email": "me@x.com", "responseStatus": "needsAction",
                     "optional": false, "self": true}
                ]
            })))
            .expect(1)
            .mount(&server).await;

        // Ana must still read `accepted`, and `If-Match` must be the master's
        // own version. Built from `ev` instead, Ana would read `declined` and
        // nothing here matches — the call 404s and the `unwrap()` panics.
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/primary/events/master1"))
            .and(wiremock::matchers::header("if-match", "\"master-etag\""))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "attendees": [
                    {"email": "ana@x.com", "responseStatus": "accepted",
                     "optional": false, "additionalGuests": 0},
                    {"email": "me@x.com", "responseStatus": "declined",
                     "optional": false, "additionalGuests": 0}
                ]
            })))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"master-2\""
            })))
            .expect(1)
            .mount(&server).await;

        let client = omacal_google::CalendarClient::new(server.uri(), "at-1");
        let body_attendees = attendees_with_self_response(&ev.attendees, "declined").unwrap();
        respond_via_client(
            &pool, "declined", "all", ev.start_utc, ev, "primary", body_attendees, &client,
        )
        .await
        .unwrap();

        // `master1` is a different Google id than `exception1`, so the local
        // exception row is left for the next sync rather than stamped with
        // the master's response.
        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.etag.as_deref(), Some("\"exception-etag\""),
            "the master's etag must not be stamped onto the exception's own row");
    }

    /// `can_respond` is a predicate; this is the payload the UI actually
    /// renders from. The non-demo arm is what stops the demo arm being
    /// vacuous — a fixture that could not be answered either way would prove
    /// nothing about demo mode.
    #[tokio::test]
    ///
    /// Driven through `state_with`, the same `AppState` the `#[tauri::command]`
    /// wrapper builds from, rather than through a loose `demo` argument: the
    /// wrapper is then a call with nothing left to get wrong, and this test is
    /// what proves the flag it carries is the app's own.
    async fn the_detail_payload_reports_no_rsvp_in_demo_mode() {
        let ev = stored(vec![guest(true)]);
        let (pool, id) = seeded_pool_with(&ev).await; // its calendar is seeded `owner`

        let live = event_detail_impl(&state_with(pool.clone(), false), id).await.unwrap();
        assert!(live.can_respond, "the fixture must be answerable outside demo mode");

        let demo = event_detail_impl(&state_with(pool, true), id).await.unwrap();
        assert!(
            !demo.can_respond,
            "demo mode offered RSVP buttons that `demo_sync_guard` can only refuse"
        );
    }

    /// The Repeat control needs the real RRULE to decide whether it can represent
    /// it (see `write::repeat_from_rrule`). Dropping it here would make every
    /// exotic rule look like "Never" and invite a silent overwrite.
    #[tokio::test]
    async fn detail_carries_the_raw_recurrence_rule() {
        let mut ev = stored(vec![]);
        ev.recurrence = Some("RRULE:FREQ=MONTHLY;BYDAY=-1FR".into());
        let (pool, id) = seeded_pool_with(&ev).await; // its calendar is seeded `owner`

        // The real id `seeded_pool_with` assigned its one calendar — not
        // assumed to be any particular number, so this cannot pass by
        // coincidence with `stored`'s own hardcoded `calendar_id: 1`. Task 5
        // routes writes by this field, so a wrong value here does not fail a
        // test there — it creates the event on the wrong calendar.
        let cal_id: i64 =
            sqlx::query_scalar("SELECT id FROM calendars LIMIT 1").fetch_one(&pool).await.unwrap();

        let d = event_detail_impl(&state_with(pool, false), id).await.unwrap();
        assert_eq!(d.calendar_id, cal_id, "calendar_id must be the event's own, not dropped or hardcoded");
        assert_eq!(d.recurrence.as_deref(), Some("RRULE:FREQ=MONTHLY;BYDAY=-1FR"));
        assert!(d.can_edit);
    }

    /// The other half of the same payload: the raw rule tells the form what to
    /// *show*, this tells it whether the Repeat control may be used at all.
    ///
    /// Computed here rather than in TypeScript on purpose — `repeat_from_rrule`
    /// owns both the exact base rules and the strict plain-weekly grammar, and
    /// a second copy of either in the UI would drift. Three different rules
    /// producing three different answers is what makes this test able to fail:
    /// a stub returning any one constant is caught by the other two.
    #[tokio::test]
    async fn detail_reports_which_repeat_option_the_rule_is() {
        let detail_for = |recurrence: Option<&'static str>| async move {
            let mut ev = stored(vec![]);
            ev.recurrence = recurrence.map(str::to_string);
            let (pool, id) = seeded_pool_with(&ev).await;
            event_detail_impl(&state_with(pool, false), id).await.unwrap()
        };

        // A rule `rrule_for` authors, matched exactly.
        assert_eq!(detail_for(Some("RRULE:FREQ=WEEKLY")).await.repeat, "weekly");
        let patterned = detail_for(Some("RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR")).await;
        assert_eq!(patterned.repeat, "weekly");
        assert_eq!(patterned.weekly_days, ["MO", "WE", "FR"]);
        let counted = detail_for(Some("RRULE:FREQ=DAILY;COUNT=8")).await;
        assert_eq!(counted.repeat, "daily");
        assert_eq!(counted.repeat_end, crate::write::RepeatEnd::After { count: 8 });
        // A rule it does not, and could not: the form must refuse to overwrite it.
        assert_eq!(
            detail_for(Some("RRULE:FREQ=WEEKLY;INTERVAL=2")).await.repeat,
            "custom",
            "a fortnightly rule must not be reported as one omacal can express"
        );
        // No rule at all is not the same as an unrepresentable one.
        assert_eq!(detail_for(None).await.repeat, "never");
    }

    /// **The display half of the boundary defect.** An all-day event has a
    /// *date*, and the store holds it as midnight in the **calendar's** zone —
    /// Google sends a bare `date` and `omacal_sync::resolve` falls back to
    /// `calendars.timezone`. Only that zone reads the date back unchanged. The
    /// UI derives one from `start_ms` in the *browser's* zone today, which for
    /// any user east of the calendar is the previous day, so a trip on the 10th
    /// opens the form showing the 9th and saves as a two-day event starting the
    /// 9th with `sendUpdates=all`. These fields are what let it stop.
    ///
    /// **`Europe/Lisbon`, and the brief's `America/New_York` would have proved
    /// nothing.** The mutation this must catch is deriving the dates in UTC, and
    /// that only shows when the calendar's midnight falls on a *different UTC
    /// date*. New York is UTC-4 in August: midnight there is 04:00 UTC the same
    /// morning, so UTC answers `2026-08-10` too and the wrong zone passes
    /// unnoticed — on both dates. Lisbon is UTC+1 in August, so each of these
    /// instants sits on the previous UTC date, which the fixture checks below
    /// pin: if either stops straddling midnight this test goes on passing while
    /// proving nothing.
    ///
    /// The event's own `start_tz` is `UTC` here (`stored`'s), and deliberately
    /// not the calendar's: reading `e.start_tz` instead of `c.timezone` is the
    /// other way to get this wrong, and it fails here rather than shipping.
    #[tokio::test]
    async fn an_all_day_detail_reports_dates_in_the_calendars_zone() {
        const CAL_TZ: &str = "Europe/Lisbon";
        let iso = |ms: i64| jiff::Timestamp::from_millisecond(ms).unwrap().to_string();

        // One day, and a three-day trip. The single-day case is the one the
        // form shows most often and the one where both dates are the same, so
        // it cannot tell an inclusive end from a copy of the start — the trip
        // is what does, and what catches an exclusive end shipped verbatim.
        let mut one_day = all_day_row("2026-08-10", "2026-08-11", CAL_TZ);
        let mut trip = all_day_row("2026-08-10", "2026-08-13", CAL_TZ);

        assert_eq!(iso(one_day.start_utc), "2026-08-09T23:00:00Z",
            "fixture check: midnight in the calendar's zone must fall on the *previous* \
             UTC date, or a UTC derivation answers the same date and passes unnoticed");
        assert_eq!(iso(one_day.end_utc), "2026-08-10T23:00:00Z", "fixture check: as above");
        assert_eq!(iso(trip.end_utc), "2026-08-12T23:00:00Z", "fixture check: as above");
        assert_eq!(one_day.start_tz, "UTC",
            "fixture check: the event's own zone must differ from its calendar's, or \
             reading `start_tz` instead of the calendar's zone passes unnoticed");

        let (pool, id) = seeded_pool_on_cal(&mut one_day, CAL_TZ).await;
        let d = event_detail_impl(&state_with(pool, false), id).await.unwrap();
        assert_eq!(d.start_date.as_deref(), Some("2026-08-10"),
            "a trip on the 10th must not be reported as starting on the 9th");
        assert_eq!(d.end_date.as_deref(), Some("2026-08-10"),
            "a one-day event's inclusive last day is its first day, not the exclusive \
             midnight after it");

        let (pool, id) = seeded_pool_on_cal(&mut trip, CAL_TZ).await;
        let d = event_detail_impl(&state_with(pool, false), id).await.unwrap();
        assert_eq!(d.start_date.as_deref(), Some("2026-08-10"));
        assert_eq!(d.end_date.as_deref(), Some("2026-08-12"),
            "the inclusive last day is the day the user would point at — one day back \
             from the exclusive end the store holds, never the exclusive date itself");
    }

    /// The other arm, and not a formality: a timed event has no date of its own
    /// — which day its instant falls on is a question about the reader — so
    /// answering one here would hand the form a date to save back as an all-day
    /// event.
    ///
    /// The fixture is [`an_all_day_detail_reports_dates_in_the_calendars_zone`]'s
    /// own row with the flag turned off, so the two tests differ in exactly
    /// that flag and nothing else. The same instants report dates there and
    /// none here, which is what makes this test about `is_all_day` rather than
    /// about the times.
    #[tokio::test]
    async fn a_timed_detail_reports_no_dates() {
        const CAL_TZ: &str = "Europe/Lisbon";

        let mut ev = all_day_row("2026-08-10", "2026-08-11", CAL_TZ);
        ev.is_all_day = false;

        let (pool, id) = seeded_pool_on_cal(&mut ev, CAL_TZ).await;
        let d = event_detail_impl(&state_with(pool, false), id).await.unwrap();

        assert!(!d.is_all_day, "fixture check: this row is the timed arm");
        assert_eq!(d.start_date, None, "a timed event has no date of its own");
        assert_eq!(d.end_date, None, "a timed event has no date of its own");
    }

    /// **The Rust↔TypeScript wire contract for [`EventDetail`], pinned by
    /// name.**
    ///
    /// This struct only ever crosses as JSON — `invoke<EventDetail>` on the
    /// other side — and until this test nothing in the workspace serialized it
    /// at all. Adding `#[serde(rename = "startDate")]` to
    /// [`EventDetail::start_date`] passed `cargo test`, `cargo clippy` **and**
    /// the UI's `npm run check`, while every all-day form read `undefined` for
    /// the date it now depends on: no Playwright spec reaches Rust (they all
    /// stub the detail themselves) and no Rust test looked at the JSON. The
    /// names were pinned by nothing.
    ///
    /// The **whole** key set rather than the two new fields, because every name
    /// here carries the same risk and a rename is as cheap for any of them.
    /// Sorted, because the contract is the names — the UI reads properties, not
    /// positions. The expected list is `ui/src/lib/eventdetail.ts`'s
    /// `EventDetail` type: the two are meant to be diffed against each other,
    /// and a field added on one side has to be added on the other or fail here.
    #[tokio::test]
    async fn an_event_detail_serializes_under_the_names_the_ui_reads() {
        const CAL_TZ: &str = "Europe/Lisbon";

        // An all-day row with a guest on it, so both halves below have
        // something real to look at: a timed row would serialize `start_date`
        // as `null` and an empty guest list would make the attendee check
        // vacuous.
        let mut ev = all_day_row("2026-08-10", "2026-08-11", CAL_TZ);
        ev.attendees = vec![guest(true)];
        let (pool, id) = seeded_pool_on_cal(&mut ev, CAL_TZ).await;
        let d = event_detail_impl(&state_with(pool, false), id).await.unwrap();

        let json = serde_json::to_value(&d).unwrap();
        let obj = json.as_object().expect("an EventDetail must serialize as a JSON object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();

        let mut expected = [
            "id", "calendar_id", "title", "description", "location",
            "conference_uri", "start_ms", "end_ms", "start_date", "end_date",
            "is_all_day", "is_recurring", "recurrence", "repeat", "weekly_days",
            "repeat_end", "color",
            "organizer_email", "self_response", "can_respond", "can_edit",
            "attendees", "reminders", "calendar_default_reminders",
        ];
        expected.sort_unstable();
        assert_eq!(keys, expected, "the UI reads these by name off `invoke`'s result");

        // Not merely present: an all-day event must carry both dates as
        // strings. `valueFromDetail` takes them verbatim and has no fallback —
        // deliberately, since the only fallback available is a date derived in
        // the *browser's* zone, which is the defect this plan exists to close.
        assert!(
            obj["start_date"].is_string() && obj["end_date"].is_string(),
            "an all-day detail must carry both dates, not null"
        );

        // The nested attendee, which crosses the same wire inside this one.
        // `is_self` decides the form's `selfEmail`, which `mailableGuests`
        // excludes by, and is read by property name exactly like the rest.
        //
        // A *subset* check, not equality: `comment` and `additional_guests` are
        // carried through purely so an RSVP patch does not erase them, and the
        // UI's own `Attendee` type deliberately does not declare them.
        let attendee = obj["attendees"].as_array().expect("attendees is an array")[0]
            .as_object()
            .expect("an attendee is an object");
        for name in ["email", "display_name", "response_status", "optional", "is_self"] {
            assert!(
                attendee.contains_key(name),
                "the UI's `Attendee` reads `{name}`, which is not on the wire under that name"
            );
        }
    }

    /// `respond_impl`'s own `can_respond(state.demo, …)` — the second demo
    /// gate on the write path, behind [`respond_to_event_impl`]'s. Nothing
    /// reached it before this test, because the guard in front always fired
    /// first, so `state.demo` there could be replaced with `false` and the
    /// workspace stayed green.
    ///
    /// It refuses *before* `load_config`, which is what makes it worth having:
    /// were the outer guard ever deleted, this one still stops demo mode
    /// reaching the config file, the Keychain and Google.
    #[tokio::test]
    async fn responding_refuses_in_demo_mode_even_with_the_outer_guard_bypassed() {
        let ev = stored(vec![guest(true)]);
        let (pool, id) = seeded_pool_with(&ev).await; // seeded `owner`, with a `self` guest

        // What stops the assertion below being vacuous: this fixture clears
        // every *other* condition, so only demo mode can be refusing it.
        // Checked through the predicate rather than by calling `respond_impl`
        // with `demo: false` — past `can_respond` that call reads the real
        // `~/.config/omacal/config.toml`, then the Keychain, then Google,
        // which no test may do.
        assert!(can_respond(false, "owner", &ev.attendees));

        let err = respond_impl(&state_with(pool, true), id, "declined", "all", 0)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("cannot be answered from omacal"), "{err}");
    }

    /// An `AppState` a test can hold: demo mode's whole point is that nothing
    /// below the gate runs, so the token cache starts empty and stays that
    /// way.
    fn state_with(pool: SqlitePool, demo: bool) -> AppState {
        AppState { pool, demo, tokens: Default::default(), reauth: Default::default(), update: Default::default(), update_checked_at: Default::default(), system_tz_change: Default::default(), open_date: Default::default() }
    }

    /// "Demo mode must never write to the real database or reach Google",
    /// applied to the first command in this app that writes to somebody's
    /// real calendar. Deleting the guard from `respond_to_event_impl` leaves
    /// this reading `~/.config/omacal/config.toml`, then the Keychain, then
    /// PATCHing Google with `sendUpdates=all` — against whatever account the
    /// demo database happens to name.
    #[tokio::test]
    async fn responding_refuses_in_demo_mode_without_touching_config_keyring_or_google() {
        let ev = stored(vec![guest(true)]);
        let (pool, id) = seeded_pool_with(&ev).await;
        let state = state_with(pool, true);

        let err = respond_to_event_impl(&state, id, "declined", "this", ev.start_utc)
            .await
            .unwrap_err();
        assert_eq!(err, crate::DEMO_SYNC_MESSAGE);

        // Past the guard this would have folded Google's answer back onto the
        // row — or failed with a config/keyring error, which is not this
        // message either.
        let (row, _, _) = omacal_store::event_by_id(&state.pool, id).await.unwrap().unwrap();
        assert_eq!(row.etag.as_deref(), Some("\"old\""), "demo mode wrote to the store");
        assert_eq!(row.self_response.as_deref(), Some("needsAction"));
    }

    /// The same guard on the other new command. `refresh_event` only reads
    /// from Google, but it reads with a real account's access token — so it
    /// still needs the config file and the Keychain, and demo mode has
    /// neither an account nor any business asking for one.
    #[tokio::test]
    async fn refreshing_refuses_in_demo_mode_without_touching_config_keyring_or_google() {
        let ev = stored(vec![guest(true)]);
        let (pool, id) = seeded_pool_with(&ev).await;
        let state = state_with(pool, true);

        let err = refresh_event_impl(&state, id).await.unwrap_err();
        assert_eq!(err, crate::DEMO_SYNC_MESSAGE);

        let (row, _, _) = omacal_store::event_by_id(&state.pool, id).await.unwrap().unwrap();
        assert_eq!(row.etag.as_deref(), Some("\"old\""), "demo mode wrote to the store");
    }

    /// Critical 2's failure mode, exercised end to end: no instance is found
    /// for a bare master row (`master == fallback`), and there is no PATCH
    /// mock mounted at all — if the fix regressed to "fall back to the
    /// master", this test would fail via a 404 rather than by silently
    /// succeeding and patching the whole series.
    #[tokio::test]
    async fn an_empty_instance_lookup_on_a_bare_master_errors_rather_than_patching_the_series() {
        let mut ev = stored(vec![guest(true)]);
        ev.google_id = "master1".into();
        ev.recurrence = Some("RRULE:FREQ=DAILY".into());
        let (pool, _id) = seeded_pool_with(&ev).await;

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/primary/events/master1/instances"))
            .respond_with(wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"items": []})))
            .mount(&server).await;

        let client = omacal_google::CalendarClient::new(server.uri(), "at-1");
        let body_attendees = attendees_with_self_response(&ev.attendees, "declined").unwrap();
        let start = ev.start_utc;
        let err = respond_via_client(
            &pool, "declined", "this", start, ev, "primary", body_attendees, &client,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("could not find that occurrence"), "{err}");
    }

    // --- create_event: `create_impl` / `create_via_client`, the first pair
    // in this file that writes something into existence rather than
    // changing something that already exists.

    /// One calendar with the given `access_role` and `timezone`, owned by a
    /// fresh account — everything `create_impl` needs to resolve before it
    /// can build a request. Returns the calendar's local row id, the same
    /// shape `seeded_pool_with` returns an event id for.
    async fn seed_calendar_with_tz(pool: &SqlitePool, access_role: &str, timezone: &str) -> i64 {
        sqlx::query("INSERT INTO accounts (google_sub, email, created_at) VALUES ('s','e@x',0)")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO calendars (account_id, google_id, summary, timezone, access_role)
             VALUES (1, 'cal@x.com', 'Cal', ?1, ?2)",
        )
        .bind(timezone)
        .bind(access_role)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query_scalar("SELECT id FROM calendars WHERE google_id = 'cal@x.com'")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// `seed_calendar_with_tz` with `UTC`, for the tests below that don't
    /// care what the calendar's own zone is.
    async fn seed_calendar(pool: &SqlitePool, access_role: &str) -> i64 {
        seed_calendar_with_tz(pool, access_role, "UTC").await
    }

    /// A plain one-hour timed event, with a repeating rule set on purpose:
    /// `a_created_event_is_stored_locally` asserts the whole request body,
    /// and `recurrence: None` would let a mutation that silently dropped
    /// `f.recurrence` from that body pass unnoticed, since there would be
    /// nothing there to drop.
    fn sample_fields() -> crate::write::EventFields {
        crate::write::EventFields {
            summary: Some("Lunch".into()),
            location: None,
            description: None,
            when: crate::write::When::Timed {
                start_ms: 1_786_442_400_000,
                end_ms: 1_786_446_000_000,
            },
            tz: "Europe/Sofia".into(),
            recurrence: Some(Some("RRULE:FREQ=WEEKLY".into())),
            guests: None,
            // `Some` for `recurrence`'s own reason above: the whole-body
            // assertion must have a `reminders` key to lose.
            reminders: Some(crate::write::RemindersInput {
                use_default: false,
                overrides: vec![crate::write::ReminderInput {
                    method: "popup".into(),
                    minutes: 10,
                }],
            }),
        }
    }

    /// The `start` and `end` a timed pair renders to, through the very
    /// function the request builders use.
    ///
    /// Built rather than written out, deliberately: an expectation spelling
    /// `{"dateTime": …, "timeZone": …}` by hand is a second copy of the wire
    /// format, and the two would drift. Going through `when_json` means a
    /// change to the shape moves every expectation with it — while still
    /// binding the *values*, which is what these tests are actually about.
    fn timed_json(start_ms: i64, end_ms: i64, tz: &str) -> (serde_json::Value, serde_json::Value) {
        crate::write::when_json(&crate::write::When::Timed { start_ms, end_ms }, tz)
    }

    /// Demo mode must reach neither Google nor the real database. Same guard
    /// shape as `respond`, and asserted the same way: the demo failure must be
    /// the demo message, not a config or keyring error — and here, not a
    /// "calendar not found" database error either, since `calendar_id: 1` on
    /// a bare `connect_memory` pool names no calendar at all. The guard has to
    /// fire before `calendar_for_write` is ever called, or this would report
    /// the wrong failure.
    #[tokio::test]
    async fn creating_refuses_in_demo_mode_without_touching_config_keyring_or_google() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let err =
            create_impl(&state_with(pool, true), 1, sample_fields(), "none").await.unwrap_err();
        assert!(err.to_string().contains("demo"), "got: {err}");
        // Binds the emitter to `errors.rs`'s allowlist: checking only
        // `.contains` above would leave this green even if the two literals
        // drifted apart (a trailing period added to one, say), while
        // `create_event`'s real caller started reading OPAQUE instead.
        assert_eq!(crate::errors::user_facing(&err), "demo mode — there is nothing to create");
    }

    /// A subscribed holiday calendar, or one shared with you read-only, is
    /// `reader`. Creating into it must be refused before any request is
    /// built — not left to Google's own 403 — so this fixture points at no
    /// mock server at all: a request going out at all would panic on the
    /// missing `CalendarClient`, not merely fail an assertion.
    #[tokio::test]
    async fn creating_into_a_read_only_calendar_is_refused() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let cal = seed_calendar(&pool, "reader").await;
        let err =
            create_impl(&state_with(pool, false), cal, sample_fields(), "none").await.unwrap_err();
        assert!(err.to_string().contains("not writable"), "got: {err}");
        assert_eq!(crate::errors::user_facing(&err), "this calendar is not writable from omacal");
    }

    /// **A create carrying guests invites them.**
    ///
    /// The array is `attendees_for_edit(&[], wanted)` — the same builder the
    /// edit path uses, against an empty "already on the event" list, because a
    /// brand-new event has nobody on it. Reusing it rather than writing a
    /// second array here is what keeps one authority for an attendee's shape:
    /// a new guest is `needsAction` with an explicit `optional` and
    /// `additionalGuests`, whichever path invited them.
    ///
    /// Asserted on the **whole body** with `body_json`, which is what makes
    /// "the guests were dropped" and "the guests were sent flat as `{email}`"
    /// both failures rather than one. The second matters: `{email}` alone is
    /// the shape that resets an RSVP (guest-list spec §2), and on a create it
    /// would look harmless right up until the same habit reached an edit.
    #[tokio::test]
    async fn creating_an_event_with_guests_invites_them() {
        let fields = crate::write::EventFields {
            guests: Some(vec![
                crate::write::Guest { email: "dan@x.com".into(), optional: false },
                crate::write::Guest { email: "eve@x.com".into(), optional: true },
            ]),
            ..sample_fields()
        };
        let (start, end) = crate::write::when_json(&fields.when, &fields.tz);
        let expected_body = serde_json::json!({
            "start": start,
            "end":   end,
            "summary": "Lunch",
            "recurrence": ["RRULE:FREQ=WEEKLY"],
            "reminders": { "useDefault": false,
                           "overrides": [{ "method": "popup", "minutes": 10 }] },
            "attendees": [
                { "email": "dan@x.com", "responseStatus": "needsAction",
                  "optional": false, "additionalGuests": 0 },
                { "email": "eve@x.com", "responseStatus": "needsAction",
                  "optional": true,  "additionalGuests": 0 },
            ],
        });

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .and(wiremock::matchers::body_json(expected_body))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "g-new", "status": "confirmed", "etag": "\"e1\"",
                "summary": "Lunch",
                "start": {"dateTime": "2026-08-10T12:00:00+03:00"},
                "end":   {"dateTime": "2026-08-10T13:00:00+03:00"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let pool = omacal_store::connect_memory().await.unwrap();
        let cal = seed_calendar(&pool, "owner").await;
        let client = omacal_google::CalendarClient::new(server.uri(), "tok");

        create_via_client(&pool, cal, "cal@x.com", "UTC", fields, "none", &client)
            .await
            .unwrap();
    }

    /// An **empty** list sends no `attendees` key at all.
    ///
    /// A form that always submits its guest list submits an empty one for an
    /// event with nobody on it, and on a create the two possible readings —
    /// no key, or `attendees: []` — produce the same event. Absent is the
    /// smaller claim, and pinning it here is what stops a later `if let Some`
    /// quietly starting to send `"attendees": []` on every ordinary create.
    ///
    /// `body_json` matches the whole document, so the *absence* is asserted
    /// rather than merely not-checked: an extra key fails the match and the
    /// mock's `expect(1)` goes unmet.
    #[tokio::test]
    async fn creating_an_event_with_an_empty_guest_list_sends_no_attendees() {
        let fields = crate::write::EventFields { guests: Some(vec![]), ..sample_fields() };
        let (start, end) = crate::write::when_json(&fields.when, &fields.tz);
        let expected_body = serde_json::json!({
            "start": start,
            "end":   end,
            "summary": "Lunch",
            "recurrence": ["RRULE:FREQ=WEEKLY"],
            "reminders": { "useDefault": false,
                           "overrides": [{ "method": "popup", "minutes": 10 }] },
        });

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .and(wiremock::matchers::body_json(expected_body))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "g-new", "status": "confirmed", "etag": "\"e1\"",
                "summary": "Lunch",
                "start": {"dateTime": "2026-08-10T12:00:00+03:00"},
                "end":   {"dateTime": "2026-08-10T13:00:00+03:00"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let pool = omacal_store::connect_memory().await.unwrap();
        let cal = seed_calendar(&pool, "owner").await;
        let client = omacal_google::CalendarClient::new(server.uri(), "tok");

        create_via_client(&pool, cal, "cal@x.com", "UTC", fields, "none", &client)
            .await
            .unwrap();
    }

    /// **What `sendUpdates` actually reaches Google on a create**, asserted on
    /// the wire for both values.
    ///
    /// Both, deliberately. A create that ignored its argument and always sent
    /// `"none"` — which is exactly what this path did until guests could be
    /// invited — passes the `"none"` half on its own, and the `"all"` half is
    /// the only one that can mail anybody. Guest-list spec §7: the
    /// don't-notify path must be witnessed, not assumed, and so must the other.
    #[tokio::test]
    async fn a_create_sends_the_send_updates_it_was_given() {
        for send_updates in ["all", "none"] {
            let server = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
                .and(wiremock::matchers::query_param("sendUpdates", send_updates))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({
                        "id": "g-new", "status": "confirmed", "etag": "\"e1\"",
                        "summary": "Lunch",
                        "start": {"dateTime": "2026-08-10T12:00:00+03:00"},
                        "end":   {"dateTime": "2026-08-10T13:00:00+03:00"}
                    }),
                ))
                .expect(1)
                .mount(&server)
                .await;

            let pool = omacal_store::connect_memory().await.unwrap();
            let cal = seed_calendar(&pool, "owner").await;
            let client = omacal_google::CalendarClient::new(server.uri(), "tok");
            let fields = crate::write::EventFields {
                guests: Some(vec![crate::write::Guest {
                    email: "dan@x.com".into(),
                    optional: false,
                }]),
                ..sample_fields()
            };

            create_via_client(&pool, cal, "cal@x.com", "UTC", fields, send_updates, &client)
                .await
                .unwrap_or_else(|e| panic!("{send_updates}: {e}"));
        }
    }

    /// The end-to-end write-back: `create_via_client` posts to Google, then
    /// stores the response through `omacal_sync::to_stored` — the same
    /// mapping a regular sync uses — via `upsert_event`, and returns the
    /// local row id it landed on.
    ///
    /// The mock binds both the destination (`path`) and the payload
    /// (`body_json`, matched as the whole document) — not just that *a* POST
    /// happened. Without both, three separate mistakes all pass 295/295:
    /// posting to the wrong calendar id, silently dropping `recurrence` from
    /// the body, and swapping `start`/`end`. `body_json` compares the whole
    /// document, so it also tells "recurrence absent" from "recurrence
    /// present and null" for free — `body["recurrence"].is_null()` alone
    /// cannot, since `Value`'s `Index` returns `Null` for a missing key too.
    #[tokio::test]
    async fn a_created_event_is_stored_locally() {
        let fields = sample_fields();
        let (start, end) = crate::write::when_json(&fields.when, &fields.tz);
        let expected_body = serde_json::json!({
            "start": start,
            "end":   end,
            "summary": "Lunch",
            "recurrence": ["RRULE:FREQ=WEEKLY"],
            "reminders": { "useDefault": false,
                           "overrides": [{ "method": "popup", "minutes": 10 }] },
        });

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .and(wiremock::matchers::body_json(expected_body))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "g-new", "status": "confirmed", "etag": "\"e1\"",
                "summary": "Lunch",
                "start": {"dateTime": "2026-08-10T12:00:00+03:00"},
                "end":   {"dateTime": "2026-08-10T13:00:00+03:00"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let pool = omacal_store::connect_memory().await.unwrap();
        let cal = seed_calendar(&pool, "owner").await;
        let client = omacal_google::CalendarClient::new(server.uri(), "tok");

        let id = create_via_client(&pool, cal, "cal@x.com", "UTC", fields, "none", &client)
            .await
            .unwrap();

        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.google_id, "g-new");
        assert_eq!(row.calendar_id, cal, "the row must land on the calendar that was asked for");
    }

    /// The duplicate-create guard. Google answers 200 — the event exists,
    /// the guests are mailed — but the response cannot be mapped for storage
    /// (no start/end). The error must be `CREATED_NOT_STORED`, the one
    /// sentence that does not read as "try again": any other message here is
    /// an invitation to mail the whole list a second time.
    #[tokio::test]
    async fn a_create_that_reached_google_never_reads_as_a_failed_create() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "g-half", "status": "confirmed", "etag": "\"e1\"",
                "summary": "Lunch"
                // no start/end: `to_stored` cannot map this
            })))
            .expect(1)
            .mount(&server)
            .await;

        let pool = omacal_store::connect_memory().await.unwrap();
        let cal = seed_calendar(&pool, "owner").await;
        let client = omacal_google::CalendarClient::new(server.uri(), "tok");

        let err = create_via_client(&pool, cal, "cal@x.com", "UTC", sample_fields(), "all", &client)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), CREATED_NOT_STORED,
            "past the insert, every failure must wear the created-not-stored sentence");
    }

    /// The boundary from the other side: a failure *before* the insert — here
    /// Google refusing it outright — is a genuinely failed create, retrying
    /// is safe, and the guard's sentence must not appear.
    #[tokio::test]
    async fn a_create_google_refused_is_still_a_plain_failure() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let pool = omacal_store::connect_memory().await.unwrap();
        let cal = seed_calendar(&pool, "owner").await;
        let client = omacal_google::CalendarClient::new(server.uri(), "tok");

        let err = create_via_client(&pool, cal, "cal@x.com", "UTC", sample_fields(), "all", &client)
            .await
            .unwrap_err();
        assert_ne!(err.to_string(), CREATED_NOT_STORED,
            "nothing was created, so nothing may claim it was");
    }

    /// All-day dates carry no timezone of their own on Google's wire format
    /// (a bare `{"date": "..."}` , no `timeZone`) — see `create_via_client`'s
    /// own doc comment. `resolve` (in `omacal-sync`) therefore always falls
    /// back to whatever `cal_tz` it is handed, and sync always passes
    /// `calendars.timezone`. This pins that `create_via_client` does too:
    /// authored in `America/New_York`, stored on a calendar whose own zone is
    /// `Pacific/Auckland`, the row must land where the calendar's zone puts
    /// it — not where the authoring zone would have.
    #[tokio::test]
    async fn an_all_day_create_resolves_against_the_calendars_own_timezone_not_the_authoring_one() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            // The other half of the same property, and only bindable since the
            // form began sending dates: the body carries the date the user
            // picked, with no `timeZone` and no trace of the authoring zone.
            // Without this the test could only ever speak for `to_stored`.
            .and(wiremock::matchers::body_json(serde_json::json!({
                "start": {"date": "2026-08-10"},
                "end":   {"date": "2026-08-11"},
                "summary": "Lunch",
                "recurrence": ["RRULE:FREQ=WEEKLY"],
                "reminders": { "useDefault": false,
                               "overrides": [{ "method": "popup", "minutes": 10 }] },
            })))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "g-allday", "status": "confirmed", "etag": "\"e1\"",
                "start": {"date": "2026-08-10"},
                "end":   {"date": "2026-08-11"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let pool = omacal_store::connect_memory().await.unwrap();
        let cal = seed_calendar_with_tz(&pool, "owner", "Pacific/Auckland").await;
        let client = omacal_google::CalendarClient::new(server.uri(), "tok");

        let mut fields = sample_fields();
        fields.when = crate::write::When::AllDay {
            start_date: "2026-08-10".into(),
            end_date: "2026-08-11".into(),
        };
        fields.tz = "America/New_York".into(); // the authoring zone — must be ignored

        let id =
            create_via_client(&pool, cal, "cal@x.com", "Pacific/Auckland", fields, "none", &client)
                .await
                .unwrap();

        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        let expected_start_utc = "2026-08-10"
            .parse::<jiff::civil::Date>()
            .unwrap()
            .to_datetime(jiff::civil::Time::midnight())
            .in_tz("Pacific/Auckland")
            .unwrap()
            .timestamp()
            .as_millisecond();
        assert_eq!(
            row.start_utc, expected_start_utc,
            "an all-day create must resolve against the calendar's own timezone, not the \
             authoring one — otherwise it lands at a different instant than the next sync \
             would recompute it at"
        );
    }

    // --- update_event: `update_impl` / `update_via_client`. The dangerous
    // pair. An occurrence in the grid is derived, not a row (spec §1), so
    // "just this one" has to resolve to Google's own instance id before it
    // patches anything — and unlike an RSVP, the payload here is the event
    // itself rather than one enum, still with `sendUpdates=all` behind it.

    const HOUR: i64 = 3_600_000;

    /// 2026-07-27T09:00:00Z, a Monday: the series' own DTSTART, which is what
    /// the master's stored row carries.
    const DTSTART: i64 = 1_785_142_800_000;

    /// 2026-08-10T09:00:00Z — the occurrence two weeks later, i.e. the block
    /// the user actually clicked. Deliberately not occurrence #0: every
    /// assertion below about which instant a body carries can then tell the
    /// clicked occurrence from the series start, which a fixture sitting on
    /// the master's own start could not.
    const OCCURRENCE: i64 = 1_786_352_400_000;

    /// A weekly series master as this store holds it: one row whose
    /// `start_utc` is the series DTSTART, shared by every occurrence the grid
    /// expands out of it.
    fn weekly_master(rule: &str) -> omacal_store::StoredEvent {
        let mut ev = stored(vec![]);
        ev.google_id = "master1".into();
        ev.summary = Some("Standup".into());
        ev.recurrence = Some(rule.into());
        ev.start_utc = DTSTART;
        ev.end_utc = DTSTART + HOUR;
        ev.etag = Some("\"master-etag\"".into());
        ev
    }

    /// `seeded_pool_with`, but on `seed_calendar_with_tz`'s `cal@x.com`
    /// calendar and with `ev.calendar_id` set to the row that was actually
    /// inserted rather than `stored`'s hardcoded `1` — so nothing here passes
    /// by coincidence. `cal@x.com` also exercises the path encoding a bare
    /// `primary` cannot, and these tests assert on request paths.
    async fn seeded_pool_on_cal(
        ev: &mut omacal_store::StoredEvent,
        cal_tz: &str,
    ) -> (SqlitePool, i64) {
        let pool = omacal_store::connect_memory().await.unwrap();
        let cal = seed_calendar_with_tz(&pool, "owner", cal_tz).await;
        ev.calendar_id = cal;
        let id = omacal_store::upsert_event(&pool, ev).await.unwrap();
        (pool, id)
    }

    /// [`seeded_pool_on_cal`]'s fixture on a **`reader`** calendar, for the
    /// tests whose whole subject is a gate sitting *above* the writability
    /// check.
    ///
    /// Not a stylistic preference, and not about read-only calendars at all. On
    /// an `owner` calendar the only thing between such a test and
    /// `crate::load_config()` — the real `~/.config/omacal/config.toml`, then
    /// the real Keychain — is the single gate it exists to test. That is exactly
    /// the shape that put a Task 6 test on the wrong side of a guard the moment
    /// Task 7 implemented the scope it had used as its stand-in for
    /// "unimplemented"; see
    /// [`an_unimplemented_scope_is_refused_rather_than_treated_as_this_occurrence`].
    ///
    /// A `reader` calendar puts a second gate underneath the first. The
    /// assertion stays just as discriminating — remove the gate under test and
    /// the message is wrong either way — but a future task that implements the
    /// scope makes these fail **loudly at the writability check** rather than
    /// falling through to a credential no test may touch.
    async fn seeded_pool_on_read_only_cal(
        ev: &mut omacal_store::StoredEvent,
    ) -> (SqlitePool, i64) {
        let pool = omacal_store::connect_memory().await.unwrap();
        let cal = seed_calendar_with_tz(&pool, "reader", "UTC").await;
        ev.calendar_id = cal;
        let id = omacal_store::upsert_event(&pool, ev).await.unwrap();
        (pool, id)
    }

    /// What the form sends back: the fields it was pre-filled from, with the
    /// user's change applied. Two things are deliberate.
    ///
    /// `tz` is the machine's zone and never the fixture event's own — editing
    /// a New York meeting from a Sofia laptop must not re-zone the meeting.
    /// `recurrence: None` is "the user did not touch Repeat", the state spec
    /// §6 turns on.
    fn form(summary: &str, start_ms: i64, end_ms: i64) -> crate::write::EventFields {
        crate::write::EventFields {
            summary: Some(summary.into()),
            location: None,
            description: None,
            when: crate::write::When::Timed { start_ms, end_ms },
            tz: "Europe/Sofia".into(),
            recurrence: None,
            // The guest list was not touched. Every edit test that predates
            // guest editing says so through this field, which is what keeps
            // them assertions about the fields they are actually named for.
            guests: None,
            // Likewise untouched, for the same reason.
            reminders: None,
        }
    }

    /// [`form`]'s all-day sibling: the dates the form now sends instead of a
    /// pair of instants. `end` is **exclusive**, the day after the last one, as
    /// on the wire and in the store.
    ///
    /// It keeps `form`'s `tz` — the machine's, not the calendar's — on purpose.
    /// An all-day event has no zone, and every assertion that passes with a
    /// foreign one sitting here is one more thing proving the dates never
    /// consult it.
    fn all_day_form(summary: &str, start: &str, end: &str) -> crate::write::EventFields {
        crate::write::EventFields {
            when: crate::write::When::AllDay {
                start_date: start.into(),
                end_date: end.into(),
            },
            ..form(summary, 0, 0)
        }
    }

    /// The wire shape of one expanded occurrence. It carries times because it
    /// has to: the instance is the resource being patched, so its own start,
    /// end and etag are what the request is built against — `to_stored`
    /// returns `None` for an event whose times will not parse.
    fn wire_occurrence(id: &str, start_ms: i64, end_ms: i64, etag: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id, "status": "confirmed", "etag": etag,
            "start": {"dateTime": omacal_sync::to_rfc3339(start_ms)},
            "end":   {"dateTime": omacal_sync::to_rfc3339(end_ms)},
        })
    }

    /// Every request the server saw, for the assertions that are about what
    /// was *not* sent. `.expect(n)` can only speak for requests that matched a
    /// mock; an unmatched one is answered with a bare 404 and is otherwise
    /// invisible.
    async fn requests(server: &wiremock::MockServer) -> Vec<wiremock::Request> {
        server.received_requests().await.expect("request recording is on by default")
    }

    /// The same list as `METHOD /path`, for assertions about requests that
    /// should never have been sent at all.
    ///
    /// `wiremock::Request`'s own `Debug` renders the body as a `Vec<u8>`, so a
    /// message naming the offending request buries it behind several hundred
    /// integers — on the two refusal tests, precisely where the reader needs to
    /// see "POST /calendars/…/events" and nothing else.
    fn methods_and_paths(sent: &[wiremock::Request]) -> Vec<String> {
        sent.iter().map(|r| format!("{} {}", r.method.as_str(), r.url.path())).collect()
    }

    /// The defect this whole design guards against. "This one" must patch the
    /// instance id Google returns, never the master's — a master patch with
    /// `sendUpdates=all` rewrites every occurrence of the series and mails the
    /// change to the entire guest list.
    #[tokio::test]
    async fn editing_one_occurrence_patches_the_instance_not_the_master() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        // Bracketed by the *clicked* occurrence, never by `ev.start_utc`: the
        // query params are matched, so a window derived from the master's own
        // start does not match this mock and the call 404s.
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1/instances"))
            .and(wiremock::matchers::query_param("timeMin", omacal_sync::to_rfc3339(OCCURRENCE)))
            .and(wiremock::matchers::query_param(
                "timeMax",
                omacal_sync::to_rfc3339(OCCURRENCE + 1000),
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [wire_occurrence(
                    "master1_20260810T090000Z", OCCURRENCE, OCCURRENCE + HOUR, "\"i1\"")]
            })))
            .expect(1)
            .mount(&server)
            .await;

        // The instance's own id *and* the instance's own etag: `"master-etag"`
        // is the version of a different resource, and the body is the whole
        // document so a stray `start`, `end` or `recurrence` fails here too.
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1_20260810T090000Z"))
            .and(wiremock::matchers::header("if-match", "\"i1\""))
            .and(wiremock::matchers::body_json(
                serde_json::json!({"summary": "Standup (moved)"}),
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(wire_occurrence(
                "master1_20260810T090000Z",
                OCCURRENCE,
                OCCURRENCE + HOUR,
                "\"i2\"",
            )))
            .expect(1)
            .mount(&server)
            .await;
        // No PATCH on `master1` is mounted: one arriving there is a 404, and
        // the `unwrap()` below fails rather than the test passing quietly.

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "this",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (moved)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();

        // The instance is a different Google resource than the row that was
        // loaded, so nothing may be folded back onto that row — the same rule
        // `respond_via_client` follows, and here it would put one occurrence's
        // new title on the whole series.
        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(
            row.etag.as_deref(),
            Some("\"master-etag\""),
            "the instance's etag was stamped onto the master's own row"
        );
        assert_eq!(
            row.summary.as_deref(),
            Some("Standup"),
            "one occurrence's new title was written onto the series' row"
        );
    }

    /// An occurrence that resolves to nothing must fail loudly. Plan 2's
    /// original fallback silently widened "this one" into "all of them";
    /// here that means sending the edited event to every occurrence in the
    /// series and telling the guest list about it.
    #[tokio::test]
    async fn an_unresolvable_occurrence_is_an_error_not_a_master_patch() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1/instances"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"items": []})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        let err = update_via_client(
            &pool,
            "this",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (moved)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("could not find that occurrence"), "{err}");

        // Read off the server rather than inferred from the `Err`: a fallback
        // to the master would 404 (no PATCH mock is mounted), and a 404 is an
        // `Err` too — indistinguishable from this one without looking.
        assert!(
            requests(&server).await.iter().all(|r| r.method.as_str() != "PATCH"),
            "an unresolvable occurrence sent a PATCH anyway"
        );
    }

    /// Scope `"all"` is one request, to the master, with no instance lookup —
    /// and, since that master *is* the row this store holds, the response
    /// folds back into it.
    #[tokio::test]
    async fn editing_all_events_patches_the_master() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .and(wiremock::matchers::header("if-match", "\"master-etag\""))
            .and(wiremock::matchers::body_json(
                serde_json::json!({"summary": "Standup (moved)"}),
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"master-2\"", "sequence": 4,
                "summary": "Standup (moved)",
                "recurrence": ["RRULE:FREQ=WEEKLY"],
                "start": {"dateTime": omacal_sync::to_rfc3339(DTSTART)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(DTSTART + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "all",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (moved)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();

        assert!(
            requests(&server).await.iter().all(|r| !r.url.path().ends_with("/instances")),
            "scope \"all\" resolved an instance: \"this one\" and \"all of them\" must not converge"
        );

        // Same Google resource as the row that was loaded, so the patch
        // response is folded back in — otherwise the popover shows the old
        // title until the next sync.
        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.summary.as_deref(), Some("Standup (moved)"), "the write-back did not happen");
        assert_eq!(row.etag.as_deref(), Some("\"master-2\""));
        assert_eq!(row.sequence, 4);
        assert_eq!(
            row.start_utc, DTSTART,
            "the series start moved on a title-only edit"
        );
    }

    /// Scope `"all"` from an already-moved exception row, in its *ordinary*
    /// shape: the row's own start is the occurrence the user clicked. The
    /// target is the master — a different Google resource, with its own
    /// DTSTART two weeks earlier — while the form was pre-filled from the
    /// moved occurrence. A title-only edit must carry no `start` at all;
    /// carrying one drags the series' DTSTART onto the exception's date and
    /// drops every occurrence before it, with `sendUpdates=all` behind it.
    ///
    /// The realistic configuration of the branch, and the regression test for
    /// it. It is deliberately *not* the test that binds the choice of
    /// `is_recurring` over `recurrence.is_some()`: with the anchor's other arm
    /// reading `ev.start_utc`, the two agree whenever the row's start and the
    /// clicked occurrence coincide, which is precisely this fixture. Verified
    /// by running it under that mutation — it passes. The sibling test
    /// `editing_all_events_from_an_exception_row_asks_the_master_and_anchors_on_the_click`
    /// separates the two values and is the one that fails there; deleting it
    /// as a duplicate of this would leave that branch unbound.
    #[tokio::test]
    async fn editing_all_events_from_a_moved_occurrence_leaves_the_series_start_alone() {
        let mut ev = stored(vec![]);
        ev.google_id = "master1_20260810T090000Z".into();
        ev.summary = Some("Standup".into());
        ev.recurring_event_id = Some("master1".into());
        ev.start_utc = OCCURRENCE + 5 * HOUR; // this occurrence was moved
        ev.end_utc = OCCURRENCE + 6 * HOUR;
        ev.etag = Some("\"exc\"".into());
        let occ = ev.start_utc;
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"m1\"",
                "summary": "Standup",
                "recurrence": ["RRULE:FREQ=WEEKLY"],
                "start": {"dateTime": omacal_sync::to_rfc3339(DTSTART)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(DTSTART + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"m2\"",
                "summary": "Standup (moved)",
                "recurrence": ["RRULE:FREQ=WEEKLY"],
                "start": {"dateTime": omacal_sync::to_rfc3339(DTSTART)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(DTSTART + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "all",
            occ,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (moved)", occ, occ + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();

        let sent = requests(&server).await;
        let patch = sent.iter().find(|r| r.method.as_str() == "PATCH").unwrap();
        let body: serde_json::Value = serde_json::from_slice(&patch.body).unwrap();
        assert!(
            body.get("start").is_none(),
            "a title-only edit moved the series' DTSTART: {body}"
        );
        assert_eq!(
            patch.headers.get("if-match").unwrap(),
            "\"m1\"",
            "the exception's etag was sent as the master's version"
        );
    }

    /// Spec §6 end to end, not only in the pure builder. The Repeat dropdown
    /// cannot express "the last Friday of the month", so a save that carried
    /// `recurrence` would quietly rewrite this series into something simpler
    /// and the user would have no way to know.
    #[tokio::test]
    async fn editing_a_title_never_sends_recurrence() {
        let mut ev = weekly_master("RRULE:FREQ=MONTHLY;BYDAY=-1FR");
        ev.summary = Some("Retro".into());
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        // Matched as a whole document, so this also fails on a `start` the
        // user never moved, not only on a wrong value.
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .and(wiremock::matchers::body_json(serde_json::json!({"summary": "Retro (moved)"})))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"master-2\"",
                "summary": "Retro (moved)",
                "recurrence": ["RRULE:FREQ=MONTHLY;BYDAY=-1FR"],
                "start": {"dateTime": omacal_sync::to_rfc3339(DTSTART)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(DTSTART + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "all",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Retro (moved)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();

        // Named directly as well, so a regression reads as "recurrence was
        // sent" rather than as a 404. `.get()`, not `body["recurrence"]`:
        // `Value`'s `Index` answers `Null` for a missing key too, which is how
        // a safety-critical arm shipped unguarded earlier on this branch.
        let sent = requests(&server).await;
        let body: serde_json::Value = serde_json::from_slice(&sent[0].body).unwrap();
        assert!(body.get("recurrence").is_none(), "recurrence was sent: {body}");
    }

    /// Two rules at once, each of which moves a real meeting if it is wrong.
    ///
    /// The form's instants are the *clicked occurrence's*, and the master is
    /// anchored two weeks earlier. Sending the occurrence's absolute start to
    /// the master would drag the series' DTSTART forward to that date and take
    /// every earlier occurrence with it, so a time change reaches the target
    /// as the shift the user made, applied to the target's own start.
    ///
    /// The zone is the event's own stored one, not the machine's: the instant
    /// is carried by the epoch milliseconds, and `timeZone` only says which
    /// zone the event is displayed in. A New York meeting edited from a Sofia
    /// laptop must stay a New York meeting.
    #[tokio::test]
    async fn changing_the_time_for_all_events_shifts_the_series_start_and_keeps_the_events_own_zone()
    {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        ev.start_tz = "America/New_York".into();
        ev.end_tz = "America/New_York".into();
        // The calendar's own zone is a third, different one, so a body built
        // from it fails here too.
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "Pacific/Auckland").await;

        let (start, end) = timed_json(DTSTART + HOUR, DTSTART + 2 * HOUR, "America/New_York");
        let expected = serde_json::json!({ "start": start, "end": end });

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .and(wiremock::matchers::body_json(expected))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"master-2\"",
                "summary": "Standup",
                "recurrence": ["RRULE:FREQ=WEEKLY"],
                "start": {"dateTime": omacal_sync::to_rfc3339(DTSTART + HOUR),
                          "timeZone": "America/New_York"},
                "end":   {"dateTime": omacal_sync::to_rfc3339(DTSTART + 2 * HOUR),
                          "timeZone": "America/New_York"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        // The user dragged *this occurrence* an hour later and chose "all
        // events"; the title is untouched.
        update_via_client(
            &pool,
            "all",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "Pacific/Auckland",
            form("Standup", OCCURRENCE + HOUR, OCCURRENCE + 2 * HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();
    }

    /// The conflict path, and the shape of the retry. Somebody renamed the
    /// event between the form opening and the save; the user changed only the
    /// location. Re-deriving "what changed" against the freshly-read copy
    /// would make the stale title look like an edit and send it — putting
    /// their rename back, with `sendUpdates=all` behind it. The retry carries
    /// the same one field the user actually changed, against the fresh etag.
    #[tokio::test]
    async fn a_stale_etag_retries_once_against_the_fresh_version_without_reverting_the_other_change()
    {
        let mut ev = stored(vec![]);
        ev.summary = Some("Lunch".into());
        ev.location = Some("Room 4A".into());
        ev.start_utc = OCCURRENCE;
        ev.end_utc = OCCURRENCE + HOUR;
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await; // google_id "ev1", etag "old"

        let mut after = form("Lunch", OCCURRENCE, OCCURRENCE + HOUR);
        after.location = Some("Room 5".into());

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/ev1"))
            .and(wiremock::matchers::header("if-match", "\"old\""))
            .respond_with(wiremock::ResponseTemplate::new(412))
            .expect(1)
            .mount(&server)
            .await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/ev1"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ev1", "status": "confirmed", "etag": "\"fresh\"",
                "summary": "Lunch with Ana",
                "location": "Room 4A",
                "start": {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/ev1"))
            .and(wiremock::matchers::header("if-match", "\"fresh\""))
            .and(wiremock::matchers::body_json(serde_json::json!({"location": "Room 5"})))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ev1", "status": "confirmed", "etag": "\"e3\"",
                "summary": "Lunch with Ana",
                "location": "Room 5",
                "start": {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(&pool, "all", OCCURRENCE, ev, "cal@x.com", "UTC", after, "all", &client)
            .await
            .unwrap();

        // Both halves of the outcome: our change landed, and theirs survived.
        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.location.as_deref(), Some("Room 5"));
        assert_eq!(row.summary.as_deref(), Some("Lunch with Ana"));
    }

    /// **What `sendUpdates` actually reaches Google**, asserted on the wire
    /// rather than on an argument: an internal assertion passes happily while
    /// the request carries something else.
    ///
    /// Both values, because they are opposite instructions. `"all"` is the
    /// form's — a time typed on purpose deserves to be told about — and
    /// `"none"` is a drag's, because a gesture can happen by accident and a
    /// slip of the mouse must not mail a guest list (drag spec §2).
    ///
    /// `query_param` **and** `.expect(1)`: the matcher says what a matching
    /// request looks like, and `.expect(1)` is what insists one happened. A
    /// request carrying the other value matches no mock, and wiremock's
    /// unmatched-request 404 would come back through a path that does not
    /// distinguish it from a transport failure.
    #[tokio::test]
    async fn a_move_sends_the_send_updates_it_was_given() {
        for send_updates in ["all", "none"] {
            let mut ev = stored(vec![guest(true)]);
            ev.summary = Some("Standup".into());
            ev.start_utc = OCCURRENCE;
            ev.end_utc = OCCURRENCE + HOUR;
            let (pool, _id) = seeded_pool_on_cal(&mut ev.clone(), "UTC").await;

            let server = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::method("PATCH"))
                .and(wiremock::matchers::path("/calendars/cal%40x.com/events/ev1"))
                .and(wiremock::matchers::query_param("sendUpdates", send_updates))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({
                        "id": "ev1", "status": "confirmed", "etag": "\"e2\"",
                        "summary": "Standup",
                        "start": {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE + HOUR)},
                        "end":   {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE + 2 * HOUR)}
                    }),
                ))
                .expect(1)
                .mount(&server)
                .await;

            // An hour later: a move, which is the only thing a drag changes.
            let after = form("Standup", OCCURRENCE + HOUR, OCCURRENCE + 2 * HOUR);
            let client = omacal_google::CalendarClient::new(server.uri(), "tok");
            update_via_client(
                &pool,
                "all",
                OCCURRENCE,
                ev,
                "cal@x.com",
                "UTC",
                after,
                send_updates,
                &client,
            )
            .await
            .unwrap();
            // `.expect(1)` is checked when the server drops at the end of this
            // iteration; the `unwrap` above catches a 404 first.
        }
    }

    /// A save with nothing changed must not become a request at all. On the
    /// form's path every PATCH carries `sendUpdates=all`, so an empty edit
    /// would still mail the guest list about a change nobody made.
    #[tokio::test]
    async fn an_edit_that_changes_nothing_sends_no_request() {
        let mut ev = stored(vec![]);
        ev.summary = Some("Lunch".into());
        ev.start_utc = OCCURRENCE;
        ev.end_utc = OCCURRENCE + HOUR;
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        // Nothing is mounted, so any request is a 404 — but the assertion
        // below does not rely on that either.
        let server = wiremock::MockServer::start().await;
        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "all",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Lunch", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();

        assert!(
            requests(&server).await.is_empty(),
            "a save that changed nothing still went to Google"
        );
    }

    /// [`form`] with a guest list attached — what a save from a form that
    /// *does* offer guest editing sends.
    fn form_with_guests(
        summary: &str,
        start_ms: i64,
        end_ms: i64,
        guests: Vec<crate::write::Guest>,
    ) -> crate::write::EventFields {
        crate::write::EventFields { guests: Some(guests), ..form(summary, start_ms, end_ms) }
    }

    /// **Spec §7, on the wire: the assertion the whole design exists for.**
    ///
    /// Three attendees, one of them `accepted`, gains a fourth — and the
    /// request must carry the first three with their own `responseStatus`
    /// intact. Asserted with `body_json` against the **whole document**, not
    /// with a matcher on part of it: a partial matcher passes just as happily
    /// when the body also carries a `summary` nobody typed, and the failure
    /// mode here is a field that should not be in the body at all.
    ///
    /// Nothing else changed, so `attendees` is the only key. That is itself
    /// load bearing — a body that also resent the title would prove the diff
    /// discipline had been abandoned along with the echo-back.
    #[tokio::test]
    async fn adding_a_guest_resends_the_whole_list_with_every_answer_intact() {
        let mut ev = stored(three());
        ev.summary = Some("Lunch".into());
        ev.start_utc = OCCURRENCE;
        ev.end_utc = OCCURRENCE + HOUR;
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let expected = serde_json::json!({
            "attendees": [
                { "email": "ana@x.com", "responseStatus": "accepted", "optional": false,
                  "additionalGuests": 1, "displayName": "Ana", "comment": "running 5 late" },
                { "email": "me@x.com", "responseStatus": "needsAction", "optional": false,
                  "additionalGuests": 0 },
                { "email": "petya@x.com", "responseStatus": "declined", "optional": true,
                  "additionalGuests": 2 },
                { "email": "dan@x.com", "responseStatus": "needsAction", "optional": false,
                  "additionalGuests": 0 },
            ]
        });

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/ev1"))
            .and(wiremock::matchers::body_json(expected))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ev1", "status": "confirmed", "etag": "\"e2\"",
                "summary": "Lunch",
                "start": {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let after = form_with_guests(
            "Lunch",
            OCCURRENCE,
            OCCURRENCE + HOUR,
            guests_plus(&three(), "dan@x.com"),
        );
        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(&pool, "all", OCCURRENCE, ev, "cal@x.com", "UTC", after, "none", &client)
            .await
            .unwrap();
    }

    /// A whole-list replace built from a stale read un-invites whoever was
    /// added elsewhere since, so the write must say **which version it was
    /// built from** (spec §2). The etag path already exists; this is what
    /// stops a guest change being the one write that forgets to use it.
    #[tokio::test]
    async fn a_guest_change_is_conditioned_on_the_version_it_was_built_from() {
        let mut ev = stored(three()); // etag "old"
        ev.summary = Some("Lunch".into());
        ev.start_utc = OCCURRENCE;
        ev.end_utc = OCCURRENCE + HOUR;
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/ev1"))
            .and(wiremock::matchers::header("if-match", "\"old\""))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ev1", "status": "confirmed", "etag": "\"e2\"",
                "summary": "Lunch",
                "start": {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let after = form_with_guests(
            "Lunch",
            OCCURRENCE,
            OCCURRENCE + HOUR,
            guests_without(&three(), "petya@x.com"),
        );
        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(&pool, "all", OCCURRENCE, ev, "cal@x.com", "UTC", after, "none", &client)
            .await
            .unwrap();
    }

    /// **A 412 on a guest-list change is reported, never retried.**
    ///
    /// Every other field in an edit body is a diff, so rebuilding it against a
    /// fresh read leaves the other person's change standing. `attendees` is not
    /// a diff — it is the whole list as omacal last read it, and a 412 is
    /// Google saying that reading is out of date. A retry would send the stale
    /// list over the current one and silently un-invite anyone added since.
    ///
    /// Witnessed by the **absence of a second PATCH**, and by no GET at all: a
    /// version that re-read the event in order to retry would leave a GET in
    /// the log even if the second write then failed for another reason.
    #[tokio::test]
    async fn a_guest_list_conflict_is_reported_rather_than_retried_over_the_current_list() {
        let mut ev = stored(three());
        ev.summary = Some("Lunch".into());
        ev.start_utc = OCCURRENCE;
        ev.end_utc = OCCURRENCE + HOUR;
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/ev1"))
            .respond_with(wiremock::ResponseTemplate::new(412))
            .expect(1)
            .mount(&server)
            .await;

        let after = form_with_guests(
            "Lunch",
            OCCURRENCE,
            OCCURRENCE + HOUR,
            guests_without(&three(), "petya@x.com"),
        );
        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        let err = update_via_client(
            &pool, "all", OCCURRENCE, ev, "cal@x.com", "UTC", after, "none", &client,
        )
        .await
        .expect_err("a lost race on a guest list must not be swallowed");

        assert_eq!(err.to_string(), CONFLICT_GUESTS);
        assert_eq!(
            methods_and_paths(&requests(&server).await),
            vec!["PATCH /calendars/cal%40x.com/events/ev1"],
            "the stale guest list was sent a second time, or re-read in order to be"
        );
    }

    /// And the other half of that rule, so it cannot be satisfied by refusing
    /// to retry *anything*: an edit that leaves the guest list alone still gets
    /// its retry, and the other person's change still survives it. The two
    /// arms are one `if` apart, and only a pair of tests can say the `if` is
    /// there.
    #[tokio::test]
    async fn a_conflict_on_an_edit_that_touches_no_guests_still_retries() {
        let mut ev = stored(three());
        ev.summary = Some("Lunch".into());
        ev.location = Some("Room 4A".into());
        ev.start_utc = OCCURRENCE;
        ev.end_utc = OCCURRENCE + HOUR;
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::header("if-match", "\"old\""))
            .respond_with(wiremock::ResponseTemplate::new(412))
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/ev1"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ev1", "status": "confirmed", "etag": "\"fresh\"",
                "summary": "Lunch with Ana", "location": "Room 4A",
                "start": {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::header("if-match", "\"fresh\""))
            .and(wiremock::matchers::body_json(serde_json::json!({"location": "Room 5"})))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ev1", "status": "confirmed", "etag": "\"e3\"",
                "summary": "Lunch with Ana", "location": "Room 5",
                "start": {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        // The guest list is handed over **unchanged** rather than omitted: the
        // form always sends one, so "no guest change" has to mean "the same
        // list", not "the field was absent". A rule keyed on `guests.is_some()`
        // rather than on the body would fail here, which is the point.
        let after =
            form_with_guests("Lunch", OCCURRENCE, OCCURRENCE + HOUR, guests_of(&three()));
        let after = crate::write::EventFields { location: Some("Room 5".into()), ..after };
        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(&pool, "all", OCCURRENCE, ev, "cal@x.com", "UTC", after, "all", &client)
            .await
            .unwrap();

        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.location.as_deref(), Some("Room 5"));
        assert_eq!(row.summary.as_deref(), Some("Lunch with Ana"), "their change was reverted");
    }

    /// A guest list the user did not touch sends **no `attendees` at all**.
    ///
    /// Absent means "leave it alone", which is the only safe instruction for a
    /// whole-list replace built from a possibly-stale read. A form that resent
    /// the list on every save would rewrite the attendees of every event it
    /// ever touched, from whatever omacal happened to hold.
    #[test]
    fn a_guest_list_that_did_not_change_sends_no_attendees() {
        let mut ev = stored(three());
        ev.summary = Some("Lunch".into());
        ev.start_utc = OCCURRENCE;
        ev.end_utc = OCCURRENCE + HOUR;

        let after =
            form_with_guests("Lunch", OCCURRENCE, OCCURRENCE + HOUR, guests_of(&three()));
        let body = edit_patch_body(&ev, OCCURRENCE, OCCURRENCE + HOUR, OCCURRENCE, "UTC", &after);
        assert_eq!(body, serde_json::json!({}), "an untouched guest list reached the wire");

        // And a path that offers no guest editing at all says the same thing a
        // different way, which is what keeps a drag structurally unable to
        // rewrite a guest list.
        let untouched = form("Lunch", OCCURRENCE, OCCURRENCE + HOUR);
        assert!(untouched.guests.is_none());
        let body =
            edit_patch_body(&ev, OCCURRENCE, OCCURRENCE + HOUR, OCCURRENCE, "UTC", &untouched);
        assert_eq!(body, serde_json::json!({}));
    }

    /// Reminders the user did not touch send **no `reminders` at all** —
    /// spec §2's absent state, and the guest rule again: for a whole-object
    /// replace built from a possibly-stale read, absence is the only safe
    /// instruction. Both faces of untouched are here: a form that echoed the
    /// stored value back exactly (equality, no key), and a path with no
    /// reminder editing at all (absence, no key).
    #[test]
    fn reminders_that_did_not_change_send_nothing() {
        let mut ev = stored(three());
        ev.summary = Some("Lunch".into());
        ev.start_utc = OCCURRENCE;
        ev.end_utc = OCCURRENCE + HOUR;
        ev.reminders = omacal_store::Reminders {
            use_default: false,
            overrides: vec![omacal_store::Reminder { method: "popup".into(), minutes: 10 }],
        };

        let mut after = form("Lunch", OCCURRENCE, OCCURRENCE + HOUR);
        after.reminders = Some(reminders_as_input(&ev.reminders));
        let body = edit_patch_body(&ev, OCCURRENCE, OCCURRENCE + HOUR, OCCURRENCE, "UTC", &after);
        assert_eq!(body, serde_json::json!({}), "unchanged reminders reached the wire");

        let untouched = form("Lunch", OCCURRENCE, OCCURRENCE + HOUR);
        assert!(untouched.reminders.is_none());
        let body =
            edit_patch_body(&ev, OCCURRENCE, OCCURRENCE + HOUR, OCCURRENCE, "UTC", &untouched);
        assert_eq!(body, serde_json::json!({}));
    }

    /// …and a reminder change alone is a real edit, carrying the **whole**
    /// object — the only shape Google accepts for `reminders` (spec §2).
    #[test]
    fn a_reminder_change_alone_is_a_real_edit() {
        let mut ev = stored(three());
        ev.summary = Some("Lunch".into());
        ev.start_utc = OCCURRENCE;
        ev.end_utc = OCCURRENCE + HOUR;
        ev.reminders = omacal_store::Reminders { use_default: true, overrides: Vec::new() };

        let mut after = form("Lunch", OCCURRENCE, OCCURRENCE + HOUR);
        after.reminders = Some(crate::write::RemindersInput {
            use_default: false,
            overrides: vec![crate::write::ReminderInput { method: "popup".into(), minutes: 15 }],
        });
        let body = edit_patch_body(&ev, OCCURRENCE, OCCURRENCE + HOUR, OCCURRENCE, "UTC", &after);
        assert_eq!(
            body,
            serde_json::json!({ "reminders": { "useDefault": false,
                "overrides": [{ "method": "popup", "minutes": 15 }] } })
        );
    }

    /// …and a guest change on its own is **not** an empty body, so the two
    /// no-op guards do not swallow the one edit this feature exists to make.
    #[test]
    fn a_guest_change_alone_is_a_real_edit() {
        let mut ev = stored(three());
        ev.summary = Some("Lunch".into());
        ev.start_utc = OCCURRENCE;
        ev.end_utc = OCCURRENCE + HOUR;

        let after = form_with_guests(
            "Lunch",
            OCCURRENCE,
            OCCURRENCE + HOUR,
            guests_without(&three(), "petya@x.com"),
        );
        let body = edit_patch_body(&ev, OCCURRENCE, OCCURRENCE + HOUR, OCCURRENCE, "UTC", &after);

        assert_ne!(body, serde_json::json!({}), "a removal was read as a no-op and dropped");
        assert_eq!(body["attendees"].as_array().map(Vec::len), Some(2));
        assert!(body.get("summary").is_none(), "an untouched title rode along with the guests");
    }

    /// The zone rule, as a pure function of the two zones in play. A timed
    /// event keeps its own stored zone on *both* sides of the diff, so the
    /// `tz` term in `changed_fields`' times trigger cannot fire on an edit
    /// made from a machine somewhere else. An all-day event takes the
    /// calendar's, because Google returns all-day events with no `timeZone`
    /// of their own and sync resolves them against `calendars.timezone` — the
    /// same reason `create_via_client` uses it.
    #[test]
    fn a_timed_edit_keeps_the_events_own_zone_and_an_all_day_edit_takes_the_calendars() {
        assert_eq!(edit_zone(false, "Pacific/Auckland", "America/New_York"), "America/New_York");
        assert_eq!(edit_zone(true, "Pacific/Auckland", "America/New_York"), "Pacific/Auckland");
    }

    /// Midnight on `date` in `tz`, as epoch milliseconds — the coordinates an
    /// all-day event actually lives in on either side of the boundary.
    fn midnight_in(date: &str, tz: &str) -> i64 {
        format!("{date}T00:00:00[{tz}]")
            .parse::<jiff::Zoned>()
            .unwrap_or_else(|e| panic!("{date} in {tz}: {e}"))
            .timestamp()
            .as_millisecond()
    }

    /// One all-day row, for the tests either side of the boundary. Stored the
    /// way sync stores one: midnight in the **calendar's** zone, because Google
    /// returns a bare `date` and `omacal_sync::resolve` falls back to
    /// `calendars.timezone`.
    fn all_day_row(start: &str, end: &str, cal_tz: &str) -> omacal_store::StoredEvent {
        let mut ev = stored(vec![]);
        ev.google_id = "allday1".into();
        ev.summary = Some("Berlin trip".into());
        ev.is_all_day = true;
        ev.start_utc = midnight_in(start, cal_tz);
        ev.end_utc = midnight_in(end, cal_tz);
        ev
    }

    /// **The defect this plan closes, now asserted the right way round.** It
    /// was pinned here for eight tasks as
    /// `bug_an_untouched_all_day_date_moves_a_day_when_the_calendar_zone_is_not_the_browsers`,
    /// asserting `2026-08-09` on purpose and naming `2026-08-10` as correct at
    /// every line. Those expectations are inverted below, deliberately, which
    /// is the whole reason it was written that way round.
    ///
    /// **What used to happen:** an all-day event, saved with only its title
    /// changed, moved a day and mailed every guest (`sendUpdates=all`).
    ///
    /// **Why.** An all-day event has no instant — it has a *date*. Both sides
    /// of this boundary turned that date into an instant, in different zones:
    /// the store held midnight in the *calendar's*, the form built midnight in
    /// the *browser's*. A date nobody touched came back as a different instant,
    /// the times trigger fired, and the instant was rendered back to a `date`
    /// in [`edit_zone`]'s zone — the calendar's — landing a day before.
    ///
    /// **What closes it** is that the form no longer sends an instant at all.
    /// `EventInput` carries a `date` for an all-day event, which is Google's
    /// own model, and `crate::write::When` makes the old triple
    /// unrepresentable: there is no zone on the all-day arm to convert in, and
    /// nothing to convert. The two sides now compare *strings*.
    ///
    /// The browser's zone is still here, on `after.tz`, and is exactly the
    /// point: it differs from the calendar's by seven hours in August, and the
    /// body must come out the same as if it did not exist. `edit_patch_body`
    /// replaces it with [`edit_zone`]'s answer, and `when_json`'s date arm
    /// never reads a zone at all.
    #[test]
    fn an_untouched_all_day_date_sends_no_times_when_the_calendar_zone_is_not_the_browsers() {
        // August, so New York is UTC-4 and Sofia UTC+3: seven hours apart, and
        // the calendar is the western of the two.
        const CAL_TZ: &str = "America/New_York";
        const BROWSER_TZ: &str = "Europe/Sofia";

        let ev = all_day_row("2026-08-10", "2026-08-11", CAL_TZ);
        assert_ne!(
            ev.start_utc,
            midnight_in("2026-08-10", BROWSER_TZ),
            "fixture check: the two zones must genuinely disagree about this \
             date's instant, or the assertions below prove nothing"
        );

        // What the form sends after the user edits the title and nothing else:
        // the dates it displayed, with the browser's zone alongside them.
        let mut after = all_day_form("Berlin trip (booked)", "2026-08-10", "2026-08-11");
        after.tz = BROWSER_TZ.into();

        let body = edit_patch_body(&ev, ev.start_utc, ev.end_utc, ev.start_utc, CAL_TZ, &after);

        assert_eq!(body["summary"], "Berlin trip (booked)", "the edit the user actually made");
        assert!(
            body.get("start").is_none(),
            "the date did not change, so no `start` may be sent — this is the defect: {body}"
        );
        assert!(body.get("end").is_none(), "no `end` either: {body}");
    }

    /// The old defect's control arm, and the reason the suite stayed green
    /// through eight tasks: with one zone in play the two coordinate systems
    /// coincided and the body carried the title alone.
    ///
    /// It stays because it is the *other* half of the pair. On its own the test
    /// above could be satisfied by an `edit_patch_body` that never sends times
    /// at all; this one has always demanded the machinery work, and
    /// [`a_changed_all_day_date_sends_the_dates_the_user_picked`] demands it
    /// still notice a real change.
    #[test]
    fn an_untouched_all_day_date_sends_no_times_when_both_zones_agree() {
        const TZ: &str = "America/New_York";

        let ev = all_day_row("2026-08-10", "2026-08-11", TZ);
        let mut after = all_day_form("Berlin trip (booked)", "2026-08-10", "2026-08-11");
        after.tz = TZ.into();

        let body = edit_patch_body(&ev, ev.start_utc, ev.end_utc, ev.start_utc, TZ, &after);

        assert_eq!(body["summary"], "Berlin trip (booked)");
        assert!(body.get("start").is_none(), "an untouched date must send no start: {body}");
        assert!(body.get("end").is_none(), "an untouched date must send no end: {body}");
    }

    /// The control the two above need: a date the user *did* change must reach
    /// Google as the date they picked, in a bare `date` with no `timeZone`
    /// beside it — from a browser seven hours away from the calendar.
    ///
    /// Without this, "sends no times" is satisfiable by sending no times ever.
    #[test]
    fn a_changed_all_day_date_sends_the_dates_the_user_picked() {
        const CAL_TZ: &str = "America/New_York";

        let ev = all_day_row("2026-08-10", "2026-08-11", CAL_TZ);
        let mut after = all_day_form("Berlin trip", "2026-08-12", "2026-08-14");
        after.tz = "Europe/Sofia".into();

        let body = edit_patch_body(&ev, ev.start_utc, ev.end_utc, ev.start_utc, CAL_TZ, &after);

        assert_eq!(body["start"], serde_json::json!({ "date": "2026-08-12" }), "{body}");
        assert_eq!(body["end"], serde_json::json!({ "date": "2026-08-14" }), "{body}");
    }

    /// Plan 5's anchoring rule, in the date domain — and the harm it exists to
    /// prevent is at its worst here.
    ///
    /// The form is pre-filled from the **clicked** occurrence, seven months
    /// after the master's own dates. A title-only save with scope `"all"` must
    /// send no dates at all: sent verbatim they would drag the series' DTSTART
    /// onto the clicked date and drop every occurrence before it, with
    /// `sendUpdates=all` behind it.
    ///
    /// **`Pacific/Auckland` and not New York, and that is load bearing.** This
    /// is also the one test that can catch a date derived in UTC rather than in
    /// the calendar's own zone. New York cannot: it is west of UTC, so midnight
    /// there is 04:00Z *the same day* and the two derivations agree. Auckland
    /// is UTC+12, so midnight there is midday the **previous** day in UTC — and
    /// a UTC derivation then puts the master's before-side a day behind its
    /// after-side, so an untouched save sends dates. Asserted as a fixture
    /// check below rather than left to the reader.
    #[test]
    fn a_title_only_edit_of_an_all_day_series_sends_no_dates() {
        const CAL_TZ: &str = "Pacific/Auckland";

        let mut ev = all_day_row("2026-01-05", "2026-01-06", CAL_TZ);
        ev.recurrence = Some("RRULE:FREQ=WEEKLY".into());
        assert_eq!(
            jiff::Timestamp::from_millisecond(ev.start_utc).unwrap().to_string(),
            "2026-01-04T11:00:00Z",
            "fixture check: the stored instant must fall on the *previous* date in UTC, or \
             a date derived in UTC instead of the calendar's zone passes this unnoticed"
        );

        // The occurrence the user clicked, months down the series.
        let occurrence = midnight_in("2026-08-10", CAL_TZ);
        let mut after = all_day_form("Berlin trip (booked)", "2026-08-10", "2026-08-11");
        after.tz = "Europe/Sofia".into();

        let body = edit_patch_body(&ev, ev.start_utc, ev.end_utc, occurrence, CAL_TZ, &after);

        assert_eq!(body["summary"], "Berlin trip (booked)");
        assert!(
            body.get("start").is_none(),
            "the master's DTSTART would move seven months onto the clicked date: {body}"
        );
        assert!(body.get("end").is_none(), "{body}");
    }

    /// The same series, actually moved: the master takes the **shift** — one
    /// day — not the clicked occurrence's own dates.
    ///
    /// This is the assertion the test above cannot make. Together they pin that
    /// the arithmetic is relative: `2026-01-06`, not `2026-08-11` (the form's
    /// value sent verbatim) and not `2026-01-05` (the movement dropped).
    #[test]
    fn moving_an_all_day_series_shifts_the_master_by_the_days_the_user_moved() {
        const CAL_TZ: &str = "Pacific/Auckland";

        let mut ev = all_day_row("2026-01-05", "2026-01-06", CAL_TZ);
        ev.recurrence = Some("RRULE:FREQ=WEEKLY".into());

        let occurrence = midnight_in("2026-08-10", CAL_TZ);
        // The user drags the clicked occurrence one day on, and lengthens it to
        // two days — so start and end move by different amounts.
        let mut after = all_day_form("Berlin trip", "2026-08-11", "2026-08-13");
        after.tz = "Europe/Sofia".into();

        let body = edit_patch_body(&ev, ev.start_utc, ev.end_utc, occurrence, CAL_TZ, &after);

        assert_eq!(
            body["start"],
            serde_json::json!({ "date": "2026-01-06" }),
            "the master moved by one day, which is what the user did: {body}"
        );
        assert_eq!(
            body["end"],
            serde_json::json!({ "date": "2026-01-08" }),
            "the master's end moved by two days — one for the move, one for the \
             extra day of length: {body}"
        );
    }

    /// **The zone `shifted_when`'s all-day arm reads its dates in, and the only
    /// fixture in this file that can see it.**
    ///
    /// That arm calls `date_in_zone` four times — on the target's two ends and
    /// on the anchor's two — and every one of them is a *subtraction away* from
    /// mattering. Put the whole arm on UTC and Auckland and New York both stay
    /// green, because each shifts the target and the anchor by the **same**
    /// number of UTC days in both seasons: Auckland is UTC+13 and UTC+12, so
    /// midnight is the previous UTC date all year; New York is UTC-5 and UTC-4,
    /// so it is the same UTC date all year. Either way the offsets cancel out of
    /// `to - from` and the answer survives a zone it was never given.
    ///
    /// `Europe/Lisbon` is the one that does not cancel: **UTC+0 in January and
    /// UTC+1 in August**, so the master's midnight is the same UTC date and the
    /// clicked occurrence's is the previous one. Read in UTC the anchor moves a
    /// day and the target does not, so a title-only save sends
    /// `{"start":{"date":"2026-01-06"}}` — the series' DTSTART a day forward,
    /// every occurrence with it, `sendUpdates=all`.
    ///
    /// The two fixture checks below are what keep that true. If either instant
    /// stops straddling midnight the way it does here, this test goes on passing
    /// while proving nothing, which is exactly how the suite stayed blind to the
    /// defect this whole plan exists to close.
    #[test]
    fn an_all_day_series_reads_its_dates_in_the_calendars_zone_not_utc() {
        const CAL_TZ: &str = "Europe/Lisbon";

        let mut ev = all_day_row("2026-01-05", "2026-01-06", CAL_TZ);
        ev.recurrence = Some("RRULE:FREQ=WEEKLY".into());
        let occurrence = midnight_in("2026-08-10", CAL_TZ);

        assert_eq!(
            jiff::Timestamp::from_millisecond(ev.start_utc).unwrap().to_string(),
            "2026-01-05T00:00:00Z",
            "fixture check: the master's midnight must fall on the *same* UTC date"
        );
        assert_eq!(
            jiff::Timestamp::from_millisecond(occurrence).unwrap().to_string(),
            "2026-08-09T23:00:00Z",
            "fixture check: the occurrence's midnight must fall on the *previous* UTC \
             date. Together with the check above, that is what stops the two offsets \
             cancelling — without it a UTC derivation passes unnoticed"
        );

        // The user changed the title and nothing else.
        let mut after = all_day_form("Berlin trip (booked)", "2026-08-10", "2026-08-11");
        after.tz = "Europe/Sofia".into();

        let body = edit_patch_body(&ev, ev.start_utc, ev.end_utc, occurrence, CAL_TZ, &after);

        assert_eq!(body["summary"], "Berlin trip (booked)");
        assert!(
            body.get("start").is_none(),
            "the dates were derived in the wrong zone: the master's DTSTART moves a day \
             forward and takes every occurrence with it: {body}"
        );
        assert!(body.get("end").is_none(), "{body}");
    }

    /// Turning **All day on** for a whole series, with the form's date agreeing
    /// with the event's own zone.
    ///
    /// The arm is chosen by the *form's* variant, so this crosses from a timed
    /// before-side to an all-day after-side — the mixed case that has no branch
    /// of its own. The master must become an all-day event on **its own** date,
    /// not on the clicked occurrence's seven months later.
    ///
    /// `cal_tz` is deliberately a third zone nothing should reach: for a timed
    /// row `edit_zone` answers `ev.start_tz`, so an implementation that took the
    /// calendar's zone here fails on this fixture rather than passing by
    /// coincidence.
    ///
    /// **`America/Argentina/Buenos_Aires` and not just any third zone.** These
    /// dates are a *difference*, so a wrong zone only shows when it moves the
    /// target and the anchor by different numbers of days. Buenos Aires is
    /// UTC-3 with no daylight saving, against New York's UTC-5 and UTC-4: it
    /// reads the January instant as the next day and the August one as the same
    /// day, so the two do not cancel. `Pacific/Auckland` was the first choice
    /// here and was **wrong** — it shifts both by +1 and the substitution passes
    /// unnoticed.
    #[test]
    fn turning_on_all_day_for_a_series_lands_on_the_masters_own_date() {
        const CAL_TZ: &str = "America/Argentina/Buenos_Aires";

        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        ev.start_tz = "America/New_York".into();
        ev.end_tz = "America/New_York".into();
        ev.start_utc = ny("2026-01-05T22:00:00");
        ev.end_utc = ny("2026-01-05T23:00:00");

        let occurrence = ny("2026-08-10T22:00:00");
        assert_ne!(
            crate::write::date_in_zone(ev.start_utc, CAL_TZ) == "2026-01-05",
            crate::write::date_in_zone(occurrence, CAL_TZ) == "2026-08-10",
            "fixture check: the calendar's zone must disagree with the event's own on \
             exactly one of these two instants. If it disagrees on both, the offsets \
             cancel out of the subtraction and a wrong zone passes unnoticed"
        );

        // A New York user: the form shows the 10th, which is the occurrence's
        // date in the event's own zone, so the shift is zero.
        let after = all_day_form("Standup", "2026-08-10", "2026-08-11");

        let body = edit_patch_body(&ev, ev.start_utc, ev.end_utc, occurrence, CAL_TZ, &after);

        assert_eq!(
            body["start"],
            serde_json::json!({ "date": "2026-01-05" }),
            "the master became all-day on the clicked occurrence's date rather than its \
             own, which drops every occurrence before it: {body}"
        );
        assert_eq!(body["end"], serde_json::json!({ "date": "2026-01-06" }), "{body}");
    }

    /// The same toggle from a browser a day ahead of the event's own zone — the
    /// residual `shifted_when` documents, pinned so its **size** is a fact
    /// rather than a claim.
    ///
    /// 22:00 in New York is 05:00 the next morning in Sofia, so the form shows
    /// the 11th and the user means the 11th. That reads as a one-day shift, and
    /// the master moves one day. It is wrong — the display half of this same
    /// boundary, which design §3 leaves open for timed rows — but it is wrong by
    /// **a day**, not by the seven months a form value sent verbatim would cost.
    ///
    /// If a later task closes the display half for timed rows, this test is the
    /// one to invert: `2026-01-05`/`2026-01-06` becomes the right answer.
    #[test]
    fn turning_on_all_day_from_a_browser_a_day_ahead_moves_the_master_by_one_day_not_seven_months() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        ev.start_tz = "America/New_York".into();
        ev.end_tz = "America/New_York".into();
        ev.start_utc = ny("2026-01-05T22:00:00");
        ev.end_utc = ny("2026-01-05T23:00:00");

        let occurrence = ny("2026-08-10T22:00:00");
        assert_eq!(
            crate::write::date_in_zone(occurrence, "Europe/Sofia"),
            "2026-08-11",
            "fixture check: the browser must genuinely read this occurrence as the next \
             day, or this test is the one above with different numbers"
        );

        let after = all_day_form("Standup", "2026-08-11", "2026-08-12");

        // The same non-cancelling third zone as the test above, for the same
        // reason — see its fixture check.
        let body = edit_patch_body(
            &ev,
            ev.start_utc,
            ev.end_utc,
            occurrence,
            "America/Argentina/Buenos_Aires",
            &after,
        );

        assert_eq!(
            body["start"],
            serde_json::json!({ "date": "2026-01-06" }),
            "bounded: one day off the master's own date. Not 2026-08-11, which is the \
             form's value sent verbatim and would drop seven months of occurrences: {body}"
        );
        assert_eq!(body["end"], serde_json::json!({ "date": "2026-01-07" }), "{body}");
    }

    /// A scope this command does not implement must be refused rather than
    /// left to fall through: [`target_event_id`] reads every scope that is not
    /// `"all"` as "this one", so an unrecognised one would silently edit a
    /// single occurrence of the series the user asked to do something else
    /// with.
    ///
    /// Also proves the guard runs before `load_config` — past it this test
    /// would read the real `~/.config/omacal/config.toml` and then the real
    /// Keychain, which no test may do, and fail with that message instead.
    ///
    /// **The scope here must be one nothing implements, and stay that way.**
    /// This test used `"following"` while that scope was Task 7's stand-in for
    /// "unimplemented", and implementing it turned the test into exactly the
    /// thing its own second paragraph forbids: it ran through `load_config`,
    /// read the real config off disk, and failed at the Keychain with "No
    /// matching credential found". It passed the assertion for a year of
    /// nobody looking only because the seeded account (`e@x`) happens to have
    /// no keyring entry. `"thisAndPrevious"` is Google's own vocabulary for a
    /// scope this app has no plans for.
    ///
    /// **And the choice of scope is not the only defence, because it cannot be.**
    /// Nobody predicted the first incident either. The fixture is a `reader`
    /// calendar ([`seeded_pool_on_read_only_cal`]) so that a future task
    /// implementing this scope hits the writability gate — "this calendar is not
    /// writable from omacal" — instead of falling through to `load_config`. The
    /// assertion below is unchanged and just as discriminating: delete the scope
    /// gate and it fails either way.
    #[tokio::test]
    async fn an_unimplemented_scope_is_refused_rather_than_treated_as_this_occurrence() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, id) = seeded_pool_on_read_only_cal(&mut ev).await;

        let err = update_impl(
            &state_with(pool, false),
            id,
            "thisAndPrevious",
            OCCURRENCE,
            form("Standup (moved)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("not available yet"), "got: {err}");
    }

    /// Demo mode must reach neither Google nor the real database, on this
    /// verb as on the other two. Asserted the same way `create`'s is: the
    /// failure must be the demo message rather than a config or keyring error,
    /// which is only true if the guard is the first statement.
    #[tokio::test]
    async fn updating_refuses_in_demo_mode_without_touching_config_keyring_or_google() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;
        let state = state_with(pool, true);

        let err = update_impl(
            &state,
            id,
            "all",
            OCCURRENCE,
            form("Standup (moved)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("demo"), "got: {err}");
        // Binds the emitter to `errors.rs`'s allowlist, so the two literals
        // cannot drift apart while the user quietly starts reading OPAQUE.
        assert_eq!(crate::errors::user_facing(&err), "demo mode — there is nothing to save");

        let (row, _, _) = omacal_store::event_by_id(&state.pool, id).await.unwrap().unwrap();
        assert_eq!(row.summary.as_deref(), Some("Standup"), "demo mode wrote to the store");
    }

    /// The retry's other half, and the one the first version of this got
    /// wrong. The user touched only the title; somebody else *moved* the
    /// event in the meantime. The retry re-reads the target for its version,
    /// so the target's start is not the same value it was on the first
    /// attempt — and anchoring the movement on it would make the movement
    /// absolute, turning the absence of a user edit into the presence of a
    /// revert. The meeting would be rescheduled back and the guest list mailed
    /// about it, which is the exact harm the retry exists to prevent.
    ///
    /// `a_stale_etag_retries_once_...` cannot catch this: its GET returns the
    /// event at unchanged times, so an absolute anchor and a relative one give
    /// the same answer there.
    #[tokio::test]
    async fn a_retry_after_someone_else_moved_the_event_does_not_reschedule_it_back() {
        let mut ev = stored(vec![]);
        ev.summary = Some("Brunch".into());
        ev.start_utc = OCCURRENCE;
        ev.end_utc = OCCURRENCE + HOUR;
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await; // google_id "ev1", etag "old"

        // Their move: one day later, after the form was already open.
        let moved = OCCURRENCE + 24 * HOUR;

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/ev1"))
            .and(wiremock::matchers::header("if-match", "\"old\""))
            .respond_with(wiremock::ResponseTemplate::new(412))
            .expect(1)
            .mount(&server)
            .await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/ev1"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ev1", "status": "confirmed", "etag": "\"fresh\"",
                "summary": "Brunch",
                "start": {"dateTime": omacal_sync::to_rfc3339(moved)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(moved + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        // The title, and nothing else. A `start`/`end` here at all is the bug:
        // matched as a whole document, so their move being re-sent as
        // `OCCURRENCE` fails on this mock rather than passing quietly.
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/ev1"))
            .and(wiremock::matchers::header("if-match", "\"fresh\""))
            .and(wiremock::matchers::body_json(serde_json::json!({"summary": "Brunch (moved)"})))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ev1", "status": "confirmed", "etag": "\"e3\"",
                "summary": "Brunch (moved)",
                "start": {"dateTime": omacal_sync::to_rfc3339(moved)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(moved + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "all",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Brunch (moved)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();
    }

    /// Defence in depth against the defect class this whole design exists for.
    /// A one-off has no occurrences, so `occurrence_start_ms` names nothing —
    /// and the anchor must be the event's own start, not whatever the caller
    /// passed. Anchoring on the argument would let the very mistake Plan 2
    /// shipped (handing a series' DTSTART where an occurrence's start belongs)
    /// move an event nobody asked to move.
    ///
    /// The value below is deliberately wrong, which is the only way to tell
    /// the two apart: everywhere else in this file the caller passes the right
    /// one and both readings agree.
    #[tokio::test]
    async fn a_one_off_ignores_the_occurrence_anchor_and_takes_the_form_at_face_value() {
        let mut ev = stored(vec![]);
        ev.summary = Some("Lunch".into());
        ev.start_utc = OCCURRENCE;
        ev.end_utc = OCCURRENCE + HOUR;
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        // The user moved it an hour later. The body must say exactly that.
        let (start, end) = timed_json(OCCURRENCE + HOUR, OCCURRENCE + 2 * HOUR, "UTC");
        let expected = serde_json::json!({ "start": start, "end": end });

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/ev1"))
            .and(wiremock::matchers::body_json(expected))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ev1", "status": "confirmed", "etag": "\"e2\"", "summary": "Lunch",
                "start": {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE + HOUR)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE + 2 * HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "all",
            DTSTART, // wrong on purpose: two weeks off, and irrelevant here
            ev,
            "cal@x.com",
            "UTC",
            form("Lunch", OCCURRENCE + HOUR, OCCURRENCE + 2 * HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();
    }

    /// The one branch nothing else reaches: `"all"` from a *materialised
    /// exception*. It is the only path that fetches the master with
    /// `get_event`, the only place outside the instance lookup where the
    /// etag-provenance rule applies, and the only shape where the row's own
    /// `recurrence` is `None` while the row is still one occurrence of a
    /// series.
    ///
    /// That last part is why `is_recurring` and not `ev.recurrence.is_some()`:
    /// read the second way, an exception is "not recurring", the anchor
    /// becomes the row's own start, and a *title-only* edit sends the
    /// exception's instant to a master anchored two weeks earlier — the
    /// data-loss body, silent and green.
    ///
    /// The row's start and the clicked occurrence deliberately differ here (a
    /// sync moved the exception after the grid painted, so the form still
    /// holds what the user saw), which is the only way to tell the two
    /// readings apart.
    #[tokio::test]
    async fn editing_all_events_from_an_exception_row_asks_the_master_and_anchors_on_the_click() {
        let mut ev = stored(vec![]);
        ev.google_id = "exception1".into();
        ev.summary = Some("Standup".into());
        ev.recurring_event_id = Some("master1".into());
        ev.original_start_utc = Some(OCCURRENCE);
        ev.start_utc = OCCURRENCE + 5 * HOUR; // moved since the grid painted
        ev.end_utc = OCCURRENCE + 6 * HOUR;
        ev.etag = Some("\"exception-etag\"".into());
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        // Nothing here has the master in hand, and there is no version of it
        // to condition on without asking.
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"master-etag\"",
                "summary": "Standup",
                "recurrence": ["RRULE:FREQ=WEEKLY"],
                "start": {"dateTime": omacal_sync::to_rfc3339(DTSTART)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(DTSTART + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        // The master's own version, and a body with no times in it at all.
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .and(wiremock::matchers::header("if-match", "\"master-etag\""))
            .and(wiremock::matchers::body_json(
                serde_json::json!({"summary": "Standup (moved)"}),
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"master-2\"",
                "summary": "Standup (moved)",
                "recurrence": ["RRULE:FREQ=WEEKLY"],
                "start": {"dateTime": omacal_sync::to_rfc3339(DTSTART)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(DTSTART + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "all",
            OCCURRENCE, // what the user clicked, and what the form was filled from
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (moved)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();

        // `master1` is a different Google id than `exception1`, so the local
        // exception row is left for the next sync rather than stamped with the
        // master's state.
        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(
            row.etag.as_deref(),
            Some("\"exception-etag\""),
            "the master's etag was stamped onto the exception's own row"
        );
    }

    /// A wall clock in New York as an instant. `2026-03-08T02:00` is that
    /// zone's spring-forward for 2026, which the two tests below sit either
    /// side of.
    fn ny(wall: &str) -> i64 {
        wall.parse::<jiff::civil::DateTime>()
            .unwrap()
            .in_tz("America/New_York")
            .unwrap()
            .timestamp()
            .as_millisecond()
    }

    /// The elapsed-time trap, end to end. Moving an occurrence from the
    /// Saturday before a spring-forward to the Sunday after it is one day of
    /// calendar time and 23 hours of elapsed time. The master is a month
    /// earlier, on the winter side, so a millisecond delta arrives an hour
    /// early and quietly moves a 09:00 series to 08:00 — for everybody, with
    /// `sendUpdates=all`.
    #[tokio::test]
    async fn a_timed_shift_across_a_transition_keeps_the_series_time_of_day() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        ev.start_tz = "America/New_York".into();
        ev.end_tz = "America/New_York".into();
        ev.start_utc = ny("2026-02-07T09:00:00");
        ev.end_utc = ny("2026-02-07T10:00:00");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let occurrence = ny("2026-03-07T09:00:00");
        let moved = ny("2026-03-08T09:00:00");
        assert_eq!(
            moved - occurrence,
            23 * HOUR,
            "fixture check: the move must actually cross the transition"
        );

        let (start, end) = timed_json(
            ny("2026-02-08T09:00:00"),
            ny("2026-02-08T10:00:00"),
            "America/New_York",
        );
        let expected = serde_json::json!({ "start": start, "end": end });

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .and(wiremock::matchers::body_json(expected))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"master-2\"",
                "summary": "Standup",
                "recurrence": ["RRULE:FREQ=WEEKLY"],
                "start": {"dateTime": omacal_sync::to_rfc3339(ny("2026-02-08T09:00:00")),
                          "timeZone": "America/New_York"},
                "end":   {"dateTime": omacal_sync::to_rfc3339(ny("2026-02-08T10:00:00")),
                          "timeZone": "America/New_York"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "all",
            occurrence,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup", moved, moved + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();
    }

    /// The same trap on an all-day series, one level down from where it used
    /// to sit.
    ///
    /// The dates themselves no longer touch an instant — the form sends them
    /// and `crate::write::shifted_date` moves them in whole days. **One piece
    /// of instant arithmetic survives on this path**: `edit_patch_body` derives
    /// the clicked occurrence's own end (`anchor_end`) by moving the anchor by
    /// the target's span, and then reads a *date* off it. That is the value
    /// every all-day end is compared against.
    ///
    /// So the fixture is a **fall-back**, not a spring-forward. 24 hours after
    /// midnight on a 25-hour day is 23:00 the *same* day, so a plain
    /// millisecond delta names the wrong date for the occurrence's end — and
    /// the user, who touched nothing but the title, gets an `end` a day later
    /// than the master's, PATCHed with `sendUpdates=all`. A spring-forward
    /// hides it: 24 hours into a 23-hour day is 01:00 the next day, which is
    /// the right date by luck.
    ///
    /// All-day resolves against the *calendar's* zone (Google sends a bare
    /// `date` with no zone of its own), so the calendar here is the New York
    /// one and the event's stored `start_tz` is left elsewhere on purpose.
    #[tokio::test]
    async fn a_title_only_all_day_edit_across_a_fall_back_still_sends_no_dates() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        ev.is_all_day = true;
        ev.start_tz = "Europe/Sofia".into(); // must be ignored: all-day takes the calendar's
        ev.start_utc = ny("2026-02-07T00:00:00");
        ev.end_utc = ny("2026-02-08T00:00:00");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "America/New_York").await;

        let occurrence = ny("2026-11-01T00:00:00");
        assert_eq!(
            ny("2026-11-02T00:00:00") - occurrence,
            25 * HOUR,
            "fixture check: the clicked occurrence must sit on a fall-back day, or this \
             proves nothing about the arithmetic it is aimed at"
        );
        assert_eq!(
            ev.end_utc - ev.start_utc,
            24 * HOUR,
            "fixture check: the master's own span must have no transition in it, so the \
             only place one can enter is the anchor"
        );

        // `body_json` compares the whole document, so this is the assertion:
        // the title goes out and nothing else does. A `start` of any value —
        // right date or wrong — fails to match, and the unmatched request is
        // answered with a bare 404 that fails the `unwrap` below as well.
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .and(wiremock::matchers::body_json(
                serde_json::json!({ "summary": "Standup (booked)" }),
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"master-2\"",
                "summary": "Standup (booked)",
                "recurrence": ["RRULE:FREQ=WEEKLY"],
                "start": {"date": "2026-02-07"},
                "end":   {"date": "2026-02-08"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        // The user changed the title and nothing else. The form is pre-filled
        // from the clicked occurrence, so it carries that occurrence's dates.
        let after = all_day_form("Standup (booked)", "2026-11-01", "2026-11-02");

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "all",
            occurrence,
            ev,
            "cal@x.com",
            "America/New_York",
            after,
            "all",
            &client,
        )
        .await
        .unwrap();
    }

    /// The control for the test above: the same series, on the same fall-back
    /// day, actually moved. Without it "sends no dates" is satisfiable by a
    /// function that never sends dates.
    ///
    /// The master moves by the one day the user moved the occurrence, not to
    /// the occurrence's own date nine months later.
    #[tokio::test]
    async fn an_all_day_series_moved_across_a_fall_back_shifts_the_master_by_one_day() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        ev.is_all_day = true;
        ev.start_tz = "Europe/Sofia".into();
        ev.start_utc = ny("2026-02-07T00:00:00");
        ev.end_utc = ny("2026-02-08T00:00:00");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "America/New_York").await;

        let occurrence = ny("2026-11-01T00:00:00");

        let expected = serde_json::json!({
            "start": {"date": "2026-02-08"},
            "end":   {"date": "2026-02-09"},
        });

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .and(wiremock::matchers::body_json(expected))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"master-2\"",
                "summary": "Standup",
                "recurrence": ["RRULE:FREQ=WEEKLY"],
                "start": {"date": "2026-02-08"},
                "end":   {"date": "2026-02-09"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        let after = all_day_form("Standup", "2026-11-02", "2026-11-03");
        update_via_client(
            &pool,
            "all",
            occurrence,
            ev,
            "cal@x.com",
            "America/New_York",
            after,
            "all",
            &client,
        )
        .await
        .unwrap();
    }

    /// Editing an occurrence somebody has just deleted. Google answers the
    /// lookup with a cancelled instance, which carries no usable times — and
    /// the times of the resource being patched are what the request is built
    /// against, so this stops rather than guessing. Named plainly for the
    /// user, since it is a thing that genuinely happens.
    #[tokio::test]
    async fn editing_an_occurrence_that_has_been_cancelled_says_so_and_patches_nothing() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1/instances"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"id": "master1_20260810T090000Z", "status": "cancelled",
                           "etag": "\"gone\""}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        let err = update_via_client(
            &pool,
            "this",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (moved)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("no longer on the calendar"), "{err}");
        assert_eq!(
            crate::errors::user_facing(&err),
            "that occurrence is no longer on the calendar"
        );
        assert!(
            requests(&server).await.iter().all(|r| r.method.as_str() != "PATCH"),
            "a cancelled occurrence was patched anyway"
        );
    }

    /// A subscribed holiday calendar, or one shared with you read-only. The
    /// refusal happens before `load_config`, the Keychain or Google see the
    /// request — the same shape `creating_into_a_read_only_calendar_is_refused`
    /// has, and the reason no mock server is needed here.
    #[tokio::test]
    async fn updating_an_event_on_a_read_only_calendar_is_refused() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let cal = seed_calendar_with_tz(&pool, "reader", "UTC").await;
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        ev.calendar_id = cal;
        let id = omacal_store::upsert_event(&pool, &ev).await.unwrap();

        let err = update_impl(
            &state_with(pool, false),
            id,
            "all",
            OCCURRENCE,
            form("Standup (moved)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("not writable"), "got: {err}");
        assert_eq!(crate::errors::user_facing(&err), "this calendar is not writable from omacal");
    }

    // --- update_event, scope "following": the series split. The only path in
    // this app that makes two writes Google cannot make atomically, so the
    // *order* of those writes is a property in its own right and gets its own
    // tests — as does what happens when the second one fails.

    /// `OCCURRENCE` less one second, as an RFC 5545 `UNTIL`. Verify with:
    /// `python3 -c "import datetime as d; print(d.datetime.fromtimestamp(1786352400-1, d.timezone.utc))"`
    const UNTIL_BEFORE_OCCURRENCE: &str = "RRULE:FREQ=WEEKLY;UNTIL=20260810T085959Z";

    /// A guest list whose every writable field is set to something different,
    /// so a body built from anything less than a full round trip fails on it:
    /// an optional guest, a comment, extra guests, two display names and one
    /// without, and three different answers. Google replaces the array
    /// wholesale, so each of those is a real thing to lose.
    fn wire_guests() -> serde_json::Value {
        serde_json::json!([
            {"email": "ana@x.com", "displayName": "Ana", "responseStatus": "accepted",
             "optional": false, "self": true, "additionalGuests": 0},
            {"email": "bo@x.com", "responseStatus": "declined", "optional": true,
             "comment": "double-booked", "additionalGuests": 2},
            {"email": "cy@x.com", "displayName": "Cy", "responseStatus": "needsAction",
             "optional": false, "additionalGuests": 0}
        ])
    }

    /// The same list as it must go back out: `self` dropped (Google's own
    /// per-request annotation, not ours to send) and every other field intact,
    /// including the answers people already gave — a split is the same meeting
    /// continuing, so nobody is asked to RSVP again.
    fn expected_guests() -> serde_json::Value {
        serde_json::json!([
            {"email": "ana@x.com", "displayName": "Ana", "responseStatus": "accepted",
             "optional": false, "additionalGuests": 0},
            {"email": "bo@x.com", "responseStatus": "declined", "optional": true,
             "comment": "double-booked", "additionalGuests": 2},
            {"email": "cy@x.com", "displayName": "Cy", "responseStatus": "needsAction",
             "optional": false, "additionalGuests": 0}
        ])
    }

    /// The series master as Google describes it. A split reads its rule, its
    /// guest list and its version from *here* rather than from the local row:
    /// this fixture's etag (`"m1"`) differs from `weekly_master`'s
    /// (`"master-etag"`) and its guest list is one the local row does not have
    /// at all, so a body built from the stored copy fails every assertion
    /// below rather than passing by coincidence.
    fn wire_master(rules: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "id": "master1", "status": "confirmed", "etag": "\"m1\"",
            "summary": "Standup",
            "recurrence": rules,
            "attendees": wire_guests(),
            "start": {"dateTime": omacal_sync::to_rfc3339(DTSTART)},
            "end":   {"dateTime": omacal_sync::to_rfc3339(DTSTART + HOUR)}
        })
    }

    /// What a successful split's POST returns: the tail, as its own series.
    fn wire_new_series(start_ms: i64) -> serde_json::Value {
        serde_json::json!({
            "id": "tail1", "status": "confirmed", "etag": "\"t1\"",
            "summary": "Standup (from here)",
            "recurrence": ["RRULE:FREQ=WEEKLY"],
            "attendees": wire_guests(),
            "start": {"dateTime": omacal_sync::to_rfc3339(start_ms)},
            "end":   {"dateTime": omacal_sync::to_rfc3339(start_ms + HOUR)}
        })
    }

    /// The GET every split begins with. Mounted with `.expect(1)` because the
    /// whole design rests on the master being read before it is written.
    async fn mount_master(server: &wiremock::MockServer, rules: &[&str]) {
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(wire_master(rules)),
            )
            .expect(1)
            .mount(server)
            .await;
    }

    /// The property the whole task exists for, asserted on the wire rather
    /// than on the source.
    ///
    /// Create-then-truncate failing halfway leaves an overlapping duplicate:
    /// visible, deletable, nothing lost. Truncate-then-create failing halfway
    /// deletes the tail of the series with no record of what was in it. There
    /// is no transaction available across two Google events, so the order is
    /// the only thing standing between a user and that second outcome.
    ///
    /// Both mocks match path *and* body — and the POST its `sendUpdates` too.
    /// Matching the method alone would let a body with the wrong times, no
    /// recurrence, no guest list or a truncation aimed at the wrong instant
    /// sail through while this test went on passing.
    #[tokio::test]
    async fn following_creates_the_new_series_before_truncating_the_old() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .and(wiremock::matchers::query_param("sendUpdates", "all"))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "start": timed_json(OCCURRENCE, OCCURRENCE + HOUR, "UTC").0,
                "end":   timed_json(OCCURRENCE, OCCURRENCE + HOUR, "UTC").1,
                "summary": "Standup (from here)",
                "recurrence": ["RRULE:FREQ=WEEKLY"],
                "attendees": expected_guests(),
            })))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(wire_new_series(OCCURRENCE)),
            )
            .expect(1)
            .mount(&server)
            .await;

        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .and(wiremock::matchers::header("if-match", "\"m1\""))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "recurrence": [UNTIL_BEFORE_OCCURRENCE],
            })))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"m2\"",
                "summary": "Standup",
                "recurrence": [UNTIL_BEFORE_OCCURRENCE],
                "attendees": wire_guests(),
                "start": {"dateTime": omacal_sync::to_rfc3339(DTSTART)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(DTSTART + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "following",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (from here)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();

        let sent = requests(&server).await;
        let post = sent.iter().position(|r| r.method.as_str() == "POST");
        let patch = sent.iter().position(|r| r.method.as_str() == "PATCH");
        assert!(post.is_some() && patch.is_some(), "one of the two writes never happened: {sent:?}");
        assert!(
            post < patch,
            "the original was shortened before the tail existed: a failure between the two \
             would have deleted the rest of the series with nothing to restore it from"
        );

        // Both halves reached the store, and neither wrote over the other:
        // `upsert_event` is keyed on `(calendar_id, google_id)`.
        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(
            row.recurrence.as_deref(),
            Some(UNTIL_BEFORE_OCCURRENCE),
            "the local master still expands past the split"
        );
        assert_eq!(row.etag.as_deref(), Some("\"m2\""));
        let tail: i64 = sqlx::query_scalar("SELECT id FROM events WHERE google_id = 'tail1'")
            .fetch_one(&pool)
            .await
            .expect("the new series was not stored locally");
        assert_ne!(tail, row.id, "the tail was written over the master's own row");
    }

    /// **"This and following" carries the guest list the user asked for.**
    ///
    /// The split is two writes and only one of them can hold a guest list: the
    /// truncation below is `recurrence` and nothing else, by design. So a guest
    /// change made on this scope either rides on the tail's POST or is lost —
    /// and lost is what it was, silently, until this test.
    ///
    /// Bo removed, Dan invited, **and the title left alone**, which does two
    /// jobs. It keeps the assertion about guests rather than about everything.
    /// And it drives the case where a guest change is the *only* change, which
    /// the `"following"` no-op guard sees first: that guard reads
    /// `edit_patch_body`'s emptiness, so a version whose body did not carry
    /// attendees would return before either write and this would fail on the
    /// absent POST rather than on its contents.
    ///
    /// Everyone carried across keeps what they had — `attendees_for_edit` is
    /// the same echo-back the patch path uses — and the master's own PATCH is
    /// asserted as `recurrence` alone, so the truncation cannot grow an
    /// attendee list by accident.
    #[tokio::test]
    async fn a_following_save_that_changes_the_guest_list_carries_the_new_list_to_the_tail() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;

        // Ana and Cy carried across with their own answers; Bo gone; Dan new
        // and un-answered. Stored order first, the addition after.
        let reconciled = serde_json::json!([
            {"email": "ana@x.com", "displayName": "Ana", "responseStatus": "accepted",
             "optional": false, "additionalGuests": 0},
            {"email": "cy@x.com", "displayName": "Cy", "responseStatus": "needsAction",
             "optional": false, "additionalGuests": 0},
            {"email": "dan@x.com", "responseStatus": "needsAction",
             "optional": false, "additionalGuests": 0}
        ]);

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "start": timed_json(OCCURRENCE, OCCURRENCE + HOUR, "UTC").0,
                "end":   timed_json(OCCURRENCE, OCCURRENCE + HOUR, "UTC").1,
                "summary": "Standup",
                "recurrence": ["RRULE:FREQ=WEEKLY"],
                "attendees": reconciled,
            })))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(wire_new_series(OCCURRENCE)),
            )
            .expect(1)
            .mount(&server)
            .await;

        // `recurrence` and nothing else. The people who were invited to the
        // occurrences that already happened are not the user's to rewrite from
        // a save aimed at the future.
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "recurrence": [UNTIL_BEFORE_OCCURRENCE],
            })))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"m2\"",
                "summary": "Standup",
                "recurrence": [UNTIL_BEFORE_OCCURRENCE],
                "attendees": wire_guests(),
                "start": {"dateTime": omacal_sync::to_rfc3339(DTSTART)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(DTSTART + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let after = crate::write::EventFields {
            guests: Some(vec![
                crate::write::Guest { email: "ana@x.com".into(), optional: false },
                crate::write::Guest { email: "cy@x.com".into(), optional: false },
                crate::write::Guest { email: "dan@x.com".into(), optional: false },
            ]),
            ..form("Standup", OCCURRENCE, OCCURRENCE + HOUR)
        };
        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool, "following", OCCURRENCE, ev, "cal@x.com", "UTC", after, "all", &client,
        )
        .await
        .unwrap();
    }

    /// **"Save without notifying" means it on this scope too.**
    ///
    /// A split is two writes and both used to carry `"all"` unconditionally,
    /// which was right while every save notified. Guest-list spec §3 makes it a
    /// choice, and a split that mailed regardless would make the choice a lie
    /// on one scope in three — the user presses *Save without notifying* and
    /// every guest is told twice, once by the tail's creation and once by the
    /// truncation.
    ///
    /// **Both writes**, asserted separately with `query_param` + `.expect(1)`:
    /// threading it into the POST alone leaves the PATCH mailing everyone, and
    /// a spec that checked only the POST would call that fixed.
    #[tokio::test]
    async fn a_following_save_carries_the_notify_choice_to_both_of_its_writes() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .and(wiremock::matchers::query_param("sendUpdates", "none"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(wire_new_series(OCCURRENCE)),
            )
            .expect(1)
            .mount(&server)
            .await;

        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .and(wiremock::matchers::query_param("sendUpdates", "none"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"m2\"",
                "summary": "Standup",
                "recurrence": [UNTIL_BEFORE_OCCURRENCE],
                "attendees": wire_guests(),
                "start": {"dateTime": omacal_sync::to_rfc3339(DTSTART)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(DTSTART + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "following",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (from here)", OCCURRENCE, OCCURRENCE + HOUR),
            "none",
            &client,
        )
        .await
        .unwrap();
    }

    /// And the other side of the same `match`, so it cannot be satisfied by a
    /// version that reconciles unconditionally: a `"following"` save whose form
    /// hands back the list **unchanged** still sends the series' own guest list
    /// across, with every answer on it.
    ///
    /// The distinction matters because the form always sends a list. "No guest
    /// change" therefore means "the same list", never "the field was absent",
    /// and the two arms have to agree about what that produces.
    #[tokio::test]
    async fn a_following_save_that_leaves_the_guest_list_alone_carries_the_series_list_across() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "start": timed_json(OCCURRENCE, OCCURRENCE + HOUR, "UTC").0,
                "end":   timed_json(OCCURRENCE, OCCURRENCE + HOUR, "UTC").1,
                "summary": "Standup (from here)",
                "recurrence": ["RRULE:FREQ=WEEKLY"],
                "attendees": expected_guests(),
            })))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(wire_new_series(OCCURRENCE)),
            )
            .expect(1)
            .mount(&server)
            .await;

        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"m2\"",
                "summary": "Standup",
                "recurrence": [UNTIL_BEFORE_OCCURRENCE],
                "attendees": wire_guests(),
                "start": {"dateTime": omacal_sync::to_rfc3339(DTSTART)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(DTSTART + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        // The master's list as the form would show it: Bo is stored optional,
        // and saying so is what makes this an *unchanged* list rather than one
        // that quietly demotes him.
        let after = crate::write::EventFields {
            guests: Some(vec![
                crate::write::Guest { email: "ana@x.com".into(), optional: false },
                crate::write::Guest { email: "bo@x.com".into(), optional: true },
                crate::write::Guest { email: "cy@x.com".into(), optional: false },
            ]),
            ..form("Standup (from here)", OCCURRENCE, OCCURRENCE + HOUR)
        };
        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool, "following", OCCURRENCE, ev, "cal@x.com", "UTC", after, "all", &client,
        )
        .await
        .unwrap();
    }

    /// `an_edit_that_changes_nothing_sends_no_request`'s rule, for the one
    /// scope that never reaches the guard enforcing it.
    ///
    /// `"following"` returns into [`split_series`] well before the empty-PATCH
    /// check, and a split's payload is not a diff: the POST carries the whole
    /// tail whether or not anything changed, and it carries it with
    /// `sendUpdates=all`. So a save that touched nothing used to create a
    /// second series and truncate the first — two Google resources where there
    /// was one, every guest mailed twice about an edit nobody made, and nothing
    /// in this app that can undo it.
    ///
    /// **No requests at all**, read off the server's own record rather than
    /// inferred from `Ok`: the master GET is a real request too, and a guard
    /// placed after it would leave this passing on the strength of a return
    /// value while the whole reason for the check — the two writes behind that
    /// GET — went untested.
    ///
    /// The whole split is mounted, and answered successfully, *on purpose*.
    /// Leaving the server bare would fail this test too, but by panicking on
    /// the 404 the first stray request earns — which reports a transport error
    /// and never reaches the assertion that names what was actually sent. With
    /// the responses in place a guard that stopped working produces the real
    /// failure: `POST /calendars/…/events`, in the message.
    ///
    /// The form is `weekly_master`'s own values moved onto the clicked
    /// occurrence, which is exactly what `valueFromDetail` pre-fills and a user
    /// who changes nothing sends back.
    #[tokio::test]
    async fn a_following_save_that_changes_nothing_sends_no_request() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        // Deliberately without `.expect(..)`: these exist to make the failure
        // legible, not to assert anything themselves. The assertion is below.
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(wire_master(&["RRULE:FREQ=WEEKLY"])),
            )
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(wire_new_series(OCCURRENCE)),
            )
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(wire_master(&[
                UNTIL_BEFORE_OCCURRENCE,
            ])))
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "following",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();

        assert_eq!(
            methods_and_paths(&requests(&server).await),
            Vec::<String>::new(),
            "a \"this and following\" save that changed nothing still went to Google — every \
             guest on the series was mailed twice for it"
        );
    }

    /// The half that keeps the guard above from being too wide, and the reason
    /// it asks about `original_start_utc` rather than only about the diff.
    ///
    /// An exception is by construction *not* what the rule expands to at its
    /// slot: somebody has already moved this occurrence five hours later.
    /// Splitting from it re-anchors the whole tail onto where it was moved to,
    /// so the calendar afterwards genuinely differs from the calendar before —
    /// even though every field in the form is the row's own value, untouched.
    /// That is an edit the UI offers no other way of expressing, and swallowing
    /// it silently would be worse than the duplicate the guard exists to
    /// prevent.
    ///
    /// Drop the `original_start_utc` term and this fails with no writes at all;
    /// drop the whole guard and
    /// `a_following_save_that_changes_nothing_sends_no_request` fails instead.
    /// Neither can be made to pass by weakening the other.
    #[tokio::test]
    async fn a_following_save_from_a_moved_occurrence_still_splits_with_the_form_untouched() {
        let mut ev = exception_row("master1_20260810T090000Z", OCCURRENCE, "confirmed");
        ev.start_utc = OCCURRENCE + 5 * HOUR; // dragged, and the block clicked
        ev.end_utc = OCCURRENCE + 6 * HOUR;
        let clicked = ev.start_utc;
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(wire_new_series(clicked)),
            )
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "recurrence": [UNTIL_BEFORE_OCCURRENCE],
            })))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(wire_master(&[
                UNTIL_BEFORE_OCCURRENCE,
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "following",
            clicked,
            ev,
            "cal@x.com",
            "UTC",
            // The row's own summary and its own span: nothing the user touched.
            form("Standup", clicked, clicked + HOUR),
            "all",
            &client,
        )
        .await
        .expect("a split that re-anchors the tail must not be refused as a no-op");

        let sent = requests(&server).await;
        let post = sent
            .iter()
            .find(|r| r.method.as_str() == "POST")
            .unwrap_or_else(|| panic!("the tail was never created: {:?}", methods_and_paths(&sent)));
        let body: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
        assert_eq!(
            body["start"],
            timed_json(clicked, clicked + HOUR, "UTC").0,
            "the tail did not take the moved occurrence's own start, which is the whole of \
             what this split changes: {body}"
        );
        assert!(
            sent.iter().any(|r| r.method.as_str() == "PATCH"),
            "the original was never shortened: {:?}",
            methods_and_paths(&sent)
        );
    }

    /// The failure the ordering exists to make survivable. The tail is already
    /// on Google by the time the truncate fails, so the user has two
    /// overlapping series — and the error has to say so, because nothing else
    /// will: both series render, one on top of the other, and "save failed"
    /// would send them looking for an edit that did not happen instead of a
    /// duplicate that did.
    ///
    /// The local master must still carry its *original* rule and version. A
    /// write-back there would leave the store claiming a truncation Google
    /// never performed, and the next edit would then condition on an etag that
    /// does not exist.
    #[tokio::test]
    async fn a_failed_truncate_reports_the_leftover_duplicate() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(wire_new_series(OCCURRENCE)),
            )
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .respond_with(wiremock::ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        let err = update_via_client(
            &pool,
            "following",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (from here)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap_err();

        assert!(
            err.to_string().contains("two overlapping series"),
            "the leftover duplicate went unmentioned: {err}"
        );
        // Bound to `errors.rs`'s allowlist as well: a `.contains` check alone
        // stays green while the user reads the generic OPAQUE message and is
        // never told there is a duplicate to delete.
        assert_eq!(
            crate::errors::user_facing(&err),
            "the new series was created but the original could not be shortened — \
             you now have two overlapping series and should delete one"
        );

        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(
            row.recurrence.as_deref(),
            Some("RRULE:FREQ=WEEKLY"),
            "the store recorded a truncation Google refused"
        );
        assert_eq!(row.etag.as_deref(), Some("\"master-etag\""), "the master's version moved");
    }

    /// The gap that would have cost every guest the second half of their own
    /// series. A split is one meeting continuing; creating the tail without
    /// `attendees` does not leave the guest list blank for the organiser to
    /// notice, it removes everybody from every occurrence after the split —
    /// and the organiser, looking at the event they just edited, sees nothing
    /// wrong.
    ///
    /// Asserted off `received_requests` rather than only through the matcher
    /// in the ordering test, so this failure reads as a missing guest list
    /// instead of an unmatched request.
    #[tokio::test]
    async fn the_new_series_carries_the_masters_whole_guest_list_and_tells_them() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        // The local row carries a *different*, one-person guest list. A body
        // built from the stored copy rather than the master fails here.
        ev.attendees = vec![guest(true)];
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(wire_new_series(OCCURRENCE)),
            )
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(wire_master(&[
                UNTIL_BEFORE_OCCURRENCE,
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "following",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (from here)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();

        let sent = requests(&server).await;
        let post = sent.iter().find(|r| r.method.as_str() == "POST").expect("no create was sent");
        let body: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
        // `.get`, not `body["attendees"]`: `Value`'s `Index` answers `Null` for
        // a missing key, so indexing cannot tell "no guest list" from "an
        // empty one".
        assert!(
            body.get("attendees").is_some(),
            "the second half of the series was created with no guests at all: {body}"
        );
        assert_eq!(
            body["attendees"], expected_guests(),
            "the guest list did not round-trip: every field this drops is erased from the \
             real event, including other people's answers, comments and guest counts"
        );
        assert_eq!(
            post.url.query(),
            Some("sendUpdates=all"),
            "the tail was created without telling its guests, while the truncation mailed \
             them that the series had changed — half a story each"
        );
    }

    /// The truncation carries the ending and nothing else.
    ///
    /// `UNTIL` is inclusive, so it lands one second before the occurrence that
    /// moved to the new series: on it, and that occurrence exists twice; a
    /// second late, and it exists nowhere. And the body is `recurrence` alone
    /// — the user's edits belong to the tail, so a `summary` or `start` here
    /// would rewrite the *past* occurrences too and mail every guest about it.
    #[tokio::test]
    async fn the_original_is_truncated_to_end_one_second_before_the_split() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY", "EXDATE:20260817T090000Z"]).await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(wire_new_series(OCCURRENCE)),
            )
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(wire_master(&[
                UNTIL_BEFORE_OCCURRENCE,
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "following",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (from here)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();

        let sent = requests(&server).await;
        let patch = sent.iter().find(|r| r.method.as_str() == "PATCH").expect("no truncate");
        let body: serde_json::Value = serde_json::from_slice(&patch.body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "recurrence": [UNTIL_BEFORE_OCCURRENCE, "EXDATE:20260817T090000Z"],
            }),
            "the truncation either aimed at the wrong instant, rewrote the past occurrences \
             as well, or lost the series' EXDATE list"
        );
    }

    /// The other half of the same rule: an `EXDATE` names an occurrence
    /// somebody deleted, so it has to travel with the tail as well. Dropping
    /// it from the new series resurrects a meeting that was cancelled — and
    /// `sendUpdates=all` then invites everyone to it.
    ///
    /// Also the Repeat-was-touched arm: a chosen rule replaces the `RRULE`
    /// line and leaves every other line alone.
    #[tokio::test]
    async fn a_chosen_repeat_replaces_only_the_rule_and_the_exdates_travel_with_the_tail() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY", "EXDATE:20260817T090000Z"]).await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(wire_new_series(OCCURRENCE)),
            )
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(wire_master(&[
                UNTIL_BEFORE_OCCURRENCE,
            ])))
            .mount(&server)
            .await;

        let mut after = form("Standup (from here)", OCCURRENCE, OCCURRENCE + HOUR);
        after.recurrence = Some(Some("RRULE:FREQ=DAILY".into()));

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(&pool, "following", OCCURRENCE, ev, "cal@x.com", "UTC", after, "all", &client)
            .await
            .unwrap();

        let sent = requests(&server).await;
        let post = sent.iter().find(|r| r.method.as_str() == "POST").expect("no create");
        let body: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
        assert_eq!(
            body["recurrence"],
            serde_json::json!(["RRULE:FREQ=DAILY", "EXDATE:20260817T090000Z"]),
            "the chosen rule either did not replace the old one, or the EXDATE was dropped \
             and a cancelled occurrence came back: {body}"
        );
    }

    /// The new series is anchored *at* the clicked occurrence, so the form's
    /// instants are already in its own coordinates and go out untouched. Every
    /// other write in this file shifts them, because it is aimed at a resource
    /// anchored somewhere else; applying that shift here would move the tail by
    /// the distance between the master and the occurrence — two weeks, in this
    /// fixture — on top of the move the user actually made.
    #[tokio::test]
    async fn the_new_series_takes_the_forms_times_absolutely_rather_than_shifting_them() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;
        let moved = OCCURRENCE + 2 * HOUR;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(wire_new_series(moved)),
            )
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(wire_master(&[
                UNTIL_BEFORE_OCCURRENCE,
            ])))
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        // The user dragged this occurrence two hours later and chose "this and
        // following"; the clicked block is still `OCCURRENCE`.
        update_via_client(
            &pool,
            "following",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup", moved, moved + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();

        let sent = requests(&server).await;
        let post = sent.iter().find(|r| r.method.as_str() == "POST").expect("no create");
        let body: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
        let (start, end) = timed_json(moved, moved + HOUR, "UTC");
        assert_eq!(body["start"], start, "the tail did not start where the user put it: {body}");
        assert_eq!(body["end"], end);
        // The truncation is still aimed at the *clicked* occurrence, not at
        // where the user moved it to: everything before the block they acted on
        // must stay in the original series.
        let patch = sent.iter().find(|r| r.method.as_str() == "PATCH").expect("no truncate");
        let patched: serde_json::Value = serde_json::from_slice(&patch.body).unwrap();
        assert_eq!(patched, serde_json::json!({ "recurrence": [UNTIL_BEFORE_OCCURRENCE] }));
    }

    /// Splitting at the series' own first occurrence has nothing to keep on
    /// the original side. Truncating anyway leaves a master whose rule expands
    /// to no occurrences at all: an event Google still holds, that renders in
    /// no grid, that the user cannot see and therefore cannot delete — and one
    /// more of them every time they do it. "This and following" from the first
    /// occurrence *is* "all events", so that is what it does.
    #[tokio::test]
    async fn splitting_at_the_first_occurrence_edits_every_event_instead_of_orphaning_the_master() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .and(wiremock::matchers::body_json(
                serde_json::json!({"summary": "Standup (renamed)"}),
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(wire_master(&[
                "RRULE:FREQ=WEEKLY",
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "following",
            DTSTART, // the series' own first occurrence
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (renamed)", DTSTART, DTSTART + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();

        let sent = requests(&server).await;
        assert!(
            sent.iter().all(|r| r.method.as_str() != "POST"),
            "a second series was created with nothing left in the first"
        );
        assert!(
            sent.iter().all(|r| !r.url.path().ends_with("/instances")),
            "\"following\" from the first occurrence became \"this one\""
        );
    }

    /// `COUNT` and `UNTIL` are mutually exclusive, so the original converts
    /// cleanly — but the *tail* would need `COUNT` less however many
    /// occurrences the first half consumed, and only a full expansion of the
    /// rule knows that number. Guessing it wrong in one direction schedules a
    /// meeting nobody asked for and in the other quietly drops one; carrying
    /// `COUNT` across unchanged does the first, every time, by the width of
    /// the whole first half.
    ///
    /// So it stops, and it stops *before either write* — the strong part of
    /// the claim. Refusing after the create would leave exactly the duplicate
    /// this whole design is arranged to avoid.
    #[tokio::test]
    async fn a_series_that_ends_after_a_set_number_of_times_is_refused_before_any_write() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY;COUNT=10");
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY;COUNT=10"]).await;
        // Neither write is mounted: one arriving is a bare 404, which fails
        // the assertions below rather than passing quietly.

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        let err = update_via_client(
            &pool,
            "following",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (from here)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap_err();

        // What was *sent* is asserted before what was said, and the order is
        // for the diagnostic rather than the logic. Move the refusal below the
        // create and the unmounted POST answers 404, so the error becomes a
        // transport failure: asserting the message first reports `http error:
        // 404 Not Found`, which reads like a network fault instead of the
        // design violation it is.
        let sent = requests(&server).await;
        assert!(
            sent.iter().all(|r| matches!(r.method.as_str(), "GET")),
            "a refusal still wrote something — the check must run before either write: {:?}",
            methods_and_paths(&sent)
        );
        assert!(err.to_string().contains("set number of times"), "got: {err}");
        assert_eq!(
            crate::errors::user_facing(&err),
            "omacal cannot split a series that ends after a set number of times — \
             edit all events instead"
        );
        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.recurrence.as_deref(), Some("RRULE:FREQ=WEEKLY;COUNT=10"));
    }

    /// One occurrence of `master1` changed on its own: a separate Google event
    /// pointing back at the master, overriding the slot at `original_start`.
    /// `status` is a parameter because a *cancelled* exception is the one that
    /// looks harmless and is not — it is the only record that an occurrence was
    /// deleted, so losing it does not leave a gap, it brings a cancelled
    /// meeting back.
    fn exception_row(google_id: &str, original_start: i64, status: &str) -> omacal_store::StoredEvent {
        let mut ev = stored(vec![]);
        ev.google_id = google_id.into();
        ev.summary = Some("Standup".into());
        ev.recurring_event_id = Some("master1".into());
        ev.original_start_utc = Some(original_start);
        ev.start_utc = original_start;
        ev.end_utc = original_start + HOUR;
        ev.status = status.into();
        ev
    }

    /// Occurrences after the split point that somebody already moved or deleted
    /// belong to the *original* master, and truncating it takes them with it —
    /// Google drops every instance past `UNTIL`, materialised ones included.
    /// The new series is generated fresh from the rule and knows nothing about
    /// them, so a move is silently undone and a deletion silently reversed.
    ///
    /// That is the same class of loss the create-before-truncate ordering
    /// exists to prevent, and it cannot be shipped quietly for the same reason:
    /// the user cannot see what went, so they cannot put it back. It refuses
    /// **before either write** — refusing after the create would leave the
    /// duplicate this whole design is arranged to avoid — and names the count,
    /// so the user knows the scale of what they are being asked to redo.
    ///
    /// The fixture is deliberately mixed: one moved occurrence and one
    /// cancelled one after the split, and one moved occurrence *before* it that
    /// must not be counted, since it stays with the original series and is in
    /// no danger. A check that counted every exception of the master would pass
    /// a `> 0` assertion while refusing splits that are perfectly safe.
    #[tokio::test]
    async fn a_split_that_would_strand_moved_occurrences_is_refused_before_any_write() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;
        for mut row in [
            // Before the split: stays with the original, must not be counted.
            exception_row("master1_20260803T090000Z", OCCURRENCE - 7 * 24 * HOUR, "confirmed"),
            // After the split: both would be stranded.
            exception_row("master1_20260817T090000Z", OCCURRENCE + 7 * 24 * HOUR, "confirmed"),
            exception_row("master1_20260824T090000Z", OCCURRENCE + 14 * 24 * HOUR, "cancelled"),
        ] {
            row.calendar_id = ev.calendar_id;
            omacal_store::upsert_event(&pool, &row).await.unwrap();
        }

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;
        // Neither write is mounted: one arriving is a bare 404, which fails the
        // assertions below rather than passing quietly.

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        let err = update_via_client(
            &pool,
            "following",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (from here)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap_err();

        // Asserted before the message, for the same diagnostic reason as the
        // `COUNT` refusal above: with the check below the create, the unmounted
        // POST 404s and the message assertion would report a transport error
        // rather than naming the write that should never have happened.
        let sent = requests(&server).await;
        assert!(
            sent.iter().all(|r| r.method.as_str() == "GET"),
            "the split was refused but something was written anyway — the check must run \
             before either write: {:?}",
            methods_and_paths(&sent)
        );

        // The whole rendered string, not a `.contains`: it pins the count to
        // the two occurrences actually at risk (three would mean the one before
        // the split was swept in, one would mean the cancelled one was missed),
        // and binds the message to `errors.rs`'s prefix entry, whose safety
        // argument is that nothing but this number ever follows it.
        assert_eq!(
            crate::errors::user_facing(&err),
            "some later occurrences of this series were moved or deleted on their own, and a \
             split cannot carry them across — edit all events instead, or re-create them \
             afterwards. Occurrences affected: 2"
        );
        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.recurrence.as_deref(), Some("RRULE:FREQ=WEEKLY"));
    }

    /// The occurrence being split *at* must not be counted against itself.
    ///
    /// Dragging one occurrence and later splitting from it is ordinary, and
    /// that occurrence is not stranded: it becomes the first occurrence of the
    /// new series, carrying the form's values. Counting it refuses exactly the
    /// case the split handles best — and does so on a comparison (`>=` on the
    /// slot it overrides) that looks obviously right.
    ///
    /// One *other* exception sits further down the tail, so the split is still
    /// refused and the assertion is on the number: `1`, not `2`. Dropping the
    /// exclusion says `2`; dropping the whole check refuses nothing at all.
    #[tokio::test]
    async fn the_occurrence_being_split_at_is_not_counted_as_stranded_by_its_own_split() {
        let mut ev = exception_row("master1_20260810T090000Z", OCCURRENCE, "confirmed");
        ev.start_utc = OCCURRENCE + 5 * HOUR; // dragged, and the block clicked
        ev.end_utc = OCCURRENCE + 6 * HOUR;
        let clicked = ev.start_utc;
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let mut later =
            exception_row("master1_20260824T090000Z", OCCURRENCE + 14 * 24 * HOUR, "confirmed");
        later.calendar_id = ev.calendar_id;
        omacal_store::upsert_event(&pool, &later).await.unwrap();

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        let err = update_via_client(
            &pool,
            "following",
            clicked,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (from here)", clicked, clicked + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap_err();

        assert_eq!(
            crate::errors::user_facing(&err),
            "some later occurrences of this series were moved or deleted on their own, and a \
             split cannot carry them across — edit all events instead, or re-create them \
             afterwards. Occurrences affected: 1",
            "the occurrence the user split at was counted as a casualty of its own split"
        );
    }

    /// The other half of that rule, and the one that keeps it from being
    /// vacuous: exceptions belonging to a *different* series, and ones before
    /// the split point, must not block a split that strands nothing. A check
    /// keyed on the wrong column — or on no column at all — refuses every split
    /// on any calendar that has ever had an occurrence moved.
    #[tokio::test]
    async fn exceptions_of_other_series_and_earlier_occurrences_do_not_block_a_split() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let mut earlier =
            exception_row("master1_20260803T090000Z", OCCURRENCE - 7 * 24 * HOUR, "confirmed");
        earlier.calendar_id = ev.calendar_id;
        omacal_store::upsert_event(&pool, &earlier).await.unwrap();

        // A different series entirely, with an occurrence moved well past the
        // split point.
        let mut other = exception_row("other_20260824T090000Z", OCCURRENCE + 14 * 24 * HOUR, "confirmed");
        other.recurring_event_id = Some("other-master".into());
        other.calendar_id = ev.calendar_id;
        omacal_store::upsert_event(&pool, &other).await.unwrap();

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(wire_new_series(OCCURRENCE)),
            )
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "recurrence": [UNTIL_BEFORE_OCCURRENCE],
            })))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(wire_master(&[
                UNTIL_BEFORE_OCCURRENCE,
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "following",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (from here)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .expect("a split that strands nothing must not be refused");
    }

    /// RFC 5545 §3.3.10: `UNTIL` must carry the same value type as `DTSTART`.
    /// An all-day series' `DTSTART` is a bare date, so its `UNTIL` is one too
    /// — and a date-valued `UNTIL` is inclusive of the day it names, so the
    /// ending is the *previous* date, not the previous second.
    ///
    /// The zone that date is read in is the calendar's, per [`edit_zone`]'s
    /// all-day arm. `Pacific/Auckland` is UTC+12, so its midnight falls on the
    /// previous date in UTC: a truncation that read the instant in UTC would
    /// end the series a day early and take an occurrence the user meant to
    /// keep.
    ///
    /// This fixture has the master and the tail both all-day, so it says
    /// nothing about *where* the value type came from — the two candidate
    /// expressions agree here. `the_until_follows_the_masters_value_type_not_the_new_series`
    /// is the one that separates them; this is the ordinary shape.
    #[tokio::test]
    async fn splitting_an_all_day_series_ends_it_on_the_previous_date() {
        let midnight = |wall: &str| {
            wall.parse::<jiff::civil::DateTime>()
                .unwrap()
                .in_tz("Pacific/Auckland")
                .unwrap()
                .timestamp()
                .as_millisecond()
        };
        let dtstart = midnight("2026-07-27T00:00:00");
        let occurrence = midnight("2026-08-10T00:00:00");
        assert_eq!(
            jiff::Timestamp::from_millisecond(occurrence).unwrap().to_string(),
            "2026-08-09T12:00:00Z",
            "fixture check: this instant must fall on the previous date in UTC, or the \
             assertion below proves nothing"
        );

        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        ev.is_all_day = true;
        ev.start_utc = dtstart;
        ev.end_utc = dtstart + 24 * HOUR;
        ev.start_tz = "Pacific/Auckland".into();
        ev.end_tz = "Pacific/Auckland".into();
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "Pacific/Auckland").await;

        let all_day_master = serde_json::json!({
            "id": "master1", "status": "confirmed", "etag": "\"m1\"",
            "summary": "On call",
            "recurrence": ["RRULE:FREQ=WEEKLY"],
            "start": {"date": "2026-07-27"},
            "end":   {"date": "2026-07-28"}
        });
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(all_day_master.clone()),
            )
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "tail1", "status": "confirmed", "etag": "\"t1\"",
                "summary": "On call",
                "recurrence": ["RRULE:FREQ=WEEKLY"],
                "start": {"date": "2026-08-10"},
                "end":   {"date": "2026-08-11"}
            })))
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(all_day_master),
            )
            .expect(1)
            .mount(&server)
            .await;

        let after = all_day_form("On call", "2026-08-10", "2026-08-11");

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "following",
            occurrence,
            ev,
            "cal@x.com",
            "Pacific/Auckland",
            after,
            "all",
            &client,
        )
        .await
        .unwrap();

        let sent = requests(&server).await;
        let patch = sent.iter().find(|r| r.method.as_str() == "PATCH").expect("no truncate");
        let body: serde_json::Value = serde_json::from_slice(&patch.body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({ "recurrence": ["RRULE:FREQ=WEEKLY;UNTIL=20260809"] }),
            "an all-day series was ended with a date-time UNTIL, on the wrong day, or both"
        );
        let post = sent.iter().find(|r| r.method.as_str() == "POST").expect("no create");
        let created: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
        assert_eq!(
            created["start"],
            serde_json::json!({"date": "2026-08-10"}),
            "the tail's own start disagrees with the date the original was ended on"
        );
    }

    /// **Where the `UNTIL`'s value type comes from**, in the one configuration
    /// that can tell: an all-day series split into a *timed* remainder.
    ///
    /// Every other fixture in this file has the master and the tail agreeing on
    /// their shape, so `master_row.is_all_day` and `after.when.is_all_day()`
    /// are the same value and either one produces a passing test. They are not
    /// the same rule. The truncation patches `recurrence` alone, so the master
    /// keeps the bare-date `start` it already had, and RFC 5545 requires its
    /// `UNTIL` to stay a bare date — whatever the user chose for the new event.
    ///
    /// `when` is a required absolute field on every `updateEvent` call, so this
    /// is reachable the moment the UI ships the scope: toggle "All day" off,
    /// pick "this and following", and a rule built from the form's shape puts
    /// `UNTIL=20260809T115959Z` on a series whose `DTSTART` is `VALUE=DATE`.
    #[tokio::test]
    async fn the_until_follows_the_masters_value_type_not_the_new_series() {
        let midnight = |wall: &str| {
            wall.parse::<jiff::civil::DateTime>()
                .unwrap()
                .in_tz("Pacific/Auckland")
                .unwrap()
                .timestamp()
                .as_millisecond()
        };
        let dtstart = midnight("2026-07-27T00:00:00");
        let occurrence = midnight("2026-08-10T00:00:00");

        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        ev.is_all_day = true;
        ev.start_utc = dtstart;
        ev.end_utc = dtstart + 24 * HOUR;
        ev.start_tz = "Pacific/Auckland".into();
        ev.end_tz = "Pacific/Auckland".into();
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "Pacific/Auckland").await;

        let all_day_master = serde_json::json!({
            "id": "master1", "status": "confirmed", "etag": "\"m1\"",
            "summary": "On call",
            "recurrence": ["RRULE:FREQ=WEEKLY"],
            "start": {"date": "2026-07-27"},
            "end":   {"date": "2026-07-28"}
        });
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(all_day_master.clone()),
            )
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "tail1", "status": "confirmed", "etag": "\"t1\"",
                "summary": "On call",
                "recurrence": ["RRULE:FREQ=WEEKLY"],
                "start": {"dateTime": omacal_sync::to_rfc3339(occurrence + 9 * HOUR)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(occurrence + 10 * HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(all_day_master))
            .expect(1)
            .mount(&server)
            .await;

        // The user turned "All day" off and gave the remainder real times.
        let after = form("On call", occurrence + 9 * HOUR, occurrence + 10 * HOUR);
        assert!(
            !after.when.is_all_day(),
            "fixture check: the tail must be timed, or this proves nothing"
        );

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "following",
            occurrence,
            ev,
            "cal@x.com",
            "Pacific/Auckland",
            after,
            "all",
            &client,
        )
        .await
        .unwrap();

        let sent = requests(&server).await;
        let patch = sent.iter().find(|r| r.method.as_str() == "PATCH").expect("no truncate");
        let body: serde_json::Value = serde_json::from_slice(&patch.body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({ "recurrence": ["RRULE:FREQ=WEEKLY;UNTIL=20260809"] }),
            "the UNTIL was built from the *new* series' all-day flag: an all-day master \
             ended with a date-time UNTIL, which RFC 5545 forbids and other clients reject"
        );
        // The same request pair, disagreeing on purpose: the tail really is
        // timed. Without this the assertion above could pass on a fixture where
        // nothing diverged after all.
        let post = sent.iter().find(|r| r.method.as_str() == "POST").expect("no create");
        let created: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
        assert!(
            created["start"].get("dateTime").is_some(),
            "fixture check: the tail must go out timed: {created}"
        );
    }

    /// The mirror, so the rule is bound in both directions rather than only
    /// where "bare date" happens to be the answer: a *timed* series split into
    /// an all-day remainder keeps its date-time `UNTIL`.
    ///
    /// Without this, a truncation hardcoded to the all-day form — or one that
    /// read the tail's flag — would still pass the test above.
    #[tokio::test]
    async fn a_timed_master_keeps_a_date_time_until_when_the_tail_is_all_day() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "tail1", "status": "confirmed", "etag": "\"t1\"",
                "summary": "Standup (from here)",
                "recurrence": ["RRULE:FREQ=WEEKLY"],
                "start": {"date": "2026-08-10"},
                "end":   {"date": "2026-08-11"}
            })))
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(wire_master(&[
                UNTIL_BEFORE_OCCURRENCE,
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let after = all_day_form("Standup (from here)", "2026-08-10", "2026-08-11");

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(&pool, "following", OCCURRENCE, ev, "cal@x.com", "UTC", after, "all", &client)
            .await
            .unwrap();

        let sent = requests(&server).await;
        let patch = sent.iter().find(|r| r.method.as_str() == "PATCH").expect("no truncate");
        let body: serde_json::Value = serde_json::from_slice(&patch.body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({ "recurrence": [UNTIL_BEFORE_OCCURRENCE] }),
            "a timed master was ended with a bare-date UNTIL because the *new* series is \
             all-day: the value types must match the rule's own DTSTART"
        );
        let post = sent.iter().find(|r| r.method.as_str() == "POST").expect("no create");
        let created: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
        assert_eq!(
            created["start"],
            serde_json::json!({"date": "2026-08-10"}),
            "fixture check: the tail must go out all-day, or the two flags never diverged"
        );
    }

    /// The same split, driven from a materialised exception row instead of the
    /// master. The rule, the guest list and the version all have to come from
    /// the master — the exception has a rule of its own (none), a guest list
    /// that may differ, and an etag that would only ever be rejected.
    ///
    /// And the exception's own row must be left alone: `upsert_event` is keyed
    /// on `(calendar_id, google_id)`, so folding the master's new state back
    /// here would write straight over a different event.
    #[tokio::test]
    async fn splitting_from_an_exception_row_reads_the_master_and_leaves_that_row_alone() {
        let mut ev = stored(vec![]);
        ev.google_id = "master1_20260810T090000Z".into();
        ev.summary = Some("Standup".into());
        ev.recurring_event_id = Some("master1".into());
        ev.start_utc = OCCURRENCE;
        ev.end_utc = OCCURRENCE + HOUR;
        ev.etag = Some("\"exc\"".into());
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(wire_new_series(OCCURRENCE)),
            )
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .and(wiremock::matchers::header("if-match", "\"m1\""))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "recurrence": [UNTIL_BEFORE_OCCURRENCE],
            })))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(wire_master(&[
                UNTIL_BEFORE_OCCURRENCE,
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "following",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (from here)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();

        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.google_id, "master1_20260810T090000Z");
        assert_eq!(
            row.etag.as_deref(),
            Some("\"exc\""),
            "the master's version was stamped onto the exception's row"
        );
        assert_eq!(
            row.recurrence, None,
            "the master's truncated rule was written onto the exception's row"
        );
    }

    /// Splitting at an occurrence somebody had already dragged somewhere else.
    ///
    /// `UNTIL` is compared against the instants the *rule* generates, so the
    /// truncation has to be aimed at the occurrence's slot on that grid — its
    /// `originalStartTime` — and not at where it is now rendered. This fixture
    /// moved the 2026-08-10 occurrence five hours later; truncating there
    /// leaves `UNTIL` after the 09:00 slot the rule still generates, so the
    /// shortened series keeps an occurrence the user split away and it shows up
    /// underneath the new one. Dragged the other way it is the mirror image: an
    /// occurrence that should have stayed disappears from both series.
    #[tokio::test]
    async fn splitting_at_a_moved_occurrence_truncates_at_its_slot_not_at_where_it_was_dragged() {
        let mut ev = stored(vec![]);
        ev.google_id = "master1_20260810T090000Z".into();
        ev.summary = Some("Standup".into());
        ev.recurring_event_id = Some("master1".into());
        ev.start_utc = OCCURRENCE + 5 * HOUR; // dragged five hours later
        ev.end_utc = OCCURRENCE + 6 * HOUR;
        ev.original_start_utc = Some(OCCURRENCE); // the slot the rule generates
        ev.etag = Some("\"exc\"".into());
        let clicked = ev.start_utc;
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(wire_new_series(clicked)),
            )
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(wire_master(&[
                UNTIL_BEFORE_OCCURRENCE,
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "following",
            clicked,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (from here)", clicked, clicked + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();

        let sent = requests(&server).await;
        let patch = sent.iter().find(|r| r.method.as_str() == "PATCH").expect("no truncate");
        let body: serde_json::Value = serde_json::from_slice(&patch.body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({ "recurrence": [UNTIL_BEFORE_OCCURRENCE] }),
            "the truncation was aimed at where the occurrence was dragged to, so the rule \
             still generates the slot it came from"
        );
        // The tail, by contrast, starts where the user actually wants it —
        // the moved time, not the slot.
        let post = sent.iter().find(|r| r.method.as_str() == "POST").expect("no create");
        let created: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
        assert_eq!(created["start"], timed_json(clicked, clicked + HOUR, "UTC").0);
    }

    /// The same rule at the other end of the series: an occurrence dragged
    /// later is still the *first* occurrence if its slot is the series' own
    /// start, so "this and following" from it is still "all events". Aiming
    /// the check at the dragged position instead splits the series and leaves
    /// behind a master that expands to nothing — invisible, and undeletable
    /// from this app.
    #[tokio::test]
    async fn a_dragged_first_occurrence_is_still_the_first_occurrence() {
        let mut ev = stored(vec![]);
        ev.google_id = "master1_20260727T090000Z".into();
        ev.summary = Some("Standup".into());
        ev.recurring_event_id = Some("master1".into());
        ev.start_utc = DTSTART + 5 * HOUR; // the first occurrence, dragged
        ev.end_utc = DTSTART + 6 * HOUR;
        ev.original_start_utc = Some(DTSTART);
        let clicked = ev.start_utc;
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .and(wiremock::matchers::body_json(
                serde_json::json!({"summary": "Standup (renamed)"}),
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(wire_master(&[
                "RRULE:FREQ=WEEKLY",
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "following",
            clicked,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (renamed)", clicked, clicked + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();

        assert!(
            requests(&server).await.iter().all(|r| r.method.as_str() != "POST"),
            "the series was split at its own first occurrence, leaving a master that expands \
             to nothing"
        );
    }

    /// A truncation Google confirms in terms this store cannot read must leave
    /// the local row alone rather than half-updating it.
    ///
    /// The obvious fallback — [`merge_patched`], which the two sibling write
    /// paths use — is wrong *here specifically*, and quietly: it carries etag,
    /// sequence and attendees, and this is the one write whose whole payload is
    /// `recurrence`, which it does not carry. Applying it would leave the row
    /// stamped with the new version while still holding the **untruncated**
    /// rule: the grid would go on expanding the series straight through the
    /// split, and the next edit would condition on an etag whose rule the store
    /// does not have. Both writes have landed by this point, so the honest
    /// answer is to leave the row for sync and report success.
    #[tokio::test]
    async fn a_truncation_this_store_cannot_read_leaves_the_local_row_for_sync() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(wire_new_series(OCCURRENCE)),
            )
            .mount(&server)
            .await;
        // Accepted, and answered with a version this store cannot turn into a
        // row: the times will not resolve. `to_stored` answers `None`.
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "master1", "status": "confirmed", "etag": "\"m2\"", "sequence": 9,
                "recurrence": [UNTIL_BEFORE_OCCURRENCE],
                "start": {}, "end": {}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "following",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Standup (from here)", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .expect("both writes landed, so this must not report a failure");

        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(
            row.etag.as_deref(),
            Some("\"master-etag\""),
            "the row took the new version without the rule that came with it: the grid still \
             expands past the split, and the next edit conditions on an etag whose rule the \
             store does not have"
        );
        assert_eq!(row.recurrence.as_deref(), Some("RRULE:FREQ=WEEKLY"));
        assert_eq!(row.sequence, 1, "sequence moved without the rule moving with it");
    }

    /// A row belonging to no series has no "following": there is one event,
    /// and editing it and everything after it is editing it. It must not
    /// become an instances lookup — `target_event_id` reads every scope that
    /// is not `"all"` as "this one" — and it must not try to split anything.
    #[tokio::test]
    async fn following_on_a_one_off_edits_that_event_and_looks_up_no_instances() {
        let mut ev = stored(vec![]);
        ev.summary = Some("Lunch".into());
        ev.start_utc = OCCURRENCE;
        ev.end_utc = OCCURRENCE + HOUR;
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/ev1"))
            .and(wiremock::matchers::body_json(serde_json::json!({"summary": "Brunch"})))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ev1", "status": "confirmed", "etag": "\"e2\"",
                "summary": "Brunch",
                "start": {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE)},
                "end":   {"dateTime": omacal_sync::to_rfc3339(OCCURRENCE + HOUR)}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        update_via_client(
            &pool,
            "following",
            OCCURRENCE,
            ev,
            "cal@x.com",
            "UTC",
            form("Brunch", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
            &client,
        )
        .await
        .unwrap();

        let sent = requests(&server).await;
        assert_eq!(sent.len(), 1, "a one-off split fetched or created something: {sent:?}");
    }

    /// The guard ordering, for the scope this task adds. `update_impl` reads
    /// the real `~/.config/omacal/config.toml` and the real Keychain the
    /// moment control reaches `load_config`, and then writes to a real
    /// calendar with `sendUpdates=all` — twice, one of them a create. The demo
    /// gate has to fire before any of that, and before the database is touched
    /// at all: `id: 1` names no row on a bare `connect_memory` pool, so a gate
    /// that ran second would report a missing event instead.
    #[tokio::test]
    async fn splitting_refuses_in_demo_mode_without_touching_config_keyring_or_google() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let err = update_impl(
            &state_with(pool, true),
            1,
            "following",
            OCCURRENCE,
            form("Standup", OCCURRENCE, OCCURRENCE + HOUR),
            "all",
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("demo"), "got: {err}");
        assert_eq!(crate::errors::user_facing(&err), "demo mode — there is nothing to save");
    }

    /// [`attendees_verbatim`] on its own, where the two properties that make
    /// it different from [`attendees_with_self_response`] are visible: nobody's
    /// answer changes, and a list with no `self` row still comes back whole.
    /// Answering `None` there — the sibling's rule — would drop the guest list
    /// of every series on a calendar shared with you.
    #[test]
    fn a_copied_guest_list_keeps_everybodys_answer_and_survives_having_no_self_row() {
        let list = vec![
            Attendee {
                email: "bo@x.com".into(),
                display_name: None,
                response_status: "declined".into(),
                optional: true,
                is_self: false,
                comment: Some("double-booked".into()),
                additional_guests: 2,
            },
            Attendee {
                email: "cy@x.com".into(),
                display_name: Some("Cy".into()),
                response_status: "accepted".into(),
                optional: false,
                is_self: false,
                comment: None,
                additional_guests: 0,
            },
        ];
        assert!(attendees_with_self_response(&list, "accepted").is_none());

        let out = attendees_verbatim(&list);
        assert_eq!(
            serde_json::json!(out),
            serde_json::json!([
                {"email": "bo@x.com", "responseStatus": "declined", "optional": true,
                 "comment": "double-booked", "additionalGuests": 2},
                {"email": "cy@x.com", "displayName": "Cy", "responseStatus": "accepted",
                 "optional": false, "additionalGuests": 0}
            ]),
            "a copied guest list must round-trip every writable field and change no answers"
        );
    }

    // --- delete_event_cmd: `delete_impl` / `delete_via_client`. The one verb
    // whose mistakes cannot be undone — there is no copy of a deleted series
    // this app can reach, and every request below goes out with
    // `sendUpdates=all`.
    //
    // **Every mock here carries `.expect(1)`, and that is not decoration.**
    // `CalendarClient::delete_event` treats `404` as success, correctly: an
    // event that is already gone is what the caller asked for. `wiremock`
    // answers every *unmatched* request with `404` as well. So a delete aimed at
    // the wrong path, or with `sendUpdates` dropped, returns `Ok` and leaves
    // every assertion about the outcome intact — an expectation, or
    // `received_requests`, is the only thing that can see it happen.

    /// A `DELETE` on one event path, matching `sendUpdates=all`.
    ///
    /// The query parameter is matched rather than assumed: it is what tells the
    /// guest list their meeting is cancelled, and a delete that quietly stopped
    /// sending it would leave a room full of people with a calendar entry for a
    /// meeting nobody is coming to.
    async fn mount_delete(server: &wiremock::MockServer, path: &str) {
        wiremock::Mock::given(wiremock::matchers::method("DELETE"))
            .and(wiremock::matchers::path(path))
            .and(wiremock::matchers::query_param("sendUpdates", "all"))
            .respond_with(wiremock::ResponseTemplate::new(204))
            .expect(1)
            .mount(server)
            .await;
    }

    /// The shortened master as Google answers the truncating `PATCH`, with a
    /// version (`"m2"`) `wire_master`'s `"m1"` can be told from — so a
    /// write-back that never happened shows up in the local row's etag as well
    /// as in its rule.
    fn wire_shortened_master(rules: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "id": "master1", "status": "confirmed", "etag": "\"m2\"",
            "summary": "Standup",
            "recurrence": rules,
            "attendees": wire_guests(),
            "start": {"dateTime": omacal_sync::to_rfc3339(DTSTART)},
            "end":   {"dateTime": omacal_sync::to_rfc3339(DTSTART + HOUR)}
        })
    }

    /// Whether one `METHOD /path` was among the requests the server saw. For
    /// the assertions that are about a request which must **never** have been
    /// sent, where `.expect(0)` cannot help: an unmounted path is answered with
    /// a bare `404`, which this app reads as a successful delete.
    fn was_sent(sent: &[wiremock::Request], method_and_path: &str) -> bool {
        methods_and_paths(sent).iter().any(|r| r == method_and_path)
    }

    /// The body of the truncating `PATCH`, as JSON.
    ///
    /// The mocks below match on the body as well as the path, which is what
    /// keeps a wrong rule from being answered `200` — and which makes their
    /// failures unreadable on its own: the call comes back `http error: 404 Not
    /// Found` and never says which rule went out. Every truncation test
    /// therefore asserts this *before* unwrapping the call's own result, so a
    /// regression reports the two rules side by side.
    fn the_patch(sent: &[wiremock::Request]) -> &wiremock::Request {
        sent.iter()
            .find(|r| r.method.as_str() == "PATCH")
            .unwrap_or_else(|| panic!("no truncation was sent: {:?}", methods_and_paths(sent)))
    }

    fn patch_body(sent: &[wiremock::Request]) -> serde_json::Value {
        serde_json::from_slice(&the_patch(sent).body).expect("the truncation's body was not JSON")
    }

    /// One request header as a string, for the same diagnostic reason as
    /// [`patch_body`]: the mock matches `If-Match`, so a truncation conditioned
    /// on the wrong version is answered `404` and reports only that.
    fn header_of(req: &wiremock::Request, name: &str) -> Option<String> {
        req.headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
    }

    /// The instance lookup as it actually went out, and one of its query
    /// parameters.
    ///
    /// Same diagnostic reason again, for the value this task made it possible to
    /// get wrong: `delete_via_client` now holds *two* instants — the clicked
    /// block's `occurrence_start_ms` and the rule's own `split_at_ms` — and
    /// bracketing the lookup by the second resolves a dragged occurrence to
    /// whatever sits at the slot it left. The mock matches `timeMin`, so that
    /// swap is answered `404` and reported as `http error: 404 Not Found`, which
    /// is true and says nothing about the window. Asserting it first names it.
    fn the_lookup(sent: &[wiremock::Request]) -> &wiremock::Request {
        sent.iter()
            .find(|r| r.url.path().ends_with("/instances"))
            .unwrap_or_else(|| panic!("no instance lookup was sent: {:?}", methods_and_paths(sent)))
    }

    fn query_param_of(req: &wiremock::Request, key: &str) -> Option<String> {
        req.url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned())
    }

    const MASTER_PATH: &str = "/calendars/cal%40x.com/events/master1";
    const INSTANCE_ID: &str = "master1_20260810T090000Z";

    /// The defect the whole resolution path exists to prevent, on the verb
    /// where it is unrecoverable. "This one" must delete the instance id Google
    /// returns; deleting `master1` instead is not a larger version of the same
    /// thing, it removes **every occurrence of the series, past ones included**,
    /// and mails a cancellation to the whole guest list.
    ///
    /// No `DELETE` is mounted on the master, and that alone would prove nothing:
    /// an unmatched request is answered `404`, which `delete_event` reports as
    /// success. `received_requests` is what sees it.
    #[tokio::test]
    async fn deleting_one_occurrence_deletes_the_instance_not_the_master() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        // Bracketed by the *clicked* occurrence, never by `ev.start_utc`: the
        // query params are matched, so a window derived from the master's own
        // start misses this mock, and the empty answer that follows would
        // resolve to nothing at all.
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1/instances"))
            .and(wiremock::matchers::query_param("timeMin", omacal_sync::to_rfc3339(OCCURRENCE)))
            .and(wiremock::matchers::query_param(
                "timeMax",
                omacal_sync::to_rfc3339(OCCURRENCE + 1000),
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [wire_occurrence(INSTANCE_ID, OCCURRENCE, OCCURRENCE + HOUR, "\"i1\"")]
            })))
            .expect(1)
            .mount(&server)
            .await;
        mount_delete(&server, &format!("/calendars/cal%40x.com/events/{INSTANCE_ID}")).await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        delete_via_client(&pool, "this", OCCURRENCE, ev, "cal@x.com", "UTC", &client)
            .await
            .unwrap();

        let sent = requests(&server).await;
        assert!(
            !was_sent(&sent, &format!("DELETE {MASTER_PATH}")),
            "one occurrence was deleted by deleting the whole series: {:?}",
            methods_and_paths(&sent)
        );

        // The master's row stays exactly as it was: only Google knows that one
        // occurrence is gone, and this store has no row for the cancelled
        // exception it has just materialised.
        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.recurrence.as_deref(), Some("RRULE:FREQ=WEEKLY"));
        assert_eq!(row.status, "confirmed");
        assert_eq!(row.etag.as_deref(), Some("\"master-etag\""));
    }

    /// "All events" targets the master directly and asks Google nothing first —
    /// there is no occurrence to resolve, and an instances lookup here would be
    /// a request spent to answer a question nobody asked.
    ///
    /// The local row goes with it. Unlike the occurrence above, nothing is left
    /// for sync to discover: this store knows the series is gone because it is
    /// the thing that deleted it.
    #[tokio::test]
    async fn deleting_all_deletes_the_master() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_delete(&server, MASTER_PATH).await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        delete_via_client(&pool, "all", OCCURRENCE, ev, "cal@x.com", "UTC", &client)
            .await
            .unwrap();

        let sent = requests(&server).await;
        assert_eq!(
            methods_and_paths(&sent),
            vec![format!("DELETE {MASTER_PATH}")],
            "deleting a whole series resolved an occurrence it had no use for"
        );
        assert!(
            omacal_store::event_by_id(&pool, id).await.unwrap().is_none(),
            "the series was deleted on Google and the local row was left behind, so the grid \
             goes on expanding a series that no longer exists"
        );
    }

    /// "This and following" is a **truncation, never a delete**, and this is the
    /// test that says so.
    ///
    /// The occurrences before the split live in the same Google event as the
    /// ones after it. Deleting the master to remove the tail therefore also
    /// removes every meeting that already happened — the one outcome the user
    /// chose this scope in order to avoid — and mails a cancellation for all of
    /// them.
    ///
    /// No `DELETE` is mounted anywhere, which on its own proves nothing:
    /// `wiremock` answers an unmatched request with `404` and `delete_event`
    /// reads `404` as success, so a regression to a delete would pass every
    /// other assertion here. The `received_requests` check is the one that
    /// catches it, and the `PATCH` expectation is what catches the mirror image
    /// — a truncation that never went out at all.
    #[tokio::test]
    async fn deleting_following_truncates_and_never_issues_a_delete() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;
        // Body *and* `If-Match` matched: the version comes from the master
        // Google just described, not from the local row (`"master-etag"`), and
        // the body is the shortened rule and nothing else — a stray `start`,
        // `summary` or `attendees` fails here rather than quietly rewriting the
        // half of the series the user is keeping.
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path(MASTER_PATH))
            .and(wiremock::matchers::header("if-match", "\"m1\""))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "recurrence": [UNTIL_BEFORE_OCCURRENCE],
            })))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(wire_shortened_master(&[UNTIL_BEFORE_OCCURRENCE])),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        let outcome =
            delete_via_client(&pool, "following", OCCURRENCE, ev, "cal@x.com", "UTC", &client)
                .await;

        let sent = requests(&server).await;
        assert!(
            !sent.iter().any(|r| r.method.as_str() == "DELETE"),
            "deleting from one occurrence onwards deleted the event the earlier occurrences \
             live in: {:?}",
            methods_and_paths(&sent)
        );
        assert_eq!(patch_body(&sent), serde_json::json!({ "recurrence": [UNTIL_BEFORE_OCCURRENCE] }));
        assert_eq!(
            header_of(the_patch(&sent), "if-match").as_deref(),
            Some("\"m1\""),
            "the truncation was conditioned on the local row's version (\"master-etag\") rather \
             than on the master Google had just described, so a concurrent edit either goes \
             unnoticed or rejects a write that was fine"
        );
        outcome.unwrap();

        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(
            row.recurrence.as_deref(),
            Some(UNTIL_BEFORE_OCCURRENCE),
            "the local master still expands past the point the user deleted from"
        );
        assert_eq!(row.etag.as_deref(), Some("\"m2\""));
    }

    /// A one-off has one Google event and one local row, and both go.
    ///
    /// The `.expect(1)` on the mock is the whole test on the wire side: with a
    /// `404` reading as success there is no other way to tell a delete that
    /// happened from one that never left.
    #[tokio::test]
    async fn deleting_removes_the_local_row() {
        let mut ev = stored(vec![]);
        ev.summary = Some("Lunch".into());
        ev.start_utc = OCCURRENCE;
        ev.end_utc = OCCURRENCE + HOUR;
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_delete(&server, "/calendars/cal%40x.com/events/ev1").await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        delete_via_client(&pool, "this", OCCURRENCE, ev, "cal@x.com", "UTC", &client)
            .await
            .unwrap();

        let sent = requests(&server).await;
        assert_eq!(
            sent.len(),
            1,
            "a one-off delete resolved instances it has none of: {:?}",
            methods_and_paths(&sent)
        );
        assert!(
            omacal_store::event_by_id(&pool, id).await.unwrap().is_none(),
            "the event was deleted on Google but its row stayed, so it goes on rendering"
        );
    }

    /// Demo mode must reach neither Google nor the real database, on this verb
    /// most of all: past the gate this reads the real
    /// `~/.config/omacal/config.toml`, then the Keychain, and then **deletes
    /// from a real calendar with `sendUpdates=all`**, against whatever account
    /// the demo database happens to name.
    ///
    /// Asserted in both directions. On a bare pool `id: 1` names no row at all,
    /// so a gate that ran after the lookup would report "that event is no longer
    /// here" instead — that is the ordering. With a real row present, the row is
    /// still there afterwards — that is the effect.
    #[tokio::test]
    async fn deleting_refuses_in_demo_mode() {
        let bare = omacal_store::connect_memory().await.unwrap();
        let err = delete_impl(&state_with(bare, true), 1, "all", OCCURRENCE).await.unwrap_err();
        assert!(err.to_string().contains("demo"), "got: {err}");
        // Binds the emitter to `errors.rs`'s allowlist, so the two literals
        // cannot drift apart while the user quietly starts reading OPAQUE.
        assert_eq!(crate::errors::user_facing(&err), "demo mode — there is nothing to delete");

        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;
        let err = delete_impl(&state_with(pool.clone(), true), id, "all", OCCURRENCE)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("demo"), "got: {err}");
        assert!(
            omacal_store::event_by_id(&pool, id).await.unwrap().is_some(),
            "demo mode deleted a row"
        );
    }

    /// The empty-lookup rule, on the verb that makes it matter most.
    ///
    /// `master == fallback` is the shape produced when the clicked row *is* the
    /// series master. Falling back to it there does not delete "one occurrence,
    /// approximately" — it deletes the entire series. And nothing downstream
    /// would notice: a `404` from a delete is success, so the widening would be
    /// reported as a job well done.
    ///
    /// No `DELETE` is mounted, so a regression is caught by
    /// `received_requests` rather than by a failing request.
    #[tokio::test]
    async fn an_empty_lookup_on_a_bare_master_is_refused_rather_than_deleting_the_series() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1/instances"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"items": []})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        let outcome =
            delete_via_client(&pool, "this", OCCURRENCE, ev, "cal@x.com", "UTC", &client).await;

        // Asserted before the error, and the order is the diagnostic. Under a
        // regression to the fallback the delete *succeeds* — a `404` on an
        // unmounted path is success to `delete_event` — so reading the `Err`
        // first reports only "unwrap_err on an Ok value" and never names the
        // request that should not have been sent.
        let sent = requests(&server).await;
        assert!(
            !sent.iter().any(|r| r.method.as_str() == "DELETE"),
            "an unresolvable occurrence was widened into a delete: {:?}",
            methods_and_paths(&sent)
        );
        let err = outcome.unwrap_err();
        assert_eq!(
            crate::errors::user_facing(&err),
            "could not find that occurrence on the calendar"
        );
        assert!(omacal_store::event_by_id(&pool, id).await.unwrap().is_some());
    }

    /// Deleting an occurrence somebody had already moved, where the local row
    /// **is** the resource being deleted.
    ///
    /// The row must be marked cancelled and *not* removed, and the difference is
    /// visible rather than bookkeeping: a cancelled exception is the only record
    /// that a slot of the series is empty (`commands::suppressed_slots`).
    /// Deleting the row instead lets the master expand straight back into that
    /// slot, so the occurrence the user just deleted reappears on the grid — and
    /// it is the one thing a delete may never do.
    #[tokio::test]
    async fn deleting_a_moved_occurrence_cancels_its_row_rather_than_removing_it() {
        let mut ev = exception_row(INSTANCE_ID, OCCURRENCE, "confirmed");
        ev.start_utc = OCCURRENCE + 5 * HOUR; // dragged, and the block clicked
        ev.end_utc = OCCURRENCE + 6 * HOUR;
        let clicked = ev.start_utc;
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1/instances"))
            .and(wiremock::matchers::query_param("timeMin", omacal_sync::to_rfc3339(clicked)))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [wire_occurrence(INSTANCE_ID, clicked, clicked + HOUR, "\"i1\"")]
            })))
            .expect(1)
            .mount(&server)
            .await;
        mount_delete(&server, &format!("/calendars/cal%40x.com/events/{INSTANCE_ID}")).await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        let outcome =
            delete_via_client(&pool, "this", clicked, ev, "cal@x.com", "UTC", &client).await;

        // Before the result: this row is dragged, so the clicked block and the
        // rule's slot are five hours apart and a lookup bracketed by the wrong
        // one is only visible here.
        let sent = requests(&server).await;
        assert_eq!(
            query_param_of(the_lookup(&sent), "timeMin"),
            Some(omacal_sync::to_rfc3339(clicked)),
            "the lookup was bracketed by the occurrence's slot rather than by where its block \
             is drawn, so a dragged occurrence resolves to whatever now sits at its old time"
        );
        outcome.unwrap();

        let (row, _, _) = omacal_store::event_by_id(&pool, id)
            .await
            .unwrap()
            .expect("the exception's row was removed, so the master expands into its slot again");
        assert_eq!(row.status, "cancelled");
        assert_eq!(
            row.original_start_utc,
            Some(OCCURRENCE),
            "the slot the row suppresses was lost, which is the whole reason to keep it"
        );
        assert_eq!(row.recurring_event_id.as_deref(), Some("master1"));
    }

    /// The provenance rule applied to the *local* half: a row is touched only
    /// when it is the resource that was deleted.
    ///
    /// The lookup is bracketed by where the clicked block is drawn, and an
    /// occurrence somebody dragged can land on another occurrence's slot — so
    /// what comes back, and what is then deleted, is a different event from the
    /// one that was clicked. Marking the clicked row cancelled there suppresses
    /// a slot that is still occupied, while the occurrence that really went goes
    /// on rendering until the next sync: two wrong days for the price of one.
    ///
    /// Written because a mutation that dropped the `event_id == ev.google_id`
    /// test survived the whole suite: every other fixture here has the clicked
    /// row either being the deleted resource or carrying `recurrence` (which the
    /// inner arms skip anyway), so nothing else could tell the guard was there.
    #[tokio::test]
    async fn deleting_an_occurrence_that_resolves_to_another_event_leaves_the_clicked_row_alone() {
        let mut ev = exception_row(INSTANCE_ID, OCCURRENCE, "confirmed");
        ev.start_utc = OCCURRENCE + 7 * 24 * HOUR; // dragged onto a later occurrence's slot
        ev.end_utc = ev.start_utc + HOUR;
        let clicked = ev.start_utc;
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/calendars/cal%40x.com/events/master1/instances"))
            .and(wiremock::matchers::query_param("timeMin", omacal_sync::to_rfc3339(clicked)))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [wire_occurrence(
                    "master1_20260817T090000Z", clicked, clicked + HOUR, "\"i2\"")]
            })))
            .expect(1)
            .mount(&server)
            .await;
        mount_delete(&server, "/calendars/cal%40x.com/events/master1_20260817T090000Z").await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        let outcome =
            delete_via_client(&pool, "this", clicked, ev, "cal@x.com", "UTC", &client).await;

        let sent = requests(&server).await;
        assert_eq!(
            query_param_of(the_lookup(&sent), "timeMin"),
            Some(omacal_sync::to_rfc3339(clicked)),
            "the lookup was bracketed by the occurrence's slot rather than by where its block \
             is drawn, so a dragged occurrence resolves to whatever now sits at its old time"
        );
        outcome.unwrap();

        let (row, _, _) = omacal_store::event_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(
            row.status, "confirmed",
            "the clicked row was cancelled for a deletion that landed on a different event, so \
             its slot is now suppressed while the occurrence that was really deleted still renders"
        );
        assert_eq!(row.original_start_utc, Some(OCCURRENCE));
    }

    /// "All events" from an exception row: the master is the target, and every
    /// local row of the series goes — the master's and the exceptions'.
    ///
    /// Google removes the series entire, materialised exceptions included, so a
    /// local exception left behind names an event that exists nowhere. It
    /// carries no rule of its own, so `events_in_window` returns it and the grid
    /// draws it as an ordinary meeting: the series vanishes and the occurrence
    /// somebody once dragged stays, with nothing left on screen to delete it
    /// from.
    #[tokio::test]
    async fn deleting_all_from_an_exception_row_takes_the_master_and_the_whole_local_series() {
        let mut ev = exception_row(INSTANCE_ID, OCCURRENCE, "confirmed");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let mut master = weekly_master("RRULE:FREQ=WEEKLY");
        master.calendar_id = ev.calendar_id;
        omacal_store::upsert_event(&pool, &master).await.unwrap();
        let mut other = exception_row("master1_20260824T090000Z", OCCURRENCE + 14 * 24 * HOUR, "cancelled");
        other.calendar_id = ev.calendar_id;
        omacal_store::upsert_event(&pool, &other).await.unwrap();

        let server = wiremock::MockServer::start().await;
        mount_delete(&server, MASTER_PATH).await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        delete_via_client(&pool, "all", OCCURRENCE, ev, "cal@x.com", "UTC", &client)
            .await
            .unwrap();

        // Straight off the table: `events_in_window` hides cancelled rows unless
        // they are exceptions, and one of the rows that has to go is exactly
        // that.
        let left: Vec<String> = sqlx::query_scalar("SELECT google_id FROM events")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(left.is_empty(), "rows of a deleted series survived it: {left:?}");
    }

    /// A series that ends after a fixed number of times **is** deletable from
    /// one occurrence onwards, and this is where this scope parts company with
    /// the split it otherwise resembles.
    ///
    /// `split_series` refuses `COUNT` because the *tail* it creates would need
    /// `COUNT` less however many occurrences the first half consumed — a number
    /// only a full expansion knows. There is no tail here. `truncated_rule`
    /// drops `COUNT` and adds `UNTIL`, which is the whole and correct answer for
    /// a series that now ends on a date; reusing the split's refusal would have
    /// turned an ordinary delete into "edit all events instead", which deletes
    /// the occurrences the user asked to keep.
    #[tokio::test]
    async fn deleting_following_from_a_count_series_truncates_rather_than_refusing() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY;COUNT=10");
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY;COUNT=10"]).await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path(MASTER_PATH))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "recurrence": [UNTIL_BEFORE_OCCURRENCE],
            })))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(wire_shortened_master(&[UNTIL_BEFORE_OCCURRENCE])),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        let outcome =
            delete_via_client(&pool, "following", OCCURRENCE, ev, "cal@x.com", "UTC", &client)
                .await;

        let sent = requests(&server).await;
        assert_eq!(
            patch_body(&sent),
            serde_json::json!({ "recurrence": [UNTIL_BEFORE_OCCURRENCE] }),
            "COUNT and UNTIL are mutually exclusive in RFC 5545, so a truncation has to drop it"
        );
        outcome.expect("a COUNT series has no tail to re-count, so there is nothing to refuse");
    }

    /// Deleting from the series' own first occurrence onwards is deleting the
    /// series, and it has to become that rather than a truncation.
    ///
    /// A master truncated to end before its own `DTSTART` expands to nothing: an
    /// event Google still holds, that renders in no grid, that the user cannot
    /// see and therefore cannot delete — one more every time they do it. There
    /// is nothing before the first occurrence to keep, so "this and following"
    /// there is "all events".
    #[tokio::test]
    async fn deleting_following_at_the_first_occurrence_deletes_the_whole_series() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;
        mount_delete(&server, MASTER_PATH).await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        let outcome =
            delete_via_client(&pool, "following", DTSTART, ev, "cal@x.com", "UTC", &client).await;

        // Before the result, for the usual reason: no `PATCH` is mounted, so a
        // regression reports `http error: 404 Not Found` and never names the
        // truncation that should not have been attempted.
        let sent = requests(&server).await;
        assert!(
            !sent.iter().any(|r| r.method.as_str() == "PATCH"),
            "the series was truncated to end before its own start, leaving a master that \
             expands to nothing: {:?}",
            methods_and_paths(&sent)
        );
        outcome.unwrap();
        assert!(omacal_store::event_by_id(&pool, id).await.unwrap().is_none());
    }

    /// A row belonging to no series has no "following": there is one event, and
    /// deleting it and everything after it is deleting it. It must not become an
    /// instances lookup — `target_event_id` reads every scope that is not
    /// `"all"` as "this one" — and it must not try to truncate anything.
    #[tokio::test]
    async fn deleting_following_on_a_one_off_deletes_that_event_and_looks_up_nothing() {
        let mut ev = stored(vec![]);
        ev.summary = Some("Lunch".into());
        ev.start_utc = OCCURRENCE;
        ev.end_utc = OCCURRENCE + HOUR;
        let (pool, id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_delete(&server, "/calendars/cal%40x.com/events/ev1").await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        delete_via_client(&pool, "following", OCCURRENCE, ev, "cal@x.com", "UTC", &client)
            .await
            .unwrap();

        let sent = requests(&server).await;
        assert_eq!(
            methods_and_paths(&sent),
            vec!["DELETE /calendars/cal%40x.com/events/ev1".to_string()],
            "a one-off delete fetched or truncated something"
        );
        assert!(omacal_store::event_by_id(&pool, id).await.unwrap().is_none());
    }

    /// The truncation is aimed at the occurrence's slot **on the rule's grid**,
    /// not at where its block is drawn.
    ///
    /// `UNTIL` is compared against the instants the rule generates. This fixture
    /// dragged the 09:00 occurrence five hours later; ending the series where it
    /// now sits leaves `UNTIL` after the 09:00 slot the rule still produces, so
    /// an occurrence the user deleted comes back. Dragged the other way it is
    /// the mirror image, and one they meant to keep disappears.
    #[tokio::test]
    async fn deleting_following_truncates_at_the_slot_not_where_the_occurrence_was_dragged() {
        let mut ev = exception_row(INSTANCE_ID, OCCURRENCE, "confirmed");
        ev.start_utc = OCCURRENCE + 5 * HOUR;
        ev.end_utc = OCCURRENCE + 6 * HOUR;
        let clicked = ev.start_utc;
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "UTC").await;

        let server = wiremock::MockServer::start().await;
        mount_master(&server, &["RRULE:FREQ=WEEKLY"]).await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path(MASTER_PATH))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "recurrence": [UNTIL_BEFORE_OCCURRENCE],
            })))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(wire_shortened_master(&[UNTIL_BEFORE_OCCURRENCE])),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        let outcome =
            delete_via_client(&pool, "following", clicked, ev, "cal@x.com", "UTC", &client).await;

        let sent = requests(&server).await;
        assert_eq!(
            patch_body(&sent),
            serde_json::json!({ "recurrence": [UNTIL_BEFORE_OCCURRENCE] }),
            "the truncation was aimed at where the occurrence was dragged to, so the rule still \
             generates the slot it came from and the occurrence comes back"
        );
        outcome.unwrap();
    }

    /// RFC 5545 §3.3.10: `UNTIL` carries the same value type as the `DTSTART` of
    /// the rule it belongs to. The body is `recurrence` alone, so the master
    /// keeps the bare-date `start` it already had and its `UNTIL` has to be a
    /// bare date — and a date-valued `UNTIL` is inclusive of the day it names,
    /// so the ending is the *previous* date, not the previous second.
    ///
    /// `Pacific/Auckland` is UTC+12, so its midnight falls on the previous date
    /// in UTC: read in UTC this would end the series a day early and delete an
    /// occurrence the user asked to keep.
    ///
    /// Unlike the split, there is no second event here whose flag could be taken
    /// by mistake — `truncate_series` is not handed the clicked row at all — so
    /// what this pins is that the value type is read from the master rather than
    /// assumed.
    #[tokio::test]
    async fn deleting_following_on_an_all_day_series_ends_it_on_the_previous_date() {
        let midnight = |wall: &str| {
            wall.parse::<jiff::civil::DateTime>()
                .unwrap()
                .in_tz("Pacific/Auckland")
                .unwrap()
                .timestamp()
                .as_millisecond()
        };
        let dtstart = midnight("2026-07-27T00:00:00");
        let occurrence = midnight("2026-08-10T00:00:00");
        assert_eq!(
            jiff::Timestamp::from_millisecond(occurrence).unwrap().to_string(),
            "2026-08-09T12:00:00Z",
            "fixture check: this instant must fall on the previous date in UTC, or the \
             assertion below proves nothing"
        );

        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        ev.is_all_day = true;
        ev.start_utc = dtstart;
        ev.end_utc = dtstart + 24 * HOUR;
        ev.start_tz = "Pacific/Auckland".into();
        ev.end_tz = "Pacific/Auckland".into();
        let (pool, _id) = seeded_pool_on_cal(&mut ev, "Pacific/Auckland").await;

        let all_day_master = serde_json::json!({
            "id": "master1", "status": "confirmed", "etag": "\"m1\"",
            "summary": "On call",
            "recurrence": ["RRULE:FREQ=WEEKLY"],
            "start": {"date": "2026-07-27"},
            "end":   {"date": "2026-07-28"}
        });
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(MASTER_PATH))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(all_day_master.clone()),
            )
            .expect(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path(MASTER_PATH))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "recurrence": ["RRULE:FREQ=WEEKLY;UNTIL=20260809"],
            })))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(all_day_master))
            .expect(1)
            .mount(&server)
            .await;

        let client = omacal_google::CalendarClient::new(server.uri(), "tok");
        let outcome = delete_via_client(
            &pool,
            "following",
            occurrence,
            ev,
            "cal@x.com",
            "Pacific/Auckland",
            &client,
        )
        .await;

        let sent = requests(&server).await;
        assert_eq!(
            patch_body(&sent),
            serde_json::json!({ "recurrence": ["RRULE:FREQ=WEEKLY;UNTIL=20260809"] }),
            "an all-day series was ended with a date-time UNTIL, on the wrong day, or both"
        );
        outcome.unwrap();
    }

    /// A subscribed holiday calendar, or one shared with you read-only, is
    /// `reader`. The refusal happens before `load_config`, the Keychain or
    /// Google see the request — the same shape the other three verbs have, and
    /// the reason no mock server is needed here.
    #[tokio::test]
    async fn deleting_an_event_on_a_read_only_calendar_is_refused() {
        let pool = omacal_store::connect_memory().await.unwrap();
        let cal = seed_calendar_with_tz(&pool, "reader", "UTC").await;
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        ev.calendar_id = cal;
        let id = omacal_store::upsert_event(&pool, &ev).await.unwrap();

        let err = delete_impl(&state_with(pool.clone(), false), id, "all", OCCURRENCE)
            .await
            .unwrap_err();
        assert_eq!(crate::errors::user_facing(&err), "this calendar is not writable from omacal");
        assert!(omacal_store::event_by_id(&pool, id).await.unwrap().is_some());
    }

    /// A scope this command does not implement must be refused, not read as
    /// "this occurrence" — [`target_event_id`] reads every scope that is not
    /// `"all"` that way, so an unrecognised one arriving from a future UI would
    /// silently delete a single occurrence of a series the user asked to do
    /// something else with.
    ///
    /// `"thisAndPrevious"` is Google's own vocabulary for a scope this app has
    /// no plans for, and it is chosen for the reason recorded on
    /// `an_unimplemented_scope_is_refused_rather_than_treated_as_this_occurrence`:
    /// the scope named here must be one **nothing is going to implement**. That
    /// test used `"following"` as its stand-in, and the moment Task 7 shipped
    /// the scope it ran past the gate and read the real
    /// `~/.config/omacal/config.toml`, which no test may do.
    ///
    /// **Choosing the scope carefully is not enough on its own**, because that
    /// is what was done last time. The fixture is a `reader` calendar
    /// ([`seeded_pool_on_read_only_cal`]), so if this scope is ever implemented
    /// the writability gate underneath catches it — "this calendar is not
    /// writable from omacal" — and nothing reaches `load_config` or the
    /// Keychain. The assertion is unchanged and still fails if the scope gate
    /// goes.
    #[tokio::test]
    async fn an_unimplemented_delete_scope_is_refused_rather_than_treated_as_this_occurrence() {
        let mut ev = weekly_master("RRULE:FREQ=WEEKLY");
        let (pool, id) = seeded_pool_on_read_only_cal(&mut ev).await;

        let err = delete_impl(&state_with(pool, false), id, "thisAndPrevious", OCCURRENCE)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not available yet"), "got: {err}");
    }
}


/// One row of the guest field's autocomplete (2026-08-23): a person from
/// the user's own meeting history, serialized as the UI reads it. The
/// corpus is `omacal_store::known_guests`'s — no People API, no new OAuth
/// scope; see that function's comment for the reasoning.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct KnownGuestRow {
    pub email: String,
    pub display_name: Option<String>,
    pub met: i64,
}

#[tauri::command]
pub(crate) async fn known_guests(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<KnownGuestRow>, String> {
    Ok(omacal_store::known_guests(&state.pool)
        .await
        .map_err(|e| crate::errors::user_facing(&e))?
        .into_iter()
        .map(|g| KnownGuestRow { email: g.email, display_name: g.display_name, met: g.met })
        .collect())
}
