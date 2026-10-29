// The item's "⋯" menu in the top bar (docs/ARCHITECTURE.md §9.3, §9.6.4): Star, Full width (docs), Duplicate, Move…,
// Archive, Export (the page's formats) and Delete. Props are frozen (PLAN W0b). The page renders it (in TopBar's
// `actions`) and adds its own export formats: docs Markdown and HTML (W4), boards .inkboard and SVG (W6). In a
// collection the caller may only read (D152) it offers Star and the exports alone.
import { MoreHorizontal } from 'lucide-react';
import { useNavigate } from 'react-router';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import { usePatchItem } from '../../api/queries/items';
import { itemWritable } from '../../lib/access';
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

/** The document's "Full width" setting (`ItemDetail.full_width`; `usePatchItem` updates the cached detail at once). */
export function fullWidthOf(item: ItemDetail): boolean {
  return item.full_width;
}

export function ItemMenu({ item, extraExports }: ItemMenuProps) {
  const navigate = useNavigate();
  const actions = useItemActions();
  const patch = usePatchItem().mutateAsync;
  const s = item.item;
  const readOnly = item.trashed_at !== null;
  const writable = itemWritable(item);

  const run = (action: () => void | Promise<void>) => () => {
    Promise.resolve()
      .then(action)
      .catch((e: unknown) => errorToast(e));
  };

  const fullWidth = fullWidthOf(item);
  // Optimistic: usePatchItem sets the cached detail's `full_width` first and restores it when the PATCH fails.
  const toggleFullWidth = async () => {
    await patch({ item: s, patch: { full_width: !fullWidth } });
  };

  const entries: MenuEntry[] = [
    { label: item.starred ? 'Unstar' : 'Star', onSelect: run(() => actions.setStarred(s, !item.starred)) },
  ];
  if (writable && s.kind === 'doc') {
    entries.push({ label: 'Full width', checkbox: true, checked: fullWidth, onSelect: run(toggleFullWidth) });
  }
  if (writable) {
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
  }
  if (extraExports.length > 0) entries.push({ separator: true });
  for (const x of extraExports) entries.push({ label: `Export ${x.label}`, onSelect: run(() => x.run()) });
  if (writable) {
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
  }

  return (
    <div className="item-menu">
      <Menu label="Item menu" entries={entries} align="end">
        <MoreHorizontal size={16} aria-hidden />
      </Menu>
    </div>
  );
}
