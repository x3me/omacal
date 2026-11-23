<!-- ui/src/lib/PriorityMarks.svelte -->
<script lang="ts">
  import { priorityLabel, PRIORITY_MARKS } from './tasks';

  /** A task's priority the way Fantastical draws it (#190): `!!!` High, `!!`
   *  Medium, `!` Low, nothing for none. Drawn on the grid's task chips — the
   *  TASKS row, the hour pins and the landing chip a drag draws for either —
   *  which set `--cal` to the calendar's colour. The Tasks pane keeps its word
   *  chip, held to 4.5:1 in every built-in theme; these follow the calendar's
   *  colour, so they can only be as clear as it is.
   *
   *  The colour is inked toward the text as an event's title is, from the same
   *  `--event-ink` knob and a chip's 60% fallback, so a theme that darkens its
   *  event titles darkens these with them: they are exactly as clear as an
   *  all-day event's title in the same calendar. (The task's own title beside
   *  them is plain `--text`.)
   *
   *  The marks are decoration for a screen reader: the span names the level
   *  and the glyphs inside are hidden, so it does not say "exclamation
   *  exclamation". `flex: 0 0 auto`, so a long title ellipsises before the
   *  marks are ever cut; on an hour pin too narrow to keep 2ch of title
   *  beside them, `WeekGrid` takes them out of the paint instead. */
  let { priority }: { priority: number } = $props();

  const level = $derived(priorityLabel(priority));
</script>

{#if level}
  <span class="prio" data-level={level.toLowerCase()} role="img"
        aria-label="{level} priority"><span aria-hidden="true">{PRIORITY_MARKS[level]}</span></span>
{/if}

<style>
  .prio { flex: 0 0 auto; font-weight: 800; letter-spacing: -.1em; line-height: 1;
          white-space: nowrap; user-select: none; -webkit-user-select: none;
          color: color-mix(in srgb, var(--cal) var(--event-ink, 60%), var(--text)); }
</style>
