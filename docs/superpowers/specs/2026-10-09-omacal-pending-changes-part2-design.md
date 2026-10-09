# The form's Save shows at once, too

Part 2 of three (part 1: `2026-10-09-omacal-pending-changes-design.md`, moves
and deletes, shipped in 5.7.0 and field-checked by Plamen; part 3: Create).
Plamen approved the scope and both design sections on 2026-10-09: every edit
the event form can make shows at once, except a change to the repeat rule.

Everything in part 1 still holds: optimistic display, not offline editing;
the local database stays exactly what Google accepted; a change is cleared
only by a load that began after its own follow-up sync; a refusal undoes the
change visibly. This spec adds one change kind and teaches the overlays it.

## 1. Behaviour

1. **Save, and the meeting changes at once.** The form closes as it does
   today, and the meeting is drawn the way it was saved, faded with a dashed
   outline, while "Saving 1 change…" shows in the header. That covers the
   title, the time and date, the location, the calendar (and so its colour),
   the guest count, and removing the video call.
2. **All-day on or off moves it straight away**: from the hour grid into the
   all-day band, or back. In Month and Big Year the meeting takes the first
   free lane and every other bar stays where it is (or it counts in "+N more"
   when there is no free lane).
3. **The details card shows the saved values.** Clicking the pending meeting
   shows the new title, location, description, calendar and guest list, with
   no Edit or Delete (part 1's lock).
4. **A refusal undoes it.** The meeting returns to how it was, and the header
   says `Could not save “<title>”: <reason>`. The user reopens the form to
   try again, as today.
5. **What still waits for the save, as today:**
   - a changed **repeat rule** (only the backend expands occurrences);
   - **all-day on or off for "following" or "all events"** of a series;
   - **adding a Google Meet link** (the link exists only once Google mints
     it); removing one shows at once.
   These are still queued: the meeting is locked and counted in "Saving",
   but drawn unchanged until the save lands.
6. **"Following" and "all events"** of a series: every visible occurrence
   takes the new values and shifts by the same time change, exactly as part
   1's moves do.
7. **A meeting still saving cannot be edited, dragged or deleted again.**
8. **Creates are untouched** (part 3).

## 2. The change

`PendingChange` (`ui/src/lib/pendingview.ts`) gains:

```ts
| {
    kind: 'edit'; id: number; occurrenceStartMs: number; scope: Scope;
    /** Where it is drawn now, or null when its time is not redrawn (a repeat
     *  change, or all-day switched on a series). All-day times are the
     *  display-zone midnights of the first day and of the day after the last. */
    when: { allDay: boolean; startMs: number; endMs: number } | null;
    /** New values to draw; an absent key leaves the payload's. */
    patch: { title?: string; location?: string | null; color?: string;
             calendar_id?: number; attendees?: number; conference?: null };
    /** The details card's own values (description and guest list), drawn by
     *  `overlayDetail`; absent keys leave the stored detail's. */
    detail: { description?: string | null; guests?: { email: string; optional: boolean }[] };
  }
```

`covers` and `locks` treat an edit like a move: matched by id and scope, and
locked where it came from and where `when` puts it.

## 3. Mapping the form to the change — `ui/src/lib/pendingedit.ts`

A pure function, `editChange(input) → PendingChange`, from the edit
request (`id`, `occurrenceStartMs`, the occurrence's current `startMs`/`endMs`,
whether the detail `is_recurring`, its current `calendar_id`), the form's
`EventFormResult`, the calendar list (for `color_hex`), and a
`dayMs(ymd) → number` (App's `ymdMs`: display-zone midnight).

- `when`: `null` if `fields.repeat`, `fields.weeklyDays` or `fields.repeatEnd`
  is present (the repeat controls changed), or if the all-day state changes
  on a recurring event whose scope is not `'this'`. Otherwise a timed
  `WhenInput` gives `{ allDay: false, startMs, endMs }`, and an all-day one
  gives `{ allDay: true, startMs: dayMs(startDate), endMs: dayMs(endDate) }`
  (the form's `endDate` is already exclusive).
- `patch.title` from `fields.summary` (`'(no title)'` when null, as the
  backend draws it); `patch.location` from `fields.location`.
- `patch.calendar_id` and `patch.color` only when `result.calendarId` differs
  from the current calendar: the target calendar's `color_hex`.
- `patch.attendees` when `fields.guests` is present: `guests.length`. The
  form's list is everyone stored on the event, the user and the organiser
  included (`valueFromDetail`), which is what the backend counts
  (`attendees.len()`).
- `patch.conference = null` when `fields.conference === 'none'`.
- `detail.description` from `fields.description`; `detail.guests` from
  `fields.guests` when present.

## 4. The overlays

- **Week (`overlayWeek`).** A timed→timed edit is a move plus `patch`. A
  timed→all-day edit leaves its day and joins `all_day_events` with a lane
  over the columns whose days it covers (`sliceWeek` re-packs the band, so
  the lane number is irrelevant). An all-day→timed edit leaves the band
  (its lane dropped, `idx` remapped) and joins its day, which is re-laid
  out. An all-day date change redraws its lane. Every redrawn copy is
  flagged `pending`. An edit with `when: null` redraws nothing (it only
  locks).
- **Month (`overlayMonth`).** Timed chips move between cells as in part 1,
  with `patch`. An all-day item is removed from its bars (a gap remains) or
  added by **first fit**: the lowest lane in which its columns are free, if
  below `lane_cap`; otherwise it is added to that row's `bar_overflow` for
  each column it covers. Other bars never move.
- **Big Year (`overlayBigYear`).** The same first fit for pills, against the
  payload's `lane_cap`.
- **The details card (`overlayDetail(detail, changes, occurrenceStartMs)`).**
  For an edit covering that occurrence: `title`, `location`, `calendar_id`,
  `description`, `is_all_day` (from `when`), `conference_uri: null` when the
  call was removed, and `attendees` rebuilt from `detail.guests` in its
  order: an address already stored keeps its stored entry (its answer, its
  name, `is_self`) with the new `optional`, a new one is added as
  `needsAction`, and one no longer on the list is dropped. App passes the popover
  `overlayDetail(gridDetail, pendingChanges(), gridSelStart)`.

## 5. Wiring

`saveForm`'s edit arm queues instead of waiting:
`queueChange(editChange(…), { write: () => updateEvent(request.id,
result.scope, request.occurrenceStartMs, result.fields, result.notify,
result.calendarId), ...queuedRefresh, onfailure: … })`. The form request
carries the occurrence's current `startMs`/`endMs` and the detail's
`is_recurring` and `calendar_id`, which it already has when the form opens.
The create arm, Quick Add and imports keep `refreshAfterWrite`.

## 6. Testing

- **Unit:** `pendingedit.spec.ts` (each mapping rule, including every
  `when: null` case); `pendingview.spec.ts` gains the edit cases (patch on a
  timed card; timed↔all-day in Week; first fit and overflow in Month and Big
  Year; `when: null` draws nothing but locks); `overlayDetail` (guest list
  rebuild keeps answers, drops removed, adds new, keeps self). Each proven
  red first.
- **App (`app.spec.ts`)**, with `holdNextWrite('update_event')`: a new title
  shows at once and dashed; a time change moves the card; all-day on moves it
  into the band; a repeat change keeps the old drawing but is locked and
  counted; a refused save reverts with `Could not save`; the pending card's
  popover shows the new title.
- Full suite, then a fresh whole-branch review, as in part 1.
