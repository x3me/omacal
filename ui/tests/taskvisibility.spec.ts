// A calendar's show/hide switch decides what `list_tasks` answers (the backend
// joins on `selected`), so every way of flipping it has to re-read the tasks,
// not just the events: the TASKS row and the sidebar draw from one shared
// list. The `task-visibility` scenario's stub keeps each calendar's
// `selected` flag and filters `list_tasks` by it, as `tasks_for_ui` does.

import { test, expect, type Page } from '@playwright/test';
import { APP_NOW } from './fixtures';

const app = '/tests/harness/index.html?c=App&f=task-visibility';
const side = (page: Page) => page.getByRole('complementary', { name: 'Tasks' });
const onGrid = (page: Page, title: string) => page.locator('.trow .tchip .tt', { hasText: title });

test.describe('tasks follow the calendars shown', () => {
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(APP_NOW);
    await page.goto(app);
    await page.getByRole('button', { name: 'Menu', exact: true }).click();
    await page.getByRole('button', { name: 'Tasks…' }).click();
    await expect(side(page)).toBeVisible();
    await expect(side(page).getByText('Team errand')).toBeVisible();
    await expect(onGrid(page, 'Team errand')).toBeVisible();
    await expect(onGrid(page, 'Home errand')).toBeVisible();
  });

  const gone = async (page: Page) => {
    await expect(onGrid(page, 'Team errand')).toHaveCount(0);
    await expect(side(page).getByText('Team errand')).toHaveCount(0);
    // The other calendar's task is untouched.
    await expect(onGrid(page, 'Home errand')).toBeVisible();
    await expect(side(page).getByText('Home errand')).toBeVisible();
  };

  test('hiding a calendar in Menu → Calendars drops its tasks, and showing it brings them back', async ({ page }) => {
    await page.getByRole('button', { name: 'Menu', exact: true }).click();
    await page.getByRole('button', { name: /Calendars/ }).click();
    await page.getByRole('checkbox', { name: 'Team', exact: true }).click();
    await gone(page);

    await page.getByRole('checkbox', { name: 'Team', exact: true }).click();
    await expect(onGrid(page, 'Team errand')).toBeVisible();
    await expect(side(page).getByText('Team errand')).toBeVisible();
  });

  test('hiding a calendar from the Big Year legend drops its tasks', async ({ page }) => {
    await page.keyboard.press('5');
    await expect(page.locator('.legend .item')).toHaveCount(3);
    await page.locator('.legend .item', { hasText: 'Team' }).click();
    await expect(side(page).getByText('Team errand')).toHaveCount(0);
    await expect(side(page).getByText('Home errand')).toBeVisible();
    // Back on the week, the TASKS row has the same answer.
    await page.keyboard.press('2');
    await gone(page);
  });
});
