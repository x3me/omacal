import { test, expect } from '@playwright/test';
import { dateTimeFormat } from '../src/lib/intlcache';

/** Building an `Intl.DateTimeFormat` costs far more than using one, and the
 *  time gutter, the second-zone clock and every date label built a fresh
 *  one on each call. One per (locale, options), reused: the formatter is
 *  cached, never its output, so a zone's abbreviation still follows DST. */
test('one formatter per locale and options, reused', () => {
  const a = dateTimeFormat('en-US', { timeZone: 'Europe/Sofia', hour: 'numeric' });
  const b = dateTimeFormat('en-US', { timeZone: 'Europe/Sofia', hour: 'numeric' });
  const c = dateTimeFormat('en-US', { timeZone: 'Asia/Kolkata', hour: 'numeric' });
  expect(a).toBe(b);
  expect(a).not.toBe(c);
  // The same answers a fresh one gives.
  const ms = Date.UTC(2026, 6, 1, 9);
  expect(a.format(ms)).toBe(new Intl.DateTimeFormat('en-US', { timeZone: 'Europe/Sofia', hour: 'numeric' }).format(ms));
});

test('a zone the engine refuses still throws, and is not remembered', () => {
  expect(() => dateTimeFormat('en-US', { timeZone: 'Not/AZone' })).toThrow();
  expect(() => dateTimeFormat('en-US', { timeZone: 'Not/AZone' })).toThrow();
});
