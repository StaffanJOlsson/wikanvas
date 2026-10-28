// The quick switcher (Mod+K; docs/ARCHITECTURE.md §9.3, §9.6.3): an empty query shows recently viewed items, typing
// searches titles (`/search/titles`). ↑/↓ select, Enter opens, Mod+Enter opens in a new tab, Esc closes.
import { useEffect, useId, useState } from 'react';
import { useNavigate } from 'react-router';
import type { Crumb } from '../../api/gen/Crumb';
import type { ItemSummary } from '../../api/gen/ItemSummary';
import { flattenPages, useList } from '../../api/queries/lists';
import { useTitleSearch } from '../../api/queries/search';
import { modPressed } from '../../app/hotkeys';
import { itemPath } from '../../lib/urls';
import { closeDialog } from '../../state/dialogs';
import { KindIcon } from '../item/KindIcon';
import { ItemPath } from '../list/ItemPath';
import { Dialog } from '../ui/Dialog';

export const SWITCHER_DEBOUNCE_MS = 120;
export const SWITCHER_LIMIT = 10;

interface Entry {
  item: ItemSummary;
  path: Crumb[];
}

function useDebounced(value: string, ms: number): string {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

export function QuickSwitcher() {
  const navigate = useNavigate();
  const listId = useId();
  const [q, setQ] = useState('');
  const [selected, setSelected] = useState(0);
  const query = useDebounced(q, SWITCHER_DEBOUNCE_MS).trim();
  const empty = q.trim() === '';
  const recent = useList('recent', { limit: SWITCHER_LIMIT, enabled: empty });
  const titles = useTitleSearch(empty ? '' : query, SWITCHER_LIMIT);

  const entries: Entry[] = empty
    ? flattenPages(recent.data).slice(0, SWITCHER_LIMIT).map((l) => ({ item: l, path: l.path }))
    : (titles.data ?? []).map((t) => ({ item: t.item, path: t.path }));

  useEffect(() => setSelected(0), [empty, query]);

  const open = (entry: Entry | undefined, newTab: boolean) => {
    if (!entry) return;
    const path = itemPath(entry.item);
    closeDialog();
    if (newTab) window.open(path, '_blank', 'noopener');
    else void navigate(path);
  };

  const optionId = (i: number) => `${listId}-${i}`;
  const loading = empty ? recent.isPending : titles.isFetching || query !== q.trim();

  return (
    <Dialog title="Quick switcher" hideTitle onClose={closeDialog} className="quick-switcher">
      <input
        className="switcher-input"
        role="combobox"
        aria-label="Search titles"
        aria-expanded="true"
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={entries[selected] ? optionId(selected) : undefined}
        placeholder="Search titles…"
        data-autofocus
        value={q}
        onChange={(e) => setQ(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'ArrowDown') {
            e.preventDefault();
            setSelected((s) => Math.min(entries.length - 1, s + 1));
          } else if (e.key === 'ArrowUp') {
            e.preventDefault();
            setSelected((s) => Math.max(0, s - 1));
          } else if (e.key === 'Enter') {
            e.preventDefault();
            open(entries[selected], modPressed(e));
          }
        }}
      />
      <div className="switcher-heading">{empty ? 'Recently viewed' : 'Titles'}</div>
      <ul id={listId} role="listbox" aria-label={empty ? 'Recently viewed' : 'Matching titles'} className="switcher-list">
        {entries.map((entry, i) => (
          <li
            key={entry.item.id}
            id={optionId(i)}
            role="option"
            aria-selected={i === selected}
            className={i === selected ? 'selected' : undefined}
            onMouseMove={() => setSelected(i)}
            onClick={(e) => open(entry, modPressed(e))}
          >
            <KindIcon kind={entry.item.kind} />
            <span className="switcher-title">{entry.item.title || 'Untitled'}</span>
            <ItemPath collectionId={entry.item.collection_id} path={entry.path} />
          </li>
        ))}
      </ul>
      {entries.length === 0 && !loading && (
        <p className="switcher-empty">{empty ? 'Nothing viewed yet.' : 'No title matches.'}</p>
      )}
    </Dialog>
  );
}
