// The board frame host's state, outside React (docs/ARCHITECTURE.md §9.5, D37: BoardFrameHost is mounted beside the
// routes, so pages reach it through an external store, not a React context).
//
// - `useHostStore` holds what pages render from: the connected frame, a failure, the download progress, the surface
//   the iframe covers, and whether the current board is shown.
// - `frameEvents` are the FrameEvents the host connects with. Each event carries an itemId; events for any board but
//   the current one are ignored (§10.4: "the host ignores events whose id is not current"). Host keys go to
//   `runHostShortcut`.
// - `flushPrevious(session)` waits for the board shown before (or the same board's earlier session, whose last PUT may
//   still be running) to save; a page calls it before its GET (§9.5 "Open": 1. flush, 2. GET, then open).
// - `openBoard(session, args)` flushes the previous board's session before it opens the next one (§9.5 "Open").
// - Board document embeds (M1.1, §18.5): the current board's last `EmbedLayout` (dropped on every open, a frame
//   reset or removal), and the overlay's `EmbedSink`, which gets the current board's embed events; after every
//   `open()` the host asks the frame for a layout (`requestEmbedLayout`).
import { create } from 'zustand';
import { runHostShortcut } from '../app/hotkeys';
import type {
  BoardFrameApi,
  DropItemEvent,
  EmbedEditEndEvent,
  EmbedLayout,
  EmbedOpenEvent,
  FrameError,
  FrameEvents,
  OpenArgs,
} from './bridge';

/** Why boards open read-only in BoardPreview instead of the editor. */
export type Failure = Pick<FrameError, 'kind' | 'message'>;

export interface HostState {
  /** A BoardFrameHost is mounted (AppShell). */
  mounted: boolean;
  /** The iframe, once created (the first board visit) and until it is removed. */
  iframe: HTMLIFrameElement | null;
  /** The connected frame API. */
  frame: BoardFrameApi | null;
  failure: Failure | null;
  /** The engine download (`onProgress`), until it finished. */
  progress: { loaded: number; total: number } | null;
  /** The BoardSurface element the iframe covers. */
  surface: HTMLElement | null;
  /** The current board is open in the frame, so the iframe may be shown over the surface. */
  shown: boolean;
  /** When each board was last saved (ms), for "Changes up to HH:MM were saved" after a panic. */
  savedAt: Readonly<Record<string, number>>;
  /** The fixed layer holding the iframe and the embeds overlay (`.board-frame-layer`; Focus fullscreens it). */
  layer: HTMLElement | null;
}

const initial: HostState = {
  mounted: false,
  iframe: null,
  frame: null,
  failure: null,
  progress: null,
  surface: null,
  shown: false,
  savedAt: {},
  layer: null,
};

export const useHostStore = create<HostState>(() => ({ ...initial }));

/** One open board, as the board page drives it. */
export interface BoardSession {
  readonly itemId: string;
  /** Saves pending changes now (the save loop's flush). */
  flush(): Promise<void>;
  onChange?(revision: number): void;
  onSave?(): void;
  onReady?(): void;
  /** The frame could not load this board (`onError{kind:'load'}`). */
  onLoadError?(message: string): void;
}

let current: string | null = null;
let active: BoardSession | null = null;

/** Where the current board's embed events go (the overlay's controller, `embed/controller.ts`). */
export interface EmbedSink {
  layout(l: EmbedLayout): void;
  open(e: EmbedOpenEvent): void;
  editEnd(e: EmbedEditEndEvent): void;
  drop(e: DropItemEvent): void;
  /** A board is about to be opened (another one, or the same one again), or the frame was reset or removed: the
   *  layout is gone, the overlay is emptied, editing ends. */
  reset(): void;
  /** `open()` of `itemId` resolved. */
  opened(a: { itemId: string; readOnly: boolean }): void;
}

let sink: EmbedSink | null = null;
let embedLayout: EmbedLayout | null = null;
/** The `items` of each board's last layout (kept across a board switch: the page discards "Text doc" documents that
 *  are on no card after it left the board, §18.6 (a)). */
const itemsByBoard = new Map<string, readonly string[]>();

/** Sets the overlay's sink (null: none). */
export function setEmbedSink(s: EmbedSink | null): void {
  sink = s;
}

/** The current board's last layout. */
export function currentEmbedLayout(): EmbedLayout | null {
  return embedLayout;
}

/** The `items` of `itemId`'s last layout, or null when none was reported. */
export function lastEmbedItems(itemId: string): readonly string[] | null {
  return itemsByBoard.get(itemId) ?? null;
}

function dropLayout(): void {
  embedLayout = null;
  sink?.reset();
}

/** `frame.open(args)` with the embed bookkeeping around it (BoardSurface's reopen of the same board uses it too). */
export async function openInFrame(frame: BoardFrameApi, args: OpenArgs): Promise<void> {
  dropLayout();
  await frame.open(args);
  if (current !== args.itemId) return;
  try {
    frame.requestEmbedLayout?.();
  } catch (e) {
    console.warn('requestEmbedLayout failed', e);
  }
  sink?.opened({ itemId: args.itemId, readOnly: args.readOnly });
}

/** The itemId the frame shows (or is about to show). */
export function currentItem(): string | null {
  return current;
}

/** The FrameEvents of the host's `connect`. */
export const frameEvents: FrameEvents = {
  onProgress(p) {
    useHostStore.setState({ progress: p.loaded >= p.total ? null : p });
  },
  onReady(itemId) {
    if (itemId === current && active?.itemId === itemId) active.onReady?.();
  },
  onChange(e) {
    if (e.itemId === current && active?.itemId === e.itemId) active.onChange?.(e.revision);
  },
  onSave(itemId) {
    if (itemId === current && active?.itemId === itemId) active.onSave?.();
  },
  onError(e) {
    if (e.itemId !== null && e.itemId !== current) return;
    if (e.kind === 'load') {
      active?.onLoadError?.(e.message);
      return;
    }
    useHostStore.setState((s) => ({ failure: s.failure ?? { kind: e.kind, message: e.message }, shown: false }));
  },
  onHostKey(k) {
    runHostShortcut(k);
  },
  onEmbedLayout(e) {
    if (e.itemId !== current) return;
    embedLayout = e;
    itemsByBoard.set(e.itemId, e.items);
    sink?.layout(e);
  },
  onEmbedOpen(e) {
    if (e.itemId === current && active?.itemId === e.itemId) sink?.open(e);
  },
  onEmbedEditEnd(e) {
    if (e.itemId === current) sink?.editEnd(e);
  },
  onDropItem(e) {
    if (e.itemId === current && active?.itemId === e.itemId) sink?.drop(e);
  },
};

/**
 * §9.5 "Open", step 1: waits until the session the frame showed before `session` has saved. That session stays
 * active until its page's last flush is done (`releaseBoard`), so this also waits for a PUT still running when the
 * same board is opened again: the GET that follows then sees that save.
 */
export async function flushPrevious(session: BoardSession): Promise<void> {
  const prev = active;
  if (!prev || prev === session) return;
  try {
    await prev.flush();
  } catch (e) {
    console.warn('saving the previous board failed', e);
  }
}

/**
 * Opens a board in the frame for `session`: first flushes the session of the board shown before (so its last
 * changes are saved before `setBoard` replaces them), then makes `session` the current one and calls `frame.open`.
 */
export async function openBoard(session: BoardSession, args: OpenArgs): Promise<void> {
  await flushPrevious(session);
  const frame = useHostStore.getState().frame;
  if (!frame) throw new Error('the board frame is not connected');
  active = session;
  current = args.itemId;
  await openInFrame(frame, args);
}

/** The page of `session` is gone: its events stop (its last flush is the page's job). */
export function releaseBoard(session: BoardSession): void {
  if (active === session) active = null;
}

/** Gives the host the element the iframe covers; the returned function releases it. */
export function attachSurface(surface: HTMLElement): () => void {
  useHostStore.setState({ surface });
  return () => {
    if (useHostStore.getState().surface === surface) useHostStore.setState({ surface: null, shown: false });
  };
}

/** Whether the iframe may be shown over the surface (the current board is open). */
export function showBoard(shown: boolean): void {
  useHostStore.setState({ shown });
}

/** Records a successful save of `itemId`. */
export function noteSaved(itemId: string, at: number = Date.now()): void {
  useHostStore.setState((s) => ({ savedAt: { ...s.savedAt, [itemId]: at } }));
}

/** Forgets the frame (its iframe was removed, or is about to be recreated); keeps `mounted` and the surface. */
export function resetFrame(): void {
  current = null;
  active = null;
  dropLayout();
  useHostStore.setState({ iframe: null, frame: null, failure: null, progress: null, shown: false });
}

/** Back to the initial state (tests, and BoardFrameHost's unmount). */
export function resetHost(): void {
  current = null;
  active = null;
  dropLayout();
  itemsByBoard.clear();
  useHostStore.setState({ ...initial });
}
