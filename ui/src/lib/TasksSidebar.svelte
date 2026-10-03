<!-- ui/src/lib/TasksSidebar.svelte -->
<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { dateFormat } from './date.svelte';
  import { weekStartDay } from './weekstartstore.svelte';
  import DateField from './DateField.svelte';
  import TimeField from './TimeField.svelte';
  import ListPicker, { type ListChoice } from './ListPicker.svelte';
  import { beganDrag } from './drag';
  import { beginCarry, carried, endCarry, moveCarry, taskLanding, type TaskLanding } from './taskdrag.svelte';
  import { clockFormat } from './clock.svelte';
  import {
    createLocalTaskList, createTask, createTaskList, deleteTask, deleteTaskList, renameTaskList,
    searchDoneTasks, setTaskCompleted, updateTask, sortTasks, priorityLabel, priorityOption,
    PRIORITY_CHOICES, type Task, type TaskList, type TaskSort,
  } from './tasks';
  import { taskSort, setTaskSort } from './tasksort.svelte';
  import { setSetting } from './settings';
  import {
    refreshTasks, setTaskLists, setTaskRows, taskListRows, taskRevision, taskRows,
  } from './taskstore.svelte';
  import { TASKS_WIDTH_DEFAULT, TASKS_WIDTH_MAX, TASKS_WIDTH_MIN, clampTasksWidth } from './taskwidth';
  import {
    dateInputValue, doneLabel, doneToday, dueFromInputs, dueLabel, isOverdue, quickDue,
    timeInputValue, todayStartMs, whenOf, WHEN_LABEL, WHEN_ORDER, type When,
  } from './taskdates';

  /** The tasks list, beside the calendar rather than over it.
   *
   *  One control at the top does one job: it regroups the rows and nothing
   *  else. "By when" answers what needs doing now; "By list" mirrors the
   *  calendar list. Both are readings of the same tasks, so switching never
   *  changes which tasks are here — only the headings they sit under.
   *
   *  A row opens in place for editing. That is the whole of what a task can
   *  be given here: a title, a due date, a note and the list it is on, which
   *  is exactly what a VTODO carries and this app can write back. */
  let { onclose, width = TASKS_WIDTH_DEFAULT, onresize }: {
    onclose: () => void;
    /** The panel's width in pixels (#130). The caller owns it, because the
     *  caller is what stores it. */
    width?: number;
    /** A width the user just dragged (or stepped with the arrow keys). Sent
     *  on every move, so the panel follows the hand; the caller decides when
     *  to write it down. */
    onresize?: (px: number) => void;
  } = $props();

  /** The edge is the control (#130), the way the grid's own zoom is a
   *  gesture rather than a field. It is a `separator` with a tab stop, so
   *  the arrow keys reach a width a mouse-less hand could not. */
  let resizing = $state(false);

  function startResize(e: PointerEvent) {
    if (!onresize || e.button !== 0) return;
    e.preventDefault();
    const handle = e.currentTarget as HTMLElement;
    const originX = e.clientX;
    const from = width;
    handle.setPointerCapture(e.pointerId);
    resizing = true;
    const move = (m: PointerEvent) => onresize(clampTasksWidth(from + (m.clientX - originX)));
    const done = () => {
      resizing = false;
      handle.releasePointerCapture?.(e.pointerId);
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', done);
      handle.removeEventListener('pointercancel', done);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', done);
    handle.addEventListener('pointercancel', done);
  }

  function resizeKey(e: KeyboardEvent) {
    if (!onresize) return;
    const step = e.shiftKey ? 32 : 8;
    if (e.key === 'ArrowLeft') onresize(clampTasksWidth(width - step));
    else if (e.key === 'ArrowRight') onresize(clampTasksWidth(width + step));
    else if (e.key === 'Home') onresize(TASKS_WIDTH_MIN);
    else if (e.key === 'End') onresize(TASKS_WIDTH_MAX);
    else return;
    e.preventDefault();
  }

  /** `taskstore`'s one copy, which the grid draws from too: a task ticked
   *  off on the week is ticked off here, and the other way round. */
  const tasks = $derived(taskRows());
  const lists = $derived(taskListRows());
  let note = $state<string | null>(null);
  let busyIds = $state<Set<number>>(new Set());

  type Grouping = 'when' | 'list';
  let grouping = $state<Grouping>('when');

  let newTitle = $state('');
  let filterListId = $state<number | null>(null);
  let adding = $state(false);

  /** The row open for editing, and its fields while they are being typed.
   *  `list` is the list it will be on after Save: the one it is on, until
   *  another is picked (Plamen, 2026-09-18: the editor did not say which list
   *  a task was on, or offer another). */
  let editingId = $state<number | null>(null);
  let draft = $state({ summary: '', date: '', time: '', notes: '', list: 0, priority: 0 });
  /** The time field holds something that is not a time. Save waits for it:
   *  saving would quietly make the task all-day. */
  let timeInvalid = $state(false);
  let saving = $state(false);

  /** Recomputed on every load rather than ticking: the groups only move at
   *  midnight, and a list that reshuffles under a reader is worse than one
   *  that is a few hours stale until the next sync. */
  let nowMs = $state(Date.now());

  /** Whether the on-this-device list is being made, so the button cannot be
   *  pressed twice into two lists. (The backend is idempotent as well.) */
  let making = $state(false);

  async function makeLocalList() {
    if (making) return;
    making = true;
    note = null;
    try {
      setTaskLists(await createLocalTaskList());
      await load();
    } catch (e) {
      note = String(e);
    } finally {
      making = false;
    }
  }

  /** A list being named in place: a new one, or an existing one's rename by
   *  its id. One at a time, like the task editor. */
  let naming = $state<'new' | number | null>(null);
  let listName = $state('');
  let listBusy = $state(false);
  let listInput: HTMLInputElement | undefined = $state();
  /** The list whose delete is waiting for a yes. */
  let confirmingDelete = $state<number | null>(null);

  /** "New list" (2026-09-17, Plamen: lists for the tasks that sync with
   *  nothing). Lists are what "By list" shows, so that is where the name is
   *  typed, at the end of them. */
  async function startNewList() {
    grouping = 'list';
    confirmingDelete = null;
    naming = 'new';
    listName = '';
    note = null;
    await tick();
    listInput?.focus();
    listInput?.scrollIntoView({ block: 'nearest' });
  }

  async function startRename(id: number, name: string) {
    confirmingDelete = null;
    naming = id;
    listName = name;
    note = null;
    await tick();
    listInput?.select();
  }

  async function saveListName() {
    if (listBusy || naming === null) return;
    listBusy = true;
    note = null;
    try {
      const wanted = listName.trim();
      if (naming === 'new') {
        const before = new Set(lists.map((l) => l.calendarId));
        const next = await createTaskList(wanted);
        setTaskLists(next);
        // Where the next task typed goes: the list just made for it.
        const made = next.find((l) => !before.has(l.calendarId));
        if (made) filterListId = made.calendarId;
      } else {
        setTaskLists(await renameTaskList(naming, wanted));
      }
      naming = null;
    } catch (e) {
      // Left open: the name is one letter from right, not to be typed again.
      note = String(e);
    } finally {
      listBusy = false;
    }
  }

  async function deleteList(id: number) {
    if (listBusy) return;
    listBusy = true;
    note = null;
    try {
      setTaskLists(await deleteTaskList(id));
      if (filterListId === id) filterListId = null;
      if (editingId !== null && (tasks ?? []).find((t) => t.id === editingId)?.calendarId === id) editingId = null;
      confirmingDelete = null;
      await load();
    } catch (e) {
      note = String(e);
    } finally {
      listBusy = false;
    }
  }

  const listKeys = (e: KeyboardEvent) => {
    if (e.key === 'Enter') { e.preventDefault(); void saveListName(); }
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); naming = null; }
  };

  async function load() {
    try {
      await refreshTasks();
      nowMs = Date.now();
    } catch (e) {
      note = String(e);
      // Out of "Loading…", but never over a list the grid already has.
      if (taskRows() === null) setTaskRows([]);
    }
  }
  $effect(() => { void load(); });

  const targetList = $derived(
    filterListId === null
      ? (lists[0] ?? null)
      : (lists.find((l) => l.calendarId === filterListId) ?? null),
  );
  const open = $derived((tasks ?? []).filter((t) => !t.completed));
  /** The Done list shows today's by default: what was finished today is
   *  still part of today, and a week of ticked boxes buries it. The rest is
   *  one press away (`earlier`). */
  const done = $derived((tasks ?? []).filter((t) => doneToday(t, nowMs)));

  /** The Done history, fetched only when somebody opens it: completed
   *  before today, newest first, searchable, a page at a time. */
  const EARLIER_PAGE = 30;
  let earlierOpen = $state(false);
  let earlierQuery = $state('');
  let earlier = $state<Task[] | null>(null);
  let earlierMore = $state(false);
  /** Which request is the latest, so a slow answer to "ba" cannot land over
   *  the answer to "bank" typed after it. */
  let earlierAsk = 0;

  async function loadEarlier(count = EARLIER_PAGE, offset = 0) {
    const ask = ++earlierAsk;
    try {
      const page = await searchDoneTasks(earlierQuery.trim(), todayStartMs(nowMs), offset, count);
      if (ask !== earlierAsk) return;
      earlier = offset === 0 ? page.tasks : [...(earlier ?? []), ...page.tasks];
      earlierMore = page.more;
    } catch (e) {
      if (ask === earlierAsk) note = String(e);
    }
  }

  let earlierTimer: ReturnType<typeof setTimeout> | undefined;
  function searchEarlier() {
    clearTimeout(earlierTimer);
    earlierTimer = setTimeout(() => void loadEarlier(), 180);
  }

  // Asked again whenever the tasks change while it is open — a reopened task
  // leaves it, one completed in the grid moves into today's — keeping as
  // many rows as were already loaded, so "Show more" is not undone.
  $effect(() => {
    taskRevision();
    if (!earlierOpen) return;
    // Untracked: the query and the loaded rows are read inside, and typing
    // is `searchEarlier`'s to debounce, not this effect's to chase.
    untrack(() => void loadEarlier(Math.max(EARLIER_PAGE, earlier?.length ?? 0)));
  });
  $effect(() => () => clearTimeout(earlierTimer));

  /** The rows under each heading, in the order the headings appear. */
  const byWhen = $derived(
    WHEN_ORDER.map((when) => ({
      key: when as string,
      label: WHEN_LABEL[when],
      warn: when === 'overdue',
      color: null as string | null,
      list: null as TaskList | null,
      rows: sortTasks(
        open.filter((t) => whenOf(t, nowMs, weekStartDay()) === when),
        taskSort(),
      ),
    })).filter((g) => g.rows.length > 0),
  );
  /** Every list, the empty ones too: a list just made has nothing on it yet,
   *  and one that vanished until it did would look like it was never made. */
  const byList = $derived(
    lists.map((l) => ({
      key: `l${l.calendarId}`,
      label: l.name,
      warn: false,
      color: l.color,
      list: l,
      rows: sortTasks(
        open.filter((t) => t.calendarId === l.calendarId),
        taskSort(),
      ),
    })),
  );
  const groups = $derived(grouping === 'when' ? byWhen : byList);

  const listColor = (t: Task) =>
    t.color ?? lists.find((l) => l.calendarId === t.calendarId)?.color ?? 'var(--muted)';

  /** What the add row's picker offers: where the next task goes. */
  const addChoices = $derived<ListChoice[]>([
    { id: null, name: 'All lists', color: null },
    ...lists.map((l) => ({ id: l.calendarId, name: l.name, color: l.color })),
  ]);

  /** What the editor's picker offers: every list a task can be on, and the
   *  one this task is on even if it is not among them, so the field never
   *  shows nothing. */
  const moveChoices = (t: Task): ListChoice[] => {
    const all = lists.map((l) => ({ id: l.calendarId, name: l.name, color: l.color }));
    return all.some((c) => c.id === t.calendarId)
      ? all
      : [{ id: t.calendarId, name: t.calendar, color: t.color }, ...all];
  };

  async function toggle(task: Task) {
    if (busyIds.has(task.id)) return;
    note = null;
    busyIds = new Set([...busyIds, task.id]);
    try {
      setTaskRows(await setTaskCompleted(task.id, !task.completed));
    } catch (e) {
      note = String(e);
    } finally {
      const next = new Set(busyIds);
      next.delete(task.id);
      busyIds = next;
    }
  }

  async function remove(task: Task) {
    note = null;
    try {
      if (editingId === task.id) editingId = null;
      setTaskRows(await deleteTask(task.id));
    } catch (e) {
      note = String(e);
    }
  }

  async function add() {
    if (adding || targetList === null || newTitle.trim() === '') return;
    note = null;
    adding = true;
    try {
      setTaskRows(await createTask(targetList.calendarId, newTitle, null));
      newTitle = '';
    } catch (e) {
      note = String(e);
    } finally {
      adding = false;
    }
  }

  /** A list's own new line (Plamen, 2026-09-18: "an elegant way to add a
   *  task in a particular list"): "Add a task" at the end of a list opens an
   *  empty row there, the title is typed in place, and a date or an hour is
   *  one press away rather than in the way. Enter adds it and leaves a fresh
   *  line for the next, the way a list is written down. One line at a time,
   *  like the editor. */
  let lineList = $state<number | null>(null);
  let line = $state({ summary: '', date: '', time: '' });
  let lineWhen = $state(false);
  let lineTimeInvalid = $state(false);
  let lineBusy = $state(false);
  let lineInput: HTMLInputElement | undefined = $state();

  async function openLine(listId: number) {
    lineList = listId;
    line = { summary: '', date: '', time: '' };
    lineWhen = false;
    lineTimeInvalid = false;
    note = null;
    await tick();
    lineInput?.focus();
  }

  async function addLine() {
    if (lineBusy || lineList === null || line.summary.trim() === '' || lineTimeInvalid) return;
    lineBusy = true;
    note = null;
    const { ms, allDay } = dueFromInputs(line.date, line.time);
    try {
      setTaskRows(await createTask(lineList, line.summary, ms, allDay));
      nowMs = Date.now();
      line = { summary: '', date: '', time: '' };
      lineWhen = false;
    } catch (e) {
      // Kept as typed: it is one fix away from going in.
      note = String(e);
    } finally {
      lineBusy = false;
    }
    await tick();
    lineInput?.focus();
  }

  /** Leaving a line nobody wrote on closes it (Plamen, 2026-09-18: it
   *  stayed open after the pointer moved on). A line with a title, a date or
   *  an hour on it stays: a stray click must not cost what was typed. Moving
   *  between the line's own controls is not leaving. */
  function leaveLine(e: FocusEvent) {
    const to = e.relatedTarget as Node | null;
    if (to && (e.currentTarget as HTMLElement).contains(to)) return;
    if (line.summary.trim() === '' && line.date === '' && line.time === '') lineList = null;
  }

  const lineKeys = (e: KeyboardEvent) => {
    if (e.key === 'Enter') { e.preventDefault(); void addLine(); }
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); lineList = null; }
  };

  /** The line's due, said the way a row says it, while the date and time
   *  fields are folded away. */
  const lineDue = $derived.by(() => {
    const { ms, allDay } = dueFromInputs(line.date, line.time);
    return ms === null ? '' : dueLabel({ dueMs: ms, dueAllDay: allDay } as Task, nowMs, clockFormat(), dateFormat());
  });

  /** A task picked up by its title and carried onto the calendar (#115). A
   *  press that travels past the grid's own drag threshold is a carry; one
   *  that does not is the click that opens the editor, as it always was. */
  let press: { task: Task; x: number; y: number } | null = null;
  /** Swallows the click a carry that ends back on its own title would
   *  otherwise turn into, and nothing after it. */
  let carriedNotClicked = false;

  function pressTask(task: Task, e: PointerEvent) {
    if (e.button !== 0 || !task.canWrite) return;
    press = { task, x: e.clientX, y: e.clientY };
    window.addEventListener('pointermove', carryMove);
    window.addEventListener('pointerup', carryEnd);
    window.addEventListener('pointercancel', carryCancel);
    // Capture, so Escape ends the carry before anything else hears it.
    window.addEventListener('keydown', carryKey, true);
  }

  function carryMove(e: PointerEvent) {
    if (!press) return;
    if (!carried()) {
      if (!beganDrag(e.clientX - press.x, e.clientY - press.y)) return;
      beginCarry({ id: press.task.id, summary: press.task.summary, color: listColor(press.task) }, e.clientX, e.clientY);
      window.getSelection()?.removeAllRanges();
      return;
    }
    moveCarry(e.clientX, e.clientY);
  }

  function stopCarry(commit: boolean) {
    window.removeEventListener('pointermove', carryMove);
    window.removeEventListener('pointerup', carryEnd);
    window.removeEventListener('pointercancel', carryCancel);
    window.removeEventListener('keydown', carryKey, true);
    const task = press?.task;
    press = null;
    if (carried()) {
      carriedNotClicked = true;
      setTimeout(() => (carriedNotClicked = false), 0);
    }
    const at = endCarry(commit);
    if (task && at) void dropTask(task, at);
  }

  const carryEnd = () => stopCarry(true);
  const carryCancel = () => stopCarry(false);
  const carryKey = (e: KeyboardEvent) => {
    if (e.key !== 'Escape' || !carried()) return;
    e.preventDefault();
    e.stopPropagation();
    stopCarry(false);
  };
  $effect(() => () => {
    if (press) stopCarry(false);
  });

  /** The drop: the task is due where it landed, a date on the TASKS row and
   *  an hour in the grid. It stays on its list, with its title and note. */
  async function dropTask(task: Task, at: TaskLanding) {
    note = null;
    try {
      setTaskRows(await updateTask(task.id, task.summary, at.dueMs, at.allDay, task.notes));
      nowMs = Date.now();
    } catch (e) {
      note = String(e);
    }
  }

  function edit(task: Task) {
    if (!task.canWrite) return;
    editingId = task.id;
    timeInvalid = false;
    draft = {
      summary: task.summary,
      date: task.dueMs === null ? '' : dateInputValue(task.dueMs),
      time: timeInputValue(task),
      notes: task.notes ?? '',
      list: task.calendarId,
      // The raw stored value, so an untouched save writes the same integer
      // back — a server's non-canonical 7 is preserved, not normalised.
      priority: task.priority,
    };
  }

  /** A quick answer sets the day and clears the hour: "Tomorrow" means the
   *  day, not tomorrow-at-this-time. */
  function setQuick(kind: 'today' | 'tomorrow' | 'nextWeek') {
    draft = { ...draft, date: dateInputValue(quickDue(kind, nowMs)), time: '' };
  }

  /** The order switch: repaint at once through the rune, and persist. A
   *  failed write leaves the new order on screen and the old one stored; the
   *  next load restores it, which is the honest outcome for a preference. */
  function chooseSort(sort: TaskSort) {
    setTaskSort(sort);
    void setSetting('taskSort', sort).catch((e) => (note = String(e)));
  }

  async function save() {
    if (saving || editingId === null || draft.summary.trim() === '' || timeInvalid) return;
    saving = true;
    note = null;
    const { ms, allDay } = dueFromInputs(draft.date, draft.time);
    const from = (tasks ?? []).find((t) => t.id === editingId)?.calendarId;
    try {
      setTaskRows(await updateTask(
        editingId, draft.summary, ms, allDay, draft.notes.trim() || null,
        draft.list !== from ? draft.list : null, draft.priority,
      ));
      nowMs = Date.now();
      editingId = null;
    } catch (e) {
      note = String(e);
    } finally {
      saving = false;
    }
  }
</script>

{#snippet doneRow(t: Task, dated: boolean)}
  <div class="row done">
    <input
      type="checkbox"
      checked={true}
      disabled={!t.canWrite || busyIds.has(t.id)}
      aria-label="Reopen {t.summary}"
      onchange={() => toggle(t)}
    />
    <span class="tick" style:background={listColor(t)}></span>
    <span class="title">{t.summary}</span>
    {#if dated}
      <span class="due">{doneLabel(t, nowMs, dateFormat())}</span>
    {/if}
  </div>
{/snippet}

<aside class="side" aria-label="Tasks" style="width:{width}px; flex-basis:{width}px">
  <!-- The task in flight, under the pointer. Faded over anywhere it cannot
       land; the grid draws where it would. -->
  {#if carried()}
    {@const c = carried()!}
    <div class="carry" class:nowhere={taskLanding() === null} aria-hidden="true"
         style:left="{c.x + 12}px" style:top="{c.y + 10}px">
      <span class="tick" style:background={c.color ?? 'var(--muted)'}></span>{c.summary}
    </div>
  {/if}
  <div class="top">
    <h2>Tasks</h2>
    <div class="flex"></div>
    <!-- One control, one job: it regroups the rows below and changes
         nothing else on the screen. -->
    <div class="seg" role="group" aria-label="Group tasks">
      <button class:on={grouping === 'when'} aria-pressed={grouping === 'when'}
              onclick={() => (grouping = 'when')}>By when</button>
      <button class:on={grouping === 'list'} aria-pressed={grouping === 'list'}
              onclick={() => (grouping = 'list')}>By list</button>
    </div>
    <!-- A second view switch, orthogonal to grouping: it orders the rows
         *within* a group, and is kept. -->
    <div class="seg" role="group" aria-label="Order tasks">
      <button class:on={taskSort() === 'date'} aria-pressed={taskSort() === 'date'}
              onclick={() => chooseSort('date')}>Date</button>
      <button class:on={taskSort() === 'priority'} aria-pressed={taskSort() === 'priority'}
              onclick={() => chooseSort('priority')}>Priority</button>
    </div>
    <button class="close" aria-label="Close tasks" onclick={onclose}>×</button>
  </div>

  {#if note}
    <p class="note" role="alert">{note}</p>
  {/if}

  {#if lists.length > 0}
    <form class="add" onsubmit={(e) => { e.preventDefault(); void add(); }}>
      <input
        type="text"
        placeholder={lists.length > 1 && targetList ? `Add to ${targetList.name}…` : 'Add a task…'}
        aria-label="New task title"
        bind:value={newTitle}
        disabled={adding}
      />
      {#if lists.length > 1}
        <ListPicker dotOnly label="Task list" choices={addChoices} value={filterListId} disabled={adding}
                    onpick={(id) => (filterListId = id)} />
      {/if}
      <!-- A list of its own for what is being added: made on this device,
           named where "By list" shows the lists. -->
      <button type="button" class="newlist" aria-label="New list" title="New list on this device"
              onclick={() => void startNewList()}>
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" focusable="false">
          <path d="M3 4.5h6M3 8h6M3 11.5h4M12.5 9v5M10 11.5h5" fill="none" stroke="currentColor"
                stroke-width="1.4" stroke-linecap="round" />
        </svg>
      </button>
    </form>
  {/if}

  <div class="rows quiet-scroll">
    {#if tasks === null}
      <p class="empty">Loading…</p>
    {:else if tasks.length === 0 && (lists.length === 0 || grouping === 'when')}
      <!-- Two different nothings. With a list, the pane is empty because
           nothing is due; with none, it is empty because there is nowhere to
           put a task — and a Google account never brings one, since Google
           keeps tasks in another product. The button is the way out that
           needs no server at all. "By list" is neither: it shows the lists,
           empty ones included, so it falls through to them. -->
      {#if lists.length > 0}
        <p class="empty">No tasks yet.</p>
      {:else}
        <p class="empty">
          No task lists yet. Keep them on this machine, or connect an iCloud
          or CalDAV account (Settings → Accounts) to keep them on a server.
        </p>
        <div class="empty-do">
          <button type="button" class="make" onclick={makeLocalList} disabled={making}>
            {making ? 'Creating…' : 'Create a list on this device'}
          </button>
        </div>
      {/if}
    {:else}
      {#each groups as g (g.key)}
        <div class="head" class:listhead={g.list !== null}>
          {#if g.color}<span class="tick" style:background={g.color}></span>{/if}
          {#if g.list && naming === g.list.calendarId}
            <input class="lname" aria-label="List name" bind:this={listInput} bind:value={listName}
                   disabled={listBusy} onkeydown={listKeys} />
            <button type="button" class="lact on" onclick={() => void saveListName()} disabled={listBusy}>Save</button>
            <button type="button" class="lact" onclick={() => (naming = null)}>Cancel</button>
          {:else}
            <span class="hlabel" class:warn={g.warn}>{g.label}</span>
            <span class="count">{g.rows.length}</span>
            {#if g.list?.local}
              <!-- Only a list on this device is this pane's to rename or
                   delete; a server's list is the server's. -->
              <span class="lacts">
                <button type="button" class="lact" aria-label="Rename {g.label}"
                        onclick={() => void startRename(g.list!.calendarId, g.list!.name)}>Rename</button>
                <button type="button" class="lact" aria-label="Delete {g.label}"
                        onclick={() => { naming = null; confirmingDelete = g.list!.calendarId; }}>Delete</button>
              </span>
            {/if}
          {/if}
        </div>
        {#if g.list && confirmingDelete === g.list.calendarId}
          <div class="confirm" role="alert">
            <span>Delete “{g.label}” and all its tasks, done ones too?</span>
            <button type="button" class="lact danger" onclick={() => void deleteList(g.list!.calendarId)}
                    disabled={listBusy}>Delete list</button>
            <button type="button" class="lact" onclick={() => (confirmingDelete = null)}>Keep</button>
          </div>
        {/if}
        {#if g.list && g.rows.length === 0}
          <p class="empty nothing">Nothing on this list.</p>
        {/if}
        {#each g.rows as t (t.id)}
          {#if editingId === t.id}
            <!-- The row, open where it sits. Nothing moves and nothing
                 covers the week. -->
            <div class="editor">
              <input class="etitle" aria-label="Task title" bind:value={draft.summary}
                     disabled={saving} />
              <!-- Which list it is on, and the way to another: moved when
                   Save is pressed, with the rest of the edit. -->
              <div class="elist">
                <ListPicker compact label="List" choices={moveChoices(t)} value={draft.list}
                            disabled={saving || lists.length < 2}
                            onpick={(id) => { if (id !== null) draft = { ...draft, list: id }; }} />
              </div>
              <div class="quick">
                <button onclick={() => setQuick('today')} disabled={saving}>Today</button>
                <button onclick={() => setQuick('tomorrow')} disabled={saving}>Tomorrow</button>
                <button onclick={() => setQuick('nextWeek')} disabled={saving}>Next week</button>
                <button class="clear" onclick={() => (draft = { ...draft, date: '', time: '' })}
                        disabled={saving}>Clear</button>
              </div>
              <div class="when">
                <DateField label="Due date" bind:value={draft.date} disabled={saving} />
                <!-- Offered with or without a date: a time picked first lands
                     on today, and the date field says so, rather than the
                     field sitting disabled until the order is guessed. -->
                <TimeField label="Due time" bind:value={draft.time} bind:invalid={timeInvalid} disabled={saving}
                           isToday={draft.date === '' || draft.date === dateInputValue(Date.now())}
                           onchange={(v) => { if (v && draft.date === '') draft.date = dateInputValue(Date.now()); }} />
              </div>
              <label class="efield">
                <span class="elab">Priority</span>
                <!-- A native select, so its keyboard behaviour is the
                     platform's; the global rule gives it the chevron and the
                     chrome below gives it the same field look as the date and
                     time inputs. The raw value is what the app stores, 0 for
                     none. -->
                <select aria-label="Priority" disabled={saving}
                        onchange={(e) => (draft = { ...draft, priority: Number(e.currentTarget.value) })}>
                  {#each PRIORITY_CHOICES as c}
                    <option value={c.value} selected={priorityOption(draft.priority) === c.value}>{c.label}</option>
                  {/each}
                </select>
              </label>
              <textarea class="enotes" aria-label="Notes" rows="2" placeholder="Notes"
                        bind:value={draft.notes} disabled={saving}></textarea>
              <div class="eact">
                <button onclick={() => (editingId = null)} disabled={saving}>Cancel</button>
                <button class="go" onclick={() => void save()}
                        disabled={saving || draft.summary.trim() === '' || timeInvalid}>
                  {saving ? 'Saving…' : 'Save'}
                </button>
              </div>
            </div>
          {:else}
            <div class="row">
              <input
                type="checkbox"
                checked={false}
                disabled={!t.canWrite || busyIds.has(t.id)}
                aria-label="Complete {t.summary}"
                onchange={() => toggle(t)}
              />
              <span class="tick" style:background={listColor(t)}></span>
              <!-- The title opens the editor, and is the grab handle for
                   carrying the task onto the calendar. -->
              <button class="title" disabled={!t.canWrite}
                      onpointerdown={(e) => pressTask(t, e)}
                      onclick={() => { if (!carriedNotClicked) edit(t); }}>{t.summary}</button>
              {#if priorityLabel(t.priority)}
                <!-- The word carries the level; the tone is decoration, so it
                     reads in greyscale and to a screen reader. -->
                <span class="pchip {priorityLabel(t.priority)!.toLowerCase()}"
                      aria-label="Priority: {priorityLabel(t.priority)}">{priorityLabel(t.priority)}</span>
              {/if}
              {#if t.dueMs !== null}
                <span class="due" class:overdue={isOverdue(t, nowMs)}>
                  {dueLabel(t, nowMs, clockFormat(), dateFormat())}
                </span>
              {/if}
              {#if t.canWrite}
                <button class="del" aria-label="Delete {t.summary}" onclick={() => remove(t)}>×</button>
              {/if}
            </div>
          {/if}
        {/each}
        {#if g.list}
          {@const listId = g.list.calendarId}
          {#if lineList === listId}
            <div class="newline" onfocusout={leaveLine}>
              <div class="row">
                <span class="box" aria-hidden="true"></span>
                <span class="tick" style:background={g.color ?? 'var(--muted)'}></span>
                <input class="ntitle" aria-label="New task on {g.label}" placeholder="New task"
                       bind:this={lineInput} bind:value={line.summary} disabled={lineBusy}
                       onkeydown={lineKeys} />
                {#if lineDue && !lineWhen}<span class="due">{lineDue}</span>{/if}
                <button type="button" class="nwhen" class:set={lineDue !== ''} aria-label="Date and time"
                        aria-expanded={lineWhen} title="Date and time" onclick={() => (lineWhen = !lineWhen)}>
                  <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true" focusable="false">
                    <rect x="2.2" y="3.2" width="11.6" height="10.6" rx="2" fill="none" stroke="currentColor" stroke-width="1.3" />
                    <path d="M2.2 6.6h11.6M5.4 1.8v2.6M10.6 1.8v2.6" fill="none" stroke="currentColor"
                          stroke-width="1.3" stroke-linecap="round" />
                  </svg>
                </button>
              </div>
              {#if lineWhen}
                <!-- The editor's own date controls: one way to say when,
                     whether the task is new or not. -->
                <div class="nwhen-fields">
                  <div class="quick">
                    <button onclick={() => (line = { ...line, date: dateInputValue(quickDue('today', nowMs)), time: '' })}
                            disabled={lineBusy}>Today</button>
                    <button onclick={() => (line = { ...line, date: dateInputValue(quickDue('tomorrow', nowMs)), time: '' })}
                            disabled={lineBusy}>Tomorrow</button>
                    <button onclick={() => (line = { ...line, date: dateInputValue(quickDue('nextWeek', nowMs)), time: '' })}
                            disabled={lineBusy}>Next week</button>
                    {#if line.date}
                      <button class="clear" onclick={() => (line = { ...line, date: '', time: '' })}
                              disabled={lineBusy}>Clear</button>
                    {/if}
                  </div>
                  <div class="when">
                    <DateField label="Due date" bind:value={line.date} disabled={lineBusy} />
                    <TimeField label="Due time" bind:value={line.time} bind:invalid={lineTimeInvalid} disabled={lineBusy}
                               isToday={line.date === '' || line.date === dateInputValue(Date.now())}
                               onchange={(v) => { if (v && line.date === '') line.date = dateInputValue(Date.now()); }} />
                  </div>
                </div>
              {/if}
            </div>
          {:else}
            <button type="button" class="addline" onclick={() => void openLine(listId)}>+ Add a task</button>
          {/if}
        {/if}
      {/each}

      {#if grouping === 'list' && naming === 'new'}
        <div class="head listhead">
          <input class="lname" aria-label="New list name" placeholder="List name" bind:this={listInput}
                 bind:value={listName} disabled={listBusy} onkeydown={listKeys} />
          <button type="button" class="lact on" onclick={() => void saveListName()}
                  disabled={listBusy || listName.trim() === ''}>Create</button>
          <button type="button" class="lact" onclick={() => (naming = null)}>Cancel</button>
        </div>
      {:else if grouping === 'list'}
        <button type="button" class="earlier-toggle" onclick={() => void startNewList()}>+ New list</button>
      {/if}

      {#if done.length > 0}
        <div class="head"><span class="hlabel">Done today</span><span class="count">{done.length}</span></div>
        {#each done as t (t.id)}
          {@render doneRow(t, false)}
        {/each}
      {/if}

      {#if lists.length > 0 || earlierOpen}
        <button type="button" class="earlier-toggle" aria-expanded={earlierOpen}
                onclick={() => {
                  earlierOpen = !earlierOpen;
                  if (!earlierOpen) { earlierQuery = ''; earlier = null; }
                }}>
          {earlierOpen ? 'Hide earlier done tasks' : 'Show earlier done tasks'}
        </button>
      {/if}
      {#if earlierOpen}
        <div class="head"><span class="hlabel">Done earlier</span></div>
        <input class="earlier-search" type="search" aria-label="Search done tasks"
               placeholder="Search done tasks…" bind:value={earlierQuery} oninput={searchEarlier} />
        {#if earlier === null}
          <p class="empty">Loading…</p>
        {:else if earlier.length === 0}
          <p class="empty">{earlierQuery.trim() ? 'No done tasks match.' : 'Nothing done before today.'}</p>
        {:else}
          {#each earlier as t (t.id)}
            {@render doneRow(t, true)}
          {/each}
          {#if earlierMore}
            <button type="button" class="earlier-toggle" onclick={() => void loadEarlier(EARLIER_PAGE, earlier?.length ?? 0)}>
              Show more
            </button>
          {/if}
        {/if}
      {/if}
    {/if}
  </div>
  <!-- The edge, and the whole of the control. `separator` rather than a
       button: what it does is change a size, and the arrow keys are how it
       is done without a pointer. -->
  {#if onresize}
    <!-- ARIA's window-splitter pattern to the letter: a focusable
         `separator` carrying the value it moves. Svelte's a11y rules know
         `separator` only as decoration, and a <button> with the role trips
         the mirror-image rule, so the two warnings are answered here rather
         than by wearing the wrong element. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="resize"
      class:on={resizing}
      role="separator"
      aria-orientation="vertical"
      aria-label="Tasks width"
      aria-valuenow={width}
      aria-valuemin={TASKS_WIDTH_MIN}
      aria-valuemax={TASKS_WIDTH_MAX}
      tabindex="0"
      onpointerdown={startResize}
      onkeydown={resizeKey}
      ondblclick={() => onresize?.(TASKS_WIDTH_DEFAULT)}
    ></div>
  {/if}
</aside>

<style>
  .side { position: relative; display: flex; flex-direction: column; flex-grow: 0; flex-shrink: 0;
          min-height: 0; border-right: 1px solid var(--hairline); font-size: 12px; }
  /* Over the border rather than beside it: a 1px hairline is not something
     a hand can catch, and a strip that took layout width would move the
     panel's contents every time the pointer approached. */
  /* Over the border rather than beside it: a 1px hairline is not something
     a hand can catch. */
  .resize { position: absolute; top: 0; bottom: 0; right: -3px; width: 7px; z-index: 2;
            cursor: col-resize; touch-action: none; }
  .resize:hover::after, .resize.on::after, .resize:focus-visible::after {
    content: ''; position: absolute; top: 0; bottom: 0; left: 3px; width: 1px;
    background: var(--accent); }
  .resize:focus-visible { outline: none; }
  .top { display: flex; align-items: center; gap: 8px; padding: 12px 10px 10px 14px; }
  .empty-do { padding: 0 14px 14px; }
  .make { font: inherit; font-size: 12px; cursor: pointer; color: var(--text);
          background: color-mix(in srgb, var(--text) 6%, transparent);
          border: 1px solid var(--hairline); border-radius: 7px; padding: 6px 10px; }
  .make:hover:not(:disabled) { background: color-mix(in srgb, var(--text) 10%, transparent); }
  .make:disabled { opacity: 0.6; cursor: default; }
  h2 { margin: 0; font-size: 13px; font-weight: 600; color: var(--text); }
  .flex { flex-grow: 1; }
  .seg { display: flex; gap: 2px; background: color-mix(in srgb, var(--text) 4%, transparent);
         border-radius: 7px; padding: 2px; }
  .seg button { appearance: none; -webkit-appearance: none; font: inherit; border: 0;
                background: none; color: var(--muted); padding: 3px 9px; border-radius: 5px;
                cursor: pointer; }
  .seg button.on { background: color-mix(in srgb, var(--text) 8%, transparent);
                   color: var(--text); font-weight: 500; }
  .close { appearance: none; -webkit-appearance: none; font: inherit; font-size: 15px;
           border: 0; background: none; color: var(--muted); cursor: pointer; padding: 0 2px; }

  .note { margin: 0 12px 8px; color: var(--error); }
  .add { display: flex; gap: 6px; margin: 0 12px 10px; }
  .add input { flex-grow: 1; min-width: 0; font: inherit; padding: 7px 10px; border-radius: 7px;
               border: 1px solid var(--hairline); background: var(--surface); color: var(--text); }
  .newlist { display: inline-flex; align-items: center; justify-content: center; flex: 0 0 auto;
             width: 30px; border-radius: 7px; border: 1px solid var(--hairline); cursor: pointer;
             background: var(--surface); color: var(--text); padding: 0; }
  .newlist:hover { background: color-mix(in srgb, var(--text) 8%, var(--surface)); }
  /* A list's heading carries its own two actions, quiet until pointed at or
     reached by the keyboard, so a pane of lists reads as a pane of lists. */
  .lacts { margin-left: auto; display: inline-flex; gap: 2px; opacity: 0; }
  .listhead:hover .lacts, .listhead:focus-within .lacts { opacity: 1; }
  .lact { appearance: none; -webkit-appearance: none; font: inherit; font-size: 10.5px;
          color: var(--muted); background: none; border: 0; border-radius: 4px; cursor: pointer;
          padding: 2px 5px; }
  .lact:hover:not(:disabled) { color: var(--text); background: color-mix(in srgb, var(--text) 8%, transparent); }
  .lact.on { color: var(--accent); }
  .lact.danger { color: var(--error); }
  .lact:disabled { opacity: .5; cursor: default; }
  .lname { flex: 1; min-width: 0; font: inherit; font-size: 12px; padding: 3px 7px; border-radius: 5px;
           border: 1px solid var(--accent); background: var(--bg); color: var(--text); }
  .confirm { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 6px; margin: 0 6px 6px;
             padding: 6px 8px; border-radius: 6px; font-size: 11.5px;
             background: color-mix(in srgb, var(--error) 10%, transparent); }
  .confirm span { flex-basis: 100%; }
  .nothing { padding: 2px 6px 6px; font-size: 11.5px; }

  .rows { flex-grow: 1; overflow-y: auto; padding: 0 8px 12px; }
  .head { display: flex; align-items: center; gap: 7px; padding: 10px 6px 5px; }
  .hlabel { font-size: 10.5px; letter-spacing: .08em; text-transform: uppercase;
            color: var(--text); font-weight: 600; }
  .hlabel.warn { color: var(--error); }
  .count { font-size: 10.5px; color: var(--muted); }

  .row { display: flex; align-items: center; gap: 9px; padding: 6px; border-radius: 6px; }
  .row:hover { background: color-mix(in srgb, var(--text) 3.5%, transparent); }
  /* A 2px tick of the list's colour: the same vocabulary the grid's blocks
     use for the calendar they belong to. */
  .tick { width: 2px; height: 13px; flex: 0 0 2px; border-radius: 1px; }
  .title { appearance: none; -webkit-appearance: none; font: inherit; text-align: left;
           border: 0; background: none; color: var(--text); flex-grow: 1; min-width: 0;
           padding: 0; cursor: pointer; overflow: hidden; text-overflow: ellipsis;
           white-space: nowrap; }
  .title:disabled { cursor: default; }
  /* A press on a title may become a carry, and a carry must not paint a
     selection across the pane on its way out. */
  .row:not(.done) .title { -webkit-user-select: none; user-select: none; }
  .carry { position: fixed; z-index: 90; pointer-events: none; display: flex; align-items: center;
           gap: 7px; max-width: 240px; padding: 5px 10px 5px 8px; border-radius: 6px;
           font-size: 12px; color: var(--text); white-space: nowrap; overflow: hidden;
           text-overflow: ellipsis; background: var(--surface); border: 1px solid var(--accent);
           box-shadow: 0 6px 20px rgba(0, 0, 0, .35); }
  .carry.nowhere { opacity: .55; border-color: var(--hairline); }
  /* The word carries the level; the tone is decoration. Sits between title and
     due, `flex: 0 0 auto` so it never pushes the due date off. */
  .pchip { flex: 0 0 auto; font-size: 10px; font-weight: 600; line-height: 1;
           padding: 1px 6px; border-radius: 999px; border: 1px solid currentColor;
           white-space: nowrap; }
  .pchip.high { color: var(--error); }
  .pchip.medium { color: var(--accent); }
  .pchip.low { color: var(--muted); }
  .due { font-size: 11px; color: var(--muted); font-variant-numeric: tabular-nums;
         white-space: nowrap; }
  .due.overdue { color: var(--error); }
  .del { font: inherit; color: var(--muted); background: none; border: 0; cursor: pointer;
         padding: 0 2px; visibility: hidden; }
  .row:hover .del { visibility: visible; }
  .del:hover { color: var(--text); }
  .row.done .title { color: var(--muted); text-decoration: line-through; }

  .editor { background: var(--surface); border-radius: 8px; padding: 9px;
            box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 40%, transparent);
            display: flex; flex-direction: column; gap: 8px; margin: 2px 0; }
  .etitle { font: inherit; font-weight: 500; color: var(--text); background: none; border: 0;
            border-bottom: 1px solid color-mix(in srgb, var(--accent) 45%, transparent);
            padding: 0 0 3px; }
  .elist { display: flex; min-width: 0; }
  .quick { display: flex; flex-wrap: wrap; gap: 4px; }
  .quick button { appearance: none; -webkit-appearance: none; font: inherit; font-size: 11px;
                  padding: 3px 8px; border-radius: 5px; border: 0; cursor: pointer;
                  background: color-mix(in srgb, var(--text) 5%, transparent); color: var(--text); }
  .quick button.clear { background: none; color: var(--muted); }
  .when { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 8px; }
  .enotes { font: inherit; font-size: 11px; resize: vertical; padding: 6px 8px; border-radius: 5px;
            border: 1px solid var(--hairline); background: var(--bg); color: var(--text); }
  /* The editor's one native select. The global `select` rule gives it the
     chevron and `appearance: none`, but no font, colour, border or radius —
     so without this it drew the platform's light widget beside the dark
     DateField/TimeField. `background-color`, not the `background` shorthand,
     which would clobber the global chevron. */
  .efield { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .elab { font-size: 9.5px; color: var(--muted); letter-spacing: .05em; }
  .efield select { font: inherit; font-size: 12.5px; color: var(--text);
                   background-color: color-mix(in srgb, var(--text) 5%, transparent);
                   border: 1px solid var(--hairline); border-radius: 5px;
                   padding: 4px 22px 4px 6px; }
  .efield select:focus { outline: 1px solid var(--accent); outline-offset: -1px; }
  .efield select:disabled { opacity: .5; cursor: default; }
  .eact { display: flex; justify-content: flex-end; gap: 6px; }
  .eact button { appearance: none; -webkit-appearance: none; font: inherit; font-size: 11.5px;
                 padding: 4px 11px; border-radius: 6px; cursor: pointer;
                 border: 1px solid var(--hairline); background: none; color: var(--text); }
  .eact .go { background: var(--accent); color: var(--on-accent); border-color: transparent; }
  .eact button:disabled, .quick button:disabled, .add input:disabled { opacity: .5; cursor: default; }

  .empty { color: var(--muted); padding: 10px 6px; line-height: 1.5; }

  /* "Add a task" sits where the next row would, its text in the titles'
     column, quiet until pointed at. */
  .addline { appearance: none; -webkit-appearance: none; display: block; font: inherit;
             font-size: 11.5px; color: var(--muted); background: none; border: 0; cursor: pointer;
             text-align: left; padding: 4px 6px 6px 40px; opacity: .8; }
  .addline:hover { color: var(--text); opacity: 1; }
  .newline { border-radius: 6px; margin: 1px 0 4px;
             background: color-mix(in srgb, var(--text) 3.5%, transparent); }
  /* An unticked box's outline that is not a control yet: nothing to tick. */
  .box { width: 14px; height: 14px; flex: none; box-sizing: border-box; border-radius: 3px;
         border: 1px dashed color-mix(in srgb, var(--text) 32%, transparent); }
  .ntitle { flex-grow: 1; min-width: 0; font: inherit; color: var(--text); background: none;
            border: 0; padding: 0; outline: none; }
  .ntitle::placeholder { color: var(--muted); opacity: .75; }
  .nwhen { display: inline-flex; align-items: center; justify-content: center; flex: none;
           color: var(--muted); background: none; border: 0; border-radius: 4px; cursor: pointer;
           padding: 2px 3px; }
  .nwhen:hover, .nwhen[aria-expanded='true'] { color: var(--text);
           background: color-mix(in srgb, var(--text) 8%, transparent); }
  .nwhen.set { color: var(--accent); }
  .nwhen:focus-visible { outline: 1px solid var(--accent); outline-offset: -1px; }
  .nwhen-fields { display: flex; flex-direction: column; gap: 8px; padding: 2px 8px 9px 40px; }
  .earlier-toggle { appearance: none; -webkit-appearance: none; font: inherit; font-size: 11px;
                    color: var(--muted); background: none; border: 0; cursor: pointer;
                    padding: 8px 6px 4px; text-align: left; }
  .earlier-toggle:hover { color: var(--text); }
  .earlier-search { font: inherit; font-size: 11.5px; width: 100%; box-sizing: border-box;
                    margin: 2px 0 4px; padding: 5px 8px; border-radius: 6px;
                    border: 1px solid var(--hairline); background: var(--surface); color: var(--text); }
</style>
