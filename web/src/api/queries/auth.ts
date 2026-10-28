// Account and session routes (docs/ARCHITECTURE.md §7.2, §8): `/auth/me`, sign-in and sign-out, setup, the password
// and the session list.
import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
  type UseMutationResult,
  type UseQueryResult,
} from '@tanstack/react-query';
import { api, type ApiError } from '../client';
import type { LoginRequest } from '../gen/LoginRequest';
import type { Me } from '../gen/Me';
import type { MePatch } from '../gen/MePatch';
import type { PasswordChange } from '../gen/PasswordChange';
import type { SessionInfo } from '../gen/SessionInfo';
import type { SetupRequest } from '../gen/SetupRequest';
import type { SetupStatus } from '../gen/SetupStatus';
import type { UserEnvelope } from '../gen/UserEnvelope';

export const meKey = ['me'] as const;
export const sessionsKey = ['sessions'] as const;

/** `GET /auth/me`. Its 401 is an answer (AuthGate redirects), so it never sends `auth-lost`. */
export function fetchMe(signal?: AbortSignal): Promise<Me> {
  return api<Me>('GET', '/auth/me', { signal, authLost: false });
}

/** The signed-in user, workspace and auth method (AuthGate loads it before any signed-in route). */
export function useMe(): UseQueryResult<Me, ApiError> {
  return useQuery<Me, ApiError>({ queryKey: meKey, queryFn: ({ signal }) => fetchMe(signal), staleTime: 60_000 });
}

/** `POST /auth/login`. A 401 `invalid_credentials` is the wrong email or password. */
export function login(body: LoginRequest): Promise<UserEnvelope> {
  return api<UserEnvelope>('POST', '/auth/login', { json: body, authLost: false });
}

/** `POST /auth/logout` (204, clears the cookie). */
export function logout(): Promise<void> {
  return api<void>('POST', '/auth/logout', { authLost: false });
}

/**
 * After a sign-in: drop the cached `/auth/me` answer (a 401 or 409 from before would send AuthGate straight back to
 * the sign-in page) and refetch everything else that is on screen.
 */
export async function afterSignIn(qc: QueryClient): Promise<void> {
  qc.removeQueries({ queryKey: meKey, exact: true });
  await qc.invalidateQueries();
}

export function fetchSetupStatus(signal?: AbortSignal): Promise<SetupStatus> {
  return api<SetupStatus>('GET', '/setup', { signal, authLost: false });
}

export function useSetupStatus(): UseQueryResult<SetupStatus, ApiError> {
  return useQuery<SetupStatus, ApiError>({ queryKey: ['setup'], queryFn: ({ signal }) => fetchSetupStatus(signal) });
}

/** `POST /setup`: creates the owner, seeds "Getting started" and signs in (201 `{user}`). */
export function setup(body: SetupRequest): Promise<UserEnvelope> {
  return api<UserEnvelope>('POST', '/setup', { json: body, authLost: false });
}

/** `PATCH /auth/me` (name, email, theme). Updates the cached `me`. */
export function usePatchMe(): UseMutationResult<UserEnvelope, ApiError, MePatch> {
  const qc = useQueryClient();
  return useMutation<UserEnvelope, ApiError, MePatch>({
    mutationFn: (patch) => api<UserEnvelope>('PATCH', '/auth/me', { json: patch }),
    onSuccess: ({ user }) => qc.setQueryData<Me>(meKey, (me) => (me ? { ...me, user } : me)),
  });
}

/** `POST /auth/password` (session only; revokes the other sessions). Clears `must_change_password`. */
export function useChangePassword(): UseMutationResult<void, ApiError, PasswordChange> {
  const qc = useQueryClient();
  return useMutation<void, ApiError, PasswordChange>({
    mutationFn: (body) => api<void>('POST', '/auth/password', { json: body }),
    onSuccess: () => {
      qc.setQueryData<Me>(meKey, (me) => (me ? { ...me, must_change_password: false } : me));
      void qc.invalidateQueries({ queryKey: meKey });
      void qc.invalidateQueries({ queryKey: sessionsKey });
    },
  });
}

export function useSessions(): UseQueryResult<SessionInfo[], ApiError> {
  return useQuery<SessionInfo[], ApiError>({
    queryKey: sessionsKey,
    queryFn: ({ signal }) => api<SessionInfo[]>('GET', '/auth/sessions', { signal }),
  });
}

/** `DELETE /auth/sessions/{id}`, or with `'others'` every session but this one. */
export function useRevokeSession(): UseMutationResult<void, ApiError, string> {
  const qc = useQueryClient();
  return useMutation<void, ApiError, string>({
    mutationFn: (id) =>
      id === 'others'
        ? api<void>('DELETE', '/auth/sessions?others=true')
        : api<void>('DELETE', `/auth/sessions/${encodeURIComponent(id)}`),
    onSettled: () => qc.invalidateQueries({ queryKey: sessionsKey }),
  });
}
