// The pending-changes queue (spec 2026-10-09), as a plain class so Node-run
// specs can drive it: `pending.svelte.ts` wraps it in a rune for the views.
//
// `responses.svelte.ts` is the model, deliberately: one tail so writes run in
// order, and the checkpoint rule so a load already in flight when a save
// finishes cannot clear what that save is about.

import { locks, type PendingChange } from './pendingview';

export type Work = {
  /** The write to Google. A rejection means nothing changed there: the change
   *  is dropped from the screen and `onfailure` says why. */
  write: () => Promise<unknown>;
  /** After a successful write: sync and reload. Its own failure is its own to
   *  report, and does not undo the change: Google has it. */
  after: () => Promise<void>;
  onfailure: (error: unknown) => void;
};

/** `seq` is null while held (a question is open) and set once committed. */
type Entry = { token: symbol; change: PendingChange; seq: number | null; saved: boolean };

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
    this.entries = [...this.entries, { token, change, seq: null, saved: false }];
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
      : [...this.entries, { token, change, seq, saved: false }];
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
      this.entries = this.entries.map((e) => (e.token === token ? { ...e, saved: true } : e));
      this.onchange();
      try { await work.after(); } catch { /* `after` reports its own failure */ }
    });
    this.tail = done;
    return done;
  }

  queue(change: PendingChange, work: Work): Promise<void> {
    return this.commit(this.hold(change), change, work);
  }

  /** Taken when a load begins: the highest change already saved. */
  checkpoint(): number {
    return Math.max(0, ...this.entries.filter((e) => e.saved).map((e) => e.seq ?? 0));
  }

  /** Called when that load has landed: what it already contains is no longer
   *  pending. */
  reconcile(checkpoint: number): void {
    const before = this.entries.length;
    this.entries = this.entries.filter((e) => !(e.saved && (e.seq ?? 0) <= checkpoint));
    if (this.entries.length !== before) this.onchange();
  }

  /** Resolves once every queued write, including ones added meanwhile, is done. */
  async idle(): Promise<void> {
    let batch;
    do { batch = this.tail; await batch; } while (batch !== this.tail);
  }
}
