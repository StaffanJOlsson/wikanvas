// The editing surface of a board page (docs/ARCHITECTURE.md §9.3, §9.5): it hands its canvas element to the frame
// host (which lays the iframe over it), opens the board in the frame, runs the save loop, and shows BoardLoading, the
// ConflictBanner, the restore prompt for an unsaved snapshot, the large-board warning, and a visually hidden list of
// the board's text (`BoardMeta.text_excerpt`) for screen readers.
import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import type { ItemDetail } from '../api/gen/ItemDetail';
import { fetchBoard, putBoard } from '../api/queries/boards';
import { useConfig } from '../api/queries/config';
import { updateCachedItem, useCreateItem } from '../api/queries/items';
import { itemPath } from '../lib/urls';
import { BoardLoading } from './BoardLoading';
import type { BoardFrameContext } from './BoardFrameHost';
import type { BoardFrameApi } from './bridge';
import { ConflictBanner } from './ConflictBanner';
import {
  attachSurface,
  flushPrevious,
  noteSaved,
  openBoard,
  releaseBoard,
  showBoard,
  useHostStore,
  type BoardSession,
} from './host';
import { BoardSaveLoop, LARGE_BOARD_BYTES, type BoardConflict } from './saveLoop';
import { useResolvedTheme } from './theme';
import { clearUnsaved, isNewerThan, readUnsaved, snapshotTime, type UnsavedSnapshot } from './unsaved';

interface SessionState {
  loading: boolean;
  conflict: BoardConflict | null;
  restore: UnsavedSnapshot | null;
  large: number | null;
  error: string | null;
  busy: boolean;
  actionError: string | null;
}

const fresh: SessionState = {
  loading: true,
  conflict: null,
  restore: null,
  large: null,
  error: null,
  busy: false,
  actionError: null,
};

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export interface BoardSurfaceProps {
  item: ItemDetail;
  host: BoardFrameContext;
}

export function BoardSurface({ item, host }: BoardSurfaceProps) {
  const itemId = item.item.id;
  const frame = host.frame;
  const qc = useQueryClient();
  const navigate = useNavigate();
  const create = useCreateItem();
  const config = useConfig();
  const theme = useResolvedTheme();
  const readOnly = item.trashed_at !== null || item.archived_at !== null;
  const limitBytes = (config.data?.limits.max_board_mb ?? 50) * 1024 * 1024;
  const canvas = useRef<HTMLDivElement>(null);
  const [s, setS] = useState<SessionState>(fresh);
  const loopRef = useRef<BoardSaveLoop | null>(null);
  // The latest values for the open effect, which runs once per frame and board.
  const latest = useRef({ item, theme, readOnly, limitBytes });
  latest.current = { item, theme, readOnly, limitBytes };

  useLayoutEffect(() => {
    const el = canvas.current;
    return el ? attachSurface(el) : undefined;
  }, []);

  // Open the board once the frame is connected; flush and release it when the page leaves it.
  useEffect(() => {
    if (!frame) return;
    let alive = true;
    const update = (patch: Partial<SessionState>) => {
      if (alive) setS((st) => ({ ...st, ...patch }));
    };
    const loop = new BoardSaveLoop({
      itemId,
      rev: latest.current.item.board?.rev ?? 1,
      frame: () => useHostStore.getState().frame,
      limitBytes: latest.current.limitBytes,
      onSaved: (r) => {
        noteSaved(itemId);
        updateCachedItem(qc, itemId, (d) => ({ ...d, board: r.meta }));
      },
      onConflict: (conflict) => update({ conflict }),
      onLarge: (large) => update({ large }),
    });
    loopRef.current = loop;
    const session: BoardSession = {
      itemId,
      flush: () => loop.flush(),
      onChange: (revision) => loop.change(revision),
      onSave: () => void loop.flush(),
      onLoadError: (m) => update({ error: `The board could not be opened: ${m}`, loading: false }),
    };
    loop.attach();
    setS(fresh);
    void (async () => {
      // §9.5 Open: the board shown before saves first (it may be this board, left a moment ago with a PUT running),
      // so the GET below sees its last save.
      await flushPrevious(session);
      if (!alive) return;
      const loaded = await fetchBoard(itemId);
      if (!alive) return;
      const snap = await readUnsaved(itemId);
      if (!alive) return;
      const { item: it, theme: t, readOnly: ro } = latest.current;
      loop.rev = loaded.rev;
      await openBoard(session, { itemId, board: loaded.board, name: it.item.title, readOnly: ro, theme: t });
      if (!alive) return;
      showBoard(true);
      const lastWrite = it.item.content_at ?? it.item.updated_at;
      update({
        loading: false,
        restore: snap && !ro && isNewerThan(snap, lastWrite) ? snap : null,
        large: loaded.board.length > LARGE_BOARD_BYTES ? loaded.board.length : null,
      });
    })().catch((e: unknown) => update({ loading: false, error: `The board could not be loaded: ${message(e)}` }));
    return () => {
      alive = false;
      showBoard(false);
      if (loopRef.current === loop) loopRef.current = null;
      void loop.flush().finally(() => {
        loop.dispose();
        releaseBoard(session);
      });
    };
  }, [frame, itemId, qc]);

  // Title and read-only state follow the item.
  const title = item.item.title;
  useEffect(() => {
    if (frame?.current() === itemId) frame.setName(title);
  }, [frame, itemId, title]);
  useEffect(() => {
    if (frame?.current() === itemId) frame.setReadOnly(readOnly);
  }, [frame, itemId, readOnly]);

  const act = async (work: (f: BoardFrameApi, loop: BoardSaveLoop) => Promise<void>) => {
    const loop = loopRef.current;
    if (!frame || !loop) return;
    setS((st) => ({ ...st, busy: true, actionError: null }));
    try {
      await work(frame, loop);
      setS((st) => ({ ...st, busy: false }));
    } catch (e) {
      setS((st) => ({ ...st, busy: false, actionError: message(e) }));
    }
  };

  const loadLatest = () =>
    act(async (f, loop) => {
      const loaded = await fetchBoard(itemId);
      await f.open({ itemId, board: loaded.board, name: title, readOnly, theme });
      loop.resume(loaded.rev);
      await clearUnsaved(itemId);
      setS((st) => ({ ...st, conflict: null }));
    });

  const keepCopy = () =>
    act(async (f) => {
      const conflict = s.conflict;
      if (!conflict) return;
      const copy = await create.mutateAsync({
        collection_id: item.item.collection_id,
        parent_id: item.item.parent_id,
        kind: 'board',
        title: `${title || 'Untitled'} (copy)`,
      });
      await putBoard(copy.id, conflict.board, 1, f.engineVersion);
      await clearUnsaved(itemId);
      setS((st) => ({ ...st, conflict: null }));
      void navigate(itemPath(copy));
    });

  const restore = () =>
    act(async (f, loop) => {
      const snap = s.restore;
      if (!snap) return;
      await f.open({ itemId, board: snap.board, name: title, readOnly, theme });
      loop.rev = snap.baseRev;
      setS((st) => ({ ...st, restore: null }));
      await loop.saveNow();
    });

  const discard = () =>
    act(async () => {
      await clearUnsaved(itemId);
      setS((st) => ({ ...st, restore: null }));
    });

  const lines = (item.board?.text_excerpt ?? '').split('\n').filter((l) => l.trim() !== '');
  const mb = (n: number) => Math.round(n / (1024 * 1024));

  return (
    <div className="board-surface">
      {s.conflict && (
        <ConflictBanner onLoadLatest={() => void loadLatest()} onKeepCopy={() => void keepCopy()} busy={s.busy}
          error={s.actionError} />
      )}
      {s.restore && (
        <div role="alert" className="board-banner">
          <span>Restore unsaved changes from {snapshotTime(s.restore.at)}?</span>
          <button type="button" disabled={s.busy} onClick={() => void restore()}>
            Restore
          </button>
          <button type="button" disabled={s.busy} onClick={() => void discard()}>
            Discard
          </button>
        </div>
      )}
      {s.large !== null && (
        <div role="status" className="board-banner board-warning">
          This board is {mb(s.large)} MB. Boards over {mb(limitBytes)} MB cannot be saved; large images make a board
          slow to open and save.
        </div>
      )}
      {s.error && (
        <div role="alert" className="board-banner board-error">
          {s.error}
        </div>
      )}
      <div ref={canvas} className="board-canvas" data-testid="board-canvas">
        {(s.loading || !frame) && <BoardLoading item={item} progress={host.progress} />}
      </div>
      {lines.length > 0 && (
        <ul className="visually-hidden" aria-label="Text on this board">
          {lines.map((l, i) => (
            <li key={i}>{l}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
