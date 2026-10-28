import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { fallbackReason } from './fallback';
import { PHONE_QUERY } from './theme';
import { BASE, boardItem, boardPath, boardRoutes, mockFetch, renderBoardApp, type Rendered } from './testkit';

let app: Rendered | null = null;

afterEach(() => {
  app?.unmount();
  app = null;
});

/** The preview's <img> of page 0 at the board's rev. */
function previewImg(): HTMLImageElement | null {
  return document.querySelector('.board-preview img');
}

function stubMatchMedia(matching: string[]) {
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: matching.includes(query),
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  }));
}

describe('read-only fallback (BoardPreview)', () => {
  it('config.inkboard === null: "Whiteboard engine not installed", and no iframe', async () => {
    const a = boardItem(1);
    vi.stubGlobal('fetch', mockFetch(boardRoutes(a, undefined, 1, false)).fetch);
    const r = (app = renderBoardApp(boardPath(a)));
    expect(await screen.findByText('Whiteboard engine not installed')).toBeTruthy();
    expect(previewImg()?.getAttribute('src')).toBe('/api/v1/items/board-1/board/pages/0?rev=1');
    expect(document.querySelector('iframe')).toBeNull();
    expect(r.fake.iframes).toHaveLength(0);
  });

  it('a webgl error from the frame opens the preview with the reason', async () => {
    const a = boardItem(1);
    vi.stubGlobal('fetch', mockFetch(boardRoutes(a)).fetch);
    const r = (app = renderBoardApp(boardPath(a), { manualConnect: true }));
    await waitFor(() => expect(r.fake.iframes).toHaveLength(1));
    await act(async () => r.fake.rejectConnect('webgl', 'WebGL2 is not available'));
    expect(await screen.findByText(/cannot run the whiteboard editor: WebGL2 is not available/)).toBeTruthy();
    expect(previewImg()).not.toBeNull();
    expect(document.querySelector('iframe')?.style.visibility).toBe('hidden');
  });

  it('a panic says up to when changes were saved, and Reload recreates the iframe', async () => {
    const a = boardItem(1);
    vi.stubGlobal('fetch', mockFetch(boardRoutes(a)).fetch);
    const r = (app = renderBoardApp(boardPath(a)));
    await waitFor(() => expect(r.fake.api.current()).toBe('board-1'));
    act(() => r.fake.fail('panic', 'RuntimeError: unreachable'));
    expect(await screen.findByText(/The whiteboard engine stopped\. Changes up to \d{1,2}:\d{2} were saved\./)).toBeTruthy();
    const first = document.querySelector('iframe');
    const opens = () => r.fake.calls.filter((c) => c.method === 'open').length;
    const before = opens();
    fireEvent.click(screen.getByRole('button', { name: 'Reload the editor' }));
    await waitFor(() => expect(r.fake.iframes).toHaveLength(2));
    expect(first?.isConnected).toBe(false);
    expect(document.querySelectorAll('iframe')).toHaveLength(1);
    await waitFor(() => expect(screen.queryByText(/engine stopped/)).toBeNull());
    await waitFor(() => expect(opens()).toBeGreaterThan(before));
    expect(r.fake.api.current()).toBe('board-1');
  });

  it('a coarse pointer under 768 px opens the preview, with "Open editor"', async () => {
    stubMatchMedia([PHONE_QUERY]);
    const a = boardItem(1);
    vi.stubGlobal('fetch', mockFetch(boardRoutes(a)).fetch);
    const r = (app = renderBoardApp(boardPath(a)));
    expect(await screen.findByText(/read-only on small touch screens/)).toBeTruthy();
    expect(r.fake.iframes).toHaveLength(0);
    fireEvent.click(screen.getByRole('button', { name: 'Open editor' }));
    await waitFor(() => expect(r.fake.api.current()).toBe('board-1'));
    expect(r.fake.iframes).toHaveLength(1);
    expect(r.fake.iframes[0].dataset.src).toBe(`${BASE}frame.html`);
  });
});

describe('fallbackReason', () => {
  const engine = { inkboard: { base: BASE, engine_version: 'e', format_version: 2, wasm_bytes: { raw: 1, br: 1, gz: 1 } } };
  it('orders the reasons', () => {
    expect(fallbackReason({ config: engine, failure: null, hostless: false, phone: false })).toBeNull();
    expect(fallbackReason({ config: undefined, failure: null, hostless: false, phone: false })).toBeNull();
    expect(fallbackReason({ config: { inkboard: null }, failure: null, hostless: false, phone: false })?.kind).toBe(
      'no-engine',
    );
    expect(fallbackReason({ config: engine, failure: null, hostless: true, phone: false })?.kind).toBe('no-engine');
    expect(
      fallbackReason({ config: engine, failure: { kind: 'unsupported', message: 'x' }, hostless: false, phone: false })
        ?.kind,
    ).toBe('unsupported');
    expect(
      fallbackReason({ config: engine, failure: { kind: 'start', message: 'x' }, hostless: false, phone: false })?.kind,
    ).toBe('failed');
    expect(
      fallbackReason({ config: engine, failure: { kind: 'panic', message: 'x' }, hostless: false, phone: true })?.kind,
    ).toBe('panic');
    expect(fallbackReason({ config: engine, failure: null, hostless: false, phone: true })?.kind).toBe('phone');
  });
});
