// Home (docs/ARCHITECTURE.md §9.6.6): Recently viewed and Recently updated, as tabs.
import { useSearchParams } from 'react-router';
import { flattenPages, useList } from '../api/queries/lists';
import { ItemList, LoadMore } from '../components/list/ItemList';
import { PageHeader } from '../components/topbar/PageHeader';
import { relativeTime } from '../lib/time';

const TABS = [
  { id: 'viewed', label: 'Recently viewed', list: 'recent' },
  { id: 'updated', label: 'Recently updated', list: 'updated' },
] as const;

export default function Home() {
  const [params, setParams] = useSearchParams();
  const tab = TABS.find((t) => t.id === params.get('tab')) ?? TABS[0];
  const list = useList(tab.list);
  const items = flattenPages(list.data);

  return (
    <div className="page">
      <PageHeader title="Home">
        <div className="tabs" role="tablist" aria-label="Lists">
          {TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              role="tab"
              aria-selected={t.id === tab.id}
              className="tab"
              onClick={() => setParams(t.id === 'viewed' ? {} : { tab: t.id }, { replace: true })}
            >
              {t.label}
            </button>
          ))}
        </div>
      </PageHeader>
      <div role="tabpanel" aria-label={tab.label}>
        {list.isError && <p className="form-error" role="alert">{list.error.message}</p>}
        {list.isSuccess && (
          <ItemList
            label={tab.label}
            empty={tab.id === 'viewed' ? 'Nothing viewed yet.' : 'Nothing updated yet.'}
            entries={items.map((i) => ({
              item: i,
              path: i.path,
              meta: tab.id === 'viewed' && i.last_viewed_at
                ? `Viewed ${relativeTime(i.last_viewed_at)}`
                : `Updated ${relativeTime(i.updated_at)}`,
            }))}
          />
        )}
        <LoadMore hasNext={list.hasNextPage} loading={list.isFetchingNextPage} onMore={() => void list.fetchNextPage()} />
      </div>
    </div>
  );
}
