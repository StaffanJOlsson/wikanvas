// The collection dialog's Access section (D152): only for whoever manages the collection; the default access is saved
// with the dialog, a member's access at once (PUT), Remove deletes the membership, "Add a person" picks from the
// directory (people not yet members, without the owner and admins); every change invalidates what holds `my_access`.
import { act, cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { Collection } from '../../api/gen/Collection';
import type { CollectionMember } from '../../api/gen/CollectionMember';
import type { SetMember } from '../../api/gen/SetMember';
import { sidebarKey } from '../../api/queries/tree';
import { openDialog, resetDialogs } from '../../state/dialogs';
import { makeCollection, makeDirectoryUser, makeMe, makeMember, makeSidebar, mockApi } from '../../test/mockApi';
import { render } from '../../test/render';
import { Overlays } from './Overlays';

const owner = makeDirectoryUser({ id: 'u1', name: 'Owner', role: 'owner' });
const ann = makeDirectoryUser({ id: 'ann', name: 'Ann', role: 'member' });
const bob = makeDirectoryUser({ id: 'bob', name: 'Bob', role: 'member' });
const vic = makeDirectoryUser({ id: 'vic', name: 'Vic', role: 'viewer' });
const ada = makeDirectoryUser({ id: 'ada', name: 'Ada', role: 'admin' });

function setup(collection: Collection, initial: CollectionMember[]) {
  let members = [...initial];
  const api = mockApi()
    .on('GET', '/auth/me', [200, makeMe()])
    .on('GET', '/sidebar', [200, makeSidebar([collection])])
    .on('GET', '/directory', [200, [ada, ann, bob, owner, vic]])
    .on('GET', '/collections/:id/members', () => [200, members])
    .on('PATCH', '/collections/:id', (req) => [200, { ...collection, ...(req.body as object) }])
    .on('PUT', '/collections/:id/members/:user', (req) => {
      const user = [ann, bob, vic].find((u) => u.id === req.params.user)!;
      const { access } = req.body as SetMember;
      const m = makeMember(user, { access, effective_access: user.role === 'viewer' ? 'read' : access });
      members = [...members.filter((x) => x.user.id !== user.id), m];
      return [200, m];
    })
    .on('DELETE', '/collections/:id/members/:user', (req) => {
      members = members.filter((x) => x.user.id !== req.params.user);
      return [204];
    });
  const view = render(<Overlays />);
  view.queryClient.setQueryData(sidebarKey, makeSidebar([collection]));
  act(() => openDialog({ kind: 'collection', collection }));
  return { ...view, api };
}

const group = () => screen.getByRole('group', { name: 'Access' });

describe('collection access', () => {
  it('is shown only to whoever manages the collection', async () => {
    for (const my_access of ['write', 'read'] as const) {
      const { api } = setup(makeCollection({ id: 'c1', name: 'Docs', my_access }), []);
      expect(screen.getByRole('dialog', { name: 'Edit collection' })).toBeTruthy();
      expect(screen.queryByRole('group', { name: 'Access' })).toBeNull();
      expect(screen.queryByRole('combobox', { name: 'Default access' })).toBeNull();
      expect(api.calls('GET', '/collections/:id/members')).toHaveLength(0);
      cleanup();
      resetDialogs();
    }
    const { api } = setup(makeCollection({ id: 'c1', name: 'Docs' }), []);
    expect(within(group()).getByRole('combobox', { name: 'Default access' })).toBeTruthy();
    await screen.findByText('Nobody is added yet.');
    expect(api.calls('GET', '/collections/:id/members')[0]!.params.id).toBe('c1');
  });

  it('creating offers the default access, without people, and posts it', async () => {
    const created = makeCollection({ id: 'c9', name: 'Private' });
    const api = mockApi()
      .on('GET', '/sidebar', [200, makeSidebar([])])
      .on('POST', '/collections', [201, created]);
    render(<Overlays />);
    act(() => openDialog({ kind: 'collection', collection: null }));
    expect(within(group()).queryByRole('group', { name: 'Add a person' })).toBeNull();
    expect(group().textContent).toContain('You manage the new collection');
    fireEvent.change(screen.getByRole('textbox', { name: 'Name' }), { target: { value: 'Private' } });
    fireEvent.change(within(group()).getByRole('combobox', { name: 'Default access' }), { target: { value: 'none' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Create' }));
    });
    await waitFor(() => expect(api.calls('POST', '/collections')).toHaveLength(1));
    expect(api.calls('POST', '/collections')[0]!.body).toMatchObject({ name: 'Private', default_access: 'none' });
  });

  it('saves the default access with the dialog, and only when it changed', async () => {
    const c1 = makeCollection({ id: 'c1', name: 'Docs', default_access: 'write' });
    const { api, queryClient } = setup(c1, []);
    const select = within(group()).getByRole('combobox', { name: 'Default access' }) as HTMLSelectElement;
    expect(select.value).toBe('write');
    expect([...select.options].map((o) => [o.value, o.text])).toEqual([
      ['none', 'Only members'],
      ['read', 'Everyone can read'],
      ['write', 'Members edit, viewers read'],
    ]);
    expect(group().textContent).toContain('Members can edit it and viewers can read it');
    expect(group().textContent).toContain('The owner and admins manage every collection.');
    fireEvent.change(select, { target: { value: 'none' } });
    expect(group().textContent).toContain('Only the people added here can open it');
    // Nothing is sent before Save.
    expect(api.calls('PATCH')).toHaveLength(0);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    });
    await waitFor(() => expect(api.calls('PATCH', '/collections/:id')).toHaveLength(1));
    expect(api.calls('PATCH', '/collections/:id')[0]).toMatchObject({ params: { id: 'c1' }, body: { default_access: 'none' } });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Edit collection' })).toBeNull());
    expect(queryClient.getQueryState(sidebarKey)?.isInvalidated).toBe(true);

    // Unchanged: the PATCH leaves it out.
    act(() => openDialog({ kind: 'collection', collection: c1 }));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    });
    await waitFor(() => expect(api.calls('PATCH', '/collections/:id')).toHaveLength(2));
    expect(api.calls('PATCH', '/collections/:id')[1]!.body).not.toHaveProperty('default_access');
  });

  it('lists the members with what applies to them, and changes, removes and adds people at once', async () => {
    const c1 = makeCollection({ id: 'c1', name: 'Docs' });
    const { api, queryClient } = setup(c1, [
      makeMember(ann, { access: 'write' }),
      makeMember(vic, { access: 'write', effective_access: 'read' }),
      makeMember(bob, { access: 'read', effective_access: 'none', disabled: true }),
    ]);
    const people = await screen.findByRole('table', { name: 'People' });
    const rowOf = (name: string) => within(people).getByRole('combobox', { name: `Access of ${name}` }).closest('tr')!;
    expect(rowOf('Ann').textContent).toContain('Member');
    expect(rowOf('Ann').textContent).not.toContain('(');
    expect(rowOf('Vic').textContent).toContain('Viewer · reads only (viewer)');
    expect(rowOf('Bob').textContent).toContain('(disabled)');
    expect((within(people).getByRole('combobox', { name: 'Access of Vic' }) as HTMLSelectElement).value).toBe('write');

    // Change: PUT at once, before any Save.
    fireEvent.change(within(people).getByRole('combobox', { name: 'Access of Ann' }), { target: { value: 'manage' } });
    await waitFor(() => expect(api.calls('PUT', '/collections/:id/members/:user')).toHaveLength(1));
    expect(api.calls('PUT', '/collections/:id/members/:user')[0]).toMatchObject({
      params: { id: 'c1', user: 'ann' },
      body: { access: 'manage' },
    });
    await waitFor(() =>
      expect((within(people).getByRole('combobox', { name: 'Access of Ann' }) as HTMLSelectElement).value).toBe('manage'));
    await waitFor(() => expect(queryClient.getQueryState(sidebarKey)?.isInvalidated).toBe(true));

    // Remove.
    fireEvent.click(within(people).getByRole('button', { name: 'Remove Vic' }));
    await waitFor(() => expect(api.calls('DELETE', '/collections/:id/members/:user')).toHaveLength(1));
    expect(api.calls('DELETE', '/collections/:id/members/:user')[0]!.params).toEqual({ id: 'c1', user: 'vic' });
    await waitFor(() => expect(within(people).queryByRole('combobox', { name: 'Access of Vic' })).toBeNull());

    // Add: the picker offers the people who are not members, never the owner or an admin.
    const add = screen.getByRole('group', { name: 'Add a person' });
    const person = within(add).getByRole('combobox', { name: 'Person to add' }) as HTMLSelectElement;
    await waitFor(() => expect([...person.options].map((o) => o.text)).toEqual(['Choose a person…', 'Vic (Viewer)']));
    const addButton = within(add).getByRole('button', { name: 'Add' }) as HTMLButtonElement;
    expect(addButton.disabled).toBe(true);
    fireEvent.change(person, { target: { value: 'vic' } });
    fireEvent.change(within(add).getByRole('combobox', { name: 'Access to give' }), { target: { value: 'read' } });
    fireEvent.click(addButton);
    await waitFor(() => expect(api.calls('PUT', '/collections/:id/members/:user')).toHaveLength(2));
    expect(api.calls('PUT', '/collections/:id/members/:user')[1]).toMatchObject({
      params: { id: 'c1', user: 'vic' },
      body: { access: 'read' },
    });
    await within(people).findByRole('combobox', { name: 'Access of Vic' });
    expect((await screen.findByText('Added Vic to Docs'))).toBeTruthy();
    // The dialog's own form was never saved.
    expect(api.calls('PATCH')).toHaveLength(0);
  });
});
