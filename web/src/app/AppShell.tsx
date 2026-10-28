// The signed-in layout (docs/ARCHITECTURE.md §9.3): sidebar | main, with the board frame host mounted once next to
// the routes. W3 replaces the sidebar placeholder and adds the overlays (quick switcher, dialogs, toasts); the
// <BoardFrameHost/> mount stays (PLAN W3 "Owns").
import { useEffect } from 'react';
import { NavLink, Outlet } from 'react-router';
import BoardFrameHost from '../board/BoardFrameHost';
import { installBeforeUnload } from '../state/pending';
import { MAIN_ID, runHostShortcut } from './hotkeys';

const nav = [
  ['/home', 'Home'],
  ['/search', 'Search'],
  ['/starred', 'Starred'],
  ['/archive', 'Archive'],
  ['/trash', 'Trash'],
  ['/settings/account', 'Settings'],
] as const;

export function AppShell() {
  useEffect(() => installBeforeUnload(), []);
  useEffect(() => {
    // Mod+K, Mod+. and Mod+S anywhere; Escape is left to the focused widget (the frame forwards its own, §9.5).
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') runHostShortcut(e);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  return (
    <div className="app-shell">
      <nav className="app-sidebar" aria-label="Sidebar">
        {nav.map(([to, label]) => (
          <NavLink key={to} to={to}>
            {label}
          </NavLink>
        ))}
      </nav>
      <main id={MAIN_ID} className="app-main" tabIndex={-1}>
        <Outlet />
      </main>
      <BoardFrameHost />
    </div>
  );
}
