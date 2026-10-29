// The one contract between the sidebar tree (the drag source) and the board (the frame's drop listeners, and the
// overlay's own for a drop on the editing card): the native drag type and its JSON (docs/ARCHITECTURE.md §18.6 (c),
// D70). The type is declared with `DragItemType`, so a mismatch with bridge.ts does not typecheck. A board row's own
// type, read only by the document editor, is here too (D145).
import type { DraggedItem, DragItemType } from '../bridge';

export const DRAG_ITEM_TYPE: DragItemType = 'application/x-wikanvas-item';

/** The native drag data of a document row. */
export function dragDataOf(item: { id: string; title: string }): Record<DragItemType, string> {
  const data: DraggedItem = { v: 1, id: item.id, kind: 'doc', title: item.title };
  return { [DRAG_ITEM_TYPE]: JSON.stringify(data) };
}

/** The `DraggedItem` in drag data, or null for anything else (any page can start a drag with this type). */
export function parseDraggedItem(raw: string | null | undefined): DraggedItem | null {
  if (!raw) return null;
  let v: unknown;
  try {
    v = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!v || typeof v !== 'object') return null;
  const o = v as Record<string, unknown>;
  if (o.v !== 1 || o.kind !== 'doc' || typeof o.id !== 'string' || typeof o.title !== 'string') return null;
  return { v: 1, id: o.id, kind: 'doc', title: o.title };
}

/**
 * The native drag type a sidebar board row carries (D145). Only the document editor reads it, where a drop embeds the
 * board (§9.6.5); the board frame does not know it, so a board never becomes a card. Not part of `bridge.ts`.
 */
export const DRAG_BOARD_TYPE = 'application/x-wikanvas-board';

/** The JSON under `DRAG_BOARD_TYPE`. */
export interface DraggedBoard {
  v: 1;
  id: string;
  kind: 'board';
  title: string;
}

/** The native drag data of a board row. */
export function boardDragDataOf(item: { id: string; title: string }): Record<string, string> {
  const data: DraggedBoard = { v: 1, id: item.id, kind: 'board', title: item.title };
  return { [DRAG_BOARD_TYPE]: JSON.stringify(data) };
}

/** The `DraggedBoard` in drag data, or null for anything else, a board id that is not a canonical UUID included. */
export function parseDraggedBoard(raw: string | null | undefined): DraggedBoard | null {
  if (!raw) return null;
  let v: unknown;
  try {
    v = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!v || typeof v !== 'object') return null;
  const o = v as Record<string, unknown>;
  if (o.v !== 1 || o.kind !== 'board' || !isItemId(o.id) || typeof o.title !== 'string') return null;
  return { v: 1, id: o.id, kind: 'board', title: o.title };
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

/** An item id in canonical form (36 characters, lowercase hex with hyphens, as `items.id`). */
export function isItemId(s: unknown): s is string {
  return typeof s === 'string' && UUID.test(s);
}
