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
export function layout(events, start, end) {
  const rows = (events || []).filter(e => !e.all_day && e.start_ms < end && e.end_ms > start)
    .map(event => ({ event, start: Math.max(start, event.start_ms), end: Math.min(end, event.end_ms), lane: 0, lanes: 1 }))
    .sort((a, b) => a.start - b.start || a.end - b.end);
  let cluster = [], ends = [], clusterEnd = -Infinity;
  function finish() { for (const row of cluster) row.lanes = ends.length; }
  for (const row of rows) {
    if (row.start >= clusterEnd) { finish(); cluster = []; ends = []; }
    let lane = ends.findIndex(end => end <= row.start);
    if (lane < 0) lane = ends.length;
    ends[lane] = row.end;
    row.lane = lane;
    row.top = (row.start - start) / (end - start);
    row.height = (row.end - row.start) / (end - start);
    cluster.push(row);
    clusterEnd = Math.max(clusterEnd, row.end);
  }
  finish();
  return rows;
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

export function visibleRange(day) {
  const start = day.visible_start_ms, end = day.visible_end_ms;
  if (Number.isFinite(start) && Number.isFinite(end) && start >= day.day_start_ms && end <= day.day_end_ms && start < end) return { start, end };
  return { start: day.day_start_ms, end: day.day_end_ms };
}
