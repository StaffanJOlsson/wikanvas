// Which sidebar rows are expanded (docs/ARCHITECTURE.md §9.2): collection and item ids, persisted in
// localStorage['wikanvas.sidebar.expanded'] as a JSON array.
import { create } from 'zustand';
import { readJson, writeStorage } from './storage';

export const EXPANDED_KEY = 'wikanvas.sidebar.expanded';
/** At most this many ids are kept; the oldest expansions are dropped first. */
export const EXPANDED_MAX = 5000;

interface ExpandedState {
  ids: ReadonlySet<string>;
}

function load(): ReadonlySet<string> {
  const raw = readJson(EXPANDED_KEY);
  if (!Array.isArray(raw)) return new Set();
  return new Set(raw.filter((x): x is string => typeof x === 'string').slice(-EXPANDED_MAX));
}

export const useExpandedStore = create<ExpandedState>(() => ({ ids: load() }));

function commit(ids: Set<string>): void {
  let list = [...ids];
  if (list.length > EXPANDED_MAX) list = list.slice(-EXPANDED_MAX);
  const next = new Set(list);
  useExpandedStore.setState({ ids: next });
  writeStorage(EXPANDED_KEY, JSON.stringify(list));
}

export function isExpanded(id: string): boolean {
  return useExpandedStore.getState().ids.has(id);
}

export function setExpanded(id: string, open: boolean): void {
  const cur = useExpandedStore.getState().ids;
  if (cur.has(id) === open) return;
  const next = new Set(cur);
  if (open) next.add(id);
  else next.delete(id);
  commit(next);
}

export function toggleExpanded(id: string): void {
  setExpanded(id, !isExpanded(id));
}

/** Expands every id (the current item's collection and ancestors, §9.6.2). */
export function expandAll(ids: readonly string[]): void {
  const cur = useExpandedStore.getState().ids;
  if (ids.every((id) => cur.has(id))) return;
  const next = new Set(cur);
  for (const id of ids) next.add(id);
  commit(next);
}

/** Re-reads the persisted set (tests; a storage event from another tab). */
export function reloadExpanded(): void {
  useExpandedStore.setState({ ids: load() });
}

/** The expanded set; re-renders on change. */
export function useExpandedIds(): ReadonlySet<string> {
  return useExpandedStore((s) => s.ids);
}
