// Pending changes, drawn over the payloads (pending-changes specs, 2026-10-09:
// part 1, moves and deletes; part 2, the form's edits).
//
// A change the user just made is shown at once and written to Google behind
// them. The payloads stay exactly what the backend sent; this is the layer
// between them and the views. Pure, so `pendingview.spec.ts` drives every rule
// directly.

import type { BigYearPayload, DayColumn, Lane, MonthPayload, UiEvent, WeekPayload } from './api';
import type { Attendee, EventDetail } from './eventdetail';
import { dateOf, type Scope } from './eventform';
import { layOutDay } from './daylayout';

/** The values an edit redraws; an absent key leaves the payload's. */
export type EditPatch = {
  title?: string; location?: string | null; color?: string;
  calendar_id?: number; attendees?: number; conference?: null;
};

/** One change still on its way to Google. A move or an edit says where the
 *  occurrence it came from lands; for a series, every covered occurrence
 *  shifts by the same amount and takes the same length. A create carries the
 *  new event itself. */
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
    }
  | {
      /** A new event (part 3 spec §2). `id` is temporary and negative, unique
       *  per create: no payload event has one, so nothing is covered and the
       *  `id:start` keys never collide. `event` is the copy to draw. */
      kind: 'create'; id: number; event: UiEvent;
    };

/** A new event Google has not given an id yet (part 3 spec §5): drawn, but
 *  with nothing to open. */
export const isUnsaved = (ev: Pick<UiEvent, 'id'>): boolean => ev.id < 0;

/** The new events the changes draw. */
const createdEvents = (changes: readonly PendingChange[]): UiEvent[] =>
  changes.flatMap((c) => (c.kind === 'create' ? [c.event] : []));

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
  if (change.kind === 'create') return false;
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
  if (change.kind === 'create') return id === change.id;
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

/** The local midnight nearest `ms`. An all-day event is stored at midnight
 *  in its *calendar's* zone and placed by its date (`commands::all_day_columns`),
 *  so its instant can sit hours off the reader's midnight; within twelve hours
 *  either side, the nearest one is the same date (`occurrenceDate`'s bound). */
function nearestMidnight(ms: number): number {
  const d = new Date(ms);
  const pm = d.getHours() >= 12;
  d.setHours(0, 0, 0, 0);
  return pm ? addDays(d.getTime(), 1) : d.getTime();
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
    if (c.kind === 'create') continue;
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
      // Whole days from the reader's own midnight, so every occurrence of an
      // all-day series is drawn over its days and no more, whatever zone its
      // calendar keeps; 'this' lands exactly on `w`, where `locks` looks.
      start = addDays(nearestMidnight(ev.start_ms), Math.round((w.startMs - c.occurrenceStartMs) / DAY_MS));
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
 * redrawn copy (flagged `pending`), and each new event, joins every day its
 * span touches when it is timed, or the band when it is all-day. A day that changed is laid out
 * again with `layOutDay`, the Rust layout's own port, so a pending card sits
 * in its proper lane beside whatever it now overlaps.
 */
export function overlayWeek(week: WeekPayload, changes: readonly PendingChange[]): WeekPayload {
  if (changes.length === 0) return week;
  const moved = new Map<string, UiEvent>();
  const take = (from: UiEvent, to: UiEvent) => { moved.set(`${from.id}:${from.start_ms}`, to); };
  const kept = week.days.map((day) => sift(day.events, changes, take));
  const band = relane(week.all_day, week.all_day_events, week.overflow, changes, take);
  const arrivals = [...moved.values(), ...createdEvents(changes)];
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
 * The month as the pending changes have left it, by the rules
 * `commands::assemble_month` draws it with. A redrawn timed copy is a line in
 * the cell it starts in, in time order, and in the first cell of each later
 * row it runs into (`timed_column`); an all-day one joins the bars by first
 * fit, so no other bar moves.
 */
export function overlayMonth(month: MonthPayload, changes: readonly PendingChange[]): MonthPayload {
  if (changes.length === 0) return month;
  const moved = new Map<string, UiEvent>();
  const take = (from: UiEvent, to: UiEvent) => { moved.set(`${from.id}:${from.start_ms}`, to); };
  const kept = month.rows.map((row) => row.cells.map((cell) => sift(cell.timed, changes, take)));
  const bars = month.rows.map((row) => relane(row.bars, row.bar_events, row.bar_overflow, changes, take));
  const arrivals = [...moved.values(), ...createdEvents(changes)];
  const lines = arrivals.filter((e) => !e.is_all_day);
  const spans = arrivals.filter((e) => e.is_all_day);
  const rows = month.rows.map((row, r) => {
    const first = row.cells[0].start_ms;
    const cells = row.cells.map((cell, c) => {
      const here = lines.filter((e) => (e.start_ms >= cell.start_ms && e.start_ms < cell.end_ms)
        || (c === 0 && e.start_ms < first && e.end_ms > first));
      const k = kept[r][c];
      if (!k.changed && here.length === 0) return cell;
      return { ...cell, timed: [...k.out, ...here].sort(byStart) };
    });
    const placed = placeLanes(bars[r], spans, row.cells, month.lane_cap);
    return { ...row, cells, bars: placed.lanes, bar_events: placed.events, bar_overflow: placed.overflow };
  });
  return { ...month, rows };
}

/** The Big Year ribbon draws only all-day events (`commands::assemble_big_year`).
 *  A pill that is deleted leaves; one that is moved or edited is placed again
 *  by first fit, and one made timed leaves. The ribbon carries no timed
 *  meetings, so one switched to all-day appears when the save lands (part 2
 *  spec §4). */
export function overlayBigYear(big: BigYearPayload, changes: readonly PendingChange[]): BigYearPayload {
  if (changes.length === 0) return big;
  const moved = new Map<string, UiEvent>();
  const take = (from: UiEvent, to: UiEvent) => { moved.set(`${from.id}:${from.start_ms}`, to); };
  const kept = big.rows.map((row) => relane(row.pills, row.pill_events, row.overflow, changes, take));
  const spans = [...moved.values(), ...createdEvents(changes)].filter((e) => e.is_all_day);
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
      // An all-day card reads its day off `start_date`, shifted by how far the
      // clicked card sits from `start_ms` (`occurrenceDate`), and a timed
      // detail carries neither: so both come from `when`, as one pair.
      ...(c.when.allDay
        ? { start_ms: c.when.startMs, end_ms: c.when.endMs,
            start_date: dateOf(c.when.startMs), end_date: dateOf(addDays(c.when.endMs, -1)) }
        : { start_date: null, end_date: null }),
    };
  }
  return out;
}
