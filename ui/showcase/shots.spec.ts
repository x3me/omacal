/**
 * The website's screenshots. Renders the real `App` in the test harness from
 * payloads the real backend produced (`dump.sh`: the demo seed at a fixed
 * moment, plus real Omarchy palettes), so every lane, overflow and colour is
 * the app's own. Run: `showcase/dump.sh && npx playwright test -c
 * showcase/playwright.config.ts`. PNGs land in target/showcase/shots.
 */
import { test, type Page } from '@playwright/test';
import { readFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';

const DATA = process.env.SHOWCASE_DATA ?? join(import.meta.dirname, '../../target/showcase');
const OUT = process.env.SHOWCASE_OUT ?? join(DATA, 'shots');
mkdirSync(OUT, { recursive: true });

const load = (name: string) => JSON.parse(readFileSync(join(DATA, `${name}.json`), 'utf8'));
const meta = load('meta') as { now: number; tz: string; weekStart: number; year: number; month: number };
const palettes = load('palettes') as Record<string, unknown>;

const DAY = 24 * 3_600_000;
const isoDay = (ms: number) => new Date(ms).toLocaleDateString('en-CA', { timeZone: meta.tz });

/** A week of weather, one sky per day, so every header carries one. */
function weather() {
  const skies = [
    ['clear', 19, 9], ['partly_cloudy', 17, 10], ['clear', 20, 8], ['cloudy', 15, 9],
    ['drizzle', 13, 8], ['partly_cloudy', 16, 7], ['clear', 18, 6],
  ] as const;
  return {
    days: skies.map(([bucket, tmax, tmin], i) => ({
      date: isoDay(meta.weekStart + i * DAY + 12 * 3_600_000), bucket, tmax, tmin,
      rain_chance: bucket === 'drizzle' ? 70 : 10, wind_max_kmh: 12, sunrise: '07:31', sunset: '18:27',
    })),
    place: 'Berlin', source: 'settings',
    current: { bucket: 'clear', temp: 16.2, feels: 15.4, humidity: 61, wind_kmh: 8, at: `${isoDay(meta.now)}T11:30` },
    fetched_at: meta.now - 10 * 60_000,
  };
}

/** The stub's own settings store, pre-filled: a working day's hours. */
const SETTINGS = { visibleStartHour: 8, visibleEndHour: 19, hourHeight: 88 };

async function open(page: Page, theme = 'tokyo-night', settings: Record<string, unknown> = {}) {
  const canned = {
    get_palette: palettes[theme],
    get_week: load('week'), get_range: load('week'), get_day: load('day'),
    get_month: load('month'), get_big_year: load('bigyear'),
    get_calendars: load('calendars'), list_tasks: load('tasks'),
    // The demo's two lists, as `task_lists` names a local account's: the
    // demo account is not one the command lists, so it is spelled here.
    task_lists: [
      { calendarId: 3, name: 'Tasks', color: '#e2a03f', local: true },
      { calendarId: 4, name: 'Home', color: '#c678dd', local: true },
    ],
    // Your own row reads as a person on the website, not as the demo seed.
    event_detail: {
      ...load('event_detail'),
      attendees: load('event_detail').attendees.map((a: { self?: boolean; is_self?: boolean; display_name?: string }) =>
        (a.self || a.is_self) ? { ...a, display_name: 'Alex' } : a),
    },
    get_weather: weather(),
    get_status: {
      accounts: ['you@example.com'], needs_reauth: [], update: null, system_tz_change: null,
      version: '5.1.3', last_sync_ms: meta.now - 60_000, demo: false, overlay_titlebar: false, self_update: false,
    },
    pending_invites: [],
  };
  await page.addInitScript(([c, s]) => {
    (window as any).__canned = c;
    sessionStorage.setItem('omacal-stub-settings', JSON.stringify(s));
  }, [canned, { ...SETTINGS, ...settings }] as const);
  await page.clock.setFixedTime(meta.now);
  await page.goto('/tests/harness/index.html?c=App&f=showcase');
  await page.waitForLoadState('networkidle');
  await page.waitForTimeout(700);
}

const shot = (page: Page, name: string, clip?: { x: number; y: number; width: number; height: number }) =>
  page.screenshot({ path: join(OUT, `${name}.png`), clip });

test('week', async ({ page }) => {
  await open(page);
  await shot(page, 'week');
});

test('tasks', async ({ page }) => {
  await open(page);
  await page.getByRole('button', { name: 'Menu' }).click();
  await page.getByRole('button', { name: 'Tasks…' }).click();
  await page.waitForTimeout(400);
  await shot(page, 'tasks');
});

test('month', async ({ page }) => {
  await open(page);
  await page.keyboard.press('3');
  await page.waitForTimeout(500);
  await shot(page, 'month');
});

test('bigyear', async ({ page }) => {
  await open(page);
  await page.keyboard.press('5');
  await page.waitForTimeout(500);
  await shot(page, 'bigyear');
});

for (const theme of Object.keys(palettes)) {
  test(`theme ${theme}`, async ({ page }) => {
    await open(page, theme);
    await shot(page, `theme-${theme}`);
  });
}

/** A dialog on its own, with room for its shadow, at twice the pixels. */
async function crop(page: Page, name: string, dialog: import('@playwright/test').Locator, pad = 28) {
  const b = (await dialog.boundingBox())!;
  const x = Math.max(0, b.x - pad), y = Math.max(0, b.y - pad);
  await shot(page, name, { x, y, width: b.width + 2 * pad, height: b.height + 2 * pad });
}

test.describe('details', () => {
  test.use({ deviceScaleFactor: 2 });

  test('event popover', async ({ page }) => {
    await open(page);
    await page.getByRole('button', { name: /Excitel weekly/ }).first().click();
    const dialog = page.getByRole('dialog', { name: /Excitel weekly/ });
    await dialog.waitFor();
    await page.waitForTimeout(300);
    await crop(page, 'popover', dialog);
  });

  test('weather card', async ({ page }) => {
    await open(page);
    await page.locator('.wx').nth(2).click();
    const dialog = page.getByRole('dialog').first();
    await dialog.waitFor();
    await page.waitForTimeout(300);
    await crop(page, 'weather', dialog);
  });

  test('quick add', async ({ page }) => {
    await open(page);
    await page.keyboard.press('q');
    const quick = page.getByRole('dialog', { name: 'Quick add event' });
    await quick.getByLabel('Describe the event').fill('Lunch with Ana tomorrow at 1pm for an hour invite ana@example.com');
    await page.waitForTimeout(400);
    await crop(page, 'quickadd', quick);
  });
});
