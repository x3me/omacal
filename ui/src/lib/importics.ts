import { invoke } from '@tauri-apps/api/core';

/** What an import will do with one event, as `import::Planned` serialises. */
export type Planned =
  | {
      kind: 'import';
      summary: string;
      start_ms: number;
      all_day: boolean;
      /** The repeat in the preview's words, end included ("weekly on Mon,
       *  Wed, 10 times"), or `null` for a single event. */
      repeat: string | null;
      /** How many guests the file listed and the import will not carry. */
      dropped_guests: number;
    }
  | { kind: 'skip'; summary: string; reason: string };

/** What it did, once it had done it. */
export type ImportReport = {
  imported: number;
  skipped: Planned[];
  /** Attempted and turned down by the server, named one by one. */
  failed: Planned[];
};

export const planIcsImport = (calendarId: number, path: string) =>
  invoke<Planned[]>('plan_ics_import', { calendarId, path });

export const runIcsImport = (calendarId: number, path: string) =>
  invoke<ImportReport>('run_ics_import', { calendarId, path });

/** The file chooser behind the form's "Import .ics…" (#180): the picked
 *  path, or `null` when it was dismissed. Only a path, because what happens
 *  next is the panel a dropped file opens. */
export const pickIcsFile = () => invoke<string | null>('pick_ics_file');

/** The sentence above the preview.
 *
 *  It names the guest count because that is the one thing an import quietly
 *  leaves behind: the events come in, their guest lists do not, and a user
 *  who is told afterwards has been surprised rather than informed. Written
 *  here rather than in Rust so the wording lives with the panel that shows
 *  it, and is tested with it. */
export const planSummary = (plan: Planned[]): string => {
  const importing = plan.filter((p) => p.kind === 'import').length;
  const skipped = plan.length - importing;
  const guests = plan.reduce(
    (n, p) => n + (p.kind === 'import' ? p.dropped_guests : 0), 0,
  );
  let s = `${importing} event${importing === 1 ? '' : 's'} to import, ${skipped} skipped`;
  if (guests > 0) s += `, ${guests} guest ${guests === 1 ? 'entry' : 'entries'} left behind`;
  return s;
};

/** The last path segment, for naming the file in the panel's title. */
export const fileName = (path: string): string =>
  path.split(/[\\/]/).filter(Boolean).pop() ?? path;

/** Whether a dropped path is one this can import at all. Extension only:
 *  the backend reads and judges the contents, and a file that lies about
 *  its name is refused there rather than here. */
export const isIcs = (path: string): boolean => /\.ics$/i.test(path.trim());
