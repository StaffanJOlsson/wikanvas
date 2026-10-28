// Host shortcuts (docs/ARCHITECTURE.md §9.5, §9.6.3): Mod+K (quick switcher), Mod+. (toggle the sidebar), Escape
// (focus the main region) and Mod+S (flush saves). The board frame forwards its keys through
// `FrameEvents.onHostKey` (W6 passes them to `runHostShortcut`); W3's global keydown listener can call it with the
// KeyboardEvent itself. Mod+Shift+P (publish) is not a shortcut in M1 (D23).
import { flushPending } from '../state/pending';

/** A key as the frame forwards it: `mod` is Ctrl, or Cmd on macOS. */
export interface HostKey {
  key: string;
  mod: boolean;
  shift: boolean;
  alt: boolean;
}

export type HostShortcut = 'quick-switcher' | 'toggle-sidebar' | 'focus-main' | 'save';

type Handler = () => void;

const handlers = new Map<HostShortcut, Handler>();

/** The id of the element `focus-main` focuses by default (AppShell's `<main>`). */
export const MAIN_ID = 'main';

const defaults: Partial<Record<HostShortcut, Handler>> = {
  'focus-main': () => document.getElementById(MAIN_ID)?.focus(),
  save: () => void flushPending(),
};

function isMac(): boolean {
  return typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.userAgent);
}

/** The HostKey of a keyboard event (Mod is Cmd on macOS and Ctrl elsewhere). */
export function hostKeyOf(e: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'shiftKey' | 'altKey'>): HostKey {
  return { key: e.key, mod: isMac() ? e.metaKey : e.ctrlKey, shift: e.shiftKey, alt: e.altKey };
}

/** Which host shortcut a key is, or null. */
export function shortcutFor(k: HostKey): HostShortcut | null {
  const key = k.key.length === 1 ? k.key.toLowerCase() : k.key;
  if (key === 'Escape' && !k.mod && !k.shift && !k.alt) return 'focus-main';
  if (!k.mod || k.alt) return null;
  if (k.shift) return null; // Mod+Shift+P and every other Mod+Shift chord: nothing in M1.
  if (key === 'k') return 'quick-switcher';
  if (key === '.') return 'toggle-sidebar';
  if (key === 's') return 'save';
  return null;
}

/**
 * Sets the handler of a shortcut (the quick switcher and the sidebar register theirs on mount). Returns the function
 * that removes it again, restoring the default (if any).
 */
export function onHostShortcut(shortcut: HostShortcut, handler: Handler): () => void {
  handlers.set(shortcut, handler);
  return () => {
    if (handlers.get(shortcut) === handler) handlers.delete(shortcut);
  };
}

function isKeyboardEvent(e: HostKey | KeyboardEvent): e is KeyboardEvent {
  return typeof KeyboardEvent !== 'undefined' && e instanceof KeyboardEvent;
}

/**
 * Runs the host shortcut for `e`, if it is one with a handler. Returns true when it ran; for a KeyboardEvent it then
 * also calls `preventDefault()`.
 */
export function runHostShortcut(e: HostKey | KeyboardEvent): boolean {
  const k = isKeyboardEvent(e) ? hostKeyOf(e) : e;
  const shortcut = shortcutFor(k);
  if (shortcut === null) return false;
  const handler = handlers.get(shortcut) ?? defaults[shortcut];
  if (!handler) return false;
  if (isKeyboardEvent(e)) e.preventDefault();
  handler();
  return true;
}
