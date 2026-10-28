// Paginated item lists (docs/ARCHITECTURE.md §7.1 pagination, §7.2): `/recent`, `/updated`, `/starred`, `/archive` and
// `/drafts` return `Page<ListedItem>` (`items`, `next_cursor`); the hooks follow `next_cursor` page by page and stop at
// null. Also the Trash (roots only, unpaginated) and its purge routes.
import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
  type InfiniteData,
  type UseInfiniteQueryResult,
  type UseMutationResult,
  type UseQueryResult,
} from '@tanstack/react-query';
import { api, type ApiError } from '../client';
import type { ListedItem } from '../gen/ListedItem';
import type { Page } from '../gen/Page';
import type { TrashList } from '../gen/TrashList';
import { sidebarKey } from './tree';

export type ListName = 'recent' | 'updated' | 'starred' | 'archive' | 'drafts';

export const PAGE_SIZE = 25;

export function listKey(name: ListName | 'trash'): readonly ['list', string] {
  return ['list', name] as const;
}

export const trashKey = listKey('trash');

type Param = string | number | boolean | null | undefined;

/** `path?k=v&…&cursor=…` with null, undefined and empty values left out. */
export function withQuery(path: string, params: Record<string, Param>): string {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v === null || v === undefined || v === '') continue;
    q.set(k, String(v));
  }
  const s = q.toString();
  return s ? `${path}?${s}` : path;
}

/** One page of a paginated route; `cursor` null for the first page. */
export function fetchPage<T>(
  path: string,
  params: Record<string, Param>,
  cursor: string | null,
  signal?: AbortSignal,
): Promise<Page<T>> {
  return api<Page<T>>('GET', withQuery(path, { ...params, cursor }), { signal });
}

/** The next page's cursor, or undefined to stop (TanStack's convention) when `next_cursor` is null. */
export function nextCursor<T>(last: Page<T>): string | undefined {
  return last.next_cursor ?? undefined;
}

export type PagedResult<T> = UseInfiniteQueryResult<InfiniteData<Page<T>, string | null>, ApiError>;

/** Every loaded item of an infinite list, in order. */
export function flattenPages<T>(data: InfiniteData<Page<T>, string | null> | undefined): T[] {
  return data?.pages.flatMap((p) => p.items) ?? [];
}

/** One of the paginated lists. `fetchNextPage()` follows `next_cursor`; `hasNextPage` is false after a null cursor. */
export function useList(name: ListName, opts: { limit?: number; enabled?: boolean } = {}): PagedResult<ListedItem> {
  const limit = opts.limit ?? PAGE_SIZE;
  return useInfiniteQuery<Page<ListedItem>, ApiError, InfiniteData<Page<ListedItem>, string | null>, readonly unknown[], string | null>({
    queryKey: [...listKey(name), { limit }],
    queryFn: ({ pageParam, signal }) => fetchPage<ListedItem>(`/${name}`, { limit }, pageParam, signal),
    initialPageParam: null,
    getNextPageParam: nextCursor,
    enabled: opts.enabled ?? true,
  });
}

/** `GET /trash`: trashed collections and item roots, with purge dates. */
export function useTrashList(): UseQueryResult<TrashList, ApiError> {
  return useQuery<TrashList, ApiError>({
    queryKey: trashKey,
    queryFn: ({ signal }) => api<TrashList>('GET', '/trash', { signal }),
  });
}

export type PurgeTarget = { kind: 'item'; id: string } | { kind: 'collection'; id: string } | { kind: 'all' };

/** Delete forever: `DELETE /trash/items/{id}`, `/trash/collections/{id}` or `/trash` (admin scope). */
export function usePurge(): UseMutationResult<void, ApiError, PurgeTarget> {
  const qc = useQueryClient();
  return useMutation<void, ApiError, PurgeTarget>({
    mutationFn: (t) =>
      t.kind === 'all'
        ? api<void>('DELETE', '/trash')
        : api<void>('DELETE', `/trash/${t.kind === 'item' ? 'items' : 'collections'}/${encodeURIComponent(t.id)}`),
    onMutate: (t) => {
      qc.setQueryData<TrashList>(trashKey, (list) => {
        if (!list) return list;
        if (t.kind === 'all') return { collections: [], items: [] };
        if (t.kind === 'item') return { ...list, items: list.items.filter((i) => i.id !== t.id) };
        return {
          collections: list.collections.filter((c) => c.id !== t.id),
          items: list.items.filter((i) => i.collection_id !== t.id),
        };
      });
    },
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: trashKey });
      void qc.invalidateQueries({ queryKey: sidebarKey });
    },
  });
}
