// The item actions of the sidebar rows, the tree keyboard and the item menu (docs/ARCHITECTURE.md §9.6.2–§9.6.4),
// with their confirmations and toasts: create, rename, move (with "Moved to … · Undo"), duplicate, star, archive and
// trash (with Undo). The query hooks underneath do the optimistic updates and rollbacks.
import { useQueryClient, type QueryClient } from '@tanstack/react-query';
import { useMemo } from 'react';
import { useNavigate } from 'react-router';
import type { Collection } from '../../api/gen/Collection';
import type { ItemKind } from '../../api/gen/ItemKind';
import type { ItemSummary } from '../../api/gen/ItemSummary';
import type { Sidebar } from '../../api/gen/Sidebar';
import {
  useArchiveItem,
  useCreateItem,
  useDuplicateItem,
  usePatchItem,
  useRestoreItem,
  useStar,
  useTrash,
  useUnarchiveItem,
} from '../../api/queries/items';
import { findCachedSummary, sidebarKey, treeKey, useMoveItem, type TreeData } from '../../api/queries/tree';
import { itemPath } from '../../lib/urls';
import { confirmAction } from '../../state/dialogs';
import { expandAll } from '../../state/expanded';
import { errorToast, toast } from '../../state/toasts';
import { buildTree, descendantIds, type Forest } from '../tree/buildTree';
import { currentPlace, dropToMove, type Move } from '../tree/positions';
import type { DropInstruction } from '../tree/drop';

/** The cached tree of a collection as a Forest, or undefined when it is not loaded. */
export function cachedForest(qc: QueryClient, collectionId: string): Forest | undefined {
  const t = qc.getQueryData<TreeData>(treeKey(collectionId));
  if (!t) return undefined;
  const order = qc.getQueryData<Sidebar>(sidebarKey)?.collections.find((c) => c.id === collectionId)?.child_order
    ?? t.collection.child_order;
  return buildTree(collectionId, t.items, order);
}

/** Every cached forest, by collection id. */
export function cachedForests(qc: QueryClient): Map<string, Forest | undefined> {
  const out = new Map<string, Forest | undefined>();
  for (const [key] of qc.getQueriesData<TreeData>({ queryKey: ['tree'] })) {
    const cid = key[1];
    if (typeof cid === 'string') out.set(cid, cachedForest(qc, cid));
  }
  return out;
}

function collectionOf(qc: QueryClient, id: string): Collection | undefined {
  return qc.getQueryData<Sidebar>(sidebarKey)?.collections.find((c) => c.id === id);
}

/** "the parent's title" or the collection name, for "Moved to …". */
function placeName(qc: QueryClient, to: Move): string {
  if (to.parent_id) {
    const p = findCachedSummary(qc, to.parent_id);
    return p ? p.title || 'Untitled' : 'its new place';
  }
  return collectionOf(qc, to.collection_id)?.name ?? 'the collection';
}

export interface MoveOptions {
  /** Show "Moved to … · Undo" (8 s). Default true. */
  undoable?: boolean;
}

export function useItemActions() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  // mutateAsync is stable for the life of each hook, so the actions object is too.
  const move = useMoveItem().mutateAsync;
  const trash = useTrash().mutateAsync;
  const archive = useArchiveItem().mutateAsync;
  const unarchive = useUnarchiveItem().mutateAsync;
  const restore = useRestoreItem().mutateAsync;
  const duplicate = useDuplicateItem().mutateAsync;
  const star = useStar().mutateAsync;
  const patch = usePatchItem().mutateAsync;
  const create = useCreateItem().mutateAsync;

  return useMemo(() => {
    const moveTo = async (item: ItemSummary, to: Move, opts: MoveOptions = {}): Promise<ItemSummary | null> => {
      const back = currentPlace(cachedForest(qc, item.collection_id), item);
      try {
        const moved = await move({ item, to });
        if (to.parent_id) expandAll([to.collection_id, to.parent_id]);
        if (opts.undoable ?? true) {
          toast({
            message: `Moved to ${placeName(qc, to)}`,
            action: { label: 'Undo', run: () => void moveTo(moved, back, { undoable: false }) },
          });
        }
        return moved;
      } catch (e) {
        errorToast(e, 'The item was not moved');
        return null;
      }
    };

    return {
      moveTo,

      /** Performs a drop instruction (sidebar drag and drop, keyboard moves); false when it changed nothing. */
      async drop(item: ItemSummary, instruction: DropInstruction, opts: MoveOptions = {}): Promise<boolean> {
        const to = dropToMove(instruction, item, cachedForests(qc));
        if (!to) return false;
        return (await moveTo(item, to, opts)) !== null;
      },

      async create(collectionId: string, parentId: string | null, kind: ItemKind): Promise<ItemSummary | null> {
        try {
          const created = await create({ collection_id: collectionId, parent_id: parentId, kind });
          expandAll(parentId ? [collectionId, parentId] : [collectionId]);
          void navigate(itemPath(created));
          return created;
        } catch (e) {
          errorToast(e, `The ${kind === 'board' ? 'board' : 'document'} was not created`);
          return null;
        }
      },

      async rename(item: ItemSummary, title: string): Promise<void> {
        const next = title.trim();
        if (next === item.title) return;
        try {
          await patch({ item, patch: { title: next } });
        } catch (e) {
          errorToast(e, 'The title was not saved');
        }
      },

      async duplicate(item: ItemSummary): Promise<void> {
        try {
          const copy = await duplicate(item);
          void navigate(itemPath(copy));
        } catch (e) {
          errorToast(e, 'The item was not duplicated');
        }
      },

      async setStarred(item: ItemSummary, starred: boolean): Promise<void> {
        try {
          await star({ itemId: item.id, starred, item });
        } catch (e) {
          errorToast(e);
        }
      },

      async archive(item: ItemSummary): Promise<boolean> {
        try {
          const res = await archive(item);
          toast({
            message: res.archived > 1 ? `Archived with ${res.archived - 1} nested` : 'Archived',
            action: { label: 'Undo', run: () => void unarchive(item).catch((e: unknown) => errorToast(e)) },
          });
          return true;
        } catch (e) {
          errorToast(e, 'The item was not archived');
          return false;
        }
      },

      /** Moves to Trash; asks first when the item has nested pages (§9.6.3). Resolves false when cancelled. */
      async trash(item: ItemSummary): Promise<boolean> {
        const forest = cachedForest(qc, item.collection_id);
        const nested = forest?.byId.has(item.id) ? descendantIds(forest, item.id).size : item.child_count;
        if (nested > 0) {
          const ok = await confirmAction({
            title: `Move “${item.title || 'Untitled'}” to Trash?`,
            message: `Its ${nested === 1 ? 'nested page goes' : `${nested} nested pages go`} with it. You can restore them from Trash.`,
            confirmLabel: 'Move to Trash',
            danger: true,
          });
          if (!ok) return false;
        }
        try {
          await trash(item);
          toast({
            message: 'Moved to Trash',
            action: { label: 'Undo', run: () => void restore(item).catch((e: unknown) => errorToast(e)) },
          });
          return true;
        } catch (e) {
          errorToast(e, 'The item was not moved to Trash');
          return false;
        }
      },
    };
  }, [qc, navigate, move, trash, archive, unarchive, restore, duplicate, star, patch, create]);
}

export type ItemActions = ReturnType<typeof useItemActions>;
