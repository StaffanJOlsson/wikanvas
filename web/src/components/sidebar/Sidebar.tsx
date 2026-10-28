// The sidebar (docs/ARCHITECTURE.md §9.3, §9.6.2): workspace header, nav (Home, Search, Starred; no Drafts entry or
// badge in M1, D23), STARRED, COLLECTIONS with "+ New collection…", the footer (Archive, Trash, Settings; Invite is
// hidden) and the resize handle. Below 1024 px it is a drawer (AppShell).
import { autoScrollForElements } from '@atlaskit/pragmatic-drag-and-drop-auto-scroll/element';
import { Archive, House, Plus, Search, Settings, Star, Trash2 } from 'lucide-react';
import { useEffect, useMemo, useRef, type ReactNode } from 'react';
import { Link, NavLink, useLocation } from 'react-router';
import { useMe } from '../../api/queries/auth';
import { useSidebar } from '../../api/queries/tree';
import { openDialog } from '../../state/dialogs';
import { CollectionsTree } from './CollectionsTree';
import { ResizeHandle } from './ResizeHandle';
import { StarredSection } from './StarredSection';
import { WorkspaceHeader } from './WorkspaceHeader';

export const SIDEBAR_ID = 'sidebar';

function NavItem({ to, icon, label }: { to: string; icon: ReactNode; label: string }) {
  return (
    <NavLink to={to} className="sidebar-row nav-row">
      {icon}
      <span className="row-text">{label}</span>
    </NavLink>
  );
}

export function Sidebar() {
  const me = useMe();
  const sidebar = useSidebar();
  const scroll = useRef<HTMLDivElement>(null);
  const location = useLocation();

  useEffect(() => {
    const element = scroll.current;
    if (!element) return;
    return autoScrollForElements({ element });
  }, []);

  const data = sidebar.data;
  const starredIds = useMemo(() => new Set(data?.stars.map((s) => s.id) ?? []), [data?.stars]);

  return (
    <aside id={SIDEBAR_ID} className="sidebar" aria-label="Sidebar">
      <WorkspaceHeader me={me.data} />
      <nav className="sidebar-nav" aria-label="Main">
        <NavItem to="/home" icon={<House size={16} aria-hidden />} label="Home" />
        <NavItem to="/search" icon={<Search size={16} aria-hidden />} label="Search" />
        <NavItem to="/starred" icon={<Star size={16} aria-hidden />} label="Starred" />
      </nav>
      {/* overflow is also set inline: the auto-scroller checks the computed style when it attaches. */}
      <div className="sidebar-scroll" ref={scroll} style={{ overflowY: 'auto' }}>
        {data && <StarredSection stars={data.stars} />}
        <section className="sidebar-section">
          <div className="sidebar-section-label" id="sidebar-collections">Collections</div>
          {sidebar.isError && (
            <div className="sidebar-error" role="alert">
              The sidebar could not be loaded.{' '}
              <button type="button" className="link-button" onClick={() => void sidebar.refetch()}>Retry</button>
            </div>
          )}
          {data && (
            <CollectionsTree
              collections={data.collections}
              starredIds={starredIds}
              scrollRef={scroll}
              labelledBy="sidebar-collections"
            />
          )}
          <button
            type="button"
            className="sidebar-row new-collection"
            onClick={() => openDialog({ kind: 'collection', collection: null })}
          >
            <Plus size={16} aria-hidden />
            <span className="row-text">New collection…</span>
          </button>
        </section>
      </div>
      <nav className="sidebar-footer" aria-label="Archive, Trash and Settings">
        <NavItem to="/archive" icon={<Archive size={16} aria-hidden />} label="Archive" />
        <NavItem to="/trash" icon={<Trash2 size={16} aria-hidden />} label="Trash" />
        <Link
          to="/settings/account"
          className="sidebar-row nav-row"
          aria-current={location.pathname.startsWith('/settings/') ? 'page' : undefined}
        >
          <Settings size={16} aria-hidden />
          <span className="row-text">Settings</span>
        </Link>
      </nav>
      <ResizeHandle />
    </aside>
  );
}
