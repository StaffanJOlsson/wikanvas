import { act, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { MoveItem } from '../../api/gen/MoveItem';
import { treeKey, type TreeData } from '../../api/queries/tree';
import { makeCollection, mockApi, treeReply } from '../../test/mockApi';
import { renderSidebar, waitForRows } from '../../test/sidebarHarness';
import { items } from '../../test/treeFixtures';
import { buildTree } from '../tree/buildTree';
import { resolveItemDrop } from '../tree/drop';
import { visibleRows } from '../tree/visibleRows';

describe('drop on a collection row', () => {
  const c1 = makeCollection({ id: 'c1', name: 'Work', item_count: 3 });
  const c2 = makeCollection({ id: 'c2', name: 'Home', item_count: 1 });
  const list1 = items('c1', 'A', 'A>A1', 'B');
  const list2 = items('c2', 'X');

  it('resolves to the end of that collection, from any collection', () => {
    const forests = new Map([['c1', buildTree('c1', list1, 'manual')], ['c2', buildTree('c2', list2, 'manual')]]);
    const rows = visibleRows({ collections: [c1, c2], forests, expanded: new Set(['c1', 'c2']) });
    const index = rows.findIndex((r) => r.key === 'c2');
    for (const clientY of [1, 20, 39]) {
      expect(resolveItemDrop({ rows, index, dragged: { id: 'A', collectionId: 'c1' }, forests, clientX: 0, clientY, rect: { top: 0, height: 40, left: 0 } }))
        .toEqual({ instruction: { type: 'collection', collectionId: 'c2' }, indicator: { rowKey: 'c2', kind: 'box', depth: 0 } });
    }
  });

  it('moves the item with its subtree to the top level of the other collection (cross-collection)', async () => {
    let l1 = [...list1];
    let l2 = [...list2];
    const api = mockApi();
    const { probe, queryClient } = renderSidebar({ api, collections: [c1, c2], expanded: ['c1', 'c2', 'A'] });
    api.on('GET', '/collections/c1/tree', () => treeReply(c1, l1, `"t${l1.length}"`));
    api.on('GET', '/collections/c2/tree', () => treeReply(c2, l2, `"t${l2.length}"`));
    await waitForRows(['Work', 'A', 'A1', 'B', 'Home', 'X']);
    const answer = deferred<void>();
    api.on('POST', '/items/:id/move', async (req) => {
      await answer.promise;
      const body = req.body as MoveItem;
      const moved = l1.filter((i) => i.id === 'A' || i.id === 'A1').map((i) => ({ ...i, collection_id: body.collection_id! }));
      l1 = l1.filter((i) => i.id !== 'A' && i.id !== 'A1');
      l2 = [...l2, ...moved.map((i) => (i.id === 'A' ? { ...i, sort_key: 'a9' } : i))];
      return [200, l2.find((i) => i.id === 'A')!];
    });

    let done!: Promise<boolean>;
    act(() => {
      done = probe.actions.drop(list1[0]!, { type: 'collection', collectionId: 'c2' });
    });
    // Optimistic, before the server answers: A and A1 left c1 and sit at the end of c2.
    await waitForRows(['Work', 'B', 'Home', 'X', 'A', 'A1']);
    expect(queryClient.getQueryData<TreeData>(treeKey('c1'))!.items.map((i) => i.id)).toEqual(['B']);
    expect(queryClient.getQueryData<TreeData>(treeKey('c2'))!.items.filter((i) => i.collection_id === 'c2').map((i) => i.id))
      .toEqual(['X', 'A', 'A1']);
    expect(api.calls('POST', '/items/:id/move')[0]!).toMatchObject({
      params: { id: 'A' },
      body: { collection_id: 'c2', parent_id: null },
    });

    answer.resolve();
    await act(async () => {
      expect(await done).toBe(true);
    });
    expect((await screen.findByRole('status')).textContent).toContain('Moved to Home');
    await waitForRows(['Work', 'B', 'Home', 'X', 'A', 'A1']);
  });
});

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}
