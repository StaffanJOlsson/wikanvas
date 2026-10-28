// After loading an item or a collection, its page replaces the URL with the canonical one: the current slug, and the
// right kind prefix (docs/ARCHITECTURE.md §9.1). Search and hash are kept.
import { useEffect } from 'react';
import { useLocation, useNavigate } from 'react-router';

export function useCanonicalPath(path: string | null | undefined): void {
  const location = useLocation();
  const navigate = useNavigate();
  useEffect(() => {
    if (path && location.pathname !== path) {
      void navigate({ pathname: path, search: location.search, hash: location.hash }, { replace: true });
    }
  }, [path, location.pathname, location.search, location.hash, navigate]);
}
