import { test, expect, type Page } from '@playwright/test';
import { APP_GUESTS_ID, APP_NOW, APP_WRITE_CALENDARS, POPOVER_DETAILS } from './fixtures';

/** Two combined copies and two independent events. Moving the first event
 * into their slot sorts it after the combined event on the refreshed grid. */
async function setup(page: Page, equal = false) {
  await page.clock.setFixedTime(APP_NOW);
  await page.addInitScript(({ original, calendar, equal }) => {
    const start = original.start_ms;
    const hour = 3_600_000;
    const details = [
      { ...original, id: 60020, calendar_id: 30, title: 'Move me', color: '#bb55cc',
        start_ms: start - (equal ? 0 : hour / 4), end_ms: start + hour * (equal ? 1 : .75) },
      { ...original, title: 'Shared planning', calendar_id: 9, color: '#5577ee',
        start_ms: start, end_ms: start + hour },
      { ...original, id: 60030, calendar_id: 40, title: 'Separate planning', color: '#ee9955',
        start_ms: start, end_ms: start + hour },
      { ...original, id: 50001, calendar_id: 20, title: 'Shared planning', color: '#20b080',
        start_ms: start, end_ms: start + hour },
    ].map(e => ({ ...e, attendees: [], is_recurring: false, recurrence: null }));
    const calendars = details.map((e, i) => ({ ...calendar, id: e.calendar_id,
      summary: ['Studio', 'Work', 'Team', 'Home'][i], color_hex: e.color }));
    Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, set(ipc) {
      const invoke = ipc.invoke;
      ipc.invoke = async (cmd: string, args: any, ...rest: any[]) => {
        if (cmd === 'get_calendars') return calendars;
        if (['event_detail', 'refresh_event'].includes(cmd)) return details.find(e => e.id === args.id) ?? invoke(cmd, args, ...rest);
        const result = await invoke(cmd, args, ...rest);
        if (cmd === 'get_settings') return { ...result, visibleStartHour: 8, visibleEndHour: 18 };
        if (cmd === 'update_event') {
          const event = details.find(e => e.id === args.id)!;
          event.start_ms = args.fields.when.startMs;
          event.end_ms = args.fields.when.endMs;
        }
        if (['get_week', 'get_range', 'get_day'].includes(cmd)) {
          result.all_day = []; result.all_day_events = []; result.overflow = [];
          const base = result.days.flatMap((d: any) => d.events).find((e: any) => e.id === original.id);
          for (const day of result.days) {
            day.events = []; day.placed = [];
            if (!base || start < day.start_ms || start >= day.end_ms) continue;
            day.events = details.slice(0, 3).map(e => ({ ...base, ...e, attendees: 0,
              copies: e.id === original.id ? [details[1], details[3]].map(copy => ({
                id: copy.id, calendar_id: copy.calendar_id, color: copy.color,
                start_ms: copy.start_ms, end_ms: copy.end_ms,
              })) : undefined,
            })).sort((a, b) => a.start_ms - b.start_ms || a.id - b.id);
            day.placed = day.events.map((e: any, idx: number) => ({ idx, column: idx, columns: 3,
              top: (e.start_ms - day.start_ms) / (day.end_ms - day.start_ms),
              height: (e.end_ms - e.start_ms) / (day.end_ms - day.start_ms),
            }));
            if ((window as any).__reorderHoverEvents) {
              // A refresh may reorder the payload without changing its layout.
              day.events.push(day.events.shift());
              day.placed = day.placed.map((p: any) => ({ ...p, idx: (p.idx + 2) % 3 }))
                .sort((a: any, b: any) => a.idx - b.idx);
            }
          }
        }
        return result;
      };
      Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: ipc });
    }});
  }, { original: POPOVER_DETAILS[APP_GUESTS_ID], calendar: APP_WRITE_CALENDARS[2], equal });
  await page.goto('/tests/harness/index.html?c=App&f=writable');
  const moving = page.locator('.ev[data-event-id="60020"]');
  const shared = page.locator(`.ev[data-event-id="${APP_GUESTS_ID}"]`);
  const peer = page.locator('.ev[data-event-id="60030"]');
  await moving.scrollIntoViewIfNeeded();
  await expect(moving).toBeInViewport();
  return { moving, shared, peer };
}

test('dragging into a shared overlap does not transfer focus or leave empty copy panels', async ({ page }) => {
  const { moving, shared, peer } = await setup(page);
  const before = (await moving.boundingBox())!;
  const col = await moving.evaluate(el => {
    const r = el.closest('.col')!.getBoundingClientRect();
    return { x: r.x, width: r.width, height: r.height };
  });
  const x = before.x + before.width / 2, y = before.y + before.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x, y + col.height / 96, { steps: 5 });
  await page.mouse.up();
  await expect.poll(() => page.evaluate(() => window.__harness.calls.filter(c => c.cmd === 'update_event').length)).toBe(1);
  await expect(moving).toHaveAttribute('data-event-start-ms', String(POPOVER_DETAILS[APP_GUESTS_ID].start_ms));
  await page.mouse.move(0, 0);
  await expect(page.locator('.copy-panels')).toHaveCount(0);

  const resting = (await shared.boundingBox())!;
  await page.mouse.move(resting.x + resting.width / 4, resting.y + resting.height / 2);
  await expect(shared.locator(':scope > b')).toBeVisible();
  await expect(shared.locator('.calendar-colors > span')).toHaveCount(4);
  const home = page.getByRole('button', { name: 'Open Home copy', exact: true });
  await home.hover();
  await expect(shared.locator('.tip .tt')).toBeVisible();
  await expect(shared.locator('.tip .calendar-label')).toHaveText('Home');

  // Crossing to either independent event must dismiss every shared panel;
  // both its title and normal click target remain available after the move.
  for (const event of [moving, peer]) {
    const lane = await event.evaluate(el => {
      const r = el.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
    });
    await page.mouse.move(lane.x, lane.y);
    await expect(event).toHaveClass(/hovered/);
    await expect(page.locator('.copy-panels')).toHaveCount(0);
    await expect(event.locator(':scope > b')).toBeVisible();
    await expect(event.locator('.tip .tt')).toBeVisible();
    await expect(event.locator('.calendar-colors > span')).toHaveCount(4);
  }
});

test('a sync reorder keeps hover with the same shared occurrence', async ({ page }) => {
  const { shared, peer } = await setup(page);
  const box = (await shared.boundingBox())!;
  await page.mouse.move(box.x + box.width / 4, box.y + box.height / 2);
  await expect(shared).toHaveClass(/copies-open/);
  await page.getByRole('button', { name: 'Open Home copy', exact: true }).hover();
  await page.evaluate(async () => {
    (window as any).__reorderHoverEvents = true;
    await window.__harness.emit('sync-finished', null);
  });
  await expect.poll(() => shared.evaluate(el => {
    const buttons = [...el.closest('.col')!.querySelectorAll('.ev')];
    return buttons[0]?.getAttribute('data-event-id');
  })).toBe(String(APP_GUESTS_ID));
  await expect(shared).toHaveClass(/copies-open/);
  await expect(shared.locator(':scope > b')).toBeVisible();
  await expect(shared.locator('.tip .tt')).toBeVisible();
  await expect(peer).not.toHaveClass(/hovered/);
});

for (const destination of ['outside', 'neighbor'] as const) {
  test(`releasing a shared-copy drag ${destination} does not leave a second expansion`, async ({ page }) => {
    const { shared, peer } = await setup(page, true);
    const box = (await shared.boundingBox())!;
    const peerBox = (await peer.boundingBox())!;
    await page.mouse.move(box.x + box.width / 4, box.y + box.height / 2);
    await expect(shared.locator('.calendar-colors > span')).toHaveCount(4);
    const copy = (await page.getByRole('button', { name: 'Open Work copy', exact: true }).boundingBox())!;
    await page.mouse.move(copy.x + copy.width / 2, copy.y + copy.height / 2);
    await page.mouse.down();
    const to = destination === 'outside' ? { x: 0, y: 0 }
      : { x: peerBox.x + peerBox.width / 2, y: peerBox.y + peerBox.height / 2 };
    await page.mouse.move(to.x, to.y, { steps: 5 });
    await page.mouse.up();
    await page.mouse.move(to.x + 1, to.y + 1);
    await expect(page.locator('.copy-panels')).toHaveCount(0);
    if (destination === 'neighbor') {
      await expect(peer).toHaveClass(/hovered/);
      await expect(peer.locator(':scope > b')).toBeVisible();
      await expect(peer.locator('.calendar-colors > span')).toHaveCount(4);
    }
    await expect(page.getByRole('dialog')).toHaveCount(0);
    expect(await page.evaluate(() => window.__harness.calls.filter(c => c.cmd === 'update_event'))).toEqual([]);
    await page.mouse.move(box.x + box.width / 4, box.y + box.height / 2);
    await expect(shared.locator(':scope > b')).toBeVisible();
    await expect(shared.locator('.calendar-colors > span')).toHaveCount(4);
  });
}

test('keyboard expansion follows the focused calendar color and keeps four hover lanes', async ({ page }) => {
  const { shared, peer } = await setup(page, true);
  const peerBox = (await peer.boundingBox())!;
  await shared.focus();
  await expect(shared).toHaveClass(/copies-open/);
  await expect(shared.locator('.calendar-colors > span')).toHaveCount(4);
  const expanded = (await shared.boundingBox())!;
  const work = page.getByRole('button', { name: 'Open Work copy', exact: true });
  const home = page.getByRole('button', { name: 'Open Home copy', exact: true });
  const panels = shared.locator('..').locator('.copy-color');
  await expect(panels).toHaveCount(1);
  expect((await panels.first().boundingBox())!.width).toBeCloseTo(expanded.width, 0);
  await page.keyboard.press('Tab');
  await expect(work).toBeFocused();
  await expect(panels).toHaveCSS('box-shadow', /rgb\(85, 119, 238\)/);
  await page.keyboard.press('Tab');
  await expect(home).toBeFocused();
  await expect(panels).toHaveCSS('box-shadow', /rgb\(32, 176, 128\)/);
  await page.mouse.move(peerBox.x + peerBox.width / 2, peerBox.y + peerBox.height / 2);
  await expect(peer).toHaveClass(/hovered/);
  await expect(page.locator('.copy-panels')).toHaveCount(0);
  await expect(peer.locator(':scope > b')).toBeVisible();
});
