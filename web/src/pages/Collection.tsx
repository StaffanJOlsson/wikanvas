// Placeholder page (W0b); W3 implements it: description, sort and the top-level items (docs/ARCHITECTURE.md §9.6.6).
import { useQuery } from '@tanstack/react-query';
import { useParams } from 'react-router';
import { api, isApiError, type ApiError } from '../api/client';
import type { Collection as CollectionDto } from '../api/gen/Collection';
import { collectionPath, urlIdFromSlugId } from '../lib/urls';
import { useCanonicalPath } from '../lib/useCanonicalPath';
import NotFound from './NotFound';
import { PlaceholderPage } from './PlaceholderPage';

export default function Collection() {
  const { slugId } = useParams();
  const ref = urlIdFromSlugId(slugId);
  const collection = useQuery<CollectionDto, ApiError>({
    queryKey: ['collection', ref],
    queryFn: ({ signal }) => api<CollectionDto>('GET', `/collections/${encodeURIComponent(ref ?? '')}`, { signal }),
    enabled: ref !== null,
  });
  useCanonicalPath(collection.data ? collectionPath(collection.data) : null);

  if (ref === null || isApiError(collection.error, 'not_found')) return <NotFound />;
  if (collection.isPending) return null;
  if (collection.isError) return <PlaceholderPage title="Collection">{collection.error.message}</PlaceholderPage>;
  return <PlaceholderPage title={collection.data.name}>{collection.data.description}</PlaceholderPage>;
}
