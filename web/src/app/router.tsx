// The route table (docs/ARCHITECTURE.md §9.1), for React Router's data router. /setup, /login and /invite/:token sit
// outside AuthGate; every other route renders inside AppShell. DocPage (W4) and BoardPage (W6) are lazy chunks; a
// chunk that fails to load runs the update check (RouteError).
import { createBrowserRouter, Navigate, type RouteObject } from 'react-router';
import type { ComponentType } from 'react';
import Archive from '../pages/Archive';
import Collection from '../pages/Collection';
import Home from '../pages/Home';
import InvitePage from '../pages/Invite';
import ItemRedirect from '../pages/ItemRedirect';
import Login from '../pages/Login';
import NotFound from '../pages/NotFound';
import Search from '../pages/Search';
import Settings from '../pages/Settings';
import Setup from '../pages/Setup';
import Starred from '../pages/Starred';
import Trash from '../pages/Trash';
import { AppShell } from './AppShell';
import { AuthGate } from './AuthGate';
import { RouteError } from './UpdateBanner';

/** Shown in place of a lazy page while its chunk loads on the first navigation (the shell stays visible). */
function RouteLoading() {
  return null;
}

/**
 * A lazily loaded page route: its chunk loads on first visit; a failed load shows RouteError (the update check).
 * React Router drops the error of a lazy `Component` on a route without a loader, so a failed import resolves to a
 * component that throws it while rendering, where the route's ErrorBoundary catches it.
 */
export function lazyPage(path: string, load: () => Promise<{ default: ComponentType }>): RouteObject {
  return {
    path,
    HydrateFallback: RouteLoading,
    ErrorBoundary: RouteError,
    lazy: {
      Component: async () => {
        try {
          return (await load()).default;
        } catch (e) {
          return function ChunkLoadFailed(): never {
            throw e;
          };
        }
      },
    },
  };
}

export const routes: RouteObject[] = [
  { path: '/setup', Component: Setup },
  { path: '/login', Component: Login },
  { path: '/invite/:token', Component: InvitePage },
  {
    Component: AuthGate,
    children: [
      {
        Component: AppShell,
        children: [
          {
            // A pathless layout: a page that throws is replaced by RouteError inside the shell.
            ErrorBoundary: RouteError,
            children: [
              { index: true, element: <Navigate to="/home" replace /> },
              { path: 'home', Component: Home },
              { path: 'search', Component: Search },
              { path: 'starred', Component: Starred },
              { path: 'archive', Component: Archive },
              { path: 'trash', Component: Trash },
              { path: 'settings/:section', Component: Settings },
              { path: 'collection/:slugId', Component: Collection },
              // An item by its UUID (the href of a document's item links): a redirect to its page.
              { path: 'item/:id', Component: ItemRedirect },
              lazyPage('doc/:slugId', () => import('../pages/DocPage')),
              lazyPage('board/:slugId', () => import('../pages/BoardPage')),
              // Also /drafts: there is no Drafts page in M1 (D23).
              { path: '*', Component: NotFound },
            ],
          },
        ],
      },
    ],
  },
];

export function createRouter() {
  return createBrowserRouter(routes);
}
