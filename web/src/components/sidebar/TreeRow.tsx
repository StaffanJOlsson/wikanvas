// One row of the collections tree (docs/ARCHITECTURE.md §9.3, §9.6.2): a collection row
// [icon/caret][name][hover: sort ⇅, +, ⋯] or an item row [indent][caret][kind icon][title][hover: +, ⋯]. Rows are
// ARIA treeitems with a roving tabindex; the label is a link, so middle-click and Mod+click open a new tab. Each row is a
// pragmatic-dnd draggable and drop target; CollectionsTree's monitor decides what a drop means. A document row also
// carries native drag data (`application/x-wikanvas-item`, docs/ARCHITECTURE.md §18.6 (c)), which only another document
// reads: the board frame, where dropping it places a card. A board row carries `application/x-wikanvas-board` (D145),
// which only the document editor reads, where dropping it embeds the board. The tree's own drag and drop is unchanged.
import { combine } from '@atlaskit/pragmatic-drag-and-drop/combine';
import { draggable, dropTargetForElements } from '@atlaskit/pragmatic-drag-and-drop/adapter/element-adapter';
import { ArrowUpDown, ChevronRight, Ellipsis, Plus } from 'lucide-react';
import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { Link } from 'react-router';
import { collectionPath, itemPath } from '../../lib/urls';
import { BOARD_HOVER_MS, boardRowHovered } from '../../state/hover';
import { CollectionIcon } from '../collection/CollectionIcon';
import { CHILD_ORDERS, type useCollectionActions } from '../collection/useCollectionActions';
import { KindIcon } from '../item/KindIcon';
import type { ItemActions } from '../item/useItemActions';
import { Menu, type MenuEntry } from '../ui/Menu';
import type { DropIndicator } from '../tree/drop';
import { indentX } from '../tree/hitZones';
import type { Row } from '../tree/visibleRows';
import { openDialog } from '../../state/dialogs';
import { boardDragDataOf, dragDataOf } from '../../board/embed/dragItem';

export const DRAG_ITEM = 'item';
export const DRAG_COLLECTION = 'collection';

export interface TreeRowProps {
  row: Row;
  focused: boolean;
  current: boolean;
  renaming: boolean;
  menuOpen: boolean;
  starred: boolean;
  indicator: DropIndicator | null;
  style?: CSSProperties;
  actions: ItemActions;
  collectionActions: ReturnType<typeof useCollectionActions>;
  onToggle(row: Row): void;
  onFocusRow(key: string): void;
  /** Starts the inline rename (menu "Rename", F2). */
  onRename(row: Row): void;
  /** The inline rename ended: the new title, or null when cancelled. */
  onRenameDone(row: Row, title: string | null): void;
  onMenuClosed(): void;
}

function RenameInput({ initial, onDone }: { initial: string; onDone(title: string | null): void }) {
  const [value, setValue] = useState(initial);
  const done = useRef(false);
  const finish = (title: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(title);
  };
  return (
    <input
      className="row-rename"
      aria-label="Title"
      autoFocus
      value={value}
      placeholder="Untitled"
      onChange={(e) => setValue(e.target.value)}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === 'Enter') {
          e.preventDefault();
          finish(value);
        } else if (e.key === 'Escape') {
          e.preventDefault();
          finish(null);
        }
      }}
      onBlur={() => finish(value)}
    />
  );
}

export function TreeRow(props: TreeRowProps) {
  const { row, focused, current, renaming, menuOpen, starred, indicator, style, actions, collectionActions } = props;
  const el = useRef<HTMLDivElement>(null);
  const [dragging, setDragging] = useState(false);
  const [menusOpen, setMenusOpen] = useState(0);
  const collectionId = row.kind === 'item' ? row.collectionId : row.collection.id;
  const isDoc = row.kind === 'item' && row.item.kind === 'doc';
  const isBoard = row.kind === 'item' && row.item.kind === 'board';
  // Read when the drag starts, so a rename since the row mounted is carried.
  const title = useRef('');
  title.current = row.kind === 'item' ? row.item.title : '';

  useEffect(() => {
    const element = el.current;
    if (!element || renaming) return;
    const data = row.kind === 'item'
      ? { wikanvasDrag: DRAG_ITEM, itemId: row.key, collectionId }
      : { wikanvasDrag: DRAG_COLLECTION, collectionId };
    const id = row.key;
    return combine(
      draggable({
        element,
        getInitialData: () => data,
        ...(isDoc ? { getInitialDataForExternal: () => dragDataOf({ id, title: title.current }) } : {}),
        ...(isBoard ? { getInitialDataForExternal: () => boardDragDataOf({ id, title: title.current }) } : {}),
        onDragStart: () => setDragging(true),
        onDrop: () => setDragging(false),
      }),
      dropTargetForElements({
        element,
        getData: () => ({ wikanvasRow: row.key }),
        canDrop: ({ source }) => source.data.wikanvasDrag === DRAG_ITEM || source.data.wikanvasDrag === DRAG_COLLECTION,
      }),
    );
  }, [row.key, row.kind, isDoc, isBoard, collectionId, renaming]);

  // Board rows warm the engine cache after a 300 ms hover (§9.5); W6 registers the warm-up.
  const hoverTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const onEnter = () => {
    if (row.kind !== 'item' || row.item.kind !== 'board') return;
    const id = row.item.id;
    hoverTimer.current = setTimeout(() => boardRowHovered(id), BOARD_HOVER_MS);
  };
  const onLeave = () => {
    if (hoverTimer.current !== null) clearTimeout(hoverTimer.current);
    hoverTimer.current = null;
  };
  useEffect(() => onLeave, []);

  const trackMenu = (open: boolean) => setMenusOpen((n) => Math.max(0, n + (open ? 1 : -1)));

  const className = [
    'tree-row',
    row.kind === 'collection' ? 'collection-row' : 'item-row',
    current ? 'current' : '',
    dragging ? 'dragging' : '',
    menusOpen > 0 || menuOpen ? 'menu-open' : '',
    indicator ? `drop-${indicator.kind}` : '',
  ].filter(Boolean).join(' ');

  const caret = row.hasChildren ? (
    <button
      type="button"
      className="row-caret"
      tabIndex={-1}
      aria-hidden
      onClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
        props.onToggle(row);
      }}
    >
      <ChevronRight size={14} className={row.expanded ? 'open' : undefined} />
    </button>
  ) : <span className="row-caret" aria-hidden />;

  let label: string;
  let to: string;
  let icon: ReactNode;
  let menus: ReactNode;
  if (row.kind === 'collection') {
    const c = row.collection;
    label = c.name;
    to = collectionPath(c);
    icon = (
      <span className="row-icon collection-icon-slot">
        <CollectionIcon collection={c} />
        {caret}
      </span>
    );
    const sortEntries: MenuEntry[] = CHILD_ORDERS.map(({ order, label: l }) => ({
      label: l,
      checked: c.child_order === order,
      onSelect: () => void collectionActions.setOrder(c, order),
    }));
    menus = (
      <span className="row-actions">
        <Menu label="Sort" entries={sortEntries} onOpenChange={trackMenu}>
          <ArrowUpDown size={14} aria-hidden />
        </Menu>
        <Menu
          label={`New in ${c.name}`}
          title="New…"
          onOpenChange={trackMenu}
          entries={[
            { label: 'Document', accel: 'D', onSelect: () => void actions.create(c.id, null, 'doc') },
            { label: 'Board', accel: 'B', onSelect: () => void actions.create(c.id, null, 'board') },
          ]}
        >
          <Plus size={14} aria-hidden />
        </Menu>
        <Menu
          label={`${c.name} menu`}
          title="More"
          align="end"
          open={menuOpen ? true : undefined}
          onOpenChange={(o) => {
            trackMenu(o);
            if (!o) props.onMenuClosed();
          }}
          entries={[
            { label: 'Edit…', onSelect: () => collectionActions.edit(c) },
            { label: 'New document', onSelect: () => void actions.create(c.id, null, 'doc') },
            { label: 'New board', onSelect: () => void actions.create(c.id, null, 'board') },
            { heading: 'Sort' },
            ...sortEntries,
            { separator: true },
            { label: 'Delete…', danger: true, onSelect: () => void collectionActions.remove(c) },
          ]}
        >
          <Ellipsis size={14} aria-hidden />
        </Menu>
      </span>
    );
  } else {
    const it = row.item;
    label = it.title || 'Untitled';
    to = itemPath(it);
    icon = (
      <>
        {caret}
        <span className="row-icon"><KindIcon kind={it.kind} /></span>
      </>
    );
    menus = (
      <span className="row-actions">
        <Menu
          label={`New inside ${label}`}
          title="New nested…"
          onOpenChange={trackMenu}
          entries={[
            { label: 'Document', accel: 'D', onSelect: () => void actions.create(row.collectionId, it.id, 'doc') },
            { label: 'Board', accel: 'B', onSelect: () => void actions.create(row.collectionId, it.id, 'board') },
          ]}
        >
          <Plus size={14} aria-hidden />
        </Menu>
        <Menu
          label={`${label} menu`}
          title="More"
          align="end"
          open={menuOpen ? true : undefined}
          onOpenChange={(o) => {
            trackMenu(o);
            if (!o) props.onMenuClosed();
          }}
          entries={[
            { label: 'Rename', onSelect: () => props.onRename(row) },
            { label: 'New child document', onSelect: () => void actions.create(row.collectionId, it.id, 'doc') },
            { label: 'New child board', onSelect: () => void actions.create(row.collectionId, it.id, 'board') },
            { label: 'Duplicate', onSelect: () => void actions.duplicate(it) },
            { label: 'Move…', onSelect: () => openDialog({ kind: 'move', item: it }) },
            { label: starred ? 'Unstar' : 'Star', onSelect: () => void actions.setStarred(it, !starred) },
            { label: 'Archive', onSelect: () => void actions.archive(it) },
            { separator: true },
            { label: 'Delete', danger: true, onSelect: () => void actions.trash(it) },
          ]}
        >
          <Ellipsis size={14} aria-hidden />
        </Menu>
      </span>
    );
  }

  const indicatorStyle = indicator && indicator.kind !== 'box'
    ? ({ '--drop-indent': `${indentX(indicator.depth)}px` } as CSSProperties)
    : undefined;

  return (
    <div
      ref={el}
      role="treeitem"
      aria-level={row.depth + 1}
      aria-expanded={row.hasChildren ? row.expanded : undefined}
      aria-selected={current}
      aria-current={current ? 'page' : undefined}
      aria-posinset={row.kind === 'item' ? row.posInSet : undefined}
      aria-setsize={row.kind === 'item' ? row.setSize : undefined}
      aria-label={label}
      data-key={row.key}
      tabIndex={focused ? 0 : -1}
      className={className}
      style={{ ...style, ...indicatorStyle, paddingLeft: indentX(row.depth) }}
      onFocus={(e) => {
        if (e.target === e.currentTarget) props.onFocusRow(row.key);
      }}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
    >
      {icon}
      {renaming && row.kind === 'item' ? (
        <RenameInput initial={row.item.title} onDone={(t) => props.onRenameDone(row, t)} />
      ) : (
        <Link className="row-label" to={to} tabIndex={-1} draggable={false} onClick={() => props.onFocusRow(row.key)}>
          {label}
        </Link>
      )}
      {!renaming && menus}
    </div>
  );
}
