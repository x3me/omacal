# A new event shows when you save it, too

Part 3 of three (part 1: `2026-10-09-omacal-pending-changes-design.md`, moves
and deletes, shipped in 5.7.0; part 2:
`2026-10-09-omacal-pending-changes-part2-design.md`, the form's edits, on
main). Plamen chose the scope and approved both design sections on
2026-10-10: every create the app makes from the form or Quick Add shows at
once, a repeating one as its first occurrence; a just-created event is inert
while it saves; a refusal offers to reopen what was typed.

Everything in parts 1 and 2 still holds: optimistic display, not offline
editing; the local database stays exactly what Google accepted; a change is
cleared only by a load that began after the store held it; a refusal undoes
the change visibly. This spec adds one change kind, the first with no event
id of its own.

## 1. Behaviour

1. **Save, and it is there.** The form closes as today, and the new event is
   drawn where it was saved, faded with a dashed outline, while "Saving 1
   change…" shows in the header. That covers the form's Save from every
   entry point (a click or drag on the grid, `n`, Duplicate, Ctrl+V paste,
   Quick Add's "continue in form") and Quick Add's direct create.
2. **Where it is drawn** is the backend's own rule, as in part 2: in Day and
   Week a timed event joins its day, laid out beside what it overlaps, and an
   all-day one joins the band; in Month a timed event is a line in the cell it
   starts in (and in the first cell of each later row it runs into), and an
   all-day one takes the first free bar lane or joins "+N more"; Big Year
   draws all-day events only.
3. **A repeating event shows its first occurrence at once** (the form's own
   date and time); the rest appear when it saves. Only the backend expands a
   rule.
4. **While it saves it is inert.** A click opens nothing, and it cannot be
   dragged, resized or right-click-edited. In Day and Week its hover tooltip
   adds "Saving…"; Month and Big Year have no hover tooltip, so the dashed
   look says it there. Once saved it is an ordinary event.
5. **A refusal takes it away.** The header says `Could not create “<title>”:
   <reason>` with a **Reopen** button that opens a create form holding
   everything that was typed. The button lasts as long as that message does.
6. **"Created on Google but not stored" is not a refusal.** The event exists
   and its guests may already be mailed, so it stays drawn, the banner keeps
   the backend's sentence (as today), and the next sync brings it in.
7. **Navigating away and back** keeps it, in every view.
8. **A requested Meet link** appears once saved; the title and time show at
   once.
9. **`.ics` imports are unchanged** (`refreshAfterWrite`).

Known inexactness: the guest count drawn while saving is the number of
guests typed; Google may add the organiser to the list it stores. The count
is shown only in Filmstrip, and the save corrects it.

## 2. The change

`PendingChange` (`ui/src/lib/pendingview.ts`) gains:

```ts
| {
    kind: 'create';
    /** Temporary and negative, unique per create: no payload event has one,
     *  so `covers` never matches and `id:start` keys never collide. */
    id: number;
    /** The copy to draw, flagged `pending`. */
    event: UiEvent;
  }
```

`covers` is false for every payload event (ids differ). `locks(change, id,
startMs)` is true for the change's own id, so `isPending` answers for the
drawn copy. `landing` returns null for it (nothing is moved from anywhere).

## 3. Mapping the form to the change — `ui/src/lib/pendingedit.ts`

`createChange(result, calendars, dayMs, id) → PendingChange`, pure, beside
`editChange`. The rule that turns the form's `WhenInput` into drawn times
moves into one helper both use (`drawnWhen(when, dayMs)`): a timed
`WhenInput` gives its own instants; an all-day one gives `dayMs(startDate)`
and `dayMs(endDate)` (the form's `endDate` is already exclusive).

The drawn `UiEvent`:

- `id`: the given temporary id; `calendar_id`: `result.calendarId`;
  `color`: that calendar's `color_hex`.
- `title`: `fields.summary ?? '(no title)'`; `location`: `fields.location`.
- `start_ms`/`end_ms`/`is_all_day` from `drawnWhen`.
- `attendees`: `fields.guests?.length ?? 0`.
- `recurring`: `fields.repeat !== undefined` (`toEventInput` sends the repeat
  fields on a create only when a repeat is set).
- `response: 'accepted'` (the user is the organiser), `conference: null`,
  `all_guests_declined: false`, `pending: true`.

`App` takes ids from a counter in `pending.svelte.ts` (`unsavedId()`: -1, -2,
…).

## 4. The overlays

`overlayWeek`, `overlayMonth` and `overlayBigYear` add each create change's
`event` to the copies they already place (part 2's arrivals), so a new event
lands by exactly the rules a redrawn edit does: re-laid-out day columns in
Week, the band by first fit, Month lines by start cell and bars by first fit
(`bar_overflow` when no lane is free), Big Year pills for all-day only.
`overlayDetail` is untouched: nothing opens.

## 5. Inert while saving

An event with a negative id is unsaved. WeekGrid's `openPopover` (click,
Enter, right-click edit) and App's `openOccurrence` (Month, Big Year, list
mode) return at once for it, so no `event_detail` call is made. The grab
guard already refuses a `pending` event. EventBlock's own hover tooltip adds
"Saving…" for it.

## 6. Reopen

- `EventFormResult` gains `value: EventFormValue`, the form's own value at
  Save. `QuickEventModal`'s result carries its parsed value the same way.
- The header banner takes an optional action, `{ forError, label, run }`,
  shown beside the error only while `error === forError`, so a later error or
  a cleared banner removes it.
- Reopen runs `form = { mode: 'create', anchor: keyboardAnchor(), initial:
  value }`, the form Quick Add's "continue" opens.

## 7. Wiring

The form's create arm and `saveQuick` queue instead of waiting, and stop
setting `busy`:

```ts
const title = result.fields.summary ?? '(no title)';
void queueChange(createChange(result, calendars, ymdMs, unsavedId()), {
  write: async () => {
    try {
      await createEvent(result.calendarId, result.fields, result.notify);
      return true; // `create_impl` stored the row it answers with
    } catch (e) {
      if (!String(e).startsWith('The event was created on Google')) throw e;
      error = String(e); // it exists: keep it drawn until the sync brings it
      return false;
    }
  },
  ...queuedRefresh,
  onfailure: (e) => {
    const message = `Could not create “${title}”: ${String(e)}`;
    error = message;
    errorAction = { forError: message, label: 'Reopen',
                    run: () => { form = { mode: 'create', anchor: keyboardAnchor(), initial: result.value }; } };
  },
});
```

A write resolving `true` is part 2's "the store already holds it": the
change counts as synced at once, and the next load draws the real event and
drops the copy in the same update. The created-not-stored write resolves
`false`, so the copy waits for the follow-up sync.

## 8. Testing

- **Unit:** `pendingedit.spec.ts` (`createChange`: timed, all-day through
  `dayMs`, repeating marks `recurring` and draws only the first occurrence,
  the calendar's colour, `(no title)`, guest count, `accepted`, `pending`);
  `pendingview.spec.ts` (a create in Week, laid out beside an overlap, and in
  the band; Month line in its start cell, all-day bar by first fit and into
  overflow; Big Year all-day pill, timed nothing; `covers` never matches;
  `locks` its own id). Each proven red first.
- **App (`app.spec.ts`)**, with `holdNextWrite('create_event')` (the harness
  gains it): a new event shows at once, dashed, counted; an all-day one in
  the band; clicking it makes no `event_detail` call and opens nothing; a
  refused create disappears with `Could not create` and Reopen opens the form
  with the typed title; created-not-stored keeps it drawn with the sentence;
  Quick Add's direct create shows at once; a new event survives a week step
  while its write is held; a repeating create draws one occurrence.
- Full suite, then a fresh whole-branch review, as in parts 1 and 2.
