// The Move dialog (§9.3, §9.6.2 "Move…"; §9.6.3: the item and its descendants are invalid targets): every collection's
// top level and every other item are offered; picking one moves the item last under it.
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { ItemSummary } from '../../api/gen/ItemSummary';
import type { MoveItem } from '../../api/gen/MoveItem';
import { openDialog } from '../../state/dialogs';
import { makeCollection, makeSidebar, mockApi, treeReply } from '../../test/mockApi';
import { render } from '../../test/render';
import { items } from '../../test/treeFixtures';
import { Overlays } from './Overlays';

const c1 = makeCollection({ id: 'c1', name: 'Docs', item_count: 5 });
const c2 = makeCollection({ id: 'c2', name: 'Notes', item_count: 1 });
// Docs: A [A1 [A1a], A2], B — Notes: X
const t1 = items('c1', 'A', 'A>A1', 'A1>A1a', 'A>A2', 'B');
const t2 = items('c2', 'X');
const item = (id: string) => [...t1, ...t2].find((i) => i.id === id)!;

function setup(moving: ItemSummary) {
  const api = mockApi()
    .on('GET', '/sidebar', [200, makeSidebar([c1, c2])])
    .on('GET', '/collections/c1/tree', treeReply(c1, t1))
    .on('GET', '/collections/c2/tree', treeReply(c2, t2))
    .on('POST', '/items/:id/move', (req) => [200, { ...item(req.params.id!), ...(req.body as MoveItem), sort_key: 'z' }]);
  render(<Overlays />);
  act(() => openDialog({ kind: 'move', item: moving }));
  return api;
}

/** The offered places, by label. */
function places(): string[] {
  return screen.queryAllByRole('option').map((o) => o.lastElementChild?.textContent ?? '');
}

describe('Move dialog', () => {
  it('offers every collection top level and every item except the item itself and its descendants', async () => {
    setup(item('A1'));
    expect(screen.getByRole('dialog', { name: 'Move “A1”' })).toBeTruthy();
    await waitFor(() => expect(places()).toEqual(['Docs (top level)', 'A', 'A2', 'B', 'Notes (top level)', 'X']));

    // Its current parent is offered but changes nothing.
    fireEvent.click(screen.getAllByRole('option').find((o) => o.lastElementChild?.textContent === 'A')!);
    expect((screen.getByRole('button', { name: 'Move' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('a top-level item with a subtree: none of the subtree is offered; the filter narrows the list', async () => {
    setup(item('A'));
    await waitFor(() => expect(places()).toEqual(['Docs (top level)', 'B', 'Notes (top level)', 'X']));
    fireEvent.change(screen.getByRole('textbox', { name: 'Filter places' }), { target: { value: 'not' } });
    expect(places()).toEqual(['Notes (top level)']);
  });

  it('moving sends the new parent without a position (last), across collections too', async () => {
    const api = setup(item('A1'));
    await waitFor(() => expect(places()).toContain('X'));
    fireEvent.click(screen.getAllByRole('option').find((o) => o.lastElementChild?.textContent === 'X')!);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Move' }));
    });
    await waitFor(() => expect(api.calls('POST', '/items/:id/move')).toHaveLength(1));
    expect(api.calls('POST', '/items/:id/move')[0]).toMatchObject({
      params: { id: 'A1' },
      body: { collection_id: 'c2', parent_id: 'X' },
    });
    expect(api.calls('POST', '/items/:id/move')[0]!.body).not.toHaveProperty('position');
    expect(screen.queryByRole('dialog', { name: 'Move “A1”' })).toBeNull();
    expect((await screen.findByRole('status')).textContent).toContain('Moved to X');
  });

  it('offers only the collections the caller may change (D152)', async () => {
    const c3 = makeCollection({ id: 'c3', name: 'Shared', item_count: 1, my_access: 'read' });
    const c4 = makeCollection({ id: 'c4', name: 'Team', item_count: 0, my_access: 'write' });
    const api = mockApi()
      .on('GET', '/sidebar', [200, makeSidebar([c1, c3, c4])])
      .on('GET', '/collections/c1/tree', treeReply(c1, t1))
      .on('GET', '/collections/c3/tree', treeReply(c3, items('c3', 'S')))
      .on('GET', '/collections/c4/tree', treeReply(c4, []));
    render(<Overlays />);
    act(() => openDialog({ kind: 'move', item: item('B') }));
    await waitFor(() => expect(places()).toEqual(['Docs (top level)', 'A', 'A1', 'A1a', 'A2', 'Team (top level)']));
    // The read-only collection's tree is not even loaded.
    expect(api.calls('GET', '/collections/c3/tree')).toHaveLength(0);
  });
});
