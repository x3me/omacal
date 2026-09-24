<script lang="ts">
  import type { Appearance } from './settings';
  import type { Palette } from './theme';

  /**
   * The theme choice as cards, each a miniature drawn in the palette it
   * would paint — a title-bar line in the text colour, the accent, and three
   * cards on the surface — so a theme is chosen by how it looks rather than
   * by its name. The palettes are the backend's (`appearance_previews`), the
   * same values choosing a card applies, so a card cannot preview one thing
   * and paint another.
   *
   * A radio group: one tab stop (the chosen card), arrows move and choose,
   * as a group of radio buttons does.
   */
  let {
    value,
    options,
    previews,
    disabled = false,
    onchange,
  }: {
    value: Appearance;
    options: ReadonlyArray<[Appearance, string]>;
    previews: ReadonlyMap<Appearance, Palette>;
    disabled?: boolean;
    onchange: (a: Appearance) => void;
  } = $props();

  let group = $state<HTMLDivElement | null>(null);

  function onkeydown(e: KeyboardEvent, index: number) {
    const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key];
    if (!step) return;
    e.preventDefault();
    const next = (index + step + options.length) % options.length;
    onchange(options[next][0]);
    group?.querySelectorAll<HTMLButtonElement>('[role="radio"]')[next]?.focus();
  }
</script>

<div class="themes" id="appearance" role="radiogroup" aria-label="Theme" bind:this={group}>
  {#each options as [id, label], i (id)}
    {@const p = previews.get(id)}
    <button
      type="button"
      role="radio"
      aria-checked={value === id}
      tabindex={value === id ? 0 : -1}
      class="card"
      class:on={value === id}
      {disabled}
      onclick={() => onchange(id)}
      onkeydown={(e) => onkeydown(e, i)}
    >
      <span
        class="mini"
        style:--mb={p?.bg}
        style:--ms={p?.surface}
        style:--mt={p?.text}
        style:--ma={p?.accent}
        aria-hidden="true"
      >
        <span class="bar"><span class="line"></span><span class="dot"></span></span>
        <span class="blocks"><span></span><span></span><span></span></span>
      </span>
      <span class="tname">{label}</span>
      <!-- An element, not `::after` content: generated text joins the
           accessible name, which made the chosen card "Light✓". -->
      {#if value === id}<span class="tick" aria-hidden="true">✓</span>{/if}
    </button>
  {/each}
</div>

<style>
  .themes { display: grid; grid-template-columns: repeat(auto-fill, minmax(132px, 1fr)); gap: 12px; width: 100%; }
  .card {
    position: relative; display: flex; flex-direction: column; gap: 7px; padding: 7px;
    border: 1px solid var(--border, color-mix(in srgb, var(--text) 14%, transparent)); border-radius: 8px;
    background: transparent; color: var(--text); text-align: left; cursor: pointer; font: inherit;
  }
  .card:hover:not(:disabled) { border-color: color-mix(in srgb, var(--accent) 55%, transparent); }
  .card.on { border-color: var(--accent); box-shadow: 0 0 0 1px var(--accent); }
  .card:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .card:disabled { cursor: default; opacity: .6; }
  .tick {
    position: absolute; top: -7px; right: -7px; width: 18px; height: 18px;
    border-radius: 50%; background: var(--accent); color: var(--bg);
    font-size: 11px; line-height: 18px; text-align: center; font-weight: 700;
  }
  .mini {
    display: flex; flex-direction: column; gap: 7px; padding: 8px; border-radius: 5px;
    background: var(--mb, var(--surface));
    border: 1px solid color-mix(in srgb, var(--mt, var(--text)) 12%, transparent);
  }
  .bar { display: flex; align-items: center; gap: 8px; }
  .line { flex: 1; height: 4px; border-radius: 2px; background: var(--mt, var(--text)); opacity: .85; }
  .dot { width: 11px; height: 11px; border-radius: 3px; background: var(--ma, var(--accent)); }
  .blocks { display: grid; grid-template-columns: repeat(3, 1fr); gap: 5px; }
  .blocks span { height: 22px; border-radius: 3px; background: var(--ms, var(--surface)); }
  .tname { font-size: 11.5px; line-height: 1.3; }
</style>
