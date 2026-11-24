import { test, expect } from '@playwright/test';
import { isoWeek, rangeWeeks, rowWeek } from '../src/lib/weeknumbers';
import { APP_NOW, appJanuaryMonth } from './fixtures';

const app = '/tests/harness/index.html?c=App&f=default';

test('ISO numbers follow Thursday across year boundaries and rolling ranges', () => {
  expect(isoWeek(new Date(2021, 0, 1))).toEqual({ year: 2020, week: 53 });
  expect(isoWeek(new Date(2024, 0, 1))).toEqual({ year: 2024, week: 1 });
  expect(rowWeek(new Date(2024, 0, 28).getTime())).toBe(5); // Sunday through Saturday
  expect(rowWeek(new Date(2024, 0, 27).getTime())).toBe(5); // Saturday through Friday
  expect(rangeWeeks(new Date(2024, 0, 26).getTime(), 3)).toBe('Week 4');
  expect(rangeWeeks(new Date(2024, 0, 26).getTime(), 5)).toBe('Weeks 4–5');
  expect(rangeWeeks(new Date(2023, 11, 29).getTime(), 5)).toBe('Weeks 52–1');
});

test('Appearance enables and persists the Sunday week and Month row numbers', async ({ page }) => {
  await page.clock.setFixedTime(APP_NOW);
  await page.addInitScript(() => {
    if (!sessionStorage.getItem('omacal-stub-settings')) {
      sessionStorage.setItem('omacal-stub-settings', JSON.stringify({ weekStart: 'sunday' }));
    }
  });
  await page.goto(app);
  await expect(page.locator('[data-testid="week-body"] .col')).toHaveCount(7);
  await expect(page.locator('[data-week-number]')).toHaveCount(0);
  await page.getByRole('button', { name: 'Menu' }).click();
  await page.getByRole('button', { name: 'Settings…' }).click();
  const modal = page.getByRole('dialog', { name: 'Settings' });
  await modal.getByRole('tab', { name: 'Appearance' }).click();
  const setting = modal.getByRole('checkbox', { name: 'Show week numbers' });
  await expect(setting).not.toBeChecked();
  await setting.check();
  await expect.poll(() => page.evaluate(() => window.__harness.settingValues('showWeekNumbers'))).toEqual([true]);
  await expect(page.locator('header [data-week-number]')).toHaveText('Week 5');
  await page.keyboard.press('Escape');
  await page.evaluate((month) => window.__harness.setResponseData({ month }), appJanuaryMonth());
  await page.getByRole('group', { name: 'View' }).getByRole('button', { name: 'Month', exact: true }).click();
  await expect(page.locator('.mrow [data-week-number]')).toHaveText(['1', '2', '3', '4', '5', '6']);
  await expect(page.getByRole('button', { name: 'Board prep' })).toBeVisible();
  await page.reload();
  await page.getByRole('group', { name: 'View' }).getByRole('button', { name: 'Week', exact: true }).click();
  await expect(page.locator('header [data-week-number]')).toHaveText('Week 5');
});

test('rolling range shows both numbers only when it crosses an ISO boundary', async ({ page }) => {
  await page.clock.setFixedTime(new Date('2024-01-26T12:00:00Z'));
  await page.addInitScript(() => sessionStorage.setItem('omacal-stub-settings', JSON.stringify({
    showWeekNumbers: true, weekStartsToday: true, weekViewDays: 5,
  })));
  await page.goto(app);
  await expect(page.locator('header [data-week-number]')).toHaveText('Weeks 4–5');
});

for (const [days, expected] of [[3, 'Week 4'], [7, 'Weeks 4–5']] as const) {
  test(`a rolling ${days}-day view shows ${expected}`, async ({ page }) => {
    await page.clock.setFixedTime(new Date('2024-01-26T12:00:00Z'));
    await page.addInitScript((count) => sessionStorage.setItem('omacal-stub-settings', JSON.stringify({
      showWeekNumbers: true, weekStartsToday: true, weekViewDays: count,
    })), days);
    await page.goto(app);
    await expect(page.locator('header [data-week-number]')).toHaveText(expected);
  });
}

test('Year week labels align with real day rows and keep day buttons usable', async ({ page }) => {
  await page.goto('/tests/harness/index.html?c=YearGrid&f=y2026-weeks');
  const january = page.locator('.ymonth').first();
  await expect(january.locator('[data-week-number]')).toHaveText(['1', '2', '3', '4', '5']);
  const number = await january.locator('[data-week-number]').first().boundingBox();
  const day = await january.getByRole('button', { name: '1', exact: true }).boundingBox();
  expect(number).not.toBeNull();
  expect(day).not.toBeNull();
  expect(number!.x + number!.width).toBeLessThanOrEqual(day!.x);
  expect(Math.abs(number!.y + number!.height / 2 - (day!.y + day!.height / 2))).toBeLessThan(2);
  await january.getByRole('button', { name: '1', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__lastDayPick)).toBe(Date.UTC(2026, 0, 1));
});

test('panning a fixed week updates the range in the header', async ({ page }) => {
  await page.clock.setFixedTime(APP_NOW);
  await page.addInitScript(() => sessionStorage.setItem('omacal-stub-settings', JSON.stringify({ showWeekNumbers: true })));
  await page.goto(app);
  const body = page.locator('[data-testid="week-body"]');
  const first = body.locator('.col').first();
  await expect(first).toHaveAttribute('data-start-ms', String(Date.UTC(2024, 0, 29)));
  await expect(page.locator('header [data-week-number]')).toHaveText('Week 5');
  const width = (await first.boundingBox())!.width;
  await body.hover();
  await page.mouse.wheel(Math.round(width * 0.65), 0);
  await expect(page.locator('header [data-week-number]')).toHaveText('Weeks 5–6');
});

/** A strong swipe turns a whole week and leaves the window aligned, so it is
 *  named like any other week: with a Sunday start, Sun 4 – Sat 10 Feb is
 *  "Week 6", as › reaches it, not "Weeks 5–6" (the panned window's range). */
test('a whole-week swipe on a Sunday week is still one week number', async ({ page }) => {
  await page.clock.setFixedTime(APP_NOW);
  await page.addInitScript(() => sessionStorage.setItem('omacal-stub-settings', JSON.stringify({
    showWeekNumbers: true, weekStart: 'sunday',
  })));
  await page.goto(app);
  const first = page.locator('[data-testid="week-body"] .col').first();
  await expect(first).toHaveAttribute('data-start-ms', String(Date.UTC(2024, 0, 28)));
  await expect(page.locator('header [data-week-number]')).toHaveText('Week 5');
  await page.evaluate(() => {
    const body = document.querySelector('[data-testid="week-body"]')!;
    const r = body.getBoundingClientRect();
    for (let i = 0; i < 4; i++) {
      body.dispatchEvent(new WheelEvent('wheel', {
        deltaX: 60, deltaY: 0, clientX: r.left + r.width / 2, clientY: r.top + 100, bubbles: true, cancelable: true,
      }));
      const t = performance.now();
      while (performance.now() - t < 8) { /* busy-wait: a strong flick's spacing */ }
    }
  });
  await expect(first).toHaveAttribute('data-start-ms', String(Date.UTC(2024, 1, 4)));
  await expect(page.locator('header [data-week-number]')).toHaveText('Week 6');
});

test('a rolling window starting on a hidden weekend uses the first displayed day', async ({ page }) => {
  await page.clock.setFixedTime(new Date('2024-01-28T12:00:00Z'));
  await page.addInitScript(() => sessionStorage.setItem('omacal-stub-settings', JSON.stringify({
    showWeekNumbers: true, weekStartsToday: true, weekViewDays: 3, hideWeekends: true,
  })));
  await page.goto(app);
  await expect(page.locator('header [data-week-number]')).toHaveText('Week 5');
});
