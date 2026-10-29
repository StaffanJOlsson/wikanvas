// Access per collection (docs/ARCHITECTURE.md §8 "Access per collection", D152): a collection's members
// (`GET/PUT/DELETE /collections/{id}/members[/{user_id}]`, `manage` only), the workspace directory to pick people from
// (`GET /directory`, any signed-in account; never an email), and the caller's access to a collection by id from the
// sidebar. A membership change can change the caller's own access, so it invalidates everything that carries
// `my_access`: the sidebar, the collection page, the collection's tree and the item details.
import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
  type UseMutationResult,
  type UseQueryResult,
} from '@tanstack/react-query';
import { useCallback } from 'react';
import { api, type ApiError } from '../client';
import type { CollectionAccess } from '../gen/CollectionAccess';
import type { CollectionMember } from '../gen/CollectionMember';
import type { DirectoryUser } from '../gen/DirectoryUser';
import type { MemberAccess } from '../gen/MemberAccess';
import { sidebarKey, treeKey, useSidebar } from './tree';

const enc = encodeURIComponent;

export function membersKey(collectionId: string): readonly ['members', string] {
  return ['members', collectionId] as const;
}

export const directoryKey = ['directory'] as const;

/** `GET /collections/{id}/members`: the memberships, by name (the caller needs `manage`). */
export function useMembers(collectionId: string | null | undefined, enabled = true): UseQueryResult<CollectionMember[], ApiError> {
  return useQuery<CollectionMember[], ApiError>({
    queryKey: membersKey(collectionId ?? ''),
    queryFn: ({ signal }) => api<CollectionMember[]>('GET', `/collections/${enc(collectionId!)}/members`, { signal }),
    enabled: enabled && !!collectionId,
  });
}

/** `GET /directory`: every account that can sign in (id, name, role), by name. */
export function useDirectory(enabled = true): UseQueryResult<DirectoryUser[], ApiError> {
  return useQuery<DirectoryUser[], ApiError>({
    queryKey: directoryKey,
    queryFn: ({ signal }) => api<DirectoryUser[]>('GET', '/directory', { signal }),
    enabled,
  });
}

/** Everything that holds a `my_access` of the collection, and its members. */
export function invalidateAccess(qc: QueryClient, collectionId: string): void {
  void qc.invalidateQueries({ queryKey: membersKey(collectionId) });
  void qc.invalidateQueries({ queryKey: sidebarKey });
  void qc.invalidateQueries({ queryKey: ['collection'] });
  void qc.invalidateQueries({ queryKey: treeKey(collectionId) });
  void qc.invalidateQueries({ queryKey: ['item'] });
}

function byName(a: CollectionMember, b: CollectionMember): number {
  return a.user.name.localeCompare(b.user.name);
}

export interface SetMemberVars {
  collectionId: string;
  userId: string;
  access: MemberAccess;
}

/** `PUT /collections/{id}/members/{user_id}`: adds the person or changes their access. */
export function useSetMember(): UseMutationResult<CollectionMember, ApiError, SetMemberVars> {
  const qc = useQueryClient();
  return useMutation<CollectionMember, ApiError, SetMemberVars>({
    mutationFn: ({ collectionId, userId, access }) =>
      api<CollectionMember>('PUT', `/collections/${enc(collectionId)}/members/${enc(userId)}`, { json: { access } }),
    onSuccess: (member, { collectionId }) => {
      qc.setQueryData<CollectionMember[]>(membersKey(collectionId), (list) =>
        list ? [...list.filter((m) => m.user.id !== member.user.id), member].sort(byName) : list);
    },
    onSettled: (_m, _e, { collectionId }) => invalidateAccess(qc, collectionId),
  });
}

export interface RemoveMemberVars {
  collectionId: string;
  userId: string;
}

/** `DELETE /collections/{id}/members/{user_id}`, optimistic in the list. */
export function useRemoveMember(): UseMutationResult<void, ApiError, RemoveMemberVars, { before: CollectionMember[] | undefined }> {
  const qc = useQueryClient();
  return useMutation<void, ApiError, RemoveMemberVars, { before: CollectionMember[] | undefined }>({
    mutationFn: ({ collectionId, userId }) => api<void>('DELETE', `/collections/${enc(collectionId)}/members/${enc(userId)}`),
    onMutate: async ({ collectionId, userId }) => {
      await qc.cancelQueries({ queryKey: membersKey(collectionId) });
      const before = qc.getQueryData<CollectionMember[]>(membersKey(collectionId));
      qc.setQueryData<CollectionMember[]>(membersKey(collectionId), (list) => list?.filter((m) => m.user.id !== userId));
      return { before };
    },
    onError: (_e, { collectionId }, ctx) => qc.setQueryData(membersKey(collectionId), ctx?.before),
    onSettled: (_r, _e, { collectionId }) => invalidateAccess(qc, collectionId),
  });
}

export interface AccessLookup {
  /** The sidebar has loaded: a collection it does not list is one the caller may not read. */
  loaded: boolean;
  /** The caller's access to a collection, from the sidebar; undefined while it loads or when it is not listed. */
  of(collectionId: string): CollectionAccess | undefined;
}

/** The caller's access to collections by id (the sidebar lists every collection they may read, with `my_access`). */
export function useAccessLookup(): AccessLookup {
  const sidebar = useSidebar();
  const collections = sidebar.data?.collections;
  const of = useCallback(
    (collectionId: string) => collections?.find((c) => c.id === collectionId)?.my_access,
    [collections],
  );
  return { loaded: collections !== undefined, of };
}
