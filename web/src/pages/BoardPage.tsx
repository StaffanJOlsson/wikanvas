// The board page, lazily loaded at /board/:slugId (docs/ARCHITECTURE.md §9.1, §9.3, §9.5, §9.6.4): the top bar with
// the editable title crumb, the star, the Nested popover, the Focus (full screen) button and the item menu (exports:
// .inkboard and SVG), then either BoardSurface over the frame host or the read-only BoardPreview (fallback.ts).
import { FileText, ListTree, Maximize2, Presentation } from 'lucide-react';
import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';
import { Link, useLocation, useNavigate, useParams } from 'react-router';
import { api, API_BASE, isApiError } from '../api/client';
import type { ItemDetail } from '../api/gen/ItemDetail';
import { boardDownloadUrl, useChildren } from '../api/queries/boards';
import { useConfig, useCreateItem, useItem, useRecordView } from '../api/queries/items';
import { BoardPreview } from '../board/BoardPreview';
import { BoardSurface } from '../board/BoardSurface';
import { useBoardFrame, type BoardFrameContext } from '../board/BoardFrameHost';
import { fallbackReason } from '../board/fallback';
import { PHONE_QUERY, useMediaQuery } from '../board/theme';
import { ItemMenu, type ExtraExport } from '../components/item/ItemMenu';
import { StarToggle } from '../components/item/StarToggle';
import { TopBar } from '../components/topbar/TopBar';
import { itemPath, slugify, urlIdFromSlugId } from '../lib/urls';
import { useCanonicalPath } from '../lib/useCanonicalPath';
import { flushPending } from '../state/pending';
import NotFound from './NotFound';
import { PlaceholderPage } from './PlaceholderPage';
import '../board/board.css';

export default function BoardPage() {
  const { slugId } = useParams();
  const ref = urlIdFromSlugId(slugId);
  const item = useItem(ref);
  useCanonicalPath(item.data ? itemPath(item.data.item) : null);

  if (ref === null || isApiError(item.error, 'not_found')) return <NotFound />;
  if (item.isPending) return null;
  if (item.isError) return <PlaceholderPage title="Board">{item.error.message}</PlaceholderPage>;
  if (item.data.item.kind !== 'board') return <NotFound />;
  return <BoardView item={item.data} />;
}

/** Clicks a temporary download link. */
function download(href: string, filename?: string): void {
  const a = document.createElement('a');
  a.href = href;
  if (filename) a.download = filename;
  a.rel = 'noopener';
  document.body.appendChild(a);
  a.click();
  a.remove();
}

/** The board page body for a loaded item (the editor or the preview). */
export function BoardView({ item }: { item: ItemDetail }) {
  const host = useBoardFrame();
  const config = useConfig();
  const phone = useMediaQuery(PHONE_QUERY);
  const [editorOnPhone, setEditorOnPhone] = useState(false);
  const id = item.item.id;
  const qc = useQueryClient();
  const location = useLocation();
  useRecordView(id);
  // As on the document page (§9.6.4): only a board this page session created (navigated to with
  // `{ state: { created: true } }`) is discarded when it is left untitled and empty. BoardView stays mounted from one
  // board to the next, so the flag is taken once per board, before the canonical-path replace drops the state.
  const created = useRef<{ id: string; created: boolean } | null>(null);
  if (created.current?.id !== id) {
    created.current = { id, created: (location.state as { created?: boolean } | null)?.created === true };
  }
  const { collection_id: collectionId, parent_id: parentId } = item.item;
  useEffect(() => {
    const discard = created.current?.id === id && created.current.created;
    return () => {
      if (!discard) return;
      // The save loop and the title input save first; the server decides (409 `not_empty` once the board has a
      // title, an object or a child, which is ignored).
      void flushPending(id).then(() =>
        api('DELETE', `/items/${encodeURIComponent(id)}?discard_empty=true`).then(
          () => {
            void qc.invalidateQueries({ queryKey: ['tree', collectionId] });
            void qc.invalidateQueries({ queryKey: ['sidebar'] });
            if (parentId) void qc.invalidateQueries({ queryKey: ['children', parentId] });
          },
          () => undefined,
        ),
      );
    };
  }, [id, qc, collectionId, parentId]);
  const reason = fallbackReason({
    config: config.isError ? null : config.data,
    failure: host?.failure ?? null,
    hostless: host === null,
    phone: phone && !editorOnPhone,
  });

  const exports: ExtraExport[] = [
    {
      label: '.inkboard',
      run: async () => {
        await flushPending(id);
        download(boardDownloadUrl(id));
      },
    },
    {
      label: 'SVG',
      run: async () => {
        await flushPending(id);
        download(`${API_BASE}/items/${encodeURIComponent(id)}/board/pages/0`, `${slugify(item.item.title)}.svg`);
      },
    },
  ];

  return (
    <article className="board-page">
      <TopBar
        item={item}
        titleCrumb="editable"
        actions={
          <>
            <StarToggle item={item} />
            <NestedPopover item={item} />
            <FocusButton host={host} disabled={reason !== null} />
            <ItemMenu item={item} extraExports={exports} />
          </>
        }
      />
      {reason !== null || host === null ? (
        <BoardPreview
          item={item}
          reason={reason ?? { kind: 'no-engine', message: 'Whiteboard engine not installed' }}
          onOpenEditor={() => setEditorOnPhone(true)}
          onReload={() => host?.reload()}
          savedAt={host?.savedAt[id] ?? null}
        />
      ) : (
        <BoardSurface item={item} host={host} />
      )}
    </article>
  );
}

/** Focus: the board full screen (the iframe's own fullscreen, `allow="fullscreen"`). */
function FocusButton({ host, disabled }: { host: BoardFrameContext | null; disabled: boolean }) {
  return (
    <button
      type="button"
      className="icon-button"
      aria-label="Focus"
      title="Focus (full screen)"
      disabled={disabled || !host?.frame}
      onClick={() => void host?.requestFullscreen()}
    >
      <Maximize2 size={16} aria-hidden />
    </button>
  );
}

/** Nested: the board's child pages, and "+ Add a page" (a child document). */
function NestedPopover({ item }: { item: ItemDetail }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const children = useChildren(item.item.id, open);
  const create = useCreateItem();
  const navigate = useNavigate();

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (root.current && !root.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);

  const count = item.item.child_count;
  return (
    <div className="board-nested" ref={root}>
      <button
        type="button"
        className="icon-button"
        aria-label="Nested pages"
        title="Nested pages"
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <ListTree size={16} aria-hidden />
        {count > 0 && <span>{count}</span>}
      </button>
      {open && (
        <div role="dialog" aria-label="Nested pages">
          {children.data && children.data.length > 0 ? (
            <ul>
              {children.data.map((c) => (
                <li key={c.id}>
                  <Link to={itemPath(c)} onClick={() => setOpen(false)}>
                    {c.kind === 'board' ? <Presentation size={16} aria-hidden /> : <FileText size={16} aria-hidden />}{' '}
                    {c.title || 'Untitled'}
                  </Link>
                </li>
              ))}
            </ul>
          ) : (
            <p className="empty">{children.isPending ? 'Loading…' : 'No nested pages'}</p>
          )}
          <button
            type="button"
            className="add-page"
            disabled={create.isPending}
            onClick={() =>
              create.mutate(
                { collection_id: item.item.collection_id, parent_id: item.item.id, kind: 'doc' },
                { onSuccess: (c) => void navigate(itemPath(c), { state: { created: true } }) },
              )
            }
          >
            + Add a page
          </button>
        </div>
      )}
    </div>
  );
}
