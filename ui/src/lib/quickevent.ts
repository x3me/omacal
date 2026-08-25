import type { Calendar } from './calendars';
import {
  WEEKDAY_OPTIONS, blankValue, blankValueAt, dateOf, normalizedWeeklyDays,
  repeatEndProblem, sameWeeklyDays, videoCallProblem,
  type EventFormValue, type RepeatEnd, type VideoCall, type WeekdayCode,
} from './eventform';
import { meetingProvider, meetingUrl } from './location';

/** The facts quick-add needs from App. `anchorDayMs` is the day currently on
 * screen; relative words such as “tomorrow” still use the real `nowMs`. */
export type QuickEventContext = {
  nowMs: number;
  anchorDayMs: number;
  calendarId: number | null;
  defaultDurationMinutes: number;
  calendars: Calendar[];
  /** Used only for ambiguous numeric dates such as 4/5. */
  dateOrder?: 'mdy' | 'dmy';
};

export type QuickIslandKind =
  | 'date' | 'time' | 'duration' | 'guest' | 'calendar'
  | 'repeat' | 'repeatEnd' | 'location' | 'description' | 'video' | 'allDay';

export type QuickIsland = { kind: QuickIslandKind; text: string };

export type QuickEventResult = {
  /** The ordinary create-form value represented by the line. */
  value: EventFormValue;
  /** The unmodified create seed. Useful when turning the value into a wire
   * input directly; creates still send populated repeat/video fields. */
  baseline: EventFormValue;
  islands: QuickIsland[];
  warnings: string[];
  errors: string[];
  ready: boolean;
  /** Whether the line explicitly named a start, rather than accepting the
   * normal next-half-hour default. */
  explicitTime: boolean;
  /** Whether the line explicitly named a date, rather than using the day on
   * screen. */
  explicitDate: boolean;
};

type DateParts = { year: number; month: number; day: number };
type Clock = { hour: number; minute: number; inferred: boolean };

const EMAIL_RE = /[A-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Z0-9](?:[A-Z0-9-]{0,61}[A-Z0-9])?(?:\.[A-Z0-9](?:[A-Z0-9-]{0,61}[A-Z0-9])?)+/gi;
const MONTHS: Record<string, number> = {
  jan: 1, january: 1, feb: 2, february: 2, mar: 3, march: 3,
  apr: 4, april: 4, may: 5, jun: 6, june: 6, jul: 7, july: 7,
  aug: 8, august: 8, sep: 9, sept: 9, september: 9, oct: 10,
  october: 10, nov: 11, november: 11, dec: 12, december: 12,
};
const MONTH_WORD = Object.keys(MONTHS).sort((a, b) => b.length - a.length).join('|');
const WEEKDAYS: Record<string, number> = {
  sun: 0, sunday: 0, mon: 1, monday: 1, tue: 2, tues: 2, tuesday: 2,
  wed: 3, weds: 3, wednesday: 3, thu: 4, thur: 4, thurs: 4,
  thursday: 4, fri: 5, friday: 5, sat: 6, saturday: 6,
};
const WEEKDAY_WORD = Object.keys(WEEKDAYS).sort((a, b) => b.length - a.length).join('|');
const WEEKDAY_NAME_TOKEN = '(?:sundays?|sun|su|mondays?|mon|mo|tuesdays?|tues|tue|tu|'
  + 'wednesdays?|weds|wed|we|thursdays?|thurs|thur|thu|th|fridays?|fri|fr|'
  + 'saturdays?|sat|sa|[smtwrf])\\.?';
const WEEKDAY_LIST_SEPARATOR = '(?:\\s*(?:,|/|&|\\+)\\s*|\\s+and\\s+|\\s+)';
const MERIDIEM = `[ap](?:\\.?m\\.?)?`;
const CLOCK_TOKEN = `(?:noon|midnight|(?:\\d{3,4})(?:\\s*(?:${MERIDIEM}))?|(?:[01]?\\d|2[0-3])(?:(?::|\\.)[0-5]\\d)?\\s*(?:${MERIDIEM})?)`;

/** A character-preserving mask. Extractors all scan the original line, while
 * `available` prevents a clock inside `note:"call at 2pm"` from becoming the
 * event time. At the end, exactly the unclaimed characters become the title. */
class Claims {
  readonly chars: string[];
  readonly used: boolean[];
  readonly islands: QuickIsland[] = [];

  constructor(readonly source: string) {
    // RegExp match indices are UTF-16 code-unit offsets. `split('')` uses the
    // same unit; spreading a string uses Unicode code points instead, so one
    // emoji before “2pm” shifted every later claim and left pieces of the
    // command in the title.
    this.chars = source.split('');
    this.used = this.chars.map(() => false);
  }

  available(start: number, end: number): boolean {
    for (let i = start; i < end; i += 1) if (this.used[i]) return false;
    return true;
  }

  claim(start: number, end: number, kind: QuickIslandKind): boolean {
    if (!this.available(start, end)) return false;
    for (let i = start; i < end; i += 1) this.used[i] = true;
    this.islands.push({ kind, text: this.source.slice(start, end).trim() });
    return true;
  }

  title(): string {
    const rest = this.chars.map((c, i) => (this.used[i] ? ' ' : c)).join('');
    return rest
      .replace(/\s+/g, ' ')
      .replace(/\s+([,;|])/g, '$1')
      .replace(/^[\s,;|·–—-]+|[\s,;|·–—-]+$/g, '')
      // Connectors that introduced a claimed address/value should not become a
      // title suffix: “Lunch with ana@x.com” is titled “Lunch”, not “Lunch with”.
      // The group repeats so “Meet with a@x.com and b@x.com” loses both
      // trailing connectors in one pass, rather than stopping at “Meet with”.
      .replace(/(?:\b(?:and|with|invite|invites|guest|guests|attendee|attendees|to)\b[\s,;|·–—-]*)+$/i, '')
      .replace(/[\s,;|·–—-]+$/g, '')
      .trim();
  }
}

function scan(re: RegExp, source: string, visit: (m: RegExpExecArray) => void) {
  re.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(source))) {
    visit(m);
    if (m[0].length === 0) re.lastIndex += 1;
  }
}

const pad = (n: number) => String(n).padStart(2, '0');
const ymd = (p: DateParts) => `${String(p.year).padStart(4, '0')}-${pad(p.month)}-${pad(p.day)}`;
const clockText = (c: Clock) => `${pad(c.hour)}:${pad(c.minute)}`;

function validDate(year: number, month: number, day: number): DateParts | null {
  if (year < 1 || year > 9999 || month < 1 || month > 12 || day < 1 || day > 31) return null;
  const d = new Date(year, month - 1, day, 12);
  return d.getFullYear() === year && d.getMonth() === month - 1 && d.getDate() === day
    ? { year, month, day }
    : null;
}

function partsOf(ms: number): DateParts {
  const d = new Date(ms);
  return { year: d.getFullYear(), month: d.getMonth() + 1, day: d.getDate() };
}

function futureDate(month: number, day: number, year: number | null, nowMs: number): DateParts | null {
  if (year !== null) return validDate(year, month, day);
  const now = new Date(nowMs);
  let candidate = validDate(now.getFullYear(), month, day);
  if (!candidate) return null;
  const atNoon = new Date(candidate.year, month - 1, day, 12).getTime();
  const todayNoon = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 12).getTime();
  if (atNoon < todayNoon) candidate = validDate(now.getFullYear() + 1, month, day);
  return candidate;
}

function addDays(p: DateParts, amount: number): DateParts {
  const d = new Date(p.year, p.month - 1, p.day + amount, 12);
  return partsOf(d.getTime());
}

function weekdayDate(nowMs: number, weekday: number, forceNext: boolean): DateParts {
  const today = new Date(nowMs);
  today.setHours(12, 0, 0, 0);
  let delta = (weekday - today.getDay() + 7) % 7;
  if (forceNext && delta === 0) delta = 7;
  today.setDate(today.getDate() + delta);
  return partsOf(today.getTime());
}

const weekdayIndex = (code: WeekdayCode): number =>
  WEEKDAY_OPTIONS.findIndex((day) => day.code === code);

/** The nearest selected weekday, including today. A repeat pattern supplies
 * its own first valid date; leaving DTSTART on an unselected day would create
 * a stray first occurrence outside the cadence the preview promises. */
function weekdayPatternDate(nowMs: number, days: readonly WeekdayCode[]): DateParts {
  const today = new Date(nowMs).getDay();
  const first = Math.min(...days.map((day) => (weekdayIndex(day) - today + 7) % 7));
  return addDays(partsOf(nowMs), first);
}

const NAMED_DAY: Record<string, WeekdayCode> = {
  sunday: 'SU', sundays: 'SU', sun: 'SU', su: 'SU',
  monday: 'MO', mondays: 'MO', mon: 'MO', mo: 'MO', m: 'MO',
  tuesday: 'TU', tuesdays: 'TU', tues: 'TU', tue: 'TU', tu: 'TU', t: 'TU',
  wednesday: 'WE', wednesdays: 'WE', weds: 'WE', wed: 'WE', we: 'WE', w: 'WE',
  thursday: 'TH', thursdays: 'TH', thurs: 'TH', thur: 'TH', thu: 'TH', th: 'TH', r: 'TH',
  friday: 'FR', fridays: 'FR', fri: 'FR', fr: 'FR', f: 'FR',
  saturday: 'SA', saturdays: 'SA', sat: 'SA', sa: 'SA',
};

/** Parses concatenated weekday abbreviations in calendar order. Longest
 * aliases and backtracking make both `TTh` and `TuTh` work; the two meanings
 * of `S` resolve from position (`SM` is Sun/Mon, `FS` is Fri/Sat, `SS` is
 * Sun/Sat). More than one successful result is genuinely ambiguous. */
function compactWeekdays(raw: string): WeekdayCode[] | null {
  const text = raw.replace(/[\s,\/&+.-]+/g, '').toLowerCase();
  if (text === '') return null;
  const aliases: Array<[string, WeekdayCode]> = [
    ...Object.entries(NAMED_DAY).map(([name, code]) => [name, code] as [string, WeekdayCode]),
    ['s', 'SU'] as [string, WeekdayCode],
    ['s', 'SA'] as [string, WeekdayCode],
  ].sort((a, b) => b[0].length - a[0].length);
  const answers = new Map<string, WeekdayCode[]>();
  const walk = (offset: number, last: number, days: WeekdayCode[]) => {
    if (offset === text.length) {
      if (days.length > 0) answers.set(days.join(','), days);
      return;
    }
    if (days.length === 7) return;
    for (const [alias, code] of aliases) {
      const index = weekdayIndex(code);
      if (index <= last || !text.startsWith(alias, offset)) continue;
      walk(offset + alias.length, index, [...days, code]);
    }
  };
  walk(0, -1, []);
  return answers.size === 1 ? [...answers.values()][0] : null;
}

/** Named/space-separated lists may arrive in any order (`Fri Mon Wed`) and
 * are sorted for the wire. A list made entirely from the SMTWRFS button
 * letters goes through the compact parser so its two S positions stay exact. */
function weekdayExpression(raw: string): WeekdayCode[] | null {
  const tokens = raw
    .replace(/\band\b/ig, ' ')
    .split(/[\s,\/&+]+/)
    .map((token) => token.replace(/\.$/, ''))
    .filter(Boolean);
  if (tokens.length === 0) return null;
  if (tokens.every((token) => /^[SMTWRF]$/i.test(token))) {
    return compactWeekdays(tokens.join(''));
  }
  if (tokens.length === 1) return compactWeekdays(tokens[0]);
  const days: WeekdayCode[] = [];
  for (const token of tokens) {
    const code = NAMED_DAY[token.toLowerCase()];
    if (!code) return null;
    days.push(code);
  }
  const answer = normalizedWeeklyDays(days);
  return answer.length > 0 ? answer : null;
}

function inferredDateOrder(): 'mdy' | 'dmy' {
  try {
    const first = new Intl.DateTimeFormat(undefined).formatToParts(new Date(2001, 10, 22))
      .find((p) => p.type === 'month' || p.type === 'day')?.type;
    return first === 'day' ? 'dmy' : 'mdy';
  } catch {
    return 'mdy';
  }
}

function yearOf(raw: string | undefined): number | null {
  if (!raw) return null;
  const n = Number(raw);
  return raw.length === 2 ? 2000 + n : n;
}

function suffixOf(raw: string): 'a' | 'p' | null {
  const compact = raw.toLowerCase().replace(/[.\s]/g, '');
  const m = compact.match(/([ap])m?$/);
  return (m?.[1] as 'a' | 'p' | undefined) ?? null;
}

function parseClock(raw: string, inherited: 'a' | 'p' | null = null, allowBare = false): Clock | null {
  const compact = raw.toLowerCase().replace(/[.\s]/g, '');
  if (compact === 'noon') return { hour: 12, minute: 0, inferred: false };
  if (compact === 'midnight') return { hour: 0, minute: 0, inferred: false };

  const ownSuffix = suffixOf(raw);
  const suffix = ownSuffix ?? inherited;
  const body = compact.replace(/[ap]m?$/, '');
  let hour: number;
  let minute: number;
  if (body.includes(':')) {
    [hour, minute] = body.split(':').map(Number);
  } else if (body.length >= 3) {
    hour = Number(body.slice(0, -2));
    minute = Number(body.slice(-2));
  } else {
    hour = Number(body);
    minute = 0;
  }
  if (!Number.isInteger(hour) || !Number.isInteger(minute) || minute > 59) return null;
  if (suffix) {
    if (hour < 1 || hour > 12) return null;
    if (suffix === 'a') hour = hour === 12 ? 0 : hour;
    else hour = hour === 12 ? 12 : hour + 12;
    // A range conventionally shares one suffix across both ends
    // (“2–3:30pm”). That suffix is explicit information, not a guess. Only a
    // truly bare clock such as “at 2” earns the visible inference warning.
    return { hour, minute, inferred: ownSuffix === null && inherited === null };
  }
  if (body.includes(':') || body.length >= 3) {
    return hour <= 23 ? { hour, minute, inferred: false } : null;
  }
  if (!allowBare || hour < 1 || hour > 12) return null;
  // A visible, documented convenience rather than a silent guess: the caller
  // adds a warning for it. Early single-digit hours read as afternoon; 7–11 as
  // morning, and 12 as noon.
  return { hour: hour <= 6 ? hour + 12 : hour, minute, inferred: true };
}

const clockMinutes = (clock: Clock) => clock.hour * 60 + clock.minute;

/** Minutes moving forward from `start` to `end`; equal clocks mean a full
 * day, never a zero-length event. */
function forwardMinutes(start: Clock, end: Clock): number {
  const delta = (clockMinutes(end) - clockMinutes(start) + 24 * 60) % (24 * 60);
  return delta === 0 ? 24 * 60 : delta;
}

/** Parses a compact range by considering both meridiems only for genuinely
 * bare one/two-digit clocks. The shortest forward span wins, with the normal
 * bare-clock convention as the tie-breaker. This makes common forms read as
 * people intend: `11–1pm` is 11am–1pm, `11pm–1` ends at 1am, and `6–7` is a
 * one-hour evening event rather than thirteen hours overnight. */
function parseClockRange(first: string, second: string): [Clock | null, Clock | null] {
  const firstSuffix = suffixOf(first);
  const secondSuffix = suffixOf(second);
  const bare = (raw: string) => /^\s*\d{1,2}\s*$/.test(raw);
  const choices = (raw: string, suffix: 'a' | 'p' | null): Array<'a' | 'p' | null> =>
    suffix ? [suffix] : bare(raw) ? ['a', 'p'] : [null];
  const preferredStart = parseClock(first, null, true);
  const preferredEnd = parseClock(second, null, true);
  const candidates: Array<{ start: Clock; end: Clock; span: number; tie: number }> = [];

  for (const firstChoice of choices(first, firstSuffix)) {
    for (const secondChoice of choices(second, secondSuffix)) {
      const start = parseClock(first, firstChoice, true);
      const end = parseClock(second, secondChoice, true);
      if (!start || !end) continue;
      const tie = (start.hour === preferredStart?.hour ? 0 : 2)
        + (end.hour === preferredEnd?.hour ? 0 : 1);
      candidates.push({ start, end, span: forwardMinutes(start, end), tie });
    }
  }
  candidates.sort((a, b) => a.span - b.span || a.tie - b.tie);
  const chosen = candidates[0];
  if (!chosen) return [null, null];

  // A suffix on either end conventionally governs the compact range, so it
  // is explicit enough not to nag. With no suffix, keep the existing visible
  // warning that tells the user which of the possible clocks was chosen.
  if (!firstSuffix && !secondSuffix) {
    if (bare(first)) chosen.start.inferred = true;
    if (bare(second)) chosen.end.inferred = true;
  }
  return [chosen.start, chosen.end];
}

function localInstant(date: DateParts, clock: Clock): number | null {
  const d = new Date(date.year, date.month - 1, date.day, clock.hour, clock.minute, 0, 0);
  // Date normalises a clock skipped by daylight saving. That is useful for
  // arithmetic and dangerous for creation, where 02:30 becoming 03:30 would be
  // an unannounced change, so verify the civil fields survived.
  if (d.getFullYear() !== date.year || d.getMonth() !== date.month - 1 || d.getDate() !== date.day
      || d.getHours() !== clock.hour || d.getMinutes() !== clock.minute) return null;
  return d.getTime();
}

function directive(
  claims: Claims,
  names: string,
  kind: QuickIslandKind,
): { value: string; match: RegExpExecArray } | null {
  const re = new RegExp(`\\b(?:${names})\\s*:\\s*(?:"([^"]+)"|'([^']+)'|([^\\s,;|]+))`, 'ig');
  let answer: { value: string; match: RegExpExecArray } | null = null;
  scan(re, claims.source, (m) => {
    if (answer || !claims.claim(m.index, m.index + m[0].length, kind)) return;
    answer = { value: (m[1] ?? m[2] ?? m[3]).trim(), match: m };
  });
  return answer;
}

function resolveCalendar(raw: string, calendars: Calendar[]): { id: number | null; error: string | null } {
  const wanted = raw.trim().toLowerCase();
  const exact = calendars.filter((c) => c.summary.trim().toLowerCase() === wanted);
  if (exact.length === 1) return { id: exact[0].id, error: null };
  const qualified = calendars.filter((c) => {
    const names = [
      `${c.summary}@${c.account_email}`,
      `${c.summary} ${c.account_email}`,
      `${c.summary} · ${c.account_email}`,
    ].map((s) => s.toLowerCase());
    return names.includes(wanted);
  });
  if (qualified.length === 1) return { id: qualified[0].id, error: null };
  if (exact.length > 1) {
    return { id: null, error: `More than one calendar is named “${raw}”; add the account email.` };
  }
  return { id: null, error: `No calendar matches “${raw}”.` };
}

function videoFromProvider(provider: string, uri: string | null): VideoCall {
  return {
    provider: provider === 'Google Meet' ? 'googleMeet' : provider === 'Zoom' ? 'zoom' : 'other',
    uri,
    source: 'new',
  };
}

/** Parses one quick-add line. Extraction order is about protection, not word
 * order: quoted directives and URLs claim their text first, then every other
 * recogniser scans the whole line. “30m tomorrow Meet Tim at 2p” and “Meet Tim
 * at 2p tomorrow for 30m” therefore reach the same value. */
export function parseQuickEvent(source: string, context: QuickEventContext): QuickEventResult {
  const claims = new Claims(source);
  const warnings: string[] = [];
  const errors: string[] = [];
  const baseline = blankValue(context.nowMs, context.calendarId, context.anchorDayMs);

  let calendarId = context.calendarId;
  const cal = directive(claims, 'cal|calendar', 'calendar');
  if (cal) {
    const resolved = resolveCalendar(cal.value, context.calendars);
    calendarId = resolved.id;
    if (resolved.error) errors.push(resolved.error);
  }
  const location = directive(claims, 'loc|location|place', 'location')?.value ?? '';
  const description = directive(claims, 'note|notes|description|desc', 'description')?.value ?? '';

  // Recognised meeting URLs are commands in their own right. Ordinary URLs
  // are left unclaimed and remain part of the title unless put in loc:/note:.
  let videoCall: VideoCall | null = null;
  scan(/https?:\/\/[^\s,;|]+/ig, source, (m) => {
    if (!claims.available(m.index, m.index + m[0].length)) return;
    const uri = meetingUrl(m[0]);
    const provider = meetingProvider(uri);
    if (!uri || !provider) return;
    if (!claims.claim(m.index, m.index + m[0].length, 'video')) return;
    const candidate = videoFromProvider(provider, uri);
    if (videoCall && videoCall.provider !== candidate.provider) {
      errors.push('The line asks for more than one video-call provider.');
    } else {
      videoCall = candidate;
    }
  });

  const videoCommand = /(?:\+\s*(google\s*meet|gmeet|meet|zoom)\b|\bvideo\s*:\s*(google\s*meet|gmeet|meet|zoom)\b|\bmake\s+it\s+(?:a\s+)?(google\s*meet|gmeet|zoom)\b|\badd\s+(?:a\s+)?(google\s*meet|gmeet|zoom)(?:\s+(?:call|link))?\b|\b(google\s*meet|gmeet)\b)/ig;
  scan(videoCommand, source, (m) => {
    if (!claims.available(m.index, m.index + m[0].length)) return;
    const word = (m.slice(1).find(Boolean) ?? '').toLowerCase();
    const provider = word.includes('zoom') ? 'Zoom' : 'Google Meet';
    if (!claims.claim(m.index, m.index + m[0].length, 'video')) return;
    const candidate = videoFromProvider(provider, null);
    if (videoCall?.uri && videoCall.provider === candidate.provider) return;
    if (videoCall && videoCall.provider !== candidate.provider) {
      errors.push('The line asks for both Zoom and Google Meet.');
    } else {
      videoCall = candidate;
    }
  });

  const guests: { email: string; optional: boolean }[] = [];
  const seenGuests = new Set<string>();
  scan(EMAIL_RE, source, (m) => {
    if (!claims.available(m.index, m.index + m[0].length)) return;
    const before = source.slice(Math.max(0, m.index - 24), m.index);
    const optionalMatch = before.match(/(?:optional|opt|cc)\s*:\s*$/i);
    const inviteMatch = before.match(/(?:invite|invites|guest|guests|attendee|attendees)\s*[:=]?\s*$/i);
    const prefix = optionalMatch ?? inviteMatch;
    const start = prefix ? m.index - prefix[0].length : m.index;
    if (!claims.claim(start, m.index + m[0].length, 'guest')) return;
    const key = m[0].toLowerCase();
    if (seenGuests.has(key)) return;
    seenGuests.add(key);
    guests.push({ email: m[0], optional: !!optionalMatch });
  });

  let repeat = 'never';
  let weeklyDays: WeekdayCode[] | null = null;
  const chooseWeeklyPattern = (days: WeekdayCode[] | null, m: RegExpExecArray) => {
    if (!claims.claim(m.index, m.index + m[0].length, 'repeat')) return;
    if (!days || days.length === 0) {
      errors.push('Use Su or Sa when a single S could mean Sunday or Saturday.');
      return;
    }
    const normalized = normalizedWeeklyDays(days);
    if (repeat !== 'never'
        && (repeat !== 'weekly' || weeklyDays === null
          || !sameWeeklyDays(weeklyDays, normalized))) {
      errors.push('The line names more than one repeat pattern.');
      return;
    }
    repeat = 'weekly';
    weeklyDays = normalized;
  };

  // Explicit forms may name one day; bare lists need two, so an ordinary
  // `Tue 9am` remains a date while `weekly Tue` and `Mon Wed Fri` are cadences.
  const explicitDayList = new RegExp(
    `\\b(?:every|weekly|repeat\\s*:\\s*)\\s*(${WEEKDAY_NAME_TOKEN}`
      + `(?:${WEEKDAY_LIST_SEPARATOR}${WEEKDAY_NAME_TOKEN})*)(?![A-Za-z])`,
    'ig',
  );
  scan(explicitDayList, source, (m) => chooseWeeklyPattern(weekdayExpression(m[1]), m));

  const bareDayList = new RegExp(
    `\\b(${WEEKDAY_NAME_TOKEN}(?:${WEEKDAY_LIST_SEPARATOR}${WEEKDAY_NAME_TOKEN})+)(?![A-Za-z])`,
    'ig',
  );
  scan(bareDayList, source, (m) => {
    if (!claims.available(m.index, m.index + m[0].length)) return;
    const days = weekdayExpression(m[1]);
    if (days) chooseWeeklyPattern(days, m);
  });

  // A prefix makes casing irrelevant (`every mwf`). Without one, only the
  // conventional visual codes are claimed, which keeps ordinary lowercase
  // words out of recurrence while accepting MWF, TTh, TuTh and MonWedFri.
  scan(/\b(?:every|weekly|repeat\s*:\s*)\s*([A-Za-z]{1,24})\b/ig, source, (m) => {
    if (!claims.available(m.index, m.index + m[0].length)) return;
    const days = compactWeekdays(m[1]);
    if (days) chooseWeeklyPattern(days, m);
  });
  scan(/\b[A-Za-z]{2,24}\b/g, source, (m) => {
    if (!claims.available(m.index, m.index + m[0].length)) return;
    const conventional = /^[SMTWRFS]{2,7}$/.test(m[0])
      || /^(?:Su|Mo|Tu|We|Th|Fr|Sa|[SMTWRFS]){2,}$/.test(m[0])
      || /^(?:Sun|Mon|Tue|Wed|Thu|Fri|Sat){2,}$/.test(m[0]);
    if (!conventional) return;
    const days = compactWeekdays(m[0]);
    if (days && days.length >= 2) chooseWeeklyPattern(days, m);
  });

  const recurrence = /\b(?:repeat\s*:\s*)?(every\s+weekday|every\s+day|every\s+week|every\s+month|every\s+year|weekdays?|daily|dly|weekly|wkly|monthly|mthly|yearly|yrly|annually)\b/ig;
  scan(recurrence, source, (m) => {
    if (!claims.claim(m.index, m.index + m[0].length, 'repeat')) return;
    const word = m[1].toLowerCase().replace(/\s+/g, ' ');
    const next = word.includes('weekday') ? 'weekdays'
      : word.includes('day') || word === 'dly' ? 'daily'
        : word.includes('week') || word === 'wkly' ? 'weekly'
          : word.includes('month') || word === 'mthly' ? 'monthly' : 'yearly';
    if (repeat !== 'never' && repeat !== next) {
      errors.push('The line names more than one repeat pattern.');
    } else repeat = next;
  });

  let repeatEnd: RepeatEnd = { kind: 'never' };
  const chooseRepeatEnd = (candidate: RepeatEnd | null, m: RegExpExecArray) => {
    if (!claims.claim(m.index, m.index + m[0].length, 'repeatEnd')) return;
    if (!candidate) {
      errors.push(`“${m[0].trim()}” is not a valid repeat ending.`);
      return;
    }
    if (repeatEnd.kind !== 'never') {
      errors.push('The line names more than one repeat ending.');
      return;
    }
    repeatEnd = candidate;
  };

  scan(/\b(?:end(?:s|ing)?\s+after|for)\s+(\d+)\s*(?:times?|occurrences?|events?)\b/ig, source, (m) => {
    const count = Number(m[1]);
    chooseRepeatEnd(Number.isSafeInteger(count) && count > 0 ? { kind: 'after', count } : null, m);
  });

  // Repeat-ending dates are claimed before ordinary event dates, so
  // “MWF until Sep 30” does not move the first occurrence to Sep 30. The
  // supported date vocabulary deliberately mirrors the high-value forms of
  // the ordinary date parser: relative, weekday, ISO, named and numeric.
  const END_PREFIX = '(?:ends?(?:\\s+on)?|ending\\s+on|until)';
  scan(new RegExp(`\\b${END_PREFIX}\\s+(today|tomorrow|tmr|tmrw)\\b`, 'ig'), source, (m) => {
    const base = partsOf(context.nowMs);
    chooseRepeatEnd({ kind: 'on', date: ymd(m[1].toLowerCase() === 'today' ? base : addDays(base, 1)) }, m);
  });
  scan(new RegExp(`\\b${END_PREFIX}\\s+(?:(next|this)\\s+)?(${WEEKDAY_WORD})\\b`, 'ig'), source, (m) => {
    const date = weekdayDate(
      context.nowMs, WEEKDAYS[m[2].toLowerCase()], m[1]?.toLowerCase() === 'next',
    );
    chooseRepeatEnd({ kind: 'on', date: ymd(date) }, m);
  });
  scan(new RegExp(`\\b${END_PREFIX}\\s+(\\d{4})-(\\d{1,2})-(\\d{1,2})\\b`, 'ig'), source, (m) => {
    const date = validDate(Number(m[1]), Number(m[2]), Number(m[3]));
    chooseRepeatEnd(date ? { kind: 'on', date: ymd(date) } : null, m);
  });
  scan(new RegExp(
    `\\b${END_PREFIX}\\s+(${MONTH_WORD})\\.?\\s+(\\d{1,2})(?:st|nd|rd|th)?(?:,?\\s+(\\d{4}))?\\b`,
    'ig',
  ), source, (m) => {
    const date = futureDate(MONTHS[m[1].toLowerCase()], Number(m[2]), yearOf(m[3]), context.nowMs);
    chooseRepeatEnd(date ? { kind: 'on', date: ymd(date) } : null, m);
  });
  scan(new RegExp(
    `\\b${END_PREFIX}\\s+(\\d{1,2})(?:st|nd|rd|th)?\\s+(${MONTH_WORD})\\.?(?:\\s+(\\d{4}))?\\b`,
    'ig',
  ), source, (m) => {
    const date = futureDate(MONTHS[m[2].toLowerCase()], Number(m[1]), yearOf(m[3]), context.nowMs);
    chooseRepeatEnd(date ? { kind: 'on', date: ymd(date) } : null, m);
  });
  scan(new RegExp(
    `\\b${END_PREFIX}\\s+(\\d{1,2})\\s*\\/\\s*(\\d{1,2})(?:\\s*\\/\\s*(\\d{2,4}))?\\b`,
    'ig',
  ), source, (m) => {
    const a = Number(m[1]); const b = Number(m[2]);
    const order = a > 12 ? 'dmy' : b > 12 ? 'mdy' : (context.dateOrder ?? inferredDateOrder());
    const date = futureDate(
      order === 'mdy' ? a : b, order === 'mdy' ? b : a, yearOf(m[3]), context.nowMs,
    );
    chooseRepeatEnd(date ? { kind: 'on', date: ymd(date) } : null, m);
  });

  let isAllDay = false;
  scan(/\b(?:all[\s-]*day|allday)\b/ig, source, (m) => {
    if (isAllDay || !claims.claim(m.index, m.index + m[0].length, 'allDay')) return;
    isAllDay = true;
  });

  let chosenDate: DateParts | null = weeklyDays === null
    ? null
    : weekdayPatternDate(context.nowMs, weeklyDays);
  let chosenDateIsCadenceSeed = weeklyDays !== null;
  let explicitDate = weeklyDays !== null;
  let tonight = false;
  const chooseDate = (candidate: DateParts | null, m: RegExpExecArray) => {
    if (!claims.claim(m.index, m.index + m[0].length, 'date')) return;
    explicitDate = true;
    if (!candidate) {
      errors.push(`“${m[0].trim()}” is not a real date.`);
      return;
    }
    // A weekly pattern chooses its nearest valid first occurrence only as a
    // fallback. An explicit date in the same line is more specific and may
    // replace that seed (for example, “MWF next Fri”). A second *explicit*
    // date remains a conflict.
    if (chosenDateIsCadenceSeed) {
      chosenDate = candidate;
      chosenDateIsCadenceSeed = false;
      return;
    }
    if (chosenDate && ymd(chosenDate) !== ymd(candidate)) {
      errors.push('The line names more than one date.');
      return;
    }
    chosenDate = candidate;
    chosenDateIsCadenceSeed = false;
  };

  scan(/\b(?:on\s+)?(today|tomorrow|tmr|tmrw|tonight)\b/ig, source, (m) => {
    const word = m[1].toLowerCase();
    const base = partsOf(context.nowMs);
    tonight ||= word === 'tonight';
    chooseDate(word === 'today' || word === 'tonight' ? base : addDays(base, 1), m);
  });
  const weekday = new RegExp(`\\b(?:on\\s+)?(?:(next|this)\\s+)?(${WEEKDAY_WORD})\\b`, 'ig');
  scan(weekday, source, (m) => {
    chooseDate(weekdayDate(context.nowMs, WEEKDAYS[m[2].toLowerCase()], m[1]?.toLowerCase() === 'next'), m);
  });
  scan(/\b(?:on\s+)?(\d{4})-(\d{1,2})-(\d{1,2})\b/g, source, (m) => {
    chooseDate(validDate(Number(m[1]), Number(m[2]), Number(m[3])), m);
  });
  const namedMonthFirst = new RegExp(`\\b(?:on\\s+)?(${MONTH_WORD})\\.?\\s+(\\d{1,2})(?:st|nd|rd|th)?(?:,?\\s+(\\d{4}))?\\b`, 'ig');
  scan(namedMonthFirst, source, (m) => {
    chooseDate(futureDate(MONTHS[m[1].toLowerCase()], Number(m[2]), yearOf(m[3]), context.nowMs), m);
  });
  const namedDayFirst = new RegExp(`\\b(?:on\\s+)?(\\d{1,2})(?:st|nd|rd|th)?\\s+(${MONTH_WORD})\\.?(?:\\s+(\\d{4}))?\\b`, 'ig');
  scan(namedDayFirst, source, (m) => {
    chooseDate(futureDate(MONTHS[m[2].toLowerCase()], Number(m[1]), yearOf(m[3]), context.nowMs), m);
  });
  scan(/\b(?:on\s+)?(\d{1,2})\s*\/\s*(\d{1,2})(?:\s*\/\s*(\d{2,4}))?\b/g, source, (m) => {
    const a = Number(m[1]); const b = Number(m[2]);
    const order = a > 12 ? 'dmy' : b > 12 ? 'mdy' : (context.dateOrder ?? inferredDateOrder());
    const month = order === 'mdy' ? a : b; const day = order === 'mdy' ? b : a;
    chooseDate(futureDate(month, day, yearOf(m[3]), context.nowMs), m);
  });
  scan(/\b(?:on\s+)?(\d{1,2})[.-](\d{1,2})[.-](\d{4})\b/g, source, (m) => {
    const a = Number(m[1]); const b = Number(m[2]);
    const order = a > 12 ? 'dmy' : b > 12 ? 'mdy' : (context.dateOrder ?? inferredDateOrder());
    chooseDate(validDate(Number(m[3]), order === 'mdy' ? a : b, order === 'mdy' ? b : a), m);
  });
  scan(/\b(?:on\s+)?(?:the\s+)?(\d{1,2})(?:st|nd|rd|th)\b/ig, source, (m) => {
    const now = new Date(context.nowMs);
    let month = now.getMonth() + 1; let year = now.getFullYear();
    let candidate = validDate(year, month, Number(m[1]));
    if (!candidate || new Date(year, month - 1, Number(m[1]), 23, 59).getTime() < context.nowMs) {
      month += 1; if (month === 13) { month = 1; year += 1; }
      candidate = validDate(year, month, Number(m[1]));
    }
    chooseDate(candidate, m);
  });

  let durationMinutes: number | null = null;
  const chooseDuration = (minutes: number, m: RegExpExecArray) => {
    if (!claims.claim(m.index, m.index + m[0].length, 'duration')) return;
    if (!Number.isFinite(minutes) || minutes <= 0) {
      errors.push('Duration must be greater than zero.');
      return;
    }
    if (durationMinutes !== null && durationMinutes !== minutes) {
      errors.push('The line names more than one duration.');
    } else durationMinutes = Math.round(minutes);
  };
  scan(/\b(?:for\s+)?(?:half\s+(?:an?\s+)?hour|½\s*(?:h|hr|hour))\b/ig, source, (m) => chooseDuration(30, m));
  scan(/\b(?:for\s+)?(?:a|an|one)\s+hour\b/ig, source, (m) => chooseDuration(60, m));
  scan(/\b(?:for\s+)?(?:a\s+)?quarter\s+(?:of\s+an?\s+)?hour\b/ig, source, (m) => chooseDuration(15, m));
  scan(/(?:^|\s)(?:for\s+)?(\.\d+)\s*(?:h|hr|hrs|hour|hours)\b/ig, source, (m) => {
    chooseDuration(Number(m[1]) * 60, m);
  });
  scan(/\b(?:for\s+)?(\d+(?:\.\d+)?)\s*(?:h|hr|hrs|hour|hours)\s*(?:(\d+)\s*(?:m|min|mins|minute|minutes))?\b/ig, source, (m) => {
    chooseDuration(Number(m[1]) * 60 + Number(m[2] ?? 0), m);
  });
  scan(/\b(?:for\s+)?(\d+)\s*(?:m|min|mins|minute|minutes)\b/ig, source, (m) => chooseDuration(Number(m[1]), m));

  let startClock: Clock | null = null;
  let endClock: Clock | null = null;
  let explicitTime = false;
  const chooseTime = (
    start: Clock | null,
    end: Clock | null,
    m: RegExpExecArray,
    requiresEnd = false,
  ) => {
    if (!claims.claim(m.index, m.index + m[0].length, 'time')) return;
    explicitTime = true;
    if (!start || (requiresEnd && !end)) {
      errors.push(`“${m[0].trim()}” is not a valid time.`);
      return;
    }
    if (startClock) {
      errors.push('The line names more than one start time.');
      return;
    }
    startClock = start;
    endClock = end;
    if (start.inferred || end?.inferred) {
      warnings.push(`Interpreted the time as ${clockText(start)}${end ? `–${clockText(end)}` : ''}. Add am/pm to be explicit.`);
    }
  };

  const range = new RegExp(`\\b(?:from\\s+)?(${CLOCK_TOKEN})\\s*(?:to|until|till|[-–—])\\s*(${CLOCK_TOKEN})(?=$|[\\s,;|])`, 'ig');
  scan(range, source, (m) => {
    const [a, b] = parseClockRange(m[1], m[2]);
    chooseTime(a, b, m, true);
  });
  const atTime = new RegExp(`(?:\\bat\\s*|@\\s*)(${CLOCK_TOKEN})(?=$|[\\s,;|])`, 'ig');
  scan(atTime, source, (m) => chooseTime(parseClock(m[1], null, true), null, m));
  const clearTime = new RegExp(`\\b(noon|midnight|(?:\\d{1,4}(?:(?::|\\.)[0-5]\\d)?)\\s*(?:${MERIDIEM})|(?:[01]?\\d|2[0-3])(?::|\\.)[0-5]\\d)(?=$|[\\s,;|])`, 'ig');
  scan(clearTime, source, (m) => chooseTime(parseClock(m[1]), null, m));

  if (isAllDay && explicitTime) errors.push('An all-day event cannot also have a start time.');
  if (isAllDay && durationMinutes !== null) errors.push('Use dates, not minutes or hours, to span an all-day event.');
  if (endClock && durationMinutes !== null) {
    warnings.push('The explicit end time takes precedence over the duration.');
  }

  const seedDate = partsOf(new Date(`${baseline.date}T12:00:00`).getTime());
  const eventDate = chosenDate ?? seedDate;
  if (weeklyDays !== null && !(weeklyDays as WeekdayCode[]).includes(WEEKDAY_OPTIONS[
    new Date(eventDate.year, eventDate.month - 1, eventDate.day, 12).getDay()
  ].code)) {
    errors.push('The event date must be one of the selected repeat days.');
  }
  if (tonight && !startClock) {
    startClock = { hour: 19, minute: 0, inferred: false };
    explicitTime = true;
  }
  const defaultStart = parseClock(baseline.start) ?? { hour: 9, minute: 0, inferred: false };
  const start = localInstant(
    eventDate,
    isAllDay ? { hour: 12, minute: 0, inferred: false } : (startClock ?? defaultStart),
  );
  let value = { ...baseline, calendarId };
  if (start === null) {
    errors.push(`${ymd(eventDate)} does not contain the requested local time in this time zone.`);
  } else if (isAllDay) {
    value = {
      ...blankValueAt(start, calendarId),
      isAllDay: true,
      date: ymd(eventDate),
      endDate: ymd(eventDate),
      sourceStartMs: null,
      sourceEndMs: null,
    };
  } else {
    let end: number;
    if (endClock) {
      const sameDayEnd = localInstant(eventDate, endClock);
      if (sameDayEnd === null) {
        errors.push(`${ymd(eventDate)} does not contain the requested end time in this time zone.`);
        end = start + (durationMinutes ?? context.defaultDurationMinutes) * 60_000;
      } else {
        if (sameDayEnd <= start) {
          end = localInstant(addDays(eventDate, 1), endClock)
            ?? start + (durationMinutes ?? context.defaultDurationMinutes) * 60_000;
        } else end = sameDayEnd;
      }
    } else {
      end = start + (durationMinutes ?? context.defaultDurationMinutes) * 60_000;
    }
    value = blankValueAt(start, calendarId, end);
  }

  value = {
    ...value,
    title: claims.title(),
    location,
    description,
    videoCall,
    guests,
    repeat,
    weeklyDays: weeklyDays ?? value.weeklyDays,
    repeatEnd,
  };

  if (repeatEnd.kind !== 'never' && repeat === 'never') {
    errors.push('Add a repeat pattern before saying when the series ends.');
  }
  const repeatError = repeatEndProblem(value);
  if (repeatError) errors.push(repeatError);

  const selectedCalendar = context.calendars.find((c) => c.id === calendarId);
  if (calendarId === null || !selectedCalendar) errors.push('Choose a writable calendar before creating the event.');
  if (guests.length > 0 && selectedCalendar && selectedCalendar.provider !== 'google') {
    errors.push('Email invitations require a Google calendar.');
  }
  const videoError = videoCallProblem(value, selectedCalendar?.provider ?? '');
  if (videoError) errors.push(videoError);

  return {
    value,
    baseline,
    islands: claims.islands,
    warnings: [...new Set(warnings)],
    errors: [...new Set(errors)],
    ready: source.trim() !== '' && errors.length === 0,
    explicitTime,
    explicitDate,
  };
}

/** Rows shown under the source line. They are intentionally strings, not form
 * controls: quick-add previews its interpretation; Continue editing is the
 * explicit route to editable fields. */
export function quickPreviewRows(
  result: QuickEventResult,
  calendars: Calendar[],
): Array<{ label: string; value: string }> {
  const v = result.value;
  const start = new Date(`${v.date}T${v.start}:00`);
  const end = new Date(`${v.endDate}T${v.end}:00`);
  const date = start.toLocaleDateString(undefined, {
    weekday: 'short', month: 'short', day: 'numeric', year: 'numeric',
  });
  const time = v.isAllDay
    ? 'All day'
    : `${start.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })}–${end.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })}`;
  const rows = [
    { label: 'Title', value: v.title || '(no title)' },
    { label: 'When', value: `${date} · ${time}` },
  ];
  if (v.repeat !== 'never') {
    const repeat = v.repeat === 'weekdays'
      ? 'Every weekday'
      : v.repeat === 'weekly'
        ? `Weekly · ${v.weeklyDays.map((code) =>
            WEEKDAY_OPTIONS.find((day) => day.code === code)?.name.slice(0, 3) ?? code).join(', ')}`
        : v.repeat[0].toUpperCase() + v.repeat.slice(1);
    rows.push({ label: 'Repeats', value: repeat });
    if (v.repeatEnd.kind === 'on') {
      const [year, month, day] = v.repeatEnd.date.split('-').map(Number);
      const shown = new Date(Date.UTC(year, month - 1, day)).toLocaleDateString(undefined, {
        month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC',
      });
      rows.push({ label: 'Ends', value: `On ${shown}` });
    } else if (v.repeatEnd.kind === 'after') {
      rows.push({
        label: 'Ends',
        value: `After ${v.repeatEnd.count} occurrence${v.repeatEnd.count === 1 ? '' : 's'}`,
      });
    }
  }
  if (v.location) rows.push({ label: 'Location', value: v.location });
  if (v.videoCall) {
    const provider = v.videoCall.provider === 'googleMeet' ? 'Google Meet'
      : v.videoCall.provider === 'zoom' ? 'Zoom' : (meetingProvider(v.videoCall.uri) ?? 'Video call');
    rows.push({ label: 'Video', value: v.videoCall.uri ? `${provider} · ${v.videoCall.uri}` : provider });
  }
  if (v.guests.length > 0) rows.push({
    label: 'Guests',
    value: `${v.guests.map((g) => g.optional ? `${g.email} (optional)` : g.email).join(', ')} · invitations will be emailed`,
  });
  const calendar = calendars.find((c) => c.id === v.calendarId);
  if (calendar) rows.push({ label: 'Calendar', value: calendar.summary });
  if (v.description) rows.push({ label: 'Notes', value: v.description });
  return rows;
}

/** Short, copyable examples for the modal's empty state. They double as the
 * user-facing grammar: explicit directives are only needed where free text
 * would otherwise be ambiguous. */
export const QUICK_EVENT_EXAMPLES = [
  '30m at 2p Meet with Tim',
  'tomorrow 14:00–15:30 Design review invite ana@example.com',
  'MWF 9am Team sync +meet',
  'TTh 10am Office hours for 8 occurrences',
  'Lunch Fri noon loc:"Cafe Central"',
  'Demo 8/30 3pm 45m cal:Work optional:sam@example.com',
] as const;
