import { act, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { IDLE_REMOVE_MS } from './BoardFrameHost';
import type { OpenArgs } from './bridge';
import { useHostStore } from './host';
import { boardItem, boardPath, boardRoutes, envelope, mockFetch, renderBoardApp, type Rendered } from './testkit';

let app: Rendered | null = null;

afterEach(() => {
  vi.useRealTimers();
  app?.unmount();
  app = null;
});

function opened(r: Rendered): string[] {
  return r.fake.calls.filter((c) => c.method === 'open').map((c) => (c.args[0] as OpenArgs).itemId);
}

function lastActive(r: Rendered): boolean | undefined {
  const calls = r.fake.calls.filter((c) => c.method === 'setActive');
  return calls.at(-1)?.args[0] as boolean | undefined;
}

describe('BoardFrameHost', () => {
  it('one iframe across two boards; hidden, inert and setActive(false) off route; removed after 10 minutes', async () => {
    const a = boardItem(1);
    const b = boardItem(2);
    const { fetch } = mockFetch({ ...boardRoutes(a, envelope('A')), ...boardRoutes(b, envelope('B')) });
    vi.stubGlobal('fetch', fetch);
    const r = (app = renderBoardApp(boardPath(a)));
    const iframes = () => document.querySelectorAll('iframe');

    await waitFor(() => expect(r.fake.api.current()).toBe('board-1'));
    expect(iframes()).toHaveLength(1);
    const iframe = iframes()[0];
    expect(iframe.dataset.src).toBe('/inkboard/abc123def456/frame.html');
    await waitFor(() => expect(iframe.style.visibility).toBe('visible'));
    expect(iframe.hasAttribute('inert')).toBe(false);
    expect(r.fake.board()).toBe(envelope('A'));
    expect(lastActive(r)).toBe(true);

    // Another board: the same iframe, the board swapped with open (setBoard).
    await act(() => r.router.navigate(boardPath(b)));
    await waitFor(() => expect(r.fake.api.current()).toBe('board-2'));
    expect(r.fake.board()).toBe(envelope('B'));
    expect(iframes()).toHaveLength(1);
    expect(iframes()[0]).toBe(iframe);
    expect(r.fake.iframes).toHaveLength(1);
    expect(opened(r)).toEqual(['board-1', 'board-2']);

    // Off board routes: hidden, inert, paused.
    await act(() => r.router.navigate('/home'));
    await waitFor(() => expect(iframe.style.visibility).toBe('hidden'));
    expect(iframe.hasAttribute('inert')).toBe(true);
    expect(lastActive(r)).toBe(false);
    expect(iframe.isConnected).toBe(true);

    // Back on a board: resumed, still the same iframe.
    await act(() => r.router.navigate(boardPath(a)));
    await waitFor(() => expect(r.fake.api.current()).toBe('board-1'));
    await waitFor(() => expect(iframe.style.visibility).toBe('visible'));
    expect(lastActive(r)).toBe(true);
    expect(iframes()).toHaveLength(1);

    // Ten minutes away from boards: removed.
    vi.useFakeTimers();
    await act(async () => {
      await r.router.navigate('/home');
    });
    await act(async () => {
      vi.advanceTimersByTime(IDLE_REMOVE_MS - 1000);
    });
    expect(iframe.isConnected).toBe(true);
    await act(async () => {
      vi.advanceTimersByTime(1000);
    });
    expect(iframe.isConnected).toBe(false);
    expect(iframes()).toHaveLength(0);
    expect(useHostStore.getState().frame).toBeNull();
  });

  it('follows the theme and covers the surface rect', async () => {
    const a = boardItem(1);
    vi.stubGlobal('fetch', mockFetch(boardRoutes(a)).fetch);
    document.documentElement.setAttribute('data-theme', 'dark');
    const r = (app = renderBoardApp(boardPath(a)));
    await waitFor(() => expect(r.fake.api.current()).toBe('board-1'));
    expect(r.fake.theme()).toBe('dark');
    await act(async () => {
      document.documentElement.setAttribute('data-theme', 'light');
    });
    await waitFor(() => expect(r.fake.theme()).toBe('light'));
    const iframe = document.querySelector('iframe') as HTMLIFrameElement;
    expect(iframe.style.width).toMatch(/px$/);
    expect(iframe.style.height).toMatch(/px$/);
    document.documentElement.setAttribute('data-theme', 'system');
  });

  it('a frame.html 404 with a new build in /config shows the update banner', async () => {
    const a = boardItem(1);
    let configCalls = 0;
    const routes = boardRoutes(a);
    const before = routes['GET /api/v1/config'];
    const { fetch } = mockFetch({
      ...routes,
      'GET /api/v1/config': () => {
        configCalls += 1;
        if (configCalls === 1) return before as never;
        const [status, body] = before as [number, { inkboard: { base: string } }];
        return [status, { ...body, inkboard: { ...body.inkboard, base: '/inkboard/fedcba654321/' } }];
      },
      'HEAD /inkboard/abc123def456/frame.html': [404],
    });
    vi.stubGlobal('fetch', fetch);
    const r = (app = renderBoardApp(boardPath(a), { manualConnect: true }));
    await waitFor(() => expect(r.fake.iframes).toHaveLength(1));
    await act(async () => r.fake.rejectConnect('download', 'no frame script', true));
    expect(await screen.findByText(/Quire was updated/)).toBeTruthy();
    expect(configCalls).toBe(2);
    expect(screen.getByRole('button', { name: 'Reload' })).toBeTruthy();
    // Meanwhile the board is shown read-only.
    expect(screen.getByText(/could not start/)).toBeTruthy();
  });

  it('a stub frame (no engine) falls back to the preview without an update banner', async () => {
    const a = boardItem(1);
    vi.stubGlobal('fetch', mockFetch(boardRoutes(a)).fetch);
    const r = (app = renderBoardApp(boardPath(a), { manualConnect: true }));
    await waitFor(() => expect(r.fake.iframes).toHaveLength(1));
    await act(async () => r.fake.rejectConnect('unsupported', 'The whiteboard engine is not part of this build'));
    expect(await screen.findByText(/cannot run the whiteboard editor/)).toBeTruthy();
    expect(screen.queryByText(/Quire was updated/)).toBeNull();
    expect(document.querySelector('iframe')?.style.visibility).toBe('hidden');
  });
});
