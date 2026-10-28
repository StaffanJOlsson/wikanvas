// Collection actions of the sidebar and the collection page (docs/ARCHITECTURE.md §9.6.2, §9.6.3): reorder, sort,
// delete (with Undo) and the dialog openers.
import { useQueryClient } from '@tanstack/react-query';
import { useMemo } from 'react';
import type { ChildOrder } from '../../api/gen/ChildOrder';
import type { Collection } from '../../api/gen/Collection';
import type { Sidebar } from '../../api/gen/Sidebar';
import {
  sidebarKey,
  useDeleteCollection,
  useMoveCollection,
  usePatchCollection,
  useRestoreCollection,
} from '../../api/queries/tree';
import { confirmAction, openDialog } from '../../state/dialogs';
import { errorToast, toast } from '../../state/toasts';
import { listDropPosition } from '../tree/positions';

export const CHILD_ORDERS: readonly { order: ChildOrder; label: string }[] = [
  { order: 'manual', label: 'Manual' },
  { order: 'title', label: 'A–Z' },
  { order: 'updated', label: 'Last updated' },
  { order: 'created', label: 'Created' },
];

export function useCollectionActions() {
  const qc = useQueryClient();
  const move = useMoveCollection().mutateAsync;
  const patch = usePatchCollection().mutateAsync;
  const remove = useDeleteCollection().mutateAsync;
  const restore = useRestoreCollection().mutateAsync;

  return useMemo(() => ({
    edit(collection: Collection): void {
      openDialog({ kind: 'collection', collection });
    },

    create(): void {
      openDialog({ kind: 'collection', collection: null });
    },

    /** Reorders a collection before or after another (drag and drop); false when nothing changed. */
    async reorder(collection: Collection, targetId: string, edge: 'before' | 'after'): Promise<boolean> {
      const ids = qc.getQueryData<Sidebar>(sidebarKey)?.collections.map((c) => c.id) ?? [];
      const position = listDropPosition(ids, collection.id, targetId, edge);
      if (!position) return false;
      try {
        await move({ collection, position });
        return true;
      } catch (e) {
        errorToast(e, 'The collection was not moved');
        return false;
      }
    },

    async setOrder(collection: Collection, order: ChildOrder): Promise<void> {
      if (collection.child_order === order) return;
      try {
        await patch({ collection, patch: { child_order: order } });
      } catch (e) {
        errorToast(e, 'The sort order was not saved');
      }
    },

    async remove(collection: Collection): Promise<boolean> {
      const n = collection.item_count;
      const ok = await confirmAction({
        title: `Delete “${collection.name}”?`,
        message: n > 0
          ? `The collection and its ${n === 1 ? 'item move' : `${n} items move`} to Trash, where you can restore them for 30 days.`
          : 'The collection moves to Trash, where you can restore it for 30 days.',
        confirmLabel: 'Delete',
        danger: true,
      });
      if (!ok) return false;
      try {
        await remove(collection);
        toast({
          message: `Moved “${collection.name}” to Trash`,
          action: { label: 'Undo', run: () => void restore(collection.id).catch((e: unknown) => errorToast(e)) },
        });
        return true;
      } catch (e) {
        errorToast(e, 'The collection was not deleted');
        return false;
      }
    },
  }), [qc, move, patch, remove, restore]);
}
