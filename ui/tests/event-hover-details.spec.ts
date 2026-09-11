import { test, expect, type Page } from '@playwright/test';
import { APP_WRITE_CALENDARS, APP_GUESTS_ID, POPOVER_DETAILS } from './fixtures';

async function setup(page: Page, combined = false) {
  const original = POPOVER_DETAILS[APP_GUESTS_ID];
  const work = { ...original, title: combined ? 'Shared planning' : 'Work planning', calendar_id: 9, color: '#5577ee' };
  const home = { ...work, id: 60020, title: combined ? work.title : 'Home planning', calendar_id: 20, color: '#20b080' };
  const calendars = [
    { ...APP_WRITE_CALENDARS[2], id: 9, summary: 'Work', color_hex: work.color },
    { ...APP_WRITE_CALENDARS[2], id: 20, summary: 'Personal', color_hex: home.color },
  ];
  await page.addInitScript(({ work, home, calendars, combined }) => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, set(ipc) {
      const invoke = ipc.invoke;
      ipc.invoke = async (cmd: string, args: any, ...rest: any[]) => {
        if (cmd === 'get_calendars') return calendars;
        if (['event_detail', 'refresh_event'].includes(cmd)) {
          if (args.id === work.id) return work;
          if (args.id === home.id) return home;
        }
        const result = await invoke(cmd, args, ...rest);
        if (['get_week', 'get_range', 'get_day'].includes(cmd)) {
          result.all_day = []; result.all_day_events = []; result.overflow = [];
          for (const day of result.days) {
            const base = day.events.find((e: any) => e.id === work.id);
            const placement = day.placed.find((p: any) => day.events[p.idx]?.id === work.id);
            day.events = []; day.placed = [];
            if (!base) continue;
            const copies = [work, home].map(e => ({ id: e.id, calendar_id: e.calendar_id,
              start_ms: base.start_ms, end_ms: base.end_ms, color: e.color }));
            day.events = combined
              ? [{ ...base, ...copies[0], title: work.title, copies }]
              : [work, home].map((e, i) => ({ ...base, ...copies[i], title: e.title }));
            day.placed = day.events.map((e: any, i: number) => ({ idx: i, column: i,
              columns: day.events.length, top: placement.top, height: placement.height }));
          }
        }
        return result;
      };
      Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: ipc });
    }});
  }, { work, home, calendars, combined });
  await page.goto('/tests/harness/index.html?c=App&f=writable');
}

test('overlap hover identifies each calendar and opens beside its original small event', async ({ page }) => {
  await setup(page);
  const work = page.locator('.ev').filter({ hasText: 'Work planning' });
  const home = page.locator('.ev').filter({ hasText: 'Home planning' });
  await work.scrollIntoViewIfNeeded();
  const first = (await work.boundingBox())!;
  const second = (await home.boundingBox())!;
  for (const [block, box, name, color, title] of [
    [work, first, 'Work', 'rgb(85, 119, 238)', 'Work planning'],
    [home, second, 'Personal', 'rgb(32, 176, 128)', 'Home planning'],
  ] as const) {
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    const tip = page.locator('.tip');
    await expect(tip.locator('.calendar-label')).toHaveText(name);
    await expect(tip.locator('.calendar-dot')).toHaveCSS('background-color', color);
    await expect.poll(async () => (await block.boundingBox())!.width).toBeGreaterThan(box.width * 1.8);
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    const popover = page.getByRole('dialog', { name: title, exact: true });
    await expect(popover).toBeVisible();
    await expect.poll(async () => (await popover.boundingBox())!.x).toBeCloseTo(box.x + box.width + 8, 0);
    await expect(page.locator('.tip')).toHaveCount(0);
    await page.keyboard.press('Escape');
    await page.mouse.move(0, 0);
  }
});

test('combined panels use the same colored tooltip and anchor details to the chosen copy', async ({ page }) => {
  await setup(page, true);
  const block = page.locator('.ev').filter({ hasText: 'Shared planning' });
  await block.scrollIntoViewIfNeeded();
  const box = (await block.boundingBox())!;
  await page.mouse.move(box.x + box.width / 4, box.y + box.height / 2);
  for (const [name, color] of [['Work', 'rgb(85, 119, 238)'], ['Personal', 'rgb(32, 176, 128)']] as const) {
    const panel = page.getByRole('button', { name: `Open ${name} copy`, exact: true });
    await panel.hover();
    await expect(panel).not.toHaveAttribute('title', /.+/);
    await expect(page.locator('.tip .calendar-label')).toHaveText(name);
    await expect(page.locator('.tip .calendar-dot')).toHaveCSS('background-color', color);
    const rect = (await panel.boundingBox())!;
    await panel.click();
    const popover = page.getByRole('dialog', { name: 'Shared planning', exact: true });
    await expect(popover).toBeVisible();
    await expect.poll(async () => (await popover.boundingBox())!.x).toBeCloseTo(rect.x + rect.width + 8, 0);
    await page.keyboard.press('Escape');
    await page.mouse.move(0, 0);
    await page.mouse.move(box.x + box.width / 4, box.y + box.height / 2);
  }
});


test('clicking an expanded overlap places details beside the original lane', async ({ page }) => {
  await setup(page);
  const block = page.locator('.ev').filter({ hasText: 'Work planning' });
  await block.scrollIntoViewIfNeeded();
  const original = (await block.boundingBox())!;
  await page.mouse.move(original.x + original.width / 2, original.y + original.height / 2);
  await expect.poll(async () => (await block.boundingBox())!.width).toBeGreaterThan(original.width * 1.8);
  await page.mouse.click(original.x + original.width / 2, original.y + original.height / 2);
  const popover = page.getByRole('dialog', { name: 'Work planning', exact: true });
  await expect(popover).toBeVisible();
  await expect.poll(async () => (await popover.boundingBox())!.x).toBeCloseTo(original.x + original.width + 8, 0);
});
