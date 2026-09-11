<script lang="ts">
  import CalendarColors from './CalendarColors.svelte';
  import type { ColorSegment } from './combined';
  import { clockFormat } from './clock.svelte';
  import { RESIZE_EDGE_PX, beganDrag } from './drag';
  import { formatClock } from './timefmt';
  import type { EventCopy, UiEvent, Placed } from './api';
  import type { Calendar } from './calendars';
  import type { Rect } from './position';
  import { locationLabel } from './location';

  let {
    event,
    placed,
    onopen,
    onedit,
    ongrab,
    preview = null,
    liveSpan = null,
    keyboardSelected = false,
    createMode = false,
    hovered = null,
    overlapColors = [],
    copyHoverLane,
    obscured = false,
    calendars = [],
    onopencopy = null,
    onfocuschange = null,
  }: {
    event: UiEvent;
    placed: Placed;
    onopen: (event: UiEvent, rect: Rect) => void;
    /** Right-click: straight into the editor, skipping the details card
     *  (#109). Optional, so a surface that has no editor simply omits it and
     *  the block keeps the browser menu suppressed and nothing else. */
    onedit?: (event: UiEvent, rect: Rect) => void;
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
    /** Alt-hover or an active creation sweep: this block is background. */
    createMode?: boolean;
    /** Grid hover uses the packed columns even while a block is expanded.
     * Standalone blocks fall back to their own pointer boundary. */
    hovered?: boolean | null;
    overlapColors?: ColorSegment[];
    /** The bottom bars keep their packed positions so moving sideways can
     * select covered peers. Copy hit targets follow that map; the selected
     * copy's color paints the whole expanded card. */
    copyHoverLane?: { left: number; width: number };
    obscured?: boolean;
    calendars?: Calendar[];
    onopencopy?: ((copy: EventCopy, rect: Rect, thenEdit?: boolean) => void) | null;
    /** The grid owns expansion geometry for keyboard focus as well as hover. */
    onfocuschange?: ((focused: boolean) => void) | null;
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

  const calendarName = $derived(calendars.find(c => c.id === event.calendar_id)?.summary);

  function restingRect(): Rect {
    const r = button!.getBoundingClientRect();
    // Hover paints across the day, but the popup belongs beside this event's
    // packed lane. Include the same 3px edge insets as the card geometry.
    const laneWidth = (r.width + 6) / placed.columns;
    return { top: r.top, height: r.height,
      left: expanded ? r.left + placed.column * laneWidth : r.left,
      width: expanded ? Math.max(0, laneWidth - 6) : r.width };
  }
  function open() {
    hideTip();
    onopen(event, restingRect());
  }

  /** Where a right press started, or `null` when none is in flight.
   *
   *  The same threshold discipline the grid's own right-click already uses
   *  for "new event here": a create cannot ride `contextmenu`, which fires at
   *  the *press*, before the gesture has a shape — and neither can an edit.
   *  The press is remembered, and the release decides. */
  let rightPress: { x: number; y: number } | null = null;

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
  let tip = $state<{ x: number; y: number; above: boolean; calendarId?: number; color: string } | null>(null);
  let button = $state<HTMLButtonElement | null>(null);
  let pointerInside = $state(false);
  let copyFocus = $state(false);
  const activeHover = $derived(!createMode && (hovered ?? pointerInside));
  const showCopies = $derived(!createMode && !preview && !!onopencopy
    && (event.copies?.length ?? 0) > 1 && (activeHover || (hovered === null && copyFocus)));
  const expanded = $derived(activeHover || showCopies);
  $effect(() => {
    if (!expanded) hideTip();
    else if (activeHover && !showCopies && button) showTip(restingRect());
  });

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

  function showTip(r: Rect, source: { calendar_id?: number; color: string } = event) {
    // Above the block unless that would leave the viewport; clamped right so
    // a Sunday event's tooltip does not run off the screen edge.
    const above = r.top > 90;
    tip = {
      x: Math.max(6, Math.min(r.left, window.innerWidth - 286)),
      y: above ? r.top - 6 : r.top + r.height + 6,
      above, calendarId: source.calendar_id, color: source.color,
    };
  }
  const hideTip = () => (tip = null);
  function showCopyTip(copy: EventCopy, e: Event) {
    showTip((e.currentTarget as HTMLElement).getBoundingClientRect(), copy);
  }
  function chooseCopy(copy: EventCopy, e: MouseEvent, thenEdit = false) {
    hideTip();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    onopencopy?.(copy, { top: r.top, left: r.left, width: r.width, height: r.height }, thenEdit);
  }
</script>

<!-- The calendar-time geometry remains percentage based, inset by one pixel at
     each vertical edge so consecutive cards expose a two-pixel strip of the
     grid between them. Drag deltas stay in those same percentage units, and
     `transform` is absent rather than `none` while idle. -->
<div class="event-host"
  onfocusin={() => { copyFocus = true; onfocuschange?.(true); }}
  onfocusout={(e) => {
    if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
      copyFocus = false;
      onfocuschange?.(false);
    }
  }}>
<button
  class="ev {event.response}"
  data-combined-count={event.copies?.length || undefined}
  style:color={event.copies?.length ? "var(--text)" : undefined}
  class:nobodycoming={event.all_guests_declined}
  class:dragging={preview !== null}
  class:keyboard={keyboardSelected}
  class:create-mode={createMode}
  class:hovered={expanded}
  class:copies-open={showCopies}
  class:with-colors={(!showCopies && (event.copies?.length ?? 0) > 1) || overlapColors.length > 1}
  class:obscured
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
  aria-label="{event.title}, {hhmm(shownStartMs)} to {hhmm(shownEndMs)}{meta ? `, ${meta}` : ''}{event.all_guests_declined ? ', everyone declined' : ''}{event.copies?.length ? `, ${event.copies.length} calendar copies` : calendarName ? `, ${calendarName}` : ''}"
  onclick={open}
  onpointerup={(e) => {
    if (e.button !== 2 || !rightPress) return;
    const still = !beganDrag(e.clientX - rightPress.x, e.clientY - rightPress.y);
    rightPress = null;
    if (still && onedit) { hideTip(); onedit(event, restingRect()); }
  }}
  oncontextmenu={(e) => e.preventDefault()}
  onmouseenter={() => { pointerInside = true; }}
  onmouseleave={() => { pointerInside = false; }}
  onpointerdown={(e) => {
    hideTip();
    // A right press must never become a drag — `ongrab` is the left button's,
    // and `startDrag` would read a right press as a move or a resize. This
    // only remembers where it started; the release below decides.
    if (e.button === 2) { rightPress = { x: e.clientX, y: e.clientY }; return; }
    rightPress = null;
    ongrab?.(event, e);
  }}
  bind:clientHeight={heightPx}
  bind:this={button}
>
  <!-- The resize cursor's home, and nothing else's: no background, no
       handler, no size of their own beyond the band constant — the press
       still lands on the button and `edgeAt` still decides. Hidden while
       this block is the one being dragged, when the only honest cursor is
       the grid's own `grabbing`. -->
  {#if grips && !preview && !createMode && !event.copies?.length}
    <span class="grip" style="top:0; height:{RESIZE_EDGE_PX}px" aria-hidden="true"></span>
    <span class="grip bottom-grip" style="bottom:0; height:{RESIZE_EDGE_PX}px" aria-hidden="true"></span>
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
  {#if tip && !preview && !createMode}
    {@const tipCalendar = calendars.find(c => c.id === tip!.calendarId)}
    <span class="tip" class:below={!tip.above} aria-hidden="true"
          style="left:{tip.x}px; top:{tip.y}px; --cal:{tip.color};">
      <b class="tt">{event.title}</b>
      <span class="tw">{hhmm(shownStartMs)} – {hhmm(shownEndMs)}{meta ? ` · ${meta}` : ''}</span>
      {#if tipCalendar}<span class="calendar-line"><i class="calendar-dot"></i><span class="calendar-label">{tipCalendar.summary}</span></span>{/if}
    </span>
  {/if}
  <CalendarColors colors={showCopies ? [] : event.copies?.map(copy => copy.color)} segments={overlapColors} />
</button>
{#if showCopies}
  <!-- The selected calendar colors the whole card. Keep the copy targets
       separate: their fixed bottom-bar columns still select each version,
       without highlighting a different part of the card from the pointer. -->
  <div class="copy-colors" aria-hidden="true"
    style="top:calc({placed.top * 100}% + 1px); height:calc({placed.height * 100}% - 2px);">
    <span class="copy-color" style:--cal={tip?.color ?? event.copies?.[0]?.color ?? event.color}></span>
  </div>
  {#if copyHoverLane}
    <!-- A staggered card underneath must not receive a click while the shared
         view owns hover. Covered peers take ownership through the grid's
         packed-column hit test before a click can land here. -->
    <button class="copy-backdrop" tabindex="-1" aria-hidden="true" onclick={open}
      onpointerdown={(e) => { e.preventDefault(); hideTip(); }}
      style="top:calc({placed.top * 100}% + 1px); height:calc({placed.height * 100}% - 2px);"></button>
  {/if}
  <div class="copy-panels" role="group" aria-label="Calendar copies"
    style:left={copyHoverLane ? `calc(${copyHoverLane.left * 100}% + ${3 - 6 * copyHoverLane.left}px)` : undefined}
    style:width={copyHoverLane ? `calc(${copyHoverLane.width * 100}% - ${6 * copyHoverLane.width}px)` : undefined}
    style:right={copyHoverLane ? 'auto' : undefined}
    style="top:calc({placed.top * 100}% + 1px); height:calc({placed.height * 100}% - 2px);">
    {#each event.copies! as copy (copy.id)}
      {@const calendar = calendars.find(c => c.id === copy.calendar_id)}
      {@const label = `Open ${calendar?.summary ?? `Calendar ${copy.calendar_id}`} copy`}
      <button class="copy-panel" style:--cal={copy.color}
        aria-label={label}
        onmouseenter={(e) => showCopyTip(copy, e)} onmouseleave={hideTip}
        onfocus={(e) => showCopyTip(copy, e)}
        onpointerdown={(e) => {
          rightPress = e.button === 2 ? { x: e.clientX, y: e.clientY } : null;
        }}
        onpointerup={(e) => {
          if (e.button !== 2 || !rightPress) return;
          const still = !beganDrag(e.clientX - rightPress.x, e.clientY - rightPress.y);
          rightPress = null;
          if (still) chooseCopy(copy, e, true);
        }}
        oncontextmenu={(e) => e.preventDefault()}
        onclick={(e) => chooseCopy(copy, e)}></button>
    {/each}
  </div>
{/if}
</div>

<style>
  .event-host { display: contents; }
  /* The shared label floats above the colors and their fixed hover targets.
     It must never intercept a click intended for a calendar copy. */
  .ev.hovered.copies-open { pointer-events: none; background: none !important;
                    box-shadow: none !important; border-color: transparent;
                    z-index: 27 !important; }
  /* A quiet backing keeps the shared words on one surface as they cross
     calendar colors. Fit the text and preserve the panels' click targets. */
  .ev.copies-open > b, .ev.copies-open > em {
    width: fit-content; max-width: 100%; border-radius: 2px;
    background: var(--surface); box-shadow: 0 0 0 2px var(--surface);
    color: var(--text); opacity: 1;
  }
  .ev.copies-open > em { color: var(--muted); }
  .copy-colors, .copy-panels { position: absolute; left: 3px; right: 3px;
    display: flex; gap: 0; overflow: hidden;
    border-radius: var(--event-card-radius, 6px); }
  .copy-colors { z-index: 25; pointer-events: none; }
  .copy-color { flex: 1; min-width: 0;
    --event-fill: color-mix(in srgb, var(--cal) 16%, var(--bg));
    background: var(--event-fill);
    box-shadow: inset 2px 0 var(--cal), 0 -1px 0 var(--bg), 0 1px 0 var(--bg); }
  .copy-backdrop { position: absolute; left: 3px; right: 3px; z-index: 25;
    border: 0; padding: 0; border-radius: var(--event-card-radius, 6px);
    cursor: pointer; background: transparent; }
  .copy-panels { z-index: 26; }
  .copy-panel { appearance: none; -webkit-appearance: none; flex: 1; min-width: 0;
    padding: 0; margin: 0; border: 0; border-radius: 0; cursor: pointer;
    background: transparent; }
  .copy-panel:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .ev[data-combined-count].hovered { cursor: pointer; }
  /* Lifted while dragging so it reads as picked up, and above every other
     block so it is never hidden behind one it is passing over. No transition:
     the block is following a pointer and easing would put it behind the
     finger. */
  .ev.dragging { z-index: 50 !important; opacity: 0.85; cursor: grabbing; }
  .ev.keyboard { outline: 2px solid var(--accent); outline-offset: 1px;
                 z-index: 25 !important; }
  /* The grab bands, as cursors *and*, while the block is hovered, as
     something to aim at — the hit test itself is `drag.ts`'s `edgeAt`, on
     the press's own offset. The `.grip` spans carry `ns-resize` over those
     bands: sized by the same constant and gated by the same short-block
     rule, so what the cursor promises and what the press does cannot
     disagree.
     **A cursor alone was not enough** (#77, reported with a Google Calendar
     comparison and confirmed against an ordinary-length block). The band is
     6px of a ~68px block, and nothing marked it: you had to already know it
     was there to put the pointer in it, so the resize read as missing rather
     than as hidden. The bar appears only on hover, so a block at rest is
     unchanged and the committed baselines — which never hover — still hold
     these to invisibility. */
  .ev.hovered { cursor: grab; }
  .grip { position: absolute; left: 0; right: 0; cursor: ns-resize; }
  /* Centred in the band rather than filling it: the bar says "here", the
     band is what actually answers, and a filled 6px block would read as a
     border on an event that has none by design. */
  .ev.hovered .grip::after {
    content: ''; position: absolute; left: 50%; transform: translateX(-50%);
    top: 50%; margin-top: -1.5px; width: 26px; height: 3px; border-radius: 2px;
    background: color-mix(in srgb, currentColor 45%, transparent);
  }

  .ev.hovered.with-colors .bottom-grip::after { top: .5px; margin-top: 0; height: 2px; }

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
  .ev.hovered { left: 3px !important; width: calc(100% - 6px) !important; z-index: 20 !important;
              box-shadow: inset 2px 0 0 0 var(--spine),
                          0 -1px 0 0 var(--bg), 0 1px 0 0 var(--bg),
                          0 4px 14px rgba(0, 0, 0, .5); }
  /* Keep every saved event below the creation preview, regardless of its
     overlap column or keyboard selection. Alt must also override edge grips. */
  .ev.obscured { z-index: 1 !important; }
  .ev.obscured b, .ev.obscured em, .ev.obscured .rs { visibility: hidden; }
  .ev.create-mode { cursor: crosshair; z-index: 1 !important; }

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
  :global(:root[data-event-transparency]) .ev,
  :global(:root[data-event-transparency]) .copy-color {
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
  .calendar-line { display: flex; align-items: center; gap: 6px; margin-top: 4px; font-size: 11px; }
  .calendar-dot { width: 7px; height: 7px; flex: none; border-radius: 50%; background: var(--cal); }
  .calendar-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tip .tw { display: block; font-size: 11px; color: var(--muted); margin-top: 1px;
             white-space: normal; }
</style>
