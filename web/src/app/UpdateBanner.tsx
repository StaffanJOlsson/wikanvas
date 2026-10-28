// "Quire was updated. Reload" (docs/ARCHITECTURE.md §9.1), and the error boundary of the page routes: a chunk that
// fails to load triggers the update check, and shows the banner instead of an error when the server is a new build.
import { useEffect, useState } from 'react';
import { isRouteErrorResponse, useRouteError } from 'react-router';
import { checkForUpdate, isChunkLoadError, reloadPage, useUpdate } from './updateCheck';

export function UpdateBanner() {
  const available = useUpdate((s) => s.available);
  if (!available) return null;
  return (
    <div className="update-banner" role="alert">
      <span>Quire was updated.</span>
      <button type="button" className="button primary" onClick={reloadPage}>Reload</button>
    </div>
  );
}

/** The ErrorBoundary of every page route (lazy chunks included). */
export function RouteError() {
  const error = useRouteError();
  const available = useUpdate((s) => s.available);
  const chunk = isChunkLoadError(error);
  const [checked, setChecked] = useState(!chunk);

  useEffect(() => {
    if (!chunk) return;
    let live = true;
    void checkForUpdate().finally(() => {
      if (live) setChecked(true);
    });
    return () => {
      live = false;
    };
  }, [chunk, error]);

  if (available) {
    return (
      <section className="placeholder-page">
        <h1>Quire was updated</h1>
        <p>Reload to use the new version.</p>
        <button type="button" className="button primary" onClick={reloadPage}>Reload</button>
      </section>
    );
  }
  if (!checked) return null;
  const message = isRouteErrorResponse(error)
    ? `${error.status} ${error.statusText}`
    : error instanceof Error ? error.message : 'Unknown error';
  return (
    <section className="placeholder-page" role="alert">
      <h1>This page could not be shown</h1>
      <p>{message}</p>
      <button type="button" className="button" onClick={reloadPage}>Reload</button>
    </section>
  );
}
