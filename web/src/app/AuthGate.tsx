// Loads `GET /api/v1/auth/me` before any signed-in route (docs/ARCHITECTURE.md §9.1): 401 → /login?next=,
// 409 setup_required → /setup, must_change_password → /settings/account?force=1. Once `/me` has loaded, the server's
// theme wins over the local copy (§9.2), and a later 401 opens the sign-in dialog instead of leaving the page.
import { useEffect } from 'react';
import { Navigate, Outlet, useLocation } from 'react-router';
import { isApiError } from '../api/client';
import { useMe } from '../api/queries/auth';
import { openSignIn } from '../state/dialogs';
import { setTheme } from '../state/ui';

export { fetchMe, meKey } from '../api/queries/auth';

export function AuthGate() {
  const location = useLocation();
  const me = useMe();
  const theme = me.data?.user.theme;

  useEffect(() => {
    if (theme) setTheme(theme);
  }, [theme]);

  const lostLater = me.data !== undefined && isApiError(me.error) && me.error.status === 401;
  useEffect(() => {
    if (lostLater) openSignIn();
  }, [lostLater]);

  if (me.data === undefined) {
    // An earlier error is being retried (after a sign-in, or a remount): wait for the fresh answer.
    if (me.isPending || me.isFetching) return null;
    const e = me.error;
    if (isApiError(e, 'setup_required')) return <Navigate to="/setup" replace />;
    if (isApiError(e) && e.status === 401) {
      const next = location.pathname + location.search + location.hash;
      return <Navigate to={`/login?next=${encodeURIComponent(next)}`} replace />;
    }
    return (
      <div className="placeholder-page" role="alert">
        <h1>Wikanvas could not load your account</h1>
        <p>{e?.message}</p>
        <button type="button" className="button" onClick={() => void me.refetch()}>Try again</button>
      </div>
    );
  }
  if (me.data.must_change_password && location.pathname !== '/settings/account') {
    return <Navigate to="/settings/account?force=1" replace />;
  }
  return <Outlet />;
}
