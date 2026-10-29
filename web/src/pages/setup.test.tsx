// The first-run setup page (docs/ARCHITECTURE.md §8, §9.1).
import { QueryObserver } from '@tanstack/react-query';
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { routes } from '../app/router';
import { errorReply, makeConfig, makeMe, mockApi } from '../test/mockApi';
import { createTestQueryClient, renderRoutes } from '../test/render';

function fill(label: string, value: string) {
  fireEvent.change(screen.getByLabelText(label, { exact: true }), { target: { value } });
}

describe('Setup', () => {
  it('a finished setup goes to Home and never through the sign-in page', async () => {
    let done = false;
    const api = mockApi()
      .on('GET', '/setup', () => [200, { needs_setup: !done }])
      .on('GET', '/auth/me', () => (done ? [200, makeMe()] : errorReply(409, 'setup_required', 'Set up first')))
      .on('POST', '/setup', () => {
        done = true;
        return [201, { user: makeMe().user }];
      });
    // Another query on screen whose refetch is slow: afterSignIn waits for it, so the setup status (now
    // `needs_setup: false`) reaches the page before Setup navigates to Home. In a real browser the router's navigation
    // is a transition, so the same happens even without a slow query.
    const queryClient = createTestQueryClient();
    let fetches = 0;
    let release: (v: number) => void = () => undefined;
    const slow = new QueryObserver(queryClient, {
      queryKey: ['slow'],
      queryFn: () => (++fetches === 1 ? Promise.resolve(1) : new Promise<number>((r) => (release = r))),
    });
    const stop = slow.subscribe(() => undefined);
    const { router } = renderRoutes(routes, '/', queryClient);
    const visited: string[] = [];
    router.subscribe((st) => visited.push(st.location.pathname));
    await waitFor(() => expect(router.state.location.pathname).toBe('/setup'));
    fill('Setup code', 'ABCD-EFGH-IJKL');
    fill('Workspace name', 'Acme');
    fill('Your name', 'Owner');
    fill('Email', 'owner@example.com');
    fill('Password', 'a long password');
    fill('Repeat the password', 'a long password');
    fireEvent.click(screen.getByRole('button', { name: 'Create workspace' }));
    // The refetches after the POST took over a second on a loaded 4-CPU CI runner (main, 7b76978): the default 1 s
    // of waitFor timed out with the button still busy.
    await waitFor(() => expect(api.calls('GET', '/setup').length).toBeGreaterThanOrEqual(2), { timeout: 4000 });
    await waitFor(() => expect(fetches).toBe(2), { timeout: 4000 });
    // The status said "set up" while the owner was still on this page: no detour to /login.
    await new Promise((r) => setTimeout(r, 50));
    release(2);

    expect(await screen.findByRole('heading', { name: 'Home', level: 1 })).toBeTruthy();
    expect(router.state.location.pathname).toBe('/home');
    expect(visited).not.toContain('/login');
    expect(screen.queryByRole('heading', { name: 'Sign in' })).toBeNull();
    expect(api.calls('POST', '/setup')).toHaveLength(1);
    stop();
  });

  it('a workspace that is already set up sends a visitor to the sign-in page', async () => {
    mockApi()
      .on('GET', '/setup', [200, { needs_setup: false }])
      .on('GET', '/config', [200, makeConfig()]);
    const { router } = renderRoutes(routes, '/setup');
    await waitFor(() => expect(router.state.location.pathname).toBe('/login'));
    expect(await screen.findByRole('heading', { name: 'Sign in' })).toBeTruthy();
  });
});
