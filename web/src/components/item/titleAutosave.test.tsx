import { act, fireEvent, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import { flushPending, hasPending } from '../../state/pending';
import { clearSaveStatus, getSaveStatus } from '../../state/saveStatus';
import { errorReply, makeCollection, makeDetail, makeItem, mockApi } from '../../test/mockApi';
import { render } from '../../test/render';
import { ItemHeader } from './ItemHeader';
import { TITLE_DEBOUNCE_MS } from './useTitleAutosave';

const c1 = makeCollection({ id: 'c1' });
const detail: ItemDetail = makeDetail(makeItem({ id: 'i1', collection_id: 'c1', title: 'Plan' }), c1);

function setup(reply = (title: string) => [200, { ...detail.item, title }] as const) {
  const api = mockApi().on('PATCH', '/items/:id', (req) => [...reply((req.body as { title: string }).title)]);
  const view = render(<ItemHeader item={detail} onEnterBody={() => undefined} readOnly={false} />, {
    route: '/doc/plan-i100000000',
    path: '/doc/:slugId',
    routes: [{ path: '/home', element: <h1>Home</h1> }],
  });
  return { ...view, api, input: screen.getByRole('textbox', { name: 'Title' }) as HTMLInputElement };
}

afterEach(() => {
  vi.useRealTimers();
  clearSaveStatus('i1');
});

describe('title autosave', () => {
  it('waits 500 ms after the last keystroke, then PATCHes the title once', async () => {
    vi.useFakeTimers();
    const { api, input } = setup();
    fireEvent.change(input, { target: { value: 'Pla' } });
    await act(async () => vi.advanceTimersByTimeAsync(300));
    fireEvent.change(input, { target: { value: 'Plan B' } });
    await act(async () => vi.advanceTimersByTimeAsync(TITLE_DEBOUNCE_MS - 1));
    expect(api.calls('PATCH')).toHaveLength(0);
    await act(async () => vi.advanceTimersByTimeAsync(1));
    expect(api.calls('PATCH', '/items/:id')).toHaveLength(1);
    expect(api.calls('PATCH', '/items/:id')[0]!).toMatchObject({ params: { id: 'i1' }, body: { title: 'Plan B' } });
    expect(api.calls('PATCH', '/items/:id')[0]!.headers.has('If-Match')).toBe(false);
  });

  it('is registered in pending while unsaved; flushPending sends it at once', async () => {
    vi.useFakeTimers();
    const { api, input } = setup();
    expect(hasPending('i1')).toBe(false);
    fireEvent.change(input, { target: { value: 'Plan C' } });
    expect(hasPending('i1')).toBe(true);
    await act(async () => flushPending());
    expect(api.calls('PATCH', '/items/:id')).toHaveLength(1);
    expect(hasPending('i1')).toBe(false);
    await act(async () => vi.advanceTimersByTimeAsync(TITLE_DEBOUNCE_MS * 2));
    expect(api.calls('PATCH', '/items/:id')).toHaveLength(1);
  });

  it('is flushed on route change, before the debounce ends', async () => {
    vi.useFakeTimers();
    const { api, input, router } = setup();
    fireEvent.change(input, { target: { value: 'Plan D' } });
    await act(async () => {
      await router.navigate('/home');
    });
    expect(screen.getByRole('heading', { name: 'Home' })).toBeTruthy();
    expect(api.calls('PATCH', '/items/:id')).toHaveLength(1);
    expect(api.calls('PATCH', '/items/:id')[0]!.body).toEqual({ title: 'Plan D' });
  });

  it('a failed save shows an error with Retry, and stays pending', async () => {
    let fail = true;
    const { api, input } = setup((title) => (fail ? errorReply(500, 'internal', 'Disk full') : [200, { ...detail.item, title }]) as never);
    fireEvent.change(input, { target: { value: 'Plan E' } });
    await act(async () => flushPending('i1'));
    expect(getSaveStatus('i1')).toMatchObject({ state: 'error', message: 'Disk full' });
    expect(hasPending('i1')).toBe(true);
    fail = false;
    await act(async () => {
      getSaveStatus('i1').retry?.();
      await flushPending('i1');
    });
    expect(api.calls('PATCH', '/items/:id')).toHaveLength(2);
    expect(getSaveStatus('i1').state).toBe('saved');
    expect(hasPending('i1')).toBe(false);
  });
});
