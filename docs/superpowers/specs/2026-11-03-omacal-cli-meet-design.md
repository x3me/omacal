# omacal — CLI: `--meet`, the form's Google Meet choice

*2026-11-03. Follows the CLI-writes design (2026-08-27) and the app's
structured conferencing (`EventInput.conference`, the Meet half of PR #13,
merged as 806906d). It adds no new behaviour to the app. It gives the CLI
a word the form already has.*

## Goal

A script or an agent can create an event with a Google Meet link, or add one
to an event that has none: `omacal events create … --meet`. The app already
does this from the form. `EventInput.conference = "googleMeet"` becomes
`conferenceData.createRequest` with a fresh `requestId` (`write.rs`), on
both create and patch (`events.rs`). The CLI could not ask for it, so for
agents the feature did not exist.

## Scope

In: a `--meet` flag on `events create` and `events update`, its prechecks,
the minted link in the output, the skill file and the usage text.

Out, deliberately: removing a conference from the CLI (`"conference":
"none"`), Zoom (#13), and an "always add Meet" default.

## 1. The surface

```
omacal events create --title T --date D --start HH:MM --end HH:MM … --meet
omacal events update ID --occurrence MS … --meet
```

A bare flag, like `--keep-meeting`. On update, `--meet` counts as a change
by itself, so `events update 41 --occurrence MS --meet` is a whole command.

## 2. The wire

`fields.conference = "googleMeet"` when the flag is given. **When it is not
given, the key is absent, never `"none"`.** On an update, absent means
"leave the conference alone" and `"none"` means remove it. A CLI that sent
`"none"` by default would strip the meeting link from every event it
retitled. The app's own reader (`ipc::parse_request`) is what the tests
check against, as for every other CLI request.

## 3. Refusals (exit 2, before the socket)

| Condition | Why |
|---|---|
| Calendar is not Google (`calendar_mails_guests` is false) | Only Google mints Meet. The app refuses the same thing behind the socket, so this check only gives a better message and saves a round trip. |
| `reach` = `own-copy` (update) | The organizer owns `conferenceData`. A guest's copy has no meeting of its own to set. |
| The event already has a meeting link (update) | Read the way `events list` prints `conference`: `conference_uri`, then a recognised URL in Location or Description. `--meet` there would put a second meeting next to the one the guests were sent. |

`--notify` is unchanged. A Meet link mails nobody by itself, but adding one
to an event with guests is an update Google sends them when `sendUpdates`
is `all`, which is why the notify rule still applies.

## 4. Output

`--json` relays the app's envelope untouched. `data.conference_uri` is the
new link (the saved event, as the app answers a write; `events show` calls
the same value `conference`). Text mode prints `Meet: URL` after `Created:`/`Saved:`, for `--meet`
writes only, so existing output does not change.

A save that comes back without a link (Workspace policy, or the conference
still pending) is still a save. The CLI warns on stderr and exits 0. An
error here would invite the retry that creates the event twice.

## 5. Testing

In `cli_write.rs`, each test seen to fail against a deliberately broken
build (`docs/testing-standard.md`):

- The flag parses bare on both verbs and counts as an update's change.
- `meet_rule` refuses non-Google, own-copy and already-linked, and passes
  Organizer and Shared.
- The target's meeting link is found in `conference_uri` and in Location.
- The requests carry `conference: "googleMeet"`, which `parse_request`
  reads as `ConferenceInput::GoogleMeet`. Without the flag there is no key
  at all.

Mutations run: sending `"none"` by default, dropping the update field,
skipping each of the three refusals, losing the Location fallback, and
leaving the flag unparsed or not counted as a change. Each one turned a
named test red.

Against Google (guestless events, the released 6.3.0 app, deleted after):
a create with `--meet` minted a link, and one without did not; an update
added a link, and a second `--meet` was refused; a weekly series of two got
one link shared by both occurrences; a Workspace account outside the first
one minted too. The first run also caught the CLI reading the link from
the wrong key (`conference`, not `conference_uri`), which is why that read
now has its own test built from the real answer.

## 6. Deferred

- `--no-meet` (removal): the wire supports it. It is left out because it
  removes the link from a shared meeting, and the form already offers it
  with the event in view.
