import { untrack } from 'svelte';
import { invoke } from '@tauri-apps/api/core';
import type { EventDetail } from './eventdetail';

type Response = 'accepted' | 'tentative' | 'declined';
type Request = { id: number; response: Response; scope: 'this' | 'all'; occurrenceStartMs: number };
type Job = Request & { promise: Promise<EventDetail>; sequence: number; saved: boolean };
type Target = Pick<Request, 'id' | 'scope' | 'occurrenceStartMs'>;
type Failure = Target & { key: string; message: string };
const targetKey = (target: Target) => `${target.id}:${target.scope}:${target.scope === 'all' ? '' : target.occurrenceStartMs}`;

// Replies made in this window share a queue, independent of any popover.
// Saved replies keep their display override until a later payload lands.
// A failed sync of another calendar must not pin an override for the session.
let jobs = $state.raw<Job[]>([]);
let sequence = 0;
let failures = $state<Failure[]>([]);
let tail: Promise<unknown> = Promise.resolve();

export const pendingResponseCount = () => jobs.filter(job => !job.saved).length;
export const responsePending = (id: number, startMs?: number) => jobs.some(job => !job.saved && job.id === id
  && (startMs === undefined || job.scope === 'all' || job.occurrenceStartMs === startMs));
export const responseFailures = () => failures;
export const responseFailure = (id: number, startMs?: number) => [...failures].reverse().find(f =>
  f.id === id && (f.scope === 'all' || f.occurrenceStartMs === startMs));

// A visible row/popover owns its error; the header carries it when that
// surface closes. Both dismiss the same record, so an old copy cannot return.
let failureHosts = $state.raw(new Map<symbol, string[]>());
export function showResponseFailuresHere(keys: string[]) {
  const key = Symbol();
  untrack(() => { failureHosts = new Map(failureHosts).set(key, keys); });
  return () => untrack(() => {
    const next = new Map(failureHosts); next.delete(key); failureHosts = next;
  });
}
export const unshownResponseFailures = () => failures.filter(failure =>
  ![...failureHosts.values()].some(keys => keys.includes(failure.key)));

/** A load can reconcile only replies that were saved before it began. */
export const responseCheckpoint = () => Math.max(0, ...jobs.filter(job => job.saved).map(job => job.sequence));
/** Called only after a successful, non-superseded payload reaches the view. */
export function reconcileResponses(checkpoint: number) {
  jobs = jobs.filter(job => !job.saved || job.sequence > checkpoint);
}
export function dismissResponseFailure(key: string) {
  failures = failures.filter(failure => failure.key !== key);
}
function clearCoveredFailures(target: Target) {
  failures = failures.filter(f => f.id !== target.id || (target.scope !== 'all'
    && (f.scope === 'all' || f.occurrenceStartMs !== target.occurrenceStartMs)));
}

export function pendingResponse(id: number, startMs: number): Response | undefined {
  for (let i = jobs.length - 1; i >= 0; i--) {
    const job = jobs[i];
    if (job.id === id && (job.scope === 'all' || job.occurrenceStartMs === startMs)) return job.response;
  }
}

/** Wait for the current batch, including replies added while a write waits. */
export async function responsesIdle() {
  let batch;
  do { batch = tail; await batch; } while (batch !== tail);
}

export function queueResponse(request: Request, title = 'Event'): Promise<EventDetail> {
  // A double click or the same action from two surfaces must not mail the
  // guests twice. Different answers are kept in the order the user chose.
  const previous = [...jobs].reverse().find(job => job.id === request.id
    && (job.scope === 'all' || request.scope === 'all'
      || job.occurrenceStartMs === request.occurrenceStartMs));
  if (previous?.scope === request.scope && previous.response === request.response) return previous.promise;
  clearCoveredFailures(request);

  const promise = tail.then(() => invoke<EventDetail>('respond_to_event', request));
  const job = { ...request, promise, sequence: ++sequence, saved: false };
  jobs = [...jobs, job];
  // Resolve the queue tail on either outcome. A failed reply restores its
  // own UI, stays visible in the header, and never strands later replies.
  tail = promise.then(
    () => {
      jobs = jobs.map(item => item.promise === promise ? {...item, saved: true} : item);
      clearCoveredFailures(request);
    },
    error => {
      jobs = jobs.filter(item => item.promise !== promise);
      failures = [...failures.filter(f => f.key !== targetKey(request)), {
        id: request.id, scope: request.scope, occurrenceStartMs: request.occurrenceStartMs,
        key: targetKey(request), message: `${title}: could not save your response. ${String(error)}`,
      }];
    },
  );
  return promise;
}
