export interface Event {
  title: string | null; start_ms: number; end_ms: number; all_day: boolean;
  conference?: string | null; color?: string | null; calendar?: string | null;
}
export function progress(event: Event, now: number): number;
export function joinable<T extends Event>(events: T[], now: number, minutes: number): T | null;
export function uniqueAllDay<T extends Event>(events: T[]): T[];

export function currentClock(ms: number, format: '12h' | '24h', offsetSeconds: number): string;

export function countdownDuration(minutes: number): string;

export const DEFAULT_MEETING_FORMAT: string;
export function meetingLabel(template: string, values: Record<string, string>): string;

export const PER_DAY_CAP: number;
export interface AgendaPanel<T extends Event = Event> {
  agenda_days?: { date_label: string; events: T[] }[];
  day_start_ms: number;
  earlier?: string; tomorrow?: boolean; days_ahead?: number; per_day?: number;
}
export interface AgendaSection<T extends Event = Event> {
  title: string; kind: 'rows' | 'folded'; rows: T[]; more: number; anchor_ms: number; count?: number;
}
export function agendaSections<T extends Event>(panel: AgendaPanel<T> | null | undefined, now: number, opts?: { earlierOpen?: boolean }): AgendaSection<T>[];
