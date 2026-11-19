import { dateTimeFormat } from './intlcache';
export type DateFormat = 'locale' | 'mdy' | 'dmy' | 'iso' | 'long-mdy' | 'long-dmy';
export const DATE_FORMATS: { value: DateFormat; label: string }[] = [
  { value: 'locale', label: 'System default' },
  { value: 'long-mdy', label: 'Sep 7, 2026' },
  { value: 'long-dmy', label: '7 Sep 2026' },
  { value: 'mdy', label: '09/07/2026' },
  { value: 'dmy', label: '07/09/2026' },
  { value: 'iso', label: '2026-09-07' },
];

// Date-only values must pass UTC explicitly; instants use the display zone.
export function formatDate(ms: number, format: DateFormat = 'locale', options: Intl.DateTimeFormatOptions = {}): string {
  if (format === 'locale') return dateTimeFormat(undefined, Object.keys(options).length ? options : { year: 'numeric', month: 'short', day: 'numeric' }).format(ms);
  const parts = dateTimeFormat('en-US', { timeZone: options.timeZone, year: 'numeric', month: '2-digit', day: '2-digit' }).formatToParts(ms);
  const part = (name: string) => parts.find(p => p.type === name)!.value;
  const [y, m, d] = [part('year'), part('month'), part('day')];
  if (format === 'iso') return `${y}-${m}-${d}`;
  if (format === 'mdy') return `${m}/${d}/${y}`;
  if (format === 'dmy') return `${d}/${m}/${y}`;
  const month = dateTimeFormat('en-US', { timeZone: options.timeZone, month: 'short' }).format(ms);
  return format === 'long-dmy' ? `${Number(d)} ${month} ${y}` : `${month} ${Number(d)}, ${y}`;
}
