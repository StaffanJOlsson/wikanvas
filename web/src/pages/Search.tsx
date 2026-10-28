// Search (docs/ARCHITECTURE.md §9.6.6): query box, filters (collection, kind, updated after, include archived; no
// drafts filter, the API default `include_drafts=true` applies, D23), sort, and results with path and marked snippets.
// The parameters live in the URL, so a search can be bookmarked and Back works.
import { useEffect, useMemo, useRef, useState } from 'react';
import { Link, useSearchParams } from 'react-router';
import type { ItemKind } from '../api/gen/ItemKind';
import type { SearchSort } from '../api/gen/SearchSort';
import { flattenPages } from '../api/queries/lists';
import { useSearch, type SearchParams } from '../api/queries/search';
import { useSidebar } from '../api/queries/tree';
import { KindIcon } from '../components/item/KindIcon';
import { ItemPath } from '../components/list/ItemPath';
import { LoadMore } from '../components/list/ItemList';
import { Snippet } from '../components/search/Snippet';
import { PageHeader } from '../components/topbar/PageHeader';
import { relativeTime } from '../lib/time';
import { itemPath } from '../lib/urls';

export const SEARCH_DEBOUNCE_MS = 250;

const PERIODS = { day: 1, week: 7, month: 30, year: 365 } as const;
type Period = keyof typeof PERIODS;

/** The start of the local day `days` days ago, as an RFC 3339 timestamp (stable within a day, so cache keys are). */
export function updatedAfter(period: string | null, now: number = Date.now()): string | undefined {
  if (!period || !(period in PERIODS)) return undefined;
  const d = new Date(now - PERIODS[period as Period] * 86_400_000);
  d.setHours(0, 0, 0, 0);
  return d.toISOString();
}

export default function Search() {
  const [params, setParams] = useSearchParams();
  const sidebar = useSidebar();
  const q = params.get('q') ?? '';
  const [text, setText] = useState(q);
  // The query this page wrote into the URL itself; only other changes (Back, a link) replace what is typed.
  const written = useRef(q);

  useEffect(() => {
    if (q !== written.current) {
      written.current = q;
      setText(q);
    }
  }, [q]);
  useEffect(() => {
    if (text === q || (text.trim() === '' && q === '')) return;
    const t = setTimeout(() => {
      written.current = text.trim() ? text : '';
      setParams((p) => {
        const n = new URLSearchParams(p);
        if (text.trim()) n.set('q', text);
        else n.delete('q');
        return n;
      }, { replace: true });
    }, SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [text, q, setParams]);

  const setParam = (k: string, v: string | null) => setParams((p) => {
    const n = new URLSearchParams(p);
    if (v) n.set(k, v);
    else n.delete(k);
    return n;
  }, { replace: true });

  const kind = params.get('kind');
  const sort = params.get('sort') === 'updated' ? 'updated' : 'relevance';
  const period = params.get('updated');
  const search: SearchParams = useMemo(() => ({
    q,
    collection_id: params.get('collection') || undefined,
    kind: kind === 'doc' || kind === 'board' ? (kind as ItemKind) : undefined,
    updated_after: updatedAfter(period),
    include_archived: params.get('archived') === '1' ? true : undefined,
    sort: sort as SearchSort,
  }), [q, params, kind, period, sort]);
  const results = useSearch(search);
  const hits = flattenPages(results.data);

  return (
    <div className="page search-page">
      <PageHeader title="Search">
        <form role="search" className="search-form" onSubmit={(e) => {
          e.preventDefault();
          written.current = text.trim() ? text : '';
          setParam('q', text.trim() ? text : null);
        }}>
          <input
            type="search"
            className="input search-input"
            aria-label="Search"
            placeholder="Search documents and boards…"
            autoFocus
            maxLength={500}
            value={text}
            onChange={(e) => setText(e.target.value)}
          />
        </form>
        <div className="search-filters">
          <label>
            <span className="visually-hidden">Collection</span>
            <select value={params.get('collection') ?? ''} onChange={(e) => setParam('collection', e.target.value || null)}>
              <option value="">All collections</option>
              {sidebar.data?.collections.map((c) => <option key={c.id} value={c.id}>{c.name}</option>)}
            </select>
          </label>
          <label>
            <span className="visually-hidden">Kind</span>
            <select value={kind ?? ''} onChange={(e) => setParam('kind', e.target.value || null)}>
              <option value="">Documents and boards</option>
              <option value="doc">Documents</option>
              <option value="board">Boards</option>
            </select>
          </label>
          <label>
            <span className="visually-hidden">Updated</span>
            <select value={period ?? ''} onChange={(e) => setParam('updated', e.target.value || null)}>
              <option value="">Any time</option>
              <option value="day">Updated in the last day</option>
              <option value="week">Updated in the last week</option>
              <option value="month">Updated in the last month</option>
              <option value="year">Updated in the last year</option>
            </select>
          </label>
          <label className="checkbox">
            <input type="checkbox" checked={params.get('archived') === '1'} onChange={(e) => setParam('archived', e.target.checked ? '1' : null)} />
            <span>Include archived</span>
          </label>
          <label>
            <span className="visually-hidden">Sort</span>
            <select value={sort} onChange={(e) => setParam('sort', e.target.value === 'updated' ? 'updated' : null)}>
              <option value="relevance">Most relevant</option>
              <option value="updated">Recently updated</option>
            </select>
          </label>
        </div>
      </PageHeader>
      {!q.trim() && <p className="list-empty">Type to search titles and text. Use "quotes" for a phrase and -word to leave a word out.</p>}
      {results.isError && <p className="form-error" role="alert">{results.error.message}</p>}
      {q.trim() && results.isSuccess && hits.length === 0 && <p className="list-empty">Nothing found.</p>}
      {hits.length > 0 && (
        <ul className="search-results" aria-label="Results">
          {hits.map((h) => (
            <li key={h.item.id} className="search-result">
              <Link to={itemPath(h.item)} className="search-result-title">
                <KindIcon kind={h.item.kind} />
                <span>{h.item.title || 'Untitled'}</span>
              </Link>
              <div className="search-result-meta">
                <ItemPath collectionId={h.item.collection_id} path={h.path} />
                <span> · Updated {relativeTime(h.item.updated_at)}</span>
              </div>
              <Snippet parts={h.snippet} />
            </li>
          ))}
        </ul>
      )}
      <LoadMore hasNext={results.hasNextPage} loading={results.isFetchingNextPage} onMore={() => void results.fetchNextPage()} />
    </div>
  );
}
