# Pending Changes, Part 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A dragged move/resize and a delete show on screen the instant they are made, marked as saving, and are written to Google in the background; a refusal undoes them visibly.

**Architecture:** A pending-changes store (a plain `PendingQueue` class plus a thin Svelte rune wrapper) holds held, queued and saved-but-not-reloaded changes. Pure overlay functions apply those changes to the Week, Month and Big Year payloads before they reach the views, re-laying out any day that gains or loses a card with the existing `daylayout.ts` port. Jobs run one at a time and are cleared only by a load that began after their save (the invitation queue's checkpoint rule), so a refresh in flight cannot flicker a card back. No backend change.

**Tech Stack:** Svelte 5 (runes), TypeScript, Playwright (UI specs and Node-run unit specs), Tauri IPC stubbed by `ui/tests/harness/tauri.ts`.

**Spec:** `docs/superpowers/specs/2026-10-09-omacal-pending-changes-design.md`

## Global Constraints

- No backend (Rust) change. The local database stays an exact copy of what Google accepted.
- This is optimistic display, not offline editing: a refused or failed write is undone on screen with the error.
- Pending moves are timed-only (the all-day band is not draggable). Deletes can be any event.
- A pending card cannot be dragged, edited or deleted again until its save lands.
- Writes run one at a time, in the order made.
- A change is cleared only by a load that began after its save (checkpoint rule, as `responses.svelte.ts`).
- Header text: the existing `Saving N responses…` is unchanged; pending changes add a separate `Saving N change…` / `Saving N changes…` status.
- Failure messages: `Could not move “<title>”: <reason>` and `Could not delete “<title>”: <reason>`.
- Rare cases keep today's behaviour: a block combining several calendars' copies, a timed meeting crossing midnight seen in Month, Year view's day marks.
- Unit specs cannot import `.svelte.ts` files (they run in Node): all logic lives in plain `.ts`.
- House rules: every new test is proven red against deliberately broken code before it is trusted (`docs/testing-standard.md`); commit messages end with `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`; commit straight to `main` (no PRs for our own work); before pushing run `cargo clippy --workspace --all-targets -- -D warnings`, `npm --prefix ui run check`, and the full Playwright suite (exit code, never the tail).
- Run Playwright as `ui/node_modules/.bin/playwright` from `ui/`, never bare `npx`.

## Review Focus

1. **A reload already in flight when a save finishes** (a background sync's `get_week`) must not clear the pending card; only a load that started after the save may. Pinned in Task 2 (`a load begun before the save does not clear it`).
2. **A move whose sync fails after Google accepted it** must stay drawn at its new place, not jump back, and say "The change was made, but OmaCal could not refresh from Google". Pinned in Task 2 (`after() failing keeps the change`) and in `refreshAfterQueuedWrite` (Task 5).
3. **Clicking the moved card opens a popover for its new start time**, which must still count as pending, so Edit and Delete are hidden. Pinned in Task 1 (`locks` covers the landed start) and Task 6 (popover spec).
4. **"This and following" / "All events" moves** shift every visible occurrence of the series, and lock them all. Pinned in Task 1 (`'all' shifts every occurrence`, `'following' shifts from the occurrence on`).
5. **Navigating to another week and back while a save is pending** keeps the card at its new place. Pinned in Task 5 (week-step spec).

---

### Task 1: Pure overlay — `pendingview.ts`

**Files:**
- Create: `ui/src/lib/pendingview.ts`
- Modify: `ui/src/lib/api.ts` (add `pending?` to `UiEvent`)
- Test: `ui/tests/pendingview.spec.ts`

**Interfaces:**
- Consumes: `layOutDay(spans: Interval[], dayStartMs: number, dayEndMs: number): Placed[]` from `ui/src/lib/daylayout.ts`; types `UiEvent`, `WeekPayload`, `MonthPayload`, `BigYearPayload`, `Lane`, `DayColumn` from `ui/src/lib/api.ts`; `Scope` (`'this' | 'all' | 'following'`) from `ui/src/lib/eventform.ts`.
- Produces:
  - `type PendingChange = { kind: 'move'; id: number; occurrenceStartMs: number; scope: Scope; startMs: number; endMs: number } | { kind: 'delete'; id: number; occurrenceStartMs: number; scope: Scope }`
  - `covers(change: PendingChange, ev: Pick<UiEvent, 'id' | 'start_ms' | 'copies'>): boolean`
  - `locks(change: PendingChange, id: number, startMs: number): boolean`
  - `overlayWeek(week: WeekPayload, changes: readonly PendingChange[]): WeekPayload`
  - `overlayMonth(month: MonthPayload, changes: readonly PendingChange[]): MonthPayload`
  - `overlayBigYear(big: BigYearPayload, changes: readonly PendingChange[]): BigYearPayload`
  - `UiEvent.pending?: boolean` (UI-only, never sent by the backend)

- [ ] **Step 1: Add the UI-only flag to `UiEvent`**

In `ui/src/lib/api.ts`, inside `export type UiEvent = { … }`, after the `copies?: EventCopy[];` line, add:

```ts
  /** Drawn where a change the user just made puts it, while that change is
   *  still being written (`pendingview.ts`). Set by the UI only; the backend
   *  never sends it. */
  pending?: boolean;
```

- [ ] **Step 2: Write the failing tests**

Create `ui/tests/pendingview.spec.ts`:

```ts
import { test, expect } from '@playwright/test';
import type { UiEvent, WeekPayload, MonthPayload, BigYearPayload, Lane } from '../src/lib/api';
import {
  covers, locks, overlayWeek, overlayMonth, overlayBigYear, type PendingChange,
} from '../src/lib/pendingview';

const H = 3_600_000;
const DAY = 24 * H;
const MON = Date.UTC(2024, 0, 29);

const ev = (id: number, start: number, end: number, extra: Partial<UiEvent> = {}): UiEvent => ({
  id, title: `e${id}`, location: null, start_ms: start, end_ms: end, color: '#5b8def',
  response: 'accepted', is_all_day: false, attendees: 0, recurring: false, conference: null,
  all_guests_declined: false, ...extra,
});
const lane = (idx: number, lane: number, start_col: number, end_col: number): Lane =>
  ({ idx, lane, start_col, end_col, cont_left: false, cont_right: false });
const place = (i: number) => ({ idx: i, column: 0, columns: 1, top: 0, height: 0.04 });

/** Two days: Monday carries a 09:00 and a 10:00 meeting, Tuesday a 09:00. */
const week = (): WeekPayload => ({
  days: [
    { start_ms: MON, end_ms: MON + DAY, events: [ev(1, MON + 9 * H, MON + 10 * H), ev(2, MON + 10 * H, MON + 11 * H)], placed: [place(0), place(1)] },
    { start_ms: MON + DAY, end_ms: MON + 2 * DAY, events: [ev(3, MON + DAY + 9 * H, MON + DAY + 10 * H)], placed: [place(0)] },
  ],
  all_day: [lane(0, 0, 0, 0), lane(1, 0, 1, 1)],
  all_day_events: [ev(7, MON, MON + DAY, { is_all_day: true }), ev(8, MON + DAY, MON + 2 * DAY, { is_all_day: true })],
  overflow: [0, 0],
});

const move = (id: number, from: number, to: number, length = H, scope: PendingChange['scope'] = 'this'): PendingChange =>
  ({ kind: 'move', id, occurrenceStartMs: from, scope, startMs: to, endMs: to + length });
const del = (id: number, from: number, scope: PendingChange['scope'] = 'this'): PendingChange =>
  ({ kind: 'delete', id, occurrenceStartMs: from, scope });

test.describe('which occurrences a pending change speaks for', () => {
  test('this, following and all', () => {
    const c = move(1, MON + 9 * H, MON + 12 * H);
    expect(covers(c, ev(1, MON + 9 * H, 0))).toBe(true);
    expect(covers(c, ev(1, MON + DAY + 9 * H, 0))).toBe(false);
    expect(covers({ ...c, scope: 'following' }, ev(1, MON + DAY + 9 * H, 0))).toBe(true);
    expect(covers({ ...c, scope: 'following' }, ev(1, MON - DAY + 9 * H, 0))).toBe(false);
    expect(covers({ ...c, scope: 'all' }, ev(1, MON - DAY + 9 * H, 0))).toBe(true);
    expect(covers(c, ev(2, MON + 9 * H, 0))).toBe(false);
  });

  test('a block combining several calendars is never covered', () => {
    const copies: UiEvent['copies'] = [
      { id: 1, calendar_id: 1, start_ms: MON + 9 * H, end_ms: MON + 10 * H, color: '#000' },
      { id: 4, calendar_id: 2, start_ms: MON + 9 * H, end_ms: MON + 10 * H, color: '#fff' },
    ];
    expect(covers(move(1, MON + 9 * H, MON + 12 * H), ev(1, MON + 9 * H, 0, { copies }))).toBe(false);
  });

  test('a pending move locks the occurrence where it landed, not only where it was', () => {
    const c = move(1, MON + 9 * H, MON + 12 * H);
    expect(locks(c, 1, MON + 9 * H)).toBe(true);
    expect(locks(c, 1, MON + 12 * H)).toBe(true);
    expect(locks(c, 1, MON + 15 * H)).toBe(false);
    expect(locks(c, 2, MON + 12 * H)).toBe(false);
    // Moved earlier with "following": the dragged one now starts before the
    // occurrence it came from, and is still locked.
    const back = move(1, MON + 9 * H, MON + 7 * H, H, 'following');
    expect(locks(back, 1, MON + 7 * H)).toBe(true);
    expect(locks(back, 1, MON + DAY + 7 * H)).toBe(true);
  });
});

test.describe('the week, with pending changes drawn over it', () => {
  test('no changes hands back the same payload', () => {
    const w = week();
    expect(overlayWeek(w, [])).toBe(w);
  });

  test('a move leaves its day and lands, laid out, in the day it was dropped on', () => {
    const w = overlayWeek(week(), [move(1, MON + 9 * H, MON + DAY + 9 * H)]);
    expect(w.days[0].events.map((e) => e.id)).toEqual([2]);
    expect(w.days[0].placed.map((p) => p.idx)).toEqual([0]);
    const tue = w.days[1];
    expect(tue.events.map((e) => [e.id, e.start_ms, e.pending ?? false])).toEqual([
      [3, MON + DAY + 9 * H, false], [1, MON + DAY + 9 * H, true],
    ]);
    // Two meetings at the same hour: laid out side by side, not stacked.
    expect(tue.placed.map((p) => p.columns)).toEqual([2, 2]);
    expect(new Set(tue.placed.map((p) => p.column))).toEqual(new Set([0, 1]));
  });

  test('a resize keeps its start and takes its new length', () => {
    const w = overlayWeek(week(), [move(2, MON + 10 * H, MON + 10 * H, 2 * H)]);
    const moved = w.days[0].events.find((e) => e.id === 2)!;
    expect([moved.start_ms, moved.end_ms, moved.pending]).toEqual([MON + 10 * H, MON + 12 * H, true]);
  });

  test("'all' shifts every occurrence of the series by the same amount", () => {
    const w: WeekPayload = {
      ...week(),
      days: [
        { start_ms: MON, end_ms: MON + DAY, events: [ev(5, MON + 9 * H, MON + 10 * H, { recurring: true })], placed: [place(0)] },
        { start_ms: MON + DAY, end_ms: MON + 2 * DAY, events: [ev(5, MON + DAY + 9 * H, MON + DAY + 10 * H, { recurring: true })], placed: [place(0)] },
      ],
    };
    const out = overlayWeek(w, [move(5, MON + 9 * H, MON + 11 * H, H, 'all')]);
    expect(out.days.map((d) => d.events.map((e) => [e.start_ms, e.pending]))).toEqual([
      [[MON + 11 * H, true]], [[MON + DAY + 11 * H, true]],
    ]);
  });

  test("'following' shifts from the dragged occurrence on, and leaves the ones before", () => {
    const w: WeekPayload = {
      ...week(),
      days: [
        { start_ms: MON, end_ms: MON + DAY, events: [ev(5, MON + 9 * H, MON + 10 * H, { recurring: true })], placed: [place(0)] },
        { start_ms: MON + DAY, end_ms: MON + 2 * DAY, events: [ev(5, MON + DAY + 9 * H, MON + DAY + 10 * H, { recurring: true })], placed: [place(0)] },
      ],
    };
    const out = overlayWeek(w, [move(5, MON + DAY + 9 * H, MON + DAY + 11 * H, H, 'following')]);
    expect(out.days[0].events.map((e) => [e.start_ms, e.pending ?? false])).toEqual([[MON + 9 * H, false]]);
    expect(out.days[1].events.map((e) => [e.start_ms, e.pending])).toEqual([[MON + DAY + 11 * H, true]]);
  });

  test('a delete hides the meeting, and the rest keep valid layout indices', () => {
    const w = overlayWeek(week(), [del(1, MON + 9 * H)]);
    expect(w.days[0].events.map((e) => e.id)).toEqual([2]);
    expect(w.days[0].placed).toHaveLength(1);
    expect(w.days[0].placed[0].idx).toBe(0);
    expect(w.days[1].events.map((e) => e.id)).toEqual([3]);
  });

  test('a delete of an all-day item drops its lane and remaps the others', () => {
    const w = overlayWeek(week(), [del(7, MON)]);
    expect(w.all_day_events.map((e) => e.id)).toEqual([8]);
    expect(w.all_day).toEqual([lane(0, 0, 1, 1)]);
  });

  test('a meeting moved across midnight is drawn in both days it touches', () => {
    const w = overlayWeek(week(), [move(2, MON + 10 * H, MON + 23 * H, 2 * H)]);
    expect(w.days[0].events.some((e) => e.id === 2 && e.pending)).toBe(true);
    expect(w.days[1].events.some((e) => e.id === 2 && e.pending)).toBe(true);
  });
});

test.describe('the month, with pending changes drawn over it', () => {
  const month = (): MonthPayload => ({
    year: 2024, month: 1, lane_cap: 3,
    rows: [{
      cells: [
        { start_ms: MON, end_ms: MON + DAY, in_month: true, timed: [ev(1, MON + 9 * H, MON + 10 * H), ev(2, MON + 14 * H, MON + 15 * H)] },
        { start_ms: MON + DAY, end_ms: MON + 2 * DAY, in_month: true, timed: [ev(3, MON + DAY + 12 * H, MON + DAY + 13 * H)] },
      ],
      bars: [lane(0, 0, 0, 1)],
      bar_events: [ev(9, MON, MON + 2 * DAY, { is_all_day: true })],
      bar_overflow: [0, 0],
    }],
  });

  test('a moved meeting leaves its cell and joins the new one in time order, flagged', () => {
    const out = overlayMonth(month(), [move(1, MON + 9 * H, MON + DAY + 8 * H)]);
    expect(out.rows[0].cells[0].timed.map((e) => e.id)).toEqual([2]);
    expect(out.rows[0].cells[1].timed.map((e) => [e.id, e.pending ?? false])).toEqual([[1, true], [3, false]]);
  });

  test('a delete removes a chip, and a deleted bar drops its lane', () => {
    const out = overlayMonth(month(), [del(2, MON + 14 * H), del(9, MON)]);
    expect(out.rows[0].cells[0].timed.map((e) => e.id)).toEqual([1]);
    expect(out.rows[0].bar_events).toEqual([]);
    expect(out.rows[0].bars).toEqual([]);
  });
});

test.describe('the big year, with pending deletes drawn over it', () => {
  test('a deleted pill disappears and the others keep valid indices', () => {
    const big: BigYearPayload = {
      year: 2024, lane_cap: 3,
      rows: [{ days: [], pills: [lane(0, 0, 0, 1), lane(1, 1, 2, 3)],
        pill_events: [ev(7, MON, MON + DAY, { is_all_day: true }), ev(8, MON, MON + DAY, { is_all_day: true })], overflow: [] }],
    };
    const out = overlayBigYear(big, [del(7, MON)]);
    expect(out.rows[0].pill_events.map((e) => e.id)).toEqual([8]);
    expect(out.rows[0].pills).toEqual([lane(0, 1, 2, 3)]);
  });
});
```

- [ ] **Step 3: Run the tests to see them fail**

Run (from `ui/`): `./node_modules/.bin/playwright test tests/pendingview.spec.ts --project=chromium`
Expected: FAIL — module `../src/lib/pendingview` not found.

- [ ] **Step 4: Write `pendingview.ts`**

Create `ui/src/lib/pendingview.ts`:

```ts
// Pending changes, drawn over the payloads (pending-changes spec, 2026-10-09).
//
// A change the user just made — a drag, a delete — is shown at once and written
// to Google behind them. The payloads stay exactly what the backend sent; this
// is the layer between them and the views. Pure, so `pendingview.spec.ts`
// drives every rule directly.

import type { BigYearPayload, DayColumn, Lane, MonthPayload, UiEvent, WeekPayload } from './api';
import type { Scope } from './eventform';
import { layOutDay } from './daylayout';

/** One change still on its way to Google. A move carries where the dragged
 *  occurrence landed; for a series, every covered occurrence shifts by the
 *  same amount and takes the same length. */
export type PendingChange =
  | { kind: 'move'; id: number; occurrenceStartMs: number; scope: Scope; startMs: number; endMs: number }
  | { kind: 'delete'; id: number; occurrenceStartMs: number; scope: Scope };

/**
 * Whether `change` speaks for this occurrence, as the payload has it.
 *
 * Occurrences of one series share the series' `id` (`commands::to_ui`), so
 * `'all'` is every one of them and `'following'` the ones from the change's
 * occurrence on. An exception is its own row with its own id, and updates when
 * the save lands. A block combining several calendars' copies is never
 * covered: its copies are reached through its own panel, and it updates when
 * the save lands too (spec §2's rare cases).
 */
export function covers(change: PendingChange, ev: Pick<UiEvent, 'id' | 'start_ms' | 'copies'>): boolean {
  if (ev.copies?.length) return false;
  if (ev.id !== change.id) return false;
  if (change.scope === 'all') return true;
  if (change.scope === 'following') return ev.start_ms >= change.occurrenceStartMs;
  return ev.start_ms === change.occurrenceStartMs;
}

/**
 * Whether the occurrence at `startMs` is still being written, and so may not
 * be dragged, edited or deleted again. Unlike `covers`, this also answers for
 * where a move *landed*: clicking the moved card opens a popover for its new
 * start, and that card is the pending one.
 */
export function locks(change: PendingChange, id: number, startMs: number): boolean {
  if (id !== change.id) return false;
  if (change.scope === 'all') return true;
  if (change.scope === 'following') {
    const from = change.kind === 'move' ? Math.min(change.occurrenceStartMs, change.startMs) : change.occurrenceStartMs;
    return startMs >= from;
  }
  return startMs === change.occurrenceStartMs || (change.kind === 'move' && startMs === change.startMs);
}

/** What the changes do to one occurrence: hide it, and perhaps redraw it
 *  somewhere else. Applied in the order the changes were made. */
function fate(ev: UiEvent, changes: readonly PendingChange[]): { hidden: boolean; moved: UiEvent | null } {
  let hidden = false;
  let moved: UiEvent | null = null;
  for (const c of changes) {
    if (!covers(c, ev)) continue;
    hidden = true;
    if (c.kind === 'delete') { moved = null; continue; }
    const start = ev.start_ms + (c.startMs - c.occurrenceStartMs);
    moved = { ...ev, start_ms: start, end_ms: start + (c.endMs - c.startMs), pending: true };
  }
  return { hidden, moved };
}

/** Lane-packed items (all-day band, Month bars, Big Year pills) lose only what
 *  is deleted; nothing is moved among them. `idx` is remapped so every kept
 *  lane still names its own event. */
function dropDeleted(lanes: Lane[], events: UiEvent[], changes: readonly PendingChange[]): { lanes: Lane[]; events: UiEvent[] } {
  const keep = events.map((e) => !changes.some((c) => c.kind === 'delete' && covers(c, e)));
  if (keep.every(Boolean)) return { lanes, events };
  const newIdx: number[] = [];
  const kept: UiEvent[] = [];
  events.forEach((e, i) => { if (keep[i]) { newIdx[i] = kept.length; kept.push(e); } });
  return { lanes: lanes.filter((l) => keep[l.idx]).map((l) => ({ ...l, idx: newIdx[l.idx] })), events: kept };
}

const byStart = (a: UiEvent, b: UiEvent) => a.start_ms - b.start_ms || b.end_ms - a.end_ms;

/**
 * The week as the user's pending changes have left it.
 *
 * Moved and deleted occurrences leave their days; each moved one (flagged
 * `pending`) joins every day its new span touches. A day that changed is laid
 * out again with `layOutDay`, the Rust layout's own port, so a pending card
 * sits in its proper lane beside whatever it now overlaps.
 */
export function overlayWeek(week: WeekPayload, changes: readonly PendingChange[]): WeekPayload {
  if (changes.length === 0) return week;
  const moved = new Map<string, UiEvent>();
  const kept = week.days.map((day) => day.events.filter((e) => {
    const f = fate(e, changes);
    if (f.moved) moved.set(`${e.id}:${e.start_ms}`, f.moved);
    return !f.hidden;
  }));
  const days = week.days.map((day, i): DayColumn => {
    const arrivals = [...moved.values()].filter((e) => e.start_ms < day.end_ms && e.end_ms > day.start_ms);
    if (kept[i].length === day.events.length && arrivals.length === 0) return day;
    const events = [...kept[i], ...arrivals].sort(byStart);
    const placed = layOutDay(events.map((e) => ({ startMs: e.start_ms, endMs: e.end_ms })), day.start_ms, day.end_ms);
    return { ...day, events, placed };
  });
  const band = dropDeleted(week.all_day, week.all_day_events, changes);
  return { ...week, days, all_day: band.lanes, all_day_events: band.events };
}

/**
 * The month as the pending changes have left it. A moved timed meeting joins
 * the cell it now starts in, in time order, if it still fits within that day;
 * one moved across midnight shows when the save lands (spec §2's rare cases).
 */
export function overlayMonth(month: MonthPayload, changes: readonly PendingChange[]): MonthPayload {
  if (changes.length === 0) return month;
  const moved = new Map<string, UiEvent>();
  const kept = month.rows.map((row) => row.cells.map((cell) => cell.timed.filter((e) => {
    const f = fate(e, changes);
    if (f.moved) moved.set(`${e.id}:${e.start_ms}`, f.moved);
    return !f.hidden;
  })));
  const rows = month.rows.map((row, r) => {
    const cells = row.cells.map((cell, c) => {
      const arrivals = [...moved.values()].filter((e) =>
        e.start_ms >= cell.start_ms && e.start_ms < cell.end_ms && e.end_ms <= cell.end_ms);
      if (kept[r][c].length === cell.timed.length && arrivals.length === 0) return cell;
      return { ...cell, timed: [...kept[r][c], ...arrivals].sort(byStart) };
    });
    const bars = dropDeleted(row.bars, row.bar_events, changes);
    return { ...row, cells, bars: bars.lanes, bar_events: bars.events };
  });
  return { ...month, rows };
}

/** The Big Year ribbon draws only all-day and multi-day spans, which are never
 *  dragged, so only a delete changes it. */
export function overlayBigYear(big: BigYearPayload, changes: readonly PendingChange[]): BigYearPayload {
  if (!changes.some((c) => c.kind === 'delete')) return big;
  return {
    ...big,
    rows: big.rows.map((row) => {
      const pills = dropDeleted(row.pills, row.pill_events, changes);
      return { ...row, pills: pills.lanes, pill_events: pills.events };
    }),
  };
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run (from `ui/`): `./node_modules/.bin/playwright test tests/pendingview.spec.ts`
Expected: PASS on both projects.

- [ ] **Step 6: Prove each rule red**

One mutation at a time, run the spec, see the named test fail, restore (`git diff --quiet ui/src/lib/pendingview.ts` must be true after each restore):
- In `covers`, delete `if (ev.copies?.length) return false;` → `a block combining several calendars is never covered` fails.
- In `locks`, drop `|| (change.kind === 'move' && startMs === change.startMs)` → `locks the occurrence where it landed` fails.
- In `overlayWeek`, replace `layOutDay(...)` with `day.placed` → `lands, laid out` fails.
- In `dropDeleted`, replace `idx: newIdx[l.idx]` with `idx: l.idx` → the all-day remap test fails.
- In `overlayMonth`, drop `.sort(byStart)` → the month time-order test fails.

- [ ] **Step 7: Type-check and commit**

Run: `npm --prefix ui run check` → 0 errors.

```bash
git add ui/src/lib/pendingview.ts ui/src/lib/api.ts ui/tests/pendingview.spec.ts
git commit -m "feat(pending): draw pending moves and deletes over the payloads

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: The store — `PendingQueue` and its rune wrapper

**Files:**
- Create: `ui/src/lib/pendingqueue.ts`
- Create: `ui/src/lib/pending.svelte.ts`
- Test: `ui/tests/pendingqueue.spec.ts`

**Interfaces:**
- Consumes: `PendingChange`, `locks` from Task 1.
- Produces (`pendingqueue.ts`):
  - `type Work = { write: () => Promise<unknown>; after: () => Promise<void>; onfailure: (error: unknown) => void }`
  - `class PendingQueue { constructor(onchange?: () => void); changes(): PendingChange[]; count(): number; isPending(id: number, startMs: number): boolean; hold(change: PendingChange): symbol; release(token: symbol): void; commit(token: symbol, change: PendingChange, work: Work): Promise<void>; queue(change: PendingChange, work: Work): Promise<void>; checkpoint(): number; reconcile(checkpoint: number): void; idle(): Promise<void> }`
- Produces (`pending.svelte.ts`): `pendingChanges(): PendingChange[]`, `pendingChangeCount(): number`, `isPending(id, startMs): boolean`, `holdChange(change): symbol`, `releaseHold(token): void`, `commitChange(token, change, work): Promise<void>`, `queueChange(change, work): Promise<void>`, `pendingCheckpoint(): number`, `reconcilePending(checkpoint): void`, `pendingIdle(): Promise<void>`.

- [ ] **Step 1: Write the failing tests**

Create `ui/tests/pendingqueue.spec.ts`:

```ts
import { test, expect } from '@playwright/test';
import { PendingQueue, type Work } from '../src/lib/pendingqueue';
import type { PendingChange } from '../src/lib/pendingview';

const move = (id: number, from = 100, to = 200): PendingChange =>
  ({ kind: 'move', id, occurrenceStartMs: from, scope: 'this', startMs: to, endMs: to + 50 });

/** A write the test resolves or rejects by hand. */
function gate() {
  let resolve!: () => void; let reject!: (e: unknown) => void;
  const promise = new Promise<void>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}
const work = (write: () => Promise<unknown>, extra: Partial<Work> = {}): Work =>
  ({ write, after: async () => {}, onfailure: () => {}, ...extra });

test('a held change is drawn but not counted, and release takes it away', () => {
  const q = new PendingQueue();
  const t = q.hold(move(1));
  expect(q.changes()).toHaveLength(1);
  expect(q.count()).toBe(0);
  q.release(t);
  expect(q.changes()).toHaveLength(0);
});

test('a committed change is counted until written, and takes the scope it was committed with', async () => {
  const q = new PendingQueue();
  const g = gate();
  const t = q.hold(move(1));
  const done = q.commit(t, { ...move(1), scope: 'all' }, work(() => g.promise));
  expect(q.count()).toBe(1);
  expect(q.changes()[0].scope).toBe('all');
  g.resolve();
  await done;
  expect(q.count()).toBe(0);
  expect(q.changes()).toHaveLength(1); // saved, still drawn until a load lands
});

test('writes run one at a time, in the order made', async () => {
  const q = new PendingQueue();
  const order: string[] = [];
  const a = gate();
  const one = q.queue(move(1), work(async () => { order.push('a start'); await a.promise; order.push('a end'); }));
  const two = q.queue(move(2), work(async () => { order.push('b'); }));
  await Promise.resolve();
  expect(order).toEqual(['a start']);
  a.resolve();
  await Promise.all([one, two]);
  expect(order).toEqual(['a start', 'a end', 'b']);
});

test('a refused write drops the change, reports it, and does not stall the next', async () => {
  const q = new PendingQueue();
  const failures: unknown[] = [];
  const one = q.queue(move(1), work(async () => { throw new Error('no longer here'); }, { onfailure: (e) => failures.push(e) }));
  const two = q.queue(move(2), work(async () => {}));
  await Promise.all([one, two]);
  expect(String(failures[0])).toContain('no longer here');
  expect(q.changes().map((c) => c.id)).toEqual([2]);
});

test('a load begun before the save does not clear it; one begun after does', async () => {
  const q = new PendingQueue();
  const g = gate();
  const done = q.queue(move(1), work(() => g.promise));
  const before = q.checkpoint();  // a reload already in flight
  g.resolve();
  await done;
  q.reconcile(before);
  expect(q.changes()).toHaveLength(1);
  const after = q.checkpoint();   // a reload that started after the save
  q.reconcile(after);
  expect(q.changes()).toHaveLength(0);
});

test('after() failing keeps the change drawn: the write did happen', async () => {
  const q = new PendingQueue();
  await q.queue(move(1), work(async () => {}, { after: async () => { throw new Error('offline'); } }));
  expect(q.changes()).toHaveLength(1);
  expect(q.count()).toBe(0);
});

test('isPending answers for where a move came from and where it landed', () => {
  const q = new PendingQueue();
  q.hold(move(1, 100, 200));
  expect(q.isPending(1, 100)).toBe(true);
  expect(q.isPending(1, 200)).toBe(true);
  expect(q.isPending(1, 300)).toBe(false);
});

test('every change of state is announced', async () => {
  let n = 0;
  const q = new PendingQueue(() => { n++; });
  const t = q.hold(move(1));
  await q.commit(t, move(1), work(async () => {}));
  q.reconcile(q.checkpoint());
  expect(n).toBeGreaterThanOrEqual(4); // hold, commit, saved, reconcile
});
```

- [ ] **Step 2: Run the tests to see them fail**

Run (from `ui/`): `./node_modules/.bin/playwright test tests/pendingqueue.spec.ts --project=chromium`
Expected: FAIL — module `../src/lib/pendingqueue` not found.

- [ ] **Step 3: Write `pendingqueue.ts`**

Create `ui/src/lib/pendingqueue.ts`:

```ts
// The pending-changes queue (spec 2026-10-09), as a plain class so Node-run
// specs can drive it: `pending.svelte.ts` wraps it in a rune for the views.
//
// `responses.svelte.ts` is the model, deliberately: one tail so writes run in
// order, and the checkpoint rule so a load already in flight when a save
// finishes cannot clear what that save is about.

import { locks, type PendingChange } from './pendingview';

export type Work = {
  /** The write to Google. A rejection means nothing changed there: the change
   *  is dropped from the screen and `onfailure` says why. */
  write: () => Promise<unknown>;
  /** After a successful write: sync and reload. Its own failure is its own to
   *  report, and does not undo the change: Google has it. */
  after: () => Promise<void>;
  onfailure: (error: unknown) => void;
};

/** `seq` is null while held (a question is open) and set once committed. */
type Entry = { token: symbol; change: PendingChange; seq: number | null; saved: boolean };

export class PendingQueue {
  private entries: Entry[] = [];
  private seq = 0;
  private tail: Promise<unknown> = Promise.resolve();

  constructor(private readonly onchange: () => void = () => {}) {}

  /** Every change to draw: held, being written, and written but not yet
   *  reloaded. */
  changes(): PendingChange[] {
    return this.entries.map((e) => e.change);
  }

  /** Changes still being written: the header's count. A held one is not
   *  counted, because nothing is being saved yet. */
  count(): number {
    return this.entries.filter((e) => e.seq !== null && !e.saved).length;
  }

  isPending(id: number, startMs: number): boolean {
    return this.entries.some((e) => locks(e.change, id, startMs));
  }

  hold(change: PendingChange): symbol {
    const token = Symbol('pending');
    this.entries = [...this.entries, { token, change, seq: null, saved: false }];
    this.onchange();
    return token;
  }

  /** Cancel a held change. A committed one cannot be released. */
  release(token: symbol): void {
    const before = this.entries.length;
    this.entries = this.entries.filter((e) => e.token !== token || e.seq !== null);
    if (this.entries.length !== before) this.onchange();
  }

  /** Turn a held change into a write. `change` replaces what was held, because
   *  the answer to a question (the scope) can change what is drawn. */
  commit(token: symbol, change: PendingChange, work: Work): Promise<void> {
    const seq = ++this.seq;
    const held = this.entries.some((e) => e.token === token);
    this.entries = held
      ? this.entries.map((e) => (e.token === token ? { ...e, change, seq } : e))
      : [...this.entries, { token, change, seq, saved: false }];
    this.onchange();
    const done = this.tail.then(async () => {
      try {
        await work.write();
      } catch (error) {
        this.entries = this.entries.filter((e) => e.token !== token);
        this.onchange();
        work.onfailure(error);
        return;
      }
      this.entries = this.entries.map((e) => (e.token === token ? { ...e, saved: true } : e));
      this.onchange();
      try { await work.after(); } catch { /* `after` reports its own failure */ }
    });
    this.tail = done;
    return done;
  }

  queue(change: PendingChange, work: Work): Promise<void> {
    return this.commit(this.hold(change), change, work);
  }

  /** Taken when a load begins: the highest change already saved. */
  checkpoint(): number {
    return Math.max(0, ...this.entries.filter((e) => e.saved).map((e) => e.seq ?? 0));
  }

  /** Called when that load has landed: what it already contains is no longer
   *  pending. */
  reconcile(checkpoint: number): void {
    const before = this.entries.length;
    this.entries = this.entries.filter((e) => !(e.saved && (e.seq ?? 0) <= checkpoint));
    if (this.entries.length !== before) this.onchange();
  }

  /** Resolves once every queued write, including ones added meanwhile, is done. */
  async idle(): Promise<void> {
    let batch;
    do { batch = this.tail; await batch; } while (batch !== this.tail);
  }
}
```

- [ ] **Step 4: Write the rune wrapper**

Create `ui/src/lib/pending.svelte.ts`:

```ts
// The pending-changes store the views read (spec 2026-10-09). The logic is
// `PendingQueue`'s; this only makes it reactive, by bumping `version` on every
// change and reading it in each getter.

import { PendingQueue, type Work } from './pendingqueue';
import type { PendingChange } from './pendingview';

let version = $state(0);
const queue = new PendingQueue(() => { version++; });

export const pendingChanges = (): PendingChange[] => { void version; return queue.changes(); };
export const pendingChangeCount = (): number => { void version; return queue.count(); };
export const isPending = (id: number, startMs: number): boolean => { void version; return queue.isPending(id, startMs); };
export const holdChange = (change: PendingChange): symbol => queue.hold(change);
export const releaseHold = (token: symbol): void => queue.release(token);
export const commitChange = (token: symbol, change: PendingChange, work: Work): Promise<void> =>
  queue.commit(token, change, work);
export const queueChange = (change: PendingChange, work: Work): Promise<void> => queue.queue(change, work);
export const pendingCheckpoint = (): number => queue.checkpoint();
export const reconcilePending = (checkpoint: number): void => queue.reconcile(checkpoint);
export const pendingIdle = (): Promise<void> => queue.idle();
```

- [ ] **Step 5: Run the tests to see them pass**

Run (from `ui/`): `./node_modules/.bin/playwright test tests/pendingqueue.spec.ts`
Expected: PASS on both projects. Then `npm --prefix ui run check` → 0 errors.

- [ ] **Step 6: Prove each rule red**

One mutation at a time, restore after each:
- `count()`: drop `e.seq !== null &&` → `a held change is drawn but not counted` fails.
- `commit`: map `{ ...e, seq }` instead of `{ ...e, change, seq }` → `takes the scope it was committed with` fails.
- `commit`: replace `this.tail.then(async () => …)` with `(async () => …)()` → `one at a time` fails.
- `reconcile`: use `< checkpoint + 1000` → `a load begun before the save does not clear it` fails.
- `commit`: move `work.after()` inside the first `try` → `after() failing keeps the change` fails.

- [ ] **Step 7: Commit**

```bash
git add ui/src/lib/pendingqueue.ts ui/src/lib/pending.svelte.ts ui/tests/pendingqueue.spec.ts
git commit -m "feat(pending): the queue — one write at a time, cleared only by a later load

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Harness — hold and reject a write

**Files:**
- Modify: `ui/tests/harness/tauri.ts` (state near `let holdSyncOnce` ~line 196; API type near `failNextUpdate` ~line 74; implementation near `holdNextSync()` ~line 321; the `update_event` and `delete_event_cmd` cases ~line 1343)

**Interfaces:**
- Produces on `window.__harness`: `holdNextWrite(cmd: 'update_event' | 'delete_event_cmd'): void`, `releaseWrite(): Promise<void>`, `rejectWrite(message: string): Promise<void>`.

- [ ] **Step 1: Declare the API**

In the harness API type, after the `failNextUpdate(message: string): void;` member, add:

```ts
  /** Park the next call to `cmd` until `releaseWrite` or `rejectWrite`: what a
   *  pending-changes spec uses to look at the screen while Google has not
   *  answered yet. */
  holdNextWrite(cmd: 'update_event' | 'delete_event_cmd'): void;
  /** Answer a parked write as the real command would. */
  releaseWrite(): Promise<void>;
  /** Fail a parked write with `message`. */
  rejectWrite(message: string): Promise<void>;
```

- [ ] **Step 2: Add the state**

Below `let parkedSync: … | null = null;`, add:

```ts
/** A held `update_event` / `delete_event_cmd`, and the flag that arms one. */
let holdWriteOnce: 'update_event' | 'delete_event_cmd' | null = null;
let parkedWrite: { resolve: () => void; reject: (message: string) => void } | null = null;
```

- [ ] **Step 3: Implement the API**

In the harness object, after `holdNextSync() { holdSyncOnce = true; },`, add:

```ts
  holdNextWrite(cmd) { holdWriteOnce = cmd; },
  async releaseWrite() {
    parkedWrite?.resolve();
    parkedWrite = null;
    await new Promise((r) => setTimeout(r, 50));
  },
  async rejectWrite(message) {
    parkedWrite?.reject(message);
    parkedWrite = null;
    await new Promise((r) => setTimeout(r, 50));
  },
```

- [ ] **Step 4: Park the two commands**

At the top of `case 'update_event':` (before the `failUpdateOnce` check), add:

```ts
        if (holdWriteOnce === 'update_event') {
          holdWriteOnce = null;
          const answer = POPOVER_DETAILS[args.id] ?? CREATED_DETAIL;
          return new Promise((resolve, reject) => {
            parkedWrite = { resolve: () => resolve(answer), reject: (m) => reject(new Error(m)) };
          });
        }
```

At the top of `case 'delete_event_cmd':`, add:

```ts
        if (holdWriteOnce === 'delete_event_cmd') {
          holdWriteOnce = null;
          return new Promise((resolve, reject) => {
            parkedWrite = { resolve: () => resolve(null), reject: (m) => reject(new Error(m)) };
          });
        }
```

- [ ] **Step 5: Type-check and commit**

Run: `npm --prefix ui run check` → 0 errors (the harness is covered by `tsconfig.test.json`).

```bash
git add ui/tests/harness/tauri.ts
git commit -m "test(harness): hold or reject the next update_event / delete_event_cmd

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Draw pending changes — App's overlaid payloads and the pending style

**Files:**
- Modify: `ui/src/App.svelte` (imports; `visibleWeek` ~line 996; `keyboardDays` ~line 569; the four loaders ~lines 1017–1080; the render branches ~lines 2290–2345)
- Modify: `ui/src/lib/EventBlock.svelte` (button ~line 202, grips ~line 256, styles ~line 366)
- Modify: `ui/src/lib/WeekGrid.svelte` (`onedit`/`ongrab` on `EventBlock`, ~lines 2246–2248)
- Modify: `ui/src/lib/MonthGrid.svelte` (`.timed` chip ~line 184, styles ~line 288)
- Modify: `ui/src/lib/Filmstrip.svelte` (`.srow` ~line 182)

**Interfaces:**
- Consumes: `overlayWeek`, `overlayMonth`, `overlayBigYear` (Task 1); `pendingChanges`, `pendingCheckpoint`, `reconcilePending` (Task 2).
- Produces: App state `shownWeek`, `shownMonth`, `shownBigYear` (derived), consumed by Tasks 5–6 through the views. No behaviour change on its own: with no pending changes the overlays return their input unchanged.

- [ ] **Step 1: Imports**

In `ui/src/App.svelte`, below `import { responsesIdle, responseCheckpoint, reconcileResponses } from './lib/responses.svelte';`, add:

```ts
  import { pendingChanges, pendingCheckpoint, reconcilePending } from './lib/pending.svelte';
  import { overlayWeek, overlayMonth, overlayBigYear } from './lib/pendingview';
```

- [ ] **Step 2: Derive the overlaid payloads, and read the week through them**

Replace the `visibleWeek` derivation:

```ts
  const visibleWeek = $derived.by(() => {
    if (!week) return null;
    const i = visibleIndex(week.days, visibleStartMs);
    // Not there yet — the window jumped and its payload is still in flight —
    // so the whole of what is on screen stands in, as it did before padding.
    return i < 0 ? week : sliceWeek(week, i, visibleCount);
  });
```

with:

```ts
  // The payloads as the user's pending changes have left them (spec
  // 2026-10-09): what every view draws. `week`/`month`/`bigYear` stay exactly
  // what the backend sent.
  const shownWeek = $derived(week ? overlayWeek(week, pendingChanges()) : null);
  const shownMonth = $derived(month ? overlayMonth(month, pendingChanges()) : null);
  const shownBigYear = $derived(bigYear ? overlayBigYear(bigYear, pendingChanges()) : null);

  const visibleWeek = $derived.by(() => {
    if (!shownWeek) return null;
    const i = visibleIndex(shownWeek.days, visibleStartMs);
    // Not there yet — the window jumped and its payload is still in flight —
    // so the whole of what is on screen stands in, as it did before padding.
    return i < 0 ? shownWeek : sliceWeek(shownWeek, i, visibleCount);
  });
```

In `keyboardDays`, replace `if (view === 'month' && month) {` with `if (view === 'month' && shownMonth) {` and `const days = allDaysFromMonth(month);` with `const days = allDaysFromMonth(shownMonth);`.

- [ ] **Step 3: Reconcile in every loader**

Each of `loadWeek`, `loadMonth`, `loadYear`, `loadBigYear` has the pair:

```ts
    const responseVersion = untrack(responseCheckpoint);
```
and, after the payload is assigned,
```ts
      reconcileResponses(responseVersion);
```

In all four, add directly below the first line:

```ts
    const pendingVersion = untrack(pendingCheckpoint);
```
and directly below the second:
```ts
      reconcilePending(pendingVersion);
```

(Indentation differs between the loaders; match each one. `grep -n "reconcileResponses(responseVersion)" ui/src/App.svelte` must list exactly four lines, each followed by a `reconcilePending(pendingVersion);` line.)

- [ ] **Step 4: Hand the overlaid payloads to the views**

In the Month branch, replace:

```svelte
      {#if month}
        {#if listMode}
          <Filmstrip days={daysFromMonth(month)} {weather} {weatherStale} onweather={openWeather} {revealNowRequest}
```
with
```svelte
      {#if shownMonth}
        {#if listMode}
          <Filmstrip days={daysFromMonth(shownMonth)} {weather} {weatherStale} onweather={openWeather} {revealNowRequest}
```
and `<MonthGrid {month} keyboardCursor=` with `<MonthGrid month={shownMonth} keyboardCursor=`.

In the Big Year branch, replace `{#if bigYear}` with `{#if shownBigYear}` and `ribbon={bigYear}` with `ribbon={shownBigYear}`.

In the Week branch, replace `{:else if week && visibleWeek}` with `{:else if shownWeek && visibleWeek}` and `<WeekGrid {week} {calendars}` with `<WeekGrid week={shownWeek} {calendars}`.

- [ ] **Step 5: The pending style, and no grab, in `EventBlock` and `WeekGrid`**

In `ui/src/lib/EventBlock.svelte`, on the `<button class="ev {event.response}"` element, add after `class:obscured`:

```svelte
  class:pending={event.pending}
```

Change the grips condition `{#if grips && !preview && !createMode && !event.copies?.length}` to `{#if grips && !preview && !createMode && !event.copies?.length && !event.pending}`.

After the `.ev.dragging { … }` rule, add:

```css
  /* Saving (pending-changes spec): where the change put it, faded and dashed
     until Google confirms. Not grabbable — `WeekGrid` ignores the press — so
     it wears the pointer, not `grab`. */
  .ev.pending { opacity: .62; outline: 1.5px dashed var(--cal); outline-offset: -1.5px; }
  .ev.pending.hovered { cursor: pointer; }
```

In `ui/src/lib/WeekGrid.svelte`, on the `<EventBlock` inside `{#each layout.placed as p …}`, replace:

```svelte
          onedit={(ev, r) => openPopover(ev, r, true)}
          ongrab={(ev, e) => startDrag(ev, day, e)}
```
with
```svelte
          onedit={(ev, r) => { if (!ev.pending) openPopover(ev, r, true); }}
          ongrab={(ev, e) => { if (!ev.pending) startDrag(ev, day, e); }}
```

- [ ] **Step 6: The same mark in Month and list mode**

In `ui/src/lib/MonthGrid.svelte`, on the `<button class="timed"` chip, add `class:pending={ev.pending}` after `class="timed"`. After the `.timed { … }` rule, add:

```css
  .timed.pending { opacity: .62; outline: 1px dashed var(--cal); outline-offset: -1px; border-radius: 3px; }
```

In `ui/src/lib/Filmstrip.svelte`, on the `<button class="srow"`, add `class:pending={ev.pending}` after `class:nobodycoming={ev.all_guests_declined}`, and in its `<style>` add:

```css
  .srow.pending { opacity: .62; }
```

- [ ] **Step 7: Verify nothing changed without pending changes**

Run: `npm --prefix ui run check` → 0 errors.
Run (from `ui/`): `./node_modules/.bin/playwright test tests/app.spec.ts tests/components.spec.ts > ../target/pw-t4.log 2>&1; echo EXIT=$?` and read `grep -E "EXIT=|[0-9]+ (passed|failed)" ../target/pw-t4.log`. Expected: exit 0 — with no pending changes every overlay returns its input, so behaviour is identical.

- [ ] **Step 8: Commit**

```bash
git add ui/src/App.svelte ui/src/lib/EventBlock.svelte ui/src/lib/WeekGrid.svelte ui/src/lib/MonthGrid.svelte ui/src/lib/Filmstrip.svelte
git commit -m "feat(pending): every view draws the payloads with pending changes over them

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Moves and resizes show at once

**Files:**
- Modify: `ui/src/App.svelte` (`pendingMove` type ~line 1912; `moveOccurrence` ~line 1932; `commitMove` ~line 1967; new `refreshAfterQueuedWrite` after `refreshAfterWrite` ~line 1824; `MoveConfirm` mount ~line 2352)
- Test: `ui/tests/app.spec.ts` (inside `test.describe('dragging an event writes without notifying anybody', …)`, after the test `'a write that fails is reported and moves nothing'`)

**Interfaces:**
- Consumes: `holdChange`, `releaseHold`, `commitChange` (Task 2); `PendingChange` (Task 1); harness `holdNextWrite`/`releaseWrite`/`rejectWrite` (Task 3).
- Produces: `refreshAfterQueuedWrite(): Promise<void>` (also used by Task 6).

- [ ] **Step 1: Write the failing specs**

Inside the describe, after `'a write that fails is reported and moves nothing'`, add:

```ts
    /** Where `title`'s block sits in its column, as `offsetTop` (the frame a
     *  drag moves it in; see the failed-write spec above for why not a box). */
    const topOfBlock = (page: Page, title: string) => page.evaluate((t) => {
      const e = [...document.querySelectorAll('.ev')]
        .find((n) => n.getAttribute('aria-label')?.startsWith(`${t},`)) as HTMLElement;
      return e.offsetTop;
    }, title);

    test.describe('a change shows at once and saves behind you', () => {
      test('a dropped meeting stays where it lands while it saves', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('update_event'));
        const before = await topOfBlock(page, 'Board prep');

        await dragBy(page, 'Board prep', 60);
        await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);

        // Google has not answered yet: at the new place, marked, and counted.
        await expect(block(page, 'Board prep')).toHaveClass(/pending/);
        expect(await topOfBlock(page, 'Board prep')).toBeGreaterThan(before + 20);
        await expect(page.locator('header [role="status"]').filter({ hasText: 'Saving 1 change' })).toBeVisible();

        await page.evaluate(() => window.__harness.releaseWrite());
        // Saved and reloaded. (The stub's week does not apply writes, so where
        // the block ends up afterwards says nothing; only that it is no longer
        // pending does.)
        await expect(block(page, 'Board prep')).not.toHaveClass(/pending/);
        await expect(page.locator('header [role="status"]').filter({ hasText: 'change' })).toHaveCount(0);
      });

      test('a refused move goes back where it was, with the reason', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('update_event'));
        const before = await topOfBlock(page, 'Board prep');

        await dragBy(page, 'Board prep', 60);
        await expect(block(page, 'Board prep')).toHaveClass(/pending/);
        await page.evaluate(() => window.__harness.rejectWrite('that event is no longer here'));

        await expect(page.locator('.err')).toContainText('Could not move “Board prep”');
        await expect(page.locator('.err')).toContainText('no longer here');
        await expect(block(page, 'Board prep')).not.toHaveClass(/pending/);
        expect(await topOfBlock(page, 'Board prep')).toBeCloseTo(before, 0);
      });

      test('a drop that asks first holds while it asks, and Cancel puts it back', async ({ page }) => {
        await writable(page);
        const before = await topOfBlock(page, 'Client call');

        await dragBy(page, 'Client call', 60);
        await expect(movePanel(page)).toBeVisible();
        await expect(block(page, 'Client call')).toHaveClass(/pending/);
        expect(await topOfBlock(page, 'Client call')).toBeGreaterThan(before + 20);
        // Held, not saving: nothing has been sent.
        await expect(page.locator('header [role="status"]').filter({ hasText: 'change' })).toHaveCount(0);

        await page.keyboard.press('Escape');
        await expect(movePanel(page)).toBeHidden();
        await expect(block(page, 'Client call')).not.toHaveClass(/pending/);
        expect(await topOfBlock(page, 'Client call')).toBeCloseTo(before, 0);
        expect(await callsTo(page, 'update_event')).toHaveLength(0);
      });

      test('a meeting still saving cannot be dragged again', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('update_event'));
        await dragBy(page, 'Board prep', 60);
        await expect(block(page, 'Board prep')).toHaveClass(/pending/);

        await dragBy(page, 'Board prep', 60);
        await page.waitForTimeout(300);
        expect(await callsTo(page, 'update_event')).toHaveLength(1);
        await page.evaluate(() => window.__harness.releaseWrite());
      });

      test('a move still saving survives a week step and back', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('update_event'));
        await dragBy(page, 'Board prep', 60);
        await expect(block(page, 'Board prep')).toHaveClass(/pending/);
        const moved = await topOfBlock(page, 'Board prep');

        await page.getByRole('button', { name: 'Next week' }).click();
        await page.getByRole('button', { name: 'Previous week' }).click();
        await expect(block(page, 'Board prep')).toHaveClass(/pending/);
        expect(await topOfBlock(page, 'Board prep')).toBeCloseTo(moved, 0);
        await page.evaluate(() => window.__harness.releaseWrite());
      });
    });
```

The week-step buttons are `Header.svelte`'s `aria-label="Next {unit}"` / `"Previous {unit}"`, which read "Next week" / "Previous week" in Week view (the existing specs click them by those names).

- [ ] **Step 2: Run them to see them fail**

Run (from `ui/`): `./node_modules/.bin/playwright test tests/app.spec.ts -g "a change shows at once" --project=chromium`
Expected: FAIL — the dropped block has no `pending` class (the old flow snaps it back).

- [ ] **Step 3: Carry the hold through `pendingMove`**

Replace the `pendingMove` declaration's type:

```ts
  let pendingMove = $state<{
    event: UiEvent;
    span: { startMs: number; endMs: number };
    detail: EventDetail;
    anchor: Rect;
  } | null>(null);
```
with
```ts
  let pendingMove = $state<{
    event: UiEvent;
    span: { startMs: number; endMs: number };
    detail: EventDetail;
    anchor: Rect;
    /** The drop, already drawn where it landed while the question is open. */
    hold: symbol;
  } | null>(null);
```

Add these imports to the block from Task 4, Step 1:

```ts
  import { holdChange, releaseHold, commitChange } from './lib/pending.svelte';
  import type { PendingChange } from './lib/pendingview';
```
(merge into the existing `./lib/pending.svelte` import line rather than importing the module twice).

- [ ] **Step 4: Rewrite `moveOccurrence` and `commitMove`**

Replace the whole `moveOccurrence` function (keep its doc comment above) with:

```ts
  async function moveOccurrence(event: UiEvent, span: { startMs: number; endMs: number }) {
    error = null;
    // Drawn where it was dropped from this instant: while the detail is read,
    // and while any question is open (spec 2026-10-09, §2.4). Held as `'this'`;
    // the answer re-makes it with the scope chosen.
    const hold = holdChange(moveChange(event, span, 'this'));
    let detail: EventDetail;
    try {
      detail = await getEventDetail(event.id);
    } catch (e) {
      releaseHold(hold);
      error = String(e);
      return;
    }

    // Anybody the move could email. None on CalDAV however many attendees
    // there are (`mails_guests`), so a one-off there moves unasked — the same
    // silence the form's Save keeps on that provider.
    const guests = detail.mails_guests ? detail.attendees.filter((a) => !a.is_self).length : 0;
    if (guests === 0 && !detail.is_recurring) {
      // Nobody to tell and one occurrence to move: nothing to ask.
      commitMove(event, span, { scope: 'all', sendUpdates: 'none' }, hold);
      return;
    }

    const anchor = gridRectFor(event);
    pendingMove = { event, span, detail, anchor, hold };
  }

  /** The change a drop makes, for the overlay to draw. */
  function moveChange(event: UiEvent, span: { startMs: number; endMs: number }, scope: Scope): PendingChange {
    return { kind: 'move', id: event.id, occurrenceStartMs: event.start_ms, scope, startMs: span.startMs, endMs: span.endMs };
  }
```

Replace the whole `commitMove` function (keep its doc comment, and add the line below to it) with:

```ts
  function commitMove(
    event: UiEvent,
    span: { startMs: number; endMs: number },
    choice: { scope: Scope; sendUpdates: SendUpdates },
    hold: symbol,
  ) {
    void commitChange(hold, moveChange(event, span, choice.scope), {
      write: async () => {
        const detail = await getEventDetail(event.id);
        const value = valueFromDetail(detail, event.start_ms, event.end_ms);
        // The source instants are carried through untouched, and that is
        // deliberate rather than an oversight: `instantOf` passes one through
        // only while the civil pair beside it still reads as that instant, which
        // after a move it never does. Nulling them would be dead code here — a
        // mutation keeping them reddened nothing — and actively wrong for the
        // resize this grows into, where the *untouched* end should be sent as
        // the instant it was read off rather than re-derived without its
        // seconds.
        const moved: EventFormValue = {
          ...value,
          date: dateOf(span.startMs),
          endDate: dateOf(span.endMs),
          start: timeOf(span.startMs),
          end: timeOf(span.endMs),
        };
        await updateEvent(
          event.id,
          choice.scope,
          event.start_ms,
          toEventInput(moved, value, zoneName()),
          choice.sendUpdates,
        );
      },
      after: refreshAfterQueuedWrite,
      // The drop is undone on screen the moment Google refuses it, and says
      // why: §6's "a drag that appears to have worked and silently did not is
      // worse than one that visibly refuses" still holds, it just holds now.
      onfailure: (e) => { error = `Could not move “${event.title}”: ${String(e)}`; },
    });
  }
```

In `commitMove`'s doc comment, add a final paragraph:

```ts
   *
   * Since 2026-10-09 it does not wait: the drop is already drawn where it
   * landed (`pending.svelte.ts`), and the write runs in the queue behind it.
```

- [ ] **Step 5: The refresh a queued write ends with**

After the `refreshAfterWrite` function, add:

```ts
  /**
   * `refreshAfterWrite` for a queued change (spec 2026-10-09). Sync first and
   * reload once: the reload is what clears the pending card, and a local
   * reload before the sync could clear it with the old position still in the
   * store (the `'this'`-on-a-bare-master case `refreshAfterWrite` describes).
   * No `busy`: the app stays usable while this runs. If the sync fails, the
   * card stays drawn where it landed, because Google has the change, and the
   * next successful load clears it.
   */
  async function refreshAfterQueuedWrite() {
    try {
      await syncCalendar(true);
      await refreshStatus();
      await reload();
      await refreshInvites();
    } catch (e) {
      error = `The change was made, but OmaCal could not refresh from Google: ${e}`;
    }
  }
```

- [ ] **Step 6: Cancel releases, confirm commits**

In the `{#if pendingMove}` block, replace:

```svelte
      const { event, span } = p;
      pendingMove = null;
      commitMove(event, span, choice);
    }}
    oncancel={() => (pendingMove = null)}
```
with
```svelte
      const { event, span, hold } = p;
      pendingMove = null;
      commitMove(event, span, choice, hold);
    }}
    oncancel={() => {
      // Nothing written: the drop goes back where it came from.
      if (pendingMove) releaseHold(pendingMove.hold);
      pendingMove = null;
    }}
```

and update the comment above `<MoveConfirm` from "Cancelling clears this and writes nothing — the block is already home." to "Cancelling releases the hold and writes nothing — the block goes back."

- [ ] **Step 7: Run the new specs to see them pass**

Run (from `ui/`): `./node_modules/.bin/playwright test tests/app.spec.ts -g "a change shows at once"`
Expected: PASS, both projects. Then `npm --prefix ui run check` → 0 errors.

- [ ] **Step 8: Prove each red**

One at a time, restore after each:
- In `moveOccurrence`, move the `holdChange` line to just before `pendingMove = …` → `a dropped meeting stays where it lands` fails (no hold on the no-dialog path).
- In the `oncancel`, delete the `releaseHold` line → `Cancel puts it back` fails.
- In `WeekGrid`, revert `ongrab` to `startDrag(ev, day, e)` → `cannot be dragged again` fails.
- In `commitMove`, change `onfailure` to do nothing → `a refused move goes back` fails on the `.err` text.

- [ ] **Step 9: Run the existing drag specs and reconcile any that encode the old snap-back**

Run (from `ui/`): `./node_modules/.bin/playwright test tests/app.spec.ts -g "dragging an event writes" > ../target/pw-t5.log 2>&1; echo EXIT=$?`.
`'a write that fails is reported and moves nothing'` must still pass unchanged (the failure message still contains `no longer here`, and the block is back). If any other spec fails, read its assertion: only a spec asserting that the block is back at its old place *immediately after the drop, before the write answers* encodes the old behaviour; update it to the new truth with a one-line comment naming this spec, and leave every other kind of failure as a bug to fix.

- [ ] **Step 10: Commit**

```bash
git add ui/src/App.svelte ui/tests/app.spec.ts
git commit -m "feat(pending): a drop stays where it lands while Google saves it

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Deletes, the locks, and the header count

**Files:**
- Modify: `ui/src/App.svelte` (`runDelete` ~line 2025; `openEdit` ~line 1741; `askDelete` ~line 1755)
- Modify: `ui/src/lib/EventPopover.svelte` (imports; derived `locked`; the `.own` row ~line 677; shortcut actions ~line 445)
- Modify: `ui/src/lib/Header.svelte` (imports; the status ~line 369)
- Test: `ui/tests/app.spec.ts` (same describe as Task 5, inside `'a change shows at once and saves behind you'`)

**Interfaces:**
- Consumes: `queueChange`, `isPending`, `pendingChangeCount` (Task 2); `refreshAfterQueuedWrite` (Task 5); harness hold/reject (Task 3).

- [ ] **Step 1: Write the failing specs**

Inside `test.describe('a change shows at once and saves behind you', …)`, add:

```ts
      test('a deleted meeting disappears at once, and a refused delete brings it back', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('delete_event_cmd'));

        await block(page, 'Board prep').click();
        await page.getByRole('button', { name: 'Delete' }).click();
        await confirmPanel(page).getByRole('button', { name: 'Delete' }).click();

        await expect(block(page, 'Board prep')).toHaveCount(0);
        await expect(page.locator('header [role="status"]').filter({ hasText: 'Saving 1 change' })).toBeVisible();

        await page.evaluate(() => window.__harness.rejectWrite('the server said no'));
        await expect(block(page, 'Board prep')).toHaveCount(1);
        await expect(page.locator('.err')).toContainText('Could not delete “Board prep”');
      });

      test('a meeting still saving offers no Edit or Delete', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('update_event'));
        await dragBy(page, 'Board prep', 60);
        await expect(block(page, 'Board prep')).toHaveClass(/pending/);

        await block(page, 'Board prep').click();
        const popover = page.getByRole('dialog', { name: 'Board prep' });
        await expect(popover).toBeVisible();
        await expect(popover.getByRole('button', { name: 'Edit', exact: true })).toHaveCount(0);
        await expect(popover.getByRole('button', { name: 'Delete', exact: true })).toHaveCount(0);
        await page.keyboard.press('Escape');
        await page.evaluate(() => window.__harness.releaseWrite());
      });
```

The popover is a `role="dialog"` named by the event's title (`aria-label={detail.title ?? '(no title)'}`), as the existing `getByRole('dialog', { name: 'Standup' })` specs use.

- [ ] **Step 2: Run them to see them fail**

Run (from `ui/`): `./node_modules/.bin/playwright test tests/app.spec.ts -g "disappears at once|offers no Edit" --project=chromium`
Expected: FAIL — the deleted block is still there while the write is held; the pending block's popover still shows Edit and Delete.

- [ ] **Step 3: Queue the delete**

Replace the whole `runDelete` function with:

```ts
  /** The confirmed delete (spec 2026-10-09): hidden at once, written behind.
   *  The scope rule still bites hardest here: `'this'` aimed at the master's
   *  DTSTART removes the series' *first* occurrence rather than the one the
   *  user clicked, so the clicked occurrence's own start is what travels. */
  function runDelete(scope: Scope) {
    const target = pendingDelete;
    if (!target) return;
    pendingDelete = null;
    error = null;
    const { detail, startMs } = target.occurrence;
    void queueChange({ kind: 'delete', id: detail.id, occurrenceStartMs: startMs, scope }, {
      write: () => deleteEvent(detail.id, scope, startMs),
      after: refreshAfterQueuedWrite,
      onfailure: (e) => { error = `Could not delete “${detail.title ?? 'this event'}”: ${String(e)}`; },
    });
  }
```

Add `queueChange, isPending` to the `./lib/pending.svelte` import.

- [ ] **Step 4: Refuse Edit and Delete on a pending occurrence**

At the top of `openEdit`, before `closeGridEvent();`, add:

```ts
    // Still being written (spec 2026-10-09, §2.6): no second change on top.
    if (isPending(occurrence.detail.id, occurrence.startMs)) return;
```

At the top of `askDelete`, before `closeGridEvent();`, add the same two lines.

- [ ] **Step 5: The popover hides them**

In `ui/src/lib/EventPopover.svelte`, add to the imports:

```ts
  import { isPending } from './pending.svelte';
```

Below `const queuedResponse = $derived(pendingResponse(detail.id, occurrenceStartMs));`, add:

```ts
  /** A change to this occurrence is still being written: no Edit or Delete
   *  until it lands (pending-changes spec §2.6). */
  const locked = $derived(isPending(detail.id, occurrenceStartMs));
```

In the `.own` row, replace `{#if detail.can_edit}<button onclick={onedit}>Edit</button>{/if}` with `{#if detail.can_edit && !locked}<button onclick={onedit}>Edit</button>{/if}` and `{#if detail.can_edit}<button onclick={ondelete}>Delete</button>{/if}` with `{#if detail.can_edit && !locked}<button onclick={ondelete}>Delete</button>{/if}`.

In `EVENT_SHORTCUT_ACTIONS`, change both `if (!detail.can_edit) return false;` lines (in `edit` and `delete`) to `if (!detail.can_edit || locked) return false;`.

- [ ] **Step 6: The header counts pending changes**

In `ui/src/lib/Header.svelte`, add to the imports:

```ts
  import { pendingChangeCount } from './pending.svelte';
```

After the `{#if pendingResponseCount() > 0} … {/if}` block, add:

```svelte
      {#if pendingChangeCount() > 0}
        <span class="response-status" role="status">Saving {pendingChangeCount()}
          {pendingChangeCount() === 1 ? 'change' : 'changes'}…</span>
      {/if}
```

- [ ] **Step 7: Run the specs to see them pass**

Run (from `ui/`): `./node_modules/.bin/playwright test tests/app.spec.ts -g "a change shows at once"`
Expected: PASS, both projects. Then `npm --prefix ui run check` → 0 errors.

- [ ] **Step 8: Prove each red**

One at a time, restore after each:
- In `runDelete`, call `deleteEvent` directly and `await` it before `queueChange` → `disappears at once` fails (the block is still there while held).
- In `EventPopover`, drop `&& !locked` from the Delete button → `offers no Edit or Delete` fails.
- In `Header`, delete the new block → `a dropped meeting stays where it lands` (Task 5) fails on `Saving 1 change`.

- [ ] **Step 9: Commit**

```bash
git add ui/src/App.svelte ui/src/lib/EventPopover.svelte ui/src/lib/Header.svelte ui/tests/app.spec.ts
git commit -m "feat(pending): a delete hides at once; a saving meeting cannot be edited or deleted again

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: Full gates, push, and the record

**Files:**
- Modify: `~/dev/omacal-private/OPERATIONS.md` (the "Where things stand" section)

- [ ] **Step 1: Full gates**

Run from the repo root:
- `cargo clippy --workspace --all-targets -- -D warnings` → exit 0 (no Rust changed; this is the CI gate).
- `npm --prefix ui run check` → 0 errors.
- From `ui/`: `./node_modules/.bin/playwright test > ../target/pw-pending.log 2>&1; echo EXIT=$? >> ../target/pw-pending.log`, then `grep -E "EXIT=|[0-9]+ (passed|failed|flaky)" ../target/pw-pending.log`. Expected: `EXIT=0`. A screenshot golden that changed must be looked at, not regenerated blind: no golden should change, because nothing is pending in a golden.

- [ ] **Step 2: Push and verify main's own CI run, by commit**

```bash
git pull --rebase origin main && git push origin main
sha=$(git rev-parse --short=7 HEAD)
rid=$(gh run list --branch main --limit 5 --json databaseId,headSha --jq ".[] | select(.headSha|startswith(\"$sha\")) | .databaseId" | head -1)
gh run watch $rid --interval 30 > /dev/null; gh run view $rid --json headSha,conclusion,jobs --jq '"\(.headSha[0:8]) \(.conclusion) " + ([.jobs[] | "\(.name)=\(.conclusion)"] | join(", "))'
```
Expected: `success` on rust, ui (chromium), ui (webkit).

- [ ] **Step 3: Record it**

In `~/dev/omacal-private/OPERATIONS.md`'s "Where things stand", note: pending changes part 1 on main (unreleased), the commits, the field check owed (Plamen drags a real meeting on Omarchy and sees it stay put; then deletes one), and that parts 2 (form Save) and 3 (Create) are next, each with its own spec. Commit and push that repo.
