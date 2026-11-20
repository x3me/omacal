import { test, expect } from '@playwright/test';
import { whileVisible } from '../src/lib/whilevisible';

/** A document the test drives: `hidden` it sets, `visibilitychange` it fires. */
function fakeDoc(hidden: boolean) {
  const listeners = new Set<() => void>();
  return {
    hidden,
    addEventListener: (_: string, f: () => void) => listeners.add(f),
    removeEventListener: (_: string, f: () => void) => listeners.delete(f),
    set(next: boolean) { this.hidden = next; for (const f of listeners) f(); },
    listeners,
  };
}

/** Timers the test steps by hand. */
function fakeTimers() {
  let id = 0;
  const live = new Map<number, { ms: number; run: () => void }>();
  return {
    setInterval: (run: () => void, ms: number) => { live.set(++id, { ms, run }); return id; },
    clearInterval: (i: number) => { live.delete(i); },
    tick() { for (const t of live.values()) t.run(); },
    live,
  };
}

/** The menubar popup is a window that is only ever hidden, never closed, so
 *  its 15-second feed poll and its 1-second clock ran for the whole life of
 *  the app once the tray had been clicked once. They run only while it is
 *  visible now, and it refreshes once on being shown. */
test('timers run only while the document is visible, and showing it refreshes once', () => {
  const doc = fakeDoc(true);
  const timers = fakeTimers();
  let polls = 0;
  let shown = 0;
  const stop = whileVisible(doc, [{ ms: 15000, run: () => polls++ }], () => shown++, timers);

  expect(timers.live.size, 'nothing runs while hidden').toBe(0);
  doc.set(false);
  expect(shown).toBe(1);
  expect(timers.live.size).toBe(1);
  timers.tick();
  expect(polls).toBe(1);

  doc.set(true);
  expect(timers.live.size, 'hiding stops them').toBe(0);
  timers.tick();
  expect(polls).toBe(1);

  stop();
  expect(doc.listeners.size, 'and the cleanup lets go of the document').toBe(0);
});

test('a document visible from the start runs at once, without a refresh of its own', () => {
  const doc = fakeDoc(false);
  const timers = fakeTimers();
  let shown = 0;
  whileVisible(doc, [{ ms: 1000, run: () => {} }, { ms: 15000, run: () => {} }], () => shown++, timers);
  expect(timers.live.size).toBe(2);
  expect(shown, 'the caller already loads on mount').toBe(0);
});
