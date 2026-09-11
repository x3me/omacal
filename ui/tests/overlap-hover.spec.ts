import { test, expect } from '@playwright/test';

for (const fixture of ['populated', 'single-day-overlap']) {
  test(`hover switches across original overlap columns without leaving the events (${fixture})`, async ({ page }) => {
    await page.goto(`/tests/harness/index.html?c=WeekGrid&f=${fixture}`);
    const left = page.locator('.ev').filter({ hasText: 'Ops review' });
    const right = page.locator('.ev').filter({ hasText: 'Investors' });
    await left.scrollIntoViewIfNeeded();
    const col = await left.evaluate(el => {
      const r = el.closest('.col')!.getBoundingClientRect();
      return { x: r.x, width: r.width };
    });
    const box = (await left.boundingBox())!;
    const y = box.y + box.height / 2;
    for (const [fraction, active, inactive, title] of [
      [.25, left, right, 'Ops review'],
      [.51, right, left, 'Investors'],
      [.49, left, right, 'Ops review'],
      [.75, right, left, 'Investors'],
    ] as const) {
      const x = col.x + col.width * fraction;
      await page.mouse.move(x, y, { steps: 4 });
      await expect.poll(async () => (await active.boundingBox())!.width).toBeGreaterThan(col.width * .9);
      await expect.poll(async () => (await inactive.boundingBox())!.width).toBeLessThan(col.width * .6);
      await expect(page.locator('.tip .tt')).toHaveText(title);
      await expect(active.locator('.calendar-colors > span')).toHaveCount(2);
      await expect(inactive.locator('b')).toHaveCSS('visibility', 'hidden');
      await expect(active.locator('b').first()).toHaveCSS('visibility', 'visible');
      const markers = await active.locator('.calendar-colors > span').evaluateAll(els => els.map(el => {
        const r = el.getBoundingClientRect();
        return { x: r.x, width: r.width, height: r.height, color: getComputedStyle(el).backgroundColor };
      }));
      expect(markers[0].height).toBe(3);
      expect(markers[1].x).toBeCloseTo(col.x + col.width / 2 + 1, 0);
      expect(markers[0].color).not.toBe(markers[1].color);
      expect(await page.evaluate(({x,y}) => document.elementFromPoint(x,y)?.closest('.ev')?.textContent, {x,y})).toContain(title);
    }
    await page.mouse.move(0, 0);
    await expect(page.locator('.tip')).toHaveCount(0);
    await expect(page.locator('.calendar-colors')).toHaveCount(0);
    await expect(left.locator('b')).toHaveCSS('visibility', 'visible');
    await expect(right.locator('b')).toHaveCSS('visibility', 'visible');
    await expect.poll(async () => (await right.boundingBox())!.width).toBeLessThan(col.width * .6);
  });
}


test('staggered overlaps keep exposed labels without hover markers', async ({ page }) => {
  await page.goto('/tests/harness/index.html?c=WeekGrid&f=single-day-overlap');
  await page.evaluate(() => {
    const w = structuredClone((window as any).__fixtureProps.week);
    const d = w.days[0];
    const base = d.events[0];
    const hour = (d.end_ms - d.start_ms) / 24;
    d.events = [
      { ...base, title: 'Active', start_ms: d.start_ms + 10 * hour, end_ms: d.start_ms + 11 * hour },
      { ...base, id: 7001, title: 'Partial', color: '#00aa88', start_ms: d.start_ms + 10.5 * hour, end_ms: d.start_ms + 11.5 * hour },
      { ...base, id: 7002, title: 'Next', color: '#ffbb00', start_ms: d.start_ms + 11 * hour, end_ms: d.start_ms + 12 * hour },
    ];
    d.placed = [
      { idx: 0, top: 10 / 24, height: 1 / 24, column: 0, columns: 2 },
      { idx: 1, top: 10.5 / 24, height: 1 / 24, column: 1, columns: 2 },
      { idx: 2, top: 11 / 24, height: 1 / 24, column: 0, columns: 2 },
    ];
    (window as any).__setWeek(w);
  });
  const active = page.locator('.ev').filter({ hasText: 'Active' });
  await active.scrollIntoViewIfNeeded();
  const box = (await active.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height * .75);
  await expect(active.locator('.calendar-colors')).toHaveCount(0);
  await expect(page.locator('.ev').filter({ hasText: 'Partial' }).locator('b')).toHaveCSS('visibility', 'visible');
  await expect(page.locator('.ev').filter({ hasText: 'Next' }).locator('b')).toHaveCSS('visibility', 'visible');
  await page.keyboard.down('Alt');
  await expect(page.locator('.calendar-colors')).toHaveCount(0);
  await expect(page.locator('.ev').filter({ hasText: 'Partial' }).locator('b')).toHaveCSS('visibility', 'visible');
  await page.keyboard.up('Alt');
});


for (const [kind, start, end] of [
  ['nested', 10.5, 11.5], ['shared start', 10, 11], ['shared end', 11, 12],
] as const) {
  test(`hover markers identify a covered event, not an exposed enclosing event (${kind})`, async ({ page }) => {
    await page.goto('/tests/harness/index.html?c=WeekGrid&f=single-day-overlap');
    await page.evaluate(({ start, end }) => {
      const w = structuredClone((window as any).__fixtureProps.week);
      const d = w.days[0];
      const base = d.events[0];
      const hour = (d.end_ms - d.start_ms) / 24;
      d.events = [
        { ...base, title: 'Long meeting', start_ms: d.start_ms + 10 * hour, end_ms: d.start_ms + 12 * hour },
        { ...base, id: 7001, title: 'Short meeting', color: '#00aa88', start_ms: d.start_ms + start * hour, end_ms: d.start_ms + end * hour },
      ];
      d.placed = [
        { idx: 0, top: 10 / 24, height: 2 / 24, column: 0, columns: 2 },
        { idx: 1, top: start / 24, height: (end - start) / 24, column: 1, columns: 2 },
      ];
      (window as any).__setWeek(w);
    }, { start, end });
    const long = page.locator('.ev').filter({ hasText: 'Long meeting' });
    const short = page.locator('.ev').filter({ hasText: 'Short meeting' });
    await short.scrollIntoViewIfNeeded();
    const first = (await long.boundingBox())!;
    const second = (await short.boundingBox())!;
    const y = second.y + second.height / 2;
    await page.mouse.move(first.x + first.width / 2, y);
    await expect(long.locator('.calendar-colors > span')).toHaveCount(2);
    await expect(short.locator('b')).toHaveCSS('visibility', 'hidden');
    await page.mouse.move(second.x + second.width / 2, y);
    await expect.poll(async () => (await short.boundingBox())!.width).toBeGreaterThan(second.width * 1.8);
    await expect(short.locator('.calendar-colors')).toHaveCount(0);
    await expect(long.locator(':scope > b')).toHaveCSS('visibility', 'visible');
    await expect(page.locator('.tip .tt')).toHaveText('Short meeting');
  });
}

test('a mixed overlap marks only the contained calendars', async ({ page }) => {
  await page.goto('/tests/harness/index.html?c=WeekGrid&f=single-day-overlap');
  await page.evaluate(() => {
    const w = structuredClone((window as any).__fixtureProps.week);
    const d = w.days[0];
    const base = d.events[0];
    const hour = (d.end_ms - d.start_ms) / 24;
    d.events = [
      { ...base, title: 'Long meeting', start_ms: d.start_ms + 10 * hour, end_ms: d.start_ms + 12 * hour },
      { ...base, id: 7001, title: 'Contained', color: '#00aa88', start_ms: d.start_ms + 11 * hour, end_ms: d.start_ms + 11.5 * hour },
      { ...base, id: 7002, title: 'Staggered', color: '#ffbb00', start_ms: d.start_ms + 11 * hour, end_ms: d.start_ms + 13 * hour },
    ];
    d.placed = [
      { idx: 0, top: 10 / 24, height: 2 / 24, column: 0, columns: 3 },
      { idx: 1, top: 11 / 24, height: .5 / 24, column: 1, columns: 3 },
      { idx: 2, top: 11 / 24, height: 2 / 24, column: 2, columns: 3 },
    ];
    (window as any).__setWeek(w);
  });
  const long = page.locator('.ev').filter({ hasText: 'Long meeting' });
  await long.scrollIntoViewIfNeeded();
  const r = (await long.boundingBox())!;
  await page.mouse.move(r.x + r.width / 2, r.y + r.height * .625);
  await expect(long.locator('.calendar-colors > span')).toHaveCount(2);
  await expect(long.locator('.calendar-colors > span').nth(1)).toHaveCSS('background-color', 'rgb(0, 170, 136)');
  await expect(page.locator('.ev').filter({ hasText: 'Contained' }).locator('b')).toHaveCSS('visibility', 'hidden');
  await expect(page.locator('.ev').filter({ hasText: 'Staggered' }).locator('b')).toHaveCSS('visibility', 'visible');
});
