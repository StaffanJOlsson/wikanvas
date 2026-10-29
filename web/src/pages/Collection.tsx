// A collection (docs/ARCHITECTURE.md §9.6.6): its description, sort, and the top-level items, with New document /
// New board and Edit. Creating needs `write` on the collection, its menu (Edit…, sort, Delete…) and the sort `manage`
// (D152); a collection the caller may only read shows its items and nothing to change them with.
import { Ellipsis, Plus } from 'lucide-react';
import { useParams } from 'react-router';
import { isApiError } from '../api/client';
import { useCollection, useTree } from '../api/queries/tree';
import { CollectionIcon } from '../components/collection/CollectionIcon';
import { CHILD_ORDERS, useCollectionActions } from '../components/collection/useCollectionActions';
import { useItemActions } from '../components/item/useItemActions';
import { ItemList } from '../components/list/ItemList';
import { SidebarToggle } from '../components/topbar/SidebarToggle';
import { buildTree } from '../components/tree/buildTree';
import { Menu } from '../components/ui/Menu';
import { canManage, canWrite } from '../lib/access';
import { collectionPath, urlIdFromSlugId } from '../lib/urls';
import { useCanonicalPath } from '../lib/useCanonicalPath';
import NotFound from './NotFound';
import { PlaceholderPage } from './PlaceholderPage';

export default function Collection() {
  const { slugId } = useParams();
  const ref = urlIdFromSlugId(slugId);
  const collection = useCollection(ref);
  const c = collection.data;
  const tree = useTree(c?.id);
  const actions = useItemActions();
  const cactions = useCollectionActions();
  useCanonicalPath(c ? collectionPath(c) : null);

  if (ref === null || isApiError(collection.error, 'not_found')) return <NotFound />;
  if (collection.isPending) return null;
  if (!c) return <PlaceholderPage title="Collection">{collection.error?.message}</PlaceholderPage>;

  const roots = tree.data ? buildTree(c.id, tree.data.items, c.child_order).roots : [];
  const writable = canWrite(c.my_access);
  const manageable = canManage(c.my_access);
  return (
    <div className="page collection-page">
      <header className="page-header">
        <div className="page-header-row">
          <SidebarToggle />
          <CollectionIcon collection={c} size={24} />
          <h1>{c.name}</h1>
          <div className="page-actions">
            {writable && (
              <>
                <button type="button" className="button" onClick={() => void actions.create(c.id, null, 'doc')}>
                  <Plus size={16} aria-hidden /> New document
                </button>
                <button type="button" className="button" onClick={() => void actions.create(c.id, null, 'board')}>
                  <Plus size={16} aria-hidden /> New board
                </button>
              </>
            )}
            {manageable && (
              <Menu
                label="Collection menu"
                align="end"
                entries={[
                  { label: 'Edit…', onSelect: () => cactions.edit(c) },
                  { heading: 'Sort' },
                  ...CHILD_ORDERS.map(({ order, label }) => ({
                    label,
                    checked: c.child_order === order,
                    onSelect: () => void cactions.setOrder(c, order),
                  })),
                  { separator: true },
                  { label: 'Delete…', danger: true, onSelect: () => void cactions.remove(c) },
                ]}
              >
                <Ellipsis size={16} aria-hidden />
              </Menu>
            )}
          </div>
        </div>
        {c.description && <p className="collection-description">{c.description}</p>}
      </header>
      <div className="list-toolbar">
        <label>
          <span>Sort </span>
          <select
            value={c.child_order}
            disabled={!manageable}
            onChange={(e) => void cactions.setOrder(c, e.target.value as typeof c.child_order)}
          >
            {CHILD_ORDERS.map(({ order, label }) => <option key={order} value={order}>{label}</option>)}
          </select>
        </label>
      </div>
      {tree.isError && <p className="form-error" role="alert">{tree.error.message}</p>}
      {tree.isSuccess && (
        <ItemList
          label={`Top-level items of ${c.name}`}
          empty={writable ? 'This collection is empty. Create a document or a board to start.' : 'This collection is empty.'}
          entries={roots.map((n) => ({
            item: n.item,
            meta: n.children.length > 0 ? `${n.children.length} nested` : undefined,
          }))}
        />
      )}
    </div>
  );
}
