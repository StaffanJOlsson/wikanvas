import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { ListedItem } from '../../api/gen/ListedItem';
import type { TitleHit } from '../../api/gen/TitleHit';
import { itemPath } from '../../lib/urls';
import { openDialog, useDialogs } from '../../state/dialogs';
import { makeCollection, makeItem, makeSidebar, mockApi } from '../../test/mockApi';
import { render } from '../../test/render';
import { Overlays } from '../overlays/Overlays';

const mac = /Mac|iPhone|iPad/.test(navigator.userAgent);
const mod = mac ? { metaKey: true } : { ctrlKey: true };

describe('quick switcher', () => {
  const c1 = makeCollection({ id: 'c1', name: 'Handbook' });
  const recent: ListedItem[] = [
    { ...makeItem({ id: 'r1', collection_id: 'c1', title: 'Onboarding' }), path: [], last_viewed_at: '2026-10-28T08:00:00.000Z' },
    { ...makeItem({ id: 'r2', collection_id: 'c1', title: 'Holidays', kind: 'board' }), path: [{ id: 'r1', url_id: 'r100000000', kind: 'doc', title: 'Onboarding' }] },
  ];
  const hits: TitleHit[] = [
    { item: makeItem({ id: 't1', collection_id: 'c1', title: 'Release process' }), path: [] },
    { item: makeItem({ id: 't2', collection_id: 'c1', title: 'Release notes' }), path: [] },
  ];

  function setup() {
    const api = mockApi()
      .on('GET', '/sidebar', [200, makeSidebar([c1])])
      .on('GET', '/recent', [200, { items: recent, next_cursor: 'more' }])
      .on('GET', '/search/titles', (req) => [200, req.query.get('q') === 'rel' ? hits : []]);
    const view = render(<Overlays />, { route: '/home' });
    act(() => openDialog({ kind: 'quick-switcher' }));
    return { ...view, api };
  }

  it('an empty query shows recently viewed items, with their paths', async () => {
    const { api } = setup();
    await waitFor(() => expect(screen.getAllByRole('option')).toHaveLength(2));
    const options = screen.getAllByRole('option');
    expect(options.map((o) => o.querySelector('.switcher-title')?.textContent)).toEqual(['Onboarding', 'Holidays']);
    await waitFor(() => expect(screen.getAllByRole('option')[1]!.textContent).toContain('Handbook / Onboarding'));
    expect(options[0]!.getAttribute('aria-selected')).toBe('true');
    expect(api.calls('GET', '/recent')[0]!.query.get('limit')).toBe('10');
    expect(api.calls('GET', '/search/titles')).toHaveLength(0);
    expect(screen.getByRole('combobox', { name: 'Search titles' })).toBe(document.activeElement);
  });

  it('typing searches titles', async () => {
    const { api } = setup();
    await waitFor(() => expect(screen.getAllByRole('option')).toHaveLength(2));
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'rel' } });
    expect(await screen.findByRole('option', { name: /Release process/ })).toBeTruthy();
    expect(screen.getAllByRole('option').map((o) => o.querySelector('.switcher-title')?.textContent)).toEqual(['Release process', 'Release notes']);
    expect(api.calls('GET', '/search/titles').at(-1)!.query.get('q')).toBe('rel');
    expect(screen.queryByText('Onboarding')).toBeNull();
  });

  it('Enter opens the selected item and closes', async () => {
    const { router } = setup();
    await waitFor(() => expect(screen.getAllByRole('option')).toHaveLength(2));
    const box = screen.getByRole('combobox');
    fireEvent.change(box, { target: { value: 'rel' } });
    await screen.findByRole('option', { name: /Release notes/ });
    fireEvent.keyDown(box, { key: 'ArrowDown' });
    expect(screen.getByRole('option', { name: /Release notes/ }).getAttribute('aria-selected')).toBe('true');
    fireEvent.keyDown(box, { key: 'Enter' });
    await waitFor(() => expect(router.state.location.pathname).toBe(itemPath(hits[1]!.item)));
    expect(useDialogs.getState().active).toBeNull();
    expect(screen.queryByRole('combobox')).toBeNull();
  });

  it('Mod+Enter opens the selected item in a new tab', async () => {
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    const { router } = setup();
    await waitFor(() => expect(screen.getAllByRole('option')).toHaveLength(2));
    fireEvent.keyDown(screen.getByRole('combobox'), { key: 'Enter', ...mod });
    expect(open).toHaveBeenCalledWith(itemPath(recent[0]!), '_blank', 'noopener');
    expect(router.state.location.pathname).toBe('/home');
  });

  it('Esc closes it', async () => {
    setup();
    await waitFor(() => expect(screen.getAllByRole('option')).toHaveLength(2));
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('combobox')).toBeNull());
  });
});
