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
