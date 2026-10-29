// The "Wikanvas was updated. Reload" check (docs/ARCHITECTURE.md §9.1): when a lazy route chunk fails to load (W3) or
// the board frame page answers 404 (W6), refetch `/api/v1/config` and compare it with the config this page started
// with. If the server's version, commit or engine base changed, the old chunks are gone: show the banner instead of
// an error. W6 calls `checkForUpdate()` on a frame.html 404.
import { create } from 'zustand';
import { api } from '../api/client';
import type { Config } from '../api/gen/Config';
import { configBaseline } from '../api/queries/config';

interface UpdateState {
  available: boolean;
}

export const useUpdate = create<UpdateState>(() => ({ available: false }));

/** True for the errors browsers and Vite raise when a dynamically imported chunk cannot be fetched. */
export function isChunkLoadError(e: unknown): boolean {
  if (!(e instanceof Error)) return false;
  return /dynamically imported module|Importing a module script failed|error loading dynamically imported module|Failed to fetch|Unable to preload CSS|ChunkLoadError|Loading chunk/i
    .test(`${e.name} ${e.message}`);
}

/** Whether `next` is another build than `before`. */
export function isNewBuild(before: Config, next: Config): boolean {
  return before.version !== next.version
    || before.commit !== next.commit
    || (before.inkboard?.base ?? null) !== (next.inkboard?.base ?? null);
}

let inFlight: Promise<boolean> | null = null;

/**
 * Refetches the config and shows the update banner if the server is another build. Resolves true when it did. Never
 * rejects (offline: false).
 */
export function checkForUpdate(): Promise<boolean> {
  if (useUpdate.getState().available) return Promise.resolve(true);
  inFlight ??= (async () => {
    try {
      const before = configBaseline();
      if (!before) return false;
      const next = await api<Config>('GET', '/config');
      if (!isNewBuild(before, next)) return false;
      useUpdate.setState({ available: true });
      return true;
    } catch {
      return false;
    } finally {
      inFlight = null;
    }
  })();
  return inFlight;
}

/** Hides the banner again (tests). */
export function resetUpdate(): void {
  useUpdate.setState({ available: false });
}

export function reloadPage(): void {
  window.location.reload();
}
