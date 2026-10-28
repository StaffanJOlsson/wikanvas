// The board frame host (docs/ARCHITECTURE.md §9.3, §9.5): one persistent iframe of the engine's frame page, laid over
// the current BoardSurface. AppShell mounts it once, next to the routes.
//
// PLACEHOLDER (W0b): renders nothing and `useBoardFrame()` returns null, so board pages show BoardPreview. W6
// replaces the body (iframe creation, rect tracking, hidden + inert + setActive(false) off board routes, removal after
// 10 minutes, the update banner on a frame.html 404, preview mode when `config.inkboard` is null). The default export
// (no props) and `useBoardFrame(): BoardFrameContext | null` are frozen (PLAN W0b); BoardFrameContext is W6's to extend.
import type { BoardFrameApi, FrameError } from './bridge';

/** What board pages get from the host. */
export interface BoardFrameContext {
  /** The connected frame, or null while it loads (or when it failed: see `failure`). */
  readonly frame: BoardFrameApi | null;
  /** Why boards open in BoardPreview instead (no engine, unsupported browser, panic, …), or null. */
  readonly failure: Pick<FrameError, 'kind' | 'message'> | null;
  /**
   * BoardSurface hands over the element the iframe covers (its rect is tracked); the returned function releases it.
   */
  attach(surface: HTMLElement): () => void;
}

/** The board frame context, or null when no frame host is active (always null in this placeholder). */
export function useBoardFrame(): BoardFrameContext | null {
  return null;
}

export default function BoardFrameHost(): null {
  return null;
}
