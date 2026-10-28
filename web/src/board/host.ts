// The board frame host's state, outside React (docs/ARCHITECTURE.md §9.5, D37: BoardFrameHost is mounted beside the
// routes, so pages reach it through an external store, not a React context).
//
// - `useHostStore` holds what pages render from: the connected frame, a failure, the download progress, the update
//   banner, the surface the iframe covers, and whether the current board is shown.
// - `frameEvents` are the FrameEvents the host connects with. Each event carries an itemId; events for any board but
//   the current one are ignored (§10.4: "the host ignores events whose id is not current"). Host keys go to
//   `runHostShortcut`.
// - `openBoard(session, args)` flushes the previous board's session before it opens the next one (§9.5 "Open").
import { create } from 'zustand';
import { runHostShortcut } from '../app/hotkeys';
import type { BoardFrameApi, FrameError, FrameEvents, OpenArgs } from './bridge';

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
  /** `frame.html` answered 404 and `/config` names another build: show "Quire was updated. Reload". */
  updateAvailable: boolean;
  /** The BoardSurface element the iframe covers. */
  surface: HTMLElement | null;
  /** The current board is open in the frame, so the iframe may be shown over the surface. */
  shown: boolean;
  /** When each board was last saved (ms), for "Changes up to HH:MM were saved" after a panic. */
  savedAt: Readonly<Record<string, number>>;
}

const initial: HostState = {
  mounted: false,
  iframe: null,
  frame: null,
  failure: null,
  progress: null,
  updateAvailable: false,
  surface: null,
  shown: false,
  savedAt: {},
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
};

/**
 * Opens a board in the frame for `session`: first flushes the session of the board shown before (so its last
 * changes are saved before `setBoard` replaces them), then makes `session` the current one and calls `frame.open`.
 */
export async function openBoard(session: BoardSession, args: OpenArgs): Promise<void> {
  const prev = active;
  if (prev && prev !== session) await prev.flush();
  const frame = useHostStore.getState().frame;
  if (!frame) throw new Error('the board frame is not connected');
  active = session;
  current = args.itemId;
  await frame.open(args);
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
  useHostStore.setState({ iframe: null, frame: null, failure: null, progress: null, shown: false });
}

/** Back to the initial state (tests, and BoardFrameHost's unmount). */
export function resetHost(): void {
  current = null;
  active = null;
  useHostStore.setState({ ...initial });
}
