import { act, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { Collection } from '../../api/gen/Collection';
import type { MoveCollection } from '../../api/gen/MoveCollection';
import { errorReply, makeCollection, makeSidebar, mockApi, type MockReply } from '../../test/mockApi';
import { renderSidebar, waitForRows } from '../../test/sidebarHarness';

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

describe('collection reorder', () => {
  const a = makeCollection({ id: 'ca', name: 'Alpha' });
  const b = makeCollection({ id: 'cb', name: 'Beta' });
  const c = makeCollection({ id: 'cc', name: 'Gamma' });

  function setup() {
    let order: Collection[] = [a, b, c];
    const api = mockApi();
    const view = renderSidebar({ api, collections: order });
    api.on('GET', '/sidebar', () => [200, makeSidebar(order)]);
    const accept = (id: string, body: MoveCollection): Collection => {
      const moving = order.find((x) => x.id === id)!;
      const rest = order.filter((x) => x.id !== id);
      const at = body.position.after_id
        ? rest.findIndex((x) => x.id === body.position.after_id) + 1
        : rest.findIndex((x) => x.id === body.position.before_id);
      rest.splice(at, 0, moving);
      order = rest;
      return moving;
    };
    return { ...view, api, accept };
  }

  it('moves before/after another collection, optimistically, with the neighbours as position', async () => {
    const { api, probe, accept } = setup();
    await waitForRows(['Alpha', 'Beta', 'Gamma']);
    const answer = deferred<MockReply>();
    api.on('POST', '/collections/:id/move', (req) => answer.promise.then(() => [200, accept(req.params.id!, req.body as MoveCollection)]));

    let done!: Promise<boolean>;
    act(() => {
      done = probe.collections.reorder(c, 'ca', 'before');
    });
    await waitForRows(['Gamma', 'Alpha', 'Beta']);
    expect(api.calls('POST', '/collections/:id/move')[0]).toMatchObject({
      params: { id: 'cc' },
      body: { position: { after_id: null, before_id: 'ca' } },
    });
    answer.resolve([200]);
    await act(async () => {
      expect(await done).toBe(true);
    });
    await waitForRows(['Gamma', 'Alpha', 'Beta']);

    api.on('POST', '/collections/:id/move', (req) => [200, accept(req.params.id!, req.body as MoveCollection)]);
    await act(async () => {
      expect(await probe.collections.reorder(c, 'cb', 'after')).toBe(true);
    });
    expect(api.calls('POST', '/collections/:id/move')[1]!.body).toEqual({ position: { after_id: 'cb', before_id: null } });
    await waitForRows(['Alpha', 'Beta', 'Gamma']);
  });

  it('rolls back with an error toast when the server refuses', async () => {
    const { api, probe } = setup();
    await waitForRows(['Alpha', 'Beta', 'Gamma']);
    api.on('POST', '/collections/:id/move', () => errorReply(500, 'internal', 'Database is locked'));
    await act(async () => {
      expect(await probe.collections.reorder(a, 'cc', 'after')).toBe(false);
    });
    await waitForRows(['Alpha', 'Beta', 'Gamma']);
    expect((await screen.findByRole('alert')).textContent).toContain('Database is locked');
  });

  it('a reorder onto its own place sends nothing', async () => {
    const { api, probe } = setup();
    await waitForRows(['Alpha', 'Beta', 'Gamma']);
    await act(async () => {
      expect(await probe.collections.reorder(b, 'ca', 'after')).toBe(false);
      expect(await probe.collections.reorder(b, 'cc', 'before')).toBe(false);
    });
    expect(api.calls('POST')).toHaveLength(0);
  });
});
