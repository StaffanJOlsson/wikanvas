// Trash (docs/ARCHITECTURE.md §9.6.6): trashed collections and item roots with "+N nested" and the purge date,
// Restore, and Delete forever (the confirmation names the count). Everything is purged after 30 days (§12). Deleting
// forever is the owner's and admins' (`Action::Admin`, §8): a member or viewer sees no Delete forever and no Empty
// Trash. Restoring an item needs `write` on its collection (the sidebar's, or the trashed collection's own), restoring
// a collection `manage` (D152).
import { Link } from 'react-router';
import { isApiError } from '../api/client';
import { useAccessLookup } from '../api/queries/access';
import { useMe } from '../api/queries/auth';
import type { TrashedCollection } from '../api/gen/TrashedCollection';
import type { TrashedItem } from '../api/gen/TrashedItem';
import { useRestoreItem } from '../api/queries/items';
import { usePurge, useTrashList } from '../api/queries/lists';
import { useRestoreCollection } from '../api/queries/tree';
import { CollectionIcon } from '../components/collection/CollectionIcon';
import { KindIcon } from '../components/item/KindIcon';
import { ItemPath } from '../components/list/ItemPath';
import { PageHeader } from '../components/topbar/PageHeader';
import { canManage, canWrite } from '../lib/access';
import { itemPath } from '../lib/urls';
import { confirmAction } from '../state/dialogs';
import { errorToast, toast } from '../state/toasts';

const DAY = 86_400_000;

/** "Deleted forever in 12 days" (or "today", "tomorrow"). */
export function purgeText(purgeAt: string, now: number = Date.now()): string {
  const days = Math.ceil((Date.parse(purgeAt) - now) / DAY);
  if (!Number.isFinite(days)) return '';
  if (days <= 0) return 'Deleted forever today';
  if (days === 1) return 'Deleted forever tomorrow';
  return `Deleted forever in ${days} days`;
}

function pages(n: number): string {
  return n === 1 ? '1 page' : `${n} pages`;
}

/** The items a purge deletes: the root and its nested ones. */
export function itemCount(i: TrashedItem): number {
  return 1 + i.descendant_count;
}

export default function Trash() {
  const role = useMe().data?.user.role;
  // Until `/auth/me` answers (AuthGate has it cached in the app), the server's check alone decides.
  const canPurge = role === undefined || role === 'owner' || role === 'admin';
  const trash = useTrashList();
  const purge = usePurge().mutateAsync;
  const restoreItem = useRestoreItem().mutateAsync;
  const restoreCollection = useRestoreCollection().mutateAsync;
  const data = trash.data;
  const access = useAccessLookup();
  /** Until the sidebar answers, the server's check alone decides (as for `canPurge`). */
  const canRestore = (i: TrashedItem): boolean => {
    const a = data?.collections.find((c) => c.id === i.collection_id)?.my_access ?? access.of(i.collection_id);
    return a === undefined ? !access.loaded : canWrite(a);
  };
  const total = data
    ? data.items.reduce((n, i) => n + itemCount(i), 0) + data.collections.reduce((n, c) => n + c.item_count, 0)
    : 0;
  const empty = data !== undefined && data.items.length === 0 && data.collections.length === 0;

  const forever = async (title: string, message: string, run: () => Promise<void>) => {
    if (!(await confirmAction({ title, message, confirmLabel: 'Delete forever', danger: true }))) return;
    try {
      await run();
      toast({ message: 'Deleted forever', kind: 'success' });
    } catch (e) {
      errorToast(e, 'Nothing was deleted');
    }
  };

  const purgeItem = (i: TrashedItem) => forever(
    `Delete “${i.title || 'Untitled'}” forever?`,
    i.descendant_count > 0
      ? `This deletes ${pages(itemCount(i))}: the page and ${pages(i.descendant_count)} nested in it. This cannot be undone.`
      : 'This deletes 1 page. This cannot be undone.',
    () => purge({ kind: 'item', id: i.id }),
  );

  const purgeCollection = (c: TrashedCollection) => forever(
    `Delete the collection “${c.name}” forever?`,
    `This deletes the collection and ${pages(c.item_count)}. This cannot be undone.`,
    () => purge({ kind: 'collection', id: c.id }),
  );

  const restore = async (i: TrashedItem) => {
    try {
      const r = await restoreItem(i);
      toast({ message: r.moved_to_top ? 'Restored to the top of its collection' : 'Restored', kind: 'success' });
    } catch (e) {
      errorToast(e, isApiError(e, 'parent_invalid') ? 'Restore the collection first' : 'The item was not restored');
    }
  };

  return (
    <div className="page">
      <PageHeader
        title="Trash"
        actions={!empty && data && canPurge ? (
          <button
            type="button"
            className="button danger"
            onClick={() => void forever(
              'Empty the Trash?',
              `This deletes ${pages(total)}${data.collections.length > 0 ? ` and ${data.collections.length === 1 ? '1 collection' : `${data.collections.length} collections`}` : ''} forever. This cannot be undone.`,
              () => purge({ kind: 'all' }),
            )}
          >
            Empty Trash
          </button>
        ) : undefined}
      />
      <p className="page-note">
        Items in Trash are deleted forever after 30 days.
        {!canPurge && ' Only the owner and admins can delete them sooner.'}
      </p>
      {trash.isError && <p className="form-error" role="alert">{trash.error.message}</p>}
      {empty && <p className="list-empty">Trash is empty.</p>}
      {data && data.collections.length > 0 && (
        <section>
          <h2 className="list-heading">Collections</h2>
          <ul className="item-list" aria-label="Trashed collections">
            {data.collections.map((c) => (
              <li key={c.id} className="item-list-row">
                <span className="item-list-link">
                  <CollectionIcon collection={c} />
                  <span className="item-list-text">
                    <span className="item-list-title">{c.name}</span>
                    <span className="item-path">{c.item_count > 0 ? `+${c.item_count} nested` : 'Empty'}</span>
                  </span>
                </span>
                <span className="item-list-meta">{purgeText(c.purge_at)}</span>
                <span className="item-list-actions">
                  {canManage(c.my_access) && (
                    <button type="button" className="button small"
                      onClick={() => void restoreCollection(c.id).then(() => toast({ message: 'Restored', kind: 'success' }),
                        (e: unknown) => errorToast(e))}>
                      Restore
                    </button>
                  )}
                  {canPurge && (
                    <button type="button" className="button small danger" onClick={() => void purgeCollection(c)}>Delete forever</button>
                  )}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
      {data && data.items.length > 0 && (
        <section>
          {data.collections.length > 0 && <h2 className="list-heading">Documents and boards</h2>}
          <ul className="item-list" aria-label="Trashed items">
            {data.items.map((i) => (
              <li key={i.id} className="item-list-row">
                <Link to={itemPath(i)} className="item-list-link">
                  <KindIcon kind={i.kind} />
                  <span className="item-list-text">
                    <span className="item-list-title">
                      {i.title || 'Untitled'}
                      {i.descendant_count > 0 && <span className="nested-count"> +{i.descendant_count} nested</span>}
                    </span>
                    <ItemPath collectionId={i.collection_id} path={i.path} />
                  </span>
                </Link>
                <span className="item-list-meta">{purgeText(i.purge_at)}</span>
                <span className="item-list-actions">
                  {canRestore(i) && (
                    <button type="button" className="button small" onClick={() => void restore(i)}>Restore</button>
                  )}
                  {canPurge && (
                    <button type="button" className="button small danger" onClick={() => void purgeItem(i)}>Delete forever</button>
                  )}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
