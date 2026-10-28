// The item's "⋯" menu in the top bar (docs/ARCHITECTURE.md §9.3, §9.6.4). Props are frozen (PLAN W0b). The page
// renders it (in TopBar's `actions`) and adds its own export formats: docs Markdown and HTML (W4), boards .inkboard
// and SVG (W6). W3 completes the menu (Full width, Move…, confirmations, toasts, keyboard navigation).
import { useQueryClient } from '@tanstack/react-query';
import { MoreHorizontal } from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import { api } from '../../api/client';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import type { ItemSummary } from '../../api/gen/ItemSummary';
import { useStar } from '../../api/queries/items';
import { collectionPath, itemPath } from '../../lib/urls';

/** An export format the page adds to the menu's Export section. */
export interface ExtraExport {
  label: string;
  run(): void | Promise<void>;
}

export interface ItemMenuProps {
  item: ItemDetail;
  extraExports: ExtraExport[];
}

export function ItemMenu({ item, extraExports }: ItemMenuProps) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const menuId = useId();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const star = useStar();
  const id = item.item.id;
  const path = `/items/${encodeURIComponent(id)}`;

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (root.current && !root.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);

  const refreshTree = () => {
    void qc.invalidateQueries({ queryKey: ['tree', item.item.collection_id] });
    void qc.invalidateQueries({ queryKey: ['sidebar'] });
  };

  const run = (action: () => void | Promise<void>) => () => {
    setOpen(false);
    Promise.resolve()
      .then(action)
      .catch((e: unknown) => console.error('item action failed', e));
  };

  const actions: { label: string; run: () => void | Promise<void> }[] = [
    {
      label: item.starred ? 'Unstar' : 'Star',
      run: () => star.mutateAsync({ itemId: id, starred: !item.starred }),
    },
    {
      label: 'Duplicate',
      run: async () => {
        const copy = await api<ItemSummary>('POST', `${path}/duplicate`);
        refreshTree();
        navigate(itemPath(copy));
      },
    },
    {
      label: 'Archive',
      run: async () => {
        await api('POST', `${path}/archive`);
        refreshTree();
        void qc.invalidateQueries({ queryKey: ['item'] });
      },
    },
  ];

  return (
    <div className="menu item-menu" ref={root}>
      <button
        type="button"
        className="icon-button"
        aria-label="Item menu"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        onClick={() => setOpen((o) => !o)}
      >
        <MoreHorizontal size={16} aria-hidden />
      </button>
      {open && (
        <div role="menu" id={menuId} aria-label="Item menu">
          {actions.map((a) => (
            <button key={a.label} type="button" role="menuitem" onClick={run(a.run)}>
              {a.label}
            </button>
          ))}
          {extraExports.length > 0 && <div role="separator" />}
          {extraExports.map((x) => (
            <button key={x.label} type="button" role="menuitem" onClick={run(() => x.run())}>
              Export {x.label}
            </button>
          ))}
          <div role="separator" />
          <button
            type="button"
            role="menuitem"
            onClick={run(async () => {
              await api('DELETE', path);
              refreshTree();
              navigate(collectionPath(item.collection));
            })}
          >
            Delete
          </button>
        </div>
      )}
    </div>
  );
}
