/** ISO weeks are numbered in the year containing their Thursday. Work in UTC
 *  after taking local calendar parts, so DST changes cannot shift a date. */
export function isoWeek(date: Date): { year: number; week: number } {
  const d = new Date(Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()));
  d.setUTCDate(d.getUTCDate() + 4 - (d.getUTCDay() || 7));
  const year = d.getUTCFullYear();
  const first = new Date(Date.UTC(year, 0, 1));
  return { year, week: Math.ceil(((d.getTime() - first.getTime()) / 86400000 + 1) / 7) };
}

/** A seven-day calendar row is named for the Thursday within it. */
export function rowWeek(startMs: number): number {
  const thursday = new Date(startMs);
  thursday.setDate(thursday.getDate() + (4 - thursday.getDay() + 7) % 7);
  return isoWeek(thursday).week;
}

/** Rolling/panned ranges may straddle an ISO boundary, including New Year. */
export function rangeWeeks(startMs: number, days: number): string {
  const first = new Date(startMs);
  const last = new Date(startMs);
  last.setDate(last.getDate() + days - 1);
  const a = isoWeek(first), b = isoWeek(last);
  return a.year === b.year && a.week === b.week ? `Week ${a.week}` : `Weeks ${a.week}–${b.week}`;
}
