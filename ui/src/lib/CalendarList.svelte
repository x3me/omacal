<!-- ui/src/lib/CalendarList.svelte -->
<script lang="ts">
  import { tick } from 'svelte';
  import {
    byAccount, calendarInk, setCalendarLabel, setCalendarColor, setCalendarSelected, setCalendarSync, type Calendar,
  } from './calendars';
  import { CALENDAR_COLOURS } from './theme';

  let {
    calendars,
    spacious = false,
    onchange,
  }: {
    calendars: Calendar[];
    spacious?: boolean;
    onchange: () => void;
  } = $props();

  // The ids with a write in flight. A `Set` rather than a single id: with one
  // id, toggling calendar B while calendar A's call was still pending made
  // `busy` point at B and silently re-enabled A's row — a real double-submit,
  // caught by mutation-testing a delayed response. Reassigned wholesale on
  // every change (via `markBusy`) because `$state` does not make a plain
  // `Set`'s own mutations (`.add`/`.delete`) reactive — only the variable
  // binding itself.
  let busy = $state<Set<number>>(new Set());
  /** Last thing that happened: a removal's event count, or a failed toggle's
   *  error — always prefixed with which calendar it's about. Two rows can be
   *  in flight at once (see `busy` above), so an unattributed note is
   *  ambiguous about which one just settled; naming the calendar makes it
   *  unambiguous even when read mid-race. */
  let message = $state<{ text: string; kind: 'info' | 'error' } | null>(null);

  let editingLabel = $state<number | null>(null);
  let labelDraft = $state('');
  async function saveLabel(c: Calendar, reset = false) {
    markBusy(c.id, true);
    message = null;
    try {
      await setCalendarLabel(c.id, reset ? null : labelDraft.trim() || null);
      editingLabel = null;
      onchange();
    } catch (err) {
      message = { text: `${c.summary} · ${String(err)}`, kind: 'error' };
    } finally {
      markBusy(c.id, false);
    }
  }

  const groups = $derived(byAccount(calendars));

  /** Which row's swatches are open, by id. One at a time: ten swatches per row
   *  across a dozen calendars is a wall, and the colour is a rare choice. */
  let picking = $state<number | null>(null);

  /**
   * Sets or clears a calendar's colour.
   *
   * `null` clears, and that is a distinct request rather than "set it to the
   * colour Google uses" — a cleared calendar follows Google's from then on,
   * including when Google changes it. See `0006_calendar_colour.sql`.
   */
  async function chooseColour(c: Calendar, hex: string | null) {
    message = null;
    picking = null;
    markBusy(c.id, true);
    try {
      await setCalendarColor(c.id, hex);
      onchange();
    } catch (err) {
      message = { text: `${c.summary} · ${String(err)}`, kind: 'error' };
    } finally {
      markBusy(c.id, false);
    }
  }

  function markBusy(id: number, on: boolean) {
    const next = new Set(busy);
    if (on) next.add(id); else next.delete(id);
    busy = next;
  }

  async function toggleShown(c: Calendar, e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    message = null;
    markBusy(c.id, true);
    try {
      await setCalendarSelected(c.id, !c.selected);
      onchange();
    } catch (err) {
      // The click already flipped the checkbox's native `checked` property —
      // that happens before this handler even runs. `c.selected` never
      // changed (the call failed), so snap the box back to it directly
      // rather than trust that reassigning `calendars` will touch a DOM
      // property whose value looks, to the framework, like it never moved.
      input.checked = c.selected;
      message = { text: `${c.summary} · ${String(err)}`, kind: 'error' };
      onchange(); // still reload — anything else in the list may be stale too
    } finally {
      markBusy(c.id, false);
      // Re-enabling a checkbox that was disabled while it held focus does not
      // give it focus back — the browser already moved it to <body> the
      // moment `disabled` was set. Reclaim it once the attribute is actually
      // gone from the DOM (`tick()`), or a keyboard user is left stranded on
      // <body> with no visible indication of where they are.
      await tick();
      input.focus();
    }
  }

  async function toggleSync(c: Calendar, e: Event) {
    const btn = e.currentTarget as HTMLButtonElement;
    message = null;
    markBusy(c.id, true);
    try {
      const wasOn = c.sync_enabled;
      const removed = await setCalendarSync(c.id, !wasOn);
      if (wasOn) {
        message = {
          text: `${c.summary} · ${removed} event${removed === 1 ? '' : 's'} deleted`,
          kind: 'info',
        };
      }
      onchange();
    } catch (err) {
      message = { text: `${c.summary} · ${String(err)}`, kind: 'error' };
      onchange();
    } finally {
      markBusy(c.id, false);
      await tick();
      btn.focus();
    }
  }
</script>

<!--
  **The rows, extracted so one set of them can live in two hosts** — the
  header's popover and the settings modal's Calendars tab.

  An extraction rather than a second implementation, and the difference is
  checkable rather than a matter of taste: `CalendarPopover`'s own specs pass
  **unchanged**, because everything they assert — the two switches staying
  separate, the busy set, the focus reclaim, the naming of which calendar a
  message is about — is this file now, reached through the same DOM.

  Two things travelled with the rows and are the ones worth not losing:

  **`selected` and `sync_enabled` are separate switches.** Unticking hides a
  calendar and keeps its events; Remove stops syncing it and deletes them. A UI
  that collapsed the two would make "I don't want to see this today" delete a
  year of history.

  **The keyed `{#each ... (c.id)}` is keyed on the id and not the summary**, and
  the shape that makes it matter is narrower than it was previously recorded as.
  The key sits on the *inner* loop, which iterates within one account group, so
  the same name under two different accounts is two `{#each}` instances and
  never collides — that was the example the old comment gave, and it is not the
  one that fails. What does fail is **one account holding two calendars with the
  same name**, which Google lets you create: one instance, two identical keys,
  and Svelte's `each_key_duplicate` *throws* rather than rendering one row where
  two belong. A mutation is what told the difference; see
  `two calendars with the same name in one account both render`.
-->
<div class="list" class:spacious>
  {#each groups as group (group.id)}
    <div class="acct">{group.label}</div>
    {#each group.calendars as c (c.id)}
      <div class="row" class:off={!c.sync_enabled}>
        <label>
          <input
            type="checkbox"
            checked={c.selected}
            disabled={!c.sync_enabled || busy.has(c.id)}
            onchange={(e) => toggleShown(c, e)}
          />
          <span class="dot" aria-hidden="true" style="background:{calendarInk(c.color_hex)}"></span>
          <span class="name" title={c.summary}>{c.summary}</span>
        </label>
        <button class="label-button" type="button" disabled={busy.has(c.id)} aria-label="Label for {c.summary}"
          onclick={() => { editingLabel = c.id; labelDraft = c.label_override ?? c.summary; }}>Label…</button>
        <!-- The colour control. On the row, so it appears in both hosts — the
             header's popover and the settings tab — from one place. -->
        <button
          class="swatch"
          aria-label="Colour for {c.summary}"
          aria-expanded={picking === c.id}
          disabled={busy.has(c.id)}
          style="background:{calendarInk(c.color_hex)}"
          onclick={() => (picking = picking === c.id ? null : c.id)}
        ></button>
        <button
          class="sync"
          disabled={busy.has(c.id)}
          title={c.sync_enabled
            ? 'Stop syncing and delete this calendar’s local events'
            : 'Sync this calendar again'}
          onclick={(e) => toggleSync(c, e)}
        >{c.sync_enabled ? 'Remove' : 'Add'}</button>
      </div>
      {#if editingLabel === c.id}
        <form class="label-editor" onsubmit={(e) => { e.preventDefault(); saveLabel(c); }}>
          <label for="calendar-label-{c.id}">Calendar label</label>
          <input id="calendar-label-{c.id}" bind:value={labelDraft} maxlength="200" disabled={busy.has(c.id)} />
          <div>
            <button type="submit" disabled={busy.has(c.id)}>Save label</button>
            <button type="button" disabled={busy.has(c.id) || !c.label_override} onclick={() => saveLabel(c, true)}>Use provider name</button>
            <button type="button" onclick={() => (editingLabel = null)}>Cancel</button>
          </div>
          <p class="hint">Only in OmaCal. Provider name: {c.provider_summary ?? c.summary}</p>
        </form>
      {/if}
      {#if picking === c.id}
        <!-- A curated set, from `theme.ts` — no free picker. omacal draws on
             both a light and a dark Omarchy theme, and a colour chosen against
             one can be unreadable on the other; `ink.ts` then guarantees the
             text on top reads correctly for anything in the set. -->
        <div class="swatches" role="group" aria-label="Colour for {c.summary}">
          {#each CALENDAR_COLOURS as [label, hex] (hex)}
            <button
              class="pick"
              class:on={c.color_override === hex}
              aria-label={label}
              aria-pressed={c.color_override === hex}
              style="background:{hex}"
              onclick={() => chooseColour(c, hex)}
            ></button>
          {/each}
          <!-- **Clearing is its own action, not a swatch.** Choosing the colour
               Google happens to use today would look identical and stop
               following it the moment Google changed. -->
          <button
            class="clear"
            disabled={c.color_override === null}
            onclick={() => chooseColour(c, null)}
          >Use Google’s</button>
        </div>
      {/if}
    {/each}
  {/each}
  {#if message}
    <p class="note" class:err={message.kind === 'error'}>{message.text}</p>
  {/if}
  <p class="hint">
    Unticking hides a calendar. Removing stops syncing it and deletes its
    local events; you can add it back.
  </p>
</div>

<style>
  .label-editor { display: grid; gap: 8px; margin: 8px 0 16px; }
  .label-button, .label-editor button { font: inherit; font-size: 11px; color: var(--muted); cursor: pointer;
    background: color-mix(in srgb, var(--text) 6%, transparent); border: 0; border-radius: 5px; padding: 4px 8px; }
  .label-button:disabled, .label-editor button:disabled { opacity: .5; cursor: default; }
  .label-editor input { font: inherit; font-size: 13px; color: var(--text);
    background-color: color-mix(in srgb, var(--text) 5%, transparent);
    border: 1px solid var(--hairline); border-radius: 5px; padding: 4px 6px; width: 100%; box-sizing: border-box; }
  .label-editor div { display: flex; gap: 8px; flex-wrap: wrap; }
  .spacious .acct { padding: 24px 6px 10px; }
  .spacious .acct:first-child { padding-top: 0; }
  /* No box of its own — no background, no border, no padding. Each host frames
     it: the popover puts it in a floating panel, the settings tab in a column
     that is already inside one. A component that carried its own surface would
     draw a panel inside a panel in the second. */
  .list { display: flex; flex-direction: column; min-width: 0; }

  .acct { font-size: 10.5px; color: var(--muted); letter-spacing: .05em;
          padding: 6px 6px 3px; }
  /* The label absorbs the slack, so the swatch and the sync button pack into a
     fixed right-hand column instead of trailing each row's name — with
     `space-between` the swatch's x followed the label's width, and four rows
     gave four columns. */
  .row { display: flex; align-items: center; gap: 8px;
         padding: 3px 6px; border-radius: 5px; }
  .row:hover { background: color-mix(in srgb, var(--text) 5%, transparent); }
  .row.off .name { opacity: .45; }

  label { display: flex; align-items: center; gap: 7px; font-size: 12.5px;
          cursor: pointer; min-width: 0; flex: 1; }
  .dot { width: 8px; height: 8px; border-radius: 2.5px; flex: none; display: block; }
  .name { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  .swatch { width: 14px; height: 14px; border-radius: 4px; flex: none; cursor: pointer;
            border: 1px solid color-mix(in srgb, var(--text) 20%, transparent); padding: 0; }
  .swatch:disabled { opacity: .5; cursor: default; }

  .swatches { display: flex; flex-wrap: wrap; align-items: center; gap: 4px;
              padding: 4px 6px 8px; }
  .pick { width: 16px; height: 16px; border-radius: 4px; cursor: pointer; padding: 0;
          border: 1px solid color-mix(in srgb, var(--text) 20%, transparent); }
  /* The chosen one, marked by a ring rather than by a tick: a glyph on a
     16px swatch is unreadable against half the palette. */
  .pick.on { outline: 2px solid var(--text); outline-offset: 1px; }
  .clear { font: inherit; font-size: 10.5px; color: var(--muted); cursor: pointer;
           background: none; border: 1px solid var(--hairline); border-radius: 5px;
           padding: 2px 6px; margin-left: 2px; }
  .clear:disabled { opacity: .45; cursor: default; }

  /* Wide enough for either word, so a mixed list of Remove and Add rows does
     not push the swatch column about. */
  .sync { font: inherit; font-size: 11px; color: var(--muted); cursor: pointer;
          background: none; border: 1px solid var(--hairline); border-radius: 5px;
          padding: 2px 7px; flex: none; min-width: 52px; text-align: center; }

  .note { font-size: 11.5px; color: var(--muted); line-height: 1.4;
          margin: 8px 6px 0; padding: 6px 8px; border-radius: 5px;
          background: color-mix(in srgb, var(--text) 6%, transparent); }
  .note.err { color: var(--error); background: color-mix(in srgb, var(--error) 9%, transparent); }

  .hint { font-size: 10.5px; color: var(--muted); opacity: .8; line-height: 1.45;
          margin: 8px 6px 2px; }
</style>
