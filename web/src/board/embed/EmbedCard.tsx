// One card of the overlay (docs/ARCHITECTURE.md §18.5): the host's content inside the engine's box, positioned in
// world units by the controller (never by React). A header (kind icon, the live title, badges; while editing the
// title input, the save status and "Open page"), a rule, and the body: a static read-only viewer, or the real editor
// (`QuireEditor variant="embed"` on the card's DocSession) while this is the editing card.
// `data-state`: loading, static, editing, trashed, archived, missing (404, a non-UUID item, not a document), error.
import { useQuery } from '@tanstack/react-query';
import type { Editor, JSONContent } from '@tiptap/react';
import { useCallback, useEffect, useRef } from 'react';
import { Link, useNavigate } from 'react-router';
import { isApiError, type ApiError } from '../../api/client';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import { useDocJson } from '../../api/queries/docs';
import { fetchItem, itemKey } from '../../api/queries/items';
import { KindIcon } from '../../components/item/KindIcon';
import { useTitleAutosave } from '../../components/item/useTitleAutosave';
import { QuireEditor } from '../../editor/QuireEditor';
import { itemPath } from '../../lib/urls';
import { saveStateLabel, useSaveStatus } from '../../state/saveStatus';
import { isEditableTarget, type DrawnCard, type EditingCard, type EmbedController } from './controller';
import { isItemId } from './dragItem';
import { isBlankJson, StaticViewer } from './StaticViewer';

export type CardState = 'loading' | 'static' | 'editing' | 'trashed' | 'archived' | 'missing' | 'error';

export interface EmbedCardProps {
  card: DrawnCard;
  /** This card is the editing one. */
  editing: EditingCard | null;
  /** The board is read-only (trashed or archived). */
  readOnly: boolean;
  controller: EmbedController;
}

/** The target's `ItemDetail` (`['item', id]`), refetched on window focus. */
function useTarget(id: string) {
  return useQuery<ItemDetail, ApiError>({
    queryKey: itemKey(id),
    queryFn: ({ signal }) => fetchItem(id, signal),
    enabled: isItemId(id),
    refetchOnWindowFocus: true,
  });
}

function TitleInput({ detail, objectId, controller, onEnter }: {
  detail: ItemDetail;
  objectId: string;
  controller: EmbedController;
  onEnter(): void;
}) {
  const { title, setTitle, flush } = useTitleAutosave(detail);
  const ref = useCallback((el: HTMLInputElement | null) => controller.registerTitleInput(objectId, el), [controller, objectId]);
  return (
    <input
      ref={ref}
      className="board-embed-card-title board-embed-title-input"
      aria-label="Page title"
      placeholder="Untitled"
      value={title}
      onChange={(e) => setTitle(e.target.value)}
      onBlur={() => void flush()}
      onKeyDown={(e) => {
        if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
          e.preventDefault();
          onEnter();
        }
      }}
    />
  );
}

/**
 * The target's save state. While editing: the session's status, its message and Retry. On a static card, only trouble
 * (offline, not saved, signed out), by its state's label: the session that reported it is gone, so `onRetry` (a new
 * session that sends the outbox) replaces its retry, and there is none while `writable` is false (the target is in the
 * trash or archived; the card's own badge says so).
 */
function SaveState({ itemId, editing, writable, onRetry }: { itemId: string; editing: boolean; writable: boolean; onRetry(): void }) {
  const s = useSaveStatus(itemId);
  const trouble = s.state === 'offline' || s.state === 'error' || s.state === 'signed_out';
  if (!editing && !trouble) return null;
  const label = editing && s.message && trouble ? s.message : saveStateLabel(s.state);
  const retry = editing ? s.retry : writable ? onRetry : undefined;
  return (
    <span className={editing ? 'board-embed-status' : 'board-embed-badge board-embed-save'} role="status" data-state={s.state}>
      {label}
      {retry && (s.state === 'error' || s.state === 'offline') && (
        <>
          {' '}
          {/* A static card is aria-hidden: its Retry is for the pointer only (the page itself has the keyboard path). */}
          <button type="button" className="board-embed-retry" tabIndex={editing ? undefined : -1} onClick={retry}>
            Retry
          </button>
        </>
      )}
    </span>
  );
}

export function EmbedCard({ card, editing, readOnly, controller }: EmbedCardProps) {
  const { objectId, item: target } = card;
  const valid = isItemId(target);
  const q = useTarget(target);
  const detail = q.data;
  const gone = !valid || isApiError(q.error, 'not_found') || (detail !== undefined && detail.item.kind !== 'doc');
  const live = !gone && detail !== undefined;
  const docJson = useDocJson(target, live);
  const navigate = useNavigate();
  const editorRef = useRef<Editor | null>(null);
  const el = useRef<HTMLDivElement | null>(null);

  let state: CardState;
  if (gone || isApiError(docJson.error, 'not_found')) state = 'missing';
  else if (!detail) state = q.isError ? 'error' : 'loading';
  else if (detail.trashed_at !== null) state = 'trashed';
  else if (detail.archived_at !== null) state = 'archived';
  else if (docJson.isError && !docJson.data) state = 'error';
  else state = editing ? 'editing' : 'static';
  const isEditing = editing !== null;

  const ref = useCallback(
    (node: HTMLDivElement | null) => {
      el.current = node;
      controller.registerCard(objectId, node);
    },
    [controller, objectId],
  );

  // A target that stops being a live document ends the editing (trashed or archived elsewhere, purged).
  useEffect(() => {
    if (!isEditing) return;
    if (gone || (detail && !isEditableTarget(detail))) controller.targetGone(objectId);
  }, [isEditing, gone, detail, controller, objectId]);

  // While editing: Esc in the header (the title input, its buttons) leaves unless something consumed it; the editor's own
  // Esc comes through `onEscape` (ProseMirror prevents the default of every Esc, so this listener never acts on it). A pinch
  // over the card never zooms the page.
  useEffect(() => {
    const node = el.current;
    if (!isEditing || !node) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.defaultPrevented && !e.isComposing) {
        e.preventDefault();
        controller.escape(objectId);
      }
    };
    const onWheel = (e: WheelEvent) => {
      if (e.ctrlKey) e.preventDefault();
    };
    const onFocus = (e: FocusEvent) => {
      if (e.target instanceof HTMLElement) controller.cardFocused(objectId, e.target);
    };
    node.addEventListener('keydown', onKey);
    node.addEventListener('wheel', onWheel, { passive: false });
    node.addEventListener('focusin', onFocus);
    return () => {
      node.removeEventListener('keydown', onKey);
      node.removeEventListener('wheel', onWheel);
      node.removeEventListener('focusin', onFocus);
    };
  }, [isEditing, controller, objectId]);

  const title = detail ? detail.item.title || 'Untitled' : card.label || 'Untitled';
  const session = editing?.session ?? null;
  const onEditor = useCallback(
    (e: Editor | null) => {
      editorRef.current = e;
      controller.editorReady(objectId, e);
    },
    [controller, objectId],
  );

  let header;
  if (state === 'missing') {
    header = <s className="board-embed-card-title">{card.label || 'Untitled'}</s>;
  } else if (isEditing && detail) {
    header = (
      <TitleInput
        detail={detail}
        objectId={objectId}
        controller={controller}
        onEnter={() => editorRef.current?.commands.focus('start')}
      />
    );
  } else if (readOnly && detail) {
    header = (
      // The card is aria-hidden and the page is in the sidebar: a pointer affordance, out of the tab order.
      <Link className="board-embed-card-title board-embed-link" to={itemPath(detail.item)} tabIndex={-1}>
        {title}
      </Link>
    );
  } else {
    header = <span className="board-embed-card-title">{title}</span>;
  }

  const json = docJson.data?.json.doc as JSONContent | undefined;
  let body;
  if (state === 'missing') body = <p className="board-embed-message">Page not found</p>;
  else if (state === 'error') body = <p className="board-embed-message">Could not load this page</p>;
  else if (state === 'loading') body = null;
  else if (isEditing && session) {
    body = (
      <div className="board-embed-editor">
        <QuireEditor
          itemId={target}
          session={session}
          readOnly={false}
          variant="embed"
          popupRoot={controller.popupRoot}
          titleInput={() => controller.titleInput(objectId)}
          onEscape={() => controller.escape(objectId)}
          onEditor={onEditor}
        />
      </div>
    );
  } else if (json === undefined) body = null;
  else if (isBlankJson(json)) body = <p className="board-embed-blank">Empty page</p>;
  else {
    body = (
      <div className="board-embed-static">
        <StaticViewer doc={json} />
      </div>
    );
  }

  return (
    <div
      ref={ref}
      className="board-embed-card"
      data-object-id={objectId}
      data-item-id={target}
      data-state={state}
      data-editing={isEditing || undefined}
      data-selected={card.selected || undefined}
      tabIndex={isEditing ? -1 : undefined}
      role={isEditing ? 'region' : undefined}
      aria-label={isEditing ? `${title} (embedded page)` : undefined}
      aria-hidden={isEditing ? undefined : 'true'}
    >
      <div className="board-embed-header">
        <KindIcon kind="doc" />
        {header}
        {state === 'trashed' && <span className="board-embed-badge">In Trash</span>}
        {state === 'archived' && <span className="board-embed-badge">Archived</span>}
        {state === 'static' && card.selected && !readOnly && <span className="board-embed-hint">Double-click to edit</span>}
        {live && (
          <SaveState
            itemId={target}
            editing={isEditing}
            writable={!readOnly && (state === 'static' || state === 'editing')}
            onRetry={() => controller.retrySave(target)}
          />
        )}
        {isEditing && detail && (
          <button type="button" className="board-embed-open" onClick={() => void navigate(itemPath(detail.item))}>
            Open page
          </button>
        )}
      </div>
      <div
        className="board-embed-body"
        onMouseDown={(e) => {
          // A press in the editing body below the text (outside ProseMirror's own element) puts the caret at the end.
          const pm = editorRef.current?.view.dom;
          if (!isEditing || !pm || pm.contains(e.target as Node)) return;
          const t = e.target as HTMLElement;
          if (t !== e.currentTarget && !t.classList.contains('board-embed-editor') && !t.classList.contains('quire-editor-shell')) return;
          e.preventDefault();
          editorRef.current?.commands.focus('end');
        }}
      >
        {body}
      </div>
    </div>
  );
}
