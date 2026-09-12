// Window presence, not a claim that a provider has connected audio/video.
// Inputs come from Wayland; nothing is persisted or sent outside the shell.
const MAX_WINDOWS = 256, MAX_EVENTS = 256, MAX_TEXT = 1024;
// Every domain Zoom itself serves meetings from (#119). `zoom.us` alone
// refused Zoom X — Telekom's German/EU Zoom, the one a university runs —
// and with it Zoom for Government and Zoom China. One list, exported: the
// app's `location.ts` builds its matcher from this rather than keeping a
// second, subtly different idea of what a Zoom link is.
export const ZOOM_HOSTS = ['zoom.us', 'zoom-x.de', 'zoomgov.com', 'zoom.com.cn'];
export function isZoomHost(host) {
  const h = typeof host === 'string' ? host.toLowerCase() : '';
  return ZOOM_HOSTS.some(z => h === z || h.endsWith('.' + z));
}
function text(value) {
  return typeof value === 'string' && value.length <= MAX_TEXT
    && !/[\x00-\x1f\x7f-\x9f\u202a-\u202e\u2066-\u2069]/.test(value) ? value.trim().toLowerCase() : '';
}
function meeting(event) {
  if (!event || event.all_day || !Number.isFinite(event.start_ms) || !Number.isFinite(event.end_ms)
    || event.end_ms <= event.start_ms) return null;
  const url = text(event.conference);
  const match = /^https:\/\/([^/?#]+)(\/[^?#]*)?/.exec(url);
  if (!match) return null;
  const host = match[1], path = match[2] || '';
  if (host === 'meet.google.com') {
    const id = /^\/([a-z]{3}-[a-z]{4}-[a-z]{3})(?:\/|$)/.exec(path)?.[1];
    return id ? { provider: 'meet', id } : null;
  }
  if (isZoomHost(host)) {
    const id = /^\/(?:j|wc\/join)\/(\d{9,11})(?:\/|$)/.exec(path)?.[1];
    return id ? { provider: 'zoom', id } : null;
  }
  if (['teams.microsoft.com', 'teams.live.com', 'teams.cloud.microsoft'].includes(host)) {
    return /^\/(?:l\/meetup-join|meet)\//.test(path) ? { provider: 'teams', id: path } : null;
  }
  return null;
}
export function eventKey(event) {
  const m = meeting(event);
  return m ? m.provider + ':' + m.id + ':' + event.start_ms : '';
}
function windowKind(window) {
  const app = text(window?.appId), title = text(window?.title);
  if (!app || !title || /^(?:chat|calendar|activity|settings|calls)\s*[|–—-]/.test(title) || /(?:pre[- ]?join|preview|waiting room|you left|meeting (?:ended|has ended))/.test(title)) return null;
  const browser = /^(?:google-chrome|chromium|chrome|brave|firefox|org\.mozilla\.firefox|microsoft-edge|msedge|vivaldi)(?:[.-]|$)/.test(app);
  if (/^(?:zoom|us\.zoom\.xos|zoom\.real|com\.zoom\.zoom)(?:[.-]|$)/.test(app)) {
    if (/^zoom (?:workplace|cloud meetings|settings|home)$/.test(title)) return null;
    return { provider: 'zoom', title, generic: /^(?:zoom meeting|zoom - meeting)$/.test(title) };
  }
  if ((browser && /(?:google meet|\bmeet\b)/.test(title))) return { provider: 'meet', title, generic: false };
  if ((browser || /^(?:teams|teams-for-linux|com\.microsoft\.teams)(?:[.-]|$)/.test(app))
    && /microsoft teams/.test(title)) return { provider: 'teams', title, generic: false };
  return null;
}
function matches(event, kind) {
  const m = meeting(event);
  if (!m || m.provider !== kind.provider) return false;
  if (m.provider === 'meet' && kind.title.includes(m.id)) return true;
  if (m.provider === 'zoom' && new RegExp('(^|\\D)' + m.id + '(\\D|$)').test(kind.title.replace(/[ -]/g, ''))) return true;
  const title = text(event.title);
  return title.length >= 4 && kind.title.split(/\s+[|–—-]\s+/).some(part => part === title);
}

// Once associated, a window cannot claim another occurrence merely because
// the clock advanced. Closing it, or changing its title (e.g. a browser tab
// switch), ends that association. Generic Zoom windows need one unambiguous
// candidate, or a recent Join request plus a newly opened meeting window.
export function observe(previous, windows, events, now, leadMinutes, intent = null) {
  if (!Array.isArray(windows) || windows.length > MAX_WINDOWS || !Array.isArray(events)
    || events.length > MAX_EVENTS || !Number.isFinite(now)) return [];
  const lead = Math.max(0, Math.min(60, Number(leadMinutes) || 0)) * 60000;
  const seen = new Set();
  const candidates = events.filter(e => {
    const key = eventKey(e);
    if (!key || seen.has(key) || e.end_ms <= now || e.start_ms > now + lead) return false;
    seen.add(key);
    return true;
  });
  const records = Array.isArray(previous) ? previous.slice(0, MAX_WINDOWS) : [];
  return windows.map(window => {
    const key = text(window?.key), app = text(window?.appId), title = text(window?.title);
    if (!key || !app || !title) return null;
    const signature = app + '\n' + title;
    const old = records.find(r => r.key === key);
    if (old?.signature === signature) return old;
    const kind = windowKind(window);
    let found = kind ? candidates.filter(e => matches(e, kind)) : [];
    if (kind?.generic) {
      const providerEvents = candidates.filter(e => meeting(e).provider === kind.provider);
      const intended = !old && intent && now >= intent.at && now - intent.at <= 90000
        ? providerEvents.filter(e => eventKey(e) === intent.eventKey) : [];
      found = intended.length === 1 ? intended : providerEvents;
    }
    return { key, signature, eventKey: found.length === 1 ? eventKey(found[0]) : '' };
  }).filter(Boolean);
}
export function isPresent(records, event, now) {
  const key = eventKey(event);
  return !!key && event.start_ms <= now && now < event.end_ms
    && Array.isArray(records) && records.slice(0, MAX_WINDOWS).some(r => r.eventKey === key);
}
