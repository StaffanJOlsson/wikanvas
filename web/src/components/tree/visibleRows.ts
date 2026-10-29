// The sidebar's flat visible-row model (docs/ARCHITECTURE.md §9.3, §9.6.2): one row per collection, then the rows of
// its expanded subtrees. The tree renders these rows (virtualized above 300, D21) and the keyboard and drag and drop
// work on them.
import type { Collection } from '../../api/gen/Collection';
import type { CollectionAccess } from '../../api/gen/CollectionAccess';
import type { ItemSummary } from '../../api/gen/ItemSummary';
import type { Forest, TreeNode } from './buildTree';

/** Indentation stops growing after this many levels (§9.6.1). */
export const MAX_INDENT_LEVEL = 10;
/** Rows are virtualized when more than this many are visible (D21). */
export const VIRTUALIZE_ABOVE = 300;

export interface CollectionRow {
  kind: 'collection';
  key: string;
  collection: Collection;
  depth: 0;
  indent: 0;
  expanded: boolean;
  hasChildren: boolean;
}

export interface ItemRow {
  kind: 'item';
  key: string;
  item: ItemSummary;
  node: TreeNode;
  collectionId: string;
  /** The caller's access to the item's collection (D152): its edit controls and drops need `write`. */
  access: CollectionAccess;
  /** 1 for a top-level item. */
  depth: number;
  /** The indentation level: the depth, capped at MAX_INDENT_LEVEL. */
  indent: number;
  expanded: boolean;
  hasChildren: boolean;
  /** 1-based position among its siblings, and the sibling count (aria-posinset / aria-setsize). */
  posInSet: number;
  setSize: number;
}

export type Row = CollectionRow | ItemRow;

export function indentLevel(depth: number): number {
  return Math.min(Math.max(depth, 0), MAX_INDENT_LEVEL);
}

export interface VisibleRowsInput {
  collections: readonly Collection[];
  /** The loaded trees by collection id; a missing one is not loaded yet. */
  forests: ReadonlyMap<string, Forest | undefined>;
  expanded: ReadonlySet<string>;
  /** Rows shown collapsed whatever `expanded` says (the collection being dragged, §9.6.3). */
  forceCollapsed?: ReadonlySet<string>;
}

export function visibleRows({ collections, forests, expanded, forceCollapsed }: VisibleRowsInput): Row[] {
  const rows: Row[] = [];
  const isOpen = (id: string) => expanded.has(id) && !forceCollapsed?.has(id);
  for (const collection of collections) {
    const forest = forests.get(collection.id);
    const open = isOpen(collection.id);
    rows.push({
      kind: 'collection',
      key: collection.id,
      collection,
      depth: 0,
      indent: 0,
      expanded: open,
      hasChildren: forest ? forest.roots.length > 0 : collection.item_count > 0,
    });
    if (!open || !forest) continue;
    // Iterative depth-first walk, so a very deep tree cannot overflow the stack.
    const stack: { nodes: TreeNode[]; i: number }[] = [{ nodes: forest.roots, i: 0 }];
    while (stack.length > 0) {
      const top = stack[stack.length - 1]!;
      if (top.i >= top.nodes.length) {
        stack.pop();
        continue;
      }
      const node = top.nodes[top.i]!;
      top.i++;
      const hasChildren = node.children.length > 0;
      const nodeOpen = hasChildren && isOpen(node.item.id);
      rows.push({
        kind: 'item',
        key: node.item.id,
        item: node.item,
        node,
        collectionId: collection.id,
        access: collection.my_access,
        depth: node.depth,
        indent: indentLevel(node.depth),
        expanded: nodeOpen,
        hasChildren,
        posInSet: top.i,
        setSize: top.nodes.length,
      });
      if (nodeOpen) stack.push({ nodes: node.children, i: 0 });
    }
  }
  return rows;
}

/** The caller's access to the row's collection. */
export function rowAccess(row: Row): CollectionAccess {
  return row.kind === 'collection' ? row.collection.my_access : row.access;
}

/** The label a row is found by in type-ahead. */
export function rowLabel(row: Row): string {
  return row.kind === 'collection' ? row.collection.name : row.item.title || 'Untitled';
}

/** The index of the row's parent row (the parent item, or the collection for a top-level item), or -1. */
export function parentRowIndex(rows: readonly Row[], index: number): number {
  const row = rows[index];
  if (!row || row.kind === 'collection') return -1;
  for (let i = index - 1; i >= 0; i--) {
    const r = rows[i]!;
    if (r.depth < row.depth) return i;
  }
  return -1;
}
