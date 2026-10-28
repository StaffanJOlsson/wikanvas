// The "Text doc" documents a board page created (docs/ARCHITECTURE.md §18.6 (a) step 4, §18.9): when the page leaves
// the board, each one that is on no card of the board's last layout is flushed and sent `DELETE …?discard_empty=true`.
// The server decides: it keeps (409 `not_empty`, ignored) an item with a title, content or children, and one that any
// board's card still targets (an `embed` link), so a card on another board, or a removal that never reached the
// server, keeps its document. The board's own pending save goes first, so a removed card's link is gone by then.
import type { QueryClient } from '@tanstack/react-query';
import { api } from '../../api/client';
import { flushPending } from '../../state/pending';

/** Flushes and discards one created document (the server keeps it when it is not empty or still embedded). */
export async function discardTextDoc(qc: QueryClient, board: { id: string; collectionId: string }, docId: string): Promise<void> {
  await flushPending(docId);
  try {
    await api('DELETE', `/items/${encodeURIComponent(docId)}?discard_empty=true`);
  } catch {
    return; // 409 not_empty (or gone already): kept
  }
  void qc.invalidateQueries({ queryKey: ['tree', board.collectionId] });
  void qc.invalidateQueries({ queryKey: ['sidebar'] });
  void qc.invalidateQueries({ queryKey: ['children', board.id] });
  void qc.invalidateQueries({ queryKey: ['item', board.id] });
}

/**
 * The page left the board: every document of `created` that is not in `items` (the board's last layout; null when
 * none came) is discarded. Resolves when all requests settled.
 */
export async function discardTextDocs(
  qc: QueryClient,
  board: { id: string; collectionId: string },
  created: readonly string[],
  items: readonly string[] | null,
): Promise<void> {
  const unused = created.filter((id) => !(items ?? []).includes(id));
  if (unused.length === 0) return;
  await flushPending(board.id);
  await Promise.all(unused.map((id) => discardTextDoc(qc, board, id)));
}
