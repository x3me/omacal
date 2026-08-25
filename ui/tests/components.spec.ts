import { test, expect, type Page } from '@playwright/test';
import {
  FIXED_NOW, FORM_FALLBACK_ID, FORM_NOW, FORM_UNWRITABLE_ID, FORM_UNWRITABLE_NAMES,
  MON, MONTH_2026_NOW, POPOVER_DETAILS, POPOVER_REFRESHED_DETAIL,
  TRIP_END_DATE, TRIP_FIRST_DAY, TRIP_LAST_DAY, popoverWeekWithResponse, UNBREAKABLE,
  WEEK_NOW, WEEK_NOW_INSIDE, YEAR_2026_NOW,
} from './fixtures';
import { CALENDAR_SYNC_REMOVED } from './harness/tauri';

const show = (c: string, f: string) => `/tests/harness/index.html?c=${c}&f=${f}`;

test.describe('WeekGrid', () => {
  test('midnight and a wake move today off yesterday', async ({ page }) => {
    // `todayStart` was a const computed at mount, so an app left running
    // overnight kept yesterday ringed while the now-line — whose column that
    // const chooses — ran off the bottom of yesterday and vanished (seen
    // live, 2026-08-19, after a night of suspend with the app open). The
    // premise that makes this assertable at all: the suite runs in UTC and
    // the fixture's days are UTC midnights, so a fake clock inside the week
    // really is "today" to the component.
    const DAY = 24 * 3_600_000;
    await page.clock.install({ time: MON + DAY + 14 * 3_600_000 }); // Tue 14:00
    await page.goto(show('WeekGrid', 'empty'));

    const ringed = page.locator('.head.today');
    await expect(ringed).toHaveCount(1);
    await expect(ringed).toContainText('TUE');
    await expect(page.locator('.col.today .now')).toBeVisible();

    // The laptop sleeps past midnight; the first thing after waking is the
    // window taking focus — the snap the fix listens for.
    await page.clock.setSystemTime(MON + 2 * DAY + 9 * 3_600_000); // Wed 09:00
    await page.evaluate(() => window.dispatchEvent(new Event('focus')));

    await expect(page.locator('.head.today')).toHaveCount(1);
    await expect(page.locator('.head.today')).toContainText('WED');
    await expect(page.locator('.col.today .now')).toBeVisible();
  });

  test('the hour ruler grows a second clock only when settings name one', async ({ page }) => {
    await page.goto(show('WeekGrid', 'empty'));

    // Off is the default, and off is byte-for-byte the ruler the committed
    // goldens hold: one label per hour, no zone captions, 44px of gutter.
    const ruler = page.getByTestId('week-body').locator('.gutter');
    await expect(ruler.locator('span.z2')).toHaveCount(0);
    await expect(page.locator('.zl')).toHaveCount(0);

    // App's one line, played by the spec (see mount.svelte.ts). The suite
    // runs in UTC and Kolkata is +05:30 with no DST, so every conversion is
    // a constant — and the half-hour offset is the interesting case, where
    // the second clock cannot just repeat the primary's digits.
    await page.evaluate(() => (window as any).__setSecondZone('Asia/Kolkata'));

    // The ruler's own spans, not the head's caption — which wears `.z2`
    // too, for the same lane.
    const z2 = ruler.locator('span.z2');
    await expect(z2).toHaveCount(24);
    await expect(z2.first()).toHaveText('05:30'); // the 00:00 UTC rule
    await expect(z2.nth(9)).toHaveText('14:30');  // the 09:00 UTC rule

    // Both clocks are captioned once there are two — the convenience zone
    // in the outer lane, the grid's own beside its columns. What the
    // engine calls Kolkata varies by ICU ("GMT+5:30" here, "IST" in an
    // Indian locale), so the assertion accepts the family, not one string.
    await expect(page.locator('.zl.z2')).toHaveText(/GMT\+5:30|IST/);
    await expect(page.locator('.zl.z1')).toHaveText(/UTC|GMT/);
  });

  // `WeekGrid` reads the real clock three times — `todayStart` at mount (the
  // accent day-number pill and the `.col.today` tint), `nowMs` for the
  // current-time line, and `Date.now()` again for where the grid opens
  // scrolled to. Every fixture here is anchored on `MON`, a Monday in 2024, so
  // none of those marks rendered into the two committed baselines — but only
  // because the *run date* fell outside that week, which nothing asserted.
  //
  // Freezing it makes that a property of the suite rather than of the calendar.
  // Before `page.goto`, because the component reads the clock at mount; same
  // mechanism as `MonthGrid`'s `MONTH_2026_NOW` and `YearGrid`'s
  // `YEAR_2026_NOW` above.
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(WEEK_NOW);
  });

  test('renders an empty week', async ({ page }) => {
    await page.goto(show('WeekGrid', 'empty'));
    await expect(page.locator('.col')).toHaveCount(7);
    await expect(page).toHaveScreenshot('weekgrid-empty.png');
  });

  /** The forecast in the day headers: each covered day draws its *own* sky
   *  and high — three distinct buckets, so a map read off by one would
   *  redden — and a day past the horizon draws nothing rather than a guess.
   *  Absence is the other half of the contract: without the prop (every
   *  other fixture) no header ever grows a sky. */
  test('a day header carries the forecast for its own day, and only then', async ({ page }) => {
    await page.goto(show('WeekGrid', 'weather'));
    const wx = page.locator('.head .wx');
    await expect(wx).toHaveCount(3);
    await expect(wx.nth(0)).toHaveText('31°');
    // No `title=`: the native tooltip is unstylable and ghosts on
    // translucent compositors (EventBlock's doctrine, applied here).
    await expect(wx.nth(0)).not.toHaveAttribute('title', /./);
    await expect(wx.nth(2)).toHaveText('2°');
    // Thursday is past the horizon: same header, no sky.
    await expect(page.locator('.head').nth(4).locator('.wx')).toHaveCount(0);

    await page.goto(show('WeekGrid', 'empty'));
    await expect(page.locator('.head .wx')).toHaveCount(0);
  });

  // Without this, `WEEK_NOW` is unfalsifiable: a `WeekGrid` that had lost the
  // today-highlight and the current-time line entirely would satisfy every
  // other spec in this block and both baselines, because none of them ever
  // renders a week containing today. Moving the clock *into* `MON`'s week is
  // what makes "the goldens show no today marks" a statement about the clock
  // rather than about the component being incapable of drawing them.
  test('the today-highlight and the current-time line appear when today is on screen', async ({ page }) => {
    await page.clock.setFixedTime(WEEK_NOW_INSIDE);
    await page.goto(show('WeekGrid', 'empty'));
    await expect(page.locator('.col')).toHaveCount(7);

    // Wednesday, and *only* Wednesday: a component that flagged every column
    // would pass a check aimed at one of them.
    const flagged: number[] = [];
    for (let i = 0; i < 7; i++) {
      if ((await page.locator('.col').nth(i).getAttribute('class'))?.includes('today')) flagged.push(i);
    }
    expect(flagged, 'exactly the third column is today').toEqual([2]);

    // And the line is drawn at the hour it is, not merely present: 10:30 of a
    // 24-hour day is 43.75% down the column.
    const line = page.locator('.now');
    await expect(line).toHaveCount(1);
    const pct = await line.evaluate((el) => parseFloat((el as HTMLElement).style.top));
    expect(pct).toBeCloseTo(100 * (10.5 / 24), 1);
  });

  /**
   * The current-time line reads `--now`, and `--error` cannot touch it.
   *
   * This is the whole reason task #52 was not a `sed`. `#e2564a` was written
   * literally in six places, and two of them were this line and its dot —
   * the same hex meaning something completely different from "something went
   * wrong". Collapsing all six onto one `--error` would have coupled them:
   * a theme wanting a calmer "now" indicator would have silently restyled
   * every error message in the app, and nothing would have failed.
   *
   * So the assertion is not that the line is red. It is that moving `--error`
   * leaves it alone and moving `--now` does not.
   */
  test('the current-time line follows --now and is untouched by --error', async ({ page }) => {
    await page.clock.setFixedTime(WEEK_NOW_INSIDE);
    await page.goto(show('WeekGrid', 'empty'));
    const line = page.locator('.now');
    await expect(line).toHaveCount(1);

    const lineColour = () =>
      line.evaluate((el) => getComputedStyle(el).getPropertyValue('border-top-color'));
    // `::before` is the 7px dot, and it takes the same variable — read
    // separately, because a rule that moved the line and left the dot behind
    // is exactly the kind of half-done change this is here to catch.
    const dotColour = () =>
      line.evaluate((el) => getComputedStyle(el, '::before').getPropertyValue('background-color'));
    const setVar = (name: string, value: string) =>
      page.evaluate(([n, v]) => document.documentElement.style.setProperty(n, v), [name, value]);

    // Same reasoning as the header's: no golden renders this line, because
    // every fixture here is a week that does not contain today. Without this
    // assertion `--now` could be published as any colour at all and the whole
    // suite would stay green.
    expect(await lineColour(), 'the current-time colour changed value').toBe('rgb(226, 86, 74)');

    const before = await lineColour();
    const dotBefore = await dotColour();

    await setVar('--error', 'rgb(0, 255, 0)');
    expect(await lineColour(), '--error reached the current-time line').toBe(before);
    expect(await dotColour(), '--error reached the current-time dot').toBe(dotBefore);

    await setVar('--now', 'rgb(0, 0, 255)');
    expect(await lineColour(), 'the line does not read --now').toBe('rgb(0, 0, 255)');
    expect(await dotColour(), 'the dot does not read --now').toBe('rgb(0, 0, 255)');
  });

  // The other half of the pair, and the one the baselines rest on.
  test('neither mark is drawn for a week that does not contain today', async ({ page }) => {
    await page.goto(show('WeekGrid', 'empty'));
    await expect(page.locator('.col')).toHaveCount(7);
    await expect(page.locator('.col.today')).toHaveCount(0);
    await expect(page.locator('.now')).toHaveCount(0);
  });

  // Task 10. Exactly on the 10:00 line, which is where somebody aims to make a
  // 10:00 meeting — and the one place the empty-space target does not receive
  // the click unless the hour rules are made transparent to the pointer. They
  // are positioned after it in the column, so a point within half a pixel of a
  // line returns `.rule` from `elementFromPoint` without
  // `pointer-events: none`; measured in both engines. A 1px dead band every
  // two hours is not something a user would ever report as a bug, only as the
  // app "sometimes not doing anything".
  test('clicking exactly on an hour line still asks for a new event there', async ({ page }) => {
    await page.goto(show('WeekGrid', 'empty'));
    const col = page.locator('.col').first();
    const box = (await col.boundingBox())!;
    await col.click({ position: { x: box.width / 2, y: box.height * (10 / 24) } });
    expect(await page.evaluate(() => (window as any).__lastCreate)).toMatchObject({
      startMs: MON + 10 * 3_600_000,
    });
  });

  // The same guard for the current-time line, which is the worse of the two:
  // it is 1.5px plus a 7px dot, and it crawls down today's column all day, so
  // the dead band it makes is both bigger and moving.
  //
  // I first left this unspec'd on the grounds that reaching `.now` needed a
  // fixture whose week moves with the calendar. That was wrong, and the fix is
  // the pattern this suite already uses for `YearGrid`'s today-highlight: the
  // fixture stays fixed in the past and the *clock* moves to it. Frozen at
  // 10:20 on `MON` itself, the first column becomes today, `.now` renders at
  // 10:20 — and the click at 10:00 lands on the hour line while the dot sits
  // 20 minutes below, so this covers `.rule` and `.now` at once without
  // needing to know exactly where the dot fell.
  test('the current-time line does not swallow a click either', async ({ page }) => {
    await page.clock.setFixedTime(MON + 10 * 3_600_000 + 20 * 60_000);
    await page.goto(show('WeekGrid', 'empty'));
    await expect(page.locator('.col.today .now')).toHaveCount(1);

    const col = page.locator('.col').first();
    const box = (await col.boundingBox())!;
    // Straight through the dot: it is drawn by `.now::before` at the line's
    // own left edge, so this is the pixel most likely to be intercepted.
    await col.click({ position: { x: 3, y: box.height * (10 + 20 / 60) / 24 } });
    expect(await page.evaluate(() => (window as any).__lastCreate)).toMatchObject({
      startMs: MON + 10 * 3_600_000,
    });
  });

  test('renders overlaps side by side', async ({ page }) => {
    await page.goto(show('WeekGrid', 'populated'));
    // Thursday's two identical-time meetings must not sit on top of each other.
    const blocks = page.locator('.col').nth(3).locator('.ev');
    await expect(blocks).toHaveCount(2);
    const a = await blocks.nth(0).boundingBox();
    const b = await blocks.nth(1).boundingBox();
    expect(a && b).toBeTruthy();
    expect(a!.x + a!.width).toBeLessThanOrEqual(b!.x + 1);
    await expect(page).toHaveScreenshot('weekgrid-populated.png');
  });

  test('a one-day grid renders a single column', async ({ page }) => {
    await page.goto('/tests/harness/index.html?c=WeekGrid&f=single-day');
    await expect(page.locator('.col')).toHaveCount(1);
  });

  // Whole-branch review, finding 4: `.col` counting alone never noticed that
  // `--cols` had gone back to a hard-coded 7 — the single column still
  // existed, it was just drawn in the first seventh of the grid (172px of
  // 1248) with six-sevenths of the screen blank beside it. `--cols` exists to
  // produce this geometry, so the geometry is what has to be asserted.
  test('a one-day grid gives the day the whole width', async ({ page }) => {
    await page.goto('/tests/harness/index.html?c=WeekGrid&f=single-day');
    const col = page.locator('.col');
    await expect(col).toHaveCount(1);
    const colBox = (await col.boundingBox())!;
    const gridBox = (await page.getByTestId('week-body').boundingBox())!;
    // Everything but the 44px hour gutter. 0.9 sits well clear of both
    // outcomes — ~0.965 correct, ~0.138 with the column stuck at one seventh.
    expect(colBox.width / gridBox.width).toBeGreaterThan(0.9);
  });

  test("a drag rewrites the card's own clock as it goes", async ({ page }) => {
    // Moving a block used to move the pixels and leave the label lying — the
    // card kept saying 11:00 while sitting on 12:00, and the truth arrived
    // only after the drop's confirm-and-reload (2026-08-21, by request).
    // The label reads the drag's own `landed`, the exact span a drop would
    // write, so mid-drag is when this must already be true.
    await page.goto(show('WeekGrid', 'populated'));
    const b = page.getByRole('button', { name: /^Excitel weekly,/ });
    const box = (await b.boundingBox())!;
    const col = (await page.locator('.col').first().boundingBox())!;
    const hour = col.height / 24;

    // Move down one hour: the whole span slides, 11:00–12:00 → 12:00–13:00,
    // and the accessible name says so before anything is released.
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + hour, { steps: 4 });
    await expect(page.getByRole('button', { name: /^Excitel weekly, 12:00 to 13:00/ }))
      .toBeVisible();
    // Escape puts it back — the label with it.
    await page.keyboard.press('Escape');
    await page.mouse.up();
    await expect(page.getByRole('button', { name: /^Excitel weekly, 11:00 to 12:00/ }))
      .toBeVisible();

    // Resize by the bottom edge to two hours: past the 90-minute rung of the
    // density ladder, so the card's own time line *appears* mid-drag — and
    // reads the stretched span, not the stored one.
    await page.mouse.move(box.x + box.width / 2, box.y + box.height - 3);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height - 3 + hour, { steps: 4 });
    await expect(b.locator('em').first()).toHaveText('11:00 – 13:00');
    await page.keyboard.press('Escape');
    await page.mouse.up();
    // Back to an hour: the time line is gone again, leaving only the meta
    // line — the ladder stepped back down with the cancelled span.
    await expect(b.locator('em')).toHaveText(['Meet']);
  });

  test('overlapping events fan out fully in a one-day grid', async ({ page }) => {
    // Spec §4: Day always fans out rather than stacking into columns — there is
    // width to spare and no reason to compress.
    await page.goto('/tests/harness/index.html?c=WeekGrid&f=single-day-overlap');
    const blocks = page.locator('.col .ev');
    await expect(blocks).toHaveCount(2);
    const a = await blocks.nth(0).boundingBox();
    const b = await blocks.nth(1).boundingBox();
    expect(a!.x).not.toBe(b!.x);
    // 80 is not a day/week boundary — a day-wide half renders around 600px,
    // a week-wide one around 82px. It is only a sanity floor against "fanned
    // but squeezed to nothing"; do not tune it as if it separated the two.
    expect(Math.min(a!.width, b!.width)).toBeGreaterThan(80);
  });
});

// Fix round 1: `EventPopover`'s own specs mount it standalone against a
// fixture that already carries the right `occurrenceStartMs` — they can only
// prove the popover honours its own prop, never that `WeekGrid` computed
// that prop correctly from the clicked block in the first place. These
// specs click a real block in a real `WeekGrid` instead, exercising
// `openPopover` end to end: the trap itself, the supersession guard, the
// load-failure close, the after-paint refresh, and the optimistic restyle.
test.describe('WeekGrid popover flow', () => {
  const show = (f: string) => `/tests/harness/index.html?c=WeekGrid&f=${f}`;

  test("responding sends the clicked block's own start, not the series DTSTART", async ({ page }) => {
    await page.goto(show('popover'));
    await page.getByRole('button', { name: 'Standup' }).click();
    await expect(page.locator('.pop')).toBeVisible();
    await page.getByRole('button', { name: 'No' }).click();
    await page.getByRole('button', { name: 'This one' }).click();
    const call = await page.evaluate(() => (window as any).__lastRespondCall);
    // POPOVER_DETAILS[42].start_ms (the series DTSTART) vs the block's own
    // start_ms (POPOVER_RECURRING's, the fourth occurrence) — see fixtures.ts.
    expect(call.occurrenceStartMs).toBe(POPOVER_DETAILS[42].start_ms + 3 * 24 * 3_600_000);
    expect(call.occurrenceStartMs).not.toBe(POPOVER_DETAILS[42].start_ms);
  });

  // Task 10, and the same property as the spec above through a different code
  // path: Edit and Delete hand the caller an `Occurrence`, and its `startMs`
  // must be the clicked block's, never `detail.start_ms`. Both controls in one
  // spec, because they are the same relay called twice — and the popover has
  // to be gone by the time either lands, or the form would open behind a scrim
  // that is still there.
  test('edit and delete hand up the clicked block, and close the popover', async ({ page }) => {
    const seriesStart = POPOVER_DETAILS[42].start_ms;
    const blockStart = seriesStart + 3 * 24 * 3_600_000;

    await page.goto(show('popover'));
    await page.getByRole('button', { name: 'Standup' }).click();
    await expect(page.locator('.pop')).toBeVisible();
    await page.getByRole('button', { name: 'Edit' }).click();
    await expect(page.locator('.pop')).toHaveCount(0);
    const edit = await page.evaluate(() => (window as any).__lastEdit);
    expect(edit.occurrence.startMs).toBe(blockStart);
    expect(edit.occurrence.startMs).not.toBe(seriesStart);
    expect(edit.occurrence.endMs).toBe(blockStart + 30 * 60_000);

    await page.getByRole('button', { name: 'Standup' }).click();
    await expect(page.locator('.pop')).toBeVisible();
    await page.getByRole('button', { name: 'Delete' }).click();
    await expect(page.locator('.pop')).toHaveCount(0);
    const del = await page.evaluate(() => (window as any).__lastDelete);
    expect(del.occurrence.startMs).toBe(blockStart);
    expect(del.occurrence.startMs).not.toBe(seriesStart);
  });

  test('Ctrl+C hands up the clicked block and leaves the popover open', async ({ page }) => {
    const seriesStart = POPOVER_DETAILS[42].start_ms;
    const blockStart = seriesStart + 3 * 24 * 3_600_000;

    await page.goto(show('popover'));
    await page.getByRole('button', { name: 'Standup' }).click();
    await expect(page.locator('.pop')).toBeVisible();
    await page.keyboard.press('Control+c');
    // The same occurrence discipline as Edit and Delete: the clicked block's
    // own start, never the series DTSTART.
    const copy = await page.evaluate(() => (window as any).__lastCopy);
    expect(copy.occurrence.startMs).toBe(blockStart);
    expect(copy.occurrence.startMs).not.toBe(seriesStart);
    // A copy is not a dismissal — the popover stays, and says what happened.
    await expect(page.locator('.pop')).toBeVisible();
    await expect(page.locator('.pop .note')).toContainText('Copied');
  });

  test('Ctrl+C over selected text stays native copy, not an event copy', async ({ page }) => {
    await page.goto(show('popover'));
    await page.getByRole('button', { name: 'Standup' }).click();
    await expect(page.locator('.pop')).toBeVisible();
    // A real selection inside the panel — the chord must keep its older
    // meaning for it.
    await page.evaluate(() => {
      const h = document.querySelector('.pop h2, .pop h3, .pop [class*=title]') ??
        document.querySelector('.pop');
      const range = document.createRange();
      range.selectNodeContents(h!);
      const sel = window.getSelection()!;
      sel.removeAllRanges();
      sel.addRange(range);
    });
    await page.keyboard.press('Control+c');
    expect(await page.evaluate(() => (window as any).__lastCopy)).toBeNull();
  });

  test('a successful response restyles the clicked block without a refetch', async ({ page }) => {
    await page.goto(show('popover'));
    const block = page.getByRole('button', { name: 'Standup' });
    await expect(block).toHaveClass(/needsAction/);
    await block.click();
    await page.getByRole('button', { name: 'No' }).click();
    await page.getByRole('button', { name: 'This one' }).click();
    await expect(block).toHaveClass(/declined/);
  });

  test('closing the popover mid-RSVP still restyles the block once the response lands', async ({ page }) => {
    // `detail` inside EventPopover is a live prop, not a snapshot: closing
    // the popover (the scrim, here) while `respondToEvent` is still in
    // flight sets WeekGrid's own `detail` to null, and `respond()`'s
    // closure keeps running regardless — exactly the case `onresponded`'s
    // restyle exists to still get right.
    await page.goto(show('popover'));
    await page.evaluate(() => window.__harness.holdNextEventCall('respond_to_event', 42));
    const block = page.getByRole('button', { name: 'Standup' });
    await block.click();
    await expect(page.locator('.pop')).toBeVisible();
    await page.getByRole('button', { name: 'No' }).click();
    await page.getByRole('button', { name: 'This one' }).click(); // parked mid-flight
    await page.locator('.scrim').click(); // close before the response lands
    await expect(page.locator('.pop')).toHaveCount(0);
    await page.evaluate(
      (detail) => window.__harness.releaseEventCall('respond_to_event', 42, detail),
      POPOVER_DETAILS[42],
    );
    await expect(block).toHaveClass(/declined/);
  });

  test('the popover updates in place once the after-paint refresh lands', async ({ page }) => {
    await page.goto(show('popover'));
    await page.evaluate(() => window.__harness.holdNextEventCall('refresh_event', 50));
    await page.getByRole('button', { name: 'Sync' }).click();
    await expect(page.locator('.loc')).toHaveText('Room A');
    await page.evaluate(
      (detail) => window.__harness.releaseEventCall('refresh_event', 50, detail),
      POPOVER_REFRESHED_DETAIL,
    );
    await expect(page.locator('.loc')).toHaveText('Room B');
  });

  test('a failed load never shows an empty popover', async ({ page }) => {
    await page.goto(show('popover'));
    await page.evaluate(() => window.__harness.failNextEventCall('event_detail', 60, 'offline'));
    await page.getByRole('button', { name: 'Event A' }).click();
    await expect(page.locator('.pop')).toHaveCount(0);
  });

  test('a late failure for a superseded click does not close a popover that opened after it', async ({ page }) => {
    // The failure counterpart to the "stale detail" spec below: block A's
    // load is still in flight when B is opened and succeeds; A's load then
    // fails. Without the `isSelected` guard on the catch branch, that late
    // failure would call `closePopover()` unconditionally and tear down
    // B's already-open, already-successful popover.
    await page.goto(show('popover'));
    await page.evaluate(() => window.__harness.holdNextEventCall('event_detail', 60));
    await page.getByRole('button', { name: 'Event A' }).click(); // parked
    await page.getByRole('button', { name: 'Event B' }).click(); // succeeds
    await expect(page.locator('.pop h2')).toHaveText('Event B');
    await page.evaluate(() => window.__harness.rejectEventCall('event_detail', 60, 'offline'));
    await expect(page.locator('.pop')).toBeVisible();
    await expect(page.locator('.pop h2')).toHaveText('Event B');
  });

  test('a stale detail arriving after a second block was opened is ignored', async ({ page }) => {
    await page.goto(show('popover'));
    await page.evaluate(() => window.__harness.holdNextEventCall('event_detail', 60));
    await page.getByRole('button', { name: 'Event A' }).click(); // parked
    await page.getByRole('button', { name: 'Event B' }).click(); // answers immediately
    await expect(page.locator('.pop h2')).toHaveText('Event B');
    await page.evaluate(
      (detail) => window.__harness.releaseEventCall('event_detail', 60, detail),
      POPOVER_DETAILS[60],
    );
    // Still B — the late arrival for A must not clobber what's on screen.
    await expect(page.locator('.pop h2')).toHaveText('Event B');
  });

  test('an override survives a payload that still disagrees with the baseline it was recorded against', async ({ page }) => {
    await page.goto(show('popover'));
    const block = page.getByRole('button', { name: 'Standup' });
    await block.click();
    await page.getByRole('button', { name: 'No' }).click();
    await page.getByRole('button', { name: 'This one' }).click();
    await expect(block).toHaveClass(/declined/);

    // A fresh sync lands (what App.svelte's loadWeek does after a real
    // sync — replaces `week` wholesale), but Standup's own response in it
    // still reads 'needsAction', exactly what it was when the override was
    // recorded. Nothing has actually caught up yet, so the override must
    // still win.
    await page.evaluate((week) => (window as any).__setWeek(week), popoverWeekWithResponse(42, 'needsAction'));
    await expect(block).toHaveClass(/declined/);
  });

  test('an override clears once the payload moves off the baseline it was recorded against', async ({ page }) => {
    await page.goto(show('popover'));
    const block = page.getByRole('button', { name: 'Standup' });
    await block.click();
    await page.getByRole('button', { name: 'No' }).click();
    await page.getByRole('button', { name: 'This one' }).click();
    await expect(block).toHaveClass(/declined/);

    // A fresh sync lands with a response that differs from the baseline —
    // accepted from another device, or anything else. Without eviction, the
    // override would keep masking every future payload for the rest of the
    // session; the payload must win once it actually disagrees.
    await page.evaluate((week) => (window as any).__setWeek(week), popoverWeekWithResponse(42, 'accepted'));
    await expect(block).toHaveClass(/accepted/);
    await expect(block).not.toHaveClass(/declined/);
  });

  test('a late failure for one occurrence does not close a popover open for a different occurrence sharing the same id', async ({ page }) => {
    // Coverage gap: every other fixture here has at most one occurrence per
    // store row id, so dropping `start_ms` from `isSelected` (leaving only
    // `id`) still passed every other spec in this file. Two occurrences of
    // one series, sharing an id, close it.
    await page.goto(show('popover-two-occurrences'));
    await page.evaluate(() => window.__harness.holdNextEventCall('event_detail', 70));
    await page.getByRole('button', { name: 'Daily sync 1' }).click(); // parked
    await page.getByRole('button', { name: 'Daily sync 2' }).click(); // same id, different start_ms — succeeds
    await expect(page.locator('.pop')).toBeVisible();
    await page.evaluate(() => window.__harness.rejectEventCall('event_detail', 70, 'offline'));
    // Occurrence 1's late failure must not close occurrence 2's popover —
    // it would, if `isSelected` compared `id` alone.
    await expect(page.locator('.pop')).toBeVisible();
  });

  // `commands::assemble_week` routes every `is_all_day` event into
  // `all_day_events` and never into a day column, so a band chip is the only
  // representation one ever gets. The chips were plain `<div>`s carrying a
  // `title` and nothing else — no click, no role, no tab stop — which meant
  // an all-day off-site with a guest list simply could not be opened.
  test('an all-day chip opens the popover with its guest list', async ({ page }) => {
    await page.goto(show('popover-all-day'));
    await page.getByRole('button', { name: 'Team off-site' }).click();
    await expect(page.locator('.pop h2')).toHaveText('Team off-site');
    const guests = page.locator('.pop .guest');
    await expect(guests).toHaveCount(2);
    // `.who` rather than the row: the row also carries the status glyph and a
    // visually-hidden status word, and asserting on the whole row would break
    // every time either changes while proving nothing extra about who is here.
    await expect(guests.nth(0).locator('.who')).toHaveText('Ana');
    await expect(guests.nth(1).locator('.who')).toContainText('(you)');
  });

  test('an all-day chip opens from the keyboard, not only the mouse', async ({ page }) => {
    // Free with a real `<button>`, and only with one: a `<div role="button">`
    // would need its own keydown handler, and a bare `<div>` — what this was
    // — is not reachable by Tab at all. Following `EventBlock`'s element
    // choice is what makes this pass with no key handling in `AllDayBand`.
    await page.goto(show('popover-all-day'));
    await page.getByRole('button', { name: 'Team off-site' }).focus();
    await page.keyboard.press('Enter');
    await expect(page.locator('.pop h2')).toHaveText('Team off-site');
  });

  // The one that matters. All-day occurrences are contiguous by construction
  // — each ends exactly where the next begins — which is the shape the
  // backend's instance lookup resolves most delicately, and why the bracket
  // fix and the body-provenance fix had to land before this path was opened
  // at all. If the chip passed anything but its own `start_ms`, the RSVP
  // would land on the series' first day with `sendUpdates=all`.
  test("an all-day recurring RSVP sends the clicked day, not the series start", async ({ page }) => {
    await page.goto(show('popover-all-day'));
    await page.getByRole('button', { name: 'Diwali' }).click();
    await expect(page.locator('.pop')).toBeVisible();
    await page.getByRole('button', { name: 'No' }).click();
    await page.getByRole('button', { name: 'This one' }).click();
    const call = await page.evaluate(() => (window as any).__lastRespondCall);
    // POPOVER_DETAILS[81].start_ms is the series DTSTART; the clicked chip is
    // the third day of the series — see fixtures.ts.
    expect(call.occurrenceStartMs).toBe(POPOVER_DETAILS[81].start_ms + 2 * 24 * 3_600_000);
    expect(call.occurrenceStartMs).not.toBe(POPOVER_DETAILS[81].start_ms);
    expect(call.scope).toBe('this');
  });
});

test.describe('EventBlock duration ladder', () => {
  test('15 minutes shows title only', async ({ page }) => {
    await page.goto(show('EventBlock', 'ladder-15'));
    await expect(page.locator('.ev b')).toHaveText('Sync w/ Ivan');
    await expect(page.locator('.ev em')).toHaveCount(0);
  });

  test('60 minutes adds one meta line', async ({ page }) => {
    await page.goto(show('EventBlock', 'ladder-60'));
    await expect(page.locator('.ev em')).toHaveCount(1);
  });

  test('120 minutes gives the time its own line', async ({ page }) => {
    await page.goto(show('EventBlock', 'ladder-120'));
    await expect(page.locator('.ev em')).toHaveCount(2);
  });

  test('the ends wear the resize cursor, exactly where a press would resize', async ({ page }) => {
    // The resize itself has worked since 856406b; what was missing was any
    // way to know it was there — the block's only cursor was `grab`, so the
    // feature shipped invisible (reported 2026-08-26 as a request for the
    // feature that already existed). The grips carry `ns-resize` over
    // `edgeAt`'s own bands — same constant, same short-block rule — and this
    // asserts the promise matches the behaviour on both sides of that rule.
    await page.goto(show('EventBlock', 'ladder-60'));
    const grips = page.locator('.grip');
    await expect(grips).toHaveCount(2);
    for (const g of await grips.all()) {
      expect(await g.evaluate((el) => getComputedStyle(el).cursor)).toBe('ns-resize');
    }

    // A 15-minute block has no edges at all — `edgeAt` answers null there,
    // so every press stays the move gesture — and it must therefore make
    // no resize promise either.
    await page.goto(show('EventBlock', 'ladder-15'));
    await expect(page.locator('.ev')).toBeVisible();
    await expect(page.locator('.grip')).toHaveCount(0);
  });
});

// The frame is `.ev`, not `#app`, and that is the whole point of these four.
//
// `#app` here is the 220x480 box `mount.svelte.ts` gives an absolutely
// positioned block. The block itself is `placed(0.2, 15 / 1440)` — 1.042% of
// that height, so **1,100 of the golden's 105,600 pixels**. Against the
// `maxDiffPixelRatio: 0.01` this suite used to run, the allowance was 1,056:
// the entire subject of the screenshot could be erased and the assertion came
// within 44 pixels of still passing. A golden that cannot notice its subject
// disappearing is not witnessing anything.
//
// Framing the element instead makes every pixel in the image a pixel of the
// thing under test, which is what `AllDayBand chip corners` below already does
// and for the same reason. What these four witness is unchanged and is stated
// by `EventBlock.svelte` itself: "State is carried by the fill, so it survives
// at 15 minutes tall." At this size there is no room for the title or the meta
// line, so the fill, the dashed ring and the spine are the whole of it.
test.describe('EventBlock RSVP states at 15 minutes', () => {
  for (const state of ['accepted', 'needsAction', 'tentative', 'declined']) {
    test(`${state} is visually distinct`, async ({ page }) => {
      await page.goto(show('EventBlock', `rsvp-${state}-15`));
      await expect(page.locator('.ev')).toHaveClass(new RegExp(state));
      await expect(page.locator('.ev')).toHaveScreenshot(`rsvp-${state}-15.png`);
    });
  }

  // `?` means maybe here too — one language across the grid and the popover.
  // Both halves asserted, because a marker that moved to the wrong state
  // passes either test alone.
  test('a maybe carries its marker', async ({ page }) => {
    await page.goto(show('EventBlock', 'rsvp-tentative-15'));
    await expect(page.locator('.ev .rs')).toHaveText('?');
  });

  test('an unanswered invite carries no letter, only its dashed ring', async ({ page }) => {
    await page.goto(show('EventBlock', 'rsvp-needsAction-15'));
    await expect(page.locator('.ev .rs')).toHaveCount(0);
  });
});

// The hover tooltip. Ours, not the webview's: `title=` renders as the
// engine's native tooltip, which no stylesheet can reach — engine-grey on
// engine-black, invisible on a dark theme — and it named the event without
// answering *when*, the one thing a block too short for its time line
// cannot say (fixture: 60 minutes, below the 90-minute time-line rung).
test.describe('EventBlock tooltip', () => {
  test('hovering says the whole story: title, times, place', async ({ page }) => {
    await page.goto(show('EventBlock', 'ladder-60'));
    const ev = page.locator('.ev');

    // The native tooltip is gone — leaving it would show two.
    expect(await ev.getAttribute('title')).toBeNull();
    // A screen reader hears what the card shows, from the button itself.
    await expect(ev).toHaveAttribute(
      'aria-label', 'Excitel weekly, 09:00 to 10:00, Room 4A');

    await ev.hover();
    const tip = page.locator('.tip');
    await expect(tip).toContainText('Excitel weekly');
    await expect(tip).toContainText('09:00 – 10:00');
    await expect(tip).toContainText('Room 4A');
    // It genuinely appears: the 300ms mount delay is opacity, which Playwright
    // counts as visible, so visibility is asserted on the computed style.
    await expect.poll(() => tip.evaluate((el) => getComputedStyle(el).opacity)).toBe('1');

    // And leaves with the pointer.
    await page.mouse.move(0, 0);
    await expect(tip).toHaveCount(0);
  });

  test('hover outranks the inline column z-index', async ({ page }) => {
    // The block carries an inline `z-index: column+1`, and an inline style
    // beats a plain hover rule — which left a hovered block (tooltip and
    // all, `position: fixed` notwithstanding: the tooltip lives inside the
    // block's stacking context) UNDER its higher-column neighbours in any
    // dense week. The fix is `!important`, same as the hover rule's own
    // `left`/`width` and the `.dragging` rule always had; this pins the
    // computed value so the inline style can never quietly win again.
    await page.goto(show('EventBlock', 'ladder-60'));
    const ev = page.locator('.ev');
    await ev.hover();
    await expect.poll(() => ev.evaluate((el) => getComputedStyle(el).zIndex)).toBe('20');
  });
});

// A hovered block widens over its neighbours. The resting fills are near-
// transparent by design, so if hover does not also make the block opaque, the
// covered block's title reads straight through it — two labels on top of each
// other, which is worse than the squeeze hover exists to relieve.
test.describe('EventBlock hover occludes what it covers', () => {
  // Chromium reports color-mix() results as `color(srgb r g b / a)` and plain
  // colours as `rgb(...)`/`rgba(...)`. Anything else THROWS rather than
  // defaulting to opaque: a parser that assumes the good case on an
  // unrecognised format silently passes the exact test it exists to fail.
  const alpha = (css: string): number => {
    const fn = css.match(/^color\([^/)]*(?:\/\s*([0-9.]+))?\)$/);
    if (fn) return fn[1] === undefined ? 1 : parseFloat(fn[1]);
    const rgb = css.match(/^rgba?\(([^)]+)\)$/);
    if (rgb) {
      const parts = rgb[1].split(/[,\s/]+/).filter(Boolean).map(parseFloat);
      return parts.length < 4 ? 1 : parts[3];
    }
    throw new Error(`unrecognised colour format, cannot assess opacity: ${css}`);
  };

  for (const state of ['accepted', 'needsAction', 'tentative', 'declined']) {
    // Blocks overlap constantly, and every state must occlude the one behind it
    // — at rest, not only under the cursor. A translucent block lets the covered
    // event's title read through it and its rounded corners poke past, which is
    // what "ugly corners" turned out to be.
    test(`${state} is opaque at rest and on hover`, async ({ page }) => {
      await page.goto(show('EventBlock', `rsvp-${state}-15`));
      const ev = page.locator('.ev');

      const read = async () => ({
        bg: await ev.evaluate((el) => getComputedStyle(el).backgroundColor),
        op: await ev.evaluate((el) => parseFloat(getComputedStyle(el).opacity)),
      });

      const rest = await read();
      expect(alpha(rest.bg), `resting ${state} background must be opaque, got ${rest.bg}`).toBe(1);
      // Element opacity makes a block see-through regardless of its background,
      // so fading must be done with colours instead.
      expect(rest.op, `resting ${state} element opacity must be 1`).toBe(1);

      await ev.hover();
      const hov = await read();
      expect(alpha(hov.bg), `hovered ${state} background must be opaque, got ${hov.bg}`).toBe(1);
      expect(hov.op, `hovered ${state} element opacity must be 1`).toBe(1);
    });
  }

  // Element opacity used to do the fading. Colour has to carry it now, or
  // removing the transparency would quietly turn "declined" into "accepted".
  test('a declined block still reads as declined', async ({ page }) => {
    await page.goto(show('EventBlock', 'rsvp-declined-15'));
    await expect(page.locator('.ev b')).toHaveCSS('text-decoration-line', 'line-through');
    const declined = await page.locator('.ev').evaluate((el) => getComputedStyle(el).color);

    await page.goto(show('EventBlock', 'rsvp-accepted-15'));
    const accepted = await page.locator('.ev').evaluate((el) => getComputedStyle(el).color);

    expect(declined, 'declined must not render the same text colour as accepted')
      .not.toBe(accepted);
  });
});

test.describe('AllDayBand', () => {
  test('spans the right columns and flags a continuation', async ({ page }) => {
    await page.goto(show('AllDayBand', 'populated'));
    const chips = page.locator('.chip');
    await expect(chips).toHaveCount(2);
    // The span arriving from last week gets the flat dashed edge.
    await expect(chips.nth(1)).toHaveClass(/cl/);
    await expect(chips.nth(1)).toContainText('‹');
    await expect(page.locator('#app')).toHaveScreenshot('allday-populated.png');
  });

  test('reports overflow', async ({ page }) => {
    await page.goto(show('AllDayBand', 'overflow'));
    await expect(page.locator('.more')).toHaveText('+2 more');
  });

  test('renders nothing when there is nothing to show', async ({ page }) => {
    await page.goto(show('AllDayBand', 'empty'));
    await expect(page.locator('.band')).toHaveCount(0);
  });
});

// The chip's colour spine is a border on one side only, meeting a
// border-radius. That is the exact geometry behind the artifact
// `EventBlock.svelte` documents: WebKit derives each corner's curve from the
// two border widths meeting there, and in macOS WKWebView the corners away
// from the border rendered square. `EventBlock` removed the cause by
// replacing its spine with an inset shadow; a chip cannot, because `.cl` has
// to draw that spine *dashed*, which no shadow can do. So the chip keeps the
// shape and this guards it instead.
//
// Per chip and at zero tolerance, neither of which is incidental. The band's
// own `allday-populated.png` is 1280x42 under the config's
// `maxDiffPixelRatio: 0.01` — about 537 pixels of slack, against roughly 3-4
// pixels per corner. That snapshot would not notice this artifact returning;
// it has ~5% of its budget to spare on it. A chip-sized frame at
// `maxDiffPixels: 0` has none.
//
// `threshold: 0` is load-bearing, and not obviously so. `maxDiffPixels: 0`
// alone does nothing here: `threshold` is the *per-pixel* tolerance pixelmatch
// applies before a pixel counts as differing at all, and at its default of 0.2
// a squared-off corner is invisible. The chip's fill is
// `color-mix(… 16%, transparent)`, so a corner pixel flipping from page
// background to chip fill moves (23,23,26) to about (55,45,32) — a YIQ delta
// of ~314 against the 1409 that threshold 0.2 permits. Being nearly
// transparent is exactly what makes this artifact cheap to miss. Even
// threshold 0.1 (1409 -> 352) still ignores it; anything at or above ~0.095
// does. Verified by mutation, not by reading: squaring the two corners away
// from the border passes at the default and fails at 0.
//
// Zero costs nothing in stability here — the four baselines below were
// produced by a different element type on a different run and match byte for
// byte — because the frame holds no antialiased text edges that move between
// runs on a fixed platform.
//
// These four baselines were generated from the pre-change `<div>` markup
// (`git show 6d278b8:ui/src/lib/AllDayBand.svelte`) and are committed
// unmodified: the `<button>` this became reproduces them pixel for pixel in
// both engines, which is the evidence that the swap cost nothing. From here
// they guard against the artifact appearing, in either direction.
test.describe('AllDayBand chip corners', () => {
  const CHIPS = ['plain', 'cont-left', 'cont-right', 'cont-both'];
  for (const [i, name] of CHIPS.entries()) {
    test(`a ${name} chip renders pixel for pixel`, async ({ page }) => {
      await page.goto(show('AllDayBand', 'corners'));
      await expect(page.locator('.chip').nth(i)).toHaveScreenshot(
        `allday-chip-${name}.png`,
        { maxDiffPixels: 0, maxDiffPixelRatio: 0, threshold: 0 },
      );
    });
  }
});

test.describe('Header', () => {
  /** Opens the hamburger. Everything rare lives behind it now (spec §1), so
   *  most of the specs below have to get there first — which is itself the
   *  behaviour being asserted. */
  const openMenu = (page: import('@playwright/test').Page) =>
    page.getByRole('button', { name: 'Menu' }).click();

  /** The status light, by what it *says* rather than by what colour it is —
   *  spec §5. Reading a computed colour would prove the stylesheet resolved a
   *  variable, which is a fact about CSS and not about sync. */
  const lightName = async (page: import('@playwright/test').Page) =>
    page.locator('.light').getAttribute('aria-label');

  test('disconnected state offers to connect', async ({ page }) => {
    await page.goto(show('Header', 'disconnected'));
    await expect(page.getByRole('button', { name: 'Connect Google Calendar' })).toBeVisible();
    await expect(page.locator('header')).toHaveScreenshot('header-disconnected.png');
  });

  /**
   * **The sentence moved into the light** (spec §2). `Synced 5 min ago` used to
   * be text in the header; it is now the dot's accessible name and its `title`,
   * so hovering still answers *when* precisely while the header stays quiet.
   *
   * The clock is frozen for the reason it always was: `relativeTime` reads the
   * real wall clock, so "N min ago" drifts with the run date otherwise.
   */
  test('connected state carries the sync time in the light, not in words', async ({ page }) => {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(show('Header', 'connected'));

    expect(await lightName(page)).toBe('Synced 5 min ago');
    await expect(page.locator('.light')).toHaveAttribute('title', 'Synced 5 min ago');
    await expect(page.locator('.light')).toHaveClass(/\bsynced\b/);
    // And nothing in the header says it out loud any more.
    await expect(page.locator('header')).not.toContainText('Synced');
    await expect(page.locator('header')).toHaveScreenshot('header-connected.png');
  });

  /** The reconnect banner, from the fixture side: it names the account (with
   *  two connected, "an account" is a guessing game) and carries the button.
   *  The absence check runs over `connected` — same account, healthy — so
   *  what is asserted is the field gating it, not the fixture existing. */
  test('a dead account is named, and offered a reconnect', async ({ page }) => {
    await page.goto(show('Header', 'reauth'));
    const banner = page.getByTestId('reauth-banner');
    await expect(banner).toContainText('Google sign-in for me@x.com is no longer valid');
    await expect(banner.getByRole('button', { name: 'Reconnect' })).toBeEnabled();

    await page.goto(show('Header', 'connected'));
    await expect(page.getByTestId('reauth-banner')).toHaveCount(0);
  });

  /** The update notice: present when status carries one, absent otherwise
   *  (the absence over `connected` is what pins the field as the gate), calm
   *  rather than alarming (not `.err` — teaching users to ignore red is the
   *  one thing a status surface must not do), and dismissible for the
   *  session. */
  test('a newer release is offered quietly, and can be waved away', async ({ page }) => {
    await page.goto(show('Header', 'update'));
    const banner = page.getByTestId('update-banner');
    await expect(banner).toContainText('OmaCal 0.2.0 is available');
    await expect(banner).not.toHaveClass(/\berr\b/);
    await expect(banner.getByRole('button', { name: "What's new" })).toBeEnabled();

    await banner.getByRole('button', { name: 'Dismiss update notice' }).click();
    await expect(page.getByTestId('update-banner')).toHaveCount(0);

    await page.goto(show('Header', 'connected'));
    await expect(page.getByTestId('update-banner')).toHaveCount(0);

    // The packaged shape, pinned from the same fixture: no "Update" button
    // where a package manager owns the files — the sentence is the action.
    await page.goto(show('Header', 'update'));
    await expect(page.getByTestId('update-banner')).toContainText('Re-run the install command');
    await expect(page.getByTestId('update-banner').getByRole('button', { name: 'Update', exact: true })).toHaveCount(0);
  });

  /** The AppImage's shape of the same notice: `self_update` turns the
   *  sentence into an "Update" button. Clicking it asks the backend once and
   *  acknowledges in place — the process is about to restart out from under
   *  the button, so "Updating…" is the last thing this window ever says. */
  test('an AppImage offers Update, and the click is acknowledged', async ({ page }) => {
    await page.goto(show('Header', 'updatable'));
    const banner = page.getByTestId('update-banner');
    await expect(banner).toContainText('OmaCal 0.2.0 is available');
    await expect(banner).not.toContainText('Re-run the install command');

    const button = banner.getByRole('button', { name: 'Update', exact: true });
    await button.click();
    await expect(banner.getByRole('button', { name: 'Updating…' })).toBeDisabled();
  });

  /** The failed attempt reports in place and the offer stands back up: the
   *  running copy is untouched — the updater writes aside and renames — so
   *  trying again is always legitimate, and a dead button would turn one
   *  network hiccup into a permanently spent banner. */
  test('a failed update says what happened and offers again', async ({ page }) => {
    await page.goto(show('Header', 'updatefails'));
    const banner = page.getByTestId('update-banner');

    await banner.getByRole('button', { name: 'Update', exact: true }).click();
    await expect(banner).toContainText('Could not reach the release server');
    await expect(banner.getByRole('button', { name: 'Update', exact: true })).toBeEnabled();
  });

  /** The moved-zone banner: present when status says the system zone left
   *  this process behind, absent over the same account healthy (the field is
   *  the gate), red unlike the update notice (every hour on the grid is
   *  currently wrong data, not a pending option), naming where the machine
   *  went, and carrying the restart that is its only real fix — which
   *  acknowledges the click, because the re-exec takes a beat to arrive. */
  test('a system zone that moved is named, offered a restart, dismissible', async ({ page }) => {
    await page.goto(show('Header', 'tzchange'));
    const banner = page.getByTestId('tz-banner');
    await expect(banner).toContainText('This machine moved to Asia/Kolkata');
    await expect(banner).toHaveClass(/\berr\b/);

    const restart = banner.getByRole('button', { name: 'Restart' });
    await restart.click();
    await expect(banner.getByRole('button', { name: 'Restarting…' })).toBeDisabled();

    await banner.getByRole('button', { name: 'Dismiss time zone notice' }).click();
    await expect(page.getByTestId('tz-banner')).toHaveCount(0);

    await page.goto(show('Header', 'connected'));
    await expect(page.getByTestId('tz-banner')).toHaveCount(0);
  });

  /** The Settings colophon: the one place the app says what version it is —
   *  which a bug report needs findable, and the update notice's "0.2.0 is
   *  available" needs comparable against. '9.9.9' is the fixture default,
   *  nothing a real build would carry, so this fails when the wiring breaks
   *  rather than riding along on a hardcoded string. */
  test('Settings names the running version', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    await openMenu(page);
    await page.getByRole('button', { name: 'Settings…' }).click();
    await expect(page.getByTestId('app-version')).toHaveText('OmaCal 9.9.9');
  });

  /** §1: the three rare controls are gone from the header itself. Asserted as
   *  an absence *before* the menu is opened, which is the half that says they
   *  moved rather than merely that they exist somewhere. */
  test('the header itself carries none of the three moved controls', async ({ page }) => {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(show('Header', 'connected'));

    await expect(page.getByRole('button', { name: 'Sync now' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Add account' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: /^Calendars/ })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Menu' })).toBeVisible();
  });

  test('the hamburger holds all three', async ({ page }) => {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(show('Header', 'connected'));
    await openMenu(page);

    await expect(page.getByRole('button', { name: 'Sync now' })).toBeEnabled();
    await expect(page.getByRole('button', { name: 'Add account' })).toBeVisible();
    await expect(page.getByRole('button', { name: /^Calendars/ })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Settings…' })).toBeVisible();
  });

  test('Escape closes the menu', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    await openMenu(page);
    await expect(page.getByRole('button', { name: 'Settings…' })).toBeVisible();

    await page.keyboard.press('Escape');
    await expect(page.getByRole('button', { name: 'Settings…' })).toHaveCount(0);
  });

  test('choosing something from the menu closes it', async ({ page }) => {
    // A menu left standing over the thing it just did has to be dismissed
    // before the user can see what happened.
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(show('Header', 'connected'));
    await openMenu(page);
    await page.getByRole('button', { name: 'Sync now' }).click();

    await expect(page.getByRole('button', { name: 'Settings…' })).toHaveCount(0);
  });

  test('the DEMO DATA badge appears when demo is true', async ({ page }) => {
    await page.goto(show('Header', 'demo'));
    await expect(page.locator('.demo')).toHaveText('DEMO DATA');
  });

  test('a connected demo account shows the badge but never offers Sync now', async ({ page }) => {
    // The real demo account is a seeded `accounts` row (connected), but was
    // never through OAuth — sync_now refuses it server-side, so the button
    // must not appear at all rather than invite a click that only errors.
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(show('Header', 'connected-demo'));
    await expect(page.locator('.demo')).toHaveText('DEMO DATA');
    expect(await lightName(page)).toBe('Synced 5 min ago');
    await openMenu(page);
    await expect(page.getByRole('button', { name: 'Sync now' })).toHaveCount(0);
  });

  test('busy disables the connect button while signing in', async ({ page }) => {
    await page.goto(show('Header', 'busy-disconnected'));
    const btn = page.getByRole('button', { name: 'Connecting…' });
    await expect(btn).toBeVisible();
    await expect(btn).toBeDisabled();
  });

  test('busy shows the light as syncing, and disables the sync button', async ({ page }) => {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(show('Header', 'busy-connected'));
    expect(await lightName(page)).toBe('Syncing now');
    await expect(page.locator('.light')).toHaveClass(/\bsyncing\b/);

    await openMenu(page);
    await expect(page.getByRole('button', { name: 'Sync now' })).toBeDisabled();
  });

  test('busy disables the add account button while syncing', async ({ page }) => {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(show('Header', 'busy-connected'));
    await openMenu(page);
    await expect(page.getByRole('button', { name: 'Add account' })).toBeDisabled();
  });

  test('a connected account can add another', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    await openMenu(page);
    await expect(page.getByRole('button', { name: 'Add account' })).toBeVisible();
  });

  test('a disconnected user is asked to connect, not to add', async ({ page }) => {
    await page.goto(show('Header', 'disconnected'));
    // Connect stays in the header rather than moving behind the hamburger: the
    // three controls §1 moves are all rare, and this is the one thing a new
    // user has to find.
    await expect(page.getByRole('button', { name: /Connect Google Calendar/ })).toBeVisible();
    await openMenu(page);
    await expect(page.getByRole('button', { name: 'Add account' })).toHaveCount(0);
  });

  test('a signed-out light is muted, and says so rather than reading as broken', async ({ page }) => {
    // A red dot on a fresh install would be the app telling a new user it is
    // broken. Nothing has gone wrong; there is nothing to sync.
    await page.goto(show('Header', 'disconnected'));
    expect(await lightName(page)).toBe('Not signed in');
    await expect(page.locator('.light')).toHaveClass(/\bnever\b/);
  });

  test('an error turns the light to failed, and the banner still says why', async ({ page }) => {
    // Both halves: a colour alone is not a status, so the words are on the dot
    // *and* the failure keeps its own readable banner.
    await page.goto(show('Header', 'error'));
    expect(await lightName(page)).toBe('Something went wrong: network unreachable');
    await expect(page.locator('.light')).toHaveClass(/\bfailed\b/);
    await expect(page.locator('.err')).toContainText('network unreachable');
  });

  test('demo mode offers neither', async ({ page }) => {
    await page.goto(show('Header', 'demo'));
    await openMenu(page);
    await expect(page.getByRole('button', { name: 'Add account' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: /Connect/ })).toHaveCount(0);
  });

  test('the hamburger opens the settings modal, and Escape closes it', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    await openMenu(page);
    await page.getByRole('button', { name: 'Settings…' }).click();

    const modal = page.getByRole('dialog', { name: 'Settings' });
    await expect(modal).toBeVisible();
    await expect(modal.getByRole('tab')).toHaveCount(4);

    await page.keyboard.press('Escape');
    await expect(modal).toHaveCount(0);
  });

  /** Opens the modal on `tab`. Every spec below needs the same three clicks. */
  const openSettings = async (page: import('@playwright/test').Page, tab?: string) => {
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    await expect(modal).toBeVisible();
    if (tab) await modal.getByRole('tab', { name: tab }).click();
    return modal;
  };

  /**
   * The fallback rows (fallback spec §3): shown in the units a person would
   * say — the shipped 60 reads "1 hours", not "60 minutes" — and every edit
   * saves through the command, because a settings row nobody wrote survives
   * nothing.
   */
  /** The weather knob (General): checked by default — the backend ships it
   *  on — and unchecking is a write through the command, not a redraw. */
  test('General carries the weather toggle, on by default and saved off', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page);

    const toggle = modal.getByRole('checkbox', { name: 'Weather in the day headers' });
    await expect(toggle).toBeChecked();
    await toggle.uncheck();
    await expect(toggle).not.toBeChecked();

    const calls = await page.evaluate(() => (window as any).__harness.calls);
    const call = calls.find((c: any) => c.cmd === 'set_weather_enabled');
    expect(call.args).toMatchObject({ on: false });
  });

  test('Notifications shows the fallback rows in speakable units', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page, 'Notifications');

    const amounts = modal.getByLabel('Fallback amount');
    const units = modal.getByLabel('Fallback unit');
    await expect(amounts).toHaveCount(2);
    await expect(amounts.nth(0)).toHaveValue('1');
    await expect(units.nth(0)).toHaveValue('hours');
    await expect(amounts.nth(1)).toHaveValue('10');
    await expect(units.nth(1)).toHaveValue('minutes');

    // The platform-widget rule is global now (fix/56): the event form was
    // fixed and this tab shipped the same light-on-light select the same
    // day. Asserted here on the second surface, so a rule quietly moved back
    // into one component's scope reddens.
    const appearance = await units
      .first()
      .evaluate((el) => getComputedStyle(el).appearance);
    expect(appearance, 'a select must never wear the platform widget style').toBe('none');
  });

  test('removing a fallback row is a write, not a redraw', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page, 'Notifications');

    await modal.getByLabel('Remove fallback reminder').first().click();
    await expect(modal.getByLabel('Fallback amount')).toHaveCount(1);
    const calls = await page.evaluate(
      () => (window as any).__harness.calls.filter((c: any) => c.cmd === 'set_fallback_reminders'),
    );
    expect(calls).toHaveLength(1);
    expect(calls[0].args.minutes).toEqual([10]);
  });

  test('adding a fallback row appends and saves it', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page, 'Notifications');

    await modal.getByRole('button', { name: '+ Add notification' }).click();
    await expect(modal.getByLabel('Fallback amount')).toHaveCount(3);
    const calls = await page.evaluate(
      () => (window as any).__harness.calls.filter((c: any) => c.cmd === 'set_fallback_reminders'),
    );
    expect(calls[calls.length - 1].args.minutes).toEqual([60, 10, 15]);
  });

  test('General offers the default calendar, writable ones only', async ({ page }) => {
    // The reader calendar must not be offered: choosing it would promise a
    // default that every create silently repairs away.
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page);
    const pick = modal.locator('#default-cal');
    // The unmade choice is named — 'your primary' alone is a fact the user
    // would have to go look up.
    await expect(pick.locator('option')).toHaveText(['Your primary — Personal', 'Personal', 'Team']);

    await pick.selectOption({ label: 'Team' });
    const calls = await page.evaluate(
      () => (window as any).__harness.calls.filter((c: any) => c.cmd === 'set_default_calendar'),
    );
    expect(calls).toHaveLength(1);
    expect(calls[0].args.id).toBe(2);
  });

  test('General shows the stored sync interval, in minutes', async ({ page }) => {
    // Until now this was settable only by running `sqlite3` against the
    // database by hand, and both platform guides documented it that way.
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page);
    await expect(modal.getByLabel('Sync every')).toHaveValue('5');
  });

  test('a longer interval is saved', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page);
    await modal.getByLabel('Sync every').fill('15');
    await modal.getByRole('button', { name: 'Save' }).click();

    // **Beside the button, not the modal-bottom note.** The shared note sits
    // below every tab's content in a modal that scrolls, so "Saved." landed
    // off-screen exactly when the tab was full — reported live (2026-08-17)
    // as "no visual clue it saved". The row itself is where the eye is.
    const row = modal.locator('.inline', { has: page.getByLabel('Sync every') });
    await expect(row.getByTestId('interval-note')).toHaveText('Saved.');

    // Typing again retires the stale confirmation: "Saved." next to a value
    // that has not been must not survive the first keystroke.
    await modal.getByLabel('Sync every').fill('16');
    await expect(row.getByTestId('interval-note')).toHaveCount(0);
    await modal.getByLabel('Sync every').fill('15');

    // **Reopened, not read off the box.** The box holds what was typed whether
    // or not anything was stored, so asserting it here would pass against a
    // Save that did nothing. Closing and reopening re-fetches, which is the
    // only thing that can say the value survived.
    await page.keyboard.press('Escape');
    const again = await openSettings(page);
    await expect(again.getByLabel('Sync every')).toHaveValue('15');
  });

  /**
   * **§3: the floor applies and the UI says so.** A value accepted and then
   * quietly clamped is worse than one refused — the user types ten seconds, the
   * form says nothing, and the app polls every minute while they believe
   * otherwise.
   */
  test('an interval below the floor is refused, with a reason', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page);
    await modal.getByLabel('Sync every').fill('0');
    await modal.getByRole('button', { name: 'Save' }).click();

    await expect(page.getByTestId('interval-note')).toContainText('will not sync more often');
    // And nothing was stored: reopening shows what was there before.
    await page.keyboard.press('Escape');
    const again = await openSettings(page);
    await expect(again.getByLabel('Sync every')).toHaveValue('5');
  });

  test('the time zone picker applies with an explicit restart', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page);

    // Defaults to the system zone; Apply is dead until something changes —
    // a restart button that is live with nothing to apply invites misclicks.
    const box = modal.getByLabel('Time zone', { exact: true });
    await expect(box).toHaveValue('');
    const apply = modal.getByRole('button', { name: 'Apply & restart' });
    await expect(apply).toBeDisabled();

    // Searching by the CAPITAL, lower-case, finds the zone — the native
    // select this replaced only jumped on leading letters, so "sofia"
    // found nothing (reported live, minutes after it shipped). A half-typed
    // name is not applyable; picking the match is.
    await box.fill('sofia');
    await expect(apply).toBeDisabled();
    await modal.getByRole('option', { name: 'Europe/Sofia' }).click();
    await expect(box).toHaveValue('Europe/Sofia');
    await expect(apply).toBeEnabled();
    await apply.click();

    // The zone went out under the command's own name, and the form says
    // what happens next — the real app's window is gone a breath later.
    await expect(page.getByTestId('tz-note')).toHaveText('Restarting…');
    const call = await page.evaluate(() =>
      (window as any).__harness.calls.find((c: { cmd: string }) => c.cmd === 'set_display_timezone')?.args);
    expect(call).toEqual({ tz: 'Europe/Sofia' });

    // Clearing back to System default sends null, not '' — the backend's
    // vocabulary.
    await page.keyboard.press('Escape');
    const again = await openSettings(page);
    await expect(again.getByLabel('Time zone', { exact: true })).toHaveValue('Europe/Sofia');
    await again.getByLabel('Time zone', { exact: true }).fill('');
    await again.getByRole('button', { name: 'Apply & restart' }).click();
    const second = await page.evaluate(() =>
      (window as any).__harness.calls.filter((c: { cmd: string }) => c.cmd === 'set_display_timezone').pop()?.args);
    expect(second).toEqual({ tz: null });
  });

  test('the second zone applies without a restart and clears to off', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page);

    // Same combo pattern as the display zone above it, same substring
    // search — and the same dead-until-valid Apply, though this one costs
    // no restart and says so by not being labelled with one.
    const box = modal.getByLabel('Second time zone');
    await expect(box).toHaveValue('');
    const apply = modal.getByRole('button', { name: 'Apply', exact: true });
    await expect(apply).toBeDisabled();

    await box.fill('kolkata');
    await expect(apply).toBeDisabled();
    await modal.getByRole('option', { name: 'Asia/Kolkata' }).click();
    await expect(apply).toBeEnabled();
    await apply.click();

    // Stored under the command's own name — and no restart note appears,
    // because none is coming.
    const call = await page.evaluate(() =>
      (window as any).__harness.calls.find((c: { cmd: string }) => c.cmd === 'set_second_timezone')?.args);
    expect(call).toEqual({ tz: 'Asia/Kolkata' });
    await expect(page.getByTestId('tz-note')).toHaveCount(0);

    // The stored choice survives a reopen, and clearing sends null — the
    // backend's vocabulary for off, exactly as the display zone clears.
    await page.keyboard.press('Escape');
    const again = await openSettings(page);
    await expect(again.getByLabel('Second time zone')).toHaveValue('Asia/Kolkata');
    await again.getByLabel('Second time zone').fill('');
    await again.getByRole('button', { name: 'Apply', exact: true }).click();
    const second = await page.evaluate(() =>
      (window as any).__harness.calls.filter((c: { cmd: string }) => c.cmd === 'set_second_timezone').pop()?.args);
    expect(second).toEqual({ tz: null });
  });

  test('the floor is stated before anybody trips over it', async ({ page }) => {
    // Said up front, not only on refusal. A rule you can only discover by
    // breaking it is a rule the form kept to itself.
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page);
    await expect(modal).toContainText('Not less than 1 minute');
  });

  /**
   * **The rows in the tab are the same rows as in the popover** — the same
   * component, not a second implementation. What makes that checkable is one
   * file up: `CalendarPopover`'s own thirteen specs pass unchanged, because
   * everything they assert is `CalendarList` now.
   */
  test('Calendars shows the calendar rows, grouped by account', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page, 'Calendars');

    await expect(modal.locator('.acct')).toHaveText(['me@x.com']);
    await expect(modal.locator('.row')).toHaveCount(3);
    await expect(modal.locator('.name')).toHaveText(['Personal', 'Team', 'Holidays in Bulgaria']);
  });

  /**
   * **The header names the zone the grid is drawn in.** Every time on screen
   * is rendered in the browser's zone, and nothing else said which one that
   * was. The config pins `timezoneId: 'UTC'` for every test, so the label's
   * text is known exactly rather than sniffed off the host.
   */
  test('the header names the zone the grid is drawn in', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    await expect(page.locator('.tz')).toHaveText('UTC');
  });

  /**
   * **The colour swatches form a column.** With `space-between` laying the row
   * out, each swatch's x trailed its calendar's name and four rows gave four
   * columns (seen on Omarchy, 2026-08-10). 'Personal' and 'Team' differ in
   * width, which is what makes two rows enough to catch it.
   */
  test('the colour swatches sit at one x, whatever the names', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page, 'Calendars');
    await expect(modal.locator('.swatch')).toHaveCount(3);
    const xs = await modal
      .locator('.swatch')
      .evaluateAll((els) => els.map((el) => el.getBoundingClientRect().left));
    expect(new Set(xs).size, 'every swatch shares the same left edge').toBe(1);
  });

  /**
   * §4's first invariant, asserted in the new host: **`selected` and
   * `sync_enabled` are separate switches.** Unticking hides a calendar and
   * keeps its events; Remove stops syncing it and deletes them. A tab that
   * collapsed the two would make "not today" delete a year of history.
   */
  test('the tab keeps show and sync as two separate controls', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page, 'Calendars');
    const row = modal.locator('.row').first();

    await expect(row.locator('input[type=checkbox]')).toBeVisible();
    await expect(row.getByRole('button', { name: 'Remove' })).toBeVisible();
    await expect(modal).toContainText('Unticking hides a calendar');
  });

  test('a calendar can be hidden from the tab, and the app is told', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page, 'Calendars');
    await modal.locator('input[type=checkbox]').first().uncheck();

    // Through to the command, not just the checkbox: the tab is a second host
    // for these rows and a host that rendered them without wiring them would
    // look identical.
    const calls = await page.evaluate(
      () => (window as any).__harness.calls.filter((c: any) => c.cmd === 'set_calendar_selected'),
    );
    expect(calls).toHaveLength(1);
    expect(calls[0].args.on, 'unticking asks for selected: false').toBe(false);
  });

  test('changing a calendar from the tab tells the app to reload', async ({ page }) => {
    // A different fact from the command having been sent, and the one a second
    // host for these rows can silently drop: the rows call Google themselves,
    // but only the host can ask `App` to reload the grid afterwards.
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page, 'Calendars');
    await modal.locator('input[type=checkbox]').first().uncheck();

    await expect
      .poll(() => page.evaluate(() => (window as any).__calendarChanges))
      .toBeGreaterThan(0);
  });

  test('Notifications turns reminders off and on', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page, 'Notifications');
    const box = modal.getByLabel('Show reminders');
    await expect(box, 'reminders are on until turned off').toBeChecked();

    await box.uncheck();
    await page.keyboard.press('Escape');
    const again = await openSettings(page, 'Notifications');
    await expect(again.getByLabel('Show reminders')).not.toBeChecked();
  });

  test('Notifications says when the fallback speaks, and when it never does', async ({ page }) => {
    // The tab used to promise "no policy of omacal's own"; the fallback is
    // exactly such a policy, adopted deliberately (fallback spec §1), so the
    // promise changed to naming its bounds instead: only where Google is
    // silent, and clearable.
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page, 'Notifications');
    await expect(modal).toContainText('no reminders of its own and its calendar offers no');
    await expect(modal).toContainText('clear the list to turn this off');
  });

  test('Accounts lists the connected account and offers to add another', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page, 'Accounts');
    await expect(modal.getByRole('listitem')).toContainText(['me@x.com']);
    await expect(modal.getByRole('button', { name: 'Add account' })).toBeVisible();
  });

  test('signing an account out asks first, then empties the list', async ({ page }) => {
    // Two clicks by design: the first arms, the second is worded as the
    // destructive thing it is. The harness's sign_out answers with no
    // accounts left, and the tab must say so rather than show stale rows.
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page, 'Accounts');
    await modal.getByRole('button', { name: 'Sign out…' }).click();
    await expect(modal.getByRole('button', { name: 'Keep' })).toBeVisible();
    await modal.getByRole('button', { name: 'Really sign out' }).click();
    await expect(modal).toContainText('No account is connected');
    await expect(modal).toContainText('signed out');
  });

  test('what is not built yet says so rather than being absent', async ({ page }) => {
    // A control that is missing reads as "never"; a line saying it is not built
    // reads as "not yet", which is the true one. Sign-out graduated out of
    // this test in v0.2.x; what remains here is the honesty rule itself.
    await page.goto(show('Header', 'connected'));
    const modal = await openSettings(page, 'Accounts');
    await expect(modal).toContainText('Signing out removes the account');

    // Switching tabs inside the open modal, not reopening it: the scrim covers
    // the hamburger, so a second `openSettings` would be clicking a button
    // nothing can reach.
    await modal.getByRole('tab', { name: 'Notifications' }).click();
    await expect(modal).toContainText('tray and start-on-login switches are not built');
  });

  test('the settings modal does not close on a click inside it', async ({ page }) => {
    // Spec §5. A modal that dismisses when you click its own tabs is a modal
    // nobody can use.
    await page.goto(show('Header', 'connected'));
    await openMenu(page);
    await page.getByRole('button', { name: 'Settings…' }).click();

    const modal = page.getByRole('dialog', { name: 'Settings' });
    await modal.getByRole('tab', { name: 'Calendars' }).click();
    await expect(modal).toBeVisible();
    await expect(modal.getByRole('tab', { name: 'Calendars' }))
      .toHaveAttribute('aria-selected', 'true');
  });

  /**
   * The error banner and the DEMO DATA badge read their colours from
   * `--error` and `--demo`, and not from anywhere else.
   *
   * Overriding the variable at runtime, rather than comparing the rendered
   * colour against a known hex. The comparison is what a first pass at this
   * did, and it is worthless here: `#e2564a` written literally in the
   * stylesheet produces exactly the same `rgb(226, 86, 74)` as
   * `var(--error)` resolving to it, so the assertion passes just as happily
   * against the code this replaced. Only moving the variable and watching the
   * element follow can tell the two apart.
   */
  test('the error banner and the demo badge follow their own variables', async ({ page }) => {
    await page.goto(show('Header', 'error-and-demo'));
    const err = page.locator('.err');
    const demo = page.locator('.demo');
    await expect(err).toHaveText('Sync failed.');
    await expect(demo).toHaveText('DEMO DATA');

    const colour = (l: typeof err, prop: string) =>
      l.evaluate((el, p) => getComputedStyle(el).getPropertyValue(p), prop);
    const setVar = (name: string, value: string) =>
      page.evaluate(([n, v]) => document.documentElement.style.setProperty(n, v), [name, value]);

    // The value, before the wiring. Neither of these elements appears in any
    // screenshot golden — every `header-*` fixture has `error: null`, so `.err`
    // is never rendered into one — which means nothing else in the suite would
    // notice `setPalette` publishing the wrong colour. The override below
    // cannot: it proves where the colour comes from, and passes whatever the
    // resolved value is.
    expect(await colour(err, 'color'), 'the error colour changed value').toBe('rgb(226, 86, 74)');
    expect(await colour(demo, 'color'), 'the demo badge colour changed value')
      .toBe('rgb(226, 160, 63)');

    const demoBefore = await colour(demo, 'color');
    const tintBefore = await colour(err, 'background-color');

    await setVar('--error', 'rgb(0, 255, 0)');
    expect(await colour(err, 'color'), 'the banner does not read --error').toBe('rgb(0, 255, 0)');
    // The 9% wash is `color-mix`ed from the same variable rather than carried
    // as a second one, so it has to move too — and it is the half a colour
    // assertion on the text alone would miss.
    expect(await colour(err, 'background-color'), 'the banner tint does not follow --error')
      .not.toBe(tintBefore);
    expect(await colour(demo, 'color'), '--error reached the DEMO DATA badge').toBe(demoBefore);

    await setVar('--demo', 'rgb(0, 0, 255)');
    expect(await colour(demo, 'color'), 'the badge does not read --demo').toBe('rgb(0, 0, 255)');
  });

  // --- The filmstrip toggle ---------------------------------------------
  //
  // Filmstrip spec §1: it sits *beside* the view switcher and is orthogonal to
  // it — a rendering of the period, not a sixth period to be in.

  test('the toggle sits beside the switcher and reports which mode is on', async ({ page }) => {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(show('Header', 'connected'));

    const toggle = page.getByRole('button', { name: 'List view' });
    await expect(toggle).toBeVisible();
    // A toggle, not a link to a sixth view: the name stays put and the state is
    // the state, so a screen reader is never told the control has become a
    // different control.
    await expect(toggle).toHaveAttribute('aria-pressed', 'false');
    await expect(toggle).toHaveText('▦');
    await expect(toggle).toHaveAttribute('title', 'List view (F)');
    // Beside the switcher, not inside it — five slots, still five.
    await expect(page.locator('.vswitch button')).toHaveCount(5);
    await expect(page.locator('.vswitch .filmstrip')).toHaveCount(0);
  });

  test('with list mode on it says so, and shows the other glyph', async ({ page }) => {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(show('Header', 'list-on'));
    const toggle = page.getByRole('button', { name: 'List view' });
    await expect(toggle).toHaveAttribute('aria-pressed', 'true');
    await expect(toggle).toHaveText('☰');
  });

  test('clicking it asks the parent rather than flipping anything itself', async ({ page }) => {
    // The value is a stored preference; `App` owns every write in this app, and
    // this header only reports what it was given. So the glyph must **not**
    // change on its own — a header that flipped its own copy would show a list
    // that was never turned on if the write failed.
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(show('Header', 'connected'));
    const toggle = page.getByRole('button', { name: 'List view' });
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-pressed', 'false');
    await expect(toggle).toHaveText('▦');
  });

  // Filmstrip spec §2: **absent**, not present and inert. Both views, because a
  // whole-feature rule has as many places to be false as it has code paths —
  // §8 of the testing standard — and one `{#if}` covering both is not a reason
  // to test one and assume the other.
  //
  // The honest probe here **adds** rather than deletes (testing standard §3):
  // the rule is enforced by markup that is not there, so the mutation is
  // `'year'` and `'bigyear'` appended to `LISTABLE_VIEWS` in `filmstrip.ts`,
  // which makes the button appear and reddens both of these.
  for (const view of ['year', 'bigyear'] as const) {
    test(`the toggle is absent in ${view} view, not disabled`, async ({ page }) => {
      await page.clock.setFixedTime(FIXED_NOW);
      await page.goto(show('Header', view));
      // The switcher is still there, so this is a header that rendered — an
      // empty page would satisfy the absence on its own.
      await expect(page.locator('.vswitch button')).toHaveCount(5);
      await expect(page.getByRole('button', { name: 'List view' })).toHaveCount(0);
      await expect(page.locator('.filmstrip')).toHaveCount(0);
    });
  }
});

test.describe('CalendarPopover', () => {
  const show = (f: string) => `/tests/harness/index.html?c=CalendarPopover&f=${f}`;

  test('opens and groups by account', async ({ page }) => {
    await page.goto(show('two-accounts'));
    await page.getByRole('button', { name: /Calendars/ }).click();
    await expect(page.locator('.acct')).toHaveCount(2);
  });

  // Task 7: a parent (`App`, right after sign-in) can drive the panel open
  // through the bindable `open` prop, without going through the trigger.
  test('a parent can open the picker', async ({ page }) => {
    await page.goto(show('open-on-mount'));
    await expect(page.locator('.panel')).toBeVisible();
  });

  test('it still starts closed by default', async ({ page }) => {
    await page.goto(show('two-accounts'));
    await expect(page.locator('.panel')).toHaveCount(0);
  });

  /**
   * **The `each_key_duplicate` hazard §4 names**, and the reason the rows are
   * keyed on `c.id` rather than on the summary.
   *
   * **Two calendars with the same name in one account**, and the account is
   * the part that took a mutation to get right: the key is on the *inner*
   * loop, which iterates within a single group, so the same name under two
   * different accounts is two `{#each}` instances and never collides. The
   * hazard as previously recorded named that shape, and it is not the one that
   * throws.
   *
   * Google lets you create two calendars with the same name, so one account
   * holding two is reachable — and Svelte throws on the duplicate key rather
   * than rendering one row where two belong, taking the whole panel with it.
   */
  test('two calendars with the same name in one account both render', async ({ page }) => {
    await page.goto(show('same-summary-one-account'));
    await page.getByRole('button', { name: /Calendars/ }).click();

    await expect(page.locator('.acct')).toHaveText(['me@x.com']);
    await expect(page.locator('.name')).toHaveText(['UK Holidays', 'UK Holidays']);
  });

  // --- Per-calendar colour ------------------------------------------------

  const openColours = async (page: import('@playwright/test').Page, name: string) => {
    await page.getByRole('button', { name: /Calendars/ }).click();
    await page.getByRole('button', { name: `Colour for ${name}` }).click();
  };

  test('a calendar offers the curated set, and no free picker', async ({ page }) => {
    // Curated because omacal draws on both a light and a dark Omarchy theme,
    // and a colour chosen against one can be unreadable on the other.
    await page.goto(show('colours'));
    await openColours(page, 'Work');

    await expect(page.locator('.swatches .pick')).toHaveCount(10);
    await expect(page.locator('input[type=color]'), 'no free picker').toHaveCount(0);
    // Named, not just coloured: a swatch a screen reader calls "button" is one
    // nobody can choose deliberately.
    await expect(page.getByRole('button', { name: 'Amber' })).toBeVisible();
  });

  test('choosing a colour asks for it, locally', async ({ page }) => {
    await page.goto(show('colours'));
    await openColours(page, 'Work');
    await page.getByRole('button', { name: 'Amber' }).click();

    const calls = await page.evaluate(() => window.__harness.calls);
    const set = calls.filter((c) => c.cmd === 'set_calendar_color');
    expect(set).toHaveLength(1);
    expect(set[0].args).toEqual({ id: 1, hex: '#e2a03f' });
    // **Nothing reaches Google.** The whole design is a column beside Google's
    // colour rather than a `calendarList.patch`, so an event write here would
    // be the defect rather than a detail.
    expect(calls.filter((c) => c.cmd === 'update_event')).toHaveLength(0);
  });

  test('the chosen swatch is marked, and only on the calendar that chose it', async ({ page }) => {
    await page.goto(show('colours'));
    await openColours(page, 'Personal');
    await expect(page.getByRole('button', { name: 'Amber' })).toHaveAttribute('aria-pressed', 'true');
    await expect(page.getByRole('button', { name: 'Blue' })).toHaveAttribute('aria-pressed', 'false');
  });

  /**
   * **Clearing is its own action, not a swatch** — and the offer is only live
   * where there is something to clear. Choosing the colour Google happens to
   * use today would look identical on screen and stop following it the moment
   * Google changed.
   */
  test('clearing an override is offered only where there is one', async ({ page }) => {
    await page.goto(show('colours'));
    await openColours(page, 'Work');
    await expect(page.getByRole('button', { name: /Use Google/ })).toBeDisabled();

    await page.keyboard.press('Escape');
    await openColours(page, 'Personal');
    await expect(page.getByRole('button', { name: /Use Google/ })).toBeEnabled();
  });

  test('clearing asks for null, not for a colour', async ({ page }) => {
    // The half a stored-copy implementation gets wrong: it would send the hex
    // Google currently uses, which reads identically until Google changes it.
    await page.goto(show('colours'));
    await openColours(page, 'Personal');
    await page.getByRole('button', { name: /Use Google/ }).click();

    const [call] = await page.evaluate(() =>
      window.__harness.calls.filter((c) => c.cmd === 'set_calendar_color'),
    );
    expect(call.args).toEqual({ id: 2, hex: null });
  });

  test('the row dot shows the colour the calendar is actually drawn in', async ({ page }) => {
    // The override, where there is one — the same `COALESCE` the grid reads,
    // so the row and the events can never disagree.
    await page.goto(show('colours'));
    await page.getByRole('button', { name: /Calendars/ }).click();
    // The computed fill, not the inline attribute: Svelte may normalise the
    // style string, and what matters is what is painted.
    const fill = await page.locator('.row .dot').nth(1).evaluate(
      (el) => getComputedStyle(el).backgroundColor,
    );
    expect(fill).toBe('rgb(226, 160, 63)'); // #e2a03f
  });

  test('counts only calendars that are both synced and shown', async ({ page }) => {
    await page.goto(show('mixed'));
    // 3 calendars: one hidden, one removed, one visible.
    await expect(page.locator('.trigger .count')).toHaveText('1');
  });

  test('a removed calendar cannot be ticked', async ({ page }) => {
    await page.goto(show('mixed'));
    await page.getByRole('button', { name: /Calendars/ }).click();
    const off = page.locator('.row.off').first();
    await expect(off.locator('input[type=checkbox]')).toBeDisabled();
    await expect(off.locator('.sync')).toHaveText('Add');
  });

  test('clicking away closes it', async ({ page }) => {
    await page.goto(show('two-accounts'));
    await page.getByRole('button', { name: /Calendars/ }).click();
    await expect(page.locator('.panel')).toBeVisible();
    await page.locator('.scrim').click();
    await expect(page.locator('.panel')).toHaveCount(0);
  });

  test('Escape closes it', async ({ page }) => {
    await page.goto(show('two-accounts'));
    await page.getByRole('button', { name: /Calendars/ }).click();
    await expect(page.locator('.panel')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator('.panel')).toHaveCount(0);
  });

  // Fix round 1, finding 1a: Tab once from the trigger and focus lands on
  // `.scrim` — a sibling of `.panel`, not a descendant of either element the
  // old per-element keydown handlers were attached to. Only a window-level
  // listener hears Escape from there.
  test('Escape closes it when focus is on the scrim', async ({ page }) => {
    await page.goto(show('two-accounts'));
    await page.getByRole('button', { name: /Calendars/ }).click();
    await page.locator('.scrim').focus();
    await page.keyboard.press('Escape');
    await expect(page.locator('.panel')).toHaveCount(0);
  });

  // Fix round 1, finding 1b: disabling a focused checkbox mid-toggle drops
  // focus to <body> (browser default, both engines) — nowhere a listener on
  // the trigger or the panel could ever hear from. Holds the call open so the
  // checkbox is still disabled, hence still stuck on <body>, when Escape
  // is pressed.
  test('Escape closes it once a toggle has moved focus to <body>', async ({ page }) => {
    await page.goto(show('single'));
    await page.evaluate(() => window.__harness.holdNextCalendarCall('set_calendar_selected'));
    await page.getByRole('button', { name: /Calendars/ }).click();

    const box = page.locator('input[type=checkbox]');
    await box.focus();
    await box.click(); // parked — the checkbox disables and focus falls to <body>
    await expect(box).toBeDisabled();

    await page.keyboard.press('Escape');
    await expect(page.locator('.panel')).toHaveCount(0);
  });

  // Resolution 4: the browser flips a checkbox's `checked` property on click,
  // before any handler runs. If `set_calendar_selected` then fails, the box
  // is left showing a state the store never actually reached until the
  // component explicitly snaps it back.
  test('a failed toggle snaps the checkbox back and reports the error', async ({ page }) => {
    await page.goto(show('single'));
    await page.evaluate(() =>
      window.__harness.failNextCalendarCall('set_calendar_selected', 'database is locked'),
    );
    await page.getByRole('button', { name: /Calendars/ }).click();

    const box = page.locator('input[type=checkbox]');
    await expect(box).toBeChecked(); // fixture calendar starts selected
    await box.click();

    // Fix round 1, finding 3: attributed to the calendar it's about — "Work"
    // is the `single` fixture's one calendar — so a note can't be misread as
    // belonging to some other row that settled around the same time.
    await expect(page.locator('.note.err')).toHaveText('Work · database is locked');
    // The click already flipped it once; a naive implementation stops here.
    await expect(box).toBeChecked();
  });

  // Fix round 1 (Task 7), finding 1: the `message = null` reset moved from
  // `toggle()` into an `$effect` keyed on `open`, so a parent-driven open
  // clears a stale note too. Nothing exercised that effect at all — a mutant
  // that drops the `open` read (runs the reset once, at mount, and never
  // again) left every existing test green. `message` is component state, not
  // DOM: it survives the panel unmounting on close, so a stale error from
  // before Escape must not still be showing after the panel reopens.
  test('reopening the panel clears a stale error message', async ({ page }) => {
    await page.goto(show('single'));
    await page.evaluate(() =>
      window.__harness.failNextCalendarCall('set_calendar_selected', 'database is locked'),
    );
    await page.getByRole('button', { name: /Calendars/ }).click();
    await page.locator('input[type=checkbox]').click();
    await expect(page.locator('.note.err')).toHaveText('Work · database is locked');

    await page.keyboard.press('Escape');
    await expect(page.locator('.panel')).toHaveCount(0);

    await page.getByRole('button', { name: /Calendars/ }).click();
    await expect(page.locator('.note')).toHaveCount(0);
  });

  // Fix round 1, finding 2: `busy` used to be a single id, so toggling a
  // second row while the first was still in flight pointed `busy` at the
  // second id and silently re-enabled the first — a real double-submit.
  // Holds the first row's call open and toggles the second immediately after,
  // so the two are genuinely concurrent rather than sequential.
  test('a row stays disabled until its own call resolves, even if another row toggles meanwhile', async ({ page }) => {
    await page.goto(show('mixed'));
    await page.evaluate(() => window.__harness.holdNextCalendarCall('set_calendar_selected'));
    await page.getByRole('button', { name: /Calendars/ }).click();

    // `mixed` has 3 calendars, one of them `sync_enabled: false` (`.row.off`,
    // permanently disabled); the other two are the rows this test toggles.
    const rows = page.locator('.row:not(.off) input[type=checkbox]');
    await rows.nth(0).click(); // consumes the hold — parked
    await rows.nth(1).click(); // no hold armed for it — resolves right away

    await expect(rows.nth(0)).toBeDisabled();
    await expect(rows.nth(1)).toBeEnabled();

    await page.evaluate(() => window.__harness.releaseCalendarCall('set_calendar_selected', undefined));
    await expect(rows.nth(0)).toBeEnabled();
  });

  // Resolution 1: `setCalendarSync` resolves with the number of events the
  // removal deleted specifically so the UI can report it — throwing that
  // count away would make the removal look like it did nothing. Fix round 1,
  // finding 3: also names the calendar, for the same reason as the failed-
  // toggle note above.
  test('removing a calendar reports how many events were deleted, naming the calendar', async ({ page }) => {
    await page.goto(show('single'));
    await page.getByRole('button', { name: /Calendars/ }).click();
    await page.getByRole('button', { name: 'Remove' }).click();
    await expect(page.locator('.note')).toHaveText(`Work · ${CALENDAR_SYNC_REMOVED} events deleted`);
  });
});

test.describe('EventPopover', () => {
  const show = (f: string) => `/tests/harness/index.html?c=EventPopover&f=${f}`;

  test('the panel never scrolls sideways, whatever a field holds', async ({ page }) => {
    // Reported from Plamen's calendar: a full-width horizontal scrollbar along
    // the bottom of the popover. It came from an organizer address —
    // `c_<40 hex>@group.calendar.google.com` — but the organizer field is not
    // what this asserts, on purpose. Suppressing that one address fixes the
    // case that was reported and leaves the panel just as fragile: a title, a
    // location, a conference URI or an attendee address can each be one long
    // unbroken token. This is the spec for the panel, and it is the one that
    // has to hold when the next such field is added.
    //
    // `.pop` is 320px wide with `overflow-y: auto`, which makes the other axis
    // `auto` as well rather than `visible` — so overflowing content does not
    // spill, it grows a scroller. `scrollWidth > clientWidth` is exactly that
    // scroller, and is the thing the user saw.
    await page.goto(show('unbreakable'));
    const pop = page.locator('.pop');
    await expect(pop).toBeVisible();

    // The fixture's own premise: the token really is far longer than the panel
    // can lay out on one line. Asserted rather than assumed, so shortening
    // `UNBREAKABLE` cannot quietly leave this test measuring nothing.
    expect(UNBREAKABLE.length).toBeGreaterThan(150);
    const width = (await pop.boundingBox())!.width;
    expect(width).toBeLessThan(400);

    // The three fields that actually drive the panel's overflow, measured with
    // the guard removed: the title (1994px on its own), the location (1589px)
    // and the organizer (1658px). More than one, deliberately — a fix applied
    // to whichever selector was reported would pass a single-field version of
    // this test and still ship the bug.
    //
    // `.desc`, `.conf` and `.who` are *not* on this list and it is worth
    // saying why, so nobody adds them thinking the list was an oversight:
    // `.desc` carries its own `word-break: break-word`, `.conf`'s text is the
    // fixed label "Join video call" (its long URI never leaves the `href`),
    // and `.who` clips itself with an ellipsis so its own overflow never
    // reaches the panel.
    for (const sel of ['h2', '.loc', '.organizer']) {
      await expect(pop.locator(sel), sel).toBeVisible();
      await expect(pop.locator(sel), sel).toContainText(UNBREAKABLE);
    }

    const scroll = await pop.evaluate((el) => ({
      scrollWidth: el.scrollWidth, clientWidth: el.clientWidth,
    }));
    expect(scroll.scrollWidth).toBeLessThanOrEqual(scroll.clientWidth);
  });

  /**
   * The Join control, from either of the two places a meeting can hide.
   *
   * Google's structured conference data has always driven this button, which
   * meant an invitation minted by an Outlook or Zoom shop — where the link
   * lives in `location` and nowhere else — showed the provider's name with
   * nothing to click. These three cover the rule: the link is offered, an
   * unrecognised link is not, and Google's own field still wins.
   */
  test('a meeting link in the location becomes a Join button', async ({ page }) => {
    await page.goto(show('location-holds-a-real-zoom-link'));
    const conf = page.locator('.pop .conf');
    await expect(conf).toBeVisible();
    await expect(conf).toHaveText('Join video call');
    await expect(conf).toHaveAttribute('href', 'https://us02web.zoom.us/j/123456?pwd=x');
    // And the raw link is not *also* printed underneath its own button.
    await expect(page.locator('.pop .loc')).toHaveCount(0);
  });

  test('a map link in the location is not offered as a meeting', async ({ page }) => {
    await page.goto(show('location-holds-a-map-link'));
    // The popover rendered — otherwise the absence below proves nothing.
    await expect(page.locator('.pop')).toBeVisible();
    await expect(page.locator('.pop .conf')).toHaveCount(0);
    // It is still shown as a place; it just is not a meeting.
    await expect(page.locator('.pop .loc')).toBeVisible();
  });

  test('a generated calendar address is not shown as the organizer', async ({ page }) => {
    // "Organized by" followed by forty hex characters names no one. `.pop` is
    // asserted visible first so this cannot pass by the popover having failed
    // to render at all, which is the way a hidden-row assertion usually lies.
    await page.goto(show('machine-organizer'));
    await expect(page.locator('.pop')).toBeVisible();
    await expect(page.locator('.organizer')).toHaveCount(0);
  });

  test('a real organizer address is still shown', async ({ page }) => {
    // The other half of the pair, and not optional: without it the assertion
    // above is satisfied by a component that stopped rendering the row at all.
    await page.goto(show('human-organizer'));
    await expect(page.locator('.organizer')).toHaveText('Organized by plamen@excitel.com');
  });

  test('shows the guest list with each response', async ({ page }) => {
    await page.goto(show('standup'));
    await expect(page.locator('.guest')).toHaveCount(3);
    await expect(page.locator('.guest.accepted')).toHaveCount(1);
    await expect(page.locator('.guest.declined')).toHaveCount(1);
  });

  test('each guest carries a status glyph, and a different one per status', async ({ page }) => {
    // The whole point of the glyph is that "coming" and "hasn't replied" are
    // told apart at a glance. Asserting each mark's own character is what
    // catches a table that has gone uniform — a count of `.mark` would pass
    // even if every guest showed the same symbol.
    await page.goto(show('standup'));
    await expect(page.locator('.guest.accepted .mark')).toHaveText('✓');
    await expect(page.locator('.guest.declined .mark')).toHaveText('✕');
    // `?` means maybe, the letter Google and Outlook use for it; an
    // unanswered guest is the empty ring — nothing there, honestly. The ring
    // itself still renders (the count above includes this row), so empty text
    // is a statement, not a missing element.
    await expect(page.locator('.guest.needsAction .mark')).toHaveText('');
  });

  test('the status is said on hover, not only drawn', async ({ page }) => {
    // The glyphs are one keystroke tall and the tilde in particular was read
    // as "no idea what that is" (Omarchy, 2026-08-10). `.sr` serves screen
    // readers; `title` is the sighted reader's version of the same word.
    await page.goto(show('standup'));
    await expect(page.locator('.guest.accepted')).toHaveAttribute('title', 'accepted');
    await expect(page.locator('.guest.needsAction')).toHaveAttribute('title', 'no reply yet');
  });

  test('the status is announced, not only drawn', async ({ page }) => {
    // The ring is aria-hidden, so without the visually-hidden word a screen
    // reader would hear a name and nothing about whether they are coming.
    await page.goto(show('standup'));
    await expect(page.locator('.guest.accepted .sr')).toHaveText('accepted');
    await expect(page.locator('.guest.needsAction .sr')).toHaveText('no reply yet');
  });

  test('the panel claims to be modal and takes focus on open', async ({ page }) => {
    // The scrim already makes the grid behind unclickable. Without
    // `aria-modal` and the focus move, the tab order still begins wherever
    // the click left it — outside the panel, walking through a week of blocks
    // a mouse can no longer reach.
    //
    // Deliberately asserts where focus *starts*, not where Tab goes next:
    // WebKit only tabs to buttons and links when Full Keyboard Access is on,
    // so a Tab assertion here would be testing a browser preference. Focus
    // containment proper (wrapping Tab at the last control) is not
    // implemented — starting inside is what this covers.
    await page.goto(show('standup'));
    await expect(page.locator('.pop')).toHaveAttribute('aria-modal', 'true');
    await expect(page.locator('.pop')).toBeFocused();
  });

  test('a description containing markup is shown as text', async ({ page }) => {
    await page.goto(show('nasty-description'));
    await expect(page.locator('.desc')).toContainText('<script>alert(1)</script>');
    await expect(page.locator('.desc script')).toHaveCount(0);
  });

  test('a one-off event offers no scope choice', async ({ page }) => {
    await page.goto(show('standup'));
    await expect(page.locator('.rsvp')).toBeVisible();
    await expect(page.locator('.scope')).toHaveCount(0);
  });

  test('a location that only repeats the description’s URL is said once', async ({ page }) => {
    await page.goto(show('location-echoes-description'));
    await expect(page.locator('.desc a')).toHaveText('https://zoom.example/j/123?pwd=abc');
    await expect(page.locator('.loc')).toHaveCount(0);
  });

  test('a URL the description lacks still shows, and so does a room', async ({ page }) => {
    // The two controls, or the suppression above is satisfiable by never
    // rendering a location at all.
    await page.goto(show('location-url-not-in-description'));
    await expect(page.locator('.loc')).toHaveText('https://zoom.example/j/123?pwd=abc');

    await page.goto(show('location-room-in-description'));
    await expect(page.locator('.loc')).toHaveText('Room 4A');
  });

  test('a recurring event says its cadence, and asks nothing at rest', async ({ page }) => {
    // The scope question is only relevant once the user touches something
    // (2026-08-11, by request); a reader gets the fact that matters instead.
    await page.goto(show('recurring'));
    await expect(page.locator('.cadence')).toHaveText('Repeats daily');
    await expect(page.locator('.scope')).toHaveCount(0);
  });

  test('the scope is asked when a response is chosen, and Cancel withdraws it', async ({ page }) => {
    await page.goto(show('recurring'));
    await page.getByRole('button', { name: 'No' }).click();
    await expect(page.locator('.scope')).toBeVisible();
    // Nothing sent yet: the question is open, not answered by default.
    expect(await page.evaluate(() => (window as any).__lastRespondCall ?? null)).toBeNull();
    await page.getByRole('button', { name: 'Cancel' }).click();
    await expect(page.locator('.scope')).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).__lastRespondCall ?? null)).toBeNull();
  });

  test('a read-only calendar offers no rsvp at all', async ({ page }) => {
    await page.goto(show('readonly'));
    await expect(page.locator('.guest')).toHaveCount(3);
    await expect(page.locator('.rsvp')).toHaveCount(0);
  });

  test('a failed response rolls the choice back and says why', async ({ page }) => {
    await page.goto(show('respond-fails'));
    await page.getByRole('button', { name: 'No' }).click();
    await expect(page.locator('.note.err')).toBeVisible();
    await expect(page.getByRole('button', { name: 'No' })).not.toHaveClass(/chosen/);
  });

  test('responding to a later occurrence sends that occurrence, not the series start', async ({ page }) => {
    // The trap named in the Interfaces block: `detail.start_ms` is the series
    // DTSTART for a master row, and passing it silently patches occurrence #0
    // for everyone. Assert the fourth argument is the clicked block's own start.
    await page.goto(show('recurring-fourth-occurrence'));
    await page.getByRole('button', { name: 'No' }).click();
    await page.getByRole('button', { name: 'This one' }).click();
    const call = await page.evaluate(() => (window as any).__lastRespondCall);
    expect(call.occurrenceStartMs).toBe(1786600800000); // Thu 13 Aug, the clicked block
    expect(call.occurrenceStartMs).not.toBe(1786341600000); // Mon 10 Aug, the series start
    expect(call.scope).toBe('this');
  });

  test('choosing "All of them" sends that scope, not the default', async ({ page }) => {
    await page.goto(show('recurring-fourth-occurrence'));
    await page.getByRole('button', { name: 'No' }).click();
    await page.getByRole('button', { name: 'All of them' }).click();
    const call = await page.evaluate(() => (window as any).__lastRespondCall);
    expect(call.scope).toBe('all');
  });

  test('a successful response shows immediately, without waiting for a sync', async ({ page }) => {
    // The backend deliberately returns the master's unchanged detail after a
    // "this one" RSVP, so nothing moves on screen unless the UI reflects the
    // choice itself. Five minutes of a dead button reads as a failure.
    await page.goto(show('recurring-fourth-occurrence'));
    await page.getByRole('button', { name: 'No' }).click();
    await page.getByRole('button', { name: 'This one' }).click();
    await expect(page.getByRole('button', { name: 'No' })).toHaveClass(/chosen/);
  });

  test('a successful non-recurring response also updates the guest list, not just the buttons', async ({ page }) => {
    // Unlike the bare-master "this one" case above, the backend really does
    // write back here (every non-recurring event, and `scope: 'all'`) — the
    // guest list's own "you" row must catch up too, or the buttons would say
    // "No" while the row right below them still reads needsAction.
    await page.goto(show('writes-back'));
    await page.getByRole('button', { name: 'No' }).click();
    await expect(page.locator('.guest')).toHaveClass(/declined/);
  });

  test('escape closes it even when focus has fallen to the body', async ({ page }) => {
    // Plan 1c shipped this bug once: a keydown handler on the panel misses
    // Escape entirely once a disabled control drops focus to <body>, and a
    // test that only presses Escape with the trigger focused cannot see it.
    await page.goto(show('standup'));
    await expect(page.locator('.pop')).toBeVisible();
    await page.evaluate(() => (document.activeElement as HTMLElement)?.blur());
    expect(await page.evaluate(() => document.activeElement?.tagName)).toBe('BODY');
    await page.keyboard.press('Escape');
    await expect(page.locator('.pop')).toHaveCount(0);
  });

  // Task 10. A pair, deliberately: `detail()`'s own default is
  // `can_edit: false`, so the "shown" half fails until its fixture opts in
  // explicitly, and the "hidden" half is the one that could pass vacuously —
  // which is the safe way round, since an absent control ships nothing to
  // somebody who may not use it. Together they discriminate both ways.
  test('an event the user can write to offers Edit and Delete', async ({ page }) => {
    await page.goto(show('editable'));
    await expect(page.getByRole('button', { name: 'Edit' })).toHaveCount(1);
    await expect(page.getByRole('button', { name: 'Delete' })).toHaveCount(1);
  });

  test('an event the user cannot write to offers neither', async ({ page }) => {
    // Offering either on a calendar this account only reads would produce a
    // Save — or a Delete confirmation with no undo behind it — that
    // `update_impl`'s own writability check could only refuse, after the user
    // had already decided to go through with it.
    await page.goto(show('standup'));
    await expect(page.locator('.pop')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Edit' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Delete' })).toHaveCount(0);
  });

  test('clicking a guest list does not close it', async ({ page }) => {
    // The scrim must sit behind the panel, not over it.
    await page.goto(show('standup'));
    await page.locator('.guest').first().click();
    await expect(page.locator('.pop')).toBeVisible();
  });

  // Task 6's sweep. The `when` line rendered `detail.start_ms` through
  // `toLocaleDateString`/`toLocaleTimeString` in the browser's zone, and no
  // spec asserted on it at all — five plans, and the two fixtures below are
  // the first that could ever have caught it, because every other one here
  // has `occurrenceStartMs === detail.start_ms`.
  //
  // For a series `detail.start_ms` is the **master's** DTSTART. The block on
  // the grid beside this panel is drawn from the occurrence's own `start_ms`,
  // so the two disagreeing puts a popover on screen contradicting the thing it
  // was opened from.
  test('a later occurrence shows its own day and clock, not the master’s', async ({ page }) => {
    await page.goto(show('recurring-across-a-fall-back'));

    // The fixture's own premise, asserted rather than described: seven days
    // and **one hour** apart, because Sofia's clocks go back between them. A
    // fixture that stopped straddling would still separate the two dates but
    // could no longer separate the two clocks, and half of this spec would
    // pass vacuously.
    const gapHours = await page.evaluate(() => {
      const f = (window as any).__fixtureProps;
      return (f.occurrenceStartMs - f.detail.start_ms) / 3_600_000;
    });
    expect(gapHours).toBe(169);

    const when = page.locator('.when');
    await expect(when).toContainText('Mon, Oct 26');
    await expect(when).toContainText('07:00');
    await expect(when).toContainText('07:30');
    // The master's own day and clock, which is what this used to show.
    await expect(when).not.toContainText('Oct 19');
    await expect(when).not.toContainText('06:00');
  });
});

// The all-day arm of the same line, in a browser **west** of UTC.
//
// Three separate ways to get this wrong, and the zone above is what makes the
// third visible: an all-day day rebuilt from a `yyyy-mm-dd` through `Date.UTC`
// and then formatted without `timeZone: 'UTC'` is put straight back through the
// browser's zone, which east of the reader is the previous day. From the
// project's default UTC browser that mistake is invisible.
test.describe('EventPopover on an all-day series east of the browser', () => {
  test.use({ timezoneId: 'America/New_York' });

  test('shows the clicked day, in the calendar’s zone, not a reading of an instant', async ({ page }) => {
    await page.goto('/tests/harness/index.html?c=EventPopover&f=all-day-series-east-of-the-browser');

    // Both premises, in the page so they are read in the browser's own zone.
    // Unless this browser reads the stored instants as *different* days from
    // the ones the calendar keeps them on, every assertion below is satisfied
    // by the fixture rather than by the component.
    const premise = await page.evaluate(() => {
      const f = (window as any).__fixtureProps;
      const day = (ms: number) =>
        new Date(ms).toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric' });
      return {
        browserReadsRow: day(f.detail.start_ms),
        browserReadsOccurrence: day(f.occurrenceStartMs),
        rowDate: f.detail.start_date,
        shiftDays: (f.occurrenceStartMs - f.detail.start_ms) / 86_400_000,
      };
    });
    // The calendar keeps the master on the 10th; this browser reads that same
    // instant as the 9th, and the clicked chip's as the 12th. The right answer
    // — the 13th — is a date neither reading produces.
    expect(premise.rowDate).toBe('2026-08-10');
    expect(premise.browserReadsRow).toBe('Sun, Aug 9');
    expect(premise.browserReadsOccurrence).toBe('Wed, Aug 12');
    expect(premise.shiftDays).toBe(3);

    await expect(page.locator('.when')).toHaveText('Thu, Aug 13');
  });

  test('an all-day event shows no clock at all', async ({ page }) => {
    // The `{#if !detail.is_all_day}` guard, which is the reason the times may
    // be read off instants on the other arm without this one having to care.
    await page.goto('/tests/harness/index.html?c=EventPopover&f=all-day-series-east-of-the-browser');
    await expect(page.locator('.when')).not.toContainText(':');
  });
});

test.describe('MonthGrid', () => {
  const show = (f: string) => `/tests/harness/index.html?c=MonthGrid&f=${f}`;

  // `MonthGrid` computes `todayStart` from `new Date()` while every fixture
  // here is a fixed August 2026, so the clock is frozen ahead of *every*
  // navigation in this block, not only the today-highlight spec below. Two
  // separate reasons, and only the first is about the new spec: an unfrozen
  // clock leaves the highlight untestable, and it also leaves the rest of the
  // block quietly reading wall-clock time in a suite that otherwise controls
  // it. Same mechanism as `App`'s own `beforeEach` and `YearGrid`'s
  // `YEAR_2026_NOW`; it has to run before `page.goto`, since the component
  // reads the clock once, at mount.
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(MONTH_2026_NOW);
  });

  test('renders six rows of seven, with out-of-month days dimmed', async ({ page }) => {
    await page.goto(show('august'));
    await expect(page.locator('.mrow')).toHaveCount(6);
    await expect(page.locator('.mcell')).toHaveCount(42);
    await expect(page.locator('.mcell.out')).toHaveCount(11); // 5 leading + 6 trailing
  });

  test('exactly one cell is today, and it is the day the clock is on', async ({ page }) => {
    // Both halves of the claim, because either alone is satisfied by a real
    // defect. `toHaveCount(1)` on `.mcell.today` says nothing about *which*
    // cell; asserting the 10th carries the class passes just as happily while
    // the 9th carries it too — turning the `===` into a `<=` highlights every
    // day up to today and would still find the 10th among them. So the whole
    // 42-cell vector is compared at once, the way the ribbon's weekend-stripe
    // spec compares all 28 of its columns rather than sampling one.
    await page.goto(show('august'));
    const cells = page.locator('.mcell');
    // Retried, so it also establishes that the component actually mounted —
    // and it is what makes the fixed loop bound below cover every cell there
    // is, rather than the first 42 of however many exist.
    await expect(cells).toHaveCount(42);

    const flagged: number[] = [];
    for (let i = 0; i < 42; i++) {
      if ((await cells.nth(i).getAttribute('class'))?.includes('today')) flagged.push(i);
    }
    // Row 2, column 0 — Mon 10 Aug 2026, the day `MONTH_2026_NOW` is 14:00 on.
    expect(flagged).toEqual([14]);
    // Not implied by the vector above, and not a restatement of it: that one
    // says which *cell* carries the class, this one says that cell is the one
    // showing the 10th. They read different things — a class attribute and a
    // rendered date — so a day number computed one day out leaves the vector
    // untouched and fails here alone.
    await expect(cells.nth(14).locator('.num')).toHaveText('10');
  });

  test('a multi-day event is one bar, not one chip per day', async ({ page }) => {
    await page.goto(show('august'));
    await expect(page.locator('.bar', { hasText: 'Berlin trip' })).toHaveCount(1);
  });

  test('two co-existing bars in one row each keep their own title', async ({ page }) => {
    // `august` never packs more than one bar per row, so `idx` and `lane`
    // never diverge there — a `bar_events[lane.idx]` / `bar_events[lane.lane]`
    // mix-up would still pass every other spec in this file.
    await page.goto(show('two-bars'));
    const bars = page.locator('.bar');
    await expect(bars).toHaveCount(2);
    await expect(bars.nth(0)).toContainText('Berlin trip');
    await expect(bars.nth(1)).toContainText('Team offsite');
  });

  test('a timed event shows a dot and a title, and no time', async ({ page }) => {
    // Spec §2: a time prefix costs about a third of a narrow cell.
    await page.goto(show('august'));
    const line = page.locator('.mcell .timed').first();
    await expect(line).toContainText('Standup');
    await expect(line).not.toContainText(':');
  });

  test('+N more asks the parent for that day', async ({ page }) => {
    await page.goto(show('busy-day'));
    await page.locator('.more').first().click();
    const picked = await page.evaluate(() => (window as any).__lastDayPick);
    expect(picked).toBe(1786320000000); // Mon 10 Aug 2026 00:00 UTC, the busy cell's own start
  });

  test('clicking the day number asks the parent for that day too', async ({ page }) => {
    await page.goto(show('august'));
    await page.locator('.mcell .num').nth(14).click();
    expect(await page.evaluate(() => (window as any).__lastDayPick)).toBeTruthy();
  });

  // Task 10. Both halves in one spec, because the risk is precisely that they
  // become the same click: the empty-space target covers the whole cell, and
  // the day number keeps its own click only by sitting above it. Invert the
  // `z-index` pair in `MonthGrid`'s styles and the second half fails — the
  // target swallows the number.
  test('empty cell space asks for a new event on that day, and the day number still does not', async ({ page }) => {
    // The grid's first cell, Mon 27 Jul, which carries nothing but its own
    // number — so a click on the middle of it is genuinely empty space and
    // nothing else could have answered.
    await page.goto(show('august'));
    const cell = page.locator('.mcell').first();

    await cell.locator('.newhere').click();
    expect(await page.evaluate(() => (window as any).__lastCreate)).toMatchObject({
      startMs: Date.UTC(2026, 6, 27),
    });
    expect(await page.evaluate(() => (window as any).__lastDayPick)).toBeFalsy();

    await cell.locator('.num').click();
    expect(await page.evaluate(() => (window as any).__lastDayPick)).toBe(Date.UTC(2026, 6, 27));
  });

  test('clicking an event opens the popover, not the day', async ({ page }) => {
    await page.goto(show('august'));
    await page.locator('.mcell .timed').first().click();
    expect(await page.evaluate(() => (window as any).__lastOpen)).toBeTruthy();
    expect(await page.evaluate(() => (window as any).__lastDayPick)).toBeFalsy();
  });

  test('a timed line keeps a real, readable height', async ({ page }) => {
    // `MAX_BAR_LANES`'s own comment explains why: unlike `BigYearRibbon`,
    // `.bars` here is deliberately content-sized rather than reserving
    // `MAX_BAR_LANES` fixed tracks with `grid-template-rows` — a month row
    // has only ~95px to divide, and a reserved bar strip leaves too little
    // for the cell below, measured to squeeze every timed line down to
    // 0.05px. Healthy is ~10px; 4px sits with margin on both sides of that
    // gap without pinning to the exact pixel value.
    await page.goto(show('busy-day'));
    const line = page.locator('.mcell .timed').first();
    expect((await line.boundingBox())!.height).toBeGreaterThan(4);
  });
});

test.describe('YearGrid', () => {
  const show = (f: string) => `/tests/harness/index.html?c=YearGrid&f=${f}`;

  // Lifted out of the today-highlight spec below, which used to be the only
  // one here that froze the clock — the other four still read the run date
  // through `YearGrid`'s own `todayStart`. Nothing they assert can observe it
  // today (the `today` class is independent of `dotted` and `unsynced`, and
  // this block takes no screenshots), so this changes no result; it removes a
  // wall-clock read from four more spec paths, which is the same thing being
  // done for `MonthGrid` above.
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(YEAR_2026_NOW);
  });

  test('renders twelve months', async ({ page }) => {
    await page.goto(show('y2026'));
    await expect(page.locator('.ymonth')).toHaveCount(12);
  });

  test('a day with an all-day event gets a dot', async ({ page }) => {
    await page.goto(show('y2026'));
    await expect(page.locator('.yday.dotted')).toHaveCount(1);
  });

  test('today is a filled disc, on the right day and no other', async ({ page }) => {
    // `YearGrid` reads the real wall clock; `y2026` is a fixed calendar year,
    // so the clock must be frozen to an instant inside it — same pattern as
    // `FIXED_NOW` for the Header specs above — or this becomes a permanent
    // failure the moment the real date rolls past 2026. That freeze is the
    // `beforeEach` at the top of this block.
    //
    // This was `expect('.yday.today').toHaveCount(1)`, which pinned *how many*
    // discs and not *which day*: shift the highlight a day and the count is
    // still exactly 1, so the spec passed while the grid marked the wrong
    // date. The month-by-month vector below pins both at once — the same
    // treatment `MonthGrid`'s today spec gets, in the shape this grid has.
    //
    // Read per month rather than as one flat 365-long list because that is the
    // claim worth making: *no other month* has a disc, and June's is the 10th.
    // A flat day-of-year index would say the same thing in a number nobody can
    // check by eye.
    await page.goto(show('y2026'));
    const months = page.locator('.ymonth');
    // Retried, so it also establishes the component mounted — and it is what
    // makes the fixed loop bound below cover every month there is. `YearGrid`
    // takes its payload as a prop and renders in one pass, so once the twelve
    // months are here, so is every `.yday` inside them.
    await expect(months).toHaveCount(12);

    const flagged: string[][] = [];
    for (let i = 0; i < 12; i++) {
      flagged.push(await months.nth(i).locator('.yday.today').allTextContents());
    }
    // `YEAR_2026_NOW` is Wed 10 Jun 2026, so June — index 5 — and nothing else.
    expect(flagged).toEqual([[], [], [], [], [], ['10'], [], [], [], [], [], []]);
  });

  test('unsynced days are distinct from empty ones', async ({ page }) => {
    // §6: an empty January must not read as a free January.
    await page.goto(show('y2026'));
    const unsynced = page.locator('.yday.unsynced').first();
    await expect(unsynced).toBeVisible();
    await expect(unsynced).not.toHaveClass(/dotted/);
  });

  test('clicking a date asks the parent for that day', async ({ page }) => {
    await page.goto(show('y2026'));
    await page.locator('.yday').nth(200).click();
    expect(await page.evaluate(() => (window as any).__lastDayPick)).toBeTruthy();
  });
});

test.describe('BigYearRibbon', () => {
  const show = (f: string) => `/tests/harness/index.html?c=BigYearRibbon&f=${f}`;

  test('renders fourteen rows of twenty-eight', async ({ page }) => {
    await page.goto(show('y2026'));
    await expect(page.locator('.rrow')).toHaveCount(14);
    await expect(page.locator('.rrow').first().locator('.rday')).toHaveCount(28);
  });

  test('weekend shading forms straight vertical stripes', async ({ page }) => {
    // The 28-day row exists for this. Assert the column indices, not a
    // screenshot: this is the property, and a screenshot would also pass
    // for a subtly different one.
    await page.goto(show('y2026'));
    for (const r of [0, 7, 13]) {
      const cols: number[] = [];
      const days = page.locator('.rrow').nth(r).locator('.rday');
      for (let i = 0; i < 28; i++) {
        if ((await days.nth(i).getAttribute('class'))?.includes('wknd')) cols.push(i);
      }
      expect(cols).toEqual([5, 6, 12, 13, 19, 20, 26, 27]);
    }
  });

  test('days outside the year are dimmed, not blank', async ({ page }) => {
    await page.goto(show('y2026'));
    const out = page.locator('.rday.out').first();
    await expect(out).toBeVisible();
    await expect(out).not.toBeEmpty();
  });

  test('a span crossing a row shows a continuation marker on both halves', async ({ page }) => {
    await page.goto(show('crossing'));
    await expect(page.locator('.pill.cont')).toHaveCount(2);
  });

  // ---- a title appears once, not on every row ------------------------------

  /** `crossingBigYear`'s span, which is the only text either of its pills
   *  could carry. Read from one place so "the head has it" and "the tail does
   *  not" cannot drift apart. */
  const CROSSING_TITLE = 'Sun-Tue trip';

  test('a title is printed once, on the segment that starts the run', async ({ page }) => {
    // `crossing` is one event across a row boundary: `rows[0]` holds the head
    // (columns 25-27, `cont_right`) and `rows[1]` the tail (columns 0-1,
    // `cont_left`), in that DOM order. A three-row conference used to print its
    // name three times.
    await page.goto(show('crossing'));
    const pills = page.locator('.pill');
    await expect(pills).toHaveCount(2);
    await expect(pills.nth(0)).toHaveText(CROSSING_TITLE);
    // Bare, which also covers the `‹` that used to be the tail's first
    // characters — an empty segment cannot be carrying a chevron.
    await expect(pills.nth(1)).toHaveText('');
  });

  test('a bare continuation is still a pill: it spans its days and it opens', async ({ page }) => {
    // The other half of "shows no text": everything else about the segment is
    // unchanged. Without this, deleting the tail element outright would satisfy
    // the spec above.
    await page.goto(show('crossing'));
    const tail = page.locator('.pill.cl');

    // Two of the row's twenty-eight columns, because the span runs into day 2 —
    // measured against a day in the same row rather than written as a pixel
    // count, so it stays true at any width. The fill is what carries the run
    // now that the text is gone, so a tail collapsed to nothing would be the
    // whole feature lost.
    const tailBox = (await tail.boundingBox())!;
    const dayBox = (await page.locator('.rrow').nth(1).locator('.rday').first().boundingBox())!;
    expect(tailBox.width).toBeGreaterThan(dayBox.width);
    expect(tailBox.width).toBeLessThan(3 * dayBox.width);

    await tail.click();
    expect(await page.evaluate(() => (window as any).__lastOpen?.event?.title))
      .toBe(CROSSING_TITLE);
  });

  test('a continuation segment still has an accessible name', async ({ page }) => {
    // The regression the change above would otherwise have shipped: the tail's
    // only content *was* the title, so it became a `<button>` with nothing in
    // it, and a control with no name is unreachable by name to anything driving
    // the app through the accessibility tree.
    //
    // Both assertions are here on purpose and only the first one bites if the
    // `aria-label` is removed. Measured, by deleting the label and running the
    // `getByRole` line ahead of the other: it still finds both buttons, in
    // WebKit and in Chromium — `title` is also on the element and both engines
    // fall back to it for the accessible name. So the second line cannot be
    // this test's witness, which is exactly the reason the attribute is
    // asserted directly rather than through the name.
    //
    // It is still worth having: it is the one that says the name genuinely
    // resolves in each engine, rather than that an attribute is spelled right.
    // The point of the label is that the name stops depending on a fallback the
    // two engines are free to disagree about, and only one of these two lines
    // can see that the fallback is gone.
    await page.goto(show('crossing'));
    await expect(page.locator('.pill.cl')).toHaveAttribute('aria-label', CROSSING_TITLE);
    await expect(page.getByRole('button', { name: CROSSING_TITLE, exact: true })).toHaveCount(2);
  });

  test('the legend names each calendar that has a pill', async ({ page }) => {
    await page.goto(show('y2026'));
    await expect(page.locator('.legend .item')).toHaveCount(2);
  });

  test('two calendars sharing a name still render the whole ribbon', async ({ page }) => {
    // Two accounts subscribed to the same public calendar report the same
    // `summary`, which `get_big_year` copies verbatim into `name`. Keying the
    // legend by `name` makes Svelte 5 throw `each_key_duplicate`, and that is
    // not a broken legend — the component never mounts, so the rows go with
    // it. The rows are asserted first for exactly that reason: the legend
    // count alone would not say which failure mode this is guarding.
    await page.goto(show('same-name-legend'));
    await expect(page.locator('.rrow')).toHaveCount(14);
    await expect(page.locator('.pill')).toHaveCount(2);
    await expect(page.locator('.legend .item')).toHaveCount(2);
  });

  test('clicking a pill opens the popover', async ({ page }) => {
    await page.goto(show('y2026'));
    await page.locator('.pill').first().click();
    expect(await page.evaluate(() => (window as any).__lastOpen)).toBeTruthy();
  });

  // ---- solid pills: choosing an ink against the fill, not against the theme --
  //
  // `foregroundFor` itself is tabled in `ink.spec.ts`, in Node and without a
  // browser context. What is left here is the part that is genuinely about CSS:
  // that the ink it picks reaches the pill, that the fill really is the
  // calendar's colour at full strength, and that the two edge cases the solid
  // fill created — an unparseable `--cal`, and a continuation marker that used
  // to be drawn in the fill's own colour — still leave something readable.

  /** Theme variables as the engine computes them.
   *
   *  `rgba(0,0,0,.88)` and `rgba(0, 0, 0, 0.88)` are one colour and two
   *  strings, so a spec comparing a pill's computed `color` against
   *  `theme.ts`'s literal would be failing on serialisation rather than on the
   *  property. Putting both sides through the same probe normalises that.
   *
   *  Resolved rather than copied so these tests say "the ink the theme
   *  publishes for a light fill" and not "rgba(0, 0, 0, 0.88)". A copy of those
   *  literals here would be a second place that knows them, which is the thing
   *  `theme.ts` exists to prevent. */
  const resolveInks = (page: Page, vars: string[]) => page.evaluate((vs: string[]) => {
    const probe = document.createElement('span');
    document.body.appendChild(probe);
    const out = vs.map((v) => {
      probe.style.color = v;
      return getComputedStyle(probe).color;
    });
    probe.remove();
    return out;
  }, vars);

  test('a pale fill and a dark one on the same row take different inks', async ({ page }) => {
    // The rendered half of `ink.spec.ts`: that the decision reaches the pill at
    // all. Both pills are in one row, which is the arrangement that makes a
    // fixed `color:` impossible — omacal shows Google's pale yellow beside its
    // dark blue, and a single foreground fails one of them.
    await page.goto(show('pill-inks'));
    const pills = page.locator('.pill');
    await expect(pills).toHaveCount(3);

    const [light, dark] = await resolveInks(page, ['var(--ink-on-light)', 'var(--ink-on-dark)']);
    // Without this the two assertions below could both hold with one ink.
    expect(light).not.toBe(dark);

    const ink = (n: number) => pills.nth(n).evaluate((el) => getComputedStyle(el).color);
    expect(await ink(0)).toBe(light); // #f6bf26, pale
    expect(await ink(1)).toBe(dark);  // #3f51b5, dark

    // …and that the fill really is the calendar's colour at full strength,
    // which is what makes the ink question exist at all. Under the old 16%
    // wash both pills' backgrounds were within a few percent of the theme's
    // own and this would read `rgba(…, 0.16)`.
    const fill = await pills.nth(0).evaluate((el) => getComputedStyle(el).backgroundColor);
    expect(fill).toBe('rgb(246, 191, 38)');
  });

  test('an unreadable colour is still readable', async ({ page }) => {
    // `ev.color` is non-nullable and `to_ui` only ever produces a hex, so this
    // is not a payload the backend sends — it is the guard on `foregroundFor`
    // being called during a render, where a throw takes the whole ribbon down
    // rather than one pill.
    //
    // The claim about *why* `var(--text)` is the right fallback is checked
    // here, not reasoned about: `background: var(--cal)` with a value the
    // browser cannot parse is invalid at computed-value time, so the
    // declaration drops to `transparent` and what shows behind the text is the
    // app's own background — which is exactly the surface `--text` is legible
    // on. If an engine ever resolved that differently the fallback would be
    // wrong, and this is what would say so.
    await page.goto(show('pill-inks'));
    const broken = page.locator('.pill').nth(2);
    await expect(broken).toHaveAttribute('title', 'Unreadable');

    const seen = await broken.evaluate((el) => ({
      color: getComputedStyle(el).color,
      background: getComputedStyle(el).backgroundColor,
    }));
    const [text] = await resolveInks(page, ['var(--text)']);
    // The fill the browser could not paint, and so the surface the text is
    // really sitting on: the app's own background, not the calendar's colour.
    expect(seen.background).toBe('rgba(0, 0, 0, 0)');
    expect(seen.color).toBe(text);
    // …and `--text` is not one of the two inks, so this is a genuinely
    // different branch rather than the light/dark decision landing somewhere
    // that happens to be legible.
    const [light, dark] = await resolveInks(page, ['var(--ink-on-light)', 'var(--ink-on-dark)']);
    expect([light, dark]).not.toContain(text);
  });

  test('a continuation edge stays visible against a solid fill', async ({ page }) => {
    // The dashed left edge is what says "this started earlier". It used to be
    // `border-left-style: dashed` over `border-left: 2px solid var(--cal)`,
    // which worked against a 16% wash and paints nothing at all against a fill
    // that *is* `--cal`: same colour, same colour, no marker. Asserted against
    // the pill's own background rather than against a named colour, so it
    // stays the right question if either end ever changes.
    await page.goto(show('crossing'));
    const tail = page.locator('.pill.cl');
    await expect(tail).toHaveCount(1);

    const edge = await tail.evaluate((el) => {
      const s = getComputedStyle(el);
      return { color: s.borderLeftColor, style: s.borderLeftStyle, background: s.backgroundColor };
    });
    expect(edge.style).toBe('dashed');
    expect(edge.color).not.toBe(edge.background);
  });

  // Task 10. The ribbon's day strip carried no click handler at all before
  // this, so it is the one grid where "empty space" is the whole day cell —
  // and the `z-index` pair that keeps the day number above the target is the
  // same shape `MonthGrid` needs, for the same reason.
  test('clicking a day asks the parent for a new event on it', async ({ page }) => {
    await page.goto(show('y2026'));
    // Row 0, column 4: four days after the ribbon's own anchor of Mon 29 Dec
    // 2025, so Fri 2 Jan 2026. No pill on it (row 0's runs 8-10) and not a
    // first-of-month (which would put a `.mchip` in the way), so nothing else
    // could have answered.
    //
    // Off-centre on purpose: a ribbon day is about 45px wide, its date label
    // sits at the top of it, and the click has to land on the part that is
    // actually empty — which is also what proves the label is still on top of
    // the target rather than buried under it.
    //
    // `y: 3` is inside the date band, which is the one strip of the day box
    // that `.pills` does not cover. That makes this spec blind to the overlay
    // swallowing clicks, and it is why "a day under the pill strip is still
    // clickable" below exists as a separate test rather than as another
    // assertion here.
    await page.locator('.rrow').first().locator('.rday .newhere').nth(4)
      .click({ position: { x: 3, y: 3 } });
    expect(await page.evaluate(() => (window as any).__lastCreate)).toMatchObject({
      startMs: Date.UTC(2026, 0, 2),
    });
  });

  test('a day under the pill strip is still clickable', async ({ page }) => {
    // The trap the overlay sets. `.pills` spans all 28 columns across the
    // middle of every day box, and a grid container receives pointer events
    // across its whole box — not only where a pill is actually drawn. Without
    // `pointer-events: none` on the strip (and `auto` back on the pills
    // themselves) it silently eats every click meant for the empty space
    // underneath it, on exactly the rows that have events on them.
    //
    // Row 0 of `y2026` is the right row for that and a row with no pills at all
    // is not: an empty row still has a `.pills` box covering it, but a spec that
    // used one would be indistinguishable from a spec that got lucky. Column 4
    // (Fri 2 Jan 2026) has no pill on it — row 0's runs 8-10 — and is not a
    // first-of-month, so nothing else could have answered.
    await page.goto(show('y2026'));
    const row = page.locator('.rrow').first();
    await expect(row.locator('.pill')).toHaveCount(1); // the strip is real here

    const day = row.locator('.rday').nth(4);
    const dayBox = (await day.boundingBox())!;
    const strip = (await row.locator('.pills').boundingBox())!;
    const pill = (await row.locator('.pill').boundingBox())!;

    // The point: horizontally over an empty day, vertically in the middle of
    // the strip. Derived from the boxes rather than written as pixels, so it
    // stays the right point if the date band or the lane height changes.
    const x = dayBox.x + dayBox.width / 2;
    const y = strip.y + strip.height / 2;

    // The premise, in two halves. The point is genuinely underneath the strip
    // — otherwise this is just the previous test again — and it is not on a
    // pill, or `oncreate` would rightly never fire.
    expect(y).toBeGreaterThan(strip.y);
    expect(y).toBeLessThan(strip.y + strip.height);
    expect(x > pill.x && x < pill.x + pill.width).toBe(false);

    await page.mouse.click(x, y);
    expect(await page.evaluate(() => (window as any).__lastCreate)).toMatchObject({
      startMs: Date.UTC(2026, 0, 2),
    });
  });

  // ---- an event lights up across its whole span ----------------------------

  test('hovering a segment lights every row the same occurrence runs through', async ({ page }) => {
    // The multi-row half. `crossing` is one occurrence with a segment in each
    // of two rows, and the point of the feature is that both light at once —
    // with a dozen bars stacked, finding where a trip ends is the problem.
    await page.goto(show('crossing'));
    const pills = page.locator('.pill');
    await expect(pills).toHaveCount(2);
    await expect(page.locator('.pill.lit')).toHaveCount(0);

    await pills.nth(0).hover();
    await expect(page.locator('.pill.lit')).toHaveCount(2);

    // And it lets go. Without this a highlight that never cleared would satisfy
    // every other assertion here.
    await page.locator('.legend').hover();
    await expect(page.locator('.pill.lit')).toHaveCount(0);
  });

  test('hovering one occurrence of a series lights that one only', async ({ page }) => {
    // **The witness that matters.** Every occurrence of a recurring series
    // carries its master row's id, so keying the highlight on `ev.id` — the
    // obvious repair once `lane.idx` is ruled out — lights the whole series:
    // hover January's standup and all fifty-two glow.
    //
    // A ribbon of single events cannot see that, because there id and
    // occurrence are the same thing and the broken version passes. `recurring`
    // is three occurrences sharing one id, plus one unrelated event.
    //
    // The middle occurrence, not the first: an implementation that lit "the
    // first segment with this id" would be accidentally right on the first.
    await page.goto(show('recurring'));
    const pills = page.locator('.pill');
    await expect(pills).toHaveCount(4);

    // The fixture's premise, asserted rather than trusted: three of these pills
    // really do share one id, and really do carry different starts. If that
    // ever stopped being true the test below would still pass and would no
    // longer be about anything.
    const identity = await pills.evaluateAll((els) => els.map((el) => el.getAttribute('title')));
    expect(identity.filter((t) => t === 'Standup')).toHaveLength(3);

    await pills.nth(1).hover();
    const lit = page.locator('.pill.lit');
    await expect(lit).toHaveCount(1);
    // …and it is the one under the cursor, not merely "one of them".
    const litBox = (await lit.boundingBox())!;
    const hoveredBox = (await pills.nth(1).boundingBox())!;
    expect(litBox.x).toBeCloseTo(hoveredBox.x, 0);
  });

  test('the occurrence whose popover is open stays lit, across its rows', async ({ page }) => {
    // The other input to the same predicate, and it arrives as a prop: App
    // hands down the `gridSelId`/`gridSelStart` it already keeps for the open
    // popover. Both segments of the crossing span light, with nothing hovered.
    await page.goto(show('crossing-open'));
    await expect(page.locator('.pill')).toHaveCount(2);
    await expect(page.locator('.pill.lit')).toHaveCount(2);
  });

  test('an open popover on one occurrence of a series lights that one only', async ({ page }) => {
    // The recurring trap through the prop rather than through the cursor. Both
    // inputs go through one predicate, but only a spec per input can say so.
    await page.goto(show('recurring-open'));
    await expect(page.locator('.pill')).toHaveCount(4);
    await expect(page.locator('.pill.lit')).toHaveCount(1);
  });

  test('a highlight changes nothing about where a pill sits', async ({ page }) => {
    // These pills are under the cursor by definition. A highlight that moved
    // anything would make the ribbon jump out from under it, and could re-enter
    // the pill and flicker. `filter` and an inset `box-shadow` are chosen for
    // exactly that: they paint without taking part in layout.
    //
    // Every segment's box is compared, not just the hovered one — a highlight
    // that grew the hovered pill would push its neighbours about too, and the
    // second row's segment is the one that would show it.
    await page.goto(show('crossing'));
    const pills = page.locator('.pill');
    const boxes = async () => pills.evaluateAll((els) => els.map((el) => {
      const r = el.getBoundingClientRect();
      return [r.x, r.y, r.width, r.height].map((n) => Math.round(n * 100) / 100);
    }));

    const before = await boxes();
    await pills.nth(0).hover();
    await expect(page.locator('.pill.lit')).toHaveCount(2); // it really is lit
    expect(await boxes()).toEqual(before);
  });

  // ---- the day box is the container, and the event is drawn in it ----------
  //
  // What used to be here was "a fully packed row keeps its day strip, and its
  // weekend stripes with it", from when `.pills` and `.rdays` were siblings in
  // a flex column and a tall pill strip could squeeze the day strip to zero.
  // Its last assertion was `.rdays` height >= one `.rday` height, and under the
  // overlay that **cannot fail**: `.rday` is a grid item in `.rdays`'s single
  // row, so it stretches to exactly `.rdays`'s height and the comparison is
  // `x >= x` whatever the layout does. It was left passing by the restructure
  // rather than made true by it, which is the worse of the two ways for a spec
  // to survive a change. Replaced by the three below, which are about the
  // arrangement that now exists.

  test('the day box is the whole row, not a strip under the pills', async ({ page }) => {
    // The headline of the change. A day cell is the full-height box — the same
    // box the event is drawn in — rather than a thin band of numbers beneath a
    // band of floating bars.
    //
    // Measured against `.rrow` rather than written as a number so it stays true
    // at any window height. The 1px is `.rrow`'s own top border, which is not
    // part of the box its contents get; read off the element for the same
    // reason.
    await page.goto(show('y2026'));
    const row = page.locator('.rrow').nth(1);
    const rowBox = (await row.boundingBox())!;
    const dayBox = (await row.locator('.rday').first().boundingBox())!;
    const border = await row.evaluate((el) => parseFloat(getComputedStyle(el).borderTopWidth));
    expect(border).toBe(1); // the line below is meaningless if this is 0
    expect(dayBox.height).toBeCloseTo(rowBox.height - border, 0);
  });

  test('a pill is drawn inside its day box, below the date', async ({ page }) => {
    // The other half, and the one that fails against the old arrangement: a
    // pill used to live in a strip *above* the day boxes, so its bottom edge
    // was at or above the day's top edge. Now it is within the day's own box,
    // and below the date label that sits at the top of it.
    //
    // Row 1 of `y2026` is Team off-site on columns 4-6, which is a day that
    // genuinely carries the pill — the containment claim is empty if the pill
    // and the day box are not the same days.
    await page.goto(show('y2026'));
    const row = page.locator('.rrow').nth(1);
    const pill = (await row.locator('.pill').boundingBox())!;
    const day = (await row.locator('.rday').nth(5).boundingBox())!;
    const label = (await row.locator('.rday').nth(5).locator('.dlabel').boundingBox())!;

    // The pill really is over that day, horizontally — otherwise the vertical
    // containment below would be a claim about two unrelated boxes.
    expect(pill.x).toBeLessThan(day.x);
    expect(pill.x + pill.width).toBeGreaterThan(day.x + day.width);

    // Inside the box, top and bottom.
    expect(pill.y).toBeGreaterThanOrEqual(day.y);
    expect(pill.y + pill.height).toBeLessThanOrEqual(day.y + day.height + 0.5);

    // …and clear of the date, which sits at the top of the box rather than in
    // the middle of it. Both halves matter: the first says the date is at the
    // top at all, the second that the pills start below it.
    expect(label.y).toBeCloseTo(day.y, 0);
    expect(pill.y).toBeGreaterThanOrEqual(label.y + label.height);
  });

  test('a row that needs a third lane grows to fit it', async ({ page }) => {
    // The property the overlay could most easily have dropped. `pack_lanes`
    // caps at three and the reservation at 720p is two, so this row packs one
    // lane more than the layout budgeted for — and the row has to grow, or the
    // third lane is drawn over the next row's dates.
    //
    // It survives because `.pills` is a *grid item* sharing a cell with
    // `.rdays`, not an absolutely positioned overlay: both layers still size
    // the row. An absolutely positioned strip contributes nothing to its
    // parent's height, and this is the spec that would have caught that.
    // At a viewport this short every row is clamped to its own minimum — the
    // total (13 quiet rows plus the packed one) is well past what `.rows` has,
    // so `.rows` scrolls and no row gets a share of spare height. That is
    // deliberate: with slack available the quiet rows grow to meet the packed
    // one anyway, and the difference this test is about shrinks to under a
    // pixel. Clamped, it is the whole third lane.
    await page.setViewportSize({ width: 1280, height: 500 });
    await page.goto(show('three-lanes-exact'));
    const row = page.locator('.rrow').first();
    await expect(row.locator('.pill')).toHaveCount(3);

    // The strip grew past the reservation: three lanes, not the two reserved.
    const quietPills = (await page.locator('.rrow').nth(1).locator('.pills').boundingBox())!;
    const packedPills = (await row.locator('.pills').boundingBox())!;
    expect(packedPills.height).toBeGreaterThan(quietPills.height);

    // …and the row grew with it. This is the half an absolute overlay fails:
    // out of flow, `.pills` contributes nothing to its parent's height, every
    // row clamps to the same date-band minimum, and the third lane is drawn
    // over the row below. Measured against a quiet row rather than as a number,
    // so it says "this row is taller *because* of its lanes".
    const packedRow = (await row.boundingBox())!;
    const quietRow = (await page.locator('.rrow').nth(1).boundingBox())!;
    expect(packedRow.height).toBeGreaterThan(quietRow.height);
    // …and it is tall enough to actually contain the strip, not merely taller.
    expect(packedPills.y + packedPills.height)
      .toBeLessThanOrEqual(packedRow.y + packedRow.height + 0.5);
  });

  // ---- the reservation follows the height the window actually has ----------

  /** How many lane tracks `.pills` reserves, read off a row with no pills at
   *  all — so it is the reservation being measured and not the content. The
   *  computed `grid-template-rows` of a grid container is its resolved track
   *  list, which is what "reserved" means here. */
  const reservedLanes = (page: Page, rrow: number) => page
    .locator('.rrow').nth(rrow).locator('.pills')
    .evaluate((el) => getComputedStyle(el).gridTemplateRows.split(/\s+/).length);

  /** `.rows` scrolling, in pixels. `<= 0` is "fourteen rows on one screen". */
  const rowsOverflow = (page: Page) => page.locator('.rows')
    .evaluate((el) => el.scrollHeight - el.clientHeight);

  test('all fourteen rows fit on one screen with no scroll', async ({ page }) => {
    // The design doc's own promise for this view (spec §4): "Big Year — one
    // screen, the whole year." Pinned so the budget it depends on can't drift
    // back out of reach without a spec noticing, the way the original fixed
    // 3-lane reservation did.
    //
    // The container is 620px of `.rows` at the suite's default 1280x720
    // viewport (`devices['Desktop Chrome']`/`['Desktop Safari']` in
    // playwright.config.ts — no `setViewportSize` here, same as every other
    // spec in this file), against fourteen rows' own 615px minimum at the
    // 14px lanes of the 2026-08-14 readability pass — a 5px margin, the
    // tightest this promise has ever been held by. It used
    // to be 530px, which is *less* than that minimum: the ribbon was pinned at
    // `calc(100vh - 190px)` and this spec passed only because a fourteen-row
    // ribbon with one pill per row is a hair under what 530px holds. It now
    // reads the box `App` genuinely leaves a view, which
    // `app.spec.ts`'s "a standalone view gets the same box the app gives it"
    // pins to the real thing — so this is a claim about 720p in the app and
    // not only about the harness.
    //
    // 720p is below the 924px at which a third lane fits, so this is also the
    // regression net for the reservation not creeping up: pinned to a constant
    // three, `.rows` overflows here by 205px (the 14px-lane arithmetic, and
    // the order of what this fails with). It deliberately does *not* also assert the lane count.
    // Three lanes at 720p always overflow, so a lane-count line here could
    // never fail on its own — it would be a passenger, and the reservation at
    // the small end is claimed where it can be seen instead: "a tall window
    // levels the busiest row", whose 720p half needs the bulge that only two
    // reserved lanes produce.
    await page.goto(show('y2026'));
    await expect(page.locator('.rrow')).toHaveCount(14);
    expect(await rowsOverflow(page)).toBeLessThanOrEqual(0);
  });

  test('a window tall enough for a third lane reserves one', async ({ page }) => {
    // The other end. 924px is the measured height at which fourteen rows of
    // three reserved lanes plus the legend first fit — `.pills`'s own comment
    // has the arithmetic and how it was measured. Set one pixel above it rather
    // than exactly on it: this spec is about the reservation being taken when
    // there is room, and sitting on the boundary would make it a spec about the
    // boundary, which the pair below is for.
    await page.setViewportSize({ width: 1280, height: 925 });
    await page.goto(show('y2026'));
    await expect(page.locator('.rrow')).toHaveCount(14);
    expect(await reservedLanes(page, 2)).toBe(3);
    // §4 still holds here — taking the third lane must not cost the promise it
    // was traded against in the first place.
    expect(await rowsOverflow(page)).toBeLessThanOrEqual(0);
  });

  test('the third lane is taken exactly where it starts fitting', async ({ page }) => {
    // The threshold itself, from both sides, one pixel apart. A spec that only
    // checked a tall window and a short one would pass for a threshold anywhere
    // between them; this is what says the number is 924 and not "somewhere
    // around 950".
    await page.goto(show('y2026'));

    await page.setViewportSize({ width: 1280, height: 924 });
    expect(await reservedLanes(page, 2)).toBe(3);
    expect(await rowsOverflow(page)).toBeLessThanOrEqual(0);

    await page.setViewportSize({ width: 1280, height: 923 });
    expect(await reservedLanes(page, 2)).toBe(2);
    expect(await rowsOverflow(page)).toBeLessThanOrEqual(0);
  });

  test('a tall window levels the busiest row with the quiet ones', async ({ page }) => {
    // Why the third lane is worth reserving at all: uniform rows. Levelling
    // every row's height was Big Year's original property, and a fixed
    // reservation of two gave it up at every window size to keep §4 at the
    // small end. A tall window does not have to pay that.
    //
    // `three-lanes-exact` is three genuinely overlapping spans with nothing
    // folded away — the busiest shape a three-lane reservation can level. (Its
    // neighbour `three-lanes` also overflows, so its strip carries a fourth
    // implicit track for the "+N more" line and stands proud of a quiet row
    // whatever the reservation is.)
    await page.goto(show('three-lanes-exact'));
    const packed = page.locator('.rrow').nth(0).locator('.pills');
    const quiet = page.locator('.rrow').nth(1).locator('.pills');
    await expect(page.locator('.rrow').nth(0).locator('.pill')).toHaveCount(3);

    await page.setViewportSize({ width: 1280, height: 925 });

    // The premise of the whole method, and it needs stating because it is one
    // CSS declaration away from being false: `.pills` is the lane strip, so its
    // height *means* "the lanes this row has". `align-self: start` is what makes
    // that so; stretched to the shared grid cell it would be the row height
    // minus the date band instead, and both comparisons below would quietly
    // turn into comparisons of two row heights. Measured, that is not a
    // hypothetical — under `stretch` the two rows differ only by where the
    // border falls, 38.14 against 38.16, and this test passed in WebKit and
    // failed in Chromium on 0.02px of rounding. Reading the box against the
    // tracks it is supposed to be makes it fail in both, for the real reason.
    const boxIsItsTracks = async (strip: typeof quiet) => {
      const m = await strip.evaluate((el) => {
        const cs = getComputedStyle(el);
        return {
          height: el.getBoundingClientRect().height,
          tracks: cs.gridTemplateRows.split(/\s+/).map(parseFloat),
          gap: parseFloat(cs.rowGap),
        };
      });
      const tracks = m.tracks.reduce((a, b) => a + b, 0) + (m.tracks.length - 1) * m.gap;
      expect(m.tracks.length).toBeGreaterThan(0);
      expect(m.height).toBeCloseTo(tracks, 1);
    };
    await boxIsItsTracks(quiet);
    await boxIsItsTracks(packed);
    expect((await packed.boundingBox())!.height).toBe((await quiet.boundingBox())!.height);

    // …and at 720p it does bulge, which is the compromise the small end still
    // makes. Asserted rather than left implied: without it, a reservation stuck
    // at three would satisfy the line above and this test would be claiming a
    // property of the *layout* rather than of the window.
    await page.setViewportSize({ width: 1280, height: 720 });
    expect((await packed.boundingBox())!.height)
      .toBeGreaterThan((await quiet.boundingBox())!.height);
  });

  // The reported defect, at the one place App's own specs cannot see it: its
  // `get_big_year` stub returns an empty legend, so the App-level height specs
  // in `app.spec.ts` only ever exercise a ribbon with nothing under its rows.
  // The legend is what made the old rule *look* nearly right — `100vh - 190px`
  // was 150px of guessed chrome plus 40px reserved for a legend that is not
  // 40px tall — and it is why the number the user reported (~95px short) was
  // smaller than the 123px this leaves with no legend at all. Nothing reserves
  // anything now: `.legend` takes what it needs, `.rows` takes the rest, and
  // the two together reach the bottom of the box the parent gave the ribbon.
  test('the rows take whatever the legend does not', async ({ page }) => {
    await page.goto(show('y2026'));
    await expect(page.locator('.legend .item')).toHaveCount(2);
    // Measured against the mount container rather than the window: this
    // component no longer knows anything about the window, which is the fix.
    const gap = await page.evaluate(() => {
      const box = document.getElementById('app')!.getBoundingClientRect();
      return box.bottom - document.querySelector('.legend')!.getBoundingClientRect().bottom;
    });
    // `.ribbon`'s own padding, and nothing else — read off the element so this
    // says "nothing but the padding" rather than "nothing but four pixels".
    const pad = await page.locator('.ribbon').evaluate(
      (el) => parseFloat(getComputedStyle(el).paddingBottom),
    );
    expect(gap).toBeCloseTo(pad, 0);
  });

  test('days outside the synced window are hatched, not left blank', async ({ page }) => {
    // §6: the window opens 180 days back, so a ribbon anchored the previous
    // December always begins outside it. Nothing else distinguishes those days
    // from an in-window day with nothing on it, so without this an unsynced
    // stretch reads as "free".
    await page.goto(show('unsynced'));
    await expect(page.locator('.rrow').nth(0).locator('.rday.unsynced')).toHaveCount(28);
    await expect(page.locator('.rrow').nth(1).locator('.rday.unsynced')).toHaveCount(28);
    await expect(page.locator('.rrow').nth(2).locator('.rday.unsynced')).toHaveCount(0);
    // The hatch is a real painted background, not just a class name.
    const hatched = page.locator('.rday.unsynced').first();
    expect(await hatched.evaluate((el) => getComputedStyle(el).backgroundImage))
      .toContain('repeating-linear-gradient');
  });

  test('two co-existing pills in one row each keep their own title', async ({ page }) => {
    // `y2026` and `crossing` never pack more than one pill per row, so `idx`
    // and `lane` never diverge there — a `pill_events[lane.idx]` /
    // `pill_events[lane.lane]` mix-up would still pass every other spec in
    // this file. Same guard as MonthGrid's `two-bars` spec.
    await page.goto(show('two-pills'));
    const pills = page.locator('.pill');
    await expect(pills).toHaveCount(2);
    await expect(pills.nth(0)).toContainText('Berlin trip');
    await expect(pills.nth(1)).toContainText('Team offsite');
  });
});

test.describe('Filmstrip', () => {
  const show = (f: string) => `/tests/harness/index.html?c=Filmstrip&f=${f}`;

  // Frozen inside the fixture's Monday, mid-afternoon-gap, because the list
  // reads the clock now: the marker between "has started" and "still to
  // come" is drawn from it. 13:00 sits between Standup (09:00) and Ops
  // review (14:00), which is what lets the marker spec name its neighbours.
  // Same mechanism as every grid block above; before `page.goto`, since the
  // component reads the clock at mount.
  const STRIP_NOW = MON + 13 * 3_600_000;
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(STRIP_NOW);
  });

  // Which days are listed and in what order is `filmstrip.ts`'s, and
  // `filmstrip.spec.ts` holds it to that against the payloads directly. This
  // block is about what a day and a row actually *say*.

  test('one section per day, each named by the day it is about', async ({ page }) => {
    await page.goto(show('week'));
    const days = page.locator('.sday');
    await expect(days).toHaveCount(4);
    // The dates, not merely the count: a list that rendered four sections all
    // titled Monday satisfies a count.
    await expect(days.nth(0).locator('.sdate')).toHaveText('Mon, Jan 1');
    await expect(days.nth(1).locator('.sdate')).toHaveText('Wed, Jan 3');
    await expect(days.nth(2).locator('.sdate')).toHaveText('Thu, Jan 4');
    await expect(days.nth(3).locator('.sdate')).toHaveText('Fri, Jan 5');
  });

  test('a row shows the time, the title and the place', async ({ page }) => {
    // Spec §5. The location goes through `locationLabel`, which is why the
    // fixture's Wednesday event carries a bare Zoom URL: a row printing
    // `https://us02we…` would pass a test that only asked whether *something*
    // was there.
    await page.goto(show('week'));
    const row = page.locator('.sday').nth(0).locator('.srow');
    await expect(row.nth(0).locator('.when')).toHaveText('09:00–09:30');
    await expect(row.nth(0).locator('b')).toHaveText('Standup');
    // Standup has no location at all, so nothing is invented for it.
    await expect(row.nth(0).locator('.where')).toHaveCount(0);

    await expect(row.nth(1).locator('.when')).toHaveText('14:00–15:00');
    await expect(row.nth(1).locator('.where')).toHaveText('Room 4A');

    const zoom = page.locator('.sday').nth(1).locator('.srow').nth(1);
    await expect(zoom.locator('b')).toHaveText('Board prep');
    await expect(zoom.locator('.where')).toHaveText('Zoom');
  });

  test('an all-day event says so rather than showing a time it does not have', async ({ page }) => {
    // Spec §5, and the ordering rule beside it — Wednesday is the fixture's one
    // day holding both kinds.
    await page.goto(show('week'));
    const rows = page.locator('.sday').nth(1).locator('.srow');
    await expect(rows).toHaveCount(2);
    await expect(rows.nth(0).locator('.when')).toHaveText('All day');
    await expect(rows.nth(0).locator('b')).toHaveText('Rahul on leave');
    // Not "00:00" and not blank: the two failure shapes an "All day" label
    // exists to prevent.
    await expect(rows.nth(0).locator('.when')).not.toContainText(':');
  });

  test('the calendar\'s colour reaches the row, and is drawn with', async ({ page }) => {
    // Spec §5: the same `--cal` the grid's own chips use, so a recoloured
    // calendar is recoloured here for free. Asserted on the *computed* shadow
    // rather than on the inline custom property, because a `--cal` that is set
    // and never read renders a colourless list while passing the weaker check.
    // The spine is an inset box-shadow, not a `border-left` — see `.srow`'s
    // own comment for the WebKitGTK artifact a one-sided border paints — and
    // a regex rather than the full serialisation, because the engines order
    // the colour and the offsets differently.
    await page.goto(show('week'));
    const allDay = page.locator('.sday').nth(1).locator('.srow').nth(0);
    // `#e2a03f`, the fixture's own colour for that event.
    await expect(allDay).toHaveCSS('box-shadow', /rgb\(226, 160, 63\)/);
    // …and a different event on a different calendar is a different colour, so
    // this is reading the event rather than one hardcoded value.
    const timed = page.locator('.sday').nth(0).locator('.srow').nth(0);
    await expect(timed).toHaveCSS('box-shadow', /rgb\(91, 141, 239\)/);
  });

  test('clicking a row hands the occurrence up rather than opening anything itself', async ({ page }) => {
    // Spec §6: no second way to reach an event's detail. This component owns no
    // popover — `App` opens the one it already had, through `openOccurrence`.
    await page.goto(show('week'));
    await page.locator('.sday').nth(0).locator('.srow').nth(1).click();
    const opened = await page.evaluate(() => (window as any).__lastOpen);
    expect(opened.event).toMatchObject({ title: 'Ops review', start_ms: MON + 14 * 3_600_000 });
    // The anchor is the row's own box on screen, which is what puts the popover
    // beside what was clicked.
    expect(opened.rect.width).toBeGreaterThan(0);
    await expect(page.locator('.pop')).toHaveCount(0);
  });

  test('a month is grouped the same way, across its rows', async ({ page }) => {
    // The month payload reaches the same component through a different
    // grouping — bars and cells rather than a band and day columns — so the
    // rendering is checked through both. §8 of the testing standard.
    await page.goto(show('month'));
    const days = page.locator('.sday');
    await expect(days).toHaveCount(4);
    await expect(days.nth(2).locator('.sdate')).toHaveText('Wed, Aug 5');
    const rows = days.nth(2).locator('.srow');
    await expect(rows.nth(0).locator('.when')).toHaveText('All day');
    await expect(rows.nth(1).locator('.when')).toHaveText('15:00–16:00');
  });

  test('a period with nothing in it says so rather than rendering blank', async ({ page }) => {
    // Spec §3. Skipping empty days is exactly what makes an empty period
    // indistinguishable from a broken view without this line.
    await page.goto(show('empty'));
    await expect(page.locator('.sday')).toHaveCount(0);
    await expect(page.locator('.none')).toHaveText('Nothing scheduled.');
  });

  /** The current instant, drawn once, between the row that has started and
   *  the row still to come — and only in today's section. The other days
   *  carry no marker at all: "now" repeated down every section would orient
   *  nobody. */
  test('the now marker sits between past and future rows of today only', async ({ page }) => {
    await page.goto(show('week'));
    const monday = page.locator('.sday').nth(0);
    await expect(monday.locator('.nowrow')).toHaveCount(1);
    await expect(monday.locator('.nowrow .when')).toHaveText('13:00');
    // Between its neighbours, not merely present: previous row Standup
    // (ended 09:30), next row Ops review (14:00).
    const rows = monday.locator('ul > li');
    await expect(rows.nth(0).locator('b')).toHaveText('Standup');
    await expect(rows.nth(1)).toHaveClass(/nowrow/);
    await expect(rows.nth(2).locator('b')).toHaveText('Ops review');
    // Today only.
    await expect(page.locator('.nowrow')).toHaveCount(1);
  });

  /** The meta cluster, each piece on its own row with a plain row as the
   *  control: repeats-glyph for a series, an invitee count only when there is
   *  somebody besides the user to count, and nothing on an event carrying
   *  none of it. */
  test('a row says what repeats and who is invited, and only then', async ({ page }) => {
    await page.goto(show('meta'));
    const rows = page.locator('.srow');
    await expect(rows.nth(0).locator('.rep')).toBeVisible();       // Standup repeats
    await expect(rows.nth(0).locator('.who')).toHaveCount(0);
    await expect(rows.nth(1).locator('.who')).toHaveText('4');     // Mesh discussion
    await expect(rows.nth(1).locator('.rep')).toHaveCount(0);
    await expect(rows.nth(3).locator('.rep')).toHaveCount(0);      // Solo focus: nothing
    await expect(rows.nth(3).locator('.who')).toHaveCount(0);
  });

  /** The Join chip: only where there is a meeting, a sibling of the row
   *  rather than a control nested inside one, and backend-side by id — the
   *  command goes out with the event's id and the row does NOT also open.
   *  (The URL never travels; `commands::open_conference` re-derives it, for
   *  `open_latest_release`'s reason.) */
  test('Join goes to the backend by id and does not open the row', async ({ page }) => {
    await page.goto(show('meta'));
    const items = page.locator('.srow-li');
    await expect(items.nth(2).locator('.join')).toBeVisible();     // Daily Dev Sync
    await expect(items.nth(3).locator('.join')).toHaveCount(0);    // Solo focus

    await items.nth(2).locator('.join').click();
    const calls = await page.evaluate(() => (window as any).__harness.calls);
    const call = calls.find((c: any) => c.cmd === 'open_conference');
    expect(call.args).toMatchObject({ id: expect.any(Number) });
    expect(await page.evaluate(() => (window as any).__lastOpen)).toBeFalsy();

    // In the left cluster with everything else, not flushed to the far edge
    // of a 1280px row — the field note that reshaped this row applies to its
    // own control most of all.
    const joinBox = (await items.nth(2).locator('.join').boundingBox())!;
    expect(joinBox.x).toBeLessThan(400);
  });

  /** Chrome is not selectable text (app.css): a drag that misses a control
   *  used to sweep a selection across every row — WebKit's selection boxes
   *  read as faint borders wrapping each line. The popover keeps selection,
   *  because its locations and links are exactly what people copy. */
  test('a drag across the list selects nothing', async ({ page }) => {
    await page.goto(show('week'));
    const rows = page.locator('.srow');
    const a = (await rows.nth(0).boundingBox())!;
    const b = (await rows.nth(1).boundingBox())!;
    await page.mouse.move(a.x + 10, a.y + a.height / 2);
    await page.mouse.down();
    await page.mouse.move(b.x + b.width - 10, b.y + b.height / 2, { steps: 6 });
    await page.mouse.up();
    expect(await page.evaluate(() => String(window.getSelection()))).toBe('');
  });

  /** The forecast beside the day heading — the same map `WeekGrid`'s headers
   *  read, keyed the same way, so the list and the grid name one sky for one
   *  day. The `meta` fixture covers Monday only; the absence on a section
   *  past the horizon rides along free in every other fixture. */
  test('a day heading carries its forecast', async ({ page }) => {
    await page.goto(show('meta'));
    const wx = page.locator('.sdate .wx');
    await expect(wx).toHaveCount(1);
    await expect(wx).toHaveText('31°');
    await page.goto(show('week'));
    await expect(page.locator('.sdate .wx')).toHaveCount(0);
  });

  /** The regression the redesign exists to fix: the location sits beside the
   *  title, not flushed to the far edge of a 1280px row. Asserted as a bound
   *  rather than a pixel — the old layout put Room 4A past x=900, the new one
   *  keeps it inside the row's first stretch. */
  test('the location stays next to the title', async ({ page }) => {
    await page.goto(show('meta'));
    const where = page.locator('.srow', { hasText: 'Mesh discussion' }).locator('.where');
    const box = (await where.boundingBox())!;
    expect(box.x).toBeLessThan(400);
  });
});

test.describe('EventForm', () => {
  /**
   * Navigate with the clock frozen, every time.
   *
   * Not optional and not per-spec: the `create` fixture is built from
   * `Date.now()` inside the page (see fixtures.ts), because the "next half
   * hour" default is the thing under test and a fixture that pinned the
   * instant itself could not tell a form that applies the default from one
   * that was handed the answer. Freezing it here means no spec in this block
   * can forget, and none of them rots into a failure on a future date.
   */
  const open = async (page: import('@playwright/test').Page, fixture: string) => {
    await page.clock.setFixedTime(FORM_NOW);
    await page.goto(`/tests/harness/index.html?c=EventForm&f=${fixture}`);
    await expect(page.locator('.pop')).toBeVisible();
  };

  /** Everything the form handed `onsave`, in order — `[]` when it refused.
   *  An array, not a slot: half of what these specs assert is that nothing
   *  was saved at all. */
  const saves = (page: import('@playwright/test').Page) =>
    page.evaluate(() => (window as any).__saves as any[]);

  test('the typed times echo in the second zone when settings name one', async ({ page }) => {
    await open(page, 'create');
    // Absent while the setting is off — the form has never had this line,
    // and without a second zone it still does not.
    await expect(page.getByTestId('second-zone')).toHaveCount(0);

    await page.evaluate(() => (window as any).__setSecondZone('Asia/Kolkata'));
    await page.getByLabel('Start', { exact: true }).fill('09:00');
    await page.getByLabel('End', { exact: true }).fill('10:30');

    // UTC 09:00–10:30 on Kolkata's clock, live as the times are typed —
    // the same keystroke-commit the grid ghost follows. The zone's short
    // name varies by ICU, so it is matched as a family.
    const line = page.getByTestId('second-zone');
    await expect(line).toContainText('14:30–16:00');
    await expect(line).toContainText(/GMT\+5:30|IST/);

    // All-day silences it: a day is a day in every zone, and printing
    // "05:30–05:30" under a trip would be the line lying with digits.
    await page.getByText('All day').click();
    await expect(page.getByTestId('second-zone')).toHaveCount(0);
  });

  test('typing a fragment offers people from meeting history', async ({ page }) => {
    // "Right now I have to write them from memory or find them in mail"
    // (2026-08-23): the corpus is the store's own attendee history — the
    // harness stub plays that role — matched on name or address, exactly the
    // timezone picker's rule. Clicking a row adds the guest whole.
    await open(page, 'create');
    await page.getByLabel('Add guest').fill('isk');
    const list = page.getByRole('listbox', { name: 'People you have met with' });
    await expect(list).toBeVisible();
    await list.getByRole('option', { name: /Iskren Hadzhinedev/ }).click();
    await expect(page.locator('.guests')).toContainText('iskren.h@x3me.net');
    await expect(page.getByLabel('Add guest')).toHaveValue('');
    await expect(list).toHaveCount(0);

    // Someone already aboard is not offered again.
    await page.getByLabel('Add guest').fill('isk');
    await expect(list).toHaveCount(0);
  });

  test('the list answers to the keyboard, and Escape closes it, not the form', async ({ page }) => {
    await open(page, 'create');
    const input = page.getByLabel('Add guest');
    await input.fill('x3me');
    const list = page.getByRole('listbox', { name: 'People you have met with' });
    await expect(list).toBeVisible();

    // Escape with the list open is the list's alone — the form survives.
    await input.press('Escape');
    await expect(list).toHaveCount(0);
    await expect(page.locator('.pop')).toBeVisible();

    // Typing again re-opens; arrows walk it; Return takes the second row.
    await input.press('Backspace');
    await expect(list).toBeVisible();
    await input.press('ArrowDown');
    await input.press('ArrowDown');
    await input.press('Enter');
    await expect(page.locator('.guests')).toContainText('eva.m@x3me.net');
    // And nothing was saved by that Return — the form is still open.
    expect(await saves(page)).toEqual([]);
  });

  /** The time fields speak the app's clock, not the engine's: they are
   *  text now (the native input rendered the system locale's AM/PM over a
   *  24h grid), rendered through `displayClock` and parsed through
   *  `parseClock` — so a dial entry lands as storage, an unparseable one
   *  snaps back to the last real time, and either clock is accepted on the
   *  way in. The 12h rendering itself is pinned unit-side (timefmt.spec). */
  test('a time typed in either clock lands, and a typo snaps back', async ({ page }) => {
    await open(page, 'create');
    const start = page.getByLabel('Start', { exact: true });

    // `blur()` after each fill: commit rides `change`, which fires on
    // leaving the field — exactly what clicking Save does in real use.
    await start.fill('9:30 pm');
    await start.blur();
    await expect(start).toHaveValue('21:30');

    await start.fill('half past nine');
    await start.blur();
    await expect(start).toHaveValue('21:30');
  });

  test('only writable calendars are offered', async ({ page }) => {
    // A subscribed holiday calendar is a `reader` and a room is a
    // `freeBusyReader`; `create_impl` refuses both server-side, so offering
    // either produces a Save that can only fail. Two unwritable roles, not
    // one, so a filter written as "anything but reader" is caught too.
    await open(page, 'create');
    const select = page.getByLabel('Calendar', { exact: true });
    await expect(select.locator('option')).toHaveCount(2);
    await expect(select.locator('option')).toHaveText(['Personal', 'Team']);
    for (const name of FORM_UNWRITABLE_NAMES) {
      await expect(select.locator('option').filter({ hasText: name })).toHaveCount(0);
    }
  });

  test('a create seeded with a calendar it cannot write to falls back to one it can', async ({ page }) => {
    // Filtering the option list is not filtering the value. Seeded with the
    // reader's id, the select rendered *blank* — no option matches — and Save
    // then sent that id with nothing on screen to say so. Task 10 chooses this
    // seed, so the shape is reachable from the next task rather than theoretical.
    await open(page, 'create-seeded-unwritable');
    const select = page.getByLabel('Calendar', { exact: true });
    await expect(select).not.toHaveValue('');
    await expect(select).toHaveValue(String(FORM_FALLBACK_ID));

    await page.getByRole('button', { name: 'Create' }).click();
    const [saved] = await saves(page);
    // What is shown and what is saved have to agree; that is the property that
    // was broken, so both halves are asserted.
    expect(saved.calendarId).toBe(FORM_FALLBACK_ID);
    expect(saved.calendarId).not.toBe(FORM_UNWRITABLE_ID);
  });

  test('the calendar can be chosen on a create and not on an edit', async ({ page }) => {
    // `update_event` takes no calendar id — it reads the target from
    // `event_for_write(id)` — so an enabled control on an edit silently
    // discards the choice. Both arms in one spec: `disabled={true}` always
    // would pass the edit half on its own.
    await open(page, 'create');
    await expect(page.getByLabel('Calendar', { exact: true })).toBeEnabled();

    await open(page, 'with-guests');
    await expect(page.getByLabel('Calendar', { exact: true })).toBeDisabled();
  });

  test('moving the start date takes the end date with it', async ({ page }) => {
    // Otherwise changing the date of an ordinary one-hour meeting leaves the
    // end date on the old day and Save refuses a range the user never asked
    // for. Asserted through to the saved instants, not just the input: the
    // point is that the save is *accepted* and lands on the new day.
    await open(page, 'create');
    await page.getByLabel('Date', { exact: true }).fill('2026-08-12');
    await expect(page.getByLabel('End date', { exact: true })).toHaveValue('2026-08-12');

    await page.getByRole('button', { name: 'Create' }).click();
    const [saved] = await saves(page);
    // 09:30 on 12 Aug 2026, UTC — the project's `timezoneId`.
    expect(saved.fields.when.kind).toBe('timed');
    expect(saved.fields.when.startMs).toBe(Date.UTC(2026, 7, 12, 9, 30));
    expect(saved.fields.when.endMs).toBe(Date.UTC(2026, 7, 12, 10, 30));
  });

  test('save is refused when the end is before the start', async ({ page }) => {
    // Refused, not corrected. Silently swapping the ends would save something
    // nobody asked for, and on an event with guests mail it to all of them.
    await open(page, 'end-before-start');
    await page.getByRole('button', { name: 'Create' }).click();
    // Nothing saved, asserted first: that is the safety property, and it is
    // the one whose failure names the actual defect. Telling the user why
    // matters too, but a form that saved a backwards event and then apologised
    // would still have saved it.
    expect(await saves(page)).toEqual([]);
    await expect(page.getByTestId('form-error')).toBeVisible();
  });

  /**
   * §7.3, driven through the real inputs.
   *
   * `eventform.spec.ts` proves `timeProblem` answers; this proves the **form
   * asks it and shows the answer**, which is a different claim and the one the
   * defect was about. A spec pointed only at the function stays green if the
   * save handler stops calling it — the same trap §7.2 was caught by.
   */
  test.describe('a time typed into an hour that does not exist', () => {
    // Santiago's clocks go forward 00:00 -> 01:00 on 6 Sep 2026, so that day
    // has no 00:30. **The browser's zone**: a skipped hour is a property of the
    // clock being typed into, and a fixture in a zone with no transition that
    // day would pass against the old code.
    test.use({ timezoneId: 'America/Santiago' });

    test('save is refused, the reason is shown, and the field is marked', async ({ page }) => {
      await open(page, 'create');

      await page.getByLabel('Date', { exact: true }).fill('2026-09-06');
      await page.getByLabel('Start', { exact: true }).fill('00:30');
      await page.getByRole('button', { name: 'Create' }).click();

      // The safety property first: nothing was written. A form that saved an
      // event an hour from where it was typed and then explained itself would
      // still have saved it.
      expect(await saves(page)).toEqual([]);

      const err = page.getByTestId('form-error');
      await expect(err).toBeVisible();
      // The time, the date, and the reason. Without the reason this names a
      // fact about the user's calendar that reads like a bug in the app.
      await expect(err).toContainText('00:30');
      await expect(err).toContainText('2026-09-06');
      await expect(err).toContainText('clocks go forward');

      // Marked, not only described. Nothing else on this form looks wrong, so
      // pointing at the field is most of the help.
      await expect(page.getByLabel('Start', { exact: true })).toHaveAttribute('aria-invalid', 'true');
      await expect(page.getByLabel('End', { exact: true })).not.toHaveAttribute('aria-invalid', 'true');
    });

    test('a real time on the same day saves', async ({ page }) => {
      // The control, and it is not decoration: every assertion above is
      // satisfied by a form that refuses everything on a transition date.
      await open(page, 'create');

      await page.getByLabel('Date', { exact: true }).fill('2026-09-06');
      await page.getByLabel('Start', { exact: true }).fill('01:30');
      await page.getByLabel('End', { exact: true }).fill('02:00');
      await page.getByRole('button', { name: 'Create' }).click();

      expect(await saves(page)).toHaveLength(1);
      await expect(page.getByTestId('form-error')).toBeHidden();
    });
  });

  test('an unrepresentable repeat rule is shown as a disabled Custom option', async ({ page }) => {
    // Spec §6's UI half. `write::repeat_from_rrule` answered `custom` for this
    // fortnightly rule, and the form's job is to show what it cannot rewrite
    // rather than quietly present it as something it can.
    await open(page, 'custom-repeat');
    const select = page.getByLabel('Repeat', { exact: true });
    await expect(select).toHaveValue('custom');

    const custom = select.locator('option[value="custom"]');
    // The *entry* is disabled; the select is not. Disabling the select would
    // make the rule unchangeable rather than un-clobberable, and the whole
    // design is that replacing it stays possible as an explicit act.
    //
    // Asserted through the DOM property rather than `toBeDisabled()`, which
    // resolves disabledness through the ARIA state and reports an `<option>`
    // carrying a real `disabled` attribute as enabled. The property is what
    // actually makes the entry unselectable, so it is what is worth asserting.
    expect(await custom.evaluate((el) => (el as HTMLOptionElement).disabled)).toBe(true);
    await expect(select.locator('option:not([disabled])')).toHaveCount(6);
    await expect(select).toBeEnabled();
    // In words, not as the raw rule: `RRULE:FREQ=WEEKLY;INTERVAL=2` is not
    // something to ask a user to read before deciding whether to replace it.
    await expect(custom).toHaveText('Custom · Every 2 weeks');
  });

  test('Weekly exposes SMTWRFS buttons and saves the pushed day pattern', async ({ page }) => {
    await open(page, 'create');
    await page.getByLabel('Repeat', { exact: true }).selectOption('weekly');

    const days = page.getByRole('group', { name: 'Repeat on' });
    await expect(days.getByRole('button')).toHaveCount(7);
    await expect(days.getByRole('button', { name: 'Wednesday' })).toHaveAttribute('aria-pressed', 'true');
    await days.getByRole('button', { name: 'Monday' }).click();
    await days.getByRole('button', { name: 'Friday' }).click();

    await page.getByRole('button', { name: 'Create' }).click();
    const [saved] = await saves(page);
    expect(saved.fields.repeat).toBe('weekly');
    expect(saved.fields.weeklyDays).toEqual(['MO', 'WE', 'FR']);
  });

  test('a repeating event can end on a date or after a number of occurrences', async ({ page }) => {
    await open(page, 'create');
    await page.getByLabel('Repeat', { exact: true }).selectOption('weekly');

    const ends = page.getByLabel('Repeat ends');
    await expect(ends).toHaveValue('never');
    await ends.selectOption('on');
    await page.getByLabel('Repeat end date').fill('2026-09-30');
    await page.getByRole('button', { name: 'Create' }).click();
    let [saved] = await saves(page);
    expect(saved.fields.repeatEnd).toEqual({ kind: 'on', date: '2026-09-30' });

    await open(page, 'create');
    await page.getByLabel('Repeat', { exact: true }).selectOption('daily');
    await page.getByLabel('Repeat ends').selectOption('after');
    await page.getByLabel('Number of occurrences').fill('12');
    await page.getByRole('button', { name: 'Create' }).click();
    [saved] = (await saves(page)).slice(-1);
    expect(saved.fields.repeatEnd).toEqual({ kind: 'after', count: 12 });
  });

  test('an invalid repeat ending is named and never saved', async ({ page }) => {
    await open(page, 'create');
    await page.getByLabel('Repeat', { exact: true }).selectOption('daily');
    await page.getByLabel('Repeat ends').selectOption('after');
    await page.getByLabel('Number of occurrences').fill('0');
    await page.getByRole('button', { name: 'Create' }).click();
    await expect(page.getByTestId('form-error')).toContainText('at least 1 occurrence');
    expect(await saves(page)).toEqual([]);
  });

  test('an event with guests says the choice is on the buttons, not on Save', async ({ page }) => {
    // **This replaces "Saving will notify 4 guests."** That sentence was true
    // when `update_event` was handed `all` unconditionally; spec §3 makes it a
    // choice, so a warning that states an outcome would now be describing one
    // of the two buttons. The count still excludes the person doing the saving
    // — the fixture has five attendees for exactly that reason.
    await open(page, 'with-guests');
    await expect(page.getByTestId('guest-notice')).toContainText('4 guests');
  });

  test('a description is rendered as text, never as markup', async ({ page }) => {
    // Anyone who knows the user's email can put an event on their calendar,
    // description included, and this webview can invoke Tauri commands.
    await open(page, 'nasty-description');
    await expect(page.locator('img')).toHaveCount(0);
    // And byte for byte: sanitising on the way *in* would rewrite what the
    // author typed and then save the rewrite back over the real event —
    // `stripTags` alone would leave this field empty.
    await expect(page.getByLabel('Description', { exact: true }))
      .toHaveValue('<img src=x onerror=alert(1)>');
  });

  test('a new event opens at the next half hour', async ({ page }) => {
    // 09:12 frozen, so 09:30 is a rounding rather than an echo of the clock.
    await open(page, 'create');
    await expect(page.getByLabel('Date', { exact: true })).toHaveValue('2026-08-05');
    await expect(page.getByLabel('Start', { exact: true })).toHaveValue('09:30');
    await expect(page.getByLabel('End', { exact: true })).toHaveValue('10:30');
  });

  test('a recurring edit offers three scopes and says what All events does', async ({ page }) => {
    // "All events" on a time change shifts the whole series rather than
    // pinning every occurrence to the edited date — deliberate (the
    // alternative drops occurrences before the clicked one) and impossible to
    // infer from three radio labels.
    await open(page, 'recurring-edit');
    await expect(page.getByRole('radio')).toHaveCount(3);
    await expect(page.getByRole('radio', { name: 'This event' })).toBeChecked();
    await expect(page.getByTestId('all-events-note')).toHaveCount(0);

    await page.getByRole('radio', { name: 'All events' }).check();
    await expect(page.getByTestId('all-events-note')).toContainText('every occurrence an hour later');
  });

  test('a one-off event offers no scope choice', async ({ page }) => {
    // Without this the scope spec above passes on a form that always shows
    // three radios, whatever it was given.
    await open(page, 'with-guests');
    await expect(page.getByRole('radio')).toHaveCount(0);
  });

  test('a multi-day all-day event keeps its last day, and saves it back unchanged', async ({ page }) => {
    // Google's `end.date` is exclusive and so is the store's `end_ms`: a
    // three-day trip starting Mon 10 Aug ends at midnight on Thu 13th. Showing
    // that date reads a day long; sending back the date shown shortens the trip
    // by a day and mails everyone about it. Both ends are asserted, because
    // converting on only one side is the failure that looks right on screen.
    await open(page, 'multi-day-all-day');
    await expect(page.getByLabel('First day', { exact: true })).toHaveValue(TRIP_FIRST_DAY);
    await expect(page.getByLabel('Last day', { exact: true })).toHaveValue(TRIP_LAST_DAY);

    await page.getByRole('button', { name: 'Save' }).click();
    const [saved] = await saves(page);
    expect(saved.fields.when.kind).toBe('allDay');
    // Still the exclusive end, now named as the date it always was. Playwright
    // pins `timezoneId: 'UTC'`, so this is the same assertion in a different
    // unit — which is exactly why the zone-crossing version of it belongs in a
    // describe of its own, and does not exist yet.
    expect(saved.fields.when.endDate).toBe(TRIP_END_DATE);
  });

  test('saving without touching Repeat sends no rule at all', async ({ page }) => {
    // The property the whole `custom` design rests on: an absent `repeat` means
    // "the user did not touch Repeat", and the existing rule is left alone.
    // Sending `custom` — or anything else — would rewrite a fortnightly meeting
    // as something omacal can express, for the whole guest list.
    await open(page, 'custom-repeat');
    await page.getByRole('button', { name: 'Save' }).click();
    const [saved] = await saves(page);
    expect(Object.keys(saved.fields)).not.toContain('repeat');
  });

  test('choosing another repeat option sends the overwrite explicitly', async ({ page }) => {
    // The other half: leaving an untouched rule alone must not turn into never
    // being able to change one.
    await open(page, 'custom-repeat');
    await page.getByLabel('Repeat', { exact: true }).selectOption('weekly');
    await page.getByRole('button', { name: 'Save' }).click();
    const [saved] = await saves(page);
    expect(saved.fields.repeat).toBe('weekly');
    expect(saved.fields.weeklyDays).toEqual(['WE']);
  });

  /**
   * **A closed panel stops listening — and this is the only spec that can say
   * so.**
   *
   * `escapeCloses` puts the handler on `window`, which it must: focus does not
   * stay put, and nothing short of `window` hears Escape from `<body>`. The
   * price is that the component now owns removing it, where five
   * `<svelte:window>` elements used to have Svelte own it — unprovable but
   * unbreakable. This is what buys that guarantee back.
   *
   * The leak is invisible while the panel is *closed*, because every call
   * site's guard reads false. It is visible when a panel is destroyed while
   * **open**: the closure still captures the state it had, the guard still
   * passes, and the parent is told to close a component that no longer exists.
   *
   * That shape has already cost this project a real defect one layer over — a
   * drag whose `window` handlers outlived their grid sent a write to Google
   * from a view the user had left. Counted rather than flagged, because a
   * boolean cannot tell one call from two.
   */
  test('a form destroyed by Escape does not answer the next one', async ({ page }) => {
    await open(page, 'create');
    const cancels = () => page.evaluate(() => (window as any).__cancels as number);

    await page.keyboard.press('Escape');
    await expect(page.locator('.pop')).toHaveCount(0);
    expect(await cancels(), 'the first Escape closes it').toBe(1);

    // The form is gone. A listener that outlived it would answer this one too.
    await page.keyboard.press('Escape');
    expect(await cancels(), 'a destroyed form must not still be listening').toBe(1);
  });

  // --- The guest list (spec §1, §4, §5) ------------------------------------

  /** The rows currently on the form, by address. */
  const guestRows = (page: import('@playwright/test').Page) =>
    page.locator('[data-guest]');

  /** Types `address` into the add field and presses Add. */
  const addGuest = async (page: import('@playwright/test').Page, address: string) => {
    await page.getByLabel('Add guest', { exact: true }).fill(address);
    await page.getByRole('button', { name: 'Add', exact: true }).click();
  };

  /** Answers the notify choice: presses the form's own action, then the
   *  panel's. Both names, because the two differ on a create — the form says
   *  `Create` and so does the panel under it (see `SaveConfirm`'s `verb`). */
  const answerNotify = async (
    page: import('@playwright/test').Page,
    button: string,
    action = 'Save',
  ) => {
    await page.getByRole('button', { name: action, exact: true }).click();
    await page.getByRole('button', { name: button, exact: true }).click();
  };

  test('the guest list shows everyone on the event, the user included', async ({ page }) => {
    // Five rows, not four. The *notice* excludes the person doing the saving
    // because it counts who gets an email; the list is what the event's
    // attendees would become, and leaving yourself out of that would take you
    // off the event on the next save.
    await open(page, 'with-guests');
    await expect(guestRows(page)).toHaveCount(5);
    await expect(page.locator('[data-guest="me@x.com"]')).toBeVisible();
  });

  /**
   * **The guest editor is on the create path.**
   *
   * It was gated behind `initial.isEdit` for as long as `create_impl` refused
   * a create carrying guests — a form offering what the write path refuses is
   * a form that can only disappoint. Both are gone; this is the witness that
   * the *form* half actually went.
   */
  test('a create can invite somebody', async ({ page }) => {
    await open(page, 'create-guests');
    await expect(page.getByTestId('guests')).toBeVisible();

    await addGuest(page, 'ana@x.com');
    await expect(guestRows(page)).toHaveCount(1);

    await answerNotify(page, 'Create without notifying', 'Create');

    const [saved] = await saves(page);
    expect(saved.fields.guests).toEqual([{ email: 'ana@x.com', optional: false }]);
  });

  /** A create with nobody on it must not grow a dialog. Nobody to tell means
   *  nothing to choose between, and the save goes straight out — the same
   *  shortcut an edit takes, now reached through `mailableGuests`. */
  test('a create with no guests still saves without asking', async ({ page }) => {
    await open(page, 'create-guests');
    await page.getByLabel('Title', { exact: true }).fill('Lunch');
    await page.getByRole('button', { name: 'Create', exact: true }).click();

    const [saved] = await saves(page);
    expect(saved.notify).toBe('none');
    expect(saved.fields.guests).toBeUndefined();
  });

  /** Both answers, because a panel wired to one constant passes either half
   *  alone — and `all` is the only one that mails anybody. */
  test('a create asks before it notifies, and carries the answer', async ({ page }) => {
    for (const [button, expected] of [
      ['Create without notifying', 'none'],
      ['Create and notify guests', 'all'],
    ] as const) {
      await open(page, 'create-guests');
      await addGuest(page, 'ana@x.com');
      await answerNotify(page, button, 'Create');

      const [saved] = await saves(page);
      expect(saved.notify, button).toBe(expected);
    }
  });

  /** The panel says what the button under it will do. "Save" on a form whose
   *  own action reads "Create" is a small lie in the one dialog whose entire
   *  job is to be unambiguous about mailing other people. Both arms, because
   *  a `verb` hardcoded either way passes one of them. */
  test('the notify panel says Create on a create and Save on an edit', async ({ page }) => {
    await open(page, 'create-guests');
    await addGuest(page, 'ana@x.com');
    await page.getByRole('button', { name: 'Create', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'Create event' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Create without notifying' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Create and notify guests' })).toBeVisible();

    await open(page, 'with-guests');
    await page.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'Save event' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Save without notifying' })).toBeVisible();
  });

  test('an address typed in is added, and reaches the save', async ({ page }) => {
    await open(page, 'with-guests');
    await addGuest(page, 'dan@x.com');
    await expect(guestRows(page)).toHaveCount(6);

    await answerNotify(page, 'Save without notifying');
    const [saved] = await saves(page);
    expect(saved.fields.guests.map((g: any) => g.email)).toContain('dan@x.com');
    // And everyone who was already there is still there: `attendees` is a
    // whole-list replace, so a payload carrying only the new person removes
    // the rest.
    expect(saved.fields.guests).toHaveLength(6);
  });

  test('Return in the address field adds a guest rather than saving', async ({ page }) => {
    // The field lives inside the `<form>`, so an unhandled Return submits it —
    // saving an event with a half-typed guest list and, on an event with
    // guests, opening the notify choice for a change the user had not finished
    // making.
    await open(page, 'with-guests');
    await page.getByLabel('Add guest', { exact: true }).fill('dan@x.com');
    await page.getByLabel('Add guest', { exact: true }).press('Enter');

    await expect(guestRows(page)).toHaveCount(6);
    expect(await saves(page), 'Return must not have saved anything').toEqual([]);
  });

  /**
   * §5: **refused in the form, before Save** — never by a 400 from Google,
   * which arrives after the user has stopped looking.
   */
  test('an address that is not one is refused, and adds no row', async ({ page }) => {
    await open(page, 'with-guests');
    await addGuest(page, 'not-an-address');

    await expect(page.getByTestId('form-error')).toBeVisible();
    await expect(guestRows(page)).toHaveCount(5);
    // Marked as well as described, the same rule the time fields follow: the
    // message says what is wrong and the field says which one.
    await expect(page.getByLabel('Add guest', { exact: true }))
      .toHaveAttribute('aria-invalid', 'true');
  });

  /** §5: a duplicate is a no-op — not an error, and not a second row. */
  test('an address already invited adds no second row and no error', async ({ page }) => {
    await open(page, 'with-guests');
    await addGuest(page, 'ANA@x.com');

    await expect(guestRows(page)).toHaveCount(5);
    await expect(page.getByTestId('form-error')).toHaveCount(0);
  });

  /** §5: **the organizer cannot be removed.** Google refuses it, so the control
   *  is absent rather than present and disappointing. */
  test('the organizer has no remove control, and everyone else does', async ({ page }) => {
    await open(page, 'with-guests');
    await expect(
      page.locator('[data-guest="ana@x.com"]').getByRole('button'),
      'Ana is the organizer',
    ).toHaveCount(0);
    await expect(page.locator('[data-guest="petya@x.com"]').getByRole('button')).toHaveCount(1);
  });

  /**
   * §5: **removing yourself is not declining, and must not look like an RSVP.**
   * Two things say so — the control names what it does rather than saying
   * "remove", and the form says what it is not.
   */
  test('removing yourself is offered, and does not read as an RSVP', async ({ page }) => {
    await open(page, 'with-guests');
    await expect(
      page.getByRole('button', { name: 'Remove yourself from this event' }),
    ).toBeVisible();
    await expect(page.getByTestId('self-guest-hint')).toContainText('not the same as declining');
  });

  test('removing a guest sends the list without them', async ({ page }) => {
    await open(page, 'with-guests');
    await page.getByRole('button', { name: 'Remove petya@x.com' }).click();
    await expect(guestRows(page)).toHaveCount(4);

    await answerNotify(page, 'Save without notifying');
    const [saved] = await saves(page);
    expect(saved.fields.guests.map((g: any) => g.email)).not.toContain('petya@x.com');
    expect(saved.fields.guests).toHaveLength(4);
  });

  /** §4: the optional flag rides on the same whole-list replace, so this is a
   *  toggle like any other — driven **both ways**, because a control wired to
   *  set `true` passes a one-directional test. */
  test('a guest can be marked optional, and unmarked', async ({ page }) => {
    await open(page, 'with-guests');
    const petya = page.getByLabel('Optional: petya@x.com');
    const ivan = page.getByLabel('Optional: ivan@x.com');
    await expect(ivan, 'the fixture ships one already optional').toBeChecked();

    await petya.check();
    await ivan.uncheck();

    await answerNotify(page, 'Save without notifying');
    const [saved] = await saves(page);
    const by = (e: string) => saved.fields.guests.find((g: any) => g.email === e);
    expect(by('petya@x.com').optional).toBe(true);
    expect(by('ivan@x.com').optional).toBe(false);
  });

  test('a save that left the guest list alone sends no guests at all', async ({ page }) => {
    // Absent means "leave the list alone", which is the only safe instruction
    // for a whole-list replace built from a possibly-stale read.
    await open(page, 'with-guests');
    await answerNotify(page, 'Save without notifying');
    const [saved] = await saves(page);
    expect(Object.keys(saved.fields)).not.toContain('guests');
  });

  // --- The notify choice on Save (spec §3) ---------------------------------

  /**
   * **The don't-notify path, witnessed rather than assumed** — asserted on what
   * the write is asked to send, exactly as the drag specs do it.
   *
   * `sendUpdates=all` used to be unconditional here. Correcting a typo in an
   * address should not mail the whole room, and mail to other people is a
   * deliberate act rather than a consequence of pressing Save.
   */
  test('Save without notifying sends none', async ({ page }) => {
    await open(page, 'with-guests');
    await answerNotify(page, 'Save without notifying');

    const [saved] = await saves(page);
    expect(saved.notify).toBe('none');
  });

  test('Save and notify guests sends all', async ({ page }) => {
    // The other half, and the only path to `all` from this form. Without it
    // "never notify" would satisfy the spec above.
    await open(page, 'with-guests');
    await answerNotify(page, 'Save and notify guests');

    const [saved] = await saves(page);
    expect(saved.notify).toBe('all');
  });

  test('Save on an event with guests saves nothing until the choice is made', async ({ page }) => {
    // The panel must *gate* the save rather than merely appear. Witnessed by
    // the absence of a save while it is open — a dialog that is visible and
    // does not gate is the failure this shape is aimed at.
    await open(page, 'with-guests');
    await page.getByRole('button', { name: 'Save' }).click();

    await expect(page.getByRole('dialog', { name: 'Save event' })).toBeVisible();
    expect(await saves(page)).toEqual([]);
  });

  test('cancelling the choice saves nothing and leaves the form open', async ({ page }) => {
    await open(page, 'with-guests');
    await page.getByRole('button', { name: 'Save' }).click();
    await page.getByRole('dialog', { name: 'Save event' })
      .getByRole('button', { name: 'Cancel' }).click();

    expect(await saves(page)).toEqual([]);
    await expect(page.getByRole('dialog', { name: 'Edit event' })).toBeVisible();
  });

  /**
   * Escape closes the choice and **not the form behind it.** Both listen on
   * `window` — they have to, for the reason each says — so without a guard one
   * keystroke dismisses both and the user loses everything they typed.
   */
  test('Escape on the choice returns to the form rather than closing it', async ({ page }) => {
    await open(page, 'with-guests');
    await page.getByRole('button', { name: 'Save' }).click();
    await expect(page.getByRole('dialog', { name: 'Save event' })).toBeVisible();

    await page.keyboard.press('Escape');

    await expect(page.getByRole('dialog', { name: 'Save event' })).toHaveCount(0);
    await expect(page.getByRole('dialog', { name: 'Edit event' })).toBeVisible();
    expect(await saves(page)).toEqual([]);
  });

  test('an event with nobody on it saves straight away, and notifies nobody', async ({ page }) => {
    // No guests, so there is nothing to choose between: asking would be a
    // dialog with one answer. `none` rather than `all`, so that `all` appears
    // exactly where somebody chose it — the same rule the drag path follows.
    await open(page, 'custom-repeat');
    await page.getByRole('button', { name: 'Save' }).click();

    await expect(page.getByRole('dialog', { name: 'Save event' })).toHaveCount(0);
    const [saved] = await saves(page);
    expect(saved.notify).toBe('none');
  });
});

test.describe('DeleteConfirm', () => {
  const show = (f: string) => `/tests/harness/index.html?c=DeleteConfirm&f=${f}`;

  const open = async (page: import('@playwright/test').Page, fixture: string) => {
    await page.goto(show(fixture));
    await expect(page.locator('.pop')).toBeVisible();
  };

  /** Every scope the panel handed `onconfirm`, in order — `[]` when it asked
   *  and nothing was confirmed. An array, not a slot, for the reason
   *  `EventForm`'s `__saves` is one: half of what these assert is that nothing
   *  happened at all. */
  const confirms = (page: import('@playwright/test').Page) =>
    page.evaluate(() => (window as any).__confirms as any[]);

  test('names the event, says who gets emailed, and warns there is no undo', async ({ page }) => {
    // Three things a confirmation with no undo behind it has to be honest
    // about, and one it must not be: the fixture has three attendees, one of
    // them the signed-in user, so the count is two. Telling somebody they are
    // about to email themselves is just wrong.
    await open(page, 'one-off');
    await expect(page.locator('h2')).toContainText('Board prep');
    await expect(page.getByTestId('delete-guest-notice')).toContainText('2 guests are told by email');
    await expect(page.getByTestId('delete-no-undo')).toContainText('cannot be undone');
    // A one-off has one deletion, so naming it three ways would be three
    // different words for the same act.
    await expect(page.getByRole('radio')).toHaveCount(0);
    expect(await confirms(page)).toEqual([]);
  });

  test('the three scopes are three different operations, and each says which', async ({ page }) => {
    // Not three sizes of one deletion. "This and following" deletes nothing at
    // all — it patches the series' rule so it stops earlier, which is the only
    // way to lose the tail without also losing the occurrences before the
    // clicked one, since they are all the same Google event. "All events" takes
    // the past with it. Neither is inferable from a three-item radio list.
    await open(page, 'recurring');
    const scopes = page.locator('.scope label');
    await expect(scopes).toHaveCount(3);
    await expect(scopes.nth(1)).toContainText('deletes nothing');
    await expect(scopes.nth(1)).toContainText('shortens the series');
    await expect(scopes.nth(2)).toContainText('already happened');
    // Every scope notifies: `sendUpdates=all` is unconditional on the DELETE
    // and on the "this and following" PATCH alike, so the notice may not read
    // as if it applied to only one of the three.
    await expect(page.getByTestId('delete-guest-notice')).toContainText('Whichever you choose');

    // And the chosen scope is the one that comes back — a panel that always
    // confirmed `'this'` would satisfy every assertion above.
    await page.getByRole('radio', { name: 'This and following' }).check();
    await page.getByRole('button', { name: 'Delete' }).click();
    expect(await confirms(page)).toEqual(['following']);
  });

  // Each radio bound to the scope it actually sends, one spec per option, so
  // that no option can be silently rewired to another. The two that are not
  // the default matter most and differ most: "All events" removes a whole
  // series *including its past*, "This and following" removes nothing at all
  // and merely shortens the rule. Wiring the first to the second leaves the
  // panel reading exactly right and is a different, irreversible act — with
  // mail going out either way. Only an assertion per option catches it.
  for (const [label, scope] of [
    ['This event', 'this'],
    ['This and following', 'following'],
    ['All events', 'all'],
  ] as const) {
    test(`"${label}" sends the scope ${scope}`, async ({ page }) => {
      await open(page, 'recurring');
      await page.getByRole('radio', { name: label }).check();
      await page.getByRole('button', { name: 'Delete' }).click();
      expect(await confirms(page)).toEqual([scope]);
    });
  }

  test('an event with nobody on it claims nothing about guests', async ({ page }) => {
    // "0 guests are told by email" is both untrue and alarming. The no-undo
    // line stays either way: that one is about the event, not the guest list.
    await open(page, 'no-guests');
    await expect(page.getByTestId('delete-guest-notice')).toHaveCount(0);
    await expect(page.getByTestId('delete-no-undo')).toBeVisible();
  });
});

test.describe('Header invitation tray', () => {
  /** The tray exists because a notification toast can be missed (it was,
   *  live, 2026-08-17) — so the header itself must carry the debt: a badge
   *  while invitations await an answer, a list with the answer buttons
   *  behind it, and nothing at all at inbox-zero. */

  test('inbox-zero renders no badge at all', async ({ page }) => {
    await page.goto(show('Header', 'connected'));
    await expect(page.getByRole('button', { name: 'Menu' })).toBeVisible();
    await expect(page.getByRole('button', { name: /pending invitation/ })).toHaveCount(0);
  });

  test('the badge counts, and the tray lists both kinds of row', async ({ page }) => {
    await page.goto(show('Header', 'with-invites'));
    const badge = page.getByRole('button', { name: '2 pending invitations' });
    await expect(badge).toBeVisible();
    await badge.click();

    const rows = page.getByTestId('invite-row');
    await expect(rows).toHaveCount(2);
    await expect(rows.nth(0)).toContainText('NVP sync meeting');
    await expect(rows.nth(0)).toContainText('from ana@x.com');
    await expect(rows.nth(0).getByRole('button', { name: 'Yes' })).toBeVisible();
    await expect(rows.nth(0).getByRole('button', { name: 'Maybe' })).toBeVisible();
    await expect(rows.nth(0).getByRole('button', { name: 'No' })).toBeVisible();

    // The CalDAV row is real and listed — but no RSVP write exists for it,
    // so it says where the answer lives instead of offering three buttons
    // that could only fail.
    await expect(rows.nth(1)).toContainText('Team offsite');
    await expect(rows.nth(1)).toContainText('answer at your provider');
    await expect(rows.nth(1).getByRole('button', { name: 'Yes' })).toHaveCount(0);
    // Its days come from the calendar-zone strings the backend sent — the
    // fixture's `2024-01-30`, whatever zone this browser runs in.
    await expect(rows.nth(1)).toContainText('Jan 30');
    await expect(rows.nth(1)).toContainText('All day');
  });

  test('Yes answers the whole series with the invite\'s own start', async ({ page }) => {
    await page.goto(show('Header', 'with-invites'));
    await page.getByRole('button', { name: '2 pending invitations' }).click();
    await page.getByTestId('invite-row').nth(0).getByRole('button', { name: 'Yes' }).click();

    // `App` was told — the fact a second host for these rows could drop.
    await expect.poll(() => page.evaluate(() => (window as any).__inviteAnswers)).toBe(1);

    // The premise read from the fixture itself, not a copy typed here.
    const expected = await page.evaluate(() => {
      const inv = (window as any).__fixtureProps.invites[0];
      return { id: inv.id, startMs: inv.start_ms };
    });
    const call = await page.evaluate(() =>
      (window as any).__harness.calls.find((c: { cmd: string }) => c.cmd === 'respond_to_event')?.args);
    expect(call).toEqual({
      id: expected.id,
      response: 'accepted',
      scope: 'all',
      occurrenceStartMs: expected.startMs,
    });
  });

  test('declines by others list beside the invitations, and × acknowledges', async ({ page }) => {
    await page.goto(show('Header', 'with-declines'));
    // One invitation and two declines, counted together and named apart.
    const badge = page.getByRole('button', { name: '1 pending invitation, 2 declines' });
    await expect(badge).toHaveText('✉ 3');
    await badge.click();

    const rows = page.getByTestId('decline-row');
    await expect(rows).toHaveCount(2);
    await expect(page.locator('.sect span')).toHaveText('Declined your meeting');
    await expect(rows.nth(0)).toContainText('Victor declined');
    await expect(rows.nth(0)).toContainText('Weekly ops');
    // No display name falls back to the address; the all-day when line
    // comes from the calendar-zone date strings.
    await expect(rows.nth(1)).toContainText('iskren@x.com declined');
    await expect(rows.nth(1)).toContainText('Jan 30');
    // Declines ask only to be seen: no Yes/Maybe/No on these rows.
    await expect(rows.nth(0).getByRole('button', { name: 'Yes' })).toHaveCount(0);

    await rows.nth(0).getByRole('button', { name: 'Dismiss decline by Victor' }).click();
    // Gone immediately — the optimistic hide — and the badge follows.
    await expect(page.getByTestId('decline-row')).toHaveCount(1);
    await expect(
      page.getByRole('button', { name: '1 pending invitation, 1 decline' })
    ).toHaveText('✉ 2');
    // The acknowledgement went out under the stable ids, and App was told.
    const call = await page.evaluate(() =>
      (window as any).__harness.calls.find((c: { cmd: string }) => c.cmd === 'dismiss_decline_notice')?.args);
    expect(call).toEqual({ calendarId: 1, gid: 'weekly-ops', email: 'victor@x.com' });
    await expect.poll(() => page.evaluate(() => (window as any).__inviteAnswers)).toBe(1);
  });

  test('Dismiss all clears every decline in one stroke, invitations untouched', async ({ page }) => {
    await page.goto(show('Header', 'with-declines'));
    await page.getByRole('button', { name: '1 pending invitation, 2 declines' }).click();
    await page.getByRole('button', { name: 'Dismiss all' }).click();

    await expect(page.getByTestId('decline-row')).toHaveCount(0);
    // The invitation still awaits its answer — Dismiss all is declines-only.
    await expect(page.getByTestId('invite-row')).toHaveCount(1);
    await expect(
      page.getByRole('button', { name: '1 pending invitation' })
    ).toHaveText('✉ 1');
    expect(await page.evaluate(() =>
      (window as any).__harness.calls.some((c: { cmd: string }) => c.cmd === 'dismiss_all_decline_notices'))).toBe(true);
    await expect.poll(() => page.evaluate(() => (window as any).__inviteAnswers)).toBe(1);
  });

  test('one decline offers only its own ×, not a Dismiss all', async ({ page }) => {
    // A single decline's × is already under the finger; "all" of one is
    // noise. Reached by dismissing one of the fixture's two.
    await page.goto(show('Header', 'with-declines'));
    await page.getByRole('button', { name: '1 pending invitation, 2 declines' }).click();
    await page.getByRole('button', { name: 'Dismiss decline by Victor' }).click();
    await expect(page.getByTestId('decline-row')).toHaveCount(1);
    await expect(page.getByRole('button', { name: 'Dismiss all' })).toHaveCount(0);
  });

  test('rescheduled and cancelled meetings list with their old and new slots', async ({ page }) => {
    await page.goto(show('Header', 'with-changes'));
    const badge = page.getByRole('button', {
      name: '1 decline, 2 rescheduled, 2 cancelled',
    });
    await expect(badge).toHaveText('✉ 5');
    await badge.click();

    const moved = page.getByTestId('moved-row');
    await expect(moved).toHaveCount(2);
    // Timed: old day+time → new day+time. The premise (a five-hour, next-day
    // move) is the fixture's own instants.
    await expect(moved.nth(0)).toContainText('NVP sync');
    await expect(moved.nth(0)).toContainText('→');
    // All-day: the calendar-zone date strings, not instant-derived days.
    await expect(moved.nth(1)).toContainText('Offsite');
    await expect(moved.nth(1)).toContainText('Jan 30');
    await expect(moved.nth(1)).toContainText('Feb 2');

    const cancelled = page.getByTestId('cancelled-row');
    await expect(cancelled).toHaveCount(2);
    await expect(cancelled.nth(0)).toContainText('Retro');
    await expect(cancelled.nth(0)).toContainText('was ');

    // One × acknowledges one change, under its stable ids.
    await moved.nth(0).getByRole('button', { name: 'Dismiss reschedule of NVP sync' }).click();
    await expect(page.getByTestId('moved-row')).toHaveCount(1);
    const call = await page.evaluate(() =>
      (window as any).__harness.calls.find((c: { cmd: string }) => c.cmd === 'dismiss_change_notice')?.args);
    expect(call).toEqual({ calendarId: 1, gid: 'nvp' });
  });

  test('answering a reschedule RSVPs at the backend\'s scope and dismisses in one click', async ({ page }) => {
    // A reschedule is a new proposal the old yes should not cover silently
    // (2026-08-21, by request): the moved row offers the same three answers
    // an invitation does, and answering *is* dealing with the notice.
    await page.goto(show('Header', 'with-changes'));
    await page.getByRole('button', { name: '1 decline, 2 rescheduled, 2 cancelled' }).click();

    const nvp = page.getByTestId('moved-row').nth(0);
    await expect(nvp).toContainText('NVP sync');
    await nvp.getByRole('button', { name: 'No' }).click();

    // The write is the popover's own, at the scope and start the backend
    // decided — a moved exception answers this occurrence at its new slot.
    const sent = await page.evaluate(() => (window as any).__lastRespondCall);
    expect(sent.id).toBe(71);
    expect(sent.response).toBe('declined');
    expect(sent.scope).toBe('this');
    expect(sent.occurrenceStartMs).toBe(MON + 58 * 3_600_000);

    // One click did both: the row is gone and its notice acknowledged.
    await expect(page.getByTestId('moved-row')).toHaveCount(1);
    const dismissed = await page.evaluate(() =>
      (window as any).__harness.calls.find((c: { cmd: string }) => c.cmd === 'dismiss_change_notice')?.args);
    expect(dismissed).toEqual({ calendarId: 1, gid: 'nvp' });
  });

  test('a reschedule that cannot be answered offers only the ×', async ({ page }) => {
    // `can_respond` is the backend's gate, same as the invitation rows': a
    // CalDAV move is real and listed, but the answer lives at the provider —
    // three buttons that could only fail must not render. Cancelled rows
    // never answer: there is nothing left to answer to.
    await page.goto(show('Header', 'with-changes'));
    await page.getByRole('button', { name: '1 decline, 2 rescheduled, 2 cancelled' }).click();

    const offsite = page.getByTestId('moved-row').nth(1);
    await expect(offsite).toContainText('Offsite');
    await expect(offsite.getByRole('button', { name: 'No' })).toHaveCount(0);
    await expect(page.getByTestId('cancelled-row').getByRole('button', { name: 'No' }))
      .toHaveCount(0);
  });

  test('Dismiss all is per section: cancelled goes, rescheduled and declines stay', async ({ page }) => {
    await page.goto(show('Header', 'with-changes'));
    await page.getByRole('button', { name: '1 decline, 2 rescheduled, 2 cancelled' }).click();

    // The Cancelled section's own stroke — scoped by the section, since
    // Rescheduled offers an identically labelled button.
    const cancelledSect = page.locator('.sect', { hasText: 'Cancelled' });
    await cancelledSect.getByRole('button', { name: 'Dismiss all' }).click();

    await expect(page.getByTestId('cancelled-row')).toHaveCount(0);
    await expect(page.getByTestId('moved-row')).toHaveCount(2);
    await expect(page.getByTestId('decline-row')).toHaveCount(1);
    const call = await page.evaluate(() =>
      (window as any).__harness.calls.find((c: { cmd: string }) => c.cmd === 'dismiss_all_change_notices')?.args);
    expect(call).toEqual({ kind: 'cancelled' });
    await expect(
      page.getByRole('button', { name: '1 decline, 2 rescheduled' })
    ).toHaveText('✉ 3');
  });

  test('the panel stays on screen when a tiled window wraps the header', async ({ page }) => {
    // In a tiled (narrow) window the header wraps and the badge lands near
    // the LEFT edge — and the panel, hard-anchored to hang right, walked
    // off the screen (seen live, 2026-08-19). The panel now hangs from
    // whichever side has room, and is capped to the viewport.
    await page.setViewportSize({ width: 420, height: 700 });
    await page.goto(show('Header', 'with-invites'));
    await page.getByRole('button', { name: '2 pending invitations' }).click();

    const box = await page.getByRole('group', { name: 'Pending invitations' }).boundingBox();
    if (!box) throw new Error('panel not rendered');
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(421);
    // And its rows are actually readable, not a sliver.
    expect(box.width).toBeGreaterThan(300);
  });

  test('a failed answer stays on its row instead of vanishing', async ({ page }) => {
    await page.goto(show('Header', 'with-invites'));
    await page.evaluate(() =>
      (window as any).__harness.failNextEventCall(
        'respond_to_event', 901, 'could not reach Google right now.'));

    await page.getByRole('button', { name: '2 pending invitations' }).click();
    const row = page.getByTestId('invite-row').nth(0);
    await row.getByRole('button', { name: 'No' }).click();

    await expect(row).toContainText('could not reach Google right now.');
    await expect(row.getByRole('button', { name: 'No' })).toBeEnabled();
    expect(await page.evaluate(() => (window as any).__inviteAnswers)).toBe(0);
  });
});
