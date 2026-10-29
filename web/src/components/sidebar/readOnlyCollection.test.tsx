// The sidebar with access per collection (D152): a collection the caller may only read offers nothing that changes it
// (no create, sort, rename, move, duplicate, archive or delete; its items keep Star), refuses every drop into it, and
// its items drop nowhere in the tree; a writable one creates but its own menu, sort and reorder need `manage`. A viewer
// has no "New collection…".
import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { makeCollection, makeMe } from '../../test/mockApi';
import { findRow, renderSidebar } from '../../test/sidebarHarness';
import { items } from '../../test/treeFixtures';
import { buildTree } from '../tree/buildTree';
import { resolveItemDrop } from '../tree/drop';
import { visibleRows } from '../tree/visibleRows';

type Registered = {
  element: HTMLElement;
  getInitialData?(): Record<string, unknown>;
  canDrop?(args: { source: { data: Record<string, unknown> } }): boolean;
};

const captured = vi.hoisted(() => ({ draggables: [] as unknown[], targets: [] as unknown[] }));

vi.mock('@atlaskit/pragmatic-drag-and-drop/adapter/element-adapter', async (importOriginal) => {
  const mod = await importOriginal<typeof import('@atlaskit/pragmatic-drag-and-drop/adapter/element-adapter')>();
  return {
    ...mod,
    draggable: (args: Parameters<typeof mod.draggable>[0]) => {
      captured.draggables.push(args);
      return mod.draggable(args);
    },
    dropTargetForElements: (args: Parameters<typeof mod.dropTargetForElements>[0]) => {
      captured.targets.push(args);
      return mod.dropTargetForElements(args);
    },
  };
});

function latest(list: unknown[], row: HTMLElement): Registered | undefined {
  return (list as Registered[]).filter((a) => a.element === row).at(-1);
}

const work = makeCollection({ id: 'work', name: 'Work', my_access: 'manage', item_count: 1 });
const team = makeCollection({ id: 'team', name: 'Team', my_access: 'write', item_count: 1 });
const shared = makeCollection({ id: 'shared', name: 'Shared', my_access: 'read', item_count: 1 });
const trees = { work: items('work', 'A'), team: items('team', 'T'), shared: items('shared', 'X') };

function setup() {
  return renderSidebar({ collections: [work, team, shared], trees, expanded: ['work', 'team', 'shared'] });
}

function menuEntries(button: HTMLElement): string[] {
  fireEvent.click(button);
  const entries = [...screen.getByRole('menu').querySelectorAll('[role^="menuitem"]')].map((b) => b.textContent ?? '');
  fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
  return entries;
}

describe('sidebar access per collection', () => {
  it('a read-only collection offers no create, sort or menu, and its items only Star', async () => {
    setup();
    const x = await findRow('X');
    const sharedRow = screen.getByRole('treeitem', { name: 'Shared' });
    expect(within(sharedRow).queryAllByRole('button')).toHaveLength(0);
    expect(within(x).queryByRole('button', { name: 'New inside X' })).toBeNull();
    expect(menuEntries(within(x).getByRole('button', { name: 'X menu' }))).toEqual(['Star']);

    // Writable, not managed: create, but no sort, Edit… or Delete….
    const teamRow = screen.getByRole('treeitem', { name: 'Team' });
    expect(within(teamRow).queryByRole('button', { name: 'Sort' })).toBeNull();
    expect(within(teamRow).getByRole('button', { name: 'New in Team' })).toBeTruthy();
    expect(menuEntries(within(teamRow).getByRole('button', { name: 'Team menu' }))).toEqual(['New document', 'New board']);
    const t = await findRow('T');
    expect(menuEntries(within(t).getByRole('button', { name: 'T menu' }))).toContain('Delete');

    // Managed: everything.
    const workRow = screen.getByRole('treeitem', { name: 'Work' });
    expect(within(workRow).getByRole('button', { name: 'Sort' })).toBeTruthy();
    const workMenu = menuEntries(within(workRow).getByRole('button', { name: 'Work menu' }));
    expect(workMenu).toContain('Edit…');
    expect(workMenu).toContain('Delete…');
  });

  it('the keyboard renames, deletes and moves nothing in a read-only collection', async () => {
    const { api } = setup();
    const x = await findRow('X');
    x.focus();
    for (const key of ['F2', 'Delete']) fireEvent.keyDown(x, { key });
    fireEvent.keyDown(x, { key: 'ArrowUp', altKey: true, shiftKey: true });
    fireEvent.keyDown(screen.getByRole('treeitem', { name: 'Shared' }), { key: 'F10', shiftKey: true });
    expect(screen.queryByRole('textbox', { name: 'Title' })).toBeNull();
    expect(screen.queryByRole('menu')).toBeNull();
    expect(api.calls().filter((c) => c.method !== 'GET')).toHaveLength(0);
    // A writable item still renames with F2.
    const a = await findRow('A');
    a.focus();
    fireEvent.keyDown(a, { key: 'F2' });
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Title' })).toBeTruthy());
  });

  it('nothing drops into a read-only collection, and its items drop nowhere in the tree', async () => {
    setup();
    const x = await findRow('X');
    const a = await findRow('A');
    const sharedRow = screen.getByRole('treeitem', { name: 'Shared' });
    const workRow = screen.getByRole('treeitem', { name: 'Work' });
    const teamRow = screen.getByRole('treeitem', { name: 'Team' });
    const item = (writable: boolean) => ({ source: { data: { wikanvasDrag: 'item', writable } } });

    expect(latest(captured.targets, sharedRow)!.canDrop!(item(true))).toBe(false);
    expect(latest(captured.targets, x)!.canDrop!(item(true))).toBe(false);
    expect(latest(captured.targets, workRow)!.canDrop!(item(true))).toBe(true);
    expect(latest(captured.targets, a)!.canDrop!(item(true))).toBe(true);
    expect(latest(captured.targets, workRow)!.canDrop!(item(false))).toBe(false);
    // The read-only item still drags (to a board or a document), marked as not movable.
    expect(latest(captured.draggables, x)!.getInitialData!()).toMatchObject({ itemId: 'X', writable: false });
    expect(latest(captured.draggables, a)!.getInitialData!()).toMatchObject({ itemId: 'A', writable: true });
    // Only a managed collection drags (to reorder).
    expect(latest(captured.draggables, workRow)).toBeDefined();
    expect(latest(captured.draggables, teamRow)).toBeUndefined();
    expect(latest(captured.draggables, sharedRow)).toBeUndefined();

    // The drop resolution refuses the same.
    const forests = new Map(Object.entries(trees).map(([cid, list]) => [cid, buildTree(cid, list, 'manual')]));
    const rows = visibleRows({ collections: [work, team, shared], forests, expanded: new Set(['work', 'team', 'shared']) });
    const at = (key: string) => rows.findIndex((r) => r.key === key);
    const rect = { top: 0, height: 40, left: 0 };
    const drop = (key: string, dragged: { id: string; collectionId: string; writable?: boolean }) =>
      resolveItemDrop({ rows, index: at(key), dragged, forests, clientX: 0, clientY: 20, rect });
    expect(drop('shared', { id: 'A', collectionId: 'work' })).toBeNull();
    expect(drop('X', { id: 'A', collectionId: 'work' })).toBeNull();
    expect(drop('work', { id: 'X', collectionId: 'shared', writable: false })).toBeNull();
    expect(drop('team', { id: 'A', collectionId: 'work' })).not.toBeNull();
    expect(drop('T', { id: 'A', collectionId: 'work' })).not.toBeNull();
  });

  it('a viewer has no "New collection…"', async () => {
    renderSidebar({ collections: [shared], me: makeMe({ user: { ...makeMe().user, role: 'viewer' } }) });
    await screen.findByRole('treeitem', { name: 'Shared' });
    await waitFor(() => expect(screen.queryByRole('button', { name: /New collection/ })).toBeNull());
  });
});
