// A calendar with no colour of its own — a CalDAV task list from a server that
// publishes no `calendar-color` — is drawn in `--accent` by the Calendars
// list. The Tasks pane, the TASKS band and Big Year's legend used to fall back
// to `--muted` instead, so the same list was one colour in Calendars and
// another everywhere else, until the user picked a colour. Every surface asks
// `calendarInk` now; this holds them to the swatch the list draws.

import { test, expect, type Page } from '@playwright/test';
import { APP_NOW } from './fixtures';
import { calendarInk } from '../src/lib/calendars';

const app = '/tests/harness/index.html?c=App&f=uncoloured-lists';

/** A colour expression as the browser computes it, so `var(--accent)` and a
 *  resolved rgb() compare equal. */
const computed = (page: Page, css: string) => page.evaluate((c) => {
  const probe = document.createElement('i');
  probe.style.color = c;
  document.body.append(probe);
  const out = getComputedStyle(probe).color;
  probe.remove();
  return out;
}, css);

const bg = (loc: ReturnType<Page['locator']>) =>
  loc.evaluate((el) => getComputedStyle(el).backgroundColor);

test('the colour a calendar is drawn in is its own, or --accent when it has none', () => {
  expect(calendarInk('#2dd4bf')).toBe('#2dd4bf');
  expect(calendarInk(null)).toBe('var(--accent)');
  expect(calendarInk(undefined)).toBe('var(--accent)');
  expect(calendarInk('')).toBe('var(--accent)');
});

test.describe('an uncoloured list', () => {
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(APP_NOW);
    await page.goto(app);
  });

  test('is --accent in Calendars, the Tasks pane and the TASKS band alike', async ({ page }) => {
    const accent = await computed(page, 'var(--accent)');
    expect(accent).not.toBe(await computed(page, 'var(--muted)'));

    // The swatch Calendars draws: the colour this list has always had.
    await page.getByRole('button', { name: 'Menu', exact: true }).click();
    await page.getByRole('button', { name: /Calendars/ }).click();
    const swatch = page.getByRole('button', { name: 'Colour for Work' });
    await expect(swatch).toBeVisible();
    expect(await bg(swatch)).toBe(accent);
    await page.keyboard.press('Escape');
    await page.keyboard.press('Escape');

    // The Tasks pane: the row's tick.
    await page.getByRole('button', { name: 'Menu', exact: true }).click();
    await page.getByRole('button', { name: 'Tasks…' }).click();
    const side = page.getByRole('complementary', { name: 'Tasks' });
    expect(await bg(side.locator('.row', { hasText: 'Ship the release' }).locator('.tick'))).toBe(accent);

    // The TASKS band: the colour the chip is tinted from.
    const chip = page.locator('.trow .tchip', { hasText: 'Ship the release' });
    await expect(chip).toBeVisible();
    const [cal, acc] = await chip.evaluate((el) => {
      const s = getComputedStyle(el);
      return [s.getPropertyValue('--cal').trim(), s.getPropertyValue('--accent').trim()];
    });
    expect(acc).not.toBe('');
    expect(cal).toBe(acc);
  });

  test('wears the list swatch under By list too', async ({ page }) => {
    const accent = await computed(page, 'var(--accent)');
    await page.getByRole('button', { name: 'Menu', exact: true }).click();
    await page.getByRole('button', { name: 'Tasks…' }).click();
    const side = page.getByRole('complementary', { name: 'Tasks' });
    await side.getByRole('button', { name: 'By list' }).click();
    const head = side.locator('.head.listhead', { hasText: 'Work' });
    await expect(head).toBeVisible();
    expect(await bg(head.locator('.tick').first())).toBe(accent);
  });

  test('is --accent in Big Year\'s legend', async ({ page }) => {
    const accent = await computed(page, 'var(--accent)');
    await expect(page.locator('.vswitch button')).toHaveCount(5); // mount race, as app.spec.ts's legend spec
    await page.keyboard.press('5');
    const dot = page.locator('.legend .item', { hasText: 'Work' }).locator('.dot');
    await expect(dot).toBeVisible();
    expect(await bg(dot)).toBe(accent);
  });
});
