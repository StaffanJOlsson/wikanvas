import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { CollectionAccess } from '../api/gen/CollectionAccess';
import type { TrashList } from '../api/gen/TrashList';
import { Overlays } from '../components/overlays/Overlays';
import type { Role } from '../api/gen/Role';
import { makeCollection, makeItem, makeMe, makeSidebar, mockApi } from '../test/mockApi';
import { render } from '../test/render';
import Trash, { purgeText } from './Trash';

const DAY = 86_400_000;
const c1 = makeCollection({ id: 'c1', name: 'Handbook' });
const inDays = (n: number) => new Date(Date.now() + n * DAY - 60_000).toISOString();

function trashList(): TrashList {
  return {
    collections: [{ ...makeCollection({ id: 'old', name: 'Old projects', item_count: 7 }), deleted_at: inDays(-1), purge_at: inDays(29) }],
    items: [
      { ...makeItem({ id: 'big', collection_id: 'c1', title: 'Roadmap' }), deleted_at: inDays(-2), purge_at: inDays(28), descendant_count: 3, path: [{ id: 'p', url_id: 'p000000000', kind: 'doc', title: 'Plans' }] },
      { ...makeItem({ id: 'one', collection_id: 'c1', title: 'Scratch' }), deleted_at: inDays(-3), purge_at: inDays(1), descendant_count: 0, path: [] },
    ],
  };
}

function setup(role: Role = 'owner', access: CollectionAccess = 'manage', list: TrashList = trashList()) {
  const api = mockApi()
    .on('GET', '/auth/me', [200, makeMe({ user: { ...makeMe().user, role } })])
    .on('GET', '/sidebar', [200, makeSidebar([{ ...c1, my_access: access }])])
    .on('GET', '/trash', [200, list])
    .on('POST', '/items/:id/restore', (req) => [200, { ...makeItem({ id: req.params.id!, collection_id: 'c1' }), moved_to_top: false }])
    .on('DELETE', '/trash/items/:id', [204])
    .on('DELETE', '/trash/collections/:id', [204])
    .on('DELETE', '/trash', [204]);
  render(<><Trash /><Overlays /></>, { route: '/trash' });
  return api;
}

const row = async (name: RegExp) => (await screen.findByText(name)).closest('li') as HTMLElement;

describe('Trash', () => {
  it('a member or viewer never deletes forever (D148); a member with write restores', async () => {
    for (const [role, access] of [['member', 'write'], ['viewer', 'read']] as const) {
      const api = setup(role, access);
      const big = await row(/^Roadmap/);
      await waitFor(() => expect(within(big).queryByRole('button', { name: 'Delete forever' })).toBeNull());
      if (access === 'write') expect(within(big).getByRole('button', { name: 'Restore' })).toBeTruthy();
      expect(screen.queryByRole('button', { name: 'Empty Trash' })).toBeNull();
      expect(screen.getByText(/Only the owner and admins can delete them sooner/)).toBeTruthy();
      expect(api.calls('DELETE')).toHaveLength(0);
      cleanup();
    }
  });

  it('lists roots with "+N nested", their path and the purge date', async () => {
    setup();
    const big = await row(/^Roadmap/);
    expect(big.textContent).toContain('+3 nested');
    await waitFor(() => expect(big.textContent).toContain('Handbook / Plans'));
    expect(big.textContent).toContain('Deleted forever in 28 days');
    expect((await row(/^Scratch/)).textContent).toContain('Deleted forever tomorrow');
    expect((await row(/^Old projects/)).textContent).toContain('+7 nested');
    expect(purgeText(new Date(Date.now() - 1000).toISOString())).toBe('Deleted forever today');
  });

  it('restores an item', async () => {
    const api = setup();
    fireEvent.click(within(await row(/^Roadmap/)).getByRole('button', { name: 'Restore' }));
    await waitFor(() => expect(api.calls('POST', '/items/:id/restore')).toHaveLength(1));
    expect(api.calls('POST', '/items/:id/restore')[0]!.params.id).toBe('big');
    expect((await screen.findByRole('status')).textContent).toContain('Restored');
  });

  it('delete forever asks first, with the count of what goes', async () => {
    const api = setup();
    fireEvent.click(within(await row(/^Roadmap/)).getByRole('button', { name: 'Delete forever' }));
    let dialog = await screen.findByRole('alertdialog');
    expect(dialog.textContent).toContain('Delete “Roadmap” forever?');
    expect(dialog.textContent).toContain('This deletes 4 pages');
    // Cancel: nothing is deleted.
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull());
    expect(api.calls('DELETE')).toHaveLength(0);

    fireEvent.click(within(await row(/^Roadmap/)).getByRole('button', { name: 'Delete forever' }));
    dialog = await screen.findByRole('alertdialog');
    fireEvent.click(within(dialog).getByRole('button', { name: 'Delete forever' }));
    await waitFor(() => expect(api.calls('DELETE', '/trash/items/:id')).toHaveLength(1));
    expect(api.calls('DELETE', '/trash/items/:id')[0]!.params.id).toBe('big');

    fireEvent.click(within(await row(/^Scratch/)).getByRole('button', { name: 'Delete forever' }));
    expect((await screen.findByRole('alertdialog')).textContent).toContain('This deletes 1 page.');
  });

  it('a trashed collection is deleted with its items, and Empty Trash counts everything', async () => {
    const api = setup();
    fireEvent.click(within(await row(/^Old projects/)).getByRole('button', { name: 'Delete forever' }));
    let dialog = await screen.findByRole('alertdialog');
    expect(dialog.textContent).toContain('the collection and 7 pages');
    fireEvent.click(within(dialog).getByRole('button', { name: 'Delete forever' }));
    await waitFor(() => expect(api.calls('DELETE', '/trash/collections/:id')).toHaveLength(1));

    fireEvent.click(screen.getByRole('button', { name: 'Empty Trash' }));
    dialog = await screen.findByRole('alertdialog');
    // Roadmap (4) + Scratch (1) + Old projects' 7, and one collection.
    expect(dialog.textContent).toContain('This deletes 12 pages and 1 collection forever');
    fireEvent.click(within(dialog).getByRole('button', { name: 'Delete forever' }));
    await waitFor(() => expect(api.calls('DELETE', '/trash')).toHaveLength(1));
  });

  it('restores an item only with write on its collection, a collection only with manage (D152)', async () => {
    const list = trashList();
    list.collections = [
      { ...list.collections[0]!, my_access: 'write' },
      { ...makeCollection({ id: 'mine', name: 'My old notes', my_access: 'manage' }), deleted_at: inDays(-1), purge_at: inDays(29) },
    ];
    list.items.push(
      { ...makeItem({ id: 'inside', collection_id: 'mine', title: 'Draft' }), deleted_at: inDays(-2), purge_at: inDays(28), descendant_count: 0, path: [] },
    );
    // A viewer reads Handbook only.
    setup('viewer', 'read', list);
    const big = await row(/^Roadmap/);
    await waitFor(() => expect(within(big).queryByRole('button', { name: 'Restore' })).toBeNull());
    expect(within(await row(/^Scratch/)).queryByRole('button', { name: 'Restore' })).toBeNull();
    // An item of a trashed collection goes by that collection's own access.
    expect(within(await row(/^Draft/)).getByRole('button', { name: 'Restore' })).toBeTruthy();
    expect(within(await row(/^Old projects/)).queryByRole('button', { name: 'Restore' })).toBeNull();
    expect(within(await row(/^My old notes/)).getByRole('button', { name: 'Restore' })).toBeTruthy();
  });
});
