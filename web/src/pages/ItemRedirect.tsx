// `/item/:id` (docs/ARCHITECTURE.md §9.1): opens an item by its UUID, the DOM href of a document's item links
// (`wikanvas://item/<uuid>` in the stored mark). It loads the item and replaces the URL with the item's page, so Back
// skips it; an unknown or purged item is NotFound.
import { Navigate, useLocation, useParams } from 'react-router';
import { isApiError } from '../api/client';
import { useItem } from '../api/queries/items';
import { itemPath } from '../lib/urls';
import NotFound from './NotFound';
import { PlaceholderPage } from './PlaceholderPage';

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export default function ItemRedirect() {
  const { id } = useParams();
  const location = useLocation();
  const ref = id && UUID.test(id) ? id.toLowerCase() : null;
  const item = useItem(ref);
  if (ref === null || isApiError(item.error, 'not_found')) return <NotFound />;
  if (item.isPending) return null;
  if (item.isError) return <PlaceholderPage title="Page">{item.error.message}</PlaceholderPage>;
  return <Navigate to={{ pathname: itemPath(item.data.item), search: location.search, hash: location.hash }} replace />;
}
