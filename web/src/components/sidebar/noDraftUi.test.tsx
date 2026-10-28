// D23: items are published on create and M1 has no draft UI, even when the API reports drafts.
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { AppShell } from '../../app/AppShell';
import { runHostShortcut } from '../../app/hotkeys';
import { useDialogs } from '../../state/dialogs';
import { makeCollection, makeConfig, makeDetail, makeMe, makeSidebar, mockApi } from '../../test/mockApi';
import { render } from '../../test/render';
import { findRow, renderSidebar } from '../../test/sidebarHarness';
import { items } from '../../test/treeFixtures';
import { TopBar } from '../topbar/TopBar';
import { ItemHeader } from '../item/ItemHeader';
import { ItemMenu } from '../item/ItemMenu';

describe('no draft UI in M1', () => {
  const c1 = makeCollection({ id: 'c1', name: 'Notes', item_count: 2 });
  const list = items('c1', 'Published', 'Unpublished');
  list[1] = { ...list[1]!, published: false };

  it('the sidebar has no Drafts entry or badge, and an unpublished item looks like any other row', async () => {
    renderSidebar({ collections: [c1], trees: { c1: list }, draftsCount: 5, expanded: ['c1'], stars: [list[1]!] });
    const published = await findRow('Published');
    const unpublished = await findRow('Unpublished');
    expect(screen.queryByRole('link', { name: /draft/i })).toBeNull();
    expect(screen.queryByText(/draft/i)).toBeNull();
    expect(screen.queryByText('5')).toBeNull();
    expect(unpublished.className).toBe(published.className);
    expect(unpublished.textContent).toBe('Unpublished');
    expect(unpublished.querySelector('.draft, [data-draft]')).toBeNull();
    // The starred copy (and the tree row's label) render plainly too.
    expect(screen.getAllByRole('link', { name: 'Unpublished' })).toHaveLength(2);
    // Nav: Home, Search, Starred only.
    const nav = screen.getByRole('navigation', { name: 'Main' });
    expect([...nav.querySelectorAll('a')].map((a) => a.textContent)).toEqual(['Home', 'Search', 'Starred']);
  });

  it('the top bar has no Publish button and the meta line no publication state', () => {
    mockApi();
    const detail = makeDetail({ ...list[1]!, title: 'Cats' }, c1, { effectively_published: false });
    render(
      <>
        <TopBar item={detail} actions={<ItemMenu item={detail} extraExports={[]} />} />
        <ItemHeader item={detail} onEnterBody={() => undefined} readOnly={false} />
      </>,
    );
    expect(screen.queryByRole('button', { name: /publish/i })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Item menu' }));
    expect([...document.querySelectorAll('[role^="menuitem"]')].map((m) => m.textContent)).not.toContain('Publish');
    expect(document.body.textContent).not.toMatch(/draft|published/i);
  });

  it('Mod+Shift+P does nothing', async () => {
    const api = mockApi();
    api.on('GET', '/auth/me', [200, makeMe()]);
    api.on('GET', '/sidebar', [200, makeSidebar([c1])]);
    api.on('GET', '/config', [200, makeConfig()]);
    render(<AppShell />, { route: '/home' });
    await screen.findByRole('treeitem', { name: 'Notes' });
    const before = api.calls().length;
    expect(runHostShortcut({ key: 'p', mod: true, shift: true, alt: false })).toBe(false);
    expect(runHostShortcut({ key: 'P', mod: true, shift: true, alt: false })).toBe(false);
    for (const init of [{ ctrlKey: true }, { metaKey: true }]) {
      const e = new KeyboardEvent('keydown', { key: 'P', shiftKey: true, cancelable: true, bubbles: true, ...init });
      act(() => {
        window.dispatchEvent(e);
      });
      expect(e.defaultPrevented).toBe(false);
    }
    expect(useDialogs.getState().active).toBeNull();
    expect(api.calls().slice(before).filter((c) => c.method !== 'GET')).toHaveLength(0);
    // The listener is live: Mod+K does open the quick switcher.
    const mac = /Mac|iPhone|iPad/.test(navigator.userAgent);
    act(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', ctrlKey: !mac, metaKey: mac, cancelable: true }));
    });
    await waitFor(() => expect(useDialogs.getState().active).toEqual({ kind: 'quick-switcher' }));
  });
});
