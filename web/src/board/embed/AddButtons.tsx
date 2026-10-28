// The board page's two ways to add a card from the top bar (docs/ARCHITECTURE.md §18.6 (a), (b)): "Text doc" creates
// a new document as a child of the board, places it in the middle of the view and starts editing it; "Embed page…"
// opens a picker of documents and places the chosen one there (selected, not opened). Both are disabled while the
// board is read-only, the frame has not opened it or lacks `insertEmbed`, or (Text doc) a creation runs; the page
// renders them only when the connected frame has `insertEmbed`.
import { useQueryClient } from '@tanstack/react-query';
import { FilePlus, FileSymlink } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import type { ItemSummary } from '../../api/gen/ItemSummary';
import type { TitleHit } from '../../api/gen/TitleHit';
import { searchTitles } from '../../api/queries/docs';
import { useCreateItem } from '../../api/queries/items';
import { KindIcon } from '../../components/item/KindIcon';
import { Dialog } from '../../components/ui/Dialog';
import { errorToast } from '../../state/toasts';
import { insertCard, useEmbedStore } from './controller';
import { discardTextDoc } from './textDocs';

export const PICKER_DEBOUNCE_MS = 250;
export const PICKER_LIMIT = 20;

/** Whether the board page may add cards now: the frame opened this board, writable. */
export function useCanAddCards(boardId: string): boolean {
  return useEmbedStore((s) => s.board?.itemId === boardId && !s.board.readOnly);
}

export function TextDocButton({ item, disabled, onCreated }: {
  item: ItemDetail;
  disabled: boolean;
  /** The created document, remembered for the discard when the page leaves the board. */
  onCreated(docId: string): void;
}) {
  const qc = useQueryClient();
  const create = useCreateItem();
  const [busy, setBusy] = useState(false);
  const run = async () => {
    setBusy(true);
    try {
      const doc = await create.mutateAsync({ collection_id: item.item.collection_id, parent_id: item.item.id, kind: 'doc' });
      const placed = insertCard({ id: doc.id, title: doc.title }, 'center', { edit: true });
      if (placed === null) {
        await discardTextDoc(qc, { id: item.item.id, collectionId: item.item.collection_id }, doc.id);
      } else {
        onCreated(doc.id);
      }
    } catch (e) {
      errorToast(e, 'Could not create the page');
    } finally {
      setBusy(false);
    }
  };
  return (
    <button
      type="button"
      className="button board-add-card"
      title="A new page on this board"
      disabled={disabled || busy}
      onClick={() => void run()}
    >
      <FilePlus size={16} aria-hidden /> Text doc
    </button>
  );
}

function pathOf(h: TitleHit): string {
  return h.path.map((c) => c.title || 'Untitled').join(' / ');
}

/** "Embed a page": a title search limited to documents; Enter takes the first, Esc closes. */
export function EmbedPagePicker({ onPick, onClose }: { onPick(item: ItemSummary): void; onClose(): void }) {
  const [q, setQ] = useState('');
  const [hits, setHits] = useState<TitleHit[] | null>(null);
  const first = useRef(true);
  useEffect(() => {
    const ctl = new AbortController();
    const delay = first.current ? 0 : PICKER_DEBOUNCE_MS;
    first.current = false;
    const t = setTimeout(() => {
      searchTitles(q, PICKER_LIMIT, ctl.signal).then(
        (h) => setHits(h.filter((x) => x.item.kind === 'doc')),
        () => undefined,
      );
    }, delay);
    return () => {
      clearTimeout(t);
      ctl.abort();
    };
  }, [q]);
  return (
    <Dialog title="Embed a page" onClose={onClose} className="embed-picker">
      <input
        className="input"
        aria-label="Find a page"
        placeholder="Find a page"
        data-autofocus
        value={q}
        onChange={(e) => setQ(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
            e.preventDefault();
            const top = hits?.[0];
            if (top) onPick(top.item);
          }
        }}
      />
      <ul className="embed-picker-results">
        {(hits ?? []).map((h) => {
          const title = h.item.title || 'Untitled';
          const path = pathOf(h);
          return (
            <li key={h.item.id}>
              <button type="button" aria-label={title} onClick={() => onPick(h.item)}>
                <KindIcon kind="doc" />
                <span className="embed-picker-title">{title}</span>
                {path && <span className="embed-picker-path">{path}</span>}
              </button>
            </li>
          );
        })}
        {hits !== null && hits.length === 0 && <li className="suggest-empty">No pages found</li>}
      </ul>
    </Dialog>
  );
}

export function EmbedPageButton({ disabled }: { disabled: boolean }) {
  const [open, setOpen] = useState(false);
  // The card is placed after the dialog is gone: the dialog gives focus back to this button when it unmounts, and
  // the insert then moves it into the frame.
  const [chosen, setChosen] = useState<ItemSummary | null>(null);
  useEffect(() => {
    if (!chosen) return;
    setChosen(null);
    insertCard({ id: chosen.id, title: chosen.title }, 'center');
  }, [chosen]);
  return (
    <>
      <button
        type="button"
        className="button board-add-card"
        title="Place an existing page on this board"
        aria-haspopup="dialog"
        disabled={disabled}
        onClick={() => setOpen(true)}
      >
        <FileSymlink size={16} aria-hidden /> Embed page…
      </button>
      {open && (
        <EmbedPagePicker
          onClose={() => setOpen(false)}
          onPick={(item) => {
            setOpen(false);
            setChosen(item);
          }}
        />
      )}
    </>
  );
}
