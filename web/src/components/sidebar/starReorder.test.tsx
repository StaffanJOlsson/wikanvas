// Star reorder (§9.6.3: STARRED rows, before/after 50/50): the drop's `useStar` call reorders the STARRED section at
// once, sends `PUT /items/{id}/star {position}` with both neighbours, and rolls back when the server refuses.
import { act, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { ItemSummary } from '../../api/gen/ItemSummary';
import type { Position } from '../../api/gen/Position';
import { errorReply, makeCollection, makeItem, makeSidebar, mockApi, type MockReply } from '../../test/mockApi';
import { renderSidebar } from '../../test/sidebarHarness';
import { starReorderVars } from './StarredSection';

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

const c1 = makeCollection({ id: 'c1', item_count: 3 });
const [s1, s2, s3] = ['One', 'Two', 'Three'].map((title, i) => makeItem({ id: `s${i + 1}`, collection_id: 'c1', title })) as [
  ItemSummary, ItemSummary, ItemSummary,
];

function starNames(): string[] {
  const section = screen.getByRole('region', { name: 'Starred' });
  return within(section).getAllByRole('link').map((a) => a.textContent ?? '');
}

function setup() {
  let stars = [s1, s2, s3];
  const api = mockApi();
  const view = renderSidebar({ api, collections: [c1], stars });
  api.on('GET', '/sidebar', () => [200, makeSidebar([c1], stars)]);
  const accept = (id: string, position: Position) => {
    const moving = stars.find((s) => s.id === id)!;
    const rest = stars.filter((s) => s.id !== id);
    const at = position.after_id ? rest.findIndex((s) => s.id === position.after_id) + 1 : rest.findIndex((s) => s.id === position.before_id);
    rest.splice(at, 0, moving);
    stars = rest;
  };
  return { ...view, api, accept };
}

describe('star reorder', () => {
  it('drop maps to the neighbours; the list reorders optimistically; the PUT body is {position}', () => {
    expect(starReorderVars([s1, s2, s3], 's3', 's1', 'before')).toEqual({
      itemId: 's3', starred: true, position: { after_id: null, before_id: 's1' },
    });
    expect(starReorderVars([s1, s2, s3], 's1', 's2', 'after')).toEqual({
      itemId: 's1', starred: true, position: { after_id: 's2', before_id: 's3' },
    });
    expect(starReorderVars([s1, s2, s3], 's1', 's2', 'before')).toBeNull(); // already there
  });

  it('reorders at once and keeps the order the server confirms', async () => {
    const { api, probe, accept } = setup();
    await waitFor(() => expect(starNames()).toEqual(['One', 'Two', 'Three']));
    const answer = deferred<MockReply>();
    api.on('PUT', '/items/:id/star', (req) => answer.promise.then((r) => {
      accept(req.params.id!, (req.body as { position: Position }).position);
      return r;
    }));

    let done!: Promise<void>;
    act(() => {
      done = probe.star(starReorderVars([s1, s2, s3], 's3', 's1', 'before')!);
    });
    await waitFor(() => expect(starNames()).toEqual(['Three', 'One', 'Two']));
    expect(api.calls('PUT', '/items/:id/star')[0]).toMatchObject({
      params: { id: 's3' },
      body: { position: { after_id: null, before_id: 's1' } },
    });
    answer.resolve([204]);
    await act(async () => done);
    await waitFor(() => expect(api.calls('GET', '/sidebar').length).toBeGreaterThan(1));
    expect(starNames()).toEqual(['Three', 'One', 'Two']);
  });

  it('rolls back when the server refuses', async () => {
    const { api, probe } = setup();
    await waitFor(() => expect(starNames()).toEqual(['One', 'Two', 'Three']));
    const answer = deferred<MockReply>();
    api.on('PUT', '/items/:id/star', () => answer.promise);
    let done!: Promise<void>;
    act(() => {
      done = probe.star(starReorderVars([s1, s2, s3], 's1', 's3', 'after')!);
    });
    await waitFor(() => expect(starNames()).toEqual(['Two', 'Three', 'One']));
    answer.resolve(errorReply(404, 'not_found'));
    await act(async () => {
      await expect(done).rejects.toMatchObject({ code: 'not_found' });
    });
    await waitFor(() => expect(starNames()).toEqual(['One', 'Two', 'Three']));
  });
});
