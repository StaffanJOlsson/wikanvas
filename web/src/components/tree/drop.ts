// What a drop on a sidebar row means (docs/ARCHITECTURE.md §9.6.3):
// - on a collection row: the end of that collection's top level (moves across collections are allowed);
// - on an item row: before / inside / after by zone, "after" outdented by pointer x on a last child;
// - in a sibling group whose sort is not manual, before/after collapse to "into the parent" (the sort decides the
//   position) and the indicator highlights the parent;
// - the dragged item and its descendants are invalid targets (no indicator).
import { descendantIds, type Forest } from './buildTree';
import { itemZone, outdentTarget, pointerLevel, type RowRect } from './hitZones';
import type { Row } from './visibleRows';

export type DropInstruction =
  | { type: 'before'; targetId: string }
  | { type: 'after'; targetId: string }
  /** Last child of the target item. */
  | { type: 'child'; targetId: string }
  /** Last item of the collection's top level. */
  | { type: 'collection'; collectionId: string };

export interface DropIndicator {
  /** The row the indicator is drawn on. */
  rowKey: string;
  /** A line above or below the row, or the whole row highlighted. */
  kind: 'line-before' | 'line-after' | 'box';
  /** The depth the line is indented to. */
  depth: number;
}

export interface ResolvedDrop {
  instruction: DropInstruction;
  indicator: DropIndicator;
}

/** The item being dragged, as the resolution needs it. */
export interface DraggedItem {
  id: string;
  collectionId: string;
}

/** True when `targetId` is the dragged item or one of its descendants (§9.6.3). */
export function isInvalidTarget(forest: Forest | undefined, draggedId: string, targetId: string): boolean {
  if (targetId === draggedId) return true;
  return forest ? descendantIds(forest, draggedId).has(targetId) : false;
}

/**
 * A before/after instruction in a non-manual group becomes "into the parent": the parent item, or the collection for a
 * top-level target. Other instructions, and manual groups, are returned unchanged.
 */
export function collapseNonManual(forest: Forest, instruction: DropInstruction): DropInstruction {
  if (forest.order === 'manual') return instruction;
  if (instruction.type !== 'before' && instruction.type !== 'after') return instruction;
  const target = forest.byId.get(instruction.targetId);
  if (!target) return instruction;
  return target.parent
    ? { type: 'child', targetId: target.parent.item.id }
    : { type: 'collection', collectionId: forest.collectionId };
}

export interface ItemDropInput {
  rows: readonly Row[];
  /** The row under the pointer. */
  index: number;
  dragged: DraggedItem;
  forests: ReadonlyMap<string, Forest | undefined>;
  clientX: number;
  clientY: number;
  rect: RowRect;
}

/** The highlighted row of a "child" or "collection" instruction. */
function boxFor(rows: readonly Row[], instruction: DropInstruction): DropIndicator {
  if (instruction.type === 'collection') return { rowKey: instruction.collectionId, kind: 'box', depth: 0 };
  const row = rows.find((r) => r.key === instruction.targetId);
  return { rowKey: instruction.targetId, kind: 'box', depth: row?.depth ?? 1 };
}

/** Resolves an item being dragged over a row of the collections tree; null for an invalid target. */
export function resolveItemDrop({ rows, index, dragged, forests, clientX, clientY, rect }: ItemDropInput): ResolvedDrop | null {
  const row = rows[index];
  if (!row) return null;
  if (row.kind === 'collection') {
    const instruction: DropInstruction = { type: 'collection', collectionId: row.collection.id };
    return { instruction, indicator: { rowKey: row.key, kind: 'box', depth: 0 } };
  }

  const draggedForest = forests.get(dragged.collectionId);
  if (row.collectionId === dragged.collectionId && isInvalidTarget(draggedForest, dragged.id, row.item.id)) return null;
  const forest = forests.get(row.collectionId);
  if (!forest) return null;

  const zone = itemZone(clientY, rect);
  let instruction: DropInstruction;
  let indicator: DropIndicator;
  if (zone === 'inside') {
    instruction = { type: 'child', targetId: row.item.id };
    indicator = { rowKey: row.key, kind: 'box', depth: row.depth };
  } else if (zone === 'before') {
    instruction = { type: 'before', targetId: row.item.id };
    indicator = { rowKey: row.key, kind: 'line-before', depth: row.depth };
  } else if (row.expanded && row.node.children.length > 0) {
    // Below an expanded parent is above its first child.
    const first = row.node.children[0]!;
    instruction = { type: 'before', targetId: first.item.id };
    indicator = { rowKey: row.key, kind: 'line-after', depth: first.depth };
  } else {
    const target = outdentTarget(forest, row.node, pointerLevel(clientX, rect.left));
    instruction = { type: 'after', targetId: target.item.id };
    indicator = { rowKey: row.key, kind: 'line-after', depth: target.depth };
  }

  const collapsed = collapseNonManual(forest, instruction);
  if (collapsed !== instruction) {
    if (collapsed.type === 'child' && row.collectionId === dragged.collectionId
      && isInvalidTarget(draggedForest, dragged.id, collapsed.targetId)) return null;
    return { instruction: collapsed, indicator: boxFor(rows, collapsed) };
  }
  return { instruction, indicator };
}
