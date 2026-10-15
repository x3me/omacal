<!-- ui/src/lib/CalendarGroupSwitcher.svelte -->
<script lang="ts">
  import { escapeCloses } from './dismiss.svelte';
  import CalendarList from './CalendarList.svelte';
  import type { Calendar } from './calendars';
  import { activeGroupIndex, groupChordLabel, shownIds, type CalendarGroup } from './calendargroups';

  let {
    calendars,
    groups,
    onapply,
    onsave,
    onchange,
  }: {
    calendars: Calendar[];
    groups: CalendarGroup[];
    /** Asked to show group `i`'s calendars. `App` owns it, because the chord
     *  does the same thing without this panel open. */
    onapply: (i: number) => void;
    /** Asked to store the whole new list. Rejects with the sentence to show. */
    onsave: (next: CalendarGroup[]) => Promise<void>;
    /** A calendar was ticked, relabelled or recoloured in the list below.
     *  The same refresh `CalendarPopover` is given, so `calendars` — and with
     *  it the label on the button — follows the box as it is ticked. */
    onchange: () => void;
  } = $props();

  let open = $state(false);
  /** `''` while the "save current" field is up, `null` while it is not. */
  let naming = $state<string | null>(null);
  let renaming = $state<number | null>(null);
  let renameDraft = $state('');
  let deleting = $state<number | null>(null);
  let message = $state<string | null>(null);
  let saving = $state(false);

  const active = $derived(activeGroupIndex(groups, calendars));
  const label = $derived(
    active >= 0
      ? groups[active].name
      : calendars.every((c) => !c.sync_enabled || c.selected) ? 'All calendars' : 'Custom',
  );
  // Only what the picker can toggle: a group never holds a calendar it could
  // not show again.
  const shown = $derived(shownIds(calendars));
  /** A group of nothing would hide every calendar when applied. */
  const nothingShown = $derived(shown.length === 0);

  function toggle(e: MouseEvent) {
    open = !open;
    // WebKit does not focus a <button> on click, only on Tab — see
    // `CalendarPopover.toggle`. Without it Escape has nothing to bubble from.
    if (open) (e.currentTarget as HTMLElement).focus();
    else reset();
  }

  function reset() {
    naming = null; renaming = null; deleting = null; message = null;
  }

  function close() {
    open = false;
    reset();
  }

  // The innermost thing first: Escape in the name field backs out of the field,
  // and only a second one closes the panel. The field's own handler stops the
  // first from also reaching this listener.
  escapeCloses(() => open, close);

  /** One write for every edit, so a refusal leaves the list as it was and says
   *  why in the panel rather than in a banner behind it. */
  async function commit(next: CalendarGroup[]): Promise<boolean> {
    saving = true;
    message = null;
    try {
      await onsave(next);
      return true;
    } catch (err) {
      message = String(err);
      return false;
    } finally {
      saving = false;
    }
  }

  async function saveNew() {
    const name = (naming ?? '').trim();
    if (!name || nothingShown) return;
    if (await commit([...groups, { name, calendarIds: shown }])) naming = null;
  }

  async function saveRename(i: number) {
    const name = renameDraft.trim();
    if (!name) return;
    if (await commit(groups.map((g, j) => (j === i ? { ...g, name } : g)))) renaming = null;
  }

  const updateToShown = (i: number) =>
    nothingShown ? Promise.resolve(false)
      : commit(groups.map((g, j) => (j === i ? { ...g, calendarIds: shown } : g)));

  async function remove(i: number) {
    if (await commit(groups.filter((_, j) => j !== i))) deleting = null;
  }

  function apply(i: number) {
    onapply(i);
    close();
  }

  /** Enter saves, Escape backs out of the field and goes no further. */
  function fieldKeys(e: KeyboardEvent, save: () => void, cancel: () => void) {
    // Enter or Escape that is choosing a candidate in an IME is not ours.
    if (e.isComposing) return;
    if (e.key === 'Enter') { e.preventDefault(); save(); }
    else if (e.key === 'Escape') { e.stopPropagation(); cancel(); }
  }

  function focusOnMount(node: HTMLInputElement) {
    node.focus();
    node.select();
  }
</script>

<!--
  **A saved choice of what is shown, one click or ⌘1–⌘9 away.** The button
  names the group whose calendars are exactly the ones on screen, and says
  "All calendars" or "Custom" when none is — derived, never stored, so a
  calendar ticked by hand in the picker moves the label with it.

  **The calendar list is in the panel on purpose.** A group is "the calendars
  shown right now", so the way to make one has to be where the groups are. It
  used to live two clicks away in Menu → Calendars, and a first-time user
  opening this panel saw only an empty list and a save button. The rows are
  `CalendarList`'s, unchanged — `selected` and `sync_enabled` stay separate
  switches — and Menu → Calendars is still there for the same list.

  Everything here is about *shown*. Applying a group never touches
  `sync_enabled`; a calendar a group hides is still syncing.
-->
<div class="cg-wrap">
  <button class="cg-trigger" onclick={toggle} aria-expanded={open} aria-haspopup="true"
          title="Calendar groups ({groupChordLabel(0) ?? ''}–9)">
    <span class="cg-label">{label}</span> <span class="cg-caret" aria-hidden="true">▾</span>
  </button>

  {#if open}
    <button class="cg-scrim" aria-label="Close" onclick={close}></button>

    <div class="cg-panel" role="group" aria-label="Calendar groups">
      {#if groups.length === 0}
        <p class="cg-empty">No groups yet. Pick the calendars below, then save them as a group.</p>
      {/if}

      {#each groups as g, i (i)}
        <div class="cg-row" class:cg-active={i === active}>
          {#if renaming === i}
            <input class="cg-field" aria-label="Group name" maxlength="60" bind:value={renameDraft} use:focusOnMount
                   onkeydown={(e) => fieldKeys(e, () => saveRename(i), () => (renaming = null))} />
            <button onclick={() => saveRename(i)} disabled={saving || !renameDraft.trim()}>Save</button>
            <button onclick={() => (renaming = null)}>Cancel</button>
          {:else if deleting === i}
            <span class="cg-ask">Delete “{g.name}”?</span>
            <button class="cg-danger" onclick={() => remove(i)} disabled={saving}>Delete</button>
            <button onclick={() => (deleting = null)}>Cancel</button>
          {:else}
            <button class="cg-pick" onclick={() => apply(i)} aria-current={i === active}>
              <span class="cg-mark" aria-hidden="true">{i === active ? '✓' : ''}</span>
              <span class="cg-name">{g.name}</span>
              {#if groupChordLabel(i)}<kbd>{groupChordLabel(i)}</kbd>{/if}
            </button>
            <button class="cg-icon" aria-label="Rename {g.name}" title="Rename"
                    onclick={() => { renaming = i; renameDraft = g.name; deleting = null; message = null; }}>✎</button>
            <button class="cg-icon" aria-label="Update {g.name} to the calendars shown now"
                    title="Update to the calendars shown now" disabled={saving || nothingShown}
                    onclick={() => updateToShown(i)}>⟳</button>
            <button class="cg-icon" aria-label="Delete {g.name}" title="Delete"
                    onclick={() => { deleting = i; renaming = null; message = null; }}>✕</button>
          {/if}
        </div>
      {/each}

      <div class="cg-cals">
        <div class="cg-head">Calendars</div>
        <CalendarList {calendars} {onchange} />
      </div>

      <div class="cg-foot">
        {#if naming === null}
          <button class="cg-save" disabled={nothingShown} onclick={() => { naming = ''; message = null; }}>Save current as group…</button>
        {:else}
          <div class="cg-row">
            <input class="cg-field" aria-label="New group name" placeholder="Group name" maxlength="60"
                   bind:value={naming} use:focusOnMount
                   onkeydown={(e) => fieldKeys(e, saveNew, () => (naming = null))} />
            <button onclick={saveNew} disabled={saving || nothingShown || !(naming ?? '').trim()}>Save</button>
            <button onclick={() => (naming = null)}>Cancel</button>
          </div>
        {/if}
        {#if nothingShown}<p class="cg-note">Show at least one calendar to save a group.</p>{/if}
        {#if message}<p class="cg-error" role="alert">{message}</p>{/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .cg-wrap { position: relative; }
  button { font: inherit; font-size: 12px; color: var(--muted); cursor: pointer;
           background: color-mix(in srgb, var(--text) 6%, transparent);
           border: 0; border-radius: 6px; padding: 4px 10px; }
  button:disabled { opacity: .5; cursor: default; }
  .cg-trigger { display: flex; align-items: center; gap: 4px; max-width: 170px; }
  .cg-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cg-caret { font-size: 9px; opacity: .7; }

  .cg-scrim { position: fixed; inset: 0; background: none; border: 0; cursor: default;
           z-index: 40; padding: 0; border-radius: 0; }
  .cg-panel { position: absolute; left: 0; top: calc(100% + 6px); z-index: 41;
           min-width: 300px; max-height: 70vh; overflow-y: auto;
           display: flex; flex-direction: column; gap: 2px;
           background: var(--surface); border: 1px solid var(--hairline);
           border-radius: 8px; padding: 6px; box-shadow: 0 8px 28px rgba(0,0,0,.45); }

  .cg-row { display: flex; align-items: center; gap: 4px; }
  .cg-row button { background: none; }
  .cg-row button:hover:not(:disabled) { background: color-mix(in srgb, var(--text) 8%, transparent); }
  .cg-pick { flex: 1; display: flex; align-items: center; gap: 6px; text-align: left; color: var(--text); }
  .cg-active .cg-pick { font-weight: 600; }
  .cg-mark { width: 12px; color: var(--accent); }
  .cg-name { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  kbd { font: inherit; font-size: 11px; color: var(--muted); }
  .cg-icon { padding: 3px 6px; }
  .cg-ask { flex: 1; font-size: 12px; color: var(--text); padding-left: 6px;
         overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cg-danger { color: var(--error); }
  .cg-field { flex: 1; min-width: 0; font: inherit; font-size: 12px; color: var(--text);
           background: var(--bg); border: 1px solid var(--hairline); border-radius: 6px;
           padding: 4px 8px; }

  .cg-empty { margin: 4px 6px 6px; font-size: 12px; color: var(--muted); max-width: 260px; }
  .cg-cals { border-top: 1px solid var(--hairline); margin-top: 4px; padding-top: 6px; }
  .cg-head { font-size: 10.5px; color: var(--muted); letter-spacing: .05em; padding: 0 6px 2px; }
  .cg-foot { border-top: 1px solid var(--hairline); margin-top: 4px; padding-top: 6px; }
  .cg-save { width: 100%; text-align: left; background: none; }
  .cg-save:hover { background: color-mix(in srgb, var(--text) 8%, transparent); }
  .cg-note { margin: 6px 6px 2px; font-size: 12px; color: var(--muted); }
  .cg-error { margin: 6px 6px 2px; font-size: 12px; color: var(--error); }
</style>
