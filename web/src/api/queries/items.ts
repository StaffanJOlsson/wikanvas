// Items: one item's detail, creation, stars, trash and the other per-item mutations (docs/ARCHITECTURE.md §7.2, §9.2).
// Mutations update `['tree']`, `['sidebar']` and `['item']` optimistically and roll back on error; every response
// replaces the affected rows with the server's. The signatures of `itemKey`, `fetchItem`, `useItem`,
// `updateCachedItem`, `useStar`, `useCreateItem` and the `useConfig` re-export are frozen (PLAN W0b "Produces");
// `useStar`'s variables gained the optional `position` and `item` (additive, R3).
import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
  type QueryKey,
  type UseMutationResult,
  type UseQueryResult,
} from '@tanstack/react-query';
import { useEffect } from 'react';
import { applyCreate, applyRemove } from '../../components/tree/applyMove';
import { api, type ApiError } from '../client';
import type { ItemDetail } from '../gen/ItemDetail';
import type { ItemPatch } from '../gen/ItemPatch';
import type { ItemsArchived } from '../gen/ItemsArchived';
import type { ItemSummary } from '../gen/ItemSummary';
import type { ItemTrashed } from '../gen/ItemTrashed';
import type { NewItem } from '../gen/NewItem';
import type { Position } from '../gen/Position';
import type { RestoredItem } from '../gen/RestoredItem';
import type { Sidebar } from '../gen/Sidebar';
import {
  cachedTrees,
  childrenKey,
  findCachedSummary,
  replaceCachedSummary,
  setTreeItems,
  sidebarKey,
  treeKey,
  type TreeData,
} from './tree';

export { useConfig } from './config';

const enc = encodeURIComponent;

/** The query key of one item: `['item', id]`, where `id` is the ref the page loaded it by (UUID or url_id). */
export function itemKey(ref: string): readonly ['item', string] {
  return ['item', ref] as const;
}

export function fetchItem(ref: string, signal?: AbortSignal): Promise<ItemDetail> {
  return api<ItemDetail>('GET', `/items/${encodeURIComponent(ref)}`, { signal });
}

/**
 * `GET /api/v1/items/{ref}`: `ref` is the item's UUID or its `url_id` (from `urlIdFromSlugId`). Disabled while `ref`
 * is null or undefined. Errors are `ApiError`s (`not_found` for an unknown or purged item).
 */
export function useItem(ref: string | null | undefined): UseQueryResult<ItemDetail, ApiError> {
  return useQuery<ItemDetail, ApiError>({
    queryKey: itemKey(ref ?? ''),
    queryFn: ({ signal }) => fetchItem(ref as string, signal),
    enabled: !!ref,
  });
}

/** Applies `update` to every cached `['item', ref]` entry of item `id`, whichever ref it was loaded by. */
export function updateCachedItem(qc: QueryClient, id: string, update: (d: ItemDetail) => ItemDetail): void {
  qc.setQueriesData<ItemDetail>({ queryKey: ['item'] }, (d) => (d && d.item.id === id ? update(d) : d));
}

type Snapshot = [QueryKey, unknown][];

function snapshotAll(qc: QueryClient, prefixes: QueryKey[]): Snapshot {
  return prefixes.flatMap((p) => qc.getQueriesData({ queryKey: p }));
}

function restore(qc: QueryClient, snap: Snapshot | undefined): void {
  for (const [k, v] of snap ?? []) qc.setQueryData(k, v);
}

export interface StarVars {
  itemId: string;
  starred: boolean;
  /** Among the stars (reorder, §9.6.3); omitted: last. Only with `starred: true`. */
  position?: Position;
  /** The item's summary, so a new star shows in the sidebar at once; looked up in the caches when omitted. */
  item?: ItemSummary;
}

function moveStar(stars: ItemSummary[], id: string, position: Position): ItemSummary[] {
  const moving = stars.find((s) => s.id === id);
  if (!moving) return stars;
  const rest = stars.filter((s) => s.id !== id);
  const after = position.after_id ? rest.findIndex((s) => s.id === position.after_id) : -1;
  const before = position.before_id ? rest.findIndex((s) => s.id === position.before_id) : -1;
  const at = after >= 0 ? after + 1 : before >= 0 ? before : rest.length;
  rest.splice(at, 0, moving);
  return rest;
}

/**
 * Star, unstar or reorder a star (`PUT`/`DELETE /api/v1/items/{id}/star`), optimistically in the item cache and the
 * sidebar's STARRED section.
 */
export function useStar(): UseMutationResult<void, ApiError, StarVars> {
  const qc = useQueryClient();
  return useMutation<void, ApiError, StarVars, { snap: Snapshot }>({
    mutationFn: ({ itemId, starred, position }) =>
      starred
        ? api<void>('PUT', `/items/${encodeURIComponent(itemId)}/star`, { json: position ? { position } : {} })
        : api<void>('DELETE', `/items/${encodeURIComponent(itemId)}/star`),
    onMutate: async ({ itemId, starred, position, item }) => {
      await qc.cancelQueries({ queryKey: sidebarKey });
      const snap = snapshotAll(qc, [sidebarKey, ['item']]);
      updateCachedItem(qc, itemId, (d) => ({ ...d, starred }));
      const summary = item ?? findCachedSummary(qc, itemId);
      qc.setQueryData<Sidebar>(sidebarKey, (s) => {
        if (!s) return s;
        if (!starred) return { ...s, stars: s.stars.filter((x) => x.id !== itemId) };
        if (s.stars.some((x) => x.id === itemId)) {
          return position ? { ...s, stars: moveStar(s.stars, itemId, position) } : s;
        }
        if (!summary) return s;
        const stars = [...s.stars, summary];
        return { ...s, stars: position ? moveStar(stars, itemId, position) : stars };
      });
      return { snap };
    },
    onError: (_e, _v, ctx) => restore(qc, ctx?.snap),
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: sidebarKey });
      void qc.invalidateQueries({ queryKey: ['list', 'starred'] });
    },
  });
}

/**
 * `POST /api/v1/items`. Resolves with the created (published, D23) `ItemSummary`, which is added to the cached tree at
 * once; refreshes the collection's tree, the parent's children and the sidebar.
 */
export function useCreateItem(): UseMutationResult<ItemSummary, ApiError, NewItem> {
  const qc = useQueryClient();
  return useMutation<ItemSummary, ApiError, NewItem>({
    mutationFn: (item) => api<ItemSummary>('POST', '/items', { json: item }),
    onSuccess: (created) => {
      const t = qc.getQueryData<TreeData>(treeKey(created.collection_id));
      if (t) setTreeItems(qc, created.collection_id, applyCreate(t.items, created));
      void qc.invalidateQueries({ queryKey: ['tree', created.collection_id] });
      void qc.invalidateQueries({ queryKey: ['sidebar'] });
      if (created.parent_id !== null) {
        void qc.invalidateQueries({ queryKey: ['children', created.parent_id] });
        void qc.invalidateQueries({ queryKey: ['item', created.parent_id] });
      }
    },
  });
}

/** Removes an item's subtree from every cached tree and from the stars (trash and archive). */
function removeEverywhere(qc: QueryClient, itemId: string): void {
  for (const [cid, t] of cachedTrees(qc)) {
    if (t.items.some((i) => i.id === itemId)) setTreeItems(qc, cid, applyRemove(t.items, itemId));
  }
  qc.setQueryData<Sidebar>(sidebarKey, (s) => (s ? { ...s, stars: s.stars.filter((x) => x.id !== itemId) } : s));
}

function afterStructureChange(qc: QueryClient, item: Pick<ItemSummary, 'id' | 'collection_id' | 'parent_id'>): void {
  void qc.invalidateQueries({ queryKey: treeKey(item.collection_id) });
  void qc.invalidateQueries({ queryKey: sidebarKey });
  void qc.invalidateQueries({ queryKey: ['list'] });
  void qc.invalidateQueries({ queryKey: ['item'] });
  if (item.parent_id !== null) void qc.invalidateQueries({ queryKey: childrenKey(item.parent_id) });
}

/** `DELETE /items/{id}`: moves the item and its subtree to Trash, optimistically. */
export function useTrash(): UseMutationResult<ItemTrashed, ApiError, ItemSummary> {
  const qc = useQueryClient();
  return useMutation<ItemTrashed, ApiError, ItemSummary, { snap: Snapshot }>({
    mutationFn: (item) => api<ItemTrashed>('DELETE', `/items/${enc(item.id)}`),
    onMutate: async (item) => {
      await qc.cancelQueries({ queryKey: ['tree'] });
      const snap = snapshotAll(qc, [['tree'], sidebarKey]);
      removeEverywhere(qc, item.id);
      return { snap };
    },
    onError: (_e, _v, ctx) => restore(qc, ctx?.snap),
    onSettled: (_r, _e, item) => afterStructureChange(qc, item),
  });
}

/** `POST /items/{id}/archive`: hides the subtree from the tree, optimistically. */
export function useArchiveItem(): UseMutationResult<ItemsArchived, ApiError, ItemSummary> {
  const qc = useQueryClient();
  return useMutation<ItemsArchived, ApiError, ItemSummary, { snap: Snapshot }>({
    mutationFn: (item) => api<ItemsArchived>('POST', `/items/${enc(item.id)}/archive`),
    onMutate: async (item) => {
      await qc.cancelQueries({ queryKey: ['tree'] });
      const snap = snapshotAll(qc, [['tree'], sidebarKey]);
      removeEverywhere(qc, item.id);
      return { snap };
    },
    onError: (_e, _v, ctx) => restore(qc, ctx?.snap),
    onSettled: (_r, _e, item) => afterStructureChange(qc, item),
  });
}

/** `POST /items/{id}/unarchive` (+ `moved_to_top` when its parent is gone). */
export function useUnarchiveItem(): UseMutationResult<RestoredItem, ApiError, Pick<ItemSummary, 'id' | 'collection_id' | 'parent_id'>> {
  const qc = useQueryClient();
  return useMutation<RestoredItem, ApiError, Pick<ItemSummary, 'id' | 'collection_id' | 'parent_id'>>({
    mutationFn: (item) => api<RestoredItem>('POST', `/items/${enc(item.id)}/unarchive`),
    onSettled: (_r, _e, item) => afterStructureChange(qc, item),
  });
}

/** `POST /items/{id}/restore` from Trash (+ `moved_to_top`; 409 `parent_invalid` while its collection is deleted). */
export function useRestoreItem(): UseMutationResult<RestoredItem, ApiError, Pick<ItemSummary, 'id' | 'collection_id' | 'parent_id'>> {
  const qc = useQueryClient();
  return useMutation<RestoredItem, ApiError, Pick<ItemSummary, 'id' | 'collection_id' | 'parent_id'>>({
    mutationFn: (item) => api<RestoredItem>('POST', `/items/${enc(item.id)}/restore`),
    onSettled: (_r, _e, item) => afterStructureChange(qc, item),
  });
}

/** `POST /items/{id}/duplicate` (201): a sibling copy without children. */
export function useDuplicateItem(): UseMutationResult<ItemSummary, ApiError, ItemSummary> {
  const qc = useQueryClient();
  return useMutation<ItemSummary, ApiError, ItemSummary>({
    mutationFn: (item) => api<ItemSummary>('POST', `/items/${enc(item.id)}/duplicate`),
    onSuccess: (copy) => {
      const t = qc.getQueryData<TreeData>(treeKey(copy.collection_id));
      if (t) setTreeItems(qc, copy.collection_id, applyCreate(t.items, copy));
    },
    onSettled: (copy, _e, item) => afterStructureChange(qc, copy ?? item),
  });
}

export interface PatchItemVars {
  item: ItemSummary;
  patch: ItemPatch;
}

/** `PATCH /items/{id}` (title, full width; no If-Match: the last write wins), optimistic for the title. */
export function usePatchItem(): UseMutationResult<ItemSummary, ApiError, PatchItemVars> {
  const qc = useQueryClient();
  return useMutation<ItemSummary, ApiError, PatchItemVars, { snap: Snapshot }>({
    mutationFn: ({ item, patch }) => api<ItemSummary>('PATCH', `/items/${enc(item.id)}`, { json: patch }),
    onMutate: async ({ item, patch }) => {
      const fullWidth = patch.full_width;
      // A detail fetch already running would overwrite the optimistic `full_width` with the old value.
      if (fullWidth != null) await qc.cancelQueries({ queryKey: ['item'] });
      const snap = snapshotAll(qc, [['tree'], sidebarKey, ['item']]);
      if (patch.title != null) replaceCachedSummary(qc, { ...item, title: patch.title });
      if (fullWidth != null) updateCachedItem(qc, item.id, (d) => ({ ...d, full_width: fullWidth }));
      return { snap };
    },
    onError: (_e, _v, ctx) => restore(qc, ctx?.snap),
    onSuccess: (row) => replaceCachedSummary(qc, row),
    onSettled: (_r, _e, { item }) => {
      void qc.invalidateQueries({ queryKey: treeKey(item.collection_id) });
      void qc.invalidateQueries({ queryKey: ['item'] });
    },
  });
}

/**
 * Records one view of the item when a page opens it (`POST /items/{id}/view`, the only source of `/recent`, §7.2):
 * once per mount and item, fire and forget (a failure only leaves Recently viewed as it was), then the Recently viewed
 * list refetches.
 */
export function useRecordView(id: string): void {
  const qc = useQueryClient();
  useEffect(() => {
    api<void>('POST', `/items/${enc(id)}/view`).then(
      () => void qc.invalidateQueries({ queryKey: ['list', 'recent'] }),
      () => undefined,
    );
  }, [id, qc]);
}
