// "Text doc" (docs/ARCHITECTURE.md §18.6 (a)): a new child document of the board, placed in the middle of the view and
// opened for editing; the documents left on no card are discarded when the page leaves the board, before a created
// board's own discard; an insert failure discards at once.
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import type { ItemSummary } from '../../api/gen/ItemSummary';
import { itemKey, updateCachedItem } from '../../api/queries/items';
import {
  boardItem,
  boardPath,
  boardRoutes,
  cardEl,
  DOC_3,
  docItem,
  docRoutes,
  errorBody,
  mockFetch,
  openBoardApp,
  paragraphs,
  renderBoardApp,
  type Handler,
  type Recorded,
  type Rendered,
  type Reply,
} from '../testkit';
import { isExpanded } from '../../state/expanded';
import { embedController, useEmbedStore } from './controller';
import { deferredTextDocs, resetDeferredTextDocs } from './textDocs';

let app: Rendered | null = null;

afterEach(async () => {
  await embedController()?.settled();
  app?.unmount();
  app = null;
  resetDeferredTextDocs();
});

const DOC_4 = '0192f0e0-0000-7000-8000-000000000004';

function summary(id: string): ItemSummary {
  return { ...docItem(id).item, title: '', parent_id: 'board-1', collection_id: 'col-1' };
}

function routes(board: ItemDetail, extra: Record<string, Handler | Reply> = {}) {
  const created = [summary(DOC_3), summary(DOC_4)];
  let n = 0;
  return {
    ...boardRoutes(board),
    ...docRoutes(docItem(DOC_3, { title: '', parent_id: 'board-1' }), paragraphs('')),
    ...docRoutes(docItem(DOC_4, { title: '', parent_id: 'board-1' }), paragraphs('')),
    'POST /api/v1/items': (() => [201, created[n++]]) as Handler,
    ...extra,
  };
}

const textDoc = () => screen.getByRole('button', { name: 'Text doc' }) as HTMLButtonElement;
const deletes = (requests: Recorded[]) => requests.filter((q) => q.method === 'DELETE');

describe('Text doc', () => {
  it('creates a child document, places it at the centre titled "Untitled", and starts editing it', async () => {
    const board = boardItem(1);
    const m = mockFetch(routes(board));
    vi.stubGlobal('fetch', m.fetch);
    const r = (app = await openBoardApp(board));
    await waitFor(() => expect(textDoc().disabled).toBe(false));
    fireEvent.click(textDoc());
    await waitFor(() => expect(r.fake.inserted).toHaveLength(1));
    const post = m.requests.find((q) => q.method === 'POST' && q.path === '/api/v1/items');
    expect(JSON.parse(post?.body ?? '{}')).toEqual({ collection_id: 'col-1', parent_id: 'board-1', kind: 'doc' });
    expect(r.fake.inserted[0]).toEqual({ id: 'inserted1', item: DOC_3, title: 'Untitled', at: 'center' });
    // Editing starts on the new card at once.
    expect(r.fake.embedEditing()).toBe('inserted1');
    expect(embedController()?.editingId()).toBe('inserted1');
    // The engine reports it in its next layout; the card opens its editor, caret in the (empty) body.
    act(() => {
      r.fake.layout({ embeds: [{ objectId: 'inserted1', item: DOC_3, x: -240, y: -180, label: 'Untitled' }] });
    });
    const card = () => cardEl('inserted1') as HTMLElement;
    await waitFor(() => expect(card().dataset.state).toBe('editing'));
    await waitFor(() => expect(card().querySelector('.board-embed-editor .ProseMirror[contenteditable="true"]')).not.toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(card().querySelector('.board-embed-editor .ProseMirror')));
    const title = card().querySelector('input.board-embed-card-title') as HTMLInputElement;
    expect(title.value).toBe('');
    expect(title.placeholder).toBe('Untitled');
    // The board's sidebar row is expanded, so the new child shows in the tree.
    expect(isExpanded('board-1')).toBe(true);
  });

  it('is disabled while the board is read-only', async () => {
    const board: ItemDetail = { ...boardItem(1), archived_at: '2026-10-28T10:00:00.000Z' };
    vi.stubGlobal('fetch', mockFetch(routes(board)).fetch);
    app = await openBoardApp(board);
    expect(textDoc().disabled).toBe(true);
    expect((screen.getByRole('button', { name: 'Embed page…' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('is absent when the frame cannot embed', async () => {
    const board = boardItem(1);
    vi.stubGlobal('fetch', mockFetch(routes(board)).fetch);
    const r = (app = renderBoardApp(boardPath(board), { embeds: false }));
    await waitFor(() => expect(r.fake.api.current()).toBe('board-1'));
    expect(screen.queryByRole('button', { name: 'Text doc' })).toBeNull();
  });

  it('leaving the board discards the created documents on no card (a 409 ignored), keeps the others, then the board', async () => {
    const board = boardItem(1, { title: '' });
    const m = mockFetch(
      routes(board, {
        [`DELETE /api/v1/items/${DOC_3}`]: [409, errorBody('not_empty')],
        [`DELETE /api/v1/items/${DOC_4}`]: [204],
        'DELETE /api/v1/items/board-1': [204],
      }),
    );
    vi.stubGlobal('fetch', m.fetch);
    const r = (app = renderBoardApp('/home'));
    await act(() => r.router.navigate(boardPath(board), { state: { created: true } }));
    await waitFor(() => expect(useEmbedStore.getState().board?.itemId).toBe('board-1'));
    await waitFor(() => expect(textDoc().disabled).toBe(false));
    fireEvent.click(textDoc());
    await waitFor(() => expect(r.fake.inserted).toHaveLength(1));
    await waitFor(() => expect(textDoc().disabled).toBe(false));
    fireEvent.click(textDoc());
    await waitFor(() => expect(r.fake.inserted).toHaveLength(2));
    // Only the second card is still on the board (the first was deleted in the engine).
    act(() => {
      r.fake.layout({ embeds: [{ objectId: 'inserted2', item: DOC_4 }] });
    });
    act(() => {
      r.fake.layout({ embeds: [{ objectId: 'other', item: DOC_4 }], editing: null });
    });
    await act(() => r.router.navigate('/home'));
    await waitFor(() => expect(deletes(m.requests).map((q) => q.path)).toContain('/api/v1/items/board-1'));
    const order = deletes(m.requests).map((q) => `${q.path}${q.search}`);
    expect(order).toEqual([`/api/v1/items/${DOC_3}?discard_empty=true`, '/api/v1/items/board-1?discard_empty=true']);
  });

  it('a board switch while the document is created places nothing on the next board and discards it', async () => {
    const a = boardItem(1);
    const b = boardItem(2);
    let release: (() => void) | null = null;
    const m = mockFetch({
      ...routes(a),
      ...boardRoutes(b),
      'POST /api/v1/items': () =>
        new Promise<Reply>((resolve) => {
          release = () => resolve([201, summary(DOC_3)]);
        }),
      [`DELETE /api/v1/items/${DOC_3}`]: [204],
    });
    vi.stubGlobal('fetch', m.fetch);
    const r = (app = await openBoardApp(a));
    await waitFor(() => expect(textDoc().disabled).toBe(false));
    fireEvent.click(textDoc());
    await waitFor(() => expect(release).not.toBeNull());
    // The sidebar takes the page to another board before the POST answers.
    await act(() => r.router.navigate(boardPath(b)));
    await waitFor(() => expect(useEmbedStore.getState().board?.itemId).toBe('board-2'));
    await act(async () => release?.());
    // The child of board 1 is never placed on board 2; it is discarded like any created document on no card.
    await waitFor(() =>
      expect(deletes(m.requests).map((q) => `${q.path}${q.search}`)).toEqual([`/api/v1/items/${DOC_3}?discard_empty=true`]),
    );
    expect(r.fake.calls.filter((c) => c.method === 'insertEmbed')).toEqual([]);
    expect(r.fake.embedEditing()).toBeNull();
    expect(embedController()?.editingId()).toBeNull();
  });

  it('moving the board while its page shows it discards nothing; leaving it later still does', async () => {
    const board = boardItem(1, { title: '' });
    const m = mockFetch(
      routes(board, {
        [`DELETE /api/v1/items/${DOC_3}`]: [204],
        'DELETE /api/v1/items/board-1': [409, errorBody('not_empty')],
      }),
    );
    vi.stubGlobal('fetch', m.fetch);
    const r = (app = renderBoardApp('/home'));
    await act(() => r.router.navigate(boardPath(board), { state: { created: true } }));
    await waitFor(() => expect(useEmbedStore.getState().board?.itemId).toBe('board-1'));
    await waitFor(() => expect(textDoc().disabled).toBe(false));
    fireEvent.click(textDoc());
    await waitFor(() => expect(r.fake.inserted).toHaveLength(1));
    // The card is deleted in the engine: the document is on no card now.
    act(() => {
      r.fake.layout({ embeds: [] });
    });
    // The board is moved (the sidebar's drag and drop, another tab): its detail gets a new parent and collection.
    act(() => {
      updateCachedItem(r.queryClient, 'board-1', (d) => ({
        ...d,
        item: { ...d.item, parent_id: 'folder-1', collection_id: 'col-2' },
      }));
    });
    await act(() => new Promise((resolve) => setTimeout(resolve, 30)));
    expect(deletes(m.requests)).toEqual([]);
    await act(() => r.router.navigate('/home'));
    await waitFor(() => expect(deletes(m.requests)).toHaveLength(2));
    expect(deletes(m.requests).map((q) => `${q.path}${q.search}`)).toEqual([
      `/api/v1/items/${DOC_3}?discard_empty=true`,
      '/api/v1/items/board-1?discard_empty=true',
    ]);
  });

  it('a board switch discards with the left board’s place, not the next one’s', async () => {
    const a = boardItem(1);
    const b = boardItem(2, { collection_id: 'col-2' });
    const m = mockFetch({ ...routes(a, { [`DELETE /api/v1/items/${DOC_3}`]: [204] }), ...boardRoutes(b) });
    vi.stubGlobal('fetch', m.fetch);
    const r = (app = await openBoardApp(a));
    await waitFor(() => expect(textDoc().disabled).toBe(false));
    fireEvent.click(textDoc());
    await waitFor(() => expect(r.fake.inserted).toHaveLength(1));
    act(() => {
      r.fake.layout({ embeds: [] });
    });
    // B's detail is cached (the sidebar hovered it, an earlier visit): BoardView stays mounted from A to B.
    r.queryClient.setQueryData(itemKey(b.item.url_id), b);
    const invalidate = vi.spyOn(r.queryClient, 'invalidateQueries');
    await act(() => r.router.navigate(boardPath(b)));
    await waitFor(() => expect(deletes(m.requests)).toHaveLength(1));
    await waitFor(() => expect(invalidate.mock.calls.some(([f]) => JSON.stringify(f?.queryKey) === '["tree","col-1"]')).toBe(true));
    expect(invalidate.mock.calls.some(([f]) => JSON.stringify(f?.queryKey) === '["tree","col-2"]')).toBe(false);
  });

  it('a card on the engine’s clipboard keeps its document; a later leave discards it once no clipboard holds it', async () => {
    const a = boardItem(1);
    const b = boardItem(2);
    let gone = false;
    const m = mockFetch({
      ...routes(a),
      ...boardRoutes(b),
      [`GET /api/v1/items/${DOC_3}`]: () => (gone ? [404, errorBody('not_found')] : [200, docItem(DOC_3, { title: '', parent_id: 'board-1' })]),
      [`DELETE /api/v1/items/${DOC_3}`]: () => {
        gone = true;
        return [204];
      },
    });
    vi.stubGlobal('fetch', m.fetch);
    const r = (app = await openBoardApp(a));
    await waitFor(() => expect(textDoc().disabled).toBe(false));
    fireEvent.click(textDoc());
    await waitFor(() => expect(r.fake.inserted).toHaveLength(1));
    // The card is cut (or copied, then deleted): on no card of the board, but on the engine's clipboard.
    act(() => {
      r.fake.layout({ embeds: [], clipboard: [DOC_3] });
    });
    await act(() => r.router.navigate(boardPath(b)));
    await waitFor(() => expect(useEmbedStore.getState().board?.itemId).toBe('board-2'));
    await act(() => new Promise((resolve) => setTimeout(resolve, 30)));
    expect(deletes(m.requests)).toEqual([]);
    expect(deferredTextDocs()).toEqual([DOC_3]);
    // Still on the clipboard when the page leaves the next board: kept again.
    act(() => {
      r.fake.layout({ embeds: [], clipboard: [DOC_3] });
    });
    await act(() => r.router.navigate(boardPath(a)));
    await waitFor(() => expect(useEmbedStore.getState().board?.itemId).toBe('board-1'));
    await act(() => new Promise((resolve) => setTimeout(resolve, 30)));
    expect(deletes(m.requests)).toEqual([]);
    // Something else was copied: the next leave discards it, and its cached detail goes with it.
    expect(r.queryClient.getQueryData(itemKey(DOC_3))).toBeDefined();
    act(() => {
      r.fake.layout({ embeds: [], clipboard: [] });
    });
    await act(() => r.router.navigate(boardPath(b)));
    await waitFor(() =>
      expect(deletes(m.requests).map((q) => `${q.path}${q.search}`)).toEqual([`/api/v1/items/${DOC_3}?discard_empty=true`]),
    );
    expect(deferredTextDocs()).toEqual([]);
    await waitFor(() => expect(r.queryClient.getQueryData(itemKey(DOC_3))).toBeUndefined());
    // A card that still names it (pasted later) shows "Page not found".
    await waitFor(() => expect(useEmbedStore.getState().board?.itemId).toBe('board-2'));
    act(() => {
      r.fake.layout({ embeds: [{ objectId: 'pasted', item: DOC_3, label: 'Untitled' }] });
    });
    await waitFor(() => expect(cardEl('pasted')?.dataset.state).toBe('missing'));
  });

  it('a document kept for the clipboard and pasted on the next board stays', async () => {
    const a = boardItem(1);
    const b = boardItem(2);
    const m = mockFetch({ ...routes(a), ...boardRoutes(b), [`DELETE /api/v1/items/${DOC_3}`]: [204] });
    vi.stubGlobal('fetch', m.fetch);
    const r = (app = await openBoardApp(a));
    await waitFor(() => expect(textDoc().disabled).toBe(false));
    fireEvent.click(textDoc());
    await waitFor(() => expect(r.fake.inserted).toHaveLength(1));
    act(() => {
      r.fake.layout({ embeds: [], clipboard: [DOC_3] });
    });
    await act(() => r.router.navigate(boardPath(b)));
    await waitFor(() => expect(useEmbedStore.getState().board?.itemId).toBe('board-2'));
    // Pasted here; the clipboard then holds something else.
    act(() => {
      r.fake.layout({ embeds: [{ objectId: 'pasted', item: DOC_3 }], clipboard: [] });
    });
    await act(() => r.router.navigate('/home'));
    await act(() => new Promise((resolve) => setTimeout(resolve, 30)));
    expect(deletes(m.requests)).toEqual([]);
    expect(deferredTextDocs()).toEqual([]);
  });

  it('an insert failure shows the toast and discards the new document at once', async () => {
    const board = boardItem(1);
    const m = mockFetch(routes(board, { [`DELETE /api/v1/items/${DOC_3}`]: [204] }));
    vi.stubGlobal('fetch', m.fetch);
    const r = (app = await openBoardApp(board));
    r.fake.api.insertEmbed = vi.fn(() => null);
    await waitFor(() => expect(textDoc().disabled).toBe(false));
    fireEvent.click(textDoc());
    await screen.findByText('Could not add the page to the board');
    await waitFor(() => expect(deletes(m.requests)).toHaveLength(1));
    expect(deletes(m.requests)[0]).toMatchObject({ path: `/api/v1/items/${DOC_3}`, search: '?discard_empty=true' });
    expect(r.fake.embedEditing()).toBeNull();
  });
});
