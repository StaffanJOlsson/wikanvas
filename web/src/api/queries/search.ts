// Search (docs/ARCHITECTURE.md §7.2): `GET /search` returns `Page<SearchHit>` and is followed page by page like the
// lists; `GET /search/titles` feeds the quick switcher and mentions. No drafts filter in the web client: the API
// default `include_drafts=true` applies (D23).
import { useInfiniteQuery, useQuery, type InfiniteData, type UseQueryResult } from '@tanstack/react-query';
import type { ApiError } from '../client';
import { api } from '../client';
import type { Page } from '../gen/Page';
import type { SearchHit } from '../gen/SearchHit';
import type { SearchQuery } from '../gen/SearchQuery';
import type { TitleHit } from '../gen/TitleHit';
import { fetchPage, nextCursor, PAGE_SIZE, withQuery, type PagedResult } from './lists';

/** The search parameters the page sets (no cursor, no drafts filter). */
export type SearchParams = Omit<SearchQuery, 'cursor' | 'include_drafts'>;

export function searchKey(params: SearchParams): readonly ['search', SearchParams] {
  return ['search', params] as const;
}

export function titlesKey(q: string): readonly ['titles', string] {
  return ['titles', q] as const;
}

function queryParams(p: SearchParams) {
  return {
    q: p.q.trim(),
    collection_id: p.collection_id,
    kind: p.kind,
    updated_after: p.updated_after,
    include_archived: p.include_archived ? true : undefined,
    sort: p.sort,
    limit: p.limit ?? PAGE_SIZE,
  };
}

/** Full-text search. Disabled for an empty query (a query without a positive term finds nothing anyway). */
export function useSearch(params: SearchParams, opts: { enabled?: boolean } = {}): PagedResult<SearchHit> {
  return useInfiniteQuery<Page<SearchHit>, ApiError, InfiniteData<Page<SearchHit>, string | null>, readonly unknown[], string | null>({
    queryKey: searchKey(params),
    queryFn: ({ pageParam, signal }) => fetchPage<SearchHit>('/search', queryParams(params), pageParam, signal),
    initialPageParam: null,
    getNextPageParam: nextCursor,
    enabled: (opts.enabled ?? true) && params.q.trim() !== '',
  });
}

export function fetchTitles(q: string, limit = 10, signal?: AbortSignal): Promise<TitleHit[]> {
  return api<TitleHit[]>('GET', withQuery('/search/titles', { q: q.trim(), limit }), { signal });
}

/** Title search (`limit` 1–20, default 10). Disabled for an empty query. */
export function useTitleSearch(q: string, limit = 10): UseQueryResult<TitleHit[], ApiError> {
  return useQuery<TitleHit[], ApiError>({
    queryKey: [...titlesKey(q.trim()), limit],
    queryFn: ({ signal }) => fetchTitles(q, limit, signal),
    enabled: q.trim() !== '',
    staleTime: 10_000,
  });
}
