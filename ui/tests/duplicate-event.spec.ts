import { test, expect, type Page } from '@playwright/test';
import {
  APP_WRITE_CALENDARS, APP_GUESTS_ID, APP_SERIES_OCCURRENCE,
  POPOVER_DETAILS,
} from './fixtures';

const form = (page: Page) => page.getByRole('dialog', { name: 'New event', exact: true });
const calls = (page: Page, cmd: string) => page.evaluate((name) =>
  window.__harness.calls.filter((c) => c.cmd === name).map((c) => c.args as any), cmd);

async function setup(page: Page, readOnly = false) {
  const calendars = [...APP_WRITE_CALENDARS, {
    ...APP_WRITE_CALENDARS[2], id: 20, account_id: 2,
    account_email: 'personal@example.com', summary: 'Home',
  }];
  const detail = {
    ...POPOVER_DETAILS[APP_GUESTS_ID], can_edit: !readOnly,
    location: 'Room 4', description: 'Bring the project notes.',
    conference_uri: 'https://meet.google.com/abc-defg-hij',
  };
  await page.addInitScript(({ calendars, detail }) => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', {
      configurable: true,
      set(ipc) {
        const invoke = ipc.invoke;
        ipc.invoke = (cmd: string, args: any, ...rest: any[]) => {
          if (cmd === 'get_calendars') return Promise.resolve(calendars);
          if ((cmd === 'event_detail' || cmd === 'refresh_event') && args.id === detail.id)
            return Promise.resolve(detail);
          return invoke(cmd, args, ...rest);
        };
        Object.defineProperty(window, '__TAURI_INTERNALS__', { value: ipc, configurable: true });
      },
    });
  }, { calendars, detail });
  await page.goto('/tests/harness/index.html?c=App&f=writable');
  await expect(page.locator('.ev').filter({ hasText: 'Client call' })).toBeVisible();
}

test('duplicate opens an unsaved draft with the calendar picker, then creates on another account', async ({ page }) => {
  await setup(page);
  await page.locator('.ev').filter({ hasText: 'Client call' }).click();
  await page.getByRole('button', { name: 'Duplicate', exact: true }).click();
  await expect(form(page)).toBeVisible();
  const picker = form(page).getByRole('listbox', { name: 'Calendar', exact: true });
  await expect(picker).toBeVisible();
  await expect(picker.getByRole('option', { selected: true })).toBeFocused();
  await expect(picker.getByRole('option', { name: 'Holidays in Bulgaria' })).toHaveCount(0);
  expect(await calls(page, 'create_event')).toEqual([]);
  expect(await calls(page, 'update_event')).toEqual([]);
  await picker.getByRole('option', { name: 'Home', exact: true }).click();
  await form(page).getByRole('button', { name: 'Create', exact: true }).click();
  await page.getByRole('button', { name: 'Create without notifying', exact: true }).click();
  await expect(form(page)).toHaveCount(0);
  const [created] = await calls(page, 'create_event');
  expect(created.calendarId).toBe(20);
  expect(created.fields.summary).toBe('Client call');
  expect(created.fields.description).toBe('Bring the project notes.');
  expect(created.fields.location).toContain('Room 4');
  expect(created.fields.location).toContain('https://meet.google.com/abc-defg-hij');
  // The name rides along with the address (#114). Google ignores it — a
  // person's display name there is theirs and is echoed back, never sent —
  // and on CalDAV it is the CN the copy should keep.
  expect(created.fields.guests).toEqual([{ email: 'ana@x.com', optional: false, displayName: 'Ana' }]);
  expect(created.sendUpdates).toBe('none');
  expect(await calls(page, 'update_event')).toEqual([]);
});

test('duplicate keeps the clicked recurring occurrence and Escape cancels without writing', async ({ page }) => {
  await setup(page);
  await page.locator('.ev').filter({ hasText: 'Standup' }).click();
  await page.getByRole('button', { name: 'Duplicate', exact: true }).click();
  await expect(form(page).getByRole('listbox')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(form(page)).toBeVisible();
  await expect(form(page).getByRole('listbox')).toHaveCount(0);
  await expect(form(page).getByLabel('Date', { exact: true })).toHaveValue(new Date(APP_SERIES_OCCURRENCE).toISOString().slice(0, 10));
  await expect(form(page).getByLabel('Start', { exact: true })).toHaveValue('09:00');
  await expect(form(page).getByLabel('End', { exact: true })).toHaveValue('09:30');
  await expect(form(page).getByLabel('Repeat', { exact: true })).toHaveValue('never');
  await page.keyboard.press('Escape');
  await expect(form(page)).toHaveCount(0);
  expect(await calls(page, 'create_event')).toEqual([]);
  expect(await calls(page, 'update_event')).toEqual([]);
});

test('duplicate is available for read-only source events and guests can be removed before saving', async ({ page }) => {
  await setup(page, true);
  await page.locator('.ev').filter({ hasText: 'Client call' }).click();
  await expect(page.getByRole('button', { name: 'Edit', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Duplicate', exact: true }).click();
  await form(page).getByRole('option', { name: 'Home', exact: true }).click();
  await form(page).getByRole('button', { name: 'Remove ana@x.com', exact: true }).click();
  await form(page).getByRole('button', { name: 'Create', exact: true }).click();
  await expect(form(page)).toHaveCount(0);
  const [created] = await calls(page, 'create_event');
  expect(created.calendarId).toBe(20);
  expect(created.fields.guests ?? []).toEqual([]);
});

test('duplicate preserves the all-day occurrence dates', async ({ page }) => {
  await setup(page);
  await page.locator('.chip').filter({ hasText: 'Diwali' }).click();
  await page.getByRole('button', { name: 'Duplicate', exact: true }).click();
  await form(page).getByRole('option', { name: 'Home', exact: true }).click();
  await expect(form(page).getByLabel('All day', { exact: true })).toBeChecked();
  await expect(form(page).getByLabel('First day', { exact: true })).toHaveValue('2024-01-31');
  await expect(form(page).getByLabel('Last day', { exact: true })).toHaveValue('2024-01-31');
});

test('duplicate works from the shared list popover without replacing the copy buffer', async ({ page }) => {
  await setup(page);
  await page.locator('.ev').filter({ hasText: 'Standup' }).click();
  await page.keyboard.press('Control+c');
  await page.keyboard.press('Escape');
  await page.keyboard.press('f');
  await page.getByRole('button', { name: /Client call/ }).click();
  await page.getByRole('button', { name: 'Duplicate', exact: true }).click();
  await expect(form(page).getByRole('listbox')).toBeVisible();
  await page.keyboard.press('Escape');
  await page.keyboard.press('Escape');
  await page.keyboard.press('Control+v');
  await expect(form(page).getByLabel('Title', { exact: true })).toHaveValue('Standup');
});
