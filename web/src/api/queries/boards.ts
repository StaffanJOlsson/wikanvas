// Board API calls (docs/ARCHITECTURE.md §6.2, §7.2, §9.5). The envelope travels as text: the GET's body goes to the
// frame as it came (never parsed, so nothing a newer engine wrote is lost), and the PUT sends the frame's snapshot.
import { useQuery, type UseQueryResult } from '@tanstack/react-query';
import { API_BASE, ApiError, AUTH_LOST_EVENT, api, apiUrl } from '../client';
import type { BoardPutResult } from '../gen/BoardPutResult';
import type { ErrorBody } from '../gen/ErrorBody';
import type { ItemSummary } from '../gen/ItemSummary';

/** A board as `GET /items/{id}/board` returns it: the envelope JSON text and the revision of its ETag. */
export interface LoadedBoard {
  board: string;
  rev: number;
}

/** The ETag of a board revision, `"r{rev}"`: the `If-Match` of a PUT based on it. */
export function boardEtag(rev: number): string {
  return `"r${rev}"`;
}

/** The revision of a board ETag (`"r12"`, also weak `W/"r12"`), or null. */
export function revFromEtag(etag: string | null | undefined): number | null {
  const m = /^(?:W\/)?"r(\d+)"$/.exec((etag ?? '').trim());
  return m ? Number(m[1]) : null;
}

function itemPath(itemId: string): string {
  return `/items/${encodeURIComponent(itemId)}`;
}

/** `GET /api/v1/items/{id}/board`. Throws `ApiError` like `api()` does. */
export async function fetchBoard(itemId: string, signal?: AbortSignal): Promise<LoadedBoard> {
  let res: Response;
  try {
    res = await fetch(apiUrl(`${itemPath(itemId)}/board`), {
      credentials: 'same-origin',
      headers: { Accept: 'application/json' },
      signal,
    });
  } catch (e) {
    if (signal?.aborted) throw e;
    throw new ApiError(0, 'network', e instanceof Error ? e.message : 'Network error');
  }
  const requestId = res.headers.get('X-Request-Id');
  if (!res.ok) throw await errorOf(res, requestId);
  const rev = revFromEtag(res.headers.get('ETag'));
  if (rev === null) throw new ApiError(res.status, 'bad_response', 'The board came without its revision', undefined, requestId);
  return { board: await res.text(), rev };
}

async function errorOf(res: Response, requestId: string | null): Promise<ApiError> {
  let err: ApiError;
  try {
    const body = (await res.json()) as Partial<ErrorBody> | null;
    const info = body?.error;
    err = info && typeof info.code === 'string'
      ? new ApiError(res.status, info.code, info.message ?? info.code, info.details ?? undefined, body?.request_id ?? requestId)
      : new ApiError(res.status, 'bad_response', `HTTP ${res.status}`, undefined, requestId);
  } catch {
    err = new ApiError(res.status, 'bad_response', `HTTP ${res.status}`, undefined, requestId);
  }
  if (res.status === 401 && typeof window !== 'undefined') {
    window.dispatchEvent(new CustomEvent(AUTH_LOST_EVENT, { detail: err }));
  }
  return err;
}

/**
 * `PUT /api/v1/items/{id}/board` with `If-Match: "r{base}"` (or `*`) and, when known, `X-Wikanvas-Engine` (§6.2). A 412
 * throws an `ApiError` whose `details` are `{rev, etag}` of the current revision.
 */
export function putBoard(itemId: string, board: string, base: number | '*', engine?: string): Promise<BoardPutResult> {
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    'If-Match': base === '*' ? '*' : boardEtag(base),
  };
  if (engine) headers['X-Wikanvas-Engine'] = engine;
  return api<BoardPutResult>('PUT', `${itemPath(itemId)}/board`, { body: board, headers });
}

/** `details` of a board's 412 `rev_mismatch`. */
export interface BoardConflictDetails {
  rev: number;
  etag: string;
}

/** The current revision a 412 names, or null when `e` is not a board conflict. */
export function conflictRev(e: unknown): number | null {
  if (!(e instanceof ApiError) || e.code !== 'rev_mismatch') return null;
  const d = e.details as Partial<BoardConflictDetails> | undefined;
  return typeof d?.rev === 'number' ? d.rev : revFromEtag(d?.etag);
}

/** The `.inkboard` download of a board. */
export function boardDownloadUrl(itemId: string): string {
  return `${API_BASE}${itemPath(itemId)}/board.inkboard`;
}

/** `GET /api/v1/items/{id}/children` under the shared key `['children', id]` (§9.2). */
export function useChildren(itemId: string, enabled = true): UseQueryResult<ItemSummary[], ApiError> {
  return useQuery<ItemSummary[], ApiError>({
    queryKey: ['children', itemId],
    queryFn: ({ signal }) => api<ItemSummary[]>('GET', `${itemPath(itemId)}/children`, { signal }),
    enabled,
  });
}
