// The "Text doc" documents a board page created (docs/ARCHITECTURE.md §18.6 (a) step 4, §18.9): when the page leaves
// the board, each one that is on no card of the board's last layout is flushed and sent `DELETE …?discard_empty=true`.
// The server decides: it keeps (409 `not_empty`, ignored) an item with a title, content or children, and one that any
// board's card still targets (an `embed` link), so a card on another board, or a removal that never reached the
// server, keeps its document. The board's own pending save goes first, so a removed card's link is gone by then.
// A card on the engine's clipboard (cut, or copied and then deleted) has no link yet but can still be pasted, here or
// on the next board: its document is not discarded then, but kept for a later leave, which discards it once no
// clipboard holds it and no card of the board being left shows it. A discarded document's cached detail and content
// are reset, so a card that still names it shows "Page not found".
import type { QueryClient } from '@tanstack/react-query';
import { api } from '../../api/client';
import { docJsonKey } from '../../api/queries/docs';
import { itemKey } from '../../api/queries/items';
import { flushPending } from '../../state/pending';

/** The board a created document belongs to (its parent). */
export interface TextDocBoard {
  id: string;
  collectionId: string;
}

/** Documents kept at a leave because a card of theirs was on the engine's clipboard, with the board that made them. */
const deferred = new Map<string, TextDocBoard>();

/** Flushes and discards one created document (the server keeps it when it is not empty or still embedded). */
export async function discardTextDoc(qc: QueryClient, board: TextDocBoard, docId: string): Promise<void> {
  await flushPending(docId);
  try {
    await api('DELETE', `/items/${encodeURIComponent(docId)}?discard_empty=true`);
  } catch {
    return; // 409 not_empty (or gone already): kept
  }
  // Gone for good: nothing shows the detail or the content any longer (a card of it refetches, gets a 404).
  void qc.resetQueries({ queryKey: itemKey(docId), exact: true });
  void qc.resetQueries({ queryKey: docJsonKey(docId), exact: true });
  void qc.invalidateQueries({ queryKey: ['tree', board.collectionId] });
  void qc.invalidateQueries({ queryKey: ['sidebar'] });
  void qc.invalidateQueries({ queryKey: ['children', board.id] });
  void qc.invalidateQueries({ queryKey: ['item', board.id] });
}

/**
 * The page left `board`: every document of `created`, and every document kept by an earlier leave, that is not in
 * `items` (the board's last layout; null when none came) is discarded, unless `clipboard` (the targets of the cards on
 * the engine's clipboard; null when no engine reported it) holds it: such a one is kept for a later leave. Resolves
 * when all requests settled.
 */
export async function discardTextDocs(
  qc: QueryClient,
  board: TextDocBoard,
  created: readonly string[],
  items: readonly string[] | null,
  clipboard: readonly string[] | null = null,
): Promise<void> {
  const candidates = new Map<string, TextDocBoard>(deferred);
  for (const id of created) candidates.set(id, board);
  const unused: [string, TextDocBoard][] = [];
  for (const [id, b] of candidates) {
    if ((items ?? []).includes(id)) deferred.delete(id);
    else if ((clipboard ?? []).includes(id)) deferred.set(id, b);
    else {
      deferred.delete(id);
      unused.push([id, b]);
    }
  }
  if (unused.length === 0) return;
  await flushPending(board.id);
  await Promise.all(unused.map(([id, b]) => discardTextDoc(qc, b, id)));
}

/** The documents kept for a later leave (tests). */
export function deferredTextDocs(): string[] {
  return [...deferred.keys()];
}

/** Forgets the documents kept for a later leave (tests). */
export function resetDeferredTextDocs(): void {
  deferred.clear();
}
