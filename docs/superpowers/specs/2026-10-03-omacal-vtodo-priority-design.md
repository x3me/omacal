# Priority on tasks

OmaCal already parses a VTODO's `PRIORITY`, keeps it in `tasks.priority`,
preserves it through an edit's line-surgery, and sorts open tasks by it — and
it can neither *say* a priority nor show one. This gives tasks the missing
half: a four-level choice in the editor, a marker on the row, a `--priority`
flag on the CLI, and a date-or-priority sort.

Full implementation plan: `.scratch/vtodo-priority/spec.md` (wayfinder map at
`.scratch/vtodo-priority/map.md`).

## 1. The four levels, and the wire

None / Low / Medium / High, mapped to the RFC 5545 §3.8.1.9 integer:

| Level | `PRIORITY` |
|---|---|
| None | omitted |
| Low | `9` |
| Medium | `5` |
| High | `1` |

Read maps the RFC bands the way iCloud, Thunderbird and Tasks.org do —
absent/`0` → none, `1–4` → high, `5` → medium, `6–9` → low. **None is the
property omitted, never `PRIORITY:0`**: the two are equivalent to the spec
(`0` ≡ absent), but writing `0` churns against the clients that omit it.
CalDAV adds nothing to the rule; servers may still normalize, so write only
`1/5/9` and touch the line only when it changes.

## 2. Preserve what the user did not touch

The `PRIORITY` value is written as the raw stored integer, not the level, so
a server's non-canonical `7` passes through an unrelated edit untouched. The
control offers only the four levels; choosing one writes `1/5/9`, clearing
omits. The editor seeds its working value from the stored integer so an
untouched save re-emits identical bytes (no ETag churn).

## 3. What the user sees

- **Editor**: a native `Priority` select under the due fields — None / Low /
  Medium / High. Reuses the app's global select styling; no new control.
- **Row**: a small word-chip ("High"/"Medium"/"Low") only when set, toned by
  level — the word carries the meaning, the tone is decoration, so it reads
  in greyscale. It costs a little row width; long titles ellipsise.
- **Sort**: a persisted `task_sort` preference switches the pane's within-group
  order between **date** (due, then priority, then title — today's rule) and
  **priority** (priority with none last, then due, then title). Grouping is
  unchanged.

## 4. The CLI

`omacal tasks add|edit … [--priority none|low|medium|high]`, words not
numbers (a bare `1` reads as "low" to most people, but the RFC means highest).
Absent leaves the value alone on `edit`; `--priority none` clears. `tasks
--json` and the human row carry it. Adding the flag completes #74's "not
half-offered" promise and updates `skills/omacal/SKILL.md` in the same commit.

## 5. Where it is representable

CalDAV and iCloud (VTODO lists) and on-device lists can carry it; Google
cannot — the Tasks API and Calendar events have no priority field, and OmaCal
does not integrate Google Tasks. The control only appears on writable task
lists.

## 6. Where each piece lives

- `crates/omacal-caldav/src/ics.rs`: a `TaskPriority` enum (the authored
  levels); `TodoEdit.priority: Option<i64>` (`None` clears);
  `patch_todo_fields` strips and re-emits the line; `new_todo_ics` takes one.
- `crates/omacal-store/src/tasks.rs`: `update_task_fields` binds priority —
  the column already exists, so **no migration**.
- `src-tauri/src/tasks.rs` + `ipc.rs`: the command and socket carry it; the
  socket field is optional so an older peer degrades to "leave unchanged".
- `src-tauri/src/cli_tasks.rs` + `cli.rs`: the flag, the request, the catalog,
  and the two read shapes.
- `ui/src/lib/tasks.ts` + `TasksSidebar.svelte`: the select, the chip, a pure
  `sortTasks`, and the `task_sort` preference.
