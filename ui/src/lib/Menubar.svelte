<script lang="ts">
  import CalendarColors from './CalendarColors.svelte';
  import { formatDate, type DateFormat } from './datefmt';
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { invoke } from '@tauri-apps/api/core';
  import { applyPalette } from './theme';
  import { progress, joinable, uniqueAllDay, type Event } from '../../../packaging/omarchy-plugin/Timeline.mjs';

  type Panel = { agenda_days?: { date_label: string; events: Event[] }[]; truncated?: boolean; day_start_ms: number; day_end_ms: number; timezone: string;
    time_format: '12h' | '24h'; date_format?: DateFormat; label: boolean; join_minutes: number; events: Event[] };
  type Feed = { combine_identical_events?: boolean; events: Event[]; panel: Panel; tasks: { title: string; overdue: boolean }[] };
  let feed = $state<Feed | null>(null);
  let now = $state(Date.now());
  let error = $state('');
  const panel = $derived(feed?.panel);
  const today = $derived(displayEvents(panel?.events ?? []));
  function displayEvents(events: Event[]) { return feed?.combine_identical_events === undefined ? uniqueAllDay(events) : events; }
  const timed = $derived(today.filter(e => !e.all_day));
  const active = $derived(timed.find(e => e.start_ms <= now && e.end_ms > now));
  const nowIndex = $derived(timed.findIndex(e => e.end_ms > now));
  const allDay = $derived(today.filter(e => e.all_day));
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
        {#if allDay.length}<div class="all-day"><small>ALL DAY</small>{#each allDay as event}<button onclick={() => action(date(panel!.day_start_ms))}>{event.title ?? '(no title)'}<CalendarColors colors={event.colors} /></button>{/each}</div>{/if}
        {#if timed.length === 0}<p class="empty">No timed events today</p>{/if}
        {#each timed as event, i}
          {#if active && i === nowIndex}{@render nowMarker()}{/if}
          <button class="agenda-row" class:past={event.end_ms <= now} style:--event-color={color(event)} onclick={() => action(date(event.start_ms))}>
            <time>{clock(event.start_ms)}</time><span><strong>{event.title ?? '(no title)'}</strong><small>{event.end_ms <= now ? 'Ended' : event.start_ms <= now ? `${Math.ceil((event.end_ms - now) / 60000)}m left` : clock(event.end_ms)}{event.calendar ? ` · ${event.calendar}` : ''}</small></span><CalendarColors colors={event.colors} />
          </button>
        {/each}
        {#each panel?.agenda_days?.slice(1) ?? [] as day, i}
          <small class="section-label">{i === 0 ? 'TOMORROW' : day.date_label}</small>
          {#each displayEvents(day.events) as event}
            <button class="agenda-row" style:--event-color={color(event)} onclick={() => action(date(event.start_ms))}>
              <time>{event.all_day ? 'All day' : clock(event.start_ms)}</time>
              <span><strong>{event.title ?? '(no title)'}</strong><small>{event.calendar ?? ''}</small></span><CalendarColors colors={event.colors} />
            </button>
          {/each}
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
  .agenda-row { display: flex; width: 100%; text-align: left; align-items: center; gap: 12px; padding: 12px 8px; position: relative; border-left: 3px solid var(--event-color); margin: 5px 0; overflow: hidden; }
  time { width: 66px; flex-shrink: 0; font-variant-numeric: tabular-nums; font-size: 12px; } .agenda-row > span { min-width: 0; } .agenda-row strong, .agenda-row small { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; } small { color: var(--muted, #999); font-size: 11px; } .agenda-row small { margin-top: 4px; }
  .now-marker { display: flex; align-items: center; gap: 10px; color: var(--text, #e5e7eb); padding: 6px 0; margin: 8px 0; font-size: 11px; font-weight: 600; }
  .now-track { flex: 1; height: 3px; border-radius: 2px; background: color-mix(in srgb, var(--text, #e5e7eb) 15%, transparent); overflow: hidden; }
  .now-fill { display: block; height: 100%; border-radius: inherit; background: var(--now, #e2564a); }
  .all-day { border-top: 1px solid var(--hairline, #444); margin-top: 12px; padding-top: 10px; } .all-day button { position: relative; display: block; width: 100%; text-align: left; } .section-label { display: block; margin-top: 16px; } .empty { padding: 14px 0; }
</style>
