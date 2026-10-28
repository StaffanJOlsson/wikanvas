// Shows the sidebar again: rendered at the start of every top bar and page header while the sidebar is collapsed, and
// on narrow screens, where it opens the drawer (§9.6.1).
import { PanelLeft } from 'lucide-react';
import { useMediaQuery } from '../../lib/useMediaQuery';
import { DRAWER_QUERY, toggleSidebar, useUi } from '../../state/ui';

export function SidebarToggle() {
  const collapsed = useUi((s) => s.sidebarCollapsed);
  const narrow = useMediaQuery(DRAWER_QUERY);
  if (!collapsed && !narrow) return null;
  return (
    <button
      type="button"
      className="icon-button sidebar-toggle"
      aria-label="Show sidebar"
      aria-controls="sidebar"
      title="Show sidebar"
      onClick={() => toggleSidebar(narrow)}
    >
      <PanelLeft size={16} aria-hidden />
    </button>
  );
}
