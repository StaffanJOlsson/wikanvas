// Starred (docs/ARCHITECTURE.md §9.6.6): the starred items with their paths.
import { StarOff } from 'lucide-react';
import { flattenPages, useList } from '../api/queries/lists';
import { ItemList, LoadMore } from '../components/list/ItemList';
import { useItemActions } from '../components/item/useItemActions';
import { PageHeader } from '../components/topbar/PageHeader';
import { relativeTime } from '../lib/time';

export default function Starred() {
  const list = useList('starred');
  const actions = useItemActions();
  const items = flattenPages(list.data);
  return (
    <div className="page">
      <PageHeader title="Starred" />
      {list.isError && <p className="form-error" role="alert">{list.error.message}</p>}
      {list.isSuccess && (
        <ItemList
          label="Starred"
          empty="Star documents and boards to find them here and in the sidebar."
          entries={items.map((i) => ({
            item: i,
            path: i.path,
            meta: i.starred_at ? `Starred ${relativeTime(i.starred_at)}` : undefined,
            actions: (
              <button type="button" className="icon-button" aria-label={`Unstar ${i.title || 'Untitled'}`} title="Unstar"
                onClick={() => void actions.setStarred(i, false).then(() => list.refetch())}>
                <StarOff size={16} aria-hidden />
              </button>
            ),
          }))}
        />
      )}
      <LoadMore hasNext={list.hasNextPage} loading={list.isFetchingNextPage} onMore={() => void list.fetchNextPage()} />
    </div>
  );
}
