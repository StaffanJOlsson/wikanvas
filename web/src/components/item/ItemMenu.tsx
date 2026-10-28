// The item's "⋯" menu in the top bar (docs/ARCHITECTURE.md §9.3, §9.6.4): Star, Full width (docs), Duplicate, Move…,
// Archive, Export (the page's formats) and Delete. Props are frozen (PLAN W0b). The page renders it (in TopBar's
// `actions`) and adds its own export formats: docs Markdown and HTML (W4), boards .inkboard and SVG (W6).
import { MoreHorizontal } from 'lucide-react';
import { useNavigate } from 'react-router';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import { usePatchItem, updateCachedItem } from '../../api/queries/items';
import { useQueryClient } from '@tanstack/react-query';
import { collectionPath, itemPath } from '../../lib/urls';
import { openDialog } from '../../state/dialogs';
import { errorToast } from '../../state/toasts';
import { Menu, type MenuEntry } from '../ui/Menu';
import { useItemActions } from './useItemActions';

/** An export format the page adds to the menu's Export section. */
export interface ExtraExport {
  label: string;
  run(): void | Promise<void>;
}

export interface ItemMenuProps {
  item: ItemDetail;
  extraExports: ExtraExport[];
}

/**
 * The document's full-width setting. `ItemDetail` has no `full_width` field yet (request to W1 in docs/handoff/W3.md);
 * until it does, the value set in this session is kept in the cached detail.
 */
export function fullWidthOf(item: ItemDetail): boolean {
  return (item as ItemDetail & { full_width?: boolean }).full_width === true;
}

export function ItemMenu({ item, extraExports }: ItemMenuProps) {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const actions = useItemActions();
  const patch = usePatchItem().mutateAsync;
  const s = item.item;
  const readOnly = item.trashed_at !== null;

  const run = (action: () => void | Promise<void>) => () => {
    Promise.resolve()
      .then(action)
      .catch((e: unknown) => errorToast(e));
  };

  const toggleFullWidth = async () => {
    const next = !fullWidthOf(item);
    updateCachedItem(qc, s.id, (d) => ({ ...d, full_width: next }) as ItemDetail);
    try {
      await patch({ item: s, patch: { full_width: next } });
    } catch (e) {
      updateCachedItem(qc, s.id, (d) => ({ ...d, full_width: !next }) as ItemDetail);
      throw e;
    }
  };

  const entries: MenuEntry[] = [
    { label: item.starred ? 'Unstar' : 'Star', onSelect: run(() => actions.setStarred(s, !item.starred)) },
  ];
  if (s.kind === 'doc') {
    entries.push({ label: 'Full width', checkbox: true, checked: fullWidthOf(item), onSelect: run(toggleFullWidth) });
  }
  entries.push(
    { label: 'Duplicate', disabled: readOnly, onSelect: run(() => actions.duplicate(s)) },
    { label: 'Move…', disabled: readOnly, onSelect: () => openDialog({ kind: 'move', item: s }) },
    {
      label: 'Archive',
      disabled: readOnly || item.archived_at !== null,
      onSelect: run(async () => {
        await actions.archive(s);
      }),
    },
  );
  if (extraExports.length > 0) entries.push({ separator: true });
  for (const x of extraExports) entries.push({ label: `Export ${x.label}`, onSelect: run(() => x.run()) });
  entries.push({ separator: true }, {
    label: 'Delete',
    danger: true,
    disabled: readOnly,
    onSelect: run(async () => {
      if (!(await actions.trash(s))) return;
      const parent = item.breadcrumbs[item.breadcrumbs.length - 1];
      void navigate(parent ? itemPath(parent) : collectionPath(item.collection));
    }),
  });

  return (
    <div className="item-menu">
      <Menu label="Item menu" entries={entries} align="end">
        <MoreHorizontal size={16} aria-hidden />
      </Menu>
    </div>
  );
}
