// Engine cache warm-up (docs/ARCHITECTURE.md §9.5): hovering a board row for 300 ms fetches the engine's wasm and JS
// with `fetch(url, {cache: 'force-cache', priority: 'low'})`, so the first board opens from the HTTP cache. A
// `<link rel=prefetch>` would fall under the SPA CSP's `default-src 'none'`, so it is never used. Each engine build is
// warmed once per page load. The sidebar (W3, `TreeRow`) times the hover and calls `boardRowHovered`; the board host
// registers the warm-up there with `useBoardRowWarmup()`, since it is mounted once beside the routes.
import { useEffect } from 'react';
import { useConfig } from '../api/queries/config';
import { inkboardBase } from '../lib/urls';
import { onBoardRowHover } from '../state/hover';

/** The engine files worth warming (the frame page itself is small and loads with the iframe). */
export const WARM_FILES = ['inkboard_bg.wasm', 'inkboard.js'] as const;

const warmed = new Set<string>();

/** Fetches the engine files of the build at `base` into the HTTP cache, once. */
export function warmEngine(base: string): void {
  if (warmed.has(base)) return;
  warmed.add(base);
  for (const file of WARM_FILES) {
    const init: RequestInit & { priority?: 'high' | 'low' | 'auto' } = {
      cache: 'force-cache',
      credentials: 'same-origin',
      priority: 'low',
    };
    fetch(`${base}${file}`, init)
      .then((r) => r.arrayBuffer())
      .catch(() => warmed.delete(base));
  }
}

/** Tests: forget what was warmed. */
export function resetWarmup(): void {
  warmed.clear();
}

/**
 * Registers the warm-up for the sidebar's 300 ms board-row hover (`onBoardRowHover`) while mounted: it warms the
 * embedded engine of the current config, and does nothing without one.
 */
export function useBoardRowWarmup(): void {
  const base = inkboardBase(useConfig().data);
  useEffect(() => {
    if (base === null) return;
    return onBoardRowHover(() => warmEngine(base));
  }, [base]);
}
