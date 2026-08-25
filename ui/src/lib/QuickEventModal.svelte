<script lang="ts">
  import { onMount } from 'svelte';
  import type { Calendar } from './calendars';
  import { escapeCloses } from './dismiss.svelte';
  import { toEventInput, type EventFormResult, type EventFormValue } from './eventform';
  import {
    parseQuickEvent, quickPreviewRows, QUICK_EVENT_EXAMPLES,
  } from './quickevent';

  let {
    nowMs,
    anchorDayMs,
    calendarId,
    defaultDurationMinutes,
    calendars,
    oncreate,
    onedit,
    onclose,
  }: {
    nowMs: number;
    anchorDayMs: number;
    calendarId: number | null;
    defaultDurationMinutes: number;
    calendars: Calendar[];
    oncreate: (result: EventFormResult) => void;
    onedit: (value: EventFormValue) => void;
    onclose: () => void;
  } = $props();

  let line = $state('');
  let fieldEl: HTMLTextAreaElement | undefined = $state();
  let editEl: HTMLButtonElement | undefined = $state();
  const parsed = $derived(parseQuickEvent(line, {
    nowMs, anchorDayMs, calendarId, defaultDurationMinutes, calendars,
  }));
  const rows = $derived(quickPreviewRows(parsed, calendars));
  const guests = $derived(parsed.value.guests.length);

  onMount(() => fieldEl?.focus());
  escapeCloses(() => true, () => onclose());

  function create() {
    if (!parsed.ready || parsed.value.calendarId === null) return;
    oncreate({
      calendarId: parsed.value.calendarId,
      scope: 'this',
      fields: toEventInput(parsed.value, parsed.baseline),
      // Typing an address is an explicit invitation, and the button names the
      // mail effect before it is pressed. Continue editing takes the existing
      // form path when somebody wants the send/don't-send choice instead.
      notify: guests > 0 ? 'all' : 'none',
    });
  }

  function continueEditing() {
    if (line.trim() === '') return;
    onedit(parsed.value);
  }
</script>

<button class="scrim" aria-label="Close quick add" onclick={onclose}></button>

<div class="panel" role="dialog" aria-modal="true" aria-label="Quick add event">
  <form
    onsubmit={(e) => {
      e.preventDefault();
      create();
    }}
  >
    <header>
      <div>
        <h2>Quick add</h2>
        <p>Say what, when, how long, and who to invite — in any order.</p>
      </div>
      <kbd>Q</kbd>
    </header>

    <textarea
      bind:this={fieldEl}
      bind:value={line}
      rows="2"
      aria-label="Describe the event"
      placeholder="30 min at 2pm Meet with Tim invite tim@example.com"
      autocomplete="off"
      spellcheck="true"
      onkeydown={(e) => {
        // One line is the command. Shift+Enter remains available to somebody
        // deliberately putting a line break in a title; plain Enter is the
        // submit-and-create path the panel is for.
        if (e.key === 'Enter' && !e.shiftKey && !e.altKey && !e.ctrlKey && !e.metaKey) {
          e.preventDefault();
          create();
        } else if (e.key === 'Tab' && !e.shiftKey && !e.altKey && !e.ctrlKey && !e.metaKey
            && line.trim() !== '') {
          // The parsed line has two outcomes. Tab selects the conservative
          // one directly: Space then opens the full editor through the
          // button's native behavior, while Enter remains direct creation.
          e.preventDefault();
          editEl?.focus();
        }
      }}
    ></textarea>

    {#if line.trim() === ''}
      <div class="examples">
        <span>Try one</span>
        {#each QUICK_EVENT_EXAMPLES as example}
          <button type="button" onclick={() => (line = example)}>{example}</button>
        {/each}
        <p>
          Dates: <code>tomorrow</code>, <code>Fri</code>, <code>Aug 30</code> ·
          Time: <code>2p</code>, <code>14:00</code>, <code>2–3:30pm</code> ·
          Repeat: <code>MWF</code>, <code>TTh</code>, <code>every Tue</code> ·
          End: <code>until Sep 30</code>, <code>for 10 occurrences</code> ·
          Extras: <code>+meet</code>, <code>cal:Work</code>, <code>loc:"Room 4"</code>
        </p>
      </div>
    {:else}
      <div class="interpretation" aria-live="polite">
        {#if parsed.islands.length > 0}
          <div class="matched" aria-label="Matched phrases">
            <span>Matched</span>
            {#each parsed.islands as island, i (`${island.kind}-${island.text}-${i}`)}
              <b title={island.kind}>{island.text}</b>
            {/each}
          </div>
        {/if}

        <dl>
          {#each rows as row (row.label)}
            <div>
              <dt>{row.label}</dt>
              <dd>{row.value}</dd>
            </div>
          {/each}
        </dl>

        {#each parsed.warnings as warning}
          <p class="warning">{warning}</p>
        {/each}
        {#each parsed.errors as error}
          <p class="error">{error}</p>
        {/each}
      </div>
    {/if}

    <footer>
      <button type="button" class="quiet" onclick={onclose}>Cancel</button>
      <span class="grow"></span>
      <button
        bind:this={editEl}
        type="button"
        class="edit"
        aria-label="Continue editing"
        title="Space: continue editing · Enter: create"
        disabled={line.trim() === ''}
        onclick={continueEditing}
        onkeydown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey && !e.altKey && !e.ctrlKey && !e.metaKey) {
            e.preventDefault();
            create();
          }
        }}
      >Continue editing <kbd>Space</kbd></button>
      <button type="submit" class="primary" disabled={!parsed.ready}>
        {guests > 0 ? `Create & send ${guests} invite${guests === 1 ? '' : 's'}` : 'Create event'}
        <kbd>Enter</kbd>
      </button>
    </footer>
  </form>
</div>

<style>
  .scrim { position: fixed; inset: 0; z-index: 50; cursor: default; border: 0;
           background: color-mix(in srgb, var(--bg) 48%, transparent); }
  .panel { position: fixed; z-index: 51; left: 50%; top: 50%;
           transform: translate(-50%, -50%); width: 580px;
           max-width: calc(100vw - 32px); max-height: min(80vh, 680px);
           overflow-y: auto; color: var(--text); background: var(--surface);
           border: 1px solid var(--hairline); border-radius: 12px;
           box-shadow: 0 18px 60px rgba(0, 0, 0, .55); }
  form { display: flex; flex-direction: column; }
  header { display: flex; align-items: flex-start; justify-content: space-between;
           gap: 16px; padding: 16px 18px 12px; }
  h2 { margin: 0; font-size: 15px; font-weight: 650; letter-spacing: -.015em; }
  header p { margin: 3px 0 0; color: var(--muted); font-size: 11px; }
  kbd { padding: 2px 6px; border: 1px solid var(--hairline); border-radius: 5px;
        color: var(--muted); font: 10px/1.3 inherit; }
  textarea { box-sizing: border-box; width: calc(100% - 28px); margin: 0 14px;
             resize: none; padding: 12px 13px; font: 500 17px/1.45 inherit;
             letter-spacing: -.015em; color: var(--text);
             background: color-mix(in srgb, var(--text) 4%, transparent);
             border: 1px solid var(--hairline); border-radius: 8px; }
  textarea:focus { outline: 1px solid var(--accent); outline-offset: -1px; }
  textarea::placeholder { color: color-mix(in srgb, var(--muted) 70%, transparent); }

  .examples { display: flex; flex-direction: column; align-items: flex-start;
              gap: 3px; padding: 10px 18px 14px; }
  .examples > span, .matched > span { color: var(--muted); font-size: 9px;
                                     letter-spacing: .06em; text-transform: uppercase; }
  .examples button { max-width: 100%; overflow: hidden; text-overflow: ellipsis;
                     white-space: nowrap; border: 0; border-radius: 5px;
                     padding: 3px 6px; color: var(--muted); background: none;
                     font: 11px/1.35 inherit; text-align: left; cursor: pointer; }
  .examples button:hover { color: var(--text);
                           background: color-mix(in srgb, var(--text) 6%, transparent); }
  .examples p { margin: 6px 0 0; color: var(--muted); font-size: 9.5px; line-height: 1.55; }
  code { color: var(--text); font: inherit; }

  .interpretation { display: flex; flex-direction: column; gap: 8px;
                    padding: 10px 18px 14px; }
  .matched { display: flex; align-items: center; flex-wrap: wrap; gap: 4px; }
  .matched > span { margin-right: 3px; }
  .matched b { max-width: 150px; overflow: hidden; text-overflow: ellipsis;
               white-space: nowrap; padding: 2px 6px; border-radius: 999px;
               color: var(--muted); font: 500 9.5px/1.35 inherit;
               background: color-mix(in srgb, var(--accent) 10%, transparent); }
  dl { display: flex; flex-direction: column; gap: 1px; margin: 0;
       border: 1px solid var(--hairline); border-radius: 8px; overflow: hidden; }
  dl > div { display: grid; grid-template-columns: 76px minmax(0, 1fr); gap: 10px;
             padding: 6px 9px; background: color-mix(in srgb, var(--text) 3%, transparent); }
  dl > div + div { border-top: 1px solid var(--hairline); }
  dt { color: var(--muted); font-size: 9.5px; }
  dd { min-width: 0; margin: 0; overflow-wrap: anywhere; font-size: 11px; }
  .warning, .error { margin: 0; padding: 6px 8px; border-radius: 6px;
                     font-size: 10.5px; line-height: 1.4; }
  .warning { color: var(--muted); background: color-mix(in srgb, var(--accent) 8%, transparent); }
  .error { color: var(--error); background: color-mix(in srgb, var(--error) 9%, transparent); }

  footer { display: flex; align-items: center; gap: 7px; padding: 10px 14px;
           position: sticky; bottom: 0; z-index: 1;
           border-top: 1px solid var(--hairline); background: var(--surface); }
  .grow { flex: 1; }
  footer button { border: 1px solid var(--hairline); border-radius: 6px;
                  padding: 6px 11px; color: var(--text); background: none;
                  font: 11.5px/1.2 inherit; cursor: pointer; }
  footer button:disabled { opacity: .42; cursor: default; }
  .quiet { color: var(--muted); border-color: transparent; }
  .edit { background: color-mix(in srgb, var(--text) 5%, transparent); }
  footer kbd { margin-left: 5px; padding: 1px 4px; border-color: currentColor;
               opacity: .65; font-size: 8.5px; }
  footer .primary { color: var(--on-accent); background: var(--accent);
                    border-color: var(--accent); font-weight: 600; }
</style>
