// The signed-in layout (docs/ARCHITECTURE.md §9.3): sidebar | main, with the board frame host mounted once next to
// the routes (W6's <BoardFrameHost/>; PLAN W3 "Owns"), and the overlays. Below 1024 px the sidebar is a drawer.
// Global keys: Mod+K, Mod+. and Mod+S (runHostShortcut, also fed by the board frame), and `?` outside text fields.
// Route changes flush pending saves (§9.2).
import { useEffect, useRef, type CSSProperties } from 'react';
import { Outlet, useLocation } from 'react-router';
import { useConfig } from '../api/queries/config';
import BoardFrameHost from '../board/BoardFrameHost';
import { Overlays } from '../components/overlays/Overlays';
import { Sidebar } from '../components/sidebar/Sidebar';
import { useMediaQuery } from '../lib/useMediaQuery';
import { openDialog, toggleQuickSwitcher } from '../state/dialogs';
import { flushPending, installBeforeUnload } from '../state/pending';
import { DRAWER_QUERY, setDrawerOpen, toggleSidebar, useUi } from '../state/ui';
import { MAIN_ID, onHostShortcut, runHostShortcut } from './hotkeys';
import { UpdateBanner } from './UpdateBanner';

/** Whether a key event target takes text (where `?` is just a character). */
export function isTextField(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false;
  if (t.isContentEditable) return true;
  const tag = t.tagName;
  if (tag === 'TEXTAREA' || tag === 'SELECT') return true;
  if (tag !== 'INPUT') return false;
  const type = (t as HTMLInputElement).type;
  return !['checkbox', 'radio', 'button', 'submit', 'reset', 'range', 'color', 'file'].includes(type);
}

export function AppShell() {
  const location = useLocation();
  // The config this page starts with is the baseline of the update check (§9.1).
  useConfig();
  const narrow = useMediaQuery(DRAWER_QUERY);
  const width = useUi((s) => s.sidebarWidth);
  const collapsed = useUi((s) => s.sidebarCollapsed);
  const drawerOpen = useUi((s) => s.drawerOpen);

  useEffect(() => installBeforeUnload(), []);

  const path = useRef(location.pathname);
  useEffect(() => {
    if (path.current === location.pathname) return;
    path.current = location.pathname;
    void flushPending();
    setDrawerOpen(false);
  }, [location.pathname]);

  useEffect(() => {
    const offs = [
      onHostShortcut('quick-switcher', toggleQuickSwitcher),
      onHostShortcut('toggle-sidebar', () => toggleSidebar()),
    ];
    // Escape is left to the focused widget (the frame forwards its own, §9.5).
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      if (e.key !== 'Escape' && runHostShortcut(e)) return;
      if (e.key === '?' && !e.ctrlKey && !e.metaKey && !e.altKey && !isTextField(e.target)) {
        e.preventDefault();
        openDialog({ kind: 'shortcuts' });
      }
    };
    window.addEventListener('keydown', onKey);
    return () => {
      offs.forEach((off) => off());
      window.removeEventListener('keydown', onKey);
    };
  }, []);

  const mode = narrow ? 'drawer' : collapsed ? 'collapsed' : 'expanded';
  const sidebarHidden = narrow ? !drawerOpen : collapsed;
  return (
    <div
      className="app-shell"
      data-sidebar={mode}
      data-drawer-open={narrow && drawerOpen ? '' : undefined}
      style={{ '--sidebar-width': `${width}px` } as CSSProperties}
    >
      <UpdateBanner />
      <div className="sidebar-slot" inert={sidebarHidden}>
        <Sidebar />
      </div>
      {narrow && drawerOpen && <div className="drawer-backdrop" onClick={() => setDrawerOpen(false)} aria-hidden />}
      <main id={MAIN_ID} className="app-main" tabIndex={-1}>
        <Outlet />
      </main>
      <BoardFrameHost />
      <Overlays />
    </div>
  );
}
