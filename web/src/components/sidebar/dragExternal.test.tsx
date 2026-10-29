// The sidebar as a drag source for boards (docs/ARCHITECTURE.md §18.6 (c), D70): a document row's pragmatic-dnd
// draggable carries `application/x-wikanvas-item` as native data (read by the board frame); board and collection rows
// carry none, and the tree's own drag data is unchanged.
import { act, screen, waitFor } from '@testing-library/react';
import { replaceCachedSummary } from '../../api/queries/tree';
import { describe, expect, it, vi } from 'vitest';
import { makeCollection, makeItem } from '../../test/mockApi';
import { findRow, renderSidebar } from '../../test/sidebarHarness';

type DraggableArgs = {
  element: HTMLElement;
  getInitialData(): Record<string, unknown>;
  getInitialDataForExternal?(): Record<string, string>;
};

const captured = vi.hoisted(() => [] as unknown[]);

vi.mock('@atlaskit/pragmatic-drag-and-drop/adapter/element-adapter', async (importOriginal) => {
  const mod = await importOriginal<typeof import('@atlaskit/pragmatic-drag-and-drop/adapter/element-adapter')>();
  return {
    ...mod,
    draggable: (args: Parameters<typeof mod.draggable>[0]) => {
      captured.push(args);
      return mod.draggable(args);
    },
  };
});

/** The latest draggable registered for a row element. */
function argsOf(row: HTMLElement): DraggableArgs {
  const all = (captured as DraggableArgs[]).filter((a) => a.element === row);
  const last = all.at(-1);
  if (!last) throw new Error('no draggable for the row');
  return last;
}

describe('sidebar rows as drag sources for boards', () => {
  it('a document row carries application/x-wikanvas-item; board and collection rows carry none', async () => {
    const c1 = makeCollection({ id: 'c1', name: 'Handbook', item_count: 2 });
    const doc = makeItem({ id: '0192f0e0-0000-7000-8000-0000000000d1', collection_id: 'c1', title: 'Meeting notes', sort_key: 'a0' });
    const board = makeItem({ id: '0192f0e0-0000-7000-8000-0000000000d2', collection_id: 'c1', title: 'Plan', kind: 'board', sort_key: 'a1' });
    const { queryClient } = renderSidebar({ collections: [c1], trees: { c1: [doc, board] }, expanded: ['c1'] });
    const docRow = await findRow('Meeting notes');
    const boardRow = await findRow('Plan');
    const collectionRow = screen.getByRole('treeitem', { name: 'Handbook' });

    const d = argsOf(docRow);
    expect(d.getInitialDataForExternal?.()).toEqual({
      'application/x-wikanvas-item': JSON.stringify({ v: 1, id: doc.id, kind: 'doc', title: 'Meeting notes' }),
    });
    // The tree's own data is as before.
    expect(d.getInitialData()).toMatchObject({ wikanvasDrag: 'item', itemId: doc.id, collectionId: 'c1' });
    expect(argsOf(boardRow).getInitialDataForExternal).toBeUndefined();
    expect(argsOf(collectionRow).getInitialDataForExternal).toBeUndefined();

    // A rename since the row mounted is carried (the title is read when the drag starts).
    act(() => replaceCachedSummary(queryClient, { ...doc, title: 'Renamed notes' }));
    await findRow('Renamed notes');
    await waitFor(() =>
      expect(JSON.parse(argsOf(docRow).getInitialDataForExternal?.()['application/x-wikanvas-item'] ?? '{}').title).toBe('Renamed notes'),
    );
  });
});
