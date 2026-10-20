// Calendar groups: a saved set of which calendars are shown, switched from the
// header or with ⌘1–⌘9. The helpers are pure and run without a page; the rest
// drives the real App against the `calendar-groups` scenario, whose stub keeps
// each calendar's `selected` flag so a switch is visible in the label.

import { test, expect, type Page } from '@playwright/test';
import { APP_NOW } from './fixtures';
import { activeGroupIndex, applyRefusal, groupIndexForCode, liveIds, shownIds } from '../src/lib/calendargroups';
import type { Calendar } from '../src/lib/calendars';

const cal = (id: number, selected: boolean, sync_enabled = true) => ({ id, selected, sync_enabled }) as Calendar;

test.describe('calendar group helpers', () => {
  test('the active group is the one whose calendars are exactly the ones shown', () => {
    const cals = [cal(1, true), cal(2, true), cal(3, false)];
    expect(shownIds(cals)).toEqual([1, 2]);
    const groups = [
      { name: 'One', calendarIds: [1] },
      { name: 'Two', calendarIds: [2, 1] },
    ];
    expect(activeGroupIndex(groups, cals)).toBe(1);
    expect(activeGroupIndex(groups, [cal(1, true), cal(2, false), cal(3, false)])).toBe(0);
    expect(activeGroupIndex(groups, [cal(1, true), cal(2, true), cal(3, true)])).toBe(-1);
    expect(activeGroupIndex([], cals)).toBe(-1);
  });

  test('an id that names no calendar is ignored, on both sides of the comparison', () => {
    const cals = [cal(1, true), cal(2, false)];
    const group = { name: 'Gone', calendarIds: [1, 99] };
    expect(liveIds(group, cals)).toEqual([1]);
    expect(activeGroupIndex([group], cals)).toBe(0);
  });

  test('only calendars the picker can toggle count, for shown, live and active', () => {
    // 3 is a local list or a removed calendar: drawn, but its checkbox is disabled.
    const cals = [cal(1, true), cal(2, false), cal(3, true, false), cal(4, false, false)];
    expect(shownIds(cals)).toEqual([1]);
    const group = { name: 'One', calendarIds: [1, 3, 4] };
    expect(liveIds(group, cals)).toEqual([1]);
    // 3 being drawn does not stop the group matching: it is not the picker's to show.
    expect(activeGroupIndex([group], cals)).toBe(0);
    expect(activeGroupIndex([{ name: 'Two', calendarIds: [1, 2] }], cals)).toBe(-1);
  });

  test('a group whose calendars are all gone is refused, not applied as "hide everything"', () => {
    const cals = [cal(1, true), cal(3, true, false)];
    expect(applyRefusal({ name: 'Old', calendarIds: [5, 6] }, cals)).toMatch(/gone/);
    expect(applyRefusal({ name: 'Off', calendarIds: [3] }, cals)).toMatch(/gone/);
    expect(applyRefusal({ name: 'Part', calendarIds: [1, 6] }, cals)).toBeNull();
    expect(applyRefusal({ name: 'Empty', calendarIds: [] }, cals)).toBeNull();
  });

  test('the chord reads the physical digit key, not what the layout writes', () => {
    expect(groupIndexForCode('Digit1')).toBe(0);
    expect(groupIndexForCode('Digit9')).toBe(8);
    expect(groupIndexForCode('Digit0')).toBeNull();
    expect(groupIndexForCode('Numpad1')).toBeNull();
    expect(groupIndexForCode('KeyA')).toBeNull();
  });
});

const app = '/tests/harness/index.html?c=App&f=calendar-groups';
const trigger = (page: Page) => page.locator('button.cg-trigger');
const shownCalls = (page: Page) =>
  page.evaluate(() => window.__harness.calls.filter((c) => c.cmd === 'set_calendars_shown').map((c) => (c.args as any).ids));

async function saveGroup(page: Page, name: string) {
  await trigger(page).click();
  await page.getByRole('button', { name: 'Save current as group…' }).click();
  await page.getByLabel('New group name').fill(name);
  await page.getByLabel('New group name').press('Enter');
  await expect(page.getByLabel('New group name')).toHaveCount(0);
  await page.keyboard.press('Escape');
}

async function toggleTeam(page: Page) {
  await page.getByRole('button', { name: 'Menu', exact: true }).click();
  await page.getByRole('button', { name: /Calendars/ }).click();
  await page.getByRole('checkbox', { name: /Team/ }).first().click();
  await page.keyboard.press('Escape');
  await page.keyboard.press('Escape');
}

test.describe('calendar groups in the header', () => {
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(APP_NOW);
    await page.goto(app);
  });

  test('save the shown calendars, then switch by click and by the chord', async ({ page }) => {
    // Nothing saved: everything is shown and no group says so.
    await expect(trigger(page)).toContainText('All calendars');

    await saveGroup(page, '  Everything ');
    await expect(trigger(page)).toContainText('Everything');
    expect(await page.evaluate(() => window.__harness.settingValues('calendarGroups').slice(-1)[0]))
      .toEqual([{ name: 'Everything', calendarIds: [7, 8, 9] }]);

    // Hide one by hand: no group matches any more.
    await toggleTeam(page);
    await expect(trigger(page)).toContainText('Custom');
    await saveGroup(page, 'Focus');
    await expect(trigger(page)).toContainText('Focus');

    // By click: the first group shows all three again, in one call.
    await trigger(page).click();
    const everything = page.locator('.cg-pick', { hasText: 'Everything' });
    await expect(everything).toContainText(/(⌘|Ctrl)1/);
    await everything.click();
    await expect(trigger(page)).toContainText('Everything');
    expect((await shownCalls(page)).slice(-1)[0]).toEqual([7, 8, 9]);

    // By the chord, with nothing open. Digit2 is the second group.
    await page.keyboard.press('Control+2');
    await expect(trigger(page)).toContainText('Focus');
    expect((await shownCalls(page)).slice(-1)[0]).toEqual([7, 9]);

    // A digit with no group behind it does nothing.
    const before = (await shownCalls(page)).length;
    await page.keyboard.press('Control+5');
    expect((await shownCalls(page)).length).toBe(before);
    await page.keyboard.press('Control+1');
    await expect(trigger(page)).toContainText('Everything');
  });

  test('rename, update to the shown calendars, and delete', async ({ page }) => {
    await saveGroup(page, 'Everything');
    await toggleTeam(page);

    await trigger(page).click();
    await page.getByRole('button', { name: 'Update Everything to the calendars shown now' }).click();
    await expect.poll(() => page.evaluate(() => window.__harness.settingValues('calendarGroups').slice(-1)[0]))
      .toEqual([{ name: 'Everything', calendarIds: [7, 9] }]);
    // It matches what is shown, so it is the active one again.
    await expect(trigger(page)).toContainText('Everything');

    await page.getByRole('button', { name: 'Rename Everything' }).click();
    await page.getByLabel('Group name').fill('Lean');
    await page.getByLabel('Group name').press('Enter');
    await expect(trigger(page)).toContainText('Lean');

    // A duplicate name is the backend's to refuse; the stub does not, so this
    // part of the contract is the Rust test's. Delete asks first.
    await page.getByRole('button', { name: 'Delete Lean' }).click();
    await page.getByRole('button', { name: 'Cancel' }).click();
    await expect(page.getByRole('button', { name: 'Delete Lean' })).toBeVisible();
    await page.getByRole('button', { name: 'Delete Lean' }).click();
    await page.getByRole('button', { name: 'Delete', exact: true }).click();
    await expect(page.getByText('No groups yet.')).toBeVisible();
    await expect(trigger(page)).toContainText('Custom');
  });

  test('the switcher carries the calendar toggles: tick one there, then save the group', async ({ page }) => {
    await trigger(page).click();
    await expect(page.getByText('No groups yet. Pick the calendars below, then save them as a group.')).toBeVisible();
    const panel = page.getByRole('group', { name: 'Calendar groups' });
    // The same rows as Menu → Calendars, with no menu opened.
    await panel.getByRole('checkbox', { name: /Team/ }).click();
    // The panel stays open and the button follows the box.
    await expect(panel).toBeVisible();
    await expect(trigger(page)).toContainText('Custom');

    await panel.getByRole('button', { name: 'Save current as group…' }).click();
    await page.getByLabel('New group name').fill('No team');
    await page.getByLabel('New group name').press('Enter');
    await expect(page.getByLabel('New group name')).toHaveCount(0);
    await expect(trigger(page)).toContainText('No team');
    expect(await page.evaluate(() => window.__harness.settingValues('calendarGroups').slice(-1)[0]))
      .toEqual([{ name: 'No team', calendarIds: [7, 9] }]);

    // Ticking it back moves the label off the group again.
    await panel.getByRole('checkbox', { name: /Team/ }).click();
    await expect(trigger(page)).toContainText('All calendars');
    // Escape closes the panel; the list is not a second layer to dismiss first.
    await page.keyboard.press('Escape');
    await expect(panel).toHaveCount(0);
  });

  test('a local list is neither saved into a group nor hidden by one', async ({ page }) => {
    // Save with the local list drawn: its id (40) is not captured.
    await saveGroup(page, 'Everything');
    expect(await page.evaluate(() => window.__harness.settingValues('calendarGroups').slice(-1)[0]))
      .toEqual([{ name: 'Everything', calendarIds: [7, 8, 9] }]);
    await toggleTeam(page);
    await page.keyboard.press('Control+1');
    await expect(trigger(page)).toContainText('Everything');
    // The apply is still sent only the live ids; the stub keeps the list on.
    expect((await shownCalls(page)).slice(-1)[0]).toEqual([7, 8, 9]);
  });

  test('a group with no live calendars is refused, by click and by the chord', async ({ page }) => {
    await saveGroup(page, 'Old');
    // Rewrite it to ids that name nothing, as a reconnected account leaves it.
    await page.evaluate(() => {
      const raw = JSON.parse(sessionStorage.getItem('omacal-stub-settings') ?? '{}');
      raw.calendarGroups = [{ name: 'Old', calendarIds: [101, 102] }];
      sessionStorage.setItem('omacal-stub-settings', JSON.stringify(raw));
    });
    await page.reload();
    await expect(trigger(page)).toBeVisible();
    const before = (await shownCalls(page)).length;
    await page.keyboard.press('Control+1');
    await expect(page.getByText(/calendars are gone/)).toBeVisible();
    expect((await shownCalls(page)).length).toBe(before);
    // And from the panel.
    await trigger(page).click();
    await page.locator('.cg-pick', { hasText: 'Old' }).click();
    expect((await shownCalls(page)).length).toBe(before);
    await expect(page.getByText(/Old · This group.s calendars are gone/)).toBeVisible();
  });

  test('saving is refused when no calendar is shown', async ({ page }) => {
    await page.getByRole('button', { name: 'Menu', exact: true }).click();
    await page.getByRole('button', { name: /Calendars/ }).click();
    for (const name of [/Holidays/, /Team/, /Personal/]) await page.getByRole('checkbox', { name }).first().click();
    await page.keyboard.press('Escape');
    await page.keyboard.press('Escape');
    await trigger(page).click();
    await expect(page.getByRole('button', { name: 'Save current as group…' })).toBeDisabled();
    await expect(page.getByText('Show at least one calendar to save a group.')).toBeVisible();
  });
});
