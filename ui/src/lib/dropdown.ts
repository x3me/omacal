// A dropdown that no scrolling ancestor can clip: drawn `position: fixed`
// and placed against its field by `placeDropdown`, then kept there while
// anything scrolls or the window resizes — a wheel over a dropdown's scrim
// still scrolls the pane it sits in, and the field moves with it.
//
// An action rather than a component, because `DateField`'s calendar and
// `TimeField`'s list are different popups with the same one problem.

import { placeDropdown } from './position';

export function dropdown(popup: HTMLElement, field: HTMLElement | undefined) {
  let anchor = field;
  const place = () => {
    if (!anchor) return;
    const f = anchor.getBoundingClientRect();
    const p = popup.getBoundingClientRect();
    const at = placeDropdown(
      { top: f.top, left: f.left, width: f.width, height: f.height },
      { width: p.width, height: p.height },
      { width: window.innerWidth, height: window.innerHeight },
    );
    // Settings is centred with a transform, which makes it the containing
    // block even for a fixed popup. Convert viewport coordinates back to
    // that block or the list opens a whole dialog-width away from its field.
    const parent = popup.offsetParent;
    const origin = parent?.getBoundingClientRect();
    popup.style.top = `${at.top - (origin ? origin.top + parent!.clientTop : 0)}px`;
    popup.style.left = `${at.left - (origin ? origin.left + parent!.clientLeft : 0)}px`;
    popup.style.visibility = 'visible';
  };
  // Hidden for the one frame before it is measured, so it never flashes at
  // the window's corner.
  popup.style.visibility = 'hidden';
  place();
  const onScroll = (e: Event) => {
    // The list scrolling its own rows moves nothing.
    if (e.target instanceof Node && popup.contains(e.target)) return;
    place();
  };
  window.addEventListener('scroll', onScroll, true);
  window.addEventListener('resize', place);
  return {
    update(next: HTMLElement | undefined) {
      anchor = next;
      place();
    },
    destroy() {
      window.removeEventListener('scroll', onScroll, true);
      window.removeEventListener('resize', place);
    },
  };
}
