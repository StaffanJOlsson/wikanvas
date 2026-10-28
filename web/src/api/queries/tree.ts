// The sidebar and the collection trees (docs/ARCHITECTURE.md §7.2, §9.2, D21): `GET /sidebar`, the flat
// `GET /collections/{id}/tree` with its ETag (304 keeps the cache), and the structural mutations, applied
// optimistically and rolled back on error. Every mutation response replaces the affected rows with the server's rows.
import {
  useMutation,
  useQueries,
  useQuery,
  useQueryClient,
  type QueryClient,
  type QueryKey,
  type UseMutationResult,
  type UseQueryResult,
} from '@tanstack/react-query';
import { applyMove, applyRow } from '../../components/tree/applyMove';
import type { Move } from '../../components/tree/positions';
import { api, apiResponse, type ApiError } from '../client';
import type { Collection } from '../gen/Collection';
import type { CollectionDeleted } from '../gen/CollectionDeleted';
import type { CollectionPatch } from '../gen/CollectionPatch';
import type { CollectionTree } from '../gen/CollectionTree';
import type { ItemDetail } from '../gen/ItemDetail';
import type { ItemSummary } from '../gen/ItemSummary';
import type { NewCollection } from '../gen/NewCollection';
import type { Position } from '../gen/Position';
import type { Sidebar } from '../gen/Sidebar';

export const sidebarKey = ['sidebar'] as const;

export function treeKey(collectionId: string): readonly ['tree', string] {
  return ['tree', collectionId] as const;
}

export function childrenKey(itemId: string): readonly ['children', string] {
  return ['children', itemId] as const;
}

/** A cached tree: the response plus its ETag (null after an optimistic edit, so the next fetch is unconditional). */
export interface TreeData extends CollectionTree {
  etag: string | null;
}

const enc = encodeURIComponent;

export function fetchSidebar(signal?: AbortSignal): Promise<Sidebar> {
  return api<Sidebar>('GET', '/sidebar', { signal });
}

/** `GET /sidebar`: collections in order, stars in order, and the drafts count (unused in M1's UI, D23). */
export function useSidebar(): UseQueryResult<Sidebar, ApiError> {
  return useQuery<Sidebar, ApiError>({ queryKey: sidebarKey, queryFn: ({ signal }) => fetchSidebar(signal) });
}

/** `GET /collections/{id}/tree` with `If-None-Match`; a 304 returns the cached tree. */
export async function fetchTree(qc: QueryClient, collectionId: string, signal?: AbortSignal): Promise<TreeData> {
  const cached = qc.getQueryData<TreeData>(treeKey(collectionId));
  const path = `/collections/${enc(collectionId)}/tree`;
  const etag = cached?.etag ?? undefined;
  const res = await apiResponse<CollectionTree>('GET', path, etag ? { signal, etag } : { signal });
  if (res.notModified) {
    if (cached) return cached;
    const fresh = await apiResponse<CollectionTree>('GET', path, { signal });
    return { ...fresh.data, etag: fresh.etag };
  }
  return { ...res.data, etag: res.etag };
}

function treeQuery(qc: QueryClient, collectionId: string) {
  return {
    queryKey: treeKey(collectionId) as QueryKey,
    queryFn: ({ signal }: { signal: AbortSignal }) => fetchTree(qc, collectionId, signal),
  };
}

/** One collection's whole live tree, flat (D21). Disabled without an id. */
export function useTree(collectionId: string | null | undefined): UseQueryResult<TreeData, ApiError> {
  const qc = useQueryClient();
  return useQuery<TreeData, ApiError>({
    ...treeQuery(qc, collectionId ?? ''),
    enabled: !!collectionId,
  });
}

/** The trees of several collections (the expanded ones in the sidebar), by collection id. */
export function useTrees(collectionIds: readonly string[]): ReadonlyMap<string, TreeData | undefined> {
  const qc = useQueryClient();
  const results = useQueries({ queries: collectionIds.map((cid) => treeQuery(qc, cid)) });
  const map = new Map<string, TreeData | undefined>();
  collectionIds.forEach((cid, i) => map.set(cid, results[i]?.data as TreeData | undefined));
  return map;
}

/** `GET /items/{id}/children` (the nested list under a document, W4). */
export function useChildren(itemId: string | null | undefined): UseQueryResult<ItemSummary[], ApiError> {
  return useQuery<ItemSummary[], ApiError>({
    queryKey: childrenKey(itemId ?? ''),
    queryFn: ({ signal }) => api<ItemSummary[]>('GET', `/items/${enc(itemId!)}/children`, { signal }),
    enabled: !!itemId,
  });
}

/** A collection by id or url_id (the collection page). Uses the sidebar's copy as placeholder. */
export function useCollection(ref: string | null | undefined): UseQueryResult<Collection, ApiError> {
  const qc = useQueryClient();
  return useQuery<Collection, ApiError>({
    queryKey: ['collection', ref ?? ''],
    queryFn: ({ signal }) => api<Collection>('GET', `/collections/${enc(ref!)}`, { signal }),
    enabled: !!ref,
    placeholderData: () =>
      qc.getQueryData<Sidebar>(sidebarKey)?.collections.find((c) => c.id === ref || c.url_id === ref),
  });
}

// ---- cache helpers ----

/** Every cached tree, by collection id. */
export function cachedTrees(qc: QueryClient): Map<string, TreeData> {
  const out = new Map<string, TreeData>();
  for (const [key, data] of qc.getQueriesData<TreeData>({ queryKey: ['tree'] })) {
    if (data && typeof key[1] === 'string') out.set(key[1], data);
  }
  return out;
}

/** Sets a tree's items (and drops its ETag: the cache no longer equals any server version). */
export function setTreeItems(qc: QueryClient, collectionId: string, items: ItemSummary[]): void {
  qc.setQueryData<TreeData>(treeKey(collectionId), (d) => (d ? { ...d, items, etag: null } : d));
}

/** The cached summary of an item, from any tree, the stars, or a loaded item. */
export function findCachedSummary(qc: QueryClient, itemId: string): ItemSummary | undefined {
  for (const t of cachedTrees(qc).values()) {
    const hit = t.items.find((i) => i.id === itemId);
    if (hit) return hit;
  }
  const star = qc.getQueryData<Sidebar>(sidebarKey)?.stars.find((s) => s.id === itemId);
  if (star) return star;
  for (const [, d] of qc.getQueriesData<ItemDetail>({ queryKey: ['item'] })) {
    if (d?.item.id === itemId) return d.item;
  }
  return undefined;
}

/** Replaces an item's row wherever it is cached (trees, stars, item details). */
export function replaceCachedSummary(qc: QueryClient, row: ItemSummary): void {
  for (const [cid, t] of cachedTrees(qc)) {
    if (t.items.some((i) => i.id === row.id)) {
      qc.setQueryData<TreeData>(treeKey(cid), { ...t, items: applyRow(t.items, row) });
    }
  }
  qc.setQueryData<Sidebar>(sidebarKey, (s) =>
    s ? { ...s, stars: s.stars.map((x) => (x.id === row.id ? { ...x, ...row } : x)) } : s);
  qc.setQueriesData<ItemDetail>({ queryKey: ['item'] }, (d) =>
    d && d.item.id === row.id ? { ...d, item: { ...d.item, ...row } } : d);
}

type Snapshot = [QueryKey, unknown][];

function snapshot(qc: QueryClient, keys: QueryKey[]): Snapshot {
  return keys.map((k) => [k, qc.getQueryData(k)]);
}

function restore(qc: QueryClient, snap: Snapshot | undefined): void {
  for (const [k, v] of snap ?? []) qc.setQueryData(k, v);
}

// ---- item moves ----

export interface MoveItemVars {
  item: ItemSummary;
  to: Move;
}

/**
 * `POST /items/{id}/move`, optimistic: the row (and its subtree) moves in the cached trees at once; an error (409
 * `cycle`, `parent_invalid`, …) restores them. Callers show the toasts (`useItemActions`).
 */
export function useMoveItem(): UseMutationResult<ItemSummary, ApiError, MoveItemVars, { snap: Snapshot }> {
  const qc = useQueryClient();
  return useMutation<ItemSummary, ApiError, MoveItemVars, { snap: Snapshot }>({
    mutationFn: ({ item, to }) => api<ItemSummary>('POST', `/items/${enc(item.id)}/move`, { json: to }),
    onMutate: async ({ item, to }) => {
      const cids = [...new Set([item.collection_id, to.collection_id])];
      await Promise.all(cids.map((cid) => qc.cancelQueries({ queryKey: treeKey(cid) })));
      const snap = snapshot(qc, cids.map((cid) => treeKey(cid)));
      const lists = new Map<string, ItemSummary[]>();
      for (const cid of cids) {
        const t = qc.getQueryData<TreeData>(treeKey(cid));
        if (t) lists.set(cid, t.items);
      }
      for (const [cid, items] of applyMove(lists, item.id, to)) setTreeItems(qc, cid, items);
      return { snap };
    },
    onError: (_e, _vars, ctx) => restore(qc, ctx?.snap),
    onSuccess: (row) => replaceCachedSummary(qc, row),
    onSettled: (_row, _e, { item, to }) => {
      for (const cid of new Set([item.collection_id, to.collection_id])) {
        void qc.invalidateQueries({ queryKey: treeKey(cid) });
      }
      void qc.invalidateQueries({ queryKey: sidebarKey });
      void qc.invalidateQueries({ queryKey: ['item'] });
      void qc.invalidateQueries({ queryKey: ['children'] });
    },
  });
}

// ---- collections ----

/** Moves a collection within the list: after `after_id`, else before `before_id`, else last. */
export function reorder<T extends { id: string }>(list: readonly T[], id: string, position: Position): T[] {
  const moving = list.find((x) => x.id === id);
  if (!moving) return [...list];
  const rest = list.filter((x) => x.id !== id);
  let at = rest.length;
  const after = position.after_id ? rest.findIndex((x) => x.id === position.after_id) : -1;
  const before = position.before_id ? rest.findIndex((x) => x.id === position.before_id) : -1;
  if (after >= 0) at = after + 1;
  else if (before >= 0) at = before;
  rest.splice(at, 0, moving);
  return rest;
}

export interface MoveCollectionVars {
  collection: Collection;
  position: Position;
}

/** `POST /collections/{id}/move`, optimistic in the sidebar. */
export function useMoveCollection(): UseMutationResult<Collection, ApiError, MoveCollectionVars, { snap: Snapshot }> {
  const qc = useQueryClient();
  return useMutation<Collection, ApiError, MoveCollectionVars, { snap: Snapshot }>({
    mutationFn: ({ collection, position }) =>
      api<Collection>('POST', `/collections/${enc(collection.id)}/move`, { json: { position } }),
    onMutate: async ({ collection, position }) => {
      await qc.cancelQueries({ queryKey: sidebarKey });
      const snap = snapshot(qc, [sidebarKey]);
      qc.setQueryData<Sidebar>(sidebarKey, (s) =>
        s ? { ...s, collections: reorder(s.collections, collection.id, position) } : s);
      return { snap };
    },
    onError: (_e, _v, ctx) => restore(qc, ctx?.snap),
    onSuccess: (c) => replaceCachedCollection(qc, c),
    onSettled: () => qc.invalidateQueries({ queryKey: sidebarKey }),
  });
}

/** Replaces a collection wherever it is cached (sidebar, its tree, the collection page, item details). */
export function replaceCachedCollection(qc: QueryClient, c: Collection): void {
  qc.setQueryData<Sidebar>(sidebarKey, (s) =>
    s ? { ...s, collections: s.collections.map((x) => (x.id === c.id ? c : x)) } : s);
  qc.setQueryData<TreeData>(treeKey(c.id), (t) => (t ? { ...t, collection: c } : t));
  qc.setQueriesData<Collection>({ queryKey: ['collection'] }, (x) => (x && x.id === c.id ? c : x));
  qc.setQueriesData<ItemDetail>({ queryKey: ['item'] }, (d) =>
    d && d.collection.id === c.id ? { ...d, collection: c } : d);
}

/** `POST /collections` (201). The new collection is appended to the sidebar. */
export function useCreateCollection(): UseMutationResult<Collection, ApiError, NewCollection> {
  const qc = useQueryClient();
  return useMutation<Collection, ApiError, NewCollection>({
    mutationFn: (body) => api<Collection>('POST', '/collections', { json: body }),
    onSuccess: (c) => {
      qc.setQueryData<Sidebar>(sidebarKey, (s) =>
        s && !s.collections.some((x) => x.id === c.id) ? { ...s, collections: [...s.collections, c] } : s);
      void qc.invalidateQueries({ queryKey: sidebarKey });
    },
  });
}

export interface PatchCollectionVars {
  collection: Collection;
  patch: CollectionPatch;
}

/** `PATCH /collections/{id}` (name, icon, colour, description, sort), optimistic everywhere it is cached. */
export function usePatchCollection(): UseMutationResult<Collection, ApiError, PatchCollectionVars, { snap: Snapshot }> {
  const qc = useQueryClient();
  return useMutation<Collection, ApiError, PatchCollectionVars, { snap: Snapshot }>({
    mutationFn: ({ collection, patch }) => api<Collection>('PATCH', `/collections/${enc(collection.id)}`, { json: patch }),
    onMutate: async ({ collection, patch }) => {
      await qc.cancelQueries({ queryKey: sidebarKey });
      const snap = snapshot(qc, [sidebarKey, treeKey(collection.id)]);
      const next: Collection = { ...collection };
      if (patch.name != null) next.name = patch.name;
      if (patch.description != null) next.description = patch.description;
      if (patch.color != null) next.color = patch.color;
      if (patch.icon !== undefined) next.icon = patch.icon;
      if (patch.child_order != null) next.child_order = patch.child_order;
      replaceCachedCollection(qc, next);
      return { snap };
    },
    onError: (_e, _v, ctx) => restore(qc, ctx?.snap),
    onSuccess: (c) => replaceCachedCollection(qc, c),
    onSettled: (_c, _e, { collection, patch }) => {
      void qc.invalidateQueries({ queryKey: sidebarKey });
      if (patch.child_order != null) void qc.invalidateQueries({ queryKey: treeKey(collection.id) });
      void qc.invalidateQueries({ queryKey: ['collection'] });
    },
  });
}

/** `DELETE /collections/{id}`: a soft delete, with its items, into Trash. Optimistic in the sidebar. */
export function useDeleteCollection(): UseMutationResult<CollectionDeleted, ApiError, Collection, { snap: Snapshot }> {
  const qc = useQueryClient();
  return useMutation<CollectionDeleted, ApiError, Collection, { snap: Snapshot }>({
    mutationFn: (c) => api<CollectionDeleted>('DELETE', `/collections/${enc(c.id)}`),
    onMutate: async (c) => {
      await qc.cancelQueries({ queryKey: sidebarKey });
      const snap = snapshot(qc, [sidebarKey]);
      qc.setQueryData<Sidebar>(sidebarKey, (s) =>
        s ? {
          ...s,
          collections: s.collections.filter((x) => x.id !== c.id),
          stars: s.stars.filter((x) => x.collection_id !== c.id),
        } : s);
      return { snap };
    },
    onError: (_e, _v, ctx) => restore(qc, ctx?.snap),
    onSettled: (_r, _e, c) => {
      qc.removeQueries({ queryKey: treeKey(c.id) });
      void qc.invalidateQueries({ queryKey: sidebarKey });
      void qc.invalidateQueries({ queryKey: ['list'] });
      void qc.invalidateQueries({ queryKey: ['item'] });
    },
  });
}

/** `POST /collections/{id}/restore` (from Trash). */
export function useRestoreCollection(): UseMutationResult<Collection, ApiError, string> {
  const qc = useQueryClient();
  return useMutation<Collection, ApiError, string>({
    mutationFn: (id) => api<Collection>('POST', `/collections/${enc(id)}/restore`),
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: sidebarKey });
      void qc.invalidateQueries({ queryKey: ['list'] });
      void qc.invalidateQueries({ queryKey: ['tree'] });
    },
  });
}
