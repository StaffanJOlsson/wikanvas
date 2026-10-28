// ItemMenu's "Full width" (W3-D9): `ItemDetail.full_width` is the server's value; the toggle sets the cached detail at
// once, the PATCH's refetch confirms it, and a failed PATCH rolls it back.
import type { QueryClient } from '@tanstack/react-query';
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { useItem } from '../../api/queries/items';
import type { ItemPatch } from '../../api/gen/ItemPatch';
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

function setup(failPatch?: MockReply) {
  // The server keeps the value: a PATCH changes what the next GET returns.
  let stored = false;
  const api = mockApi()
    .on('GET', `/items/${doc.url_id}`, () => [200, { ...detail, full_width: stored }])
    .on('PATCH', '/items/:id', (req) => {
      if (failPatch) return failPatch;
      stored = (req.body as ItemPatch).full_width ?? stored;
      return [200, doc];
    });
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
  it('is the server value: on after the PATCH and the refetch of the detail, then off again', async () => {
    const { api, queryClient } = setup();
    await waitFor(() => expect(screen.getByTestId('width').textContent).toBe('normal'));
    await toggle();
    // Optimistic: the cached detail changes first.
    await waitFor(() => expect(screen.getByTestId('width').textContent).toBe('full'));
    await waitFor(() => expect(api.calls('PATCH', '/items/:id')).toHaveLength(1));
    expect(api.calls('PATCH', '/items/:id')[0]!.body).toEqual({ full_width: true });
    // The PATCH invalidates ['item']: the detail is fetched again and carries the stored value.
    await waitFor(() => expect(api.calls('GET', `/items/${doc.url_id}`)).toHaveLength(2));
    await settled(queryClient);
    expect(queryClient.getQueryData<{ full_width: boolean }>(['item', doc.url_id])?.full_width).toBe(true);
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
    const { queryClient } = setup(errorReply(500, 'internal', 'Boom'));
    await toggle();
    expect(await screen.findByRole('alert')).toBeTruthy();
    await settled(queryClient);
    expect(screen.getByTestId('width').textContent).toBe('normal');
    expect(checked()).toBe('false');
  });

  it('reads ItemDetail.full_width', () => {
    expect(fullWidthOf({ ...detail, full_width: true })).toBe(true);
    expect(fullWidthOf(detail)).toBe(false);
  });
});
