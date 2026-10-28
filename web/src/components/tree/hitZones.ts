// Drop geometry (docs/ARCHITECTURE.md §9.6.3): item rows have zones before 25% / inside 50% / after 25%; collection
// and starred rows reorder before/after 50/50. On the last child of an expanded parent, a pointer left of the row's
// indent outdents the drop to after the parent (one level per indent step).
import { type TreeNode, siblingsOf, type Forest } from './buildTree';
import { indentLevel } from './visibleRows';

/** Row padding before the first indent step, and the indent per level (§9.6.1: 16 px). */
export const ROW_PAD_PX = 8;
export const INDENT_PX = 16;

export type ItemZone = 'before' | 'inside' | 'after';
export type Edge = 'before' | 'after';

export interface RowRect {
  top: number;
  height: number;
  left: number;
}

/** The zone of an item row under the pointer: top 25% before, middle 50% inside, bottom 25% after. */
export function itemZone(clientY: number, rect: Pick<RowRect, 'top' | 'height'>): ItemZone {
  if (rect.height <= 0) return 'inside';
  const f = (clientY - rect.top) / rect.height;
  if (f < 0.25) return 'before';
  if (f >= 0.75) return 'after';
  return 'inside';
}

/** The half of a collection or starred row under the pointer. */
export function edgeZone(clientY: number, rect: Pick<RowRect, 'top' | 'height'>): Edge {
  return clientY - rect.top < rect.height / 2 ? 'before' : 'after';
}

/** The x where a row of `depth` starts its content, relative to the row's left edge. */
export function indentX(depth: number): number {
  return ROW_PAD_PX + indentLevel(depth) * INDENT_PX;
}

/** The indent level the pointer is at (0 = the collection's level). */
export function pointerLevel(clientX: number, rowLeft: number): number {
  return Math.max(0, Math.floor((clientX - rowLeft - ROW_PAD_PX) / INDENT_PX));
}

function isLastChild(forest: Forest, node: TreeNode): boolean {
  const sibs = siblingsOf(forest, node);
  return sibs[sibs.length - 1] === node;
}

/**
 * The node an "after" drop on `node` really goes after. While the pointer is left of the current node's indent and
 * the node is the last child of its parent, the drop moves out one level: after the parent. `node` must not show
 * expanded children (the "after" zone of an expanded parent means "before its first child").
 */
export function outdentTarget(forest: Forest, node: TreeNode, level: number): TreeNode {
  let n = node;
  while (n.parent !== null && level < indentLevel(n.depth) && isLastChild(forest, n)) n = n.parent;
  return n;
}
