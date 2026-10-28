// Document autosave (docs/ARCHITECTURE.md §9.4): one DocSession per open document.
// - Load: GET the doc and apply it with origin 'server'; then apply the IndexedDB outbox (`quire.outbox/<itemId>`)
//   locally, which queues it.
// - Queue and send: every local update is queued and the merged pending update is written to the outbox (at most one
//   write per 250 ms, and always right before a POST). A 1 s debounce, at most 5 s apart, then POST
//   `Y.mergeUpdates(queue)`; on success those updates and the outbox go. 409 `missing_deps`: POST
//   `Y.encodeStateAsUpdate(ydoc, serverSV)`. Network error: `offline`, back off 1, 2, 5, 10, then 30 s, and retry at
//   once on `online`. Other errors: `error` with a retry action; 401: `signed_out`.
// - `pm_json` (the editor's JSON) rides along at most every 30 s and when leaving the page.
// - Focus or visibility → visible: GET `?since=<local SV>`, applied with origin 'server'. Visibility → hidden, blur,
//   route change and Ctrl/Cmd+S flush. On `pagehide`, a payload under 60 KB goes with `fetch(keepalive)`; a larger
//   one stays in the outbox.
import { createStore, del, get, set, type UseStore } from 'idb-keyval';
import * as Y from 'yjs';
import { isApiError } from '../api/client';
import { fetchDoc, fromBase64, postUpdate } from '../api/queries/docs';
import type { DocState } from '../api/gen/DocState';
import type { DocUpdateResult } from '../api/gen/DocUpdateResult';
import type { MissingDepsDetails } from '../api/gen/MissingDepsDetails';
import { registerPending } from '../state/pending';
import { setSaveStatus, type SaveStatus } from '../state/saveStatus';

export const DEBOUNCE_MS = 1000;
export const MAX_WAIT_MS = 5000;
export const OUTBOX_THROTTLE_MS = 250;
export const PM_JSON_EVERY_MS = 30_000;
export const KEEPALIVE_MAX_BYTES = 60 * 1024;
/** Retry delays after a network error, in ms; the last repeats. */
export const BACKOFF_MS = [1000, 2000, 5000, 10_000, 30_000];

/** The origin the session applies server state with (UniqueID and the queue ignore it). */
export const SERVER_ORIGIN = 'server';

/** Where unsent updates wait: IndexedDB `quire.outbox`, one entry per item. */
export interface Outbox {
  get(itemId: string): Promise<Uint8Array | undefined>;
  set(itemId: string, update: Uint8Array): Promise<void>;
  del(itemId: string): Promise<void>;
}

let store: UseStore | null = null;

/** The IndexedDB outbox (`quire.outbox/<itemId>`). */
export const idbOutbox: Outbox = {
  get: (id) => get<Uint8Array>(id, (store ??= createStore('quire.outbox', 'updates'))),
  set: (id, u) => set(id, u, (store ??= createStore('quire.outbox', 'updates'))),
  del: (id) => del(id, (store ??= createStore('quire.outbox', 'updates'))),
};

export interface DocSessionDeps {
  fetchDoc(itemId: string, since?: Uint8Array): Promise<DocState>;
  postUpdate(itemId: string, update: Uint8Array, pmJson?: unknown, keepalive?: boolean): Promise<DocUpdateResult>;
  outbox: Outbox;
  status(itemId: string, s: SaveStatus): void;
  now(): number;
  /** Where focus, visibility, online and pagehide events come from (default: `window` and `document`). */
  win?: Window;
  doc?: Document;
}

const defaultDeps = (): DocSessionDeps => ({
  fetchDoc: (id, since) => fetchDoc(id, since),
  postUpdate,
  outbox: idbOutbox,
  status: setSaveStatus,
  now: () => Date.now(),
});

export class DocSession {
  readonly itemId: string;
  readonly ydoc: Y.Doc;
  /** The newest seq the server confirmed. */
  seq = 0;
  private deps: DocSessionDeps;
  private queue: Uint8Array[] = [];
  private debounce: ReturnType<typeof setTimeout> | null = null;
  private maxWait: ReturnType<typeof setTimeout> | null = null;
  private retry: ReturnType<typeof setTimeout> | null = null;
  private outboxTimer: ReturnType<typeof setTimeout> | null = null;
  private lastOutboxWrite = -Infinity;
  private lastPmJson = -Infinity;
  private failures = 0;
  private inflight: Promise<void> | null = null;
  private destroyed = false;
  private pmJson: (() => unknown) | null = null;
  private cleanups: (() => void)[] = [];

  private constructor(itemId: string, ydoc: Y.Doc, deps: DocSessionDeps) {
    this.itemId = itemId;
    this.ydoc = ydoc;
    this.deps = deps;
  }

  /** Loads the document (server state, then the outbox) and starts saving local changes. */
  static async open(itemId: string, deps: Partial<DocSessionDeps> = {}): Promise<DocSession> {
    const d = { ...defaultDeps(), ...deps };
    const state = await d.fetchDoc(itemId);
    const ydoc = new Y.Doc();
    Y.applyUpdate(ydoc, fromBase64(state.update), SERVER_ORIGIN);
    const s = new DocSession(itemId, ydoc, d);
    s.seq = state.seq;
    ydoc.on('update', s.onUpdate);
    const pending = await d.outbox.get(itemId).catch(() => undefined);
    if (pending && pending.length > 0) Y.applyUpdate(ydoc, pending, 'outbox');
    s.listen();
    return s;
  }

  /** The editor's JSON, attached to a POST at most every 30 s and when leaving. */
  setPmJsonProvider(f: (() => unknown) | null): void {
    this.pmJson = f;
  }

  hasPending(): boolean {
    return this.queue.length > 0 || this.inflight !== null;
  }

  private onUpdate = (update: Uint8Array, origin: unknown) => {
    if (origin === SERVER_ORIGIN || this.destroyed) return;
    this.queue.push(update);
    this.writeOutboxThrottled();
    this.schedule();
  };

  private schedule(): void {
    if (this.debounce) clearTimeout(this.debounce);
    this.debounce = setTimeout(() => void this.send(), DEBOUNCE_MS);
    if (!this.maxWait) this.maxWait = setTimeout(() => void this.send(), MAX_WAIT_MS);
  }

  private clearTimers(): void {
    if (this.debounce) clearTimeout(this.debounce);
    if (this.maxWait) clearTimeout(this.maxWait);
    if (this.retry) clearTimeout(this.retry);
    this.debounce = this.maxWait = this.retry = null;
  }

  private pendingUpdate(): Uint8Array {
    return this.queue.length === 1 ? this.queue[0] : Y.mergeUpdates(this.queue);
  }

  private writeOutboxThrottled(): void {
    const now = this.deps.now();
    if (now - this.lastOutboxWrite >= OUTBOX_THROTTLE_MS) {
      void this.writeOutbox();
      return;
    }
    if (this.outboxTimer) return;
    this.outboxTimer = setTimeout(() => {
      this.outboxTimer = null;
      void this.writeOutbox();
    }, OUTBOX_THROTTLE_MS - (now - this.lastOutboxWrite));
  }

  private async writeOutbox(): Promise<void> {
    this.lastOutboxWrite = this.deps.now();
    if (this.queue.length === 0) return;
    try {
      await this.deps.outbox.set(this.itemId, this.pendingUpdate());
    } catch (e) {
      console.warn('outbox write failed', e);
    }
  }

  /** Sends everything queued now (and waits for a send in flight). */
  async flush(): Promise<void> {
    if (this.inflight) await this.inflight;
    if (this.queue.length > 0) await this.send();
  }

  private send(): Promise<void> {
    if (this.inflight) return this.inflight;
    this.clearTimers();
    if (this.queue.length === 0) return Promise.resolve();
    const run = this.post().finally(() => {
      this.inflight = null;
    });
    this.inflight = run;
    return run;
  }

  private pmJsonNow(force: boolean): unknown {
    if (!this.pmJson) return undefined;
    const now = this.deps.now();
    if (!force && now - this.lastPmJson < PM_JSON_EVERY_MS) return undefined;
    this.lastPmJson = now;
    return this.pmJson();
  }

  private async post(): Promise<void> {
    const n = this.queue.length;
    const update = this.pendingUpdate();
    if (this.outboxTimer) {
      clearTimeout(this.outboxTimer);
      this.outboxTimer = null;
    }
    await this.writeOutbox();
    this.deps.status(this.itemId, { state: 'saving' });
    try {
      let result: DocUpdateResult;
      try {
        result = await this.deps.postUpdate(this.itemId, update, this.pmJsonNow(false));
      } catch (e) {
        if (!isApiError(e, 'missing_deps')) throw e;
        // The server lacks something we have: send all of it relative to the server's state.
        const sv = fromBase64((e.details as MissingDepsDetails).state_vector);
        result = await this.deps.postUpdate(this.itemId, Y.encodeStateAsUpdate(this.ydoc, sv));
      }
      this.seq = result.seq;
      this.failures = 0;
      this.queue.splice(0, n);
      if (this.queue.length === 0) {
        await this.deps.outbox.del(this.itemId).catch(() => undefined);
        this.deps.status(this.itemId, { state: 'saved' });
      } else {
        await this.writeOutbox();
        this.schedule();
      }
    } catch (e) {
      this.failed(e);
    }
  }

  private failed(e: unknown): void {
    const retry = () => void this.send();
    if (isApiError(e) && e.status === 0) {
      const delay = BACKOFF_MS[Math.min(this.failures, BACKOFF_MS.length - 1)];
      this.failures += 1;
      this.deps.status(this.itemId, { state: 'offline', retry });
      this.retry = setTimeout(retry, delay);
    } else if (isApiError(e) && e.status === 401) {
      this.deps.status(this.itemId, { state: 'signed_out', retry });
    } else {
      const message = e instanceof Error ? e.message : String(e);
      this.deps.status(this.itemId, { state: 'error', message, retry });
    }
  }

  /** GET `?since=<local SV>` and apply what the server has with origin 'server' (another tab, an API write). */
  async sync(): Promise<void> {
    try {
      const state = await this.deps.fetchDoc(this.itemId, Y.encodeStateVector(this.ydoc));
      Y.applyUpdate(this.ydoc, fromBase64(state.update), SERVER_ORIGIN);
      if (state.seq > this.seq && this.queue.length === 0) this.seq = state.seq;
    } catch (e) {
      if (!(isApiError(e) && e.status === 0)) console.warn('doc sync failed', e);
    }
  }

  /** Leaving the page: a small payload goes with keepalive, a large one stays in the outbox. */
  pagehide(): void {
    if (this.queue.length === 0) return;
    const update = this.pendingUpdate();
    void this.writeOutbox();
    if (update.length < KEEPALIVE_MAX_BYTES) {
      const n = this.queue.length;
      void this.deps
        .postUpdate(this.itemId, update, this.pmJsonNow(true), true)
        .then(() => {
          this.queue.splice(0, n);
          if (this.queue.length === 0) return this.deps.outbox.del(this.itemId);
        })
        .catch(() => undefined);
    }
  }

  private listen(): void {
    const win = this.deps.win ?? (typeof window !== 'undefined' ? window : undefined);
    const doc = this.deps.doc ?? (typeof document !== 'undefined' ? document : undefined);
    const on = (target: EventTarget | undefined, type: string, f: EventListener) => {
      if (!target) return;
      target.addEventListener(type, f);
      this.cleanups.push(() => target.removeEventListener(type, f));
    };
    on(doc, 'visibilitychange', () => {
      if (doc?.visibilityState === 'hidden') void this.flush();
      else void this.sync();
    });
    on(win, 'focus', () => void this.sync());
    on(win, 'blur', () => void this.flush());
    on(win, 'online', () => {
      if (this.retry) {
        clearTimeout(this.retry);
        this.retry = null;
      }
      if (this.queue.length > 0) void this.send();
    });
    on(win, 'pagehide', () => this.pagehide());
    this.cleanups.push(registerPending(this.itemId, { hasPending: () => this.hasPending(), flush: () => this.flush() }));
  }

  /** Stops listening (after a last flush by the caller). The Y.Doc stays usable. */
  destroy(): void {
    this.destroyed = true;
    this.clearTimers();
    if (this.outboxTimer) clearTimeout(this.outboxTimer);
    for (const c of this.cleanups.splice(0)) c();
    this.ydoc.off('update', this.onUpdate);
  }
}
