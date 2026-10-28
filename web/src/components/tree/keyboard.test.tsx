// Every key of the §9.6.3 tree keyboard table, on the rendered sidebar tree.
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { ItemSummary } from '../../api/gen/ItemSummary';
import type { MoveItem } from '../../api/gen/MoveItem';
import { MAIN_ID } from '../../app/hotkeys';
import { collectionPath, itemPath } from '../../lib/urls';
import { isExpanded } from '../../state/expanded';
import { makeCollection, mockApi, treeReply } from '../../test/mockApi';
import { findRow, renderSidebar, rowNames, waitForRows } from '../../test/sidebarHarness';
import { items } from '../../test/treeFixtures';

const mac = /Mac|iPhone|iPad/.test(navigator.userAgent);
const mod = mac ? { metaKey: true } : { ctrlKey: true };

const c1 = makeCollection({ id: 'c1', name: 'Docs', item_count: 6 });

function setup(expanded: string[] = ['c1']) {
  // Docs: Alpha [Alpha one, Alpha two], Beta, Bravo, Gamma
  let list: ItemSummary[] = items('c1', 'Alpha', 'Alpha>Alpha one', 'Alpha>Alpha two', 'Beta', 'Bravo', 'Gamma');
  const api = mockApi();
  const view = renderSidebar({ api, collections: [c1], expanded });
  api.on('GET', '/collections/c1/tree', () => treeReply(c1, list, `"t${list.map((i) => `${i.parent_id}${i.sort_key}`).join()}"`));
  api.on('POST', '/items/:id/move', (req) => {
    const body = req.body as MoveItem;
    const moved = { ...list.find((i) => i.id === req.params.id)!, parent_id: body.parent_id, sort_key: 'z' };
    list = list.map((i) => (i.id === moved.id ? moved : i));
    return [200, moved];
  });
  api.on('PATCH', '/items/:id', (req) => {
    const renamed = { ...list.find((i) => i.id === req.params.id)!, title: (req.body as { title: string }).title };
    list = list.map((i) => (i.id === renamed.id ? renamed : i));
    return [200, renamed];
  });
  return { ...view, api, item: (id: string) => list.find((i) => i.id === id)! };
}

const key = (el: Element, k: string, extra: KeyboardEventInit = {}) => {
  act(() => {
    fireEvent.keyDown(el, { key: k, ...extra });
  });
};
const focused = () => document.activeElement?.getAttribute('aria-label') ?? document.activeElement?.id;

describe('tree keyboard', () => {
  it('↑ / ↓, Home / End move between visible rows', async () => {
    setup(['c1', 'Alpha']);
    await waitForRows(['Docs', 'Alpha', 'Alpha one', 'Alpha two', 'Beta', 'Bravo', 'Gamma']);
    const first = screen.getByRole('treeitem', { name: 'Docs' });
    first.focus();
    key(first, 'ArrowDown');
    expect(focused()).toBe('Alpha');
    key(document.activeElement!, 'ArrowDown');
    expect(focused()).toBe('Alpha one');
    key(document.activeElement!, 'ArrowUp');
    expect(focused()).toBe('Alpha');
    key(document.activeElement!, 'End');
    expect(focused()).toBe('Gamma');
    key(document.activeElement!, 'ArrowDown');
    expect(focused()).toBe('Gamma');
    key(document.activeElement!, 'Home');
    expect(focused()).toBe('Docs');
    // Roving tabindex: only the focused row is a tab stop.
    expect(screen.getAllByRole('treeitem').filter((r) => r.tabIndex === 0).map((r) => r.getAttribute('aria-label'))).toEqual(['Docs']);
  });

  it('→ expands, then goes to the first child; ← collapses, then goes to the parent; leaves ignore →', async () => {
    setup();
    const alpha = await findRow('Alpha');
    alpha.focus();
    expect(alpha.getAttribute('aria-expanded')).toBe('false');
    key(alpha, 'ArrowRight');
    await waitFor(() => expect(screen.getByRole('treeitem', { name: 'Alpha' }).getAttribute('aria-expanded')).toBe('true'));
    expect(isExpanded('Alpha')).toBe(true);
    key(document.activeElement!, 'ArrowRight');
    expect(focused()).toBe('Alpha one');
    key(document.activeElement!, 'ArrowRight');
    expect(focused()).toBe('Alpha one');
    key(document.activeElement!, 'ArrowLeft');
    expect(focused()).toBe('Alpha');
    key(document.activeElement!, 'ArrowLeft');
    await waitFor(() => expect(screen.getByRole('treeitem', { name: 'Alpha' }).getAttribute('aria-expanded')).toBe('false'));
    key(document.activeElement!, 'ArrowLeft');
    expect(focused()).toBe('Docs');
    key(document.activeElement!, 'ArrowLeft');
    await waitFor(() => expect(rowNames()).toEqual(['Docs']));
  });

  it('Enter opens the item or the collection page; Mod+Enter opens a new tab', async () => {
    const { router, item } = setup();
    const beta = await findRow('Beta');
    beta.focus();
    key(beta, 'Enter');
    await waitFor(() => expect(router.state.location.pathname).toBe(itemPath(item('Beta'))));
    key(screen.getByRole('treeitem', { name: 'Docs' }), 'Enter');
    await waitFor(() => expect(router.state.location.pathname).toBe(collectionPath(c1)));
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    key(screen.getByRole('treeitem', { name: 'Gamma' }), 'Enter', mod);
    expect(open).toHaveBeenCalledWith(itemPath(item('Gamma')), '_blank', 'noopener');
    expect(router.state.location.pathname).toBe(collectionPath(c1));
  });

  it('F2 renames inline: Enter saves, Esc cancels', async () => {
    const { api } = setup();
    const beta = await findRow('Beta');
    beta.focus();
    key(beta, 'F2');
    let input = (await screen.findByRole('textbox', { name: 'Title' })) as HTMLInputElement;
    expect(input.value).toBe('Beta');
    fireEvent.change(input, { target: { value: 'Beta renamed' } });
    key(input, 'Enter');
    await waitFor(() => expect(api.calls('PATCH', '/items/:id')).toHaveLength(1));
    expect(api.calls('PATCH', '/items/:id')[0]!).toMatchObject({ params: { id: 'Beta' }, body: { title: 'Beta renamed' } });
    expect(await findRow('Beta renamed')).toBeTruthy();

    key(screen.getByRole('treeitem', { name: 'Gamma' }), 'F2');
    input = (await screen.findByRole('textbox', { name: 'Title' })) as HTMLInputElement;
    fireEvent.change(input, { target: { value: 'nope' } });
    key(input, 'Escape');
    await waitFor(() => expect(screen.queryByRole('textbox', { name: 'Title' })).toBeNull());
    expect(api.calls('PATCH', '/items/:id')).toHaveLength(1);
    expect(focused()).toBe('Gamma');
  });

  it('Alt+Shift+→ indents (last child of the previous sibling), Alt+Shift+← outdents (right after the parent)', async () => {
    const { api } = setup(['c1', 'Alpha']);
    const beta = await findRow('Beta');
    beta.focus();
    key(beta, 'ArrowRight', { altKey: true, shiftKey: true });
    await waitFor(() => expect(api.calls('POST', '/items/:id/move')).toHaveLength(1));
    expect(api.calls('POST', '/items/:id/move')[0]!).toMatchObject({
      params: { id: 'Beta' },
      body: { collection_id: 'c1', parent_id: 'Alpha' },
    });
    await waitFor(() => expect(screen.getByRole('treeitem', { name: 'Beta' }).getAttribute('aria-level')).toBe('3'));
    await waitFor(() => expect(focused()).toBe('Beta'));

    key(screen.getByRole('treeitem', { name: 'Alpha one' }), 'ArrowLeft', { altKey: true, shiftKey: true });
    await waitFor(() => expect(api.calls('POST', '/items/:id/move')).toHaveLength(2));
    expect(api.calls('POST', '/items/:id/move')[1]!).toMatchObject({
      params: { id: 'Alpha one' },
      body: { collection_id: 'c1', parent_id: null, position: { after_id: 'Alpha', before_id: 'Bravo' } },
    });
    // A top-level item cannot be outdented, the first child cannot be indented: no request.
    key(screen.getByRole('treeitem', { name: 'Gamma' }), 'ArrowLeft', { altKey: true, shiftKey: true });
    await act(async () => undefined);
    expect(api.calls('POST', '/items/:id/move')).toHaveLength(2);
  });

  it('Alt+Shift+↑ / ↓ reorder among siblings', async () => {
    const { api } = setup();
    const bravo = await findRow('Bravo');
    bravo.focus();
    key(bravo, 'ArrowUp', { altKey: true, shiftKey: true });
    await waitFor(() => expect(api.calls('POST', '/items/:id/move')).toHaveLength(1));
    expect(api.calls('POST', '/items/:id/move')[0]!.body).toEqual({
      collection_id: 'c1', parent_id: null, position: { after_id: 'Alpha', before_id: 'Beta' },
    });
    key(screen.getByRole('treeitem', { name: 'Alpha' }), 'ArrowDown', { altKey: true, shiftKey: true });
    await waitFor(() => expect(api.calls('POST', '/items/:id/move')).toHaveLength(2));
    expect(api.calls('POST', '/items/:id/move')[1]!).toMatchObject({ params: { id: 'Alpha' } });
    expect((api.calls('POST', '/items/:id/move')[1]!.body as MoveItem).position?.after_id).toBeTruthy();
  });

  it('Delete moves to Trash, asking first when the item has nested pages', async () => {
    const { api } = setup();
    api.on('DELETE', '/items/:id', () => [200, { trashed: 1 }]);
    const gamma = await findRow('Gamma');
    gamma.focus();
    key(gamma, 'Delete');
    await waitFor(() => expect(api.calls('DELETE', '/items/:id')).toHaveLength(1));
    expect(api.calls('DELETE', '/items/:id')[0]!.params.id).toBe('Gamma');
    expect(screen.queryByRole('alertdialog')).toBeNull();

    key(screen.getByRole('treeitem', { name: 'Alpha' }), 'Delete');
    const dialog = await screen.findByRole('alertdialog');
    expect(dialog.textContent).toContain('2 nested pages');
    fireEvent.click(screen.getByRole('button', { name: 'Move to Trash' }));
    await waitFor(() => expect(api.calls('DELETE', '/items/:id')).toHaveLength(2));
    expect(api.calls('DELETE', '/items/:id')[1]!.params.id).toBe('Alpha');
  });

  it('typing jumps to the next row starting with the typed prefix (500 ms buffer)', async () => {
    setup();
    await findRow('Gamma');
    const docs = screen.getByRole('treeitem', { name: 'Docs' });
    docs.focus();
    key(docs, 'b');
    expect(focused()).toBe('Beta');
    key(document.activeElement!, 'b');
    expect(focused()).toBe('Bravo');
    key(document.activeElement!, 'g');
    expect(focused()).toBe('Gamma');
    // "br" typed quickly finds Bravo from anywhere.
    screen.getByRole('treeitem', { name: 'Docs' }).focus();
    key(document.activeElement!, 'b');
    key(document.activeElement!, 'r');
    expect(focused()).toBe('Bravo');
  });

  it('Esc focuses the main region', async () => {
    setup();
    const beta = await findRow('Beta');
    beta.focus();
    key(beta, 'Escape');
    expect(document.activeElement?.id).toBe(MAIN_ID);
  });
});
