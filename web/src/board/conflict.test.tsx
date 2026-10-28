import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ItemSummary } from '../api/gen/ItemSummary';
import {
  boardItem,
  boardPath,
  boardReply,
  boardRoutes,
  envelope,
  errorBody,
  mockFetch,
  putResult,
  renderBoardApp,
  type Rendered,
} from './testkit';
import { readUnsaved } from './unsaved';

let app: Rendered | null = null;

afterEach(() => {
  app?.unmount();
  app = null;
});

/** The user edits the board and presses Ctrl/Cmd+S in the canvas. */
function editAndSave(r: Rendered, board: string) {
  act(() => {
    r.fake.edit(board);
    r.fake.pressSave();
  });
}

describe('ConflictBanner', () => {
  it('a 412 shows the banner; "Load latest" opens the server board and saving resumes from its rev', async () => {
    const a = boardItem(1);
    let server = { board: envelope('first'), rev: 1 };
    let conflict = true;
    const { fetch, requests } = mockFetch({
      ...boardRoutes(a),
      'GET /api/v1/items/board-1/board': () => boardReply(server.board, server.rev),
      'PUT /api/v1/items/board-1/board': () =>
        conflict ? [412, errorBody('rev_mismatch', { rev: 3, etag: '"r3"' })] : [200, putResult(4)],
    });
    vi.stubGlobal('fetch', fetch);
    const r = (app = renderBoardApp(boardPath(a)));
    await waitFor(() => expect(r.fake.board()).toBe(envelope('first')));

    server = { board: envelope('theirs'), rev: 3 };
    editAndSave(r, envelope('mine'));
    const banner = await screen.findByText(/changed somewhere else/);
    expect(banner.closest('[role="alert"]')).not.toBeNull();
    const put = requests.find((q) => q.method === 'PUT');
    expect(put?.headers.get('If-Match')).toBe('"r1"');
    expect(put?.body).toBe(envelope('mine'));
    // The snapshot that could not be saved is still in IndexedDB.
    expect((await readUnsaved('board-1'))?.board).toBe(envelope('mine'));

    fireEvent.click(screen.getByRole('button', { name: 'Load latest' }));
    await waitFor(() => expect(r.fake.board()).toBe(envelope('theirs')));
    await waitFor(() => expect(screen.queryByText(/changed somewhere else/)).toBeNull());
    expect(await readUnsaved('board-1')).toBeUndefined();

    // Saving resumes, based on the loaded revision.
    conflict = false;
    editAndSave(r, envelope('mine again'));
    await waitFor(() => expect(requests.filter((q) => q.method === 'PUT')).toHaveLength(2));
    const second = requests.filter((q) => q.method === 'PUT')[1];
    expect(second.headers.get('If-Match')).toBe('"r3"');
    expect(second.body).toBe(envelope('mine again'));
  });

  it('"Keep mine as a copy" creates a sibling board and saves my board to it at r1', async () => {
    const a = boardItem(1, { parent_id: 'parent-1' });
    const copy = boardItem(9, { title: 'Board 1 (copy)', parent_id: 'parent-1' });
    const created: ItemSummary = copy.item;
    const { fetch, requests } = mockFetch({
      ...boardRoutes(a),
      ...boardRoutes(copy, envelope('mine')),
      'PUT /api/v1/items/board-1/board': [412, errorBody('rev_mismatch', { rev: 2, etag: '"r2"' })],
      'POST /api/v1/items': [201, created],
      'PUT /api/v1/items/board-9/board': [200, putResult(2)],
    });
    vi.stubGlobal('fetch', fetch);
    const r = (app = renderBoardApp(boardPath(a)));
    await waitFor(() => expect(r.fake.api.current()).toBe('board-1'));
    editAndSave(r, envelope('mine'));
    await screen.findByText(/changed somewhere else/);

    fireEvent.click(screen.getByRole('button', { name: 'Keep mine as a copy' }));
    await waitFor(() => expect(r.router.state.location.pathname).toBe(boardPath(copy)));
    const post = requests.find((q) => q.method === 'POST' && q.path === '/api/v1/items');
    expect(JSON.parse(post?.body ?? '{}')).toEqual({
      collection_id: 'col-1',
      parent_id: 'parent-1',
      kind: 'board',
      title: 'Board 1 (copy)',
    });
    const put = requests.find((q) => q.method === 'PUT' && q.path === '/api/v1/items/board-9/board');
    expect(put?.headers.get('If-Match')).toBe('"r1"');
    expect(put?.body).toBe(envelope('mine'));
    expect(requests.findIndex((q) => q === post)).toBeLessThan(requests.findIndex((q) => q === put));
    // The original's unsaved snapshot is gone; the copy opens in the frame.
    expect(await readUnsaved('board-1')).toBeUndefined();
    await waitFor(() => expect(r.fake.api.current()).toBe('board-9'));
    // The original was never overwritten.
    expect(requests.filter((q) => q.method === 'PUT' && q.path === '/api/v1/items/board-1/board')).toHaveLength(1);
  });
});
