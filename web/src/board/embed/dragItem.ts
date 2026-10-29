// The one contract between the sidebar tree (the drag source) and the board (the frame's drop listeners, and the
// overlay's own for a drop on the editing card): the native drag type and its JSON (docs/ARCHITECTURE.md §18.6 (c),
// D70). The type is declared with `DragItemType`, so a mismatch with bridge.ts does not typecheck.
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

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

/** An item id in canonical form (36 characters, lowercase hex with hyphens, as `items.id`). */
export function isItemId(s: unknown): s is string {
  return typeof s === 'string' && UUID.test(s);
}
