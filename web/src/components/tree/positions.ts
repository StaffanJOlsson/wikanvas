// A drop instruction as the body of `POST /api/v1/items/{id}/move` (docs/ARCHITECTURE.md §7.1, D6): the client names
// the neighbours `{after_id, before_id}` and the server computes the sort key. Neighbours never include the moved item
// itself. "child" and "collection" drops omit the position: the item goes last.
import type { ItemSummary } from '../../api/gen/ItemSummary';
import type { MoveItem } from '../../api/gen/MoveItem';
import type { Position } from '../../api/gen/Position';
import { type Forest, type TreeNode } from './buildTree';
import type { DropInstruction } from './drop';

/** The body of a move, with the collection always named. */
export interface Move extends MoveItem {
  collection_id: string;
}

function group(forest: Forest, parentId: string | null): TreeNode[] {
  if (parentId === null) return forest.roots;
  return forest.byId.get(parentId)?.children ?? [];
}

/** Where an item is now, as a move that would put it back there (the Undo of §9.6.3). */
export function currentPlace(forest: Forest | undefined, item: ItemSummary): Move {
  const sibs = forest ? group(forest, item.parent_id) : [];
  const i = sibs.findIndex((n) => n.item.id === item.id);
  if (i < 0) return { collection_id: item.collection_id, parent_id: item.parent_id };
  return {
    collection_id: item.collection_id,
    parent_id: item.parent_id,
    position: { after_id: sibs[i - 1]?.item.id ?? null, before_id: sibs[i + 1]?.item.id ?? null },
  };
}

function findForest(forests: ReadonlyMap<string, Forest | undefined>, itemId: string): Forest | undefined {
  for (const f of forests.values()) if (f?.byId.has(itemId)) return f;
  return undefined;
}

/**
 * The move a drop of `item` performs, or null when it would change nothing (same parent and same neighbours, the same
 * parent in a non-manual group, or before/after the item itself).
 */
export function dropToMove(
  instruction: DropInstruction,
  item: ItemSummary,
  forests: ReadonlyMap<string, Forest | undefined>,
): Move | null {
  const own = forests.get(item.collection_id);
  const ownSibs = own ? group(own, item.parent_id) : [];
  const ownIndex = ownSibs.findIndex((n) => n.item.id === item.id);
  const isLast = ownIndex === ownSibs.length - 1;

  if (instruction.type === 'collection') {
    const cid = instruction.collectionId;
    const order = forests.get(cid)?.order ?? 'manual';
    if (cid === item.collection_id && item.parent_id === null && (isLast || order !== 'manual')) return null;
    return { collection_id: cid, parent_id: null };
  }

  const forest = findForest(forests, instruction.targetId);
  const target = forest?.byId.get(instruction.targetId);
  if (!forest || !target) return null;
  const cid = forest.collectionId;
  const sameGroup = (parentId: string | null) => cid === item.collection_id && item.parent_id === parentId;

  if (instruction.type === 'child') {
    if (sameGroup(target.item.id) && (isLast || forest.order !== 'manual')) return null;
    return { collection_id: cid, parent_id: target.item.id };
  }

  // Before or after itself: where it already is (e.g. "below its expanded parent" when it is the first child).
  if (target.item.id === item.id) return null;
  const parentId = target.item.parent_id;
  const sibs = group(forest, parentId).filter((n) => n.item.id !== item.id);
  const i = sibs.indexOf(target);
  if (instruction.type === 'before') {
    if (sameGroup(parentId) && ownSibs[ownIndex + 1] === target) return null;
    return {
      collection_id: cid,
      parent_id: parentId,
      position: { after_id: sibs[i - 1]?.item.id ?? null, before_id: target.item.id },
    };
  }
  if (sameGroup(parentId) && ownIndex > 0 && ownSibs[ownIndex - 1] === target) return null;
  return {
    collection_id: cid,
    parent_id: parentId,
    position: { after_id: target.item.id, before_id: sibs[i + 1]?.item.id ?? null },
  };
}

/** Keyboard moves of §9.6.3 (Alt+Shift+arrows) as drop instructions; null when the move is not possible. */
export function keyboardInstruction(
  forest: Forest,
  itemId: string,
  key: 'indent' | 'outdent' | 'up' | 'down',
): DropInstruction | null {
  const node = forest.byId.get(itemId);
  if (!node) return null;
  const sibs = node.parent ? node.parent.children : forest.roots;
  const i = sibs.indexOf(node);
  switch (key) {
    case 'indent': {
      const prev = sibs[i - 1];
      return prev ? { type: 'child', targetId: prev.item.id } : null;
    }
    case 'outdent':
      return node.parent ? { type: 'after', targetId: node.parent.item.id } : null;
    case 'up': {
      if (forest.order !== 'manual') return null;
      const prev = sibs[i - 1];
      return prev ? { type: 'before', targetId: prev.item.id } : null;
    }
    case 'down': {
      if (forest.order !== 'manual') return null;
      const next = sibs[i + 1];
      return next ? { type: 'after', targetId: next.item.id } : null;
    }
  }
}

/**
 * The position of a reorder in a flat list (collections, stars; §9.6.3: before/after 50/50): before or after
 * `targetId`, with the neighbours named and the moved row left out. Null when nothing would change.
 */
export function listDropPosition(
  ids: readonly string[],
  draggedId: string,
  targetId: string,
  edge: 'before' | 'after',
): Position | null {
  if (draggedId === targetId) return null;
  const from = ids.indexOf(draggedId);
  const rest = ids.filter((id) => id !== draggedId);
  const t = rest.indexOf(targetId);
  if (t < 0) return null;
  const at = edge === 'before' ? t : t + 1;
  if (from === at) return null;
  return { after_id: rest[at - 1] ?? null, before_id: rest[at] ?? null };
}
