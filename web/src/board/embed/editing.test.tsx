// Editing a card (docs/ARCHITECTURE.md §18.5 "Editing", D65, D66): one card at a time on a DocSession, entered on
// onEmbedOpen, and left on Esc, onEmbedEditEnd, a pointerdown outside, a later layout without it, the board turning
// read-only; never opened on a read-only board or for a dead target.
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import type { Editor } from '@tiptap/react';
import { EditorView } from '@tiptap/pm/view';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import { updateCachedItem } from '../../api/queries/items';
import type { EmbedLayout } from '../bridge';
import {
  boardItem,
  boardRoutes,
  cardEl,
  DOC_1,
  DOC_2,
  docItem,
  docRoutes,
  docStateOf,
  errorBody,
  mockFetch,
  openBoardApp,
  paragraphs,
  type Handler,
  type Recorded,
  type Rendered,
  type Reply,
} from '../testkit';
import { embedController } from './controller';

let app: Rendered | null = null;

afterEach(async () => {
  await embedController()?.settled();
  app?.unmount();
  app = null;
});

const board = boardItem(1);

/** The open layout: zoom 1, origin (400, 300); c1 at (-520, -200), c2 at (40, -200), both 480 × 360. */
const L0 = {
  zoom: 1,
  originX: 400,
  originY: 300,
  embeds: [
    { objectId: 'c1', item: DOC_1, x: -520, y: -200, label: 'First page' },
    { objectId: 'c2', item: DOC_2, x: 40, y: -200, label: 'Second page' },
  ],
};

async function setup(extra: Record<string, Handler | Reply> = {}, item: ItemDetail = board) {
  const m = mockFetch({
    ...boardRoutes(item),
    ...docRoutes(docItem(DOC_1, { title: 'First page' }), paragraphs('kangaroo')),
    ...docRoutes(docItem(DOC_2, { title: 'Second page' }), paragraphs('wombat')),
    ...extra,
  });
  vi.stubGlobal('fetch', m.fetch);
  const r = (app = await openBoardApp(item));
  const iframe = document.querySelector('iframe') as HTMLIFrameElement;
  iframe.getBoundingClientRect = () => ({ left: 100, top: 50, right: 900, bottom: 650, width: 800, height: 600, x: 100, y: 50, toJSON: () => ({}) }) as DOMRect;
  act(() => {
    r.fake.layout(L0);
  });
  await waitFor(() => expect(cardEl('c1')?.dataset.state).toBe('static'));
  await waitFor(() => expect(cardEl('c2')?.dataset.state).toBe('static'));
  return { r, requests: m.requests };
}

/** A route that answers only when the test says so (`release` waits until it was asked). */
function held(reply: Reply) {
  let resolve: ((r: Reply) => void) | null = null;
  const handler: Handler = () => new Promise<Reply>((res) => (resolve = res));
  return {
    handler,
    async release() {
      await waitFor(() => expect(resolve).not.toBeNull());
      resolve?.(reply);
    },
  };
}

const editorDom = (objectId = 'c1') =>
  cardEl(objectId)?.querySelector<HTMLElement>('.board-embed-editor .ProseMirror[contenteditable="true"]') ?? null;
const editorOf = (objectId = 'c1') => (editorDom(objectId) as unknown as { editor: Editor }).editor;
const editingCalls = (r: Rendered) => r.fake.calls.filter((c) => c.method === 'setEmbedEditing').map((c) => c.args[0]);
const posts = (requests: Recorded[], id: string) =>
  requests.filter((q) => q.method === 'POST' && q.path === `/api/v1/items/${id}/doc/updates`);

async function openCard(r: Rendered, objectId = 'c1', point: { x: number; y: number } | null = null) {
  act(() => r.fake.openEmbed(objectId, point));
  await waitFor(() => expect(editorDom(objectId)).not.toBeNull());
}

describe('editing a card', () => {
  it('opens on onEmbedOpen: the engine told, the card focused before the session opens, then the editor with the caret at the point', async () => {
    const doc = held([200, docStateOf(paragraphs('kangaroo'))]);
    const { r, requests } = await setup({ [`GET /api/v1/items/${DOC_1}/doc`]: doc.handler });
    const pos = vi.spyOn(EditorView.prototype, 'posAtCoords').mockReturnValue({ pos: 4, inside: -1 });
    // World (-300, 0) is frame (100, 300) at the open: in c1's body.
    act(() => r.fake.openEmbed('c1', { x: 100, y: 300 }));
    expect(editingCalls(r)).toEqual(['c1']);
    const card = cardEl('c1') as HTMLElement;
    expect(card.dataset.state).toBe('editing');
    expect(document.activeElement).toBe(card);
    expect(card.getAttribute('role')).toBe('region');
    expect(card.getAttribute('aria-label')).toBe('First page (embedded page)');
    // The camera moves while the document loads.
    act(() => {
      r.fake.layout({ ...L0, zoom: 1.5, originX: 700, originY: 260 });
    });
    expect(cardEl('c1')?.dataset.state).toBe('editing');
    await act(() => doc.release());
    await waitFor(() => expect(editorDom()).not.toBeNull());
    // World (-300, 0) is frame (250, 260) now, viewport (350, 310) with the iframe at (100, 50).
    await waitFor(() => expect(pos).toHaveBeenCalledWith({ left: 350, top: 310 }));
    const editor = editorOf();
    expect(editor.state.selection.from).toBe(4);
    expect(document.activeElement).toBe(editorDom());
    expect(editor.getText()).toBe('kangaroo');
    // Typing writes the document, never the board.
    act(() => {
      editor.commands.insertContent(' numbat');
    });
    fireEvent.keyDown(editorDom() as HTMLElement, { key: 'Escape' });
    await waitFor(() => expect(posts(requests, DOC_1)).toHaveLength(1));
    expect(requests.some((q) => q.method === 'PUT')).toBe(false);
    expect(requests.some((q) => q.method === 'POST' && q.path.includes(DOC_2))).toBe(false);
  });

  it('Esc: setEmbedEditing(null), the session flushed, the frame focused; the static card shows the last text', async () => {
    const { r, requests } = await setup();
    await openCard(r);
    act(() => {
      editorOf().commands.insertContent('wombat ');
    });
    const before = r.fake.calls.filter((c) => c.method === 'focus').length;
    fireEvent.keyDown(editorDom() as HTMLElement, { key: 'Escape' });
    expect(editingCalls(r)).toEqual(['c1', null]);
    expect(r.fake.calls.filter((c) => c.method === 'focus').length).toBe(before + 1);
    expect(cardEl('c1')?.dataset.state).toBe('static');
    await waitFor(() => expect(posts(requests, DOC_1)).toHaveLength(1));
    expect(cardEl('c1')?.querySelector('.board-embed-static .ProseMirror')?.textContent).toContain('wombat');
  });

  it('Esc while the session still opens ends it', async () => {
    const doc = held([200, docStateOf(paragraphs('kangaroo'))]);
    const { r, requests } = await setup({ [`GET /api/v1/items/${DOC_1}/doc`]: doc.handler });
    act(() => r.fake.openEmbed('c1', null));
    const card = cardEl('c1') as HTMLElement;
    expect(document.activeElement).toBe(card);
    // The session's GET is out.
    await waitFor(() => expect(requests.some((q) => q.method === 'GET' && q.path === `/api/v1/items/${DOC_1}/doc`)).toBe(true));
    fireEvent.keyDown(card, { key: 'Escape' });
    expect(editingCalls(r)).toEqual(['c1', null]);
    expect(card.dataset.state).toBe('static');
    await act(() => doc.release());
    await act(() => new Promise((resolve) => setTimeout(resolve, 30)));
    expect(editorDom()).toBeNull();
    expect(cardEl('c1')?.dataset.state).toBe('static');
  });

  it('onEmbedEditEnd ends it without moving focus or telling the engine again', async () => {
    const { r } = await setup();
    await openCard(r);
    const focusCalls = r.fake.calls.filter((c) => c.method === 'focus').length;
    act(() => r.fake.endEdit('c1', 'press'));
    expect(cardEl('c1')?.dataset.state).toBe('static');
    expect(editingCalls(r)).toEqual(['c1']);
    expect(r.fake.calls.filter((c) => c.method === 'focus').length).toBe(focusCalls);
  });

  it('the first layout after the open (editing null) keeps it; a later one without the card, or no longer editing it, ends it', async () => {
    const { r } = await setup();
    act(() => r.fake.openEmbed('c1', null));
    // Queued by the engine in the frame of the double-click: still `editing: null`.
    act(() => {
      r.fake.layout({ ...L0, editing: null });
    });
    expect(cardEl('c1')?.dataset.state).toBe('editing');
    act(() => {
      r.fake.layout({ ...L0, editing: 'c1' });
    });
    expect(cardEl('c1')?.dataset.state).toBe('editing');
    act(() => {
      r.fake.layout({ ...L0, editing: null });
    });
    expect(cardEl('c1')?.dataset.state).toBe('static');

    // A layout without the card (deleted, another page, a hidden layer) ends it at once.
    act(() => r.fake.openEmbed('c2', null));
    expect(cardEl('c2')?.dataset.state).toBe('editing');
    act(() => {
      r.fake.layout({ ...L0, embeds: [L0.embeds[0]], editing: null });
    });
    expect(cardEl('c2')).toBeNull();
    expect(editingCalls(r).at(-1)).toBeNull();
    expect(embedController()?.editingId()).toBeNull();
  });

  it('a pointerdown outside the card ends it; one in the popup root or in the card does not', async () => {
    const { r } = await setup();
    await openCard(r);
    const root = document.querySelector('.board-embed-popups') as HTMLElement;
    const inPopup = document.createElement('div');
    root.appendChild(inPopup);
    fireEvent.pointerDown(inPopup);
    fireEvent.pointerDown(editorDom() as HTMLElement);
    expect(cardEl('c1')?.dataset.state).toBe('editing');
    const before = r.fake.calls.filter((c) => c.method === 'focus').length;
    fireEvent.pointerDown(document.body);
    expect(cardEl('c1')?.dataset.state).toBe('static');
    expect(editingCalls(r)).toEqual(['c1', null]);
    expect(r.fake.calls.filter((c) => c.method === 'focus').length).toBe(before);
  });

  it('an open of the editing card changes nothing; another card ends and flushes the first before it opens', async () => {
    const { r, requests } = await setup();
    await openCard(r);
    act(() => r.fake.openEmbed('c1', null));
    expect(editingCalls(r)).toEqual(['c1']);
    act(() => {
      editorOf().commands.insertContent('typed ');
    });
    act(() => r.fake.openEmbed('c2', null));
    expect(cardEl('c1')?.dataset.state).toBe('static');
    expect(cardEl('c2')?.dataset.state).toBe('editing');
    expect(editingCalls(r)).toEqual(['c1', 'c2']);
    await waitFor(() => expect(editorDom('c2')).not.toBeNull());
    const post = requests.findIndex((q) => q.method === 'POST' && q.path === `/api/v1/items/${DOC_1}/doc/updates`);
    const get = requests.findIndex((q) => q.method === 'GET' && q.path === `/api/v1/items/${DOC_2}/doc`);
    expect(post).toBeGreaterThanOrEqual(0);
    expect(get).toBeGreaterThan(post);
  });

  it('never opens for a trashed, archived or missing target', async () => {
    const trashed = '0192f0e0-0000-7000-8000-0000000000a1';
    const archived = '0192f0e0-0000-7000-8000-0000000000a2';
    const missing = '0192f0e0-0000-7000-8000-0000000000a3';
    const { r } = await setup({
      ...docRoutes(docItem(trashed, {}, { trashed_at: '2026-10-28T10:00:00.000Z' })),
      ...docRoutes(docItem(archived, {}, { archived_at: '2026-10-28T10:00:00.000Z' })),
      [`GET /api/v1/items/${missing}`]: [404, errorBody('not_found')],
    });
    const embeds = [
      { objectId: 't', item: trashed, x: 0, y: 0 },
      { objectId: 'a', item: archived, x: 500, y: 0 },
      { objectId: 'm', item: missing, x: 0, y: 400 },
      { objectId: 'n', item: 'not-a-uuid', x: 500, y: 400 },
    ];
    act(() => {
      r.fake.layout({ embeds });
    });
    await waitFor(() => expect(cardEl('t')?.dataset.state).toBe('trashed'));
    await waitFor(() => expect(cardEl('a')?.dataset.state).toBe('archived'));
    await waitFor(() => expect(cardEl('m')?.dataset.state).toBe('missing'));
    act(() => {
      for (const id of ['t', 'a', 'm', 'n']) r.fake.openEmbed(id, null);
    });
    expect(editingCalls(r)).toEqual([]);
  });

  it('never opens on a read-only board', async () => {
    const ro = boardItem(1, {}, );
    const trashedBoard: ItemDetail = { ...ro, trashed_at: '2026-10-28T10:00:00.000Z' };
    const { r } = await setup({}, trashedBoard);
    // The engine sends nothing while read-only; the host refuses an event anyway.
    act(() => {
      r.fake.events()?.onEmbedOpen?.({ itemId: 'board-1', objectId: 'c1', item: DOC_1, point: null });
    });
    expect(editingCalls(r)).toEqual([]);
    expect(cardEl('c1')?.dataset.state).toBe('static');
  });

  it('ends when the board turns read-only', async () => {
    const { r } = await setup();
    await openCard(r);
    act(() => {
      updateCachedItem(r.queryClient, 'board-1', (d) => ({ ...d, trashed_at: '2026-10-28T10:00:00.000Z' }));
    });
    await waitFor(() => expect(cardEl('c1')?.dataset.state).toBe('static'));
    expect(editingCalls(r)).toEqual(['c1', null]);
  });

  it('an open failure shows the toast and ends the editing', async () => {
    const { r } = await setup({ [`GET /api/v1/items/${DOC_1}/doc`]: [500, errorBody('internal')] });
    act(() => r.fake.openEmbed('c1', null));
    await screen.findByText('Could not open the page');
    expect(editingCalls(r)).toEqual(['c1', null]);
    expect(cardEl('c1')?.dataset.state).toBe('static');
  });

  it('an undrawn card whose target is missing ends silently; one whose target cannot be fetched ends with the toast', async () => {
    const MISSING = '0192f0e0-0000-7000-8000-0000000000e4';
    const BROKEN = '0192f0e0-0000-7000-8000-0000000000e5';
    const { r } = await setup({
      [`GET /api/v1/items/${MISSING}`]: [404, errorBody('not_found')],
      [`GET /api/v1/items/${BROKEN}`]: [500, errorBody('internal')],
    });
    // Zoomed out below 0.5: no static card is drawn, so these targets were never fetched (E3-D5).
    const embeds = [
      ...L0.embeds,
      { objectId: 'm', item: MISSING, x: 600, y: -200 },
      { objectId: 'b', item: BROKEN, x: 1200, y: -200 },
    ];
    act(() => {
      r.fake.layout({ ...L0, zoom: 0.3, embeds });
    });
    act(() => r.fake.openEmbed('m', null));
    await waitFor(() => expect(editingCalls(r)).toEqual(['m', null]));
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(screen.queryByText('Could not open the page')).toBeNull();
    act(() => r.fake.openEmbed('b', null));
    await screen.findByText('Could not open the page');
    expect(editingCalls(r)).toEqual(['m', null, 'b', null]);
    expect(embedController()?.editingId()).toBeNull();
  });

  it('Esc with the slash menu open closes the menu and keeps editing; the next Esc leaves', async () => {
    const { r } = await setup();
    await openCard(r);
    const editor = editorOf();
    act(() => {
      editor.commands.focus('end');
      for (const ch of ' /') {
        const { from, to } = editor.state.selection;
        editor.view.dispatch(editor.state.tr.insertText(ch, from, to));
      }
    });
    const root = document.querySelector('.board-embed-popups') as HTMLElement;
    await waitFor(() => expect(root.querySelector('[role="listbox"][aria-label="Insert a block"]')).not.toBeNull());
    fireEvent.keyDown(editorDom() as HTMLElement, { key: 'Escape' });
    await waitFor(() => expect(root.querySelector('[role="listbox"]')).toBeNull());
    expect(cardEl('c1')?.dataset.state).toBe('editing');
    expect(editingCalls(r)).toEqual(['c1']);
    fireEvent.keyDown(editorDom() as HTMLElement, { key: 'Escape' });
    expect(cardEl('c1')?.dataset.state).toBe('static');
    expect(editingCalls(r)).toEqual(['c1', null]);
  });

  it('a Ctrl+wheel over the editing card is prevented (no page zoom); a plain wheel is not', async () => {
    const { r } = await setup();
    await openCard(r);
    const pinch = new WheelEvent('wheel', { ctrlKey: true, bubbles: true, cancelable: true, deltaY: -10 });
    // happy-dom's WheelEvent is a UIEvent without modifier keys; browsers' is a MouseEvent with `ctrlKey`.
    if (!pinch.ctrlKey) Object.defineProperty(pinch, 'ctrlKey', { value: true });
    editorDom()?.dispatchEvent(pinch);
    expect(pinch.defaultPrevented).toBe(true);
    const scroll = new WheelEvent('wheel', { bubbles: true, cancelable: true, deltaY: 10 });
    editorDom()?.dispatchEvent(scroll);
    expect(scroll.defaultPrevented).toBe(false);
  });

  it('a press in the body below the text puts the caret at the end', async () => {
    const { r } = await setup();
    await openCard(r);
    const editor = editorOf();
    act(() => {
      editor.commands.setTextSelection(1);
    });
    const body = cardEl('c1')?.querySelector('.board-embed-body') as HTMLElement;
    const down = new MouseEvent('mousedown', { bubbles: true, cancelable: true });
    act(() => {
      body.dispatchEvent(down);
    });
    expect(down.defaultPrevented).toBe(true);
    expect(editor.state.selection.from).toBe(editor.state.doc.content.size - 1);
    expect(cardEl('c1')?.dataset.state).toBe('editing');
  });

  it('the editing card is drawn whatever the zoom, and its layout boxes keep coming', async () => {
    const { r } = await setup();
    await openCard(r);
    const small: Partial<EmbedLayout> = { zoom: 0.2, originX: 400, originY: 300 };
    act(() => {
      r.fake.layout({ ...L0, ...small });
    });
    expect(cardEl('c1')?.dataset.state).toBe('editing');
    expect(cardEl('c2')).toBeNull();
  });
});
