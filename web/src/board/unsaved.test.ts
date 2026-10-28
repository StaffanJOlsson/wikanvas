import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '../api/client';
import { createFakeFrame } from './fakeFrame';
import { BoardSaveLoop } from './saveLoop';
import {
  boardItem,
  boardPath,
  boardRoutes,
  envelope,
  mockFetch,
  putResult,
  renderBoardApp,
  type Rendered,
} from './testkit';
import { clearUnsaved, isNewerThan, readUnsaved, unsavedKey, writeUnsaved } from './unsaved';

let app: Rendered | null = null;

beforeEach(async () => {
  await clearUnsaved('a');
  await clearUnsaved('board-1');
});

afterEach(() => {
  app?.unmount();
  app = null;
});

async function loopWith(put: (id: string, board: string, base: number) => Promise<ReturnType<typeof putResult>>) {
  const fake = createFakeFrame();
  await fake.api.open({ itemId: 'a', board: '{}', name: 'A', readOnly: false, theme: 'light' });
  const loop = new BoardSaveLoop({ itemId: 'a', rev: 4, frame: () => fake.api, put, now: () => 1234 });
  return { fake, loop };
}

describe('unsaved snapshot (IndexedDB)', () => {
  it('is written before the PUT and cleared on 200', async () => {
    let seen: unknown;
    const { fake, loop } = await loopWith(async (_id, _b, base) => {
      seen = await readUnsaved('a');
      return putResult(base + 1);
    });
    fake.edit('{"x":1}');
    await loop.flush();
    expect(seen).toEqual({ baseRev: 4, board: '{"x":1}', at: 1234 });
    expect(await readUnsaved('a')).toBeUndefined();
    expect(unsavedKey('a')).toBe('quire.board.unsaved/a');
  });

  it('stays when the PUT fails', async () => {
    const { fake, loop } = await loopWith(async () => {
      throw new ApiError(500, 'internal', 'boom');
    });
    fake.edit('{"x":2}');
    await loop.flush();
    expect(await readUnsaved('a')).toEqual({ baseRev: 4, board: '{"x":2}', at: 1234 });
    loop.dispose();
  });

  it('is newer than the board when written after its last content write', () => {
    const at = Date.parse('2026-10-28T10:00:00.000Z');
    const snap = { baseRev: 1, board: '{}', at };
    expect(isNewerThan(snap, '2026-10-28T09:59:59.000Z')).toBe(true);
    expect(isNewerThan(snap, '2026-10-28T10:00:00.000Z')).toBe(false);
    expect(isNewerThan(snap, null)).toBe(true);
  });

  it('offers "Restore unsaved changes from HH:MM?" when newer, and restoring saves it on its base rev', async () => {
    const a = boardItem(1);
    const lastWrite = Date.parse(a.item.content_at ?? '');
    await writeUnsaved('board-1', { baseRev: 1, board: envelope('unsaved work'), at: lastWrite + 60_000 });
    const { fetch, requests } = mockFetch({ ...boardRoutes(a), 'PUT /api/v1/items/board-1/board': [200, putResult(2)] });
    vi.stubGlobal('fetch', fetch);
    const r = (app = renderBoardApp(boardPath(a)));
    const prompt = await screen.findByText(/Restore unsaved changes from \d{1,2}:\d{2}/);
    expect(prompt).toBeTruthy();
    expect(r.fake.board()).toBe(envelope('Hello'));
    fireEvent.click(screen.getByRole('button', { name: 'Restore' }));
    await waitFor(() => expect(requests.some((q) => q.method === 'PUT')).toBe(true));
    const put = requests.find((q) => q.method === 'PUT');
    expect(put?.headers.get('If-Match')).toBe('"r1"');
    expect(put?.body).toBe(envelope('unsaved work'));
    expect(r.fake.board()).toBe(envelope('unsaved work'));
    await waitFor(async () => expect(await readUnsaved('board-1')).toBeUndefined());
    expect(screen.queryByText(/Restore unsaved changes/)).toBeNull();
  });

  it('offers nothing for a snapshot older than the board, and Discard clears a newer one', async () => {
    const a = boardItem(1);
    const lastWrite = Date.parse(a.item.content_at ?? '');
    await writeUnsaved('board-1', { baseRev: 1, board: envelope('old'), at: lastWrite - 1000 });
    vi.stubGlobal('fetch', mockFetch(boardRoutes(a)).fetch);
    let r = (app = renderBoardApp(boardPath(a)));
    await waitFor(() => expect(r.fake.api.current()).toBe('board-1'));
    await act(async () => {});
    expect(screen.queryByText(/Restore unsaved changes/)).toBeNull();
    r.unmount();

    await writeUnsaved('board-1', { baseRev: 1, board: envelope('newer'), at: lastWrite + 1000 });
    r = app = renderBoardApp(boardPath(a));
    fireEvent.click(await screen.findByRole('button', { name: 'Discard' }));
    await waitFor(async () => expect(await readUnsaved('board-1')).toBeUndefined());
    expect(screen.queryByText(/Restore unsaved changes/)).toBeNull();
    expect(r.fake.board()).toBe(envelope('Hello'));
  });
});
