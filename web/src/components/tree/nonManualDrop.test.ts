import { describe, expect, it } from 'vitest';
import { items, rowIndex, treeCase } from '../../test/treeFixtures';
import { collapseNonManual, resolveItemDrop } from './drop';
import { dropToMove } from './positions';

const rect = { top: 0, height: 40, left: 0 };
const BEFORE = 2;
const INSIDE = 20;
const AFTER = 38;

describe('drops in a non-manual sibling group', () => {
  const list = items('c1', 'A', 'A>A1', 'A>A2', 'B', 'C');

  it('before/after a nested item becomes "into the parent", and the parent row is highlighted', () => {
    const { rows, forests } = treeCase(list, ['A'], 'title');
    for (const y of [BEFORE, AFTER]) {
      const r = resolveItemDrop({ rows, index: rowIndex(rows, 'A2'), dragged: { id: 'C', collectionId: 'c1' }, forests, clientX: 200, clientY: y, rect });
      expect(r).toEqual({ instruction: { type: 'child', targetId: 'A' }, indicator: { rowKey: 'A', kind: 'box', depth: 1 } });
    }
  });

  it('before/after a top-level item becomes "into the collection"', () => {
    const { rows, forests } = treeCase(list, [], 'updated');
    const r = resolveItemDrop({ rows, index: rowIndex(rows, 'B'), dragged: { id: 'A1', collectionId: 'c1' }, forests, clientX: 200, clientY: BEFORE, rect });
    expect(r).toEqual({ instruction: { type: 'collection', collectionId: 'c1' }, indicator: { rowKey: 'c1', kind: 'box', depth: 0 } });
  });

  it('"inside" stays inside, and manual groups keep before/after', () => {
    const nonManual = treeCase(list, [], 'created');
    expect(resolveItemDrop({ rows: nonManual.rows, index: rowIndex(nonManual.rows, 'B'), dragged: { id: 'C', collectionId: 'c1' }, forests: nonManual.forests, clientX: 200, clientY: INSIDE, rect })!.instruction)
      .toEqual({ type: 'child', targetId: 'B' });
    const manual = treeCase(list, [], 'manual');
    expect(resolveItemDrop({ rows: manual.rows, index: rowIndex(manual.rows, 'B'), dragged: { id: 'C', collectionId: 'c1' }, forests: manual.forests, clientX: 200, clientY: BEFORE, rect })!.instruction)
      .toEqual({ type: 'before', targetId: 'B' });
  });

  it('the move appends to the parent without a position (the sort decides)', () => {
    const { forest, forests } = treeCase(list, ['A'], 'title');
    const instruction = collapseNonManual(forest, { type: 'after', targetId: 'A1' });
    expect(instruction).toEqual({ type: 'child', targetId: 'A' });
    const c = list.find((i) => i.id === 'C')!;
    expect(dropToMove(instruction, c, forests)).toEqual({ collection_id: 'c1', parent_id: 'A' });
    // Already in that group: nothing to do.
    const a1 = list.find((i) => i.id === 'A1')!;
    expect(dropToMove(instruction, a1, forests)).toBeNull();
  });
});
