// Subscribes to a CSS media query (the drawer breakpoint, §9.6.1).
import { useSyncExternalStore } from 'react';

function query(q: string): MediaQueryList | null {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function' ? window.matchMedia(q) : null;
}

export function useMediaQuery(q: string): boolean {
  return useSyncExternalStore(
    (onChange) => {
      const mql = query(q);
      if (!mql) return () => undefined;
      mql.addEventListener('change', onChange);
      return () => mql.removeEventListener('change', onChange);
    },
    () => query(q)?.matches ?? false,
    () => false,
  );
}
