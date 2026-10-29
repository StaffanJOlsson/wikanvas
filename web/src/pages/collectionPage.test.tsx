// The collection page with access per collection (D152): creating needs `write`, the collection menu and the sort
// `manage`; a collection the caller may only read lists its items and nothing to change them with.
import { screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { CollectionAccess } from '../api/gen/CollectionAccess';
import { collectionPath } from '../lib/urls';
import { makeCollection, makeSidebar, mockApi, treeReply } from '../test/mockApi';
import { render } from '../test/render';
import { items } from '../test/treeFixtures';
import Collection from './Collection';

function renderPage(my_access: CollectionAccess) {
  const c = makeCollection({ id: 'c1', name: 'Handbook', my_access, item_count: 1 });
  mockApi()
    .on('GET', '/sidebar', [200, makeSidebar([c])])
    .on('GET', `/collections/${c.url_id}`, [200, c])
    .on('GET', '/collections/c1/tree', treeReply(c, items('c1', 'Welcome')));
  return render(<Collection />, { route: collectionPath(c), path: '/collection/:slugId' });
}

describe('collection page', () => {
  it('read-only: the items, no create buttons, no collection menu, the sort shown but fixed', async () => {
    renderPage('read');
    await screen.findByRole('heading', { name: 'Handbook' });
    await screen.findByText('Welcome');
    expect(screen.queryByRole('button', { name: /New document/ })).toBeNull();
    expect(screen.queryByRole('button', { name: /New board/ })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Collection menu' })).toBeNull();
    expect((screen.getByRole('combobox') as HTMLSelectElement).disabled).toBe(true);
  });

  it('writable: create buttons, but the menu and the sort need manage', async () => {
    renderPage('write');
    await screen.findByRole('heading', { name: 'Handbook' });
    expect(screen.getByRole('button', { name: /New document/ })).toBeTruthy();
    expect(screen.getByRole('button', { name: /New board/ })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Collection menu' })).toBeNull();
    expect((screen.getByRole('combobox') as HTMLSelectElement).disabled).toBe(true);
  });

  it('managed: everything', async () => {
    renderPage('manage');
    await screen.findByRole('heading', { name: 'Handbook' });
    await waitFor(() => expect(screen.getByRole('button', { name: 'Collection menu' })).toBeTruthy());
    expect((screen.getByRole('combobox') as HTMLSelectElement).disabled).toBe(false);
  });
});
