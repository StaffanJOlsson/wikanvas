// public/theme-init.js runs before the first paint (docs/ARCHITECTURE.md §9.2): it sets <html data-theme> from
// localStorage['wikanvas.theme'], and falls back to 'system'.
import { describe, expect, it, vi } from 'vitest';
import source from '../../public/theme-init.js?raw';

function run(): string | null {
  document.documentElement.removeAttribute('data-theme');
  new Function(source)();
  return document.documentElement.getAttribute('data-theme');
}

describe('theme-init.js', () => {
  it.each(['light', 'dark', 'system'])('applies a stored %s', (theme) => {
    localStorage.setItem('wikanvas.theme', theme);
    expect(run()).toBe(theme);
  });

  it('falls back to system without a stored theme or with garbage', () => {
    expect(run()).toBe('system');
    localStorage.setItem('wikanvas.theme', 'neon');
    expect(run()).toBe('system');
  });

  it('falls back to system when storage throws', () => {
    const spy = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new DOMException('denied', 'SecurityError');
    });
    expect(run()).toBe('system');
    spy.mockRestore();
  });

  it('is a plain script: no module syntax, no inline-script dependency', () => {
    expect(source).not.toMatch(/^\s*(import|export)\s/m);
    expect(source).toContain("getItem('wikanvas.theme')");
  });
});
