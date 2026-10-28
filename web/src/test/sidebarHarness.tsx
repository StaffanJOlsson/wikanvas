// Renders the sidebar the way AppShell does (with the overlays and a <main>), over a MockApi with collections, trees
// and stars, and exposes the item and collection actions the drag-and-drop monitor uses.
import { screen, waitFor } from '@testing-library/react';
import type { Collection } from '../api/gen/Collection';
import type { ItemDetail } from '../api/gen/ItemDetail';
import type { ItemSummary } from '../api/gen/ItemSummary';
import { MAIN_ID } from '../app/hotkeys';
import { useCollectionActions } from '../components/collection/useCollectionActions';
import { useItemActions, type ItemActions } from '../components/item/useItemActions';
import { Overlays } from '../components/overlays/Overlays';
import { Sidebar } from '../components/sidebar/Sidebar';
import { EXPANDED_KEY, reloadExpanded } from '../state/expanded';
import { makeMe, makeSidebar, mockApi, treeReply, type MockApi } from './mockApi';
import { render } from './render';

export interface Probe {
  actions: ItemActions;
  collections: ReturnType<typeof useCollectionActions>;
}

function ProbeActions({ probe }: { probe: Partial<Probe> }) {
  probe.actions = useItemActions();
  probe.collections = useCollectionActions();
  return null;
}

export interface SidebarSetup {
  collections: Collection[];
  trees?: Record<string, ItemSummary[]>;
  stars?: ItemSummary[];
  draftsCount?: number;
  expanded?: string[];
  route?: string;
  /** The item the route shows (for `/doc/…` routes). */
  detail?: ItemDetail;
  api?: MockApi;
}

export function renderSidebar(setup: SidebarSetup) {
  const api = setup.api ?? mockApi();
  api.on('GET', '/auth/me', [200, makeMe()]);
  api.on('GET', '/sidebar', [200, makeSidebar(setup.collections, setup.stars ?? [], setup.draftsCount ?? 0)]);
  for (const c of setup.collections) {
    api.on('GET', `/collections/${c.id}/tree`, treeReply(c, setup.trees?.[c.id] ?? []));
  }
  if (setup.detail) api.on('GET', `/items/${setup.detail.item.url_id}`, [200, setup.detail]);
  localStorage.setItem(EXPANDED_KEY, JSON.stringify(setup.expanded ?? []));
  reloadExpanded();
  const probe: Partial<Probe> = {};
  const result = render(
    <>
      <Sidebar />
      <main id={MAIN_ID} tabIndex={-1} />
      <Overlays />
      <ProbeActions probe={probe} />
    </>,
    { route: setup.route ?? '/home' },
  );
  return { ...result, api, probe: probe as Probe };
}

/** Waits until the tree shows a row. */
export async function findRow(name: string): Promise<HTMLElement> {
  return screen.findByRole('treeitem', { name });
}

/** The tree's row names, in order. */
export function rowNames(): string[] {
  return screen.queryAllByRole('treeitem').map((r) => r.getAttribute('aria-label') ?? '');
}

export async function waitForRows(expected: string[]): Promise<void> {
  await waitFor(() => {
    if (JSON.stringify(rowNames()) !== JSON.stringify(expected)) throw new Error(`rows ${JSON.stringify(rowNames())}`);
  });
}
