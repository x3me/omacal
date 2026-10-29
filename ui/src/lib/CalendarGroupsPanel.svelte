<!-- ui/src/lib/CalendarGroupsPanel.svelte -->
<script lang="ts">
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
    /** A group's row was clicked. The host decides whether that also closes it. */
    onapply: (i: number) => void;
    /** Asked to store the whole new list. Rejects with the sentence to show. */
    onsave: (next: CalendarGroup[]) => Promise<void>;
    /** A calendar was ticked, relabelled or recoloured in the list below. */
    onchange: () => void;
  } = $props();

  /** `''` while the "save current" field is up, `null` while it is not. */
  let naming = $state<string | null>(null);
  let renaming = $state<number | null>(null);
  let renameDraft = $state('');
  let deleting = $state<number | null>(null);
  let message = $state<string | null>(null);
  let saving = $state(false);

  const active = $derived(activeGroupIndex(groups, calendars));
  // Only what the picker can toggle: a group never holds a calendar it could
  // not show again.
  const shown = $derived(shownIds(calendars));
  /** A group of nothing would hide every calendar when applied. */
  const nothingShown = $derived(shown.length === 0);

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
  **One body, two hosts.** The header's group switcher and Menu → Calendars
  both float this in their own panel, so the save field, the rows and their
  rename / update / delete exist once. The host owns opening, closing and
  Escape; this owns everything inside. Each open mounts a fresh one, so a
  half-typed name or a stale refusal never outlives its panel.

  **The calendar list is here on purpose.** A group is "the calendars shown
  right now", so the way to make one has to sit beside the boxes that say
  which those are. The rows are `CalendarList`'s, unchanged — `selected` and
  `sync_enabled` stay separate switches.

  With no group saved this is the calendar list plus one quiet "Save current
  as group…" line, and nothing else: Menu → Calendars is where the first group
  is made, and someone who never makes one should not be shown an empty
  section about it.
-->
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
      <button class="cg-pick" onclick={() => onapply(i)} aria-current={i === active}>
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

<div class="cg-cals" class:cg-split={groups.length > 0}>
  {#if groups.length > 0}<div class="cg-head">Calendars</div>{/if}
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
  <!-- Said only to someone using groups: a person who has never made one and
       hides every calendar is not told about a feature they did not reach for. -->
  {#if nothingShown && (naming !== null || groups.length > 0)}<p class="cg-note">Show at least one calendar to save a group.</p>{/if}
  {#if message}<p class="cg-error" role="alert">{message}</p>{/if}
</div>

<style>
  button { font: inherit; font-size: 12px; color: var(--muted); cursor: pointer;
           background: none; border: 0; border-radius: 6px; padding: 4px 10px; }
  button:disabled { opacity: .5; cursor: default; }
  button:hover:not(:disabled) { background: color-mix(in srgb, var(--text) 8%, transparent); }

  .cg-row { display: flex; align-items: center; gap: 4px; }
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

  .cg-split { border-top: 1px solid var(--hairline); margin-top: 4px; padding-top: 6px; }
  .cg-head { font-size: 10.5px; color: var(--muted); letter-spacing: .05em; padding: 0 6px 2px; }
  .cg-foot { border-top: 1px solid var(--hairline); margin-top: 4px; padding-top: 6px; }
  .cg-save { width: 100%; text-align: left; }
  .cg-note { margin: 6px 6px 2px; font-size: 12px; color: var(--muted); }
  .cg-error { margin: 6px 6px 2px; font-size: 12px; color: var(--error); }
</style>
