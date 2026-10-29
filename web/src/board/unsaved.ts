// The board's unsaved snapshot (docs/ARCHITECTURE.md §9.5): before every PUT the save loop writes the snapshot to
// IndexedDB under `wikanvas.board.unsaved/<itemId>` as `{baseRev, board, at}`, and clears it after a 200. When a board
// opens and a snapshot newer than the board's last write is found, the page offers "Restore unsaved changes from
// HH:MM?". IndexedDB can be missing or full (private windows): every call then quietly does nothing.
import { del, get, set } from 'idb-keyval';

export interface UnsavedSnapshot {
  /** The revision the snapshot's changes were made on (its PUT's If-Match). */
  baseRev: number;
  /** The envelope JSON. */
  board: string;
  /** When it was written, ms since the epoch. */
  at: number;
}

/** The IndexedDB key of an item's snapshot. */
export function unsavedKey(itemId: string): string {
  return `wikanvas.board.unsaved/${itemId}`;
}

export async function writeUnsaved(itemId: string, snapshot: UnsavedSnapshot): Promise<void> {
  try {
    await set(unsavedKey(itemId), snapshot);
  } catch (e) {
    console.warn('could not keep the unsaved board snapshot', e);
  }
}

export async function readUnsaved(itemId: string): Promise<UnsavedSnapshot | undefined> {
  try {
    const v = (await get(unsavedKey(itemId))) as UnsavedSnapshot | undefined;
    return v && typeof v.board === 'string' && typeof v.baseRev === 'number' && typeof v.at === 'number' ? v : undefined;
  } catch {
    return undefined;
  }
}

export async function clearUnsaved(itemId: string): Promise<void> {
  try {
    await del(unsavedKey(itemId));
  } catch {
    // Nothing to clear.
  }
}

/**
 * Whether a snapshot holds work the server does not have: it was written after the board's last content write
 * (`content_at`, else `updated_at`, of the item).
 */
export function isNewerThan(snapshot: UnsavedSnapshot, lastWrite: string | number | null | undefined): boolean {
  const t = typeof lastWrite === 'number' ? lastWrite : lastWrite ? Date.parse(lastWrite) : 0;
  return snapshot.at > (Number.isNaN(t) ? 0 : t);
}

/** "HH:MM" of a snapshot, in the browser's time zone. */
export function snapshotTime(at: number): string {
  return new Date(at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
}
