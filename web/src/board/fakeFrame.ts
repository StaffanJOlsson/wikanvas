// A fake board frame (PLAN W6): implements BoardFrameApi without the engine, so the host, the save loop and the board
// page are tested before W5's real frame lands. It keeps the boards it was given, a revision counter and the dirty
// state as the engine would (§10.3: `open` resets undo, `markSaved(r)` clears dirty when nothing changed after r), logs
// every call, and lets a test act as the user or the engine: `edit()`, `pressSave()`, `fail(kind)`, `progress(…)`.
// M1.1 (§18.4): it has the embed members (`insertEmbed`, `setEmbedEditing`, `setEmbedTitles`, `requestEmbedLayout`;
// `embeds: false` makes a frame without them, like M1's), and the engine's side of them as helpers: `layout(l)` sends
// an `onEmbedLayout`, `openEmbed(objectId, point)` a double-click, `endEdit(objectId, reason)` the engine ending the
// editing, `focusBack(objectId)` the frame giving the focus back after a middle-button pan, `dropItem(item, point)` a
// sidebar drop; `inserted` lists the cards `insertEmbed` placed.
import type {
  BoardFrameApi,
  DraggedItem,
  EmbedBox,
  EmbedEditEndReason,
  EmbedLayout,
  FrameErrorKind,
  FrameEvents,
  FramePoint,
  InsertEmbedArgs,
  OpenArgs,
} from './bridge';
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
  // M1.1, board document embeds:
  /** Sends `onEmbedLayout` for the current board: the given fields over defaults (page `p1`, zoom 1, origin 0, an
   *  800 × 600 canvas, no occluders, `items` from `embeds`, `editing` the frame's). Returns what was sent. */
  layout(l?: Partial<Omit<EmbedLayout, 'embeds'>> & { embeds?: readonly Partial<EmbedBox>[] }): EmbedLayout;
  /** The last layout sent. */
  lastLayout(): EmbedLayout | null;
  /** A double-click (`point`) or Enter (`null`) on card `objectId` of the last layout: `onEmbedOpen`. */
  openEmbed(objectId: string, point?: FramePoint | null): void;
  /** The engine ends the editing (a press, focus, the card gone): `onEmbedEditEnd`. */
  endEdit(objectId: string, reason?: EmbedEditEndReason): void;
  /** The frame took the focus during the editing without ending it (a middle-button pan): `onEmbedFocusBack`. */
  focusBack(objectId: string): void;
  /** A sidebar document dropped on the canvas: `onDropItem`. */
  dropItem(item: DraggedItem, point: FramePoint): void;
  /** The cards `insertEmbed` placed (id and arguments), in order. */
  readonly inserted: ({ id: string } & InsertEmbedArgs)[];
  /** What `setEmbedEditing` set last. */
  embedEditing(): string | null;
  /** What `setEmbedTitles` set last. */
  embedTitles(): Readonly<Record<string, string>>;
}

export interface FakeFrameOptions {
  /** Keep `connect` pending until `resolveConnect()` or `rejectConnect()`. */
  manualConnect?: boolean;
  engineVersion?: string;
  /** Without the M1.1 embed members (default: with them). */
  embeds?: boolean;
}

/** A card of a fake layout: defaults for everything not given. */
export function embedBox(over: Partial<EmbedBox> & { objectId: string }): EmbedBox {
  return {
    item: '0192f0e0-0000-7000-8000-000000000001',
    x: -240,
    y: -180,
    w: 480,
    h: 360,
    angle: 0,
    label: 'Untitled',
    selected: false,
    locked: false,
    ...over,
  };
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
  let editing: string | null = null;
  let titles: Readonly<Record<string, string>> = {};
  let last: EmbedLayout | null = null;
  let seq = 0;
  const inserted: ({ id: string } & InsertEmbedArgs)[] = [];

  const log = (method: keyof BoardFrameApi, ...args: unknown[]) => calls.push({ method, args });

  const embedApi: Pick<BoardFrameApi, 'insertEmbed' | 'setEmbedEditing' | 'setEmbedTitles' | 'requestEmbedLayout'> = {
    insertEmbed(a) {
      log('insertEmbed', a);
      if (current === null) throw new Error('no board is open');
      if (readOnly) return null;
      const id = `inserted${++seq}`;
      inserted.push({ id, ...a });
      return id;
    },
    setEmbedEditing(id) {
      log('setEmbedEditing', id);
      editing = id;
    },
    setEmbedTitles(t) {
      log('setEmbedTitles', t);
      titles = { ...t };
    },
    requestEmbedLayout() {
      log('requestEmbedLayout');
    },
  };

  const api: BoardFrameApi = {
    ...(opts.embeds === false ? {} : embedApi),
    engineVersion: opts.engineVersion ?? '0.2.0-beta.3+embed.quire.fake',
    formatVersion: 2,
    async open(a: OpenArgs) {
      log('open', a);
      // host_load_doc: any embed editing ends ('gone'), the title map is cleared. Like frame.js, the 'gone' carries
      // the board being left (the engine reports it after setBoard, `leaving`), not the one being opened.
      const was = editing;
      const leaving = current;
      editing = null;
      titles = {};
      last = null;
      current = a.itemId;
      if (was !== null && leaving !== null && opts.embeds !== false) {
        events?.onEmbedEditEnd?.({ itemId: leaving, objectId: was, reason: 'gone' });
      }
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
    layout(l = {}) {
      if (current === null) throw new Error('no board is open');
      const embeds = (l.embeds ?? []).map((e, i) => embedBox({ objectId: `card${i + 1}`, ...e }));
      const sent: EmbedLayout = {
        itemId: current,
        page: 'p1',
        zoom: 1,
        originX: 0,
        originY: 0,
        canvas: { x: 0, y: 0, width: 800, height: 600 },
        occluders: [],
        items: [...new Set(embeds.map((e) => e.item))].sort(),
        editing,
        ...l,
        embeds,
      };
      last = sent;
      events?.onEmbedLayout?.(sent);
      return sent;
    },
    lastLayout: () => last,
    openEmbed(objectId, point = null) {
      if (current === null) throw new Error('no board is open');
      if (readOnly) return;
      const card = last?.embeds.find((e) => e.objectId === objectId);
      if (!card) throw new Error(`no card ${objectId} in the last layout`);
      events?.onEmbedOpen?.({ itemId: current, objectId, item: card.item, point });
    },
    endEdit(objectId, reason = 'press') {
      if (current === null) throw new Error('no board is open');
      if (editing === objectId) editing = null;
      events?.onEmbedEditEnd?.({ itemId: current, objectId, reason });
    },
    focusBack(objectId) {
      if (current === null) throw new Error('no board is open');
      events?.onEmbedFocusBack?.({ itemId: current, objectId });
    },
    dropItem(item, point) {
      if (current === null) throw new Error('no board is open');
      if (readOnly) return;
      events?.onDropItem?.({ itemId: current, item, point });
    },
    inserted,
    embedEditing: () => editing,
    embedTitles: () => titles,
  };
}
