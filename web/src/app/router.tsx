// The route table (docs/ARCHITECTURE.md §9.1), for React Router's data router. /setup and /login sit outside
// AuthGate; every other route renders inside AppShell. DocPage (W4) and BoardPage (W6) are lazy chunks.
import { createBrowserRouter, Navigate, type RouteObject } from 'react-router';
import Archive from '../pages/Archive';
import Collection from '../pages/Collection';
import Home from '../pages/Home';
import Login from '../pages/Login';
import NotFound from '../pages/NotFound';
import Search from '../pages/Search';
import Settings from '../pages/Settings';
import Setup from '../pages/Setup';
import Starred from '../pages/Starred';
import Trash from '../pages/Trash';
import { AppShell } from './AppShell';
import { AuthGate } from './AuthGate';

/** Shown in place of a lazy page while its chunk loads on the first navigation (the shell stays visible). */
function RouteLoading() {
  return null;
}

export const routes: RouteObject[] = [
  { path: '/setup', Component: Setup },
  { path: '/login', Component: Login },
  {
    Component: AuthGate,
    children: [
      {
        Component: AppShell,
        children: [
          { index: true, element: <Navigate to="/home" replace /> },
          { path: 'home', Component: Home },
          { path: 'search', Component: Search },
          { path: 'starred', Component: Starred },
          { path: 'archive', Component: Archive },
          { path: 'trash', Component: Trash },
          { path: 'settings/:section', Component: Settings },
          { path: 'collection/:slugId', Component: Collection },
          {
            path: 'doc/:slugId',
            HydrateFallback: RouteLoading,
            lazy: { Component: async () => (await import('../pages/DocPage')).default },
          },
          {
            path: 'board/:slugId',
            HydrateFallback: RouteLoading,
            lazy: { Component: async () => (await import('../pages/BoardPage')).default },
          },
          // Also /drafts: there is no Drafts page in M1 (D23).
          { path: '*', Component: NotFound },
        ],
      },
    ],
  },
];

export function createRouter() {
  return createBrowserRouter(routes);
}
