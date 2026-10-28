// The board embed card (docs/ARCHITECTURE.md §9.6.5): the page-0 SVG preview as an <img>, never the engine.
import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { ItemDetail } from '../api/gen/ItemDetail';
import { boardPreviewUrl } from '../lib/urls';
import { createTestQueryClient, render } from '../test/render';
import { BoardEmbedCard } from './views';

const ID = '0192f0a0-3333-7000-8000-00000000000c';

describe('BoardEmbedCard', () => {
  it('renders the page-0 preview <img> and never loads /inkboard/', () => {
    const fetchSpy = vi.fn(async () => new Response('{}', { status: 200 }));
    vi.stubGlobal('fetch', fetchSpy);
    const qc = createTestQueryClient();
    qc.setQueryData(['item', ID], {
      item: { id: ID, url_id: 'BbBbBbBbBb', kind: 'board', title: 'Plan', collection_id: 'c', parent_id: null },
      board: { rev: 7, pages: [{ id: 'p', name: 'Page 1', svg_len: 10 }], objects: 3, background: '#ffffff', bytes: 10,
        preview_rev: 7, text_excerpt: 'Plan excerpt' },
      trashed_at: null,
    } as unknown as ItemDetail);
    const { container } = render(<BoardEmbedCard itemId={ID} />, { queryClient: qc });
    const img = screen.getByRole('img');
    expect(img.getAttribute('src')).toBe(boardPreviewUrl(ID, 7, 0));
    expect(img.getAttribute('src')).toBe(`/api/v1/items/${ID}/board/pages/0?rev=7`);
    expect(screen.getByText('Plan')).toBeTruthy();
    expect(container.querySelector('iframe')).toBeNull();
    expect(container.innerHTML).not.toContain('/inkboard/');
    for (const call of fetchSpy.mock.calls as unknown as [RequestInfo][]) {
      expect(String(call[0])).not.toContain('/inkboard/');
    }
    expect(screen.getByRole('link').getAttribute('href')).toBe('/board/plan-BbBbBbBbBb');
  });
});
