import { describe, expect, it } from 'vitest';
import { makeCollection } from '../../test/mockApi';
import { items, treeCase } from '../../test/treeFixtures';
import { buildTree, type Forest } from './buildTree';
import { MAX_INDENT_LEVEL, parentRowIndex, visibleRows } from './visibleRows';

const keys = (rows: { key: string }[]) => rows.map((r) => r.key);

describe('visibleRows', () => {
  const list = items('c1', 'A', 'A>A1', 'A1>A1a', 'A>A2', 'B');

  it('shows only the collection rows while collapsed', () => {
    const c1 = makeCollection({ id: 'c1', item_count: 5 });
    const c2 = makeCollection({ id: 'c2' });
    const forests = new Map<string, Forest | undefined>([['c1', buildTree('c1', list, 'manual')], ['c2', undefined]]);
    const rows = visibleRows({ collections: [c1, c2], forests, expanded: new Set() });
    expect(keys(rows)).toEqual(['c1', 'c2']);
    expect(rows[0]).toMatchObject({ kind: 'collection', hasChildren: true, expanded: false });
    expect(rows[1]).toMatchObject({ hasChildren: false });
  });

  it('follows the expanded set: collapsed parents hide their subtree', () => {
    expect(keys(treeCase(list, []).rows)).toEqual(['c1', 'A', 'B']);
    expect(keys(treeCase(list, ['A']).rows)).toEqual(['c1', 'A', 'A1', 'A2', 'B']);
    const all = treeCase(list, ['A', 'A1']).rows;
    expect(keys(all)).toEqual(['c1', 'A', 'A1', 'A1a', 'A2', 'B']);
    expect(all.map((r) => r.depth)).toEqual([0, 1, 2, 3, 2, 1]);
    expect(all[1]).toMatchObject({ expanded: true, hasChildren: true, posInSet: 1, setSize: 2 });
    expect(all[3]).toMatchObject({ expanded: false, hasChildren: false });
    // Expanding a leaf changes nothing.
    expect(keys(treeCase(list, ['A', 'B']).rows)).toEqual(['c1', 'A', 'A1', 'A2', 'B']);
  });

  it('an expanded collection whose tree is not loaded shows no items yet', () => {
    const c1 = makeCollection({ id: 'c1', item_count: 3 });
    const rows = visibleRows({ collections: [c1], forests: new Map([['c1', undefined]]), expanded: new Set(['c1']) });
    expect(keys(rows)).toEqual(['c1']);
    expect(rows[0]).toMatchObject({ expanded: true, hasChildren: true });
  });

  it('caps the indent at level 10', () => {
    const chain = ['L1'];
    for (let i = 2; i <= 13; i++) chain.push(`L${i - 1}>L${i}`);
    const ids = chain.map((s) => s.split('>').pop()!);
    const { rows } = treeCase(items('c1', ...chain), ids);
    const deepest = rows[rows.length - 1]!;
    expect(deepest.key).toBe('L13');
    expect(deepest.depth).toBe(13);
    expect(deepest.indent).toBe(MAX_INDENT_LEVEL);
    expect(rows.find((r) => r.key === 'L9')!.indent).toBe(9);
    expect(rows.find((r) => r.key === 'L11')!.indent).toBe(10);
  });

  it('forceCollapsed hides a dragged collection without touching the expanded set', () => {
    const c1 = makeCollection({ id: 'c1' });
    const rows = visibleRows({
      collections: [c1],
      forests: new Map([['c1', buildTree('c1', list, 'manual')]]),
      expanded: new Set(['c1', 'A']),
      forceCollapsed: new Set(['c1']),
    });
    expect(keys(rows)).toEqual(['c1']);
  });

  it('parentRowIndex finds the parent row, and the collection for top-level items', () => {
    const { rows } = treeCase(list, ['A', 'A1']);
    expect(parentRowIndex(rows, keys(rows).indexOf('A1a'))).toBe(keys(rows).indexOf('A1'));
    expect(parentRowIndex(rows, keys(rows).indexOf('B'))).toBe(0);
    expect(parentRowIndex(rows, 0)).toBe(-1);
  });
});
