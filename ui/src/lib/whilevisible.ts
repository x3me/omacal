// Timers that run only while their window can be seen.
//
// The menubar popup is a window that is only ever hidden, never closed
// (`menubar.rs`), so anything it ran on an interval ran for the whole life of
// the app once the tray had been clicked once: a 15-second feed poll that
// asks the backend for an agenda, and a 1-second clock. Nobody looks at a
// hidden window, so this runs them only while the document is visible, and
// refreshes once when it is shown again.

type Doc = {
  hidden: boolean;
  addEventListener(type: 'visibilitychange', f: () => void): void;
  removeEventListener(type: 'visibilitychange', f: () => void): void;
};

type Timers = {
  setInterval(run: () => void, ms: number): unknown;
  clearInterval(id: unknown): void;
};

const realTimers: Timers = {
  setInterval: (run, ms) => setInterval(run, ms),
  clearInterval: (id) => clearInterval(id as ReturnType<typeof setInterval>),
};

/**
 * Runs each of `every` on its interval while `doc` is visible, stops them all
 * when it is hidden, and calls `onShown` each time it becomes visible again
 * (not at the start: the caller loads on mount already). Returns the cleanup.
 * The document and the timers are parameters so a test can drive both.
 */
export function whileVisible(
  doc: Doc,
  every: { ms: number; run: () => void }[],
  onShown?: () => void,
  timers: Timers = realTimers,
): () => void {
  let ids: unknown[] = [];
  const start = () => { if (ids.length === 0) ids = every.map((e) => timers.setInterval(e.run, e.ms)); };
  const stop = () => { for (const id of ids) timers.clearInterval(id); ids = []; };
  const changed = () => {
    if (doc.hidden) { stop(); return; }
    onShown?.();
    start();
  };
  if (!doc.hidden) start();
  doc.addEventListener('visibilitychange', changed);
  return () => { stop(); doc.removeEventListener('visibilitychange', changed); };
}
