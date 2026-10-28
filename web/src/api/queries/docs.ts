// Documents over the API (docs/ARCHITECTURE.md §6.1, §7.2, §7.5): the Yjs sync the doc session uses, the export
// URL, the title search the mention menu uses, the nested-items list, and the ProseMirror JSON a board card shows
// (`['docJson', id]`, §18.5 "Static cards").
import { useQuery, useQueryClient, type QueryClient, type UseQueryResult } from '@tanstack/react-query';
import { api, apiResponse, apiUrl, type ApiError } from '../client';
import type { DocJson } from '../gen/DocJson';
import type { DocState } from '../gen/DocState';
import type { DocUpdateResult } from '../gen/DocUpdateResult';
import type { ItemSummary } from '../gen/ItemSummary';
import type { TitleHit } from '../gen/TitleHit';

const path = (itemId: string) => `/items/${encodeURIComponent(itemId)}/doc`;

/** Base64 of bytes (the API's encoding of Yjs updates and state vectors). */
export function toBase64(bytes: Uint8Array): string {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) {
    s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(s);
}

/** Bytes of base64. */
export function fromBase64(s: string): Uint8Array {
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** `GET /items/{id}/doc`, with `since` (a Yjs state vector) for only what it lacks. */
export function fetchDoc(itemId: string, since?: Uint8Array, signal?: AbortSignal): Promise<DocState> {
  const q = since ? `?since=${encodeURIComponent(toBase64(since))}` : '';
  return api<DocState>('GET', `${path(itemId)}${q}`, { signal });
}

/** `POST /items/{id}/doc/updates`. */
export function postUpdate(
  itemId: string,
  update: Uint8Array,
  pmJson?: unknown,
  keepalive?: boolean,
): Promise<DocUpdateResult> {
  const json: { update: string; pm_json?: unknown } = { update: toBase64(update) };
  if (pmJson !== undefined) json.pm_json = pmJson;
  return api<DocUpdateResult>('POST', `${path(itemId)}/updates`, { json, keepalive });
}

/** The Markdown export: `GET /api/v1/items/{id}/doc/markdown?download=true` (server-side, §6.4, D49). */
export function markdownDownloadUrl(itemId: string): string {
  return apiUrl(`${path(itemId)}/markdown?download=true`);
}

/** `GET /api/v1/search/titles?q&limit` (W1): the mention menu and "Embed board…". */
export function searchTitles(q: string, limit = 8, signal?: AbortSignal): Promise<TitleHit[]> {
  return api<TitleHit[]>('GET', `/search/titles?q=${encodeURIComponent(q)}&limit=${limit}`, { signal });
}

/** The item's children (`GET /items/{id}/children`, W1), for the nested list below the body. */
export function useChildren(itemId: string): UseQueryResult<ItemSummary[], ApiError> {
  return useQuery<ItemSummary[], ApiError>({
    queryKey: ['children', itemId],
    queryFn: ({ signal }) => api<ItemSummary[]>('GET', `/items/${encodeURIComponent(itemId)}/children`, { signal }),
  });
}

// ------------------------------------------------------------------------------------------------- board cards

/** A cached `GET /items/{id}/doc/prosemirror`: the body and the ETag it came with (`"d{seq}"`). */
export interface CachedDocJson {
  json: DocJson;
  /** Sent as `If-None-Match` on the next fetch; null when the entry was written locally (an editor's last JSON). */
  etag: string | null;
}

/** `['docJson', id]` (M1.1, §18.5). */
export function docJsonKey(itemId: string): readonly ['docJson', string] {
  return ['docJson', itemId] as const;
}

/**
 * `GET /items/{id}/doc/prosemirror`, revalidated with `If-None-Match` against `prev`: a 304 keeps `prev`.
 */
export async function fetchDocJson(itemId: string, prev?: CachedDocJson, signal?: AbortSignal): Promise<CachedDocJson> {
  const r = await apiResponse<DocJson>('GET', `${path(itemId)}/prosemirror`, {
    signal,
    ...(prev?.etag ? { etag: prev.etag } : {}),
  });
  if (r.notModified && prev) return prev;
  return { json: r.data, etag: r.etag };
}

/** The query options of `['docJson', id]`: 30 s fresh, refetched on window focus (this key only). */
export function docJsonQuery(qc: QueryClient, itemId: string) {
  return {
    queryKey: docJsonKey(itemId),
    queryFn: ({ signal }: { signal: AbortSignal }) =>
      fetchDocJson(itemId, qc.getQueryData<CachedDocJson>(docJsonKey(itemId)), signal),
    staleTime: 30_000,
    refetchOnWindowFocus: true,
  } as const;
}

/** A document's ProseMirror JSON for a board card (read-only; never written from here). */
export function useDocJson(itemId: string, enabled = true): UseQueryResult<CachedDocJson, ApiError> {
  const qc = useQueryClient();
  return useQuery<CachedDocJson, ApiError>({ ...docJsonQuery(qc, itemId), enabled });
}
