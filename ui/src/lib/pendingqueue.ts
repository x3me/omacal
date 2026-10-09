// The pending-changes queue (spec 2026-10-09), as a plain class so Node-run
// specs can drive it: `pending.svelte.ts` wraps it in a rune for the views.
//
// `responses.svelte.ts` is the model, deliberately: one tail so writes run in
// order, and the checkpoint rule so a load already in flight when a save
// finishes cannot clear what that save is about.

import { locks, type PendingChange } from './pendingview';

/** How a written change reaches the local store and the screen. */
export type Refresh = {
  /** A sync that starts after the write. Only once it is done does the local
   *  store hold the change: for "this occurrence" of a series the backend
   *  leaves the store alone and this sync brings it in, so the checkpoint
   *  waits for it, not for the write (review finding 1, 2026-10-09). */
  sync: () => Promise<void>;
  /** The reload that then clears the card. Reports its own failure. */
  reload: () => Promise<void>;
};

export type Work = Refresh & {
  /** The write to Google. A rejection means nothing changed there: the change
   *  is dropped from the screen and `onfailure` says why. */
  write: () => Promise<unknown>;
  onfailure: (error: unknown) => void;
  /** The sync after a successful write failed: Google has the change, so it
   *  stays drawn, and `resync` retries the sync later. */
  onsyncfailure: (error: unknown) => void;
};

/** `seq` is null while held (a question is open) and set once committed.
 *  `saved`: Google took the write. `synced`: a sync that began after the write
 *  has finished, so the local store has it. `syncFailed`: that sync failed and
 *  waits for `resync`. */
type Entry = {
  token: symbol; change: PendingChange; seq: number | null;
  saved: boolean; synced: boolean; syncFailed: boolean;
};

export class PendingQueue {
  private entries: Entry[] = [];
  private seq = 0;
  private tail: Promise<unknown> = Promise.resolve();

  constructor(private readonly onchange: () => void = () => {}) {}

  /** Every change to draw: held, being written, and written but not yet
   *  reloaded. */
  changes(): PendingChange[] {
    return this.entries.map((e) => e.change);
  }

  /** Changes still being written: the header's count. A held one is not
   *  counted, because nothing is being saved yet. */
  count(): number {
    return this.entries.filter((e) => e.seq !== null && !e.saved).length;
  }

  isPending(id: number, startMs: number): boolean {
    return this.entries.some((e) => locks(e.change, id, startMs));
  }

  hold(change: PendingChange): symbol {
    const token = Symbol('pending');
    this.entries = [...this.entries, { token, change, seq: null, saved: false, synced: false, syncFailed: false }];
    this.onchange();
    return token;
  }

  /** Cancel a held change. A committed one cannot be released. */
  release(token: symbol): void {
    const before = this.entries.length;
    this.entries = this.entries.filter((e) => e.token !== token || e.seq !== null);
    if (this.entries.length !== before) this.onchange();
  }

  /** Turn a held change into a write. `change` replaces what was held, because
   *  the answer to a question (the scope) can change what is drawn. */
  commit(token: symbol, change: PendingChange, work: Work): Promise<void> {
    const seq = ++this.seq;
    const held = this.entries.some((e) => e.token === token);
    this.entries = held
      ? this.entries.map((e) => (e.token === token ? { ...e, change, seq } : e))
      : [...this.entries, { token, change, seq, saved: false, synced: false, syncFailed: false }];
    this.onchange();
    const done = this.tail.then(async () => {
      try {
        await work.write();
      } catch (error) {
        this.entries = this.entries.filter((e) => e.token !== token);
        this.onchange();
        work.onfailure(error);
        return;
      }
      this.mark(token, { saved: true });
      try {
        await work.sync();
      } catch (error) {
        this.mark(token, { syncFailed: true });
        work.onsyncfailure(error);
        return;
      }
      this.mark(token, { synced: true });
      try { await work.reload(); } catch { /* `reload` reports its own failure */ }
    });
    this.tail = done;
    return done;
  }

  queue(change: PendingChange, work: Work): Promise<void> {
    return this.commit(this.hold(change), change, work);
  }

  /** Taken when a load begins: the highest change the local store already
   *  holds — synced, not merely written. */
  checkpoint(): number {
    return Math.max(0, ...this.entries.filter((e) => e.synced).map((e) => e.seq ?? 0));
  }

  /** Called when that load has landed: what it already contains is no longer
   *  pending. */
  reconcile(checkpoint: number): void {
    const before = this.entries.length;
    this.entries = this.entries.filter((e) => !(e.synced && (e.seq ?? 0) <= checkpoint));
    if (this.entries.length !== before) this.onchange();
  }

  /** Retry the sync for every change whose follow-up sync failed. Called when
   *  a background sync finishes, which says the server is reachable again;
   *  `refresh.sync` starts a pass of its own after it, so it began after every
   *  one of those writes. Does nothing when no sync has failed. */
  async resync(refresh: Refresh): Promise<void> {
    const tokens = this.entries.filter((e) => e.syncFailed).map((e) => e.token);
    if (tokens.length === 0) return;
    try { await refresh.sync(); } catch { return; }
    for (const token of tokens) this.mark(token, { synced: true, syncFailed: false });
    try { await refresh.reload(); } catch { /* reports its own failure */ }
  }

  private mark(token: symbol, patch: Partial<Pick<Entry, 'saved' | 'synced' | 'syncFailed'>>): void {
    this.entries = this.entries.map((e) => (e.token === token ? { ...e, ...patch } : e));
    this.onchange();
  }

  /** Resolves once every queued write, including ones added meanwhile, is done. */
  async idle(): Promise<void> {
    let batch;
    do { batch = this.tail; await batch; } while (batch !== this.tail);
  }
}
