import { ZOOM_HOSTS } from '../../../packaging/omarchy-plugin/MeetingPresence.mjs';

// Matched against the URL's host. Order does not matter; hosts are distinct.
// Zoom's entry is built from the widget's own list (#119): `zoom.us` alone
// refused Zoom X, Zoom for Government and Zoom China, and a second copy of
// the list here is exactly what this file's own comment below warns against.
const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const ZOOM_RE = new RegExp(`(^|\\.)(${ZOOM_HOSTS.map(escape).join('|')})$`, 'i');
const PROVIDERS: Array<[RegExp, string]> = [
  [ZOOM_RE, 'Zoom'],
  [/(^|\.)meet\.google\.com$/i, 'Google Meet'],
  [/(^|\.)teams\.microsoft\.com$/i, 'Teams'],
  [/(^|\.)teams\.live\.com$/i, 'Teams'],
  [/(^|\.)webex\.com$/i, 'Webex'],
  [/(^|\.)meet\.jit\.si$/i, 'Jitsi'],
];

const URL_RE = /https?:\/\/[^\s,;<>"']+/i;
// Global counterpart for `meetingUrl`'s walk — see its own doc comment for
// why the character class excludes `<`, `>`, `"` and `'`.
const URL_RE_ALL = /https?:\/\/[^\s,;<>"']+/gi;

/** The recognised meeting provider behind one URL, or `null`.
 *
 * Kept beside `meetingUrl`'s allowlist so quick event creation and the event
 * editor do not grow a second, subtly different definition of “a Zoom link”.
 * Callers may hand this either a bare URL or a sentence containing one; the
 * same punctuation and host checks as `meetingUrl` apply. */
export function meetingProvider(raw: string | null): string | null {
  const url = meetingUrl(raw);
  if (!url) return null;
  try {
    const host = new URL(url).hostname;
    return PROVIDERS.find(([re]) => re.test(host))?.[1] ?? null;
  } catch {
    return null;
  }
}

/**
 * The joinable meeting URL a location or description holds, or `null`.
 *
 * **Recognised providers only**, and that is the whole of the restraint here.
 * Google puts a Meet link in structured conference data, which is where the
 * popover's Join control has always come from — but an invitation minted by
 * anybody else arrives with its Zoom or Teams link sitting in `location` or
 * the description text and nowhere else, so the app could name the provider
 * (see `locationLabel`) and still give you nothing to click. This closes
 * that. Rust twin: `location_meeting_url`/`conference_join_url` in
 * `upcoming.rs`, which is what the Join button's click actually resolves
 * through — this function only drives the displayed `href` and the
 * location/description echo check, so keep the two in step rather than
 * letting them quietly diverge on what counts as joinable.
 *
 * An *unrecognised* link stays unclickable on purpose: a location or
 * description is as likely to hold a map pin, an agenda doc or a ticket as a
 * meeting, and a button labelled "Join video call" that opens a restaurant is
 * worse than no button at all.
 *
 * Every URL in the text is tried, not just the first — a description that
 * opens with an agenda link before the meeting link would otherwise give up
 * on that first, unrecognised one. The character class excludes `<`, `>`,
 * `"` and `'` so this is safe to call directly on raw HTML: without it, an
 * invite anchor (`<a href="https://…/j/123?pwd=x">Join Zoom Meeting</a>`)
 * scans straight through the closing quote into the link text and returns
 * `…pwd=x">Join`, a URL that looks plausible and 404s.
 *
 * The trailing-punctuation trim matters more than it looks: a link written
 * into a sentence ("dial in at https://…/j/123.") otherwise carries the full
 * stop into the URL and 404s.
 */
export function meetingUrl(raw: string | null): string | null {
  const text = (raw ?? '').trim();
  if (!text) return null;

  URL_RE_ALL.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = URL_RE_ALL.exec(text))) {
    const url = match[0].replace(/[),.;:!?\]]+$/, '');
    let host = '';
    try {
      host = new URL(url).hostname;
    } catch {
      continue;
    }
    if (PROVIDERS.some(([re]) => re.test(host))) return url;
  }
  return null;
}

/**
 * What to print in an event block's meta line.
 *
 * Google puts the joining link in `location` as well as in conference data, so
 * a naive render shows `https://us02we…` — a truncated URL, which tells you
 * nothing at a glance. A real place always wins over a link; a link alone
 * becomes its provider's name, or failing that its host.
 */
export function locationLabel(raw: string | null): string {
  const text = (raw ?? '').trim();
  if (!text) return '';

  const match = text.match(URL_RE);
  if (!match) return text;

  // A place written alongside the link is the useful half. Cut the string at
  // the URL rather than deleting it in place, and trim separators off each
  // half independently — a link sandwiched between two place fragments
  // ("Board room, <url>, 3rd floor") otherwise leaves the comma from both
  // sides behind ("Board room, , 3rd floor"). Colon is included here too, so
  // a trailing label ("Zoom:") loses it in the same pass.
  const idx = match.index ?? text.indexOf(match[0]);
  const stripSeparators = (s: string) =>
    s.replace(/^[\s,;:–—-]+/, '').replace(/[\s,;:–—-]+$/, '').trim();
  const left = stripSeparators(text.slice(0, idx));
  const right = stripSeparators(text.slice(idx + match[0].length));
  const withoutUrl = left && right ? `${left}, ${right}` : left || right;

  let host = '';
  try {
    host = new URL(match[0]).hostname;
  } catch {
    return withoutUrl || text;
  }

  const provider = PROVIDERS.find(([re]) => re.test(host))?.[1];

  // "Zoom: https://…" — the leading word is just naming the link, not a place.
  if (withoutUrl && provider && withoutUrl.toLowerCase() === provider.toLowerCase()) {
    return provider;
  }
  if (withoutUrl) return withoutUrl;

  return provider ?? host;
}
