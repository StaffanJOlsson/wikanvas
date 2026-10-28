// The read-only board (docs/ARCHITECTURE.md §9.5 "Read-only fallback"): the server's SVG page previews
// (`boardPreviewUrl(id, rev, n)`) in an <img>, with page tabs, drag to pan, wheel or pinch to zoom, and Fit. It is
// shown when the editor cannot run (fallback.ts), and by default on phones, with an "Open editor" button. After a
// panic it says up to when the changes were saved and offers a reload that recreates the frame.
import { useCallback, useEffect, useRef, useState, type PointerEvent, type WheelEvent } from 'react';
import type { ItemDetail } from '../api/gen/ItemDetail';
import { boardPreviewUrl } from '../lib/urls';
import type { FallbackReason } from './fallback';
import { snapshotTime } from './unsaved';

export interface BoardPreviewProps {
  item: ItemDetail;
  reason: FallbackReason;
  /** Phones: open the editor anyway. */
  onOpenEditor?(): void;
  /** After a panic: recreate the frame. */
  onReload?(): void;
  /** When the board was last saved (ms), for the panic message. */
  savedAt?: number | null;
}

interface View {
  x: number;
  y: number;
  scale: number;
}

const MIN_SCALE = 0.1;
const MAX_SCALE = 8;
const clamp = (s: number) => Math.min(MAX_SCALE, Math.max(MIN_SCALE, s));

export function BoardPreview({ item, reason, onOpenEditor, onReload, savedAt }: BoardPreviewProps) {
  const board = item.board;
  const pages = board?.pages ?? [];
  const [page, setPage] = useState(0);
  const [view, setView] = useState<View>({ x: 0, y: 0, scale: 1 });
  const [empty, setEmpty] = useState(false);
  const viewport = useRef<HTMLDivElement>(null);
  const img = useRef<HTMLImageElement>(null);
  const pointers = useRef(new Map<number, { x: number; y: number }>());
  const pinch = useRef<number | null>(null);

  const fit = useCallback(() => {
    const vp = viewport.current;
    const im = img.current;
    if (!vp || !im || !im.naturalWidth || !im.naturalHeight) {
      setView({ x: 0, y: 0, scale: 1 });
      return;
    }
    const scale = clamp(Math.min(vp.clientWidth / im.naturalWidth, vp.clientHeight / im.naturalHeight));
    setView({
      x: (vp.clientWidth - im.naturalWidth * scale) / 2,
      y: (vp.clientHeight - im.naturalHeight * scale) / 2,
      scale,
    });
  }, []);

  useEffect(() => {
    setEmpty(false);
    setView({ x: 0, y: 0, scale: 1 });
  }, [page, board?.rev]);

  const zoomAt = (cx: number, cy: number, factor: number) =>
    setView((v) => {
      const scale = clamp(v.scale * factor);
      const k = scale / v.scale;
      return { scale, x: cx - (cx - v.x) * k, y: cy - (cy - v.y) * k };
    });

  const onWheel = (e: WheelEvent<HTMLDivElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    zoomAt(e.clientX - r.left, e.clientY - r.top, Math.exp(-e.deltaY * 0.0015));
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    e.currentTarget.setPointerCapture?.(e.pointerId);
    pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const ps = pointers.current;
    const prev = ps.get(e.pointerId);
    if (!prev) return;
    ps.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (ps.size === 1) {
      setView((v) => ({ ...v, x: v.x + e.clientX - prev.x, y: v.y + e.clientY - prev.y }));
    } else if (ps.size === 2) {
      const [a, b] = [...ps.values()];
      const d = Math.hypot(a.x - b.x, a.y - b.y);
      if (pinch.current !== null && pinch.current > 0) {
        const r = e.currentTarget.getBoundingClientRect();
        zoomAt((a.x + b.x) / 2 - r.left, (a.y + b.y) / 2 - r.top, d / pinch.current);
      }
      pinch.current = d;
    }
  };
  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    pointers.current.delete(e.pointerId);
    if (pointers.current.size < 2) pinch.current = null;
  };

  const panicSaved = savedAt ?? (item.item.content_at ? Date.parse(item.item.content_at) : null);

  return (
    <section className="board-preview" aria-label="Board preview">
      <div className="board-preview-bar">
        <span className="board-preview-reason" role="status">
          {reason.kind === 'panic'
            ? `The whiteboard engine stopped.${panicSaved ? ` Changes up to ${snapshotTime(panicSaved)} were saved.` : ''}`
            : reason.message}
        </span>
        {reason.kind === 'phone' && onOpenEditor && (
          <button type="button" onClick={onOpenEditor}>
            Open editor
          </button>
        )}
        {reason.kind === 'panic' && onReload && (
          <button type="button" onClick={onReload}>
            Reload the editor
          </button>
        )}
        {pages.length > 1 && (
          <div role="tablist" aria-label="Pages">
            {pages.map((p, i) => (
              <button
                key={p.id}
                type="button"
                role="tab"
                aria-selected={i === page}
                onClick={() => setPage(i)}
              >
                {p.name || `Page ${i + 1}`}
              </button>
            ))}
          </div>
        )}
        <button type="button" onClick={fit}>
          Fit
        </button>
      </div>
      <div
        ref={viewport}
        className="board-preview-viewport"
        onWheel={onWheel}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
      >
        {board && pages.length > 0 && !empty ? (
          <img
            ref={img}
            src={boardPreviewUrl(item.item.id, board.rev, page)}
            alt={board.text_excerpt || item.item.title || 'Board'}
            draggable={false}
            onLoad={fit}
            onError={() => setEmpty(true)}
            style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.scale})` }}
          />
        ) : (
          <div className="board-preview-empty">This page is empty.</div>
        )}
      </div>
    </section>
  );
}
