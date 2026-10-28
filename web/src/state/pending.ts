// The registry of unsaved work (docs/ARCHITECTURE.md §9.2). The doc session, the board save loop and the title input
// register one entry each per item; `beforeunload`, route changes and Ctrl/Cmd+S use it to flush or warn.

export interface PendingEntry {
  /** True while something is not yet saved (queued, debounced or in flight). */
  hasPending(): boolean;
  /** Saves now; resolves when the save finished (or failed: errors are reported through saveStatus). */
  flush(): void | Promise<void>;
}

const registry = new Map<string, Set<PendingEntry>>();

/** Registers `entry` for `itemId`. Several entries per item are fine. Returns the function that unregisters it. */
export function registerPending(itemId: string, entry: PendingEntry): () => void {
  let set = registry.get(itemId);
  if (!set) {
    set = new Set();
    registry.set(itemId, set);
  }
  set.add(entry);
  let done = false;
  return () => {
    if (done) return;
    done = true;
    const s = registry.get(itemId);
    if (!s) return;
    s.delete(entry);
    if (s.size === 0) registry.delete(itemId);
  };
}

function entries(itemId?: string): PendingEntry[] {
  if (itemId !== undefined) return [...(registry.get(itemId) ?? [])];
  return [...registry.values()].flatMap((s) => [...s]);
}

/** Whether the item (or, without an id, any item) has unsaved work. */
export function hasPending(itemId?: string): boolean {
  return entries(itemId).some((e) => e.hasPending());
}

/**
 * Flushes every entry of the item (or of every item) that has pending work, in parallel. Resolves once all of them
 * settled; a failing flush does not stop the others and never rejects this promise.
 */
export async function flushPending(itemId?: string): Promise<void> {
  const work = entries(itemId)
    .filter((e) => e.hasPending())
    .map(async (e) => e.flush());
  const results = await Promise.allSettled(work);
  for (const r of results) {
    if (r.status === 'rejected') console.warn('flush failed', r.reason);
  }
}

/** The `beforeunload` handler: prompts while anything is pending. */
export function onBeforeUnload(e: BeforeUnloadEvent): void {
  if (!hasPending()) return;
  e.preventDefault();
  // Older browsers need returnValue set to show the prompt.
  e.returnValue = '';
}

/** Installs `onBeforeUnload` on `window`; returns the function that removes it. AppShell calls it once. */
export function installBeforeUnload(target: Window = window): () => void {
  target.addEventListener('beforeunload', onBeforeUnload);
  return () => target.removeEventListener('beforeunload', onBeforeUnload);
}
