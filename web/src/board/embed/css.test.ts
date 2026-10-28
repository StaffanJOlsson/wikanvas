// The overlay's CSS contract (docs/ARCHITECTURE.md §18.5, D62, D67), read from the sheets (happy-dom computes no
// styles): every box between the overlay and the editor is `overflow: clip` or visible, never `hidden` (a hidden box is
// a scroll container that ProseMirror's scrollRectIntoView and caret-into-view scroll, shifting every card off its
// engine box) and no rule of the overlay has `will-change` (Chromium keeps such a layer's raster scale, so text stays
// blurred after zooming in); only the editing body scrolls; cards are paper: every value of the light theme is
// re-declared on `.board-embed-card`, the ones derived from it at `:root` (`--focus`, `--shadow-pop`) included.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

// Relative to web/, where Vitest runs (`pnpm --dir web …`).
const strip = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '');
const BOARD = strip(readFileSync('src/board/board.css', 'utf8'));
const TOKENS = strip(readFileSync('src/styles/tokens.css', 'utf8'));

/** Every `selector { declarations }` of a sheet (comma lists split), in order. */
function rules(css: string): { selector: string; body: string }[] {
  const out: { selector: string; body: string }[] = [];
  for (const m of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    for (const s of m[1]!.split(',')) out.push({ selector: s.trim().replace(/\s+/g, ' '), body: m[2]! });
  }
  return out;
}

/** The declarations of the rules whose selector is exactly `selector`, later ones winning. */
function declarations(css: string, selector: string): Map<string, string> {
  const out = new Map<string, string>();
  for (const r of rules(css).filter((x) => x.selector === selector)) {
    for (const d of r.body.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim(), d.slice(i + 1).trim().replace(/\s+/g, ' '));
    }
  }
  return out;
}

const embedRules = () => rules(BOARD).filter((r) => /\.board-embed/.test(r.selector));

describe('board embeds CSS', () => {
  it('the overlay and the card boxes clip, never hide; only the editing body scrolls', () => {
    expect(declarations(BOARD, '.board-embeds').get('overflow')).toBe('clip');
    expect(declarations(BOARD, '.board-embed-card').get('overflow')).toBe('clip');
    expect(declarations(BOARD, '.board-embed-body').get('overflow')).toBe('clip');
    const editing = declarations(BOARD, '.board-embed-card[data-editing] .board-embed-body');
    expect(editing.get('overflow-y')).toBe('auto');
    expect(editing.get('overflow-x')).toBe('clip');
    expect(editing.get('overscroll-behavior')).toBe('contain');
    // The editor's ancestors inside the overlay: none of them may become a script-scrollable box.
    const ancestors = ['.board-embeds', '.board-embeds-world', '.board-embed-card', '.board-embed-editor'];
    for (const r of embedRules()) {
      const last = r.selector.split(' ').at(-1) ?? '';
      const isAncestor = ancestors.some((a) => last === a || last.startsWith(`${a}[`) || last.startsWith(`${a}:`));
      if (!isAncestor) continue;
      expect(r.body, r.selector).not.toMatch(/overflow(-[xy])?\s*:\s*(hidden|auto|scroll)/);
    }
  });

  it('has no will-change anywhere in the overlay', () => {
    for (const r of embedRules()) expect(r.body, r.selector).not.toMatch(/will-change/);
  });

  it('cards are paper: every light theme value is re-declared on the card, the derived ones too', () => {
    const light = declarations(TOKENS, ':root[data-theme="light"]');
    const card = declarations(BOARD, '.board-embed-card');
    expect(light.size).toBeGreaterThan(10);
    expect(card.get('color-scheme')).toBe('light');
    for (const [name, value] of light) {
      if (name === 'color-scheme') continue;
      expect(card.get(name), name).toBe(value);
    }
    // Defined once at :root from --accent, so it would keep the dark accent inside a card (a selected image).
    const focus = declarations(TOKENS, ':root').get('--focus');
    expect(focus).toBeDefined();
    expect(card.get('--focus')).toBe(focus);
  });
});
