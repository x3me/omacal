import { test, expect, type Page } from '@playwright/test';
import { APP_MON, APP_NOW, APP_SERIES_ID, APP_ONE_OFF_ID, appWritableWeek, POPOVER_DETAILS, FIXTURES } from './fixtures';

const series = APP_SERIES_ID, other = APP_ONE_OFF_ID;
const calls = (page: Page, cmd: string) => page.evaluate(c => window.__harness.calls.filter(x => x.cmd === c).length, cmd);
const block = (page: Page, name: string) => page.locator('.ev').filter({hasText: name});
const week = () => {
  const w = structuredClone(appWritableWeek());
  const day = w.days[0];
  day.events.push({...day.events[0], title: 'Standup next', start_ms: day.events[0].start_ms + 86400000, end_ms: day.events[0].end_ms + 86400000});
  day.placed.push({...day.placed[0], idx: day.events.length - 1, top: .18});
  for (const d of w.days) for (const e of d.events) e.response = 'needsAction';
  return w;
};
async function open(page: Page) {
  await page.clock.setFixedTime(APP_NOW);
  await page.goto('/tests/harness/index.html?c=App&f=writable');
  await expect(block(page, 'Standup')).toHaveCount(1);
  const w = week();
  const invites = w.days[0].events.slice(0, 2).map(e => ({id: e.id, title: e.title, start_ms: e.start_ms, end_ms: e.end_ms,
    is_all_day: false, start_date: null, end_date: null, organizer_email: 'host@example.com', color: '#5b8def', can_respond: true}));
  await page.evaluate(async data => { window.__harness.setResponseData(data); await window.__harness.emit('sync-finished', null); }, {week: w, invites});
  await expect(block(page, 'Standup')).toHaveCount(2);
}
async function answer(page: Page, title: string) {
  if (!await page.getByRole('group', {name: 'Pending invitations'}).count())
    await page.getByRole('button', {name: /pending invitation/}).click();
  await page.getByTestId('invite-row').filter({hasText: title}).getByRole('button', {name: 'Yes', exact: true}).click();
}

for (const surface of ['tray', 'popover'] as const) test(`a saved ${surface} reply keeps every series block answered until the reload lands`, async ({page}) => {
  await open(page);
  await page.evaluate(({series, other}) => {
    window.__harness.holdNextEventCall('respond_to_event', series);
    window.__harness.holdNextEventCall('respond_to_event', other);
    window.__harness.holdNextSync();
  }, {series, other});
  if (surface === 'tray') await answer(page, 'Standup');
  else {
    await block(page, 'Standup').first().click();
    await page.getByRole('button', {name: 'Yes', exact: true}).click();
    await page.getByRole('button', {name: 'All of them'}).click();
    await page.keyboard.press('Escape');
  }
  await answer(page, 'Board prep');
  await expect(block(page, 'Standup')).toHaveClass([/accepted/, /accepted/]);
  await page.evaluate(({id, detail}) => window.__harness.releaseEventCall('respond_to_event', id, detail), {id: series, detail: POPOVER_DETAILS[series]});
  await expect.poll(() => calls(page, 'respond_to_event')).toBe(2);
  await expect(block(page, 'Standup')).toHaveClass([/accepted/, /accepted/]);
  await page.evaluate(({id, detail}) => window.__harness.releaseEventCall('respond_to_event', id, detail), {id: other, detail: POPOVER_DETAILS[other]});
  await expect.poll(() => calls(page, 'sync_now')).toBe(1);
  await expect(block(page, 'Standup')).toHaveClass([/accepted/, /accepted/]);
  const w = week();
  for (const d of w.days) for (const e of d.events) e.response = 'declined';
  await page.evaluate(({w, start}) => { window.__harness.setResponseData({week: w, invites: []}); window.__harness.hold(start); }, {w, start: APP_MON});
  await page.evaluate(() => window.__harness.releaseSync());
  await expect(block(page, 'Standup')).toHaveClass([/accepted/, /accepted/]);
  await page.evaluate(start => window.__harness.release(start), APP_MON);
  await expect(block(page, 'Standup')).toHaveClass([/declined/, /declined/]);
});

for (const kind of ['decline', 'all declines', 'change', 'all changes']) test(`dismissing ${kind} refreshes locally without a full sync`, async ({page}) => {
  await open(page);
  const f = FIXTURES.Header[kind.includes('decline') ? 'with-declines' : 'with-changes'] as any;
  await page.evaluate(async data => { window.__harness.setResponseData(data); await window.__harness.emit('sync-finished', null); }, {invites: [], declines: f.declines, changes: f.changes ?? []});
  await page.getByRole('button', {name: /decline/}).click();
  const before = await calls(page, 'get_week');
  const cmd = kind === 'decline' ? 'dismiss_decline_notice' : kind === 'all declines' ? 'dismiss_all_decline_notices' : kind === 'change' ? 'dismiss_change_notice' : 'dismiss_all_change_notices';
  const label = kind === 'decline' ? 'Dismiss decline by Victor' : kind === 'all declines' ? 'Dismiss all' : kind === 'change' ? 'Dismiss reschedule of NVP sync' : 'Dismiss all';
  await page.getByRole('button', {name: label, exact: true}).first().click();
  await expect.poll(() => calls(page, cmd)).toBe(1);
  await expect.poll(() => calls(page, 'get_week')).toBeGreaterThan(before);
  expect(await calls(page, 'sync_now')).toBe(0);
});

test('saving and failure feedback never move the calendar or its open popover', async ({page}) => {
  await open(page);
  await block(page, 'Standup').first().click();
  const top = (await page.getByTestId('week-body').boundingBox())!.y;
  const pop = (await page.locator('.pop').boundingBox())!.y;
  await page.evaluate(id => window.__harness.holdNextEventCall('respond_to_event', id), series);
  await page.getByRole('button', {name: 'Yes', exact: true}).click();
  await page.getByRole('button', {name: 'All of them'}).click();
  await expect(page.locator('header [role="status"]')).toContainText('Saving 1 response');
  expect((await page.getByTestId('week-body').boundingBox())!.y).toBe(top);
  expect((await page.locator('.pop').boundingBox())!.y).toBe(pop);
  await page.evaluate(id => window.__harness.rejectEventCall('respond_to_event', id, 'Reply failed.'), series);
  await expect(page.getByText(/could not save your response/)).toHaveCount(1);
  expect((await page.getByTestId('week-body').boundingBox())!.y).toBe(top);
  await page.keyboard.press('Escape');
  await expect(page.locator('header [role="alert"]')).toContainText('Reply failed.');
  expect((await page.getByTestId('week-body').boundingBox())!.y).toBe(top);
  await page.getByRole('button', {name: 'Dismiss response error'}).click();
  await block(page, 'Standup').first().click();
  await expect(page.getByText(/could not save your response/)).toHaveCount(0);
});

test('a later successful answer clears an earlier failure for the same invitation', async ({page}) => {
  await open(page);
  await page.evaluate(id => {
    window.__harness.holdNextEventCall('respond_to_event', id);
    const q = (window as any).__responses;
    void q.queueResponse({id, response: 'accepted', scope: 'all', occurrenceStartMs: 1}, 'Standup').catch(() => {});
    void q.queueResponse({id, response: 'declined', scope: 'all', occurrenceStartMs: 1}, 'Standup');
  }, series);
  await expect.poll(() => calls(page, 'respond_to_event')).toBe(1);
  await page.evaluate(id => window.__harness.rejectEventCall('respond_to_event', id, 'First reply failed.'), series);
  await expect.poll(() => calls(page, 'respond_to_event')).toBe(2);
  await expect(page.getByRole('alert')).toHaveCount(0);
});

test('a refresh requested during a failed sync still runs', async ({page}) => {
  await open(page);
  await page.evaluate(() => window.__harness.holdNextSync());
  await answer(page, 'Standup');
  await expect.poll(() => calls(page, 'sync_now')).toBe(1);
  await answer(page, 'Board prep');
  await expect.poll(() => calls(page, 'respond_to_event')).toBe(2);
  await page.evaluate(() => window.__harness.rejectSync('Offline for the first sync.'));
  await expect.poll(() => calls(page, 'sync_now')).toBe(2);
  await expect(page.getByRole('img', {name: /Syncing now/})).toHaveCount(0);
});

test('a server attendee response wins over the completed optimistic reply', async ({page}) => {
  await page.goto('/tests/harness/index.html?c=EventPopover&f=writes-back');
  const id = await page.evaluate(() => (window as any).__fixtureProps.detail.id);
  await page.evaluate(id => window.__harness.holdNextEventCall('respond_to_event', id), id);
  await page.getByRole('button', {name: 'No', exact: true}).click();
  await expect(page.locator('.guest')).toHaveClass(/declined/);
  await page.evaluate(id => window.__harness.releaseEventCall('respond_to_event', id, {
    ...(window as any).__fixtureProps.detail,
    attendees: [{email: 'me@x.com', display_name: null, is_self: true, optional: false, response_status: 'tentative'}],
  }), id);
  await expect(page.locator('.guest')).toHaveClass(/tentative/);
});

test('a newer post-sync reload can retire saved replies when the original reload is superseded', async ({page}) => {
  await open(page);
  await page.evaluate(() => window.__harness.holdNextSync());
  await answer(page, 'Standup');
  await expect.poll(() => calls(page, 'sync_now')).toBe(1);
  const w = week();
  for (const d of w.days) for (const e of d.events) e.response = 'declined';
  await page.evaluate(({w, start}) => {
    window.__harness.setResponseData({week: w, invites: []}); window.__harness.hold(start);
  }, {w, start: APP_MON});
  await page.evaluate(() => window.__harness.releaseSync());
  await expect.poll(() => page.evaluate(() => window.__harness.held())).toBe(1);
  await page.evaluate(() => window.__harness.emit('sync-finished', null));
  await expect(block(page, 'Standup')).toHaveClass([/declined/, /declined/]);
  await page.evaluate(start => window.__harness.release(start), APP_MON);
  await expect(block(page, 'Standup')).toHaveClass([/declined/, /declined/]);
});

test('a failed post-sync reload retains the answer until a later payload succeeds', async ({page}) => {
  await open(page);
  await page.evaluate(() => window.__harness.holdNextSync());
  await answer(page, 'Standup');
  await expect.poll(() => calls(page, 'sync_now')).toBe(1);
  await page.evaluate(() => window.__harness.failNextWeek('Could not reload the calendar.'));
  await page.evaluate(() => window.__harness.releaseSync());
  await expect(block(page, 'Standup')).toHaveClass([/accepted/, /accepted/]);
  const w = week();
  for (const d of w.days) for (const e of d.events) e.response = 'declined';
  await page.evaluate(async w => { window.__harness.setResponseData({week: w, invites: []}); await window.__harness.emit('sync-finished', null); }, w);
  await expect(block(page, 'Standup')).toHaveClass([/declined/, /declined/]);
});

test('a sync only retires answers saved before it started', async ({page}) => {
  await open(page);
  await page.evaluate(() => window.__harness.holdNextSync());
  await answer(page, 'Standup');
  await expect.poll(() => calls(page, 'sync_now')).toBe(1);
  await answer(page, 'Board prep');
  await expect.poll(() => calls(page, 'respond_to_event')).toBe(2);
  await page.evaluate(() => window.__harness.holdNextSync());
  await page.evaluate(() => window.__harness.releaseSync());
  await expect.poll(() => calls(page, 'sync_now')).toBe(2);
  await expect(block(page, 'Board prep')).toHaveClass(/accepted/);
  await page.evaluate(() => window.__harness.releaseSync());
  await expect(block(page, 'Board prep')).toHaveClass(/needsAction/);
});

test('tray error dismissal clears the shared notice and a narrow header keeps it on screen', async ({page}) => {
  await page.setViewportSize({width: 540, height: 800});
  await page.goto('/tests/harness/index.html?c=Header&f=with-invites');
  await page.evaluate(() => window.__harness.failNextEventCall('respond_to_event', 901, 'Network unavailable.'));
  await page.getByRole('button', {name: '2 pending invitations'}).click();
  await page.getByTestId('invite-row').first().getByRole('button', {name: 'Yes', exact: true}).click();
  await expect(page.getByText(/Network unavailable/)).toHaveCount(1);
  await page.getByRole('button', {name: 'Close invitations'}).click();
  const error = page.getByRole('alert');
  await expect(error).toContainText('Network unavailable.');
  const box = (await error.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(540);
  await page.getByRole('button', {name: '2 pending invitations'}).click();
  await page.getByRole('button', {name: 'Dismiss response error'}).click();
  await expect(page.getByText(/Network unavailable/)).toHaveCount(0);
  await page.getByRole('button', {name: 'Close invitations'}).click();
  await expect(page.getByRole('alert')).toHaveCount(0);
});
