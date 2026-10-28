// Hover hooks the sidebar calls for rows (docs/ARCHITECTURE.md §9.5): hovering a board row for 300 ms warms the
// engine's cache. The board host (W6) registers the warm-up with `onBoardRowHover`; without a registration nothing
// happens.

export const BOARD_HOVER_MS = 300;

type Handler = (itemId: string) => void;

const handlers = new Set<Handler>();

/** Registers a handler for "a board row was hovered for 300 ms"; returns the function that removes it. */
export function onBoardRowHover(handler: Handler): () => void {
  handlers.add(handler);
  return () => {
    handlers.delete(handler);
  };
}

/** Called by the sidebar once a board row has been hovered for BOARD_HOVER_MS. */
export function boardRowHovered(itemId: string): void {
  for (const h of handlers) {
    try {
      h(itemId);
    } catch (e) {
      console.warn('board hover handler failed', e);
    }
  }
}
