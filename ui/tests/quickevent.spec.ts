import { expect, test } from '@playwright/test';
import type { Calendar } from '../src/lib/calendars';
import {
  parseQuickEvent, quickPreviewRows, type QuickEventContext,
} from '../src/lib/quickevent';
import { toEventInput } from '../src/lib/eventform';

const at = (y: number, m: number, d: number, h = 0, minute = 0) =>
  new Date(y, m - 1, d, h, minute).getTime();

const calendar = (
  id: number,
  summary: string,
  provider = 'google',
  account = 'me@example.com',
): Calendar => ({
  id, account_id: id, account_email: account, summary,
  color_hex: null, color_override: null, selected: true, sync_enabled: true,
  is_primary: id === 1, access_role: 'owner', provider,
});

const calendars = [
  calendar(1, 'Personal'),
  calendar(2, 'Work', 'google', 'work@example.com'),
  calendar(3, 'Home', 'caldav', 'home@example.com'),
];

const context = (overrides: Partial<QuickEventContext> = {}): QuickEventContext => ({
  nowMs: at(2026, 8, 25, 10, 10), // Tuesday
  anchorDayMs: at(2026, 8, 25),
  calendarId: 1,
  defaultDurationMinutes: 60,
  calendars,
  dateOrder: 'mdy',
  ...overrides,
});

function permutations<T>(items: T[]): T[][] {
  if (items.length < 2) return [items];
  return items.flatMap((item, i) =>
    permutations([...items.slice(0, i), ...items.slice(i + 1)]).map((tail) => [item, ...tail]));
}

test.describe('quick event natural language', () => {
  test('extracts duration, start, title and guest in every ordering', () => {
    const parts = ['30 min', 'at 2pm', 'Meet with Tim', 'invite tim@example.com'];
    for (const words of permutations(parts)) {
      const parsed = parseQuickEvent(words.join(' '), context());
      expect(parsed.errors, words.join(' | ')).toEqual([]);
      expect(parsed.value.title, words.join(' | ')).toBe('Meet with Tim');
      expect(parsed.value.date).toBe('2026-08-25');
      expect(parsed.value.start).toBe('14:00');
      expect(parsed.value.end).toBe('14:30');
      expect(parsed.value.guests).toEqual([{ email: 'tim@example.com', optional: false }]);
    }
  });

  test('accepts compact clocks, duration abbreviations and flexible spacing', () => {
    for (const line of [
      '30m@2p Meet Tim',
      'Meet Tim 30min at2pm',
      'Meet Tim at 2 p.m. for .5h',
      '2:00pm Meet Tim 30 mins',
      'Meet Tim @ 1400 for 30m',
    ]) {
      const parsed = parseQuickEvent(line, context());
      expect(parsed.errors, line).toEqual([]);
      expect(parsed.value.start, line).toBe('14:00');
      expect(parsed.value.end, line).toBe('14:30');
      expect(parsed.value.title, line).toBe('Meet Tim');
    }
  });

  test('supports time ranges and propagates a meridiem across the range', () => {
    const afternoon = parseQuickEvent('Review 2-3:30pm tomorrow', context());
    expect(afternoon.errors).toEqual([]);
    expect(afternoon.value.date).toBe('2026-08-26');
    expect(afternoon.value.start).toBe('14:00');
    expect(afternoon.value.end).toBe('15:30');
    expect(afternoon.warnings).toEqual([]);

    const twentyFour = parseQuickEvent('Review from 14:00 to 15:30', context());
    expect(twentyFour.value.start).toBe('14:00');
    expect(twentyFour.value.end).toBe('15:30');

    const overnight = parseQuickEvent('Deploy 11pm–1am', context());
    expect(overnight.value.start).toBe('23:00');
    expect(overnight.value.end).toBe('01:00');
    expect(overnight.value.endDate).toBe('2026-08-26');
  });

  test('chooses the conventional short span for compact ambiguous ranges', () => {
    const cases: Array<[string, string, string, string]> = [
      ['Review 11-1pm', '11:00', '13:00', '2026-08-25'],
      ['Deploy 11pm-1', '23:00', '01:00', '2026-08-26'],
      ['Workout 6-7', '18:00', '19:00', '2026-08-25'],
    ];
    for (const [line, start, end, endDate] of cases) {
      const parsed = parseQuickEvent(line, context());
      expect(parsed.errors, line).toEqual([]);
      expect(parsed.value.start, line).toBe(start);
      expect(parsed.value.end, line).toBe(end);
      expect(parsed.value.endDate, line).toBe(endDate);
    }
    expect(parseQuickEvent('Workout 6-7', context()).warnings.join(' '))
      .toContain('Add am/pm');
  });

  test('keeps claims aligned when emoji precede parsed facts', () => {
    const parsed = parseQuickEvent(
      '🗓️ Meet Tim 30m at 2p invite tim@example.com', context(),
    );
    expect(parsed.errors).toEqual([]);
    expect(parsed.value.title).toBe('🗓️ Meet Tim');
    expect(parsed.value.start).toBe('14:00');
    expect(parsed.value.end).toBe('14:30');
    expect(parsed.value.guests).toEqual([
      { email: 'tim@example.com', optional: false },
    ]);
  });

  test('understands relative, weekday, named, ISO, numeric and ordinal dates', () => {
    const cases: Array<[string, string]> = [
      ['tomorrow 2p Call', '2026-08-26'],
      ['Fri 2p Call', '2026-08-28'],
      ['next Tue 2p Call', '2026-09-01'],
      ['2026-09-04 2p Call', '2026-09-04'],
      ['Sep 4 2p Call', '2026-09-04'],
      ['4 Sept 2026 2p Call', '2026-09-04'],
      ['9/4 2p Call', '2026-09-04'],
      ['on the 4th 2p Call', '2026-09-04'],
    ];
    for (const [line, wanted] of cases) {
      const parsed = parseQuickEvent(line, context());
      expect(parsed.errors, line).toEqual([]);
      expect(parsed.value.date, line).toBe(wanted);
      expect(parsed.value.title, line).toBe('Call');
    }
  });

  test('uses the viewed day and normal default duration when omitted', () => {
    const parsed = parseQuickEvent('Write release notes', context({
      anchorDayMs: at(2026, 9, 10), defaultDurationMinutes: 45,
    }));
    expect(parsed.ready).toBe(true);
    expect(parsed.explicitDate).toBe(false);
    expect(parsed.explicitTime).toBe(false);
    expect(parsed.value.date).toBe('2026-09-10');
    expect(at(2026, 9, 10, Number(parsed.value.end.slice(0, 2)), Number(parsed.value.end.slice(3)))
      - at(2026, 9, 10, Number(parsed.value.start.slice(0, 2)), Number(parsed.value.start.slice(3))))
      .toBe(45 * 60_000);
  });

  test('calls out an inferred bare clock rather than hiding the guess', () => {
    const parsed = parseQuickEvent('at 2 Call Tim', context());
    expect(parsed.value.start).toBe('14:00');
    expect(parsed.warnings.join(' ')).toContain('Add am/pm');
  });

  test('extracts, deduplicates and marks optional email guests', () => {
    const parsed = parseQuickEvent(
      'Review invite ANA@example.com optional:bob@example.com ana@example.com 2p', context(),
    );
    expect(parsed.value.title).toBe('Review');
    expect(parsed.value.guests).toEqual([
      { email: 'ANA@example.com', optional: false },
      { email: 'bob@example.com', optional: true },
    ]);
    const wire = toEventInput(parsed.value, parsed.baseline);
    expect(wire.guests).toEqual(parsed.value.guests);
  });

  test('does not leave invitation connectors behind in the title', () => {
    const parsed = parseQuickEvent(
      'Meet with ana@example.com and bo@example.com tomorrow at 2p', context(),
    );
    expect(parsed.value.title).toBe('Meet');
    expect(parsed.value.guests.map((g) => g.email)).toEqual([
      'ana@example.com', 'bo@example.com',
    ]);
  });

  test('recognises recurrence without confusing the title word Meet for Google Meet', () => {
    const parsed = parseQuickEvent('weekly Tue 9a Meet with Tim', context());
    expect(parsed.value.repeat).toBe('weekly');
    expect(parsed.value.weeklyDays).toEqual(['TU']);
    expect(parsed.value.date).toBe('2026-08-25');
    expect(parsed.value.title).toBe('Meet with Tim');
    expect(parsed.value.videoCall).toBeNull();
    expect(toEventInput(parsed.value, parsed.baseline).repeat).toBe('weekly');
    expect(toEventInput(parsed.value, parsed.baseline).weeklyDays).toEqual(['TU']);
  });

  test('turns the SMTWRFS shorthand into a custom weekly cadence', () => {
    const parsed = parseQuickEvent('MWF at 9am', context());
    expect(parsed.errors).toEqual([]);
    expect(parsed.value.title).toBe('');
    expect(parsed.value.repeat).toBe('weekly');
    expect(parsed.value.weeklyDays).toEqual(['MO', 'WE', 'FR']);
    // Tuesday is not in MWF, so DTSTART advances to the next real occurrence.
    expect(parsed.value.date).toBe('2026-08-26');
    expect(parsed.value.start).toBe('09:00');
    expect(toEventInput(parsed.value, parsed.baseline)).toMatchObject({
      repeat: 'weekly', weeklyDays: ['MO', 'WE', 'FR'],
    });
  });

  test('accepts common compact, spaced and named weekday patterns', () => {
    const cases: Array<[string, string[]]> = [
      ['TTh 9am Sync', ['TU', 'TH']],
      ['TuTh 9am Sync', ['TU', 'TH']],
      ['TR 9am Sync', ['TU', 'TH']],
      ['M/W/F 9am Sync', ['MO', 'WE', 'FR']],
      ['M W F 9am Sync', ['MO', 'WE', 'FR']],
      ['Mon Wed Fri 9am Sync', ['MO', 'WE', 'FR']],
      ['every Fri, Mon, Wed 9am Sync', ['MO', 'WE', 'FR']],
      ['SMTWRFS 9am Sync', ['SU', 'MO', 'TU', 'WE', 'TH', 'FR', 'SA']],
    ];
    for (const [line, days] of cases) {
      const parsed = parseQuickEvent(line, context());
      expect(parsed.errors, line).toEqual([]);
      expect(parsed.value.repeat, line).toBe('weekly');
      expect(parsed.value.weeklyDays, line).toEqual(days);
      expect(parsed.value.title, line).toBe('Sync');
    }
  });

  test('finds a weekly pattern in every ordering of the other facts', () => {
    const parts = ['MWF', 'at 9am', 'Team sync', 'invite team@example.com'];
    for (const words of permutations(parts)) {
      const parsed = parseQuickEvent(words.join(' '), context());
      expect(parsed.errors, words.join(' | ')).toEqual([]);
      expect(parsed.value.title, words.join(' | ')).toBe('Team sync');
      expect(parsed.value.weeklyDays, words.join(' | ')).toEqual(['MO', 'WE', 'FR']);
      expect(parsed.value.guests, words.join(' | ')).toEqual([
        { email: 'team@example.com', optional: false },
      ]);
    }
  });

  test('understands end-on and end-after recurrence phrases in any position', () => {
    for (const words of permutations(['MWF', '9am', 'Team sync', 'until Sep 30'])) {
      const parsed = parseQuickEvent(words.join(' '), context());
      expect(parsed.errors, words.join(' | ')).toEqual([]);
      expect(parsed.value.title, words.join(' | ')).toBe('Team sync');
      expect(parsed.value.repeatEnd, words.join(' | ')).toEqual({
        kind: 'on', date: '2026-09-30',
      });
      expect(toEventInput(parsed.value, parsed.baseline).repeatEnd).toEqual({
        kind: 'on', date: '2026-09-30',
      });
    }

    for (const phrase of ['for 8 times', 'for 8 occurrences', 'ends after 8 events']) {
      const parsed = parseQuickEvent(`TTh 10am Office hours ${phrase}`, context());
      expect(parsed.errors, phrase).toEqual([]);
      expect(parsed.value.repeatEnd, phrase).toEqual({ kind: 'after', count: 8 });
      expect(parsed.value.title, phrase).toBe('Office hours');
    }
  });

  test('a repeat ending requires a cadence and validates its boundary', () => {
    expect(parseQuickEvent('9am Sync until Sep 30', context()).errors.join(' '))
      .toContain('repeat pattern');
    expect(parseQuickEvent('MWF 9am Sync end after 0 occurrences', context()).errors.join(' '))
      .toContain('valid repeat ending');
    expect(parseQuickEvent('MWF next Fri 9am Sync until tomorrow', context()).errors.join(' '))
      .toContain('cannot be before');
  });

  test('keeps non-cadence acronyms as titles and explains an ambiguous lone S', () => {
    const title = parseQuickEvent('WTF planning at 9am', context());
    expect(title.value.repeat).toBe('never');
    expect(title.value.title).toBe('WTF planning');

    const ambiguous = parseQuickEvent('every S at 9am Sync', context());
    expect(ambiguous.ready).toBe(false);
    expect(ambiguous.errors.join(' ')).toContain('Su or Sa');
  });

  test('understands video commands and real meeting URLs', () => {
    const meet = parseQuickEvent('30m 2p Meet Tim +meet', context());
    expect(meet.value.videoCall).toEqual({ provider: 'googleMeet', uri: null, source: 'new' });
    expect(toEventInput(meet.value, meet.baseline).conference).toBe('googleMeet');

    const zoom = parseQuickEvent(
      '30m 2p Meet Tim https://us02web.zoom.us/j/123456?pwd=x', context(),
    );
    expect(zoom.errors).toEqual([]);
    expect(zoom.value.videoCall?.provider).toBe('zoom');
    expect(toEventInput(zoom.value, zoom.baseline).location)
      .toBe('Zoom: https://us02web.zoom.us/j/123456?pwd=x');

    const needsLink = parseQuickEvent('30m 2p Meet Tim +zoom', context());
    expect(needsLink.ready).toBe(false);
    expect(needsLink.errors.join(' ')).toContain('Paste the Zoom meeting link');
  });

  test('supports quoted location and notes plus a named calendar', () => {
    const parsed = parseQuickEvent(
      '2p 45m Project kickoff loc:"Room 4 West" note:"Bring the draft" cal:Work', context(),
    );
    expect(parsed.errors).toEqual([]);
    expect(parsed.value.title).toBe('Project kickoff');
    expect(parsed.value.location).toBe('Room 4 West');
    expect(parsed.value.description).toBe('Bring the draft');
    expect(parsed.value.calendarId).toBe(2);
  });

  test('requires an account qualifier for duplicate calendar names', () => {
    const duplicates = [
      calendar(1, 'Work', 'google', 'one@example.com'),
      calendar(2, 'Work', 'google', 'two@example.com'),
    ];
    const ambiguous = parseQuickEvent('2p Call cal:Work', context({ calendars: duplicates }));
    expect(ambiguous.ready).toBe(false);
    expect(ambiguous.errors.join(' ')).toContain('More than one calendar');

    const qualified = parseQuickEvent(
      '2p Call calendar:"Work@two@example.com"', context({ calendars: duplicates }),
    );
    expect(qualified.errors).toEqual([]);
    expect(qualified.value.calendarId).toBe(2);
  });

  test('blocks conflicting facts and provider-incompatible actions', () => {
    expect(parseQuickEvent('today tomorrow 2p Call', context()).errors.join(' '))
      .toContain('more than one date');
    expect(parseQuickEvent('2p 3p Call', context()).errors.join(' '))
      .toContain('more than one start time');
    expect(parseQuickEvent('2p Call +meet +zoom', context()).errors.join(' '))
      .toContain('both Zoom and Google Meet');
    expect(parseQuickEvent('2p Call invite a@example.com cal:Home', context()).errors.join(' '))
      .toContain('Email invitations require a Google calendar');
    expect(parseQuickEvent('2p Call +meet cal:Home', context()).errors.join(' '))
      .toContain('Google Meet can only be added');
  });

  test('preview rows state the non-editable interpretation and invite effect', () => {
    const parsed = parseQuickEvent('30m 2p Review invite tim@example.com +meet', context());
    const rows = quickPreviewRows(parsed, calendars);
    expect(rows.find((r) => r.label === 'Title')?.value).toBe('Review');
    expect(rows.find((r) => r.label === 'Video')?.value).toBe('Google Meet');
    expect(rows.find((r) => r.label === 'Guests')?.value).toContain('invitations will be emailed');
  });

  test('preview names the weekly days instead of only saying Weekly', () => {
    const parsed = parseQuickEvent('MWF 9am Team sync for 8 occurrences', context());
    expect(quickPreviewRows(parsed, calendars).find((row) => row.label === 'Repeats')?.value)
      .toBe('Weekly · Mon, Wed, Fri');
    expect(quickPreviewRows(parsed, calendars).find((row) => row.label === 'Ends')?.value)
      .toBe('After 8 occurrences');
  });
});
