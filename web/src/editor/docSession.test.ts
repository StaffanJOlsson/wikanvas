// DocSession (docs/ARCHITECTURE.md §9.4): debounce and max wait, the outbox (throttled writes, replay), the
// missing_deps resend, the flush on visibilitychange → hidden, and the offline back-off.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as Y from 'yjs';
import { ApiError } from '../api/client';
import { fromBase64, toBase64 } from '../api/queries/docs';
import type { DocState } from '../api/gen/DocState';
import type { SaveStatus } from '../state/saveStatus';
import { BACKOFF_MS, DocSession, idbOutbox, type DocSessionDeps, type Outbox } from './docSession';

const EMPTY: DocState = { update: toBase64(new Uint8Array([0, 0])), state_vector: '', seq: 0, field: 'default', schema_version: 1 };

function memoryOutbox() {
  const data = new Map<string, Uint8Array>();
  const writes: number[] = [];
  const outbox: Outbox = {
    get: async (id) => data.get(id),
    set: async (id, u) => {
      writes.push(Date.now());
      data.set(id, u);
    },
    del: async (id) => {
      data.delete(id);
    },
  };
  return { outbox, data, writes };
}

function typeInto(doc: Y.Doc, text: string) {
  doc.getText('t').insert(doc.getText('t').length, text);
}

function deps(over: Partial<DocSessionDeps> = {}) {
  const statuses: SaveStatus[] = [];
  const posts: Uint8Array[] = [];
  const mem = memoryOutbox();
  const d: DocSessionDeps = {
    fetchDoc: vi.fn(async () => EMPTY),
    postUpdate: vi.fn(async (_id: string, u: Uint8Array) => {
      posts.push(u);
      return { seq: posts.length, state_vector: '', updated_at: '' };
    }),
    outbox: mem.outbox,
    status: (_id, s) => statuses.push(s),
    now: () => Date.now(),
    win: new EventTarget() as unknown as Window,
    doc: Object.assign(new EventTarget(), { visibilityState: 'visible' }) as unknown as Document,
    ...over,
  };
  return { d, statuses, posts, mem };
}

const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

describe('DocSession', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('debounces 1 s, at most 5 s apart', async () => {
    const { d, posts } = deps();
    const s = await DocSession.open('doc-1', d);
    typeInto(s.ydoc, 'a');
    await vi.advanceTimersByTimeAsync(900);
    expect(posts).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(200);
    expect(posts).toHaveLength(1);
    // Typing every 500 ms never lets the debounce fire: the 5 s max wait does.
    for (let i = 0; i < 9; i++) {
      typeInto(s.ydoc, 'b');
      await vi.advanceTimersByTimeAsync(500);
    }
    expect(posts).toHaveLength(1);
    typeInto(s.ydoc, 'c');
    await vi.advanceTimersByTimeAsync(600);
    expect(posts).toHaveLength(2);
    // The POSTed updates together hold every change.
    const check = new Y.Doc();
    for (const p of posts) Y.applyUpdate(check, p);
    expect(check.getText('t').toString()).toBe(`a${'b'.repeat(9)}c`);
    expect(s.hasPending()).toBe(false);
    typeInto(s.ydoc, 'd');
    expect(s.hasPending()).toBe(true);
    await s.flush();
    expect(posts).toHaveLength(3);
    expect(s.hasPending()).toBe(false);
    s.destroy();
  });

  it('writes the outbox on every update, at most every 250 ms, and before a POST', async () => {
    const { d, mem } = deps();
    const s = await DocSession.open('doc-2', d);
    typeInto(s.ydoc, 'x');
    await settle();
    expect(mem.writes).toHaveLength(1);
    typeInto(s.ydoc, 'y');
    typeInto(s.ydoc, 'z');
    await settle();
    expect(mem.writes).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(260);
    expect(mem.writes).toHaveLength(2);
    const saved = mem.data.get('doc-2');
    expect(saved).toBeDefined();
    const check = new Y.Doc();
    Y.applyUpdate(check, saved as Uint8Array);
    expect(check.getText('t').toString()).toBe('xyz');
    await vi.advanceTimersByTimeAsync(1000);
    // Written right before the POST, then dropped once it succeeded.
    expect(mem.writes.length).toBeGreaterThanOrEqual(3);
    expect(mem.data.has('doc-2')).toBe(false);
    s.destroy();
  });

  it('resends with the server state vector on missing_deps', async () => {
    const server = new Y.Doc();
    const calls: Uint8Array[] = [];
    let first = true;
    const { d } = deps({
      postUpdate: vi.fn(async (_id: string, u: Uint8Array) => {
        calls.push(u);
        if (first) {
          first = false;
          throw new ApiError(409, 'missing_deps', 'missing', { state_vector: toBase64(Y.encodeStateVector(server)) });
        }
        Y.applyUpdate(server, u);
        return { seq: 1, state_vector: '', updated_at: '' };
      }),
    });
    const s = await DocSession.open('doc-3', d);
    typeInto(s.ydoc, 'hello');
    await vi.advanceTimersByTimeAsync(1100);
    expect(calls).toHaveLength(2);
    expect(server.getText('t').toString()).toBe('hello');
    expect(s.hasPending()).toBe(false);
    s.destroy();
  });

  it('flushes on visibilitychange → hidden', async () => {
    const { d, posts } = deps();
    const s = await DocSession.open('doc-4', d);
    typeInto(s.ydoc, 'q');
    (d.doc as unknown as { visibilityState: string }).visibilityState = 'hidden';
    d.doc?.dispatchEvent(new Event('visibilitychange'));
    await settle();
    expect(posts).toHaveLength(1);
    s.destroy();
  });

  it('backs off 1, 2, 5, 10, then 30 s when offline, and retries at once when online', async () => {
    const statuses: SaveStatus[] = [];
    const attempts: number[] = [];
    let online = false;
    const { d } = deps({
      status: (_id, st) => statuses.push(st),
      postUpdate: vi.fn(async () => {
        attempts.push(Date.now());
        if (!online) throw new ApiError(0, 'network', 'offline');
        return { seq: 1, state_vector: '', updated_at: '' };
      }),
    });
    const s = await DocSession.open('doc-5', d);
    const t0 = Date.now();
    typeInto(s.ydoc, 'w');
    await vi.advanceTimersByTimeAsync(1000);
    expect(attempts).toHaveLength(1);
    expect(statuses.at(-1)?.state).toBe('offline');
    let expected = attempts[0];
    for (const delay of [...BACKOFF_MS, 30_000]) {
      expected += delay;
      await vi.advanceTimersByTimeAsync(delay);
      expect(attempts.at(-1)).toBe(expected);
    }
    expect(attempts.length).toBe(BACKOFF_MS.length + 2);
    online = true;
    d.win?.dispatchEvent(new Event('online'));
    await settle();
    expect(attempts.length).toBe(BACKOFF_MS.length + 3);
    expect(statuses.at(-1)?.state).toBe('saved');
    expect(attempts[0] - t0).toBe(1000);
    s.destroy();
  });

  it('shows signed_out on 401 and error with a retry otherwise', async () => {
    const statuses: SaveStatus[] = [];
    let code = 401;
    const { d } = deps({
      status: (_id, st) => statuses.push(st),
      postUpdate: vi.fn(async () => {
        throw new ApiError(code, code === 401 ? 'unauthenticated' : 'internal', 'no');
      }),
    });
    const s = await DocSession.open('doc-6', d);
    typeInto(s.ydoc, 'e');
    await vi.advanceTimersByTimeAsync(1000);
    expect(statuses.at(-1)?.state).toBe('signed_out');
    code = 500;
    statuses.at(-1)?.retry?.();
    await settle();
    expect(statuses.at(-1)?.state).toBe('error');
    expect(typeof statuses.at(-1)?.retry).toBe('function');
    s.destroy();
  });
});

describe('DocSession outbox replay (IndexedDB)', () => {
  it('replays unsent updates from the IndexedDB outbox when the document opens', async () => {
    const author = new Y.Doc();
    typeInto(author, 'unsent work');
    await idbOutbox.set('doc-7', Y.encodeStateAsUpdate(author));
    const posts: Uint8Array[] = [];
    const s = await DocSession.open('doc-7', {
      fetchDoc: async () => EMPTY,
      postUpdate: async (_id, u) => {
        posts.push(u);
        return { seq: 1, state_vector: '', updated_at: '' };
      },
      status: () => undefined,
      win: new EventTarget() as unknown as Window,
      doc: Object.assign(new EventTarget(), { visibilityState: 'visible' }) as unknown as Document,
    });
    expect(s.ydoc.getText('t').toString()).toBe('unsent work');
    expect(s.hasPending()).toBe(true);
    await s.flush();
    expect(posts).toHaveLength(1);
    const check = new Y.Doc();
    Y.applyUpdate(check, posts[0]);
    expect(check.getText('t').toString()).toBe('unsent work');
    expect(await idbOutbox.get('doc-7')).toBeUndefined();
    s.destroy();
    expect(fromBase64(toBase64(new Uint8Array([1, 2, 255])))).toEqual(new Uint8Array([1, 2, 255]));
  });
});
