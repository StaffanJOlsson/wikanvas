// AppShell (docs/ARCHITECTURE.md §9.2, §9.3, §9.6.1, §9.6.3): a route change flushes pending saves (W4's doc session
// relies on it); the drawer below 1024 px; the global keys Mod+S, Mod+. and `?`; `auth-lost` opens the sign-in dialog.
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { AUTH_LOST_EVENT } from '../api/client';
import { registerPending } from '../state/pending';
import { DRAWER_QUERY, setDrawerOpen, useUi } from '../state/ui';
import { makeCollection, makeConfig, makeMe, makeSidebar, mockApi } from '../test/mockApi';
import { renderRoutes } from '../test/render';
import { AppShell } from './AppShell';

// W6 replaces the placeholder frame host; these tests are about the shell around it.
vi.mock('../board/BoardFrameHost', () => ({ default: () => null, useBoardFrame: () => null }));

const mac = /Mac|iPhone|iPad/.test(navigator.userAgent);
const mod = mac ? { metaKey: true } : { ctrlKey: true };

function narrowScreen(narrow: boolean) {
  vi.spyOn(window, 'matchMedia').mockImplementation((q: string) => ({
    matches: narrow && q === DRAWER_QUERY,
    media: q,
    onchange: null,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    addListener: () => undefined,
    removeListener: () => undefined,
    dispatchEvent: () => false,
  }) as MediaQueryList);
}

function setup() {
  const api = mockApi()
    .on('GET', '/auth/me', [200, makeMe()])
    .on('GET', '/config', [200, makeConfig()])
    .on('GET', '/sidebar', [200, makeSidebar([makeCollection({ id: 'c1', name: 'Docs' })])]);
  const view = renderRoutes([{
    path: '/',
    element: <AppShell />,
    children: [
      { path: 'home', element: <h1>Home</h1> },
      { path: 'other', element: <><h1>Other</h1><input aria-label="Field" /></> },
    ],
  }], '/home');
  const shell = () => document.querySelector<HTMLElement>('.app-shell')!;
  return { ...view, api, shell };
}

describe('AppShell', () => {
  it('flushes pending saves when the route changes, not on a query change', async () => {
    const flush = vi.fn(async () => undefined);
    const off = registerPending('doc-1', { hasPending: () => true, flush });
    const { router } = setup();
    await screen.findByRole('heading', { name: 'Home' });
    expect(flush).not.toHaveBeenCalled();
    await act(async () => router.navigate('/home?tab=updated'));
    expect(flush).not.toHaveBeenCalled();
    await act(async () => router.navigate('/other'));
    await screen.findByRole('heading', { name: 'Other' });
    expect(flush).toHaveBeenCalledOnce();
    off();
  });

  it('below 1024 px the sidebar is a drawer: hidden (inert) until opened, closed by the backdrop or a route change', async () => {
    narrowScreen(true);
    const { router, shell } = setup();
    await screen.findByRole('heading', { name: 'Home' });
    expect(shell().dataset.sidebar).toBe('drawer');
    const slot = document.querySelector('.sidebar-slot')!;
    expect(slot.hasAttribute('inert')).toBe(true);

    act(() => setDrawerOpen(true));
    expect(shell().hasAttribute('data-drawer-open')).toBe(true);
    expect(slot.hasAttribute('inert')).toBe(false);
    fireEvent.click(document.querySelector('.drawer-backdrop')!);
    expect(useUi.getState().drawerOpen).toBe(false);
    expect(slot.hasAttribute('inert')).toBe(true);

    act(() => setDrawerOpen(true));
    await act(async () => router.navigate('/other'));
    expect(useUi.getState().drawerOpen).toBe(false);
    expect(shell().hasAttribute('data-drawer-open')).toBe(false);
  });

  it('at 1024 px and wider the sidebar is beside the page; Mod+. collapses it', async () => {
    narrowScreen(false);
    const { shell } = setup();
    await screen.findByRole('heading', { name: 'Home' });
    expect(shell().dataset.sidebar).toBe('expanded');
    expect(document.querySelector('.sidebar-slot')!.hasAttribute('inert')).toBe(false);
    act(() => {
      fireEvent.keyDown(window, { key: '.', ...mod });
    });
    expect(shell().dataset.sidebar).toBe('collapsed');
    expect(document.querySelector('.sidebar-slot')!.hasAttribute('inert')).toBe(true);
  });

  it('Mod+S flushes pending saves and is prevented (no browser save dialog)', async () => {
    const flush = vi.fn(async () => undefined);
    const off = registerPending('doc-1', { hasPending: () => true, flush });
    setup();
    await screen.findByRole('heading', { name: 'Home' });
    let passed = true;
    act(() => {
      passed = fireEvent.keyDown(document.body, { key: 's', ...mod });
    });
    expect(passed).toBe(false);
    expect(flush).toHaveBeenCalledOnce();
    off();
  });

  it('`?` opens the shortcuts dialog outside text fields only', async () => {
    const { router } = setup();
    await screen.findByRole('heading', { name: 'Home' });
    await act(async () => router.navigate('/other'));
    const field = await screen.findByRole('textbox', { name: 'Field' });
    act(() => {
      fireEvent.keyDown(field, { key: '?', shiftKey: true });
    });
    expect(screen.queryByRole('dialog', { name: 'Keyboard shortcuts' })).toBeNull();
    act(() => {
      fireEvent.keyDown(document.body, { key: '?', shiftKey: true });
    });
    expect(screen.getByRole('dialog', { name: 'Keyboard shortcuts' })).toBeTruthy();
  });

  it('an `auth-lost` event opens the sign-in dialog over the page', async () => {
    setup();
    await screen.findByRole('heading', { name: 'Home' });
    expect(screen.queryByRole('dialog', { name: 'Sign in again' })).toBeNull();
    act(() => {
      window.dispatchEvent(new Event(AUTH_LOST_EVENT));
    });
    await waitFor(() => expect(screen.getByRole('dialog', { name: 'Sign in again' })).toBeTruthy());
    // The page stays where it was.
    expect(screen.getByRole('heading', { name: 'Home' })).toBeTruthy();
  });
});
