import type { Event } from './Timeline.mjs';
export interface Window { key: string; appId: string; title: string }
export interface Presence { key: string; signature: string; eventKey: string }
export const ZOOM_HOSTS: readonly string[];
export function isZoomHost(host: string): boolean;
export function eventKey(event: Event | null): string;
export function observe(previous: Presence[], windows: Window[], events: Event[], now: number, leadMinutes: number, intent?: { at: number; eventKey: string } | null): Presence[];
export function isPresent(records: Presence[], event: Event | null, now: number): boolean;
