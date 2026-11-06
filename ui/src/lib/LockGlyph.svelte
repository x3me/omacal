<!-- ui/src/lib/LockGlyph.svelte -->
<script lang="ts">
  /** The private-event lock, in the app's 16-unit icon grid like the
   *  filmstrip's repeat/people/camera glyphs. Filled body and a stroked
   *  shackle, the shape Fantastical uses, because at 10px a line-only lock
   *  turns into a smudge. `currentColor`, so it takes the event's own ink.
   *
   *  `role="img"` with a label rather than a `<title>`: a title renders as
   *  the engine's native tooltip, which `EventBlock` bans for the reasons it
   *  gives. Hosts that are buttons with their own `aria-label` say "private"
   *  in that label too, since a button's label replaces its content's. */
  let { size = 10 }: { size?: number } = $props();
</script>

<svg class="lock-glyph" viewBox="0 0 16 16" width={size} height={size} role="img" aria-label="Private">
  <path d="M4.9 7.2V5.3a3.1 3.1 0 0 1 6.2 0v1.9" fill="none" stroke="currentColor" stroke-width="1.7" />
  <rect x="2.6" y="7" width="10.8" height="8" rx="1.8" fill="currentColor" />
</svg>

<style>
  .lock-glyph { display: block; flex: none; }
</style>
