// App.svelte — the wiring the component specs cannot reach: two Tauri event
// listeners, the week-loading effect, and what the user is shown when any of
// it goes wrong. Everything here runs against the real component with a
// stubbed IPC layer (tests/harness/tauri.ts).

import { test, expect, type Page } from '@playwright/test';
import {
  CHORDS, EDIT_CHORDS, EVENT_SHORTCUT_LIST, SHORTCUT_LIST, SHORTCUT_TEXT,
} from '../src/lib/shortcuts';
import {
  APP_MON, APP_NOW, weekLabel,
  APP_SOLO_SERIES_ID,
  APP_PRIMARY_CALENDAR_ID, APP_READER_CALENDAR_ID,
  APP_SERIES_DTSTART, APP_SERIES_OCCURRENCE,
  APP_ALLDAY_OCCURRENCE, APP_ALLDAY_SERIES_DTSTART,
  XZONE_NOW, XZONE_STORED_START, XZONE_WEEK_START, XZONE_DAY,
  XZONE_DISPLAY_MISREADING,
  APP_ALLDAY_ID,
} from './fixtures';
import { NO_CONFIG_ERROR } from './harness/tauri';
import { APP_CHROME_PX } from './harness/viewbox';

const WEEK = 7 * 24 * 3_600_000;
/** The week the app opens on, and the one a click on › moves to. */
const W1 = APP_MON;
const W2 = APP_MON + WEEK;

const app = (fixture = 'default') => `/tests/harness/index.html?c=App&f=${fixture}`;

test.describe('App', () => {
  test.beforeEach(async ({ page }) => {
    // `weekStart(new Date())` decides which week the app opens on, so the
    // clock has to be frozen before the page loads, not after.
    await page.clock.setFixedTime(APP_NOW);
  });

  test('loads the current week and reports the last sync', async ({ page }) => {
    await page.goto(app());
    await expect(page.locator('h1')).toHaveText('January 2024');
    await expect(page.locator('.ev b')).toHaveText(weekLabel(W1));
    // The sentence lives on the status light now (spec §2) rather than in the
    // header's own text. Same fact, same precision — a glance gets the colour
    // and a hover gets the minutes.
    await expect(page.locator('.light')).toHaveAttribute('aria-label', 'Synced 5 min ago');
    await expect(page.locator('.err')).toHaveCount(0);
  });

  test('navigating forward loads the next week', async ({ page }) => {
    await page.goto(app());
    await expect(page.locator('.ev b')).toHaveText(weekLabel(W1));
    await page.getByRole('button', { name: 'Next week' }).click();
    await expect(page.locator('h1')).toHaveText('February 2024');
    await expect(page.locator('.ev b')).toHaveText(weekLabel(W2));
  });

  /** The `open-date` entrance — `omacal 2024-03-14` against a running app.
   *  The backend has already shown the window; the event alone moves the
   *  calendar, in whatever view is already on screen. */
  test('an open-date event moves the calendar to that day', async ({ page }) => {
    await page.goto(app());
    await expect(page.locator('h1')).toHaveText('January 2024');
    await page.evaluate(() => window.__harness.emit('open-date', '2024-03-14'));
    await expect(page.locator('h1')).toHaveText('March 2024');
  });

  /** The `open-event` entrance — a clicked reminder: land on the day *and*
   *  open the popover on the occurrence, the same arrival a chosen search
   *  hit gets. The occurrence sits 45 days out for the reason the Dentist
   *  search fixture does: in the app's own January, a version that never
   *  moved the anchor would pass. */
  test('a clicked reminder lands on the occurrence and opens it', async ({ page }) => {
    await page.goto(app());
    await expect(page.locator('h1')).toHaveText('January 2024');
    await page.evaluate(([id, start]) => window.__harness.emit('open-event', {
      id, startMs: start, endMs: (start as number) + 3_600_000,
    }), [APP_SOLO_SERIES_ID, APP_NOW + 45 * 24 * 3_600_000]);
    await expect(page.locator('h1')).toHaveText('March 2024');
    await expect(page.getByRole('dialog', { name: 'Gym' })).toBeVisible();
  });

  /** The fresh-launch half of the same entrance: the date was parked on the
   *  backend before the webview existed, and mount collects it — so the app
   *  opens on that day instead of on today. */
  test('a dated fresh launch opens on that date', async ({ page }) => {
    await page.goto(app('launched-with-date'));
    await expect(page.locator('h1')).toHaveText('March 2024');
  });

  // D3. A background reload is issued for the week on screen, the user moves
  // on before it answers, and the answer arrives last. Without a stamp on each
  // request the late answer wins and paints the old week under the new week's
  // header — which is `$derived` from `weekStartMs` and has already moved —
  // and nothing short of navigating again ever puts it right.
  test('a slow background reload never repaints a week the user has left', async ({ page }) => {
    await page.goto(app());
    await expect(page.locator('.ev b')).toHaveText(weekLabel(W1));

    // Park the reload that `sync-finished` is about to trigger for W1.
    await page.evaluate((w) => window.__harness.hold(w), W1);
    await page.evaluate(() => window.__harness.emit('sync-finished', { upserted: 3 }));
    await expect.poll(() => page.evaluate(() => window.__harness.held())).toBe(1);

    // The user moves to next week while that request is still in flight.
    await page.getByRole('button', { name: 'Next week' }).click();
    await expect(page.locator('h1')).toHaveText('February 2024');
    await expect(page.locator('.ev b')).toHaveText(weekLabel(W2));

    // Now the stale answer lands. It must be dropped, not painted.
    await page.evaluate((w) => window.__harness.release(w), W1);
    await expect(page.locator('.ev b')).toHaveText(weekLabel(W2));
    await expect(page.locator('h1')).toHaveText('February 2024');
  });

  // D2. The header's "Synced N ago" is computed from the last *successful*
  // sync, so it is structurally incapable of reporting that sync is broken.
  test('a failed background sync says so', async ({ page }) => {
    await page.goto(app());
    await expect(page.locator('.err')).toHaveCount(0);

    await page.evaluate(() =>
      window.__harness.emit('sync-failed', {
        message: 'Sync failed — OmaCal could not reach Google. It will keep trying.',
      }),
    );
    await expect(page.locator('.err')).toContainText('could not reach Google');
  });

  // An account whose refresh token died gets its own surface: a standing
  // banner naming the account, with the one button that fixes it. The generic
  // "it will keep trying" toast would be a lie here — the backend has stopped
  // trying — which is why this is `needs_reauth` on status and not `error`.
  test('a dead account offers reconnect, and reconnecting clears it', async ({ page }) => {
    await page.goto(app('needs-reauth'));

    const banner = page.getByTestId('reauth-banner');
    await expect(banner).toContainText('me@x.com');
    // The light agrees with the banner rather than reading "Synced 5 min ago"
    // in green off the last success.
    await expect(page.locator('.light'))
      .toHaveAttribute('aria-label', 'An account needs to be reconnected');

    await banner.getByRole('button', { name: 'Reconnect' }).click();

    // The click runs the real sign-in flow, and the refreshed status stops
    // asking — banner gone, light back on the last successful sync.
    await expect(banner).toHaveCount(0);
    const signIns = await page.evaluate(() =>
      window.__harness.calls.filter((c) => c.cmd === 'sign_in').length);
    expect(signIns).toBe(1);
  });

  // The update notice's one action goes through the backend: no URL crosses
  // from the webview, the command opens the page the daily check fetched.
  test("What's new asks the backend to open the release page", async ({ page }) => {
    await page.goto(app('update-available'));
    const banner = page.getByTestId('update-banner');
    await expect(banner).toContainText('OmaCal 0.2.0 is available');

    await banner.getByRole('button', { name: "What's new" }).click();
    const calls = await page.evaluate(() =>
      window.__harness.calls.filter((c) => c.cmd === 'open_latest_release'));
    expect(calls).toHaveLength(1);
    // No argument at all — the URL is the backend's to choose.
    expect(calls[0].args).toEqual({});
  });

  // D2, the other half: the label has to keep counting on its own. It used to
  // recompute only when `status` changed — that is, only when a sync
  // succeeded — so it froze at its last value exactly when sync stopped.
  test('the synced label keeps counting while nothing happens', async ({ page }) => {
    await page.clock.install({ time: APP_NOW });
    await page.goto(app());
    await expect(page.locator('.light')).toHaveAttribute('aria-label', 'Synced 5 min ago');

    await page.clock.fastForward('10:00');
    // Unweakened by the move: the light's name is built from the same
    // `relativeTime` the label was, so this still catches a value that froze
    // at its last sync — which is exactly when its staleness is worth seeing.
    await expect(page.locator('.light')).toHaveAttribute('aria-label', 'Synced 15 min ago');
  });

  // D6. This reload used to sit outside any try/catch, so a failure here was
  // an unhandled rejection in the console and nothing at all on screen.
  test('a failed background reload surfaces instead of vanishing', async ({ page }) => {
    await page.goto(app());
    await expect(page.locator('.ev b')).toHaveText(weekLabel(W1));

    await page.evaluate(() => window.__harness.failNextWeek('database is locked'));
    await page.evaluate(() => window.__harness.emit('sync-finished', { upserted: 1 }));
    await expect(page.locator('.err')).toContainText('database is locked');
  });

  // D5. The likeliest first-run failure, and the half that matters — "Create
  // it with client_id and client_secret" — is the half a 320px ellipsised
  // line threw away.
  test('the first-run config error is readable in full', async ({ page }) => {
    await page.goto(app('no-config'));
    await page.getByRole('button', { name: 'Connect Google Calendar' }).click();

    const err = page.locator('.err');
    await expect(err).toContainText('Create it with client_id and client_secret');

    // Not clipped horizontally, and taller than the single line it used to be
    // squeezed into.
    const clipped = await err.evaluate((el) => el.scrollWidth > el.clientWidth + 1);
    expect(clipped, 'the error text is being cut off horizontally').toBe(false);
    const box = await err.boundingBox();
    expect(box!.height).toBeGreaterThan(24);
  });

  test('the error clears when the user moves to another week', async ({ page }) => {
    await page.goto(app('no-config'));
    await page.getByRole('button', { name: 'Connect Google Calendar' }).click();
    await expect(page.locator('.err')).toContainText(NO_CONFIG_ERROR);

    await page.getByRole('button', { name: 'Next week' }).click();
    await expect(page.locator('.err')).toHaveCount(0);
  });

  // Task 6: the calendar popover is Header's own concern, but the reload
  // that feeds it is App's. Without it, a second account's calendars stay
  // invisible until the app is relaunched — a second account you cannot see
  // is the same as no second account.
  test('signing in reloads the calendar list', async ({ page }) => {
    await page.goto(app());
    const calendarCalls = () =>
      page.evaluate(() =>
        window.__harness.calls.filter((c) => c.cmd === 'get_calendars').length,
      );
    // Wait for the mount effect's own `get_calendars` to land before taking
    // the baseline — otherwise that call can race the click below and land
    // during the "after" window instead, inflating the count for the wrong
    // reason and passing even if sign-in never reloads anything.
    await expect.poll(calendarCalls).toBeGreaterThan(0);
    const before = await calendarCalls();

    // Behind the hamburger now (spec §1), so getting to it is part of the act.
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Add account' }).click();
    await expect.poll(calendarCalls).toBeGreaterThan(before);
  });

  // Task 7: the picker opens after every sign-in — first account or fifth —
  // so a freshly imported set of calendars, all switched on by default, is
  // never left syncing behind the user's back without them having seen it.
  test('signing in opens the picker with the new calendars in it', async ({ page }) => {
    await page.goto(app('sign-in-adds-account'));
    await page.getByRole('button', { name: /Connect|Add account/ }).click();
    await expect(page.locator('.panel')).toBeVisible();
    await expect(page.locator('.acct')).toHaveCount(1);
  });

  // Fix round 1 (Task 7), finding 2: `open` is bound two levels deep —
  // `App` to `Header` to `CalendarPopover` — and every existing spec signs
  // in exactly once, so a one-way binding on either leg is invisible: the
  // panel would still be visible right after that first sign-in even if
  // closing it never made its way back up to `App`'s own `pickerOpen`. Only
  // a second sign-in, after the first close, can catch `pickerOpen` stuck
  // true and unable to reopen the (by-then-closed) child.
  test('closing the picker then signing in again reopens it', async ({ page }) => {
    await page.goto(app('sign-in-adds-account'));
    await page.getByRole('button', { name: /Connect|Add account/ }).click();
    await expect(page.locator('.panel')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator('.panel')).toHaveCount(0);
    // **One Escape closes one layer.** The picker is gone and the menu it
    // opened inside is still standing, which is what lets the next click reach
    // Add account — and is the behaviour three `window` keydown listeners
    // would otherwise collapse into one keystroke.
    await page.getByRole('button', { name: 'Add account' }).click();
    await expect(page.locator('.panel')).toBeVisible();
  });

  // Same round trip, via the click-away close path rather than Escape —
  // both are ways `CalendarPopover` sets its own `open` to false, and the
  // reviewer who found this confirmed both propagate up correctly.
  test('clicking away then signing in again reopens it', async ({ page }) => {
    await page.goto(app('sign-in-adds-account'));
    await page.getByRole('button', { name: /Connect|Add account/ }).click();
    await expect(page.locator('.panel')).toBeVisible();
    // **The picker's own scrim**, named exactly: the menu it now sits inside
    // has one too, and clicking the wrong one would close the wrong layer.
    await page.getByRole('button', { name: 'Close', exact: true }).click();
    await expect(page.locator('.panel')).toHaveCount(0);

    await page.getByRole('button', { name: 'Add account' }).click();
    await expect(page.locator('.panel')).toBeVisible();
  });

  // Task 5: the switcher, the keyboard, and the shared anchor date. `.col` is
  // WeekGrid's own day-column class (see `WeekGrid.svelte`; the brief's draft
  // used `.daycol` as a placeholder, same correction Task 2 already made for
  // its own two specs).

  // Plan 4 Task 5: Year and Big Year join the other three, replacing the
  // "two of them not yet built" spec this superseded — `disabled` slots are
  // gone for good (DoD: "no disabled slots").
  test('all five views are reachable', async ({ page }) => {
    await page.goto(app('connected'));
    // Wait on the buttons *existing* before asserting none are disabled. The
    // disabled-count assertion alone is satisfied by an unmounted page — zero
    // buttons are disabled when there are zero buttons — so it waited for
    // nothing and the keypress below landed before `<svelte:window onkeydown>`
    // had attached. Same mount race the two specs after this one guard against.
    await expect(page.locator('.vswitch button')).toHaveCount(5);
    await expect(page.locator('.vswitch button[disabled]')).toHaveCount(0);
    await page.keyboard.press('4');
    await expect(page.locator('.ymonth')).toHaveCount(12);
    await page.keyboard.press('5');
    await expect(page.locator('.rrow')).toHaveCount(14);
  });

  test('H and L step by a year in the year views', async ({ page }) => {
    await page.goto(app('connected'));
    // Same mount race every other spec in this file guards against — see
    // "number keys switch views" above: fired before `<svelte:window
    // onkeydown>` has attached, the keypress lands on nothing and is gone.
    await expect(page.locator('.vswitch button')).toHaveCount(5);
    await page.keyboard.press('4');
    const before = await page.locator('.ygrid').getAttribute('data-year');
    await page.keyboard.press('l');
    const after = await page.locator('.ygrid').getAttribute('data-year');
    expect(Number(after)).toBe(Number(before) + 1);
  });

  test('the keys work on a layout that writes its own script (#38)', async ({ page }) => {
    // @xmha97's top-row digits write Persian ۱۲۳, so only the numpad, which
    // writes Latin digits on every layout, switched views; his `h` writes
    // something else too. The physical key is read instead when the layout
    // writes a non-ASCII character. `dispatchEvent` because Playwright's
    // keyboard cannot type a layout; the app reads the same `key`/`code`
    // pair a real press would carry.
    await page.goto(app('connected'));
    await expect(page.locator('.vswitch button')).toHaveCount(5); // mount race — see above
    await page.locator('body').dispatchEvent('keydown', { key: '۱', code: 'Digit1' });
    await expect(page.locator('.col')).toHaveCount(1);
    await page.locator('body').dispatchEvent('keydown', { key: '۳', code: 'Digit3' });
    await expect(page.locator('.mcell').first()).toBeVisible();
    await expect(page.locator('h1')).toHaveText('January 2024');
    // Bulgarian phonetic: х on the H key steps back a month.
    await page.locator('body').dispatchEvent('keydown', { key: 'х', code: 'KeyH' });
    await expect(page.locator('h1')).toHaveText('December 2023');
    // Dvorak's d on the same physical key means d, which is no shortcut.
    await page.locator('body').dispatchEvent('keydown', { key: 'd', code: 'KeyH' });
    await expect(page.locator('h1')).toHaveText('December 2023');
  });

  test('big year reaches this year and next, and no further back', async ({ page }) => {
    // Spec §4: it is a planning surface — what is coming, not what happened.
    await page.goto(app('connected'));
    await expect(page.locator('.vswitch button')).toHaveCount(5); // mount race — see above
    await page.keyboard.press('5');
    const opened = await page.locator('.ribbon').getAttribute('data-year');
    await page.keyboard.press('h');
    expect(await page.locator('.ribbon').getAttribute('data-year')).toBe(opened);
    await page.keyboard.press('l');
    expect(Number(await page.locator('.ribbon').getAttribute('data-year'))).toBe(Number(opened) + 1);
    await page.keyboard.press('l');
    expect(Number(await page.locator('.ribbon').getAttribute('data-year'))).toBe(Number(opened) + 1);
  });

  test('the arrows step a year too, and say so', async ({ page }) => {
    await page.goto(app('connected'));
    await expect(page.locator('.vswitch button')).toHaveCount(5); // mount race — see above
    await page.keyboard.press('4');
    await expect(page.getByRole('button', { name: 'Next year' })).toBeVisible();
    const before = await page.locator('.ygrid').getAttribute('data-year');
    await page.getByRole('button', { name: 'Next year' }).click();
    expect(Number(await page.locator('.ygrid').getAttribute('data-year'))).toBe(Number(before) + 1);
  });

  test('number keys switch views', async ({ page }) => {
    await page.goto(app('connected'));
    // `page.keyboard.press` is a one-shot dispatch, not an auto-waiting
    // locator action — pressed before `<svelte:window onkeydown>` has
    // actually attached (mount.svelte.ts imports `App.svelte` via a dynamic
    // `import()`, which can still be in flight when `goto` resolves), the
    // keydown lands on a window with no listener yet and is gone for good;
    // nothing about it is retried. Waiting for the switcher itself — the
    // thing that same mount attaches — is what makes the keypress land on a
    // window that's actually listening.
    await expect(page.locator('.vswitch button')).toHaveCount(5);
    await page.keyboard.press('3');
    await expect(page.locator('.mrow')).toHaveCount(6);
    await page.keyboard.press('1');
    await expect(page.locator('.col')).toHaveCount(1);
  });

  test('the anchor date survives a view switch', async ({ page }) => {
    // Spec §5: switching Month -> Day lands on the day you were looking at,
    // not on today. This is what makes "+N more" and the day-number click
    // work as handoffs rather than jumps.
    await page.goto(app('connected'));
    await expect(page.locator('.vswitch button')).toHaveCount(5); // see above
    await page.keyboard.press('3');
    await page.locator('.mcell .num').nth(14).click();
    await expect(page.locator('.col')).toHaveCount(1);
    const shown = await page.locator('.col').getAttribute('data-start-ms');
    // The day that was clicked: cell 14 of `busyDayMonth`, which is row 2
    // column 0 — Mon 10 Aug 2026 00:00 UTC, the grid's own midnight for it.
    expect(Number(shown)).toBe(1786320000000);
  });

  test('the legend under the ribbon hides a calendar, and the ribbon reloads', async ({ page }) => {
    // The strip under Big Year was a read-only key. It now flips the same
    // `selected` switch the sidebar's checkbox does (2026-09-03), and the
    // ribbon refetches so its pills follow the switch.
    await page.goto(app('writable'));
    await expect(page.locator('.vswitch button')).toHaveCount(5); // mount race — see above
    await page.keyboard.press('5');
    await expect(page.locator('.rrow')).toHaveCount(14);
    await expect(page.locator('.legend .item')).toHaveCount(3);
    const calls = (cmd: string) => page.evaluate(
      (c) => window.__harness.calls.filter((call) => call.cmd === c).map((call) => call.args), cmd,
    );
    const fetchesBefore = (await calls('get_big_year')).length;

    await page.locator('.legend .item', { hasText: 'Team' }).click();
    await expect.poll(() => calls('set_calendar_selected')).toEqual([{ id: 8, on: false }]);
    await expect.poll(async () => (await calls('get_big_year')).length).toBeGreaterThan(fetchesBefore);
  });

  test('the title reserves its width, so the arrows never move between months', async ({ page }) => {
    // "September 2026" to "May 2026" moved the arrows, Today and the
    // switcher by more than an arrow's own width, so stepping months by
    // repeated clicks misclicked (reported 2026-09-03). The title's box is
    // now as wide as the widest month in the locale, whichever is shown —
    // and the year alone, in the year views, sits in the same slot.
    await page.goto(app('connected'));
    await expect(page.locator('.vswitch button')).toHaveCount(5); // mount race — see above
    await page.keyboard.press('3');
    await expect(page.locator('h1')).toHaveText('January 2024');
    const next = page.getByRole('button', { name: 'Next month' });
    const arrowX = (await next.boundingBox())!.x;
    const switcherX = (await page.locator('.vswitch').boundingBox())!.x;

    for (let i = 0; i < 4; i++) await page.keyboard.press('l');
    await expect(page.locator('h1')).toHaveText('May 2024');
    expect((await next.boundingBox())!.x).toBe(arrowX);
    expect((await page.locator('.vswitch').boundingBox())!.x).toBe(switcherX);

    await page.keyboard.press('4');
    await expect(page.locator('h1')).toHaveText('2024');
    expect((await page.getByRole('button', { name: 'Next year' }).boundingBox())!.x).toBe(arrowX);
  });

  test('the year views are titled by the year alone, and the arrows move it', async ({ page }) => {
    // The `<h1>` formatted month-and-year for every view, so Big Year — a
    // ribbon of the whole year — was headed "September 2026" after the
    // anchor's month, and `›`, which steps a whole year, appeared to leave
    // the title alone (reported 2026-09-03). The clock is frozen to Jan
    // 2024, which is also Big Year's lower bound (spec §4).
    await page.goto(app('connected'));
    await expect(page.locator('.vswitch button')).toHaveCount(5); // mount race — see above
    await page.keyboard.press('4');
    await expect(page.locator('h1')).toHaveText('2024');
    await page.keyboard.press('l');
    await expect(page.locator('h1')).toHaveText('2025');

    await page.keyboard.press('5');
    await expect(page.locator('h1')).toHaveText('2024');
    await page.getByRole('button', { name: 'Next year' }).click();
    await expect(page.locator('h1')).toHaveText('2025');

    // And back in a month-shaped view the month returns — the anchor never
    // moved while the year counters did.
    await page.keyboard.press('2');
    await expect(page.locator('h1')).toHaveText('January 2024');
  });

  test('the anchor date reaches Year view too, but never Big Year', async ({ page }) => {
    // Spec §5 says "every switch", and Year is a switch. `yearNum` is its own
    // counter seeded from the real clock, so Year used to open on the current
    // year no matter where the anchor stood — here, anchored in Aug 2026 under
    // a clock frozen to Jan 2024, it opened on 2024.
    await page.goto(app('connected'));
    await expect(page.locator('.vswitch button')).toHaveCount(5); // mount race — see above
    await page.keyboard.press('3');
    await page.locator('.mcell .num').nth(14).click(); // anchors on 10 Aug 2026
    await expect(page.locator('.col')).toHaveCount(1);

    await page.keyboard.press('4');
    await expect(page.locator('.ygrid')).toHaveAttribute('data-year', '2026');

    // Big Year is deliberately left out: spec §4 bounds it to the real current
    // year and the next, so an anchor in the past has nowhere to put it —
    // seeding it would either break the bound or drag the anchor forward past
    // it. The clock is frozen to Jan 2024, so that bound is 2024.
    await page.keyboard.press('5');
    await expect(page.locator('.ribbon')).toHaveAttribute('data-year', '2024');
  });

  // Whole-branch review, finding 1: the `<h1>` was derived from `weekStartMs`
  // in every view, so Day and Month — which render `anchorMs`'s own month —
  // were titled with the month the *Monday of that week* falls in. The two
  // disagree whenever the anchor's week began in the previous month, which is
  // two keystrokes away from any given day. 1 Feb 2024 is the shape: a
  // Thursday, in the week that started Mon 29 Jan.
  //
  // All three units in one spec, because the bug is precisely that they
  // shared one: Day and Month must read February, Week must still read
  // January for the same anchor.
  test('the title names the month of the unit actually on screen', async ({ page }) => {
    await page.goto(app('connected'));
    await expect(page.locator('.vswitch button')).toHaveCount(5); // see above
    await page.keyboard.press('1'); // Day view, anchored on Mon 29 Jan (APP_MON)
    await page.keyboard.press('l'); // 30 Jan
    await page.keyboard.press('l'); // 31 Jan
    await page.keyboard.press('l'); // 1 Feb — a Thursday, week still begins 29 Jan
    await expect(page.locator('h1')).toHaveText('February 2024');

    await page.keyboard.press('3'); // Month view: the February grid, titled for February
    await expect(page.locator('.mrow')).toHaveCount(6);
    await expect(page.locator('h1')).toHaveText('February 2024');

    // Week keeps its own rule — the month its Monday falls in — which is what
    // makes this anchor tell the three units apart at all.
    await page.keyboard.press('2');
    await expect(page.locator('h1')).toHaveText('January 2024');
  });

  // The same seam by mouse. `‹`/`›` used to step a week in *every* view while
  // announcing themselves as "Previous week"/"Next week" — in Month view that
  // moved the grid by a week, sometimes not changing the month at all and
  // sometimes crossing a boundary. One spec per unit, so each proves its own
  // motion rather than the first failure hiding the rest.
  test('the header arrows step a day in Day view, and say so', async ({ page }) => {
    await page.goto(app('connected'));
    await expect(page.locator('.vswitch button')).toHaveCount(5); // see above
    await page.keyboard.press('1');
    const next = page.getByRole('button', { name: 'Next day' });
    await expect(next).toBeVisible();
    await expect(page.locator('.col')).toHaveCount(1);

    const before = Number(await page.locator('.col').getAttribute('data-start-ms'));
    await next.click();
    // `toHaveAttribute` retries, so this waits for the re-fetch rather than
    // racing the click against it.
    await expect(page.locator('.col'))
      .toHaveAttribute('data-start-ms', String(before + 24 * 3600 * 1000)); // 30 Jan
    await page.getByRole('button', { name: 'Previous day' }).click();
    await expect(page.locator('.col')).toHaveAttribute('data-start-ms', String(before));
  });

  // Read off Day view's own column rather than the `<h1>`, so this pins the
  // *motion* independently of the title fix above: a week step from 29 Jan
  // lands on 5 Feb, which names February just as correctly as 29 Feb does.
  test('the header arrows step a month in Month view, and say so', async ({ page }) => {
    await page.goto(app('connected'));
    await expect(page.locator('.vswitch button')).toHaveCount(5); // see above
    await page.keyboard.press('3'); // Month view, anchored Mon 29 Jan (APP_MON)
    const next = page.getByRole('button', { name: 'Next month' });
    await expect(next).toBeVisible();
    await next.click();

    await page.keyboard.press('1');
    await expect(page.locator('.col'))
      .toHaveAttribute('data-start-ms', String(Date.UTC(2024, 1, 29))); // 29 Feb, not 5 Feb

    await page.keyboard.press('3');
    await page.getByRole('button', { name: 'Previous month' }).click();
    await page.keyboard.press('1');
    await expect(page.locator('.col'))
      .toHaveAttribute('data-start-ms', String(Date.UTC(2024, 0, 29))); // back to 29 Jan
  });

  test('H and L step by the current view\'s unit', async ({ page }) => {
    await page.goto(app('connected'));
    await expect(page.locator('.vswitch button')).toHaveCount(5); // see above
    await page.keyboard.press('2');
    const before = await page.locator('.col').first().getAttribute('data-start-ms');
    await page.keyboard.press('l');
    const after = await page.locator('.col').first().getAttribute('data-start-ms');
    expect(Number(after) - Number(before)).toBe(7 * 24 * 3600 * 1000);
  });

  // Fix round 1, finding 2: the pinned "H and L step" spec above only ever
  // presses `2` first, so the day and month units were unexercised — the
  // reviewer changed the month branch to step by seven days (silently
  // behaving like week-stepping) and all 262 tests passed. Day and month
  // each get their own spec below.
  test('H and L step by a day when Day view is active', async ({ page }) => {
    await page.goto(app('connected'));
    await expect(page.locator('.vswitch button')).toHaveCount(5);
    await page.keyboard.press('1');
    const before = await page.locator('.col').first().getAttribute('data-start-ms');
    await page.keyboard.press('l');
    const after = await page.locator('.col').first().getAttribute('data-start-ms');
    expect(Number(after) - Number(before)).toBe(24 * 3600 * 1000);
  });

  // Fix round 1, finding 1: a bare `setMonth` overflows for a day-of-month
  // the target month doesn't have — Jan 31 `+1` used to land on Mar 3,
  // skipping February outright, and stayed wrong forever after (`H` from
  // there landed Feb 3, not back on Jan 31). This spec starts from the 31st
  // for exactly that reason — it is finding 1's regression spec and finding
  // 2's month-unit spec at once. 2024 is a leap year, so the correct,
  // clamped landing is Feb 29, not Feb 28 or Mar 3.
  test('H and L step by a month, clamped to the target month\'s last day', async ({ page }) => {
    await page.goto(app('connected'));
    await expect(page.locator('.vswitch button')).toHaveCount(5);
    await page.keyboard.press('1'); // Day view, anchored on 29 Jan (APP_MON)
    await page.keyboard.press('l'); // 30 Jan
    await page.keyboard.press('l'); // 31 Jan
    await page.keyboard.press('3'); // Month view; the anchor survives the switch (31 Jan)
    await page.keyboard.press('l'); // step +1 month from the 31st
    await page.keyboard.press('1'); // back to Day view to read the anchor off `.col`
    const shown = await page.locator('.col').getAttribute('data-start-ms');
    expect(Number(shown)).toBe(Date.UTC(2024, 1, 29)); // 29 Feb 2024 — clamped, not skipped to March
  });

  // Fix round 1, finding 4: `MonthGrid`'s own spec proves it *calls*
  // `onopen`; nothing proved App's `monthSelId`/`monthDetail`/`EventPopover`
  // wiring on the receiving end actually opens anything. The reviewer
  // replaced `onopen={openMonthEvent}` with a no-op and all 38 App specs
  // stayed green — the DoD's "clicking an event in any view opens the
  // popover" was unverified for Month.
  test('clicking an event in Month view opens the popover', async ({ page }) => {
    await page.goto(app('connected'));
    await expect(page.locator('.vswitch button')).toHaveCount(5);
    await page.keyboard.press('3');
    await page.locator('.mcell .timed').first().click();
    await expect(page.locator('.pop')).toBeVisible();
    await expect(page.locator('.pop h2')).toHaveText('Standup');
  });

  test('T returns to today', async ({ page }) => {
    await page.goto(app('connected'));
    const col = page.locator('.col').first();
    // Capture where the app opened rather than inventing a global to remember
    // it — whatever "today" is for the fixture clock, two steps forward and T
    // must land back on exactly this value. `getAttribute` is itself an
    // auto-waiting locator read, so it already doubles as the mount-stability
    // wait the other specs above take explicitly.
    const opened = await col.getAttribute('data-start-ms');
    await page.keyboard.press('l');
    await page.keyboard.press('l');
    expect(await col.getAttribute('data-start-ms')).not.toBe(opened);
    await page.getByTestId('week-body').evaluate((el) => { el.scrollTop = 0; });
    await page.keyboard.press('t');
    expect(await col.getAttribute('data-start-ms')).toBe(opened);
    // The reveal request must wait for today's payload rather than being
    // consumed by the week that was still mounted when T was pressed.
    await expect.poll(() => page.evaluate(() => {
      const viewport = document.querySelector<HTMLElement>('[data-testid="week-body"]')!;
      const marker = viewport.querySelector<HTMLElement>('.now')!;
      const box = viewport.getBoundingClientRect();
      const position = (marker.getBoundingClientRect().top - box.top) / box.height;
      return Math.abs(position - 0.45);
    })).toBeLessThan(0.07);
  });

  test('Today recenters NOW in Week and Day, even when already on today', async ({ page }) => {
    await page.clock.setFixedTime(APP_MON + 16 * 3_600_000);
    await page.goto(app('connected'));

    const hideNow = () => page.getByTestId('week-body').evaluate((el) => { el.scrollTop = 0; });
    const nowPosition = () => page.evaluate(() => {
      const viewport = document.querySelector<HTMLElement>('[data-testid="week-body"]')!;
      const marker = viewport.querySelector<HTMLElement>('.now')!;
      const box = viewport.getBoundingClientRect();
      return (marker.getBoundingClientRect().top - box.top) / box.height;
    });
    const expectCentered = async () => {
      await expect.poll(async () => Math.abs((await nowPosition()) - 0.45))
        .toBeLessThan(0.07);
    };

    // Already on today's week: changing the anchor alone would be a no-op, so
    // this proves the key itself carries an explicit reveal request.
    await hideNow();
    await page.keyboard.press('t');
    await expectCentered();

    // The header button is the same action, including on a second invocation.
    await hideNow();
    await page.getByRole('button', { name: 'Today', exact: true }).click();
    await expectCentered();

    // Day uses the same clock grid with one column and keeps the same contract.
    await page.keyboard.press('1');
    await expect(page.locator('.col')).toHaveCount(1);
    await hideNow();
    await page.keyboard.press('t');
    await expectCentered();
  });

  // Own spec, guarding the keyboard trap (not in the brief's five): the event
  // popover has RSVP buttons and a description, and a stray `3` while one of
  // those buttons has focus must not switch views out from under it.
  test('typing keys are ignored while the event popover has focus', async ({ page }) => {
    await page.goto(app('connected'));
    await page.locator('.ev').click();
    await expect(page.locator('.pop')).toBeVisible();
    await page.getByRole('button', { name: 'Yes' }).focus();
    await page.keyboard.press('3');
    await expect(page.locator('.mrow')).toHaveCount(0);
    await expect(page.locator('.pop')).toBeVisible();
  });

  /**
   * An occurrence-scoped RSVP writes an *exception row*, and the block on
   * screen keeps carrying the master's id until the payload reloads — so a
   * reopened popover read the master's own answer, eternally the old one
   * (seen live, 2026-08-11: decline, reopen, "Yes", decline again). The
   * restyle override bridges the pixels; the reload is what fixes identity,
   * and it is the reload this asserts.
   */
  test('an RSVP is followed by a payload reload, not only a restyle', async ({ page }) => {
    await page.goto(app('connected'));
    await page.locator('.ev').click();
    await expect(page.locator('.pop')).toBeVisible();
    // Counted with the popover already open, so the startup fetch is fully
    // in the tally — sampled at goto, the startup fetch alone satisfied the
    // poll and the test passed with the reload deleted (caught red-handed).
    const before = (await callsTo(page, 'get_week')).length;
    await page.getByRole('button', { name: 'No' }).click();
    await expect
      .poll(async () => (await callsTo(page, 'get_week')).length, {
        message: 'the week payload must be re-fetched after a successful RSVP',
      })
      .toBeGreaterThan(before);
  });

  // Task 10: creating, editing and deleting, wired into the views.
  //
  // Every one of these drives the real `App` against the stubbed IPC layer and
  // asserts on the *arguments* a write command was given, because that is where
  // this branch's one unrecoverable mistake lives: `occurrenceStartMs` has to be
  // the clicked block's own `start_ms` and never `detail.start_ms`, which for a
  // series is the master's DTSTART. Both values type-check, both read
  // correctly, and the wrong one silently rewrites or deletes occurrence #0 with
  // `sendUpdates=all` behind it. See `eventdetail.ts`.

  /** Every argument list the app has passed to `cmd`, in order. */
  const callsTo = (page: Page, cmd: string): Promise<any[]> =>
    page.evaluate(
      (c) => window.__harness.calls.filter((call) => call.cmd === c).map((call) => call.args),
      cmd,
    );

  /** The `writable` scenario, opened and settled: two editable events on
   *  Monday's column, and a calendar list that has actually landed.
   *
   *  Waiting for `.trigger` — the calendar picker's own button, which only
   *  exists once `calendars` is non-empty — is not decoration. `App` seeds a
   *  new event's calendar from that list, and `EventForm` snapshots the seed on
   *  mount; a form opened before `get_calendars` answered would carry `null`
   *  and refuse to save, intermittently.
   *
   *  The picker now lives behind the hamburger, so the menu has to be opened to
   *  see it and closed again afterwards. Deliberately the same signal rather
   *  than a weaker one that happens to be visible: what this waits for is
   *  `get_calendars` having answered, and nothing else in the header says so. */
  const writable = async (page: Page) => {
    await page.goto(app('writable'));
    await page.getByRole('button', { name: 'Menu' }).click();
    await expect(page.locator('.trigger')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator('.trigger')).toHaveCount(0);
    await expect(page.locator('.vswitch button')).toHaveCount(5); // mount race — see above
  };

  const block = (page: Page, title: string) => page.locator('.ev').filter({ hasText: title });
  /** An `AllDayBand` chip. A different element and a different component from
   *  `block` above — `commands::assemble_week` puts every `is_all_day` event in
   *  the band and never in a day column, so this is the *only* way to reach an
   *  all-day event's popover, and therefore its edit and delete. */
  const chip = (page: Page, title: string) => page.locator('.chip').filter({ hasText: title });
  const newForm = (page: Page) => page.getByRole('dialog', { name: 'New event' });
  const editForm = (page: Page) => page.getByRole('dialog', { name: 'Edit event' });
  const confirmPanel = (page: Page) => page.getByRole('dialog', { name: 'Delete event' });

  /**
   * **A drag writes, and it cannot notify anybody.**
   *
   * Asserted here rather than in the grid because this is the boundary the app
   * actually crosses: what the grid hands up is one thing, what reaches
   * `update_event` is another, and only the second can email a guest list.
   * (The other boundary — what reaches Google — is
   * `events::tests::a_move_sends_the_send_updates_it_was_given`, on the wire
   * with wiremock.)
   */
  test.describe('dragging an event writes without notifying anybody', () => {
    /** Grabs `title`'s block and drags it `dy` px, releasing. */
    const dragBy = async (page: Page, title: string, dy: number) => {
      const b = block(page, title).first();
      await b.scrollIntoViewIfNeeded();
      const box = await b.boundingBox();
      if (!box) throw new Error(`no box for ${title}`);
      const cx = box.x + box.width / 2;
      const cy = box.y + box.height / 2;
      await page.mouse.move(cx, cy);
      await page.mouse.down();
      await page.mouse.move(cx, cy + dy, { steps: 4 });
      await page.mouse.up();
      return { cx, cy };
    };

    test('the value sent is none, never all', async ({ page }) => {
      await writable(page);
      await dragBy(page, 'Board prep', 60);

      await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);
      const [args] = await callsTo(page, 'update_event');

      expect(args.sendUpdates).toBe('none');
      // Said twice on purpose. The line above fails if the value changes; this
      // one fails if the argument is dropped altogether and the Rust side
      // starts defaulting, which the line above would not notice.
      expect(args.sendUpdates).not.toBe('all');
    });

    test('the moved span is what gets written', async ({ page }) => {
      await writable(page);
      // Four snap steps, computed rather than guessed: a 1200px column over a
      // 24-hour day makes a 15-minute step 12.5px, so "an hour down" is 50px
      // and not the round number it looks like.
      const col = await page.locator('.col').first().boundingBox();
      if (!col) throw new Error('no column');
      await dragBy(page, 'Board prep', (col.height / 96) * 4);

      await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);
      const [args] = await callsTo(page, 'update_event');

      // `occurrenceStartMs` is the clicked block's own start — the invariant
      // every write in this file turns on — and the payload's times are the
      // ones it was dropped on, an hour later on a 1200px day.
      expect(args.fields.when.kind).toBe('timed');
      expect(args.fields.when.startMs - args.occurrenceStartMs).toBe(60 * 60_000);
      expect(args.fields.when.endMs - args.fields.when.startMs).toBe(60 * 60_000);
    });

    /**
     * A drag long enough to cross midnight writes the **next day**.
     *
     * Here because nothing else discriminates it: every other drag in this
     * block stays inside one day, where the date the payload carries is the
     * date it started on and sending the old one is indistinguishable. A
     * mutation that never applied the moved date reddened nothing until this
     * spec existed.
     *
     * Task 3's preview is vertical-only, so the block visibly runs past the
     * bottom of its column on a drag this long — a rendering limitation
     * recorded there, not a disagreement about the instant. What is written is
     * where the pointer actually is.
     */
    test('a drag past midnight writes the next day, not the same time today', async ({ page }) => {
      await writable(page);
      const col = await page.locator('.col').first().boundingBox();
      if (!col) throw new Error('no column');

      // 'Board prep' sits at 14:00; twelve hours down is 02:00 tomorrow.
      await dragBy(page, 'Board prep', (col.height / 24) * 12);

      await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);
      const [args] = await callsTo(page, 'update_event');

      expect(args.fields.when.startMs - args.occurrenceStartMs).toBe(12 * 60 * 60_000);
      // Said as a date as well as a delta, because the delta alone is what a
      // payload carrying the old date would still get right if the *time* had
      // been applied on it.
      const landedOn = await page.evaluate(
        (ms) => new Date(ms).toISOString().slice(0, 10),
        args.fields.when.startMs,
      );
      const startedOn = await page.evaluate(
        (ms) => new Date(ms).toISOString().slice(0, 10),
        args.occurrenceStartMs,
      );
      expect(landedOn).not.toBe(startedOn);
    });

    /**
     * **The absence of a call, never a no-op response.** §4 says a drop that
     * lands where it started takes no action at all, and the only way to say
     * that is to look for a request that is not there.
     */
    test('a drop where it started issues no request at all', async ({ page }) => {
      await writable(page);

      const b = block(page, 'Board prep').first();
      await b.scrollIntoViewIfNeeded();
      const box = await b.boundingBox();
      if (!box) throw new Error('no box');
      const cx = box.x + box.width / 2;
      const cy = box.y + box.height / 2;
      await page.mouse.move(cx, cy);
      await page.mouse.down();
      await page.mouse.move(cx, cy + 80, { steps: 4 });
      await page.mouse.move(cx, cy, { steps: 4 });
      await page.mouse.up();

      // Given a moment to be wrong in: `moveOccurrence` is async, so asserting
      // immediately would pass against a build that issued the write a tick
      // later.
      await page.waitForTimeout(300);
      expect(await callsTo(page, 'update_event')).toHaveLength(0);
      // And nothing was even looked up — the guard is in the grid, before any
      // of the write path is entered.
      expect(await callsTo(page, 'event_detail')).toHaveLength(0);
    });

    /**
     * §6: a drag that appears to have worked and silently did not is worse
     * than one that visibly refuses. The block is already back where it
     * started — the grid returns it on drop and only a refresh moves it — so
     * what a failure has to do is *say so*.
     */
    /** The move confirmation, by its accessible name. */
    const movePanel = (page: Page) => page.getByRole('dialog', { name: 'Move event' });

    /**
     * **No dialog where there is nobody to tell and nothing to choose.**
     *
     * §2 is explicit that silence is correct here, and it is not merely
     * tidiness: a confirmation nobody needs is one people learn to dismiss
     * without reading, which is exactly the habit the *other* cases depend on
     * them not having.
     */
    test('an event with no guests that does not repeat is moved without asking', async ({ page }) => {
      await writable(page);
      await dragBy(page, 'Board prep', 60);

      await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);
      await expect(movePanel(page)).toBeHidden();
      const [args] = await callsTo(page, 'update_event');
      expect(args.sendUpdates).toBe('none');
    });

    test('an event with guests asks, and Move without notifying sends none', async ({ page }) => {
      await writable(page);
      await dragBy(page, 'Client call', 60);

      await expect(movePanel(page)).toBeVisible();
      // Nothing is written while the question is on screen.
      expect(await callsTo(page, 'update_event')).toHaveLength(0);
      // One guest, and the person doing the moving is not one of them.
      await expect(page.getByTestId('move-guest-notice')).toContainText('1 guest');
      // A one-off: there is no scope to choose, so the chooser is not offered.
      await expect(movePanel(page).getByRole('radiogroup')).toHaveCount(0);

      await expect(
        movePanel(page).getByRole('button', { name: 'Move without notifying' }),
      ).toBeFocused();
      await page.keyboard.press('Enter');

      await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);
      const [args] = await callsTo(page, 'update_event');
      expect(args.sendUpdates).toBe('none');
    });

    /**
     * **A guest's drag reaches nobody, and the panel says so.** Google keeps
     * an invitee's copy apart from the organizer's, so moving somebody else's
     * meeting changes this calendar alone. The notify choice would be a
     * promise to mail people about a change none of them receives, so it is
     * not offered: one button, named for what it does, and `none` on the
     * wire because there is no mail to send.
     */
    test('moving somebody else\'s meeting names it as your copy and offers no notify choice',
      async ({ page }) => {
        await writable(page);
        await dragBy(page, 'Vendor review', 60);

        await expect(movePanel(page)).toBeVisible();
        await expect(page.getByTestId('move-own-copy-notice')).toContainText('only your copy');
        await expect(page.getByTestId('move-own-copy-notice')).toContainText('ana@x.com');
        await expect(page.getByTestId('move-guest-notice')).toHaveCount(0);
        await expect(
          movePanel(page).getByRole('button', { name: 'Move and notify guests' }),
        ).toHaveCount(0);

        await expect(movePanel(page).getByRole('button', { name: 'Move my copy' })).toBeFocused();
        await page.keyboard.press('Enter');

        await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);
        const [args] = await callsTo(page, 'update_event');
        expect(args.sendUpdates).toBe('none');
      });

    /**
     * The exception, kept a real choice: an organizer who ticked "guests can
     * modify" made a guest's move reach everyone, so the notify question is
     * back — and the panel says why it is.
     */
    test('when the organizer lets guests modify, the move reaches everyone and asks', async ({ page }) => {
      await writable(page);
      await dragBy(page, 'Design review', 60);

      await expect(movePanel(page)).toBeVisible();
      await expect(page.getByTestId('move-own-copy-notice')).toHaveCount(0);
      await expect(page.getByTestId('move-guest-notice')).toContainText('reaches everyone');
      await movePanel(page).getByRole('button', { name: 'Move and notify guests' }).click();

      await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);
      const [args] = await callsTo(page, 'update_event');
      expect(args.sendUpdates).toBe('all');
    });

    /**
     * **The only route to `all` in the whole drag path.** Sending mail is the
     * deliberate choice, never the default and never a side effect of a
     * gesture — which is why the *other* button is the primary one.
     */
    test('Move and notify guests is the one path that sends all', async ({ page }) => {
      await writable(page);
      await dragBy(page, 'Client call', 60);

      await movePanel(page).getByRole('button', { name: 'Move and notify guests' }).click();

      await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);
      const [args] = await callsTo(page, 'update_event');
      expect(args.sendUpdates).toBe('all');
    });

    /**
     * **Cancel issues no request at all** — the absence of a call, never a
     * no-op response or a returned success. The same shape as the no-op drop,
     * and for the same reason: a dialog that is merely *visible* proves
     * nothing about what happens when it closes.
     */
    test('Cancel issues no request at all and the block does not move', async ({ page }) => {
      await writable(page);
      const topOf = () =>
        page.evaluate(() => {
          const e = [...document.querySelectorAll('.ev')]
            .find((n) => n.getAttribute('aria-label')?.startsWith('Client call,')) as HTMLElement;
          return e.offsetTop;
        });

      const before = await topOf();
      await dragBy(page, 'Client call', 60);
      await expect(movePanel(page)).toBeVisible();

      await movePanel(page).getByRole('button', { name: 'Cancel' }).click();

      // Given a moment to be wrong in: the write is async, so asserting the
      // instant the panel closes would pass against a build that issued it a
      // tick later.
      await page.waitForTimeout(300);
      expect(await callsTo(page, 'update_event')).toHaveLength(0);
      await expect(movePanel(page)).toBeHidden();
      expect(await topOf(), 'the block returns to where it was').toBeCloseTo(before, 0);
    });

    /**
     * §3: **one dialog, never two.** An event that both repeats and has guests
     * gets the scope prompt *carrying* the notify choice, rather than a second
     * panel appearing behind the first.
     */
    test('a repeating event with guests gets exactly one dialog', async ({ page }) => {
      await writable(page);
      await dragBy(page, 'Standup', 60);

      // One panel on screen, and it is the move one — asserted as a count so a
      // second dialog stacked behind it fails here rather than passing a
      // `toBeVisible` on whichever is on top.
      await expect(page.getByRole('dialog')).toHaveCount(1);
      await expect(movePanel(page)).toBeVisible();

      // Both questions, in the one panel.
      await expect(movePanel(page).getByRole('radiogroup', { name: 'Move' })).toBeVisible();
      await expect(
        movePanel(page).getByRole('button', { name: 'Move and notify guests' }),
      ).toBeVisible();
    });

    /**
     * §3: **a series asks even with nobody on it.** The scope question is not
     * about notifying — "this one", "this and following" and "all" are three
     * different writes whether or not anybody hears about them.
     *
     * Here because the two questions are independent and every other fixture
     * answers them together: deleting the recurring half of the check reddened
     * nothing until this existed, since every repeating event also had guests.
     */
    test('a repeating event with no guests still asks which occurrences', async ({ page }) => {
      await writable(page);
      await dragBy(page, 'Gym', 60);

      await expect(movePanel(page)).toBeVisible();
      expect(await callsTo(page, 'update_event')).toHaveLength(0);
      await expect(movePanel(page).getByRole('radiogroup', { name: 'Move' })).toBeVisible();

      // And **no notify choice**, because there is nobody to notify. One
      // button that moves, and Cancel.
      await expect(page.getByTestId('move-guest-notice')).toHaveCount(0);
      await expect(
        movePanel(page).getByRole('button', { name: 'Move and notify guests' }),
      ).toHaveCount(0);

      await expect(movePanel(page).getByRole('radio', { name: 'This event' })).toBeFocused();
      await page.keyboard.press('ArrowDown');
      await expect(
        movePanel(page).getByRole('radio', { name: 'This and following' }),
      ).toBeChecked();
      await page.keyboard.press('Enter');
      await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);
      const [args] = await callsTo(page, 'update_event');
      expect(args.sendUpdates).toBe('none');
      expect(args.scope).toBe('following');
    });

    /** Escape closes a confirmation and writes nothing — the same key that
     *  closes everything else here, and the same guarantee as Cancel. */
    test('Escape closes the dialog and issues no request', async ({ page }) => {
      await writable(page);
      await dragBy(page, 'Client call', 60);
      await expect(movePanel(page)).toBeVisible();

      await page.keyboard.press('Escape');

      await expect(movePanel(page)).toBeHidden();
      await page.waitForTimeout(300);
      expect(await callsTo(page, 'update_event')).toHaveLength(0);
    });

    test('the scope chosen in the dialog is the scope written', async ({ page }) => {
      await writable(page);
      await dragBy(page, 'Standup', 60);

      await movePanel(page).getByRole('radio', { name: 'All events' }).check();
      await movePanel(page).getByRole('button', { name: 'Move without notifying' }).click();

      await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);
      const [args] = await callsTo(page, 'update_event');
      expect(args.scope).toBe('all');
      // The default is 'this', so a dialog that ignored the radio would send
      // that — this is not satisfied by whatever the panel happened to open on.
      expect(args.scope).not.toBe('this');
      expect(args.sendUpdates).toBe('none');
    });

    /** Drags `title`'s block sideways by `dx` and vertically by `dy`. */
    const dragXY = async (page: Page, title: string, dx: number, dy: number) => {
      const b = block(page, title).first();
      await b.scrollIntoViewIfNeeded();
      const box = await b.boundingBox();
      if (!box) throw new Error(`no box for ${title}`);
      const cx = box.x + box.width / 2;
      const cy = box.y + box.height / 2;
      await page.mouse.move(cx, cy);
      await page.mouse.down();
      await page.mouse.move(cx + dx, cy + dy, { steps: 6 });
      await page.mouse.up();
    };

    /** Drags an edge of `title`'s block by `dy`. */
    const dragEdge = async (page: Page, title: string, edge: 'top' | 'bottom', dy: number) => {
      const b = block(page, title).first();
      await b.scrollIntoViewIfNeeded();
      const box = await b.boundingBox();
      if (!box) throw new Error(`no box for ${title}`);
      const cx = box.x + box.width / 2;
      const cy = edge === 'top' ? box.y + 2 : box.y + box.height - 2;
      await page.mouse.move(cx, cy);
      await page.mouse.down();
      await page.mouse.move(cx, cy + dy, { steps: 6 });
      await page.mouse.up();
    };

    /**
     * Task #64: the two new gestures reach the same write with the same
     * guarantees. Asserted rather than assumed — a gesture added in front of a
     * write path is exactly where a guarantee stops applying quietly.
     */
    test('a drag to another day writes that day, still without notifying', async ({ page }) => {
      await writable(page);
      const col = await page.locator('.col').first().boundingBox();
      if (!col) throw new Error('no column');

      await dragXY(page, 'Board prep', col.width, 0);

      await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);
      const [args] = await callsTo(page, 'update_event');
      expect(args.fields.when.startMs - args.occurrenceStartMs).toBe(24 * 60 * 60_000);
      expect(args.sendUpdates).toBe('none');
    });

    test('a resize writes the new span, still without notifying', async ({ page }) => {
      await writable(page);
      const col = await page.locator('.col').first().boundingBox();
      if (!col) throw new Error('no column');

      await dragEdge(page, 'Board prep', 'bottom', (col.height / 96) * 2);

      await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);
      const [args] = await callsTo(page, 'update_event');
      // The start is untouched and the end is half an hour later — a resize,
      // not a move, all the way through to the payload.
      expect(args.fields.when.startMs).toBe(args.occurrenceStartMs);
      expect(args.fields.when.endMs - args.fields.when.startMs).toBe(90 * 60_000);
      expect(args.sendUpdates).toBe('none');
    });

    /** The gate applies to the new gestures too: a resize on an event with
     *  guests asks before it writes, exactly as a move does. */
    test('a resize on an event with guests still asks first', async ({ page }) => {
      await writable(page);
      const col = await page.locator('.col').first().boundingBox();
      if (!col) throw new Error('no column');

      await dragEdge(page, 'Client call', 'bottom', (col.height / 96) * 2);

      await expect(movePanel(page)).toBeVisible();
      expect(await callsTo(page, 'update_event')).toHaveLength(0);

      await movePanel(page).getByRole('button', { name: 'Move and notify guests' }).click();
      await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);
      const [args] = await callsTo(page, 'update_event');
      expect(args.sendUpdates).toBe('all');
    });

    test('a write that fails is reported and moves nothing', async ({ page }) => {
      await writable(page);
      await page.evaluate(() => window.__harness.failNextUpdate('that event is no longer here'));

      // `offsetTop`, not a bounding box: the box is in viewport coordinates
      // and this grid scrolls, so two measurements either side of a
      // `scrollIntoViewIfNeeded` compare different origins — which is exactly
      // what this spec did until it read -209 against 240. `offsetTop` is
      // relative to the column the block is positioned in, which is the frame
      // the drag actually moves it in.
      const topOf = () =>
        page.evaluate(() => {
          const e = [...document.querySelectorAll('.ev')]
            .find((n) => n.getAttribute('aria-label')?.startsWith('Board prep,')) as HTMLElement;
          return e.offsetTop;
        });

      const before = await topOf();
      await dragBy(page, 'Board prep', 60);

      await expect(page.locator('.err')).toBeVisible();
      await expect(page.locator('.err')).toContainText('no longer here');

      expect(await topOf(), 'a failed write must leave the block where it was')
        .toBeCloseTo(before, 0);
    });
  });

  test('n opens the form on the anchor date', async ({ page }) => {
    // The anchor, not today. The clock is frozen to midday on Mon 29 Jan, so a
    // form built from `Date.now()` would open on the 29th — two `l`s away from
    // where the user is actually looking.
    await writable(page);
    await page.keyboard.press('1'); // Day view, anchored on Mon 29 Jan (APP_MON)
    await page.keyboard.press('l'); // 30 Jan
    await page.keyboard.press('l'); // 31 Jan
    await expect(page.locator('.col'))
      .toHaveAttribute('data-start-ms', String(Date.UTC(2024, 0, 31)));

    await page.keyboard.press('n');
    await expect(newForm(page)).toBeVisible();
    await expect(newForm(page).getByLabel('Date', { exact: true })).toHaveValue('2024-01-31');
    // The *time* still comes from the clock — the next half hour after midday.
    // Both halves asserted, because a form that took the whole instant from the
    // anchor would open at midnight and look almost right.
    await expect(newForm(page).getByLabel('Start', { exact: true })).toHaveValue('12:30');
  });

  test('n is consumed instead of becoming the new event title', async ({ page }) => {
    await writable(page);
    // WebKitGTK applies a keydown's default insertion after the shortcut has
    // mounted and focused the title field. A synthetic cancelable event makes
    // that contract observable in Playwright; the empty title confirms the
    // form itself still opens cleanly.
    const prevented = await page.evaluate(() => {
      const event = new KeyboardEvent('keydown', {
        key: 'n', cancelable: true, bubbles: true,
      });
      window.dispatchEvent(event);
      return event.defaultPrevented;
    });
    expect(prevented, 'an unconsumed n is typed into the title it just focused').toBe(true);
    await expect(newForm(page)).toBeVisible();
    await expect(newForm(page).getByLabel('Title', { exact: true })).toHaveValue('');
  });

  test('clicking empty grid space opens the form at that time', async ({ page }) => {
    await writable(page);
    const col = page.locator('.col').first();
    await expect(col).toHaveAttribute('data-start-ms', String(APP_MON));
    const box = (await col.boundingBox())!;

    // 10:15 down the column: the middle of the 10:00 half hour, so this asserts
    // the snapping rule rather than pixel-exact hit testing — a dozen pixels of
    // slack either way still lands in the same slot. Well clear of both blocks,
    // which sit inside the column's top tenth.
    await col.click({ position: { x: box.width / 2, y: box.height * (10.25 / 24) } });

    await expect(newForm(page)).toBeVisible();
    await expect(newForm(page).getByLabel('Date', { exact: true })).toHaveValue('2024-01-29');
    await expect(newForm(page).getByLabel('Start', { exact: true })).toHaveValue('10:00');
    await expect(newForm(page).getByLabel('End', { exact: true })).toHaveValue('11:00');
  });

  /**
   * **The all-day draft stays visible too.** Releasing a sideways sweep used
   * to take its ribbon with it, so the span stopped being on screen at
   * exactly the moment the user was deciding about it (reported
   * 2026-08-31). It becomes the dashed not-yet-real bar instead — same
   * place, same days, joined across the columns rather than three chips.
   */
  test('a sideways sweep leaves its ribbon behind as the form’s draft', async ({ page }) => {
    await writable(page);
    const col = page.locator('.col').first();
    const box = (await col.boundingBox())!;
    const y = box.y + box.height * (10.25 / 24);
    await page.mouse.move(box.x + box.width / 2, y);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + 2 * box.width, y, { steps: 8 });
    const ribbon = (await page.locator('.sweep').first().boundingBox())!;
    await page.mouse.up();

    await expect(newForm(page)).toBeVisible();
    await expect(newForm(page).getByLabel('All day')).toBeChecked();

    const draft = page.getByTestId('form-preview');
    await expect(draft).toHaveCount(3);
    const after = (await draft.first().boundingBox())!;
    expect(Math.round(after.y), 'stays where the hand drew it').toBe(Math.round(ribbon.y));

    // Joined, not overlapping: two translucent fills stacked at a join read
    // as a bright seam down the middle of what should be one bar.
    const overlap = await page.evaluate(() => {
      const e = [...document.querySelectorAll('[data-testid="form-preview"]')];
      const a = e[0].getBoundingClientRect(), b = e[1].getBoundingClientRect();
      return +(a.right - b.left).toFixed(2);
    });
    expect(overlap).toBe(0);
  });

  test('the open form draws its event on the grid, live', async ({ page }) => {
    // A click opens the form on 10:00–11:00 — and the grid must show that
    // block already, not after Save: the whole point is watching the event
    // land while its times are still being typed (2026-08-20, by request).
    await writable(page);
    const col = page.locator('.col').first();
    const box = (await col.boundingBox())!;
    await col.click({ position: { x: box.width / 2, y: box.height * (10.25 / 24) } });
    await expect(newForm(page)).toBeVisible();

    const ghost = page.getByTestId('form-preview');
    await expect(ghost).toBeVisible();
    const before = (await ghost.boundingBox())!;

    // Stretch the end to 12:30: the ghost must follow the keystrokes. Two
    // and a half times taller, within rendering slack — the premise (an hour
    // → 2h30) is in the same fields the click spec above pins.
    await newForm(page).getByLabel('End', { exact: true }).fill('12:30');
    await expect
      .poll(async () => (await ghost.boundingBox())!.height / before.height)
      .toBeGreaterThan(2.2);

    // Cancel takes the draft with it — a ghost that outlives its form is a
    // phantom meeting.
    await page.keyboard.press('Escape');
    await expect(ghost).toHaveCount(0);
  });

  test('Ctrl+C on an event, Ctrl+V opens the form pre-filled with it', async ({ page }) => {
    // The whole chain through App, not the halves the component specs pin:
    // the popover's chord fills a buffer that outlives the popover, and the
    // paste opens a *create* on it — a draft to fine-tune, never a write.
    await writable(page);
    await block(page, 'Standup').click();
    await expect(page.getByRole('dialog', { name: 'Standup' })).toBeVisible();
    await page.keyboard.press('Control+c');
    await expect(page.locator('.pop .note')).toContainText('Copied');
    await page.keyboard.press('Escape');
    await expect(page.locator('.pop')).toHaveCount(0);

    // The paste lands on the day under the mouse (2026-08-20, by request) —
    // point at Thursday's column, a different day from both the copied
    // block's Monday and the DTSTART, so a paste that ignored the pointer
    // cannot pass by luck.
    // A fixed on-screen y rather than the column's own centre: the column is
    // the full 24 hours and scrolls, so its box centre can sit below the
    // viewport — where a real mouse can never be, and where
    // `elementFromPoint` sees nothing.
    const thu = page.locator('.col').nth(3);
    const box = (await thu.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, 400);
    await page.keyboard.press('Control+v');
    await expect(newForm(page)).toBeVisible();
    await expect(newForm(page).getByLabel('Title', { exact: true })).toHaveValue('Standup');
    await expect(newForm(page).getByLabel('Date', { exact: true })).toHaveValue('2024-02-01');
    // The copied time window, on the pointed day.
    await expect(newForm(page).getByLabel('Start', { exact: true })).toHaveValue('09:00');
    // A draft like any other: the live ghost draws it, and Escape takes it
    // away having created nothing.
    await expect(page.getByTestId('form-preview')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(newForm(page)).toHaveCount(0);
  });

  test('a paste with the mouse over no day falls back to the n answer', async ({ page }) => {
    // Parked over the app header, the hit-test finds nothing — the chord must
    // still work, and land where `n` would.
    await writable(page);
    await block(page, 'Standup').click();
    await page.keyboard.press('Control+c');
    await page.keyboard.press('Escape');

    await page.keyboard.press('n');
    const nDate = await newForm(page).getByLabel('Date', { exact: true }).inputValue();
    await page.keyboard.press('Escape');
    await expect(newForm(page)).toHaveCount(0);

    await page.mouse.move(5, 5);
    await page.keyboard.press('Control+v');
    await expect(newForm(page)).toBeVisible();
    await expect(newForm(page).getByLabel('Title', { exact: true })).toHaveValue('Standup');
    await expect(newForm(page).getByLabel('Date', { exact: true })).toHaveValue(nDate);
    await page.keyboard.press('Escape');
  });

  test('a pasted event is created with the copied invitees', async ({ page }) => {
    // The boundary that matters, same as the notify pair above: the pasted
    // form *showing* the guest list is one thing, the list reaching
    // `create_event` is another — and the unchanged-means-absent diff read a
    // pre-filled list as nothing to say, so pasted events arrived guestless
    // (2026-08-20, reported the day paste shipped).
    await writable(page);
    await block(page, 'Client call').click();
    await expect(page.getByRole('dialog', { name: 'Client call' })).toBeVisible();
    await page.keyboard.press('Control+c');
    await page.keyboard.press('Escape');

    await page.keyboard.press('Control+v');
    await expect(newForm(page)).toBeVisible();
    await newForm(page).getByRole('button', { name: 'Create', exact: true }).click();
    // Guests on board means Create asks who to tell, same as a typed list.
    await page.getByRole('button', { name: 'Create without notifying', exact: true }).click();
    await expect(newForm(page)).toHaveCount(0);

    const [args] = await callsTo(page, 'create_event');
    // Ana alone: `me@x.com` is the copier's own row (`is_self` on the
    // source), and the copier is the new event's organizer, not its guest —
    // sending it invited the user to their own copy.
    expect(args.fields.guests).toEqual([
      { email: 'ana@x.com', optional: false },
    ]);
  });

  test('Ctrl+V with nothing copied is not a key omacal answers to', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('Control+v');
    await expect(newForm(page)).toHaveCount(0);
  });

  /**
   * Task 6, at the top of the stack: **sweeping empty grid opens the form on
   * the span that was swept**, rather than creating anything silently.
   *
   * Here rather than only in `WeekGrid`'s own specs for the reason the two
   * Month/Big Year specs below give: a callback prop asserted at the component
   * proves the grid hands a span up, and says nothing about whether `App` does
   * anything with it. `oncreate={() => {}}` left the suite green once already.
   *
   * The end is the half of it that can regress on its own — the start reaches
   * the form through the same argument a click has always used, so a build that
   * dropped the span entirely would still open on 10:00 and only the **End**
   * field would disagree.
   */
  test('sweeping empty grid space opens the form on the swept span', async ({ page }) => {
    await writable(page);
    const col = page.locator('.col').first();
    await expect(col).toHaveAttribute('data-start-ms', String(APP_MON));
    // Scrolled so both ends of the sweep are on screen; the pane is ~590px of
    // a 1680px column, and it opens part-way down.
    await page.locator('[data-testid="week-body"]').evaluate((el) => {
      el.scrollTop = Math.max(0, (10.5 / 24) * el.scrollHeight - el.clientHeight / 2);
    });
    const box = (await col.boundingBox())!;
    const x = box.x + box.width / 2;
    const y = (hour: number) => box.y + (box.height * hour) / 24;

    await page.mouse.move(x, y(10));
    await page.mouse.down();
    await page.mouse.move(x, y(11), { steps: 6 });
    await page.mouse.up();

    await expect(newForm(page)).toBeVisible();
    await expect(newForm(page).getByLabel('Date', { exact: true })).toHaveValue('2024-01-29');
    await expect(newForm(page).getByLabel('Start', { exact: true })).toHaveValue('10:00');
    await expect(newForm(page).getByLabel('End', { exact: true })).toHaveValue('11:00');
  });

  /**
   * Right-click is the other spelling of "new event here" — the gesture
   * every desktop calendar answers. Same column, same slot arithmetic as the
   * left-click spec above; only the button differs.
   */
  test('right-clicking empty grid space opens the form at that time', async ({ page }) => {
    await writable(page);
    const col = page.locator('.col').first();
    await page.locator('[data-testid="week-body"]').evaluate((el) => {
      el.scrollTop = Math.max(0, (10.5 / 24) * el.scrollHeight - el.clientHeight / 2);
    });
    const box = (await col.boundingBox())!;
    await page.mouse.click(box.x + box.width / 2, box.y + (box.height * 10) / 24, { button: 'right' });

    await expect(newForm(page)).toBeVisible();
    await expect(newForm(page).getByLabel('Start', { exact: true })).toHaveValue('10:00');
  });

  /**
   * And the browser's own menu stays out of the app — except over text
   * fields, where right-click carries copy and paste. `dispatchEvent`
   * returns false when a handler called `preventDefault`, which is the
   * closest a DOM test can get to "no native menu appeared".
   */
  test('the webview context menu is suppressed, except in text fields', async ({ page }) => {
    await writable(page);
    const allowed = (selector: string) => page.evaluate((sel) => {
      const el = document.querySelector(sel)!;
      return el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }));
    }, selector);

    expect(await allowed('.col'), 'the grid must not offer Reload and View Source').toBe(false);

    await page.keyboard.press('n');
    await expect(newForm(page)).toBeVisible();
    expect(await allowed('input.title'), 'a text field keeps its copy/paste menu').toBe(true);
  });

  /**
   * **A gesture cannot outlive the grid it was made in.**
   *
   * `WeekGrid` hangs its pointer handlers off `window` while a sweep or a drag
   * is in flight, because a pointer that leaves the column must still be
   * followed. Nothing removes them if the component goes away first — switching
   * view unmounts it — so the release lands in a closure belonging to a grid
   * that is no longer on screen, and asks `App` for a form on a span from it.
   *
   * Only reachable at this level: the component's own specs never unmount it.
   */
  test('a sweep abandoned by switching view opens no form', async ({ page }) => {
    await writable(page);
    const col = page.locator('.col').first();
    const box = (await col.boundingBox())!;
    const x = box.x + box.width / 2;

    await page.mouse.move(x, box.y + box.height * (10 / 24));
    await page.mouse.down();
    await page.mouse.move(x, box.y + box.height * (11 / 24), { steps: 6 });

    await page.keyboard.press('3'); // Month — this grid unmounts
    await expect(page.locator('.mcell').first()).toBeVisible();
    await page.mouse.up();

    await expect(newForm(page), 'a grid that is gone asks for nothing').toHaveCount(0);
  });

  /**
   * The same rule for the more expensive gesture, and the reason the one above
   * is worth fixing rather than tolerating: an abandoned *drag* does not open a
   * form, it **writes** — a request to Google from a grid the user left.
   *
   * The drag path has had this since Task 3. It is asserted here now because
   * the sweep was about to be a second copy of it.
   */
  test('a drag abandoned by switching view writes nothing', async ({ page }) => {
    await writable(page);
    const b = page.locator('.ev').filter({ hasText: 'Board prep' }).first();
    await b.scrollIntoViewIfNeeded();
    const box = (await b.boundingBox())!;
    const cx = box.x + box.width / 2;
    const cy = box.y + box.height / 2;

    await page.mouse.move(cx, cy);
    await page.mouse.down();
    await page.mouse.move(cx, cy + 60, { steps: 4 });

    await page.keyboard.press('3'); // Month — this grid unmounts
    await expect(page.locator('.mcell').first()).toBeVisible();
    await page.mouse.up();

    await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(0);
  });

  // Year and Big Year keep their own counters and never touch `anchorMs`, so
  // `n` there used to open a form in the year the user had navigated *away*
  // from. That matters more in Year than anywhere else: it is the one view
  // with no empty grid space to click, so `n` is its only way to create, and a
  // substitute that lands in the wrong year is not a substitute. One spec per
  // view, because the two read different counters.
  test('n follows the year on screen in Year view, not the anchor', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('4'); // Year, re-seeded from the anchor: 2024
    await expect(page.locator('.ygrid')).toHaveAttribute('data-year', '2024');
    await page.keyboard.press('l');
    await expect(page.locator('.ygrid')).toHaveAttribute('data-year', '2025');

    await page.keyboard.press('n');
    await expect(newForm(page)).toBeVisible();
    // The anchor's own month and day — 29 January — moved into the year on
    // screen, rather than 2024's.
    await expect(newForm(page).getByLabel('Date', { exact: true })).toHaveValue('2025-01-29');
  });

  test('n follows the year on screen in Big Year view too', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('5');
    await expect(page.locator('.ribbon')).toHaveAttribute('data-year', '2024');
    await page.keyboard.press('l'); // its own bound is the real current year and next
    await expect(page.locator('.ribbon')).toHaveAttribute('data-year', '2025');

    await page.keyboard.press('n');
    await expect(newForm(page)).toBeVisible();
    await expect(newForm(page).getByLabel('Date', { exact: true })).toHaveValue('2025-01-29');
  });

  // Four entry points were delivered; the two above witness Day/Week and the
  // keyboard. These two are Month and Big Year, whose `oncreate` reaches `App`
  // through components `WeekGrid`'s specs never touch — `oncreate={() => {}}`
  // on both left the suite green. Fix round 1's finding 4 was this exact shape
  // for `onopen`, one control earlier.
  test('clicking a Month cell opens the form on that day', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('3');
    // The grid's first cell, Mon 27 Jul 2026, which carries only its own
    // number — so the middle of it is genuinely empty space.
    await page.locator('.mcell').first().locator('.newhere').click();
    await expect(newForm(page)).toBeVisible();
    await expect(newForm(page).getByLabel('Date', { exact: true })).toHaveValue('2026-07-27');
  });

  test('clicking a Big Year day opens the form on that day', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('5');
    // The ribbon is anchored on the Monday on or before 1 Jan of the year on
    // screen. The clock is frozen in Jan 2024 and 1 Jan 2024 *is* a Monday, so
    // the anchor is that day itself and row 0 column 10 is Thu 11 Jan.
    // Off-centre because a ribbon day is ~45px by ~15px with its own number in
    // the middle.
    await page.locator('.rrow').first().locator('.rday .newhere').nth(10)
      .click({ position: { x: 3, y: 3 } });
    await expect(newForm(page)).toBeVisible();
    await expect(newForm(page).getByLabel('Date', { exact: true })).toHaveValue('2024-01-11');
  });

  test('a new event lands on a calendar the user can write to', async ({ page }) => {
    // The list leads with a `reader` (a subscribed holiday calendar, the
    // ordinary case) and then a `writer`, with the user's own primary last, so
    // this id rejects both `calendars[0]` and "the first writable one".
    await writable(page);
    await page.keyboard.press('n');
    await expect(newForm(page)).toBeVisible();
    await newForm(page).getByLabel('Title', { exact: true }).fill('Lunch');
    await newForm(page).getByRole('button', { name: 'Create' }).click();
    await expect(newForm(page)).toHaveCount(0);

    const [args] = await callsTo(page, 'create_event');
    expect(args.calendarId).toBe(APP_PRIMARY_CALENDAR_ID);
    expect(args.calendarId).not.toBe(APP_READER_CALENDAR_ID);
    expect(args.fields.summary).toBe('Lunch');
  });

  test('q quick-add previews one line and creates it directly', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('q');

    const quick = page.getByRole('dialog', { name: 'Quick add event' });
    await expect(quick).toBeVisible();
    const source = quick.getByLabel('Describe the event');
    await expect(source).toBeFocused();
    await source.fill('30 min at 2pm Meet with Tim invite tim@example.com +meet');

    // The interpretation is a read-only receipt, not a compact second form.
    await expect(quick.getByText('Meet with Tim', { exact: true })).toBeVisible();
    await expect(quick.getByText('Google Meet', { exact: true })).toBeVisible();
    await expect(quick.getByText(/tim@example\.com.*invitations will be emailed/)).toBeVisible();
    await expect(quick.locator('.interpretation input, .interpretation select, .interpretation textarea'))
      .toHaveCount(0);

    await quick.getByRole('button', { name: 'Create & send 1 invite' }).click();
    await expect(quick).toHaveCount(0);

    const [args] = await callsTo(page, 'create_event');
    expect(args.calendarId).toBe(APP_PRIMARY_CALENDAR_ID);
    expect(args.sendUpdates).toBe('all');
    expect(args.fields.summary).toBe('Meet with Tim');
    expect(args.fields.guests).toEqual([{ email: 'tim@example.com', optional: false }]);
    expect(args.fields.conference).toBe('googleMeet');
    expect(args.fields.when.endMs - args.fields.when.startMs).toBe(30 * 60_000);
    expect(new Date(args.fields.when.startMs).getUTCHours()).toBe(14);
  });

  test('the header entry point opens quick-add too', async ({ page }) => {
    await writable(page);
    await page.getByRole('button', { name: 'Quick add event' }).click();
    await expect(page.getByRole('dialog', { name: 'Quick add event' })).toBeVisible();
  });

  test('Continue editing carries the interpretation into the full editor', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('q');
    const quick = page.getByRole('dialog', { name: 'Quick add event' });
    await quick.getByLabel('Describe the event').fill(
      'tomorrow 3p 45m Design review invite ana@example.com +meet',
    );
    await quick.getByRole('button', { name: 'Continue editing' }).click();

    const form = newForm(page);
    await expect(quick).toHaveCount(0);
    await expect(form).toBeVisible();
    await expect(form.getByLabel('Title', { exact: true })).toHaveValue('Design review');
    await expect(form.getByLabel('Add video call')).toHaveValue('googleMeet');
    await expect(form.locator('[data-guest="ana@example.com"]')).toBeVisible();
    await expect(form.getByText('A unique Google Meet link will be generated when you save.'))
      .toBeVisible();
  });

  test('Tab selects Continue editing: Space opens it, while Enter creates directly', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('q');
    let quick = page.getByRole('dialog', { name: 'Quick add event' });
    let source = quick.getByLabel('Describe the event');
    await source.fill('tomorrow 3p Design review');
    await source.press('Tab');
    await expect(quick.getByRole('button', { name: 'Continue editing' })).toBeFocused();
    await page.keyboard.press('Space');
    await expect(quick).toHaveCount(0);
    await expect(newForm(page)).toBeVisible();
    expect(await callsTo(page, 'create_event')).toEqual([]);

    await newForm(page).getByRole('button', { name: 'Cancel' }).click();
    await page.keyboard.press('q');
    quick = page.getByRole('dialog', { name: 'Quick add event' });
    source = quick.getByLabel('Describe the event');
    await source.fill('tomorrow 3p Send directly');
    await source.press('Tab');
    await expect(quick.getByRole('button', { name: 'Continue editing' })).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(quick).toHaveCount(0);
    await expect.poll(() => callsTo(page, 'create_event')).toHaveLength(1);
    expect((await callsTo(page, 'create_event'))[0].fields.summary).toBe('Send directly');
  });

  test('quick-add previews and creates a bounded weekly series', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('q');
    const quick = page.getByRole('dialog', { name: 'Quick add event' });
    const source = quick.getByLabel('Describe the event');
    await source.fill('MWF 9am Team sync until Sep 30');
    await expect(quick.getByText('Weekly · Mon, Wed, Fri', { exact: true })).toBeVisible();
    await expect(quick.getByText(/On Sep 30, 2024/)).toBeVisible();
    await source.press('Enter');
    const [args] = await callsTo(page, 'create_event');
    expect(args.fields.repeatEnd).toEqual({ kind: 'on', date: '2024-09-30' });
  });

  test('MWF quick-add previews and hands the same weekly pattern to creation', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('q');
    const quick = page.getByRole('dialog', { name: 'Quick add event' });
    await quick.getByLabel('Describe the event').fill('MWF at 9am Team sync');
    await expect(quick.getByText('Weekly · Mon, Wed, Fri', { exact: true })).toBeVisible();
    // The app fixture freezes time on Monday 29 Jan; Monday belongs to MWF,
    // so the first occurrence is today rather than needlessly skipping ahead.
    await expect(quick.getByText(/Mon, Jan 29, 2024.*9:00 AM/)).toBeVisible();

    await quick.getByRole('button', { name: 'Continue editing' }).click();
    const form = newForm(page);
    const repeatOn = form.getByRole('group', { name: 'Repeat on' });
    for (const name of ['Monday', 'Wednesday', 'Friday']) {
      await expect(repeatOn.getByRole('button', { name })).toHaveAttribute('aria-pressed', 'true');
    }
    await expect(repeatOn.getByRole('button', { name: 'Tuesday' }))
      .toHaveAttribute('aria-pressed', 'false');

    await form.getByRole('button', { name: 'Create' }).click();
    const [args] = await callsTo(page, 'create_event');
    expect(args.fields.repeat).toBe('weekly');
    expect(args.fields.weeklyDays).toEqual(['MO', 'WE', 'FR']);
  });

  test('Make it a zoom continues into the Zoom hook and saves a pasted link', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('q');
    const quick = page.getByRole('dialog', { name: 'Quick add event' });
    await quick.getByLabel('Describe the event').fill('tomorrow 3p 45m Design review make it a zoom');

    // Calendar credentials cannot mint a Zoom meeting, so direct creation is
    // blocked until the real link exists; the requested provider still rides
    // into the ordinary editor rather than being discarded.
    await expect(quick.getByRole('button', { name: 'Create event' })).toBeDisabled();
    await expect(quick.getByText('Paste the Zoom meeting link before creating the event.'))
      .toBeVisible();
    await quick.getByRole('button', { name: 'Continue editing' }).click();

    const form = newForm(page);
    const video = form.getByLabel('Add video call');
    await expect(video).toHaveValue('zoom');
    await expect(video.locator('option')).toHaveText([
      'Add Video Call', 'Google Meet', 'Zoom',
    ]);
    const zoom = form.getByLabel('Zoom meeting link');
    await zoom.fill('https://us02web.zoom.us/j/987654321');
    await form.getByRole('button', { name: 'Create', exact: true }).click();

    const [args] = await callsTo(page, 'create_event');
    expect(args.fields.summary).toBe('Design review');
    expect(args.fields.location).toBe('Zoom: https://us02web.zoom.us/j/987654321');
    expect(args.fields.conference).toBeUndefined();
  });

  test('Add Video Call generates Meet from an ordinary new event', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('n');
    const form = newForm(page);
    await form.getByLabel('Title', { exact: true }).fill('Planning');
    await form.getByLabel('Add video call').selectOption('googleMeet');
    await form.getByRole('button', { name: 'Create', exact: true }).click();

    const [args] = await callsTo(page, 'create_event');
    expect(args.fields.conference).toBe('googleMeet');
  });

  /**
   * **A create can invite people, and only a deliberate answer mails them.**
   *
   * Asserted here rather than in the component specs because this is the
   * boundary that matters: what the form hands up is one thing, what reaches
   * `create_event` is another, and only the second can email anybody. (The
   * other boundary — what reaches Google — is
   * `events::tests::a_create_sends_the_send_updates_it_was_given`, on the wire
   * with wiremock.)
   *
   * Both answers, because `sendUpdates` wired to a constant passes either half
   * on its own — and `'none'` is the constant it was until this task.
   */
  for (const [button, expected] of [
    ['Create without notifying', 'none'],
    ['Create and notify guests', 'all'],
  ] as const) {
    test(`${button} reaches create_event as ${expected}`, async ({ page }) => {
      await writable(page);
      await page.keyboard.press('n');
      await expect(newForm(page)).toBeVisible();
      await newForm(page).getByLabel('Title', { exact: true }).fill('Design review');
      await newForm(page).getByLabel('Add guest').fill('ana@x.com');
      await newForm(page).getByRole('button', { name: 'Add', exact: true }).click();
      await newForm(page).getByRole('button', { name: 'Create', exact: true }).click();
      await page.getByRole('button', { name: button, exact: true }).click();
      await expect(newForm(page)).toHaveCount(0);

      const [args] = await callsTo(page, 'create_event');
      expect(args.sendUpdates).toBe(expected);
      expect(args.fields.guests).toEqual([{ email: 'ana@x.com', optional: false }]);
    });
  }

  /**
   * The duplicate-create guard, from the UI side. The backend's fixed
   * sentence (events.rs `CREATED_NOT_STORED`) means "Google succeeded, only
   * the local half failed" — the event exists and the guests are mailed —
   * so `App` must run its ordinary post-write sync (which fetches the event
   * like any other) rather than stopping the way it does for a real
   * failure. Stopping is what invites the user to create the event again.
   * Both halves asserted: the banner still says what happened, and
   * `sync_now` proves the heal ran.
   */
  test('a create that reached Google heals by syncing, not by retrying', async ({ page }) => {
    await writable(page);
    await page.evaluate(() => window.__harness.failNextCreate(
      'The event was created on Google, but OmaCal could not record it locally. ' +
      'The next sync will bring it in — do not create it again.'));

    await page.keyboard.press('n');
    await newForm(page).getByLabel('Title', { exact: true }).fill('Lunch');
    await newForm(page).getByRole('button', { name: 'Create', exact: true }).click();
    await expect(newForm(page)).toHaveCount(0);

    // The heal ran: one sync, and the reload it feeds ends with the banner
    // clear — the event on the grid IS the report, and a standing "do not
    // create it again" beside a visible event would only confuse. (A heal
    // that cannot reach Google is refreshAfterWrite's own catch, whose
    // message correctly says the change was made.)
    await expect.poll(() => page.evaluate(() =>
      window.__harness.calls.filter((c) => c.cmd === 'sync_now').length)).toBe(1);
    await expect(page.locator('.err')).toHaveCount(0);
    const creates = await page.evaluate(() =>
      window.__harness.calls.filter((c) => c.cmd === 'create_event').length);
    expect(creates, 'exactly one create — the guard exists so there is never a second').toBe(1);
  });

  /** And the boundary: an ordinary failed create still stops — no sync, no
   *  refetch — because retrying a create that never happened is safe and
   *  syncing would imply something was written. */
  test('a create Google refused stops without the heal', async ({ page }) => {
    await writable(page);
    await page.evaluate(() => window.__harness.failNextCreate('this calendar is not writable from OmaCal'));

    await page.keyboard.press('n');
    await newForm(page).getByLabel('Title', { exact: true }).fill('Lunch');
    await newForm(page).getByRole('button', { name: 'Create', exact: true }).click();

    await expect(page.locator('.err')).toContainText('not writable');
    const synced = await page.evaluate(() =>
      window.__harness.calls.filter((c) => c.cmd === 'sync_now').length);
    expect(synced, 'nothing was created, so nothing may sync as if it was').toBe(0);
  });

  /**
   * The whole chain of the default-calendar setting, in one pass: chosen in
   * Settings, told to `App` through `onsettingschange`, honoured by the next
   * `n` — **without a restart**. The propagation is the part worth the test:
   * `App` reads settings once at startup, and with the callback dropped this
   * still saves correctly and lands every create on the old default until
   * the app is relaunched.
   */
  test('a default chosen in Settings is where n lands next', async ({ page }) => {
    await writable(page);
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    await modal.locator('#default-cal').selectOption({ label: 'Team' });
    await page.keyboard.press('Escape');
    await expect(modal).toHaveCount(0);

    await page.keyboard.press('n');
    await expect(newForm(page)).toBeVisible();
    await newForm(page).getByRole('button', { name: 'Calendar' }).click();
    await expect(newForm(page).getByRole('option', { name: 'Team' }))
      .toHaveAttribute('aria-selected', 'true');
  });

  test('a default meeting duration chosen in Settings is used by n immediately', async ({ page }) => {
    await writable(page);
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    await modal.getByRole('spinbutton', { name: 'Default meeting duration' }).fill('45');
    await modal.getByRole('button', { name: 'Save default meeting duration' }).click();
    await page.keyboard.press('Escape');

    await page.keyboard.press('n');
    await newForm(page).getByLabel('Title', { exact: true }).fill('Short review');
    await newForm(page).getByRole('button', { name: 'Create', exact: true }).click();

    const [args] = await callsTo(page, 'create_event');
    expect(args.fields.when.kind).toBe('timed');
    expect(args.fields.when.endMs - args.fields.when.startMs).toBe(45 * 60_000);
  });

  /**
   * A draft can be placed by hand, the way a saved event can (2026-09-02, by
   * request: "when I create a new meeting I want to be able to move it
   * visually also").
   *
   * Driven through `App` rather than `WeekGrid` alone, because the loop is
   * the point: the grid reports the gesture, the *form* owns the times, and
   * the ghost only follows because the new value comes back out again. A
   * spec against the grid by itself could pass with the form untouched,
   * which is precisely the bug worth catching — a draft that appears to move
   * and saves its old time.
   */
  test('a new event’s draft can be dragged on the grid, and the form follows', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('n');
    const form = newForm(page);
    await expect(form).toBeVisible();
    const startField = form.getByLabel('Start', { exact: true });
    const endField = form.getByLabel('End', { exact: true });
    const before = {
      start: await startField.inputValue(),
      end: await endField.inputValue(),
    };

    const ghost = page.locator('[data-testid="form-preview"]').first();
    await expect(ghost).toBeVisible();
    const box = (await ghost.boundingBox())!;
    // A column is 24 hours over its height, so an hour is height/24. Two
    // slots down — far enough to pass the 4px threshold and to land on a
    // different quarter hour whatever the rounding.
    const col = (await page.locator('.col').first().boundingBox())!;
    const hourPx = col.height / 24;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + hourPx, { steps: 6 });
    await page.mouse.up();

    // The form's own fields moved, which is the fact that survives to a save.
    await expect
      .poll(async () => await startField.inputValue(), { timeout: 3000 })
      .not.toBe(before.start);
    const after = {
      start: await startField.inputValue(),
      end: await endField.inputValue(),
    };
    expect(after.end).not.toBe(before.end);
    // A move, not a resize: the draft kept its length.
    const mins = (hhmm: string) => {
      const [h, m] = hhmm.split(':').map(Number);
      return h * 60 + m;
    };
    expect(mins(after.end) - mins(after.start)).toBe(mins(before.end) - mins(before.start));
    expect(mins(after.start)).toBeGreaterThan(mins(before.start));
  });

  test('dragging the draft’s bottom edge extends it without moving its start', async ({ page }) => {
    // The other half of the gesture, and the one a move-only implementation
    // would silently fail: `edgeAt` decides which this is from where inside
    // the ghost the press landed.
    await writable(page);
    await page.keyboard.press('n');
    const form = newForm(page);
    const startField = form.getByLabel('Start', { exact: true });
    const endField = form.getByLabel('End', { exact: true });
    const before = {
      start: await startField.inputValue(),
      end: await endField.inputValue(),
    };

    const ghost = page.locator('[data-testid="form-preview"]').first();
    const box = (await ghost.boundingBox())!;
    const col = (await page.locator('.col').first().boundingBox())!;
    const hourPx = col.height / 24;
    // Inside the bottom resize band, not the middle.
    await page.mouse.move(box.x + box.width / 2, box.y + box.height - 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height - 2 + hourPx, { steps: 6 });
    await page.mouse.up();

    await expect
      .poll(async () => await endField.inputValue(), { timeout: 3000 })
      .not.toBe(before.end);
    expect(await startField.inputValue()).toBe(before.start);
  });

  test('the dragged draft is what the create actually writes', async ({ page }) => {
    // The whole point of the loop. A ghost that moves on screen and a form
    // that saves its original time would look right and be wrong, and only a
    // spec that reads the *request* can tell those apart.
    await writable(page);
    await page.keyboard.press('n');
    const form = newForm(page);
    const startField = form.getByLabel('Start', { exact: true });
    const before = await startField.inputValue();

    const ghost = page.locator('[data-testid="form-preview"]').first();
    const box = (await ghost.boundingBox())!;
    const col = (await page.locator('.col').first().boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(
      box.x + box.width / 2, box.y + box.height / 2 + col.height / 24, { steps: 6 },
    );
    await page.mouse.up();
    await expect.poll(async () => await startField.inputValue()).not.toBe(before);
    const moved = await startField.inputValue();

    await form.getByLabel('Title', { exact: true }).fill('Dragged into place');
    await form.getByRole('button', { name: 'Create', exact: true }).click();

    const [args] = await callsTo(page, 'create_event');
    // Read back **in the page**, not in Node: the browser context runs an
    // emulated zone and the test process does not, so a clock string built
    // here would be comparing two different days' worth of hours.
    const hhmm = await page.evaluate((ms: number) => {
      const d = new Date(ms);
      const p = (n: number) => String(n).padStart(2, '0');
      return `${p(d.getHours())}:${p(d.getMinutes())}`;
    }, args.fields.when.startMs as number);
    expect(hhmm).toBe(moved);
  });

  test('the form still sits above its own draft where the two overlap', async ({ page }) => {
    // The ghost has to rise above the form's scrim to be grabbable at all,
    // which puts it one layer from the panel itself. If it ever wins that
    // comparison, the fields under it stop taking clicks — a worse bug than
    // the one being fixed, and invisible in a screenshot.
    await writable(page);
    await page.keyboard.press('n');
    const form = newForm(page);
    await expect(form).toBeVisible();
    const panel = (await form.boundingBox())!;
    const hit = await page.evaluate(({ x, y }) => {
      const el = document.elementFromPoint(x, y);
      return el?.closest('[data-testid="form-preview"]') ? 'ghost' : 'form';
    }, { x: panel.x + panel.width / 2, y: panel.y + 12 });
    expect(hit).toBe('form');
  });

  /**
   * Issue #30. omacal has no theme of its own — it wears Omarchy's — so on
   * any other desktop the dark fallback was the only answer the app had, for
   * good. The palette *application* is pinned by the `theme-changed` specs
   * above, in both directions; what this pins is that a user can ask for the
   * other one, and that asking for nothing still means what it always did.
   */
  test('the appearance can be chosen, and still defaults to the desktop', async ({ page }) => {
    await writable(page);
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    await modal.getByRole('tab', { name: 'Appearance' }).click();
    const select = modal.locator('#appearance');
    await expect(select).toHaveValue('auto');
    await expect(select.locator('option')).toHaveText([
      'Follow the desktop theme', 'Light', 'Dark',
    ]);

    await select.selectOption('light');
    const [args] = await callsTo(page, 'set_appearance');
    expect(args).toEqual({ appearance: 'light' });

    // Reopened, the select shows what was stored rather than its own default.
    await page.keyboard.press('Escape');
    await expect(modal).toHaveCount(0);
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    await modal.getByRole('tab', { name: 'Appearance' }).click();
    await expect(modal.locator('#appearance')).toHaveValue('light');
  });

  /**
   * Issue #36. The row exists, its list is asserted whole so a choice cannot
   * quietly vanish, the choice reaches the command, and the default is the
   * one every install already had: follow the desktop.
   */
  test('the window frame can be chosen, and still follows the desktop', async ({ page }) => {
    await writable(page);
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    await modal.getByRole('tab', { name: 'Appearance' }).click();
    const select = modal.locator('#window-frame');
    await expect(select).toHaveValue('auto');
    await expect(select.locator('option')).toHaveText(['Follow the desktop', 'Shown', 'Hidden']);

    await select.selectOption('shown');
    const [args] = await callsTo(page, 'set_window_frame');
    expect(args).toEqual({ frame: 'shown' });

    // Reopened, the select shows what was stored rather than its own default.
    await page.keyboard.press('Escape');
    await expect(modal).toHaveCount(0);
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    await modal.getByRole('tab', { name: 'Appearance' }).click();
    await expect(modal.locator('#window-frame')).toHaveValue('shown');
  });

  /**
   * The macOS half of #36: the backend reports no frame choice there, since
   * the overlay title bar is the frame and a row that did nothing would be
   * worse than no row. Seeded as the stub's answer rather than as a platform
   * flag, because an absent answer is all the form knows about.
   */
  test('with no frame choice to make, the row is not offered', async ({ page }) => {
    await page.addInitScript(() => {
      sessionStorage.setItem('omacal-stub-settings', JSON.stringify({ windowFrame: null }));
    });
    await writable(page);
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    await modal.getByRole('tab', { name: 'Appearance' }).click();
    // The tab still has its other rows — only the frame is missing.
    await expect(modal.locator('#appearance')).toHaveCount(1);
    await expect(modal.locator('#window-frame')).toHaveCount(0);
  });

  /**
   * The split itself, since a control can be moved back by accident as
   * easily as it was moved here: the look lives on Appearance and the
   * behaviour on General, and neither tab carries the other's rows.
   */
  test('Appearance holds the look and General the behaviour', async ({ page }) => {
    await writable(page);
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    await expect(modal.getByRole('tab')).toHaveText([
      'General', 'Appearance', 'Calendars', 'Accounts', 'Notifications',
    ]);
    const look = ['#appearance', '#window-frame', '#week-view'];
    const behaviour = ['#sync-interval', '#time-format', '#display-tz', '#start-on-login'];

    for (const id of look) await expect(modal.locator(id)).toHaveCount(0);
    for (const id of behaviour) await expect(modal.locator(id)).toHaveCount(1);

    await modal.getByRole('tab', { name: 'Appearance' }).click();
    for (const id of look) await expect(modal.locator(id)).toHaveCount(1);
    for (const id of behaviour) await expect(modal.locator(id)).toHaveCount(0);
  });

  /**
   * Issue #26. The setting exists; the default does not move. Both halves are
   * asserted here because the second is the one a future edit is likeliest to
   * break — an install that never touches this checkbox must go on hiding on
   * close, which is what keeps its reminders firing.
   */
  test('closing the window quits only once the setting says so', async ({ page }) => {
    await writable(page);
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    const box = modal.getByRole('checkbox', { name: 'Closing the window quits omacal' });
    await expect(box).not.toBeChecked();

    await box.check();
    const [args] = await callsTo(page, 'set_quit_on_close');
    expect(args).toEqual({ on: true });

    // Reopened, the box shows what was stored rather than its own default.
    await page.keyboard.press('Escape');
    await expect(modal).toHaveCount(0);
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    await expect(
      modal.getByRole('checkbox', { name: 'Closing the window quits omacal' }),
    ).toBeChecked();
  });

  /**
   * Issue #28. A self-hosted CalDAV server is named by LAN address and signed
   * into with a bare username, and the form was built for neither: the
   * identity field was `type="email"`, so the browser's own validation
   * refused to submit "plamen" and nothing was ever sent. That is the
   * *ordinary* shape of a Radicale account, so the form has to carry it as
   * far as the backend before anything can be said about the server.
   */
  test('a CalDAV server on the LAN connects with a bare username', async ({ page }) => {
    await writable(page);
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    await modal.getByRole('tab', { name: 'Accounts' }).click();
    await modal.getByRole('button', { name: 'Add CalDAV account' }).click();

    await modal.getByLabel('Server address').fill('http://192.168.1.5:5232/');
    // The rule the transport enforces, said while the address is being typed
    // rather than after a failed attempt.
    await expect(modal.getByText(/Plain http is accepted only/)).toBeVisible();
    await modal.getByLabel('Email or account name').fill('plamen');
    await modal.getByLabel('Password').fill('hunter2');
    await modal.getByRole('button', { name: 'Connect' }).click();

    await expect(modal.getByText(/plamen connected/)).toBeVisible();
    const [args] = await callsTo(page, 'connect_caldav');
    expect(args, 'the submit never reached the backend').toMatchObject({
      kind: 'caldav',
      serverUrl: 'http://192.168.1.5:5232/',
      email: 'plamen',
    });
  });

  /**
   * The picker wears the chosen calendar's colour (2026-08-10, by request):
   * names identify calendars to a reader who knows them, colour to everyone.
   * Both halves asserted — the resting colour and it *following* a change —
   * because a dot painted once at mount passes the first alone.
   */
  test('the calendar picker wears the chosen calendar’s colour', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('n');
    await expect(newForm(page)).toBeVisible();

    const dot = newForm(page).getByRole('button', { name: 'Calendar' }).locator('i');
    await expect(dot).toHaveCSS('background-color', 'rgb(91, 141, 239)'); // Personal #5b8def
    await newForm(page).getByRole('button', { name: 'Calendar' }).click();
    await newForm(page).getByRole('option', { name: 'Team' }).click();
    await expect(dot).toHaveCSS('background-color', 'rgb(47, 191, 113)'); // Team #2fbf71
  });

  /**
   * The dot answers "which calendar" for anyone looking at the *form*; the
   * draft on the grid answers it for anyone looking at the *calendar* — "so
   * it's visually more clear which calendar I'm using to create a meeting"
   * (2026-08-31, by request). Both directions of the switch are asserted:
   * this fixture's default calendar happens to share `--accent`'s blue, so
   * only the *change* can tell a wired colour from the stylesheet fallback.
   */
  test('the grid draft wears the calendar the form would write to', async ({ page }) => {
    await writable(page);
    const col = page.locator('.col').first();
    const box = (await col.boundingBox())!;
    await col.click({ position: { x: box.width / 2, y: box.height * (10.25 / 24) } });
    await expect(newForm(page)).toBeVisible();

    const ghost = page.getByTestId('form-preview');
    await expect(ghost).toBeVisible();
    await expect(ghost).toHaveCSS('border-top-color', 'rgb(91, 141, 239)'); // Personal #5b8def

    const picker = newForm(page).getByRole('button', { name: 'Calendar' });
    await picker.click();
    await newForm(page).getByRole('option', { name: 'Team' }).click();
    await expect(ghost).toHaveCSS('border-top-color', 'rgb(47, 191, 113)'); // Team #2fbf71

    // And back, so the tile is following the choice rather than latching on
    // the first change.
    await picker.click();
    await newForm(page).getByRole('option', { name: 'Personal' }).click();
    await expect(ghost).toHaveCSS('border-top-color', 'rgb(91, 141, 239)');
  });

  /**
   * The gesture is already the colour of the draft it becomes: the sweep used
   * to be `--accent` on the grounds that no calendar had been chosen, but the
   * form opens on the default one, so releasing the button recoloured the tile
   * for no reason the user could see.
   */
  test('the sweep wears the colour a create would land in', async ({ page }) => {
    await writable(page);
    // Make the default calendar one whose colour is *not* `--accent`, which
    // is the only way this assertion can fail when the wiring is missing.
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    await modal.locator('#default-cal').selectOption({ label: 'Team' });
    await page.keyboard.press('Escape');
    await expect(modal).toHaveCount(0);

    const col = page.locator('.col').first();
    const box = (await col.boundingBox())!;
    const y = box.y + box.height * (10.25 / 24);
    await page.mouse.move(box.x + box.width / 2, y);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2, y + box.height / 12, { steps: 6 });

    // Mid-gesture: the spine down the left of the ribbon, in Team's green.
    await expect(page.locator('.sweep')).toHaveCSS(
      'box-shadow', 'rgb(47, 191, 113) 2px 0px 0px 0px inset',
    );
    await page.mouse.up();

    // And the draft it becomes keeps it.
    await expect(newForm(page)).toBeVisible();
    await expect(page.getByTestId('form-preview'))
      .toHaveCSS('border-top-color', 'rgb(47, 191, 113)');
  });

  /**
   * The reminder rows through the rendered form, not only the pure layer —
   * a markup wired to nothing keeps every `eventform.spec.ts` test green.
   * The add control's default row is 15 minutes, and one added row must reach
   * `create_event` as explicit overrides (reminders spec §§2–3).
   */
  test('a reminder added in the form reaches the create', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('n');
    await expect(newForm(page)).toBeVisible();
    await newForm(page).getByLabel('Title', { exact: true }).fill('Lunch');
    await newForm(page).getByRole('button', { name: '+ Add notification' }).click();
    await expect(newForm(page).getByLabel('Reminder amount')).toHaveValue('15');
    await newForm(page).getByRole('button', { name: 'Create' }).click();
    await expect(newForm(page)).toHaveCount(0);

    const [args] = await callsTo(page, 'create_event');
    expect(args.fields.reminders).toEqual({
      useDefault: false,
      overrides: [{ method: 'popup', minutes: 15 }],
    });
  });

  /**
   * The form's selects opt out of the platform widget style. WebKitGTK paints
   * a native select with a light background the form's light text vanishes on
   * (seen on Omarchy, 2026-08-10); Chromium and WKWebView honour the custom
   * background, so only computed style — not a pixel on these engines — can
   * witness the rule that protects the third.
   */
  test('a select never wears the platform widget background', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('n');
    await expect(newForm(page)).toBeVisible();
    const appearance = await newForm(page)
      .getByLabel('Repeat', { exact: true })
      .evaluate((el) => getComputedStyle(el).appearance);
    expect(appearance).toBe('none');
  });

  /// The occurrence-identity property, at the top of the stack: the clicked
  /// block's own start_ms must reach the command, not detail.start_ms.
  test('editing an occurrence sends the clicked block start, not the series start', async ({ page }) => {
    await writable(page);
    const syncsBefore = (await callsTo(page, 'sync_now')).length;
    await block(page, 'Standup').click();
    await expect(page.getByRole('dialog', { name: 'Standup' })).toBeVisible();
    await page.getByRole('button', { name: 'Edit' }).click();

    // The form is anchored on the occurrence too — 1 Feb, not the series'
    // 29 Jan. That is the same value, one step earlier, and getting it wrong
    // here would make an untouched time read as a move of three days.
    await expect(editForm(page)).toBeVisible();
    await expect(editForm(page).getByLabel('Date', { exact: true })).toHaveValue('2024-02-01');

    // This event has guests, so Save asks who to tell before it saves
    // anything (guest-list spec §3). Answering is now part of saving.
    await editForm(page).getByRole('button', { name: 'Save' }).click();
    await page.getByRole('button', { name: 'Save without notifying', exact: true }).click();
    await expect(editForm(page)).toHaveCount(0);

    const [args] = await callsTo(page, 'update_event');
    expect(args.occurrenceStartMs).toBe(APP_SERIES_OCCURRENCE);
    expect(args.occurrenceStartMs).not.toBe(APP_SERIES_DTSTART);
    // **The don't-notify path, at the boundary the app actually crosses.**
    // The component specs assert what the form hands up; this is what reaches
    // the command, and only this can email anybody. It used to be `'all'`
    // unconditionally.
    expect(args.sendUpdates).toBe('none');
    // Task 9's anchoring invariant, asserted at the only place both values
    // exist together: an untouched time means these two are equal *exactly*,
    // because the Rust side reads any difference between them as a move.
    expect(args.fields.when.startMs).toBe(args.occurrenceStartMs);

    // And the edit is followed by a sync, not just a local re-read. This is
    // the *edit* path's own witness: it shares `refreshAfterWrite` with the
    // delete path, but a shared function is not a shared assertion — replacing
    // this call site alone with `reload()` left the whole suite green, and the
    // consequence is the one `updateEvent`'s doc comment names, that a `this`
    // edit against a bare master is not written back locally and the popover
    // goes on showing the old title for up to a sync interval.
    await expect
      .poll(async () => (await callsTo(page, 'sync_now')).length)
      .toBeGreaterThan(syncsBefore);
  });

  test('delete asks for confirmation and names the event', async ({ page }) => {
    await writable(page);
    await block(page, 'Standup').click();
    await page.getByRole('button', { name: 'Delete' }).click();

    await expect(confirmPanel(page)).toBeVisible();
    await expect(confirmPanel(page).locator('h2')).toContainText('Standup');
    // Asked, not done. Nothing may have gone to Google at the moment the
    // question is on screen — that is the whole of what "confirm" means, and
    // there is no undo behind it.
    expect(await callsTo(page, 'delete_event_cmd')).toEqual([]);
  });

  /**
   * A guest cannot delete somebody else's event, only their copy of it —
   * and Google's help is explicit that Calendar then tells the organizer
   * they declined. The panel says that, names the organizer, and labels the
   * button for what it does; the guest count it used to print would have
   * been a claim about who loses the event.
   */
  test('deleting somebody else\'s meeting is removing your copy, and says so', async ({ page }) => {
    await writable(page);
    await block(page, 'Vendor review').click();
    await page.getByRole('button', { name: 'Delete' }).click();

    await expect(confirmPanel(page)).toBeVisible();
    await expect(confirmPanel(page).getByTestId('delete-own-copy-notice'))
      .toContainText('off your calendar only');
    await expect(confirmPanel(page).getByTestId('delete-own-copy-notice'))
      .toContainText('tells ana@x.com you declined');
    await expect(confirmPanel(page).getByTestId('delete-guest-notice')).toHaveCount(0);
    await expect(confirmPanel(page).getByRole('button', { name: 'Delete' })).toHaveCount(0);
    await confirmPanel(page).getByRole('button', { name: 'Remove from my calendar' }).click();

    await expect.poll(() => callsTo(page, 'delete_event_cmd')).toHaveLength(1);
  });

  test('a non-recurring event offers no scope choice', async ({ page }) => {
    // Without this, the three-scope spec below passes on a confirmation that
    // always shows three radios, whatever it was given.
    await writable(page);
    await block(page, 'Board prep').click();
    await page.getByRole('button', { name: 'Delete' }).click();
    await expect(confirmPanel(page)).toBeVisible();
    await expect(confirmPanel(page).getByRole('radio')).toHaveCount(0);
  });

  test('a recurring event offers all three scopes', async ({ page }) => {
    await writable(page);
    await block(page, 'Standup').click();
    await page.getByRole('button', { name: 'Delete' }).click();

    const panel = confirmPanel(page);
    await expect(panel.getByRole('radio')).toHaveCount(3);
    await expect(panel.getByRole('radio', { name: 'This event' })).toBeChecked();
    await expect(panel.getByRole('radio', { name: 'This and following' })).toHaveCount(1);
    await expect(panel.getByRole('radio', { name: 'All events' })).toHaveCount(1);
  });

  test('deleting an occurrence sends the clicked block start, and syncs after it', async ({ page }) => {
    await writable(page);
    const syncsBefore = (await callsTo(page, 'sync_now')).length;

    await block(page, 'Standup').click();
    await page.getByRole('button', { name: 'Delete' }).click();
    await confirmPanel(page).getByRole('button', { name: 'Delete' }).click();
    await expect(confirmPanel(page)).toHaveCount(0);

    const [args] = await callsTo(page, 'delete_event_cmd');
    expect(args.scope).toBe('this');
    expect(args.occurrenceStartMs).toBe(APP_SERIES_OCCURRENCE);
    expect(args.occurrenceStartMs).not.toBe(APP_SERIES_DTSTART);

    // A `this` delete against a bare master patches a Google resource this app
    // has no row for, so the backend correctly skips its local write-back and
    // the block stays on screen. Re-reading the database cannot find that out;
    // only a sync can, which is why the refresh after a write is a sync.
    await expect
      .poll(async () => (await callsTo(page, 'sync_now')).length)
      .toBeGreaterThan(syncsBefore);
  });

  // Whole-branch review, finding 3: Task 10 wired `AllDayBand` chips into the
  // same popover -> edit/delete route as `EventBlock` (`WeekGrid.svelte`,
  // `onopen={openPopover}`), and nothing witnessed it end to end — searching
  // this file for `all_day`/`all-day`/`AllDay` returned nothing, and
  // `AllDayBand`'s own specs are screenshots and chip corners, which never
  // click a chip at all.
  //
  // It matters more than an ordinary gap. An all-day event has no `EventBlock`
  // to fall back on, so a chip is its whole surface; and the all-day edit path
  // is exactly where this branch's *pinned* date-boundary defect lives
  // (`eventform.spec.ts`), so the one route that reaches a known-shipped bug
  // was the one route with no App-level witness. Both specs assert on
  // `occurrenceStartMs`, for the reason the block above gives: it must be the
  // chip's own day and never `detail.start_ms`, which here is the all-day
  // series' DTSTART two days earlier.

  test('editing from an all-day chip sends the chip\'s own day', async ({ page }) => {
    await writable(page);
    await chip(page, 'Diwali').click();
    await expect(page.getByRole('dialog', { name: 'Diwali' })).toBeVisible();
    await page.getByRole('button', { name: 'Edit' }).click();

    // The form is anchored on the clicked day too — Wed 31 Jan, not the
    // series' Mon 29 Jan — which is the same value one step earlier, and the
    // step at which getting it wrong turns an untouched date into a two-day
    // move of every following occurrence. `First day`, not `Date`: an all-day
    // event's form renders the date pair under its own labels (see
    // `EventForm.svelte`), which is itself a small proof that the all-day
    // branch is the one on screen.
    await expect(editForm(page)).toBeVisible();
    await expect(editForm(page).getByLabel('First day', { exact: true }))
      .toHaveValue('2024-01-31');

    await editForm(page).getByRole('button', { name: 'Save' }).click();
    await page.getByRole('button', { name: 'Save without notifying', exact: true }).click();
    await expect(editForm(page)).toHaveCount(0);

    const [args] = await callsTo(page, 'update_event');
    expect(args.occurrenceStartMs).toBe(APP_ALLDAY_OCCURRENCE);
    expect(args.occurrenceStartMs).not.toBe(APP_ALLDAY_SERIES_DTSTART);
  });

  test('deleting from an all-day chip sends the chip\'s own day', async ({ page }) => {
    await writable(page);
    await chip(page, 'Diwali').click();
    await expect(page.getByRole('dialog', { name: 'Diwali' })).toBeVisible();
    await page.getByRole('button', { name: 'Delete' }).click();

    await expect(confirmPanel(page)).toBeVisible();
    await confirmPanel(page).getByRole('button', { name: 'Delete' }).click();
    await expect(confirmPanel(page)).toHaveCount(0);

    const [args] = await callsTo(page, 'delete_event_cmd');
    // `'this'` is the confirmation's own default, and it is the scope that
    // makes the day matter most: aimed at the DTSTART it removes the series'
    // *first* occurrence rather than the one whose chip was clicked, and mails
    // every guest about it.
    expect(args.scope).toBe('this');
    expect(args.occurrenceStartMs).toBe(APP_ALLDAY_OCCURRENCE);
    expect(args.occurrenceStartMs).not.toBe(APP_ALLDAY_SERIES_DTSTART);
  });

  test('cancelling the confirmation with Enter deletes nothing', async ({ page }) => {
    await writable(page);
    await block(page, 'Board prep').click();
    await page.getByRole('button', { name: 'Delete' }).click();
    await expect(confirmPanel(page).getByRole('button', { name: 'Cancel' })).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(confirmPanel(page)).toHaveCount(0);
    expect(await callsTo(page, 'delete_event_cmd')).toEqual([]);
  });

  /**
   * **The only path from a save to `sendUpdates=all`.**
   *
   * Its sibling above witnesses the don't-notify path; without this one, "never
   * notify" would satisfy it and the choice would be a choice in name. Asserted
   * at the command, because that is the boundary that can email a guest list.
   */
  test('Save and notify guests is the one way a save mails anybody', async ({ page }) => {
    await writable(page);
    await block(page, 'Standup').click();
    await page.getByRole('button', { name: 'Edit' }).click();
    await expect(editForm(page)).toBeVisible();

    await editForm(page).getByRole('button', { name: 'Save' }).click();
    await page.getByRole('button', { name: 'Save and notify guests', exact: true }).click();

    await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);
    const [args] = await callsTo(page, 'update_event');
    expect(args.sendUpdates).toBe('all');
  });

  /**
   * The same fact on the save path: editing somebody else's meeting saves
   * your copy, and the panel says so instead of offering to notify people
   * whose copies do not change. Asserted at the command as well — the one
   * button sends `none`, and nothing on this path can send `all`.
   */
  test('saving an edit to somebody else\'s meeting names it as your copy', async ({ page }) => {
    await writable(page);
    await block(page, 'Vendor review').click();
    await page.getByRole('button', { name: 'Edit' }).click();
    await expect(editForm(page)).toBeVisible();

    await editForm(page).getByRole('button', { name: 'Save' }).click();
    const panel = page.getByRole('dialog', { name: 'Save event' });
    await expect(panel.getByTestId('save-own-copy-notice')).toContainText('only your copy');
    await expect(panel.getByRole('button', { name: 'Save and notify guests' })).toHaveCount(0);
    await panel.getByRole('button', { name: 'Save my copy' }).click();

    await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);
    const [args] = await callsTo(page, 'update_event');
    expect(args.sendUpdates).toBe('none');
  });

  /** §3 again, from the side that matters most: **Cancel writes nothing at
   *  all** — witnessed by the absence of a call, never by a visible dialog. */
  test('cancelling the notify choice issues no write', async ({ page }) => {
    await writable(page);
    await block(page, 'Standup').click();
    await page.getByRole('button', { name: 'Edit' }).click();
    await editForm(page).getByRole('button', { name: 'Save' }).click();

    await page.getByRole('dialog', { name: 'Save event' })
      .getByRole('button', { name: 'Cancel' }).click();

    await expect(editForm(page), 'the form keeps what was typed into it').toBeVisible();
    expect(await callsTo(page, 'update_event')).toHaveLength(0);
  });

  /**
   * A guest added in the form reaches `update_event` as the whole list.
   *
   * The end-to-end witness that the guest editor is wired to the write path at
   * all: the component specs assert what the form hands up, and `oncreate`-
   * shaped gaps between a form and `App` have shipped on this project before.
   */
  test('a guest added in the form reaches the command as the whole list', async ({ page }) => {
    await writable(page);
    await block(page, 'Standup').click();
    await page.getByRole('button', { name: 'Edit' }).click();
    await editForm(page).getByLabel('Add guest', { exact: true }).fill('dan@x.com');
    await editForm(page).getByRole('button', { name: 'Add', exact: true }).click();

    await editForm(page).getByRole('button', { name: 'Save' }).click();
    await page.getByRole('button', { name: 'Save without notifying', exact: true }).click();

    await expect.poll(() => callsTo(page, 'update_event')).toHaveLength(1);
    const [args] = await callsTo(page, 'update_event');
    // Everyone, not just the new one: `attendees` is a whole-list replace, so a
    // payload carrying only the addition removes the rest of the room.
    expect(args.fields.guests.map((g: any) => g.email)).toEqual(
      ['ana@x.com', 'petya@x.com', 'me@x.com', 'dan@x.com'],
    );
  });

  test('editing from the Month view popover reaches the form too', async ({ page }) => {
    // Month and Big Year do not go through `WeekGrid`: `App` renders a second
    // `EventPopover` of its own for them, so its Edit/Delete wiring is
    // separate code that every spec above leaves untouched. This is finding 4
    // of Fix round 1 again, one control further on — that one caught
    // `onopen={() => {}}` on this exact popover with 38 App specs still green.
    await writable(page);
    await page.keyboard.press('3');
    await page.locator('.mcell .timed').first().click();
    await expect(page.getByRole('dialog', { name: 'Standup' })).toBeVisible();
    await page.getByRole('button', { name: 'Edit' }).click();

    await expect(editForm(page)).toBeVisible();
    // And the popover it was opened from is gone, rather than left behind its
    // own scrim under the form — two stacked modals, each claiming
    // `aria-modal`, with the one underneath unreachable.
    await expect(page.getByRole('dialog', { name: 'Standup' })).toHaveCount(0);
    // The clicked block's own instant, carried through `App`'s own
    // `gridSelStart`/`gridSelEnd`. `busyDayMonth` builds its Standup at
    // `BUSY_DAY_START_MS + 9h`, and that constant is now the busy cell's own
    // midnight (see its comment in fixtures.ts), so under the project's
    // `timezoneId: 'UTC'` the form reads 09:00.
    //
    // The *date* is what makes this discriminating: this event's detail is a
    // weekly master whose DTSTART is Mon **3** Aug, so `App` handing the form
    // `gridDetail.start_ms` — the Plan 2 defect, in its second instance —
    // shows 2026-08-03 here. The End field binds `gridSelEnd` the same way.
    await expect(editForm(page).getByLabel('Date', { exact: true })).toHaveValue('2026-08-10');
    await expect(editForm(page).getByLabel('Start', { exact: true })).toHaveValue('09:00');
    await expect(editForm(page).getByLabel('End', { exact: true })).toHaveValue('09:30');
  });

  test('the views do not move under an open form', async ({ page }) => {
    // Focus is moved to a control *outside* the panel first, and that is the
    // whole spec. `isTypingTarget` already drops a view key aimed at an
    // `<input>` or at anything inside `.pop`, and the form takes focus on its
    // title field on mount — so a spec that presses `3` straight after opening
    // the form passes whether or not the modal guard exists. (Written that way
    // first; removing the guard left it green.)
    //
    // Reachable without contrivance: the panel has no focus trap, so tabbing
    // past its Create button lands on whatever is behind the scrim, and a bare
    // `3` from there would switch the view under an open form.
    await writable(page);
    await page.keyboard.press('n');
    await expect(newForm(page)).toBeVisible();

    await page.getByRole('button', { name: 'Today' }).focus();
    await page.keyboard.press('3');
    await expect(page.locator('.mrow')).toHaveCount(0);
    await expect(newForm(page)).toBeVisible();
  });

  // --- Search (spec §1, §6, §7) --------------------------------------------

  const search = (page: Page) => page.getByRole('dialog', { name: 'Search' });
  const field = (page: Page) => page.getByLabel('Search events');
  const results = (page: Page) => search(page).getByRole('listitem');

  test('slash opens search, and Escape closes it leaving the view alone', async ({ page }) => {
    // §1: the calendar is still behind it, and closing without choosing leaves
    // the user exactly where they were. The month title is the witness — it is
    // what moves when the anchor does.
    await writable(page);
    const title = await page.locator('h1').textContent();

    await page.keyboard.press('/');
    await expect(search(page)).toBeVisible();
    await expect(field(page)).toBeFocused();

    await page.keyboard.press('Escape');
    await expect(search(page)).toHaveCount(0);
    expect(await page.locator('h1').textContent()).toBe(title);
  });

  test('the slash that opens search is consumed, not typed into it', async ({ page }) => {
    // Seen on WebKitGTK proper (Omarchy, 2026-08-10): the keydown's default
    // action — inserting the character — runs after the overlay has mounted
    // and focused its field, so an unconsumed `/` becomes the first character
    // of every query and "sync" is searched as "/sync". Playwright's own
    // `press` cannot reach that ordering (its insertion lands before the field
    // exists), so the field staying empty here proves nothing — the witness is
    // the contract itself: the handler must cancel the event.
    await writable(page);
    const prevented = await page.evaluate(() => {
      const e = new KeyboardEvent('keydown', { key: '/', cancelable: true, bubbles: true });
      window.dispatchEvent(e);
      return e.defaultPrevented;
    });
    expect(prevented, 'an unconsumed / is typed into the field it just focused').toBe(true);
    await expect(search(page)).toBeVisible();
    await expect(field(page)).toHaveValue('');
  });

  test('the header offers a control, not a field', async ({ page }) => {
    // The header was emptied on purpose; putting a permanent input back into
    // it would undo that.
    await writable(page);
    await expect(page.locator('header input')).toHaveCount(0);
    await page.getByRole('button', { name: 'Search' }).click();
    await expect(search(page)).toBeVisible();
  });

  test('results appear as you type, and clear when the field does', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('/');
    await field(page).fill('standup');

    await expect(results(page)).toHaveCount(2);
    await expect(results(page).first()).toContainText('Standup');

    await field(page).fill('');
    await expect(results(page)).toHaveCount(0);
  });

  test('a query that matches nothing says so', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('/');
    await field(page).fill('zzzz');
    await expect(search(page)).toContainText('Nothing matches');
  });

  /**
   * **The race, driven deliberately.**
   *
   * Type `stan`, then `board`. The first query is held and released *after*
   * the second has already answered — which is exactly the ordering that
   * overwrites current results with stale ones, and exactly what a debounce
   * makes rarer rather than impossible.
   *
   * The overlay tags each request and drops any answer that is not the latest.
   * Without that guard this shows `Standup` after the user typed `board`, with
   * no keystroke left to correct it because the field already says `board`.
   */
  test('a slow answer to an earlier query does not overwrite a later one', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('/');

    await page.evaluate(() => window.__harness.holdNextSearch());
    await field(page).fill('stan');          // held
    await field(page).fill('board');         // answers immediately
    await expect(results(page)).toHaveCount(1);
    await expect(results(page).first()).toContainText('Board prep');

    // The stale answer arrives now.
    await page.evaluate(() => window.__harness.releaseSearch());

    await expect(results(page)).toHaveCount(1);
    await expect(
      results(page).first(),
      'the superseded query must not replace what is on screen',
    ).toContainText('Board prep');
  });

  /**
   * §6: the calendar moves to that date **in the view already on screen**, the
   * popover opens, and search closes rather than lingering behind it.
   */
  test('choosing a result moves the calendar and opens the event', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('4'); // Year — the view must not change either
    await expect(page.locator('.ygrid')).toBeVisible();

    await page.keyboard.press('/');
    await field(page).fill('board');
    await results(page).first().click();

    await expect(search(page), 'search does not linger behind the popover').toHaveCount(0);
    await expect(page.getByRole('dialog', { name: 'Board prep' })).toBeVisible();
    await expect(page.locator('.ygrid'), 'still the view they were in').toBeVisible();
  });

  /**
   * §6's first half, on its own: **the calendar moves to the result's date.**
   *
   * The `Dentist` fixture sits 45 days out, in a different month from the one
   * the app opens on — which is what makes the move witnessable. Every other
   * searchable event is in the same January as the anchor, so choosing one
   * moves nothing and a version that never moved the anchor would pass.
   */
  test('choosing a result moves the calendar to that date', async ({ page }) => {
    await writable(page);
    await expect(page.locator('h1')).toHaveText('January 2024');

    await page.keyboard.press('/');
    await field(page).fill('dentist');
    await results(page).first().click();

    await expect(page.locator('h1')).toHaveText('March 2024');
  });

  /**
   * The single-key shortcuts must not fire while search is open — and the
   * *field* is not the whole of it. `isTypingTarget` already spares an input;
   * what it cannot spare is focus on a **result button**, which is one Tab
   * away and where `4` would otherwise switch view out from under the panel.
   */
  test('a view shortcut does nothing while search is open', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('/');
    await field(page).fill('board');
    // The **button**, not the `<li>` around it. A list item is not focusable,
    // so focusing one leaves focus in the field — where `isTypingTarget`
    // already spares the key, and the guard under test never runs. That is
    // what the first version of this spec did, and a mutation is what said so.
    await results(page).first().getByRole('button').focus();
    await expect(field(page)).not.toBeFocused();

    await page.keyboard.press('4');

    await expect(search(page), 'search must still be open').toBeVisible();
    await expect(page.locator('.ygrid'), 'and Year must not have opened').toHaveCount(0);
  });

  /** §7: a lookup and nothing else. Opening and choosing must not write. */
  test('searching and choosing writes nothing', async ({ page }) => {
    await writable(page);
    await page.keyboard.press('/');
    await field(page).fill('board');
    await results(page).first().click();
    await expect(page.getByRole('dialog', { name: 'Board prep' })).toBeVisible();

    for (const cmd of ['update_event', 'create_event', 'delete_event_cmd', 'sync_now']) {
      expect(await callsTo(page, cmd), `${cmd} must not have been called`).toHaveLength(0);
    }
  });

  test('typing in the search field does not trigger the view shortcuts', async ({ page }) => {
    // `1`-`5`, `h`, `l`, `t`, `n` and `/` are all bare keys, and a search field
    // is full of them. Without the guard, typing "lunch" walks the calendar
    // forward, jumps to today and opens an event form.
    await writable(page);
    await page.keyboard.press('/');
    await field(page).fill('lunch');

    await expect(field(page)).toHaveValue('lunch');
    await expect(newForm(page), 'the n in lunch must not open a form').toHaveCount(0);
    await expect(search(page)).toBeVisible();
  });

  // --- The filmstrip toggle (spec §1-§6) -----------------------------------

  const toggle = (page: Page) => page.getByRole('button', { name: 'List view' });
  const rows = (page: Page) => page.locator('.srow');

  /** The app, opened and settled far enough that a bare keypress can land —
   *  same mount race every spec in this file guards against, and the same
   *  witness (`.vswitch`, which the same mount attaches the key handler with).
   *  Waiting for the toggle as well, because it is only rendered once the
   *  stored preference has been read. */
  const opened = async (page: Page, fixture = 'default') => {
    await page.goto(app(fixture));
    await expect(page.locator('.vswitch button')).toHaveCount(5);
    await expect(toggle(page)).toBeVisible();
  };

  test('F draws the period as a list, and F again puts the grid back', async ({ page }) => {
    await opened(page);
    await expect(page.locator('.ev')).toHaveCount(1);

    await page.keyboard.press('f');
    await expect(rows(page)).toHaveCount(1);
    await expect(rows(page)).toHaveText(/2024-01-29/);
    // The grid is *gone*, not merely covered: `WeekGrid`'s own scrolling body.
    await expect(page.getByTestId('week-body')).toHaveCount(0);
    await expect(page.locator('.ev')).toHaveCount(0);

    await page.keyboard.press('f');
    await expect(page.locator('.ev')).toHaveCount(1);
    await expect(rows(page)).toHaveCount(0);
  });

  test('Today reveals the NOW row in list view', async ({ page }) => {
    await page.clock.setFixedTime(APP_MON + 20 * 3_600_000);
    await page.setViewportSize({ width: 900, height: 260 });
    await opened(page);
    await page.keyboard.press('f');

    const strip = page.locator('.strip');
    const marker = strip.locator('.nowrow');
    await expect(marker).toHaveCount(1);
    await strip.evaluate((el) => {
      (el as HTMLElement).style.flex = 'none';
      (el as HTMLElement).style.height = '30px';
      el.scrollTop = 0;
    });
    await expect.poll(async () => {
      const viewport = await strip.boundingBox();
      const now = await marker.boundingBox();
      return !!viewport && !!now && now.y > viewport.y + viewport.height;
    }).toBe(true);

    await page.keyboard.press('t');
    await expect.poll(async () => {
      const viewport = await strip.boundingBox();
      const now = await marker.boundingBox();
      return !!viewport && !!now
        && now.y >= viewport.y
        && now.y + now.height <= viewport.y + viewport.height;
    }).toBe(true);
  });

  test('the control and the key are the same switch', async ({ page }) => {
    await opened(page);
    await toggle(page).click();
    await expect(toggle(page)).toHaveAttribute('aria-pressed', 'true');
    await expect(rows(page)).toHaveCount(1);

    // …and the key turns off what the click turned on, so the two really are
    // one piece of state rather than two that agree by coincidence.
    await page.keyboard.press('f');
    await expect(toggle(page)).toHaveAttribute('aria-pressed', 'false');
    await expect(rows(page)).toHaveCount(0);
  });

  test('turning it on stores it', async ({ page }) => {
    await opened(page);
    expect(await callsTo(page, 'set_list_mode')).toHaveLength(0);
    await page.keyboard.press('f');
    await expect(rows(page)).toHaveCount(1);
    expect(await callsTo(page, 'set_list_mode')).toEqual([{ on: true }]);
  });

  /**
   * **The one assertion a single session cannot make** (spec §7).
   *
   * A test that flips the toggle and looks at the screen proves a variable
   * changed. Only ending the session and starting another can tell that from a
   * stored preference — so this reloads the page, which throws away every
   * `$state` in the app and re-runs the harness from scratch, and then asserts
   * the list is *already* there before anything has been clicked.
   *
   * The stub's settings outlive the reload for exactly this reason; see
   * `loadSettings` in `harness/tauri.ts`. A stub that seeded the value instead
   * would pass this against an app that never wrote anything.
   */
  test('the choice survives a restart', async ({ page }) => {
    await opened(page);
    await page.keyboard.press('f');
    await expect(rows(page)).toHaveCount(1);

    await page.reload();
    await expect(page.locator('.vswitch button')).toHaveCount(5);
    await expect(rows(page), 'the list did not survive the reload').toHaveCount(1);
    await expect(toggle(page)).toHaveAttribute('aria-pressed', 'true');
    await expect(page.locator('.ev')).toHaveCount(0);

    // And off again, which is the half that says the *value* was stored rather
    // than the app having learned to always open as a list.
    await page.keyboard.press('f');
    await expect(page.locator('.ev')).toHaveCount(1);
    await page.reload();
    await expect(page.locator('.vswitch button')).toHaveCount(5);
    await expect(page.locator('.ev')).toHaveCount(1);
    await expect(rows(page)).toHaveCount(0);
  });

  /**
   * A choice made while the stored one is still loading is the one that stands.
   *
   * `F` works the instant `<svelte:window>` is listening, and `get_settings` is
   * a round trip. Without a supersession stamp the read — which describes the
   * world *before* the keystroke — lands afterwards and puts the calendar back,
   * having also silently disagreed with the row `set_list_mode` wrote in the
   * meantime. The user sees the key not working.
   *
   * The read is armed through `addInitScript` rather than through the harness,
   * because the one worth holding is issued on mount, before `window.__harness`
   * exists for a spec to call.
   */
  test('a choice made while the stored one is still loading is not undone by it', async ({ page }) => {
    await page.addInitScript(() => { (window as any).__holdSettings = true; });
    await page.goto(app());
    await expect(page.locator('.vswitch button')).toHaveCount(5);

    await page.keyboard.press('f');
    await expect(rows(page)).toHaveCount(1);

    // Now the parked answer lands, saying "grid" — the truth as of the question
    // it was asked, and stale by one keystroke.
    await page.evaluate(() => window.__harness.releaseSettings());
    await expect(rows(page), 'the stale read undid the keystroke').toHaveCount(1);
    await expect(toggle(page)).toHaveAttribute('aria-pressed', 'true');
    // …and the write that keystroke made is what survives to the next launch.
    expect(await callsTo(page, 'set_list_mode')).toEqual([{ on: true }]);
  });

  test('the choice follows the user across views', async ({ page }) => {
    // Spec §4: it is how you read the calendar, not a per-visit decision.
    await opened(page);
    await page.keyboard.press('f');
    await expect(rows(page)).toHaveCount(1);

    await page.keyboard.press('3'); // Month
    await expect(page.locator('.mrow'), 'Month must not draw its grid').toHaveCount(0);
    // `busyDayMonth`'s one populated cell, four timed events on Mon 10 Aug.
    await expect(page.locator('.sday')).toHaveCount(1);
    await expect(rows(page)).toHaveCount(4);

    await page.keyboard.press('1'); // Day
    await expect(page.getByTestId('week-body')).toHaveCount(0);
    await expect(page.locator('.strip')).toHaveCount(1);
  });

  /**
   * Spec §2: the toggle is **absent** in Year and Big Year rather than present
   * and inert — and `F` is absent with it, since a key that stored a preference
   * in the one view offering no way to see or undo it would be worse than the
   * control it duplicates.
   *
   * The honest probe **adds** rather than deletes (testing standard §3): the
   * rule is enforced by markup that is not there, so the mutation is `'year'`
   * and `'bigyear'` appended to `LISTABLE_VIEWS` in `filmstrip.ts`.
   */
  for (const [key, view, grid] of [['4', 'year', '.ygrid'], ['5', 'bigyear', '.ribbon']] as const) {
    test(`the toggle is absent in ${view} view, and F does nothing there`, async ({ page }) => {
      await opened(page);
      await page.keyboard.press(key);
      await expect(page.locator(grid)).toHaveCount(1);
      await expect(toggle(page)).toHaveCount(0);

      await page.keyboard.press('f');
      // Nothing on screen changed…
      await expect(page.locator(grid)).toHaveCount(1);
      await expect(page.locator('.strip')).toHaveCount(0);
      // …**and nothing was stored**, which is the half a look at the screen
      // cannot make: a view that simply has no list to draw would satisfy the
      // assertions above while the preference had already been flipped
      // underneath it. Proved by an absence, per testing standard §7.
      expect(await callsTo(page, 'set_list_mode')).toHaveLength(0);

      // Back to a view that has one, and it is still a grid.
      await page.keyboard.press('2');
      await expect(page.locator('.ev')).toHaveCount(1);
      await expect(page.locator('.strip')).toHaveCount(0);
    });
  }

  test('a row opens the same popover the grid does', async ({ page }) => {
    // Spec §6: no second way to reach an event's detail. Same `openOccurrence`,
    // so the same `event_detail` round trip and the same panel.
    await opened(page);
    await page.keyboard.press('f');
    await rows(page).first().click();
    await expect(page.getByRole('dialog', { name: weekLabel(W1) })).toBeVisible();
    expect(await callsTo(page, 'event_detail')).toHaveLength(1);
  });

  test('a drag in list mode writes nothing, because there is no drag', async ({ page }) => {
    // Spec §6: a list has no geometry to drop onto, so the handlers do not
    // exist — not disabled, absent. Witnessed by an absence (testing standard
    // §7): the same press-move-release that moves a block in the grid, and no
    // request at all.
    await opened(page);
    await page.keyboard.press('f');
    const row = rows(page).first();
    const box = (await row.boundingBox())!;

    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + 160, { steps: 12 });
    await page.mouse.up();

    expect(await callsTo(page, 'update_event')).toHaveLength(0);
    // The row is still where it was — nothing moved it, and nothing offered to.
    await expect(rows(page)).toHaveCount(1);
  });

  test('n still opens the form while the calendar is a list', async ({ page }) => {
    // Spec §6: creating still works through `n` and through the form.
    await opened(page, 'writable');
    await page.keyboard.press('f');
    await expect(page.locator('.strip')).toHaveCount(1);
    await page.keyboard.press('n');
    await expect(newForm(page)).toBeVisible();
  });

  test('edit and delete are hidden when can_edit is false', async ({ page }) => {
    // The default scenario's one event takes `detail()`'s own `can_edit: false`
    // — which is why that default is `false` (see fixtures.ts): a fixture list
    // that were editable throughout would satisfy the "shown" specs above by
    // itself, and a `can_edit` check nobody wrote would look implemented.
    await page.goto(app());
    await page.locator('.ev').click();
    await expect(page.locator('.pop')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Edit' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Delete' })).toHaveCount(0);

    // **Guest-list spec §5's `can_edit` rule, and it is this same gate.**
    // Editing the guest list lives in the edit form and nowhere else, so an
    // event that offers no Edit offers no way to reach it — asserted here
    // rather than as a second flag on the form, which would be a rule that is
    // always true in the app and therefore untestable against anything real.
    await expect(page.getByTestId('guests')).toHaveCount(0);
  });

  test('a theme-changed event repaints without a reload', async ({ page }) => {
    await page.goto(app());
    await page.evaluate(() =>
      window.__harness.emit('theme-changed', {
        bg: '#fdfdfb', surface: '#ffffff', text: '#1a1a1a',
        muted: '#6b6b70', accent: '#c04a2b', is_dark: false,
      }),
    );
    await expect
      .poll(() =>
        page.evaluate(() =>
          document.documentElement.style.getPropertyValue('--accent').trim(),
        ),
      )
      .toBe('#c04a2b');
  });

  test('the accent chooses its own ink, against the fill and not the theme', async ({ page }) => {
    // Text on an accent fill used to be `var(--bg)` — right whenever the
    // background happened to contrast with the accent, wrong the day a light
    // theme carried a light accent. Both palettes below are light; only the
    // accent differs. If the ink followed `is_dark` instead of the fill's own
    // luminance, the two assertions could not both hold.
    await page.goto(app());
    const onAccent = () => page.evaluate(() =>
      document.documentElement.style.getPropertyValue('--on-accent').trim());

    await page.evaluate(() => window.__harness.emit('theme-changed', {
      bg: '#fdfdfb', surface: '#ffffff', text: '#1a1a1a',
      muted: '#6b6b70', accent: '#e0af68', is_dark: false,
    }));
    await expect.poll(onAccent).toBe('var(--ink-on-light)');

    await page.evaluate(() => window.__harness.emit('theme-changed', {
      bg: '#fdfdfb', surface: '#ffffff', text: '#1a1a1a',
      muted: '#6b6b70', accent: '#3b2a5a', is_dark: false,
    }));
    await expect.poll(onAccent).toBe('var(--ink-on-dark)');
  });

  test('color-scheme follows the palette, so engine chrome matches the theme', async ({ page }) => {
    // The scrollbar is the one piece of the window omacal does not paint: the
    // engine draws it from `color-scheme`, which no published variable
    // reaches. Left unset it defaults to light — which is how a dark calendar
    // shipped with a white scrollbar on macOS. Inline style rather than
    // computed, like the `--accent` assertion above: `setPalette` writing it
    // onto :root is the contract under test.
    await page.goto(app());
    await expect
      .poll(() => page.evaluate(() => document.documentElement.style.colorScheme))
      .toBe('dark');

    // Both directions, because `is_dark: false` must mean light chrome — dark
    // scrollbars on a light Omarchy theme would be this bug mirrored.
    await page.evaluate(() =>
      window.__harness.emit('theme-changed', {
        bg: '#fdfdfb', surface: '#ffffff', text: '#1a1a1a',
        muted: '#6b6b70', accent: '#c04a2b', is_dark: false,
      }),
    );
    await expect
      .poll(() => page.evaluate(() => document.documentElement.style.colorScheme))
      .toBe('light');
  });
});

// The grid, the popover and the form must name the SAME day for an all-day
// event on a foreign-zone calendar — and it must be the **calendar's** day.
//
// Plan 6 deliberately did not write this spec: every app fixture was a UTC
// calendar in a UTC browser, where the stored midnight, the column it falls in
// and the date it reads back as are all one thing, so an agreement assertion
// would have been satisfied by the fixture rather than by the app. Plan 7's
// placement fix is what makes it a real claim, and this is where the three
// consumers of an all-day date meet: the chip's column comes from
// `commands::all_day_columns`, the popover's day from `EventDetail.start_date`,
// and the form's from `valueFromDetail` reading that same field. All three
// bottom out in `write::all_day_span_dates`.
//
// Both zones are load-bearing, and the pairing is asymmetric (see
// `crossZoneWeek` in fixtures.ts): `Pacific/Auckland` (+12) separates the
// *calendar* side, `Europe/Sofia` (+3) the *display* side, and a UTC browser
// separates neither — which is why this describe overrides the project's own
// `timezoneId` rather than reusing it.
test.describe('App: an all-day event on a calendar east of the display', () => {
  test.use({ timezoneId: 'Europe/Sofia' });

  test.beforeEach(async ({ page }) => {
    // Before `goto`, not after: `App` reads `Date.now()` on mount to pick the
    // week it opens on, and the harness refuses any week but the fixture's.
    await page.clock.setFixedTime(XZONE_NOW);
  });

  /** The same capture `callsTo` inside the `App` describe reads, redeclared
   *  here rather than hoisted: that one is scoped to its own describe, and a
   *  shared helper would have to move above both for one caller. */
  const weekRequests = (page: Page): Promise<any[]> =>
    page.evaluate(() =>
      window.__harness.calls.filter((c) => c.cmd === 'get_week').map((c) => c.args),
    );

  /** The columns the chip actually overlaps on screen, and each one's own
   *  `start_ms`.
   *
   *  Geometry, not `lane.start_col` read back out of the fixture: the claim is
   *  which day of the week the user sees the chip under, and the band and the
   *  body are two separate CSS grids that only line up because both reserve the
   *  same 44px gutter. Reading the number back would assert the fixture against
   *  itself and would not notice the two drifting apart.
   *
   *  A pixel of tolerance either side: a chip carries a 2px right margin and
   *  column edges are fractional, so a strict comparison would be reporting
   *  subpixel rounding rather than placement. It is far smaller than the ~176px
   *  column it would have to swallow to hide a one-column error. */
  const columnsUnderTheChip = (page: Page) =>
    page.evaluate(() => {
      const chip = document.querySelector('.chip')!.getBoundingClientRect();
      return [...document.querySelectorAll('.col')]
        .map((c) => ({
          startMs: Number((c as HTMLElement).dataset.startMs),
          box: c.getBoundingClientRect(),
        }))
        .filter(({ box }) => chip.left < box.right - 1 && chip.right > box.left + 1)
        .map(({ startMs }) => startMs);
    });

  test('the chip is drawn under the day its own calendar names, and its popover names that same day', async ({ page }) => {
    await page.goto('/tests/harness/index.html?c=App&f=cross-zone');
    await expect(page.locator('.chip')).toHaveText('Berlin trip');

    // The fixture's own premise, all four legs, read in the page so the
    // browser's real zone answers rather than Node's. Without every one of
    // these the assertions below are satisfied by the fixture rather than by
    // the app.
    const premise = await page.evaluate(
      ([stored, weekStart]) => {
        const dateIn = (ms: number, tz?: string) => {
          const p = Object.fromEntries(
            new Intl.DateTimeFormat('en-US',
              { timeZone: tz, year: 'numeric', month: '2-digit', day: '2-digit' })
              .formatToParts(new Date(ms)).map((x) => [x.type, x.value]));
          return `${p.year}-${p.month}-${p.day}`;
        };
        const wednesday = weekStart + 2 * 24 * 3_600_000;
        return {
          browserZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
          calendarReadsTheEvent: dateIn(stored, 'Pacific/Auckland'),
          displayReadsTheEvent: dateIn(stored),
          displayReadsWednesday: dateIn(wednesday),
          utcReadsWednesday: dateIn(wednesday, 'UTC'),
        };
      },
      [XZONE_STORED_START, XZONE_WEEK_START],
    );
    // `test.use` actually took. Under the project's default UTC browser this
    // whole spec would still pass its grid half and prove nothing about the
    // display side, so it is checked rather than assumed.
    expect(premise.browserZone).toBe('Europe/Sofia');
    // The stored instant really is midnight on 12 Aug in the calendar's own
    // zone — i.e. this fixture describes an all-day event that can exist.
    expect(premise.calendarReadsTheEvent).toBe(XZONE_DAY);
    // …and the display reads that same instant as the day before. A calendar
    // zone that agreed with the display could not see this defect at all.
    expect(premise.displayReadsTheEvent).toBe(XZONE_DISPLAY_MISREADING);
    // The display side. Sofia's Wednesday midnight is still Tuesday in UTC, so
    // a column's date has to be read in the display zone — from a UTC browser
    // these two are the same string and nothing separates them.
    expect(premise.displayReadsWednesday).toBe(XZONE_DAY);
    expect(premise.utcReadsWednesday).toBe(XZONE_DISPLAY_MISREADING);

    // And the week on screen is the week the fixture describes: the harness
    // refuses any other, but a refusal surfaces as an error banner rather than
    // a failure, so the request is asserted here too.
    const weeks = await weekRequests(page);
    expect(weeks.map((a) => a.weekStartMs)).toEqual([XZONE_WEEK_START]);

    // The grid half: one column, and it is the calendar's day. The old
    // placement drew this chip across columns 1 and 2 — Tue *and* Wed, a
    // two-day bar for a one-day event — so "exactly one" is as load-bearing as
    // "which one".
    const columns = await columnsUnderTheChip(page);
    expect(columns).toHaveLength(1);
    const chipColumnMs = columns[0];

    // The popover half.
    await page.locator('.chip').click();
    await expect(page.getByRole('dialog', { name: 'Berlin trip' })).toBeVisible();

    const said = await page.evaluate(
      (ms) => ({
        popover: document.querySelector('.when')!.textContent!.trim(),
        column: new Date(ms).toLocaleDateString([],
          { weekday: 'short', month: 'short', day: 'numeric' }),
      }),
      chipColumnMs,
    );

    // The agreement itself: the day the popover names *is* the day the column
    // the chip sits in names, both spelled the way the app spells them.
    expect(said.popover).toBe(said.column);
    // And an absolute value on both, so two wrong-but-equal answers cannot
    // satisfy the line above.
    expect(said.popover).toBe('Wed, Aug 12');
    // Which is the *calendar's* date, not the display's reading of the stored
    // instant — the column the chip used to be drawn under.
    expect(said.popover).not.toContain('Aug 11');
  });

  test('the edit form opens on that same day too', async ({ page }) => {
    // The third consumer of the same date, and the one an edit actually saves
    // through: `valueFromDetail` reads `detail.start_date` rather than deriving
    // a day from an instant.
    await page.goto('/tests/harness/index.html?c=App&f=cross-zone');
    await expect(page.locator('.chip')).toHaveText('Berlin trip');

    await page.locator('.chip').click();
    await page.getByRole('button', { name: 'Edit' }).click();

    const form = page.getByRole('dialog', { name: 'Edit event' });
    await expect(form).toBeVisible();
    // `First day`/`Last day`, not `Date`/`End date`: those labels are the
    // all-day branch of the form, so finding them at all proves the branch
    // under test is the one on screen.
    await expect(form.getByLabel('First day', { exact: true })).toHaveValue(XZONE_DAY);
    // Inclusive, so a one-day event names the same date twice — never the
    // exclusive 13th the stored `end_ms` points at, which is what this line
    // catches (measured: replacing `end_date` with that exclusive date fails
    // here).
    //
    // What it cannot catch, in *this* zone pair, is `endDate` going back to a
    // reading of `end_ms` in the browser's zone. Auckland's exclusive end is
    // 2026-08-12T12:00Z; Sofia reads that as the 12th, which is also the answer
    // the inclusive-end derivation gives — the wrong zone and the missing
    // "one day back" cancel exactly. That is the UI-side mirror of the ledger's
    // rule that Auckland witnesses the calendar zone but never the end of a
    // span. `eventform.spec.ts`'s "a one-day trip on a calendar west of the
    // browser keeps its last day" (New York calendar, Tokyo browser) is the
    // test that does catch it, and it fails under exactly that mutation.
    await expect(form.getByLabel('Last day', { exact: true })).toHaveValue(XZONE_DAY);
  });
});

/**
 * Height, and who is entitled to it.
 *
 * Reported from the running app: on a tall display Big Year stopped about 95px
 * short of the bottom of the window. There was no flex chain — `main` had a
 * padding and nothing else, `html`/`body`/`#app` had no height rules at all,
 * and each view sized itself off the window with its own guess at what
 * surrounded it (`calc(100vh - 150px)` in three of them, `- 190px` in the
 * ribbon). Measured against `c76de53` at 1920x1080, every one of those guesses
 * was too big, and by a different amount: Week left 42px of the window
 * unclaimed, Month 69px, Year 79px and Big Year 123px with no legend on it.
 * None of the figures moved with the viewport, which is the signature of a
 * constant rather than a layout.
 *
 * These live at App level rather than in `components.spec.ts` because the
 * chrome is the whole subject: a view mounted on its own has no `Header` above
 * it and no `main` around it, so it is the one place the property cannot be
 * observed. `components.spec.ts`'s BigYearRibbon block covers what App's own
 * `get_big_year` stub cannot — a ribbon with a legend under its rows.
 */
test.describe('App: a view claims the height the window leaves it', () => {
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(APP_NOW);
  });

  /**
   * The last y-coordinate a view is entitled to: the bottom of the *window*,
   * less `main`'s own `padding-bottom`, which is the only thing allowed to sit
   * below a view.
   *
   * Measured against `window.innerHeight`, and never against `main`'s own box.
   * A first pass at these specs did the latter — `main.getBoundingClientRect()
   * .bottom - paddingBottom`, which reads like the same thing and is not. With
   * the old rules reinstated `main` has no height of its own; it is a block
   * element sized by its contents, so "the bottom of `main`" is *defined* by
   * wherever the view happens to stop, and the difference between the two is
   * zero for any view whatsoever — including one leaving 123px of the window
   * empty underneath it. Measured, rather than reasoned about after the fact:
   * against that reference six of these seven specs passed with the defect
   * fully reinstated.
   */
  const mainContentBottom = (page: Page) =>
    page.locator('main').evaluate(
      (el) => window.innerHeight - parseFloat(getComputedStyle(el).paddingBottom),
    );

  const bottomOf = (page: Page, sel: string) =>
    page.locator(sel).evaluate((el) => el.getBoundingClientRect().bottom);

  /**
   * Open the app at `size` and switch to the view `key` selects.
   *
   * The `h1` wait is not decoration. `App` listens for keys on
   * `<svelte:window>`, so the listener does not exist until it has mounted, and
   * a `press` issued before that is delivered to a page that drops it — the
   * view never changes and the spec then times out waiting for a selector that
   * belongs to the view it asked for. Every other keyboard spec in this file
   * happens to await something first; these had nothing to await, which is how
   * it surfaced here rather than there.
   */
  async function open(page: Page, size: { width: number; height: number }, key: string) {
    await page.setViewportSize(size);
    await page.goto(app());
    await expect(page.locator('h1')).toBeVisible();
    await page.keyboard.press(key);
  }

  /** A tall window — the shape the defect was reported on. Any height well
   *  clear of what the old constants subtracted works; the point is that the
   *  slack below a view used to be the same number at every size. */
  const TALL = { width: 1280, height: 1600 };

  // Each view's last root element, which is the one that has to reach the
  // bottom, plus something inside it to wait on. `.body` for Week rather than
  // `.grid`, because WeekGrid has three roots and two of them carry `.grid` —
  // the day-name row and the scroller; the wait is on the inner selector for
  // the same reason, since `.grid` is ambiguous until Month is actually on
  // screen and a strict-mode violation is a confusing way to learn that.
  const LAST_ROOT: Array<[string, string, string, string]> = [
    ['Week', '2', '.body', '.col'],
    ['Month', '3', '.grid', '.mrow'],
    ['Year', '4', '.ygrid', '.ymonth'],
  ];

  for (const [name, key, sel, ready] of LAST_ROOT) {
    test(`${name} reaches the bottom of a tall window`, async ({ page }) => {
      await open(page, TALL, key);
      await expect(page.locator(ready).first()).toBeVisible();
      // Signed on purpose in the message: positive is the defect (a view
      // stopping short), negative would be a view overflowing `main`, and the
      // two want different fixes.
      const slack = (await mainContentBottom(page)) - (await bottomOf(page, sel));
      expect(Math.abs(slack)).toBeLessThanOrEqual(1);
    });
  }

  // The list is the fifth thing `main` can hold, and it hangs off the same
  // `flex: 1`. Not in the loop above because reaching it takes a second step —
  // the toggle — rather than a view key.
  //
  // Its own failure shape is the one Plan 11 fixed for the other four, one
  // direction over: a `.strip` that does not stretch is content-sized, so a
  // list longer than the window overflows `main` and the **window** scrolls by
  // however much it overran. `main` is `height: 100%` with `box-sizing:
  // border-box`, so that is a scrollbar on the whole app rather than on the
  // list.
  test('the list reaches the bottom of a tall window too', async ({ page }) => {
    await open(page, TALL, '2');
    await expect(page.locator('.col')).toHaveCount(7);
    await page.getByRole('button', { name: 'List view' }).click();
    await expect(page.locator('.strip')).toHaveCount(1);

    const slack = (await mainContentBottom(page)) - (await bottomOf(page, '.strip'));
    expect(Math.abs(slack)).toBeLessThanOrEqual(1);
    // And the app itself does not scroll — the list scrolls inside its own box.
    const overflowed = await page.evaluate(
      () => document.documentElement.scrollHeight > window.innerHeight + 1,
    );
    expect(overflowed, 'the window scrolls rather than the list').toBe(false);
  });

  // Big Year gets its own rather than joining the loop above, because reaching
  // the bottom is only half of it: `.ribbon` could fill the window while
  // `.rows` inside it stayed at its old 530px and left the slack one level
  // further in. This follows it all the way to the fourteenth row.
  test('Big Year fills a tall window down to its last row', async ({ page }) => {
    await open(page, TALL, '5');
    await expect(page.locator('.rrow')).toHaveCount(14);

    // `.ribbon`'s own padding is the one thing entitled to sit below the last
    // row. Read off the element rather than written here as a 4, so this says
    // "nothing but the padding" instead of "nothing but four pixels".
    const pad = await page.locator('.ribbon').evaluate(
      (el) => parseFloat(getComputedStyle(el).paddingBottom),
    );
    const lastRowBottom = await page.locator('.rrow').last().evaluate(
      (el) => el.getBoundingClientRect().bottom,
    );
    // 123px before this changed, at every window height; `pad` (4px) after.
    const unclaimed = (await mainContentBottom(page)) - lastRowBottom;
    expect(unclaimed).toBeLessThanOrEqual(pad + 1);
  });

  // Plan 4 settled this after a long argument and it is not to be traded away
  // for the fix above: a window too short for fourteen rows scrolls them
  // rather than squeezing them until the day strips disappear.
  //
  // 400px, and not the 720p the requirement is usually stated at, because at
  // 720p there is now nothing to observe. The ribbon used to be handed 530px
  // of rows there against fourteen rows' own 539px minimum, so every row sat
  // clamped at its minimum and 720p *was* the squeeze; claiming the real
  // height moves that to 649px and the rows simply fit. A 720p version of this
  // spec was written first and then deleted, because it could not fail:
  // measured against three separate mutations — `.ribbon` losing its
  // `min-height: 0`, `.rrow`/`.rdays` gaining one, and the pre-fix
  // `calc(100vh - 190px)` reinstated — it passed against all three. App's own
  // `get_big_year` stub returns a ribbon with no pills on it at all, so there
  // is nothing there that a row can be crushed *by*. That 720p still holds is
  // covered where the fixtures to make it bite live: `components.spec.ts`'s
  // "all fourteen rows fit on one screen with no scroll", against a mount box
  // this file's last spec pins to the app's own.
  //
  // Here the scroll is real, and this does fail — measured — the moment
  // `.ribbon` loses its `min-height: 0`: `.ribbon` refuses to shrink below its
  // fourteen rows, overflows `main`, and the rows run off the bottom of the
  // screen with no scroller to reach them.
  test('a window too short for the ribbon scrolls it rather than crushing it', async ({ page }) => {
    await open(page, { width: 1280, height: 400 }, '5');
    await expect(page.locator('.rrow')).toHaveCount(14);

    const overflow = await page.locator('.rows').evaluate((el) => el.scrollHeight - el.clientHeight);
    expect(overflow).toBeGreaterThan(0); // it genuinely has to scroll here

    await page.locator('.rows').evaluate((el) => { el.scrollTop = el.scrollHeight; });
    const reached = await page.evaluate(() => {
      const rows = document.querySelector('.rows')!.getBoundingClientRect();
      const last = document.querySelectorAll('.rrow')[13]!.getBoundingClientRect();
      return last.bottom - rows.bottom;
    });
    expect(Math.abs(reached)).toBeLessThanOrEqual(1);
    // Nothing here asserts the day strips are un-crushed as well, deliberately.
    // The obvious extra line — the strip being at least as tall as the day it
    // draws — could not be shown to fail on its own at this size: giving
    // `.rdays` a `min-height: 0` leaves it at 15px anyway, because `.rrow`
    // still refuses to shrink below its own contents, and giving `.rrow` one
    // too trips the reachability line above first. The crush has a spec that
    // does bite, against a fixture with pills in it to do the crushing:
    // `components.spec.ts`'s "a fully packed row keeps its day strip".
  });

  // `tests/harness/viewbox.ts` carries the one chrome constant left anywhere,
  // because a view mounted standalone has no `Header` to measure and `flex: 1`
  // against a bare `<div>` does nothing. This is what stops it going stale: the
  // app derives the same box at layout time and never states it, so the two can
  // only be compared by measuring the real thing. `.ribbon` is
  // `BigYearRibbon`'s only root and is `flex: 1`, so its height *is* the box
  // `main` leaves a view.
  //
  // If this fails, `Header` has changed height (a wrapped row, a new control)
  // and `APP_CHROME_PX` is the number to update — not this expectation.
  test('a standalone view gets the same box the app gives it', async ({ page }) => {
    await open(page, { width: 1280, height: 720 }, '5');
    await expect(page.locator('.rrow')).toHaveCount(14);
    const viewBox = await page.locator('.ribbon').evaluate(
      (el) => el.getBoundingClientRect().height,
    );
    expect(viewBox).toBeCloseTo(720 - APP_CHROME_PX, 0);
  });
});

/**
 * One band, not two.
 *
 * Reported from the running app, beside macOS Calendar: omacal had a dead
 * title strip with "omacal" centred in it and then its own header underneath,
 * where Calendar has a single band with the traffic lights inline. The fix is
 * `titleBarStyle: "Overlay"` plus `hiddenTitle` in `tauri.conf.json`, which
 * takes the strip away — and takes the only thing the window could be dragged
 * by with it, and leaves macOS drawing its three controls *over* the top-left
 * of the webview.
 *
 * So there are two properties here and they pull in opposite directions: on
 * macOS the header has to start clear of those controls, and on Linux it must
 * not, because `titleBarStyle` is a macOS-only key and Omarchy still has its
 * controls in a strip of their own. Reserving room there is a dead ~60px gap.
 * `status.overlay_titlebar` is the one thing `ui/src` knows about either
 * platform; `src-tauri/src/status.rs` decides it, and
 * `overlay_reserves_room_only_on_macos` there is the other half of this pair.
 *
 * At App level rather than in `components.spec.ts` because the geometry is the
 * subject: what has to clear the controls is the title's distance from the
 * *window's* left edge, and a `Header` mounted on its own has no `main` around
 * it and so no gutter to measure against.
 */
test.describe('App: one band, not two', () => {
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(APP_NOW);
  });

  /**
   * How far into the webview macOS's own window controls reach, in CSS pixels
   * from its top-left corner.
   *
   * Measured, not taken on trust — the figure this replaced was a guess of 78.
   * An `NSWindow` built exactly as tao builds this app's (`.fullSizeContentView`
   * with `titlebarAppearsTransparent`, which is what tauri-runtime-wry maps
   * `TitleBarStyle::Overlay` to) reports its close, minimise and zoom buttons at
   * x 7..21, 27..41 and 47..61, all of them y 6..22, on macOS 26.3.1. So 61 is
   * where the last of them ends.
   *
   * Deliberately not read from `Header.svelte`'s own padding: that number is
   * what the fix *does*, and this one is the fact it has to be right about.
   */
  const CONTROLS_RIGHT_PX = 61;

  /** Close enough to touching the zoom button to read as a mistake. */
  const MIN_CLEARANCE_PX = 8;

  /** The x the app's own gutter puts content at — `main`'s padding, and on
   *  Linux the whole of what stands between the window edge and the title. */
  const gutterLeft = (page: Page) =>
    page.locator('main').evaluate((el) => {
      const box = el.getBoundingClientRect();
      return box.left + parseFloat(getComputedStyle(el).paddingLeft);
    });

  const leftOf = (page: Page, sel: string) =>
    page.locator(sel).evaluate((el) => el.getBoundingClientRect().left);

  test('the title clears the window controls drawn over it', async ({ page }) => {
    await page.goto(app('overlay-titlebar'));
    await expect(page.locator('h1')).toBeVisible();

    const titleLeft = await leftOf(page, 'h1');
    expect(
      titleLeft,
      `the month title starts at x=${titleLeft}, under macOS's own window controls`,
    ).toBeGreaterThanOrEqual(CONTROLS_RIGHT_PX + MIN_CLEARANCE_PX);

    // And the room is made *inside* the header rather than by indenting the
    // whole app: the grid below keeps the gutter every other view is drawn to.
    expect(titleLeft).toBeGreaterThan(await gutterLeft(page));
  });

  test('no room is reserved when the window controls have a strip of their own', async ({ page }) => {
    await page.goto(app());
    await expect(page.locator('h1')).toBeVisible();

    const gutter = await gutterLeft(page);
    // The premise, without which the assertion below is satisfied by an app
    // that insets *everything* by 76px and still leaves Omarchy a dead gap:
    // the gutter itself has to sit inside the span macOS's controls would
    // occupy, so "flush with the gutter" is genuinely "no room reserved".
    expect(gutter, 'the app gutter is already past where the controls would be')
      .toBeLessThan(CONTROLS_RIGHT_PX);
    expect(
      await leftOf(page, 'h1'),
      'Omarchy has a dead gap at the left of its header',
    ).toBeCloseTo(gutter, 1);
  });

  /**
   * With the title bar gone there is nothing left to move the window by except
   * what the DOM names. Tauri's injected handler (its own
   * `window/scripts/drag.js`) walks the composed path from whatever was
   * pressed and gives up at the first interactive element, so a drag handle is
   * only a handle if it is not itself a control — and a control that became one
   * would stop being clickable.
   */
  test('the header is a drag handle, and none of its controls is', async ({ page }) => {
    await page.goto(app());
    await expect(page.locator('h1')).toBeVisible();

    const handles = await page.evaluate(() => {
      const header = document.querySelector('header')!;
      const all = [
        ...(header.hasAttribute('data-tauri-drag-region') ? [header] : []),
        ...header.querySelectorAll<HTMLElement>('[data-tauri-drag-region]'),
      ];
      return all.map((el) => {
        const b = el.getBoundingClientRect();
        return { tag: el.tagName, width: b.width, height: b.height };
      });
    });

    // Tauri's own list of what blocks a drag, from the script named above.
    const CLICKABLE = ['A', 'BUTTON', 'INPUT', 'SELECT', 'TEXTAREA', 'LABEL', 'SUMMARY'];
    const draggable = handles.filter(
      (h) => !CLICKABLE.includes(h.tag) && h.width > 0 && h.height > 0,
    );
    expect(draggable.length, `the window cannot be moved: handles are ${JSON.stringify(handles)}`)
      .toBeGreaterThan(0);

    const controls = handles.filter((h) => CLICKABLE.includes(h.tag));
    expect(controls, 'a header control was turned into a drag handle and stopped clicking')
      .toEqual([]);
  });

  /**
   * And it has to reach the very top of the window, which is a separate thing
   * from existing.
   *
   * A drag region covers its own box and nothing above it. `main` used to hold
   * 14px of `padding-top` above the header, so the topmost strip of the
   * window — the first place anyone puts the cursor to move a window, and on
   * macOS the strip the traffic lights themselves sit in — belonged to `main`
   * and did nothing. The header carries that padding now.
   *
   * The width half matters as much as the top half: a handle that reaches y=0
   * and is 120px wide is the title on its own, which is not a title bar.
   */
  test('a drag handle reaches the top edge of the window', async ({ page }) => {
    await page.goto(app());
    await expect(page.locator('h1')).toBeVisible();

    const CLICKABLE = ['A', 'BUTTON', 'INPUT', 'SELECT', 'TEXTAREA', 'LABEL', 'SUMMARY'];
    const { topmost, viewportWidth } = await page.evaluate((clickable) => {
      const header = document.querySelector('header')!;
      const all = [
        ...(header.hasAttribute('data-tauri-drag-region') ? [header] : []),
        ...header.querySelectorAll<HTMLElement>('[data-tauri-drag-region]'),
      ];
      const boxes = all
        .filter((el) => !clickable.includes(el.tagName))
        .map((el) => {
          const b = el.getBoundingClientRect();
          return { tag: el.tagName, top: b.top, width: b.width };
        })
        .sort((x, y) => x.top - y.top);
      return { topmost: boxes[0] ?? null, viewportWidth: window.innerWidth };
    }, CLICKABLE);

    expect(topmost, 'there is no non-interactive drag handle at all').not.toBeNull();
    expect(
      topmost!.top,
      `the top ${topmost!.top}px of the window cannot be grabbed to move it`,
    ).toBeLessThanOrEqual(0.5);
    // Not the full width: `main`'s horizontal gutter is still `main`'s, and the
    // outermost pixels of a window are its resize edge anyway. Half is enough
    // to tell a band from a label.
    expect(topmost!.width, 'the only handle reaching the top is too narrow to be a title bar')
      .toBeGreaterThan(viewportWidth / 2);
  });

  /**
   * The trap that `data-tauri-drag-region="deep"` would spring. `deep` hands
   * the whole subtree over, and `CalendarPopover`'s panel opens *inside* the
   * header — so its account labels, calendar names and hint text would drag
   * the window rather than be read. Only the panel's own buttons and
   * checkboxes would still work.
   *
   * `writable`, because the default scenario's `get_calendars` answers with an
   * empty list and the popover then never renders at all.
   */
  test('the calendar panel is text to read, not somewhere to drag the window from', async ({ page }) => {
    await page.goto(app('writable'));
    // Two clicks now: the picker moved behind the hamburger, and it moved
    // *into the menu* — which is still inside `<header>`, so this assertion is
    // about exactly the same ancestry it always was.
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: /^Calendars/ }).click();
    await expect(page.locator('.panel')).toBeVisible();

    const offenders = await page.locator('.panel').evaluate((panel) => {
      const named: string[] = [];
      // Up to and including <header>: an ancestor marked `deep` above the
      // panel is what would swallow it.
      for (let n: HTMLElement | null = panel as HTMLElement; n; n = n.parentElement) {
        if (n.getAttribute('data-tauri-drag-region') === 'deep') named.push(`deep on ${n.tagName}`);
        if (n.tagName === 'HEADER') break;
      }
      // And nothing inside the panel may be a handle in its own right.
      if (panel.hasAttribute('data-tauri-drag-region')) named.push('the panel itself');
      for (const el of panel.querySelectorAll('[data-tauri-drag-region]')) {
        named.push(`.panel ${el.tagName}`);
      }
      return named;
    });

    expect(offenders).toEqual([]);
  });
});

/**
 * The clock format, from the select in Settings to every place the app prints
 * a time.
 *
 * Deliberately end-to-end rather than a second table beside `timefmt.spec.ts`:
 * the arithmetic is proved there, and what can still be wrong here is the
 * wiring — a rune seeded at startup but not on change, a component still
 * carrying its own copy of `hhmm`, a ruler nobody remembered follows the same
 * preference. Each assertion below is a *different component* reading the same
 * setting, which is the only thing this file can say that the unit spec cannot.
 */
test.describe('App: the clock format', () => {
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(APP_NOW);
  });

  /** The hour ruler down the left of Week, as a plain list of its labels. */
  const ruler = (page: Page) => page.locator('[data-testid="week-body"] .gutter span');

  const chooseFormat = async (page: Page, label: string) => {
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    await modal.locator('#time-format').selectOption({ label });
    await page.keyboard.press('Escape');
    await expect(modal).toHaveCount(0);
  };

  test('the ruler and the blocks start on the 24-hour clock', async ({ page }) => {
    await page.goto(app());
    await expect(ruler(page).first()).toHaveText('00');
    await expect(ruler(page).nth(13)).toHaveText('13');
  });

  /**
   * **Without a restart.** `App` seeds the rune once at startup, so a change
   * that only ever reached the database would still pass a test that reloaded
   * the page — and would leave the running app showing the old clock until it
   * was quit. That is what `onsettingschange` is for and what this asserts.
   */
  test('choosing the 12-hour clock repaints the ruler immediately', async ({ page }) => {
    await page.goto(app());
    await expect(ruler(page).first()).toHaveText('00');

    await chooseFormat(page, '1:30 PM');

    await expect(ruler(page).first()).toHaveText('12a');
    await expect(ruler(page).nth(11)).toHaveText('11a');
    await expect(ruler(page).nth(12)).toHaveText('12p');
    await expect(ruler(page).nth(13)).toHaveText('1p');
  });

  /** The choice is a stored preference, not a session variable — the same
   *  claim `listMode` earns its own spec for, and for the same reason: a
   *  variable and a row are indistinguishable until something reloads. */
  test('the chosen clock survives a reload', async ({ page }) => {
    await page.goto(app());
    await chooseFormat(page, '1:30 PM');
    await expect(ruler(page).first()).toHaveText('12a');

    await page.reload();
    await expect(ruler(page).first()).toHaveText('12a');
  });

  /** A second component, reading the same rune. The popover prints the
   *  occurrence's start and end, and it is opened from the grid rather than
   *  constructed here so the path is the one the app actually takes. */
  test('the popover prints its times in the chosen clock', async ({ page }) => {
    await page.goto(app());
    await chooseFormat(page, '1:30 PM');

    await page.locator('.col .ev').first().click();
    const when = page.locator('.pop .when');
    await expect(when).toContainText(/\b(AM|PM)\b/);
    await expect(when).not.toContainText(/\b\d{2}:\d{2}\s*–/);
  });

  /** And back again: a setting that could be turned on and not off is half a
   *  setting, and the 24-hour branch is the one every golden depends on. */
  test('choosing the 24-hour clock puts the ruler back', async ({ page }) => {
    await page.goto(app());
    await chooseFormat(page, '1:30 PM');
    await expect(ruler(page).first()).toHaveText('12a');

    await chooseFormat(page, '13:30');
    await expect(ruler(page).first()).toHaveText('00');
    await expect(ruler(page).nth(13)).toHaveText('13');
  });
});

/**
 * The temperature unit, from the select in Settings to the forecast high in
 * the day headings.
 *
 * Deliberately end-to-end rather than a second table beside
 * `temperature.spec.ts`: the arithmetic is proved there, and what can still be
 * wrong is the wiring. `App` seeds the `tempunit.svelte.ts` rune in **two**
 * places — once from the startup read of settings, once from the settings
 * modal's `onsettingschange` — and `components.spec.ts` reaches neither, since
 * it mounts `WeekGrid` alone and drives the rune directly through
 * `__setTemperatureUnit`. That proves the conversion and nothing about who
 * calls it.
 *
 * The two call sites need separate instruments, because the rune's own default
 * is Celsius: an `App` that never seeded it on startup draws exactly what one
 * that did draws, so the opening headers can witness nothing. Only a session
 * that *begins* with Fahrenheit already stored can tell those apart, which is
 * what the reload case below is for.
 */
test.describe('App: the temperature unit', () => {
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(APP_NOW);
  });

  /** The forecast highs in Week's day headers. `APP_WEATHER` covers the Monday
   *  the app opens on and no other day, so the count is part of the claim: a
   *  lookup that painted every header the same sky would draw seven of these,
   *  not one. (`components.spec.ts` pins the same premise with `toHaveCount(3)`
   *  against its own three-day fixture.) */
  const highs = (page: Page) => page.locator('.head .wx');

  const chooseUnit = async (page: Page, label: string) => {
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    await modal.locator('#temperature-unit').selectOption({ label });
    await page.keyboard.press('Escape');
    await expect(modal).toHaveCount(0);
  };

  test('the headers open on Celsius, on the covered day alone', async ({ page }) => {
    await page.goto(app('weather'));
    await expect(highs(page)).toHaveCount(1);
    await expect(highs(page).first()).toHaveText('31°');
  });

  /**
   * **Without a restart.** 31°C is 87.8°F, so a header that converted an
   * already-rounded 31° would print 87° and redden here — the same pair
   * `components.spec.ts` pins, arriving through the settings modal rather
   * than through a direct call to the rune.
   */
  test('choosing Fahrenheit repaints the day header immediately', async ({ page }) => {
    await page.goto(app('weather'));
    await expect(highs(page).first()).toHaveText('31°');

    await chooseUnit(page, '72°F');

    await expect(highs(page).first()).toHaveText('88°');
  });

  /** And back again: a setting that could be turned on and not off is half a
   *  setting, the rule the clock format above earns its own case for. */
  test('choosing Celsius puts the headers back', async ({ page }) => {
    await page.goto(app('weather'));
    await chooseUnit(page, '72°F');
    await expect(highs(page).first()).toHaveText('88°');

    await chooseUnit(page, '22°C');
    await expect(highs(page).first()).toHaveText('31°');
  });

  /** The **startup** seed, which nothing above can fail against for the reason
   *  the block comment gives. A reload is the only way to begin a session with
   *  the preference already stored — the stub's settings outlive one on
   *  purpose (`loadSettings`, `harness/tauri.ts`). */
  test('the chosen unit survives a reload', async ({ page }) => {
    await page.goto(app('weather'));
    await chooseUnit(page, '72°F');
    await expect(highs(page).first()).toHaveText('88°');

    await page.reload();
    await expect(highs(page).first()).toHaveText('88°');
  });

  /** A second component, reading the same rune. Filmstrip prints the same
   *  forecast high beside its day heading and rounds it with a `formatTemp`
   *  call of its own, so Week being right says nothing about it — and
   *  `components.spec.ts` covers that heading in Celsius only. */
  test('the filmstrip heading follows the chosen unit too', async ({ page }) => {
    await page.goto(app('weather'));
    await chooseUnit(page, '72°F');

    await page.keyboard.press('f');
    await expect(page.locator('.sdate .wx').first()).toHaveText('88°');
  });
});

/**
 * The keyboard sheet, and the table it and the handler both read.
 *
 * The point of the table is that it cannot drift from what the keys actually
 * do, and most of that guarantee is structural rather than testable: `App`
 * dispatches by looking a row up, and `SHORTCUT_ACTIONS` is typed
 * `Record<ShortcutId, …>`, so a row added without a handler does not compile.
 * What is left for a spec is the other end — that the sheet prints every row,
 * and that the keys it prints are the keys that work.
 */
test.describe('App: keyboard event navigation', () => {
  test.beforeEach(async ({ page }) => {
    // Before the first event, so the ordinary traversal specs still start at
    // the top. The late-today spec below advances this same clock to exercise
    // the time-aware entrance separately.
    await page.clock.setFixedTime(APP_MON + 8 * 3_600_000);
    await page.goto(app('keyboard-navigation'));
    // Gate on App having mounted before asserting the cursor's absence. An
    // empty pre-mount document also has no cursor, but does not yet have the
    // window key handler the next line of each test needs.
    await expect(page.locator('.vswitch button.active')).toBeVisible();
    await expect(page.locator('[data-kbd-selected-day]')).toHaveCount(0);
    await expect(page.locator('[data-kbd-selected-event]')).toHaveCount(0);
    await expect(page.locator('.kbd-status')).toHaveCount(0);
  });

  const selectedTitle = (page: Page) =>
    page.locator('[data-kbd-selected-event]');
  const deleteCalls = (page: Page) => page.evaluate(() =>
    window.__harness.calls.filter((call) => call.cmd === 'delete_event_cmd'),
  );

  test('the cursor appears only after keyboard navigation begins', async ({ page }) => {
    await page.keyboard.press('w');
    await expect(page.locator('[data-kbd-selected-day]')).toBeVisible();
    await expect(page.locator('.kbd-status')).toContainText('Selected Tuesday');
  });

  test('w and b select adjacent days', async ({ page }) => {
    await page.keyboard.press('w');
    await expect(page.locator('[data-kbd-selected-day]'))
      .toHaveAttribute('data-start-ms', String(APP_MON + 24 * 3_600_000));

    await page.keyboard.press('b');
    await expect(page.locator('[data-kbd-selected-day]'))
      .toHaveAttribute('data-start-ms', String(APP_MON));
  });

  test('j and k cross day boundaries in event order', async ({ page }) => {
    await page.keyboard.press('j');
    await expect(selectedTitle(page)).toContainText('Plan the launch');
    await page.keyboard.press('j');
    await expect(selectedTitle(page)).toContainText('Review notes');
    await page.keyboard.press('j');
    await expect(selectedTitle(page)).toContainText('Tuesday brief');
    await expect(page.locator('[data-kbd-selected-day]'))
      .toHaveAttribute('data-start-ms', String(APP_MON + 24 * 3_600_000));

    await page.keyboard.press('k');
    await expect(selectedTitle(page)).toContainText('Review notes');
    await page.keyboard.press('k');
    await expect(selectedTitle(page)).toContainText('Plan the launch');
  });

  test('entering a day through its first all-day item reveals its first timed event one hour down', async ({ page }) => {
    await page.keyboard.press('w');
    await page.keyboard.press('j');
    await expect(selectedTitle(page)).toContainText('Tuesday brief');
    await page.keyboard.press('k');
    await expect(selectedTitle(page)).toContainText('Review notes');
    await page.keyboard.press('k');
    await expect(selectedTitle(page)).toContainText('Plan the launch');

    const body = page.getByTestId('week-body');
    await body.evaluate((el) => { el.scrollTop = el.scrollHeight; });
    expect(await body.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);

    await page.keyboard.press('k');
    await expect(selectedTitle(page)).toContainText('All-day planning');
    await expect.poll(() => page.evaluate(() => {
      const viewport = document.querySelector<HTMLElement>('[data-testid="week-body"]')!;
      const firstTimed = [...viewport.querySelectorAll<HTMLElement>('.ev')]
        .find((event) => event.textContent?.includes('Plan the launch'))!;
      const oneHour = viewport.scrollHeight / 24;
      return Math.abs(firstTimed.getBoundingClientRect().top
        - viewport.getBoundingClientRect().top - oneHour);
    })).toBeLessThan(4);
  });

  test('j late today begins with tomorrow instead of replaying past events', async ({ page }) => {
    await page.clock.setFixedTime(APP_MON + 22 * 3_600_000);
    // This row is still visibly part of today, but has no clock position. It
    // must not keep the time-aware entrance on today after every timed event
    // has ended.
    await expect(page.getByText('All-day planning', { exact: true })).toBeVisible();

    await page.keyboard.press('j');
    await expect(selectedTitle(page)).toContainText('Tuesday brief');
    await expect(page.locator('[data-kbd-selected-day]'))
      .toHaveAttribute('data-start-ms', String(APP_MON + 24 * 3_600_000));
  });

  test('j on a future day starts with that day’s first event', async ({ page }) => {
    for (let i = 0; i < 3; i++) await page.keyboard.press('w');

    await page.keyboard.press('j');
    await expect(selectedTitle(page)).toContainText('Off-site');
    await expect(page.locator('[data-kbd-selected-day]'))
      .toHaveAttribute('data-start-ms', String(APP_MON + 3 * 24 * 3_600_000));
  });

  test('k from a selected day begins at its last event, then crosses backward', async ({ page }) => {
    await page.keyboard.press('w');
    await page.keyboard.press('k');
    await expect(selectedTitle(page)).toContainText('Tuesday brief');
    await page.keyboard.press('k');
    await expect(selectedTitle(page)).toContainText('Review notes');
  });

  test('j continues into the first event of the next loaded period', async ({ page }) => {
    for (let i = 0; i < 5; i++) await page.keyboard.press('j');
    await expect(selectedTitle(page)).toContainText('All-day planning');
    await expect(page.locator('[data-kbd-selected-day]'))
      .toHaveAttribute('data-start-ms', String(APP_MON + WEEK));
  });

  test('o and Enter open details, whose own keyboard interaction stays native', async ({ page }) => {
    await page.keyboard.press('j');
    await page.keyboard.press('o');

    const dialog = page.getByRole('dialog', { name: 'Plan the launch' });
    await expect(dialog).toBeVisible();
    await expect(dialog).toBeFocused();

    const join = dialog.getByRole('link', { name: 'Join video call' });
    await join.focus();
    await page.keyboard.press('Enter');
    await expect.poll(() => page.evaluate(() =>
      window.__harness.calls.filter((call) => call.cmd === 'open_conference').length,
    )).toBe(1);

    // `j` belongs to the dialog while its focus is inside: it must not move
    // the calendar cursor behind the still-open details.
    await page.keyboard.press('j');
    await expect(selectedTitle(page)).toContainText('Plan the launch');

    // A disabled control can briefly leave focus on body. The still-open
    // dialog owns bare keys even then; `n` must not create a form behind it.
    await page.evaluate(() => (document.activeElement as HTMLElement)?.blur());
    await page.keyboard.press('n');
    await expect(page.getByRole('dialog', { name: 'New event' })).toHaveCount(0);
    await expect(dialog).toBeVisible();

    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    await page.keyboard.press('Enter');
    await expect(page.getByRole('dialog', { name: 'Plan the launch' })).toBeVisible();
  });

  test('Backspace on the selected event asks before deleting', async ({ page }) => {
    await page.keyboard.press('j');
    await page.keyboard.press('j');
    await expect(selectedTitle(page)).toContainText('Review notes');
    await page.keyboard.press('Backspace');

    const confirm = page.getByRole('dialog', { name: 'Delete event' });
    await expect(confirm).toBeVisible();
    await expect(confirm.locator('h2')).toContainText('Review notes');
    expect(await deleteCalls(page)).toEqual([]);
  });

  test('Backspace on an open event asks before deleting', async ({ page }) => {
    await page.keyboard.press('j');
    await page.keyboard.press('j');
    await page.keyboard.press('Enter');
    await expect(page.getByRole('dialog', { name: 'Review notes' })).toBeVisible();
    await page.keyboard.press('Backspace');

    const confirm = page.getByRole('dialog', { name: 'Delete event' });
    await expect(confirm).toBeVisible();
    await expect(confirm.locator('h2')).toContainText('Review notes');
    expect(await deleteCalls(page)).toEqual([]);
  });
});

test.describe('App: week panning', () => {
  const callsTo = (page: Page, cmd: string): Promise<any[]> =>
    page.evaluate(
      (c) => window.__harness.calls.filter((call) => call.cmd === c).map((call) => call.args),
      cmd,
    );

  test.beforeEach(async ({ page }) => {
    await page.goto(app('writable'));
    await expect(page.locator('[data-testid="week-body"]')).toBeVisible();
  });

  test('a horizontal wheel slides the week by days and Today re-aligns it', async ({ page }) => {
    // The default scenario rather than the block's `writable`: its stub
    // echoes the week actually asked for, padding and all, which is what a
    // slide into the padding has to be seen against. `writable` answers one
    // fixed week whatever is asked, so it is served unpadded and the grid
    // shows it whole — right for its specs, blind for this one.
    await page.goto(app());
    // The aligned start is whatever the app itself first fetched — this
    // runs on the real clock, so a pinned constant here would be asserting
    // somebody else's week (it was, and failed).
    await expect(page.locator('.col')).toHaveCount(7);
    const [first] = await callsTo(page, 'get_week');
    const aligned = first.weekStartMs;
    // Padded by a week either side (2026-09-03), so the slide has columns
    // to reveal; at rest only the window is on the track.
    expect(first.pad).toBe(7);
    await expect(page.locator('.col')).toHaveCount(7);
    const colWidth = (await page.locator('.col').first().boundingBox())!.width;

    await page.locator('[data-testid="week-body"]').hover();
    // Half a column's width of wheel is a day of track — the gain is two, so
    // a week is a swipe and not the width of a touchpad. 0.65 columns of
    // wheel: one day committed as its column is crossed, the rest settling
    // back after the lull. The test environment's zone is UTC, so day
    // arithmetic by constant is safe *here*; the app's own arithmetic goes
    // through Date, as the spec asks.
    await page.mouse.wheel(Math.round(colWidth * 0.65), 0);
    // Mid-gesture the whole payload is on the track, padding included.
    await expect(page.locator('.col')).toHaveCount(21);
    await expect
      .poll(async () => (await callsTo(page, 'get_range')).map((a) => a.dayStartMs))
      .toContain(aligned + 86_400_000);
    const ranges = await callsTo(page, 'get_range');
    // A panned fixed week still shows its full seven days (spec §3), padded.
    expect(ranges[ranges.length - 1]).toMatchObject({ dayCount: 7, pad: 7 });
    // Settled: the window alone again, a day on.
    await expect(page.locator('.col')).toHaveCount(7);
    expect(await page.locator('.col').first().getAttribute('data-start-ms'))
      .toBe(String(aligned + 86_400_000));

    // Today ends the peek: the very next week fetch is the aligned Monday
    // again, through get_week — the pre-pan path, byte for byte.
    const weeksBefore = (await callsTo(page, 'get_week')).length;
    await page.keyboard.press('t');
    await expect
      .poll(async () => (await callsTo(page, 'get_week')).length)
      .toBeGreaterThan(weeksBefore);
    const weeks = await callsTo(page, 'get_week');
    expect(weeks[weeks.length - 1].weekStartMs).toBe(aligned);
  });

  test('a vertical wheel scrolls and never pans', async ({ page }) => {
    await page.locator('[data-testid="week-body"]').hover();
    await page.mouse.wheel(0, 240);
    // Absence needs a beat to be meaningful — a fetch would have been issued
    // synchronously with the effect flush this wait outlives.
    await page.waitForTimeout(200);
    expect(await callsTo(page, 'get_range')).toEqual([]);
  });
});

test.describe('App: the keyboard sheet', () => {
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(APP_NOW);
  });

  const sheet = (page: Page) => page.getByRole('dialog', { name: 'Keyboard shortcuts' });

  /**
   * Open the app **and wait for it to be listening**.
   *
   * Not ceremony: `<svelte:window onkeydown>` is attached when `App` mounts,
   * and a `page.goto` resolves well before that. Every test here presses a
   * bare key as its first act, so without this gate the press lands on a page
   * with no handler and the assertion that follows fails for a reason that has
   * nothing to do with what it is testing. Asserting the *absence* of the
   * sheet first does not gate anything — an empty page has no sheet either.
   */
  const openApp = async (page: Page) => {
    await page.goto(app());
    await expect(page.locator('.vswitch button.active')).toBeVisible();
  };

  /**
   * **`Shift+Slash`, not `'?'`.** Playwright will happily synthesise a bare
   * `?` with `shiftKey: false`, which no keyboard on the layouts this ships to
   * can produce — and a spec written that way passes with `e.shiftKey` added
   * to the handler's modifier bail, which would make the key dead for every
   * real user. The mutation sweep caught exactly that; this is the fixture
   * that can witness it (testing standard §6).
   */
  test('? opens the sheet and Escape closes it', async ({ page }) => {
    await openApp(page);
    await expect(sheet(page)).toHaveCount(0);

    await page.keyboard.press('Shift+Slash');
    await expect(sheet(page)).toBeVisible();

    await page.keyboard.press('Escape');
    await expect(sheet(page)).toHaveCount(0);
  });

  test('Ctrl+Comma opens General settings and Escape closes it', async ({ page }) => {
    await openApp(page);
    await page.keyboard.press('Control+Comma');

    const settings = page.getByRole('dialog', { name: 'Settings' });
    await expect(settings).toBeVisible();
    await expect(settings.getByRole('tab', { name: 'General' })).toHaveAttribute(
      'aria-selected', 'true',
    );

    await page.keyboard.press('Escape');
    await expect(settings).toHaveCount(0);
  });

  /**
   * **Every row, read from the table itself.** A sheet that hardcoded its own
   * list would pass a test that hardcoded the same list, and the two would
   * drift together — so the expectation here is imported from the module under
   * test, and what it proves is the rendering, not the contents.
   */
  test('the sheet prints every shortcut in the table', async ({ page }) => {
    await openApp(page);
    await page.keyboard.press('Shift+Slash');

    // Anchored, and escaped: the labels include `?` and `/`, and `1` through
    // `5` would otherwise be read as backreferences rather than as digits —
    // which fails in the direction that matters, but silently could not.
    const literal = (t: string) => new RegExp(`^${t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`);

    for (const s of SHORTCUT_LIST) {
      await expect(
        sheet(page).locator('[data-shortcut-scope="calendar"] dt', { hasText: literal(s.label) }),
        `${s.label} (${s.id}) is bound but not listed`,
      ).toHaveCount(1);
      await expect(
        sheet(page).locator('.what', { hasText: literal(SHORTCUT_TEXT[s.id]) }),
        `${s.id} is listed under a key but not described`,
      ).toHaveCount(1);
    }
    for (const c of CHORDS) {
      await expect(
        sheet(page).locator('dt', { hasText: literal(c.label) }),
      ).toHaveCount(1);
      await expect(sheet(page).locator('.what', { hasText: literal(c.text) })).toHaveCount(1);
    }

    for (const s of EVENT_SHORTCUT_LIST) {
      await expect(
        sheet(page).locator('[data-shortcut-scope="event"] dt', { hasText: literal(s.label) }),
        `${s.label} (${s.id}) is scoped to event details but not listed`,
      ).toHaveCount(1);
      await expect(
        sheet(page).locator('.what', { hasText: literal(s.text) }),
        `${s.id} is listed under a key but not described`,
      ).toHaveCount(1);
    }

    for (const c of EDIT_CHORDS) {
      await expect(
        sheet(page).locator('[data-shortcut-scope="edit"] dt', { hasText: literal(c.label) }),
      ).toHaveCount(1);
      await expect(sheet(page).locator('.what', { hasText: literal(c.text) })).toHaveCount(1);
    }
  });

  /** The keys the sheet advertises are the keys that work — driven from the
   *  same array, so a row whose binding was never wired up is caught here
   *  rather than by a user pressing it. Only the five view keys can be
   *  asserted generically; the rest have their own specs above. */
  test('every view key in the table switches to its view', async ({ page }) => {
    await openApp(page);
    for (const s of SHORTCUT_LIST.filter((s) => s.view)) {
      await page.keyboard.press(s.key);
      await expect(
        page.locator('.vswitch button.active'),
        `${s.key} should select ${s.view}`,
      ).toHaveText(SHORTCUT_TEXT[s.id]);
    }
  });

  /** The sheet is a panel like the others: while it is open the bare keys
   *  behind it are dead, or `3` read through it would switch the view under a
   *  dialog the user is still reading. */
  test('a view key does nothing while the sheet is open', async ({ page }) => {
    await openApp(page);
    await page.keyboard.press('2');
    const before = await page.locator('.vswitch button.active').textContent();

    await page.keyboard.press('Shift+Slash');
    await page.keyboard.press('3');

    await expect(sheet(page)).toBeVisible();
    await expect(page.locator('.vswitch button.active')).toHaveText(before ?? '');
  });

  /** `?` is Shift-`/` on the layouts this ships to, so the modifier bail must
   *  not include Shift — and `/` itself must still open search, not the
   *  sheet. The two share a physical key and only `e.key` tells them apart. */
  test('/ still opens search, and does not open the sheet', async ({ page }) => {
    await openApp(page);
    await page.keyboard.press('/');
    await expect(page.getByRole('dialog', { name: 'Search' })).toBeVisible();
    await expect(sheet(page)).toHaveCount(0);
    // The `consumes` flag, witnessed. WebKitGTK runs the keydown's default
    // action *after* the overlay has mounted and focused its field, so an
    // unconsumed `/` becomes the first character of the query and "sync" is
    // searched as "/sync". Asserting only that the overlay opened cannot see
    // it — the sweep proved that by deleting the `preventDefault` and staying
    // green. This runs under webkit too, which is where the bug lives.
    await expect(page.getByRole('searchbox', { name: 'Search events' })).toHaveValue('');
  });
});

/**
 * The first day of the week, from the select in Settings to the four views
 * that draw a week-aligned row.
 *
 * The arithmetic is proved twice already — `weekstart.spec.ts` for the
 * browser's half and the Rust suite for the grids' — so what is left here is
 * the wiring: a rune seeded but not refreshed, a header still spelling its own
 * days, a grid asking the backend for one week and drawing another.
 */
test.describe('App: the first day of the week', () => {
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(APP_NOW);
  });

  const openApp = async (page: Page) => {
    await page.goto(app());
    await expect(page.locator('.vswitch button.active')).toBeVisible();
  };

  /** Picks a row of the single Week view select — one control since issue
   *  #31, because the rolling view was unfindable behind two. `day` is the
   *  weekday name; `rollingDays` picks a "N days from today" row instead. */
  const chooseWeekView = async (page: Page, label: string) => {
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    await modal.getByRole('tab', { name: 'Appearance' }).click();
    await modal.locator('#week-view').selectOption({ label });
    await page.keyboard.press('Escape');
    await expect(modal).toHaveCount(0);
  };
  const chooseStart = (page: Page, day: string) =>
    chooseWeekView(page, `Whole week, from ${day}`);

  /** Month's own header row — the labels the user reads above the grid. */
  const monthHeader = (page: Page) => page.locator('.head span');

  test('Month opens on Monday, as it always has', async ({ page }) => {
    await openApp(page);
    await page.keyboard.press('3');
    await expect(monthHeader(page).first()).toHaveText('MON');
    await expect(monthHeader(page).last()).toHaveText('SUN');
  });

  /** **Without a restart**, the same claim the clock format earns: `App`
   *  seeds the rune once, so a change that only reached the database would
   *  still pass a test that reloaded. */
  test('choosing Sunday rotates the Month header immediately', async ({ page }) => {
    await openApp(page);
    await page.keyboard.press('3');
    await expect(monthHeader(page).first()).toHaveText('MON');

    await chooseStart(page, 'Sunday');

    await expect(monthHeader(page).first()).toHaveText('SUN');
    await expect(monthHeader(page).last()).toHaveText('SAT');
  });

  test('choosing Saturday rotates it again, and Monday puts it back', async ({ page }) => {
    await openApp(page);
    await page.keyboard.press('3');

    await chooseStart(page, 'Saturday');
    await expect(monthHeader(page).first()).toHaveText('SAT');
    await expect(monthHeader(page).last()).toHaveText('FRI');

    await chooseStart(page, 'Monday');
    await expect(monthHeader(page).first()).toHaveText('MON');
  });

  /** A stored preference, not a session variable. */
  test('the chosen first day survives a reload', async ({ page }) => {
    await openApp(page);
    await chooseStart(page, 'Sunday');
    await page.keyboard.press('3');
    await expect(monthHeader(page).first()).toHaveText('SUN');

    await page.reload();
    await page.keyboard.press('3');
    await expect(monthHeader(page).first()).toHaveText('SUN');
  });

  /**
   * The half a header test cannot see: Week asks the backend for a *week*,
   * and the day it names has to move with the setting too. If `weekStart` in
   * `api.ts` kept its own idea of Monday, the header above would rotate and
   * the seven columns below it would not.
   */
  test('Week asks for the week the setting names', async ({ page }) => {
    await openApp(page);
    const firstColumn = page.locator('[data-testid="week-body"] .col').first();
    const mondayMs = await firstColumn.getAttribute('data-start-ms');

    await chooseStart(page, 'Sunday');

    const sundayMs = await firstColumn.getAttribute('data-start-ms');
    expect(Number(sundayMs)).toBe(Number(mondayMs) - 24 * 3_600_000);
  });

  test('Today starts a rolling 3, 5, or 7-day Week view on the current day', async ({ page }) => {
    const day = 24 * 3_600_000;
    const wednesday = APP_NOW + 2 * day;
    await page.clock.setFixedTime(wednesday);
    await openApp(page);

    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    await modal.getByRole('tab', { name: 'Appearance' }).click();
    // The whole point of the one-control shape: the rolling sizes are on
    // the list you are already reading, not behind another answer.
    const weekView = modal.locator('#week-view');
    await expect(weekView.locator('option')).toHaveText([
      'Whole week, from Monday', 'Whole week, from Sunday', 'Whole week, from Saturday',
      '3 days from today', '5 days from today', '7 days from today',
    ]);
    await weekView.selectOption({ label: '3 days from today' });
    await page.keyboard.press('Escape');

    const columns = page.locator('[data-testid="week-body"] .col');
    await expect(columns).toHaveCount(3);
    await expect(columns.first()).toHaveAttribute('data-start-ms', String(wednesday - 12 * 3_600_000));
    await expect(page.getByRole('button', { name: 'Next 3 days' })).toBeVisible();

    await page.getByRole('button', { name: 'Next 3 days' }).click();
    await expect(columns.first()).toHaveAttribute(
      'data-start-ms', String(wednesday - 12 * 3_600_000 + 3 * day),
    );

    // All three supported sizes change the payload, not only the select.
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const reopened = page.getByRole('dialog', { name: 'Settings' });
    await reopened.getByRole('tab', { name: 'Appearance' }).click();
    await reopened.locator('#week-view').selectOption({ label: '5 days from today' });
    await expect(columns).toHaveCount(5);
    await reopened.locator('#week-view').selectOption({ label: '7 days from today' });
    await expect(columns).toHaveCount(7);

    // And back out of rolling through the same control: seven columns again,
    // but anchored on the weekday rather than on today. One select has to
    // carry the return trip as well as the way in.
    await reopened.locator('#week-view').selectOption({ label: 'Whole week, from Monday' });
    await expect(columns).toHaveCount(7);
    await expect(columns.first()).not.toHaveAttribute(
      'data-start-ms', String(wednesday - 12 * 3_600_000),
    );
  });

  test('a rolling range follows today across midnight but preserves Month alignment', async ({ page }) => {
    const day = 24 * 3_600_000;
    const wednesdayNoon = APP_NOW + 2 * day;
    await page.clock.setFixedTime(wednesdayNoon);
    await openApp(page);

    await chooseStart(page, 'Sunday');
    await chooseWeekView(page, '7 days from today');
    const first = page.locator('[data-testid="week-body"] .col').first();
    const wednesday = wednesdayNoon - 12 * 3_600_000;
    await expect(first).toHaveAttribute('data-start-ms', String(wednesday));

    await page.clock.setSystemTime(wednesdayNoon + day);
    await page.evaluate(() => window.dispatchEvent(new Event('focus')));
    await expect(first).toHaveAttribute('data-start-ms', String(wednesday + day));

    await page.keyboard.press('3');
    await expect(monthHeader(page).first()).toHaveText('SUN');
  });
});

test.describe('App: zooming the hours', () => {
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(APP_NOW);
  });
  const colHeight = (page: Page) =>
    page.locator('.col').first().evaluate((el) => el.getBoundingClientRect().height);
  const lastStored = (page: Page) => page.evaluate(() => {
    const writes = window.__harness.calls.filter((c) => c.cmd === 'set_hour_height');
    return writes.length ? writes[writes.length - 1].args : null;
  });

  test('Ctrl+= zooms the hours, and the height is kept for next time', async ({ page }) => {
    await page.goto(app());
    await expect(page.locator('.col')).toHaveCount(7);
    expect(await colHeight(page)).toBe(1680);

    await page.keyboard.press('Control+=');
    await expect.poll(() => colHeight(page)).toBe(88 * 24); // 70 x 1.25 = 87.5, drawn at 88
    await page.keyboard.press('Control+-');
    await expect.poll(() => colHeight(page)).toBe(1680);
    await page.keyboard.press('Control+=');
    await page.keyboard.press('Control+=');
    await expect.poll(() => colHeight(page)).toBe(109 * 24); // 109.375, drawn and stored at 109

    // Stored once the keys have gone quiet — whole pixels, the final value.
    await expect.poll(() => lastStored(page), { timeout: 3000 }).toEqual({ px: 109 });

    // And read back on the next launch: the stub keeps its settings for the tab.
    await page.reload();
    await expect(page.locator('.col')).toHaveCount(7);
    await expect.poll(() => colHeight(page)).toBe(109 * 24);

    // Ctrl+0 is the way back.
    await page.keyboard.press('Control+0');
    await expect.poll(() => colHeight(page)).toBe(1680);
  });

  test('the keys mean nothing where there is no hour grid', async ({ page }) => {
    await page.goto(app());
    await expect(page.locator('.col')).toHaveCount(7);
    await page.keyboard.press('3');
    await expect(page.locator('.mcell').first()).toBeVisible();
    await page.keyboard.press('Control+=');
    await page.keyboard.press('2');
    await expect(page.locator('.col')).toHaveCount(7);
    expect(await colHeight(page)).toBe(1680);
    expect(await lastStored(page)).toBeNull();
  });
});

test.describe('App: a click that cannot be answered says so', () => {
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(APP_NOW);
  });

  test('an all-day chip whose detail will not load leaves an error, not silence', async ({ page }) => {
    // The shape of the 2026-09-04 report: pressing an all-day event did
    // nothing at all. The click was reaching the grid and the detail fetch
    // was failing, and the app kept that to itself.
    await page.goto(app('writable'));
    const chip = page.getByRole('button', { name: 'Diwali' });
    await expect(chip).toBeVisible();
    await page.evaluate((id) => window.__harness.failNextEventCall(
      'event_detail', id, 'that event is no longer here',
    ), APP_ALLDAY_ID);

    await chip.click();
    await expect(page.locator('.err')).toContainText('Could not open "Diwali"');
    await expect(page.locator('.err')).toContainText('no longer here');
    await expect(page.locator('.pop')).toHaveCount(0);

    // And the next click, with the failure spent, opens normally.
    await chip.click();
    await expect(page.getByRole('dialog', { name: 'Diwali' })).toBeVisible();
  });
});

test.describe("App: showing today's date", () => {
  test.beforeEach(async ({ page }) => {
    await page.clock.setFixedTime(APP_NOW);
  });

  test("today's date can be asked for, and is remembered", async ({ page }) => {
    // Asked for 2026-09-04. One switch dresses two surfaces: the tray icon
    // *becomes* the date, because a tray host draws icons and nothing else,
    // and the bar widget draws it beside its mark from the feed. Off by
    // default — the mark is what says which app it is.
    await page.goto(app('writable'));
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    const modal = page.getByRole('dialog', { name: 'Settings' });
    await modal.getByRole('tab', { name: 'Appearance' }).click();
    const box = modal.getByLabel("Show today's date");
    await expect(box).not.toBeChecked();

    await box.check();
    const calls = await page.evaluate(
      () => window.__harness.calls.filter((c) => c.cmd === 'set_show_date').map((c) => c.args),
    );
    expect(calls).toEqual([{ on: true }]);

    // Reopened, the box shows what was stored rather than its own default.
    await page.keyboard.press('Escape');
    await expect(modal).toHaveCount(0);
    await page.getByRole('button', { name: 'Menu' }).click();
    await page.getByRole('button', { name: 'Settings…' }).click();
    await modal.getByRole('tab', { name: 'Appearance' }).click();
    await expect(modal.getByLabel("Show today's date")).toBeChecked();
  });
});
