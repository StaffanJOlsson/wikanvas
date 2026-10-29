// Accounts and invite links (docs/ARCHITECTURE.md §7.2, §9.6.6, D148): the account list and its changes, the pending
// invites, and the public routes of the invite page. A new invite's link is returned once by `POST /invites` and must
// never be cached, like a token's secret (§9.2): `createInvite` resolves with it for the dialog's own state and puts
// only the `Invite` (no link) into `['invites']`.
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
import type { AcceptInvite } from '../gen/AcceptInvite';
import type { CreatedInvite } from '../gen/CreatedInvite';
import type { Invite } from '../gen/Invite';
import type { InviteLookup } from '../gen/InviteLookup';
import type { NewInvite } from '../gen/NewInvite';
import type { UserEnvelope } from '../gen/UserEnvelope';
import type { UserInfo } from '../gen/UserInfo';
import type { UserPatch } from '../gen/UserPatch';

export const usersKey = ['users'] as const;
export const invitesKey = ['invites'] as const;

/** `GET /users`: every account, the owner first, disabled ones included (the owner and admins only). */
export function useUsers(enabled = true): UseQueryResult<UserInfo[], ApiError> {
  return useQuery<UserInfo[], ApiError>({
    queryKey: usersKey,
    queryFn: ({ signal }) => api<UserInfo[]>('GET', '/users', { signal }),
    enabled,
  });
}

/** `PATCH /users/{id}`: a role and/or disabled. The answer replaces the row in `['users']`. */
export function usePatchUser(): UseMutationResult<UserInfo, ApiError, { id: string; patch: UserPatch }> {
  const qc = useQueryClient();
  return useMutation<UserInfo, ApiError, { id: string; patch: UserPatch }>({
    mutationFn: ({ id, patch }) => api<UserInfo>('PATCH', `/users/${encodeURIComponent(id)}`, { json: patch }),
    onSuccess: (user) => {
      qc.setQueryData<UserInfo[]>(usersKey, (list) => list?.map((u) => (u.id === user.id ? user : u)));
      // Disabling or demoting an admin revokes the invites they made.
      void qc.invalidateQueries({ queryKey: invitesKey });
    },
    onSettled: () => qc.invalidateQueries({ queryKey: usersKey }),
  });
}

/** `GET /invites`: the pending invites, newest first, never a link. */
export function useInvites(enabled = true): UseQueryResult<Invite[], ApiError> {
  return useQuery<Invite[], ApiError>({
    queryKey: invitesKey,
    queryFn: ({ signal }) => api<Invite[]>('GET', '/invites', { signal }),
    enabled,
  });
}

/** `POST /invites`. Adds the new `Invite` (never its link) to the cached list. */
export async function createInvite(qc: QueryClient, body: NewInvite): Promise<CreatedInvite> {
  const created = await api<CreatedInvite>('POST', '/invites', { json: body });
  qc.setQueryData<Invite[]>(invitesKey, (list) =>
    list ? [created.invite, ...list.filter((i) => i.id !== created.invite.id)] : list);
  void qc.invalidateQueries({ queryKey: invitesKey });
  return created;
}

export interface CreateInviteState {
  create(body: NewInvite): Promise<CreatedInvite>;
  isPending: boolean;
}

/** `createInvite` with pending state, for the "Invite people" dialog. Holds no response. */
export function useCreateInvite(): CreateInviteState {
  const qc = useQueryClient();
  const [isPending, setPending] = useState(false);
  const create = useCallback(async (body: NewInvite) => {
    setPending(true);
    try {
      return await createInvite(qc, body);
    } finally {
      setPending(false);
    }
  }, [qc]);
  return { create, isPending };
}

/** `DELETE /invites/{id}`, optimistic in the list. */
export function useRevokeInvite(): UseMutationResult<void, ApiError, Invite> {
  const qc = useQueryClient();
  return useMutation<void, ApiError, Invite, { before: Invite[] | undefined }>({
    mutationFn: (i) => api<void>('DELETE', `/invites/${encodeURIComponent(i.id)}`),
    onMutate: async (i) => {
      await qc.cancelQueries({ queryKey: invitesKey });
      const before = qc.getQueryData<Invite[]>(invitesKey);
      qc.setQueryData<Invite[]>(invitesKey, (list) => list?.filter((x) => x.id !== i.id));
      return { before };
    },
    onError: (_e, _i, ctx) => qc.setQueryData(invitesKey, ctx?.before),
    onSettled: () => qc.invalidateQueries({ queryKey: invitesKey }),
  });
}

/** The public `GET /invites/{token}`: the invite page's state, workspace and role. Never sends `auth-lost`. */
export function useInviteLookup(token: string): UseQueryResult<InviteLookup, ApiError> {
  return useQuery<InviteLookup, ApiError>({
    queryKey: ['invite', token],
    queryFn: ({ signal }) =>
      api<InviteLookup>('GET', `/invites/${encodeURIComponent(token)}`, { signal, authLost: false }),
    retry: false,
    staleTime: Infinity,
  });
}

/** The public `POST /invites/{token}/accept`: creates the account and signs it in (201 `{user}`). */
export function acceptInvite(token: string, body: AcceptInvite): Promise<UserEnvelope> {
  return api<UserEnvelope>('POST', `/invites/${encodeURIComponent(token)}/accept`, { json: body, authLost: false });
}
