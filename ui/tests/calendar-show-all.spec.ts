// ui/tests/calendar-show-all.spec.ts
// One switch over every calendar's box (#171): "Hide all" while everything is
// shown, "Show all" as soon as anything is hidden, the mixed state drawn as
// such. It only hides — the same `selected` a row's box moves, through the
// one-transaction `set_calendars_shown` a calendar group applies with — and
// only the calendars whose own box can be ticked.
import { test, expect, type Page } from '@playwright/test';
import { APP_NOW } from './fixtures';

const app = '/tests/harness/index.html?c=App&f=calendar-groups';
const shownCalls = (page: Page) =>
  page.evaluate(() => window.__harness.calls.filter((c) => c.cmd === 'set_calendars_shown').map((c) => (c.args as any).ids));
const callsTo = (page: Page, cmd: string) =>
  page.evaluate((cmd) => window.__harness.calls.filter((c) => c.cmd === cmd).length, cmd);

/** Menu → Calendars, open; resolves to its panel. */
async function menuCalendars(page: Page) {
  await page.getByRole('button', { name: 'Menu', exact: true }).click();
  await page.getByRole('button', { name: /Calendars/ }).click();
  return page.getByRole('group', { name: 'Calendars', exact: true });
}

const ROWS = ['Holidays in Bulgaria', 'Team', 'Personal'];

test.describe('Hide all / Show all in the calendar list', () => {
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(APP_NOW);
    await page.goto(app);
  });

  test('with everything shown it is ticked and hides them all; then it shows them all again', async ({ page }) => {
    const panel = await menuCalendars(page);
    const all = panel.getByRole('checkbox', { name: 'Hide all', exact: true });
    await expect(all).toBeChecked();

    await all.click();
    expect((await shownCalls(page)).slice(-1)[0]).toEqual([]);
    for (const name of ROWS) await expect(panel.getByRole('checkbox', { name, exact: true })).not.toBeChecked();
    // The local list's box cannot be ticked, so the switch does not reach it.
    await expect(panel.getByRole('checkbox', { name: 'Chores', exact: true })).toBeChecked();

    const back = panel.getByRole('checkbox', { name: 'Show all', exact: true });
    await expect(back).not.toBeChecked();
    expect(await back.evaluate((el: HTMLInputElement) => el.indeterminate), 'none shown is not mixed').toBe(false);
    await back.click();
    expect((await shownCalls(page)).slice(-1)[0]).toEqual([7, 8, 9]);
    for (const name of ROWS) await expect(panel.getByRole('checkbox', { name, exact: true })).toBeChecked();
    await expect(panel.getByRole('checkbox', { name: 'Hide all', exact: true })).toBeChecked();
  });

  test('with some hidden it is mixed, drawn as mixed, and shows them all', async ({ page }) => {
    const panel = await menuCalendars(page);
    await panel.getByRole('checkbox', { name: 'Team', exact: true }).click();

    const all = panel.getByRole('checkbox', { name: 'Show all', exact: true });
    expect(await all.evaluate((el: HTMLInputElement) => el.indeterminate)).toBe(true);
    // Drawn, not left looking empty: the app paints its own boxes, and a
    // mixed one wears the accent a ticked one does.
    const fill = (name: string) =>
      panel.getByRole('checkbox', { name, exact: true }).evaluate((el) => getComputedStyle(el).backgroundColor);
    expect(await fill('Show all')).toBe(await fill('Personal'));
    expect(await fill('Show all')).not.toBe(await fill('Team'));

    await all.click();
    expect((await shownCalls(page)).slice(-1)[0]).toEqual([7, 8, 9]);
    await expect(panel.getByRole('checkbox', { name: 'Team', exact: true })).toBeChecked();
    await expect(panel.getByRole('checkbox', { name: 'Hide all', exact: true })).toBeChecked();
  });

  test('it only hides: nothing stops syncing', async ({ page }) => {
    const panel = await menuCalendars(page);
    await panel.getByRole('checkbox', { name: 'Hide all', exact: true }).click();
    await panel.getByRole('checkbox', { name: 'Show all', exact: true }).click();
    expect(await callsTo(page, 'set_calendar_sync')).toBe(0);
    await expect(panel.getByRole('button', { name: 'Remove' })).toHaveCount(3);
  });

  test('a failed switch puts the box back and says so', async ({ page }) => {
    await page.evaluate(() => window.__harness.failNextCalendarCall('set_calendars_shown', 'database is locked'));
    const panel = await menuCalendars(page);
    const all = panel.getByRole('checkbox', { name: 'Hide all', exact: true });
    await all.click();
    await expect(panel).toContainText('All calendars · database is locked');
    await expect(all).toBeChecked();
    for (const name of ROWS) await expect(panel.getByRole('checkbox', { name, exact: true })).toBeChecked();
  });

  test('while it is in flight the rows wait, and while a row is in flight it waits', async ({ page }) => {
    const panel = await menuCalendars(page);
    await page.evaluate(() => window.__harness.holdNextCalendarCall('set_calendars_shown'));
    await panel.getByRole('checkbox', { name: 'Hide all', exact: true }).click();
    for (const name of ROWS) await expect(panel.getByRole('checkbox', { name, exact: true })).toBeDisabled();
    await page.evaluate(() => window.__harness.releaseCalendarCall('set_calendars_shown', undefined));
    for (const name of ROWS) await expect(panel.getByRole('checkbox', { name, exact: true })).toBeEnabled();

    await page.evaluate(() => window.__harness.holdNextCalendarCall('set_calendar_selected'));
    await panel.getByRole('checkbox', { name: 'Team', exact: true }).click();
    await expect(panel.getByRole('checkbox', { name: 'Show all', exact: true })).toBeDisabled();
    await page.evaluate(() => window.__harness.releaseCalendarCall('set_calendar_selected', undefined));
    await expect(panel.getByRole('checkbox', { name: 'Show all', exact: true })).toBeEnabled();
  });

  test('Settings → Calendars carries it too', async ({ page }) => {
    await page.getByRole('button', { name: 'Menu', exact: true }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    await modal.getByRole('tab', { name: 'Calendars' }).click();
    await modal.getByRole('checkbox', { name: 'Hide all', exact: true }).click();
    expect((await shownCalls(page)).slice(-1)[0]).toEqual([]);
  });
});

test('one calendar to tick has no switch over it', async ({ page }) => {
  await page.goto('/tests/harness/index.html?c=CalendarPopover&f=single');
  await page.getByRole('button', { name: /Calendars/ }).click();
  await expect(page.locator('input[type=checkbox]')).toHaveCount(1);
  await expect(page.getByRole('checkbox', { name: /^(Hide|Show) all$/ })).toHaveCount(0);
});
