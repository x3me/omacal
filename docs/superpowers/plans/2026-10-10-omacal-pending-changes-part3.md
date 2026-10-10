# Pending Changes, Part 3 (Create) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A new event saved from the form or Quick Add is drawn at once, dashed and inert, created on Google behind the user; a refusal takes it away and offers Reopen.

**Architecture:** Parts 1 and 2's overlay. A `kind: 'create'` pending change carries the `UiEvent` to draw under a temporary negative id; the three overlays add it to the copies they already place. The create's write resolves `true` (part 2's "the store already holds it"), so the next load draws the real event and drops the copy in one update.

**Tech Stack:** Svelte 5 runes, TypeScript, Playwright (unit specs run in Node; app specs against the harness in Chromium and WebKit).

**Spec:** `docs/superpowers/specs/2026-10-10-omacal-pending-changes-part3-design.md`

## Global Constraints

- Payloads stay exactly what the backend sent; every change is drawn by the pure overlays in `ui/src/lib/pendingview.ts`.
- Unit specs run in Node: logic lives in plain `.ts`; only `pending.svelte.ts` and components use runes.
- Refusal text, verbatim: `Could not create “<title>”: <reason>` (curly quotes), `<title>` = `fields.summary ?? '(no title)'`.
- The created-not-stored sentence is matched by `String(e).startsWith('The event was created on Google')`, as today.
- The banner button's label is `Reopen`; the tooltip line is `Saving…` (one ellipsis character).
- A calendar with no `color_hex` draws `#5b8def` (`commands.rs` `DEFAULT_EVENT_COLOR`).
- `.ics` imports keep `refreshAfterWrite`.
- Commands run from `ui/` unless stated: unit `./node_modules/.bin/playwright test tests/<file> --project=chromium`; app `./node_modules/.bin/playwright test tests/app.spec.ts -g "<name>"`; types `npm run check`.
- Every commit ends with `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`.

## Review Focus

1. **A load that brings in the real event before the queue's own reload** (a week step after the write landed): the event is drawn once, not twice, and not dashed. Pinned in Task 3 (`a new event the next load brings in is drawn once`).
2. **Two creates saved in quick succession**: both drawn, each its own, no key collision. Pinned in Task 1 (`two new events are both drawn, each its own`).
3. **An all-day create spanning two Month rows**: a bar in each row, continued across the edge. Pinned in Task 1 (`a new all-day event crossing a Month row is a bar in both rows`).
4. **A create with a Meet link requested**: drawn with no link (no Join) until it saves. Pinned in Task 2 (`no title, guests and a requested Meet`).
5. **Clicking an unsaved event in Month** (App's own guard, not WeekGrid's): nothing opens, no `event_detail` call, no error. Pinned in Task 3 (`in Month, a new event opens nothing while it saves`).

Known gap, deliberately not built: Backspace on the keyboard cursor over an unsaved event asks the backend for its detail and reports that the event is not there (a second-long window, keyboard mode only).

---

### Task 1: The create change, drawn in every view

**Files:**
- Modify: `ui/src/lib/pendingview.ts`
- Test: `ui/tests/pendingview.spec.ts`

**Interfaces:**
- Produces: `PendingChange` variant `{ kind: 'create'; id: number; event: UiEvent }`; `export const isUnsaved = (ev: Pick<UiEvent, 'id'>): boolean`.

- [ ] **Step 1: Write the failing tests** — append to `ui/tests/pendingview.spec.ts`, and add `isUnsaved` to the import from `'../src/lib/pendingview'`:

```ts
const created = (id: number, start: number, end: number, extra: Partial<UiEvent> = {}): PendingChange =>
  ({ kind: 'create', id, event: ev(id, start, end, { title: 'New', pending: true, ...extra }) });

test.describe('creates', () => {
  test('a new meeting joins its day, laid out beside what it overlaps', () => {
    const w = overlayWeek(week(), [created(-1, MON + 9 * H, MON + 10 * H)]);
    const mon = w.days[0];
    expect(mon.events.map((e) => [e.id, e.pending ?? false])).toEqual([[1, false], [-1, true], [2, false]]);
    expect(mon.placed.filter((p) => mon.events[p.idx].id !== 2).map((p) => p.columns)).toEqual([2, 2]);
  });

  test('a new all-day event joins the band by first fit', () => {
    const w = overlayWeek(week(), [created(-1, MON, MON + DAY, { is_all_day: true })]);
    const i = w.all_day_events.findIndex((e) => e.id === -1);
    expect(w.all_day.find((l) => l.idx === i)).toMatchObject({ lane: 1, start_col: 0, end_col: 0 });
    expect(w.all_day.filter((l) => l.idx !== i)).toEqual(week().all_day); // no other lane moved
  });

  test('two new events are both drawn, each its own', () => {
    const w = overlayWeek(week(), [created(-1, MON + 13 * H, MON + 14 * H), created(-2, MON + 13 * H, MON + 14 * H)]);
    expect(w.days[0].events.filter((e) => e.id < 0).map((e) => e.id).sort()).toEqual([-2, -1]);
  });

  test('covers no stored event, and locks only its own id', () => {
    const c = created(-1, MON + 9 * H, MON + 10 * H);
    expect(covers(c, ev(1, MON + 9 * H, MON + 10 * H))).toBe(false);
    expect(locks(c, -1, MON + 9 * H)).toBe(true);
    expect(locks(c, 1, MON + 9 * H)).toBe(false);
    expect([isUnsaved({ id: -1 }), isUnsaved({ id: 4 })]).toEqual([true, false]);
  });

  test('in Month a new meeting is a line in its start cell, a new all-day event a bar', () => {
    const mon = local(2024, 0, 29), tue = local(2024, 0, 30), wed = local(2024, 0, 31);
    const m: MonthPayload = {
      year: 2024, month: 1, lane_cap: 3,
      rows: [{
        cells: [
          { start_ms: mon, end_ms: tue, in_month: true, timed: [] },
          { start_ms: tue, end_ms: wed, in_month: true, timed: [] },
        ],
        bars: [], bar_events: [], bar_overflow: [],
      }],
    };
    const out = overlayMonth(m, [created(-1, tue + 9 * H, tue + 10 * H), created(-2, mon, wed, { is_all_day: true })]);
    const row = out.rows[0];
    expect(row.cells.map((c) => c.timed.map((e) => e.id))).toEqual([[], [-1]]);
    const i = row.bar_events.findIndex((e) => e.id === -2);
    expect(row.bars.find((l) => l.idx === i)).toMatchObject({ lane: 0, start_col: 0, end_col: 1 });
  });

  test('a new all-day event crossing a Month row is a bar in both rows', () => {
    const sun = local(2024, 1, 4), mon = local(2024, 1, 5), tue = local(2024, 1, 6);
    const row = (a: number, b: number) => ({
      cells: [{ start_ms: a, end_ms: b, in_month: true, timed: [] as UiEvent[] }],
      bars: [] as Lane[], bar_events: [] as UiEvent[], bar_overflow: [] as number[],
    });
    const m: MonthPayload = { year: 2024, month: 2, lane_cap: 3, rows: [row(sun, mon), row(mon, tue)] };
    const out = overlayMonth(m, [created(-1, sun, tue, { is_all_day: true })]);
    expect(out.rows.map((r) => r.bars.map((l) => [l.start_col, l.end_col, l.cont_left, l.cont_right])))
      .toEqual([[[0, 0, false, true]], [[0, 0, true, false]]]);
  });

  test('in Big Year a new all-day event takes a pill, a new timed one nothing', () => {
    const days = Array.from({ length: 28 }, (_, i) => ({ start_ms: local(2024, 0, 29 + i), in_year: true, unsynced: false }));
    const big: BigYearPayload = { year: 2024, lane_cap: 3, rows: [{ days, pills: [], pill_events: [], overflow: [] }] };
    const out = overlayBigYear(big, [
      created(-1, days[2].start_ms, days[3].start_ms, { is_all_day: true }),
      created(-2, days[4].start_ms + 9 * H, days[4].start_ms + 10 * H),
    ]);
    expect(out.rows[0].pill_events.map((e) => e.id)).toEqual([-1]);
    expect(out.rows[0].pills).toEqual([lane(0, 0, 2, 2)]);
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `./node_modules/.bin/playwright test tests/pendingview.spec.ts --project=chromium`
Expected: FAIL — `does not provide an export named 'isUnsaved'`.

- [ ] **Step 3: Implement** in `ui/src/lib/pendingview.ts`.

Add the variant to `PendingChange` (after the `edit` arm):

```ts
  | {
      /** A new event (part 3 spec §2). `id` is temporary and negative, unique
       *  per create: no payload event has one, so nothing is covered and the
       *  `id:start` keys never collide. `event` is the copy to draw. */
      kind: 'create'; id: number; event: UiEvent;
    };
```

Below the type, add:

```ts
/** A new event Google has not given an id yet (part 3 spec §5): drawn, but
 *  with nothing to open. */
export const isUnsaved = (ev: Pick<UiEvent, 'id'>): boolean => ev.id < 0;

/** The new events the changes draw. */
const createdEvents = (changes: readonly PendingChange[]): UiEvent[] =>
  changes.flatMap((c) => (c.kind === 'create' ? [c.event] : []));
```

In `covers`, first line of the body:

```ts
  if (change.kind === 'create') return false;
```

In `locks`, first line of the body:

```ts
  if (change.kind === 'create') return id === change.id;
```

In `fate`'s loop, after `if (!covers(c, ev)) continue;`:

```ts
    if (c.kind === 'create') continue;
```

In `overlayWeek` and `overlayMonth`, replace `const arrivals = [...moved.values()];` with:

```ts
  const arrivals = [...moved.values(), ...createdEvents(changes)];
```

In `overlayBigYear`, replace `const spans = [...moved.values()].filter((e) => e.is_all_day);` with:

```ts
  const spans = [...moved.values(), ...createdEvents(changes)].filter((e) => e.is_all_day);
```

Update the doc comments of `overlayWeek` ("Moved, edited and deleted occurrences leave… ; a new one joins the same way"), and of `PendingChange` ("A move, an edit or a create says where…").

- [ ] **Step 4: Run the tests and the type check**

Run: `./node_modules/.bin/playwright test tests/pendingview.spec.ts --project=chromium` then `npm run check`
Expected: all pass; 0 errors.

- [ ] **Step 5: Commit**

```bash
git add ui/src/lib/pendingview.ts ui/tests/pendingview.spec.ts
git commit -m "feat(pending): a new event drawn in every view

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: The form's Save as a pending create

**Files:**
- Modify: `ui/src/lib/pendingedit.ts`
- Test: `ui/tests/pendingedit.spec.ts`

**Interfaces:**
- Consumes: Task 1's `create` variant of `PendingChange`.
- Produces: `export function drawnWhen(when: WhenInput, dayMs: (ymd: string) => number): { allDay: boolean; startMs: number; endMs: number }`; `export function createChange(result: EventFormResult, calendars: Pick<Calendar, 'id' | 'color_hex'>[], dayMs: (ymd: string) => number, id: number): PendingChange`.

- [ ] **Step 1: Write the failing tests** — in `ui/tests/pendingedit.spec.ts`, change the import to `import { createChange, drawnWhen, editChange, type EditRequest } from '../src/lib/pendingedit';` and append:

```ts
const make = (r: EventFormResult, cals: { id: number; color_hex: string | null }[] = calendars) =>
  createChange(r, cals, dayMs, -1) as any;

test('a new timed event is drawn at its own times, in its calendar\'s colour, as saving', () => {
  const c = make(result({ summary: 'Lunch', location: 'Cafe' }, { calendarId: 2 }));
  expect(c).toMatchObject({ kind: 'create', id: -1 });
  expect(c.event).toEqual({
    id: -1, calendar_id: 2, color: '#222222', title: 'Lunch', location: 'Cafe',
    start_ms: T, end_ms: T + H, is_all_day: false, response: 'accepted', attendees: 0,
    recurring: false, conference: null, all_guests_declined: false, pending: true,
  });
});

test('a new all-day event uses display-zone midnights, end exclusive', () => {
  const c = make(result({ when: { kind: 'allDay', startDate: '2024-01-29', endDate: '2024-01-31' } }));
  expect([c.event.is_all_day, c.event.start_ms, c.event.end_ms]).toEqual([true, dayMs('2024-01-29'), dayMs('2024-01-31')]);
});

test('a new repeating event is marked recurring and drawn once, at its own times', () => {
  const c = make(result({ repeat: 'weekly', weeklyDays: ['MO'] } as any));
  expect([c.event.recurring, c.event.start_ms, c.event.end_ms]).toEqual([true, T, T + H]);
});

test('no title, guests and a requested Meet', () => {
  const c = make(result({
    summary: null, conference: 'googleMeet',
    guests: [{ email: 'a@x.com', optional: false }, { email: 'b@x.com', optional: true }],
  } as any));
  expect([c.event.title, c.event.attendees, c.event.conference]).toEqual(['(no title)', 2, null]);
});

test('a calendar with no colour draws the backend\'s default', () => {
  expect(make(result({}, { calendarId: 3 }), [{ id: 3, color_hex: null }]).event.color).toBe('#5b8def');
});

test('drawnWhen is the rule both a create and an edit draw by', () => {
  expect(drawnWhen({ kind: 'timed', startMs: T, endMs: T + H }, dayMs)).toEqual({ allDay: false, startMs: T, endMs: T + H });
  expect(drawnWhen({ kind: 'allDay', startDate: '2024-01-29', endDate: '2024-01-30' }, dayMs))
    .toEqual({ allDay: true, startMs: dayMs('2024-01-29'), endMs: dayMs('2024-01-30') });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `./node_modules/.bin/playwright test tests/pendingedit.spec.ts --project=chromium`
Expected: FAIL — `does not provide an export named 'createChange'`.

- [ ] **Step 3: Implement** in `ui/src/lib/pendingedit.ts`. Add `import type { WhenInput } from './eventdetail';` and update the file's header comment to "The event form's Save, as a pending change (part 2 spec: edits; part 3 spec: creates)". Add:

```ts
/** Where a form's `WhenInput` is drawn: a timed one at its own instants, an
 *  all-day one from the display-zone midnight of its first day to that of the
 *  day after its last (the form's `endDate` is already exclusive). */
export function drawnWhen(
  when: WhenInput, dayMs: (ymd: string) => number,
): { allDay: boolean; startMs: number; endMs: number } {
  return when.kind === 'timed'
    ? { allDay: false, startMs: when.startMs, endMs: when.endMs }
    : { allDay: true, startMs: dayMs(when.startDate), endMs: dayMs(when.endDate) };
}

/** `commands.rs`' `DEFAULT_EVENT_COLOR`: what the backend draws for a
 *  calendar with no colour of its own. */
const DEFAULT_EVENT_COLOR = '#5b8def';

/**
 * A new event, as the overlays draw it until Google has it (part 3 spec §3).
 *
 * `id` is the temporary negative id `unsavedId()` hands out. A repeating
 * event is drawn as the form's own first occurrence: only the backend expands
 * a rule. `toEventInput` sends the repeat fields on a create only when a
 * repeat is set. A requested Meet link exists only once Google mints it.
 */
export function createChange(
  result: EventFormResult, calendars: Pick<Calendar, 'id' | 'color_hex'>[],
  dayMs: (ymd: string) => number, id: number,
): PendingChange {
  const f = result.fields;
  const w = drawnWhen(f.when, dayMs);
  return {
    kind: 'create', id,
    event: {
      id, calendar_id: result.calendarId,
      color: calendars.find((c) => c.id === result.calendarId)?.color_hex ?? DEFAULT_EVENT_COLOR,
      title: f.summary ?? '(no title)', location: f.location,
      start_ms: w.startMs, end_ms: w.endMs, is_all_day: w.allDay,
      response: 'accepted', attendees: f.guests?.length ?? 0,
      recurring: f.repeat !== undefined, conference: null, all_guests_declined: false,
      pending: true,
    },
  };
}
```

In `editChange`, replace the timed/all-day branches of `when` with `drawnWhen`:

```ts
  const when = repeatChanged || seriesSwitch ? null : drawnWhen(f.when, dayMs);
```

- [ ] **Step 4: Run the tests and the type check**

Run: `./node_modules/.bin/playwright test tests/pendingedit.spec.ts --project=chromium` then `npm run check`
Expected: all pass (the existing edit tests included); 0 errors.

- [ ] **Step 5: Commit**

```bash
git add ui/src/lib/pendingedit.ts ui/tests/pendingedit.spec.ts
git commit -m "feat(pending): the form's Save as a pending create

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: A create queues, and an unsaved event is inert

**Files:**
- Modify: `ui/src/lib/pending.svelte.ts` (`unsavedId`)
- Modify: `ui/src/App.svelte` (create arm, `saveQuick`, `openOccurrence` guard, imports)
- Modify: `ui/src/lib/WeekGrid.svelte` (`openPopover` guard)
- Modify: `ui/src/lib/EventBlock.svelte` (tooltip line)
- Modify: `ui/tests/harness/tauri.ts` (`holdNextWrite('create_event')`)
- Test: `ui/tests/app.spec.ts`

**Interfaces:**
- Consumes: `createChange` (Task 2), `isUnsaved` (Task 1), part 2's `queueChange`, `queuedRefresh`, `ymdMs`.
- Produces: `export const unsavedId = (): number` in `pending.svelte.ts`; `function queueCreate(result: EventFormResult): void` in `App.svelte` (Task 4 adds Reopen to its `onfailure`).

- [ ] **Step 1: Let the harness hold a create.** In `ui/tests/harness/tauri.ts`: widen `holdWriteOnce`'s type and `holdNextWrite`'s parameter to `'update_event' | 'delete_event_cmd' | 'create_event'`, and at the top of `case 'create_event':` add:

```ts
        if (holdWriteOnce === 'create_event') {
          holdWriteOnce = null;
          return new Promise((resolve, reject) => {
            parkedWrite = { resolve: (given) => resolve(given ?? CREATED_DETAIL), reject: (m) => reject(new Error(m)) };
          });
        }
```

- [ ] **Step 2: Write the failing app specs** — inside `test.describe('a change shows at once and saves behind you', …)` in `ui/tests/app.spec.ts`, add `CREATED_DETAIL` to the fixtures import if it is not there, and add:

```ts
      /** `n`, a title, Create: a new event on the anchor day (Mon 29 Jan). */
      const createLunch = async (page: Page, setUp: (form: ReturnType<typeof newForm>) => Promise<void> = async () => {}) => {
        await page.keyboard.press('n');
        await expect(newForm(page)).toBeVisible();
        await newForm(page).getByLabel('Title', { exact: true }).fill('Lunch');
        await setUp(newForm(page));
        await newForm(page).getByRole('button', { name: 'Create', exact: true }).click();
        await expect(newForm(page)).toHaveCount(0);
      };

      test('a new event shows at once, dashed, while it saves', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('create_event'));
        await createLunch(page);
        await expect(block(page, 'Lunch')).toHaveClass(/pending/);
        await expect(page.locator('header [role="status"]').filter({ hasText: 'Saving 1 change' })).toBeVisible();
        await page.evaluate(() => window.__harness.releaseWrite());
      });

      test('a new all-day event shows in the band at once', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('create_event'));
        await createLunch(page, (f) => f.getByLabel('All day').check());
        await expect(chip(page, 'Lunch')).toHaveClass(/pending/);
        await page.evaluate(() => window.__harness.releaseWrite());
      });

      test('a new event opens nothing while it saves, and says it is saving', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('create_event'));
        await createLunch(page);
        await block(page, 'Lunch').hover();
        await expect(page.locator('.tip').filter({ hasText: 'Saving…' })).toBeVisible();
        await block(page, 'Lunch').click();
        await page.waitForTimeout(300);
        await expect(page.getByRole('dialog')).toHaveCount(0);
        expect(await callsTo(page, 'event_detail')).toHaveLength(0);
        await expect(page.locator('.err')).toHaveCount(0);
        await page.evaluate(() => window.__harness.releaseWrite());
      });

      test('in Month, a new event opens nothing while it saves', async ({ page }) => {
        await writable(page);
        await page.evaluate((m) => window.__harness.setResponseData({ month: m }), appJanuaryMonth());
        await page.keyboard.press('3');
        await expect(page.locator('.timed').filter({ hasText: 'Board prep' })).toBeVisible();
        await page.evaluate(() => window.__harness.holdNextWrite('create_event'));
        await createLunch(page);
        const line = page.locator('.timed').filter({ hasText: 'Lunch' });
        await expect(line).toHaveClass(/pending/);
        await line.click();
        await page.waitForTimeout(300);
        await expect(page.getByRole('dialog')).toHaveCount(0);
        expect(await callsTo(page, 'event_detail')).toHaveLength(0);
        await expect(page.locator('.err')).toHaveCount(0);
        await page.evaluate(() => window.__harness.releaseWrite());
      });

      test('a refused create takes it away, with the reason', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('create_event'));
        await createLunch(page);
        await expect(block(page, 'Lunch')).toHaveClass(/pending/);
        await page.evaluate(() => window.__harness.rejectWrite('the server said no'));
        await expect(block(page, 'Lunch')).toHaveCount(0);
        await expect(page.locator('.err')).toContainText('Could not create “Lunch”');
        expect(await callsTo(page, 'sync_now')).toHaveLength(0);
      });

      test('a create that reached Google stays drawn until the sync brings it in', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => {
          window.__harness.holdNextSync();
          window.__harness.failNextCreate('The event was created on Google, but OmaCal could not record it locally. ' +
            'The next sync will bring it in — do not create it again.');
        });
        await createLunch(page);
        await expect(block(page, 'Lunch')).toHaveClass(/pending/);
        await expect(page.locator('.err')).toContainText('The event was created on Google');
        await page.evaluate(() => window.__harness.releaseSync());
        await expect(block(page, 'Lunch')).toHaveCount(0); // the harness's week has no Lunch
        expect(await callsTo(page, 'create_event')).toHaveLength(1);
      });

      test('Quick Add\'s direct create shows at once', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('create_event'));
        await page.keyboard.press('q');
        const quick = page.getByRole('dialog', { name: 'Quick add event' });
        await quick.getByLabel('Describe the event').fill('at 2pm Lunch');
        await quick.getByRole('button', { name: /^Create/ }).click();
        await expect(quick).toHaveCount(0);
        await expect(block(page, 'Lunch')).toHaveClass(/pending/);
        await page.evaluate(() => window.__harness.releaseWrite());
      });

      test('a new event survives a week step while it saves', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('create_event'));
        await createLunch(page);
        await page.getByRole('button', { name: 'Next week' }).click();
        await page.getByRole('button', { name: 'Previous week' }).click();
        await expect(block(page, 'Lunch')).toHaveClass(/pending/);
        await page.evaluate(() => window.__harness.releaseWrite());
      });

      test('a new repeating event draws one occurrence at once', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('create_event'));
        await createLunch(page, (f) => f.getByLabel('Repeat', { exact: true }).selectOption('daily'));
        await expect(block(page, 'Lunch')).toHaveCount(1);
        await expect(block(page, 'Lunch')).toHaveClass(/pending/);
        await page.evaluate(() => window.__harness.releaseWrite());
      });

      test('a new event the next load brings in is drawn once', async ({ page }) => {
        // `create_impl` stores the row it answers with, so a load after the
        // write draws the real event; the copy must go in the same update.
        await writable(page);
        const w = appWritableWeek();
        const mon = w.days[0];
        mon.events.push({ ...mon.events[1], id: CREATED_DETAIL.id, title: 'Lunch' });
        mon.placed.push({ ...mon.placed[1], idx: mon.events.length - 1, top: 0.5 });
        await page.evaluate(() => window.__harness.holdNextSync());
        await createLunch(page);
        await expect.poll(() => callsTo(page, 'sync_now')).toHaveLength(1); // written; sync held
        await page.evaluate((week) => window.__harness.setResponseData({ week }), w);
        await page.getByRole('button', { name: 'Next week' }).click();
        await page.getByRole('button', { name: 'Previous week' }).click();
        await expect(block(page, 'Lunch')).toHaveCount(1);
        await expect(block(page, 'Lunch')).not.toHaveClass(/pending/);
        await page.evaluate(() => window.__harness.releaseSync());
      });
```

- [ ] **Step 3: Run them to see them fail**

Run: `./node_modules/.bin/playwright test tests/app.spec.ts -g "new event|new all-day|refused create|reached Google stays|direct create shows|new repeating"`
Expected: FAIL — the new event is not drawn while the write is held (`toHaveClass(/pending/)` finds no `Lunch` block), and the created-not-stored spec finds no block.

- [ ] **Step 4: Implement.**

`ui/src/lib/pending.svelte.ts`, after the exports:

```ts
let unsaved = 0;
/** A temporary id for a new event until Google gives it one (part 3 spec
 *  §2): negative, so no stored event shares it. */
export const unsavedId = (): number => --unsaved;
```

`ui/src/App.svelte`: add `unsavedId` to the `./lib/pending.svelte` import, `isUnsaved` to the `./lib/pendingview` import, and `createChange` to the `./lib/pendingedit` import. Add, above `saveForm`:

```ts
  /**
   * A new event, drawn at once and created behind (part 3 spec, 2026-10-10).
   *
   * **`result.notify`, never a constant**: a create can invite people, the
   * form asks, and this carries the answer.
   *
   * `create_impl` stores the row it answers with, so a write that answers says
   * the store holds it (part 2's rule) and the next load draws the real event.
   * One failure is not a failure: the backend's fixed created-not-stored
   * sentence (events.rs, safelisted verbatim) means the event exists and its
   * guests are already mailed, so it must not be undone the way a refusal is,
   * which would invite creating it again. It stays drawn, the banner keeps the
   * sentence, and the follow-up sync fetches it like any other.
   */
  function queueCreate(result: EventFormResult) {
    error = null;
    const title = result.fields.summary ?? '(no title)';
    void queueChange(createChange(result, calendars, ymdMs, unsavedId()), {
      write: async () => {
        try {
          await createEvent(result.calendarId, result.fields, result.notify);
          return true;
        } catch (e) {
          if (!String(e).startsWith('The event was created on Google')) throw e;
          error = String(e);
          return false;
        }
      },
      ...queuedRefresh,
      onfailure: (e) => { error = `Could not create “${title}”: ${String(e)}`; },
    });
  }
```

In `saveForm`, replace everything after the edit arm's `return; }` (the `busy = true; … await refreshAfterWrite();` block) with:

```ts
    queueCreate(result);
```

Replace `saveQuick`'s body after `quickAdd = null;` the same way, and make it a plain `function` (it no longer awaits):

```ts
  function saveQuick(result: EventFormResult) {
    if (!quickAdd) return;
    quickAdd = null;
    queueCreate(result);
  }
```

Update `saveQuick`'s doc comment: "Quick-add's direct create: the same queued create as the form's (`queueCreate`), with its created-not-stored rule."

In `openOccurrence`, first line of the body:

```ts
    // Not on Google yet: nothing to open until it saves (part 3 spec §5).
    if (isUnsaved({ id })) return;
```

`ui/src/lib/WeekGrid.svelte`: import `isUnsaved` beside `overlayDetail` from `./pendingview`, and in `openPopover` after `if (draggedNotClicked) return;`:

```ts
    // Not on Google yet: nothing to open until it saves (part 3 spec §5).
    if (isUnsaved(event)) return;
```

`ui/src/lib/EventBlock.svelte`: import `isUnsaved` from `./pendingview`, and in the tooltip, after the `<span class="tw">…</span>` line:

```svelte
      {#if isUnsaved(event)}<span class="tw">Saving…</span>{/if}
```

- [ ] **Step 5: Run the new specs**

Run: `./node_modules/.bin/playwright test tests/app.spec.ts -g "new event|new all-day|refused create|reached Google stays|direct create shows|new repeating"`
Expected: all pass, both engines.

- [ ] **Step 6: Prove the guards** — one at a time, delete the `isUnsaved` line in `WeekGrid.openPopover`, then the one in `App.openOccurrence`, and run `-g "opens nothing while it saves"`: each removal turns its own spec red (an `event_detail` call and an error banner). Restore both.

- [ ] **Step 7: Run the create-adjacent specs**

Run: `./node_modules/.bin/playwright test tests/app.spec.ts tests/duplicate-event.spec.ts tests/quickevent.spec.ts`
Expected: all pass. In particular `a create that reached Google heals by syncing, not by retrying` (one `sync_now`, banner clear after the reload) and `a create Google refused stops without the heal` (no `sync_now`) must hold unchanged. If an existing spec asserted the old wait (for example a `busy` state during a create), update it to the queued behaviour and ledger the ruling.

- [ ] **Step 8: Type check and commit**

Run: `npm run check` — Expected: 0 errors.

```bash
git add ui/src/lib/pending.svelte.ts ui/src/App.svelte ui/src/lib/WeekGrid.svelte ui/src/lib/EventBlock.svelte ui/tests/harness/tauri.ts ui/tests/app.spec.ts
git commit -m "feat(pending): a new event shows at once and is created behind

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: A refused create offers Reopen

**Files:**
- Modify: `ui/src/lib/eventform.ts` (`EventFormResult.value`)
- Modify: `ui/src/lib/EventForm.svelte` (result carries the value)
- Modify: `ui/src/lib/QuickEventModal.svelte` (result carries the parsed value)
- Modify: `ui/src/lib/Header.svelte` (`errorAction`)
- Modify: `ui/src/App.svelte` (`retry` state, `queueCreate`'s `onfailure`)
- Test: `ui/tests/app.spec.ts`

**Interfaces:**
- Consumes: Task 3's `queueCreate`.
- Produces: `EventFormResult.value: EventFormValue`; Header prop `errorAction?: { label: string; run: () => void } | null`.

- [ ] **Step 1: Write the failing specs** — beside Task 3's, in the same describe:

```ts
      test('a refused create offers to reopen what was typed', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('create_event'));
        await createLunch(page, (f) => f.getByLabel('Location', { exact: true }).fill('Cafe'));
        await page.evaluate(() => window.__harness.rejectWrite('the server said no'));
        await expect(page.locator('.err')).toContainText('Could not create “Lunch”');

        await page.locator('.err').getByRole('button', { name: 'Reopen' }).click();
        await expect(newForm(page)).toBeVisible();
        await expect(newForm(page).getByLabel('Title', { exact: true })).toHaveValue('Lunch');
        await expect(newForm(page).getByLabel('Location', { exact: true })).toHaveValue('Cafe');
        await expect(page.locator('.err').getByRole('button', { name: 'Reopen' })).toHaveCount(0);
      });

      test('Reopen goes when the banner clears', async ({ page }) => {
        await writable(page);
        await page.evaluate(() => window.__harness.holdNextWrite('create_event'));
        await createLunch(page);
        await page.evaluate(() => window.__harness.rejectWrite('the server said no'));
        await expect(page.locator('.err').getByRole('button', { name: 'Reopen' })).toBeVisible();
        await page.getByRole('button', { name: 'Next week' }).click(); // a load clears the banner
        await expect(page.locator('.err')).toHaveCount(0);
        await expect(page.getByRole('button', { name: 'Reopen' })).toHaveCount(0);
      });
```

- [ ] **Step 2: Run them to see them fail**

Run: `./node_modules/.bin/playwright test tests/app.spec.ts -g "Reopen|reopen what was typed"`
Expected: FAIL — no `Reopen` button.

- [ ] **Step 3: Implement.**

`ui/src/lib/eventform.ts`, in `EventFormResult` after `fields`:

```ts
  /** The form's own value at Save, so a refused create can be reopened with
   *  everything that was typed (part 3 spec §6). */
  value: EventFormValue;
```

`ui/src/lib/EventForm.svelte`, the result line:

```ts
    const result = { calendarId: value.calendarId, scope, value: $state.snapshot(value), fields: toEventInput(value, initial, zoneName()) };
```

`ui/src/lib/QuickEventModal.svelte`, in `create()`'s `oncreate({ … })`, after `scope: 'this',`:

```ts
      value: parsed.value,
```

`ui/src/lib/Header.svelte`: add `errorAction = null,` to the destructured props (after `error,`) and to the props type:

```ts
    /** A button beside the error, for a failure with a next step (a refused
     *  create's Reopen, part 3 spec §6). */
    errorAction?: { label: string; run: () => void } | null;
```

Replace the error paragraph:

```svelte
{#if error}
  <p class="err" class:with-action={!!errorAction}>
    <span>{error}</span>
    {#if errorAction}<button type="button" onclick={errorAction.run}>{errorAction.label}</button>{/if}
  </p>
{/if}
```

and widen the two `.reauth` rules' selectors to `.reauth, .with-action { … }` and `.reauth button, .with-action button { … }`, adding to the comment above them: "and a failure with a next step (`errorAction`)".

`ui/src/App.svelte`, beside `queueCreate`:

```ts
  /** The banner's button for a refused create (part 3 spec §6): shown only
   *  while the banner still holds the message it was set with, so the next
   *  load (which clears the banner) or a later error takes it away. */
  let retry = $state<{ forError: string; label: string; run: () => void } | null>(null);
```

`queueCreate`'s `onfailure`:

```ts
      onfailure: (e) => {
        const message = `Could not create “${title}”: ${String(e)}`;
        error = message;
        retry = {
          forError: message, label: 'Reopen',
          run: () => {
            error = null;
            retry = null;
            form = { mode: 'create', anchor: keyboardAnchor(), initial: result.value };
          },
        };
      },
```

and on `<Header …>`:

```svelte
    errorAction={retry && retry.forError === error ? retry : null}
```

- [ ] **Step 4: Run the specs and the type check**

Run: `./node_modules/.bin/playwright test tests/app.spec.ts -g "Reopen|reopen what was typed"` then `npm run check`
Expected: pass, both engines; 0 errors (any other `EventFormResult` literal the check reports gets `value`).

- [ ] **Step 5: Run the form and header specs**

Run: `./node_modules/.bin/playwright test tests/app.spec.ts tests/eventform.spec.ts tests/components.spec.ts tests/quickevent.spec.ts`
Expected: all pass. A snapshot of the header banner that now wraps its text in a `<span>` is updated only if the rendering changed, with a ruling.

- [ ] **Step 6: Commit**

```bash
git add ui/src/lib/eventform.ts ui/src/lib/EventForm.svelte ui/src/lib/QuickEventModal.svelte ui/src/lib/Header.svelte ui/src/App.svelte ui/tests/app.spec.ts
git commit -m "feat(pending): a refused create offers Reopen with what was typed

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Full gates, review, push, record

- [ ] **Step 1: Full gates** — from the repo root: `cargo clippy --workspace --all-targets -- -D warnings` (exit 0); `npm --prefix ui run check` (0 errors); from `ui/`: `./node_modules/.bin/playwright test > ../target/pw-p3.log 2>&1; echo EXIT=$? >> ../target/pw-p3.log` → `EXIT=0`. A load flake (passes alone) is noted, not "fixed".
- [ ] **Step 2: Whole-branch review** by a fresh reviewer on the most capable model, given this plan's Review Focus and the ledger's rulings, then one fix pass (each fix red→green, full suite green).
- [ ] **Step 3: Push and verify main's CI run by commit sha** (`gh run list --branch main --json databaseId,headSha` filtered by the pushed sha; all three jobs `success`).
- [ ] **Step 4: Record** in `~/dev/omacal-private/OPERATIONS.md`'s "Where things stand": part 3 on main (unreleased, with part 2), commits, the field check owed (Plamen creates a real meeting, an all-day event and a repeating one; clicks one while it saves; Quick Add one). Commit and push that repo.
