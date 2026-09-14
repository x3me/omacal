import { test, expect, type Page } from '@playwright/test';
import { APP_GUESTS_ID, APP_WRITE_CALENDARS, POPOVER_DETAILS } from './fixtures';

async function setup(page: Page, kind: 'equal' | 'contained' | 'staggered' | 'enclosing' | 'mixed' | 'shared-peer') {
  const ranges = kind === 'contained' ? [[0, 1], [.25, .75]]
    : kind === 'staggered' ? [[0, 1], [.5, 1.5]]
    : kind === 'enclosing' ? [[.25, .75], [0, 1]]
    : kind === 'mixed' ? [[0, 1], [0, 1], [.5, 1.5]] : [[0, 1], [0, 1]];
  await page.addInitScript(({ ranges, kind, original, calendar }) => {
    const details = [
      { ...original, title: 'Shared planning', calendar_id: 9, color: '#5577ee' },
      { ...original, id: 50001, title: 'Shared planning', calendar_id: 20, color: '#20b080' },
      { ...original, id: 60020, title: 'Separate planning', calendar_id: 30, color: '#ee9955' },
      { ...original, id: 60021, title: 'Separate planning', calendar_id: 40, color: '#bb55cc' },
      { ...original, id: 70001, title: 'Staggered planning', calendar_id: 50, color: '#22aacc' },
    ];
    const calendars = details.map((e, i) => ({ ...calendar, id: e.calendar_id,
      summary: ['Work', 'Home', 'Team', 'Family', 'Studio'][i], color_hex: e.color }));
    Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, set(ipc) {
      const invoke = ipc.invoke;
      ipc.invoke = async (cmd: string, args: any, ...rest: any[]) => {
        if (cmd === 'get_calendars') return calendars;
        if (['event_detail', 'refresh_event'].includes(cmd)) return details.find(e => e.id === args.id) ?? invoke(cmd, args, ...rest);
        const result = await invoke(cmd, args, ...rest);
        if (['get_week', 'get_range', 'get_day'].includes(cmd)) {
          result.all_day = []; result.all_day_events = []; result.overflow = [];
          for (const day of result.days) {
            const base = day.events.find((e: any) => e.id === original.id);
            const placement = day.placed.find((p: any) => day.events[p.idx]?.id === original.id);
            day.events = []; day.placed = [];
            if (!base) continue;
            const duration = base.end_ms - base.start_ms;
            day.events = ranges.map(([from, to], i) => {
              const event = { ...base, ...details[i === 0 ? 0 : i === 1 ? 2 : 4],
                start_ms: base.start_ms + from * duration, end_ms: base.start_ms + to * duration };
              if (i === 0 || (i === 1 && kind === 'shared-peer')) {
                event.copies = details.slice(i ? 2 : 0, i ? 4 : 2).map(e => ({ id: e.id, calendar_id: e.calendar_id,
                  start_ms: event.start_ms, end_ms: event.end_ms, color: e.color }));
              }
              return event;
            });
            day.placed = ranges.map(([from, to], idx) => ({ idx, column: idx, columns: ranges.length,
              top: placement.top + from * placement.height, height: (to - from) * placement.height }));
          }
        }
        return result;
      };
      Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: ipc });
    }});
  }, { ranges, kind, original: POPOVER_DETAILS[APP_GUESTS_ID], calendar: APP_WRITE_CALENDARS[2] });
  await page.goto('/tests/harness/index.html?c=App&f=writable');
  const shared = page.locator('.ev').filter({ hasText: 'Shared planning' });
  await shared.scrollIntoViewIfNeeded();
  const box = (await shared.boundingBox())!;
  const col = await shared.evaluate(el => {
    const r = el.closest('.col')!.getBoundingClientRect();
    return { x: r.x, width: r.width };
  });
  return { shared, box, col, peer: page.locator('.ev').filter({ hasText: 'Separate planning' }) };
}

for (const kind of ['staggered', 'enclosing'] as const) {
  test(`a shared expansion retains its copy targets across ${kind} overlaps`, async ({ page }) => {
    const { shared, box, col, peer } = await setup(page, kind);
    const y = box.y + box.height * .75;
    await page.mouse.move(col.x + col.width * .15, y);
    await expect(shared).toHaveClass(/copies-open/);
    await page.mouse.move(col.x + col.width * .75, y);
    await expect(shared).toHaveClass(/copies-open/);
    await expect(peer).not.toHaveClass(/hovered/);
    await expect(page.getByRole('button', { name: 'Open Home copy', exact: true })).toBeVisible();
    await page.mouse.click(col.x + col.width * .75, y);
    await expect(page.getByRole('dialog', { name: 'Shared planning', exact: true })).toBeVisible();
    await expect(page.getByLabel('Calendar copy', { exact: true })).toHaveValue('50001');
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await page.mouse.move(0, 0);
    await page.mouse.move(col.x + col.width * .15, y);
    const exposed = (await peer.boundingBox())!;
    await page.mouse.move(col.x + col.width * .75, exposed.y + exposed.height * .9);
    await expect(peer).toHaveClass(/hovered/);
    await expect(shared).not.toHaveClass(/copies-open/);
  });
}

for (const kind of ['equal', 'contained', 'shared-peer'] as const) {
  test(`the hovered shared color fills the card while bottom bars retain the ${kind} event's hover lane`, async ({ page }) => {
    const { shared, box, col, peer } = await setup(page, kind);
    const y = box.y + box.height / 2;
    await page.mouse.move(col.x + col.width * .125, y);
    await expect(shared).toHaveClass(/copies-open/);
    const whole = (await shared.boundingBox())!;
    const work = page.getByRole('button', { name: 'Open Work copy', exact: true });
    const home = page.getByRole('button', { name: 'Open Home copy', exact: true });
    await expect.poll(async () => (await home.boundingBox())!.width).toBeCloseTo(whole.width / 4, 0);
    const panels = shared.locator('..').locator('.copy-color');
    await expect(panels).toHaveCount(1);
    const painted = (await panels.boundingBox())!;
    expect(painted.x).toBeCloseTo(whole.x, 0);
    expect(painted.width).toBeCloseTo(whole.width, 0);
    await expect(panels).toHaveCSS('box-shadow', /rgb\(85, 119, 238\)/);
    const workFill = await panels.evaluate(el => getComputedStyle(el).backgroundColor);
    const first = (await work.boundingBox())!, second = (await home.boundingBox())!;
    expect(second.x).toBeCloseTo(first.x + first.width, 0);
    await expect(shared.locator('.calendar-colors > span')).toHaveCount(kind === 'shared-peer' ? 4 : 3);
    const markers = await shared.locator('.calendar-colors > span').evaluateAll(els => els.map(el => {
      const r = el.getBoundingClientRect(); return { x: r.x, width: r.width };
    }));
    // Segments retain their intentional one-pixel inset at each edge.
    expect(markers[1].x).toBeCloseTo(second.x + 1, 0);
    expect(markers[2].x).toBeCloseTo(whole.x + whole.width / 2 + 1, 0);
    await page.mouse.move(col.x + col.width * .375, y);
    await expect(shared).toHaveClass(/copies-open/);
    // The entire card follows the copy under the pointer, while the bottom
    // bars and their hit targets stay in place.
    await expect(shared.locator('.tip .calendar-label')).toHaveText('Home');
    await expect(panels).toHaveCSS('box-shadow', /rgb\(32, 176, 128\)/);
    await expect(panels).not.toHaveCSS('background-color', workFill);
    await page.mouse.click(col.x + col.width * .375, y);
    await expect(page.getByLabel('Calendar copy', { exact: true })).toHaveValue('50001');
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await page.mouse.move(0, 0);
    await page.mouse.move(col.x + col.width * .125, y);
    await page.mouse.move(col.x + col.width * .75, y);
    await expect(peer).toHaveClass(/hovered/);
    await expect(shared).not.toHaveClass(/copies-open/);
    if (kind === 'shared-peer') {
      const family = page.getByRole('button', { name: 'Open Family copy', exact: true });
      expect((await family.boundingBox())!.x).toBeCloseTo(whole.x + whole.width * .75, 0);
      const otherPanels = peer.locator('..').locator('.copy-color');
      await expect(otherPanels).toHaveCount(1);
      expect((await otherPanels.first().boundingBox())!.width).toBeCloseTo(whole.width, 0);
      await family.hover();
      await expect(otherPanels).toHaveCSS('box-shadow', /rgb\(187, 85, 204\)/);
      await family.click();
      await expect(page.getByLabel('Calendar copy', { exact: true })).toHaveValue('60021');
    } else {
      await page.mouse.click(col.x + col.width * .75, y);
      await expect(page.getByRole('dialog', { name: 'Separate planning', exact: true })).toBeVisible();
    }
  });
}

test('a shared hover with equal and staggered neighbors only hands off to the equal event', async ({ page }) => {
  const { shared, box, col, peer } = await setup(page, 'mixed');
  const y = box.y + box.height * .75;
  await page.mouse.move(col.x + col.width * .1, y);
  await expect(shared.locator('.calendar-colors > span')).toHaveCount(3);
  await page.mouse.move(col.x + col.width * .85, y);
  await expect(shared).toHaveClass(/copies-open/);
  await page.mouse.click(col.x + col.width * .85, y);
  await expect(page.getByRole('dialog', { name: 'Shared planning', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await page.mouse.move(0, 0);
  await page.mouse.move(col.x + col.width * .1, y);
  await page.mouse.move(col.x + col.width * .5, y);
  await expect(peer).toHaveClass(/hovered/);
  await expect(shared).not.toHaveClass(/copies-open/);
  await page.mouse.click(col.x + col.width * .5, y);
  await expect(page.getByRole('dialog', { name: 'Separate planning', exact: true })).toBeVisible();
});
