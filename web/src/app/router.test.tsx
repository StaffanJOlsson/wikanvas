import { screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { Me } from '../api/gen/Me';
import { jsonFetch, renderRoutes } from '../test/render';
import { routes } from './router';

const me: Me = {
  user: { id: 'u1', email: 'owner@example.com', name: 'Owner', role: 'owner', theme: 'system' },
  workspace: { name: 'Quire', logo_url: null },
  must_change_password: false,
  auth: { method: 'session', token: null },
};

describe('router', () => {
  it.each([
    ['/home', 'Home'],
    ['/search', 'Search'],
    ['/starred', 'Starred'],
    ['/archive', 'Archive'],
    ['/trash', 'Trash'],
    ['/settings/tokens', 'Settings: API tokens'],
    ['/drafts', 'Page not found'],
    ['/settings/nope', 'Page not found'],
  ])('%s renders %s inside the shell', async (path, heading) => {
    vi.stubGlobal('fetch', jsonFetch({ 'GET /api/v1/auth/me': [200, me] }));
    renderRoutes(routes, path);
    expect(await screen.findByRole('heading', { name: heading })).toBeTruthy();
    expect(document.getElementById('main')).not.toBeNull();
  });

  it('redirects / to /home', async () => {
    vi.stubGlobal('fetch', jsonFetch({ 'GET /api/v1/auth/me': [200, me] }));
    const { router } = renderRoutes(routes, '/');
    await waitFor(() => expect(router.state.location.pathname).toBe('/home'));
  });

  it('AuthGate: 401 → /login?next=, 409 setup_required → /setup, must_change_password → account', async () => {
    vi.stubGlobal('fetch', jsonFetch({
      'GET /api/v1/auth/me': [401, { error: { code: 'unauthenticated', message: 'Sign in' }, request_id: 'r' }],
    }));
    const a = renderRoutes(routes, '/starred?x=1');
    await waitFor(() => expect(a.router.state.location.pathname).toBe('/login'));
    expect(a.router.state.location.search).toBe(`?next=${encodeURIComponent('/starred?x=1')}`);
    a.unmount();

    vi.stubGlobal('fetch', jsonFetch({
      'GET /api/v1/auth/me': [409, { error: { code: 'setup_required', message: 'Set up' }, request_id: 'r' }],
    }));
    const b = renderRoutes(routes, '/home');
    await waitFor(() => expect(b.router.state.location.pathname).toBe('/setup'));
    b.unmount();

    vi.stubGlobal('fetch', jsonFetch({ 'GET /api/v1/auth/me': [200, { ...me, must_change_password: true }] }));
    const c = renderRoutes(routes, '/home');
    await waitFor(() => expect(c.router.state.location.pathname).toBe('/settings/account'));
    expect(c.router.state.location.search).toBe('?force=1');
  });

  it('/setup and /login render without loading /auth/me', async () => {
    const fetch = vi.fn(jsonFetch({}));
    vi.stubGlobal('fetch', fetch);
    renderRoutes(routes, '/login');
    expect(await screen.findByRole('heading', { name: 'Sign in' })).toBeTruthy();
    expect(fetch).not.toHaveBeenCalled();
  });

  it('the lazy DocPage resolves an item by its url_id', async () => {
    vi.stubGlobal('fetch', jsonFetch({
      'GET /api/v1/auth/me': [200, me],
      'GET /api/v1/items/AbCdEfGh12': [404, { error: { code: 'not_found', message: 'No item' }, request_id: 'r' }],
    }));
    renderRoutes(routes, '/doc/some-title-AbCdEfGh12');
    expect(await screen.findByRole('heading', { name: 'Page not found' })).toBeTruthy();
  });
});
