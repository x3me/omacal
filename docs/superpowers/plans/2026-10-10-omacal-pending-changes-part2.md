# Pending Changes, Part 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Saving the event form for an existing meeting redraws it at once (title, time, date, all-day, location, calendar colour, guest count, removed video call) and writes it to Google in the background; a refusal undoes it visibly.

**Architecture:** Part 1's store and overlay gain a third change kind, `edit`. A pure mapper turns the form's result into that change; the overlays redraw edited occurrences (timed in day columns, all-day in the band, first-fit lanes in Month and Big Year) and a new `overlayDetail` patches the details card. `saveForm`'s edit arm queues instead of waiting. No backend change.

**Tech Stack:** Svelte 5, TypeScript, Playwright (UI specs and Node-run unit specs).

**Spec:** `docs/superpowers/specs/2026-10-09-omacal-pending-changes-part2-design.md` (part 1's spec, `2026-10-09-omacal-pending-changes-design.md`, still holds)

## Global Constraints

- No backend (Rust) change. The local database stays an exact copy of what Google accepted.
- Optimistic display, not offline editing: a refused or failed write is undone on screen with the error.
- A change is cleared only by a load that began after its own follow-up sync (part 1's queue already does this; reuse `queuedRefresh`).
- Shown at once: title, time, date, all-day on/off, location, calendar colour, guest count, removed video call.
- Waits for the save, as today (queued, locked, counted, drawn unchanged): a changed repeat rule; all-day switched for "following"/"all" of a series; adding a Google Meet link.
- Failure message: `Could not save “<title>”: <reason>`.
- Creates, Quick Add and imports are untouched (part 3).
- Unit specs run in Node: all logic in plain `.ts`; no `.svelte.ts` imports from specs.
- House rules: every new test proven red before trusted (`docs/testing-standard.md`); commits end with `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`; commit to `main`; before pushing, `cargo clippy --workspace --all-targets -- -D warnings`, `npm --prefix ui run check`, and the full Playwright suite (read the exit code); run Playwright as `ui/node_modules/.bin/playwright` from `ui/`.

## Review Focus

1. **An edit of "this occurrence" of a series** whose follow-up sync is slow: the edited card must stay redrawn through a week step and a background reload (part 1's finding, now for edits). Pinned in Task 4 (`an edited occurrence survives a week step while its sync runs`).
2. **A title-only edit of a repeating meeting with "all events"**: every visible occurrence shows the new title at once, none moves. Pinned in Task 1 (`'all' takes the new title everywhere and moves nothing`).
3. **An all-day event switched to timed**: it leaves the band and appears in its day column, laid out beside what it overlaps. Pinned in Task 1 (`all-day to timed leaves the band and joins its day`).
4. **A Month row whose bar lanes are full**: a meeting that becomes all-day joins "+N more" instead of overlapping another bar. Pinned in Task 1 (`with no free lane it joins the overflow, once`).
5. **The details card of a pending edit**: shows the new title and guest list, keeps each remaining guest's answer. Pinned in Task 1 (`overlayDetail` tests) and Task 4 (popover spec).

---

### Task 1: The overlays learn edits — `pendingview.ts`

**Files:**
- Modify (replace whole file): `ui/src/lib/pendingview.ts`
- Test: `ui/tests/pendingview.spec.ts` (keep every part-1 test; add the new ones)

**Interfaces:**
- Consumes: `layOutDay` (`ui/src/lib/daylayout.ts`); types from `ui/src/lib/api.ts` and `ui/src/lib/eventdetail.ts` (`EventDetail`, `Attendee`).
- Produces:
  - `type EditPatch = { title?: string; location?: string | null; color?: string; calendar_id?: number; attendees?: number; conference?: null }`
  - `type PendingChange` gains `{ kind: 'edit'; id: number; occurrenceStartMs: number; scope: Scope; when: { allDay: boolean; startMs: number; endMs: number } | null; patch: EditPatch; detail: { description?: string | null; guests?: { email: string; optional: boolean }[] } }`
  - `covers`, `locks`, `overlayWeek`, `overlayMonth`, `overlayBigYear` — same signatures as part 1, now handling `edit`.
  - `overlayDetail(detail: EventDetail, changes: readonly PendingChange[], occurrenceStartMs: number): EventDetail`

- [ ] **Step 1: Write the failing tests**

Append to `ui/tests/pendingview.spec.ts` (it already defines `H`, `DAY`, `MON`, `ev`, `lane`, `place`, `week`, `move`, `del`). Add `import type { EventDetail } from '../src/lib/eventdetail';`, and add `overlayDetail` and `type EditPatch` to the `pendingview` import list:

```ts
const edit = (
  id: number, from: number,
  when: { allDay: boolean; startMs: number; endMs: number } | null,
  patch: EditPatch = {},
  scope: PendingChange['scope'] = 'this',
  detail: { description?: string | null; guests?: { email: string; optional: boolean }[] } = {},
): PendingChange => ({ kind: 'edit', id, occurrenceStartMs: from, scope, when, patch, detail });
const local = (y: number, m: number, d: number) => new Date(y, m, d).getTime(); // display-zone midnight

test.describe('edits, in the week', () => {
  test('a timed edit redraws the card with its new values and time', () => {
    const w = overlayWeek(week(), [edit(1, MON + 9 * H, { allDay: false, startMs: MON + 13 * H, endMs: MON + 14 * H },
      { title: 'Renamed', color: '#ff0000' })]);
    const e = w.days[0].events.find((x) => x.id === 1)!;
    expect([e.title, e.color, e.start_ms, e.end_ms, e.pending]).toEqual(['Renamed', '#ff0000', MON + 13 * H, MON + 14 * H, true]);
  });

  test("'all' takes the new title everywhere and moves nothing", () => {
    const w: WeekPayload = {
      ...week(),
      days: [
        { start_ms: MON, end_ms: MON + DAY, events: [ev(5, MON + 9 * H, MON + 10 * H, { recurring: true })], placed: [place(0)] },
        { start_ms: MON + DAY, end_ms: MON + 2 * DAY, events: [ev(5, MON + DAY + 9 * H, MON + DAY + 10 * H, { recurring: true })], placed: [place(0)] },
      ],
    };
    const out = overlayWeek(w, [edit(5, MON + DAY + 9 * H, { allDay: false, startMs: MON + DAY + 9 * H, endMs: MON + DAY + 10 * H },
      { title: 'Daily sync' }, 'all')]);
    expect(out.days.map((d) => d.events.map((e) => [e.title, e.start_ms, e.pending]))).toEqual([
      [['Daily sync', MON + 9 * H, true]], [['Daily sync', MON + DAY + 9 * H, true]],
    ]);
  });

  test('timed to all-day leaves its day and joins the band over its days', () => {
    const tue = MON + DAY;
    const w = overlayWeek(week(), [edit(1, MON + 9 * H, { allDay: true, startMs: MON, endMs: MON + 2 * DAY })]);
    expect(w.days[0].events.map((e) => e.id)).toEqual([2]);
    const i = w.all_day_events.findIndex((e) => e.id === 1);
    expect(i).toBeGreaterThanOrEqual(0);
    expect(w.all_day_events[i]).toMatchObject({ is_all_day: true, start_ms: MON, end_ms: tue + DAY, pending: true });
    const l = w.all_day.find((x) => x.idx === i)!;
    expect([l.start_col, l.end_col]).toEqual([0, 1]);
    // Its lane is free: no other band item shares its row over those columns.
    expect(w.all_day.filter((x) => x.lane === l.lane && x.idx !== i && x.end_col >= 0 && x.start_col <= 1)).toEqual([]);
  });

  test('all-day to timed leaves the band and joins its day', () => {
    const w = overlayWeek(week(), [edit(7, MON, { allDay: false, startMs: MON + 9 * H, endMs: MON + 10 * H })]);
    expect(w.all_day_events.map((e) => e.id)).toEqual([8]);
    expect(w.all_day).toEqual([lane(0, 0, 1, 1)]);
    const mon = w.days[0];
    expect(mon.events.map((e) => [e.id, e.is_all_day, e.pending ?? false])).toEqual([[1, false, false], [7, false, true], [2, false, false]]);
    // 7 overlaps 1 at 09:00: side by side.
    expect(mon.placed.filter((p) => mon.events[p.idx].id !== 2).map((p) => p.columns)).toEqual([2, 2]);
  });

  test('an edit that waits for its save is marked as saving where it is, unchanged', () => {
    const c = edit(1, MON + 9 * H, null, { title: 'Renamed' });
    const w = overlayWeek(week(), [c]);
    expect(w.days[0].events.map((e) => [e.id, e.start_ms, e.title, e.pending ?? false])).toEqual([
      [1, MON + 9 * H, 'e1', true], [2, MON + 10 * H, 'e2', false],
    ]);
    expect(w.days[0].placed).toEqual(week().days[0].placed); // nothing moved
    expect(locks(c, 1, MON + 9 * H)).toBe(true);
  });

  test('an edit locks where it landed', () => {
    const c = edit(1, MON + 9 * H, { allDay: false, startMs: MON + 15 * H, endMs: MON + 16 * H });
    expect(locks(c, 1, MON + 15 * H)).toBe(true);
    expect(locks(c, 1, MON + 9 * H)).toBe(true);
  });
});

test.describe('edits, in the month', () => {
  const month = (bars: Lane[] = [lane(0, 0, 0, 1)], barEvents: UiEvent[] = [ev(9, MON, MON + 2 * DAY, { is_all_day: true })], cap = 3): MonthPayload => ({
    year: 2024, month: 1, lane_cap: cap,
    rows: [{
      cells: [
        { start_ms: MON, end_ms: MON + DAY, in_month: true, timed: [ev(1, MON + 9 * H, MON + 10 * H)] },
        { start_ms: MON + DAY, end_ms: MON + 2 * DAY, in_month: true, timed: [] },
      ],
      bars, bar_events: barEvents, bar_overflow: [],
    }],
  });

  test('a meeting made all-day takes the first free lane and moves no other bar', () => {
    const out = overlayMonth(month(), [edit(1, MON + 9 * H, { allDay: true, startMs: MON, endMs: MON + DAY })]);
    const row = out.rows[0];
    expect(row.cells[0].timed).toEqual([]);
    expect(row.bars[0]).toEqual(lane(0, 0, 0, 1)); // untouched
    const i = row.bar_events.findIndex((e) => e.id === 1);
    expect(row.bars.find((l) => l.idx === i)).toMatchObject({ lane: 1, start_col: 0, end_col: 0 });
  });

  test('with no free lane it joins the overflow, once', () => {
    const full = month([lane(0, 0, 0, 1)], [ev(9, MON, MON + 2 * DAY, { is_all_day: true })], 1);
    const out = overlayMonth(full, [edit(1, MON + 9 * H, { allDay: true, startMs: MON, endMs: MON + 2 * DAY })]);
    const row = out.rows[0];
    const i = row.bar_events.findIndex((e) => e.id === 1);
    expect(row.bar_overflow).toEqual([i]);
    expect(row.bars.some((l) => l.idx === i)).toBe(false);
  });

  test('a meeting moved across midnight keeps showing, as a bar', () => {
    const out = overlayMonth(month([], []), [move(1, MON + 9 * H, MON + 23 * H, 2 * H)]);
    const row = out.rows[0];
    expect(row.cells[0].timed).toEqual([]);
    const i = row.bar_events.findIndex((e) => e.id === 1);
    expect(row.bars.find((l) => l.idx === i)).toMatchObject({ start_col: 0, end_col: 1 });
  });

  test('a deleted bar takes its overflow entry with it', () => {
    const m = month([], [ev(9, MON, MON + 2 * DAY, { is_all_day: true }), ev(10, MON, MON + DAY, { is_all_day: true })]);
    m.rows[0].bar_overflow = [0, 1];
    const out = overlayMonth(m, [del(9, MON)]);
    expect(out.rows[0].bar_events.map((e) => e.id)).toEqual([10]);
    expect(out.rows[0].bar_overflow).toEqual([0]);
  });
});

test.describe('edits, in the big year', () => {
  test('a moved all-day pill takes the first free lane in its new place', () => {
    const days = Array.from({ length: 28 }, (_, i) => ({ start_ms: local(2024, 0, 29 + i), in_year: true, unsynced: false }));
    const big: BigYearPayload = {
      year: 2024, lane_cap: 3,
      rows: [{ days, pills: [lane(0, 0, 0, 0)], pill_events: [ev(7, days[0].start_ms, days[1].start_ms, { is_all_day: true })], overflow: [] }],
    };
    const out = overlayBigYear(big, [edit(7, days[0].start_ms, { allDay: true, startMs: days[2].start_ms, endMs: days[3].start_ms })]);
    const row = out.rows[0];
    expect(row.pill_events).toHaveLength(1);
    expect(row.pills).toEqual([{ ...lane(0, 0, 2, 2), cont_left: false, cont_right: false }]);
    expect(row.pill_events[0].pending).toBe(true);
  });
});

test.describe('the details card of a pending edit', () => {
  const detail = (): EventDetail => ({
    id: 1, title: 'Board prep', location: 'Room 1', description: 'old', calendar_id: 3, is_all_day: false, conference_uri: 'https://meet.google.com/x',
    attendees: [
      { email: 'me@x.com', display_name: 'Me', response_status: 'accepted', optional: false, is_self: true },
      { email: 'dan@x.com', display_name: 'Dan', response_status: 'declined', optional: false, is_self: false },
      { email: 'eve@x.com', display_name: null, response_status: 'tentative', optional: false, is_self: false },
    ],
  }) as unknown as EventDetail;

  test('shows the saved values, keeps answers, drops removed guests, adds new ones', () => {
    const c = edit(1, MON + 9 * H, { allDay: false, startMs: MON + 13 * H, endMs: MON + 14 * H },
      { title: 'Board prep v2', location: null, calendar_id: 4, conference: null }, 'this',
      { description: 'new', guests: [{ email: 'me@x.com', optional: false }, { email: 'DAN@x.com', optional: true }, { email: 'zoe@x.com', optional: false }] });
    const out = overlayDetail(detail(), [c], MON + 13 * H);
    expect([out.title, out.location, out.calendar_id, out.description, out.conference_uri]).toEqual(['Board prep v2', null, 4, 'new', null]);
    expect(out.attendees.map((a) => [a.email, a.response_status, a.optional, a.is_self])).toEqual([
      ['me@x.com', 'accepted', false, true], ['dan@x.com', 'declined', true, false], ['zoe@x.com', 'needsAction', false, false],
    ]);
  });

  test('another occurrence, or an edit that waits, leaves the card as stored', () => {
    const c = edit(1, MON + 9 * H, { allDay: false, startMs: MON + 13 * H, endMs: MON + 14 * H }, { title: 'X' });
    expect(overlayDetail(detail(), [c], MON + DAY + 9 * H).title).toBe('Board prep');
    expect(overlayDetail(detail(), [edit(1, MON + 9 * H, null, { title: 'X' })], MON + 9 * H).title).toBe('Board prep');
  });
});
```

`EditPatch` comes from the same `pendingview` import (add `type EditPatch` to it). Type-only imports are erased when the spec runs, so Step 2 still fails on the missing `overlayDetail` export and the edit behaviour, not on a type.

- [ ] **Step 2: Run them to see them fail**

Run (from `ui/`): `./node_modules/.bin/playwright test tests/pendingview.spec.ts --project=chromium`
Expected: FAIL — `overlayDetail` is not exported, and the edit cases fail (the overlay ignores `kind: 'edit'`).

- [ ] **Step 3: Replace `pendingview.ts`**

Replace the whole file with:

```ts
// Pending changes, drawn over the payloads (pending-changes specs, 2026-10-09:
// part 1, moves and deletes; part 2, the form's edits).
//
// A change the user just made is shown at once and written to Google behind
// them. The payloads stay exactly what the backend sent; this is the layer
// between them and the views. Pure, so `pendingview.spec.ts` drives every rule
// directly.

import type { BigYearPayload, DayColumn, Lane, MonthPayload, UiEvent, WeekPayload } from './api';
import type { Attendee, EventDetail } from './eventdetail';
import type { Scope } from './eventform';
import { layOutDay } from './daylayout';

/** The values an edit redraws; an absent key leaves the payload's. */
export type EditPatch = {
  title?: string; location?: string | null; color?: string;
  calendar_id?: number; attendees?: number; conference?: null;
};

/** One change still on its way to Google. A move or an edit says where the
 *  occurrence it came from lands; for a series, every covered occurrence
 *  shifts by the same amount and takes the same length. */
export type PendingChange =
  | { kind: 'move'; id: number; occurrenceStartMs: number; scope: Scope; startMs: number; endMs: number }
  | { kind: 'delete'; id: number; occurrenceStartMs: number; scope: Scope }
  | {
      kind: 'edit'; id: number; occurrenceStartMs: number; scope: Scope;
      /** Where it is drawn now, or null when its time is not redrawn (part 2
       *  spec §1.5): a repeat change, all-day switched on a series. All-day
       *  times are display-zone midnights, the end exclusive. */
      when: { allDay: boolean; startMs: number; endMs: number } | null;
      patch: EditPatch;
      /** The details card's own values (`overlayDetail`). */
      detail: { description?: string | null; guests?: { email: string; optional: boolean }[] };
    };

/**
 * Whether `change` speaks for this occurrence, as the payload has it.
 *
 * Occurrences of one series share the series' `id` (`commands::to_ui`), so
 * `'all'` is every one of them and `'following'` the ones from the change's
 * occurrence on. An exception is its own row with its own id, and updates when
 * the save lands. A block combining several calendars' copies is never
 * covered: its copies are reached through its own panel.
 */
export function covers(change: PendingChange, ev: Pick<UiEvent, 'id' | 'start_ms' | 'copies'>): boolean {
  if (ev.copies?.length) return false;
  if (ev.id !== change.id) return false;
  if (change.scope === 'all') return true;
  if (change.scope === 'following') return ev.start_ms >= change.occurrenceStartMs;
  return ev.start_ms === change.occurrenceStartMs;
}

/** Where a change puts the occurrence it came from, or null when it does not
 *  redraw it. */
function landing(change: PendingChange): { startMs: number } | null {
  if (change.kind === 'move') return change;
  if (change.kind === 'edit') return change.when;
  return null;
}

/**
 * Whether the occurrence at `startMs` is still being written, and so may not
 * be dragged, edited or deleted again. Unlike `covers`, this also answers for
 * where a move or an edit *landed*: clicking that card opens a popover for its
 * new start, and that card is the pending one.
 */
export function locks(change: PendingChange, id: number, startMs: number): boolean {
  if (id !== change.id) return false;
  if (change.scope === 'all') return true;
  const land = landing(change);
  if (change.scope === 'following') {
    const from = land ? Math.min(change.occurrenceStartMs, land.startMs) : change.occurrenceStartMs;
    return startMs >= from;
  }
  return startMs === change.occurrenceStartMs || (land !== null && startMs === land.startMs);
}

const DAY_MS = 86_400_000;

/** `ms` moved by whole local days, so an all-day midnight stays a midnight
 *  across a daylight-saving change. */
function addDays(ms: number, days: number): number {
  const d = new Date(ms);
  d.setDate(d.getDate() + days);
  return d.getTime();
}

/** Whether `ev` fits within the one local day it starts on: a cell's chip in
 *  Month, rather than a bar. */
function fitsOneDay(ev: UiEvent): boolean {
  if (ev.is_all_day) return false;
  const d = new Date(ev.start_ms);
  d.setHours(0, 0, 0, 0);
  return ev.end_ms <= addDays(d.getTime(), 1);
}

/** What the changes do to one occurrence: hide it, and perhaps redraw it
 *  somewhere else. Applied in the order the changes were made. An edit whose
 *  time waits for the save (`when: null`) leaves it where it is but `held`:
 *  marked as saving, so it cannot be grabbed (part 2 spec §1.5). */
function fate(ev: UiEvent, changes: readonly PendingChange[]): { hidden: boolean; moved: UiEvent | null; held: boolean } {
  let hidden = false;
  let held = false;
  let moved: UiEvent | null = null;
  for (const c of changes) {
    if (!covers(c, ev)) continue;
    if (c.kind === 'delete') { hidden = true; moved = null; continue; }
    if (c.kind === 'move') {
      hidden = true;
      const start = ev.start_ms + (c.startMs - c.occurrenceStartMs);
      moved = { ...ev, start_ms: start, end_ms: start + (c.endMs - c.startMs), pending: true };
      continue;
    }
    const w = c.when;
    if (!w) { held = true; continue; }
    hidden = true;
    let start: number;
    let end: number;
    if (ev.is_all_day && w.allDay) {
      // Whole days, so every occurrence of an all-day series keeps midnights.
      start = addDays(ev.start_ms, Math.round((w.startMs - c.occurrenceStartMs) / DAY_MS));
      end = addDays(start, Math.round((w.endMs - w.startMs) / DAY_MS));
    } else if (ev.is_all_day === w.allDay) {
      start = ev.start_ms + (w.startMs - c.occurrenceStartMs);
      end = start + (w.endMs - w.startMs);
    } else {
      // All-day switched: only ever 'this' (the mapper sends `when: null`
      // for a series), so the form's own times are the occurrence's.
      start = w.startMs;
      end = w.endMs;
    }
    moved = { ...ev, ...c.patch, start_ms: start, end_ms: end, is_all_day: w.allDay, pending: true };
  }
  return { hidden, moved, held: held && !hidden };
}

/** One list after the changes: hidden events dropped, held ones flagged
 *  `pending` in place, redrawn copies handed to `take`. `keep[i]` says whether
 *  `events[i]` survived, `changed` whether anything differs at all. */
function sift(
  events: UiEvent[], changes: readonly PendingChange[], take: (from: UiEvent, to: UiEvent) => void,
): { out: UiEvent[]; keep: boolean[]; changed: boolean } {
  let changed = false;
  const out: UiEvent[] = [];
  const keep = events.map((e) => {
    const f = fate(e, changes);
    if (f.moved) take(e, f.moved);
    if (f.hidden) { changed = true; return false; }
    if (f.held) { changed = true; out.push({ ...e, pending: true }); } else out.push(e);
    return true;
  });
  return { out, keep, changed };
}

/** Lane-packed items after the changes: hidden ones dropped, with `idx` and
 *  the overflow list remapped so every kept entry still names its own event.
 *  Redrawn copies are handed to `take`. */
function relane(
  lanes: Lane[], events: UiEvent[], overflow: number[], changes: readonly PendingChange[],
  take: (from: UiEvent, to: UiEvent) => void,
): { lanes: Lane[]; events: UiEvent[]; overflow: number[] } {
  const { out, keep, changed } = sift(events, changes, take);
  if (!changed) return { lanes, events, overflow };
  const newIdx: number[] = [];
  let n = 0;
  keep.forEach((k, i) => { if (k) newIdx[i] = n++; });
  return {
    lanes: lanes.filter((l) => keep[l.idx]).map((l) => ({ ...l, idx: newIdx[l.idx] })),
    events: out,
    overflow: overflow.filter((i) => keep[i]).map((i) => newIdx[i]),
  };
}

/** The columns of `cols` that `ev` covers, as a lane's extent, or null when
 *  it covers none of them. */
function extent(ev: UiEvent, cols: { start_ms: number; end_ms: number }[]):
  { start_col: number; end_col: number; cont_left: boolean; cont_right: boolean } | null {
  const hit = cols.flatMap((c, i) => (ev.start_ms < c.end_ms && ev.end_ms > c.start_ms ? [i] : []));
  if (hit.length === 0) return null;
  return {
    start_col: hit[0], end_col: hit[hit.length - 1],
    cont_left: ev.start_ms < cols[0].start_ms, cont_right: ev.end_ms > cols[cols.length - 1].end_ms,
  };
}

/** The lowest lane below `cap` whose columns `[start, end]` are free, or -1.
 *  First fit, so no lane already drawn ever moves. */
function firstFreeLane(lanes: Lane[], start: number, end: number, cap: number): number {
  for (let row = 0; row < cap; row++) {
    if (lanes.every((l) => l.lane !== row || l.end_col < start || l.start_col > end)) return row;
  }
  return -1;
}

/** Add `arrivals` to a lane-packed set by first fit; one that finds no lane
 *  joins the overflow list. */
function placeLanes(
  set: { lanes: Lane[]; events: UiEvent[]; overflow: number[] },
  arrivals: UiEvent[], cols: { start_ms: number; end_ms: number }[], cap: number,
): { lanes: Lane[]; events: UiEvent[]; overflow: number[] } {
  if (arrivals.length === 0) return set;
  const lanes = [...set.lanes];
  const events = [...set.events];
  const overflow = [...set.overflow];
  for (const e of arrivals) {
    const span = extent(e, cols);
    if (!span) continue;
    const idx = events.length;
    events.push(e);
    const row = firstFreeLane(lanes, span.start_col, span.end_col, cap);
    if (row < 0) overflow.push(idx);
    else lanes.push({ idx, lane: row, ...span });
  }
  return { lanes, events, overflow };
}

const byStart = (a: UiEvent, b: UiEvent) => a.start_ms - b.start_ms || b.end_ms - a.end_ms;
/** Enough rows for any band the week draws; `sliceWeek` re-packs it anyway. */
const WEEK_BAND_CAP = 1000;

/**
 * The week as the user's pending changes have left it.
 *
 * Moved, edited and deleted occurrences leave their day or the band. Each
 * redrawn copy (flagged `pending`) joins every day its new span touches when
 * it is timed, or the band when it is all-day. A day that changed is laid out
 * again with `layOutDay`, the Rust layout's own port, so a pending card sits
 * in its proper lane beside whatever it now overlaps.
 */
export function overlayWeek(week: WeekPayload, changes: readonly PendingChange[]): WeekPayload {
  if (changes.length === 0) return week;
  const moved = new Map<string, UiEvent>();
  const take = (from: UiEvent, to: UiEvent) => { moved.set(`${from.id}:${from.start_ms}`, to); };
  const kept = week.days.map((day) => sift(day.events, changes, take));
  const band = relane(week.all_day, week.all_day_events, week.overflow, changes, take);
  const arrivals = [...moved.values()];
  const days = week.days.map((day, i): DayColumn => {
    const here = arrivals.filter((e) => !e.is_all_day && e.start_ms < day.end_ms && e.end_ms > day.start_ms);
    const k = kept[i];
    if (!k.changed && here.length === 0) return day;
    // Only flags changed: the same events in the same order, so the layout stands.
    if (here.length === 0 && k.keep.every(Boolean)) return { ...day, events: k.out };
    const events = [...k.out, ...here].sort(byStart);
    const placed = layOutDay(events.map((e) => ({ startMs: e.start_ms, endMs: e.end_ms })), day.start_ms, day.end_ms);
    return { ...day, events, placed };
  });
  const placedBand = placeLanes(band, arrivals.filter((e) => e.is_all_day), week.days, WEEK_BAND_CAP);
  return { ...week, days, all_day: placedBand.lanes, all_day_events: placedBand.events, overflow: placedBand.overflow };
}

/**
 * The month as the pending changes have left it. A redrawn copy that fits
 * within one day joins that day's cell, in time order; anything else (all-day,
 * or crossing midnight) joins the bars by first fit, so no other bar moves.
 */
export function overlayMonth(month: MonthPayload, changes: readonly PendingChange[]): MonthPayload {
  if (changes.length === 0) return month;
  const moved = new Map<string, UiEvent>();
  const take = (from: UiEvent, to: UiEvent) => { moved.set(`${from.id}:${from.start_ms}`, to); };
  const kept = month.rows.map((row) => row.cells.map((cell) => sift(cell.timed, changes, take)));
  const bars = month.rows.map((row) => relane(row.bars, row.bar_events, row.bar_overflow, changes, take));
  const arrivals = [...moved.values()];
  const chips = arrivals.filter(fitsOneDay);
  const spans = arrivals.filter((e) => !fitsOneDay(e));
  const rows = month.rows.map((row, r) => {
    const cells = row.cells.map((cell, c) => {
      const here = chips.filter((e) => e.start_ms >= cell.start_ms && e.start_ms < cell.end_ms);
      const k = kept[r][c];
      if (!k.changed && here.length === 0) return cell;
      return { ...cell, timed: [...k.out, ...here].sort(byStart) };
    });
    const placed = placeLanes(bars[r], spans, row.cells, month.lane_cap);
    return { ...row, cells, bars: placed.lanes, bar_events: placed.events, bar_overflow: placed.overflow };
  });
  return { ...month, rows };
}

/** The Big Year ribbon draws only all-day and multi-day spans. A pill that is
 *  deleted leaves; one that is moved or edited is placed again by first fit.
 *  The ribbon carries no timed meetings, so one switched to all-day appears
 *  when the save lands (part 2 spec §4). */
export function overlayBigYear(big: BigYearPayload, changes: readonly PendingChange[]): BigYearPayload {
  if (changes.length === 0) return big;
  const moved = new Map<string, UiEvent>();
  const take = (from: UiEvent, to: UiEvent) => { moved.set(`${from.id}:${from.start_ms}`, to); };
  const kept = big.rows.map((row) => relane(row.pills, row.pill_events, row.overflow, changes, take));
  const spans = [...moved.values()].filter((e) => !fitsOneDay(e));
  return {
    ...big,
    rows: big.rows.map((row, r) => {
      const cols = row.days.map((d) => ({ start_ms: d.start_ms, end_ms: addDays(d.start_ms, 1) }));
      const placed = placeLanes(kept[r], spans, cols, big.lane_cap);
      return { ...row, pills: placed.lanes, pill_events: placed.events, overflow: placed.overflow };
    }),
  };
}

/** The guest list a pending edit leaves: the form's order; an address already
 *  stored keeps its entry (its answer, its name, `is_self`) with the new
 *  `optional`; a new address has not answered yet. */
function rebuildGuests(stored: Attendee[], guests: { email: string; optional: boolean }[]): Attendee[] {
  return guests.map((g) => {
    const was = stored.find((a) => a.email.toLowerCase() === g.email.toLowerCase());
    return was
      ? { ...was, optional: g.optional }
      : { email: g.email, display_name: null, response_status: 'needsAction', optional: g.optional, is_self: false };
  });
}

/**
 * The details card for the occurrence at `occurrenceStartMs`, as a pending
 * edit left it (part 2 spec §1.3). An edit whose time waits for the save, or
 * one for another occurrence, leaves the stored detail.
 */
export function overlayDetail(detail: EventDetail, changes: readonly PendingChange[], occurrenceStartMs: number): EventDetail {
  let out = detail;
  for (const c of changes) {
    if (c.kind !== 'edit' || c.when === null || !locks(c, detail.id, occurrenceStartMs)) continue;
    out = {
      ...out,
      ...(c.patch.title !== undefined ? { title: c.patch.title } : {}),
      ...(c.patch.location !== undefined ? { location: c.patch.location } : {}),
      ...(c.patch.calendar_id !== undefined ? { calendar_id: c.patch.calendar_id } : {}),
      ...(c.patch.conference === null ? { conference_uri: null } : {}),
      ...(c.detail.description !== undefined ? { description: c.detail.description } : {}),
      ...(c.detail.guests ? { attendees: rebuildGuests(out.attendees, c.detail.guests) } : {}),
      is_all_day: c.when.allDay,
    };
  }
  return out;
}
```

- [ ] **Step 4: Run them to see them pass**

Run (from `ui/`): `./node_modules/.bin/playwright test tests/pendingview.spec.ts` → PASS on both projects (part 1's tests included). Then `npm --prefix ui run check` → 0 errors.

- [ ] **Step 5: Prove the new rules red**

One mutation at a time, restore after each:
- In `fate`, replace `if (!w) { held = true; continue; }` with `if (!w) continue;` → `an edit that waits for its save is marked as saving where it is` fails.
- In `fate`, drop `...c.patch,` → `a timed edit redraws the card with its new values` fails.
- In `overlayWeek`, place all-day arrivals into days instead (`!e.is_all_day` → `true`) → `timed to all-day leaves its day and joins the band` fails.
- In `firstFreeLane`, return `0` unconditionally → `takes the first free lane` fails.
- In `relane`, return `overflow` unmapped → `a deleted bar takes its overflow entry with it` fails.
- In `rebuildGuests`, drop `{ ...was, optional: g.optional }` for `g`-only entries → `keeps answers` fails.

- [ ] **Step 6: Commit**

```bash
git add ui/src/lib/pendingview.ts ui/tests/pendingview.spec.ts
git commit -m "feat(pending): the overlays redraw edits, timed and all-day, and the details card

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: The form's result as a change — `pendingedit.ts`

**Files:**
- Create: `ui/src/lib/pendingedit.ts`
- Test: `ui/tests/pendingedit.spec.ts`

**Interfaces:**
- Consumes: `PendingChange`, `EditPatch` (Task 1); `EventFormResult` (`ui/src/lib/eventform.ts`); `Calendar` (`ui/src/lib/calendars.ts`).
- Produces: `type EditRequest = { id: number; occurrenceStartMs: number; wasAllDay: boolean; isRecurring: boolean; calendarId: number | null }` and `editChange(req: EditRequest, result: EventFormResult, calendars: Pick<Calendar, 'id' | 'color_hex'>[], dayMs: (ymd: string) => number): PendingChange`.

- [ ] **Step 1: Write the failing tests**

Create `ui/tests/pendingedit.spec.ts`:

```ts
import { test, expect } from '@playwright/test';
import { editChange, type EditRequest } from '../src/lib/pendingedit';
import type { EventFormResult } from '../src/lib/eventform';

const H = 3_600_000;
const T = Date.UTC(2024, 0, 29, 9);
const req = (over: Partial<EditRequest> = {}): EditRequest =>
  ({ id: 4, occurrenceStartMs: T, wasAllDay: false, isRecurring: false, calendarId: 1, ...over });
const result = (fields: Partial<EventFormResult['fields']> = {}, over: Partial<EventFormResult> = {}): EventFormResult => ({
  calendarId: 1, scope: 'this', notify: 'none',
  fields: { summary: 'Board prep', location: 'Room 1', description: null, when: { kind: 'timed', startMs: T, endMs: T + H }, tz: 'UTC', ...fields },
  ...over,
} as EventFormResult);
const calendars = [{ id: 1, color_hex: '#111111' }, { id: 2, color_hex: '#222222' }];
const dayMs = (ymd: string) => { const [y, m, d] = ymd.split('-').map(Number); return new Date(y, m - 1, d).getTime(); };
const map = (r: EventFormResult, q = req()) => editChange(q, r, calendars, dayMs);

test('a timed save lands at its new time, with its title and place', () => {
  const c = map(result({ summary: 'Renamed', location: 'Room 2', when: { kind: 'timed', startMs: T + 2 * H, endMs: T + 3 * H } }));
  expect(c).toMatchObject({
    kind: 'edit', id: 4, occurrenceStartMs: T, scope: 'this',
    when: { allDay: false, startMs: T + 2 * H, endMs: T + 3 * H },
    patch: { title: 'Renamed', location: 'Room 2' },
  });
});

test('no title is drawn as the backend draws it', () => {
  expect(map(result({ summary: null })).kind === 'edit' && (map(result({ summary: null })) as any).patch.title).toBe('(no title)');
});

test('an all-day save uses display-zone midnights, end exclusive', () => {
  const c = map(result({ when: { kind: 'allDay', startDate: '2024-01-29', endDate: '2024-01-31' } })) as any;
  expect(c.when).toEqual({ allDay: true, startMs: dayMs('2024-01-29'), endMs: dayMs('2024-01-31') });
});

test('a changed repeat rule waits for the save', () => {
  expect((map(result({ repeat: 'weekly' })) as any).when).toBeNull();
  expect((map(result({ repeatEnd: { kind: 'count', count: 3 } as any })) as any).when).toBeNull();
});

test('all-day switched on a series waits, unless it is this occurrence alone', () => {
  const allDay = { when: { kind: 'allDay' as const, startDate: '2024-01-29', endDate: '2024-01-30' } };
  expect((map(result(allDay, { scope: 'all' }), req({ isRecurring: true })) as any).when).toBeNull();
  expect((map(result(allDay, { scope: 'following' }), req({ isRecurring: true })) as any).when).toBeNull();
  expect((map(result(allDay, { scope: 'this' }), req({ isRecurring: true })) as any).when).not.toBeNull();
});

test('a new calendar brings its colour; the same calendar brings neither', () => {
  expect((map(result({}, { calendarId: 2 })) as any).patch).toMatchObject({ calendar_id: 2, color: '#222222' });
  const same = (map(result({}, { calendarId: 1 })) as any).patch;
  expect('calendar_id' in same || 'color' in same).toBe(false);
});

test('guests give the count and the card's list; untouched guests give neither', () => {
  const guests = [{ email: 'a@x.com', optional: false }, { email: 'b@x.com', optional: true }];
  const c = map(result({ guests })) as any;
  expect(c.patch.attendees).toBe(2);
  expect(c.detail.guests).toEqual(guests);
  const none = map(result()) as any;
  expect('attendees' in none.patch).toBe(false);
  expect('guests' in none.detail).toBe(false);
});

test('removing the call is drawn; adding a Meet link is not', () => {
  expect((map(result({ conference: 'none' })) as any).patch.conference).toBeNull();
  expect('conference' in (map(result({ conference: 'googleMeet' })) as any).patch).toBe(false);
});
```

- [ ] **Step 2: Run them to see them fail**

Run (from `ui/`): `./node_modules/.bin/playwright test tests/pendingedit.spec.ts --project=chromium`
Expected: FAIL — module `../src/lib/pendingedit` not found.

- [ ] **Step 3: Write `pendingedit.ts`**

```ts
// The event form's Save, as a pending change (part 2 spec, 2026-10-09).
// Pure: `pendingedit.spec.ts` drives every rule.

import type { Calendar } from './calendars';
import type { EventFormResult } from './eventform';
import type { EditPatch, PendingChange } from './pendingview';

/** What the form knew about the occurrence when it opened. */
export type EditRequest = {
  id: number;
  /** The occurrence's own start, never the series' DTSTART. */
  occurrenceStartMs: number;
  wasAllDay: boolean;
  isRecurring: boolean;
  calendarId: number | null;
};

/**
 * The change a Save makes, for the overlays to draw.
 *
 * Its time waits for the save (`when: null`) when only the backend can say
 * where the occurrences go: the repeat controls changed (`toEventInput` sends
 * them only then), or all-day was switched for more than this occurrence of a
 * series. Everything else is drawn at once.
 */
export function editChange(
  req: EditRequest, result: EventFormResult,
  calendars: Pick<Calendar, 'id' | 'color_hex'>[], dayMs: (ymd: string) => number,
): PendingChange {
  const f = result.fields;
  const repeatChanged = f.repeat !== undefined || f.weeklyDays !== undefined || f.repeatEnd !== undefined;
  const allDay = f.when.kind === 'allDay';
  const seriesSwitch = req.isRecurring && result.scope !== 'this' && allDay !== req.wasAllDay;
  const when = repeatChanged || seriesSwitch
    ? null
    : f.when.kind === 'timed'
      ? { allDay: false, startMs: f.when.startMs, endMs: f.when.endMs }
      : { allDay: true, startMs: dayMs(f.when.startDate), endMs: dayMs(f.when.endDate) };

  const patch: EditPatch = { title: f.summary ?? '(no title)', location: f.location };
  if (result.calendarId !== req.calendarId) {
    patch.calendar_id = result.calendarId;
    const color = calendars.find((c) => c.id === result.calendarId)?.color_hex;
    if (color) patch.color = color;
  }
  // The form's list is everyone stored on the event, the user and the
  // organiser included (`valueFromDetail`), which is what the backend counts.
  if (f.guests) patch.attendees = f.guests.length;
  if (f.conference === 'none') patch.conference = null;

  return {
    kind: 'edit', id: req.id, occurrenceStartMs: req.occurrenceStartMs, scope: result.scope, when, patch,
    detail: {
      description: f.description,
      ...(f.guests ? { guests: f.guests.map(({ email, optional }) => ({ email, optional })) } : {}),
    },
  };
}
```

- [ ] **Step 4: Run them to see them pass**

Run (from `ui/`): `./node_modules/.bin/playwright test tests/pendingedit.spec.ts` → PASS. `npm --prefix ui run check` → 0 errors.

- [ ] **Step 5: Prove red**

One at a time, restore after each: drop `|| seriesSwitch` → the series test fails; drop `?? '(no title)'` → the no-title test fails; drop the `if (result.calendarId !== req.calendarId)` guard (always set) → the same-calendar test fails.

- [ ] **Step 6: Commit**

```bash
git add ui/src/lib/pendingedit.ts ui/tests/pendingedit.spec.ts
git commit -m "feat(pending): the form's Save as a pending edit

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: The pending look on band chips, Month bars and Big Year pills

**Files:**
- Modify: `ui/src/lib/AllDayBand.svelte` (the `.chip` button ~line 72; styles ~line 168)
- Modify: `ui/src/lib/MonthGrid.svelte` (the `.bar` button ~line 133; styles)
- Modify: `ui/src/lib/BigYearRibbon.svelte` (the `.pill` button ~line 191; styles)

**Interfaces:** consumes `UiEvent.pending` (part 1). Produces nothing new.

- [ ] **Step 1: Add the class and the style**

In each of the three, on the button that draws the item (`class="chip"`, `class="bar"`, `class="pill"`), add `class:pending={ev.pending}` after its `class:cr={lane.cont_right}` line (use the loop variable the file uses for the event — `ev` in all three). Add to each `<style>`:

```css
  /* Saving (pending-changes spec): faded and dashed until Google confirms. */
  .chip.pending { opacity: .62; outline: 1.5px dashed var(--cal); outline-offset: -1.5px; }
```
with the selector `.bar.pending` in MonthGrid and `.pill.pending` in BigYearRibbon (all three items declare `--cal` inline).

- [ ] **Step 2: Verify nothing changed without pending changes**

Run: `npm --prefix ui run check` → 0 errors. From `ui/`: `./node_modules/.bin/playwright test tests/components.spec.ts > ../target/pw-p2t3.log 2>&1; echo EXIT=$?` → exit 0 (screenshot goldens included: no golden has a pending item).

- [ ] **Step 3: Commit**

```bash
git add ui/src/lib/AllDayBand.svelte ui/src/lib/MonthGrid.svelte ui/src/lib/BigYearRibbon.svelte
git commit -m "feat(pending): band chips, Month bars and Big Year pills wear the saving look

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: The form's Save queues — App wiring

**Files:**
- Modify: `ui/src/App.svelte` (form type ~line 1560; `openEdit` ~line 1764; `saveForm` ~line 1826; the `EventPopover` mount ~line 2440; imports)
- Test: `ui/tests/app.spec.ts` (inside `test.describe('a change shows at once and saves behind you', …)`)

**Interfaces:**
- Consumes: `editChange`, `EditRequest` (Task 2); `overlayDetail` (Task 1); `queueChange`, `pendingChanges` (part 1, `./lib/pending.svelte`); `queuedRefresh` (part 1, in App); harness `holdNextWrite('update_event')`, `releaseWrite`, `rejectWrite`, `holdNextSync`, `releaseSync`.

- [ ] **Step 1: Write the failing specs**

Inside `test.describe('a change shows at once and saves behind you', …)`, add:

```ts
      /** Opens Board prep's form from its popover. */
      const openBoardPrepForm = async (page: Page) => {
        await block(page, 'Board prep').click();
        await page.getByRole('button', { name: 'Edit' }).click();
        await expect(editForm(page)).toBeVisible();
      };

      test('a saved title shows at once, dashed, while it saves', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('update_event'));
        await openBoardPrepForm(page);
        await editForm(page).getByLabel('Title', { exact: true }).fill('Board review');
        await editForm(page).getByRole('button', { name: 'Save' }).click();

        await expect(block(page, 'Board review')).toHaveClass(/pending/);
        await expect(block(page, 'Board prep')).toHaveCount(0);
        await expect(page.locator('header [role="status"]').filter({ hasText: 'Saving 1 change' })).toBeVisible();
        await page.evaluate(() => window.__harness.releaseWrite());
      });

      test('a saved time moves the card at once', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('update_event'));
        const before = await topOfBlock(page, 'Board prep');
        await openBoardPrepForm(page);
        const start = editForm(page).getByLabel('Start', { exact: true });
        const was = await start.inputValue();
        const [h, m] = was.split(':').map(Number);
        await start.fill(`${String(h + 2).padStart(2, '0')}:${String(m).padStart(2, '0')}`);
        await start.press('Tab');
        await editForm(page).getByRole('button', { name: 'Save' }).click();

        await expect(block(page, 'Board prep')).toHaveClass(/pending/);
        expect(await topOfBlock(page, 'Board prep')).toBeGreaterThan(before + 20);
        await page.evaluate(() => window.__harness.releaseWrite());
      });

      test('All day on moves it into the band at once', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('update_event'));
        await openBoardPrepForm(page);
        await editForm(page).getByLabel('All day').check();
        await editForm(page).getByRole('button', { name: 'Save' }).click();

        await expect(block(page, 'Board prep')).toHaveCount(0);
        await expect(chip(page, 'Board prep')).toHaveClass(/pending/);
        await page.evaluate(() => window.__harness.releaseWrite());
      });

      test('a refused save puts the meeting back, with the reason', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('update_event'));
        await openBoardPrepForm(page);
        await editForm(page).getByLabel('Title', { exact: true }).fill('Board review');
        await editForm(page).getByRole('button', { name: 'Save' }).click();
        await expect(block(page, 'Board review')).toHaveClass(/pending/);

        await page.evaluate(() => window.__harness.rejectWrite('the server said no'));
        await expect(block(page, 'Board prep')).toHaveCount(1);
        await expect(block(page, 'Board review')).toHaveCount(0);
        await expect(page.locator('.err')).toContainText('Could not save “Board review”');
      });

      test('the saving card's details show the new title, without Edit or Delete', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('update_event'));
        await openBoardPrepForm(page);
        await editForm(page).getByLabel('Title', { exact: true }).fill('Board review');
        await editForm(page).getByRole('button', { name: 'Save' }).click();
        await expect(block(page, 'Board review')).toHaveClass(/pending/);

        await block(page, 'Board review').click();
        const popover = page.getByRole('dialog', { name: 'Board review' });
        await expect(popover).toBeVisible();
        await expect(popover.getByRole('button', { name: 'Edit', exact: true })).toHaveCount(0);
        await page.keyboard.press('Escape');
        await page.evaluate(() => window.__harness.releaseWrite());
      });

      test('an edited occurrence survives a week step while its sync runs', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextSync());
        await openBoardPrepForm(page);
        await editForm(page).getByLabel('Title', { exact: true }).fill('Board review');
        await editForm(page).getByRole('button', { name: 'Save' }).click();
        await expect.poll(() => callsTo(page, 'sync_now')).toHaveLength(1); // written; sync running

        await page.getByRole('button', { name: 'Next week' }).click();
        await page.getByRole('button', { name: 'Previous week' }).click();
        await page.waitForTimeout(300);
        await expect(block(page, 'Board review')).toHaveClass(/pending/);
        await page.evaluate(() => window.__harness.releaseSync());
      });

      test('a repeat change keeps the old drawing, marked as saving and locked', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('update_event'));
        const before = await topOfBlock(page, 'Board prep');
        await openBoardPrepForm(page);
        await editForm(page).getByLabel('Repeat', { exact: true }).selectOption('weekly');
        await editForm(page).getByRole('button', { name: 'Save' }).click();

        await expect(page.locator('header [role="status"]').filter({ hasText: 'Saving 1 change' })).toBeVisible();
        await expect(block(page, 'Board prep')).toHaveClass(/pending/);
        expect(await topOfBlock(page, 'Board prep')).toBeCloseTo(before, 0); // not moved
        await block(page, 'Board prep').click();
        await expect(page.getByRole('dialog', { name: 'Board prep' }).getByRole('button', { name: 'Edit', exact: true })).toHaveCount(0);
        await page.keyboard.press('Escape');
        await page.evaluate(() => window.__harness.releaseWrite());
      });
```

`Repeat` is a native `<select>` (`selectOption` works, as `components.spec.ts:4264` does) and `Start` takes `fill` (`components.spec.ts:4363`).

- [ ] **Step 2: Run them to see them fail**

Run (from `ui/`): `./node_modules/.bin/playwright test tests/app.spec.ts -g "a saved title|a saved time|All day on moves|a refused save|the saving card|an edited occurrence|a repeat change keeps" --project=chromium`
Expected: FAIL — the edited card keeps its old title/time until the write resolves.

- [ ] **Step 3: The form request carries what the change needs**

Replace the edit arm of the `form` state type:

```ts
    | { mode: 'edit'; anchor: Rect; initial: EventFormValue; id: number; occurrenceStartMs: number };
```
with
```ts
    | {
        mode: 'edit'; anchor: Rect; initial: EventFormValue; id: number; occurrenceStartMs: number;
        /** For the pending edit (part 2 spec §3): whether all-day switched on
         *  a series can be drawn, or waits for the save. */
        isRecurring: boolean;
      };
```

In `openEdit`, add `isRecurring: occurrence.detail.is_recurring,` after `occurrenceStartMs: occurrence.startMs,`.

- [ ] **Step 4: Save queues the edit**

Add imports:

```ts
  import { editChange } from './lib/pendingedit';
```
and add `overlayDetail` to the `./lib/pendingview` import.

In `saveForm`, replace everything from `busy = true;` to the end of the function with:

```ts
    if (request.mode === 'edit') {
      // Drawn as saved at once (part 2 spec, 2026-10-09); the write runs in
      // the queue behind it, and a refusal puts the meeting back.
      error = null;
      const title = result.fields.summary ?? '(no title)';
      void queueChange(
        editChange({
          id: request.id,
          occurrenceStartMs: request.occurrenceStartMs,
          wasAllDay: request.initial.isAllDay,
          isRecurring: request.isRecurring,
          calendarId: request.initial.calendarId,
        }, result, calendars, ymdMs),
        {
          // `request.occurrenceStartMs`, never `detail.start_ms`: for a series
          // the second is the master's DTSTART, and an edit aimed at it patches
          // occurrence #0 with the whole form as its payload. The scope comes
          // from the form's own chooser (Task 9).
          //
          // **`result.notify`, never a constant.** The form asks whether to
          // mail the guests; `App` does not decide it, this carries the answer.
          // `result.calendarId` is the picker's value, sent on every save: the
          // backend reads "the calendar it is already on" as no move at all.
          write: () => updateEvent(
            request.id, result.scope, request.occurrenceStartMs, result.fields, result.notify,
            result.calendarId,
          ),
          ...queuedRefresh,
          onfailure: (e) => { error = `Could not save “${title}”: ${String(e)}`; },
        },
      );
      return;
    }

    busy = true;
    error = null;
    try {
      // **`result.notify`, never a constant** — a create can invite people,
      // the form asks, and this carries the answer.
      await createEvent(result.calendarId, result.fields, result.notify);
    } catch (e) {
      error = String(e);
      // One failure is not a failure: a create that reached Google but not
      // the local store answers with the backend's fixed created-not-stored
      // sentence (events.rs, safelisted verbatim). The event exists — guests
      // are already mailed — so this must NOT stop, or the user is invited to
      // create it again. Falling through runs the ordinary post-write sync.
      if (!String(e).startsWith('The event was created on Google')) return;
    } finally {
      busy = false;
    }
    await refreshAfterWrite();
  }
```

(The two long comments that lived in the old single `try` are kept, each with the arm it belongs to.)

- [ ] **Step 5: The details card shows the pending edit**

In the `EventPopover` mount, replace `detail={gridDetail}` with `detail={overlayDetail(gridDetail, pendingChanges(), startMs)}`.

- [ ] **Step 6: Run the specs to see them pass**

Run (from `ui/`): `./node_modules/.bin/playwright test tests/app.spec.ts -g "a change shows at once"` → PASS on both projects. `npm --prefix ui run check` → 0 errors.

- [ ] **Step 7: Prove red**

One at a time, restore after each:
- In `saveForm`, make the edit arm `await updateEvent(…)` before `queueChange` → `a saved title shows at once` fails.
- Drop `onfailure`'s message → `a refused save puts the meeting back` fails on `.err`.
- Revert the popover to `detail={gridDetail}` → `the saving card's details show the new title` fails.

- [ ] **Step 8: Existing edit specs**

Run (from `ui/`): `./node_modules/.bin/playwright test tests/app.spec.ts tests/duplicate-event.spec.ts > ../target/pw-p2t4.log 2>&1; echo EXIT=$?`. A failure in an existing edit spec that asserts the *old* waiting behaviour (the card unchanged right after Save, or `busy` during an edit) is updated to the new truth with a one-line comment naming part 2's spec; any other failure is a bug to fix.

- [ ] **Step 9: Commit**

```bash
git add ui/src/App.svelte ui/tests/app.spec.ts
git commit -m "feat(pending): the form's Save shows the edit at once and saves behind

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Full gates, review, push, record

- [ ] **Step 1: Full gates** — from the repo root: `cargo clippy --workspace --all-targets -- -D warnings` (exit 0); `npm --prefix ui run check` (0 errors); from `ui/`: `./node_modules/.bin/playwright test > ../target/pw-p2.log 2>&1; echo EXIT=$? >> ../target/pw-p2.log` → `EXIT=0`. A load flake (passes alone, as part 1 saw in `eventform.spec.ts`) is noted, not "fixed".
- [ ] **Step 2: Whole-branch review** by a fresh reviewer on the most capable model, then one fix pass (each fix red→green, full suite green).
- [ ] **Step 3: Push and verify main's CI run by commit sha** (`gh run list --branch main --json databaseId,headSha` filtered by the pushed sha; all three jobs `success`).
- [ ] **Step 4: Record** in `~/dev/omacal-private/OPERATIONS.md`'s "Where things stand": part 2 on main (unreleased), commits, the field check owed (Plamen renames a real meeting and toggles all-day on one), part 3 (Create) next. Commit and push that repo.
