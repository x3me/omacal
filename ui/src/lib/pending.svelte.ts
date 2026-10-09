// The pending-changes store the views read (spec 2026-10-09). The logic is
// `PendingQueue`'s; this only makes it reactive, by bumping `version` on every
// change and reading it in each getter.

import { PendingQueue, type Refresh, type Work } from './pendingqueue';
import type { PendingChange } from './pendingview';

let version = $state(0);
const queue = new PendingQueue(() => { version++; });

export const pendingChanges = (): PendingChange[] => { void version; return queue.changes(); };
export const pendingChangeCount = (): number => { void version; return queue.count(); };
export const isPending = (id: number, startMs: number): boolean => { void version; return queue.isPending(id, startMs); };
export const holdChange = (change: PendingChange): symbol => queue.hold(change);
export const releaseHold = (token: symbol): void => queue.release(token);
export const commitChange = (token: symbol, change: PendingChange, work: Work): Promise<void> =>
  queue.commit(token, change, work);
export const queueChange = (change: PendingChange, work: Work): Promise<void> => queue.queue(change, work);
export const pendingCheckpoint = (): number => queue.checkpoint();
export const reconcilePending = (checkpoint: number): void => queue.reconcile(checkpoint);
export const pendingIdle = (): Promise<void> => queue.idle();
export const resyncPending = (refresh: Refresh): Promise<void> => queue.resync(refresh);
