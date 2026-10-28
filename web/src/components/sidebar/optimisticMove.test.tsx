import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { ItemSummary } from '../../api/gen/ItemSummary';
import type { MoveItem } from '../../api/gen/MoveItem';
import { errorReply, makeCollection, mockApi, treeReply, type MockReply } from '../../test/mockApi';
import { renderSidebar, waitForRows } from '../../test/sidebarHarness';
import { items } from '../../test/treeFixtures';

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

const c1 = makeCollection({ id: 'c1', item_count: 4 });

/** A tree mock that follows the moves the test accepts. */
function setup() {
  let list: ItemSummary[] = items('c1', 'A', 'A>A1', 'B', 'C');
  const api = mockApi();
  const view = renderSidebar({ api, collections: [c1], expanded: ['c1', 'A'] });
  api.on('GET', '/collections/c1/tree', () => treeReply(c1, list, `"t${list.map((i) => i.parent_id).join()}"`));
  const accept = (id: string, move: MoveItem): ItemSummary => {
    const sibs = list.filter((i) => i.parent_id === move.parent_id && i.id !== id).map((i) => i.sort_key).sort();
    const moved = { ...list.find((i) => i.id === id)!, parent_id: move.parent_id, sort_key: `${sibs[sibs.length - 1] ?? 'a'}x` };
    list = list.map((i) => (i.id === id ? moved : i));
    return moved;
  };
  return { ...view, api, accept, item: (id: string) => list.find((i) => i.id === id)! };
}

describe('optimistic moves', () => {
  it('moves at once, then rolls back with an error toast on 409 cycle', async () => {
    const { api, probe, item } = setup();
    await waitForRows(['c1', 'A', 'A1', 'B', 'C']);
    const answer = deferred<MockReply>();
    api.on('POST', '/items/:id/move', () => answer.promise);

    let done!: Promise<boolean>;
    act(() => {
      done = probe.actions.drop(item('C'), { type: 'child', targetId: 'A' });
    });
    // Optimistic: C is A's last child before the server answers.
    await waitForRows(['c1', 'A', 'A1', 'C', 'B']);
    expect(screen.getByRole('treeitem', { name: 'C' }).getAttribute('aria-level')).toBe('3');
    expect(api.calls('POST', '/items/:id/move')[0]!.body).toEqual({ collection_id: 'c1', parent_id: 'A' });

    answer.resolve(errorReply(409, 'cycle', 'Would create a cycle'));
    await act(async () => {
      expect(await done).toBe(false);
    });
    await waitForRows(['c1', 'A', 'A1', 'B', 'C']);
    expect(screen.getByRole('treeitem', { name: 'C' }).getAttribute('aria-level')).toBe('2');
    expect((await screen.findByRole('alert')).textContent).toMatch(/can't be moved into itself/);
  });

  it('shows "Moved to <parent> · Undo"; Undo moves it back to its old parent and position', async () => {
    const { api, probe, item, accept } = setup();
    await waitForRows(['c1', 'A', 'A1', 'B', 'C']);
    api.on('POST', '/items/:id/move', (req) => [200, accept(req.params.id!, req.body as MoveItem)]);

    await act(async () => {
      expect(await probe.actions.drop(item('C'), { type: 'before', targetId: 'A1' })).toBe(true);
    });
    expect(api.calls('POST', '/items/:id/move')[0]!.body).toEqual({
      collection_id: 'c1', parent_id: 'A', position: { after_id: null, before_id: 'A1' },
    });
    const toast = await screen.findByRole('status');
    expect(toast.textContent).toContain('Moved to A');
    await waitFor(() => expect(screen.getByRole('treeitem', { name: 'C' }).getAttribute('aria-level')).toBe('3'));

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Undo' }));
    });
    await waitFor(() => expect(api.calls('POST', '/items/:id/move')).toHaveLength(2));
    // C was the last top-level item, after B.
    expect(api.calls('POST', '/items/:id/move')[1]!.body).toEqual({
      collection_id: 'c1', parent_id: null, position: { after_id: 'B', before_id: null },
    });
    await waitFor(() => expect(screen.getByRole('treeitem', { name: 'C' }).getAttribute('aria-level')).toBe('2'));
    // The Undo itself offers no further Undo.
    expect(screen.queryByRole('button', { name: 'Undo' })).toBeNull();
  });

  it('a drop that changes nothing sends no request', async () => {
    const { api, probe, item } = setup();
    await waitForRows(['c1', 'A', 'A1', 'B', 'C']);
    await act(async () => {
      expect(await probe.actions.drop(item('C'), { type: 'after', targetId: 'B' })).toBe(false);
    });
    // A1, A's first child, dropped on A's "after" zone (before its own first child, i.e. before itself).
    await act(async () => {
      expect(await probe.actions.drop(item('A1'), { type: 'before', targetId: 'A1' })).toBe(false);
    });
    expect(api.calls('POST')).toHaveLength(0);
    expect(screen.queryByRole('status')).toBeNull();
    await waitForRows(['c1', 'A', 'A1', 'B', 'C']);
  });
});
