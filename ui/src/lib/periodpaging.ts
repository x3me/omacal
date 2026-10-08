// Month/Year period paging — the DOM side. `periodscroll.ts` owns *when* a
// gesture earns a page; this owns the listeners and the one rule that needs the
// DOM: the edge-chain check, which reads the period's own scrollport. App owns
// the guard and the step, and hands them in as callbacks so this stays a small,
// self-contained piece of plumbing.

import {
  PAGE_IDLE_MS, newAccumulator, resetAccumulator, swipePage, wheelPage,
  type PageAccumulator,
} from './periodscroll';
import type { PanSample } from './weekwindow';

export type PagingCallbacks = {
  /** Paging is live for the view on screen (Month grid or Year grid). */
  active: () => boolean;
  /** An overlay owns the screen, so no gesture pages. */
  blocked: () => boolean;
  /** Move one period: `1` forward, `-1` back — the caller's `step`. */
  onPage: (dir: 1 | -1) => void;
};

/** Attach wheel and touch paging to `host` (App's `.view`). Returns the
 *  cleanup, ready for a Svelte action's `destroy`. */
export function attachPeriodPaging(host: HTMLElement, cb: PagingCallbacks): () => void {
  let acc: PageAccumulator = newAccumulator();
  let idle: ReturnType<typeof setTimeout> | undefined;
  // A finger on the glass, for a vertical swipe. Only touch gestures are ours;
  // a mouse drag inside the grid is a create or a right-click, not a page.
  let swipe: { id: number; lastY: number; travel: number; samples: PanSample[] } | null = null;

  const scrollport = () => host.querySelector<HTMLElement>('.grid, .ygrid');

  /** Whether the period is clipped and can still scroll the way the gesture
   *  asks. While it can, it scrolls; only at the edge does a wheel page — the
   *  edge-chain rule on the map (`docs/superpowers/specs/2026-08-28-...:86`). */
  function canScroll(sc: HTMLElement, down: boolean): boolean {
    const max = sc.scrollHeight - sc.clientHeight;
    if (max <= 1) return false;
    return down ? sc.scrollTop < max - 1 : sc.scrollTop > 1;
  }

  function onWheel(e: WheelEvent) {
    if (!cb.active() || cb.blocked()) return;
    // Ctrl+wheel is the app's zoom gesture — Week zooms the hours with it, and
    // `App` cancels the browser's own page zoom — so it is never a page here.
    if (e.ctrlKey) return;
    // Vertical only (ticket 01): a horizontal-dominant gesture is not ours,
    // and neither is a wheel with no vertical travel.
    if (e.deltaY === 0 || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
    const sc = scrollport();
    if (sc && canScroll(sc, e.deltaY > 0)) return; // let the period scroll first
    e.preventDefault();
    const r = wheelPage(acc, e.deltaY, performance.now());
    acc = r.acc;
    if (r.page !== 0) cb.onPage(r.page);
    // The lock ends the tail; this ends the gesture, so a slow drag cannot
    // bank travel across separate ones.
    clearTimeout(idle);
    idle = setTimeout(() => { acc = resetAccumulator(acc); }, PAGE_IDLE_MS);
  }

  // A page suppresses the click the browser synthesises after a touch release,
  // so a swipe across a day cell never also picks that day.
  function swallowClick(e: MouseEvent) {
    e.stopPropagation();
    e.preventDefault();
  }
  function suppressNextClick() {
    host.addEventListener('click', swallowClick, { capture: true, once: true });
    // If no click follows (a genuine drag), drop the armed listener rather than
    // leave it to eat the *next* legitimate click.
    setTimeout(() => host.removeEventListener('click', swallowClick, { capture: true }), 400);
  }

  function onDown(e: PointerEvent) {
    if (e.pointerType !== 'touch' || !cb.active() || cb.blocked()) { swipe = null; return; }
    // A clipped period is scrollable under the finger; paging waits for a
    // period that fits — the touch reading of edge-chain.
    const sc = scrollport();
    if (sc && sc.scrollHeight - sc.clientHeight > 1) { swipe = null; return; }
    swipe = { id: e.pointerId, lastY: e.clientY, travel: 0, samples: [] };
  }
  function onMove(e: PointerEvent) {
    if (!swipe || e.pointerId !== swipe.id) return;
    const dy = e.clientY - swipe.lastY;
    swipe.lastY = e.clientY;
    swipe.travel += dy;
    // Increments, for the velocity estimate (`velocityOf` sums them). `days`
    // carries pixels here; the helper is generic over a signed quantity.
    swipe.samples.push({ t: performance.now(), days: dy });
    if (swipe.samples.length > 12) swipe.samples.shift();
  }
  function onUp(e: PointerEvent) {
    if (!swipe || e.pointerId !== swipe.id) return;
    const { travel, samples } = swipe;
    swipe = null;
    const page = swipePage(travel, samples);
    if (page !== 0) { suppressNextClick(); cb.onPage(page); }
  }
  function onCancel(e: PointerEvent) { if (swipe && e.pointerId === swipe.id) swipe = null; }

  host.addEventListener('wheel', onWheel, { passive: false });
  host.addEventListener('pointerdown', onDown);
  window.addEventListener('pointermove', onMove);
  window.addEventListener('pointerup', onUp);
  window.addEventListener('pointercancel', onCancel);
  return () => {
    host.removeEventListener('wheel', onWheel);
    host.removeEventListener('pointerdown', onDown);
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('pointerup', onUp);
    window.removeEventListener('pointercancel', onCancel);
    host.removeEventListener('click', swallowClick, { capture: true });
    clearTimeout(idle);
  };
}
