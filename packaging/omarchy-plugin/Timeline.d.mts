export interface Event {
  title: string | null; start_ms: number; end_ms: number; all_day: boolean;
  colors?: string[];
  conference?: string | null; color?: string | null; calendar?: string | null;
}
export function progress(event: Event, now: number): number;
export function joinable<T extends Event>(events: T[], now: number, minutes: number): T | null;
export function layout<T extends Event>(events: T[], start: number, end: number): {
  event: T; start: number; end: number; lane: number; lanes: number; top: number; height: number;
}[];

export function uniqueAllDay<T extends Event>(events: T[]): T[];

export function currentClock(ms: number, format: '12h' | '24h', offsetSeconds: number): string;

export function countdownDuration(minutes: number): string;

export const DEFAULT_MEETING_FORMAT: string;
export function meetingLabel(template: string, values: Record<string, string>): string;
export function visibleRange(day: { day_start_ms: number; day_end_ms: number; visible_start_ms?: number; visible_end_ms?: number }): { start: number; end: number };
