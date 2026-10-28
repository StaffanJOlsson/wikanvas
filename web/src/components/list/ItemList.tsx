// A list of items with their paths (Home, Starred, Archive, the collection page; docs/ARCHITECTURE.md §9.6.6), with
// "Load more" for paginated lists.
import type { ReactNode } from 'react';
import { Link } from 'react-router';
import type { Crumb } from '../../api/gen/Crumb';
import type { ItemSummary } from '../../api/gen/ItemSummary';
import { relativeTime } from '../../lib/time';
import { itemPath } from '../../lib/urls';
import { KindIcon } from '../item/KindIcon';
import { ItemPath } from './ItemPath';

export interface ListEntry {
  item: ItemSummary;
  path?: readonly Crumb[];
  /** The time shown at the right (defaults to "updated …"). */
  meta?: string;
  actions?: ReactNode;
}

export function ItemList({ entries, empty, label }: { entries: ListEntry[]; empty: string; label: string }) {
  if (entries.length === 0) return <p className="list-empty">{empty}</p>;
  return (
    <ul className="item-list" aria-label={label}>
      {entries.map(({ item, path, meta, actions }) => (
        <li key={item.id} className="item-list-row">
          <Link to={itemPath(item)} className="item-list-link">
            <KindIcon kind={item.kind} />
            <span className="item-list-text">
              <span className="item-list-title">{item.title || 'Untitled'}</span>
              {path && <ItemPath collectionId={item.collection_id} path={path} />}
            </span>
          </Link>
          <span className="item-list-meta">{meta ?? `Updated ${relativeTime(item.updated_at)}`}</span>
          {actions && <span className="item-list-actions">{actions}</span>}
        </li>
      ))}
    </ul>
  );
}

export function LoadMore({ hasNext, loading, onMore }: { hasNext: boolean; loading: boolean; onMore(): void }) {
  if (!hasNext) return null;
  return (
    <div className="load-more">
      <button type="button" className="button" disabled={loading} onClick={onMore}>
        {loading ? 'Loading…' : 'Load more'}
      </button>
    </div>
  );
}
