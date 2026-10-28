// Personal access tokens (docs/ARCHITECTURE.md §7.2, §7.3, §9.6.6). The secret of a new token is returned once by
// `POST /tokens` and must never be cached (§9.2): `createToken` resolves with it for the dialog's own state and puts
// only the `ApiToken` (no secret) into `['tokens']`. It is deliberately not a TanStack mutation, whose cache would keep
// the response.
import { useCallback, useState } from 'react';
import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
  type UseMutationResult,
  type UseQueryResult,
} from '@tanstack/react-query';
import { api, type ApiError } from '../client';
import type { ApiToken } from '../gen/ApiToken';
import type { CreatedToken } from '../gen/CreatedToken';
import type { NewToken } from '../gen/NewToken';

export const tokensKey = ['tokens'] as const;

/** `GET /tokens`: live and expired (not revoked) tokens, newest first, at most 100. */
export function useTokens(): UseQueryResult<ApiToken[], ApiError> {
  return useQuery<ApiToken[], ApiError>({
    queryKey: tokensKey,
    queryFn: ({ signal }) => api<ApiToken[]>('GET', '/tokens', { signal }),
  });
}

/** `POST /tokens`. Adds the new `ApiToken` (never the secret) to the cached list. */
export async function createToken(qc: QueryClient, body: NewToken): Promise<CreatedToken> {
  const created = await api<CreatedToken>('POST', '/tokens', { json: body });
  qc.setQueryData<ApiToken[]>(tokensKey, (list) =>
    list ? [created.api_token, ...list.filter((t) => t.id !== created.api_token.id)] : list);
  void qc.invalidateQueries({ queryKey: tokensKey });
  return created;
}

export interface CreateTokenState {
  create(body: NewToken): Promise<CreatedToken>;
  isPending: boolean;
  error: ApiError | null;
  reset(): void;
}

/** `createToken` with pending and error state, for the "New token" dialog. Holds no response. */
export function useCreateToken(): CreateTokenState {
  const qc = useQueryClient();
  const [isPending, setPending] = useState(false);
  const [error, setError] = useState<ApiError | null>(null);
  const create = useCallback(async (body: NewToken) => {
    setPending(true);
    setError(null);
    try {
      return await createToken(qc, body);
    } catch (e) {
      setError(e as ApiError);
      throw e;
    } finally {
      setPending(false);
    }
  }, [qc]);
  const reset = useCallback(() => setError(null), []);
  return { create, isPending, error, reset };
}

/** `DELETE /tokens/{id}` (also revokes the tokens it created), optimistic in the list. */
export function useRevokeToken(): UseMutationResult<void, ApiError, ApiToken> {
  const qc = useQueryClient();
  return useMutation<void, ApiError, ApiToken, { before: ApiToken[] | undefined }>({
    mutationFn: (t) => api<void>('DELETE', `/tokens/${encodeURIComponent(t.id)}`),
    onMutate: async (t) => {
      await qc.cancelQueries({ queryKey: tokensKey });
      const before = qc.getQueryData<ApiToken[]>(tokensKey);
      qc.setQueryData<ApiToken[]>(tokensKey, (list) =>
        list?.filter((x) => x.id !== t.id && x.parent_token_id !== t.id));
      return { before };
    },
    onError: (_e, _t, ctx) => qc.setQueryData(tokensKey, ctx?.before),
    onSettled: () => qc.invalidateQueries({ queryKey: tokensKey }),
  });
}
