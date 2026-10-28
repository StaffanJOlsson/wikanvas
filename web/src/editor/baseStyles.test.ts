// ProseMirror's base rules ship in editor.css, because TipTap's own inline <style> is blocked by the SPA CSP
// (`style-src 'self'`, docs/ARCHITECTURE.md §11): found by the real-browser smoke, which happy-dom cannot see.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

// Relative to web/, where Vitest runs (`pnpm --dir web …`).
const css = readFileSync('src/editor/editor.css', 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');

/** The declarations of every rule whose selector list is exactly `selector`. */
function body(selector: string): string {
  return [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
    .filter((m) => m[1]!.trim().replace(/\s+/g, ' ') === selector)
    .map((m) => m[2]!)
    .join(';');
}

describe('the editor base styles', () => {
  it('are in the stylesheet: white-space, separator images, hidden selection, gap cursor', () => {
    expect(body('.ProseMirror')).toMatch(/white-space:\s*pre-wrap/);
    expect(body('.ProseMirror')).toMatch(/white-space:\s*break-spaces/);
    expect(body('.ProseMirror')).toMatch(/position:\s*relative/);
    expect(body('.ProseMirror pre')).toMatch(/white-space:\s*pre-wrap/);
    expect(body('img.ProseMirror-separator')).toMatch(/display:\s*inline !important/);
    expect(body('img.ProseMirror-separator')).toMatch(/width:\s*0 !important/);
    expect(body('.ProseMirror-hideselection *')).toMatch(/caret-color:\s*transparent/);
    expect(body('.ProseMirror-hideselection *::selection')).toMatch(/background:\s*transparent/);
    expect(body('.ProseMirror-focused .ProseMirror-gapcursor')).toMatch(/display:\s*block/);
  });
});
