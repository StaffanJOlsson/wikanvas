// The document header (docs/ARCHITECTURE.md §9.3, §9.6.4): title input, star toggle and meta line. Props are frozen
// (PLAN W0b); W3 completes the behaviour.
import { useEffect, useState } from 'react';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import { relativeTime } from '../../lib/time';
import { StarToggle } from './StarToggle';
import { useTitleAutosave } from './useTitleAutosave';

export interface ItemHeaderProps {
  item: ItemDetail;
  /** Enter in the title: the editor focuses the start of the body. */
  onEnterBody(): void;
  /** Trashed or archived: the title cannot be edited. */
  readOnly: boolean;
}

/** The title input's id: the editor focuses it on Backspace at the start of an empty body (§9.6.4). */
export const TITLE_INPUT_ID = 'item-title';

/** "You created <rel>" while the body was never saved, else "You updated <rel>" (no publication state, D23). */
export function metaLine(item: ItemDetail, now: number = Date.now()): string {
  const s = item.item;
  return s.content_at === null
    ? `You created ${relativeTime(s.created_at, now)}`
    : `You updated ${relativeTime(s.updated_at, now)}`;
}

function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(t);
  }, [intervalMs]);
  return now;
}

export function ItemHeader({ item, onEnterBody, readOnly }: ItemHeaderProps) {
  const { title, setTitle, flush } = useTitleAutosave(item);
  const now = useNow(60_000);
  return (
    <header className="item-header">
      <input
        id={TITLE_INPUT_ID}
        className="title-input"
        aria-label="Title"
        placeholder="Untitled"
        value={title}
        readOnly={readOnly}
        onChange={(e) => setTitle(e.target.value)}
        onBlur={() => void flush()}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
            e.preventDefault();
            onEnterBody();
          }
        }}
      />
      <div className="meta-line">
        <StarToggle item={item} />
        <span>{metaLine(item, now)}</span>
      </div>
    </header>
  );
}
