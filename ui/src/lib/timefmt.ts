import { dateTimeFormat } from './intlcache';
// Printing a clock — the one place in the app that decides whether an instant
// reads as `13:30` or as `1:30 PM`.
//
// This existed three times before it existed once: `EventBlock`, `Filmstrip`
// and `EventPopover` each carried the same two-line `hhmm`, and the Week and
// Day gutter carried a fourth spelling of the same idea. Three copies of a
// constant are three places to change and two places to forget, which is
// survivable while the constant is `hour12: false` forever and stops being so
// the moment it becomes a preference.
//
// **Built by hand rather than by `Intl`, deliberately.** The old `hhmm` asked
// `toLocaleTimeString` for a fixed 24-hour clock, which works because every
// locale that matters renders that the same way — but the 12-hour side has no
// such luck (`1:30 PM`, `1:30 pm`, `13:30` in a locale that ignores the flag),
// and the *screenshot goldens* would then depend on the ICU data of whichever
// machine rendered them. The testing standard has an open failure of exactly
// that shape already (`ruleInWords`, §6). Two dozen strings are cheaper than a
// second one.

/** The stored preference. The same two spellings the settings table holds and
 *  `settings::TimeFormat` serialises, so no layer translates. */
export type TimeFormat = '12h' | '24h';

const pad2 = (n: number) => String(n).padStart(2, '0');

/** The hour on a 12-hour dial: midnight and noon are both 12, never 0. The
 *  case every hand-rolled 12-hour clock gets wrong. */
const dial = (hour24: number) => (hour24 % 12 === 0 ? 12 : hour24 % 12);

/**
 * An instant as a clock time, in the *browser's* zone — the same zone
 * `toLocaleTimeString` read, and the same one the grid lays events out in.
 *
 * `1:30 PM` and not `01:30 PM`: a leading zero on a 12-hour clock is something
 * no clock face and no other calendar does, and it is what `hour: '2-digit'`
 * would have given us for free had this stayed with `Intl`.
 */
export function formatClock(ms: number, format: TimeFormat): string {
  const d = new Date(ms);
  const h = d.getHours();
  const m = pad2(d.getMinutes());
  return format === '12h' ? `${dial(h)}:${m} ${h < 12 ? 'AM' : 'PM'}` : `${pad2(h)}:${m}`;
}

/**
 * One label on the Week and Day hour ruler, given the hour 0-23.
 *
 * The 12-hour form is compact — `12a`, `1p` — rather than `12 AM`, because the
 * gutter is a fixed-width column beside the grid and the long form is nearly
 * three times the glyphs of the `00`-`23` it replaces. A ruler wide enough to
 * spell "AM" twenty-four times is a ruler taking space from the calendar.
 */
export function gutterLabel(hour24: number, format: TimeFormat): string {
  return format === '12h' ? `${dial(hour24)}${hour24 < 12 ? 'a' : 'p'}` : pad2(hour24);
}

/**
 * The wall-clock reading of an instant in an arbitrary IANA zone — the second
 * time zone's whole mechanism.
 *
 * `Intl` where `formatClock` deliberately is not, because this is the one job
 * only `Intl` can do in a browser: the tz database lives in the engine. The
 * locale-rendering hazard the header warns about is dodged the same way the
 * hazard was named: the formatter is only ever asked for *numbers*
 * (`formatToParts`, `en-US`, `hourCycle: 'h23'`), and the strings around them
 * stay hand-built below, so a screenshot never depends on ICU's idea of a
 * clock face.
 *
 * `null` for a zone the engine does not know — a row written by a newer
 * jiff than this WebKit's ICU, or hand-edited. Callers draw nothing, which
 * beats drawing the wrong clock with a confident face.
 */
export function zoneParts(ms: number, tz: string): { h: number; m: number } | null {
  try {
    const parts = dateTimeFormat('en-US', {
      timeZone: tz, hourCycle: 'h23', hour: 'numeric', minute: 'numeric',
    }).formatToParts(new Date(ms));
    const num = (type: string) => Number(parts.find((p) => p.type === type)?.value);
    const h = num('hour');
    const m = num('minute');
    // ICU spells midnight '24' under h23 in some versions; a clock does not.
    return Number.isFinite(h) && Number.isFinite(m) ? { h: h % 24, m } : null;
  } catch {
    return null;
  }
}

/** An instant on the second zone's clock, in the app's own two spellings —
 *  `formatClock`'s twin with the zone made explicit. `''` for a zone the
 *  engine refuses, per `zoneParts`. */
export function zoneClock(ms: number, tz: string, format: TimeFormat): string {
  const p = zoneParts(ms, tz);
  if (!p) return '';
  return format === '12h'
    ? `${dial(p.h)}:${pad2(p.m)} ${p.h < 12 ? 'AM' : 'PM'}`
    : `${pad2(p.h)}:${pad2(p.m)}`;
}

/**
 * One label on the second zone's half of the hour ruler, for the instant a
 * primary hour line marks.
 *
 * Takes an instant rather than an hour, because the second zone's reading of
 * the primary's `09:00` is not an hour of anything — India against a
 * whole-hour zone puts every label on `:30`. Minutes appear only when they
 * are non-zero, so a whole-hour second zone keeps the ruler as quiet as the
 * primary's own (`07`, `7a`), and a half-hour one says what it must
 * (`07:30`, `7:30a`).
 */
export function zoneGutterLabel(ms: number, tz: string, format: TimeFormat): string {
  const p = zoneParts(ms, tz);
  if (!p) return '';
  const min = p.m === 0 ? '' : `:${pad2(p.m)}`;
  return format === '12h'
    ? `${dial(p.h)}${min}${p.h < 12 ? 'a' : 'p'}`
    : `${pad2(p.h)}${min}`;
}

/**
 * The short name a zone wears in the gutter's header and beside the form's
 * echo line — `IST`, `EEST`, and only as a last resort the city.
 *
 * Alphabetic abbreviations, insisted on rather than taken as they come:
 * `en-US` alone answers `GMT+5:30` for Kolkata, and two right-anchored
 * `GMT+X:30`-shaped names in adjacent 40px lanes met in the middle and
 * read as one mashed string (first field run, 2026-08-26). No single
 * locale's English knows every conventional abbreviation — `en-US` has
 * EEST and PST but not IST, `en-IN` the reverse — so a few are asked in
 * turn and the first proper abbreviation wins. A zone none of them can
 * name falls back to its IANA city (`Asia/Tokyo` → `Tokyo`), which still
 * orients better than the offset that caused the pile-up.
 */
export function zoneAbbrev(tz: string): string {
  const city = () => tz.split('/').pop()?.replace(/_/g, ' ') ?? tz;
  for (const locale of ['en-US', 'en-GB', 'en-IN']) {
    try {
      const name = dateTimeFormat(locale, {
        timeZone: tz, timeZoneName: 'short',
      }).formatToParts(new Date()).find((p) => p.type === 'timeZoneName')?.value;
      if (name && /^[A-Z]{2,5}$/.test(name)) return name;
    } catch {
      // An unknown zone throws for every locale alike; the city is all
      // that is left to say.
      return city();
    }
  }
  return city();
}

/**
 * A stored `HH:MM` as the clock the user chose — what the event form's time
 * fields *show*.
 *
 * The storage form never changes: `EventFormValue.start`/`.end` are `HH:MM`
 * everywhere below the form, and in 24h mode this is the identity. It exists
 * because the native `<input type="time">` this replaced rendered per the
 * *system locale* — an AM/PM form on a grid drawn in 24h (reported
 * 2026-08-25), with no attribute to say otherwise; the engine's clock is not
 * the app's clock.
 */
export function displayClock(hhmm: string, format: TimeFormat): string {
  const m = /^(\d{2}):(\d{2})$/.exec(hhmm);
  if (!m || format === '24h') return hhmm;
  const h = Number(m[1]);
  return `${dial(h)}:${m[2]} ${h < 12 ? 'AM' : 'PM'}`;
}

/**
 * Typed time → stored `HH:MM`, or `null` for anything that is not a time.
 *
 * Deliberately generous on the way in — `9`, `9:30`, `21:30`, `9:30 pm`,
 * `9.30pm`, `12am` all land — because a form field is where people type,
 * not where they conform. The rules are a clock's own: a suffix makes the
 * hour a dial hour (1–12, `12am` = midnight, `12pm` = noon); no suffix
 * makes it a 24-hour reading (0–23). Minutes are two digits or absent —
 * `9:5` is a typo, not five past nine, and guessing would store the guess.
 */
export function parseClock(text: string): string | null {
  const m = /^\s*(\d{1,2})(?:[:.](\d{2}))?\s*(?:([ap])\.?\s*m?\.?)?\s*$/i.exec(text);
  if (!m) return null;
  let h = Number(m[1]);
  const min = m[2] === undefined ? 0 : Number(m[2]);
  const suffix = m[3]?.toLowerCase();
  if (min > 59) return null;
  if (suffix) {
    if (h < 1 || h > 12) return null;
    if (suffix === 'a') h = h === 12 ? 0 : h;
    else h = h === 12 ? 12 : h + 12;
  } else if (h > 23) {
    return null;
  }
  return `${pad2(h)}:${pad2(min)}`;
}
