import { describe, expect, it } from 'vitest';
import { makeItem } from '../../test/mockApi';
import { items } from '../../test/treeFixtures';
import { ancestorIds, buildTree, comparator, descendantIds } from './buildTree';

const ids = (nodes: { item: { id: string } }[]) => nodes.map((n) => n.item.id);

describe('buildTree', () => {
  it('nests a flat list by parent_id, with depths', () => {
    const f = buildTree('c1', items('c1', 'A', 'A>A1', 'A1>A1a', 'B', 'A>A2'), 'manual');
    expect(ids(f.roots)).toEqual(['A', 'B']);
    expect(ids(f.byId.get('A')!.children)).toEqual(['A1', 'A2']);
    expect(ids(f.byId.get('A1')!.children)).toEqual(['A1a']);
    expect(f.byId.get('A1a')!.depth).toBe(3);
    expect(f.byId.get('A1a')!.parent?.item.id).toBe('A1');
    expect([...descendantIds(f, 'A')].sort()).toEqual(['A1', 'A1a', 'A2']);
    expect(ancestorIds(f, 'A1a')).toEqual(['A', 'A1']);
  });

  it('keeps orphans as roots and survives a corrupt cycle', () => {
    const orphan = makeItem({ id: 'O', collection_id: 'c1', parent_id: 'gone' });
    const x = makeItem({ id: 'X', collection_id: 'c1', parent_id: 'Y', sort_key: 'b' });
    const y = makeItem({ id: 'Y', collection_id: 'c1', parent_id: 'X', sort_key: 'c' });
    const f = buildTree('c1', [orphan, x, y], 'manual');
    expect(f.byId.size).toBe(3);
    expect(ids(f.roots)).toContain('O');
    const all = [...f.byId.values()];
    expect(all.every((n) => Number.isFinite(n.depth))).toBe(true);
    // Every node is reachable from a root exactly once.
    const seen: string[] = [];
    const walk = (nodes: typeof f.roots) => nodes.forEach((n) => {
      seen.push(n.item.id);
      walk(n.children);
    });
    walk(f.roots);
    expect(seen.sort()).toEqual(['O', 'X', 'Y']);
  });

  describe('all four sorts', () => {
    const list = [
      makeItem({ id: 'a', collection_id: 'c', title: 'banana', sort_key: 'a2', updated_at: '2026-10-01T00:00:00.000Z', created_at: '2026-01-03T00:00:00.000Z' }),
      makeItem({ id: 'b', collection_id: 'c', title: 'Apple', sort_key: 'a0', updated_at: '2026-10-03T00:00:00.000Z', created_at: '2026-01-01T00:00:00.000Z' }),
      makeItem({ id: 'c', collection_id: 'c', title: 'cherry 10', sort_key: 'a1', updated_at: '2026-10-02T00:00:00.000Z', created_at: '2026-01-02T00:00:00.000Z' }),
      makeItem({ id: 'd', collection_id: 'c', title: 'cherry 9', sort_key: 'a3', updated_at: '2026-10-02T00:00:00.000Z', created_at: '2026-01-04T00:00:00.000Z' }),
      // A child, sorted within its own group by the same order.
      makeItem({ id: 'k2', collection_id: 'c', parent_id: 'a', title: 'zeta', sort_key: 'a0' }),
      makeItem({ id: 'k1', collection_id: 'c', parent_id: 'a', title: 'Alpha', sort_key: 'a1' }),
    ];

    it('manual: by sort_key (byte order), then id', () => {
      const f = buildTree('c', list, 'manual');
      expect(ids(f.roots)).toEqual(['b', 'c', 'a', 'd']);
      expect(ids(f.byId.get('a')!.children)).toEqual(['k2', 'k1']);
      expect(comparator('manual')(makeItem({ id: 'x', collection_id: 'c', sort_key: 'Z' }), makeItem({ id: 'y', collection_id: 'c', sort_key: 'a' }))).toBeLessThan(0);
    });

    it('title: A–Z, case-insensitive, numbers in numeric order', () => {
      const f = buildTree('c', list, 'title');
      expect(ids(f.roots)).toEqual(['b', 'a', 'd', 'c']);
      expect(ids(f.byId.get('a')!.children)).toEqual(['k1', 'k2']);
    });

    it('updated: newest first, ties by sort key', () => {
      const f = buildTree('c', list, 'updated');
      expect(ids(f.roots)).toEqual(['b', 'c', 'd', 'a']);
    });

    it('created: newest first', () => {
      const f = buildTree('c', list, 'created');
      expect(ids(f.roots)).toEqual(['d', 'a', 'c', 'b']);
    });
  });
});
