// The document page, lazily loaded at /doc/:slugId (docs/ARCHITECTURE.md §9.1, §9.3).
// PLACEHOLDER (W0b): the top bar, the item header and the item menu over the W0b contracts; W4 replaces it with the
// editor, the Contents column, the nested list and the exports.
import { useParams } from 'react-router';
import { isApiError } from '../api/client';
import { useItem } from '../api/queries/items';
import { ItemHeader } from '../components/item/ItemHeader';
import { ItemMenu } from '../components/item/ItemMenu';
import { TopBar } from '../components/topbar/TopBar';
import { itemPath, urlIdFromSlugId } from '../lib/urls';
import { useCanonicalPath } from '../lib/useCanonicalPath';
import NotFound from './NotFound';
import { PlaceholderPage } from './PlaceholderPage';

export default function DocPage() {
  const { slugId } = useParams();
  const ref = urlIdFromSlugId(slugId);
  const item = useItem(ref);
  useCanonicalPath(item.data ? itemPath(item.data.item) : null);

  if (ref === null || isApiError(item.error, 'not_found')) return <NotFound />;
  if (item.isPending) return null;
  if (item.isError) return <PlaceholderPage title="Document">{item.error.message}</PlaceholderPage>;
  const d = item.data;
  return (
    <article className="doc-page">
      <TopBar item={d} actions={<ItemMenu item={d} extraExports={[]} />} />
      <ItemHeader item={d} readOnly={d.trashed_at !== null || d.archived_at !== null} onEnterBody={() => undefined} />
      <PlaceholderPage>The document editor comes with W4.</PlaceholderPage>
    </article>
  );
}
