import { invoke } from '@tauri-apps/api/core';

/** Zoom is an independent OAuth connection: configuration says the build has
 * a native/public client id; connection says the user has completed consent. */
export type ZoomStatus = {
  configured: boolean;
  connected: boolean;
};

export const getZoomStatus = () => invoke<ZoomStatus>('zoom_status');
export const connectZoom = () => invoke<ZoomStatus>('connect_zoom');
export const disconnectZoom = () => invoke<ZoomStatus>('disconnect_zoom');
