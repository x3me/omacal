import { test, expect } from '@playwright/test';
import { calendarColor, offerableCalendarId, type Calendar } from '../src/lib/calendars';
import type { EventDetail } from '../src/lib/eventdetail';
import {
  addGuest, blankValue, blankValueAt, endAfterStart, isAddress, mailableGuests, pastedValue,
  locationForVideoCall, previewSpan, removableGuest, removeGuest, ruleInWords, sameGuests,
  repeatEndProblem, sameRepeatEnd, sameVideoCall, sameWeeklyDays, shiftedEndDate,
  toEventInput, toggledGuestOptional,
  toggledWeeklyDay, valueFromDetail, videoCallProblem, weekdayCodeForDate, whenOf,
  type EventFormValue,
} from '../src/lib/eventform';

/** How long a **timed** value is, in ms.
 *
 *  `whenOf` returns a union and a timed value is the only kind with instants
 *  at all, so the narrowing is done once here rather than at each call site.
 *  It throws rather than returning a number for an all-day value: there is no
 *  honest answer, which is the point of the union. */
const spanOf = (value: EventFormValue): number => {
  const when = whenOf(value);
  if (when.kind !== 'timed') throw new Error('an all-day value has no instants');
  return when.endMs - when.startMs;
};

// Pure functions, exercised directly — same shape as `position.spec.ts` and
// `sanitize.spec.ts`. Driving these through a mounted form would need a
// fixture per case and would still only reach the ones the form happens to
// render; `ruleInWords`' fallbacks in particular are all about inputs the
// component has no way to produce.

const cal = (id: number, access_role: string): Calendar => ({
  id, account_id: 1, account_email: 'me@x.com', summary: `Cal ${id}`,
  color_hex: null, color_override: null, selected: true, sync_enabled: true,
  is_primary: false, access_role, provider: 'google',
});

const CALS = [cal(1, 'owner'), cal(2, 'writer'), cal(3, 'reader'), cal(4, 'freeBusyReader')];

test.describe('offerableCalendarId', () => {
  test('keeps an id a create can land on', () => {
    expect(offerableCalendarId(2, CALS)).toBe(2);
  });

  test('replaces an id a create cannot land on', () => {
    // The defect this exists for: filtering the options but not the value left
    // a blank select that saved the reader's id anyway.
    expect(offerableCalendarId(3, CALS)).toBe(1);
    expect(offerableCalendarId(4, CALS)).toBe(1);
  });

  test('replaces an id that is not in the list at all', () => {
    // A calendar removed by a sync between the grid loading and the form
    // opening. Same answer, and for the same reason: the value must be one of
    // the options.
    expect(offerableCalendarId(99, CALS)).toBe(1);
  });

  test('fills in a missing id', () => {
    expect(offerableCalendarId(null, CALS)).toBe(1);
  });

  test('answers null when nothing is writable', () => {
    // Not the first reader: `null` is what makes the Save guard refuse, and
    // returning a reader here would defeat the whole function.
    expect(offerableCalendarId(3, [cal(3, 'reader'), cal(4, 'freeBusyReader')])).toBeNull();
    expect(offerableCalendarId(null, [])).toBeNull();
  });
});

test.describe('calendarColor', () => {
  const painted = { ...cal(7, 'owner'), color_hex: '#8e7cc3' };

  test('answers the calendar its id names', () => {
    expect(calendarColor(7, [...CALS, painted])).toBe('#8e7cc3');
  });

  test('answers null for a calendar with no colour, and for no calendar', () => {
    // Three ways to have nothing to draw with, one answer — the callers all
    // fall back to `--accent`, and none of them needs to tell these apart.
    expect(calendarColor(1, CALS)).toBeNull();
    expect(calendarColor(99, CALS)).toBeNull();
    expect(calendarColor(null, CALS)).toBeNull();
  });
});

test.describe('custom weekly patterns', () => {
  const wednesday = (): EventFormValue => ({
    ...blankValueAt(new Date(2026, 7, 5, 9).getTime(), 1),
    date: '2026-08-05',
    endDate: '2026-08-05',
    weeklyDays: ['WE'],
  });

  test('a create sends the selected days with weekly, in stable calendar order', () => {
    const initial = wednesday();
    const value: EventFormValue = {
      ...initial, repeat: 'weekly', weeklyDays: ['FR', 'MO', 'WE'],
    };
    const sent = toEventInput(value, initial);
    expect(sent.repeat).toBe('weekly');
    expect(sent.weeklyDays).toEqual(['MO', 'WE', 'FR']);
  });

  test('an unchanged edit sends no recurrence, while a day change sends the whole pattern', () => {
    const initial = { ...wednesday(), isEdit: true, isRecurring: true, repeat: 'weekly' };
    expect(toEventInput(initial, initial).repeat).toBeUndefined();
    expect(toEventInput(initial, initial).weeklyDays).toBeUndefined();

    const changed: EventFormValue = { ...initial, weeklyDays: ['MO', 'WE', 'FR'] };
    const sent = toEventInput(changed, initial);
    expect(sent.repeat).toBe('weekly');
    expect(sent.weeklyDays).toEqual(['MO', 'WE', 'FR']);
  });

  test('the last button stays selected and removing the start day advances DTSTART', () => {
    const only = wednesday();
    expect(toggledWeeklyDay(only, 'WE')).toBe(only);

    const two: EventFormValue = { ...only, weeklyDays: ['WE', 'FR'] };
    const friday = toggledWeeklyDay(two, 'WE');
    expect(friday.weeklyDays).toEqual(['FR']);
    expect(friday.date).toBe('2026-08-07');
    expect(friday.endDate).toBe('2026-08-07');
    expect(weekdayCodeForDate(friday.date)).toBe('FR');
    expect(sameWeeklyDays(['FR', 'MO'], ['MO', 'FR'])).toBe(true);
  });
});

test.describe('repeat endings', () => {
  const recurring = (): EventFormValue => ({
    ...blankValueAt(new Date(2026, 7, 5, 9).getTime(), 1),
    date: '2026-08-05', endDate: '2026-08-05', repeat: 'weekly', weeklyDays: ['WE'],
  });

  test('creates COUNT and UNTIL inputs and treats ending-only edits as recurrence changes', () => {
    const initial = recurring();
    const counted: EventFormValue = { ...initial, repeatEnd: { kind: 'after', count: 12 } };
    expect(toEventInput(counted, initial)).toMatchObject({
      repeat: 'weekly', weeklyDays: ['WE'], repeatEnd: { kind: 'after', count: 12 },
    });

    const editing: EventFormValue = {
      ...initial, isEdit: true, isRecurring: true,
      repeatEnd: { kind: 'on', date: '2026-10-31' },
    };
    expect(toEventInput(editing, editing).repeat).toBeUndefined();
    const unbounded: EventFormValue = { ...editing, repeatEnd: { kind: 'never' } };
    expect(toEventInput(unbounded, editing)).toMatchObject({ repeat: 'weekly' });
    expect(toEventInput(unbounded, editing).repeatEnd).toBeUndefined();

    // The hidden old boundary must not travel beside `repeat: never` when a
    // bounded series is turned off. That pair is contradictory and the
    // backend deliberately refuses it.
    const stopped: EventFormValue = { ...editing, repeat: 'never' };
    expect(toEventInput(stopped, editing).repeat).toBe('never');
    expect(toEventInput(stopped, editing).repeatEnd).toBeUndefined();
  });

  test('validates count/date endings and compares tagged values exactly', () => {
    expect(repeatEndProblem({ ...recurring(), repeatEnd: { kind: 'after', count: 0 } }))
      .toContain('at least 1');
    expect(repeatEndProblem({
      ...recurring(), repeatEnd: { kind: 'on', date: '2026-08-04' },
    })).toContain('cannot be before');
    expect(repeatEndProblem({
      ...recurring(), repeatEnd: { kind: 'on', date: '2026-08-05' },
    })).toBeNull();
    expect(sameRepeatEnd({ kind: 'after', count: 4 }, { kind: 'after', count: 4 })).toBe(true);
    expect(sameRepeatEnd({ kind: 'after', count: 4 }, { kind: 'after', count: 5 })).toBe(false);
  });
});

test.describe('ruleInWords', () => {
  test('describes a rule it fully models', () => {
    expect(ruleInWords('RRULE:FREQ=WEEKLY;INTERVAL=2')).toBe('Every 2 weeks');
    expect(ruleInWords('RRULE:FREQ=MONTHLY;BYDAY=-1FR')).toBe('Monthly on the last Friday');
    expect(ruleInWords('RRULE:FREQ=DAILY;COUNT=10')).toBe('Daily, 10 times');
    expect(ruleInWords('RRULE:FREQ=WEEKLY;UNTIL=20261231')).toBe('Weekly, until Dec 31, 2026');
  });

  test('shows a rule carrying a part it does not model verbatim', () => {
    // Never a partial description: "Monthly" for a rule that also carries
    // BYMONTHDAY and BYSETPOS tells the user the rule is simpler than it is,
    // immediately before offering to replace it.
    const rule = 'RRULE:FREQ=MONTHLY;BYMONTHDAY=15;BYSETPOS=1';
    expect(ruleInWords(rule)).toBe(rule);
  });

  test('shows an over-long rule cut, and does not describe it', () => {
    // The cap used to be applied *before* parsing, so a rule whose only
    // unmodelled part sat past the cut parsed cleanly and got a full English
    // description with that part silently gone. Built here exactly that way:
    // legal, modelled parts filling more than the cap, then `BYSETPOS`.
    const byday = Array.from({ length: 45 }, (_, i) => `${(i % 4) + 1}MO`).join(',');
    const rule = `RRULE:FREQ=MONTHLY;BYDAY=${byday};BYSETPOS=2`;
    // The premise, asserted rather than assumed: `BYSETPOS` must fall *past*
    // the 200-character cap, or this proves nothing about truncation.
    expect(rule.indexOf('BYSETPOS')).toBeGreaterThan(200);

    const words = ruleInWords(rule);
    expect(words).not.toContain('Monthly on');
    expect(words.startsWith('RRULE:FREQ=MONTHLY')).toBe(true);
    expect(words.endsWith('…')).toBe(true);
  });

  test('describes a rule-plus-EXDATE blob — the commonest real custom', () => {
    // `recurrence` is newline-joined (`convert.rs`), and what actually makes a
    // real event `custom` is usually not an exotic RRULE at all: it is an
    // ordinary rule plus an EXDATE naming occurrences somebody deleted. The
    // deletions stay in the sentence — as a count, not as the raw timestamps
    // the popover was caught printing where its cadence line goes.
    expect(ruleInWords('RRULE:FREQ=WEEKLY\nEXDATE;TZID=Europe/Sofia:20260817T090000'))
      .toBe('Weekly, except 1 date');

    // The blob seen live (2026-08-16): EXDATE line first, six dates in one
    // comma list, the rule last. Line order carries no meaning.
    const live = 'EXDATE;TZID=Asia/Kolkata:20250917T133000,20260218T133000,20260318T133000,'
      + '20260520T133000,20260617T133000,20260715T133000\nRRULE:FREQ=MONTHLY;BYDAY=3WE';
    expect(ruleInWords(live)).toBe('Monthly on the third Wednesday, except 6 dates');

    // RDATEs are the same trade in the other direction.
    expect(ruleInWords('RRULE:FREQ=WEEKLY\nRDATE;TZID=Europe/Sofia:20260901T090000'))
      .toBe('Weekly, plus 1 added date');

    // The cap guards the verbatim fallback, not the parse: an EXDATE list
    // from a long-lived series sails past 200 characters while its
    // description stays one line. Asserted over-cap, or this proves nothing.
    const dates = Array.from({ length: 15 }, (_, i) =>
      `2026${String((i % 12) + 1).padStart(2, '0')}01T090000`).join(',');
    const long = `RRULE:FREQ=WEEKLY\nEXDATE;TZID=Europe/Sofia:${dates}`;
    expect(long.length).toBeGreaterThan(200);
    expect(ruleInWords(long)).toBe('Weekly, except 15 dates');
  });

  test('shows a blob carrying any line it does not fully model verbatim', () => {
    // Two rules in one recurrence is legal and beyond the vocabulary.
    const two = 'RRULE:FREQ=WEEKLY\nRRULE:FREQ=DAILY';
    expect(ruleInWords(two)).toBe(two);

    // Dates with no rule to hang them on.
    const bare = 'EXDATE;TZID=Europe/Sofia:20260817T090000\nEXDATE;TZID=Europe/Sofia:20260824T090000';
    expect(ruleInWords(bare)).toBe(bare);

    // A PERIOD-valued RDATE names spans, not dates — a count would lie.
    const period = 'RRULE:FREQ=WEEKLY\nRDATE;VALUE=PERIOD:20260817T090000Z/20260817T100000Z';
    expect(ruleInWords(period)).toBe(period);

    // An unmodelled RRULE poisons the whole blob — "Monthly, except 1 date"
    // for a BYSETPOS rule is the partial description this function refuses.
    const exotic = 'RRULE:FREQ=MONTHLY;BYDAY=MO;BYSETPOS=2\nEXDATE;TZID=Europe/Sofia:20260817T090000';
    expect(ruleInWords(exotic)).toBe(exotic);

    // A `;` before the line break leaves `COUNT=3` looking like an ordinary
    // part of the first line; glued back together, the whole thing would be
    // described as "Weekly, 3 times".
    const spanning = 'RRULE:FREQ=WEEKLY;\nCOUNT=3';
    expect(ruleInWords(spanning)).toBe(spanning);
  });

  test('shows a rule whose UNTIL is not a real date verbatim', () => {
    // `Date.UTC` normalises out of range rather than rejecting: month 13
    // became "Feb 14, 2027", and 31 February would become 3 March. A wrong
    // date here is worse than no description.
    expect(ruleInWords('RRULE:FREQ=WEEKLY;UNTIL=20261345')).toBe('RRULE:FREQ=WEEKLY;UNTIL=20261345');
    expect(ruleInWords('RRULE:FREQ=WEEKLY;UNTIL=20260231')).toBe('RRULE:FREQ=WEEKLY;UNTIL=20260231');
  });

  test('no rule at all is not a rule to describe', () => {
    expect(ruleInWords(null)).toBe('');
    expect(ruleInWords('   ')).toBe('');
  });
});

test.describe('previewSpan', () => {
  const at = (y: number, mo: number, d: number, h = 0, mi = 0) =>
    new Date(y, mo - 1, d, h, mi).getTime();

  test('a timed value describes its span, across dates included', () => {
    const v = blankValue(at(2026, 8, 5, 10, 0), 1);
    const span = previewSpan(v)!;
    // The premise from the value itself, not retyped instants.
    expect(span.startMs).toBe(at(2026, 8, 5, Number(v.start.slice(0, 2)), Number(v.start.slice(3))));
    expect(span.endMs).toBeGreaterThan(span.startMs);

    // An end date after the start date draws through midnight, exactly as
    // whenOf sends it.
    const crossing = { ...v, date: '2026-08-05', start: '23:00', endDate: '2026-08-06', end: '01:00' };
    expect(previewSpan(crossing)).toEqual({
      startMs: at(2026, 8, 5, 23, 0),
      endMs: at(2026, 8, 6, 1, 0),
    });
  });

  test('what cannot be drawn is null, never a guess', () => {
    const v = blankValue(at(2026, 8, 5, 10, 0), 1);
    expect(previewSpan({ ...v, isAllDay: true })).toBeNull();
    expect(previewSpan({ ...v, start: '' })).toBeNull();
    expect(previewSpan({ ...v, start: '11:00', endDate: v.date, end: '10:00' })).toBeNull();
    expect(previewSpan({ ...v, end: v.start, endDate: v.date })).toBeNull();
  });
});

test.describe('pastedValue', () => {
  const at = (y: number, mo: number, d: number, h = 0, mi = 0) =>
    new Date(y, mo - 1, d, h, mi).getTime();

  /** A copied meeting with everything a paste must decide about: identity
   *  fields worth carrying, series and reminder state worth dropping. */
  const copied = (): EventFormValue => ({
    ...blankValueAt(at(2026, 8, 5, 11, 0), 7, at(2026, 8, 5, 12, 30)),
    title: 'Ops review',
    location: 'HQ',
    description: 'bring the numbers',
    guests: [{ email: 'a@excitel.com', optional: false }, { email: 'b@excitel.com', optional: true }],
    repeat: 'weekly',
    recurrence: 'RRULE:FREQ=WEEKLY',
    isRecurring: true,
    isEdit: true,
    popupReminders: [30],
    emailReminders: [{ method: 'email', minutes: 60 }],
  });
  const blank = () => blankValueAt(at(2026, 9, 10, 9, 30), 3);

  test('what the user copied for crosses over; the clock lands on the target day', () => {
    const r = pastedValue(copied(), blank());
    expect(r.title).toBe('Ops review');
    expect(r.location).toBe('HQ');
    expect(r.description).toBe('bring the numbers');
    expect(r.guests).toEqual(copied().guests);
    // The target's day, the source's clock and duration.
    expect(r.date).toBe('2026-09-10');
    expect(r.start).toBe('11:00');
    expect(r.end).toBe('12:30');
    expect(r.endDate).toBe('2026-09-10');
    // Assembled from two sources — the documented "read off no instant" case.
    expect(r.sourceStartMs).toBeNull();
    expect(r.sourceEndMs).toBeNull();
  });

  test('the copier is the new organizer, never a pasted guest', () => {
    // Copying a meeting you organize used to invite you to the copy: your
    // own attendee row rode along, Google filed you as unanswered on your
    // own event, and the invite pass rang for it (2026-08-20, live). The
    // self row is identified by the source's `selfEmail`; everyone else —
    // the source's organizer included, when that is somebody else — stays.
    const withSelf = {
      ...copied(),
      selfEmail: 'b@excitel.com',
      organizerEmail: 'a@excitel.com',
    };
    expect(pastedValue(withSelf, blank()).guests).toEqual([
      { email: 'a@excitel.com', optional: false },
    ]);
    // No self to recognise — nothing is dropped.
    expect(pastedValue(copied(), blank()).guests).toEqual(copied().guests);
  });

  test('a paste is one new event on the target calendar, never a second series', () => {
    const r = pastedValue(copied(), blank());
    expect(r.repeat).toBe('never');
    expect(r.recurrence).toBeNull();
    expect(r.isRecurring).toBe(false);
    expect(r.isEdit).toBe(false);
    expect(r.calendarId).toBe(3);
    // Empty rows mean the target calendar's own defaults, same as any create.
    expect(r.popupReminders).toEqual([]);
    expect(r.emailReminders).toEqual([]);
  });

  test('a span keeps its length in days — timed across midnight, and all-day', () => {
    const overnight = {
      ...copied(), date: '2026-08-05', start: '23:00', endDate: '2026-08-07', end: '01:00',
    };
    const t = pastedValue(overnight, blank());
    expect(t.date).toBe('2026-09-10');
    expect(t.endDate).toBe('2026-09-12');
    expect(t.start).toBe('23:00');
    expect(t.end).toBe('01:00');

    const allDay = { ...copied(), isAllDay: true, date: '2026-08-05', endDate: '2026-08-06' };
    const a = pastedValue(allDay, blank());
    expect(a.isAllDay).toBe(true);
    expect(a.date).toBe('2026-09-10');
    expect(a.endDate).toBe('2026-09-11');
  });
});

test.describe('shiftedEndDate', () => {
  test('keeps the span when the start moves', () => {
    expect(shiftedEndDate('2026-08-10', '2026-08-17', '2026-08-12')).toBe('2026-08-19');
    expect(shiftedEndDate('2026-08-10', '2026-08-11', '2026-08-10')).toBe('2026-08-11');
  });

  test('crosses a month end', () => {
    expect(shiftedEndDate('2026-08-30', '2026-09-29', '2026-09-01')).toBe('2026-10-01');
  });

  test('counts days on the calendar, not in milliseconds', () => {
    // 29 March 2026 is the European spring-forward. A span measured in local
    // milliseconds lands at 23:00 the previous day across it, which reads back
    // as a date one earlier. `Date.UTC` has no transitions, so this holds
    // whatever zone the machine running it is in.
    expect(shiftedEndDate('2026-03-28', '2026-03-29', '2026-03-30')).toBe('2026-03-31');
  });

  test('leaves a backwards or unparseable range alone', () => {
    // Repairing a range the user has not been told about, as a side effect of
    // an edit to a different field, is the silent correction the Save guard
    // exists to refuse.
    expect(shiftedEndDate('2026-08-10', '2026-08-17', '2026-08-09')).toBe('2026-08-09');
    expect(shiftedEndDate('2026-08-10', '', '2026-08-12')).toBe('2026-08-12');
    expect(shiftedEndDate('', '2026-08-17', '2026-08-12')).toBe('2026-08-12');
  });
});

test.describe('blankValue', () => {
  // Built in the *host's* own zone rather than through `Date.UTC`, and every
  // assertion below is a property that holds in any zone: these run in the
  // Node process, not in the page, so Playwright's `timezoneId: 'UTC'` does
  // not reach them. `dateOf`/`timeOf` read local time, so a local instant
  // round-trips exactly wherever this is run.
  const at = (y: number, m: number, d: number, h = 0, min = 0) =>
    new Date(y, m - 1, d, h, min, 0, 0).getTime();

  const MINUTES = 60_000;

  test('a late-evening create is savable', async () => {
    // The defect this exists for. `nextHalfHour` lands on the last half hour of
    // the day, whose end is midnight *tomorrow*; the version that assigned both
    // dates the start's own day made that an end twenty-three and a half hours
    // before the start, so the form opened already refusing to save and no
    // field on it looked wrong. Reachable for an hour every evening now
    // that the default span is one.
    const v = blankValue(at(2026, 8, 5, 23, 15), 1);
    expect(v.date).toBe('2026-08-05');
    expect(v.endDate).toBe('2026-08-06');
    expect(endAfterStart(v)).toBe(true);
    expect(spanOf(v)).toBe(60 * MINUTES);
  });

  test('a chosen day keeps the time and takes the end date with it', async () => {
    // Pressing `n` on a day that is not today: the time is still the next half
    // hour, and the span survives the move — including across the midnight the
    // case above lands on.
    const v = blankValue(at(2026, 8, 5, 23, 15), 1, at(2026, 8, 12));
    expect(v.date).toBe('2026-08-12');
    expect(v.endDate).toBe('2026-08-13');
    expect(endAfterStart(v)).toBe(true);
    expect(spanOf(v)).toBe(60 * MINUTES);
  });

  test('an ordinary daytime create keeps both dates on the same day', async () => {
    // The other side of the fix: rolling the end date forward is for the events
    // that actually cross midnight, not for all of them. The clock times are
    // deliberately not pinned here — `nextHalfHour` rounds the *instant*, so
    // "09:30" is only the answer in a zone whose offset is a whole half hour.
    // What the form actually shows at 09:12 is pinned under a frozen UTC clock
    // by `EventForm`'s own "a new event opens at the next half hour".
    const v = blankValue(at(2026, 8, 5, 9, 12), 1);
    expect(v.date).toBe('2026-08-05');
    expect(v.endDate).toBe('2026-08-05');
    expect(spanOf(v)).toBe(60 * MINUTES);
  });

  test('a caller can choose the default duration, while an explicit range still wins', () => {
    const start = at(2026, 8, 5, 10, 0);
    expect(spanOf(blankValueAt(start, 1, undefined, 45))).toBe(45 * MINUTES);
    expect(spanOf(blankValue(start, 1, undefined, 75))).toBe(75 * MINUTES);

    const sweptEnd = start + 20 * MINUTES;
    expect(spanOf(blankValueAt(start, 1, sweptEnd, 45))).toBe(20 * MINUTES);
  });
});

test.describe('endAfterStart on an all-day value', () => {
  // Zone-independent, so no page: the all-day arm compares *dates*, and a date
  // has no zone to be read in. That is the property, not an implementation
  // detail — the timed arm still builds instants and still needs a page.

  /** An all-day form value with the two dates under test. The rest comes from a
   *  real blank value, so nothing here has to be kept in step with
   *  `EventFormValue` by hand. */
  const allDayValue = (date: string, endDate: string): EventFormValue => ({
    ...blankValueAt(Date.UTC(2026, 7, 10), 1), isAllDay: true, date, endDate,
  });

  test('a single-day event is savable, and a backwards one is not', () => {
    // `endDate` is the *inclusive* last day, so naming the same day twice is a
    // one-day event and must pass. Only a last day genuinely before the first
    // fails.
    expect(endAfterStart(allDayValue('2026-08-10', '2026-08-10'))).toBe(true);
    expect(endAfterStart(allDayValue('2026-08-10', '2026-08-12'))).toBe(true);
    expect(endAfterStart(allDayValue('2026-08-10', '2026-08-09'))).toBe(false);
  });

  test('a half-typed date never enables Save', () => {
    // The reason the all-day arm compares through `utcOf` rather than as
    // strings. An unparseable date reaches the comparison as `addDays`'
    // `'NaN-NaN-NaN'`, which sorts *after* every real date — so a string
    // comparison answers `true` here and lets Save fire on a form the user is
    // still typing into, sending `'NaN-NaN-NaN'` to Google as a date.
    expect(endAfterStart(allDayValue('2026-08-10', ''))).toBe(false);
    expect(endAfterStart(allDayValue('', '2026-08-10'))).toBe(false);
    expect(endAfterStart(allDayValue('2026-08-10', '2026-08'))).toBe(false);
  });
});

test.describe('blankValueAt', () => {
  const at = (y: number, m: number, d: number, h = 0, min = 0) =>
    new Date(y, m - 1, d, h, min, 0, 0).getTime();

  test('takes the time the grid gave it, not the next half hour', async () => {
    // A click on empty grid space already knows which instant it landed on.
    // Substituting the clock's own "next half hour" would move the event away
    // from where the user pointed.
    const v = blankValueAt(at(2026, 8, 5, 10, 0), 1);
    expect(v.date).toBe('2026-08-05');
    expect(v.start).toBe('10:00');
    expect(v.end).toBe('11:00');
    expect(v.endDate).toBe('2026-08-05');
    expect(v.isEdit).toBe(false);
    expect(v.calendarId).toBe(1);
  });

  test('an event starting in the last half hour of the day still ends tomorrow', async () => {
    const v = blankValueAt(at(2026, 8, 5, 23, 30), 1);
    expect(v.endDate).toBe('2026-08-06');
    expect(endAfterStart(v)).toBe(true);
  });
});

// --- The form's civil <-> instant boundary -------------------------------
//
// `dateOf`/`timeOf`/`toMs` convert between an instant and a civil (date, time)
// pair, and that conversion is neither injective nor precision-preserving:
//
//   - it drops everything below a minute
//   - a repeated wall-clock hour maps two instants onto one pair, and `toMs`
//     resolves that pair back to the earlier of them
//   - a skipped wall-clock hour is a pair naming no instant at all, and
//     `new Date(y, m, d, h, min)` silently normalises it forward
//
// **There were four characterisation specs here**, each asserting a wrong value
// on purpose and naming the right one in its comments, so the gate stayed green
// while the defect stayed visible. **All four are gone**, and so is the fifth
// this plan inverted in Rust. Task 4 took the all-day zone crossing, which lives
// below as "an all-day event's dates cross the boundary as dates"; Task 5 took
// both timed ones, which are the describe immediately below and "the anchor's
// precision" at the end of this file; Task 6 took the skipped midnight, which is
// "a create on a day whose midnight is skipped". Their comments are the
// assertions now.
//
// **A fifth characterisation spec sat here and is now gone too.** It was not
// one of the four: it is the same skipped hour reached from the other side,
// found while closing the create — a time the **user typed** into an hour that
// does not exist. The two look identical on screen and are different problems.
//
//   - A create is the *app* choosing a time, so the app may choose another. The
//     repair is arithmetic — re-anchor the moved pair on the instant it names —
//     and it needs no resolver and no message.
//   - A typed time is not the app's to move. Normalising it forward silently
//     would be a move nobody asked for, and this branch has already ruled on
//     that shape once: the per-side check refuses an incoherent pair honestly
//     rather than dragging an untouched field along with it. Closing it meant
//     the form *saying* the time does not exist — a form-level affordance, not
//     a boundary conversion — and that is what `timeProblem` and the specs
//     under "a time typed into an hour that does not exist" now are (§7.3).
//
// So `new Date(y, m, d, h, min)` is still not a resolver — it picks a pass
// silently and normalises a skipped hour forward without saying so. Nothing
// downstream trusts it to be one: the skipped hour is caught before a save and
// reported, and the repeated hour is left to `instantOf`, which protects a
// second-pass instant by never re-deriving one nobody touched.
//
// Probes behind the numbers: `scratchpad/t10rev/dst.mjs` and `anchor.mjs`
// (the reviewer's), re-run in six zones — Europe/Sofia, America/New_York,
// America/Santiago, Africa/Cairo, Australia/Lord_Howe, Pacific/Chatham.

/** A harness URL that mounts no component at all — `mount.svelte.ts` puts the
 *  pure module on `window` before it branches, so this is the cheapest page
 *  that can answer these questions in a zone Playwright controls. */
const PURE = '/tests/harness/index.html?c=eventform&f=none';

/** A one-off **timed** event as `event_detail_impl` reports one. The all-day
 *  fields are `null` on both sides together, which is the only shape the Rust
 *  side produces for `is_all_day: false`. */
const timedDetail = (startMs: number, endMs: number): EventDetail => ({
  id: 1, calendar_id: 1, title: 'Standup', description: null, location: null,
  conference_uri: null, start_ms: startMs, end_ms: endMs,
  start_date: null, end_date: null, is_all_day: false,
  is_recurring: false, recurrence: null, repeat: 'never', weekly_days: [],
  repeat_end: { kind: 'never' }, color: null,
  organizer_email: null, self_response: null, can_respond: true, can_edit: true, is_organizer: true, guests_can_modify: false,
  attendees: [],
  reminders: { use_default: true, overrides: [] }, calendar_default_reminders: [],
});

test.describe('a timed value is sent as the instants it was read off', () => {
  // 25 Oct 2026: Sofia's clocks go back 04:00 -> 03:00, so 03:00-03:59 runs
  // twice, once at UTC+3 and again at UTC+2.
  //
  // **The zone that matters here is the browser's**, not a calendar's — a
  // fourth situation, and not any of the three earlier fixtures on this branch.
  // `toMs` resolves a civil pair in the zone the browser is in, so a repeated
  // wall-clock hour *there* is what does the damage; the calendar's zone never
  // enters a timed event's boundary at all. A browser zone with no transition
  // on this date (UTC and Asia/Calcutta among them) makes every spec below pass
  // without discriminating, so each one asserts the repeat outright rather than
  // trusting the `test.use` above it.
  test.use({ timezoneId: 'Europe/Sofia' });

  /** The two instants Sofia reads as 03:00 on 25 Oct 2026, named by their own
   *  UTC offsets rather than by arithmetic on a local `Date`: an instant
   *  computed *from* the browser's reading could not disagree with that
   *  reading, and the disagreement is the entire fixture. */
  const FIRST_PASS = Date.parse('2026-10-25T03:00:00+03:00');
  const SECOND_PASS = Date.parse('2026-10-25T03:00:00+02:00');

  /** Midnight opening 25 Oct in Sofia — still UTC+3, the transition being at
   *  04:00 — as `App.createDayMs` hands it to `blankValue`. */
  const THAT_DAY = Date.parse('2026-10-25T00:00:00+03:00');

  test('a new event asked for inside the repeated hour is an hour long, and savable', async ({ page }) => {
    await page.goto(PURE);
    const v = await page.evaluate(([firstPass, thatDay]) => {
      const ef = (window as any).__eventform;
      // **Three arguments, because two is a call the application never makes.**
      // `App.newEventOnAnchor` (`n`) and `App.newEventOnDay` both pass the day
      // the user is looking at; only `newEventAt` skips `blankValue` entirely,
      // for `blankValueAt`. The `dayStartMs === undefined` early return is
      // reachable from tests and nowhere else, so a spec that took it would
      // leave the day-move branch — the one that rebuilds the value — unwitnessed.
      // Here the day *is* the clock's own day, which is what pressing `n` at
      // 03:00 on the 25th while looking at the 25th does.
      const value = ef.blankValue(firstPass, 1, thatDay);
      const when = ef.whenOf(value);
      return {
        start: value.start, end: value.end, saveable: ef.endAfterStart(value),
        span: when.endMs - when.startMs, startMs: when.startMs, endMs: when.endMs,
        date: value.date, endDate: value.endDate,
        // The premise, in the terms the defect is written in: re-parsing the
        // end's own wall clock lands on the *earlier* of the two passes.
        reparsedEnd: ef.toMs(value.endDate, value.end),
      };
    }, [FIRST_PASS, THAT_DAY] as [number, number]);

    // Fixture premise: the day move really did happen and really did land back
    // on the same day. A `dayStartMs` on any *other* day would move the dates
    // off the instants, and re-derivation there is correct rather than a bug —
    // so this spec would stop saying anything about the branch it exists for.
    expect(v.date).toBe('2026-10-25');
    expect(v.endDate).toBe('2026-10-25');

    // Fixture premise. Without an hour between two instants that read the same,
    // this zone has no repeated hour and nothing below discriminates.
    expect(SECOND_PASS - FIRST_PASS).toBe(3_600_000);

    expect(v.start).toBe('03:30');
    // Not a typo, and not the wrong value: the end instant is an hour later,
    // and by then the clocks have gone back, so its wall clock reads **the
    // same 03:30** on the same date. What the form shows is what a Sofia
    // clock says — and re-parsing that pair lands both ends on the *first*
    // pass, a span of zero.
    expect(v.end).toBe('03:30');
    expect(v.reparsedEnd).toBe(v.startMs);

    // The claim. This span used to re-derive to **zero** and `saveable`
    // used to be `false`: a form that opened already refusing to save, with no
    // field on it visibly wrong. `endAfterStart` asks `whenOf`, which is why
    // the pass-through has to live there rather than in `toEventInput` — a
    // guard that re-derived while the wire passed through would go on refusing
    // this exact form.
    expect(v.span).toBe(60 * 60_000);
    expect(v.saveable).toBe(true);
    // And the instants are the ones the clock named, not a re-parse of what
    // they happen to look like on a wall clock. The end is half past the
    // second pass.
    expect(v.startMs).toBe(FIRST_PASS + 30 * 60_000);
    expect(v.endMs).toBe(SECOND_PASS + 30 * 60_000);
  });

  test('editing only the title of an event in a repeated hour sends no times', async ({ page }) => {
    await page.goto(PURE);
    // A 03:00-03:30 standup in the **second** pass. Both ends sit inside the
    // repeated hour, so each arm of `whenOf` has to hold on its own.
    const startMs = SECOND_PASS;
    const endMs = SECOND_PASS + 30 * 60_000;

    // **A series, and its row's instants are not the clicked occurrence's.**
    // A one-off detail has `start_ms === startMs`, and against one of those a
    // `valueFromDetail` that took `detail.start_ms` as the source instead of the
    // occurrence's own is *invisible* — the whole suite stays green while the
    // pass-through is disabled for every occurrence of every series, which is
    // the commonest shape this defect has. It is also the
    // `detail.start_ms`-vs-`occurrenceStartMs` confusion `updateEvent`'s doc
    // comment and §4 of the design both name under "must not regress".
    //
    // The master's DTSTART is the same wall clock a week earlier, when Sofia was
    // still on UTC+3 throughout, so the gap is 169 hours rather than 168 — and
    // an occurrence landing in the *second* pass is what a series carries after
    // somebody moves that one occurrence, which keeps its master row and its
    // master's DTSTART.
    const master = timedDetail(Date.parse('2026-10-18T03:00:00+03:00'), Date.parse('2026-10-18T03:30:00+03:00'));
    master.is_recurring = true;

    const r = await page.evaluate(([d, first, s, e]) => {
      const ef = (window as any).__eventform;
      // Exactly what `App.openEdit` then `EventForm.save` do, with the user
      // touching nothing but the title — and `openEdit` passes the **clicked
      // block's** times, never `detail.start_ms`.
      const initial = ef.valueFromDetail(d, s, e);
      const value = { ...initial, title: 'Renamed' };
      const sent = ef.toEventInput(value, initial);
      return {
        when: sent.when, date: value.date, start: value.start, end: value.end,
        // The premise again: the browser reads the second pass as the same wall
        // clock as the first, and `toMs` resolves that reading to the first.
        readsAsFirstPass:
          ef.dateOf(first) === value.date && ef.timeOf(first) === value.start,
        reparsedStart: ef.toMs(value.date, value.start),
        reparsedEnd: ef.toMs(value.endDate, value.end),
      };
    }, [master, FIRST_PASS, startMs, endMs] as [EventDetail, number, number, number]);

    expect(r.date).toBe('2026-10-25');
    expect(r.start).toBe('03:00');
    expect(r.end).toBe('03:30');

    // Fixture premise, asserted rather than described: an hour apart, read the
    // same, and re-parsed onto the earlier one. If any of these stops holding
    // the fixture no longer straddles a transition and the assertions below
    // pass vacuously.
    expect(SECOND_PASS - FIRST_PASS).toBe(3_600_000);
    expect(r.readsAsFirstPass).toBe(true);
    expect(r.reparsedStart).toBe(startMs - 3_600_000);
    expect(r.reparsedEnd).toBe(endMs - 3_600_000);
    // And the premise that makes the row-vs-occurrence distinction bite: the
    // master's own instants are 169 hours away and could not stand in for the
    // clicked block's under any rounding.
    expect((startMs - master.start_ms) / 3_600_000).toBe(169);
    expect((endMs - master.end_ms) / 3_600_000).toBe(169);

    // The claim, as drift first so a regression reads as the movement it is.
    expect(r.when.kind).toBe('timed');
    expect(r.when.startMs - startMs).toBe(0);
    expect(r.when.endMs - endMs).toBe(0);
    // Exactly, not close. `write::shifted_like` short-circuits only on an exact
    // match, so a difference of any size is applied as a real move — a
    // start/end PATCH dragging the meeting an hour earlier, with
    // `sendUpdates=all` behind it, for somebody who only renamed it.
    expect(r.when.startMs).toBe(startMs);
    expect(r.when.endMs).toBe(endMs);
  });

  test('a time the user did edit is re-derived, not passed through', async ({ page }) => {
    await page.goto(PURE);
    const startMs = SECOND_PASS;
    const endMs = SECOND_PASS + 30 * 60_000;
    // 02:00 is an hour before the transition and names exactly one instant.
    const movedTo = Date.parse('2026-10-25T02:00:00+03:00');

    const r = await page.evaluate((d) => {
      const ef = (window as any).__eventform;
      const initial = ef.valueFromDetail(d, d.start_ms, d.end_ms);
      // The user moves the start earlier and leaves the end alone.
      const value = { ...initial, start: '02:00' };
      return {
        when: ef.toEventInput(value, initial).when,
        // This spec's own premise, which it used to borrow from its siblings:
        // run verbatim in a zone with no transition on this date — Istanbul is
        // UTC+3 all year — every assertion below passes while proving nothing,
        // because there the untouched end re-derives to itself.
        reparsedEnd: ef.toMs(value.endDate, value.end),
      };
    }, timedDetail(startMs, endMs));

    expect(r.when.kind).toBe('timed');
    // Fixture premise: the untouched end really is ambiguous, so passing it
    // through and re-deriving it are different answers.
    expect(r.reparsedEnd).toBe(endMs - 3_600_000);

    // Moved, because the user moved it. Without this the pass-through would
    // freeze every time against every edit: the form would show 02:00 and save
    // 03:00, which is the same class of silent wrongness from the other side.
    expect(r.when.startMs).toBe(movedTo);
    expect(r.when.startMs).not.toBe(startMs);
    // And the end, which they did not touch, is still its own instant — the
    // second pass, an hour after `toMs('2026-10-25', '03:30')` answers. Each
    // side is decided on its own, so editing one does not drag the other
    // through a lossy round trip.
    expect(r.when.endMs).toBe(endMs);
  });

  test('a half-edited pair inside the repeated hour is refused, not silently moved', async ({ page }) => {
    await page.goto(PURE);
    // The cost of deciding each side on its own, pinned rather than left to be
    // discovered. Same 03:00-03:30 standup in the second pass; the user shortens
    // it by fifteen minutes and touches nothing else.
    const startMs = SECOND_PASS;
    const endMs = SECOND_PASS + 30 * 60_000;

    const r = await page.evaluate((d) => {
      const ef = (window as any).__eventform;
      const initial = ef.valueFromDetail(d, d.start_ms, d.end_ms);
      const value = { ...initial, end: '03:15' };
      const when = ef.toEventInput(value, initial).when;
      return {
        when, saveable: ef.endAfterStart(value),
        // What an all-or-nothing check would have sent for the start instead.
        wholeValueReparse: ef.toMs(value.date, value.start),
      };
    }, timedDetail(startMs, endMs));

    // The start passes through untouched, because the user did not touch it…
    expect(r.when.startMs).toBe(startMs);
    // …and the end re-derives onto the *first* pass, because '03:15' is
    // ambiguous and `toMs` resolves it there. The pair is genuinely incoherent:
    // 45 minutes backwards, with no field on the form visibly wrong.
    expect(r.when.endMs - r.when.startMs).toBe(-45 * 60_000);
    expect(r.saveable).toBe(false);

    // **This is the honest half of a real trade, not an oversight.** A check
    // that re-derived both sides together whenever *any* civil field changed
    // would answer +15 minutes here and save happily — by dragging the start
    // the user never touched an hour earlier, which is exactly the move this
    // whole task exists to stop, arriving through the back door. Refusing a
    // save the user can see and fix beats making a silent one they cannot.
    expect(r.wholeValueReparse).toBe(startMs - 3_600_000);
    expect(r.when.endMs - r.wholeValueReparse).toBe(15 * 60_000);
  });
});

// --- A create on a day whose midnight does not exist ----------------------
//
// **The inversion of `CHARACTERISED: a new event on a day whose midnight is
// skipped cannot be saved`** — the last of the five characterisation specs this
// plan opened with, and the third defect §2 names. It asserted the wrong values
// on purpose for the whole of this branch and named the right ones in its
// comments; those comments are the assertions now.
//
// What used to happen: `blankValue` moved its half-hour default onto the chosen
// day by writing a new `date` beside a `start` computed from the clock on a
// *different* day. The pair was therefore read off no instant — and on a day
// whose midnight is skipped it named none either. `toMs` normalised the start
// forward to 01:30 while the end stayed at the 01:00 a separate string had put
// there: a form opening half an hour **backwards**, refusing to save, with no
// field on it visibly wrong. Reached by pressing `n`, or by clicking that day
// in Month or Big Year, while the clock reads just after midnight.
//
// It re-anchors now — the moved pair is asked what instant it names, and the
// whole value is rebuilt from that — so the end follows the start's own instant
// by a real half hour instead of being left where it was.
//
// **No resolver, and none was needed.** What carries this is `Date`
// normalisation, which is not what design §3 originally asked for: "the first
// valid instant" on this date is 01:00, and normalisation lands on 01:30 —
// forward by the *size of the gap* from where the clock was asked for. §3 is
// amended to describe what happens rather than an ideal nothing implements.
test.describe('a create on a day whose midnight is skipped', () => {
  // 6 Sep 2026: Santiago's clocks go forward 00:00 -> 01:00, so that day has no
  // 00:00 and no 00:30. Every assertion below is about that gap, so the gap
  // itself is asserted rather than trusted — a zone or a date that stopped
  // straddling would make the whole spec pass without discriminating.
  test.use({ timezoneId: 'America/Santiago' });

  test('re-anchors onto that day and opens savable', async ({ page }) => {
    await page.goto(PURE);
    const v = await page.evaluate(() => {
      const ef = (window as any).__eventform;
      const now = new Date(2026, 5, 15, 0, 0, 0, 0).getTime(); // 15 Jun, 00:00
      const day = new Date(2026, 8, 6, 0, 0, 0, 0).getTime(); // 6 Sep — normalises to 01:00
      const value = ef.blankValue(now, 1, day);
      const when = ef.whenOf(value);
      return {
        // The fixture's premise. `getTimezoneOffset` is minutes *west*, so
        // Santiago answers 240 at UTC-4 and 180 at UTC-3; the difference is
        // the hour the clocks jumped. And midnight on the 6th, asked for
        // directly, comes back as 01:00 — there is no 00:00 to land on.
        skippedMidnightHour: new Date(day).getHours(),
        offsetBefore: new Date(2026, 8, 5, 12, 0, 0, 0).getTimezoneOffset(),
        offsetAfter: new Date(2026, 8, 6, 12, 0, 0, 0).getTimezoneOffset(),
        date: value.date, endDate: value.endDate,
        start: value.start, end: value.end,
        sourceStartMs: value.sourceStartMs, sourceEndMs: value.sourceEndMs,
        startMs: when.startMs, endMs: when.endMs,
        span: when.endMs - when.startMs,
        saveable: ef.endAfterStart(value),
      };
    });

    expect(v.skippedMidnightHour).toBe(1);
    expect(v.offsetBefore - v.offsetAfter).toBe(60);

    // Half an hour past where midnight would have been, which is where
    // normalisation puts 00:30 — not 01:00, the first instant that exists.
    expect(v.date).toBe('2026-09-06');
    expect(v.start).toBe('01:30');
    expect(v.endDate).toBe('2026-09-06');
    expect(v.end).toBe('02:30');
    expect(v.span).toBe(60 * 60_000);
    expect(v.saveable).toBe(true);

    // The other half of re-anchoring, and the half a `start`/`end` assertion
    // cannot see: both source instants are now the ones this pair was really
    // read off, so the pass-through carries them instead of being silently
    // skipped because they no longer read as the fields beside them. Named by
    // their own UTC offsets — Santiago is UTC-3 once the clocks have gone
    // forward — rather than by arithmetic on a local `Date`, which could not
    // disagree with the reading under test.
    expect(v.sourceStartMs).toBe(Date.parse('2026-09-06T01:30:00-03:00'));
    expect(v.sourceEndMs).toBe(Date.parse('2026-09-06T02:30:00-03:00'));
    expect(v.startMs).toBe(v.sourceStartMs);
    expect(v.endMs).toBe(v.sourceEndMs);
  });

  test('a day that moves nothing is left exactly where the clock put it', async ({ page }) => {
    // The converse, and the reason re-anchoring needs no "did the day actually
    // move" branch: passing the clock's own day is the same arithmetic with a
    // zero in it. Without this, a `blankValue` that ignored `dayStartMs`
    // altogether would satisfy the spec above on any day but the 6th.
    await page.goto(PURE);
    const v = await page.evaluate(() => {
      const ef = (window as any).__eventform;
      const now = new Date(2026, 5, 15, 9, 12, 0, 0).getTime(); // 15 Jun, 09:12
      const sameDay = new Date(2026, 5, 15, 0, 0, 0, 0).getTime();
      const a = ef.blankValue(now, 1);
      const b = ef.blankValue(now, 1, sameDay);
      return { a, b };
    });
    expect(v.b).toEqual(v.a);
    expect(v.a.start).toBe('09:30');
    expect(v.a.date).toBe('2026-06-15');
  });
});

// --- A time typed into an hour that does not exist ------------------------
//
// **The inversion of `CHARACTERISED: typing a start into the skipped hour kills
// Save with no field visibly wrong`** — §7.3, and the last of this branch's
// characterisation specs. It asserted the wrong values on purpose and named the
// right one in its comments; those comments are the assertions now.
//
// What used to happen: `toMs` normalises a skipped civil time **forward by the
// size of the gap**, silently. Two shapes came out of that, and only one of
// them was ever reported:
//
//   - the start shifts onto the instant the end already names, the span is
//     zero, and Save dies while both inputs show times in the right order —
//     the reported defect, and the *generic* "the end time must be after the
//     start time" describes a form the user cannot see;
//   - both times are inside the gap, both shift, the span survives, and the
//     event **saves an hour from where it was typed** with no message at all.
//     Nobody reported this one because nothing went wrong on screen.
//
// It is refused now, by the field, with the reason. **Not repaired** — the
// alternative was advancing the start past the gap and dragging the end along
// to keep the duration, which contradicts the ruling this form already made:
// an incoherent pair is answered honestly rather than fixed by moving a field
// the user did not touch.
test.describe('a time typed into an hour that does not exist', () => {
  // 6 Sep 2026: Santiago's clocks go forward 00:00 -> 01:00, so that day has
  // no 00:30. **The browser's zone, not the calendar's** — a skipped hour is a
  // property of the clock being typed into, so a fixture in a zone with no
  // transition that day witnesses nothing and passes against the old code.
  test.use({ timezoneId: 'America/Santiago' });

  const typedOn6Sep = (page: any, start: string, end: string) =>
    page.evaluate(([s0, e0]: [string, string]) => {
      const ef = (window as any).__eventform;
      const now = new Date(2026, 5, 15, 0, 0, 0, 0).getTime();
      const day = new Date(2026, 8, 6, 0, 0, 0, 0).getTime(); // 6 Sep
      const opened = ef.blankValue(now, 1, day);
      // Exactly what the two `<input type="time">` bindings write: the
      // strings, and nothing else.
      const typed = { ...opened, start: s0, end: e0 };
      return {
        skippedMidnightHour: new Date(day).getHours(),
        problem: ef.timeProblem(typed),
        when: ef.whenOf(typed),
      };
    }, [start, end]);

  test('the form says which time does not exist, and why', async ({ page }) => {
    await page.goto(PURE);
    const v = await typedOn6Sep(page, '00:30', '01:30');

    // The gap itself, asserted rather than trusted: a tzdata update that moved
    // this transition would otherwise leave every assertion below passing
    // against a date with no gap on it.
    expect(v.skippedMidnightHour).toBe(1);

    expect(v.problem).not.toBeNull();
    expect(v.problem.field).toBe('start');
    expect(v.problem.message).toContain('00:30');
    expect(v.problem.message).toContain('2026-09-06');
    expect(v.problem.message).toContain('does not exist');
    // The reason, in the user's terms. Without it the message names a fact
    // about their calendar that reads like a bug in the app.
    expect(v.problem.message).toContain('clocks go forward');
  });

  /**
   * The half that never got reported, because it saved. Both times inside the
   * gap both shift forward, so the span survives and `endAfterStart` is
   * perfectly happy — a 00:00–00:30 event written to the calendar as
   * 01:00–01:30, an hour from what was typed, silently.
   */
  test('a pair that shifts intact is refused too, not saved an hour late', async ({ page }) => {
    await page.goto(PURE);
    const v = await typedOn6Sep(page, '00:00', '00:30');

    // The old behaviour, still true of the conversion and now caught before it
    // matters: both instants land an hour on, and the span is untouched.
    expect(v.when.endMs - v.when.startMs).toBe(30 * 60_000);
    expect(v.when.startMs).toBe(Date.parse('2026-09-06T01:00:00-03:00'));

    // So `endAfterStart` alone would have let this through. It is the reason
    // `timeProblem` is asked first.
    expect(v.problem).not.toBeNull();
    expect(v.problem.field).toBe('start');
  });

  test('an end typed into the gap is named as the end', async ({ page }) => {
    await page.goto(PURE);
    // A real start, an impossible end: the message has to point at the second
    // field, or it sends the user to correct one that is already right.
    const v = await typedOn6Sep(page, '23:30', '00:30');

    expect(v.problem).not.toBeNull();
    expect(v.problem.field).toBe('end');
    expect(v.problem.message).toContain('end time');
  });

  /**
   * The all-day exemption, and it is **reachable rather than defensive**.
   *
   * Type an impossible time, then tick All day: §7.2's toggle changes only the
   * flag, so the value still carries `00:30` in a field the form no longer
   * shows and `whenOf` no longer sends. Without the exemption the form would
   * refuse an all-day event on account of a hidden time — an error naming a
   * field that is not on screen, which is the exact shape of the defect this
   * whole section exists to close.
   */
  test('an all-day value is not refused for a time it does not use', async ({ page }) => {
    await page.goto(PURE);
    const v = await page.evaluate(() => {
      const ef = (window as any).__eventform;
      const now = new Date(2026, 5, 15, 0, 0, 0, 0).getTime();
      const day = new Date(2026, 8, 6, 0, 0, 0, 0).getTime(); // 6 Sep
      const typed = { ...ef.blankValue(now, 1, day), start: '00:30', end: '01:30' };
      return {
        timedProblem: ef.timeProblem(typed),
        allDayProblem: ef.timeProblem(ef.toggledAllDay(typed, true)),
        saveable: ef.endAfterStart(ef.toggledAllDay(typed, true)),
      };
    });

    // The premise: this very value *is* refused while it is timed, so the
    // assertion below is about the flag and not about the time being fine.
    expect(v.timedProblem).not.toBeNull();

    expect(v.allDayProblem).toBeNull();
    expect(v.saveable).toBe(true);
  });

  test('a time that does exist on that day is not complained about', async ({ page }) => {
    await page.goto(PURE);
    // 01:30 and 02:00 are both real on 6 Sep, on the far side of the same gap.
    // Without this the specs above are satisfied by a form that refuses every
    // time on a transition date.
    const v = await typedOn6Sep(page, '01:30', '02:00');

    expect(v.problem).toBeNull();
    expect(v.when.startMs).toBe(Date.parse('2026-09-06T01:30:00-03:00'));
  });
});

// --- An hour that happens twice -------------------------------------------
//
// The mirror of the gap, and **deliberately not an error**. A repeated hour is
// a pair naming two instants rather than none: the time the user typed does
// exist, twice, and `toMs` answers with the earlier pass. There is nothing to
// refuse and nothing they could correct.
//
// The silence is the decision, so it is specified. Design §3 records an earlier
// version of this plan that proposed resolving ambiguity explicitly to the
// first pass, and why it was wrong — it does not close the drift it claims to,
// it makes the drift deliberate. What protects a second-pass instant is
// `instantOf`, by never re-deriving one nobody touched; a warning here would
// land on the very case that is already right.
test.describe('an hour that happens twice is saved, not refused', () => {
  // 1 Nov 2026: New York's clocks go back 02:00 -> 01:00, so 01:30 happens
  // twice. Again the browser's own zone.
  test.use({ timezoneId: 'America/New_York' });

  test('a repeated hour is not a problem, and names the earlier pass', async ({ page }) => {
    await page.goto(PURE);
    const v = await page.evaluate(() => {
      const ef = (window as any).__eventform;
      const now = new Date(2026, 5, 15, 0, 0, 0, 0).getTime();
      const day = new Date(2026, 10, 1, 0, 0, 0, 0).getTime(); // 1 Nov
      const typed = { ...ef.blankValue(now, 1, day), start: '01:30', end: '01:45' };
      return {
        ambiguous: ef.ambiguousLocalTime('2026-11-01', '01:30'),
        skipped: ef.skippedLocalTime('2026-11-01', '01:30'),
        problem: ef.timeProblem(typed),
        saveable: ef.endAfterStart(typed),
        when: ef.whenOf(typed),
      };
    });

    // The premise: this pair really does name two instants that day.
    expect(v.ambiguous).toBe(true);
    // And it is not the other thing — a pair naming two is not a pair naming
    // none, and only the second is refused.
    expect(v.skipped).toBe(false);

    expect(v.problem).toBeNull();
    expect(v.saveable).toBe(true);
    // The earlier of the two, which is what `toMs` answers and what this spec
    // exists to pin rather than to justify.
    expect(v.when.startMs).toBe(Date.parse('2026-11-01T01:30:00-04:00'));
  });

  test('an ordinary hour is neither skipped nor ambiguous', async ({ page }) => {
    await page.goto(PURE);
    // The control. Without it both predicates are satisfied by a function that
    // always answers the same thing.
    const v = await page.evaluate(() => {
      const ef = (window as any).__eventform;
      return {
        skipped: ef.skippedLocalTime('2026-11-01', '09:30'),
        ambiguous: ef.ambiguousLocalTime('2026-11-01', '09:30'),
      };
    });
    expect(v.skipped).toBe(false);
    expect(v.ambiguous).toBe(false);
  });
});

// --- An all-day event's dates cross the boundary as dates -----------------
//
// **The headline defect of Plan 6, closed, and these are its witnesses.** The
// first spec below is the inversion of
// `CHARACTERISED: an all-day trip on a calendar east of the browser is shown,
// and saved, a day early`, which asserted the wrong values on purpose for
// eight tasks and named the right ones in its comments. Those comments are the
// assertions now.
//
// What used to happen: `valueFromDetail` read the date off `start_ms` with
// `dateOf`, in the **browser's** zone. The store holds midnight in the
// **calendar's**, so east of the calendar the form opened on the previous day
// — before anybody pressed anything — and Save sent that day. Only the *start*
// was wrong: the old inclusive end stepped back `DAY_MS / 2` from the exclusive
// midnight, and half a day of slack silently absorbed the offset. So a one-day
// trip was not moved a day, it was **stretched into a two-day one**, with
// `sendUpdates=all` behind the PATCH.
//
// The fixture is a **Pacific/Auckland** calendar (UTC+12) read by a
// **Europe/Sofia** browser (UTC+3). The calendar's zone is what carries the
// test — see the describe below, which says exactly how far that goes and how
// far it does not. Unless the browser reads the stored instant as a *different*
// date from the one the calendar keeps it on, taking the detail's date and
// deriving one here are the same answer and none of this proves anything, so
// every spec asserts that reading explicitly rather than describing it in a
// comment: a comment claiming a fixture discriminates has already been
// disproved by a mutation once on this branch.
const AUCKLAND_CAL = 'Pacific/Auckland';

/**
 * A one-off all-day event as `event_detail_impl` reports one.
 *
 * `startMs`/`endMs` are midnight in the **calendar's** zone, which is how sync
 * stores an all-day event: Google sends a bare `date` and `omacal_sync`
 * resolves it against `calendars.timezone`. `startDate`/`lastDate` are the same
 * two days read back in that zone — `lastDate` **inclusive**, the day a person
 * would point at, which is the shape `EventDetail.end_date` carries.
 *
 * Built here and passed into the page rather than written inside each
 * `page.evaluate`, so the four specs below cannot drift apart in the one detail
 * that matters. The recurring case sets `is_recurring` on top of it.
 */
const allDayDetail = (startDate: string, lastDate: string, startMs: number, endMs: number) => ({
  id: 1, calendar_id: 1, title: 'Berlin trip', description: null, location: null,
  conference_uri: null, start_ms: startMs, end_ms: endMs,
  start_date: startDate, end_date: lastDate,
  is_all_day: true, is_recurring: false, recurrence: null, repeat: 'never', weekly_days: [],
  repeat_end: { kind: 'never' }, color: null,
  organizer_email: null, self_response: null, can_respond: true, can_edit: true,
  attendees: [],
  reminders: { use_default: true, overrides: [] }, calendar_default_reminders: [],
});

/** Midnight on `date` in Auckland, as the store holds an all-day event.
 *
 *  The offset is written into the literal — August is NZST, UTC+12, New
 *  Zealand's own daylight saving having ended in April — rather than looked up
 *  from the zone. An instant computed *from* `Pacific/Auckland` would be
 *  midnight there by construction and could never disagree with the zone the
 *  fixture claims. `dateIn` below is what checks the two still agree. */
const aucklandMidnight = (date: string): number => Date.parse(`${date}T00:00:00+12:00`);

/** The `yyyy-mm-dd` `ms` falls on in `zone` — `en-CA` renders ISO order.
 *  Node-side, so it reads `zone` rather than the browser's own. */
const dateIn = (ms: number, zone: string): string =>
  new Intl.DateTimeFormat('en-CA', { timeZone: zone }).format(new Date(ms));

test.describe('an all-day event’s dates cross the boundary as dates', () => {
  // The **browser**; the calendar is `AUCKLAND_CAL`, nine hours east of it in
  // August.
  //
  // Honest note on what this zone does and does not buy, because a comment
  // claiming a fixture discriminates has already been disproved by a mutation
  // once on this branch. **It is the calendar's zone that separates here, not
  // the browser's.** Auckland is UTC+12, so its midnight falls on the *previous
  // UTC date* — which means the `dateOf(start_ms)` mutation fails these specs
  // under Playwright's default UTC browser too. Verified, not assumed: it
  // failed all four with `timezoneId: 'UTC'` substituted here.
  //
  // Sofia stays because it is the real scenario the plan is about — a user east
  // of the calendar, opening a form on the wrong day — and because it puts a
  // second, independent zone in the picture, so nothing can read "the browser's
  // zone" and be accidentally right. A calendar zone with a *negative* offset
  // (the brief's `America/New_York`) would separate under neither browser.
  test.use({ timezoneId: 'Europe/Sofia' });

  test('an all-day trip on a calendar east of the browser is shown, and saved, on its own day', async ({ page }) => {
    await page.goto(PURE);
    // A one-day trip on 10 Aug 2026. One day, because that is the case where
    // the old code's damage was a stretch rather than a move — and the case a
    // three-day fixture cannot show.
    const startMs = aucklandMidnight('2026-08-10');
    const endMs = aucklandMidnight('2026-08-11');
    const detail = allDayDetail('2026-08-10', '2026-08-10', startMs, endMs);

    // Fixture check: the instant really does fall on the day the detail claims,
    // *in the calendar's zone*. Without this the `+12:00` written into
    // `aucklandMidnight` is an assumption about NZ's 2026 rules that a tzdata
    // update could quietly falsify, leaving a fixture describing a shape the
    // backend cannot produce.
    expect(dateIn(startMs, AUCKLAND_CAL)).toBe(detail.start_date);
    expect(dateIn(endMs - 1, AUCKLAND_CAL)).toBe(detail.end_date);

    const r = await page.evaluate((d) => {
      const ef = (window as any).__eventform;
      // Exactly what `App.openEdit` then `App.saveForm` do, with the user
      // touching nothing at all.
      const value = ef.valueFromDetail(d, d.start_ms, d.end_ms);
      const sent = ef.toEventInput(value, value);
      return {
        shownFirstDay: value.date, shownLastDay: value.endDate, when: sent.when,
        // The browser's own reading of the stored instant — what the old code
        // used, and what this fixture has to differ from to prove anything.
        browserReading: ef.dateOf(d.start_ms),
      };
    }, detail);

    // Fixture check, asserted rather than asserted-in-a-comment: Sofia reads
    // midnight-in-Auckland on the 10th (12:00Z on the 9th) as the **9th**. If
    // this ever stops holding, `dateOf(start_ms)` and `detail.start_date` agree
    // and every assertion below passes without discriminating.
    expect(r.browserReading).toBe('2026-08-09');
    expect(r.browserReading).not.toBe(detail.start_date);

    // Correct, and correct *before anybody presses Save*. The trip is on the
    // 10th in the zone the calendar keeps it in, and that is the day the form
    // opens on.
    expect(r.shownFirstDay).toBe('2026-08-10');
    // A one-day trip names the same day twice. It used to read '2026-08-10'
    // here too — by luck, from the half-day of slack — while the first day read
    // the 9th, so the form showed a one-day trip spanning two.
    expect(r.shownLastDay).toBe('2026-08-10');

    expect(r.when.kind).toBe('allDay');
    expect(r.when.startDate).toBe('2026-08-10');
    // Unchanged from the characterised version, and that is the point: the end
    // was already right, so a fix that moved *both* would have been wrong. The
    // exclusive end of a one-day trip on the 10th is the 11th.
    expect(r.when.endDate).toBe('2026-08-11');
  });

  test('an all-day value round-trips its dates without touching an instant', async ({ page }) => {
    await page.goto(PURE);
    // Three days — Mon 10th to Wed 12th inclusive — so the start and the end
    // are different dates and neither can stand in for the other.
    const startMs = aucklandMidnight('2026-08-10');
    const endMs = aucklandMidnight('2026-08-13');
    const detail = allDayDetail('2026-08-10', '2026-08-12', startMs, endMs);

    // Fixture check, as above: the instants fall on the days the detail claims,
    // in the calendar's own zone. `endMs - 1` because `end_ms` is the exclusive
    // midnight *after* the last day, and `end_date` is that last day.
    expect(dateIn(startMs, AUCKLAND_CAL)).toBe(detail.start_date);
    expect(dateIn(endMs - 1, AUCKLAND_CAL)).toBe(detail.end_date);

    const r = await page.evaluate((d) => {
      const ef = (window as any).__eventform;
      const value = ef.valueFromDetail(d, d.start_ms, d.end_ms);
      const sent = ef.toEventInput(value, value);
      return {
        value, when: sent.when,
        browserStart: ef.dateOf(d.start_ms), browserEnd: ef.dateOf(d.end_ms),
      };
    }, detail);

    // Fixture check: the browser reads the **start** instant as a different
    // date from the one the calendar keeps it on. That is what makes the start
    // assertions below discriminate at all.
    expect(r.browserStart).toBe('2026-08-09');
    expect(r.browserStart).not.toBe(detail.start_date);
    // The **end** does not separate the same way, and saying so plainly is
    // worth more than a check that looks stronger than it is: the browser's
    // reading of the *exclusive* midnight is the inclusive last day here, and
    // that coincidence is exactly what let the old code get the end right while
    // the start was a day early — which is why the bug stretched a trip rather
    // than moving it. What the reading does differ from is the exclusive date
    // the wire carries.
    expect(r.browserEnd).toBe('2026-08-12');
    expect(r.browserEnd).not.toBe(r.when.endDate);

    // The detail's dates reach the form unchanged…
    expect(r.value.date).toBe(detail.start_date);
    expect(r.value.endDate).toBe(detail.end_date);

    // …and reach the wire unchanged, apart from the one inclusive→exclusive
    // step the next test is about.
    expect(r.when.kind).toBe('allDay');
    expect(r.when.startDate).toBe(detail.start_date);

    // No instant on the wire at all — the union's other arm is not merely
    // unpopulated, it is absent. An all-day event has no instant to send, and
    // Rust's `WhenInput` refuses a payload that carries one.
    expect('startMs' in r.when).toBe(false);
    expect('endMs' in r.when).toBe(false);
  });

  test('an all-day end date converts inclusive to exclusive exactly once', async ({ page }) => {
    await page.goto(PURE);
    // Both spans, because they fail differently. Applied **zero** times, the
    // one-day trip becomes a zero-length event Google rejects outright; applied
    // **twice**, it becomes a two-day one — the exact harm this plan exists to
    // stop, arriving from the opposite direction.
    const oneDay = allDayDetail(
      '2026-08-10', '2026-08-10',
      aucklandMidnight('2026-08-10'), aucklandMidnight('2026-08-11'),
    );
    const trip = allDayDetail(
      '2026-08-10', '2026-08-12',
      aucklandMidnight('2026-08-10'), aucklandMidnight('2026-08-13'),
    );

    // Fixture check, as in the specs above: each instant falls on the day its
    // detail claims, in the calendar's own zone.
    for (const d of [oneDay, trip]) {
      expect(dateIn(d.start_ms, AUCKLAND_CAL)).toBe(d.start_date);
      expect(dateIn(d.end_ms - 1, AUCKLAND_CAL)).toBe(d.end_date);
    }

    const r = await page.evaluate(([one, three]) => {
      const ef = (window as any).__eventform;
      const sent = (d: any) => {
        const value = ef.valueFromDetail(d, d.start_ms, d.end_ms);
        return { shownLastDay: value.endDate, when: ef.toEventInput(value, value).when };
      };
      return { one: sent(one), three: sent(three) };
    }, [oneDay, trip]);

    // What the form **displays** is the last day a person would point at.
    expect(r.one.shownLastDay).toBe('2026-08-10');
    expect(r.three.shownLastDay).toBe('2026-08-12');

    // What the **input carries** is the day after it, once.
    expect(r.one.when.endDate).toBe('2026-08-11');
    expect(r.three.when.endDate).toBe('2026-08-13');

    // The whole claim in one line each: exactly one day between what is shown
    // and what is sent. Zero would shorten every all-day event ever saved; two
    // would lengthen it.
    const days = (from: string, to: string) =>
      (Date.parse(`${to}T00:00:00Z`) - Date.parse(`${from}T00:00:00Z`)) / 86_400_000;
    expect(days(r.one.shownLastDay, r.one.when.endDate)).toBe(1);
    expect(days(r.three.shownLastDay, r.three.when.endDate)).toBe(1);

    // And the start crosses with no conversion at all — an off-by-one applied
    // to both ends would satisfy every assertion above.
    expect(r.one.when.startDate).toBe('2026-08-10');
    expect(r.three.when.startDate).toBe('2026-08-10');
  });

  test('an all-day form value opened from a series shows the clicked occurrence’s day', async ({ page }) => {
    await page.goto(PURE);
    // **Not in the brief, and a defect in it.** `EventDetail.start_date` is
    // derived from the *store row's* `start_ms`, and for a recurring series
    // that row is the master — its date is the series' DTSTART, never the day
    // on screen. Taking it verbatim is `detail.start_ms` all over again, the
    // mistake `updateEvent`'s doc comment spends a paragraph on and the one §4
    // of the design lists under "what must not regress".
    //
    // A daily all-day series starting Mon 10 Aug, with the **Thursday** chip
    // clicked. Verbatim, the form would open on the 10th; the Rust side reads
    // the difference from `occurrenceStartMs` as a deliberate move and PATCHes
    // the occurrence four days back, with `sendUpdates=all`.
    const master = allDayDetail(
      '2026-08-10', '2026-08-10',
      aucklandMidnight('2026-08-10'), aucklandMidnight('2026-08-11'),
    );
    master.is_recurring = true;
    const occurrenceStart = aucklandMidnight('2026-08-13');
    const occurrenceEnd = aucklandMidnight('2026-08-14');

    // Fixture check: the clicked block really is a different day from the
    // master's, or "moved onto the occurrence" and "taken verbatim" agree.
    expect(occurrenceStart).not.toBe(master.start_ms);
    expect(dateIn(master.start_ms, AUCKLAND_CAL)).toBe(master.start_date);
    expect(dateIn(occurrenceStart, AUCKLAND_CAL)).toBe('2026-08-13');

    const r = await page.evaluate(([d, s, e]) => {
      const ef = (window as any).__eventform;
      const value = ef.valueFromDetail(d, s, e);
      return {
        value, when: ef.toEventInput(value, value).when,
        browserReading: ef.dateOf(s),
      };
    }, [master, occurrenceStart, occurrenceEnd] as [typeof master, number, number]);

    // Fixture check: and Sofia still reads that block's instant as the previous
    // day, so this cannot be passed by falling back to `dateOf(startMs)`.
    expect(r.browserReading).toBe('2026-08-12');

    expect(r.value.date).toBe('2026-08-13');
    expect(r.value.endDate).toBe('2026-08-13');
    expect(r.when.kind).toBe('allDay');
    expect(r.when.startDate).toBe('2026-08-13');
    expect(r.when.endDate).toBe('2026-08-14');
  });
});

// --- The other end of the same boundary -----------------------------------
//
// **Fix round 1, finding 1.** The fixture above leaves the **end** arm pinned
// by nothing: reverting only `endDate` to the pre-fix
// `dateOf(endMs - DAY_MS / 2)` left the whole suite green.
//
// That is a property of the fixture, not slack in it. The old derivation is
// correct exactly while `browserOffset − calendarOffset ∈ [−12h, +12h)` —
// stepping back half a day from the exclusive midnight absorbs any offset
// difference smaller than that. Auckland (+12) read from Sofia (+3) is −9h,
// inside the window, which is *why* the headline defect stretched a one-day
// trip instead of moving it: the end came out right by construction.
//
// **This is a third fixture rule, and it is a pair property** — not the
// calendar's zone alone, as the start arm needs. It needs
// |browserOffset − calendarOffset| ≥ 12h, re-derived per fixture from both
// zones, the way the shift rule is. `America/New_York` (−4 in August) read from
// `Asia/Tokyo` (+9) is +13h, and it is an ordinary pairing rather than an
// exotic one.
//
// Under the revert this fixture shows *and saves* a one-day trip as **two
// days** — start right, end a day late. The mirror of the headline defect,
// which this commit fixed and, until now, without a witness.

/** Midnight in New York, as the store holds an all-day event. `-04:00` because
 *  August is EDT; `dateIn` checks that claim against the zone itself. */
const newYorkMidnight = (date: string): number => Date.parse(`${date}T00:00:00-04:00`);

/** The half day the pre-fix `endDate` derivation stepped back — named so the
 *  fixture check below is visibly the old code's own arithmetic. */
const HALF_DAY_MS = 12 * 3_600_000;

test.describe('an all-day event’s last day is read, not derived either', () => {
  // The **browser**; the calendar is `America/New_York`. Thirteen hours apart,
  // which is what the *end* arm needs and what the Auckland/Sofia pairing above
  // cannot give at any time of year.
  test.use({ timezoneId: 'Asia/Tokyo' });

  test('a one-day trip on a calendar west of the browser keeps its last day', async ({ page }) => {
    await page.goto(PURE);
    const startMs = newYorkMidnight('2026-08-10');
    const endMs = newYorkMidnight('2026-08-11');
    const detail = allDayDetail('2026-08-10', '2026-08-10', startMs, endMs);

    // Fixture check: the instants fall on the days the detail claims, in the
    // calendar's own zone.
    expect(dateIn(startMs, 'America/New_York')).toBe(detail.start_date);
    expect(dateIn(endMs - 1, 'America/New_York')).toBe(detail.end_date);

    // Fixture check, and the one this whole describe exists for: the **old
    // code's own arithmetic**, run here, gives a different answer from the date
    // the calendar keeps. Asserted rather than described, because the pairing
    // that makes it true is a fact about two zones and can be got wrong.
    expect(dateIn(endMs - HALF_DAY_MS, 'Asia/Tokyo')).toBe('2026-08-11');
    expect(dateIn(endMs - HALF_DAY_MS, 'Asia/Tokyo')).not.toBe(detail.end_date);
    // And the honest converse: this fixture does **not** separate the *start*
    // arm. The browser reads the start instant as the right day, so nothing
    // here would catch `dateOf(startMs)` — that is the Auckland/Sofia describe's
    // job, and the two fixtures are needed for the two arms.
    expect(dateIn(startMs, 'Asia/Tokyo')).toBe(detail.start_date);

    const r = await page.evaluate((d) => {
      const ef = (window as any).__eventform;
      const value = ef.valueFromDetail(d, d.start_ms, d.end_ms);
      return { value, when: ef.toEventInput(value, value).when };
    }, detail);

    // The last day a person would point at, unmoved.
    expect(r.value.endDate).toBe('2026-08-10');
    // …and the start, which was never in doubt here.
    expect(r.value.date).toBe('2026-08-10');

    expect(r.when.kind).toBe('allDay');
    expect(r.when.startDate).toBe('2026-08-10');
    // One day, still. Under the revert this is '2026-08-12' — a two-day event
    // mailed to the whole guest list by a save that touched only the title.
    expect(r.when.endDate).toBe('2026-08-11');
  });
});

// --- Counting the occurrence's shift in whole days -------------------------
//
// **Fix round 1, finding 2.** `occurrenceDate` divides a millisecond difference
// by a day and rounds. `Math.round` was argued in prose and asserted by
// nothing: `Math.floor` left the whole suite green, and so did `Math.ceil`.
//
// Neither is equivalent. Both instants are midnight in the calendar's zone, so
// their difference is a whole number of days **plus or minus the offset change
// between them** — and a series straddling a transition is the only place that
// shows. Spring-forward makes the gap short (95h for four days), fall-back
// makes it long (97h). `Math.floor` gets the short one wrong, `Math.ceil` the
// long one, and `Math.round` is the only one right on both.
//
// **Both fixtures are needed**; one alone leaves the other survivor alive. The
// reviewer's note suggested a single straddling fixture would close it — a
// spring-forward one does not catch `Math.ceil`, which is why there are two
// tests here rather than one.
//
// The harm is exactly what `occurrenceDate` exists to prevent: the form opens a
// day off the chip that was clicked, and a title-only save PATCHes the
// occurrence there with `sendUpdates=all`.

/** Midnight in Sofia. The offset is a parameter rather than a constant because
 *  the change *between* two of them is the whole subject: EET is UTC+2, EEST
 *  UTC+3, and every fixture below spans the switch. `dateIn` checks each. */
const sofiaMidnight = (date: string, offset: '+02:00' | '+03:00'): number =>
  Date.parse(`${date}T00:00:00${offset}`);

test.describe('an all-day occurrence’s shift is counted in whole days', () => {
  // Named rather than inherited. The subtraction in `occurrenceDate` is
  // zone-free — that is the property — so the browser's zone cannot change the
  // answer; UTC is chosen because it still reads the Sofia calendar's midnight
  // as the *previous* day, so none of these can be passed by a `dateOf`
  // derivation either.
  test.use({ timezoneId: 'UTC' });

  const SOFIA = 'Europe/Sofia';

  /** Drives `valueFromDetail` for a one-day all-day series master and a clicked
   *  chip, and reports what the form shows and what a save would send. */
  const openedOn = async (
    page: import('@playwright/test').Page,
    master: ReturnType<typeof allDayDetail>,
    chipStart: number,
    chipEnd: number,
  ) => {
    await page.goto(PURE);
    return page.evaluate(([d, s, e]) => {
      const ef = (window as any).__eventform;
      const value = ef.valueFromDetail(d, s, e);
      return {
        value, when: ef.toEventInput(value, value).when, browserReading: ef.dateOf(s),
      };
    }, [master, chipStart, chipEnd] as [typeof master, number, number]);
  };

  test('a series straddling a spring-forward keeps the clicked day', async ({ page }) => {
    // 29 Mar 2026 is the European spring-forward: 03:00 becomes 04:00, so that
    // day is 23 hours and four days of this series are 95, not 96.
    const master = allDayDetail(
      '2026-03-27', '2026-03-27',
      sofiaMidnight('2026-03-27', '+02:00'), sofiaMidnight('2026-03-28', '+02:00'),
    );
    master.is_recurring = true;
    const chipStart = sofiaMidnight('2026-03-31', '+03:00');
    const chipEnd = sofiaMidnight('2026-04-01', '+03:00');

    // Fixture checks. The gap in hours is asserted outright: it is the entire
    // reason this fixture discriminates, and a fixture that quietly stopped
    // straddling the transition would go on passing while proving nothing.
    expect((chipStart - master.start_ms) / 3_600_000).toBe(95);
    expect(dateIn(master.start_ms, SOFIA)).toBe('2026-03-27');
    expect(dateIn(chipStart, SOFIA)).toBe('2026-03-31');

    const r = await openedOn(page, master, chipStart, chipEnd);

    // And that a browser-zone derivation cannot pass this either.
    expect(r.browserReading).toBe('2026-03-30');

    // `Math.floor(95/24)` is 3, and answers '2026-03-30' — the day before the
    // chip the user clicked.
    expect(r.value.date).toBe('2026-03-31');
    expect(r.value.endDate).toBe('2026-03-31');
    expect(r.when.startDate).toBe('2026-03-31');
    expect(r.when.endDate).toBe('2026-04-01');
  });

  test('a series straddling a fall-back keeps the clicked day', async ({ page }) => {
    // 25 Oct 2026 is the European fall-back: 04:00 becomes 03:00, so that day is
    // 25 hours and four days of this series are 97. The mirror of the case
    // above, and the one that catches `Math.ceil` — which the spring-forward
    // fixture alone does not.
    const master = allDayDetail(
      '2026-10-23', '2026-10-23',
      sofiaMidnight('2026-10-23', '+03:00'), sofiaMidnight('2026-10-24', '+03:00'),
    );
    master.is_recurring = true;
    const chipStart = sofiaMidnight('2026-10-27', '+02:00');
    const chipEnd = sofiaMidnight('2026-10-28', '+02:00');

    expect((chipStart - master.start_ms) / 3_600_000).toBe(97);
    expect(dateIn(master.start_ms, SOFIA)).toBe('2026-10-23');
    expect(dateIn(chipStart, SOFIA)).toBe('2026-10-27');

    const r = await openedOn(page, master, chipStart, chipEnd);

    expect(r.browserReading).toBe('2026-10-26');

    // `Math.ceil(97/24)` is 5, and answers '2026-10-28' — the day after the
    // chip the user clicked.
    expect(r.value.date).toBe('2026-10-27');
    expect(r.value.endDate).toBe('2026-10-27');
    expect(r.when.startDate).toBe('2026-10-27');
    expect(r.when.endDate).toBe('2026-10-28');
  });
});

test.describe('the anchor’s precision', () => {
  // Zone-independent, so these need no page: what is at stake is what an
  // `HH:MM` field can *express*, not where it is read. Google stores a start to
  // the second and plenty of real events have one.
  const startMs = new Date(2026, 7, 5, 9, 0, 37, 0).getTime();
  const endMs = startMs + 30 * 60_000;

  test('a start with seconds on it keeps them', () => {
    const value = valueFromDetail(timedDetail(startMs, endMs), startMs, endMs);
    const sent = toEventInput(value, value);

    // `when` is a union, so the timed arm has to be established before its
    // fields can be read — which is the point of the union and worth spelling
    // out rather than casting past.
    expect(sent.when.kind).toBe('timed');
    if (sent.when.kind !== 'timed') throw new Error('not a timed event');
    // Task 9's anchoring invariant: an untouched time sends an anchor equal to
    // `occurrenceStartMs` *exactly*. This drift used to be -37,000 ms, and the
    // Rust side reads 37 seconds as a move like any other — a start/end PATCH
    // with `sendUpdates=all` behind it for somebody who renamed a meeting.
    expect(sent.when.startMs - startMs).toBe(0);
    expect(sent.when.startMs).toBe(startMs);
    // The form still shows only what it can show. The seconds are carried past
    // it, not displayed in it.
    expect(value.start).toBe('09:00');
  });

  test('a start the user did edit loses the seconds the form cannot express', () => {
    const initial = valueFromDetail(timedDetail(startMs, endMs), startMs, endMs);
    const value = { ...initial, start: '10:00' };
    const sent = toEventInput(value, initial);

    expect(sent.when.kind).toBe('timed');
    if (sent.when.kind !== 'timed') throw new Error('not a timed event');
    // Deliberate, and the other half of the rule: sub-minute precision is
    // *discarded* on an edit, because a form with no seconds field cannot let
    // anybody express the 37. What must not happen is the discard reading as a
    // move, which is what the spec above pins. A user who retypes the start
    // gets exactly the minute they typed.
    expect(sent.when.startMs).toBe(new Date(2026, 7, 5, 10, 0, 0, 0).getTime());
    expect(new Date(sent.when.startMs).getSeconds()).toBe(0);
    // The end, untouched, keeps its own seconds.
    expect(sent.when.endMs).toBe(endMs);
  });
});

/**
 * §7.2 — the All day switch, in all four directions.
 *
 * The spec that recorded this said plainly that **none of the four
 * all-day↔timed toggle combinations has a TypeScript-level spec**, and that is
 * how a form Save refuses survived two clicks from an ordinary edit.
 *
 * The acceptance property is one sentence: **toggling a switch is not a way to
 * reach a state Save refuses.** Every case below asserts `endAfterStart` on the
 * toggled value, which is the very question `EventForm`'s Save guard asks.
 *
 * Every case goes through `toggledAllDay`, the function the checkbox's
 * `onchange` calls. That matters more than it looks: these started out
 * modelling the flick as `{ ...value, isAllDay: x }`, which is what the old
 * `bind:checked` did — and a spec that re-implements the thing it tests stays
 * green however the component later changes. The two round-trip cases below
 * were tautologies under that model and no mutation could redden them.
 */
test.describe('the All day switch never lands on a value Save refuses', () => {
  // The browser is east of a UTC calendar and west of the Auckland one, so the
  // stored midnight reads as neither 00:00 nor the same number in both. That is
  // exactly what made the old `timeOf(startMs)` a zone artefact rather than a
  // time: 03:00 for the UTC calendar, 15:00 for Auckland.
  test.use({ timezoneId: 'Europe/Sofia' });

  /** Opens an all-day detail and flips All day off, as a click does. */
  const toggledOff = (page: any, detail: unknown) =>
    page.evaluate((d: any) => {
      const ef = (window as any).__eventform;
      const opened = ef.valueFromDetail(d, d.start_ms, d.end_ms);
      const off = ef.toggledAllDay(opened, false);
      return {
        date: off.date, endDate: off.endDate, start: off.start, end: off.end,
        saveable: ef.endAfterStart(off),
        when: ef.whenOf(off),
      };
    }, detail);

  const utcMidnight = (date: string): number => Date.parse(`${date}T00:00:00Z`);

  /**
   * The case §7.2 names. A single-day all-day event names the same date twice,
   * so before the fix both ends read as the same clock and the span was zero —
   * `12:00 → 12:00` on screen, and a Save button that refused a form with
   * nothing on it visibly wrong.
   */
  test('toggling All day off on a single-day event leaves a saveable span', async ({ page }) => {
    await page.goto(PURE);
    const detail = allDayDetail(
      '2026-08-10', '2026-08-10', utcMidnight('2026-08-10'), utcMidnight('2026-08-11'),
    );

    const off = await toggledOff(page, detail);

    expect(off.date).toBe('2026-08-10');
    expect(off.endDate).toBe('2026-08-10');
    expect(off.saveable).toBe(true);
    expect(off.when.endMs).toBeGreaterThan(off.when.startMs);
  });

  /**
   * The same event on a calendar twelve hours the other side of the browser.
   * The zone is the fixture: a stored Auckland midnight reads as 15:00 here, so
   * a start time taken from the instant is visibly not a time anybody chose.
   * Its being *equal* to the end is what refused the save; its being 15:00 at
   * all is the reason the pair was never a time.
   */
  test('toggling All day off is not at the mercy of the calendar’s zone', async ({ page }) => {
    await page.goto(PURE);
    const detail = allDayDetail(
      '2026-08-10', '2026-08-10',
      aucklandMidnight('2026-08-10'), aucklandMidnight('2026-08-11'),
    );

    const off = await toggledOff(page, detail);

    expect(off.saveable).toBe(true);
    // The same pair as the UTC-calendar case above: what a toggle produces
    // cannot depend on which zone the calendar happens to be in.
    expect(off.start).toBe('09:00');
    expect(off.end).toBe('10:00');
  });

  /**
   * A multi-day trip keeps every date it covered. **The dates are what must not
   * move** — see the round-trip spec below for why that is the property rather
   * than preserving the event's extent in instants.
   */
  test('toggling All day off on a multi-day trip keeps its first and last day', async ({ page }) => {
    await page.goto(PURE);
    const detail = allDayDetail(
      '2026-08-10', '2026-08-12', utcMidnight('2026-08-10'), utcMidnight('2026-08-13'),
    );

    const off = await toggledOff(page, detail);

    expect(off.date).toBe('2026-08-10');
    expect(off.endDate, 'the inclusive last day, unmoved').toBe('2026-08-12');
    expect(off.saveable).toBe(true);
  });

  /** The other direction, single day: a timed meeting becomes a one-day event. */
  test('toggling All day on for a timed meeting gives the day it was on', async ({ page }) => {
    await page.goto(PURE);
    const startMs = Date.parse('2026-08-10T09:00:00+03:00');
    const endMs = Date.parse('2026-08-10T09:30:00+03:00');

    const on = await page.evaluate(([s, e]) => {
      const ef = (window as any).__eventform;
      const value = { ...ef.blankValueAt(s, 1), sourceEndMs: e, end: ef.timeOf(e) };
      const flipped = ef.toggledAllDay(value, true);
      return { when: ef.whenOf(flipped), saveable: ef.endAfterStart(flipped) };
    }, [startMs, endMs]);

    expect(on.saveable).toBe(true);
    expect(on.when.startDate).toBe('2026-08-10');
    // Exclusive on the wire: one day means the 10th, ending the 11th.
    expect(on.when.endDate).toBe('2026-08-11');
  });

  /**
   * And a timed meeting that runs past midnight becomes the two days it
   * genuinely touches. Saveable, and the days are the ones its own dates named
   * — a 23:00–00:30 meeting really is on both.
   */
  test('toggling All day on for a meeting past midnight covers both days', async ({ page }) => {
    await page.goto(PURE);
    const startMs = Date.parse('2026-08-10T23:00:00+03:00');
    const endMs = Date.parse('2026-08-11T00:30:00+03:00');

    const on = await page.evaluate(([s, e]) => {
      const ef = (window as any).__eventform;
      const value = {
        ...ef.blankValueAt(s, 1),
        sourceEndMs: e, end: ef.timeOf(e), endDate: ef.dateOf(e),
      };
      const flipped = ef.toggledAllDay(value, true);
      return { when: ef.whenOf(flipped), saveable: ef.endAfterStart(flipped) };
    }, [startMs, endMs]);

    expect(on.saveable).toBe(true);
    expect(on.when.startDate).toBe('2026-08-10');
    expect(on.when.endDate, 'the 10th and the 11th, exclusive end').toBe('2026-08-12');
  });

  /**
   * **The property that decides the multi-day question.**
   *
   * A checkbox has to be reversible: flick it twice and you are where you
   * started. That is what rules out the tempting alternative for toggling off —
   * converting the all-day extent into instants, first day 00:00 to the day
   * *after* the last at 00:00. It is the more faithful reading of "how long the
   * event is", and it is not reversible: `endDate` on the all-day arm is the
   * **inclusive** last day, so reading that exclusive end back grows the trip by
   * a day on every round trip. A toggle that lengthens the event each time you
   * flick it twice is worse than one that leaves you to type an end time.
   */
  test('flicking All day off and on again returns the days it started with', async ({ page }) => {
    await page.goto(PURE);
    const detail = allDayDetail(
      '2026-08-10', '2026-08-12', utcMidnight('2026-08-10'), utcMidnight('2026-08-13'),
    );

    const r = await page.evaluate((d: any) => {
      const ef = (window as any).__eventform;
      const opened = ef.valueFromDetail(d, d.start_ms, d.end_ms);
      const off = ef.toggledAllDay(opened, false);
      const backOn = ef.toggledAllDay(off, true);
      return {
        before: ef.whenOf(opened),
        after: ef.whenOf(backOn),
        saveable: ef.endAfterStart(backOn),
      };
    }, detail);

    expect(r.after).toEqual(r.before);
    expect(r.saveable).toBe(true);
  });

  /**
   * And the same in the other direction: a timed event's times survive a trip
   * through all-day, because they live in the very fields the all-day arm
   * leaves alone. Without this, toggling on and off again would silently
   * replace a 14:00 meeting with the default hour.
   */
  test('flicking All day on and off again returns the times it started with', async ({ page }) => {
    await page.goto(PURE);
    const startMs = Date.parse('2026-08-10T14:00:00+03:00');

    const r = await page.evaluate((s) => {
      const ef = (window as any).__eventform;
      const timed = ef.blankValueAt(s, 1);
      const on = ef.toggledAllDay(timed, true);
      const backOff = ef.toggledAllDay(on, false);
      return {
        before: { start: timed.start, end: timed.end },
        after: { start: backOff.start, end: backOff.end },
        saveable: ef.endAfterStart(backOff),
      };
    }, startMs);

    expect(r.after).toEqual(r.before);
    expect(r.saveable).toBe(true);
  });
});

/**
 * A contract, pinned directly, because **no behavioural test can reach it** and
 * that is worth saying rather than leaving it to look covered.
 *
 * `sourceStartMs`'s contract is "the instant this civil pair was read off", and
 * `instantOf` passes a source through only when the pair still reads back as it.
 * On the all-day arm the times are now [`ALL_DAY_START`]/[`ALL_DAY_END`], which
 * were not read off anything, so the field would be a lie — but a harmless one:
 * for any stored instant that *could* satisfy `instantOf`'s guard, `toMs` on the
 * same pair rebuilds that very instant, so the two answers are equal. (A
 * calendar six hours west of a Sofia browser is such a case: its stored midnight
 * reads as exactly 09:00 on exactly that date.) The exceptions `instantOf`
 * exists for — a seconds value `HH:MM` cannot carry, a repeated wall-clock hour
 * — cannot arise from a stored midnight at 09:00.
 *
 * So this asserts the field, not an outcome. Deleting the `null` reddens this
 * and nothing else, which is the honest amount of coverage for a change that
 * makes a type truthful rather than a behaviour correct.
 */
/**
 * The all-day draft has to stay drawable: the ribbon a sideways sweep drew
 * used to vanish the moment the form opened, so the thing being decided
 * about stopped being visible at exactly the wrong time (reported
 * 2026-08-31).
 */
test.describe('the form ghost', () => {
  const CAL = '#8e7cc3';
  const ghost = (
    page: import('@playwright/test').Page,
    value: Record<string, unknown>,
    color: string | null = CAL,
  ) =>
    page.evaluate(
      ([v, c]) => (window as any).__eventform.previewGhost(v, c),
      [value, color] as const,
    );

  const allDay = {
    isAllDay: true, date: '2026-09-01', endDate: '2026-09-03',
    start: '', end: '', title: '',
  };

  test('an all-day value is drawn as the days it covers', async ({ page }) => {
    await page.goto(PURE);
    expect(await ghost(page, allDay)).toEqual({
      kind: 'allDay', firstDate: '2026-09-01', lastDate: '2026-09-03', color: CAL,
    });
  });

  test('a one-day all-day value covers just its day', async ({ page }) => {
    await page.goto(PURE);
    expect(await ghost(page, { ...allDay, endDate: '' })).toEqual({
      kind: 'allDay', firstDate: '2026-09-01', lastDate: '2026-09-01', color: CAL,
    });
  });

  test('an end before the start draws nothing rather than a backwards bar', async ({ page }) => {
    await page.goto(PURE);
    expect(await ghost(page, { ...allDay, endDate: '2026-08-30' })).toBeNull();
  });

  test('a timed value keeps its span', async ({ page }) => {
    await page.goto(PURE);
    const got: any = await ghost(page, {
      ...allDay, isAllDay: false, start: '09:00', end: '10:00', endDate: '2026-09-01',
    });
    expect(got.kind).toBe('timed');
    expect(got.endMs - got.startMs).toBe(60 * 60_000);
  });

  /**
   * The draft says which calendar it would land on, so the colour has to
   * reach the grid on the ghost itself — both arms, and a calendar with no
   * colour of its own has to be tellable from one that has, since the grid
   * falls back to `--accent` for exactly that case (2026-08-31, by request).
   */
  test('the calendar colour rides on both arms, and null stays null', async ({ page }) => {
    await page.goto(PURE);
    const timed = { ...allDay, isAllDay: false, start: '09:00', end: '10:00' };
    expect((await ghost(page, timed) as any).color).toBe(CAL);
    expect((await ghost(page, allDay) as any).color).toBe(CAL);
    expect((await ghost(page, timed, null) as any).color).toBeNull();
    expect((await ghost(page, allDay, null) as any).color).toBeNull();
  });
});

test('an all-day value carries no source instants, because its times were not read off any', async ({ page }) => {
  await page.goto(PURE);
  const detail = allDayDetail(
    '2026-08-10', '2026-08-10',
    Date.parse('2026-08-10T00:00:00Z'), Date.parse('2026-08-11T00:00:00Z'),
  );

  const v = await page.evaluate((d: any) => {
    const ef = (window as any).__eventform;
    const value = ef.valueFromDetail(d, d.start_ms, d.end_ms);
    return { sourceStartMs: value.sourceStartMs, sourceEndMs: value.sourceEndMs };
  }, detail);

  expect(v.sourceStartMs).toBeNull();
  expect(v.sourceEndMs).toBeNull();
});

// --- The guest list ------------------------------------------------------
//
// Pure, and exercised directly for the reason the block at the top of this file
// gives: driving them through a rendered form would reach only the cases the
// form happens to offer, and the ones that matter here — a duplicate spelled
// differently, an address that is not one, the organizer — are exactly the ones
// a form makes hard to produce.

/** `timedDetail` with a guest list on it. */
const withGuests = (attendees: EventDetail['attendees'], organizer?: string): EventDetail => ({
  ...timedDetail(0, 30 * 60_000),
  organizer_email: organizer ?? null,
  attendees,
});

const attendee = (
  email: string,
  extra: Partial<EventDetail['attendees'][number]> = {},
): EventDetail['attendees'][number] => ({
  email,
  display_name: null,
  response_status: 'needsAction',
  optional: false,
  is_self: false,
  ...extra,
});

test.describe('the guest list a form edits', () => {
  /**
   * **Everyone, the signed-in user included.**
   *
   * `mailableGuests` excludes the self row because it counts who would be
   * *mailed*, and this list is a different thing entirely: it is what the
   * event's attendees would become. Excluding yourself here would take you off
   * every event you ever saved — and, because the drag path shares this value,
   * off every event you ever dragged.
   */
  test('carries every attendee, the signed-in user included', () => {
    const value = valueFromDetail(
      withGuests([
        attendee('ana@x.com', { response_status: 'accepted' }),
        attendee('me@x.com', { is_self: true }),
        attendee('bo@x.com', { optional: true }),
      ]),
      0,
      30 * 60_000,
    );

    expect(value.guests.map((g) => g.email)).toEqual(['ana@x.com', 'me@x.com', 'bo@x.com']);
    expect(value.guests[2].optional, 'the stored optional flag comes through').toBe(true);
    // …and the count beside it still excludes the self row, because the two
    // answer different questions.
    expect(mailableGuests(value, value)).toBe(2);
  });

  test('a create starts with nobody on it', () => {
    expect(blankValueAt(0, 1).guests).toEqual([]);
  });

  test('carries the organizer, so the row that cannot be removed can be found', () => {
    const value = valueFromDetail(
      withGuests([attendee('ana@x.com')], 'ana@x.com'),
      0,
      30 * 60_000,
    );
    expect(value.organizerEmail).toBe('ana@x.com');
  });

  test('carries which row is yours, and null when you are not on the event', () => {
    const mine = valueFromDetail(
      withGuests([attendee('ana@x.com'), attendee('me@x.com', { is_self: true })]),
      0, 30 * 60_000,
    );
    expect(mine.selfEmail).toBe('me@x.com');

    const theirs = valueFromDetail(withGuests([attendee('ana@x.com')]), 0, 30 * 60_000);
    expect(theirs.selfEmail).toBeNull();
  });

  /**
   * **Who this save could mail** — the one rule, replacing `guestCount`.
   *
   * Everyone on the resulting list, plus everyone removed from it (a removal
   * with notify on sends a cancellation, guest-list spec §3), minus yourself.
   * `guestCount` answered only the first clause and only for an edit, and was
   * hard-coded 0 on a create — correct exactly while a create could not invite
   * anybody.
   */
  test.describe('mailableGuests', () => {
    /** A create's starting value, plus whatever guests were typed into it. */
    const created = (...emails: string[]): EventFormValue => ({
      ...blankValueAt(0, 1),
      guests: emails.map((email) => ({ email, optional: false })),
    });

    test('a create counts everyone typed into it', () => {
      const initial = blankValueAt(0, 1);
      expect(mailableGuests(created('ana@x.com', 'bo@x.com'), initial)).toBe(2);
    });

    test('a create with nobody on it counts nobody', () => {
      const initial = blankValueAt(0, 1);
      expect(mailableGuests(initial, initial)).toBe(0);
    });

    /**
     * The defect this rule fixes. `guestCount` counted who was on the event
     * when the form *opened*, so the first guest added to a guestless event
     * came out 0 — and 0 takes the form's straight-to-save shortcut, which
     * sends `notify: 'none'`. The invitee was added and never mailed, with
     * nothing asked and nothing said.
     */
    test('an edit counts a guest added to an event that had none', () => {
      const initial = valueFromDetail(withGuests([]), 0, 30 * 60_000);
      const value = { ...initial, guests: [{ email: 'ana@x.com', optional: false }] };
      expect(mailableGuests(value, initial)).toBe(1);
    });

    test('an untouched edit counts the other attendees, never yourself', () => {
      const initial = valueFromDetail(
        withGuests([
          attendee('ana@x.com'),
          attendee('bo@x.com'),
          attendee('me@x.com', { is_self: true }),
        ]),
        0,
        30 * 60_000,
      );
      expect(mailableGuests(initial, initial)).toBe(2);
    });

    /** A removal with notify on sends a cancellation, so the person removed is
     *  still somebody this save could mail. Counting only the resulting list
     *  would answer 1 and skip the question entirely on a save whose whole
     *  purpose was to un-invite someone. */
    test('a removed guest is still somebody the save could mail', () => {
      const initial = valueFromDetail(
        withGuests([attendee('ana@x.com'), attendee('bo@x.com')]),
        0,
        30 * 60_000,
      );
      const value = { ...initial, guests: [{ email: 'ana@x.com', optional: false }] };
      expect(mailableGuests(value, initial)).toBe(2);
    });

    /** Compared the way every other guest rule compares — Google treats a
     *  mailbox case-insensitively, and a rule that did not would count one
     *  person twice and ask about a guest nobody added. */
    test('the same person spelled two ways is one person', () => {
      const initial = valueFromDetail(withGuests([attendee('ana@x.com')]), 0, 30 * 60_000);
      const value = { ...initial, guests: [{ email: 'Ana@X.com', optional: false }] };
      expect(mailableGuests(value, initial)).toBe(1);
    });

    /** Yourself excluded from both sides, not just the resulting list:
     *  removing your own row is a thing §5 explicitly allows, and telling
     *  somebody they are about to mail themselves about it is wrong. */
    test('yourself is excluded even when you are the one being removed', () => {
      const initial = valueFromDetail(
        withGuests([attendee('ana@x.com'), attendee('me@x.com', { is_self: true })]),
        0,
        30 * 60_000,
      );
      const value = { ...initial, guests: [{ email: 'ana@x.com', optional: false }] };
      expect(mailableGuests(value, initial)).toBe(1);
    });
  });
});

test.describe('adding a guest', () => {
  const list = [{ email: 'ana@x.com', optional: false }];

  test('appends the address', () => {
    expect(addGuest(list, 'bo@x.com')).toEqual([
      { email: 'ana@x.com', optional: false },
      { email: 'bo@x.com', optional: false },
    ]);
  });

  test('trims what was typed', () => {
    expect(addGuest([], '  bo@x.com  ')[0].email).toBe('bo@x.com');
  });

  /**
   * §5: **a duplicate address is a no-op, not an error and not a second row.**
   * Returning the same list rather than a copy is what lets a caller compare by
   * identity and know nothing happened.
   */
  test('an address already on the list changes nothing', () => {
    expect(addGuest(list, 'ana@x.com')).toBe(list);
  });

  test('and neither does the same address spelled differently', () => {
    expect(addGuest(list, ' Ana@X.com ')).toBe(list);
  });

  /** Nothing typed is not an address. Its own case because an empty add is
   *  what a stray Return in the field produces. */
  test('an empty address changes nothing', () => {
    expect(addGuest(list, '   ')).toBe(list);
  });
});

test.describe('an address that is not an address', () => {
  /**
   * §5: refused **in the form, before Save** — never by a 400 coming back from
   * Google. A table, because the shapes people actually type are the point and
   * a form can only produce a handful of them.
   */
  const cases: Array<[address: string, ok: boolean, why: string]> = [
    ['ana@x.com', true, 'the ordinary case'],
    ['ana.b+tag@sub.example.co.uk', true, 'dots, a plus and a long domain'],
    ['ANA@X.COM', true, 'case is not a validity question'],
    ['  ana@x.com  ', true, 'surrounding space is trimmed, not rejected'],
    ['', false, 'nothing typed'],
    ['ana', false, 'no domain at all'],
    ['ana@', false, 'nothing after the at'],
    ['@x.com', false, 'nothing before it'],
    ['ana@x', false, 'a domain with no dot is not one anybody can be mailed at'],
    ['ana x@y.com', false, 'a space inside'],
    ['ana@@x.com', false, 'two ats'],
    ['ana@x..com', false, 'an empty domain label'],
  ];

  for (const [address, ok, why] of cases) {
    test(`"${address}" is ${ok ? 'an address' : 'refused'} — ${why}`, () => {
      expect(isAddress(address)).toBe(ok);
    });
  }
});

test.describe('removing a guest', () => {
  const list = [
    { email: 'ana@x.com', optional: false },
    { email: 'me@x.com', optional: false },
  ];

  test('takes that one off and leaves the rest', () => {
    expect(removeGuest(list, 'ana@x.com')).toEqual([{ email: 'me@x.com', optional: false }]);
  });

  test('matches an address however it is spelled', () => {
    expect(removeGuest(list, 'ANA@x.com')).toEqual([{ email: 'me@x.com', optional: false }]);
  });

  /**
   * §5: **the organizer cannot be removed.** Google refuses it, so a UI that
   * offered it would produce a save that fails for a reason the user cannot see
   * — and this is the predicate the row's own control is built from, so the
   * button is absent rather than present and disappointing.
   */
  test('the organizer is not removable', () => {
    expect(removableGuest('ana@x.com', 'ana@x.com')).toBe(false);
    expect(removableGuest('me@x.com', 'ana@x.com')).toBe(true);
    // Spelled differently, still the organizer.
    expect(removableGuest('Ana@X.com', 'ana@x.com')).toBe(false);
  });

  /** An event with no organizer on it — Google omits the field for some — makes
   *  nobody unremovable rather than everybody. */
  test('no organizer means no protected row', () => {
    expect(removableGuest('ana@x.com', null)).toBe(true);
  });
});

test.describe('marking a guest optional', () => {
  const list = [
    { email: 'ana@x.com', optional: false },
    { email: 'bo@x.com', optional: true },
  ];

  test('flips that one and leaves the rest', () => {
    expect(toggledGuestOptional(list, 'ana@x.com')).toEqual([
      { email: 'ana@x.com', optional: true },
      { email: 'bo@x.com', optional: true },
    ]);
  });

  test('flips back off again', () => {
    expect(toggledGuestOptional(list, 'bo@x.com')[1].optional).toBe(false);
  });
});

test.describe('whether the guest list changed', () => {
  const list = [
    { email: 'ana@x.com', optional: false },
    { email: 'bo@x.com', optional: true },
  ];

  test('the same list is the same list', () => {
    expect(sameGuests(list, list.map((g) => ({ ...g })))).toBe(true);
  });

  test('an added or removed address is a change', () => {
    expect(sameGuests(list, [...list, { email: 'cy@x.com', optional: false }])).toBe(false);
    expect(sameGuests(list, [list[0]])).toBe(false);
  });

  test('a flipped optional flag is a change', () => {
    expect(sameGuests(list, [list[0], { email: 'bo@x.com', optional: false }])).toBe(false);
  });

  /**
   * **Order is not a change.** The Rust side compares the array it would send
   * against the array the event already has, and reorders come out equal there
   * — so a form that called a reshuffle a change would send a whole-list
   * replace nobody asked for, on an event nobody edited.
   */
  test('a reordered list is not a change', () => {
    expect(sameGuests(list, [list[1], list[0]])).toBe(true);
  });
});

test.describe('what a save sends about guests', () => {
  const detail = withGuests(
    [attendee('ana@x.com'), attendee('me@x.com', { is_self: true })],
    'ana@x.com',
  );
  const initial = () => valueFromDetail(detail, 0, 30 * 60_000);

  /**
   * **Unchanged means absent**, the same three-state `repeat` runs on and for a
   * sharper reason: `attendees` is a whole-list replace on Google's side, so a
   * payload that carried the list on every save would rewrite every attendee of
   * every event this form ever touched, from whatever omacal last read.
   *
   * It is also what makes a **drag** structurally unable to change a guest
   * list: the drag builds its input from this same value with only the times
   * moved, so the lists compare equal and no `guests` key is sent at all.
   */
  test('a save that left the guest list alone sends no guests', () => {
    const value = initial();
    expect(toEventInput(value, initial()).guests).toBeUndefined();

    // The drag's own shape: the same value with the times moved.
    const moved = { ...value, start: '10:00', end: '10:30', sourceStartMs: null, sourceEndMs: null };
    expect(toEventInput(moved, initial()).guests).toBeUndefined();
  });

  test('a save that changed it sends the whole list', () => {
    const value = initial();
    value.guests = addGuest(value.guests, 'bo@x.com');

    expect(toEventInput(value, initial()).guests).toEqual([
      { email: 'ana@x.com', optional: false },
      { email: 'me@x.com', optional: false },
      { email: 'bo@x.com', optional: false },
    ]);
  });

  /** Removing everybody is a change like any other, and `[]` is what says so —
   *  distinct from the absent field, which means "leave the list alone". */
  test('removing everyone sends an empty list, not an absent one', () => {
    const value = initial();
    value.guests = [];
    expect(toEventInput(value, initial()).guests).toEqual([]);
  });

  test('a create sends the guests it opened with — the paste shape', () => {
    // The diff rule reads "unchanged" as "leave the list alone", which is a
    // sentence only an edit can say: a create has no server-side list, so an
    // absent field means *nobody*. Invisible while the only route for guests
    // into a create's `initial` was typing them (typed guests differ from the
    // blank list either way), and found the day paste shipped (2026-08-20):
    // a pasted form opens with the copied invitees already in `initial`,
    // untouched, and every pasted event arrived guestless.
    const pasted = pastedValue(initial(), blankValueAt(Date.UTC(2026, 8, 10, 9, 0), 1));
    expect(pasted.isEdit).toBe(false);
    // Ana only: the copier's own row is dropped by `pastedValue` — the
    // organizer of the new event is not a guest of it.
    expect(toEventInput(pasted, pasted).guests).toEqual([
      { email: 'ana@x.com', optional: false },
    ]);

    // An untouched empty list on a create still sends nothing — absent and
    // "nobody" coincide there, and no key is the smaller payload.
    const blank = blankValueAt(Date.UTC(2026, 8, 10, 9, 0), 1);
    expect(toEventInput(blank, blank).guests).toBeUndefined();
  });
});

// --- Video calls ----------------------------------------------------------

test.describe('video calls in the value and on the wire', () => {
  const opened = (detail: EventDetail) =>
    valueFromDetail(detail, detail.start_ms, detail.end_ms);

  test('reads structured Meet data separately from a physical location', () => {
    const value = opened({
      ...timedDetail(0, 30 * 60_000),
      location: 'Room 4',
      conference_uri: 'https://meet.google.com/abc-defg-hij',
    });
    expect(value.location).toBe('Room 4');
    expect(value.videoCall).toEqual({
      provider: 'googleMeet',
      uri: 'https://meet.google.com/abc-defg-hij',
      source: 'conference',
    });
  });

  test('recognises an existing Zoom link stored in Location', () => {
    const value = opened({
      ...timedDetail(0, 30 * 60_000),
      location: 'Board room · Zoom: https://us02web.zoom.us/j/123456789',
    });
    expect(value.videoCall).toEqual({
      provider: 'zoom',
      uri: 'https://us02web.zoom.us/j/123456789',
      source: 'location',
    });
  });

  test('a new Google Meet is a structured create request', () => {
    const initial = blankValueAt(Date.UTC(2026, 7, 25, 14), 1);
    const value = {
      ...initial,
      videoCall: { provider: 'googleMeet', uri: null, source: 'new' } as const,
    };
    expect(toEventInput(value, initial).conference).toBe('googleMeet');
    expect(toEventInput(value, initial).location).toBeNull();
    expect(videoCallProblem(value, 'google')).toBeNull();
    expect(videoCallProblem(value, 'caldav')).toContain('Google calendar');
  });

  test('an unchanged structured call is absent, while removing it sends null', () => {
    const initial = opened({
      ...timedDetail(0, 30 * 60_000),
      conference_uri: 'https://meet.google.com/abc-defg-hij',
    });
    expect(toEventInput(initial, initial).conference).toBeUndefined();

    const removed = { ...initial, videoCall: null };
    expect(toEventInput(removed, initial).conference).toBe('none');
  });

  test('replacing structured Meet with Zoom removes it and appends the Zoom link once', () => {
    const initial = opened({
      ...timedDetail(0, 30 * 60_000),
      location: 'Room 4',
      conference_uri: 'https://meet.google.com/abc-defg-hij',
    });
    const value: EventFormValue = {
      ...initial,
      videoCall: {
        provider: 'zoom', uri: 'https://zoom.us/j/987654321', source: 'new',
      },
    };
    const sent = toEventInput(value, initial);
    expect(sent.conference).toBe('none');
    expect(sent.location).toBe('Room 4 · Zoom: https://zoom.us/j/987654321');
    expect(locationForVideoCall(sent.location ?? '', value.videoCall, value.videoCall))
      .toBe(sent.location);
    expect(videoCallProblem(value, 'google')).toBeNull();
  });

  test('a quick-add Zoom seed still writes its URL on create', () => {
    const value = blankValueAt(Date.UTC(2026, 7, 25, 14), 1);
    value.videoCall = {
      provider: 'zoom', uri: 'https://zoom.us/j/987654321', source: 'new',
    };
    // Quick add can hand this populated value to Continue editing. On a create
    // there is no server-side before, even though `initial` is the same object.
    expect(toEventInput(value, value).location).toBe('Zoom: https://zoom.us/j/987654321');
  });

  test('a connected Zoom account becomes a create request, without a placeholder URL', () => {
    const value = blankValueAt(Date.UTC(2026, 7, 25, 14), 1);
    value.videoCall = { provider: 'zoom', uri: null, source: 'new' };
    expect(videoCallProblem(value, 'google', true)).toBeNull();
    expect(toEventInput(value, value).conference).toBe('zoom');
    expect(toEventInput(value, value).location).toBeNull();
  });

  test('automatic Zoom replaces structured conference data on an edit', () => {
    const initial = opened({
      ...timedDetail(0, 30 * 60_000),
      conference_uri: 'https://meet.google.com/abc-defg-hij',
    });
    const value: EventFormValue = {
      ...initial,
      videoCall: { provider: 'zoom', uri: null, source: 'new' },
    };
    expect(toEventInput(value, initial).conference).toBe('zoom');
  });

  test('validates Zoom links and compares conferencing by meaning, not source', () => {
    const value = blankValueAt(Date.UTC(2026, 7, 25, 14), 1);
    value.videoCall = { provider: 'zoom', uri: null, source: 'new' };
    expect(videoCallProblem(value, 'google')).toContain('Connect Zoom');
    expect(videoCallProblem(value, 'google', true)).toBeNull();
    value.isAllDay = true;
    expect(videoCallProblem(value, 'google', true)).toContain('need a start time');
    value.isAllDay = false;
    value.videoCall = { provider: 'zoom', uri: 'https://example.com/room', source: 'new' };
    expect(videoCallProblem(value, 'google')).toContain('not a zoom.us');
    expect(sameVideoCall(
      { provider: 'zoom', uri: 'https://zoom.us/j/1', source: 'conference' },
      { provider: 'zoom', uri: 'https://zoom.us/j/1', source: 'location' },
    )).toBe(true);
  });
});

// --- Reminders (reminders spec §§1–3) --------------------------------------

test.describe('reminders in the value and on the wire', () => {
  /** A timed detail whose reminder settings are the case under test. */
  const withReminders = (
    reminders: EventDetail['reminders'],
    defaults: EventDetail['calendar_default_reminders'] = [],
  ): EventDetail => ({
    ...timedDetail(1_785_398_400_000, 1_785_400_200_000),
    reminders,
    calendar_default_reminders: defaults,
  });

  const opened = (d: EventDetail) => valueFromDetail(d, d.start_ms, d.end_ms);

  test('an event with overrides opens showing its popup rows, emails carried unseen', () => {
    const value = opened(withReminders({
      use_default: false,
      overrides: [
        { method: 'popup', minutes: 15 },
        { method: 'email', minutes: 1440 },
        { method: 'popup', minutes: 120 },
      ],
    }));
    expect(value.popupReminders).toEqual([15, 120]);
    expect(value.emailReminders).toEqual([{ method: 'email', minutes: 1440 }]);
    expect(value.remindersWereDefault).toBe(false);
  });

  test('an event on calendar defaults opens showing those rows', () => {
    const value = opened(withReminders(
      { use_default: true, overrides: [] },
      [{ method: 'popup', minutes: 30 }, { method: 'email', minutes: 60 }],
    ));
    expect(value.popupReminders).toEqual([30]);
    // Seeded from the defaults, so flipping this event to explicit overrides
    // does not silently drop a default email reminder.
    expect(value.emailReminders).toEqual([{ method: 'email', minutes: 60 }]);
    expect(value.remindersWereDefault).toBe(true);
  });

  test('reminders nobody touched send no reminders at all', () => {
    const d = withReminders({
      use_default: false,
      overrides: [{ method: 'popup', minutes: 10 }],
    });
    expect('reminders' in toEventInput(opened(d), opened(d))).toBe(false);
  });

  /** The rows are a set as far as meaning goes: an order Google happens to
   *  permute must not read as an edit, or a title-only save would freeze an
   *  event's "calendar defaults" into copies of them. */
  test('a reordered list is not a change', () => {
    const d = withReminders({
      use_default: false,
      overrides: [{ method: 'popup', minutes: 10 }, { method: 'popup', minutes: 60 }],
    });
    const value = opened(d);
    value.popupReminders = [60, 10];
    expect('reminders' in toEventInput(value, opened(d))).toBe(false);
  });

  test('an added row sends the whole object, preserved emails included', () => {
    const d = withReminders({
      use_default: false,
      overrides: [{ method: 'popup', minutes: 10 }, { method: 'email', minutes: 1440 }],
    });
    const value = opened(d);
    value.popupReminders = [...value.popupReminders, 15];
    expect(toEventInput(value, opened(d)).reminders).toEqual({
      useDefault: false,
      overrides: [
        { method: 'popup', minutes: 10 },
        { method: 'popup', minutes: 15 },
        { method: 'email', minutes: 1440 },
      ],
    });
  });

  test('a row added on a create sends explicit overrides', () => {
    const initial = blankValueAt(1_785_398_400_000, 1);
    const value = { ...initial, popupReminders: [15] };
    expect(toEventInput(value, initial).reminders).toEqual({
      useDefault: false,
      overrides: [{ method: 'popup', minutes: 15 }],
    });
  });

  /** Removing every row is a change like any other — `overrides: []` with
   *  `useDefault: false` is "no reminders", distinct from the absent field,
   *  which means "leave them alone" (spec §2). */
  test('removing every row sends none, not absence', () => {
    const d = withReminders({
      use_default: false,
      overrides: [{ method: 'popup', minutes: 10 }],
    });
    const value = opened(d);
    value.popupReminders = [];
    expect(toEventInput(value, opened(d)).reminders).toEqual({
      useDefault: false,
      overrides: [],
    });
  });
});
