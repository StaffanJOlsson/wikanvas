// A fake board frame (PLAN W6): implements BoardFrameApi without the engine, so the host, the save loop and the board
// page are tested before W5's real frame lands. It keeps the boards it was given, a revision counter and the dirty
// state as the engine would (§10.3: `open` resets undo, `markSaved(r)` clears dirty when nothing changed after r), logs
// every call, and lets a test act as the user or the engine: `edit()`, `pressSave()`, `fail(kind)`, `progress(…)`.
import type { BoardFrameApi, FrameErrorKind, FrameEvents, OpenArgs } from './bridge';
import type { FrameDriver } from './frameDriver';

export type FakeCall = { method: keyof BoardFrameApi; args: unknown[] };

export interface FakeFrame {
  /** Plug into BoardFrameHost with `setFrameDriver(fake.driver)`. */
  readonly driver: FrameDriver;
  readonly api: BoardFrameApi;
  /** Every API call, in order. */
  readonly calls: FakeCall[];
  /** The events the host connected with (null before `connect`). */
  events(): FrameEvents | null;
  /** The iframes the driver created. */
  readonly iframes: HTMLIFrameElement[];
  /** The board JSON the frame shows now. */
  board(): string | null;
  active(): boolean;
  theme(): 'light' | 'dark' | null;
  readOnly(): boolean;
  name(): string | null;
  /** The user changes the board: the revision goes up, it is dirty, `onChange` fires. Returns the new revision. */
  edit(board?: string): number;
  /** Ctrl/Cmd+S in the canvas. */
  pressSave(): void;
  /** The engine reports an error for the current board (or `itemId`). */
  fail(kind: FrameErrorKind, message?: string, itemId?: string | null): void;
  progress(loaded: number, total: number): void;
  /** Resolves `connect` (by default it resolves at once). */
  resolveConnect(): void;
  /** Makes `connect` reject like the W0b stub frame (`onError`, then the rejection); with `missing`, like the real
   *  driver for a page without the frame script (no event, `{kind, message, missing: true}`). */
  rejectConnect(kind: FrameErrorKind, message?: string, missing?: boolean): void;
}

export interface FakeFrameOptions {
  /** Keep `connect` pending until `resolveConnect()` or `rejectConnect()`. */
  manualConnect?: boolean;
  engineVersion?: string;
}

export function createFakeFrame(opts: FakeFrameOptions = {}): FakeFrame {
  const calls: FakeCall[] = [];
  const iframes: HTMLIFrameElement[] = [];
  let events: FrameEvents | null = null;
  let current: string | null = null;
  let board: string | null = null;
  let revision = 0;
  let saved = 0;
  let active = true;
  let theme: 'light' | 'dark' | null = null;
  let readOnly = false;
  let name: string | null = null;
  let settle: { resolve(api: BoardFrameApi): void; reject(e: unknown): void } | null = null;

  const log = (method: keyof BoardFrameApi, ...args: unknown[]) => calls.push({ method, args });

  const api: BoardFrameApi = {
    engineVersion: opts.engineVersion ?? '0.2.0-beta.3+embed.quire.fake',
    formatVersion: 2,
    async open(a: OpenArgs) {
      log('open', a);
      current = a.itemId;
      board = a.board;
      readOnly = a.readOnly;
      name = a.name;
      theme = a.theme;
      revision += 1;
      saved = revision;
      events?.onReady?.(a.itemId);
    },
    current() {
      return current;
    },
    snapshot() {
      log('snapshot');
      if (current === null) throw new Error('no board is open');
      return { itemId: current, board: board ?? '', revision };
    },
    isDirty() {
      return current !== null && revision > saved;
    },
    markSaved(r: number) {
      log('markSaved', r);
      saved = Math.max(saved, r);
    },
    setTheme(t) {
      log('setTheme', t);
      theme = t;
    },
    setReadOnly(b) {
      log('setReadOnly', b);
      readOnly = b;
    },
    setName(n) {
      log('setName', n);
      name = n;
    },
    setActive(b) {
      log('setActive', b);
      active = b;
    },
    focus() {
      log('focus');
    },
    fitView() {
      log('fitView');
    },
  };

  const driver: FrameDriver = {
    create(src) {
      const iframe = document.createElement('iframe');
      iframe.className = 'board-frame';
      iframe.dataset.src = src;
      iframes.push(iframe);
      return iframe;
    },
    connect(_iframe, e) {
      events = e;
      if (!opts.manualConnect) return Promise.resolve(api);
      return new Promise<BoardFrameApi>((resolve, reject) => {
        settle = { resolve, reject };
      });
    },
  };

  return {
    driver,
    api,
    calls,
    iframes,
    events: () => events,
    board: () => board,
    active: () => active,
    theme: () => theme,
    readOnly: () => readOnly,
    name: () => name,
    edit(next) {
      if (current === null) throw new Error('no board is open');
      if (next !== undefined) board = next;
      revision += 1;
      events?.onChange?.({ itemId: current, revision });
      return revision;
    },
    pressSave() {
      if (current !== null) events?.onSave?.(current);
    },
    fail(kind, message = `fake ${kind}`, itemId = current) {
      events?.onError?.({ itemId, kind, message });
    },
    progress(loaded, total) {
      events?.onProgress?.({ loaded, total });
    },
    resolveConnect() {
      settle?.resolve(api);
      settle = null;
    },
    rejectConnect(kind, message = `fake ${kind}`, missing = false) {
      if (!missing) events?.onError?.({ itemId: null, kind, message });
      settle?.reject(missing ? { kind, message, missing } : { kind, message });
      settle = null;
    },
  };
}
