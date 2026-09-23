<script lang="ts">
  import CalendarColors from './CalendarColors.svelte';
  import { formatDate, type DateFormat } from './datefmt';
  import { zoneClock } from './timefmt';
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { invoke } from '@tauri-apps/api/core';
  import { applyPalette } from './theme';
  import { setSetting } from './settings';
  import { visibleRange, layout, agendaSections, progress, joinable, uniqueAllDay, type Event } from '../../../packaging/omarchy-plugin/Timeline.mjs';

  type Panel = { visible_start_ms?: number; visible_end_ms?: number; agenda_days?: { date_label: string; events: Event[] }[]; truncated?: boolean; day_start_ms: number; day_end_ms: number; timezone: string;
    time_format: '12h' | '24h'; date_format?: DateFormat; day_view: boolean; label: boolean; join_minutes: number; events: Event[] };
  type Feed = { combine_identical_events?: boolean; events: Event[]; panel: Panel; tasks: { title: string; overdue: boolean }[] };
  let feed = $state<Feed | null>(null);
  let now = $state(Date.now());
  let error = $state('');
  let scrolling: HTMLDivElement;
  let timeline = $state<HTMLDivElement>();
  let initialScroll = false;
  const panel = $derived(feed?.panel);
  // Whether the fold of finished events is open. Renderer state, not a
  // setting: a fold that stayed open across popups would not be a fold.
  let earlierOpen = $state(false);
  const sections = $derived(panel ? agendaSections(panel, now, { earlierOpen, deduplicateAllDay: feed?.combine_identical_events === undefined }) : []);
  const todayTimed = $derived(uniqueAllDay(panel?.events ?? []).filter(e => !e.all_day));
  const active = $derived(todayTimed.find(e => e.start_ms <= now && e.end_ms > now));
  const hasOngoing = $derived(sections.some(s => s.title === 'ONGOING'));
  const allDay = $derived(uniqueAllDay(panel?.events ?? []).filter(e => e.all_day));
  const range = $derived(panel ? visibleRange(panel) : { start: 0, end: 86400000 });
  const gridHeight = $derived((range.end - range.start) / 60000);
  const rows = $derived(panel ? layout(uniqueAllDay(panel.events), range.start, range.end, 24 * 60000) : []);
  const call = $derived(feed ? joinable(feed.events, now, panel?.join_minutes ?? 5) : null);
  const fraction = $derived(panel ? Math.max(0, Math.min(1, (now - range.start) / (range.end - range.start))) : 0);
  const hours = $derived(panel ? Array.from({ length: Math.ceil((range.end - range.start) / 3600000) }, (_, i) => range.start + i * 3600000) : []);
  const heading = $derived(panel ? formatDate(panel.day_start_ms, panel.date_format, { weekday: 'long', month: 'short', day: 'numeric', timeZone: panel.timezone }) : 'Today');
  // **The app's own clock, not `Intl`'s.** `timefmt.ts` says why in full: the
  // 12-hour side has no locale luck (`1:30 PM` / `1:30 pm` / `13:30` in a
  // locale that ignores the flag) and a golden of this popup would then follow
  // the ICU data of whichever machine rendered it. Asking `Intl` here also put
  // three spellings of one instant on screen at once — this popup's, the grid's
  // and the bar widget's.
  //
  // `zoneClock` takes the zone explicitly, which is what the popup needs: it
  // draws the *feed's* zone, not this process's. The fallback is only for the
  // heading, the one clock that renders before a feed arrives; an alias there
  // is harmless, since the name is resolved to an offset, never displayed.
  function clock(ms: number) {
    const tz = panel?.timezone ?? Intl.DateTimeFormat().resolvedOptions().timeZone;
    return zoneClock(ms, tz, panel?.time_format === '12h' ? '12h' : '24h');
  }
  function color(e: Event) { return /^#[0-9a-f]{6}$/i.test(e.color ?? '') ? e.color! : 'var(--accent)'; }
  function date(ms: number) {
    return new Intl.DateTimeFormat('en-CA', { year: 'numeric', month: '2-digit', day: '2-digit', timeZone: panel?.timezone }).format(ms);
  }
  async function refresh() {
    try { feed = await invoke<Feed>('menubar_feed'); now = Date.now(); error = ''; }
    catch (e) { error = String(e); }
  }
  async function action(action: string) {
    try { await invoke('menubar_action', { action }); error = ''; }
    catch (e) { error = String(e); }
  }
  async function changeView(day: boolean) {
    if (!panel) return;
    try { await setSetting('menubarDayView', day); initialScroll = false; await refresh(); }
    catch (e) { error = String(e); }
  }
  $effect(() => {
    if (panel?.day_view && scrolling && timeline && !initialScroll) {
      const gridTop = timeline.getBoundingClientRect().top - scrolling.getBoundingClientRect().top + scrolling.scrollTop;
      scrolling.scrollTop = Math.max(0, gridTop + fraction * gridHeight - 100);
      initialScroll = true;
    }
  });
  onMount(() => {
    void applyPalette(); void refresh();
    const changes = listen("menubar-changed", () => { initialScroll = false; void refresh(); });
    const timer = setInterval(() => { void refresh(); }, 15000);
    const clockTimer = setInterval(() => { now = Date.now(); }, 1000);
    const focus = () => { initialScroll = false; void applyPalette(); void refresh(); };
    window.addEventListener('focus', focus);
    return () => { void changes.then(unlisten => unlisten()); clearInterval(timer); clearInterval(clockTimer); window.removeEventListener('focus', focus); };
  });
</script>

{#snippet nowMarker()}
  <div class="now-marker">
    <span>NOW</span>
    <div class="now-track" role={active ? 'progressbar' : undefined} aria-label={active ? `Elapsed time: ${active.title ?? '(no title)'}` : 'Current time'} aria-valuemin="0" aria-valuemax="100" aria-valuenow={active ? Math.round(progress(active, now) * 100) : 0}>
      <span class="now-fill" style:width={`${active ? progress(active, now) * 100 : 0}%`}></span>
    </div>
  </div>
{/snippet}

<svelte:window onkeydown={(e) => { if (e.key === 'Escape') void action('close'); if (e.key === 'j' && call) void action('join'); }} />
<div class="popup">
  <!-- The three actions are grouped, and that grouping is the whole point:
       `justify-content: space-between` shares the free space between *every*
       pair of children, so four children spread the buttons across the full
       width of the popup instead of gathering them at the right — reported
       on macOS, where the popup is wide enough to make it obvious. Two
       children, and the title keeps the left while the actions keep the
       right. -->
  <header>
    <div><strong>OmaCal</strong><p>{heading} · {clock(now)}</p></div>
    <div class="acts">
      <button class="icon" aria-label="Add event" title="Add event with natural language" onclick={() => action('quick-add')}>+</button>
      <button class="icon" aria-label="Open preferences" title="OmaCal preferences" onclick={() => action('preferences')}>⚙</button>
      <button class="icon" aria-label="Close agenda" onclick={() => action('close')}>×</button>
    </div>
  </header>
  <nav aria-label="Calendar popup view"><button aria-pressed={!panel?.day_view} onclick={() => changeView(false)}>Agenda</button><button aria-pressed={panel?.day_view ?? false} onclick={() => changeView(true)}>Day</button></nav>
  {#if call}<button class="join" title={call.title ?? 'Meeting'} onclick={() => action('join')}>Join · {call.title ?? '(no title)'}</button>{/if}
  {#if error}<p role="alert">{error}</p>{/if}
  {#if panel?.truncated}<p>Showing the first 200 events. Open OmaCal for the complete day.</p>{/if}
  <div class="scroll" bind:this={scrolling}>
    {#if !feed}<p class="empty">Loading calendar…</p>
    {:else if panel?.day_view}
      {#if allDay.length}<div class="all-day"><small>ALL DAY</small>{#each allDay as event}<button onclick={() => action(date(panel.day_start_ms))}>{event.title ?? '(no title)'}</button>{/each}</div>{/if}
      <div class="timeline" bind:this={timeline} style:height={`${gridHeight}px`} role="group" aria-label="Day calendar">
        {#each hours as hour}<div class="hour" style:top={`${(hour - range.start) / (range.end - range.start) * 100}%`}><span>{clock(hour)}</span></div>{/each}
        {#each rows as row}
          <button class="event" class:compact={row.height * gridHeight < 40} class:past={row.event.end_ms <= now} style:top={`${row.top * 100}%`} style:height={`${row.height * gridHeight}px`}
            style:left={`calc(64px + (100% - 70px) * ${row.lane / row.lanes})`} style:width={`calc((100% - 70px) / ${row.lanes} - 3px)`}
            style:--event-color={color(row.event)} title={`${clock(row.event.start_ms)}–${clock(row.event.end_ms)} · ${row.event.title ?? '(no title)'}`}
            onclick={() => action(date(panel!.day_start_ms))}>
            <strong>{row.event.title ?? '(no title)'}</strong><small>{clock(row.event.start_ms)} – {clock(row.event.end_ms)}{row.event.conference ? ' · ▣' : ''}</small>
          </button>
        {/each}
        {#if now >= range.start && now < range.end}<div class="now-line" style:top={`${fraction * 100}%`}><span>{clock(now)}</span></div>{/if}
      </div>
    {:else}
      <div class="agenda">
        {#if sections.length === 0}<p class="empty">Nothing coming up</p>{/if}
        {#each sections as sec (sec.title)}
          {#if sec.kind === 'folded'}
            <!-- One line for what has happened, so it never pushes what is
                 next down; a click opens it, for the rare "did I miss
                 something". -->
            <button class="fold" onclick={() => (earlierOpen = true)}>{sec.count} earlier today · show</button>
          {:else if sec.title === 'ALL DAY'}
            <!-- Today's all-day events keep their compact block: a title is
                 all there is to say, and the day is the one already open. -->
            <div class="all-day"><small>ALL DAY</small>{#each sec.rows as event}<button onclick={() => action(date(sec.anchor_ms))}>{event.title ?? '(no title)'}<CalendarColors colors={event.colors} /></button>{/each}</div>
          {:else}
            {#if sec.title !== 'ONGOING' && sec.title !== 'UPCOMING'}<small class="section-label">{sec.title}</small>{/if}
            {#if sec.title === 'ONGOING' || (sec.title === 'UPCOMING' && !hasOngoing)}{@render nowMarker()}{/if}
            {#each sec.rows as event}
              <button class="agenda-row" class:past={event.end_ms <= now} style:--event-color={color(event)} onclick={() => action(date(event.all_day ? sec.anchor_ms : event.start_ms))}>
                <time>{event.all_day ? 'All day' : clock(event.start_ms)}</time>
                <span><strong>{event.title ?? '(no title)'}</strong><small>{event.all_day ? (event.calendar ?? '') : event.end_ms <= now ? 'Ended' : event.start_ms <= now ? `${Math.ceil((event.end_ms - now) / 60000)}m left` : clock(event.end_ms)}{!event.all_day && event.calendar ? ` · ${event.calendar}` : ''}</small></span><CalendarColors colors={event.colors} />
              </button>
            {/each}
            {#if sec.more > 0}
              <!-- The cut is the feed's `per_day`, the same one the Omarchy
                   widget cuts at; the row opens OmaCal on that day. -->
              <button class="more" onclick={() => action(date(sec.anchor_ms))}>+{sec.more} more · open OmaCal</button>
            {/if}
          {/if}
        {/each}
      </div>
    {/if}
    {#if feed?.tasks.length}<div class="all-day"><small>DUE</small>{#each feed.tasks as task}<button onclick={() => action('open')}>{task.overdue ? '⚠ ' : ''}{task.title}</button>{/each}</div>{/if}
  </div>
  <footer><button onclick={() => action('open')}>Open OmaCal</button><button onclick={() => action('sync')}>Sync</button><button onclick={() => action('quit')}>Quit</button></footer>
</div>

<style>
  :global(body) { margin: 0; background: var(--bg, #20232b); color: var(--text, #e5e7eb); font-family: system-ui, sans-serif; font-size: 13px; }
  :global(*) { box-sizing: border-box; }
  .popup { height: 100vh; padding: 16px; display: flex; flex-direction: column; gap: 12px; border: 1px solid var(--muted, #555); }
  header, footer, nav { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
  header { justify-content: space-between; } header strong { font-size: 17px; } p { margin: 4px 0 0; color: var(--muted, #999); }
  .acts { display: flex; align-items: center; gap: 4px; flex-shrink: 0; }
  /* A square target of a known size, so the three glyphs line up with each
     other and are big enough to hit. `+`, `⚙` and `×` have very different
     optical weights at the same font-size — the box is what makes them read
     as one row of controls rather than three unrelated marks, and `line-height:
     1` stops the taller glyph pushing the header's height around. */
  .icon { width: 30px; height: 30px; padding: 0; display: inline-flex;
          align-items: center; justify-content: center; font-size: 16px;
          line-height: 1; color: var(--muted, #999); }
  .icon:hover { color: var(--text, #e5e7eb); }
  button { font: inherit; color: inherit; background: transparent; border: 1px solid transparent; border-radius: 5px; cursor: pointer; padding: 6px 10px; }
  button:hover { background: var(--surface, #303540); } button:focus-visible { outline: 2px solid var(--accent, #87b7ff); outline-offset: -2px; }
  nav { background: var(--surface, #303540); border-radius: 6px; padding: 3px; } nav button { flex: 1; } nav button[aria-pressed='true'] { background: var(--bg, #20232b); box-shadow: 0 1px 3px #0003; }
  .join { background: var(--accent, #87b7ff); color: var(--on-accent, #10151c); flex-shrink: 0; text-overflow: ellipsis; overflow: hidden; white-space: nowrap; }
  .scroll { overflow: auto; flex: 1; min-height: 0; } footer { border-top: 1px solid var(--hairline, #444); padding-top: 8px; } footer button:first-child { margin-right: auto; }
  .fold, .more { width: 100%; text-align: left; color: var(--muted, #999); font-size: 12px; padding: 8px 8px; }
  .fold:hover, .more:hover { color: var(--text, #e5e7eb); }
  .timeline { overflow: clip; position: relative; margin-top: 12px; } .hour { position: absolute; width: 100%; border-top: 1px solid var(--hairline, #444); } .hour span { font-size: 10px; color: var(--muted, #999); }
  .event { position: absolute; text-align: left; overflow: hidden; padding: 3px 6px; border-left: 3px solid var(--event-color); background: color-mix(in srgb, var(--event-color) 20%, var(--bg, #20232b)); }
  .event.compact small { display: none; }
  .event strong, .event small { display: block; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; } .event small { font-size: 10px; margin-top: 3px; }
  .past { opacity: .5; } .now-line { position: absolute; left: 60px; right: 0; border-top: 2px solid var(--now, #e2564a); pointer-events: none; } .now-line span { position: absolute; right: 100%; top: -8px; background: var(--bg, #20232b); color: var(--text, #e5e7eb); font-size: 10px; padding-right: 3px; white-space: nowrap; }
  .agenda-row { display: flex; width: 100%; text-align: left; align-items: center; gap: 12px; padding: 12px 8px; position: relative; border-left: 3px solid var(--event-color); margin: 5px 0; overflow: hidden; }
  time { width: 66px; flex-shrink: 0; font-variant-numeric: tabular-nums; font-size: 12px; } .agenda-row > span { min-width: 0; } .agenda-row strong, .agenda-row small { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; } small { color: var(--muted, #999); font-size: 11px; } .agenda-row small { margin-top: 4px; }
  .now-marker { display: flex; align-items: center; gap: 10px; color: var(--text, #e5e7eb); padding: 6px 0; margin: 8px 0; font-size: 11px; font-weight: 600; }
  .now-track { flex: 1; height: 3px; border-radius: 2px; background: color-mix(in srgb, var(--text, #e5e7eb) 15%, transparent); overflow: hidden; }
  .now-fill { display: block; height: 100%; border-radius: inherit; background: var(--now, #e2564a); }
  .all-day { border-top: 1px solid var(--hairline, #444); margin-top: 12px; padding-top: 10px; } .all-day button { position: relative; display: block; width: 100%; text-align: left; } .section-label { display: block; margin-top: 16px; } .empty { padding: 14px 0; }
</style>
