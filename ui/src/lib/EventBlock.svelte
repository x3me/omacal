<script lang="ts">
  import { clockFormat } from './clock.svelte';
  import { RESIZE_EDGE_PX } from './drag';
  import { formatClock } from './timefmt';
  import type { UiEvent, Placed } from './api';
  import type { Rect } from './position';
  import { locationLabel } from './location';

  let {
    event,
    placed,
    onopen,
    ongrab,
    preview = null,
    liveSpan = null,
    keyboardSelected = false,
  }: {
    event: UiEvent;
    placed: Placed;
    onopen: (event: UiEvent, rect: Rect) => void;
    /** The pointer went down on this block. The grid decides whether that
     *  becomes a drag, and whether it is a move or a resize — see `drag.ts`'s
     *  threshold and `edgeAt`. This reports the press rather than interpreting
     *  it. */
    ongrab?: (event: UiEvent, e: PointerEvent) => void;
    /**
     * Where this block is being dragged to, or `null` when it is not the one
     * under the pointer.
     *
     * **Deltas on `placed`**, not absolutes: a block's position comes from the
     * backend's own layout and is not recoverable from its instants, so the
     * grid says how far it has moved rather than where it now is. Percentages
     * of the column for the two vertical numbers, exactly the units `placed`
     * is in; pixels for `dx`, because a horizontal move is whole columns and a
     * block is only a fraction of one's width.
     */
    preview?: { topDeltaPct: number; heightDeltaPct: number; dx: number } | null;
    /** The span the in-flight drag would write, so the card reads where the
     *  block *is*, not where it was picked up (2026-08-21, by request).
     *  `null` outside a drag. Presentational, like `preview` — but sourced
     *  from the drag's `landed`, the very value a drop writes, so the label
     *  and the write can never disagree. */
    liveSpan?: { startMs: number; endMs: number } | null;
    /** Named by App's vim-style cursor. This is visual selection, not DOM
     * focus; opening it moves focus into the detail dialog. */
    keyboardSelected?: boolean;
  } = $props();

  // What the card *says*: the drag's tentative span while one is in flight,
  // the event's own instants otherwise. The density ladder below follows the
  // same pair on purpose — a block resized past 90 minutes grows its time
  // line mid-drag, reading the very span the drop would write.
  const shownStartMs = $derived(liveSpan?.startMs ?? event.start_ms);
  const shownEndMs = $derived(liveSpan?.endMs ?? event.end_ms);

  const minutes = $derived((shownEndMs - shownStartMs) / 60_000);

  // Density ladder (spec §7.1). Thresholds are in minutes.
  const showMeta = $derived(minutes >= 45);
  const showTime = $derived(minutes >= 90);

  const hhmm = (ms: number) => formatClock(ms, clockFormat());

  // Location is the thing you act on when you are walking somewhere. A guest
  // count belongs here too, but attendees are not stored yet, and a hardcoded
  // zero read as "no guests" — a claim we cannot make.
  const meta = $derived(locationLabel(event.location));

  const width = $derived(100 / placed.columns);
  const left = $derived(placed.column * width);

  // `getBoundingClientRect()` here, not the block's own layout numbers
  // (`placed`, percentages of a scrolling column): the popover positions
  // itself against the viewport, and this is the one place that rect is
  // available without the parent re-deriving it from geometry it doesn't own.
  function open(e: MouseEvent) {
    hideTip();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    onopen(event, { top: r.top, left: r.left, width: r.width, height: r.height });
  }

  /** The hover tooltip's anchor, or `null` while nothing hovers.
   *
   *  Ours, not the webview's: `title=` renders as the engine's native
   *  tooltip, which no stylesheet can reach — engine-grey on engine-black,
   *  invisible on a dark theme — and it named the event without answering
   *  *when*, the thing a block too short for its time line cannot say.
   *
   *  `position: fixed` with coordinates taken at mouseenter, so the block's
   *  `overflow: hidden` cannot clip it. That escape hatch closes when an
   *  ancestor has a transform — which a drag preview does — so the render
   *  below also gates on `!preview`. */
  let tip = $state<{ x: number; y: number; above: boolean } | null>(null);

  /** The block's rendered height, for the grips below — the one fact the
   *  short-block rule needs and CSS alone cannot ask. */
  let heightPx = $state(0);

  /** Whether the ends grow resize grips. **`edgeAt`'s own rule, on the same
   *  constant**: below three bands of height there is no middle left between
   *  them and the press-time hit test answers `null` — so the cursor must not
   *  promise what the press will not do. The grips are the *affordance* for a
   *  resize that has worked since `856406b` and that nobody could find,
   *  because the only cursor the block ever showed was `grab` (reported
   *  2026-08-26: "I want to extend this to grabbing an edge" — it existed,
   *  invisibly). */
  const grips = $derived(heightPx >= RESIZE_EDGE_PX * 3);

  function showTip(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    // Above the block unless that would leave the viewport; clamped right so
    // a Sunday event's tooltip does not run off the screen edge.
    const above = r.top > 64;
    tip = {
      x: Math.max(6, Math.min(r.left, window.innerWidth - 286)),
      y: above ? r.top - 6 : r.bottom + 6,
      above,
    };
  }
  const hideTip = () => (tip = null);
</script>

<!-- The calendar-time geometry remains percentage based, inset by one pixel at
     each vertical edge so consecutive cards expose a two-pixel strip of the
     grid between them. Drag deltas stay in those same percentage units, and
     `transform` is absent rather than `none` while idle. -->
<button
  class="ev {event.response}"
  class:nobodycoming={event.all_guests_declined}
  class:dragging={preview !== null}
  class:keyboard={keyboardSelected}
  data-kbd-selected-event={keyboardSelected ? '' : undefined}
  data-event-id={event.id}
  data-event-start-ms={event.start_ms}
  style="
    top:{preview
      ? `calc(${placed.top * 100}% + ${preview.topDeltaPct}% + 1px)`
      : `calc(${placed.top * 100}% + 1px)`};
    height:{preview
      ? `calc(${placed.height * 100}% + ${preview.heightDeltaPct}% - 2px)`
      : `calc(${placed.height * 100}% - 2px)`};
    {preview && preview.dx !== 0 ? `transform: translateX(${preview.dx}px);` : ''}
    left:calc({left}% + 3px); width:calc({width}% - 6px);
    --cal:{event.color}; z-index:{placed.column + 1};
  "
  aria-label="{event.title}, {hhmm(shownStartMs)} to {hhmm(shownEndMs)}{meta ? `, ${meta}` : ''}{event.all_guests_declined ? ', everyone declined' : ''}"
  onclick={open}
  onmouseenter={showTip}
  onmouseleave={hideTip}
  onpointerdown={(e) => { hideTip(); ongrab?.(event, e); }}
  bind:clientHeight={heightPx}
>
  <!-- The resize cursor's home, and nothing else's: no background, no
       handler, no size of their own beyond the band constant — the press
       still lands on the button and `edgeAt` still decides. Hidden while
       this block is the one being dragged, when the only honest cursor is
       the grid's own `grabbing`. -->
  {#if grips && !preview}
    <span class="grip" style="top:0; height:{RESIZE_EDGE_PX}px" aria-hidden="true"></span>
    <span class="grip" style="bottom:0; height:{RESIZE_EDGE_PX}px" aria-hidden="true"></span>
  {/if}
  <!-- `?` means MAYBE — the answer niki gave, in the letter Google and
       Outlook both use for it (2026-08-10, by request). An unanswered invite
       carries no letter: its dashed ring is the whole of "nothing yet". -->
  {#if event.response === 'tentative'}<i class="rs">?</i>{/if}
  <b>{event.title}</b>
  {#if showTime}<em>{hhmm(shownStartMs)} – {hhmm(shownEndMs)}</em>{/if}
  <!-- Words, not a mark. A strike through the title and a ✕ in the corner
       were both tried and both looked like damage to the block rather than
       news about the meeting (2026-09-04). This line says it where the
       location says its own thing, in the same muted voice, and the fill
       below carries the rest at heights too short for any line at all. -->
  {#if showMeta && event.all_guests_declined}<em class="none">Everyone declined</em>{/if}
  {#if showMeta && meta}<em>{meta}</em>{/if}
  <!-- aria-hidden: the button's own label already says all of this, so the
       tooltip is presentation for the pointer, not a second announcement. -->
  {#if tip && !preview}
    <span class="tip" class:below={!tip.above} aria-hidden="true"
          style="left:{tip.x}px; top:{tip.y}px;">
      <b class="tt">{event.title}</b>
      <span class="tw">{hhmm(shownStartMs)} – {hhmm(shownEndMs)}{meta ? ` · ${meta}` : ''}</span>
    </span>
  {/if}
</button>

<style>
  /* Lifted while dragging so it reads as picked up, and above every other
     block so it is never hidden behind one it is passing over. No transition:
     the block is following a pointer and easing would put it behind the
     finger. */
  .ev.dragging { z-index: 50 !important; opacity: 0.85; cursor: grabbing; }
  .ev.keyboard { outline: 2px solid var(--accent); outline-offset: 1px;
                 z-index: 25 !important; }
  /* The grab bands, as cursors only — the hit test itself is `drag.ts`'s
     `edgeAt`, on the press's own offset. The `.grip` spans in the template
     exist purely to carry `ns-resize` over those bands: sized by the same
     constant and gated by the same short-block rule, so what the cursor
     promises and what the press does cannot disagree — and transparent, so
     the committed baselines hold them to invisibility. */
  .ev:hover { cursor: grab; }
  .grip { position: absolute; left: 0; right: 0; cursor: ns-resize; }

  .ev {
    /* A <button> keeps native chrome in macOS WKWebView unless appearance is
       cleared. Not the cause of the corner artifact below — clearing it alone
       did not fix that — but correct regardless for a fully custom control. */
    appearance: none; -webkit-appearance: none;
    position: absolute; text-align: left; cursor: pointer;
    border-radius: var(--event-card-radius, 6px); padding: 2px 8px; overflow: hidden;
    /* NO border. The colour spine is an inset shadow instead.
       A border on one side only makes WebKit derive each corner's curve from
       the two border widths meeting there, and in macOS WKWebView the corners
       away from that border rendered square. An inset shadow follows
       border-radius exactly and cannot influence corner geometry, so the cause
       is removed rather than worked around. */
    border: 0;
    /* States recolour --spine rather than redeclaring box-shadow, so the hover
       lift below is never lost by a later, more specific rule. The two outer
       one-pixel shadows paint the exposed edge strips with the page background:
       otherwise a whole-hour boundary also exposes `.rule`, making hourly
       gaps look heavier than half-hour gaps. */
    --spine: var(--cal);
    box-shadow: inset 2px 0 0 0 var(--spine),
                0 -1px 0 0 var(--bg), 0 1px 0 0 var(--bg);
    background-clip: padding-box;
    /* Composited over --bg, not `transparent`. Blocks overlap constantly, and a
       translucent fill lets the one behind read through — its title, and its
       rounded corners poking past this one's. Against the column background the
       result is indistinguishable from a 7% wash, because the column background
       IS --bg. */
    --event-fill: color-mix(in srgb, var(--cal) 7%, var(--bg));
    background: color-mix(in srgb, var(--cal) 7%, var(--bg));
    color: color-mix(in srgb, var(--cal) 65%, var(--text));
    font: inherit;
  }
  /* Hover lifts the block to full width so a squeezed 3-way pile stays
     readable without changing the layout rules (spec §7.1).

     `z-index` needs `!important` exactly as `left`/`width` beside it do: the
     element carries an inline `z-index: column+1`, which quietly outranks a
     plain rule — leaving the hovered block (and the tooltip trapped in its
     stacking context, `position: fixed` notwithstanding) UNDER its
     higher-column neighbours. `.ev.dragging` below got this right from day
     one; this rule had been losing the same fight invisibly until a dense
     iCloud week made it obvious. */
  .ev:hover { left: 3px !important; width: calc(100% - 6px) !important; z-index: 20 !important;
              box-shadow: inset 2px 0 0 0 var(--spine),
                          0 -1px 0 0 var(--bg), 0 1px 0 0 var(--bg),
                          0 4px 14px rgba(0, 0, 0, .5); }

  /* 11.5/10.5, up from 10/9 (2026-08-14): at 10px the grid read as decoration
     on a 14" screen, and Google's own week view sits at ~12px. The density
     ladder holds with room to spare — a 45-minute block is 52px at the
     grid's 70px/hour and two lines at these sizes need 33. */
  .ev b { display: block; font-size: 11.5px; font-weight: 600; line-height: 1.3;
          letter-spacing: -.01em; white-space: nowrap; overflow: hidden;
          text-overflow: ellipsis; }
  .ev em { font-style: normal; display: block; font-size: 10.5px; opacity: .62;
           line-height: 1.35; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  .rs { position: absolute; top: 1px; right: 4px; font-size: 9px;
        font-style: normal; font-weight: 700; opacity: .8; }

  /* Everyone else said no (2026-09-04). Half the usual fill, and nothing
     else: the block recedes a step without changing shape, which is as much
     as a state deserves that is still your event at its own time in its own
     colour. Two louder cuts were tried first and both were rejected on
     sight — a ✕ in the corner ("a bit ugly") and a strike through the title
     ("still not very good looking") — and both had the same fault: they
     drew damage on the block instead of telling the user something. The
     `.none` line above does the telling wherever there is room for a line,
     and the popover's tally does it in full.

     Ordered before `.declined` so that one still wins when both apply: a
     meeting you declined *and* nobody else came to is, to you, declined. */
  .ev.nobodycoming { background-color: color-mix(in srgb, var(--cal) 8%, var(--bg)); }
  .ev em.none { opacity: .75; }

  /* State is carried by the fill, so it survives at 15 minutes tall. Every
     state stays opaque: "unfilled" means the colour of the grid, not a hole
     through to whatever block is underneath. */
  /* Unanswered: hollow, with a dashed outline. The dashes are drawn from the
     calendar colour rather than `currentColor` — currentColor here is the text
     colour, the brightest thing on the block, which made the ring shout louder
     than the event it belongs to. Uniform on all four sides so corner curves
     stay symmetric. */
  .ev.needsAction {
    --event-fill: var(--bg);
    background-color: var(--bg);
    border: 1px dashed color-mix(in srgb, var(--cal) 55%, var(--bg));
    /* The dashed ring already carries the state; a full-strength spine beside
       it reads as two competing left edges. */
    --spine: color-mix(in srgb, var(--cal) 70%, var(--bg));
  }
  .ev.tentative .rs { opacity: .55; font-weight: 600; }
  .ev.tentative { background-image: repeating-linear-gradient(135deg,
                  rgba(128,128,128,.16) 0 3px, transparent 3px 7px); }
  /* Faded via its own colours rather than element opacity: `opacity` would make
     the block see-through no matter what its background is. */
  .ev.declined { --event-fill: var(--bg); background-color: var(--bg);
                 color: color-mix(in srgb, var(--cal) 22%, var(--muted));
                 --spine: color-mix(in srgb, var(--cal) 45%, var(--bg)); }
  .ev.declined b { text-decoration: line-through; }


  /* Deepens the fill on hover so an expanded block reads as lifted above the
     ones it covers. Every state is already opaque at rest, so this is emphasis
     rather than the occlusion fix itself. Last among the state rules so it
     wins over the per-state backgrounds above at equal specificity. */
  .ev:hover { --event-fill: color-mix(in srgb, var(--cal) 16%, var(--bg));
              background-color: color-mix(in srgb, var(--cal) 16%, var(--bg)); }

  /* The preference fades the fill colour, never the element: opacity here
     would also fade the title, RSVP mark, outline and colour spine. Hover
     still deepens the same fill before this final alpha is applied. */
  :global(:root[data-event-transparency]) .ev {
    background-color: color-mix(
      in srgb,
      var(--event-fill) var(--event-fill-opacity),
      transparent
    );
  }

  /* The tooltip. Fixed, so the block's own overflow:hidden cannot clip it;
     pointer-events none, so it can never steal the hover that shows it. The
     delay is in the animation, not a timer: the element mounts at once and
     stays invisible for 300ms, so a cursor passing through a column doesn't
     strobe. The spine repeats the calendar colour so the card visibly belongs
     to the block under it. */
  .tip { position: fixed; z-index: 100; pointer-events: none; max-width: 280px;
         transform: translateY(-100%);
         background: color-mix(in srgb, var(--text) 9%, var(--bg));
         color: var(--text);
         border: 1px solid color-mix(in srgb, var(--text) 16%, transparent);
         border-radius: 6px; padding: 6px 10px 6px 12px;
         box-shadow: inset 2px 0 0 0 var(--cal), 0 6px 20px rgba(0, 0, 0, .45);
         opacity: 0; animation: tip-in .12s ease .3s forwards; }
  .tip.below { transform: none; }
  @keyframes tip-in { to { opacity: 1; } }
  /* After `.ev b`/`.ev em` on purpose: the tooltip's lines must not inherit
     the block's ellipsis — a card exists precisely to say the whole thing. */
  .tip .tt { display: block; font-size: 12px; font-weight: 600; line-height: 1.35;
             white-space: normal; overflow: visible; }
  .tip .tw { display: block; font-size: 11px; color: var(--muted); margin-top: 1px;
             white-space: normal; }
</style>
