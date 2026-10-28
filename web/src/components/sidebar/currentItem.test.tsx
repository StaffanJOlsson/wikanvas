import { screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { itemPath } from '../../lib/urls';
import { EXPANDED_KEY } from '../../state/expanded';
import { makeCollection, makeDetail } from '../../test/mockApi';
import { findRow, renderSidebar } from '../../test/sidebarHarness';
import { items } from '../../test/treeFixtures';

describe('the current item in the sidebar', () => {
  const c1 = makeCollection({ id: 'c1', name: 'Handbook', item_count: 4 });
  const c2 = makeCollection({ id: 'c2', name: 'Other', item_count: 1 });
  const list = items('c1', 'A', 'A>A1', 'A1>Deep', 'B');
  const deep = list[2]!;

  it('is highlighted with aria-current, and its collection and ancestors are expanded', async () => {
    const detail = makeDetail(deep, c1, {
      breadcrumbs: [
        { id: 'A', url_id: list[0]!.url_id, kind: 'doc', title: 'A' },
        { id: 'A1', url_id: list[1]!.url_id, kind: 'doc', title: 'A1' },
      ],
    });
    renderSidebar({
      collections: [c1, c2],
      trees: { c1: list, c2: items('c2', 'Z') },
      route: itemPath(deep),
      detail,
      expanded: [],
    });
    const row = await findRow('Deep');
    expect(row.getAttribute('aria-current')).toBe('page');
    expect(row.getAttribute('aria-selected')).toBe('true');
    expect(row.className).toContain('current');
    expect(screen.getByRole('treeitem', { name: 'Handbook' }).getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByRole('treeitem', { name: 'A' }).getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByRole('treeitem', { name: 'A1' }).getAttribute('aria-expanded')).toBe('true');
    // Only the current row is marked, and the other collection stays collapsed.
    expect(screen.getAllByRole('treeitem').filter((r) => r.getAttribute('aria-current') === 'page')).toHaveLength(1);
    expect(screen.getByRole('treeitem', { name: 'Other' }).getAttribute('aria-expanded')).toBe('false');
    // The expansion is remembered.
    expect(JSON.parse(localStorage.getItem(EXPANDED_KEY)!)).toEqual(expect.arrayContaining(['c1', 'A', 'A1']));
    // The current row holds the tree's tab stop.
    expect(row.tabIndex).toBe(0);
  });

  it('a collection page highlights its collection row', async () => {
    renderSidebar({ collections: [c1, c2], route: `/collection/handbook-${c1.url_id}` });
    await waitFor(async () => expect((await findRow('Handbook')).getAttribute('aria-current')).toBe('page'));
    expect(screen.getByRole('treeitem', { name: 'Other' }).getAttribute('aria-current')).toBeNull();
  });
});
