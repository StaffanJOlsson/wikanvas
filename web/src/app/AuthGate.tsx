// Loads `GET /api/v1/auth/me` before any signed-in route (docs/ARCHITECTURE.md §9.1): 401 → /login?next=,
// 409 setup_required → /setup, must_change_password → /settings/account?force=1. W3 completes it.
import { useQuery } from '@tanstack/react-query';
import { Navigate, Outlet, useLocation } from 'react-router';
import { api, isApiError, type ApiError } from '../api/client';
import type { Me } from '../api/gen/Me';

export const meKey = ['me'] as const;

export function fetchMe(signal?: AbortSignal): Promise<Me> {
  return api<Me>('GET', '/auth/me', { signal, authLost: false });
}

export function AuthGate() {
  const location = useLocation();
  const me = useQuery<Me, ApiError>({ queryKey: meKey, queryFn: ({ signal }) => fetchMe(signal) });

  if (me.isPending) return null;
  if (me.isError) {
    const e = me.error;
    if (isApiError(e, 'setup_required')) return <Navigate to="/setup" replace />;
    if (isApiError(e) && e.status === 401) {
      const next = location.pathname + location.search + location.hash;
      return <Navigate to={`/login?next=${encodeURIComponent(next)}`} replace />;
    }
    return (
      <div className="placeholder-page" role="alert">
        Quire could not load your account: {e.message}
      </div>
    );
  }
  if (me.data.must_change_password && location.pathname !== '/settings/account') {
    return <Navigate to="/settings/account?force=1" replace />;
  }
  return <Outlet />;
}
