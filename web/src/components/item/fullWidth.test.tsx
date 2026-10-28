// ItemMenu's "Full width" (W3-D9): the toggled value survives the PATCH's refetch of `GET /items/{id}`, whose
// ItemDetail has no `full_width` field yet (request W1-2), and rolls back when the PATCH fails.
import type { QueryClient } from '@tanstack/react-query';
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { useItem } from '../../api/queries/items';
import { errorReply, makeCollection, makeDetail, makeItem, mockApi, type MockReply } from '../../test/mockApi';
import { render } from '../../test/render';
import { Toasts } from '../toasts/Toasts';
import { fullWidthOf, ItemMenu } from './ItemMenu';

const c1 = makeCollection({ id: 'c1' });
const doc = makeItem({ id: 'd1', collection_id: 'c1', title: 'Plan' });
const detail = makeDetail(doc, c1);

function Page() {
  const q = useItem(doc.url_id);
  if (!q.data) return <Toasts />;
  return (
    <>
      <Toasts />
      <p data-testid="width">{fullWidthOf(q.data) ? 'full' : 'normal'}</p>
      <ItemMenu item={q.data} extraExports={[]} />
    </>
  );
}

function setup(patchReply: MockReply = [200, doc]) {
  // The server's detail never carries `full_width` (today's DTO).
  const api = mockApi()
    .on('GET', `/items/${doc.url_id}`, [200, detail])
    .on('PATCH', '/items/:id', patchReply);
  const view = render(<Page />);
  return { ...view, api };
}

/** Waits until no query or mutation is running, and React has rendered the result. */
async function settled(qc: QueryClient) {
  await waitFor(() => {
    expect(qc.isFetching()).toBe(0);
    expect(qc.isMutating()).toBe(0);
  });
  await act(async () => undefined);
}

async function toggle() {
  fireEvent.click(await screen.findByRole('button', { name: 'Item menu' }));
  await act(async () => {
    fireEvent.click(screen.getByRole('menuitemcheckbox', { name: 'Full width' }));
  });
}

function checked(): string | null {
  fireEvent.click(screen.getByRole('button', { name: 'Item menu' }));
  const v = screen.getByRole('menuitemcheckbox', { name: 'Full width' }).getAttribute('aria-checked');
  fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
  return v;
}

describe('Full width', () => {
  it('stays on after the PATCH and the refetch of the detail', async () => {
    const { api, queryClient } = setup();
    await toggle();
    await waitFor(() => expect(api.calls('PATCH', '/items/:id')).toHaveLength(1));
    expect(api.calls('PATCH', '/items/:id')[0]!.body).toEqual({ full_width: true });
    // The PATCH invalidates ['item']: the detail is fetched again, without `full_width`.
    await waitFor(() => expect(api.calls('GET', `/items/${doc.url_id}`)).toHaveLength(2));
    await settled(queryClient);
    expect(queryClient.getQueryData<object>(['item', doc.url_id])).not.toHaveProperty('full_width');
    expect(screen.getByTestId('width').textContent).toBe('full');
    expect(checked()).toBe('true');

    await toggle();
    await waitFor(() => expect(api.calls('PATCH', '/items/:id')).toHaveLength(2));
    expect(api.calls('PATCH', '/items/:id')[1]!.body).toEqual({ full_width: false });
    await waitFor(() => expect(api.calls('GET', `/items/${doc.url_id}`)).toHaveLength(3));
    await settled(queryClient);
    expect(screen.getByTestId('width').textContent).toBe('normal');
    expect(checked()).toBe('false');
  });

  it('rolls back with an error toast when the PATCH fails', async () => {
    setup(errorReply(500, 'internal', 'Boom'));
    await toggle();
    expect(await screen.findByRole('alert')).toBeTruthy();
    await waitFor(() => expect(screen.getByTestId('width').textContent).toBe('normal'));
    expect(checked()).toBe('false');
  });

  it('the server value wins once the DTO carries the field', () => {
    expect(fullWidthOf({ ...detail, full_width: true } as typeof detail)).toBe(true);
    expect(fullWidthOf(detail)).toBe(false);
  });
});
