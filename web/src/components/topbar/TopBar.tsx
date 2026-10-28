// The item pages' top bar (docs/ARCHITECTURE.md §9.3, §9.6.4): breadcrumbs (collection icon + name / ancestors /
// current title), save status, the "+ New doc" split button (Document, Board; a sibling, or a child with Alt), and the
// page's `actions`. Props are frozen (PLAN W0b). Pages put their `<ItemMenu item extraExports/>` into `actions`,
// since only the page knows its export formats; W4 puts its Contents popover button into `leading` or `actions`.
// There is no Publish button (D23); the avatars, Share and "Viewed by" slots are hidden in M1.
import { ChevronDown, Plus } from 'lucide-react';
import type { ReactNode } from 'react';
import { Link } from 'react-router';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import type { ItemKind } from '../../api/gen/ItemKind';
import { collectionPath, itemPath } from '../../lib/urls';
import { CollectionIcon } from '../collection/CollectionIcon';
import { useItemActions } from '../item/useItemActions';
import { useTitleAutosave } from '../item/useTitleAutosave';
import { Menu } from '../ui/Menu';
import { SaveStatus } from './SaveStatus';
import { SidebarToggle } from './SidebarToggle';

export interface TopBarProps {
  item: ItemDetail;
  /**
   * The last crumb, the item's own title: plain text by default, an input that renames the item with `'editable'`
   * (the board page, §9.6.4), or left out with `'none'`.
   */
  titleCrumb?: 'editable' | 'none';
  /** Rendered at the right end (the page's buttons and its ItemMenu). */
  actions?: ReactNode;
  /** Rendered before the breadcrumbs (e.g. the Contents button). */
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
      size={Math.max(8, Math.min(48, title.length + 1))}
      onChange={(e) => setTitle(e.target.value)}
      onBlur={() => void flush()}
      onKeyDown={(e) => {
        if (e.key === 'Enter') e.currentTarget.blur();
      }}
    />
  );
}

function NewSplitButton({ item }: { item: ItemDetail }) {
  const actions = useItemActions();
  const s = item.item;
  const create = (kind: ItemKind, child: boolean) =>
    void actions.create(s.collection_id, child ? s.id : s.parent_id, kind);
  return (
    <div className="split-button">
      <button
        type="button"
        className="button new-doc"
        title="New document next to this one (Alt: inside it)"
        onClick={(e) => create('doc', e.altKey)}
      >
        <Plus size={16} aria-hidden /> New doc
      </button>
      <Menu
        label="New…"
        buttonClassName="button split-toggle"
        align="end"
        entries={[
          { label: 'Document', accel: 'D', onSelect: () => create('doc', false) },
          { label: 'Board', accel: 'B', onSelect: () => create('board', false) },
          { separator: true },
          { label: 'Nested document', onSelect: () => create('doc', true) },
          { label: 'Nested board', onSelect: () => create('board', true) },
        ]}
      >
        <ChevronDown size={14} aria-hidden />
      </Menu>
    </div>
  );
}

export function TopBar({ item, titleCrumb, actions, leading }: TopBarProps) {
  const crumbs = item.breadcrumbs;
  return (
    <div className="top-bar">
      <SidebarToggle />
      {leading}
      <nav className="breadcrumbs" aria-label="Breadcrumbs">
        <Link to={collectionPath(item.collection)} className="crumb-collection">
          <CollectionIcon collection={item.collection} />
          {item.collection.name}
        </Link>
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
      <NewSplitButton item={item} />
      {actions}
    </div>
  );
}
