// The document page, lazily loaded at /doc/:slugId (docs/ARCHITECTURE.md §9.1, §9.3, §9.4, §9.6.4, §9.6.5): the top
// bar with the Contents popover and the item menu (Markdown and HTML exports), the Contents column (≥2 headings,
// docked from 1280 px), the item header, the editor on its doc session, the read-only banner for trashed and archived
// items, the nested-items list, and the new-empty-item discard on leave (D23).
import { useQueryClient } from '@tanstack/react-query';
import type { Editor } from '@tiptap/react';
import { FileText, ListTree, PenTool, Plus } from 'lucide-react';
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { Link, useLocation, useNavigate, useParams } from 'react-router';
import { api, isApiError } from '../api/client';
import { searchTitles, useChildren } from '../api/queries/docs';
import { useCreateItem, useItem } from '../api/queries/items';
import type { ItemDetail } from '../api/gen/ItemDetail';
import type { TitleHit } from '../api/gen/TitleHit';
import { ItemHeader } from '../components/item/ItemHeader';
import { ItemMenu } from '../components/item/ItemMenu';
import { TopBar } from '../components/topbar/TopBar';
import { activeHeading, headingAnchor, showContents, type Heading } from '../editor/contents';
import { DocSession } from '../editor/docSession';
import { docExports } from '../editor/exports';
import { QuireEditor } from '../editor/QuireEditor';
import { relativeTime } from '../lib/time';
import { itemPath, urlIdFromSlugId } from '../lib/urls';
import { useCanonicalPath } from '../lib/useCanonicalPath';
import NotFound from './NotFound';
import { PlaceholderPage } from './PlaceholderPage';

/** The Contents column docks from this width (§9.6.4). */
const DOCK_QUERY = '(min-width: 1280px)';
/** An item this page treats as new and discards when left empty (when the router state does not say so). */
const NEW_ITEM_MS = 2 * 60 * 1000;

function useMedia(query: string): boolean {
  const subscribe = useCallback(
    (cb: () => void) => {
      if (typeof window === 'undefined' || !window.matchMedia) return () => undefined;
      const m = window.matchMedia(query);
      m.addEventListener('change', cb);
      return () => m.removeEventListener('change', cb);
    },
    [query],
  );
  return useSyncExternalStore(
    subscribe,
    () => typeof window !== 'undefined' && !!window.matchMedia?.(query).matches,
    () => false,
  );
}

export default function DocPage() {
  const { slugId } = useParams();
  const ref = urlIdFromSlugId(slugId);
  const item = useItem(ref);
  useCanonicalPath(item.data ? itemPath(item.data.item) : null);

  if (ref === null || isApiError(item.error, 'not_found')) return <NotFound />;
  if (item.isPending) return null;
  if (item.isError) return <PlaceholderPage title="Document">{item.error.message}</PlaceholderPage>;
  if (item.data.item.kind !== 'doc') return <NotFound />;
  return <DocView key={item.data.item.id} detail={item.data} />;
}

function ContentsList({ headings, active, onPick }: { headings: Heading[]; active: string | null; onPick?(): void }) {
  return (
    <nav className="contents" aria-label="Contents">
      <ol>
        {headings.map((h) => (
          <li key={h.blockId} data-level={h.level} aria-current={h.blockId === active ? 'location' : undefined}>
            <a href={`#${headingAnchor(h.blockId)}`} onClick={onPick}>
              {h.text}
            </a>
          </li>
        ))}
      </ol>
    </nav>
  );
}

function ContentsPopover({ headings, active }: { headings: Heading[]; active: string | null }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="menu contents-popover">
      <button type="button" className="icon-button" aria-label="Contents" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <ListTree size={16} aria-hidden />
      </button>
      {open && <ContentsList headings={headings} active={active} onPick={() => setOpen(false)} />}
    </div>
  );
}

function ReadOnlyBanner({ detail }: { detail: ItemDetail }) {
  const qc = useQueryClient();
  const trashed = detail.trashed_at !== null;
  const run = async () => {
    await api('POST', `/items/${encodeURIComponent(detail.item.id)}/${trashed ? 'restore' : 'unarchive'}`);
    await qc.invalidateQueries({ queryKey: ['item'] });
    void qc.invalidateQueries({ queryKey: ['tree', detail.item.collection_id] });
    void qc.invalidateQueries({ queryKey: ['sidebar'] });
  };
  return (
    <div className="read-only-banner" role="status">
      {trashed ? 'This document is in the Trash.' : 'This document is archived.'}{' '}
      <button type="button" onClick={() => void run()}>
        {trashed ? 'Restore' : 'Unarchive'}
      </button>
    </div>
  );
}

function NestedList({ detail }: { detail: ItemDetail }) {
  const children = useChildren(detail.item.id);
  const create = useCreateItem();
  const navigate = useNavigate();
  const now = Date.now();
  return (
    <section className="nested-list" aria-label="Pages inside">
      {children.data && children.data.length > 0 && (
        <ul>
          {children.data.map((c) => (
            <li key={c.id}>
              <Link to={itemPath(c)}>
                {c.kind === 'board' ? <PenTool size={16} aria-hidden /> : <FileText size={16} aria-hidden />}
                <span className="nested-title">{c.title || 'Untitled'}</span>
                <span className="nested-meta">{relativeTime(c.updated_at, now)}</span>
              </Link>
            </li>
          ))}
        </ul>
      )}
      {detail.trashed_at === null && detail.archived_at === null && (
        <button
          type="button"
          className="add-page"
          disabled={create.isPending}
          onClick={() =>
            create.mutate(
              { collection_id: detail.item.collection_id, parent_id: detail.item.id, kind: 'doc' },
              { onSuccess: (created) => void navigate(itemPath(created), { state: { created: true } }) },
            )
          }
        >
          <Plus size={14} aria-hidden /> Add a page
        </button>
      )}
    </section>
  );
}

/** "Embed board…": a title search limited to boards. */
function BoardPicker({ onPick, onClose }: { onPick(id: string): void; onClose(): void }) {
  const [q, setQ] = useState('');
  const [hits, setHits] = useState<TitleHit[]>([]);
  useEffect(() => {
    const ctl = new AbortController();
    searchTitles(q, 20, ctl.signal).then(
      (h) => setHits(h.filter((x) => x.item.kind === 'board')),
      () => undefined,
    );
    return () => ctl.abort();
  }, [q]);
  return (
    <div className="dialog board-picker" role="dialog" aria-label="Embed a board">
      <input autoFocus placeholder="Find a board" value={q} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.key === 'Escape' && onClose()} />
      <ul>
        {hits.map((h) => (
          <li key={h.item.id}>
            <button type="button" onClick={() => onPick(h.item.id)}>
              {h.item.title || 'Untitled board'}
            </button>
          </li>
        ))}
        {hits.length === 0 && <li className="suggest-empty">No boards found</li>}
      </ul>
      <button type="button" onClick={onClose}>
        Cancel
      </button>
    </div>
  );
}

function DocView({ detail }: { detail: ItemDetail }) {
  const id = detail.item.id;
  const readOnly = detail.trashed_at !== null || detail.archived_at !== null;
  const qc = useQueryClient();
  const location = useLocation();
  const create = useCreateItem();
  const docked = useMedia(DOCK_QUERY);
  const [session, setSession] = useState<DocSession | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [headings, setHeadings] = useState<Heading[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [picking, setPicking] = useState<Editor | null>(null);
  const editorRef = useRef<Editor | null>(null);
  const titleRef = useRef(detail.item.title);
  titleRef.current = detail.item.title;
  const createdHere = useRef(
    (location.state as { created?: boolean } | null)?.created === true ||
      (detail.item.content_at === null && detail.item.title === '' && Date.now() - Date.parse(detail.item.created_at) < NEW_ITEM_MS),
  );

  useEffect(() => {
    let cancelled = false;
    let opened: DocSession | null = null;
    DocSession.open(id).then(
      (s) => {
        if (cancelled) {
          void s.flush().finally(() => s.destroy());
          return;
        }
        opened = s;
        setSession(s);
      },
      (e: unknown) => setError(e instanceof Error ? e.message : String(e)),
    );
    return () => {
      cancelled = true;
      const s = opened;
      if (!s) return;
      const editor = editorRef.current;
      const empty = editor ? editor.isEmpty : true;
      void s.flush().finally(() => {
        s.destroy();
        // A new item left empty is discarded (the server checks again: 409 not_empty when it is not).
        if (createdHere.current && empty && titleRef.current === '') {
          api('DELETE', `/items/${encodeURIComponent(id)}?discard_empty=true`).then(
            () => {
              void qc.invalidateQueries({ queryKey: ['tree', detail.item.collection_id] });
              void qc.invalidateQueries({ queryKey: ['sidebar'] });
              if (detail.item.parent_id) void qc.invalidateQueries({ queryKey: ['children', detail.item.parent_id] });
            },
            () => undefined,
          );
        }
      });
    };
  }, [id, qc, detail.item.collection_id, detail.item.parent_id]);

  // Scroll spy over the headings (§9.6.4).
  useEffect(() => {
    if (!showContents(headings)) return;
    const onScroll = () => {
      const tops = headings
        .map((h) => ({ blockId: h.blockId, top: document.getElementById(headingAnchor(h.blockId))?.getBoundingClientRect().top }))
        .filter((t): t is { blockId: string; top: number } => typeof t.top === 'number');
      setActive(activeHeading(tops, window.innerHeight));
    };
    onScroll();
    document.addEventListener('scroll', onScroll, { capture: true, passive: true });
    return () => document.removeEventListener('scroll', onScroll, { capture: true });
  }, [headings]);

  const embedBoard = (editor: Editor, boardId: string) =>
    editor.chain().focus().insertContent({ type: 'boardEmbed', attrs: { itemId: boardId } }).run();

  const contents = showContents(headings);
  const exports = docExports(id, () => titleRef.current, () => editorRef.current);
  return (
    <article className="doc-page">
      <TopBar
        item={detail}
        leading={contents && !docked ? <ContentsPopover headings={headings} active={active} /> : undefined}
        actions={<ItemMenu item={detail} extraExports={exports} />}
      />
      <div className="doc-layout" data-contents={contents && docked ? 'docked' : undefined}>
        {contents && docked && (
          <aside className="contents-column">
            <ContentsList headings={headings} active={active} />
          </aside>
        )}
        <div className="doc-column">
          {readOnly && <ReadOnlyBanner detail={detail} />}
          <ItemHeader item={detail} readOnly={readOnly} onEnterBody={() => editorRef.current?.commands.focus('start')} />
          {error && <PlaceholderPage>{error}</PlaceholderPage>}
          {session && (
            <QuireEditor
              itemId={id}
              session={session}
              readOnly={readOnly}
              onHeadings={setHeadings}
              onEditor={(e) => {
                editorRef.current = e;
              }}
              pickBoard={(e) => setPicking(e)}
              newBoard={(e) =>
                create.mutate(
                  { collection_id: detail.item.collection_id, parent_id: id, kind: 'board' },
                  { onSuccess: (board) => embedBoard(e, board.id) },
                )
              }
            />
          )}
          <NestedList detail={detail} />
        </div>
      </div>
      {picking && (
        <BoardPicker
          onClose={() => setPicking(null)}
          onPick={(boardId) => {
            embedBoard(picking, boardId);
            setPicking(null);
          }}
        />
      )}
    </article>
  );
}
