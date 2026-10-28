// The item pages' top bar (docs/ARCHITECTURE.md §9.3, §9.6.4): breadcrumbs, save status, "+ New doc", and the
// page's `actions`. Props are frozen (PLAN W0b); W3 completes it (collection icon, the New split button with Board,
// the Contents popover slot). Pages put their `<ItemMenu item extraExports/>` into `actions`, since only the page knows
// its export formats.
import { Plus } from 'lucide-react';
import type { ReactNode } from 'react';
import { Link, useNavigate } from 'react-router';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import { useCreateItem } from '../../api/queries/items';
import { collectionPath, itemPath } from '../../lib/urls';
import { useTitleAutosave } from '../item/useTitleAutosave';
import { SaveStatus } from './SaveStatus';

export interface TopBarProps {
  item: ItemDetail;
  /**
   * The last crumb, the item's own title: plain text by default, an input that renames the item with `'editable'`
   * (the board page, §9.6.4), or left out with `'none'`.
   */
  titleCrumb?: 'editable' | 'none';
  /** Rendered at the right end (the page's buttons and its ItemMenu). */
  actions?: ReactNode;
  /** Rendered before the breadcrumbs (e.g. the sidebar toggle or the Contents button). */
  leading?: ReactNode;
}

function EditableTitle({ item }: { item: ItemDetail }) {
  const { title, setTitle, flush } = useTitleAutosave(item);
  return (
    <input
      className="title-crumb"
      aria-label="Title"
      placeholder="Untitled"
      value={title}
      onChange={(e) => setTitle(e.target.value)}
      onBlur={() => void flush()}
      onKeyDown={(e) => {
        if (e.key === 'Enter') e.currentTarget.blur();
      }}
    />
  );
}

function NewDocButton({ item }: { item: ItemDetail }) {
  const create = useCreateItem();
  const navigate = useNavigate();
  return (
    <button
      type="button"
      className="icon-button new-doc"
      title="New document (Alt: as a child)"
      disabled={create.isPending}
      onClick={(e) => {
        const parent_id = e.altKey ? item.item.id : item.item.parent_id;
        create.mutate(
          { collection_id: item.item.collection_id, parent_id, kind: 'doc' },
          { onSuccess: (created) => void navigate(itemPath(created)) },
        );
      }}
    >
      <Plus size={16} aria-hidden /> New doc
    </button>
  );
}

export function TopBar({ item, titleCrumb, actions, leading }: TopBarProps) {
  const crumbs = item.breadcrumbs;
  return (
    <div className="top-bar">
      {leading}
      <nav className="breadcrumbs" aria-label="Breadcrumbs">
        <Link to={collectionPath(item.collection)}>{item.collection.name}</Link>
        {crumbs.map((c) => (
          <span key={c.id}>
            {' / '}
            <Link to={itemPath(c)}>{c.title || 'Untitled'}</Link>
          </span>
        ))}
        {titleCrumb !== 'none' && (
          <span aria-current="page">
            {' / '}
            {titleCrumb === 'editable' ? <EditableTitle item={item} /> : item.item.title || 'Untitled'}
          </span>
        )}
      </nav>
      <SaveStatus itemId={item.item.id} />
      <NewDocButton item={item} />
      {actions}
    </div>
  );
}
