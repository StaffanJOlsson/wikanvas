// The header of the list and settings pages: the sidebar toggle, the page title and the page's own buttons.
import type { ReactNode } from 'react';
import { SidebarToggle } from './SidebarToggle';

export function PageHeader({ title, actions, children }: { title: string; actions?: ReactNode; children?: ReactNode }) {
  return (
    <header className="page-header">
      <div className="page-header-row">
        <SidebarToggle />
        <h1>{title}</h1>
        {actions && <div className="page-actions">{actions}</div>}
      </div>
      {children}
    </header>
  );
}
