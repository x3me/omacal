import { invoke } from '@tauri-apps/api/core';

/** One connected account, as `accounts::list_accounts` shapes it. */
export type Account = {
  id: number;
  email: string;
  provider: string; // 'google' | 'caldav' | 'webcal'
  /** A CalDAV account's server and login name, for the Reconnect form to fill
   *  in; null for Google and feeds. Optional: older stubs leave them out. */
  serverUrl?: string | null;
  username?: string | null;
  /** The server stopped accepting this account's password. */
  needsReconnect?: boolean;
};

/** iCloud's CalDAV address (`omacal_caldav::ICLOUD_BASE`): an account there
 *  reconnects with the iCloud-worded form. */
export const ICLOUD_BASE = 'https://caldav.icloud.com/';

/** Subscribes to a public read-only WebCal feed (`webcal://` or `https://`).
 *  Resolves to the calendar id the feed syncs into. */
export const subscribeWebcal = (url: string, name?: string) =>
  invoke<number>('subscribe_webcal', { url, name: name ?? null });

export const listAccounts = () => invoke<Account[]>('list_accounts');

/** Signs an account out: revokes the Google grant (best-effort), clears the
 *  keyring entry, and deletes the account's local data — calendars, events,
 *  tasks. Resolves to the accounts that remain. */
export const signOut = (accountId: number) => invoke<Account[]>('sign_out', { accountId });
