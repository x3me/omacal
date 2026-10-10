import { test, expect } from '@playwright/test';
import type { UiEvent, WeekPayload, MonthPayload, BigYearPayload, Lane } from '../src/lib/api';
import type { EventDetail } from '../src/lib/eventdetail';
import { occurrenceDate, type Scope } from '../src/lib/eventform';
import {
  covers, isUnsaved, locks, overlayWeek, overlayMonth, overlayBigYear, overlayDetail, type EditPatch, type PendingChange,
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

type Move = Extract<PendingChange, { kind: 'move' }>;
const move = (id: number, from: number, to: number, length = H, scope: Scope = 'this'): Move =>
  ({ kind: 'move', id, occurrenceStartMs: from, scope, startMs: to, endMs: to + length });
const del = (id: number, from: number, scope: Scope = 'this'): PendingChange =>
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

const edit = (
  id: number, from: number,
  when: { allDay: boolean; startMs: number; endMs: number } | null,
  patch: EditPatch = {},
  scope: Scope = 'this',
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

  test('a meeting moved across midnight stays a line in the cell it starts in', () => {
    // As `commands::timed_column` draws it: Month's bars are all-day only.
    // Local midnights: "crosses midnight" is the reader's midnight, so a UTC
    // fixture would answer differently on every machine east or west of it.
    const mon = local(2024, 0, 29), tue = local(2024, 0, 30), wed = local(2024, 0, 31);
    const m: MonthPayload = {
      year: 2024, month: 1, lane_cap: 3,
      rows: [{
        cells: [
          { start_ms: mon, end_ms: tue, in_month: true, timed: [ev(2, mon + 8 * H, mon + 9 * H)] },
          { start_ms: tue, end_ms: wed, in_month: true, timed: [ev(1, tue + 9 * H, tue + 10 * H)] },
        ],
        bars: [], bar_events: [], bar_overflow: [],
      }],
    };
    const out = overlayMonth(m, [move(1, tue + 9 * H, mon + 23 * H, 2 * H)]);
    const row = out.rows[0];
    expect(row.cells.map((c) => c.timed.map((e) => [e.id, e.pending ?? false]))).toEqual([[[2, false], [1, true]], []]);
    expect(row.bar_events).toEqual([]);
  });

  test('a meeting carried into the next row shows at that row\'s start too', () => {
    // `timed_column`: a meeting that began before the row and runs into it is
    // drawn in the row's first cell.
    const sun = local(2024, 1, 4), mon = local(2024, 1, 5), tue = local(2024, 1, 6);
    const row = (a: number, b: number) => ({
      cells: [{ start_ms: a, end_ms: b, in_month: true, timed: [] as UiEvent[] }],
      bars: [], bar_events: [], bar_overflow: [],
    });
    const m: MonthPayload = { year: 2024, month: 2, lane_cap: 3, rows: [row(sun, mon), row(mon, tue)] };
    m.rows[0].cells[0].timed = [ev(1, sun + 9 * H, sun + 10 * H)];
    const out = overlayMonth(m, [move(1, sun + 9 * H, sun + 23 * H, 2 * H)]);
    expect(out.rows.map((r) => r.cells[0].timed.map((e) => e.id))).toEqual([[1], [1]]);
    expect(out.rows.map((r) => r.bar_events.length)).toEqual([0, 0]);
  });

  test('a title edit of a late call keeps it a line in its cell', () => {
    const mon = local(2024, 0, 29), tue = local(2024, 0, 30), wed = local(2024, 0, 31);
    const late = ev(1, mon + 23 * H, tue + H);
    const m: MonthPayload = {
      year: 2024, month: 1, lane_cap: 3,
      rows: [{
        cells: [
          { start_ms: mon, end_ms: tue, in_month: true, timed: [late] },
          { start_ms: tue, end_ms: wed, in_month: true, timed: [] },
        ],
        bars: [], bar_events: [], bar_overflow: [],
      }],
    };
    const out = overlayMonth(m, [edit(1, mon + 23 * H, { allDay: false, startMs: mon + 23 * H, endMs: tue + H }, { title: 'Late call v2' })]);
    expect(out.rows[0].cells[0].timed.map((e) => e.title)).toEqual(['Late call v2']);
    expect(out.rows[0].bar_events).toEqual([]);
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

test.describe('timed edits, in the big year', () => {
  test('an all-day pill made a timed multi-day meeting leaves the ribbon', () => {
    // The ribbon carries all-day spans only (`commands::assemble_big_year`).
    const days = Array.from({ length: 28 }, (_, i) => ({ start_ms: local(2024, 0, 29 + i), in_year: true, unsynced: false }));
    const big: BigYearPayload = {
      year: 2024, lane_cap: 3,
      rows: [{ days, pills: [lane(0, 0, 0, 0)], pill_events: [ev(7, days[0].start_ms, days[1].start_ms, { is_all_day: true })], overflow: [] }],
    };
    const out = overlayBigYear(big, [edit(7, days[0].start_ms, { allDay: false, startMs: days[0].start_ms + 9 * H, endMs: days[1].start_ms + 10 * H })]);
    expect(out.rows[0].pill_events).toEqual([]);
    expect(out.rows[0].pills).toEqual([]);
  });
});

test.describe('all-day edits, when the calendar keeps another zone', () => {
  // An all-day event is stored at midnight in its *calendar's* zone and placed
  // by its date (`commands::all_day_columns`). Three hours off the reader's
  // midnight stands in for such a calendar on any machine.
  const mon = local(2024, 0, 29), tue = local(2024, 0, 30), wed = local(2024, 0, 31), thu = local(2024, 1, 1);
  const off = 3 * H;
  const zoned = (): WeekPayload => ({
    days: [[mon, tue], [tue, wed], [wed, thu]].map(([a, b]) => ({ start_ms: a, end_ms: b, events: [], placed: [] })),
    all_day: [lane(0, 0, 0, 0)],
    all_day_events: [ev(7, mon + off, tue + off, { is_all_day: true })],
    overflow: [],
  });

  test('a renamed all-day event keeps its one day', () => {
    const out = overlayWeek(zoned(), [edit(7, mon + off, { allDay: true, startMs: mon, endMs: tue }, { title: 'Offsite v2' })]);
    const i = out.all_day_events.findIndex((e) => e.title === 'Offsite v2');
    expect(out.all_day.filter((l) => l.idx === i).map((l) => [l.start_col, l.end_col])).toEqual([[0, 0]]);
  });

  test('an all-day event moved to Wednesday covers Wednesday alone, and is locked there', () => {
    const c = edit(7, mon + off, { allDay: true, startMs: wed, endMs: thu });
    const out = overlayWeek(zoned(), [c]);
    const i = out.all_day_events.findIndex((e) => e.id === 7);
    expect(out.all_day.filter((l) => l.idx === i).map((l) => [l.start_col, l.end_col])).toEqual([[2, 2]]);
    // The card the user clicks opens for the start it is drawn at.
    expect(locks(c, 7, out.all_day_events[i].start_ms)).toBe(true);
  });

  test("'all' moves every occurrence of an all-day series by whole days, one day each", () => {
    const w: WeekPayload = {
      ...zoned(),
      all_day: [lane(0, 0, 0, 0), lane(1, 0, 1, 1)],
      all_day_events: [ev(7, mon + off, tue + off, { is_all_day: true }), ev(7, tue + off, wed + off, { is_all_day: true })],
    };
    const out = overlayWeek(w, [edit(7, mon + off, { allDay: true, startMs: tue, endMs: wed }, {}, 'all')]);
    const cols = out.all_day.map((l) => [out.all_day_events[l.idx].id, l.start_col, l.end_col]);
    expect(cols.sort()).toEqual([[7, 1, 1], [7, 2, 2]]);
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

  test('a meeting made all-day carries the dates its card reads', () => {
    const mon = local(2024, 0, 29), wed = local(2024, 0, 31);
    const out = overlayDetail(detail(), [edit(1, MON + 9 * H, { allDay: true, startMs: mon, endMs: wed })], mon);
    expect([out.is_all_day, out.start_date, out.end_date]).toEqual([true, '2024-01-29', '2024-01-30']);
    // What the popover draws for the redrawn chip, which starts at `mon`.
    expect(occurrenceDate(out.start_date, out.start_ms, mon)).toBe('2024-01-29');
  });

  test('an all-day meeting made timed carries no dates', () => {
    const mon = local(2024, 0, 29);
    const allDay = { ...detail(), is_all_day: true, start_date: '2024-01-29', end_date: '2024-01-29', start_ms: mon, end_ms: mon + DAY };
    const out = overlayDetail(allDay, [edit(1, mon, { allDay: false, startMs: mon + 9 * H, endMs: mon + 10 * H })], mon + 9 * H);
    expect([out.is_all_day, out.start_date, out.end_date]).toEqual([false, null, null]);
  });

  test('another occurrence, or an edit that waits, leaves the card as stored', () => {
    const c = edit(1, MON + 9 * H, { allDay: false, startMs: MON + 13 * H, endMs: MON + 14 * H }, { title: 'X' });
    expect(overlayDetail(detail(), [c], MON + DAY + 9 * H).title).toBe('Board prep');
    expect(overlayDetail(detail(), [edit(1, MON + 9 * H, null, { title: 'X' })], MON + 9 * H).title).toBe('Board prep');
  });
});

const created = (id: number, start: number, end: number, extra: Partial<UiEvent> = {}): PendingChange =>
  ({ kind: 'create', id, event: ev(id, start, end, { title: 'New', pending: true, ...extra }) });

test.describe('creates', () => {
  test('a new meeting joins its day, laid out beside what it overlaps', () => {
    const w = overlayWeek(week(), [created(-1, MON + 9 * H, MON + 10 * H)]);
    const mon = w.days[0];
    expect(mon.events.map((e) => [e.id, e.pending ?? false])).toEqual([[1, false], [-1, true], [2, false]]);
    expect(mon.placed.filter((p) => mon.events[p.idx].id !== 2).map((p) => p.columns)).toEqual([2, 2]);
  });

  test('a new all-day event joins the band by first fit', () => {
    const w = overlayWeek(week(), [created(-1, MON, MON + DAY, { is_all_day: true })]);
    const i = w.all_day_events.findIndex((e) => e.id === -1);
    expect(w.all_day.find((l) => l.idx === i)).toMatchObject({ lane: 1, start_col: 0, end_col: 0 });
    expect(w.all_day.filter((l) => l.idx !== i)).toEqual(week().all_day); // no other lane moved
  });

  test('two new events are both drawn, each its own', () => {
    const w = overlayWeek(week(), [created(-1, MON + 13 * H, MON + 14 * H), created(-2, MON + 13 * H, MON + 14 * H)]);
    expect(w.days[0].events.filter((e) => e.id < 0).map((e) => e.id).sort((a, b) => a - b)).toEqual([-2, -1]);
  });

  test('a new event on a hidden calendar draws nothing, and still locks its id', () => {
    const c: PendingChange = { kind: 'create', id: -1, event: null };
    expect(overlayWeek(week(), [c]).days[0].events.map((e) => e.id)).toEqual([1, 2]);
    expect(locks(c, -1, MON)).toBe(true);
  });

  test('covers no stored event, and locks only its own id', () => {
    const c = created(-1, MON + 9 * H, MON + 10 * H);
    expect(covers(c, ev(1, MON + 9 * H, MON + 10 * H))).toBe(false);
    expect(locks(c, -1, MON + 9 * H)).toBe(true);
    expect(locks(c, 1, MON + 9 * H)).toBe(false);
    expect([isUnsaved({ id: -1 }), isUnsaved({ id: 4 })]).toEqual([true, false]);
  });

  test('in Month a new meeting is a line in its start cell, a new all-day event a bar', () => {
    const mon = local(2024, 0, 29), tue = local(2024, 0, 30), wed = local(2024, 0, 31);
    const m: MonthPayload = {
      year: 2024, month: 1, lane_cap: 3,
      rows: [{
        cells: [
          { start_ms: mon, end_ms: tue, in_month: true, timed: [] },
          { start_ms: tue, end_ms: wed, in_month: true, timed: [] },
        ],
        bars: [], bar_events: [], bar_overflow: [],
      }],
    };
    const out = overlayMonth(m, [created(-1, tue + 9 * H, tue + 10 * H), created(-2, mon, wed, { is_all_day: true })]);
    const row = out.rows[0];
    expect(row.cells.map((c) => c.timed.map((e) => e.id))).toEqual([[], [-1]]);
    const i = row.bar_events.findIndex((e) => e.id === -2);
    expect(row.bars.find((l) => l.idx === i)).toMatchObject({ lane: 0, start_col: 0, end_col: 1 });
  });

  test('a new all-day event crossing a Month row is a bar in both rows', () => {
    const sun = local(2024, 1, 4), mon = local(2024, 1, 5), tue = local(2024, 1, 6);
    const row = (a: number, b: number) => ({
      cells: [{ start_ms: a, end_ms: b, in_month: true, timed: [] as UiEvent[] }],
      bars: [] as Lane[], bar_events: [] as UiEvent[], bar_overflow: [] as number[],
    });
    const m: MonthPayload = { year: 2024, month: 2, lane_cap: 3, rows: [row(sun, mon), row(mon, tue)] };
    const out = overlayMonth(m, [created(-1, sun, tue, { is_all_day: true })]);
    expect(out.rows.map((r) => r.bars.map((l) => [l.start_col, l.end_col, l.cont_left, l.cont_right])))
      .toEqual([[[0, 0, false, true]], [[0, 0, true, false]]]);
  });

  test('in Big Year a new all-day event takes a pill, a new timed one nothing', () => {
    const days = Array.from({ length: 28 }, (_, i) => ({ start_ms: local(2024, 0, 29 + i), in_year: true, unsynced: false }));
    const big: BigYearPayload = { year: 2024, lane_cap: 3, rows: [{ days, pills: [], pill_events: [], overflow: [] }] };
    const out = overlayBigYear(big, [
      created(-1, days[2].start_ms, days[3].start_ms, { is_all_day: true }),
      created(-2, days[4].start_ms + 9 * H, days[4].start_ms + 10 * H),
    ]);
    expect(out.rows[0].pill_events.map((e) => e.id)).toEqual([-1]);
    expect(out.rows[0].pills).toEqual([lane(0, 0, 2, 2)]);
  });
});
