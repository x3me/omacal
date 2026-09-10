<script lang="ts">
  let { colors = [], segments = [], thickness = 3 }: {
    colors?: string[];
    segments?: { color: string; left: number; width: number }[];
    thickness?: number;
  } = $props();
  const items = $derived(segments.length ? segments
    : colors.map((color, i) => ({ color, left: i / colors.length, width: 1 / colors.length })));
</script>

{#if items.length > 1}
  <span class="calendar-colors" aria-hidden="true" style:height={`${thickness}px`}>
    {#each items as item}
      <span style:background-color={item.color}
        style:left={`calc(${item.left * 100}% + 1px)`}
        style:width={`max(0px, calc(${item.width * 100}% - 2px))`}></span>
    {/each}
  </span>
{/if}

<style>
  .calendar-colors { position: absolute; bottom: 0; left: 0; right: 0;
                     pointer-events: none; overflow: hidden; }
  .calendar-colors > span { position: absolute; top: 0; bottom: 0; border-radius: 1px; }
</style>
