// Dropping a sidebar document (docs/ARCHITECTURE.md §18.6 (c), D70): the frame reports `onDropItem`; the host places
// a card centred on the drop point for a live document only, titled from the target's own ItemDetail (cached, else
// fetched; the drag data's title is never stored); a drop on the editing card is taken by the overlay in the capture
// phase, before ProseMirror.
import { act, screen, waitFor } from '@testing-library/react';
import type { Editor } from '@tiptap/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import { itemKey } from '../../api/queries/items';
import type { DraggedItem } from '../bridge';
import {
  boardItem,
  boardRoutes,
  cardEl,
  DOC_1,
  DOC_2,
  docItem,
  docRoutes,
  errorBody,
  mockFetch,
  openBoardApp,
  paragraphs,
  type Rendered,
} from '../testkit';
import { embedController, INSERT_FAILED } from './controller';
import { boardDragDataOf, DRAG_ITEM_TYPE } from './dragItem';
import { MAX_SNAPSHOT_CHARS } from './titles';

let app: Rendered | null = null;

afterEach(async () => {
  await embedController()?.settled();
  app?.unmount();
  app = null;
});

const dragged = (id: string, title = 'Old title'): DraggedItem => ({ v: 1, id, kind: 'doc', title });

function setup(board: ItemDetail = boardItem(1)) {
  vi.stubGlobal(
    'fetch',
    mockFetch({
      ...boardRoutes(board),
      ...docRoutes(docItem(DOC_1, { title: 'First page' }), paragraphs('kangaroo')),
      ...docRoutes(docItem(DOC_2, { title: 'Renamed page' }), paragraphs('wombat')),
    }).fetch,
  );
  return openBoardApp(board);
}

/** A native drag event with the sidebar's data at a viewport point. */
function dragEvent(type: string, data: Record<string, string>, x: number, y: number): DragEvent {
  const dt = new DataTransfer();
  for (const [k, v] of Object.entries(data)) dt.setData(k, v);
  const e = new Event(type, { bubbles: true, cancelable: true }) as DragEvent;
  Object.defineProperty(e, 'dataTransfer', { value: dt });
  Object.defineProperty(e, 'clientX', { value: x });
  Object.defineProperty(e, 'clientY', { value: y });
  return e;
}

describe('dropping a sidebar document', () => {
  it('places a card at the drop point, titled from the target’s own detail (cached, else fetched), never from the drag data', async () => {
    const r = (app = await setup());
    r.queryClient.setQueryData(itemKey(DOC_2), docItem(DOC_2, { title: 'Renamed page' }));
    const focusBefore = r.fake.calls.filter((c) => c.method === 'focus').length;
    act(() => r.fake.dropItem(dragged(DOC_2, 'Old title'), { x: 120, y: 80 }));
    await waitFor(() => expect(r.fake.inserted).toHaveLength(1));
    expect(r.fake.inserted).toEqual([{ id: 'inserted1', item: DOC_2, title: 'Renamed page', at: { x: 120, y: 80 } }]);
    // DOC_1 is not cached: its detail is fetched first.
    expect(r.queryClient.getQueryData(itemKey(DOC_1))).toBeUndefined();
    act(() => r.fake.dropItem(dragged(DOC_1, 'Crafted <b>title</b>'), { x: 300, y: 200 }));
    await waitFor(() => expect(r.fake.inserted).toHaveLength(2));
    expect(r.fake.inserted[1]).toEqual({ id: 'inserted2', item: DOC_1, title: 'First page', at: { x: 300, y: 200 } });
    // Placed and selected, not opened; the frame keeps focus.
    expect(r.fake.embedEditing()).toBeNull();
    expect(r.fake.calls.filter((c) => c.method === 'focus').length).toBe(focusBefore + 2);
  });

  it('a non-UUID id, another board’s event, a board, trashed or missing target, or a read-only board place nothing', async () => {
    const BOARD = '0192f0e0-0000-7000-8000-0000000000b1';
    const TRASHED = '0192f0e0-0000-7000-8000-0000000000b2';
    const MISSING = '0192f0e0-0000-7000-8000-0000000000b3';
    const UNCACHED_BOARD = '0192f0e0-0000-7000-8000-0000000000b4';
    const m = mockFetch({
      ...boardRoutes(boardItem(1)),
      ...docRoutes(docItem(DOC_1, { title: 'First page' }), paragraphs('kangaroo')),
      ...docRoutes(docItem(TRASHED, {}, { trashed_at: '2026-10-28T10:00:00.000Z' })),
      [`GET /api/v1/items/${MISSING}`]: [404, errorBody('not_found')],
      [`GET /api/v1/items/${UNCACHED_BOARD}`]: [200, { ...docItem(UNCACHED_BOARD), item: { ...docItem(UNCACHED_BOARD).item, kind: 'board' } }],
    });
    vi.stubGlobal('fetch', m.fetch);
    const r = (app = await openBoardApp(boardItem(1)));
    r.queryClient.setQueryData(itemKey(BOARD), { ...docItem(BOARD), item: { ...docItem(BOARD).item, kind: 'board' } });
    act(() => {
      r.fake.dropItem(dragged('not-a-uuid'), { x: 1, y: 1 });
      r.fake.dropItem(dragged(DOC_1.toUpperCase()), { x: 1, y: 1 });
      r.fake.dropItem(dragged(BOARD), { x: 1, y: 1 });
      r.fake.dropItem(dragged(UNCACHED_BOARD), { x: 1, y: 1 });
      r.fake.dropItem(dragged(TRASHED), { x: 1, y: 1 });
      r.fake.dropItem(dragged(MISSING), { x: 1, y: 1 });
      r.fake.events()?.onDropItem?.({ itemId: 'board-2', item: dragged(DOC_1), point: { x: 1, y: 1 } });
    });
    await waitFor(() => {
      for (const id of [UNCACHED_BOARD, TRASHED, MISSING]) {
        expect(m.requests.some((q) => q.method === 'GET' && q.path === `/api/v1/items/${id}`)).toBe(true);
      }
    });
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(r.fake.calls.filter((c) => c.method === 'insertEmbed')).toHaveLength(0);
    expect(screen.queryByText(INSERT_FAILED)).toBeNull();
    r.unmount();
    app = null;

    const ro = (app = await setup({ ...boardItem(1), trashed_at: '2026-10-28T10:00:00.000Z' }));
    act(() => {
      ro.fake.events()?.onDropItem?.({ itemId: 'board-1', item: dragged(DOC_1), point: { x: 1, y: 1 } });
    });
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(ro.fake.calls.filter((c) => c.method === 'insertEmbed')).toHaveLength(0);
  });

  it('a target whose detail does not load gives the error toast; a long title is cut to the snapshot’s length', async () => {
    const BROKEN = '0192f0e0-0000-7000-8000-0000000000b5';
    const LONG = '0192f0e0-0000-7000-8000-0000000000b6';
    const long = `${'Very long title '.repeat(25)}end`;
    vi.stubGlobal(
      'fetch',
      mockFetch({
        ...boardRoutes(boardItem(1)),
        [`GET /api/v1/items/${BROKEN}`]: [500, errorBody('internal')],
        ...docRoutes(docItem(LONG, { title: long })),
      }).fetch,
    );
    const r = (app = await openBoardApp(boardItem(1)));
    act(() => r.fake.dropItem(dragged(BROKEN), { x: 1, y: 1 }));
    await screen.findByText(INSERT_FAILED);
    expect(r.fake.calls.filter((c) => c.method === 'insertEmbed')).toHaveLength(0);
    act(() => r.fake.dropItem(dragged(LONG), { x: 5, y: 5 }));
    await waitFor(() => expect(r.fake.inserted).toHaveLength(1));
    const title = r.fake.inserted[0].title;
    expect(Array.from(title)).toHaveLength(MAX_SNAPSHOT_CHARS);
    expect(long.startsWith(title)).toBe(true);
  });

  it('a drop on the editing card is taken in the capture phase and placed at its frame point', async () => {
    const r = (app = await setup());
    const iframe = document.querySelector('iframe') as HTMLIFrameElement;
    iframe.getBoundingClientRect = () => ({ left: 100, top: 50, right: 900, bottom: 650, width: 800, height: 600, x: 100, y: 50, toJSON: () => ({}) }) as DOMRect;
    act(() => {
      r.fake.layout({ embeds: [{ objectId: 'c1', item: DOC_1, x: 0, y: 0 }] });
    });
    await waitFor(() => expect(cardEl('c1')?.dataset.state).toBe('static'));
    act(() => r.fake.openEmbed('c1', null));
    const pm = await waitFor(() => {
      const el = cardEl('c1')?.querySelector<HTMLElement>('.board-embed-editor .ProseMirror[contenteditable="true"]');
      expect(el).toBeTruthy();
      return el as HTMLElement;
    });
    const editor = (pm as unknown as { editor: Editor }).editor;
    const reached = vi.fn();
    pm.addEventListener('drop', reached);
    pm.addEventListener('dragover', reached);
    const data = { [DRAG_ITEM_TYPE]: JSON.stringify(dragged(DOC_2)), 'text/plain': 'Renamed page' };
    const over = dragEvent('dragover', data, 400, 250);
    pm.dispatchEvent(over);
    expect(over.defaultPrevented).toBe(true);
    const drop = dragEvent('drop', data, 400, 250);
    act(() => {
      pm.dispatchEvent(drop);
    });
    expect(drop.defaultPrevented).toBe(true);
    expect(reached).not.toHaveBeenCalled();
    expect(editor.getText()).toBe('kangaroo');
    await waitFor(() => expect(r.fake.inserted).toHaveLength(1));
    expect(r.fake.inserted[0]).toMatchObject({ item: DOC_2, at: { x: 300, y: 200 } });
    // A drag without the type (a file, text) is left to the editor.
    const plain = dragEvent('dragover', { 'text/plain': 'hello' }, 400, 250);
    pm.dispatchEvent(plain);
    expect(reached).toHaveBeenCalledTimes(1);
    // So is a sidebar board (D145): the card's editor embeds it in the document, the board places no card.
    const board = dragEvent('dragover', boardDragDataOf({ id: '0192f0a0-5555-7000-8000-00000000000b', title: 'Plan' }), 400, 250);
    pm.dispatchEvent(board);
    expect(reached).toHaveBeenCalledTimes(2);
    expect(r.fake.inserted).toHaveLength(1);
  });
});
