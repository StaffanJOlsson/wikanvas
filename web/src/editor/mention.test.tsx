// Mentions (docs/ARCHITECTURE.md §9.6.5): the item's live title; a trashed target struck through with "In Trash"; a
// missing target "Deleted item", not clickable.
import { screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { ItemDetail } from '../api/gen/ItemDetail';
import { createTestQueryClient, jsonFetch, render } from '../test/render';
import { MentionChip } from './views';

const ID = '0192f0a0-1111-7000-8000-00000000000a';

function detail(over: Partial<ItemDetail> = {}, title = 'Live title'): ItemDetail {
  return {
    item: {
      id: ID, url_id: 'AbCdEfGh12', collection_id: 'c', parent_id: null, kind: 'doc', title, sort_key: 'a',
      child_count: 0, published: true, archived: false, updated_at: '2026-10-28T09:00:00.000Z', content_at: null,
      created_at: '2026-10-28T09:00:00.000Z',
    },
    collection: {
      id: 'c', url_id: 'CcCcCcCcCc', name: 'Notes', description: '', icon: null, color: '#8D8D8D', sort_key: 'a',
      child_order: 'manual', item_count: 1, updated_at: '2026-10-28T09:00:00.000Z',
    },
    breadcrumbs: [], starred: false, effectively_published: true, trashed_at: null, purge_at: null, archived_at: null,
    full_width: false, board: null, meta_version: 1,
    ...over,
  } as ItemDetail;
}

describe('MentionChip', () => {
  it('shows the live title from the query cache, as a link', () => {
    const qc = createTestQueryClient();
    qc.setQueryData(['item', ID], detail());
    render(<MentionChip itemId={ID} />, { queryClient: qc });
    const link = screen.getByRole('link');
    expect(link.textContent).toBe('@Live title');
    expect(link.getAttribute('href')).toBe('/doc/live-title-AbCdEfGh12');
    qc.setQueryData(['item', ID], detail({}, 'Renamed'));
    return waitFor(() => expect(screen.getByRole('link').textContent).toBe('@Renamed'));
  });

  it('strikes through a trashed target with "In Trash"', () => {
    const qc = createTestQueryClient();
    qc.setQueryData(['item', ID], detail({ trashed_at: '2026-10-28T10:00:00.000Z' }));
    render(<MentionChip itemId={ID} />, { queryClient: qc });
    const link = screen.getByRole('link');
    expect(link.querySelector('s')?.textContent).toBe('@Live title');
    expect(link.textContent).toContain('In Trash');
  });

  it('shows a missing target as "Deleted item", not clickable', async () => {
    vi.stubGlobal('fetch', jsonFetch({}));
    render(<MentionChip itemId={ID} />);
    await waitFor(() => expect(screen.getByText('@Deleted item')).toBeTruthy());
    expect(screen.queryByRole('link')).toBeNull();
    expect(screen.getByText('@Deleted item').closest('a')).toBeNull();
  });
});
