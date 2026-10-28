// The embeds overlay (docs/ARCHITECTURE.md §18.5): layouts applied to the DOM without React, hidden and inert with
// the iframe, emptied on a reset, other boards ignored, a layout requested after every open, and the popup root
// following Focus into the frame layer.
import { act, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { resetFrame } from '../host';
import {
  boardItem,
  boardRoutes,
  cardEl,
  DOC_1,
  DOC_2,
  docItem,
  docRoutes,
  mockFetch,
  openBoardApp,
  paragraphs,
  type Rendered,
} from '../testkit';

const renders = vi.hoisted(() => ({ count: 0 }));

vi.mock('./EmbedCard', async (importOriginal) => {
  const mod = await importOriginal<typeof import('./EmbedCard')>();
  const React = await import('react');
  return {
    ...mod,
    EmbedCard: (props: Parameters<typeof mod.EmbedCard>[0]) => {
      renders.count += 1;
      return React.createElement(mod.EmbedCard, props);
    },
  };
});

let app: Rendered | null = null;

afterEach(() => {
  app?.unmount();
  app = null;
});

const board = boardItem(1);

function routes() {
  return {
    ...boardRoutes(board),
    ...docRoutes(docItem(DOC_1, { title: 'First page' }), paragraphs('kangaroo')),
    ...docRoutes(docItem(DOC_2, { title: 'Second page' }), paragraphs('wombat')),
  };
}

const overlay = () => screen.getByTestId('board-embeds');
const world = () => overlay().querySelector('.board-embeds-world') as HTMLElement;

describe('embeds overlay', () => {
  it('applies a layout: the world transform (no will-change) and the inset card boxes', async () => {
    vi.stubGlobal('fetch', mockFetch(routes()).fetch);
    const r = (app = await openBoardApp(board));
    act(() => {
      r.fake.layout({
        zoom: 0.8,
        originX: 400,
        originY: 300,
        embeds: [
          { objectId: 'c1', item: DOC_1, x: -520, y: -200, w: 480, h: 360, label: 'First page' },
          { objectId: 'c2', item: DOC_2, x: 40, y: -200, w: 480, h: 360, angle: 0.25, label: 'Second page' },
        ],
      });
    });
    const w = world();
    expect(w.style.transform).toBe('translate(400px, 300px) scale(0.8)');
    expect(w.style.getPropertyValue('will-change')).toBe('');
    const c1 = cardEl('c1') as HTMLElement;
    const c2 = cardEl('c2') as HTMLElement;
    expect(c1.parentElement).toBe(w);
    expect([c1.style.left, c1.style.top, c1.style.width, c1.style.height]).toEqual(['-508px', '-188px', '456px', '336px']);
    expect(c2.style.transform).toBe('rotate(0.25rad)');
    expect(c1.dataset.itemId).toBe(DOC_1);
    // In drawing order.
    expect([...w.children]).toEqual([c1, c2]);
    await waitFor(() => expect(c1.dataset.state).toBe('static'));
    await waitFor(() => expect(c1.querySelector('.board-embed-static .ProseMirror')?.textContent).toBe('kangaroo'));
    // The engine's title map follows the targets' live titles (§18.7).
    await waitFor(() => expect(r.fake.embedTitles()).toEqual({ [DOC_1]: 'First page', [DOC_2]: 'Second page' }));
  });

  it('a camera-only layout renders no card component again', async () => {
    vi.stubGlobal('fetch', mockFetch(routes()).fetch);
    const r = (app = await openBoardApp(board));
    const embeds = [
      { objectId: 'c1', item: DOC_1, x: -520, y: -200 },
      { objectId: 'c2', item: DOC_2, x: 40, y: -200 },
    ];
    act(() => {
      r.fake.layout({ embeds });
    });
    await waitFor(() => expect(cardEl('c2')?.querySelector('.board-embed-static .ProseMirror')?.textContent).toBe('wombat'));
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    const before = renders.count;
    act(() => {
      for (let i = 1; i <= 10; i++) r.fake.layout({ embeds, zoom: 1 + i / 20, originX: i * 7, originY: -i * 3 });
    });
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(renders.count).toBe(before);
    expect(world().style.transform).toBe('translate(70px, -30px) scale(1.5)');
    // A change of the drawn set does render.
    act(() => {
      r.fake.layout({ embeds: [embeds[0]] });
    });
    expect(cardEl('c2')).toBeNull();
    expect(renders.count).toBeGreaterThan(before);
  });

  it('is hidden and inert with the iframe, and requests a layout after every open', async () => {
    vi.stubGlobal('fetch', mockFetch(routes()).fetch);
    const r = (app = await openBoardApp(board));
    const iframe = document.querySelector('iframe') as HTMLIFrameElement;
    await waitFor(() => expect(iframe.style.visibility).toBe('visible'));
    expect(overlay().hidden).toBe(false);
    expect(overlay().hasAttribute('inert')).toBe(false);
    const methods = r.fake.calls.map((c) => c.method);
    expect(methods.indexOf('requestEmbedLayout')).toBeGreaterThan(methods.indexOf('open'));
    await act(() => r.router.navigate('/home'));
    await waitFor(() => expect(iframe.style.visibility).toBe('hidden'));
    expect(overlay().hidden).toBe(true);
    expect(overlay().hasAttribute('inert')).toBe(true);
  });

  it('is emptied when the frame is reset, and ignores another board’s events', async () => {
    vi.stubGlobal('fetch', mockFetch(routes()).fetch);
    const r = (app = await openBoardApp(board));
    const layout = r.fake.layout({ embeds: [{ objectId: 'c1', item: DOC_1 }] });
    await waitFor(() => expect(cardEl('c1')).not.toBeNull());
    // Another board's layout (an event for a board that is not current) changes nothing.
    act(() => {
      r.fake.events()?.onEmbedLayout?.({ ...layout, itemId: 'board-2', embeds: [] });
    });
    expect(cardEl('c1')).not.toBeNull();
    act(() => resetFrame());
    expect(cardEl('c1')).toBeNull();
    expect(world().children).toHaveLength(0);
  });

  it('the popup root moves into the frame layer while it is full screen, and back', async () => {
    vi.stubGlobal('fetch', mockFetch(routes()).fetch);
    app = await openBoardApp(board);
    const root = document.querySelector('.board-embed-popups') as HTMLElement;
    const layer = document.querySelector('.board-frame-layer') as HTMLElement;
    expect(root.parentElement).toBe(document.body);
    let full: Element | null = layer;
    Object.defineProperty(document, 'fullscreenElement', { configurable: true, get: () => full });
    try {
      act(() => {
        document.dispatchEvent(new Event('fullscreenchange'));
      });
      expect(root.parentElement).toBe(layer);
      full = null;
      act(() => {
        document.dispatchEvent(new Event('fullscreenchange'));
      });
      expect(root.parentElement).toBe(document.body);
    } finally {
      delete (document as unknown as { fullscreenElement?: unknown }).fullscreenElement;
    }
  });
});
