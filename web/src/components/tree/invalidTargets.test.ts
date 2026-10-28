import { describe, expect, it } from 'vitest';
import { buildTree, type Forest } from './buildTree';
import { isInvalidTarget, resolveItemDrop } from './drop';
import { visibleRows } from './visibleRows';
import { makeCollection } from '../../test/mockApi';
import { items, rowIndex, treeCase } from '../../test/treeFixtures';

const rect = { top: 0, height: 40, left: 0 };

describe('invalid drop targets', () => {
  const list = items('c1', 'A', 'A>A1', 'A1>A1a', 'A>A2', 'B');
  const { forest, forests, rows } = treeCase(list, ['A', 'A1']);
  const dragged = { id: 'A', collectionId: 'c1' };

  it('the dragged item and its descendants are refused', () => {
    expect(isInvalidTarget(forest, 'A', 'A')).toBe(true);
    expect(isInvalidTarget(forest, 'A', 'A1')).toBe(true);
    expect(isInvalidTarget(forest, 'A', 'A1a')).toBe(true);
    expect(isInvalidTarget(forest, 'A', 'A2')).toBe(true);
    expect(isInvalidTarget(forest, 'A', 'B')).toBe(false);
    expect(isInvalidTarget(forest, 'A1', 'A')).toBe(false);
  });

  it('resolves to no drop (no indicator) in every zone of those rows', () => {
    for (const key of ['A', 'A1', 'A1a', 'A2']) {
      for (const y of [2, 20, 38]) {
        expect(resolveItemDrop({ rows, index: rowIndex(rows, key), dragged, forests, clientX: 0, clientY: y, rect })).toBeNull();
      }
    }
    expect(resolveItemDrop({ rows, index: rowIndex(rows, 'B'), dragged, forests, clientX: 0, clientY: 20, rect })).not.toBeNull();
    expect(resolveItemDrop({ rows, index: rowIndex(rows, 'c1'), dragged, forests, clientX: 0, clientY: 20, rect })).not.toBeNull();
  });

  it('descendants stay refused in a non-manual group', () => {
    const t = treeCase(list, ['A', 'A1'], 'title');
    expect(resolveItemDrop({ rows: t.rows, index: rowIndex(t.rows, 'A1a'), dragged: { id: 'A1', collectionId: 'c1' }, forests: t.forests, clientX: 0, clientY: 2, rect })).toBeNull();
  });

  it('rows of another collection with the same ids are not descendants', () => {
    const other = buildTree('c2', items('c2', 'Z'), 'manual');
    const fs = new Map<string, Forest | undefined>([['c1', forest], ['c2', other]]);
    const rs = visibleRows({
      collections: [makeCollection({ id: 'c1' }), makeCollection({ id: 'c2' })],
      forests: fs,
      expanded: new Set(['c1', 'c2']),
    });
    expect(resolveItemDrop({ rows: rs, index: rowIndex(rs, 'Z'), dragged, forests: fs, clientX: 0, clientY: 20, rect })!.instruction)
      .toEqual({ type: 'child', targetId: 'Z' });
  });
});
