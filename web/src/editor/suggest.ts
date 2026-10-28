// The state of the editor's suggestion popups (the slash menu and the mention menu, docs/ARCHITECTURE.md §9.6.5),
// shared between the TipTap Suggestion plugins and the React components that draw them.
import { useSyncExternalStore } from 'react';

export interface PopupItem {
  key: string;
  label: string;
  hint?: string;
  run(): void;
}

export interface PopupState {
  open: boolean;
  items: PopupItem[];
  selected: number;
  rect: DOMRect | null;
  /** Shown when there are no items (e.g. "No pages found"). */
  empty?: string;
}

/** A small external store per popup. */
export class Popup {
  private state: PopupState = { open: false, items: [], selected: 0, rect: null };
  private listeners = new Set<() => void>();

  get = (): PopupState => this.state;

  subscribe = (l: () => void): (() => void) => {
    this.listeners.add(l);
    return () => this.listeners.delete(l);
  };

  set(next: Partial<PopupState>): void {
    this.state = { ...this.state, ...next };
    if (this.state.selected >= this.state.items.length) this.state.selected = 0;
    for (const l of this.listeners) l();
  }

  close(): void {
    this.set({ open: false, items: [], selected: 0, rect: null });
  }

  /** Arrow keys, Enter and Tab; true when handled. */
  key(key: string): boolean {
    const s = this.state;
    if (!s.open) return false;
    if (key === 'ArrowDown' || key === 'ArrowUp') {
      if (s.items.length === 0) return true;
      const d = key === 'ArrowDown' ? 1 : -1;
      this.set({ selected: (s.selected + d + s.items.length) % s.items.length });
      return true;
    }
    if (key === 'Enter' || key === 'Tab') {
      const item = s.items[s.selected];
      if (!item) return false;
      item.run();
      return true;
    }
    return false;
  }
}

export function usePopup(p: Popup): PopupState {
  return useSyncExternalStore(p.subscribe, p.get, p.get);
}
