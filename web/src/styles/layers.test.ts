// The board iframe sits in a fixed layer at z-index 1 inside `.app-shell` (W6-D30), over the board canvas. Every
// overlay that can open over the canvas (menus and popovers, dialogs, toasts, the update banner, the drawer, the
// editor's floating menus, the board's Nested popover) needs a z-index above it, and `.app-main` must not start a
// stacking context of its own: the overlays inside it would then be painted at its level, under the iframe.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

// Relative to web/, where Vitest runs (`pnpm --dir web …`).
const SHEETS = ['src/styles/global.css', 'src/editor/editor.css', 'src/board/board.css'];

/** Every `selector { declarations }` of the sheets (rules inside @media included), comma lists split. */
function rules(): { selector: string; body: string }[] {
  const out: { selector: string; body: string }[] = [];
  for (const sheet of SHEETS) {
    const css = readFileSync(sheet, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
    for (const m of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
      for (const selector of m[1]!.split(',')) out.push({ selector: selector.trim().replace(/\s+/g, ' '), body: m[2]! });
    }
  }
  return out;
}

function zIndex(selector: string): number | null {
  const values = rules()
    .filter((r) => r.selector === selector)
    .flatMap((r) => [...r.body.matchAll(/z-index:\s*(-?\d+)/g)].map((m) => Number(m[1])));
  return values.length > 0 ? Math.max(...values) : null;
}

const OVERLAYS = [
  '.top-bar',
  '.popover',
  '.dialog-backdrop',
  '.toasts',
  '.update-banner',
  '.app-shell[data-sidebar="drawer"] .sidebar-slot',
  '.drawer-backdrop',
  '.contents-popover > .contents',
  '.suggest-popup',
  '.bubble-menu',
  '.table-menu',
  '.block-menu',
  '.board-picker',
  '.board-nested [role="dialog"]',
];

describe('overlays over the board frame', () => {
  it('the frame layer is at z-index 1', () => {
    expect(zIndex('.board-frame-layer')).toBe(1);
  });

  it.each(OVERLAYS)('%s is above it', (selector) => {
    const frame = zIndex('.board-frame-layer') ?? 0;
    expect(zIndex(selector), selector).toBeGreaterThan(frame);
  });

  it('.app-main starts no stacking context', () => {
    const body = rules()
      .filter((r) => r.selector === '.app-main')
      .map((r) => r.body)
      .join(';');
    expect(body).not.toMatch(/\b(z-index|transform|filter|isolation|opacity|contain|will-change|perspective)\s*:/);
  });
});
