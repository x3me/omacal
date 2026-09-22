import { invoke } from '@tauri-apps/api/core';

export type Calendar = {
  id: number;
  account_id: number;
  account_email: string;
  summary: string;
  provider_summary?: string;
  label_override?: string | null;
  /** **The colour to draw this calendar in** — its override if it has one, and
   *  Google's own otherwise, resolved in SQL. Anything that only wants to draw
   *  reads this and needs to know nothing about overrides. */
  color_hex: string | null;
  /** The override itself, or `null` when there is none. Distinct from the
   *  field above because *clearing* an override is a different state from
   *  setting it to whatever Google currently uses — only this can tell them
   *  apart, and the swatch row needs to. */
  color_override: string | null;
  /** Drawn in the grid. */
  selected: boolean;
  /** Fetched from Google at all. */
  sync_enabled: boolean;
  is_primary: boolean;
  /** Google's own word for what this account may do here: `owner`, `writer`,
   *  `reader`, `freeBusyReader`. Only the first two can be written to — see
   *  `writableCalendars` below, the one place that decides it. */
  access_role: string;
  /** The owning account's provider: `google` | `caldav` | `webcal` | `local`.
   *  Only Google mails guests (CalDAV has no guest management and no notify
   *  question; WebCal and local never mail) — `EventForm` gates on this field,
   *  and WebCal/local calendars are `reader`-gated out of writes entirely. */
  provider: string;
};

/** The two roles a create can land on. Google's `calendarList` reports two more
 *  — `reader` and `freeBusyReader` — and a subscribed holiday calendar is a
 *  `reader`, so a list that is not filtered offers Save buttons the backend can
 *  only refuse (`create_impl`'s own `can_edit` check, against the same column). */
export const writableCalendars = (cals: Calendar[]) =>
  cals.filter((c) => c.access_role === 'owner' || c.access_role === 'writer');

/**
 * `wanted` if a create could land on it, otherwise the first calendar one
 * could — or `null` when there is no such calendar at all.
 *
 * `writableCalendars` filters the *options*; this filters the *value*, and
 * both are needed. A form seeded with a `reader`'s id and only the option list
 * filtered renders a **blank** select — the browser has no option matching the
 * value — and then saves that id anyway, with nothing on screen to say so. The
 * backend refuses it (`create_impl`'s `can_edit` check), so nothing is
 * corrupted; it is exactly the "Save that can only fail" the filter exists to
 * prevent, and the seed is chosen by the caller rather than by the user.
 *
 * Falling back rather than erroring, because the value is not something the
 * user picked: the select then shows the calendar that will actually be
 * written to, and they can change it. What is shown and what is saved agree,
 * which is the property that was broken.
 */
export function offerableCalendarId(wanted: number | null, cals: Calendar[]): number | null {
  const offers = writableCalendars(cals);
  return offers.some((c) => c.id === wanted) ? wanted : (offers[0]?.id ?? null);
}

/**
 * The colour to draw something belonging to calendar `id` in, or `null` when
 * there is no such calendar or it has no colour — a caller then falls back to
 * `--accent`, the same way `CalendarPicker`'s swatch does.
 *
 * `color_hex` and not `color_override`: this answers "what colour is this
 * calendar", which is the override *or* Google's own. See the field's comment.
 */
export function calendarColor(id: number | null, cals: Calendar[]): string | null {
  return cals.find((c) => c.id === id)?.color_hex ?? null;
}

export const getCalendars = () => invoke<Calendar[]>('get_calendars');
export const setCalendarSelected = (id: number, on: boolean) =>
  invoke<void>('set_calendar_selected', { id, on });
/** Resolves to the number of local events the removal deleted. */
export const setCalendarSync = (id: number, on: boolean) =>
  invoke<number>('set_calendar_sync', { id, on });

/**
 * Sets or clears a calendar's colour, **locally and only locally**.
 *
 * `null` clears the override, and the calendar follows Google's colour again —
 * including when Google changes it. Nothing here is sent to Google: the user's
 * phone, the web UI and anyone else subscribed to the same calendar are
 * untouched.
 */
export const setCalendarColor = (id: number, hex: string | null) =>
  invoke<void>('set_calendar_color', { id, hex });

/** Account identity is independent of its email: Google and CalDAV (or two
 * CalDAV connections) may share an address. Keep first-seen account order. */
export function byAccount(cals: Calendar[]): Array<{ id: number; label: string; calendars: Calendar[] }> {
  const groups = new Map<number, { id: number; label: string; calendars: Calendar[] }>();
  for (const c of cals) {
    const g = groups.get(c.account_id);
    if (g) g.calendars.push(c);
    else {
      // The on-this-device lists belong to no server, so there is no address
      // to name beside the provider — the label is the whole answer.
      if (c.provider === 'local') {
        groups.set(c.account_id, { id: c.account_id, label: 'On this device', calendars: [c] });
        continue;
      }
      const provider =
        c.provider === 'google'
          ? 'Google'
          : c.provider === 'caldav'
            ? 'CalDAV'
            : c.provider === 'webcal'
              ? 'WebCal'
              : c.provider;
      groups.set(c.account_id, { id: c.account_id,
        label: `${provider} · ${c.account_email}`, calendars: [c] });
    }
  }
  return [...groups.values()];
}

export const setCalendarLabel = (id: number, label: string | null): Promise<void> =>
  invoke<void>('set_calendar_label', { id, label });
