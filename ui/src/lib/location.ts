// Matched against the URL's host. Order does not matter; hosts are distinct.
const PROVIDERS: Array<[RegExp, string]> = [
  [/(^|\.)zoom\.us$/i, 'Zoom'],
  [/(^|\.)meet\.google\.com$/i, 'Google Meet'],
  [/(^|\.)teams\.microsoft\.com$/i, 'Teams'],
  [/(^|\.)teams\.live\.com$/i, 'Teams'],
  [/(^|\.)webex\.com$/i, 'Webex'],
  [/(^|\.)meet\.jit\.si$/i, 'Jitsi'],
];

const URL_RE = /https?:\/\/[^\s,;]+/i;

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
 * The joinable meeting URL a location holds, or `null`.
 *
 * **Recognised providers only**, and that is the whole of the restraint here.
 * Google puts a Meet link in structured conference data, which is where the
 * popover's Join control has always come from — but an invitation minted by
 * anybody else arrives with its Zoom or Teams link sitting in `location` and
 * nowhere else, so the app could name the provider (see `locationLabel`) and
 * still give you nothing to click. This closes that.
 *
 * An *unrecognised* link stays unclickable on purpose: a location field is as
 * likely to hold a map pin, a venue's homepage or a ticket as a meeting, and a
 * button labelled "Join video call" that opens a restaurant is worse than no
 * button at all.
 *
 * The trailing-punctuation trim matters more than it looks: a link written
 * into a sentence ("dial in at https://…/j/123.") otherwise carries the full
 * stop into the URL and 404s.
 */
export function meetingUrl(raw: string | null): string | null {
  const text = (raw ?? '').trim();
  if (!text) return null;

  const match = text.match(URL_RE);
  if (!match) return null;
  const url = match[0].replace(/[),.;:!?\]]+$/, '');

  let host = '';
  try {
    host = new URL(url).hostname;
  } catch {
    return null;
  }
  return PROVIDERS.some(([re]) => re.test(host)) ? url : null;
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
