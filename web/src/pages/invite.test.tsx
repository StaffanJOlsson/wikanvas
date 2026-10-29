// The invite page `/invite/:token` (docs/ARCHITECTURE.md §9.1, §9.6.6, D148).
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { InviteLookup } from '../api/gen/InviteLookup';
import type { InviteState } from '../api/gen/InviteState';
import { routes } from '../app/router';
import { errorReply, makeMe, mockApi } from '../test/mockApi';
import { renderRoutes } from '../test/render';

const TOKEN = 'qinv_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789ABCDEF12345';

function lookup(state: InviteState = 'pending'): InviteLookup {
  return {
    valid: state === 'pending',
    state,
    workspace_name: 'Acme Wiki',
    role: 'member',
    expires_at: '2026-11-05T09:00:00.000Z',
  };
}

/** Fills the field whose label starts with `label` (an error shown in the label follows it). */
function fill(label: string, value: string) {
  fireEvent.change(screen.getByLabelText(new RegExp(`^${label}`)), { target: { value } });
}

function fillForm(password = 'a long enough password') {
  fill('Your name', ' Bea ');
  fill('Email', ' bea@example.com ');
  fill('Password', password);
  fill('Repeat the password', password);
}

describe('Invite page', () => {
  it('shows the workspace and role, creates the account and goes to Home', async () => {
    let accepted = false;
    const member = makeMe({ user: { id: 'u2', email: 'bea@example.com', name: 'Bea', role: 'member', theme: 'system' } });
    const api = mockApi()
      .on('GET', '/invites/:token', [200, lookup()])
      .on('POST', '/invites/:token/accept', () => {
        accepted = true;
        return [201, { user: member.user }];
      })
      .on('GET', '/auth/me', () => (accepted ? [200, member] : errorReply(401, 'unauthenticated')));
    const { router } = renderRoutes(routes, `/invite/${TOKEN}`);
    expect(await screen.findByRole('heading', { name: 'Join Acme Wiki' })).toBeTruthy();
    expect(screen.getByText(/as a member/)).toBeTruthy();
    // The page is outside the shell: it never asked who is signed in.
    expect(api.calls('GET', '/auth/me')).toHaveLength(0);
    expect(api.calls('GET', '/invites/:token')[0]!.params.token).toBe(TOKEN);
    fillForm();
    fireEvent.click(screen.getByRole('button', { name: 'Join Acme Wiki' }));
    await waitFor(() => expect(api.calls('POST', '/invites/:token/accept')).toHaveLength(1));
    expect(api.calls('POST', '/invites/:token/accept')[0]!.body).toEqual({
      name: 'Bea',
      email: 'bea@example.com',
      password: 'a long enough password',
    });
    expect(await screen.findByRole('heading', { name: 'Home', level: 1 }, { timeout: 4000 })).toBeTruthy();
    expect(router.state.location.pathname).toBe('/home');
  });

  it.each([
    ['used', 'This invite was used'],
    ['expired', 'This invite has expired'],
    ['revoked', 'This invite was revoked'],
  ] as const)('a %s link says so and offers sign-in', async (state, heading) => {
    mockApi().on('GET', '/invites/:token', [200, lookup(state)]);
    renderRoutes(routes, `/invite/${TOKEN}`);
    expect(await screen.findByRole('heading', { name: heading })).toBeTruthy();
    expect(screen.queryByLabelText(/^Password/)).toBeNull();
    expect(screen.getByRole('link', { name: 'Sign in' }).getAttribute('href')).toBe('/login');
  });

  it('an unknown link is not valid', async () => {
    mockApi().on('GET', '/invites/:token', errorReply(404, 'not_found', 'no such invite'));
    renderRoutes(routes, '/invite/nonsense');
    expect(await screen.findByRole('heading', { name: 'This invite link is not valid' })).toBeTruthy();
  });

  it('checks the password before sending, shows a taken email inline, and a link that went away', async () => {
    const api = mockApi()
      .on('GET', '/invites/:token', [200, lookup()])
      .on('POST', '/invites/:token/accept', errorReply(409, 'conflict', 'another user has this email', { field: 'email' }));
    renderRoutes(routes, `/invite/${TOKEN}`);
    await screen.findByRole('heading', { name: 'Join Acme Wiki' });
    fillForm('short');
    fireEvent.click(screen.getByRole('button', { name: 'Join Acme Wiki' }));
    expect((await screen.findByRole('alert')).textContent).toContain('at least 12 characters');
    expect(api.calls('POST', '/invites/:token/accept')).toHaveLength(0);
    fill('Password', 'a long enough password');
    fill('Repeat the password', 'another long password');
    fireEvent.click(screen.getByRole('button', { name: 'Join Acme Wiki' }));
    expect((await screen.findByRole('alert')).textContent).toContain('do not match');
    fill('Repeat the password', 'a long enough password');
    fireEvent.click(screen.getByRole('button', { name: 'Join Acme Wiki' }));
    const emailField = screen.getByLabelText(/^Email/).closest('label')!;
    await waitFor(() => expect(emailField.querySelector('[role="alert"]')?.textContent).toContain('Another account has this email'));
    // Used meanwhile (another tab): the accept is 404, and the page says the link is not valid.
    api.on('POST', '/invites/:token/accept', errorReply(404, 'not_found', 'this invite link was used already'));
    fireEvent.click(screen.getByRole('button', { name: 'Join Acme Wiki' }));
    expect(await screen.findByRole('heading', { name: 'This invite link is not valid' })).toBeTruthy();
  });
});
