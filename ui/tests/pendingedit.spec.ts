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

test('guests give the count and the card\'s list; untouched guests give neither', () => {
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
