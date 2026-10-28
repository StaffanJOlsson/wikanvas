import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '../api/client';
import { getSaveStatus } from '../state/saveStatus';
import { hasPending } from '../state/pending';
import { createFakeFrame, type FakeFrame } from './fakeFrame';
import { frameEvents, openBoard, resetHost, useHostStore, type BoardSession } from './host';
import { BACKOFF_MS, BoardSaveLoop, MAX_MS, QUIET_MS } from './saveLoop';
import { putResult } from './testkit';

const theme = 'light' as const;

async function openIn(fake: FakeFrame, itemId: string, board = '{"b":0}') {
  await fake.api.open({ itemId, board, name: itemId, readOnly: false, theme });
}

function setup(itemId = 'a', rev = 1) {
  const fake = createFakeFrame();
  const put = vi.fn(async (_id: string, _board: string, base: number, _engine?: string) => putResult(base + 1));
  const writeUnsaved = vi.fn(async () => {});
  const clearUnsaved = vi.fn(async () => {});
  const loop = new BoardSaveLoop({ itemId, rev, frame: () => fake.api, put, writeUnsaved, clearUnsaved });
  return { fake, put, writeUnsaved, clearUnsaved, loop };
}

/** Edits the board and tells the loop, as the frame's onChange does. */
function edit(fake: FakeFrame, loop: BoardSaveLoop, board?: string): number {
  const r = fake.edit(board);
  loop.change(r);
  return r;
}

beforeEach(() => {
  vi.useFakeTimers();
  resetHost();
});

afterEach(() => {
  vi.useRealTimers();
  resetHost();
});

describe('BoardSaveLoop', () => {
  it('saves 1.5 s after the last change', async () => {
    const { fake, put, loop } = setup();
    await openIn(fake, 'a');
    edit(fake, loop);
    await vi.advanceTimersByTimeAsync(1000);
    edit(fake, loop);
    await vi.advanceTimersByTimeAsync(QUIET_MS - 1);
    expect(put).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(put).toHaveBeenCalledTimes(1);
    const [id, , base, engine] = put.mock.calls[0];
    expect([id, base, engine]).toEqual(['a', 1, fake.api.engineVersion]);
    expect(loop.rev).toBe(2);
    expect(getSaveStatus('a').state).toBe('saved');
  });

  it('saves at most 10 s after the first unsaved change, however often it changes', async () => {
    const { fake, put, loop } = setup();
    await openIn(fake, 'a');
    for (let t = 0; t < MAX_MS - 1000; t += 1000) {
      edit(fake, loop);
      await vi.advanceTimersByTimeAsync(1000);
    }
    expect(put).not.toHaveBeenCalled();
    edit(fake, loop);
    await vi.advanceTimersByTimeAsync(1000);
    expect(put).toHaveBeenCalledTimes(1);
    // The next change starts a new window.
    edit(fake, loop);
    await vi.advanceTimersByTimeAsync(QUIET_MS);
    expect(put).toHaveBeenCalledTimes(2);
  });

  it('calls markSaved with the snapshot revision, so changes made during the PUT stay dirty and are saved next', async () => {
    const { fake, put, loop } = setup();
    await openIn(fake, 'a');
    let release!: () => void;
    put.mockImplementationOnce(
      (_id, _b, base) => new Promise((resolve) => (release = () => resolve(putResult(base + 1)))),
    );
    const snapped = edit(fake, loop, '{"b":1}');
    await vi.advanceTimersByTimeAsync(QUIET_MS);
    expect(put).toHaveBeenCalledTimes(1);
    expect(put.mock.calls[0][1]).toBe('{"b":1}');
    // A change while the PUT runs.
    edit(fake, loop, '{"b":2}');
    release();
    await vi.advanceTimersByTimeAsync(0);
    const marks = fake.calls.filter((c) => c.method === 'markSaved').map((c) => c.args[0]);
    expect(marks).toEqual([snapped]);
    expect(fake.api.isDirty()).toBe(true);
    expect(loop.hasPending()).toBe(true);
    await vi.advanceTimersByTimeAsync(QUIET_MS);
    expect(put).toHaveBeenCalledTimes(2);
    expect(put.mock.calls[1][1]).toBe('{"b":2}');
    expect(put.mock.calls[1][2]).toBe(2);
    expect(fake.api.isDirty()).toBe(false);
    expect(loop.hasPending()).toBe(false);
  });

  it('flushes the board before switching to another one', async () => {
    const fake = createFakeFrame();
    useHostStore.setState({ frame: fake.api });
    const order: string[] = [];
    const put = vi.fn(async (id: string, _b: string, base: number) => {
      order.push(`put ${id}`);
      return putResult(base + 1);
    });
    const make = (itemId: string) => {
      const loop = new BoardSaveLoop({
        itemId,
        rev: 1,
        frame: () => fake.api,
        put,
        writeUnsaved: async () => {},
        clearUnsaved: async () => {},
      });
      const session: BoardSession = { itemId, flush: () => loop.flush(), onChange: (r) => loop.change(r) };
      return { loop, session };
    };
    const a = make('a');
    const b = make('b');
    const open = fake.api.open.bind(fake.api);
    fake.api.open = async (args) => {
      order.push(`open ${args.itemId}`);
      await open(args);
    };
    await openBoard(a.session, { itemId: 'a', board: '{}', name: 'A', readOnly: false, theme });
    fake.edit('{"a":1}');
    expect(put).not.toHaveBeenCalled();
    await openBoard(b.session, { itemId: 'b', board: '{}', name: 'B', readOnly: false, theme });
    expect(order).toEqual(['open a', 'put a', 'open b']);
    expect(put.mock.calls[0][1]).toBe('{"a":1}');
    // Nothing is pending for a: its timers were cleared by the flush.
    await vi.advanceTimersByTimeAsync(MAX_MS);
    expect(put).toHaveBeenCalledTimes(1);
  });

  it('flushes when the page is hidden and on the frame onSave (Ctrl/Cmd+S)', async () => {
    const { fake, put, loop } = setup();
    useHostStore.setState({ frame: fake.api });
    loop.attach();
    const session: BoardSession = {
      itemId: 'a',
      flush: () => loop.flush(),
      onChange: (r) => loop.change(r),
      onSave: () => void loop.flush(),
    };
    await openBoard(session, { itemId: 'a', board: '{}', name: 'A', readOnly: false, theme });
    // The fake frame's events are the host's.
    fake.edit();
    frameEvents.onChange?.({ itemId: 'a', revision: fake.api.snapshot().revision });
    expect(hasPending('a')).toBe(true);
    Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true });
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(0);
    expect(put).toHaveBeenCalledTimes(1);
    Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true });

    fake.edit();
    frameEvents.onSave?.('a');
    await vi.advanceTimersByTimeAsync(0);
    expect(put).toHaveBeenCalledTimes(2);
    expect(hasPending('a')).toBe(false);
    loop.dispose();
    expect(hasPending('a')).toBe(false);
  });

  it('writes the unsaved snapshot before the PUT and clears it after the 200', async () => {
    const { fake, put, writeUnsaved, clearUnsaved, loop } = setup('a', 7);
    await openIn(fake, 'a');
    put.mockImplementationOnce(async (_id, _b, base) => {
      expect(writeUnsaved).toHaveBeenCalledTimes(1);
      expect(clearUnsaved).not.toHaveBeenCalled();
      return putResult(base + 1);
    });
    edit(fake, loop, '{"x":1}');
    await loop.flush();
    expect(writeUnsaved).toHaveBeenCalledWith('a', expect.objectContaining({ baseRev: 7, board: '{"x":1}' }));
    expect(clearUnsaved).toHaveBeenCalledWith('a');
  });

  it('stops on a 412 and reports the conflict with the board it could not save', async () => {
    const fake = createFakeFrame();
    await openIn(fake, 'a');
    const onConflict = vi.fn();
    const put = vi.fn(async () => {
      throw new ApiError(412, 'rev_mismatch', 'changed', { rev: 5, etag: '"r5"' });
    });
    const loop = new BoardSaveLoop({
      itemId: 'a',
      rev: 1,
      frame: () => fake.api,
      put,
      writeUnsaved: async () => {},
      clearUnsaved: async () => {},
      onConflict,
    });
    edit(fake, loop, '{"mine":1}');
    await loop.flush();
    expect(onConflict).toHaveBeenCalledWith({ board: '{"mine":1}', rev: 5 });
    expect(loop.isConflicted()).toBe(true);
    // No more saves until it is resolved.
    edit(fake, loop);
    await vi.advanceTimersByTimeAsync(MAX_MS);
    expect(put).toHaveBeenCalledTimes(1);
    loop.resume(5);
    put.mockImplementation(async () => putResult(6) as never);
    edit(fake, loop);
    await vi.advanceTimersByTimeAsync(QUIET_MS);
    expect(put).toHaveBeenCalledTimes(2);
    expect((put.mock.calls[1] as unknown[])[2]).toBe(5);
  });

  it('backs off on network errors and explains a 413', async () => {
    const { fake, put, loop } = setup();
    await openIn(fake, 'a');
    put.mockRejectedValueOnce(new ApiError(0, 'network', 'offline'));
    put.mockRejectedValueOnce(new ApiError(0, 'network', 'offline'));
    edit(fake, loop);
    await loop.flush();
    expect(getSaveStatus('a').state).toBe('offline');
    await vi.advanceTimersByTimeAsync(BACKOFF_MS[0]);
    expect(put).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(BACKOFF_MS[1] - 1);
    expect(put).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(1);
    expect(put).toHaveBeenCalledTimes(3);
    expect(getSaveStatus('a').state).toBe('saved');

    put.mockRejectedValueOnce(new ApiError(413, 'payload_too_large', 'big', { limit_bytes: 50 * 1024 * 1024 }));
    edit(fake, loop);
    await loop.flush();
    expect(getSaveStatus('a')).toMatchObject({ state: 'error', message: expect.stringContaining('50 MB') });
  });

  it('a frame whose engine trapped counts as gone: flush resolves and nothing is sent', async () => {
    const { fake, put, loop } = setup('a');
    await openIn(fake, 'a');
    fake.edit();
    const trap = () => {
      throw new Error('RuntimeError: unreachable');
    };
    fake.api.isDirty = trap;
    fake.api.snapshot = trap;
    await expect(loop.flush()).resolves.toBeUndefined();
    expect(loop.hasPending()).toBe(false);
    fake.api.current = trap;
    await expect(loop.flush()).resolves.toBeUndefined();
    expect(put).not.toHaveBeenCalled();
  });

  it('never saves a board the frame does not show', async () => {
    const { fake, put, loop } = setup('a');
    await openIn(fake, 'b');
    fake.edit();
    loop.change(2);
    await vi.advanceTimersByTimeAsync(MAX_MS);
    await loop.flush();
    expect(put).not.toHaveBeenCalled();
  });
});
