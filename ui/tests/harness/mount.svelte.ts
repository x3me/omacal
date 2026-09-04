import '../../src/app.css';
import { mount, unmount } from 'svelte';
import WeekGrid from '../../src/lib/WeekGrid.svelte';
import MonthGrid from '../../src/lib/MonthGrid.svelte';
import YearGrid from '../../src/lib/YearGrid.svelte';
import BigYearRibbon from '../../src/lib/BigYearRibbon.svelte';
import Filmstrip from '../../src/lib/Filmstrip.svelte';
import EventBlock from '../../src/lib/EventBlock.svelte';
import AllDayBand from '../../src/lib/AllDayBand.svelte';
import Header from '../../src/lib/Header.svelte';
import CalendarPopover from '../../src/lib/CalendarPopover.svelte';
import EventPopover from '../../src/lib/EventPopover.svelte';
import EventForm from '../../src/lib/EventForm.svelte';
import DeleteConfirm from '../../src/lib/DeleteConfirm.svelte';
import * as eventform from '../../src/lib/eventform';
import * as drag from '../../src/lib/drag';
import { FIXTURES } from '../fixtures';
import { installTauriStub } from './tauri';
import { setPalette } from '../../src/lib/theme';
import { setSecondZone } from '../../src/lib/secondzone.svelte';
import { setTemperatureUnit } from '../../src/lib/tempunit.svelte';
import { VIEW_BOX_CSS } from './viewbox';

// The form's pure date functions, reachable from a spec.
//
// `eventform.spec.ts` exercises these directly in the Node process, which is
// right for everything that does not depend on a zone. The two characterisation
// specs at the end of that file *are* about zone transitions, and Playwright's
// `timezoneId` reaches the browser context, not Node — so those run in the page
// and reach the module through here. Assigned before the branching below, so a
// spec can ask for a URL that mounts nothing at all.
(window as any).__eventform = eventform;

// The drag geometry, reachable from `drag.spec.ts` for the same reason: every
// answer it gives depends on the browser's zone, and `timezoneId` reaches the
// browser context rather than Node.
(window as any).__drag = drag;

// The second-zone rune's writer, reachable from a spec. In the app the only
// writer is `App`, seeded from settings; a component mounted standalone has
// no App above it, so a spec that wants the second clock drawn plays App's
// one line itself.
(window as any).__setSecondZone = setSecondZone;

// The temperature-unit rune's writer, reachable from a spec, for
// `__setSecondZone`'s exact reason: in the app `App` is the only writer, and
// a component mounted standalone has none above it.
(window as any).__setTemperatureUnit = setTemperatureUnit;

// Palette normally arrives from the Rust get_palette command; the harness
// applies the same `fallback_dark` values (`src-tauri/src/theme.rs`) so
// snapshots are deterministic.
//
// Pushed through `setPalette` rather than written out as a map of variables.
// That map used to carry its own copies of the three `is_dark`-derived
// variables, which made this the second place that knew the names — and
// `theme.ts` says in its own doc comment that it is "the one place". A
// variable added there but forgotten here does not fail: it resolves to
// nothing, and a component reading it renders in whatever the initial value
// is, quietly, in every spec at once.
const FALLBACK_DARK = {
  bg: '#17171a', surface: '#1e1e22', text: '#e8e8ea',
  muted: '#8a8a90', accent: '#5b8def', is_dark: true,
};
setPalette(FALLBACK_DARK);
document.body.style.background = FALLBACK_DARK.bg;
document.body.style.color = FALLBACK_DARK.text;

const params = new URLSearchParams(location.search);
const name = params.get('c') ?? 'WeekGrid';
const fixture = params.get('f') ?? 'default';

const COMPONENTS: Record<string, any> = {
  WeekGrid, MonthGrid, YearGrid, BigYearRibbon, Filmstrip, EventBlock, AllDayBand, Header,
  CalendarPopover, EventPopover, EventForm, DeleteConfirm,
};
const target = document.getElementById('app')!;

/**
 * The four views that size themselves off their parent rather than off the
 * window, and so cannot be mounted into a bare `<div>`.
 *
 * Each is a `flex: 1` item now (see any one of their stylesheets). In the app
 * that resolves against `App`'s `main`; here there is no `main`, so this
 * reproduces the one thing about it those views actually read — a column flex
 * container of the height `App` leaves them. Without it `flex: 1` is inert,
 * every view falls back to its own content height, and the specs that measure
 * one go quiet rather than red: `BigYearRibbon`'s "fourteen rows fit with no
 * scroll" would be comparing a container against the contents that *defined*
 * its height, and could never fail.
 *
 * Height only. Nothing sets a width, a padding or an offset, so the views keep
 * the full-bleed 1280px they have always been measured and screenshotted at.
 */
const FILL_HEIGHT_VIEWS = new Set([
  'WeekGrid', 'MonthGrid', 'YearGrid', 'BigYearRibbon', 'Filmstrip',
]);
if (FILL_HEIGHT_VIEWS.has(name)) {
  target.style.display = 'flex';
  target.style.flexDirection = 'column';
  target.style.height = VIEW_BOX_CSS;
}

if (name === 'App') {
  // App is the whole application, not a leaf: it takes no props, and every
  // input it has arrives over the Tauri IPC. So the stub goes in first, and
  // the component is imported only afterwards — nothing may call `invoke`
  // before `window.__TAURI_INTERNALS__` exists.
  installTauriStub(fixture);
  const { default: App } = await import('../../src/App.svelte');
  mount(App, { target });
} else {
  let props = FIXTURES[name]?.[fixture];
  if (!props) {
    target.textContent = `no fixture ${name}/${fixture}`;
  } else {
    // The fixture's own props, reachable from a spec.
    //
    // So that a spec can assert **its fixture's premise** — that two instants
    // really are 169 hours apart, that this browser really does read a stored
    // midnight as the neighbouring day — against the one copy of those numbers
    // rather than a second one typed into the spec beside it. Both halves of
    // that matter: a fixture that quietly stopped straddling a transition would
    // otherwise leave its spec passing vacuously, and a spec carrying its own
    // copy of the instants can be right about a fixture that has changed under
    // it. Read-only by convention; nothing here mutates it.
    (window as any).__fixtureProps = props;
    if (name === 'Header') {
      // `oncalendarchange` is a callback prop, not a Tauri command: the header
      // tells `App` to reload rather than reloading anything itself. Captured
      // on the window exactly as `WeekGrid`'s four are, so a spec can assert
      // the app was told — which is a different fact from the command having
      // been sent, and the one a second host for these rows can silently drop.
      (window as any).__calendarChanges = 0;
      // Same capture for the invitation tray's callback, same reasoning: the
      // command going out (`respond_to_event`, in `harness.calls`) and `App`
      // being told to refetch are two separate facts.
      (window as any).__inviteAnswers = 0;
      props = {
        ...props,
        oncalendarchange: () => { (window as any).__calendarChanges += 1; },
        oninvitesanswered: () => { (window as any).__inviteAnswers += 1; },
      };
    }
    // EventBlock is absolutely positioned; give it a sized relative parent.
    if (name === 'EventBlock') {
      target.style.position = 'relative';
      target.style.height = '480px';
      target.style.width = '220px';
    }
    // Unlike its neighbours here, CalendarPopover calls `invoke` itself —
    // ticking a box or clicking Add/Remove goes straight to the three
    // calendar commands. It still takes its `calendars` from a fixture
    // prop rather than `get_calendars`, so the scenario name only matters
    // for the write commands the stub answers.
    if (name === 'CalendarPopover') installTauriStub(fixture);
    // The settings modal opens from this header and reads its preferences over
    // the IPC, so `Header` needs the stub for the same reason `CalendarPopover`
    // does — installed unconditionally rather than gated on a fixture name,
    // since it is opening the modal and not the fixture that decides whether
    // anything reaches `invoke`.
    if (name === 'Header') installTauriStub(fixture);
    if (name === 'WeekGrid') {
      // Clicking one of its blocks opens a real `EventPopover` that calls
      // `event_detail`/`refresh_event`/`respond_to_event` itself — installed
      // unconditionally (harmless for the specs that never click anything)
      // rather than gated on the fixture name, since it is the *click*, not
      // the fixture, that decides whether anything actually reaches `invoke`.
      installTauriStub(fixture);
      // `week` is a `$state` here — this file's `.svelte.ts` extension is
      // what makes that legal outside a component — rather than the plain
      // value every other fixture prop is. A spec proving the override-
      // eviction fix needs to simulate what `App.svelte`'s `loadWeek` does
      // after a real sync: replace `week` with a freshly fetched payload,
      // out from under an `EventPopover`/block that's already been
      // interacted with. `mount()`'s props are read reactively when given
      // as a getter, so this is enough to make that live from outside.
      let week = $state(props.week);
      (window as any).__setWeek = (w: unknown) => { week = w; };
      // `oncreate`/`onedit`/`ondelete` are callback props, not Tauri commands
      // — the grid decides *which occurrence* a write is about and hands that
      // up, rather than writing anything itself. Captured on the window the
      // same way `MonthGrid`'s `onopen` is, so a spec can read exactly what it
      // was handed.
      (window as any).__lastCreate = null;
      (window as any).__lastAllDayCreate = null;
      // How many forms have been asked for. `__lastCreate` alone cannot see a
      // *second* create arriving right behind the first, which is exactly what
      // a sweep produces if the click the browser dispatches after its
      // `pointerup` is not swallowed — the user would see the last one, on the
      // half hour the release landed in, rather than the span they swept.
      (window as any).__createCount = 0;
      (window as any).__lastEdit = null;
      (window as any).__lastDelete = null;
      (window as any).__lastMove = null;
      (window as any).__lastCopy = null;
      // The sideways swipe hands whole days up the same way (`onpan`); the
      // grid itself never moves a window.
      (window as any).__lastPan = null;
      // Spread first, then shadow `week` with the live getter — naming only
      // `week` here would silently drop any second prop a fixture grows.
      mount(WeekGrid, {
        target,
        props: {
          onpan: (days: unknown) => { (window as any).__lastPan = days; },
          // A click the grid could not answer — the detail behind it would
          // not load. Captured like the rest: the grid says it, App shows it.
          onerror: (message: unknown) => { (window as any).__lastGridError = message; },
          ...props,
          get week() { return week; },
          // `endMs` is present only when the grid names one — a sweep does, a
          // click does not, and the difference is what leaves the duration of
          // a clicked create to the form's own default.
          oncreate: (startMs: unknown, rect: unknown, endMs?: unknown) => {
            (window as any).__lastCreate = { startMs, rect, endMs };
            (window as any).__createCount += 1;
          },
          // A sweep that crossed columns: days, not instants.
          oncreateallday: (firstDayMs: unknown, lastDayMs: unknown, rect: unknown) => {
            (window as any).__lastAllDayCreate = { firstDayMs, lastDayMs, rect };
            (window as any).__createCount += 1;
          },
          onedit: (occurrence: unknown, rect: unknown) => {
            (window as any).__lastEdit = { occurrence, rect };
          },
          ondelete: (occurrence: unknown, rect: unknown) => {
            (window as any).__lastDelete = { occurrence, rect };
          },
          // Ctrl+C in the popover. No rect — a copy points at nothing.
          oncopy: (occurrence: unknown) => {
            (window as any).__lastCopy = { occurrence };
          },
          // A completed drag, for the same reason as the three above: the grid
          // hands the write up rather than making it, so a spec reads exactly
          // what it was handed.
          onmove: (event: unknown, span: unknown) => {
            (window as any).__lastMove = { event, span };
          },
        },
      });
    } else if (name === 'MonthGrid') {
      // `onopen`/`ondaypick` are callback props, not Tauri commands, so they
      // have no home in `tauri.ts`'s `invoke` switch — same reasoning as
      // `__lastRespondCall` there, just one layer up: capture exactly what
      // the component handed back, on the window, for a spec to read.
      mount(MonthGrid, {
        target,
        props: {
          ...props,
          onopen: (event: unknown, rect: unknown) => {
            (window as any).__lastOpen = { event, rect };
          },
          ondaypick: (startMs: unknown) => {
            (window as any).__lastDayPick = startMs;
          },
          oncreate: (dayStartMs: unknown, rect: unknown) => {
            (window as any).__lastCreate = { startMs: dayStartMs, rect };
          },
        },
      });
    } else if (name === 'YearGrid') {
      // `ondaypick` follows the exact contract `MonthGrid` already
      // established — same `__lastDayPick` capture, reused rather than
      // inventing a second mechanism.
      mount(YearGrid, {
        target,
        props: {
          ...props,
          ondaypick: (startMs: unknown) => {
            (window as any).__lastDayPick = startMs;
          },
        },
      });
    } else if (name === 'BigYearRibbon') {
      // Same capture as `MonthGrid`'s `onopen` above: a callback prop, not a
      // Tauri command, so it has no home in `tauri.ts`'s `invoke` switch.
      mount(BigYearRibbon, {
        target,
        props: {
          ...props,
          onopen: (event: unknown, rect: unknown) => {
            (window as any).__lastOpen = { event, rect };
          },
          oncreate: (dayStartMs: unknown, rect: unknown) => {
            (window as any).__lastCreate = { startMs: dayStartMs, rect };
          },
          // The legend's toggle is a callback prop too: the ribbon names the
          // calendar and App does the writing, so a standalone mount can only
          // assert which calendar was handed up.
          ontoggle: (calendar: unknown) => {
            (window as any).__lastToggle = calendar;
          },
        },
      });
    } else if (name === 'Filmstrip') {
      // `onopen` is a callback prop, not a Tauri command — a row hands the
      // clicked occurrence up rather than opening anything itself, which is
      // spec §6's "no second way to reach an event's detail". Captured on the
      // window exactly as `MonthGrid`'s and `BigYearRibbon`'s are.
      //
      // The stub too, unlike its neighbours: a row's Join chip calls
      // `open_conference` itself — the one command this component owns — and
      // without the stub that click is an unhandled rejection instead of a
      // fact in `harness.calls`.
      installTauriStub(fixture);
      mount(Filmstrip, {
        target,
        props: {
          ...props,
          onopen: (event: unknown, rect: unknown) => {
            (window as any).__lastOpen = { event, rect };
          },
        },
      });
    } else if (name === 'DeleteConfirm') {
      // `onconfirm`/`oncancel` are callback props: this panel writes nothing
      // itself, since the caller owns both the command and the reload after
      // it. `__confirms` is an array rather than a slot for the reason
      // `EventForm`'s `__saves` is — half of what a spec asserts is that
      // nothing was confirmed at all, and one slot cannot tell "never called"
      // from "called and overwritten".
      (window as any).__confirms = [];
      let app: object;
      app = mount(DeleteConfirm, {
        target,
        props: {
          ...props,
          onconfirm: (scope: unknown) => {
            (window as any).__confirms.push(scope);
          },
          // Same reasoning as `EventPopover`/`EventForm` below: in the real app
          // the parent owns whether this panel exists, so cancelling has to
          // actually remove it here or "Escape closes it" observes nothing.
          oncancel: () => unmount(app),
        },
      });
    } else if (name === 'EventPopover') {
      // EventPopover calls `invoke` itself too (`respond_to_event`), same
      // reasoning as CalendarPopover above.
      installTauriStub(fixture);
      // In the real app, WeekGrid owns whether EventPopover exists at all —
      // `onclose` sets its `detail` to null, which unmounts the popover via
      // `{#if}`. Mounted standalone here, nothing else plays that role, so
      // `onclose` has to actually remove the component itself or "Escape
      // closes it" would have nothing to observe: the fixture's own
      // `onclose` is a no-op, since it can't know about this harness.
      (window as any).__lastCopy = null;
      (window as any).__lastEdit = null;
      (window as any).__lastDelete = null;
      let app: object;
      app = mount(EventPopover, {
        target,
        props: {
          ...props,
          onclose: () => unmount(app),
          onedit: () => {
            (window as any).__lastEdit = { edited: true };
          },
          ondelete: () => {
            (window as any).__lastDelete = { deleted: true };
          },
          oncopy: () => {
            (window as any).__lastCopy = { copied: true };
          },
        },
      });
    } else if (name === 'EventForm') {
      // `onsave`/`oncancel` are callback props, not Tauri commands — which
      // write command a save becomes (create or update, and at which scope)
      // is the caller's decision, not the form's. The form does make one
      // read of its own now — `known_guests`, the autocomplete corpus — so
      // the stub is installed. Captured on the window exactly as
      // `MonthGrid`'s `onopen` is.
      //
      // `__saves` is an array rather than a single value: half of what these
      // specs assert is that a *refused* save called nothing at all, and a
      // single slot cannot tell "never called" from "called and overwritten".
      (window as any).__saves = [];
      // How many times the parent has been told to close. **Counted, not
      // flagged**, and that is the whole of what makes an unmount leak
      // observable: a cancel that arrives *after* the component was destroyed
      // is a second call, and a boolean cannot tell one from two. See
      // `dismiss.svelte.ts` — the listener lives on `window`, so a missing
      // teardown leaves it firing into a closure whose component is gone.
      (window as any).__cancels = 0;
      installTauriStub(fixture);
      let app: object;
      app = mount(EventForm, {
        target,
        props: {
          ...props,
          onsave: (result: unknown) => {
            (window as any).__saves.push(result);
          },
          // Same reasoning as `EventPopover` above: in the real app the parent
          // owns whether the form exists at all, so cancelling has to actually
          // remove it here or "Escape closes it" would have nothing to observe.
          oncancel: () => {
            (window as any).__cancels += 1;
            unmount(app);
          },
        },
      });
    } else {
      mount(COMPONENTS[name], { target, props });
    }
  }
}
