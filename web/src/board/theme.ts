// Theme sync for the board frame (docs/ARCHITECTURE.md §9.5): the frame's chrome follows Quire's theme, which is
// `<html data-theme>` (`light`, `dark` or `system`, set by theme-init.js and the settings) resolved against
// `prefers-color-scheme`. The board's background stays board content.
import { useSyncExternalStore } from 'react';

export type ResolvedTheme = 'light' | 'dark';

const DARK_QUERY = '(prefers-color-scheme: dark)';

/** `light`/`dark` as set, `system` (or anything else) by the media query. */
export function resolveTheme(attr: string | null | undefined, prefersDark: boolean): ResolvedTheme {
  if (attr === 'light' || attr === 'dark') return attr;
  return prefersDark ? 'dark' : 'light';
}

function media(query: string): MediaQueryList | null {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function' ? window.matchMedia(query) : null;
}

/** The current resolved theme of the document. */
export function currentTheme(): ResolvedTheme {
  return resolveTheme(document.documentElement.getAttribute('data-theme'), media(DARK_QUERY)?.matches ?? false);
}

function subscribe(onChange: () => void): () => void {
  const observer = new MutationObserver(onChange);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  const mq = media(DARK_QUERY);
  mq?.addEventListener('change', onChange);
  return () => {
    observer.disconnect();
    mq?.removeEventListener('change', onChange);
  };
}

/** The resolved theme; re-renders when `data-theme` or the system preference changes. */
export function useResolvedTheme(): ResolvedTheme {
  return useSyncExternalStore(subscribe, currentTheme, () => 'light');
}

/** Whether a media query matches; re-renders when it changes. */
export function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (onChange) => {
      const mq = media(query);
      mq?.addEventListener('change', onChange);
      return () => mq?.removeEventListener('change', onChange);
    },
    () => media(query)?.matches ?? false,
    () => false,
  );
}

/** Phones: a coarse pointer on a narrow screen opens boards in BoardPreview by default (§9.5). */
export const PHONE_QUERY = '(max-width: 767px) and (pointer: coarse)';
