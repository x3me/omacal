<!-- ui/src/lib/ViewSwitcher.svelte -->
<script module lang="ts">
  // Declared in `views.ts` and re-exported here, so `import { type View }
  // from './ViewSwitcher.svelte'` goes on working while a plain `.ts` module
  // can reach it too. See `views.ts` for why the move was forced.
  export type { View } from './views';
  import type { View } from './views';
</script>

<script lang="ts">
  const SLOTS: { id: View; label: string }[] = [
    { id: 'day', label: 'Day' },
    { id: 'week', label: 'Week' },
    { id: 'month', label: 'Month' },
    { id: 'year', label: 'Year' },
    { id: 'bigyear', label: 'Big Year' },
  ];

  let { view, onpick }: { view: View; onpick: (v: View) => void } = $props();
</script>

<div class="vswitch" role="group" aria-label="View">
  {#each SLOTS as slot}
    <button
      class:active={view === slot.id}
      aria-pressed={view === slot.id}
      onclick={() => onpick(slot.id)}
    >{slot.label}</button>
  {/each}
</div>

<style>
  .vswitch { display: flex; gap: 1px; }
  .vswitch button {
    font: inherit; font-size: 12px; color: var(--muted); cursor: pointer;
    background: color-mix(in srgb, var(--text) 6%, transparent);
    border: 0; padding: 4px 10px;
  }
  .vswitch button:first-child { border-radius: 6px 0 0 6px; }
  .vswitch button:last-child { border-radius: 0 6px 6px 0; }
  .vswitch button.active { background: var(--accent-fill); color: var(--on-accent); font-weight: 600; }
  .vswitch button:disabled { opacity: .5; cursor: default; }
</style>
