// Per-item save state (docs/ARCHITECTURE.md §9.2, §9.3). The doc session (W4), the board save loop (W6) and the
// title autosave write it; the top bar's SaveStatus shows it.
import { create } from 'zustand';

export type SaveState = 'idle' | 'saving' | 'saved' | 'offline' | 'error' | 'signed_out';

export interface SaveStatus {
  state: SaveState;
  /** Shown next to (or instead of) the state's label, e.g. the error message. */
  message?: string;
  /** Offered as a "Retry" action, for `error` and `offline`. */
  retry?: () => void;
}

/** The status of an item nothing has reported on. */
export const IDLE: SaveStatus = Object.freeze({ state: 'idle' });

interface SaveStatusStore {
  byItem: Readonly<Record<string, SaveStatus>>;
}

const useStore = create<SaveStatusStore>(() => ({ byItem: {} }));

/** Sets an item's status; a bare state is shorthand for `{state}`. */
export function setSaveStatus(itemId: string, s: SaveStatus | SaveState): void {
  const next: SaveStatus = typeof s === 'string' ? { state: s } : { ...s };
  useStore.setState((st) => ({ byItem: { ...st.byItem, [itemId]: next } }));
}

/** The item's current status (outside React). */
export function getSaveStatus(itemId: string): SaveStatus {
  return useStore.getState().byItem[itemId] ?? IDLE;
}

/** Forgets the item (back to `idle`), e.g. when its page unmounts with nothing pending. */
export function clearSaveStatus(itemId: string): void {
  if (!(itemId in useStore.getState().byItem)) return;
  useStore.setState((st) => {
    const byItem = { ...st.byItem };
    delete byItem[itemId];
    return { byItem };
  });
}

/** The item's status; re-renders when it changes. */
export function useSaveStatus(itemId: string): SaveStatus {
  return useStore((st) => st.byItem[itemId] ?? IDLE);
}

/** English label of a state, for the top bar. */
export function saveStateLabel(state: SaveState): string {
  switch (state) {
    case 'idle':
      return '';
    case 'saving':
      return 'Saving…';
    case 'saved':
      return 'Saved';
    case 'offline':
      return 'Offline';
    case 'error':
      return 'Not saved';
    case 'signed_out':
      return 'Signed out';
  }
}
