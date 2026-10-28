// The flat `GET /collections/{id}/tree` list as a nested tree (docs/ARCHITECTURE.md D21, §9.6.2). Every sibling group
// of a collection is ordered by the collection's `child_order`:
// - manual: `sort_key` (byte order, the server's fractional keys, D6);
// - title: A–Z, case- and accent-insensitive, numbers in numeric order;
// - updated / created: newest first.
// Ties fall back to `sort_key`, then `id`, so the order is total and stable.
import type { ChildOrder } from '../../api/gen/ChildOrder';
import type { ItemSummary } from '../../api/gen/ItemSummary';

export interface TreeNode {
  item: ItemSummary;
  children: TreeNode[];
  parent: TreeNode | null;
  /** 1 for a top-level item; the collection row is depth 0. */
  depth: number;
}

export interface Forest {
  collectionId: string;
  order: ChildOrder;
  roots: TreeNode[];
  byId: ReadonlyMap<string, TreeNode>;
}

const collator = new Intl.Collator('en', { sensitivity: 'base', numeric: true });

function cmp(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

/** Manual order: sort key, then id. */
export function byKey(a: ItemSummary, b: ItemSummary): number {
  return cmp(a.sort_key, b.sort_key) || cmp(a.id, b.id);
}

function time(t: string): number {
  const ms = Date.parse(t);
  return Number.isNaN(ms) ? 0 : ms;
}

/** The sibling comparator of a `child_order`. */
export function comparator(order: ChildOrder): (a: ItemSummary, b: ItemSummary) => number {
  switch (order) {
    case 'manual':
      return byKey;
    case 'title':
      return (a, b) => collator.compare(a.title, b.title) || byKey(a, b);
    case 'updated':
      return (a, b) => time(b.updated_at) - time(a.updated_at) || byKey(a, b);
    case 'created':
      return (a, b) => time(b.created_at) - time(a.created_at) || byKey(a, b);
  }
}

/**
 * Builds the nested tree. An item whose parent is not in the list (or that sits on a corrupt cycle) becomes a root,
 * so nothing the server sent is lost.
 */
export function buildTree(collectionId: string, items: readonly ItemSummary[], order: ChildOrder): Forest {
  const byId = new Map<string, TreeNode>();
  for (const item of items) byId.set(item.id, { item, children: [], parent: null, depth: 1 });
  const roots: TreeNode[] = [];
  for (const node of byId.values()) {
    const pid = node.item.parent_id;
    const parent = pid !== null && pid !== node.item.id ? byId.get(pid) : undefined;
    if (parent) {
      node.parent = parent;
      parent.children.push(node);
    } else {
      roots.push(node);
    }
  }

  // Depths by walking down from the roots; anything unreachable is on a cycle: cut it loose as a root.
  const seen = new Set<TreeNode>();
  const walk = (start: TreeNode[], depth: number) => {
    const stack = start.map((n) => [n, depth] as const);
    while (stack.length > 0) {
      const [n, d] = stack.pop()!;
      if (seen.has(n)) continue;
      seen.add(n);
      n.depth = d;
      for (const c of n.children) stack.push([c, d + 1]);
    }
  };
  walk(roots, 1);
  for (const node of byId.values()) {
    if (seen.has(node)) continue;
    if (node.parent) node.parent.children = node.parent.children.filter((c) => c !== node);
    node.parent = null;
    roots.push(node);
    walk([node], 1);
  }

  const compare = comparator(order);
  const sortNodes = (nodes: TreeNode[]) => nodes.sort((a, b) => compare(a.item, b.item));
  sortNodes(roots);
  for (const node of byId.values()) sortNodes(node.children);
  return { collectionId, order, roots, byId };
}

/** The sibling group a node belongs to. */
export function siblingsOf(forest: Forest, node: TreeNode): TreeNode[] {
  return node.parent ? node.parent.children : forest.roots;
}

/** The ids of every descendant of `id` (not `id` itself). */
export function descendantIds(forest: Forest, id: string): Set<string> {
  const out = new Set<string>();
  const start = forest.byId.get(id);
  if (!start) return out;
  const stack = [...start.children];
  while (stack.length > 0) {
    const n = stack.pop()!;
    if (out.has(n.item.id)) continue;
    out.add(n.item.id);
    stack.push(...n.children);
  }
  return out;
}

/** The ancestors of `id`, root first (not `id` itself). */
export function ancestorIds(forest: Forest, id: string): string[] {
  const out: string[] = [];
  let n = forest.byId.get(id)?.parent ?? null;
  while (n && !out.includes(n.item.id)) {
    out.unshift(n.item.id);
    n = n.parent;
  }
  return out;
}
