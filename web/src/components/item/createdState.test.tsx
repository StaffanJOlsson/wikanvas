// New items open with `{ state: { created: true } }` (W4 D20): the top bar's "New doc", the sidebar's "+" and the
// item menus' New all go through `useItemActions().create`, and DocPage discards an item left empty only when the
// navigation said this page session created it.
import { act, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { itemPath } from '../../lib/urls';
import { makeCollection, makeItem, mockApi } from '../../test/mockApi';
import { renderSidebar } from '../../test/sidebarHarness';

describe('creating an item', () => {
  it('navigates to it with state.created', async () => {
    const c = makeCollection({ id: 'c1' });
    const created = makeItem({ id: 'n1', collection_id: 'c1', title: '' });
    const api = mockApi().on('POST', '/items', [201, created]);
    const { router, probe } = renderSidebar({ collections: [c], api });
    await act(async () => {
      await probe.actions.create('c1', null, 'doc');
    });
    await waitFor(() => expect(router.state.location.pathname).toBe(itemPath(created)));
    expect(router.state.location.state).toEqual({ created: true });
    expect(api.calls('POST', '/items')[0]!.body).toEqual({ collection_id: 'c1', parent_id: null, kind: 'doc' });
  });
});
