import { test, expect } from '@playwright/test';
import { observe, isPresent, eventKey, type Window } from '../../packaging/omarchy-plugin/MeetingPresence.mjs';
import type { Event } from '../../packaging/omarchy-plugin/Timeline.mjs';
const now = 1_800_000;
const meeting = (conference: string, extra = {}): Event => ({ title: 'Design sync', conference, start_ms: now - 60000, end_ms: now + 3600000, all_day: false, ...extra });
const zoom = meeting('https://us02web.zoom.us/j/12345678901?pwd=private');
const meet = meeting('https://meet.google.com/abc-defg-hij');
const teams = meeting('https://teams.microsoft.com/l/meetup-join/example');
const window = (appId: string, title: string): Window => ({ key: 'one', appId, title });
const zm = window('zoom', 'Zoom Meeting');

test('scheduled time and clicking Join alone never establish window presence', () => {
  expect(isPresent(observe([], [], [zoom], now, 5, { at: now, eventKey: eventKey(zoom) }), zoom, now)).toBe(false);
  for (const title of ['Zoom Workplace', 'Zoom Settings', 'Waiting Room', 'Preview']) {
    expect(isPresent(observe([], [window('zoom', title)], [zoom], now, 5), zoom, now)).toBe(false);
  }
});

/** #119: the presence matcher keyed on `zoom.us` too, so a meeting on Zoom X
 *  never lit up even with its own numbered window open. */
test('a meeting on a regional Zoom domain is matched like any Zoom meeting', () => {
  const zoomX = meeting('https://uni-kassel.zoom-x.de/j/12345678901?pwd=private');
  const gov = meeting('https://zoomgov.com/j/12345678901');
  for (const event of [zoomX, gov]) {
    expect(eventKey(event)).toBe('zoom:12345678901:' + event.start_ms);
    expect(isPresent(observe([], [window('zoom', 'Zoom Meeting - 123 4567 8901')], [event], now, 5), event, now)).toBe(true);
  }
  // A lookalike domain is not Zoom, so it has no key and can never be present.
  expect(eventKey(meeting('https://zoom-x.de.evil.example/j/12345678901'))).toBe('');
});

test('a specific meeting window matches its provider and calendar identity', () => {
  for (const [event, win] of [
    [zoom, zm], [zoom, window('zoom', 'Zoom Meeting - 123 4567 8901')],
    [meet, window('google-chrome', 'Meet - abc-defg-hij - Google Chrome')],
    [meet, window('firefox', 'Design sync - Google Meet — Mozilla Firefox')],
    [teams, window('teams-for-linux', 'Design sync | Microsoft Teams')],
  ] as const) {
    expect(isPresent(observe([], [win], [event], now, 5), event, now)).toBe(true);
  }
  expect(isPresent(observe([], [window('code', 'Meet - abc-defg-hij')], [meet], now, 5), meet, now)).toBe(false);
  expect(isPresent(observe([], [window('chromium', 'Meet - xxx-yyyy-zzz')], [meet], now, 5), meet, now)).toBe(false);
  expect(isPresent(observe([], [window('teams-for-linux', 'Chat | Design sync | Microsoft Teams')], [teams], now, 5), teams, now)).toBe(false);
});

test('presence turns red only during the event and disappears on close or tab change', () => {
  const event = meeting(meet.conference!, { start_ms: now + 60000 });
  const win = window('chromium', 'Meet - abc-defg-hij');
  const records = observe([], [win], [event], now, 5);
  expect(isPresent(records, event, now)).toBe(false);
  expect(isPresent(records, event, event.start_ms)).toBe(true);
  expect(isPresent(records, event, event.end_ms)).toBe(false);
  expect(isPresent(observe(records, [], [event], now + 60000, 5), event, now + 60000)).toBe(false);
  const changed = { ...win, title: 'Mail - Chromium' };
  expect(isPresent(observe(records, [changed], [event], now + 60000, 5), event, now + 60000)).toBe(false);
});

test('an old window never migrates to the next occurrence as the clock advances', () => {
  const next = meeting(zoom.conference!, { start_ms: zoom.end_ms, end_ms: zoom.end_ms + 3600000 });
  const records = observe([], [zm], [zoom, next], now, 5);
  const later = observe(records, [zm], [next], next.start_ms, 5);
  expect(isPresent(later, next, next.start_ms)).toBe(false);
  expect(isPresent(observe(later, [], [next], next.start_ms, 5), next, next.start_ms)).toBe(false);
  expect(isPresent(observe([], [{ ...zm, key: 'new' }], [next], next.start_ms, 5), next, next.start_ms)).toBe(true);
});

test('a new Zoom window can use a recent Join target while ambiguous windows stay neutral', () => {
  const next = meeting('https://zoom.us/j/98765432109', { start_ms: now + 60000 });
  const events = [zoom, next];
  expect(isPresent(observe([], [zm], events, now, 5), zoom, now)).toBe(false);
  const intent = { at: now, eventKey: eventKey(next) };
  const records = observe([], [zm], events, now, 5, intent);
  expect(isPresent(records, next, next.start_ms)).toBe(true);
  expect(isPresent(records, zoom, now)).toBe(false);
  const old = observe([], [zm], [zoom], now, 5);
  expect(isPresent(observe(old, [zm], events, now, 5, intent), next, next.start_ms)).toBe(false);
  expect(isPresent(observe([], [zm], events, now, 5, { ...intent, at: now - 90001 }), zoom, now)).toBe(false);
});

test('duplicate calendar copies do not make one meeting ambiguous', () => {
  const duplicate = { ...zoom, calendar: 'Other calendar' };
  expect(isPresent(observe([], [zm], [zoom, duplicate], now, 5), zoom, now)).toBe(true);
});

test('malformed, oversized and hostile metadata fail neutral', () => {
  for (const title of [null, 'x'.repeat(1025), 'Zoom\u202e Meeting', '<b>Zoom Meeting</b>']) {
    expect(observe([], [window('zoom', title as string)], [zoom], now, 5).some(r => r.eventKey)).toBe(false);
  }
  expect(observe([], Array(257).fill(zm), [zoom], now, 5)).toEqual([]);
  expect(observe([], [zm], Array(257).fill(zoom), now, 5)).toEqual([]);
  for (const url of ['https://zoom.us.attacker.test/j/12345678901', 'file:///zoom.us/j/12345678901', 'https://zoom.us@attacker.test/j/12345678901']) {
    expect(eventKey(meeting(url))).toBe('');
  }
});
