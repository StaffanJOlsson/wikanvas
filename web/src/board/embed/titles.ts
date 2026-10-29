// Titles (docs/ARCHITECTURE.md §18.7): the host keeps the engine's title map current. Whenever the cached
// `ItemDetail` of a card's target is loaded or changes (a rename anywhere), it sends, 250 ms later,
// `{[id]: snapshotTitle(title)}` (trimmed, 'Untitled' when blank, at most 200 characters) for every target in the
// layout's `items` whose detail is cached, and only when that map differs from the one sent last. The engine shows
// the titles in place and saves them with the board's next save; a rename alone never saves the board (D68). After
// every `open()` the engine's map is empty again, so it is resent.
import type { QueryClient } from '@tanstack/react-query';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import type { BoardFrameApi } from '../bridge';

export const TITLE_DEBOUNCE_MS = 250;

/** The longest label snapshot, in characters (code points): previews and the board's search text show it. */
export const MAX_SNAPSHOT_CHARS = 200;

/** The title a card's label snapshot gets: trimmed, `'Untitled'` when blank, at most `MAX_SNAPSHOT_CHARS` characters. */
export function snapshotTitle(title: string): string {
  const t = title.trim();
  if (!t) return 'Untitled';
  const chars = Array.from(t);
  return chars.length <= MAX_SNAPSHOT_CHARS ? t : chars.slice(0, MAX_SNAPSHOT_CHARS).join('').trimEnd();
}

/** The cached `ItemDetail` of item `id`, whichever ref (UUID or url_id) it was loaded by. */
export function cachedItem(qc: QueryClient, id: string): ItemDetail | undefined {
  const direct = qc.getQueryData<ItemDetail>(['item', id]);
  if (direct?.item?.id === id) return direct;
  for (const [, d] of qc.getQueriesData<ItemDetail>({ queryKey: ['item'] })) {
    if (d?.item?.id === id) return d;
  }
  return undefined;
}

export interface TitleSyncDeps {
  qc: QueryClient;
  frame(): BoardFrameApi | null;
  delay?: number;
}

const EMPTY = '{}';

export class TitleSync {
  private readonly deps: TitleSyncDeps;
  private items: readonly string[] = [];
  /** The map sent last, as sorted JSON; the engine's map is empty after `open()`. */
  private last = EMPTY;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private unsubscribe: (() => void) | null = null;

  constructor(deps: TitleSyncDeps) {
    this.deps = deps;
  }

  /** Starts watching the query cache. */
  start(): void {
    if (this.unsubscribe) return;
    this.unsubscribe = this.deps.qc.getQueryCache().subscribe((event) => {
      if (event.query.queryKey[0] !== 'item') return;
      if (event.type === 'added' || event.type === 'updated' || event.type === 'removed') this.schedule();
    });
  }

  /** The targets of the current board's cards (`EmbedLayout.items`). */
  setItems(items: readonly string[]): void {
    if (items.length === this.items.length && items.every((x, i) => x === this.items[i])) return;
    this.items = items;
    this.schedule();
  }

  /** The board was (re)opened: the engine's map is empty. */
  opened(): void {
    this.last = EMPTY;
    this.schedule();
  }

  /** No board: nothing to send. */
  reset(): void {
    this.items = [];
    this.last = EMPTY;
    this.clear();
  }

  /** The map the engine should hold now. */
  map(): Record<string, string> {
    const out: Record<string, string> = {};
    for (const id of this.items) {
      const d = cachedItem(this.deps.qc, id);
      if (d) out[id] = snapshotTitle(d.item.title);
    }
    return out;
  }

  /** Sends the map now when it changed. */
  flush(): void {
    this.clear();
    const frame = this.deps.frame();
    if (!frame?.setEmbedTitles) return;
    const map = this.map();
    const key = JSON.stringify(Object.keys(map).sort().map((k) => [k, map[k]]));
    const normalized = key === '[]' ? EMPTY : key;
    if (normalized === this.last) return;
    try {
      frame.setEmbedTitles(map);
      this.last = normalized;
    } catch (e) {
      console.warn('setEmbedTitles failed', e);
    }
  }

  dispose(): void {
    this.clear();
    this.unsubscribe?.();
    this.unsubscribe = null;
  }

  private schedule(): void {
    if (this.items.length === 0 && this.last === EMPTY) return;
    this.clear();
    this.timer = setTimeout(() => this.flush(), this.deps.delay ?? TITLE_DEBOUNCE_MS);
  }

  private clear(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }
}
