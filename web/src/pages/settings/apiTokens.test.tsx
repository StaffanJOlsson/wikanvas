import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { ApiToken } from '../../api/gen/ApiToken';
import type { CreatedToken } from '../../api/gen/CreatedToken';
import type { NewToken } from '../../api/gen/NewToken';
import { Overlays } from '../../components/overlays/Overlays';
import { errorReply, mockApi } from '../../test/mockApi';
import { render } from '../../test/render';
import { ApiTokens } from './ApiTokens';

const DAY = 86_400_000;
const SECRET = 'qpat_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789ABCDEF12345';

function token(over: Partial<ApiToken> & { id: string; name: string }): ApiToken {
  return {
    prefix: 'qpat_ab12cd3',
    scopes: ['read'],
    created_at: new Date(Date.now() - 3 * DAY).toISOString(),
    created_via: 'web',
    parent_token_id: null,
    expires_at: new Date(Date.now() + 30 * DAY).toISOString(),
    last_used_at: null,
    last_used_ip: null,
    ...over,
  };
}

const list: ApiToken[] = [
  token({ id: 't1', name: 'Laptop CLI', scopes: ['read', 'write'], last_used_at: new Date(Date.now() - 2 * 3_600_000).toISOString() }),
  token({ id: 't2', name: 'Old script', prefix: 'qpat_zz99yy8', expires_at: new Date(Date.now() - DAY).toISOString() }),
  token({ id: 't3', name: 'Backup agent', prefix: 'qpat_kk11jj2', scopes: ['read', 'write', 'admin'], expires_at: null }),
];

function setup() {
  const writeText = vi.fn(async () => undefined);
  Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  let tokens = [...list];
  const api = mockApi()
    .on('GET', '/tokens', () => [200, tokens])
    .on('POST', '/tokens', (req) => {
      const body = req.body as NewToken;
      const created: CreatedToken = {
        token: SECRET,
        api_token: token({ id: 'new', name: body.name, prefix: SECRET.slice(0, 12), scopes: body.scopes }),
      };
      tokens = [created.api_token, ...tokens];
      return [201, created];
    })
    .on('DELETE', '/tokens/:id', [204]);
  const view = render(<><ApiTokens /><Overlays /></>, { route: '/settings/tokens' });
  return { ...view, api, writeText };
}

const rowOf = async (name: string) => (await screen.findByRole('cell', { name })).closest('tr') as HTMLElement;

async function openDialog() {
  fireEvent.click(screen.getByRole('button', { name: 'New token' }));
  return screen.findByRole('dialog', { name: 'New API token' });
}

describe('Settings → API tokens', () => {
  it('lists prefix, scopes, last use and expiry; expired tokens are greyed', async () => {
    setup();
    const laptop = await rowOf('Laptop CLI');
    expect(laptop.textContent).toContain('qpat_ab12cd3…');
    expect([...laptop.querySelectorAll('.scope-chip')].map((c) => c.textContent)).toEqual(['read', 'write']);
    expect(laptop.textContent).toContain('2 hours ago');
    expect(laptop.textContent).toMatch(/Expires \w{3} \d+, \d{4}/);
    expect(laptop.className).not.toContain('expired');
    const old = await rowOf('Old script');
    expect(old.className).toContain('expired');
    expect(old.textContent).toContain('Expired');
    expect(old.textContent).toContain('Never used');
    expect((await rowOf('Backup agent')).textContent).toContain('Never expires');
    expect(screen.getAllByText(/wikanvas auth login --url .* --token-stdin/).length).toBeGreaterThan(0);
  });

  it('create shows the secret once with Copy; closing the dialog drops it everywhere', async () => {
    const { api, writeText, queryClient } = setup();
    await rowOf('Laptop CLI');
    const dialog = await openDialog();
    fireEvent.change(within(dialog).getByRole('textbox', { name: 'Name' }), { target: { value: 'CI runner' } });
    fireEvent.click(within(dialog).getByRole('radio', { name: /^Write/ }));
    expect(within(dialog).queryByLabelText(/^Your password/)).toBeNull();
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Create token' }));
    });
    expect(api.calls('POST', '/tokens')[0]!.body).toEqual({ name: 'CI runner', scopes: ['write'], expires_in_days: 90 });

    const secret = (await screen.findByRole('textbox', { name: 'Token' })) as HTMLInputElement;
    expect(secret.value).toBe(SECRET);
    expect(screen.getByText(/You won't see this token again/)).toBeTruthy();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    });
    expect(writeText).toHaveBeenCalledWith(SECRET);
    expect(screen.getByRole('button', { name: 'Copied' })).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Done' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(document.body.innerHTML).not.toContain(SECRET);
    const cached = JSON.stringify([
      ...queryClient.getQueryCache().getAll().map((q) => q.state.data),
      ...queryClient.getMutationCache().getAll().map((m) => m.state.data),
    ]);
    expect(cached).not.toContain(SECRET);
    expect(cached).toContain('CI runner');
    for (const store of [localStorage, sessionStorage]) {
      for (let i = 0; i < store.length; i++) expect(store.getItem(store.key(i)!)).not.toContain(SECRET);
    }
    expect(await rowOf('CI runner')).toBeTruthy();
  });

  it('"Never" shows the warning, and Admin or Never asks for the password', async () => {
    const { api } = setup();
    await rowOf('Laptop CLI');
    const dialog = await openDialog();
    const expiry = within(dialog).getByRole('combobox', { name: 'Expires after' }) as HTMLSelectElement;
    expect(expiry.value).toBe('90');
    expect(within(dialog).queryByRole('note')).toBeNull();
    fireEvent.change(expiry, { target: { value: 'never' } });
    expect(within(dialog).getByRole('note').textContent).toContain('never expires');
    expect(within(dialog).getByLabelText(/^Your password/)).toBeTruthy();
    fireEvent.change(expiry, { target: { value: '30' } });
    expect(within(dialog).queryByRole('note')).toBeNull();
    expect(within(dialog).queryByLabelText(/^Your password/)).toBeNull();
    fireEvent.click(within(dialog).getByRole('radio', { name: /^Admin/ }));
    fireEvent.change(within(dialog).getByLabelText(/^Your password/), { target: { value: 'correct horse battery' } });
    fireEvent.change(within(dialog).getByRole('textbox', { name: 'Name' }), { target: { value: 'Ops' } });
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Create token' }));
    });
    expect(api.calls('POST', '/tokens')[0]!.body).toEqual({
      name: 'Ops', scopes: ['admin'], expires_in_days: 30, password: 'correct horse battery',
    });
  });

  it('a 422 on the password field shows inline', async () => {
    const { api } = setup();
    api.on('POST', '/tokens', errorReply(422, 'validation', 'The password is wrong', { field: 'password' }));
    await rowOf('Laptop CLI');
    const dialog = await openDialog();
    fireEvent.change(within(dialog).getByRole('textbox', { name: 'Name' }), { target: { value: 'Forever' } });
    fireEvent.change(within(dialog).getByRole('combobox', { name: 'Expires after' }), { target: { value: 'never' } });
    fireEvent.change(within(dialog).getByLabelText(/^Your password/), { target: { value: 'wrong' } });
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Create token' }));
    });
    const field = within(dialog).getByLabelText(/^Your password/).closest('label')!;
    expect(within(field).getByRole('alert').textContent).toBe('The password is wrong');
    expect(screen.queryByRole('textbox', { name: 'Token' })).toBeNull();
  });

  it('revoke asks for confirmation naming the token', async () => {
    const { api } = setup();
    fireEvent.click(within(await rowOf('Old script')).getByRole('button', { name: 'Revoke Old script' }));
    const confirm = await screen.findByRole('alertdialog');
    expect(confirm.textContent).toContain('Revoke “Old script”?');
    fireEvent.click(within(confirm).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull());
    expect(api.calls('DELETE')).toHaveLength(0);
    fireEvent.click(within(await rowOf('Old script')).getByRole('button', { name: 'Revoke Old script' }));
    fireEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Revoke' }));
    await waitFor(() => expect(api.calls('DELETE', '/tokens/:id')).toHaveLength(1));
    expect(api.calls('DELETE', '/tokens/:id')[0]!.params.id).toBe('t2');
  });
});
