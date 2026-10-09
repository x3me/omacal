// The pending-changes store the views read (spec 2026-10-09). The logic is
// `PendingQueue`'s; this only makes it reactive, by bumping `version` on every
// change and reading it in each getter.

import type { EventDetail } from './eventdetail';
import { PendingQueue, type Refresh, type Work } from './pendingqueue';
import { overlayDetail, type PendingChange } from './pendingview';

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

/**
 * Closes a details card once the pending edit it shows has cleared, landed or
 * refused (part 2 review). Its values were `overlayDetail`'s, drawn over a
 * detail fetched before the save: after a landed one that detail is stale, and
 * the card's Edit would write the old values back to Google. Only a card still
 * open on the same occurrence closes, and only one an edit had redrawn; a move
 * leaves the detail as stored. Called from a component's setup, where
 * `$effect` belongs.
 */
export function closeWhenEditClears(
  card: () => { detail: EventDetail; startMs: number } | null, close: () => void,
): void {
  let redrawn: string | null = null;
  $effect(() => {
    const c = card();
    const key = c ? `${c.detail.id}:${c.startMs}` : null;
    const edited = c !== null && overlayDetail(c.detail, pendingChanges(), c.startMs) !== c.detail;
    if (!edited && key !== null && key === redrawn) close();
    redrawn = edited ? key : null;
  });
}
