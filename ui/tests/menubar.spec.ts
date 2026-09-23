import { test, expect } from '@playwright/test';
import { layout, agendaSections, meetingLabel, countdownDuration, progress, joinable, uniqueAllDay, PER_DAY_CAP, type Event } from '../../packaging/omarchy-plugin/Timeline.mjs';
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

async function popup(page: import('@playwright/test').Page, visibleStart = 0, visibleEnd = 24, options: { allDayCount?: number; shortMeetings?: boolean; tasks?: boolean } = {}) {
  await page.clock.install({ time: at(13) + 21 * 60000 });
  await page.setViewportSize({ width: 420, height: 600 });
  await page.addInitScript(({ start, end, visibleStart, visibleEnd, options }) => {
    const calls: { action: string }[] = [];
    (window as any).__actions = calls;
    const events = [
      { title: 'Morning planning', start_ms: start + 9 * 3600000, end_ms: start + 10 * 3600000, all_day: false, color: '#7da9e8' },
      { title: 'Design sync', start_ms: start + 13 * 3600000, end_ms: start + 14 * 3600000, all_day: false, color: '#7da9e8', conference: 'https://meet.google.com/abc', calendar: 'Work' },
      { title: '<b>Project review</b>', start_ms: start + 13.5 * 3600000, end_ms: start + 14.5 * 3600000, all_day: false, color: '#dba575' },
      { title: 'Labor Day', start_ms: start, end_ms: end, all_day: true, color: '#a8bd94', calendar: 'Personal' },
      { title: 'Labor Day', start_ms: start, end_ms: end, all_day: true, color: '#abcdef', calendar: 'Work' },
    ];
    for (let i = 1; i < (options.allDayCount ?? 1); i++)
      events.push({title: `All-day ${i}`, start_ms: start, end_ms: end, all_day: true, color: '#a8bd94', calendar: 'Personal'});
    if (options.shortMeetings) events.push(
      {title: 'Quick one', start_ms: start + 15 * 3600000, end_ms: start + 15.25 * 3600000, all_day: false, color: '#7da9e8'},
      {title: 'Quick two', start_ms: start + 15.25 * 3600000, end_ms: start + 15.5 * 3600000, all_day: false, color: '#dba575'},
    );
    const panel = { visible_start_ms: start + visibleStart * 3600000, visible_end_ms: start + visibleEnd * 3600000, day_start_ms: start, day_end_ms: end, events, timezone: 'UTC', date_format: 'dmy', time_format: '24h', day_view: false, label: true, join_minutes: 5 };
    const callbacks = new Map<number, (event: unknown) => void>();
    let nextCallback = 0;
    let changedCallback: number | undefined;
    (window as any).__changeMenuFormat = () => {
      panel.time_format = '12h';
      if (changedCallback !== undefined) callbacks.get(changedCallback)?.({event: 'menubar-changed', payload: null});
    };
    (window as any).__changeOtherPreferences = () => { panel.label = false; panel.join_minutes = 15; };
    (window as any).__menuPreferences = () => ({day: panel.day_view, label: panel.label, join: panel.join_minutes});
    (window as any).__changeMenuView = () => {
      panel.day_view = true;
      if (changedCallback !== undefined) callbacks.get(changedCallback)?.({ event: 'menubar-changed', payload: null });
    };
    (window as any).__TAURI_INTERNALS__ = {
      transformCallback: (callback: (event: unknown) => void) => { callbacks.set(++nextCallback, callback); return nextCallback; },
      invoke: async (cmd: string, args: any) => {
        if (cmd === 'plugin:event|listen') { if (args.event === 'menubar-changed') changedCallback = args.handler; return 1; }
        if (cmd === 'plugin:event|unlisten') return;
        if (cmd === 'menubar_feed') return structuredClone({ events: events.filter(e => e.end_ms > start + 13 * 3600000), panel, tasks: options.tasks ? [{title: 'Submit expenses', overdue: false}] : [] });
        if (cmd === 'get_palette') return { bg: '#20232b', surface: '#303540', text: '#e5e7eb', muted: '#9ca3af', accent: '#87b7ff', is_dark: true };
        if (cmd === 'set_setting' && args.setting.key === 'menubarDayView') { panel.day_view = args.setting.value; return {}; }
        if (cmd === 'set_setting' && args.setting.key === 'menubarPreferences') { const v = args.setting.value; panel.day_view = v.dayView; panel.label = v.label; panel.join_minutes = v.joinMinutes; return {}; }
        if (cmd === 'menubar_action') { calls.push(args); return; }
        throw new Error(cmd);
      },
    };
  }, { start: at(0), end: at(24), visibleStart, visibleEnd, options });
  await page.goto('/?menubar');
}

test('popup shows elapsed time, switches views, renders titles as text, and joins', async ({ page }) => {
  await popup(page);
  await expect(page.getByRole('button', { name: 'Join · Design sync' })).toBeVisible();
  // The finished meeting is folded away by default; it is still there, marked past, once opened.
  await expect(page.getByRole('button', { name: /Morning planning/ })).toHaveCount(0);
  await page.getByRole('button', { name: '1 earlier today · show' }).click();
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
  await expect(page.locator('.now-line')).toHaveText('13:22');
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


test('open agenda applies a changed clock format without waiting for polling', async ({ page }) => {
  await popup(page);
  await expect(page.locator('header p')).toHaveText('07/09/2026 · 13:21');
  await page.evaluate(() => (window as any).__changeMenuFormat());
  await expect(page.locator('header p')).toHaveText('07/09/2026 · 1:21 PM');
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
  await page.getByRole('button', { name: '1 earlier today · show' }).click();
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

// ---- the agenda plan both popups draw ------------------------------------
// One function, tested once: the Omarchy widget and the macOS popup only
// render what this returns, which is what keeps them from drifting.
const dayOf = (label: string, events: Event[]) => ({ date_label: label, events });
const plan = (over: Record<string, unknown> = {}, days?: { date_label: string; events: Event[] }[]) =>
  agendaSections({ day_start_ms: at(0), earlier: 'folded', tomorrow: true, days_ahead: 0, per_day: PER_DAY_CAP,
    agenda_days: days ?? [dayOf('Mon 7', []), dayOf('Tue 8', [])], ...over }, at(12));
const titles = (s: ReturnType<typeof agendaSections>) => s.map((x) => x.title);

test('today is never cut, whatever the per-day cap', () => {
  const nine = Array.from({ length: 9 }, (_, i) => event(`m${i}`, 13 + i, 14 + i));
  const s = plan({}, [dayOf('Mon 7', nine), dayOf('Tue 8', [])]);
  expect(titles(s)).toEqual(['UPCOMING']);
  expect(s[0].rows).toHaveLength(9);
  expect(s[0].more).toBe(0);
});

test('finished events fold into one line by default, open on request, and can be hidden', () => {
  const today = [event('a', 8, 9), event('b', 9, 10), event('c', 10, 11), event('next', 14, 15)];
  const folded = plan({}, [dayOf('Mon 7', today)]);
  expect(folded[0]).toMatchObject({ title: 'EARLIER TODAY', kind: 'folded', count: 3, rows: [] });
  expect(titles(folded)).toEqual(['EARLIER TODAY', 'UPCOMING']);
  const open = agendaSections({ day_start_ms: at(0), agenda_days: [dayOf('Mon 7', today)] }, at(12), { earlierOpen: true });
  expect(open[0]).toMatchObject({ title: 'EARLIER TODAY', kind: 'rows' });
  expect(open[0].rows.map((e) => e.title)).toEqual(['a', 'b', 'c']);
  expect(titles(plan({ earlier: 'off' }, [dayOf('Mon 7', today)]))).toEqual(['UPCOMING']);
});

test("ongoing comes before upcoming, and all-day rows are today's only", () => {
  // Whole hours only: `Date.UTC` truncates a fractional hour, and 11.5–12.5
  // would silently become 11–12 and end exactly at `now`.
  const today = [event('Trip', 0, 48, { all_day: true }), event('standup', 11, 13), event('later', 15, 16)];
  const s = plan({}, [dayOf('Mon 7', today), dayOf('Tue 8', [event('Offsite', 24, 48, { all_day: true })])]);
  expect(titles(s)).toEqual(['ALL DAY', 'ONGOING', 'UPCOMING', 'TOMORROW']);
  expect(s[3].rows[0].title).toBe('Offsite');
});

test('tomorrow is cut at the cap with "+N more" pointing into that day, and can be switched off', () => {
  const eight = Array.from({ length: 8 }, (_, i) => event(`t${i}`, 24 + 9 + i, 24 + 10 + i));
  const s = plan({}, [dayOf('Mon 7', [event('now', 11, 13)]), dayOf('Tue 8', eight)]);
  const tomorrow = s.find((x) => x.title === 'TOMORROW')!;
  expect(tomorrow.rows).toHaveLength(PER_DAY_CAP);
  expect(tomorrow.more).toBe(2);
  expect(tomorrow.anchor_ms).toBeGreaterThan(at(24));
  expect(tomorrow.anchor_ms).toBeLessThan(at(48));
  expect(titles(plan({ tomorrow: false }, [dayOf('Mon 7', [event('now', 11, 13)]), dayOf('Tue 8', eight)]))).toEqual(['ONGOING']);
});

test('further days follow tomorrow by their own date label, only as many as asked', () => {
  const days = [dayOf('Mon 7', [event('now', 11, 13)]), dayOf('Tue 8', [event('t', 33, 34)]), dayOf('Wed 9', [event('w', 57, 58)]), dayOf('Thu 10', [event('th', 81, 82)]), dayOf('Fri 11', [event('f', 105, 106)])];
  expect(titles(plan({ days_ahead: 0 }, days))).toEqual(['ONGOING', 'TOMORROW']);
  expect(titles(plan({ days_ahead: 2 }, days))).toEqual(['ONGOING', 'TOMORROW', 'Wed 9', 'Thu 10']);
  expect(titles(plan({ days_ahead: 6 }, days))).toEqual(['ONGOING', 'TOMORROW', 'Wed 9', 'Thu 10', 'Fri 11']);
});

test('when today is spent, the nearest later day with anything takes its place — and is not shown twice', () => {
  const spent = [event('done', 8, 9)];
  const skip = plan({ days_ahead: 0 }, [dayOf('Mon 7', spent), dayOf('Tue 8', []), dayOf('Wed 9', [event('w', 57, 58)])]);
  expect(titles(skip)).toEqual(['EARLIER TODAY', 'Wed 9']);
  const once = plan({ days_ahead: 0 }, [dayOf('Mon 7', spent), dayOf('Tue 8', [event('t', 33, 34)]), dayOf('Wed 9', [event('w', 57, 58)])]);
  expect(titles(once)).toEqual(['EARLIER TODAY', 'TOMORROW']);
  expect(titles(plan({ earlier: 'off' }, [dayOf('Mon 7', spent), dayOf('Tue 8', [])]))).toEqual([]);
});


test('popup view changes preserve settings changed since its last feed snapshot', async ({ page }) => {
  await popup(page);
  await expect(page.getByRole('button', {name: 'Day', exact: true})).toBeVisible();
  await page.evaluate(() => (window as any).__changeOtherPreferences());
  await page.getByRole('button', {name: 'Day', exact: true}).click();
  await expect(page.getByLabel('Day calendar')).toBeVisible();
  expect(await page.evaluate(() => (window as any).__menuPreferences())).toEqual({day: true, label: false, join: 15});
});

test('day scrolling accounts for all-day rows and leaves tasks in the same scroll area', async ({ page }) => {
  await popup(page, 0, 24, {allDayCount: 6, tasks: true});
  await page.getByRole('button', {name: 'Day', exact: true}).click();
  const scroll = page.locator('.scroll');
  await expect(page.getByLabel('Day calendar')).toBeVisible();
  const top = (await scroll.boundingBox())!.y;
  await expect(page.locator('.now-line')).toHaveText('13:21');
  const marker = await page.locator('.now-line').boundingBox();
  expect(marker!.y - top).toBeCloseTo(100, 0);
  await scroll.evaluate(el => { el.scrollTop = el.scrollHeight; });
  const task = await page.getByRole('button', {name: 'Submit expenses', exact: true}).boundingBox();
  const viewport = await scroll.boundingBox();
  expect(task!.y).toBeGreaterThanOrEqual(viewport!.y);
  expect(task!.y + task!.height).toBeLessThanOrEqual(viewport!.y + viewport!.height);
});

test('back-to-back short meetings do not cover each other at their minimum readable height', async ({ page }) => {
  await popup(page, 12, 23, {shortMeetings: true});
  await page.getByRole('button', {name: 'Day', exact: true}).click();
  const first = page.locator('.event').filter({hasText: 'Quick one'});
  const second = page.locator('.event').filter({hasText: 'Quick two'});
  await first.scrollIntoViewIfNeeded();
  const a = (await first.boundingBox())!, b = (await second.boundingBox())!;
  expect(a.height).toBeGreaterThanOrEqual(22);
  expect(a.y + a.height <= b.y || a.x + a.width <= b.x || b.x + b.width <= a.x).toBe(true);
  await expect(first).toContainText('15:00');
  await expect(second).toContainText('15:15');
});
