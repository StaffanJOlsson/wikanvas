// The COLLECTIONS tree (docs/ARCHITECTURE.md §9.3, §9.6.2, §9.6.3): role=tree over the flat visible-row model
// (virtualized above 300 rows, D21), with persisted expand state, the current item highlighted and its ancestors
// expanded, hover menus, inline rename, the ARIA tree keyboard and drag and drop (pragmatic-dnd).
import { monitorForElements } from '@atlaskit/pragmatic-drag-and-drop/adapter/element-adapter';
import { useVirtualizer } from '@tanstack/react-virtual';
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type RefObject,
} from 'react';
import { useLocation, useNavigate } from 'react-router';
import type { Collection } from '../../api/gen/Collection';
import { useItem } from '../../api/queries/items';
import { useTrees, type TreeData } from '../../api/queries/tree';
import { hostKeyOf, MAIN_ID } from '../../app/hotkeys';
import { collectionPath, itemPath, urlIdFromSlugId } from '../../lib/urls';
import { expandAll, setExpanded, toggleExpanded, useExpandedIds } from '../../state/expanded';
import { useCollectionActions } from '../collection/useCollectionActions';
import { useItemActions } from '../item/useItemActions';
import { buildTree, type Forest } from '../tree/buildTree';
import { resolveItemDrop, type DropIndicator, type DropInstruction } from '../tree/drop';
import { edgeZone } from '../tree/hitZones';
import { keyboardInstruction } from '../tree/positions';
import { VIRTUALIZE_ABOVE, visibleRows, type Row } from '../tree/visibleRows';
import { DRAG_COLLECTION, DRAG_ITEM, TreeRow } from './TreeRow';
import { TypeAhead, treeKeyAction } from './treeKeyboard';

/** Hovering "inside" a collapsed row for this long expands it (§9.6.3). */
export const AUTO_EXPAND_MS = 700;
export const ROW_HEIGHT = 30;

const forestCache = new WeakMap<TreeData, { order: string; forest: Forest }>();

function forestOf(c: Collection, tree: TreeData): Forest {
  const hit = forestCache.get(tree);
  if (hit && hit.order === c.child_order) return hit.forest;
  const forest = buildTree(c.id, tree.items, c.child_order);
  forestCache.set(tree, { order: c.child_order, forest });
  return forest;
}

/** The item or collection the current route shows. */
export function currentTarget(pathname: string): { kind: 'item' | 'collection'; urlId: string } | null {
  const m = /^\/(doc|board|collection)\/([^/]+)\/?$/.exec(pathname);
  if (!m) return null;
  const urlId = urlIdFromSlugId(m[2]);
  if (!urlId) return null;
  return { kind: m[1] === 'collection' ? 'collection' : 'item', urlId };
}

/** useMemo over a dependency list whose length may change (one entry per collection). */
function useListMemo<T>(compute: () => T, deps: readonly unknown[]): T {
  const memo = useRef<{ deps: readonly unknown[]; value: T } | null>(null);
  const prev = memo.current;
  if (prev && prev.deps.length === deps.length && prev.deps.every((d, i) => Object.is(d, deps[i]))) return prev.value;
  const value = compute();
  memo.current = { deps, value };
  return value;
}

function sameIndicator(a: DropIndicator | null, b: DropIndicator | null): boolean {
  return a === b || (!!a && !!b && a.rowKey === b.rowKey && a.kind === b.kind && a.depth === b.depth);
}

type PendingDrop =
  | { kind: 'item'; instruction: DropInstruction; itemId: string; collectionId: string }
  | { kind: 'collection'; collectionId: string; targetId: string; edge: 'before' | 'after' };

export interface CollectionsTreeProps {
  collections: readonly Collection[];
  starredIds: ReadonlySet<string>;
  /** The sidebar's scroll container (virtualization). */
  scrollRef: RefObject<HTMLElement | null>;
  labelledBy?: string;
}

export function CollectionsTree({ collections, starredIds, scrollRef, labelledBy }: CollectionsTreeProps) {
  const navigate = useNavigate();
  const location = useLocation();
  const actions = useItemActions();
  const collectionActions = useCollectionActions();
  const expanded = useExpandedIds();
  const [dragCollection, setDragCollection] = useState<string | null>(null);
  const [indicator, setIndicatorState] = useState<DropIndicator | null>(null);
  const setIndicator = useCallback((next: DropIndicator | null) => {
    setIndicatorState((cur) => (sameIndicator(cur, next) ? cur : next));
  }, []);
  const [focusKey, setFocusKey] = useState<string | null>(null);
  const [renamingKey, setRenamingKey] = useState<string | null>(null);
  const [menuKey, setMenuKey] = useState<string | null>(null);
  const treeEl = useRef<HTMLDivElement>(null);
  const typeAhead = useRef(new TypeAhead());
  const pendingFocus = useRef<string | null>(null);

  // ---- data ----
  const loadIds = useMemo(() => collections.filter((c) => expanded.has(c.id)).map((c) => c.id), [collections, expanded]);
  const trees = useTrees(loadIds);
  const forests = new Map<string, Forest | undefined>();
  for (const c of collections) {
    const t = trees.get(c.id);
    forests.set(c.id, t ? forestOf(c, t) : undefined);
  }
  const rows = useListMemo(
    () => visibleRows({
      collections,
      forests,
      expanded,
      forceCollapsed: dragCollection ? new Set([dragCollection]) : undefined,
    }),
    [collections, expanded, dragCollection, ...collections.map((c) => forests.get(c.id))],
  );

  // ---- the current item: highlighted, its collection and ancestors expanded (§9.6.2) ----
  const current = currentTarget(location.pathname);
  const detail = useItem(current?.kind === 'item' ? current.urlId : null);
  const revealed = useRef<string | null>(null);
  const d = detail.data;
  useEffect(() => {
    if (!d || revealed.current === d.item.id || d.item.url_id !== current?.urlId) return;
    revealed.current = d.item.id;
    expandAll([d.collection.id, ...d.breadcrumbs.map((c) => c.id)]);
  }, [d, current?.urlId]);
  const currentKey = useMemo(() => {
    if (!current) return null;
    const hit = rows.find((r) => (current.kind === 'item'
      ? r.kind === 'item' && r.item.url_id === current.urlId
      : r.kind === 'collection' && r.collection.url_id === current.urlId));
    return hit?.key ?? null;
  }, [rows, current?.kind, current?.urlId]);

  const tabKey = (focusKey && rows.some((r) => r.key === focusKey) ? focusKey : null) ?? currentKey ?? rows[0]?.key ?? null;

  // ---- virtualization ----
  const virtual = rows.length > VIRTUALIZE_ABOVE;
  const [scrollMargin, setScrollMargin] = useState(0);
  // The tree's offset in the scroll area (below STARRED, which can grow): checked after every render.
  useLayoutEffect(() => {
    if (!virtual || !treeEl.current || !scrollRef.current) return;
    const t = treeEl.current.getBoundingClientRect().top;
    const s = scrollRef.current.getBoundingClientRect().top;
    const margin = Math.round(t - s + scrollRef.current.scrollTop);
    if (margin !== scrollMargin) setScrollMargin(margin);
  });
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
    scrollMargin,
    enabled: virtual,
  });

  // ---- focus ----
  const focusRow = useCallback((key: string) => {
    setFocusKey(key);
    pendingFocus.current = key;
    const i = rows.findIndex((r) => r.key === key);
    if (virtual && i >= 0) virtualizer.scrollToIndex(i, { align: 'auto' });
  }, [rows, virtual, virtualizer]);

  useEffect(() => {
    const key = pendingFocus.current;
    if (!key || !treeEl.current) return;
    const el = treeEl.current.querySelector<HTMLElement>(`[data-key="${CSS.escape(key)}"]`);
    if (el) {
      pendingFocus.current = null;
      el.focus();
    }
  });

  // ---- keyboard (§9.6.3) ----
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;
    if (target.getAttribute('role') !== 'treeitem') return;
    const index = rows.findIndex((r) => r.key === target.dataset.key);
    if (index < 0) return;
    const native = e.nativeEvent;
    const altGraph = typeof native.getModifierState === 'function' && native.getModifierState('AltGraph');
    const action = treeKeyAction(rows, index, { ...hostKeyOf(native), altGraph }, typeAhead.current);
    if (!action) return;
    e.preventDefault();
    e.stopPropagation();
    if (action.type === 'focus-main') {
      document.getElementById(MAIN_ID)?.focus();
      return;
    }
    const row = rows[action.index]!;
    switch (action.type) {
      case 'focus':
        focusRow(row.key);
        return;
      case 'expand':
        setExpanded(row.key, true);
        return;
      case 'collapse':
        setExpanded(row.key, false);
        return;
      case 'open': {
        const path = row.kind === 'item' ? itemPath(row.item) : collectionPath(row.collection);
        if (action.newTab) window.open(path, '_blank', 'noopener');
        else void navigate(path);
        return;
      }
      case 'rename':
        setRenamingKey(row.key);
        return;
      case 'menu':
        setMenuKey(row.key);
        return;
      case 'delete':
        if (row.kind !== 'item') return;
        void actions.trash(row.item).then((done) => {
          if (!done) return focusRow(row.key);
          const next = rows[action.index + 1] ?? rows[action.index - 1];
          if (next && next.key !== row.key) focusRow(next.key);
        });
        return;
      case 'move': {
        if (row.kind !== 'item') return;
        const forest = forests.get(row.collectionId);
        const instruction = forest && keyboardInstruction(forest, row.item.id, action.dir);
        if (!instruction) return;
        if (action.dir === 'indent' && instruction.type === 'child') setExpanded(instruction.targetId, true);
        pendingFocus.current = row.key;
        void actions.drop(row.item, instruction, { undoable: false }).then(() => focusRow(row.key));
        return;
      }
    }
  };

  // ---- drag and drop (§9.6.3) ----
  const live = useRef({ rows, forests, actions, collectionActions, collections });
  live.current = { rows, forests, actions, collectionActions, collections };

  useEffect(() => {
    let pending: PendingDrop | null = null;
    let expandTimer: { key: string; timer: ReturnType<typeof setTimeout> } | null = null;
    const clearExpand = () => {
      if (expandTimer) clearTimeout(expandTimer.timer);
      expandTimer = null;
    };
    const scheduleExpand = (key: string) => {
      if (expandTimer?.key === key) return;
      clearExpand();
      expandTimer = { key, timer: setTimeout(() => setExpanded(key, true), AUTO_EXPAND_MS) };
    };

    type Args = Parameters<NonNullable<Parameters<typeof monitorForElements>[0]['onDrag']>>[0];
    const update = ({ source, location: loc }: Args) => {
      const { rows: rs, forests: fs } = live.current;
      const target = loc.current.dropTargets.find((t) => typeof t.data.quireRow === 'string');
      const index = target ? rs.findIndex((r) => r.key === target.data.quireRow) : -1;
      if (!target || index < 0) {
        pending = null;
        clearExpand();
        setIndicator(null);
        return;
      }
      const rect = target.element.getBoundingClientRect();
      const { clientX, clientY } = loc.current.input;
      if (source.data.quireDrag === DRAG_ITEM) {
        const itemId = String(source.data.itemId);
        const collectionId = String(source.data.collectionId);
        const resolved = resolveItemDrop({
          rows: rs,
          index,
          dragged: { id: itemId, collectionId },
          forests: fs,
          clientX,
          clientY,
          rect: { top: rect.top, height: rect.height, left: rect.left },
        });
        pending = resolved ? { kind: 'item', instruction: resolved.instruction, itemId, collectionId } : null;
        setIndicator(resolved?.indicator ?? null);
        const row = rs[index]!;
        const into = resolved && (resolved.instruction.type === 'child' || resolved.instruction.type === 'collection');
        if (into && row.hasChildren && !row.expanded && resolved.indicator.rowKey === row.key) scheduleExpand(row.key);
        else clearExpand();
        return;
      }
      if (source.data.quireDrag === DRAG_COLLECTION) {
        const draggedId = String(source.data.collectionId);
        let start = index;
        while (start > 0 && rs[start]!.kind !== 'collection') start--;
        let end = start + 1;
        while (end < rs.length && rs[end]!.kind !== 'collection') end++;
        const crow = rs[start]!;
        if (crow.kind !== 'collection' || crow.key === draggedId) {
          pending = null;
          setIndicator(null);
          return;
        }
        const edge = index === start ? edgeZone(clientY, rect) : index - start >= (end - start) / 2 ? 'after' : 'before';
        pending = { kind: 'collection', collectionId: draggedId, targetId: crow.key, edge };
        setIndicator(edge === 'before'
          ? { rowKey: crow.key, kind: 'line-before', depth: 0 }
          : { rowKey: rs[end - 1]!.key, kind: 'line-after', depth: 0 });
      }
    };

    return monitorForElements({
      canMonitor: ({ source }) => source.data.quireDrag === DRAG_ITEM || source.data.quireDrag === DRAG_COLLECTION,
      onDragStart: ({ source }) => {
        if (source.data.quireDrag === DRAG_COLLECTION) setDragCollection(String(source.data.collectionId));
      },
      onDrag: update,
      onDropTargetChange: update,
      onDrop: () => {
        clearExpand();
        setIndicator(null);
        setDragCollection(null);
        const p = pending;
        pending = null;
        if (!p) return;
        const { forests: fs, actions: act, collectionActions: cact, collections: cs } = live.current;
        if (p.kind === 'item') {
          const item = fs.get(p.collectionId)?.byId.get(p.itemId)?.item;
          if (item) void act.drop(item, p.instruction);
        } else {
          const c = cs.find((x) => x.id === p.collectionId);
          if (c) void cact.reorder(c, p.targetId, p.edge);
        }
      },
    });
  }, [setIndicator]);

  // ---- rendering ----
  const renderRow = (row: Row, style?: CSSProperties) => (
    <TreeRow
      key={row.key}
      row={row}
      style={style}
      focused={row.key === tabKey}
      current={row.key === currentKey}
      renaming={row.key === renamingKey}
      menuOpen={row.key === menuKey}
      starred={row.kind === 'item' && starredIds.has(row.item.id)}
      indicator={indicator?.rowKey === row.key ? indicator : null}
      actions={actions}
      collectionActions={collectionActions}
      onToggle={(r) => toggleExpanded(r.key)}
      onFocusRow={setFocusKey}
      onRename={(r) => setRenamingKey(r.key)}
      onRenameDone={(r, title) => {
        setRenamingKey(null);
        if (title !== null && r.kind === 'item') void actions.rename(r.item, title);
        focusRow(r.key);
      }}
      onMenuClosed={() => setMenuKey(null)}
    />
  );

  return (
    <div
      ref={treeEl}
      role="tree"
      aria-labelledby={labelledBy}
      className="collections-tree"
      onKeyDown={onKeyDown}
      style={virtual ? { height: virtualizer.getTotalSize(), position: 'relative' } : undefined}
    >
      {virtual
        ? virtualizer.getVirtualItems().map((v) => renderRow(rows[v.index]!, {
          position: 'absolute',
          top: 0,
          left: 0,
          right: 0,
          height: ROW_HEIGHT,
          transform: `translateY(${v.start - scrollMargin}px)`,
        }))
        : rows.map((row) => renderRow(row))}
    </div>
  );
}
