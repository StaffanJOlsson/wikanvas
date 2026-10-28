// Optimistic tree updates (docs/ARCHITECTURE.md §9.2): a move, a create or a trash applied to the cached flat tree
// lists before the server answers. Sort keys are the server's (D6); the provisional key given to a moved row here only
// places it until the response replaces the row with the server's.
import type { ItemSummary } from '../../api/gen/ItemSummary';
import type { Position } from '../../api/gen/Position';
import { byKey } from './buildTree';

/**
 * A string strictly between `lower` and `upper` in code-unit order (either may be null: unbounded). Display-only; never
 * sent to the server.
 */
export function provisionalKey(lower: string | null, upper: string | null): string {
  if (lower === null && upper === null) return 'a0';
  if (lower === null) return upper!.slice(0, -1);
  if (upper === null || upper <= lower) return `${lower}V`;
  const nice = `${lower}V`;
  return nice < upper ? nice : `${lower}\u0000`;
}

/** The ids of `id` and every row below it in a flat list. */
export function subtreeIds(items: readonly ItemSummary[], id: string): Set<string> {
  const children = new Map<string, string[]>();
  for (const it of items) {
    if (it.parent_id === null) continue;
    const list = children.get(it.parent_id);
    if (list) list.push(it.id);
    else children.set(it.parent_id, [it.id]);
  }
  const out = new Set<string>([id]);
  const stack = [id];
  while (stack.length > 0) {
    for (const c of children.get(stack.pop()!) ?? []) {
      if (!out.has(c)) {
        out.add(c);
        stack.push(c);
      }
    }
  }
  return out;
}

function keyFor(siblings: ItemSummary[], position: Position | null | undefined): string {
  const sorted = [...siblings].sort(byKey);
  const at = (id: string | null | undefined) => (id ? sorted.findIndex((s) => s.id === id) : -1);
  const after = at(position?.after_id);
  if (after >= 0) return provisionalKey(sorted[after]!.sort_key, sorted[after + 1]?.sort_key ?? null);
  const before = at(position?.before_id);
  if (before >= 0) return provisionalKey(sorted[before - 1]?.sort_key ?? null, sorted[before]!.sort_key);
  return provisionalKey(sorted[sorted.length - 1]?.sort_key ?? null, null);
}

function bumpChildCount(items: ItemSummary[], id: string | null, by: number): ItemSummary[] {
  if (id === null) return items;
  return items.map((it) => (it.id === id ? { ...it, child_count: Math.max(0, it.child_count + by) } : it));
}

export interface MoveTarget {
  collection_id: string;
  parent_id: string | null;
  position?: Position | null;
}

/**
 * Applies a move to the flat lists of the source and target collections (`lists` maps collection id to its cached
 * items; a target that is not loaded is left out). Returns the changed lists only.
 */
export function applyMove(
  lists: ReadonlyMap<string, readonly ItemSummary[]>,
  itemId: string,
  to: MoveTarget,
): Map<string, ItemSummary[]> {
  const out = new Map<string, ItemSummary[]>();
  let fromCid: string | null = null;
  for (const [cid, items] of lists) if (items.some((i) => i.id === itemId)) fromCid = cid;
  if (fromCid === null) return out;
  const from = lists.get(fromCid)!;
  const moving = from.find((i) => i.id === itemId)!;
  const ids = subtreeIds(from, itemId);
  const cross = to.collection_id !== fromCid;

  let source = bumpChildCount([...from], moving.parent_id, -1);
  let rows: ItemSummary[];
  if (cross) {
    rows = source.filter((i) => ids.has(i.id)).map((i) => ({ ...i, collection_id: to.collection_id }));
    source = source.filter((i) => !ids.has(i.id));
    out.set(fromCid, source);
  } else {
    rows = source.filter((i) => ids.has(i.id));
    source = source.filter((i) => !ids.has(i.id));
  }

  const targetBase = cross ? lists.get(to.collection_id) : source;
  if (!targetBase) return out;
  let target = bumpChildCount([...targetBase], to.parent_id, 1);
  const siblings = target.filter((i) => i.parent_id === to.parent_id);
  const moved = rows.map((r) =>
    r.id === itemId ? { ...r, parent_id: to.parent_id, sort_key: keyFor(siblings, to.position) } : r);
  target = [...target, ...moved];
  out.set(to.collection_id, target);
  return out;
}

/** Inserts a created item (last among its siblings unless the server's key says otherwise). */
export function applyCreate(items: readonly ItemSummary[], created: ItemSummary): ItemSummary[] {
  if (items.some((i) => i.id === created.id)) return items.map((i) => (i.id === created.id ? created : i));
  return [...bumpChildCount([...items], created.parent_id, 1), created];
}

/** Removes an item and its subtree (trash, archive). */
export function applyRemove(items: readonly ItemSummary[], itemId: string): ItemSummary[] {
  const gone = items.find((i) => i.id === itemId);
  if (!gone) return [...items];
  const ids = subtreeIds(items, itemId);
  return bumpChildCount(items.filter((i) => !ids.has(i.id)), gone.parent_id, -1);
}

/** Replaces one row with the server's copy (which carries the server sort key). */
export function applyRow(items: readonly ItemSummary[], row: ItemSummary): ItemSummary[] {
  return items.map((i) => (i.id === row.id ? { ...i, ...row } : i));
}
