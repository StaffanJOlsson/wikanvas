// A static card (docs/ARCHITECTURE.md §18.5 "Static cards", "Target states"): a read-only TipTap viewer of the
// document's ProseMirror JSON, fetched once and revalidated with If-None-Match, never writing, CSP-clean; the empty
// page, the double-click hint, the read-only link, and every target state.
import { act, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import { docJsonKey } from '../../api/queries/docs';
import { createTestQueryClient, render } from '../../test/render';
import { DOC_1, docItem, docRoutes, errorBody, mockFetch, paragraphs, type Handler, type Reply } from '../testkit';
import { EmbedController, type DrawnCard } from './controller';
import { EmbedCard } from './EmbedCard';
import { MAX_STATIC_BLOCKS } from './layout';

const card = (over: Partial<DrawnCard> = {}): DrawnCard => ({
  objectId: 'c1',
  item: DOC_1,
  label: 'Snapshot title',
  selected: false,
  ...over,
});

let controllers: EmbedController[] = [];
afterEach(() => {
  controllers.forEach((c) => c.dispose());
  controllers = [];
});

function show(cards: DrawnCard[], routes: Record<string, Handler | Reply>, readOnly = false) {
  const m = mockFetch(routes);
  vi.stubGlobal('fetch', m.fetch);
  const qc = createTestQueryClient();
  const controller = new EmbedController(qc);
  controllers.push(controller);
  const ui = (
    <div className="board-embeds-world">
      {cards.map((c) => (
        <EmbedCard key={c.objectId} card={c} editing={null} readOnly={readOnly} controller={controller} />
      ))}
    </div>
  );
  const r = render(ui, { queryClient: qc, routes: [{ path: '/doc/:slugId', element: <h1>Document page</h1> }] });
  return { ...r, requests: m.requests, qc, controller };
}

const el = (objectId = 'c1') => document.querySelector(`[data-object-id="${objectId}"]`) as HTMLElement;
const stateOf = (objectId = 'c1') => el(objectId)?.dataset.state;

describe('static card', () => {
  it('shows the document read-only, fetched once and revalidated with If-None-Match (304 keeps it); never writes', async () => {
    const styles = document.head.querySelectorAll('style').length;
    const { requests, qc } = show(
      [card(), card({ objectId: 'c2' })],
      docRoutes(docItem(DOC_1, { title: 'Live title' }), paragraphs('kangaroo', 'quokka'), 4),
    );
    await waitFor(() => expect(stateOf()).toBe('static'));
    await waitFor(() => expect(el().querySelector('.board-embed-static .ProseMirror')?.textContent).toBe('kangarooquokka'));
    const pm = el().querySelector('.board-embed-static .ProseMirror') as HTMLElement;
    expect(pm.getAttribute('contenteditable')).toBe('false');
    expect(el().getAttribute('aria-hidden')).toBe('true');
    expect(el().querySelector('.board-embed-card-title')?.textContent).toBe('Live title');
    const gets = () => requests.filter((q) => q.method === 'GET' && q.path === `/api/v1/items/${DOC_1}/doc/prosemirror`);
    expect(gets()).toHaveLength(1);
    expect(gets()[0].headers.get('If-None-Match')).toBeNull();
    await act(() => qc.invalidateQueries({ queryKey: docJsonKey(DOC_1) }));
    await waitFor(() => expect(gets()).toHaveLength(2));
    expect(gets()[1].headers.get('If-None-Match')).toBe('"d4"');
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(el().querySelector('.board-embed-static .ProseMirror')?.textContent).toBe('kangarooquokka');
    expect(requests.filter((q) => q.method !== 'GET')).toEqual([]);
    expect(document.head.querySelectorAll('style').length).toBe(styles);
  });

  it(`shows only the first ${MAX_STATIC_BLOCKS} blocks`, async () => {
    const texts = Array.from({ length: MAX_STATIC_BLOCKS + 50 }, (_, i) => `block ${i}`);
    show([card()], docRoutes(docItem(DOC_1), paragraphs(...texts)));
    await waitFor(() => expect(el().querySelectorAll('.board-embed-static .ProseMirror > p')).toHaveLength(MAX_STATIC_BLOCKS));
    const ps = el().querySelectorAll('.board-embed-static .ProseMirror > p');
    expect(ps[MAX_STATIC_BLOCKS - 1].textContent).toBe(`block ${MAX_STATIC_BLOCKS - 1}`);
  });

  it('an empty document shows "Empty page"', async () => {
    show([card(), card({ objectId: 'c2', item: '0192f0e0-0000-7000-8000-0000000000e2' })], {
      ...docRoutes(docItem(DOC_1), { type: 'doc', content: [] }),
      ...docRoutes(docItem('0192f0e0-0000-7000-8000-0000000000e2'), paragraphs('')),
    });
    await waitFor(() => expect(el().querySelector('.board-embed-empty')?.textContent).toBe('Empty page'));
    await waitFor(() => expect(el('c2').querySelector('.board-embed-empty')?.textContent).toBe('Empty page'));
    expect(el().querySelector('.ProseMirror')).toBeNull();
  });

  it('"Double-click to edit" shows only while selected on a writable board', async () => {
    const routes = docRoutes(docItem(DOC_1), paragraphs('x'));
    show([card({ selected: true }), card({ objectId: 'c2' })], routes);
    await waitFor(() => expect(stateOf()).toBe('static'));
    await waitFor(() => expect(stateOf('c2')).toBe('static'));
    expect(el().querySelector('.board-embed-hint')?.textContent).toBe('Double-click to edit');
    expect(el('c2').querySelector('.board-embed-hint')).toBeNull();
  });

  it('on a read-only board the header title is a link to the page, and there is no hint', async () => {
    const d = docItem(DOC_1, { title: 'Linked page', url_id: 'LiNkEd0001' });
    show([card({ selected: true })], docRoutes(d, paragraphs('x')), true);
    await waitFor(() => expect(stateOf()).toBe('static'));
    const link = el().querySelector('a.board-embed-card-title') as HTMLAnchorElement;
    expect(link.textContent).toBe('Linked page');
    expect(link.getAttribute('href')).toBe('/doc/linked-page-LiNkEd0001');
    expect(el().querySelector('.board-embed-hint')).toBeNull();
  });

  it('loading shows the label snapshot and an empty body', async () => {
    show([card()], { [`GET /api/v1/items/${DOC_1}`]: () => new Promise<Reply>(() => undefined) });
    await waitFor(() => expect(stateOf()).toBe('loading'));
    expect(el().querySelector('.board-embed-card-title')?.textContent).toBe('Snapshot title');
    expect(el().querySelector('.board-embed-body')?.childElementCount).toBe(0);
  });

  it('trashed and archived targets are dimmed with a badge', async () => {
    const t = '0192f0e0-0000-7000-8000-0000000000a1';
    const a = '0192f0e0-0000-7000-8000-0000000000a2';
    show([card({ item: t }), card({ objectId: 'c2', item: a })], {
      ...docRoutes(docItem(t, {}, { trashed_at: '2026-10-28T10:00:00.000Z' }), paragraphs('gone')),
      ...docRoutes(docItem(a, { archived: true }, { archived_at: '2026-10-28T10:00:00.000Z' }), paragraphs('kept')),
    });
    await waitFor(() => expect(stateOf()).toBe('trashed'));
    await waitFor(() => expect(stateOf('c2')).toBe('archived'));
    expect(el().querySelector('.board-embed-badge')?.textContent).toBe('In Trash');
    expect(el('c2').querySelector('.board-embed-badge')?.textContent).toBe('Archived');
    await waitFor(() => expect(el().querySelector('.ProseMirror')?.textContent).toBe('gone'));
  });

  it('missing: a 404, a non-UUID item and a board target; error: another failure', async () => {
    const board = '0192f0e0-0000-7000-8000-0000000000b0';
    const broken = '0192f0e0-0000-7000-8000-0000000000e0';
    const boardDetail: ItemDetail = { ...docItem(board), item: { ...docItem(board).item, kind: 'board' } };
    const { requests } = show(
      [
        card(),
        card({ objectId: 'c2', item: 'not-a-uuid' }),
        card({ objectId: 'c3', item: board }),
        card({ objectId: 'c4', item: broken }),
      ],
      {
        [`GET /api/v1/items/${DOC_1}`]: [404, errorBody('not_found')],
        [`GET /api/v1/items/${board}`]: [200, boardDetail],
        [`GET /api/v1/items/${broken}`]: [500, errorBody('internal')],
      },
    );
    await waitFor(() => expect(stateOf()).toBe('missing'));
    expect(stateOf('c2')).toBe('missing');
    await waitFor(() => expect(stateOf('c3')).toBe('missing'));
    await waitFor(() => expect(stateOf('c4')).toBe('error'));
    expect(el().querySelector('.board-embed-message')?.textContent).toBe('Page not found');
    expect(el().querySelector('s.board-embed-card-title')?.textContent).toBe('Snapshot title');
    expect(el('c4').querySelector('.board-embed-message')?.textContent).toBe('Could not load this page');
    // A non-UUID item is never fetched; a board's document is never asked for.
    expect(requests.some((q) => q.path.includes('not-a-uuid'))).toBe(false);
    expect(requests.some((q) => q.path === `/api/v1/items/${board}/doc/prosemirror`)).toBe(false);
    expect(screen.queryAllByRole('link')).toHaveLength(0);
  });
});
