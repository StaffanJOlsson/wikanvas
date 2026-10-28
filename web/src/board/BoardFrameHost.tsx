// The board frame host (docs/ARCHITECTURE.md §9.3, §9.5, D11): one persistent iframe of the engine's frame page,
// laid over the current BoardSurface. AppShell mounts it once, next to the routes.
// - The iframe is created on the first board visit that wants the editor (a BoardSurface attached), from
//   `config.inkboard.base + 'frame.html'`; with `config.inkboard === null` there is none and boards open in BoardPreview.
// - It is never moved in the DOM (that would reload it): it sits in a fixed layer and is positioned over the
//   surface's rect, which is tracked on resize and scroll.
// - Off board routes it gets `visibility: hidden`, `inert` and `setActive(false)`; back on a board route
//   `setActive(true)`. It is removed after 10 minutes away from boards.
// - If `frame.html` turns out to be missing (404), the shell's update check runs (`checkForUpdate`, W3): when the
//   server is another build, the one "Quire was updated. Reload" banner shows (AppShell's UpdateBanner).
// - Quire's theme is passed on with `setTheme`; host keys from the frame go to `runHostShortcut` (host.ts).
// - While mounted it registers the engine warm-up of a 300 ms sidebar board-row hover (`useBoardRowWarmup`).
// - M1.1 (§18.5): the layer also holds the board embeds overlay (`EmbedLayer`), after the iframe in DOM order and
//   placed by the same routine to the same rect. Focus fullscreens the layer (iframe and overlay); while the layer is
//   the fullscreen element both are placed at its rect.
// The default export (no props) and `useBoardFrame(): BoardFrameContext | null` are the frozen contract (PLAN W0b).
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef } from 'react';
import { useLocation } from 'react-router';
import { useShallow } from 'zustand/react/shallow';
import { useConfig } from '../api/queries/config';
import { checkForUpdate, useUpdate } from '../app/updateCheck';
import { inkboardBase } from '../lib/urls';
import type { BoardFrameApi, OpenArgs } from './bridge';
import { EmbedLayer } from './embed/EmbedLayer';
import { frameDriver, toConnectFailure } from './frameDriver';
import {
  attachSurface,
  frameEvents,
  openBoard,
  releaseBoard,
  resetFrame,
  resetHost,
  showBoard,
  useHostStore,
  type BoardSession,
  type Failure,
} from './host';
import { useResolvedTheme } from './theme';
import { useBoardRowWarmup } from './warmup';
import './board.css';

/** How long the iframe is kept while no board route is open. */
export const IDLE_REMOVE_MS = 10 * 60 * 1000;

/** What board pages get from the host. */
export interface BoardFrameContext {
  /** The connected frame, or null while it loads (or when it failed: see `failure`). */
  readonly frame: BoardFrameApi | null;
  /** Why boards open in BoardPreview instead (no engine, unsupported browser, panic, …), or null. */
  readonly failure: Failure | null;
  /**
   * BoardSurface hands over the element the iframe covers (its rect is tracked); the returned function releases it.
   */
  attach(surface: HTMLElement): () => void;
  // Additions (W6, PLAN R3):
  /** The engine download, while it runs (`onProgress`). */
  readonly progress: { loaded: number; total: number } | null;
  /** "Quire was updated. Reload" is showing (the shell's banner, `useUpdate`). */
  readonly updateAvailable: boolean;
  /** When each board was last saved (ms). */
  readonly savedAt: Readonly<Record<string, number>>;
  /** Flushes the previous board's session, then opens `args` for `session` (host.ts `openBoard`). */
  open(session: BoardSession, args: OpenArgs): Promise<void>;
  /** The session's page is gone. */
  release(session: BoardSession): void;
  /** The current board is (not) ready to be shown over the surface. */
  show(shown: boolean): void;
  /** Removes the iframe so the next board visit creates a new one (after a panic). */
  reload(): void;
  /** Shows the board full screen (Focus): the frame layer, so the embeds overlay is shown with the iframe. */
  requestFullscreen(): Promise<void>;
}

/** The board frame context, or null when no frame host is mounted. */
export function useBoardFrame(): BoardFrameContext | null {
  const s = useHostStore(
    useShallow((st) => ({
      mounted: st.mounted,
      frame: st.frame,
      failure: st.failure,
      progress: st.progress,
      savedAt: st.savedAt,
    })),
  );
  const updateAvailable = useUpdate((u) => u.available);
  return useMemo<BoardFrameContext | null>(() => {
    if (!s.mounted) return null;
    return {
      frame: s.frame,
      failure: s.failure,
      progress: s.progress,
      updateAvailable,
      savedAt: s.savedAt,
      attach: attachSurface,
      open: openBoard,
      release: releaseBoard,
      show: showBoard,
      reload: removeFrame,
      requestFullscreen: async () => {
        await useHostStore.getState().layer?.requestFullscreen?.();
      },
    };
  }, [s, updateAvailable]);
}

/** Removes the iframe and forgets the frame. */
function removeFrame(): void {
  useHostStore.getState().iframe?.remove();
  resetFrame();
}

/** After a connection without `quireBoardFrame`: whether `frame.html` is gone (404). */
async function frameGone(src: string): Promise<boolean> {
  try {
    const res = await fetch(src, { cache: 'no-store', credentials: 'same-origin' });
    return res.status === 404;
  } catch {
    return false;
  }
}

export default function BoardFrameHost() {
  const location = useLocation();
  const onBoard = location.pathname.startsWith('/board/');
  const config = useConfig();
  const theme = useResolvedTheme();
  const layer = useRef<HTMLDivElement>(null);
  const overlay = useRef<HTMLDivElement | null>(null);
  const overlayRef = useCallback((el: HTMLDivElement | null) => {
    overlay.current = el;
  }, []);
  const st = useHostStore(
    useShallow((s) => ({
      iframe: s.iframe,
      frame: s.frame,
      failure: s.failure,
      surface: s.surface,
      shown: s.shown,
    })),
  );
  const base = inkboardBase(config.data);
  // Hovering a sidebar board row for 300 ms warms the engine's cache (§9.5).
  useBoardRowWarmup();

  useEffect(() => {
    useHostStore.setState({ mounted: true, layer: layer.current });
    return () => {
      useHostStore.getState().iframe?.remove();
      resetHost();
    };
  }, []);

  // Create the iframe on the first board visit that wants the editor.
  const wanted = onBoard && st.surface !== null && base !== null && st.failure === null && st.iframe === null;
  useEffect(() => {
    if (!wanted || base === null || !layer.current) return;
    const src = `${base}frame.html`;
    const driver = frameDriver();
    const iframe = driver.create(src);
    iframe.style.visibility = 'hidden';
    iframe.setAttribute('inert', '');
    useHostStore.setState({ iframe, failure: null, progress: null });
    driver.connect(iframe, frameEvents).then(
      (frame) => {
        if (useHostStore.getState().iframe === iframe) useHostStore.setState({ frame });
      },
      async (e: unknown) => {
        const failure = toConnectFailure(e);
        if (useHostStore.getState().iframe !== iframe) return;
        useHostStore.setState((s) => ({ failure: s.failure ?? { kind: failure.kind, message: failure.message } }));
        // The shell's update check compares /config with the config this page started with (§9.1).
        if (failure.missing && (await frameGone(src))) void checkForUpdate();
      },
    );
    // Before the overlay, which paints above it (§18.5).
    layer.current.insertBefore(iframe, layer.current.firstChild);
  }, [wanted, base]);

  // Shown over the surface only on a board route, with the board open; hidden and inert otherwise.
  const visible = onBoard && st.surface !== null && st.shown && st.frame !== null && st.failure === null;
  useLayoutEffect(() => {
    const iframe = st.iframe;
    if (!iframe) return;
    iframe.style.visibility = visible ? 'visible' : 'hidden';
    if (visible) iframe.removeAttribute('inert');
    else iframe.setAttribute('inert', '');
  }, [st.iframe, visible]);

  // Paused off board routes (no periodic repaint).
  useEffect(() => {
    st.frame?.setActive(onBoard);
  }, [st.frame, onBoard]);

  // Removed after 10 minutes away from boards.
  useEffect(() => {
    if (onBoard || st.iframe === null) return;
    const timer = setTimeout(removeFrame, IDLE_REMOVE_MS);
    return () => clearTimeout(timer);
  }, [onBoard, st.iframe]);

  // The frame follows Quire's theme.
  useEffect(() => {
    st.frame?.setTheme(theme);
  }, [st.frame, theme]);

  // Track the surface's rect (the layer's while the layer is full screen); the overlay gets the same rect.
  const { iframe: trackedFrame, surface: trackedSurface } = st;
  useLayoutEffect(() => {
    const iframe = trackedFrame;
    const surface = trackedSurface;
    if (!iframe || !surface) return;
    let raf = 0;
    const place = () => {
      raf = 0;
      const l = layer.current;
      const full = l !== null && document.fullscreenElement === l;
      const r = (full ? l : surface).getBoundingClientRect();
      for (const el of [iframe, overlay.current]) {
        if (!el) continue;
        el.style.left = `${r.left}px`;
        el.style.top = `${r.top}px`;
        el.style.width = `${r.width}px`;
        el.style.height = `${r.height}px`;
      }
    };
    const schedule = () => {
      if (raf === 0) raf = requestAnimationFrame(place);
    };
    place();
    const ro = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(schedule);
    ro?.observe(surface);
    window.addEventListener('resize', schedule);
    window.addEventListener('scroll', schedule, true);
    document.addEventListener('fullscreenchange', schedule);
    return () => {
      if (raf !== 0) cancelAnimationFrame(raf);
      ro?.disconnect();
      window.removeEventListener('resize', schedule);
      window.removeEventListener('scroll', schedule, true);
      document.removeEventListener('fullscreenchange', schedule);
    };
  }, [trackedFrame, trackedSurface]);

  return (
    <div ref={layer} className="board-frame-layer">
      <EmbedLayer visible={visible} overlayRef={overlayRef} />
    </div>
  );
}
