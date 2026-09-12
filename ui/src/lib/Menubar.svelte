<script lang="ts">
  import { formatDate, type DateFormat } from './datefmt';
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { invoke } from '@tauri-apps/api/core';
  import { applyPalette } from './theme';
  import { agendaSections, progress, joinable, uniqueAllDay, type Event } from '../../../packaging/omarchy-plugin/Timeline.mjs';

  type Panel = { agenda_days?: { date_label: string; events: Event[] }[]; truncated?: boolean; day_start_ms: number; day_end_ms: number; timezone: string;
    time_format: '12h' | '24h'; date_format?: DateFormat; label: boolean; join_minutes: number; events: Event[] };
  type Feed = { events: Event[]; panel: Panel; tasks: { title: string; overdue: boolean }[] };
  let feed = $state<Feed | null>(null);
  let now = $state(Date.now());
  let error = $state('');
  const panel = $derived(feed?.panel);
  // Whether the fold of finished events is open. Renderer state, not a
  // setting: a fold that stayed open across popups would not be a fold.
  let earlierOpen = $state(false);
  const sections = $derived(panel ? agendaSections(panel, now, { earlierOpen }) : []);
  const todayTimed = $derived(uniqueAllDay(panel?.events ?? []).filter(e => !e.all_day));
  const active = $derived(todayTimed.find(e => e.start_ms <= now && e.end_ms > now));
  const hasOngoing = $derived(sections.some(s => s.title === 'ONGOING'));
  const call = $derived(feed ? joinable(feed.events, now, panel?.join_minutes ?? 5) : null);
  const heading = $derived(panel ? formatDate(panel.day_start_ms, panel.date_format, { weekday: 'long', month: 'short', day: 'numeric', timeZone: panel.timezone }) : 'Today');
  function clock(ms: number) {
    return new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit', hour12: panel?.time_format === '12h', timeZone: panel?.timezone }).format(ms);
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
  onMount(() => {
    void applyPalette(); void refresh();
    const changes = listen("menubar-changed", () => { void refresh(); });
    const timer = setInterval(() => { void refresh(); }, 15000);
    const clockTimer = setInterval(() => { now = Date.now(); }, 1000);
    const focus = () => { void applyPalette(); void refresh(); };
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
  {#if call}<button class="join" title={call.title ?? 'Meeting'} onclick={() => action('join')}>Join · {call.title ?? '(no title)'}</button>{/if}
  {#if error}<p role="alert">{error}</p>{/if}
  {#if panel?.truncated}<p>Showing the first 200 events. Open OmaCal for the complete day.</p>{/if}
  <div class="scroll">
    {#if !feed}<p class="empty">Loading calendar…</p>
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
            <div class="all-day"><small>ALL DAY</small>{#each sec.rows as event}<button onclick={() => action(date(sec.anchor_ms))}>{event.title ?? '(no title)'}</button>{/each}</div>
          {:else}
            {#if sec.title !== 'ONGOING' && sec.title !== 'UPCOMING'}<small class="section-label">{sec.title}</small>{/if}
            {#if sec.title === 'ONGOING' || (sec.title === 'UPCOMING' && !hasOngoing)}{@render nowMarker()}{/if}
            {#each sec.rows as event}
              <button class="agenda-row" class:past={event.end_ms <= now} style:--event-color={color(event)} onclick={() => action(date(event.all_day ? sec.anchor_ms : event.start_ms))}>
                <time>{event.all_day ? 'All day' : clock(event.start_ms)}</time>
                <span><strong>{event.title ?? '(no title)'}</strong><small>{event.all_day ? (event.calendar ?? '') : event.end_ms <= now ? 'Ended' : event.start_ms <= now ? `${Math.ceil((event.end_ms - now) / 60000)}m left` : clock(event.end_ms)}{!event.all_day && event.calendar ? ` · ${event.calendar}` : ''}</small></span>
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
  header, footer { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
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
  .join { background: var(--accent, #87b7ff); color: var(--on-accent, #10151c); flex-shrink: 0; text-overflow: ellipsis; overflow: hidden; white-space: nowrap; }
  .scroll { overflow: auto; flex: 1; min-height: 0; } footer { border-top: 1px solid var(--hairline, #444); padding-top: 8px; } footer button:first-child { margin-right: auto; }
  .past { opacity: .5; }
  .fold, .more { width: 100%; text-align: left; color: var(--muted, #999); font-size: 12px; padding: 8px 8px; }
  .fold:hover, .more:hover { color: var(--text, #e5e7eb); }
  .agenda-row { display: flex; width: 100%; text-align: left; align-items: center; gap: 12px; padding: 12px 8px; position: relative; border-left: 3px solid var(--event-color); margin: 5px 0; overflow: hidden; }
  time { width: 66px; flex-shrink: 0; font-variant-numeric: tabular-nums; font-size: 12px; } .agenda-row > span { min-width: 0; } .agenda-row strong, .agenda-row small { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; } small { color: var(--muted, #999); font-size: 11px; } .agenda-row small { margin-top: 4px; }
  .now-marker { display: flex; align-items: center; gap: 10px; color: var(--text, #e5e7eb); padding: 6px 0; margin: 8px 0; font-size: 11px; font-weight: 600; }
  .now-track { flex: 1; height: 3px; border-radius: 2px; background: color-mix(in srgb, var(--text, #e5e7eb) 15%, transparent); overflow: hidden; }
  .now-fill { display: block; height: 100%; border-radius: inherit; background: var(--now, #e2564a); }
  .all-day { border-top: 1px solid var(--hairline, #444); margin-top: 12px; padding-top: 10px; } .all-day button { display: block; width: 100%; text-align: left; } .section-label { display: block; margin-top: 16px; } .empty { padding: 14px 0; }
</style>
