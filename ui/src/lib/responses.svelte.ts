import { untrack } from 'svelte';
import { invoke } from '@tauri-apps/api/core';
import type { EventDetail } from './eventdetail';

type Response = 'accepted' | 'tentative' | 'declined';
type Request = { id: number; response: Response; scope: 'this' | 'all'; occurrenceStartMs: number };
type Job = Request & { promise: Promise<EventDetail>; sequence: number; saved: boolean };
type Failure = { id: number; message: string };

// Replies made in this window share a queue, independent of any popover.
// Saved replies keep their display override until a post-sync reload lands.
let jobs = $state.raw<Job[]>([]);
let sequence = 0;
let failures = $state<Failure[]>([]);
let tail: Promise<unknown> = Promise.resolve();

export const pendingResponseCount = () => jobs.filter(job => !job.saved).length;
export const responsePending = (id: number, startMs?: number) => jobs.some(job => !job.saved && job.id === id
  && (startMs === undefined || job.scope === 'all' || job.occurrenceStartMs === startMs));
export const responseFailures = () => failures;

// A visible row/popover owns its error; the header carries it when that
// surface closes. Both dismiss the same record, so an old copy cannot return.
let failureHosts = $state.raw(new Map<symbol, number[]>());
export function showResponseFailuresHere(ids: number[]) {
  const key = Symbol();
  untrack(() => { failureHosts = new Map(failureHosts).set(key, ids); });
  return () => untrack(() => {
    const next = new Map(failureHosts); next.delete(key); failureHosts = next;
  });
}
export const unshownResponseFailures = () => failures.filter(failure =>
  ![...failureHosts.values()].some(ids => ids.includes(failure.id)));

/** Capture only replies saved before this sync; later replies need their own. */
export const responseCheckpoint = () => Math.max(0, ...jobs.filter(job => job.saved).map(job => job.sequence));
/** Called only after a successful post-sync payload has reached the view. */
export function reconcileResponses(checkpoint: number) {
  jobs = jobs.filter(job => !job.saved || job.sequence > checkpoint);
}
export function dismissResponseFailure(id: number) {
  failures = failures.filter(failure => failure.id !== id);
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
  dismissResponseFailure(request.id);

  const promise = tail.then(() => invoke<EventDetail>('respond_to_event', request));
  const job = { ...request, promise, sequence: ++sequence, saved: false };
  jobs = [...jobs, job];
  // Resolve the queue tail on either outcome. A failed reply restores its
  // own UI, stays visible in the header, and never strands later replies.
  tail = promise.then(
    () => {
      jobs = jobs.map(item => item.promise === promise ? {...item, saved: true} : item);
      dismissResponseFailure(request.id);
    },
    error => {
      jobs = jobs.filter(item => item.promise !== promise);
      failures = [...failures.filter(f => f.id !== request.id), {
        id: request.id, message: `${title}: could not save your response. ${String(error)}`,
      }];
    },
  );
  return promise;
}
