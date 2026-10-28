// Above 300 visible rows the tree is virtualized (D21): only the rows near the viewport are in the DOM, and every row
// still says where it sits (aria-posinset / aria-setsize).
import { screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { makeCollection } from '../../test/mockApi';
import { renderSidebar } from '../../test/sidebarHarness';
import { items } from '../../test/treeFixtures';

describe('virtualized tree', () => {
  it('renders a window of rows, not all 400', async () => {
    // happy-dom has no layout: give the sidebar's scroll area a viewport of 600 px (the virtualizer reads offsetHeight).
    const real = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetHeight')!;
    vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockImplementation(function (this: HTMLElement) {
      return this.classList.contains('sidebar-scroll') ? 600 : (real.get!.call(this) as number);
    });
    const big = makeCollection({ id: 'big', name: 'Big', item_count: 400 });
    const list = items('big', ...Array.from({ length: 400 }, (_, i) => `Item ${String(i).padStart(3, '0')}`));
    renderSidebar({ collections: [big], trees: { big: list }, expanded: ['big'] });
    await waitFor(() => expect(screen.getAllByRole('treeitem').length).toBeGreaterThan(1));
    const rendered = screen.getAllByRole('treeitem');
    expect(rendered.length).toBeLessThan(100);
    const tree = screen.getByRole('tree');
    expect(Number.parseInt(tree.style.height, 10)).toBe(401 * 30);
    const first = rendered.find((r) => r.getAttribute('aria-label') === 'Item 000')!;
    expect(first.getAttribute('aria-setsize')).toBe('400');
    expect(first.getAttribute('aria-posinset')).toBe('1');
  });
});
