// The paginated hooks follow `next_cursor` page by page and stop at null (docs/ARCHITECTURE.md §7.1, D46).
import { QueryClientProvider } from '@tanstack/react-query';
import { act, renderHook, waitFor } from '@testing-library/react';
import { createElement, type ReactNode } from 'react';
import { describe, expect, it } from 'vitest';
import { makeItem, mockApi, type MockApi } from '../test/mockApi';
import { createTestQueryClient } from '../test/render';
import { flattenPages, useList, type ListName } from './queries/lists';
import { useSearch } from './queries/search';

function wrapper({ children }: { children: ReactNode }) {
  return createElement(QueryClientProvider, { client: createTestQueryClient() }, children);
}

/** Three pages behind `path`: cursors none → c1 → c2 → null. */
function paged(api: MockApi, path: string, wrap: (id: string) => unknown) {
  const pages: Record<string, [string[], string | null]> = {
    '': [['a', 'b'], 'c1'],
    c1: [['c', 'd'], 'c2'],
    c2: [['e'], null],
  };
  api.on('GET', path, (req) => {
    const [ids, next] = pages[req.query.get('cursor') ?? '']!;
    return [200, { items: ids.map(wrap), next_cursor: next }];
  });
}

async function drain(result: { current: { hasNextPage: boolean; isFetching: boolean; fetchNextPage(): Promise<unknown> } }) {
  await waitFor(() => expect(result.current.isFetching).toBe(false));
  while (result.current.hasNextPage) {
    await act(async () => {
      await result.current.fetchNextPage();
    });
    await waitFor(() => expect(result.current.isFetching).toBe(false));
  }
}

describe('paginated hooks', () => {
  it.each<ListName>(['recent', 'updated', 'starred', 'archive'])('/%s follows next_cursor and stops at null', async (name) => {
    const api = mockApi();
    paged(api, `/${name}`, (id) => ({ ...makeItem({ id, collection_id: 'c1' }), path: [] }));
    const { result } = renderHook(() => useList(name), { wrapper });
    await drain(result);
    expect(flattenPages(result.current.data).map((i) => i.id)).toEqual(['a', 'b', 'c', 'd', 'e']);
    const calls = api.calls('GET', `/${name}`);
    expect(calls.map((c) => c.query.get('cursor'))).toEqual([null, 'c1', 'c2']);
    expect(calls.every((c) => c.query.get('limit') === '25')).toBe(true);
    expect(result.current.hasNextPage).toBe(false);
  });

  it('useSearch reads Page<SearchHit>, keeps its parameters on every page, and stops at null', async () => {
    const api = mockApi();
    paged(api, '/search', (id) => ({ item: makeItem({ id, collection_id: 'c1' }), path: [], snippet: [], score: 1 }));
    const { result } = renderHook(() => useSearch({ q: 'plan', kind: 'doc', sort: 'updated' }), { wrapper });
    await drain(result);
    expect(flattenPages(result.current.data).map((h) => h.item.id)).toEqual(['a', 'b', 'c', 'd', 'e']);
    const calls = api.calls('GET', '/search');
    expect(calls).toHaveLength(3);
    for (const c of calls) {
      expect(c.query.get('q')).toBe('plan');
      expect(c.query.get('kind')).toBe('doc');
      expect(c.query.get('sort')).toBe('updated');
      // No drafts filter in the web client: the API default applies (D23).
      expect(c.query.has('include_drafts')).toBe(false);
    }
    expect(calls.map((c) => c.query.get('cursor'))).toEqual([null, 'c1', 'c2']);
  });

  it('an empty query does not search', async () => {
    const api = mockApi();
    const { result } = renderHook(() => useSearch({ q: '   ' }), { wrapper });
    await act(async () => undefined);
    expect(result.current.fetchStatus).toBe('idle');
    expect(api.calls()).toHaveLength(0);
  });
});
