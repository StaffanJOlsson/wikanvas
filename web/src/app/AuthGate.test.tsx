import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { errorReply, makeMe, mockApi } from '../test/mockApi';
import { renderRoutes } from '../test/render';
import { AuthGate } from './AuthGate';
import { routes as appRoutes } from './router';

const routes = [
  { path: '/login', element: <h1>Login page</h1> },
  { path: '/setup', element: <h1>Setup page</h1> },
  {
    Component: AuthGate,
    children: [
      { path: '/settings/account', element: <h1>Account page</h1> },
      { path: '*', element: <h1>Inside</h1> },
    ],
  },
];

describe('AuthGate', () => {
  it('401 → /login?next=<the page>', async () => {
    mockApi().on('GET', '/auth/me', errorReply(401, 'unauthenticated', 'Sign in'));
    const { router } = renderRoutes(routes, '/doc/plan-AbCdEfGh12?x=1#h-2');
    await waitFor(() => expect(router.state.location.pathname).toBe('/login'));
    expect(router.state.location.search).toBe(`?next=${encodeURIComponent('/doc/plan-AbCdEfGh12?x=1#h-2')}`);
  });

  it('409 setup_required → /setup', async () => {
    mockApi().on('GET', '/auth/me', errorReply(409, 'setup_required', 'Set up first'));
    const { router } = renderRoutes(routes, '/home');
    await waitFor(() => expect(router.state.location.pathname).toBe('/setup'));
  });

  it('must_change_password → the account settings (force), and it stays there', async () => {
    mockApi().on('GET', '/auth/me', [200, makeMe({ must_change_password: true })]);
    const { router } = renderRoutes(routes, '/home');
    await waitFor(() => expect(router.state.location.pathname).toBe('/settings/account'));
    expect(router.state.location.search).toBe('?force=1');
    expect(await screen.findByRole('heading', { name: 'Account page' })).toBeTruthy();
  });

  it('a signed-in user gets the page, the server theme applies, and /auth/me never sends auth-lost', async () => {
    const api = mockApi().on('GET', '/auth/me', [200, makeMe({ user: { ...makeMe().user, theme: 'dark' } })]);
    renderRoutes(routes, '/home');
    expect(await screen.findByRole('heading', { name: 'Inside' })).toBeTruthy();
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
    expect(localStorage.getItem('quire.theme')).toBe('dark');
    expect(api.calls('GET', '/auth/me')).toHaveLength(1);
  });

  it('another error offers a retry instead of redirecting', async () => {
    mockApi().on('GET', '/auth/me', errorReply(500, 'internal', 'Database locked'));
    const { router } = renderRoutes(routes, '/home');
    expect(await screen.findByRole('alert')).toBeTruthy();
    expect(screen.getByText('Database locked')).toBeTruthy();
    expect(router.state.location.pathname).toBe('/home');
  });

  it('signing in on /login returns to ?next= without bouncing back (the old 401 is dropped)', async () => {
    let signedIn = false;
    const api = mockApi()
      .on('GET', '/auth/me', () => (signedIn ? [200, makeMe()] : errorReply(401, 'unauthenticated', 'Sign in')))
      .on('POST', '/auth/login', () => {
        signedIn = true;
        return [200, { user: makeMe().user }];
      });
    const { router } = renderRoutes(appRoutes, '/starred');
    await waitFor(() => expect(router.state.location.pathname).toBe('/login'));
    fireEvent.change(await screen.findByLabelText('Email'), { target: { value: 'owner@example.com' } });
    fireEvent.change(screen.getByLabelText('Password'), { target: { value: 'a long password' } });
    fireEvent.click(screen.getByRole('button', { name: 'Sign in' }));
    await waitFor(() => expect(router.state.location.pathname).toBe('/starred'));
    expect(await screen.findByRole('heading', { name: 'Starred' })).toBeTruthy();
    expect(api.calls('POST', '/auth/login')[0]!.body).toEqual({ email: 'owner@example.com', password: 'a long password', remember: true });
    expect(router.state.location.pathname).toBe('/starred');
  });
});
