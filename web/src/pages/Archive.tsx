// Archive (docs/ARCHITECTURE.md §9.6.6): archived items with their paths, and Unarchive.
import { flattenPages, useList } from '../api/queries/lists';
import { useUnarchiveItem } from '../api/queries/items';
import { ItemList, LoadMore } from '../components/list/ItemList';
import { PageHeader } from '../components/topbar/PageHeader';
import { relativeTime } from '../lib/time';
import { errorToast, toast } from '../state/toasts';

export default function Archive() {
  const list = useList('archive');
  const unarchive = useUnarchiveItem().mutateAsync;
  const items = flattenPages(list.data);
  return (
    <div className="page">
      <PageHeader title="Archive" />
      {list.isError && <p className="form-error" role="alert">{list.error.message}</p>}
      {list.isSuccess && (
        <ItemList
          label="Archived items"
          empty="Archived documents and boards appear here. Archiving hides them from the tree and from search."
          entries={items.map((i) => ({
            item: i,
            path: i.path,
            meta: i.archived_at ? `Archived ${relativeTime(i.archived_at)}` : undefined,
            actions: (
              <button
                type="button"
                className="button small"
                onClick={() => void unarchive(i).then((r) => {
                  toast({ message: r.moved_to_top ? 'Unarchived, at the top of its collection' : 'Unarchived', kind: 'success' });
                }, (e: unknown) => errorToast(e))}
              >
                Unarchive
              </button>
            ),
          }))}
        />
      )}
      <LoadMore hasNext={list.hasNextPage} loading={list.isFetchingNextPage} onMore={() => void list.fetchNextPage()} />
    </div>
  );
}
