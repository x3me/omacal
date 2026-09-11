import { test, expect } from '@playwright/test';
import { meetingLabel, countdownDuration, layout, progress, joinable, uniqueAllDay, type Event } from '../../packaging/omarchy-plugin/Timeline.mjs';
const at = (hour: number) => Date.UTC(2026, 8, 7, hour);
const event = (title: string, start: number, end: number, extra = {}): Event => ({
  title, start_ms: at(start), end_ms: at(end), all_day: false, ...extra,
});

test('Join respects the configured window and exclusive end, skipping focus blocks', () => {
  const events = [event('Focus', 9, 17), event('Call', 13, 14, { conference: 'https://meet.google.com/abc' })];
  expect(joinable(events, at(13) - 5 * 60000 - 1, 5)).toBeNull();
  expect(joinable(events, at(13) - 5 * 60000, 5)?.title).toBe('Call');
  expect(joinable(events, at(13) - 1, 0)).toBeNull();
  expect(joinable(events, at(13), 0)?.title).toBe('Call');
  expect(joinable(events, at(14), 60)).toBeNull();
  expect(joinable([event('Bad', 13, 14, { conference: 'file:///tmp/example' })], at(13), 5)).toBeNull();
});

test('back-to-back Join switches at the lead window and stays with the new call', () => {
  const current = event('Current', 12, 14, { conference: 'https://meet.google.com/current' });
  const next = event('Next', 13, 14, { conference: 'https://meet.google.com/next' });
  const later = event('Later', 14, 15, { conference: 'https://meet.google.com/later' });
  const events = [later, current, next];
  expect(joinable(events, at(13) - 300001, 5)?.title).toBe('Current');
  expect(joinable(events, at(13) - 300000, 5)?.title).toBe('Next');
  expect(joinable(events, at(13) - 60000, 0)?.title).toBe('Current');
  expect(joinable(events, at(13), 0)?.title).toBe('Next');
  expect(joinable(events, at(14), 0)?.title).toBe('Later');
});

test('day layout preserves overlaps and frees lanes at exact boundaries', () => {
  const rows = layout([event('Focus', 9, 17), event('First', 10, 11), event('Second', 11, 12), event('Later', 18, 19)], at(0), at(24));
  expect(rows.map(r => [r.event.title, r.lane, r.lanes])).toEqual([
    ['Focus', 0, 2], ['First', 1, 2], ['Second', 1, 2], ['Later', 0, 1],
  ]);
  expect(rows[0].top).toBe(9 / 24);
  expect(rows[0].height).toBe(8 / 24);
});

test('progress clamps completed and future meetings; multi-day blocks clip to the day', () => {
  const e = event('Design', 13, 14);
  expect([progress(e, at(12)), progress(e, at(13) + 30 * 60000), progress(e, at(15))]).toEqual([0, .5, 1]);
  const rows = layout([event('Long', -3, 26), event('Holiday', 0, 24, { all_day: true })], at(0), at(24));
  expect(rows).toHaveLength(1);
  expect(rows[0].top).toBe(0);
  expect(rows[0].height).toBe(1);
});

async function popup(page: import('@playwright/test').Page, visibleStart = 0, visibleEnd = 24) {
  await page.clock.install({ time: at(13) + 21 * 60000 });
  await page.setViewportSize({ width: 420, height: 600 });
  await page.addInitScript(({ start, end, visibleStart, visibleEnd }) => {
    const calls: { action: string }[] = [];
    (window as any).__actions = calls;
    const events = [
      { title: 'Morning planning', start_ms: start + 9 * 3600000, end_ms: start + 10 * 3600000, all_day: false, color: '#7da9e8' },
      { title: 'Design sync', start_ms: start + 13 * 3600000, end_ms: start + 14 * 3600000, all_day: false, color: '#7da9e8', conference: 'https://meet.google.com/abc', calendar: 'Work' },
      { title: '<b>Project review</b>', start_ms: start + 13.5 * 3600000, end_ms: start + 14.5 * 3600000, all_day: false, color: '#dba575' },
      { title: 'Labor Day', start_ms: start, end_ms: end, all_day: true, color: '#a8bd94', calendar: 'Personal' },
      { title: 'Labor Day', start_ms: start, end_ms: end, all_day: true, color: '#abcdef', calendar: 'Work' },
    ];
    const panel = { visible_start_ms: start + visibleStart * 3600000, visible_end_ms: start + visibleEnd * 3600000, day_start_ms: start, day_end_ms: end, events, timezone: 'UTC', date_format: 'dmy', time_format: '24h', day_view: false, label: true, join_minutes: 5 };
    const callbacks = new Map<number, (event: unknown) => void>();
    let nextCallback = 0;
    let changedCallback: number | undefined;
    (window as any).__changeMenuView = () => {
      panel.day_view = true;
      if (changedCallback !== undefined) callbacks.get(changedCallback)?.({ event: 'menubar-changed', payload: null });
    };
    (window as any).__TAURI_INTERNALS__ = {
      transformCallback: (callback: (event: unknown) => void) => { callbacks.set(++nextCallback, callback); return nextCallback; },
      invoke: async (cmd: string, args: any) => {
        if (cmd === 'plugin:event|listen') { if (args.event === 'menubar-changed') changedCallback = args.handler; return 1; }
        if (cmd === 'plugin:event|unlisten') return;
        if (cmd === 'menubar_feed') return { events: events.filter(e => e.end_ms > start + 13 * 3600000), panel, tasks: [] };
        if (cmd === 'get_palette') return { bg: '#20232b', surface: '#303540', text: '#e5e7eb', muted: '#9ca3af', accent: '#87b7ff', is_dark: true };
        if (cmd === 'set_menubar_preferences') { panel.day_view = args.dayView; return {}; }
        if (cmd === 'menubar_action') { calls.push(args); return; }
        throw new Error(cmd);
      },
    };
  }, { start: at(0), end: at(24), visibleStart, visibleEnd });
  await page.goto('/?menubar');
}

test('popup shows elapsed time, switches views, renders titles as text, and joins', async ({ page }) => {
  await popup(page);
  await expect(page.getByRole('button', { name: 'Join · Design sync' })).toBeVisible();
  await expect(page.getByRole('button', { name: /Morning planning/ })).toHaveClass(/past/);
  await expect(page.getByText('39m left')).toBeVisible();
  await expect(page.locator('header p')).toHaveText('07/09/2026 · 13:21');
  await page.clock.fastForward(60000);
  await expect(page.locator('header p')).toHaveText('07/09/2026 · 13:22');
  await page.getByRole('button', { name: 'Add event', exact: true }).click();
  expect(await page.evaluate(() => (window as any).__actions)).toContainEqual({ action: 'quick-add' });
  await expect(page.getByRole('button', { name: 'Labor Day', exact: true })).toHaveCount(1);
  await expect(page.locator('.agenda > :first-child')).toHaveClass(/all-day/);
  await expect(page.getByText('<b>Project review</b>')).toBeVisible();
  const elapsed = page.getByRole('progressbar', { name: 'Elapsed time: Design sync' });
  await expect(elapsed).toHaveAttribute('aria-valuenow', '37');
  const track = await elapsed.boundingBox();
  const fill = await elapsed.locator('.now-fill').boundingBox();
  expect(fill!.width / track!.width).toBeCloseTo(22 / 60, 2);
  const marker = await page.locator('.now-marker').boundingBox();
  const activeRow = await page.getByRole('button', { name: /Design sync.*38m left/ }).boundingBox();
  expect(marker!.y + marker!.height).toBeLessThanOrEqual(activeRow!.y);
  await page.screenshot({ path: '/tmp/omacal-menubar-agenda.png' });
  await page.getByRole('button', { name: 'Day', exact: true }).click();
  await expect(page.getByLabel('Day calendar')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Labor Day', exact: true })).toHaveCount(1);
  await expect(page.getByLabel('Now 13:22')).toBeVisible();
  const blocks = page.locator('.event');
  const a = await blocks.nth(1).boundingBox(), b = await blocks.nth(2).boundingBox();
  expect(a && b && a.x + a.width <= b.x).toBeTruthy();
  await page.screenshot({ path: '/tmp/omacal-menubar-day.png' });
  await page.getByRole('button', { name: 'Join · Design sync' }).click();
  expect(await page.evaluate(() => (window as any).__actions)).toEqual([{ action: 'quick-add' }, { action: 'join' }]);
  await page.keyboard.press('Escape');
  expect(await page.evaluate(() => (window as any).__actions)).toContainEqual({ action: 'close' });
});

test('all-day duplicates combine only matching titles and spans without changing the feed', () => {
  const holiday = event('Holiday', 0, 24, { all_day: true, calendar: 'Personal' });
  const otherSpan = event('Holiday', 0, 48, { all_day: true });
  const unnamed = event('', 0, 24, { all_day: true });
  const meeting = event('Call', 9, 10);
  const events = [holiday, { ...holiday, calendar: 'Work' }, otherSpan, unnamed, unnamed, meeting, meeting];
  expect(uniqueAllDay(events)).toEqual([holiday, otherSpan, unnamed, unnamed, meeting, meeting]);
  expect(events).toHaveLength(7);
});


test('open popup applies changed preferences without waiting for its polling timer', async ({ page }) => {
  await popup(page);
  await expect(page.locator('.timeline')).toHaveCount(0);
  await page.evaluate(() => (window as any).__changeMenuView());
  await expect(page.locator('.timeline')).toBeVisible();
});


test('popup gear opens OmaCal preferences', async ({ page }) => {
  await popup(page);
  await page.getByRole('button', { name: 'Open preferences' }).click();
  expect(await page.evaluate(() => (window as any).__actions)).toEqual([{ action: 'preferences' }]);
});


test('countdown uses hours at sixty minutes and omits zero remaining minutes', () => {
  expect([1, 59, 60, 64, 120, 642].map(countdownDuration)).toEqual(['1m', '59m', '1h', '1h 4m', '2h', '10h 42m']);
});

test('visible hours shorten the menu day without changing event times or agenda contents', async ({ page }) => {
  await popup(page, 12, 23);
  await expect(page.getByRole('button', { name: /Morning planning/ })).toBeVisible();
  await page.getByRole('button', { name: 'Day', exact: true }).click();
  expect(await page.locator('.timeline').evaluate(el => el.getBoundingClientRect().height)).toBe(660);
  await expect(page.locator('.event').filter({ hasText: 'Morning planning' })).toHaveCount(0);
  const event = page.locator('.event').filter({ hasText: 'Design sync' });
  await expect(event).toContainText('13:00');
  expect(await event.evaluate(el => (el as HTMLElement).offsetTop)).toBe(60);
  await expect(page.locator('.hour').first()).toHaveText('12:00');
});

test('meeting label templates reorder tokens without expanding title text', () => {
  expect(meetingLabel('{countdown} · {title} ({calendar})', { countdown: 'in 5m', title: 'Design {time}', calendar: 'Work', time: '13:30' })).toBe('in 5m · Design {time} (Work)');
});
