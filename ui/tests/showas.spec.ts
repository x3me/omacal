import { test, expect, type Locator, type Page } from '@playwright/test';

// Show as and visibility in the views (2026-11-05), Fantastical's convention:
// a free event's colour bar is an outline rather than a solid stripe, and a
// private (or confidential) event carries a small lock at its top right.
// Busy and default events must look exactly as they did. The fixtures are
// `showAsWeek` / `showAsMonth` in fixtures.ts, built for this file alone so
// no other golden moves.

const show = (c: string, f: string) => `/tests/harness/index.html?c=${c}&f=${f}`;

const block = (page: Page, id: number) => page.locator(`.ev[data-event-id="${id}"]`);

/** The pseudo-element's computed style, which no locator can reach. */
const before = (el: Locator) => el.evaluate((n) => {
  const s = getComputedStyle(n, '::before');
  return { content: s.content, width: s.width, left: s.left };
});

const inside = (inner: { x: number; y: number; width: number; height: number },
                outer: { x: number; y: number; width: number; height: number }) =>
  inner.x >= outer.x && inner.y >= outer.y
  && inner.x + inner.width <= outer.x + outer.width + 0.5
  && inner.y + inner.height <= outer.y + outer.height + 0.5;

test.describe('week view blocks', () => {
  test.beforeEach(async ({ page }) => { await page.goto(show('WeekGrid', 'show-as')); });

  test('a busy, default event keeps its solid spine and has no lock', async ({ page }) => {
    const busy = block(page, 8101);
    await expect(busy).not.toHaveClass(/\bfree\b/);
    await expect(busy).not.toHaveClass(/\bprivate\b/);
    expect((await before(busy)).content).toBe('none');
    expect(await busy.evaluate((n) => getComputedStyle(n).boxShadow)).toMatch(/2px 0px 0px 0px inset/);
    await expect(busy.locator('.lock')).toHaveCount(0);
    await expect(busy).toHaveAttribute('aria-label', /^Busy review, [^,]+, Meet$/);
  });

  test('a free event draws its spine as an outline, 3px across', async ({ page }) => {
    const free = block(page, 8102);
    await expect(free).toHaveClass(/\bfree\b/);
    await expect(free).not.toHaveClass(/\bprivate\b/);
    // The outer stroke: the inset shadow thinned to one pixel.
    expect(await free.evaluate((n) => getComputedStyle(n).boxShadow)).toMatch(/1px 0px 0px 0px inset/);
    // The inner stroke: a 1px line two pixels in, so the bar spans 3px.
    expect(await before(free)).toEqual({ content: '""', width: '1px', left: '2px' });
    await expect(free.locator('.lock')).toHaveCount(0);
    await expect(free).toHaveAttribute('aria-label', /, free$/);
  });

  test('the title starts where a busy block\'s does', async ({ page }) => {
    const x = async (id: number) => (await block(page, id).locator('b').boundingBox())!.x
      - (await block(page, id).boundingBox())!.x;
    expect(await x(8102)).toBe(await x(8101));
  });

  test('a private event shows a lock in its top-right corner, inside the block', async ({ page }) => {
    const priv = block(page, 8103);
    await expect(priv).toHaveClass(/\bprivate\b/);
    await expect(priv).not.toHaveClass(/\bfree\b/);
    const lock = priv.locator('.lock');
    await expect(lock).toBeVisible();
    await expect(lock.locator('svg[aria-label="Private"]')).toHaveCount(1);
    await expect(priv).toHaveAttribute('aria-label', /, private$/);
    const b = (await priv.boundingBox())!;
    const l = (await lock.boundingBox())!;
    expect(inside(l, b)).toBe(true);
    // Top right: in the last fifth of the width, on the first line.
    expect(l.x).toBeGreaterThan(b.x + b.width * 0.8);
    expect(l.y - b.y).toBeLessThan(8);
    // Ink, not the calendar's raw colour: the glyph paints in the block's text colour.
    const [ink, glyph] = await Promise.all([
      priv.evaluate((n) => getComputedStyle(n).color),
      lock.locator('svg').evaluate((n) => getComputedStyle(n).color),
    ]);
    expect(glyph).toBe(ink);
    // The title's ellipsis stops before the lock.
    const title = (await priv.locator('b').boundingBox())!;
    const pad = await priv.locator('b').evaluate((n) => parseFloat(getComputedStyle(n).paddingRight));
    expect(title.x + title.width - pad).toBeLessThanOrEqual(l.x);
  });

  test('free and private at once wear both marks', async ({ page }) => {
    const both = block(page, 8104);
    await expect(both).toHaveClass(/\bfree\b/);
    await expect(both).toHaveClass(/\bprivate\b/);
    expect((await before(both)).content).toBe('""');
    await expect(both.locator('.lock')).toBeVisible();
    await expect(both).toHaveAttribute('aria-label', /, free, private$/);
  });

  test('the lock and the tentative mark do not share a pixel', async ({ page }) => {
    const ev = block(page, 8201);
    const l = (await ev.locator('.lock').boundingBox())!;
    const q = (await ev.locator('.rs').boundingBox())!;
    expect(q.x + q.width).toBeLessThanOrEqual(l.x);
  });

  test('the lock fits a 15-minute block', async ({ page }) => {
    const ev = block(page, 8202);
    expect(inside((await ev.locator('.lock').boundingBox())!, (await ev.boundingBox())!)).toBe(true);
  });

  test('an unanswered free event keeps its dashed ring and gains the outline', async ({ page }) => {
    const ev = block(page, 8203);
    await expect(ev).toHaveClass(/\bneedsAction\b/);
    expect(await ev.evaluate((n) => getComputedStyle(n).borderLeftStyle)).toBe('dashed');
    expect((await before(ev)).content).toBe('""');
  });
});

test('a block covered by an expanded neighbour hides its lock with its title', async ({ page }) => {
  // The overlap-hover fixture: two meetings side by side, both made private
  // here. Hovering the left one expands it over the right one, which goes
  // blank (`.obscured`); a lock left showing floats over the neighbour.
  await page.goto(show('WeekGrid', 'single-day-overlap'));
  await page.evaluate(() => {
    const w = structuredClone((window as any).__fixtureProps.week);
    for (const d of w.days) for (const e of d.events) e.visibility = 'private';
    (window as any).__setWeek(w);
  });
  const left = page.locator('.ev').filter({ hasText: 'Ops review' });
  const right = page.locator('.ev').filter({ hasText: 'Investors' });
  await left.scrollIntoViewIfNeeded();
  await expect(right.locator('.lock')).toHaveCSS('visibility', 'visible');
  const col = await left.evaluate((el) => {
    const r = el.closest('.col')!.getBoundingClientRect();
    return { x: r.x, width: r.width };
  });
  const box = (await left.boundingBox())!;
  await page.mouse.move(col.x + col.width * 0.25, box.y + box.height / 2, { steps: 4 });
  // Fixture check: the right block really is the covered one.
  await expect(right.locator('b')).toHaveCSS('visibility', 'hidden');
  await expect(right.locator('.lock')).toHaveCSS('visibility', 'hidden');
  await expect(left.locator('.lock')).toHaveCSS('visibility', 'visible');
});

test.describe('week view all-day band', () => {
  test.beforeEach(async ({ page }) => { await page.goto(show('WeekGrid', 'show-as')); });
  const chip = (page: Page, title: string) => page.locator('.chip', { hasText: title });

  test('a busy all-day chip keeps its solid 2px spine, its padding and no lock', async ({ page }) => {
    const c = chip(page, 'Conference');
    await expect(c).not.toHaveClass(/\bfree\b|\bprivate\b/);
    expect(await c.evaluate((n) => {
      const s = getComputedStyle(n);
      return [s.borderLeftStyle, s.borderLeftWidth, s.paddingLeft, s.paddingRight];
    })).toEqual(['solid', '2px', '7px', '7px']);
    await expect(c.locator('.lock')).toHaveCount(0);
  });

  test('a free all-day chip draws a double (hollow) spine without moving its label', async ({ page }) => {
    const c = chip(page, 'Offsite - free');
    await expect(c).toHaveClass(/\bfree\b/);
    expect(await c.evaluate((n) => {
      const s = getComputedStyle(n);
      return [s.borderLeftStyle, s.borderLeftWidth, s.paddingLeft];
    })).toEqual(['double', '3px', '6px']);
  });

  test('a private all-day chip carries a trailing lock inside its bounds', async ({ page }) => {
    const c = chip(page, 'Leave - private');
    await expect(c).toHaveClass(/\bprivate\b/);
    const lock = c.locator('.lock');
    await expect(lock).toBeVisible();
    await expect(c).toHaveAccessibleName(/Leave - private.*Private/);
    expect(inside((await lock.boundingBox())!, (await c.boundingBox())!)).toBe(true);
  });
});

test.describe('month view', () => {
  test.beforeEach(async ({ page }) => { await page.goto(show('MonthGrid', 'show-as')); });
  const bar = (page: Page, title: string) => page.locator('.bar', { hasText: title });
  const row = (page: Page, title: string) => page.locator('.timed', { hasText: title });

  test('bars: free is double, private has a lock, busy is unchanged', async ({ page }) => {
    const style = (l: Locator) => l.evaluate((n) => getComputedStyle(n).borderLeftStyle);
    expect(await style(bar(page, 'Offsite - free'))).toBe('double');
    expect(await style(bar(page, 'Conference'))).toBe('solid');
    expect(await bar(page, 'Conference').evaluate((n) => getComputedStyle(n).paddingLeft)).toBe('6px');
    expect(await bar(page, 'Offsite - free').evaluate((n) => getComputedStyle(n).paddingLeft)).toBe('5px');
    await expect(bar(page, 'Leave - private').locator('.lock')).toBeVisible();
    await expect(bar(page, 'Conference').locator('.lock')).toHaveCount(0);
    await expect(bar(page, 'Offsite - free').locator('.lock')).toHaveCount(0);
  });

  test('timed rows: free draws a ring for the dot, private trails a lock', async ({ page }) => {
    const dot = (title: string) => row(page, title).locator('.dot').evaluate((n) => {
      const s = getComputedStyle(n);
      return { bg: s.backgroundColor, ring: s.boxShadow };
    });
    const busy = await dot('Busy review');
    expect(busy.ring).toBe('none');
    expect(busy.bg).not.toBe('rgba(0, 0, 0, 0)');
    const free = await dot('Focus - free');
    expect(free.bg).toBe('rgba(0, 0, 0, 0)');
    expect(free.ring).toMatch(/inset/);
    await expect(row(page, 'Busy review').locator('.lock')).toHaveCount(0);
    await expect(row(page, 'Focus - free').locator('.lock')).toHaveCount(0);
    const priv = row(page, 'Doctor - private');
    await expect(priv.locator('.lock svg[aria-label="Private"]')).toBeVisible();
    expect(inside((await priv.locator('.lock').boundingBox())!, (await priv.boundingBox())!)).toBe(true);
    await expect(row(page, 'Walk - free private').locator('.lock')).toBeVisible();
  });
});

test.describe('list view', () => {
  test('free rows draw the outline and private rows carry the lock', async ({ page }) => {
    await page.goto(show('Filmstrip', 'show-as'));
    const r = (title: string) => page.locator('.srow', { hasText: title });
    expect((await before(r('Focus - free'))).content).toBe('""');
    expect((await before(r('Busy review'))).content).toBe('none');
    await expect(r('Doctor - private').locator('.lock')).toBeVisible();
    await expect(r('Busy review').locator('.lock')).toHaveCount(0);
    expect(await r('Busy review').evaluate((n) => getComputedStyle(n).boxShadow)).toMatch(/2px 0px 0px 0px inset/);
  });
});
