// Shared by the QML widget and the webview popup. All positions use real
// instants, including the repeated/missing hour on daylight-saving days.
export function progress(event, now) {
  return Math.max(0, Math.min(1, (now - event.start_ms) / Math.max(1, event.end_ms - event.start_ms)));
}
export function joinable(events, now, minutes) {
  return (events || []).filter(e => !e.all_day && e.end_ms > now
    && e.start_ms <= now + Math.max(0, Math.min(60, minutes)) * 60000
    && /^https?:\/\//.test(e.conference || ''))
    .sort((a, b) => {
      const aNext = a.start_ms > now, bNext = b.start_ms > now;
      if (aNext !== bNext) return aNext ? -1 : 1;
      return aNext ? a.start_ms - b.start_ms : b.start_ms - a.start_ms;
    })[0] || null;
}
// Presentation only: preserve the first calendar's color and never merge
// unnamed entries or entries with different date spans.
export function uniqueAllDay(events) {
  const seen = new Set();
  return (events || []).filter(event => {
    if (!event.all_day || !event.title) return true;
    const key = JSON.stringify([event.title, event.start_ms, event.end_ms]);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

export function currentClock(ms, format, offsetSeconds) {
  const d = new Date(ms + offsetSeconds * 1000);
  const h = d.getUTCHours(), m = String(d.getUTCMinutes()).padStart(2, '0');
  return format === '12h' ? `${h % 12 || 12}:${m}${h < 12 ? 'am' : 'pm'}` : `${String(h).padStart(2, '0')}:${m}`;
}

export function countdownDuration(minutes) {
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60), remainder = minutes % 60;
  return `${hours}h${remainder ? ` ${remainder}m` : ''}`;
}

export const DEFAULT_MEETING_FORMAT = '{title} @ {time}  {countdown}';
export function meetingLabel(template, values) {
  return (template || DEFAULT_MEETING_FORMAT).replace(/\{([^{}]+)\}/g, (token, key) => (values[key] ?? token).slice(0, 256)).slice(0, 256);
}

/** Rows a day other than today shows before "+N more". Today is never cut:
 *  missing a meeting is worse than a long list. Carried in the feed as
 *  `per_day` so both renderers cut at the same row; this is the fallback. */
export const PER_DAY_CAP = 6;

const DAY_MS = 86400000;

/**
 * The agenda both popups draw — the Omarchy widget and the macOS menu bar —
 * from the feed's `panel` and the clock. One function so the two cannot
 * drift: what is folded, what is cut and where "+N more" points is decided
 * here and only rendered there.
 *
 * Sections, in order, each `{ title, kind, rows, more, anchor_ms }`:
 *   ALL DAY        today's all-day events (context, never counted as
 *                  "today still has something")
 *   EARLIER TODAY  finished timed events; `kind: 'folded'` with a `count`
 *                  unless `opts.earlierOpen`, absent when `panel.earlier`
 *                  is 'off'
 *   ONGOING / UPCOMING   the rest of today, never cut
 *   TOMORROW, then dated days   when `panel.tomorrow` / `panel.days_ahead`
 *                  ask for them, each cut at `per_day` with `more` and an
 *                  `anchor_ms` inside that day for "+N more" to open
 *
 * And the rule that makes a short default safe: **when today is spent, the
 * nearest later day with anything takes its place** — Friday evening shows
 * Monday under its own date rather than nothing, whatever the day settings
 * say, and is not shown twice if the settings would also have asked for it.
 */
export function agendaSections(panel, now, opts) {
  const earlierOpen = !!(opts && opts.earlierOpen);
  // A feed from before 3.3.0 carries today's events on the panel and no day groups: today alone, then.
  const days = panel && Array.isArray(panel.agenda_days) ? panel.agenda_days
    : panel && Array.isArray(panel.events) ? [{ date_label: '', events: panel.events }] : [];
  if (!days.length) return [];
  const perDay = Number(panel.per_day) > 0 ? Math.floor(Number(panel.per_day)) : PER_DAY_CAP;
  const earlierOff = panel.earlier === 'off';
  const tomorrowOn = panel.tomorrow !== false;
  const ahead = Math.max(0, Math.min(6, Math.floor(Number(panel.days_ahead)) || 0));
  const base = Number(panel.day_start_ms) || 0;
  // Half a day in, so the instant names the right date across a DST edge.
  const anchor = (d) => base + d * DAY_MS + DAY_MS / 2;
  const list = (d) => uniqueAllDay(days[d] && Array.isArray(days[d].events) ? days[d].events : []);
  const dayTitle = (d) => (d === 1 ? 'TOMORROW' : String((days[d] && days[d].date_label) || ''));
  // No object spread in this module: the QML engine loads it too, and its
  // JavaScript rejects a spread in an object literal as a syntax error — one
  // such literal and the whole module fails to load, taking the widget with
  // it (2026-09-12, seen on the bar).
  const section = (title, rows, d, more) => ({ title: title, kind: 'rows', rows: rows, more: more || 0, anchor_ms: anchor(d) });
  const daySection = (d) => {
    const ev = list(d);
    if (!ev.length) return null;
    return section(dayTitle(d), ev.slice(0, perDay), d, Math.max(0, ev.length - perDay));
  };

  const today = list(0);
  const out = [];
  const allDay = today.filter((e) => e.all_day);
  if (allDay.length) out.push(section('ALL DAY', allDay, 0, 0));
  const earlier = today.filter((e) => !e.all_day && e.end_ms <= now);
  if (earlier.length && !earlierOff) {
    out.push(earlierOpen
      ? section('EARLIER TODAY', earlier, 0, 0)
      : { title: 'EARLIER TODAY', kind: 'folded', rows: [], count: earlier.length, more: 0, anchor_ms: anchor(0) });
  }
  const ongoing = today.filter((e) => !e.all_day && e.start_ms <= now && e.end_ms > now);
  const upcoming = today.filter((e) => !e.all_day && e.start_ms > now);
  if (ongoing.length) out.push(section('ONGOING', ongoing, 0, 0));
  if (upcoming.length) out.push(section('UPCOMING', upcoming, 0, 0));

  let replaced = -1;
  if (!ongoing.length && !upcoming.length) {
    for (let d = 1; d < days.length; d++) {
      const sec = daySection(d);
      if (sec) { out.push(sec); replaced = d; break; }
    }
  }
  const wanted = [];
  if (tomorrowOn) wanted.push(1);
  for (let d = 2; d <= 1 + ahead; d++) wanted.push(d);
  for (const d of wanted) {
    if (d === replaced || d >= days.length) continue;
    const sec = daySection(d);
    if (sec) out.push(sec);
  }
  return out;
}
