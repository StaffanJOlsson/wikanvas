import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ItemSummary } from '../api/gen/ItemSummary';
import { onHostShortcut } from '../app/hotkeys';
import { itemPath } from '../lib/urls';
import {
  boardItem,
  boardPath,
  boardReply,
  boardRoutes,
  envelope,
  mockFetch,
  putResult,
  renderBoardApp,
  type Recorded,
  type Rendered,
  type Reply,
} from './testkit';

let app: Rendered | null = null;

afterEach(() => {
  app?.unmount();
  app = null;
});

const puts = (requests: Recorded[]) => requests.filter((q) => q.method === 'PUT');

describe('board page', () => {
  it('reopening a board whose last PUT still runs waits for that save, then loads the saved revision', async () => {
    const a = boardItem(1);
    let server = { board: envelope('first'), rev: 1 };
    let answer: (() => void) | null = null;
    const { fetch, requests } = mockFetch({
      ...boardRoutes(a),
      'GET /api/v1/items/board-1/board': () => boardReply(server.board, server.rev),
      'PUT /api/v1/items/board-1/board': (req) => {
        if (req.headers.get('If-Match') !== '"r1"') return [200, putResult(server.rev + 1)];
        // The first save hangs until the test answers it.
        return new Promise<Reply>((resolve) => {
          answer = () => {
            server = { board: req.body ?? '', rev: 2 };
            resolve([200, putResult(2)]);
          };
        });
      },
    });
    vi.stubGlobal('fetch', fetch);
    const r = (app = renderBoardApp(boardPath(a)));
    await waitFor(() => expect(r.fake.board()).toBe(envelope('first')));
    act(() => {
      r.fake.edit(envelope('mine'));
    });
    // Leaving the board flushes it: the PUT starts and hangs.
    await act(() => r.router.navigate('/home'));
    await waitFor(() => expect(puts(requests)).toHaveLength(1));
    // Back on the board before the PUT is answered: no GET yet.
    await act(() => r.router.navigate(boardPath(a)));
    const gets = () => requests.filter((q) => q.method === 'GET' && q.path === '/api/v1/items/board-1/board');
    await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
    expect(gets()).toHaveLength(1);
    await act(async () => answer?.());
    // Then the GET sees the save, and the frame opens it.
    await waitFor(() => expect(gets()).toHaveLength(2));
    await waitFor(() => expect(r.fake.board()).toBe(envelope('mine')));
    // The next save is based on that revision (no 412 against its own save).
    act(() => {
      r.fake.edit(envelope('mine again'));
      r.fake.pressSave();
    });
    await waitFor(() => expect(puts(requests)).toHaveLength(2));
    expect(puts(requests)[1].headers.get('If-Match')).toBe('"r2"');
    expect(screen.queryByText(/changed somewhere else/)).toBeNull();
  });

  it('host keys from the frame go to runHostShortcut', async () => {
    const a = boardItem(2);
    const { fetch, requests } = mockFetch({ ...boardRoutes(a), 'PUT /api/v1/items/board-2/board': [200, putResult(2)] });
    vi.stubGlobal('fetch', fetch);
    const r = (app = renderBoardApp(boardPath(a)));
    await waitFor(() => expect(r.fake.api.current()).toBe('board-2'));
    const quick = vi.fn();
    const sidebar = vi.fn();
    const off = [onHostShortcut('quick-switcher', quick), onHostShortcut('toggle-sidebar', sidebar)];
    try {
      const events = r.fake.events();
      act(() => {
        events?.onHostKey?.({ key: 'k', mod: true, shift: false, alt: false });
        events?.onHostKey?.({ key: '.', mod: true, shift: false, alt: false });
        // Mod+Shift+P (publish) is no Quire shortcut in M1.
        events?.onHostKey?.({ key: 'P', mod: true, shift: true, alt: false });
      });
      expect(quick).toHaveBeenCalledTimes(1);
      expect(sidebar).toHaveBeenCalledTimes(1);
      // Mod+S from the frame saves the pending board.
      act(() => {
        r.fake.edit(envelope('edited'));
        events?.onHostKey?.({ key: 's', mod: true, shift: false, alt: false });
      });
      const mine = () => puts(requests).filter((q) => q.path === '/api/v1/items/board-2/board');
      await waitFor(() => expect(mine()).toHaveLength(1));
      expect(mine()[0].body).toBe(envelope('edited'));
    } finally {
      off.forEach((f) => f());
    }
  });

  it('a board over 15 MB shows the size warning; a small one does not', async () => {
    const a = boardItem(1);
    const b = boardItem(2);
    const big = envelope('x'.repeat(16 * 1024 * 1024));
    vi.stubGlobal('fetch', mockFetch({ ...boardRoutes(a, big), ...boardRoutes(b) }).fetch);
    const r = (app = renderBoardApp(boardPath(a)));
    const warning = await screen.findByText(/This board is 16 MB/);
    expect(warning.closest('[role="status"]')?.textContent).toMatch(/Boards over 50 MB cannot be saved/);
    await act(() => r.router.navigate(boardPath(b)));
    await waitFor(() => expect(r.fake.api.current()).toBe('board-2'));
    expect(screen.queryByText(/This board is/)).toBeNull();
  });

  it('Nested lists the child pages and adds one', async () => {
    const a = boardItem(1, { child_count: 1 });
    const child: ItemSummary = { ...a.item, id: 'doc-5', url_id: 'DoCiD00005', kind: 'doc', title: 'Notes', parent_id: 'board-1' };
    const created: ItemSummary = { ...child, id: 'doc-6', url_id: 'DoCiD00006', title: '' };
    const { fetch, requests } = mockFetch({
      ...boardRoutes(a),
      'GET /api/v1/items/board-1/children': [200, [child]],
      'POST /api/v1/items': [201, created],
    });
    vi.stubGlobal('fetch', fetch);
    const r = (app = renderBoardApp(boardPath(a)));
    const toggle = await screen.findByRole('button', { name: /Nested pages/ });
    expect(toggle.textContent).toContain('1');
    fireEvent.click(toggle);
    const dialog = await screen.findByRole('dialog', { name: 'Nested pages' });
    const link = await within(dialog).findByRole('link', { name: /Notes/ });
    expect(link.getAttribute('href')).toBe(itemPath(child));
    fireEvent.click(within(dialog).getByRole('button', { name: '+ Add a page' }));
    await waitFor(() => expect(r.router.state.location.pathname).toBe(itemPath(created)));
    // The new page may be discarded when it is left empty (DocPage reads `state.created`, D20).
    expect(r.router.state.location.state).toEqual({ created: true });
    const post = requests.find((q) => q.method === 'POST' && q.path === '/api/v1/items');
    expect(JSON.parse(post?.body ?? '{}')).toEqual({ collection_id: 'col-1', parent_id: 'board-1', kind: 'doc' });
  });

  it('Focus shows the frame full screen; the board text is listed for screen readers', async () => {
    const a = boardItem(1);
    vi.stubGlobal('fetch', mockFetch(boardRoutes(a)).fetch);
    const r = (app = renderBoardApp(boardPath(a)));
    await waitFor(() => expect(r.fake.api.current()).toBe('board-1'));
    const iframe = document.querySelector('iframe') as HTMLIFrameElement;
    const fullscreen = vi.fn(async () => {});
    iframe.requestFullscreen = fullscreen;
    const focus = screen.getByRole('button', { name: 'Focus' }) as HTMLButtonElement;
    await waitFor(() => expect(focus.disabled).toBe(false));
    fireEvent.click(focus);
    await waitFor(() => expect(fullscreen).toHaveBeenCalledTimes(1));
    // BoardMeta.text_excerpt, one line per entry, in a visually hidden list.
    const list = screen.getByRole('list', { name: 'Text on this board' });
    expect(list.className).toBe('visually-hidden');
    expect(within(list).getAllByRole('listitem').map((li) => li.textContent)).toEqual(['Page 1', 'Hello']);
  });
});
