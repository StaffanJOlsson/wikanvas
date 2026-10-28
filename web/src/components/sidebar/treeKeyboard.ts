// The ARIA tree keyboard of the sidebar (docs/ARCHITECTURE.md §9.6.3) as a pure mapping from a key on the focused row
// to an action. CollectionsTree performs the actions.
import { parentRowIndex, rowLabel, type Row } from '../tree/visibleRows';

export interface TreeKey {
  key: string;
  /** Ctrl, or Cmd on macOS. */
  mod: boolean;
  shift: boolean;
  alt: boolean;
  /** AltGr is held (Windows reports it as Ctrl+Alt); its characters still type ahead. */
  altGraph?: boolean;
}

/** `?` opens the shortcuts dialog (AppShell), so it never types ahead (W3-D5). */
const SHORTCUTS_KEY = '?';

/** The character a key types for type-ahead, or null: one printable character, Shift and AltGr allowed. */
export function typedChar(k: TreeKey): string | null {
  if ([...k.key].length !== 1 || k.key.trim() === '' || k.key === SHORTCUTS_KEY) return null;
  return k.altGraph || (!k.mod && !k.alt) ? k.key : null;
}

export type TreeKeyAction =
  | { type: 'focus'; index: number }
  | { type: 'expand'; index: number }
  | { type: 'collapse'; index: number }
  | { type: 'open'; index: number; newTab: boolean }
  | { type: 'rename'; index: number }
  | { type: 'move'; index: number; dir: 'indent' | 'outdent' | 'up' | 'down' }
  | { type: 'delete'; index: number }
  | { type: 'menu'; index: number }
  | { type: 'focus-main' };

/** Type-ahead (§9.6.3): characters typed within 500 ms of each other form one prefix. */
export class TypeAhead {
  static readonly WINDOW_MS = 500;
  private buffer = '';
  private last = -Infinity;

  /** Adds a character; returns the current prefix. */
  push(char: string, now: number): string {
    if (now - this.last > TypeAhead.WINDOW_MS) this.buffer = '';
    this.last = now;
    // Repeating one letter cycles through the rows starting with it.
    this.buffer = this.buffer === char ? char : this.buffer + char;
    return this.buffer;
  }

  /** Starts a new prefix with `char` (the old prefix plus `char` matched nothing). */
  restart(char: string, now: number): string {
    this.buffer = char;
    this.last = now;
    return char;
  }
}

/** The next row (after `from`, wrapping) whose label starts with `prefix`, case-insensitively; -1 if none. */
export function findByPrefix(rows: readonly Row[], from: number, prefix: string, includeFrom: boolean): number {
  const p = prefix.toLocaleLowerCase();
  for (let step = includeFrom ? 0 : 1; step <= rows.length; step++) {
    const i = (from + step) % rows.length;
    if (rowLabel(rows[i]!).toLocaleLowerCase().startsWith(p)) return i;
  }
  return -1;
}

/** The action for a key on row `index`, or null when the key is not the tree's. */
export function treeKeyAction(
  rows: readonly Row[],
  index: number,
  k: TreeKey,
  typeAhead: TypeAhead,
  now: number = Date.now(),
): TreeKeyAction | null {
  const row = rows[index];
  if (!row) return null;
  const plain = !k.mod && !k.shift && !k.alt;

  if (k.alt && k.shift && !k.mod) {
    if (row.kind !== 'item') return null;
    switch (k.key) {
      case 'ArrowRight':
        return { type: 'move', index, dir: 'indent' };
      case 'ArrowLeft':
        return { type: 'move', index, dir: 'outdent' };
      case 'ArrowUp':
        return { type: 'move', index, dir: 'up' };
      case 'ArrowDown':
        return { type: 'move', index, dir: 'down' };
    }
    return null;
  }

  if (k.key === 'Enter' && !k.shift && !k.alt) return { type: 'open', index, newTab: k.mod };
  if ((k.key === 'F10' && k.shift && !k.mod && !k.alt) || (k.key === 'ContextMenu' && plain)) return { type: 'menu', index };

  // Type-ahead (§9.6.3 "Printable characters"): capitals, Shift digits and punctuation, and AltGr characters too.
  const char = typedChar(k);
  if (char !== null) {
    let prefix = typeAhead.push(char, now);
    let hit = findByPrefix(rows, index, prefix, prefix.length > 1);
    if (hit < 0 && prefix.length > 1) {
      prefix = typeAhead.restart(char, now);
      hit = findByPrefix(rows, index, prefix, false);
    }
    return hit >= 0 ? { type: 'focus', index: hit } : null;
  }
  if (!plain) return null;

  switch (k.key) {
    case 'ArrowDown':
      return index + 1 < rows.length ? { type: 'focus', index: index + 1 } : null;
    case 'ArrowUp':
      return index > 0 ? { type: 'focus', index: index - 1 } : null;
    case 'Home':
      return { type: 'focus', index: 0 };
    case 'End':
      return { type: 'focus', index: rows.length - 1 };
    case 'ArrowRight':
      if (!row.hasChildren) return null;
      if (!row.expanded) return { type: 'expand', index };
      return index + 1 < rows.length && rows[index + 1]!.depth > row.depth ? { type: 'focus', index: index + 1 } : null;
    case 'ArrowLeft': {
      if (row.expanded) return { type: 'collapse', index };
      const parent = parentRowIndex(rows, index);
      return parent >= 0 ? { type: 'focus', index: parent } : null;
    }
    case 'F2':
      return row.kind === 'item' ? { type: 'rename', index } : null;
    case 'Delete':
      return row.kind === 'item' ? { type: 'delete', index } : null;
    case 'Escape':
      return { type: 'focus-main' };
  }
  return null;
}
