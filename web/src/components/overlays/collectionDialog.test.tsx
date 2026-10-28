// The collection dialog (§9.6.1, §9.6.2): 48 icons (plus the letter tile) and the 12 colours of §9.6.1; creating
// posts the choice and opens the new collection, editing patches it.
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { Collection } from '../../api/gen/Collection';
import type { NewCollection } from '../../api/gen/NewCollection';
import { collectionPath } from '../../lib/urls';
import { openDialog } from '../../state/dialogs';
import { makeCollection, makeSidebar, mockApi } from '../../test/mockApi';
import { render } from '../../test/render';
import { COLLECTION_COLORS, COLLECTION_ICON_NAMES } from '../collection/icons';
import { Overlays } from './Overlays';

// §9.6.1, in order.
const SPEC_COLOURS = '#8D8D8D #E5484D #F76B15 #FFC53D #46A758 #12A594 #00A2C7 #0090FF #3E63DD #6E56CF #AB4ABA #D6409F'
  .split(' ');

function radios(group: string): HTMLElement[] {
  return within(screen.getByRole('radiogroup', { name: group })).getAllByRole('radio');
}

describe('collection dialog', () => {
  it('offers the 12 colours of §9.6.1 and 48 distinct icons plus the letter tile', () => {
    mockApi();
    render(<Overlays />);
    act(() => openDialog({ kind: 'collection', collection: null }));
    expect(screen.getByRole('dialog', { name: 'New collection' })).toBeTruthy();
    expect(COLLECTION_COLORS).toEqual(SPEC_COLOURS);
    expect(radios('Colour').map((r) => r.getAttribute('aria-label'))).toEqual(SPEC_COLOURS);
    const icons = radios('Icon').map((r) => r.getAttribute('aria-label'));
    expect(icons).toHaveLength(49);
    expect(icons[0]).toBe('Letter');
    expect(new Set(icons.slice(1)).size).toBe(48);
    expect(icons.slice(1)).toEqual([...COLLECTION_ICON_NAMES]);
    expect(icons[1]).toBe('rocket'); // W3-D16
    // The defaults: the letter tile and the first colour.
    expect(radios('Icon')[0]!.getAttribute('aria-checked')).toBe('true');
    expect(radios('Colour')[0]!.getAttribute('aria-checked')).toBe('true');
  });

  it('creates with the chosen name, icon and colour, then opens the collection', async () => {
    const created = makeCollection({ id: 'c9', name: 'Plans', icon: 'rocket', color: '#46A758' });
    const api = mockApi()
      .on('GET', '/sidebar', [200, makeSidebar([])])
      .on('POST', '/collections', [201, created]);
    const { router } = render(<Overlays />, { routes: [{ path: '/collection/:slugId', element: <h1>Collection</h1> }] });
    act(() => openDialog({ kind: 'collection', collection: null }));
    fireEvent.change(screen.getByRole('textbox', { name: 'Name' }), { target: { value: '  Plans ' } });
    fireEvent.click(screen.getByRole('radio', { name: 'rocket' }));
    fireEvent.click(screen.getByRole('radio', { name: '#46A758' }));
    expect(screen.getByRole('radio', { name: 'rocket' }).getAttribute('aria-checked')).toBe('true');
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Create' }));
    });
    await waitFor(() => expect(api.calls('POST', '/collections')).toHaveLength(1));
    expect(api.calls('POST', '/collections')[0]!.body).toEqual({
      name: 'Plans', description: '', icon: 'rocket', color: '#46A758',
    } satisfies NewCollection);
    await waitFor(() => expect(router.state.location.pathname).toBe(collectionPath(created)));
    expect(screen.queryByRole('dialog', { name: 'New collection' })).toBeNull();
  });

  it('edits an existing collection: prefilled, and the change is patched', async () => {
    const c1: Collection = makeCollection({ id: 'c1', name: 'Docs', icon: 'book', color: '#E5484D' });
    const api = mockApi()
      .on('GET', '/sidebar', [200, makeSidebar([c1])])
      .on('PATCH', '/collections/:id', (req) => [200, { ...c1, ...(req.body as object) }]);
    render(<Overlays />);
    act(() => openDialog({ kind: 'collection', collection: c1 }));
    expect(screen.getByRole('dialog', { name: 'Edit collection' })).toBeTruthy();
    expect((screen.getByRole('textbox', { name: 'Name' }) as HTMLInputElement).value).toBe('Docs');
    expect(screen.getByRole('radio', { name: '#E5484D' }).getAttribute('aria-checked')).toBe('true');
    fireEvent.click(screen.getByRole('radio', { name: 'Letter' }));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    });
    await waitFor(() => expect(api.calls('PATCH', '/collections/:id')).toHaveLength(1));
    expect(api.calls('PATCH', '/collections/:id')[0]).toMatchObject({
      params: { id: 'c1' },
      body: { name: 'Docs', icon: null, color: '#E5484D' },
    });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Edit collection' })).toBeNull());
  });
});
