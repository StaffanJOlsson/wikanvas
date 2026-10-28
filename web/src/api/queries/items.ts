// Items: one item's detail and item creation (docs/ARCHITECTURE.md §7.2, §9.2). W3 completes the optimistic updates
// of §9.2; the signatures here are frozen (PLAN W0b "Produces").
import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
  type UseMutationResult,
  type UseQueryResult,
} from '@tanstack/react-query';
import { api, type ApiError } from '../client';
import type { ItemDetail } from '../gen/ItemDetail';
import type { ItemSummary } from '../gen/ItemSummary';
import type { NewItem } from '../gen/NewItem';

export { useConfig } from './config';

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

/**
 * Star or unstar an item (`PUT`/`DELETE /api/v1/items/{id}/star`), optimistically in the item cache. W3 completes it
 * (sidebar and star-list updates, position).
 */
export function useStar(): UseMutationResult<void, ApiError, { itemId: string; starred: boolean }> {
  const qc = useQueryClient();
  return useMutation<void, ApiError, { itemId: string; starred: boolean }>({
    mutationFn: ({ itemId, starred }) =>
      starred
        ? api<void>('PUT', `/items/${encodeURIComponent(itemId)}/star`, { json: {} })
        : api<void>('DELETE', `/items/${encodeURIComponent(itemId)}/star`),
    onMutate: ({ itemId, starred }) => updateCachedItem(qc, itemId, (d) => ({ ...d, starred })),
    onError: (_e, { itemId, starred }) => updateCachedItem(qc, itemId, (d) => ({ ...d, starred: !starred })),
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: ['sidebar'] });
      void qc.invalidateQueries({ queryKey: ['list', 'starred'] });
    },
  });
}

/**
 * `POST /api/v1/items`. Resolves with the created (published, D23) `ItemSummary`; refreshes the collection's tree, the
 * parent's children and the sidebar.
 */
export function useCreateItem(): UseMutationResult<ItemSummary, ApiError, NewItem> {
  const qc = useQueryClient();
  return useMutation<ItemSummary, ApiError, NewItem>({
    mutationFn: (item) => api<ItemSummary>('POST', '/items', { json: item }),
    onSuccess: (created) => {
      void qc.invalidateQueries({ queryKey: ['tree', created.collection_id] });
      void qc.invalidateQueries({ queryKey: ['sidebar'] });
      if (created.parent_id !== null) {
        void qc.invalidateQueries({ queryKey: ['children', created.parent_id] });
        void qc.invalidateQueries({ queryKey: ['item', created.parent_id] });
      }
    },
  });
}
