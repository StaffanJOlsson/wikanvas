// While the engine loads and the board opens (docs/ARCHITECTURE.md §9.5 "Open"): the page-0 SVG at 50 % opacity and a
// progress card fed by `onProgress`. When the engine comes from the cache there is no download to show, so the card
// appears only after 300 ms.
import { useEffect, useState } from 'react';
import type { ItemDetail } from '../api/gen/ItemDetail';
import { boardPreviewUrl } from '../lib/urls';

/** How long an open from the cache may take before the progress card shows. */
export const CARD_DELAY_MS = 300;

export interface BoardLoadingProps {
  item: ItemDetail;
  /** The engine download, while it runs. */
  progress: { loaded: number; total: number } | null;
}

export function BoardLoading({ item, progress }: BoardLoadingProps) {
  const [late, setLate] = useState(false);
  useEffect(() => {
    const t = setTimeout(() => setLate(true), CARD_DELAY_MS);
    return () => clearTimeout(t);
  }, []);
  const board = item.board;
  const downloading = progress !== null && progress.total > 0 && progress.loaded < progress.total;
  const pct = downloading ? Math.floor((progress.loaded / progress.total) * 100) : null;
  return (
    <div className="board-loading" aria-busy="true">
      {board && board.pages.length > 0 && (
        <img className="board-loading-svg" src={boardPreviewUrl(item.item.id, board.rev, 0)} alt="" />
      )}
      {(late || downloading) && (
        <div className="board-loading-card" role="status">
          {pct === null ? 'Opening the whiteboard…' : `Loading the whiteboard engine… ${pct}%`}
          {pct !== null && <progress max={100} value={pct} aria-label="Engine download" />}
        </div>
      )}
    </div>
  );
}
