// Engine cache warm-up (docs/ARCHITECTURE.md §9.5): hovering a board row for 300 ms fetches the engine's wasm and JS
// with `fetch(url, {cache: 'force-cache', priority: 'low'})`, so the first board opens from the HTTP cache. A
// `<link rel=prefetch>` would fall under the SPA CSP's `default-src 'none'`, so it is never used. Each engine build is
// warmed once per page load. The sidebar's board rows (W3) spread `useBoardWarmup()` on themselves.
import { useMemo } from 'react';
import { useConfig } from '../api/queries/config';
import { inkboardBase } from '../lib/urls';

/** How long the pointer rests on a board row before the engine is fetched. */
export const HOVER_WARM_MS = 300;

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

export interface HoverHandlers {
  onPointerEnter(): void;
  onPointerLeave(): void;
}

/** Hover handlers that warm the engine of `base()` after HOVER_WARM_MS; leaving earlier cancels. */
export function hoverWarmup(base: () => string | null): HoverHandlers {
  let timer: ReturnType<typeof setTimeout> | null = null;
  return {
    onPointerEnter() {
      if (timer !== null) return;
      timer = setTimeout(() => {
        timer = null;
        const b = base();
        if (b !== null) warmEngine(b);
      }, HOVER_WARM_MS);
    },
    onPointerLeave() {
      if (timer !== null) clearTimeout(timer);
      timer = null;
    },
  };
}

/** `<a {...useBoardWarmup()}>` on a board row: warms the embedded engine on hover (nothing without one). */
export function useBoardWarmup(): HoverHandlers {
  const config = useConfig();
  const base = inkboardBase(config.data);
  return useMemo(() => hoverWarmup(() => base), [base]);
}
