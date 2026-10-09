# A change shows when you make it, and saves behind you

Plamen, 2026-10-09: he dragged a meeting and it took a few seconds to move.
That is how every write has worked: the card goes back where it was on drop,
the app waits for Google, then reloads and the card jumps. The one exception
is answering an invitation, which since #133 (v5.0.0) updates the grid at
once and sends the reply in the background. "More human natural to expect
it to work this way" — so every change should behave like that one.

This spec is **part 1 of three**: the pending-changes core, drag move and
resize, and delete. Part 2 is the event form's Save; part 3 is Create. Each
gets its own spec and plan; they share the store and the overlay below.

## 1. What this is, and what it is not

- **It is optimistic display.** A change is drawn as done the moment it is
  made, marked as saving, and sent to Google in the background.
- **It is not offline editing.** The local database still holds only what
  Google accepted (the rule #66's answer keeps). If Google refuses the change
  or the network is down, the change is undone on screen with the error. #66
  stays parked.
- **Nothing is silently wrong.** A failure is visible the moment it happens,
  which is the point the old comment in `commitMove` was protecting ("a drag
  that appears to have worked and silently did not is worse than one that
  visibly refuses").

## 2. Behaviour

1. **A drop holds.** A dragged meeting stays where it was dropped, drawn
   faded with a dashed outline. The header shows "Saving N", the count the
   invitation queue already shows, now covering both kinds. When the save
   lands the outline goes and the card takes its proper lane beside any
   overlapping meeting.
2. **A delete hides at once**, wherever it was made: Day/Week, Month, list
   mode, the popover, the keyboard.
3. **A failure undoes itself visibly.** The card returns to its old place, or
   reappears for a delete, and the error shows in the header banner.
4. **A drop that asks first holds while it asks.** A meeting with guests to
   notify, or a repeating one (this event or all), shows at the drop point
   while `MoveConfirm` is open. Cancel puts it back; the answer starts the
   save. Delete's scope prompt behaves the same way.
5. **"All events" shifts every visible occurrence** of the series by the
   dragged occurrence's delta. An exception already moved separately in
   Google updates when the save lands.
6. **A pending card is not draggable, and cannot be deleted again**, until
   its save lands (a few seconds). Every other meeting stays fully usable.
   Several changes can be pending at once; they are written to Google one at
   a time, in the order made.
7. **Navigating away and back** (another week, Month, list mode) keeps the
   pending state until it lands.
8. **Quitting mid-save** is the invitation queue's behaviour: a write already
   sent may or may not complete. Closing the window does not interrupt
   anything, because the app keeps running.

Moves exist only for timed events in Day and Week (the all-day band is not
draggable), so pending moves are timed-only. Deletes can be anything.

## 3. Approach: draw over the payload

Three were weighed (2026-10-09, Plamen chose A):

- **A. Overlay (chosen).** Payloads stay exactly what the backend sent. A
  store of pending changes is applied over them before they reach the
  views: deleted and moved occurrences are hidden, moved cards are drawn on
  top. No backend change.
- B. Write the local row first, then Google. Real layout at once, but it
  breaks the mirror rule and a sync arriving mid-save overwrites the row.
- C. Patch the payload in the UI and re-run the lane packing there. A second
  copy of `omacal-core`'s layout, which drifts.

**A's one cost:** while pending, a moved timed card is drawn full column
width on top, not packed beside an overlapping meeting. It is packed when
the save lands. The drag preview looks the same way today.

## 4. Structure

### `ui/src/lib/pending.svelte.ts` — the store

The invitation queue (`responses.svelte.ts`) is the model, deliberately.

- A **job** is `{ seq, kind: 'move' | 'delete', target, saved }`, where the
  target names the occurrence (`id`, `occurrenceStartMs`, `scope`) and a
  move carries its landed span (and, for `scope: 'all'`, the delta).
- **One queue tail**, so writes run in order. A job runs: the write
  (`updateEvent` / `deleteEvent`), then `syncCalendar(true)`, then a reload.
  The write's own arguments are what `commitMove` / `runDelete` pass today.
- **Clearing uses the checkpoint rule.** `pendingCheckpoint()` is the highest
  `seq` saved before a load begins; `reconcilePending(checkpoint)` runs after
  a load lands and drops saved jobs at or below it. A reload already in
  flight when a save finishes therefore cannot clear the job, so the card
  never flickers back. `loadWeek` (Day and Week), `loadMonth`, `loadYear` and `loadBigYear` call it
  beside `reconcileResponses`.
- **A failure drops the job** and sets App's `error` to
  `Could not move “Title”: <reason>` / `Could not delete “Title”: <reason>`.
- **Held, not yet queued:** a drop that opens `MoveConfirm` (or a delete
  awaiting its scope) registers a *held* entry that draws like a pending one
  and is removed on Cancel or turned into a job on the answer.
- Exposes `pendingChangeCount()`, `isPending(id, occurrenceStartMs)`, and the
  current changes for the overlay.

### `ui/src/lib/pendingview.ts` — pure overlay functions

No DOM, no store import; specs drive them directly.

- `overlayWeek(week, changes) → { week, ghosts }`: removes hidden occurrences
  from `days[].events`, remaps each `placed` entry's `idx` (dropping the
  hidden ones, so the remaining events keep their lanes), does the same for
  `all_day`/`all_day_events`, and returns moved cards as
  `ghosts: { event, startMs, endMs }[]`.
- `overlayMonth(month, changes) → month`: removes hidden chips from cells and
  `bar_events`/`bars`, and puts a moved timed occurrence's chip into its new
  cell in time order, flagged `pending`.
- A change matches an occurrence by `id` + `start_ms`; `scope: 'all'`
  matches every occurrence with the same `id` and shifts it by the delta.
- List mode builds its days from these same payloads, so it needs nothing.

### Wiring

- **App:** `moveOccurrence` registers the hold before fetching the detail;
  the no-dialog path queues at once; `MoveConfirm`'s confirm queues and its
  cancel releases. `commitMove`'s body becomes the job's write, and
  `runDelete` queues the same way. Neither sets `busy` any more. App derives
  overlaid payloads and passes them to `WeekGrid`, `MonthGrid` and list mode,
  plus `ghosts` to `WeekGrid`.
- **WeekGrid** draws `ghosts` with the form preview's geometry (full column
  width, `crop`-aware), class `pending`: faded, dashed outline, no pointer
  events, so not draggable.
- **Popover and keyboard:** Delete is unavailable for an occurrence where
  `isPending` is true.
- **Header:** "Saving N" counts `pendingResponseCount() + pendingChangeCount()`.

## 5. Testing

- **Unit (`pendingview.spec.ts`, `pending.spec.ts`)**: hide and idx remap;
  a moved chip lands in the right month cell; `'all'` shifts every visible
  occurrence; the queue runs in order; a failure drops the job and reports;
  a load begun before a save does not clear it; a load begun after does.
  Each proven red against broken code first (`docs/testing-standard.md`).
- **Playwright (`app.spec.ts`)**: the harness gains a hold for
  `update_event` and `delete_event_cmd` (beside `holdSyncOnce`). With it:
  the dropped card sits at its new place, dashed, before the write
  resolves, and "Saving 1" shows; a rejected write puts it back with the
  error; Cancel on `MoveConfirm` puts it back; a delete hides at once and a
  failed one brings it back; a pending card survives a week step and back;
  a pending card does not start a drag.
- **Field**: Plamen drags a real meeting on Omarchy and sees it stay put.
