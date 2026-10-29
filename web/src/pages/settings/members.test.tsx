// Settings → Members (docs/ARCHITECTURE.md §9.6.6, D148).
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { CreatedInvite } from '../../api/gen/CreatedInvite';
import type { Invite } from '../../api/gen/Invite';
import type { NewInvite } from '../../api/gen/NewInvite';
import type { Role } from '../../api/gen/Role';
import type { UserInfo } from '../../api/gen/UserInfo';
import type { UserPatch } from '../../api/gen/UserPatch';
import { Overlays } from '../../components/overlays/Overlays';
import { makeMe, mockApi } from '../../test/mockApi';
import { render } from '../../test/render';
import Settings from '../Settings';
import { assignableRoles } from './Members';

const T0 = '2026-10-20T09:00:00.000Z';
const LINK = 'https://wiki.example.org/invite/qinv_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789ABCDEF12345';

function user(id: string, name: string, role: Role, over: Partial<UserInfo> = {}): UserInfo {
  return { id, email: `${name.toLowerCase()}@example.com`, name, role, created_at: T0, disabled_at: null, ...over };
}

const people: UserInfo[] = [
  user('u1', 'Owner', 'owner'),
  user('u2', 'Ada', 'admin'),
  user('u3', 'Bea', 'member'),
  user('u4', 'Cy', 'viewer', { disabled_at: T0 }),
];

function invite(id: string, role: Role, note: string | null = null): Invite {
  return { id, role, note, created_by: 'u1', created_at: T0, expires_at: '2026-11-05T09:00:00.000Z' };
}

function setup(myRole: Role = 'owner', myId = 'u1') {
  const writeText = vi.fn(async () => undefined);
  Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  let users = people.map((u) => ({ ...u }));
  let invites = [invite('i1', 'viewer', 'For Dee')];
  const me = makeMe({ user: { id: myId, email: 'me@example.com', name: 'Me', role: myRole, theme: 'system' } });
  const api = mockApi()
    .on('GET', '/auth/me', [200, me])
    .on('GET', '/users', () => [200, users])
    .on('PATCH', '/users/:id', (req) => {
      const patch = req.body as UserPatch;
      users = users.map((u) => (u.id === req.params.id
        ? {
          ...u,
          ...(patch.role ? { role: patch.role } : {}),
          ...(patch.disabled !== undefined && patch.disabled !== null ? { disabled_at: patch.disabled ? T0 : null } : {}),
        }
        : u));
      return [200, users.find((u) => u.id === req.params.id)];
    })
    .on('GET', '/invites', () => [200, invites])
    .on('POST', '/invites', (req) => {
      const body = req.body as NewInvite;
      const created: CreatedInvite = { invite: invite('i2', body.role, body.note ?? null), url: LINK };
      invites = [created.invite, ...invites];
      return [201, created];
    })
    .on('DELETE', '/invites/:id', (req) => {
      invites = invites.filter((i) => i.id !== req.params.id);
      return [204];
    });
  const view = render(<><Settings /><Overlays /></>, { route: '/settings/members', path: '/settings/:section' });
  return { ...view, api, writeText };
}

/** The row whose first cell (the name) starts with `name`. */
const rowOf = async (name: string) =>
  (await screen.findAllByRole('cell', { name: new RegExp(`^${name}`) }))[0]!.closest('tr') as HTMLElement;

describe('Settings → Members', () => {
  it('who may change whom', () => {
    const owner = { id: 'u1', role: 'owner' as Role };
    const admin = { id: 'u2', role: 'admin' as Role };
    expect(assignableRoles(owner, people[0]!)).toEqual([]);
    expect(assignableRoles(owner, people[1]!)).toEqual(['admin', 'member', 'viewer']);
    expect(assignableRoles(admin, people[1]!)).toEqual([]);
    expect(assignableRoles(admin, people[2]!)).toEqual(['member', 'viewer']);
    expect(assignableRoles(admin, user('u9', 'Other', 'admin'))).toEqual([]);
    expect(assignableRoles({ id: 'u3', role: 'member' }, people[3]!)).toEqual([]);
  });

  it('lists the people with a role select where allowed; changing it sends PATCH', async () => {
    const { api } = setup();
    const owner = await rowOf('Owner');
    expect(within(owner).queryByRole('combobox')).toBeNull();
    expect(owner.textContent).toContain('Owner');
    const bea = await rowOf('Bea');
    const select = within(bea).getByRole('combobox', { name: 'Role of Bea' }) as HTMLSelectElement;
    expect([...select.options].map((o) => o.value)).toEqual(['admin', 'member', 'viewer']);
    expect(select.value).toBe('member');
    fireEvent.change(select, { target: { value: 'viewer' } });
    await waitFor(() => expect(api.calls('PATCH', '/users/:id')).toHaveLength(1));
    expect(api.calls('PATCH', '/users/:id')[0]!.params.id).toBe('u3');
    expect(api.calls('PATCH', '/users/:id')[0]!.body).toEqual({ role: 'viewer' });
    await waitFor(() => expect((within(bea).getByRole('combobox') as HTMLSelectElement).value).toBe('viewer'));
    // A disabled account says so and offers Enable.
    const cy = await rowOf('Cy');
    expect(cy.className).toContain('disabled');
    expect(within(cy).getByText('Disabled')).toBeTruthy();
    fireEvent.click(within(cy).getByRole('button', { name: 'Enable Cy' }));
    await waitFor(() => expect(api.calls('PATCH', '/users/:id')).toHaveLength(2));
    expect(api.calls('PATCH', '/users/:id')[1]!.body).toEqual({ disabled: false });
    // Settings shows the section in its navigation.
    expect(screen.getByRole('navigation', { name: 'Settings' }).textContent).toContain('Members');
  });

  it('disabling asks first and names what ends', async () => {
    const { api } = setup();
    fireEvent.click(within(await rowOf('Bea')).getByRole('button', { name: 'Disable Bea' }));
    const confirm = await screen.findByRole('alertdialog');
    expect(confirm.textContent).toContain('Disable Bea?');
    expect(confirm.textContent).toContain('signed out everywhere');
    fireEvent.click(within(confirm).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull());
    expect(api.calls('PATCH')).toHaveLength(0);
    fireEvent.click(within(await rowOf('Bea')).getByRole('button', { name: 'Disable Bea' }));
    fireEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Disable' }));
    await waitFor(() => expect(api.calls('PATCH', '/users/:id')).toHaveLength(1));
    expect(api.calls('PATCH', '/users/:id')[0]!.body).toEqual({ disabled: true });
  });

  it('an admin manages members and viewers only, and cannot invite an admin', async () => {
    setup('admin', 'u2');
    expect(within(await rowOf('Ada')).queryByRole('combobox')).toBeNull();
    expect(within(await rowOf('Ada')).queryByRole('button')).toBeNull();
    const bea = within(await rowOf('Bea')).getByRole('combobox', { name: 'Role of Bea' }) as HTMLSelectElement;
    expect([...bea.options].map((o) => o.value)).toEqual(['member', 'viewer']);
    fireEvent.click(screen.getByRole('button', { name: 'Invite people' }));
    const dialog = await screen.findByRole('dialog', { name: 'Invite people' });
    expect(within(dialog).queryByRole('radio', { name: /^Admin/ })).toBeNull();
    expect(within(dialog).getByRole('radio', { name: /^Member/ })).toBeTruthy();
  });

  it('an invite link is shown once with Copy; closing the dialog drops it everywhere', async () => {
    const { api, writeText, queryClient } = setup();
    await rowOf('Bea');
    fireEvent.click(screen.getByRole('button', { name: 'Invite people' }));
    const dialog = await screen.findByRole('dialog', { name: 'Invite people' });
    expect((within(dialog).getByRole('radio', { name: /^Member/ }) as HTMLInputElement).checked).toBe(true);
    fireEvent.click(within(dialog).getByRole('radio', { name: /^Admin/ }));
    fireEvent.change(within(dialog).getByRole('combobox', { name: 'Link expires after' }), { target: { value: '30' } });
    fireEvent.change(within(dialog).getByRole('textbox', { name: /^Note/ }), { target: { value: ' For Eve ' } });
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Create link' }));
    });
    expect(api.calls('POST', '/invites')[0]!.body).toEqual({ role: 'admin', expires_in_days: 30, note: 'For Eve' });
    const link = (await screen.findByRole('textbox', { name: 'Invite link' })) as HTMLInputElement;
    expect(link.value).toBe(LINK);
    expect(screen.getByText(/You won't see it again/)).toBeTruthy();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    });
    expect(writeText).toHaveBeenCalledWith(LINK);
    fireEvent.click(screen.getByRole('button', { name: 'Done' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(document.body.innerHTML).not.toContain('qinv_');
    const cached = JSON.stringify([
      ...queryClient.getQueryCache().getAll().map((q) => q.state.data),
      ...queryClient.getMutationCache().getAll().map((m) => m.state.data),
    ]);
    expect(cached).not.toContain('qinv_');
    expect(cached).toContain('For Eve');
    for (const store of [localStorage, sessionStorage]) {
      for (let i = 0; i < store.length; i++) expect(store.getItem(store.key(i)!)).not.toContain('qinv_');
    }
    expect(await screen.findByRole('cell', { name: 'For Eve' })).toBeTruthy();
  });

  it('pending invites can be revoked after a confirmation', async () => {
    const { api } = setup();
    const row = (await screen.findByRole('cell', { name: 'For Dee' })).closest('tr') as HTMLElement;
    expect(row.textContent).toContain('Viewer');
    expect(row.textContent).toContain('Owner');
    fireEvent.click(within(row).getByRole('button', { name: /^Revoke/ }));
    fireEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Revoke' }));
    await waitFor(() => expect(api.calls('DELETE', '/invites/:id')).toHaveLength(1));
    expect(api.calls('DELETE', '/invites/:id')[0]!.params.id).toBe('i1');
    expect(await screen.findByText('No pending invites.')).toBeTruthy();
  });

  it('members and viewers see neither the section nor the people', async () => {
    const { api } = setup('member', 'u3');
    expect(await screen.findByText(/Only the owner and admins manage/)).toBeTruthy();
    expect(screen.getByRole('navigation', { name: 'Settings' }).textContent).not.toContain('Members');
    expect(api.calls('GET', '/users')).toHaveLength(0);
    expect(api.calls('GET', '/invites')).toHaveLength(0);
  });
});
