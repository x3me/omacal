<!-- ui/src/lib/SearchOverlay.svelte -->
<script lang="ts">
  import { formatDate } from './datefmt';
  import { dateFormat } from './date.svelte';
  import { onMount } from 'svelte';
  import { escapeCloses } from './dismiss.svelte';
  import { search, type SearchResults, type TaskHit, type Pick } from './search';

  /** What choosing a result hands up. The two kinds land differently — an
   *  event on the calendar's popover, a task in the Tasks pane — so the kind
   *  travels with the hit rather than being inferred from its shape. */

  let {
    onclose,
    onpick,
  }: {
    onclose: () => void;
    /** A result was chosen. The caller lands on it — this panel finds things
     *  and hands one over, exactly as `WeekGrid` hands up a drag rather than
     *  writing it. */
    onpick: (pick: Pick) => void;
  } = $props();

  let query = $state('');
  let result = $state<SearchResults>({ events: [], tasks: [] });
  let error = $state<string | null>(null);
  let fieldEl: HTMLInputElement | undefined = $state();

  /**
   * **Which query the results on screen belong to.**
   *
   * The race this exists for: type `sta`, then `stan`. Both are in flight; the
   * first answers *after* the second, and the panel shows results for a query
   * the user has already moved past — with no keystroke left to correct it,
   * because the field already says `stan`.
   *
   * **A sequence number rather than a debounce**, and that is the decision.
   * A debounce makes the race rarer without making it impossible: two requests
   * can still overlap whenever one is slow, and "rarer" is the property that
   * makes a bug survive to production. A response whose sequence is not the
   * latest is dropped, which cannot be raced.
   *
   * Debouncing on top would be a cost question — fewer calls to a local
   * SQLite read — and is not one this needs answering now (spec §7: the data
   * is local, there is no network here).
   */
  let issued = 0;

  async function run(q: string) {
    const mine = ++issued;
    if (q.trim() === '') {
      result = { events: [], tasks: [] };
      error = null;
      return;
    }
    try {
      const found = await search(q);
      // Superseded: a later keystroke has already asked its own question, and
      // its answer is the one that belongs on screen.
      if (mine !== issued) return;
      result = found;
      error = null;
      // A new answer is a new list: the old selection pointed into the last one.
      active = -1;
    } catch (e) {
      if (mine !== issued) return;
      result = { events: [], tasks: [] };
      error = String(e);
    }
  }

  onMount(() => fieldEl?.focus());

  // Nothing opens over search, so it is always the topmost layer while it
  // exists. See `escapeCloses` for why the listener is on `window`.
  escapeCloses(() => true, () => onclose());

  /** Every result in the order drawn, events then tasks: what the arrow keys
   *  walk and Enter opens. */
  const picks = $derived<Pick[]>([
    ...result.events.map((hit) => ({ kind: 'event', hit }) as Pick),
    ...result.tasks.map((hit) => ({ kind: 'task', hit }) as Pick),
  ]);
  /** The result the arrows have chosen, as an index into `picks`; -1 for
   *  none, where Enter opens the top one. Focus stays in the field, so typing
   *  goes on refining while a result is chosen. */
  let active = $state(-1);

  function key(e: KeyboardEvent) {
    if (e.key === 'ArrowDown' && picks.length) {
      e.preventDefault();
      active = Math.min(active + 1, picks.length - 1);
    } else if (e.key === 'ArrowUp' && picks.length) {
      e.preventDefault();
      active = Math.max(active - 1, 0);
    } else if (e.key === 'Enter' && picks.length && !e.isComposing) {
      e.preventDefault();
      onpick(picks[Math.max(active, 0)]);
    }
  }

  const hasEvents = $derived(result.events.length > 0);
  const hasTasks = $derived(result.tasks.length > 0);
  /** The headings name a kind only when there is another to tell it from: one
   *  section needs no label, two do. */
  const sectioned = $derived(hasEvents && hasTasks);
  const empty = $derived(query.trim() !== '' && !hasEvents && !hasTasks);

  /** The day a result sits on, for the line under its title. */
  const when = (ms: number) =>
    formatDate(new Date(ms).getTime(), dateFormat(), {
      weekday: 'short', day: 'numeric', month: 'short', year: 'numeric',
    });

  /** A task has no occurrence, so its line is its due date — or an honest
   *  "No date" rather than a blank. */
  const whenTask = (t: TaskHit) =>
    t.dueMs === null ? 'No date' : when(t.dueMs);
</script>

<!--
  Over the calendar, which stays behind it (spec §1): closing without choosing
  leaves the user exactly where they were, because nothing here moves anything.
  The scrim is transparent for the same reason — this is a panel to look
  through, not a modal to be trapped in.
-->
<button class="scrim" aria-label="Close search" onclick={onclose}></button>

<div class="panel" role="dialog" aria-modal="true" aria-label="Search">
  <input
    bind:this={fieldEl}
    bind:value={query}
    oninput={() => run(query)}
    onkeydown={key}
    aria-controls="search-results"
    aria-activedescendant={active >= 0 ? `search-hit-${active}` : undefined}
    type="search"
    aria-label="Search events and tasks"
    placeholder="Search events and tasks"
    autocomplete="off"
    spellcheck="false"
  />

  {#if error}
    <p class="err" data-testid="search-error">{error}</p>
  {:else if empty}
    <p class="none">Nothing matches “{query.trim()}”.</p>
  {:else if hasEvents || hasTasks}
    <div id="search-results" role="listbox" aria-label="Results">
    {#if hasEvents}
      <section class="sec">
        {#if sectioned}<h3>Events</h3>{/if}
        <ul>
          {#each result.events as h, i (h.eventId)}
            <li>
              <button class="hit" id="search-hit-{i}" role="option" aria-selected={active === i}
                      class:active={active === i}
                      onclick={() => onpick({ kind: 'event', hit: h })}>
                <b>{h.title}</b>
                <em>{when(h.startMs)}</em>
              </button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}
    {#if hasTasks}
      <section class="sec">
        {#if sectioned}<h3>Tasks</h3>{/if}
        <ul>
          {#each result.tasks as t, i (t.id)}
            {@const n = result.events.length + i}
            <li>
              <button class="hit" id="search-hit-{n}" role="option" aria-selected={active === n}
                      class:active={active === n}
                      onclick={() => onpick({ kind: 'task', hit: t })}>
                <b>{t.summary}</b>
                <em>{whenTask(t)} · {t.list}</em>
              </button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}
    </div>
  {/if}
</div>

<style>
  /* Transparent, unlike the settings modal's: the calendar behind this is the
     thing the user is keeping their place in. */
  .scrim { position: fixed; inset: 0; background: none; border: 0; cursor: default; z-index: 50; }

  /* Near the top rather than centred: results grow downward, and a panel that
     re-centres as they arrive moves under the pointer. */
  .panel { position: fixed; z-index: 51; top: 12vh; left: 50%; transform: translateX(-50%);
           width: 420px; max-width: calc(100vw - 32px); max-height: 70vh;
           display: flex; flex-direction: column; overflow: hidden;
           background: var(--surface); border: 1px solid var(--hairline);
           border-radius: 10px; box-shadow: 0 12px 40px rgba(0, 0, 0, .5);
           font-size: 12px; color: var(--text); }

  input { font: inherit; font-size: 14px; color: var(--text); background: none;
          border: 0; border-bottom: 1px solid var(--hairline);
          padding: 12px 14px; width: 100%; box-sizing: border-box; flex: none; }
  input:focus { outline: none; }

  .sec { display: flex; flex-direction: column; min-height: 0; }
  /* The second section under the first, divided by the same hairline the
     field uses, so two sections read as two without a second weight. */
  .sec + .sec { border-top: 1px solid var(--hairline); }
  h3 { margin: 0; padding: 6px 14px 2px; font-size: 10px; font-weight: 600;
       letter-spacing: .06em; text-transform: uppercase; color: var(--muted); }

  ul { list-style: none; margin: 0; padding: 4px; overflow-y: auto; }
  .hit { display: flex; flex-direction: column; gap: 1px; width: 100%; text-align: left;
         font: inherit; cursor: pointer; background: none; border: 0;
         border-radius: 6px; padding: 6px 10px; color: var(--text); }
  .hit:hover, .hit:focus-visible, .hit.active { background: color-mix(in srgb, var(--text) 7%, transparent); }
  .hit b { font-size: 12px; font-weight: 600; letter-spacing: -.01em; }
  .hit em { font-style: normal; font-size: 10px; color: var(--muted); }

  .none, .err { font-size: 11px; margin: 0; padding: 10px 14px; }
  .none { color: var(--muted); }
  .err { color: var(--error); }
</style>
