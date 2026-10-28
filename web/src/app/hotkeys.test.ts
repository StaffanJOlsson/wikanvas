import { describe, expect, it, vi } from 'vitest';
import { registerPending } from '../state/pending';
import { MAIN_ID, onHostShortcut, runHostShortcut, shortcutFor } from './hotkeys';

const key = (k: string, mods: Partial<{ mod: boolean; shift: boolean; alt: boolean }> = {}) => ({
  key: k,
  mod: false,
  shift: false,
  alt: false,
  ...mods,
});

describe('hotkeys', () => {
  it('maps the frame-forwarded keys', () => {
    expect(shortcutFor(key('k', { mod: true }))).toBe('quick-switcher');
    expect(shortcutFor(key('K', { mod: true }))).toBe('quick-switcher');
    expect(shortcutFor(key('.', { mod: true }))).toBe('toggle-sidebar');
    expect(shortcutFor(key('Escape'))).toBe('focus-main');
    expect(shortcutFor(key('s', { mod: true }))).toBe('save');
    expect(shortcutFor(key('k'))).toBeNull();
    expect(shortcutFor(key('p', { mod: true, shift: true }))).toBeNull(); // no publish shortcut in M1 (D23)
    expect(shortcutFor(key('k', { mod: true, alt: true }))).toBeNull();
  });

  it('runs the registered handler and reports whether it ran', () => {
    const open = vi.fn();
    const off = onHostShortcut('quick-switcher', open);
    expect(runHostShortcut(key('k', { mod: true }))).toBe(true);
    expect(open).toHaveBeenCalledOnce();
    off();
    expect(runHostShortcut(key('k', { mod: true }))).toBe(false);
    expect(runHostShortcut(key('x', { mod: true }))).toBe(false);
  });

  it('Escape focuses the main region by default', () => {
    const main = document.createElement('main');
    main.id = MAIN_ID;
    main.tabIndex = -1;
    document.body.append(main);
    expect(runHostShortcut(key('Escape'))).toBe(true);
    expect(document.activeElement).toBe(main);
    main.remove();
  });

  it('Mod+S flushes pending saves; a KeyboardEvent is normalized and prevented', () => {
    const flush = vi.fn();
    const off = registerPending('doc', { hasPending: () => true, flush });
    const mac = /Mac|iPhone|iPad/.test(navigator.userAgent);
    const e = new KeyboardEvent('keydown', { key: 's', ctrlKey: !mac, metaKey: mac, cancelable: true });
    expect(runHostShortcut(e)).toBe(true);
    expect(e.defaultPrevented).toBe(true);
    expect(flush).toHaveBeenCalledOnce();
    off();
  });
});
