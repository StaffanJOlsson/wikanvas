// The Move dialog (docs/ARCHITECTURE.md §9.3, §9.6.2 "Move…"): pick a collection's top level or any item as the new
// parent; the item itself and its descendants are not offered. The item goes last under the new parent. Only the
// collections the caller may change are offered (D152: a move needs `write` on both collections).
import { useMemo, useState } from 'react';
import type { Collection } from '../../api/gen/Collection';
import type { ItemSummary } from '../../api/gen/ItemSummary';
import { useSidebar, useTrees } from '../../api/queries/tree';
import { canWrite } from '../../lib/access';
import { closeDialog } from '../../state/dialogs';
import { CollectionIcon } from '../collection/CollectionIcon';
import { KindIcon } from '../item/KindIcon';
import { useItemActions } from '../item/useItemActions';
import { buildTree, descendantIds, type TreeNode } from '../tree/buildTree';
import { indentX } from '../tree/hitZones';
import { Dialog } from '../ui/Dialog';

interface Target {
  key: string;
  collectionId: string;
  parentId: string | null;
  label: string;
  depth: number;
  node?: TreeNode;
}

export function MoveDialog({ item }: { item: ItemSummary }) {
  const sidebar = useSidebar();
  const all = sidebar.data?.collections;
  const collections = useMemo<Collection[]>(() => (all ?? []).filter((c) => canWrite(c.my_access)), [all]);
  const trees = useTrees(collections.map((c) => c.id));
  const actions = useItemActions();
  const [filter, setFilter] = useState('');
  const [choice, setChoice] = useState<string | null>(null);

  const targets = useMemo(() => {
    const out: Target[] = [];
    for (const c of collections) {
      out.push({ key: c.id, collectionId: c.id, parentId: null, label: c.name, depth: 0 });
      const t = trees.get(c.id);
      if (!t) continue;
      const forest = buildTree(c.id, t.items, c.child_order);
      const excluded = c.id === item.collection_id ? descendantIds(forest, item.id) : new Set<string>();
      excluded.add(item.id);
      const walk = (nodes: TreeNode[]) => {
        for (const n of nodes) {
          if (excluded.has(n.item.id)) continue;
          out.push({ key: n.item.id, collectionId: c.id, parentId: n.item.id, label: n.item.title || 'Untitled', depth: n.depth, node: n });
          walk(n.children);
        }
      };
      walk(forest.roots);
    }
    return out;
  }, [collections, trees, item.id, item.collection_id]);

  const f = filter.trim().toLocaleLowerCase();
  const shown = f ? targets.filter((t) => t.label.toLocaleLowerCase().includes(f)) : targets;
  const picked = targets.find((t) => t.key === choice);
  const unchanged = picked && picked.collectionId === item.collection_id && picked.parentId === item.parent_id;

  const submit = async () => {
    if (!picked || unchanged) return;
    closeDialog();
    await actions.moveTo(item, { collection_id: picked.collectionId, parent_id: picked.parentId });
  };

  return (
    <Dialog
      title={`Move “${item.title || 'Untitled'}”`}
      onClose={closeDialog}
      className="move-dialog"
      footer={
        <>
          <button type="button" className="button" onClick={closeDialog}>Cancel</button>
          <button type="button" className="button primary" disabled={!picked || unchanged} onClick={() => void submit()}>
            Move
          </button>
        </>
      }
    >
      <input
        className="input"
        aria-label="Filter places"
        placeholder="Filter…"
        data-autofocus
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
      />
      <ul className="move-targets" role="listbox" aria-label="New place">
        {shown.map((t) => (
          <li
            key={t.key}
            role="option"
            aria-selected={t.key === choice}
            tabIndex={0}
            className={t.key === choice ? 'selected' : undefined}
            style={{ paddingLeft: indentX(t.depth) }}
            onClick={() => setChoice(t.key)}
            onDoubleClick={() => {
              setChoice(t.key);
              void submit();
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                setChoice(t.key);
              }
            }}
          >
            {t.depth === 0
              ? <CollectionIcon collection={collections.find((c) => c.id === t.collectionId)!} />
              : <KindIcon kind={t.node!.item.kind} />}
            <span>{t.depth === 0 ? `${t.label} (top level)` : t.label}</span>
          </li>
        ))}
      </ul>
    </Dialog>
  );
}
