import { invoke } from '@tauri-apps/api/core';

/**
 * One **event** search result, and the occurrence being offered.
 *
 * Mirrors `omacal_core::search::Hit`. A recurring event appears once, resolved
 * to the occurrence nearest today — see that type for why, and why it is
 * neither the first of a window nor the series' own start.
 */
export type Hit = {
  eventId: number;
  title: string;
  startMs: number;
  endMs: number;
  /** For an all-day occurrence, its date (`yyyy-mm-dd`) in its own calendar's
   *  zone: what the result names and where a jump lands. `startMs` read in
   *  this webview's zone can be the day before. `null` when timed. */
  allDayDate: string | null;
};

/**
 * One **task** search result.
 *
 * Mirrors `omacal::cli::TaskHit`, which is also `omacal tasks --json`'s row —
 * one task-row contract, not two. A task has no occurrence, so there is no
 * `startMs`; `dueMs` orders it, and `list` is the list's name.
 */
export type TaskHit = {
  id: number;
  summary: string;
  notes: string | null;
  /** A bare date (`2026-09-17`) or an RFC 3339 instant, server-formatted. */
  due: string | null;
  dueMs: number | null;
  dueAllDay: boolean;
  overdue: boolean;
  completed: boolean;
  /** RFC 5545 priority: `1` highest … `9` lowest, `0` for none (#152). */
  priority: number;
  list: string;
  listId: number;
  canWrite: boolean;
};

/**
 * What a search answers with: events and tasks, kept apart.
 *
 * Mirrors `omacal::search::SearchResults`. **Two arrays, not one list**: an
 * event is ordered by distance from today, a task by its due date, so the two
 * kinds travel side by side and the overlay draws two sections.
 */
export type SearchResults = { events: Hit[]; tasks: TaskHit[] };

/**
 * What choosing a result hands up. The two kinds land differently — an event
 * on the calendar's popover, a task in the Tasks pane — so the kind travels
 * with the hit rather than being inferred from its shape.
 */
export type Pick = { kind: 'event'; hit: Hit } | { kind: 'task'; hit: TaskHit };

/**
 * Titles containing `query`, on calendars the user displays: events nearest
 * first, open tasks by due date.
 *
 * A lookup and nothing else (spec §7): no write path, no network, and no
 * second way to reach an event's detail — choosing a result opens the popover
 * (event) or the Tasks pane (task) that already own that, with every guard.
 */
export const search = (query: string) => invoke<SearchResults>('search', { query });
