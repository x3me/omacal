---
name: omacal
description: The user's real calendar (Google, iCloud, CalDAV, WebCal subscriptions) through the omacal CLI — today's agenda, events in a date range, title search, the calendar list, and (v0.7+) writes: create (one-off or repeating), reschedule, answer and delete events. Use whenever the user asks what is on their calendar, when they are free or busy, or asks to add, move, cancel or answer a meeting from the terminal.
---

# omacal calendar

omacal is the desktop calendar app; its CLI reads the same local database
the app syncs, so answers reflect every connected account (Google, iCloud,
CalDAV, WebCal subscriptions) with no network round trip and no extra auth. Writes are executed
by the **running app** over a local socket, behind the same guards its own
form has — the CLI itself never writes the database.

## Reading

```bash
omacal agenda --json                 # next 7 days
omacal agenda --days 1 --json        # today
omacal events list --from 2026-09-01 --to 2026-09-05 --json
omacal events show 41 --json         # ONE event whole (v0.7.4+): the guest
                                     # list with each person's answer,
                                     # organizer, join link, description
omacal search quarterly review --json
omacal calendars --json              # every calendar with ids
omacal weather --json                # the app's forecast (v2.2+): place +
                                     # how it was decided, now, eight days
omacal tasks --json                  # what still needs doing (v2.3+), with
                                     # due dates, ids and which list
omacal tasks --all --json            # including recently completed
omacal tasks lists --json            # the lists a task can go on (v4.5+):
                                     # id, name, onThisDevice, open count
omacal commands --json               # machine-readable catalog of every
                                     # command and flag (v0.8.1+) — check
                                     # here before assuming a flag exists
omacal cli-help                      # full usage and exit codes
```

Always pass `--json` when consuming programmatically. Success:
`{"ok":true,"data":[...]}`. Failure: `{"ok":false,"error":{"code","message"}}`.

"Who accepted / who's coming / is X invited?" → `events show ID` is the
answer (list rows carry only a guest *count*). Its `guests` array gives
each person's `email`, `response` (same vocabulary as below), `optional`
and `isSelf` — never guess attendance from the count again.

Each event row: `eventId`, `title`, `startMs`/`endMs` (epoch ms),
`start`/`end` (RFC 3339 in the user's display zone), `allDay`, `location`,
`calendar`, `calendarId`, `attendees` (count, organizer included; 0 = solo),
`recurring`, `response` (the user's own RSVP), `organizer` (v0.7.3+: true =
this is the user's own event — trust it over any inference), `conference`
(join URL when the meeting has one). `events show` adds `reach` (v0.21+):
`organizer` (the user's event — a change goes to everyone), `own-copy`
(the user is a guest; Google keeps their copy apart, so a change moves it
on THEIR calendar alone, nobody else's copy changes and nobody is told)
or `shared` (a guest, but the organizer lets guests change it for
everyone), plus `guestsCanModify`. **Read `reach` before promising the
user that a reschedule will move the meeting for the other people** — on
`own-copy` it will not; tell them to ask the organizer instead.
`mailsGuests` (v3.7.2+) is false on anything but a Google calendar (CalDAV,
WebCal, local): OmaCal emails nobody there, so there is no notify question to ask the user, and `reach`
is Google's model and says nothing reliable about who else sees a change. WebCal feeds are read-only
subscriptions (Settings → Accounts → "Add WebCal account"); they never take writes, RSVPs, or tasks.

## Writing (requires the app to be running; omacal v0.7+)

```bash
omacal tasks add "Renew the domain" --due 2026-09-11 --json
omacal tasks add "Call the bank" --due 2026-09-11 --at 10:00 --list 3 --json
omacal tasks add "Milk" --list Groceries --json   # a list by name (v4.5+)
omacal tasks done 41 --json          # and `reopen 41` to put it back
omacal tasks edit 41 --due 2026-09-14 --json
omacal tasks edit 41 --due none --json     # clears the date; `--at none` keeps
                                           # the day and drops the hour
```


```bash
omacal events create --title "Standup" --date 2026-09-01 \
       --start 09:00 --end 09:30 --json
omacal events create --title "Trip" --date 2026-09-01 --all-day \
       --last-day 2026-09-03 --json          # last day INCLUSIVE
omacal events create --title "Gym" --date 2026-09-14 --start 20:00 --end 21:30 \
       --repeat weekly --days MO,TU,TH,FR --json    # one series, four days a week
omacal events create --title "Standup" --date 2026-09-14 --start 09:00 --end 09:15 \
       --repeat weekdays --until 2026-12-19 --json
omacal events update 41 --occurrence 1786352400000 --start 10:00 --end 10:30 --json
omacal events delete 41 --occurrence 1786352400000 --json
omacal events respond 41 yes --json          # yes | maybe | no
```

- `41` and the `--occurrence` value are `events list --json`'s own
  `eventId` and `startMs` — always read before you write.
- **A repeating event requires `--scope this|following|all`** — the CLI
  refuses to guess which occurrences you mean. Ask the user if unclear.
- **An event with guests requires `--notify all|none`** — whether the
  guests get emailed about the change is the user's call, never yours.
  Ask the user rather than defaulting. **Two exceptions, both refused
  with the reason (exit 2) if you pass `--notify all`:** `reach` =
   `own-copy` (`events show`), where an update moves the user's own copy
   alone and nobody can be notified; and **`mailsGuests` = false (anything but Google)**,
   where OmaCal emails nobody at all. Neither needs `--notify`.
 - `delete` takes no `--notify`. On Google it tells the guests, or for an
   own-copy event it removes the event from the user's calendar only and
   Google tells the organizer they declined. Off Google OmaCal emails nobody.
 - `--guest a@b` repeats for multiple guests on create. Creating with
   guests also requires `--notify`, except off Google.
- **`--repeat daily|weekdays|weekly|monthly|yearly` makes one series instead
  of many events.** Reach for it whenever the user describes a routine —
  "every Tuesday", "each weekday" — because a series is edited and deleted
  once, and fifty copies are not. `--days MO,WE,FR` refines `weekly` alone
  (`weekdays` already means Monday to Friday); **start the event on one of
  the days it names**, since the start date is itself an occurrence.
  `--until YYYY-MM-DD` and `--count N` are the two endings — pass one, or
  neither for an unbounded series. Without `--repeat` a create makes a
  single event, as before.
- **Changing an existing event's repeat is not in the CLI** — `events
  update` moves and retitles occurrences, it does not turn a single event
  into a series or edit a cadence. Send the user to the app's form.
- Times read in the user's display zone, `HH:MM`, strict.
- All-day events cannot be *edited* from the CLI yet (create/delete/respond
  work); send the user to the app for that.

## Exit codes

`0` ok · `2` usage error · `3` no database (omacal has never run /
no account connected — tell the user to launch omacal) · `4` internal
error · `5` **omacal is not running** — writes need the app; tell the
user to launch it, never try to launch it yourself · `6` **the app
refused the write** (a conflict, a guard). On 6, read the message and
change the request — retrying unchanged does nothing; on a timeout the
write's fate is UNKNOWN: check with `events list` before any retry, or
you may create a duplicate.

## Presenting results

When showing the calendar to the user (not piping into a script):

- Prefer a compact list over a table, one line per event, grouped under a
  bold day header ("**Wed, Aug 27**") — the CLI's own human layout. Tables
  only for genuine cross-day comparisons.
- Line shape: `10:30–10:55  Travel to Excitel office` — start–end, then
  title, then only the details that earn their place: a location, a join
  link (as a bare URL so it's clickable), "N guests" when more than the
  user. Omit "accepted" — the user's own yes is not news to them; do call
  out an *unanswered* invitation or a tentative.
- Reading the `response` field: `"needsAction"` is a real unanswered
  invitation — call it out. `"tentative"` is worth a mention. `"accepted"`
  is not news. **`null` is not "unanswered"** — it means no RSVP applies
  to the user at all, typically their own event or one without guests;
  never flag it. (Field lesson, 2026-08-27: an agent read null as
  unanswered and told an organizer to RSVP to their own meeting.) Since
  v0.7.3 the row also says it directly: `organizer: true` means it is the
  user's own event — no RSVP talk applies, ever.
- All-day events go on one quiet line after the timed ones, never mixed in.
- Name the timezone once, in the intro sentence, not per line.
- Lead with what the user asked ("Next free slot is…", "Four things
  today…"); the list is evidence, not the answer.

## Rules

- Reads are safe always; writes only through the commands above — never
  touch the database file directly.
- Before any destructive write (delete, or update/respond on something
  ambiguous), confirm the exact event with the user by title and time.
- Recurring events arrive already expanded: one row per occurrence in the
  window. Days with nothing simply produce no rows.
- The CLI shows exactly what the user's app shows: hidden calendars and
  declined events are absent. If something seems missing, `omacal
  calendars --json` shows what is hidden.
- Times are in the user's display zone; trust `start`/`end` for prose and
  `startMs`/`endMs` for arithmetic.
- Tasks are VTODOs on an iCloud or CalDAV list, or on lists kept on this
  machine (v4.1+: Tasks pane → "Create a list on this device", since Google
  keeps tasks in another product; v4.5+: as many as wanted, named, through
  "New list" in the pane). `omacal tasks` printing nothing can mean
  an account with no task list at all rather than a clear plate — say which
  before congratulating anybody. `due` is a bare date when the task has no
  hour and an instant when it has one — do not invent an hour for one that
  has none. `overdue` is already computed; say a task is late rather than
  working it out from the date. Adding needs a list: omit `--list` and it
  lands on the first one. `--list` takes an id or an exact name from
  `omacal tasks lists` — the only place an empty list shows up, since
  `omacal tasks` names lists beside their tasks. A name matching no list,
  or two, is refused; ask the user rather than guessing another list.
- Weather can be stale: `fetched_at` is when the app last reached the
  forecast, and it keeps the last good answer when offline. Check it before
  answering — past about six hours say so ("the forecast is from yesterday
  morning"), and never present an old reading as current.
- Weather is for `place`, which may not be where the user is: `source`
  `"detected"` (or absent) means it was guessed from the connection's IP
  and can be a city off; `"country"` means the IP told only the country, so
  `place` is a country and the forecast is for somewhere near its middle —
  say so, and do not present it as the user's local weather (they can set
  a city in Settings → Appearance → Weather location); `"settings"` means
  they set it in OmaCal (v4.5+); `"configured"` means they set it in the
  Omarchy bar. Name the place when answering about weather ("In Gurugram, Thursday
  looks like rain"). Temperatures are Celsius, unrounded; wind is km/h.
