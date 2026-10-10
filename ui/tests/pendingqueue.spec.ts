import { test, expect } from '@playwright/test';
import { PendingQueue, storeHoldsShift, type Work } from '../src/lib/pendingqueue';
import type { PendingChange } from '../src/lib/pendingview';

const move = (id: number, from = 100, to = 200): Extract<PendingChange, { kind: 'move' }> =>
  ({ kind: 'move', id, occurrenceStartMs: from, scope: 'this', startMs: to, endMs: to + 50 });

/** A write the test resolves or rejects by hand. */
function gate() {
  let resolve!: () => void; let reject!: (e: unknown) => void;
  const promise = new Promise<void>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}
const work = (write: () => Promise<unknown>, extra: Partial<Work> = {}): Work =>
  ({ write, sync: async () => {}, reload: async () => {}, onfailure: () => {}, onsyncfailure: () => {}, ...extra });

test('a held change is drawn but not counted, and release takes it away', () => {
  const q = new PendingQueue();
  const t = q.hold(move(1));
  expect(q.changes()).toHaveLength(1);
  expect(q.count()).toBe(0);
  q.release(t);
  expect(q.changes()).toHaveLength(0);
});

test('a committed change is counted until written, and takes the scope it was committed with', async () => {
  const q = new PendingQueue();
  const g = gate();
  const t = q.hold(move(1));
  const done = q.commit(t, { ...move(1), scope: 'all' }, work(() => g.promise));
  expect(q.count()).toBe(1);
  expect(q.changes()[0]).toMatchObject({ scope: 'all' });
  g.resolve();
  await done;
  expect(q.count()).toBe(0);
  expect(q.changes()).toHaveLength(1); // saved, still drawn until a load lands
});

test('writes run one at a time, in the order made', async () => {
  const q = new PendingQueue();
  const order: string[] = [];
  const a = gate();
  const one = q.queue(move(1), work(async () => { order.push('a start'); await a.promise; order.push('a end'); }));
  const two = q.queue(move(2), work(async () => { order.push('b'); }));
  await Promise.resolve();
  expect(order).toEqual(['a start']);
  a.resolve();
  await Promise.all([one, two]);
  expect(order).toEqual(['a start', 'a end', 'b']);
});

test('a refused write drops the change, reports it, and does not stall the next', async () => {
  const q = new PendingQueue();
  const failures: unknown[] = [];
  const one = q.queue(move(1), work(async () => { throw new Error('no longer here'); }, { onfailure: (e) => failures.push(e) }));
  const two = q.queue(move(2), work(async () => {}));
  await Promise.all([one, two]);
  expect(String(failures[0])).toContain('no longer here');
  expect(q.changes().map((c) => c.id)).toEqual([2]);
});

test('a load begun before the save does not clear it; one begun after does', async () => {
  const q = new PendingQueue();
  const g = gate();
  const done = q.queue(move(1), work(() => g.promise));
  const before = q.checkpoint();  // a reload already in flight
  g.resolve();
  await done;
  q.reconcile(before);
  expect(q.changes()).toHaveLength(1);
  const after = q.checkpoint();   // a reload that started after the save
  q.reconcile(after);
  expect(q.changes()).toHaveLength(0);
});

test('a load begun after the write but before its sync does not clear it', async () => {
  // "This occurrence" of a series: the backend leaves the local store alone
  // and the follow-up sync brings it in, so until that sync is done a reload
  // would still read the old place (review finding 1, 2026-10-09).
  const q = new PendingQueue();
  const sync = gate();
  const done = q.queue(move(1), work(async () => {}, { sync: () => sync.promise }));
  await new Promise((r) => setTimeout(r, 0)); // the write has landed, the sync has not
  q.reconcile(q.checkpoint());
  expect(q.changes()).toHaveLength(1);
  sync.resolve();
  await done;
  q.reconcile(q.checkpoint());
  expect(q.changes()).toHaveLength(0);
});

test('a failed sync keeps the change drawn, reports it, and a later resync clears it', async () => {
  const q = new PendingQueue();
  const reasons: unknown[] = [];
  await q.queue(move(1), work(async () => {}, {
    sync: async () => { throw new Error('offline'); },
    onsyncfailure: (e) => reasons.push(e),
  }));
  expect(String(reasons[0])).toContain('offline');
  q.reconcile(q.checkpoint());
  expect(q.changes()).toHaveLength(1); // Google has it; the store does not yet
  expect(q.count()).toBe(0);           // nothing is being written any more

  let synced = 0;
  await q.resync({ sync: async () => { synced++; }, reload: async () => {} });
  expect(synced).toBe(1);
  q.reconcile(q.checkpoint());
  expect(q.changes()).toHaveLength(0);
});

test('a resync with nothing whose sync failed does nothing', async () => {
  const q = new PendingQueue();
  const sync = gate();
  void q.queue(move(1), work(async () => {}, { sync: () => sync.promise }));
  let synced = 0;
  await q.resync({ sync: async () => { synced++; }, reload: async () => {} });
  expect(synced).toBe(0); // its own sync is still running: no second pass
  sync.resolve();
});

test('isPending answers for where a move came from and where it landed', () => {
  const q = new PendingQueue();
  q.hold(move(1, 100, 200));
  expect(q.isPending(1, 100)).toBe(true);
  expect(q.isPending(1, 200)).toBe(true);
  expect(q.isPending(1, 300)).toBe(false);
});

test('every change of state is announced', async () => {
  let n = 0;
  const q = new PendingQueue(() => { n++; });
  const t = q.hold(move(1));
  await q.commit(t, move(1), work(async () => {}));
  q.reconcile(q.checkpoint());
  expect(n).toBeGreaterThanOrEqual(4); // hold, commit, saved, reconcile
});

test('a write that says the store already holds it clears with a load begun after it, before its sync', async () => {
  // `update_via_client` folds a patch of the row it loaded straight into the
  // store: a load before the sync already draws the change, and a series shift
  // drawn over it again would move every occurrence twice (part 2 review).
  const q = new PendingQueue();
  const s = gate();
  let written!: () => void;
  const wrote = new Promise<void>((r) => { written = r; });
  const done = q.queue(move(1), work(async () => { written(); return true; }, { sync: () => s.promise }));
  await wrote;
  await new Promise((r) => setTimeout(r, 0));
  q.reconcile(q.checkpoint()); // a load that began after the write, its sync still running
  expect(q.changes()).toHaveLength(0);
  s.resolve();
  await done;
});

test('a write answering anything but true still waits for its sync', async () => {
  const q = new PendingQueue();
  const s = gate();
  let written!: () => void;
  const wrote = new Promise<void>((r) => { written = r; });
  const done = q.queue(move(1), work(async () => { written(); return { start_ms: 1 }; }, { sync: () => s.promise }));
  await wrote;
  await new Promise((r) => setTimeout(r, 0));
  q.reconcile(q.checkpoint());
  expect(q.changes()).toHaveLength(1);
  s.resolve();
  await done;
});

test('the store holds a time change once the row an update answers with has moved', () => {
  const before = { start_ms: 1_000, end_ms: 2_000 };
  expect(storeHoldsShift(before, { start_ms: 4_600, end_ms: 5_600 })).toBe(true);
  expect(storeHoldsShift(before, { start_ms: 1_000, end_ms: 3_000 })).toBe(true);
  // "This occurrence" of a series: the backend patched a new instance and
  // left the master's row, which is what it answers with, alone.
  expect(storeHoldsShift(before, { start_ms: 1_000, end_ms: 2_000 })).toBe(false);
});
