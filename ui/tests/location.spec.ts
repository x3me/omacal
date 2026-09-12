import { test, expect } from '@playwright/test';
import { locationLabel, meetingProvider, meetingUrl } from '../src/lib/location';

test.describe('locationLabel', () => {
  test('a plain place is left alone', () => {
    expect(locationLabel('TAO Office, board room')).toBe('TAO Office, board room');
    expect(locationLabel('Room 4A')).toBe('Room 4A');
  });

  test('nothing in, nothing out', () => {
    expect(locationLabel(null)).toBe('');
    expect(locationLabel('   ')).toBe('');
  });

  // Real events put the joining link in `location`, which rendered as
  // `https://us02we…` — the truncation of a URL tells you nothing.
  test('known providers become their name', () => {
    expect(locationLabel('https://us02web.zoom.us/j/123456?pwd=x')).toBe('Zoom');
    // Issue #119: Zoom serves meetings from four domains, and only one was
    // recognised. A university on Zoom X could not attach its own links.
    for (const host of ['uni-kassel.zoom-x.de', 'zoomgov.com', 'us02web.zoomgov.com', 'zoom.com.cn']) {
      expect(locationLabel(`https://${host}/j/123456789?pwd=x`), host).toBe('Zoom');
      expect(meetingProvider(`https://${host}/j/123456789`), host).toBe('Zoom');
      expect(meetingUrl(`Join: https://${host}/j/123456789`), host).toBe(`https://${host}/j/123456789`);
    }
    // Suffix match on a real domain, not a substring: a host that merely
    // contains one of them is somebody else's.
    expect(meetingProvider('https://zoom-x.de.evil.example/j/123456789')).toBeNull();
    expect(meetingProvider('https://notzoom.us/j/123456789')).toBeNull();
    expect(locationLabel('https://meet.google.com/abc-defg-hij')).toBe('Google Meet');
    expect(locationLabel('https://teams.microsoft.com/l/meetup-join/x')).toBe('Teams');
  });

  test('a labelled link keeps its label', () => {
    expect(locationLabel('Zoom: https://us02web.zoom.us/j/1')).toBe('Zoom');
  });

  test('an unknown link becomes its host, not a truncated URL', () => {
    expect(locationLabel('https://whereby.com/omacal-standup')).toBe('whereby.com');
  });

  test('a place with a link keeps the place', () => {
    // Google often writes "Room 4A, https://meet.google.com/x". The room is
    // what you act on when you are walking somewhere.
    expect(locationLabel('Room 4A, https://meet.google.com/abc')).toBe('Room 4A');
  });

  // A link sandwiched between two place fragments used to leave the comma
  // from each side behind — "Board room, , 3rd floor" — because the old
  // strip logic only trimmed separators at the very start and end of the
  // string, not around the gap the removed URL left in the middle.
  test('a url between two place fragments joins them without a doubled separator', () => {
    expect(locationLabel('Board room, https://x.com/y, 3rd floor')).toBe('Board room, 3rd floor');
  });

  test('a url between semicolon-separated fragments normalises the separator', () => {
    expect(locationLabel('Room 2; https://meet.google.com/abc; level 3')).toBe('Room 2, level 3');
  });

  test('a url touching its neighbours with no whitespace still separates cleanly', () => {
    expect(locationLabel('A,https://x.com/y,B')).toBe('A, B');
  });

  // A recorded decision, not an oversight. `URL_RE` has no `g` flag, so only
  // the first URL is ever removed and a second one survives verbatim. Stripping
  // every URL would need a rule for which provider wins when two disagree, and
  // Google does not write two links into one `location` field — so the
  // behaviour stays as it is, and this test is what says so out loud. Change
  // the behaviour and this test is where the decision gets revisited.
  test('only the first url is removed — a second one survives verbatim', () => {
    expect(locationLabel('https://meet.google.com/abc https://us02web.zoom.us/j/1'))
      .toBe('https://us02web.zoom.us/j/1');
  });
});

// `locationLabel` decides what a location *reads* as; `meetingUrl` decides
// whether it is something you can click. Deliberately separate functions:
// naming a provider is safe on any link, and offering to join one is not.
test.describe('meetingUrl', () => {
  test('a recognised provider link is joinable', () => {
    expect(meetingUrl('https://us02web.zoom.us/j/123456?pwd=x'))
      .toBe('https://us02web.zoom.us/j/123456?pwd=x');
    expect(meetingUrl('https://meet.google.com/abc-defg-hij'))
      .toBe('https://meet.google.com/abc-defg-hij');
    expect(meetingUrl('https://teams.microsoft.com/l/meetup-join/x'))
      .toBe('https://teams.microsoft.com/l/meetup-join/x');
    expect(meetingUrl('https://acme.webex.com/meet/jo'))
      .toBe('https://acme.webex.com/meet/jo');
    expect(meetingUrl('https://meet.jit.si/omacal')).toBe('https://meet.jit.si/omacal');
  });

  test('a link beside a place is still joinable', () => {
    expect(meetingUrl('Board room, https://meet.google.com/abc-defg-hij, 3rd floor'))
      .toBe('https://meet.google.com/abc-defg-hij');
  });

  // **The reason the trim exists.** A link written into a sentence carries the
  // full stop into the URL, and the joined meeting 404s — a failure that looks
  // like a broken app rather than a stray character.
  test('sentence punctuation does not become part of the URL', () => {
    expect(meetingUrl('dial in at https://meet.google.com/abc-defg-hij.'))
      .toBe('https://meet.google.com/abc-defg-hij');
    expect(meetingUrl('(https://us02web.zoom.us/j/123)'))
      .toBe('https://us02web.zoom.us/j/123');
  });

  /** The restraint, and the half most worth pinning: a location field holds
   *  map pins and venue homepages far more often than it holds meetings, and
   *  "Join video call" opening a restaurant is worse than no button. */
  test('an unrecognised link is not offered as a meeting', () => {
    expect(meetingUrl('https://maps.google.com/?q=Board+room')).toBe(null);
    expect(meetingUrl('https://example.com/tickets/9')).toBe(null);
    expect(meetingUrl('https://zoom.us.evil.example.com/j/1')).toBe(null);
  });

  test('a place with no link, and nothing at all, are both not joinable', () => {
    expect(meetingUrl('TAO Office, board room')).toBe(null);
    expect(meetingUrl(null)).toBe(null);
    expect(meetingUrl('   ')).toBe(null);
    expect(meetingUrl('zoom')).toBe(null);
  });

  // The bug the description fallback shipped with the first time: raw HTML
  // from Google, scanned with a character class built for plain-text
  // `location`, swallows the closing quote of an anchor's `href` and runs
  // into the link text — "…pwd=x">Join", a URL that looks plausible and
  // 404s. Excluding `<`, `>`, `"` and `'` stops the scan at the attribute
  // boundary, which is what makes it safe to call directly on `description`.
  test('an html description does not swallow the closing quote', () => {
    expect(
      meetingUrl('<p>Hi team,</p><p><a href="https://us02web.zoom.us/j/123?pwd=x">Join Zoom Meeting</a></p>'),
    ).toBe('https://us02web.zoom.us/j/123?pwd=x');
  });

  // Every URL in the text is tried, not just the first — a description that
  // opens with an agenda link before the meeting link would otherwise give
  // up on that first, unrecognised one.
  test('a leading unrecognised link does not shadow a later one', () => {
    expect(
      meetingUrl('Agenda: https://docs.example.com/agenda then join at https://us02web.zoom.us/j/123'),
    ).toBe('https://us02web.zoom.us/j/123');
  });
});
