<!-- ui/src/lib/ImportPanel.svelte -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { escapeCloses } from './dismiss.svelte';
  import { clockFormat } from './clock.svelte';
  import { dateFormat } from './date.svelte';
  import { formatClock } from './timefmt';
  import { formatDate } from './datefmt';
  import { writableCalendars, type Calendar } from './calendars';
  import {
    fileName, planIcsImport, planSummary, runIcsImport,
    type ImportReport, type Planned,
  } from './importics';

  /** Importing an `.ics` a user dropped on the window (#67).
   *
   *  The panel is a preview first and an action second: it says what will
   *  happen — how many events, what is skipped and why, and how many guest
   *  lists are being left behind — before there is anything to confirm.
   *  That ordering is the feature. An import writes many events at once
   *  from a file the app did not author, and the one thing worse than not
   *  importing is importing something other than what the file said. */
  let { path, calendars, onclose, onimported }: {
    path: string;
    calendars: Calendar[];
    onclose: () => void;
    /** Told when events landed, so the caller can refetch the view. */
    onimported: () => void;
  } = $props();

  const options = $derived(writableCalendars(calendars));
  let calendarId = $state<number | null>(null);
  let plan = $state<Planned[] | null>(null);
  let report = $state<ImportReport | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);

  onMount(() => {
    if (calendarId === null && options.length > 0) calendarId = options[0].id;
  });
  escapeCloses(() => !busy, () => onclose());

  // Re-planned whenever the calendar changes: the answer depends on it —
  // its zone resolves floating times, and its own events are what "already
  // here" means.
  $effect(() => {
    const id = calendarId;
    if (id === null) return;
    plan = null;
    error = null;
    void (async () => {
      try {
        plan = await planIcsImport(id, path);
      } catch (e) {
        error = String(e);
      }
    })();
  });

  async function run() {
    if (busy || calendarId === null) return;
    busy = true;
    error = null;
    try {
      report = await runIcsImport(calendarId, path);
      if (report.imported > 0) onimported();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  const arriving = $derived((plan ?? []).filter((p) => p.kind === 'import'));
  const importing = $derived(arriving.length);
  const skipped = $derived((plan ?? []).filter((p) => p.kind === 'skip'));

  const DAY_FORMAT: Intl.DateTimeFormatOptions =
    { weekday: 'short', month: 'short', day: 'numeric' };

  /** When one planned event lands, in the app's own date and clock formats
   *  (#112) — the same pair `EventPopover` uses, not the locale's, because a
   *  preview reading `1:30 PM` beside a grid reading `13:30` would be the app
   *  disagreeing with itself.
   *
   *  An all-day event says its day and stops there: it has no time to show,
   *  and printing midnight would invent one. */
  function whenOf(p: { start_ms: number; all_day: boolean }): string {
    const day = formatDate(new Date(p.start_ms).getTime(), dateFormat(), DAY_FORMAT);
    return p.all_day ? day : `${day} · ${formatClock(p.start_ms, clockFormat())}`;
  }
</script>

<div class="scrim" role="presentation" onclick={() => { if (!busy) onclose(); }}></div>
<div class="panel" role="dialog" aria-modal="true" aria-label="Import {fileName(path)}">
  <h2>Import {fileName(path)}</h2>

  {#if report}
    <p class="summary">
      {report.imported} event{report.imported === 1 ? '' : 's'} imported.
    </p>
    {#if report.failed.length > 0}
      <p class="note err">
        {report.failed.length} could not be created:
        {report.failed.map((f) => f.summary).join(', ')}
      </p>
    {/if}
    <div class="actions">
      <button class="go" onclick={onclose}>Done</button>
    </div>
  {:else}
    {#if options.length === 0}
      <p class="note">No calendar here can be written to.</p>
    {:else}
      <label class="pick">
        Into
        <select bind:value={calendarId} disabled={busy}>
          {#each options as c (c.id)}
            <option value={c.id}>{c.summary}</option>
          {/each}
        </select>
      </label>
    {/if}

    {#if error}
      <p class="note err" role="alert">{error}</p>
    {:else if plan === null && options.length > 0}
      <p class="note">Reading…</p>
    {:else if plan}
      <p class="summary">{planSummary(plan)}</p>
      <!-- Named, not counted — and now on both sides of the line (#112).
           The skipped list was already here on that principle; a count alone
           for what *will* arrive asked the user to trust a number about a
           file they did not write. An import writes many events at once, so
           the one thing worse than not importing is importing something
           other than what the file said, and a name and a date is what makes
           that checkable before the button rather than after it. -->
      {#if arriving.length > 0}
        <ul class="arriving">
          {#each arriving as p (p.summary + p.start_ms)}
            <li>
              <span class="what">{p.summary}</span>
              <span class="when">{whenOf(p)}</span>
              {#if p.repeat}<span class="rep">{p.repeat}</span>{/if}
            </li>
          {/each}
        </ul>
      {/if}
      {#if skipped.length > 0}
        <ul class="skipped">
          {#each skipped as s (s.summary + s.reason)}
            <li><span class="what">{s.summary}</span><span class="why">{s.reason}</span></li>
          {/each}
        </ul>
      {/if}
      <p class="fine">Guests are never invited by an import.</p>
    {/if}

    <div class="actions">
      <button onclick={onclose} disabled={busy}>Cancel</button>
      <button class="go" onclick={run} disabled={busy || importing === 0}>
        {busy ? 'Importing…' : `Import ${importing}`}
      </button>
    </div>
  {/if}
</div>

<style>
  .scrim { position: fixed; inset: 0; z-index: 40; background: rgba(0, 0, 0, .35); border: 0; }
  .panel { position: fixed; z-index: 41; top: 50%; left: 50%; transform: translate(-50%, -50%);
           width: 380px; max-height: 70vh; overflow-y: auto;
           background: var(--surface); border: 1px solid var(--hairline);
           border-radius: 10px; padding: 16px 18px; box-shadow: 0 10px 34px rgba(0, 0, 0, .5);
           font-size: 12.5px; color: var(--text); }
  h2 { margin: 0 0 12px; font-size: 14px; font-weight: 600; overflow-wrap: anywhere; }
  .pick { display: flex; align-items: center; gap: 8px; color: var(--muted); }
  .pick select { flex: 1; font: inherit; }
  .summary { margin: 12px 0 6px; font-weight: 500; }
  .note { margin: 8px 0; color: var(--muted); }
  .note.err { color: var(--error); }
  .skipped, .arriving { list-style: none; margin: 6px 0 0; padding: 0; max-height: 30vh;
                        overflow-y: auto; scrollbar-width: none; }
  .skipped::-webkit-scrollbar, .arriving::-webkit-scrollbar { display: none; }
  .skipped li, .arriving li { display: flex; gap: 8px; padding: 3px 0;
                              border-top: 1px solid var(--hairline); }
  .when { flex: 0 0 auto; color: var(--muted); font-size: 11px; }
  .rep { flex: 0 0 auto; color: var(--muted); font-size: 11px; opacity: .8; }
  .arriving .what { flex: 1 1 auto; }
  .what { flex: 0 1 auto; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .why { flex: 1 1 auto; color: var(--muted); font-size: 11px; }
  .fine { margin: 10px 0 0; color: var(--muted); font-size: 11px; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 14px; }
  .actions button { font: inherit; padding: 5px 12px; border-radius: 6px;
                    border: 1px solid var(--hairline); background: none; color: var(--text);
                    cursor: pointer; }
  .actions .go { background: var(--accent-fill); color: var(--on-accent); border-color: transparent; }
  .actions button:disabled { opacity: .5; cursor: default; }
</style>
