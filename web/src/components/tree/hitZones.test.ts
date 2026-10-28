import { describe, expect, it } from 'vitest';
import { items, rowIndex, treeCase } from '../../test/treeFixtures';
import { resolveItemDrop } from './drop';
import { edgeZone, indentX, itemZone, outdentTarget, pointerLevel } from './hitZones';

const rect = { top: 100, height: 40, left: 0 };

describe('hit zones', () => {
  it('item rows: before 25% / inside 50% / after 25%', () => {
    expect(itemZone(100, rect)).toBe('before');
    expect(itemZone(109.9, rect)).toBe('before');
    expect(itemZone(110, rect)).toBe('inside');
    expect(itemZone(120, rect)).toBe('inside');
    expect(itemZone(129.9, rect)).toBe('inside');
    expect(itemZone(130, rect)).toBe('after');
    expect(itemZone(139, rect)).toBe('after');
  });

  it('collection and starred rows: before/after 50/50', () => {
    expect(edgeZone(100, rect)).toBe('before');
    expect(edgeZone(119, rect)).toBe('before');
    expect(edgeZone(120, rect)).toBe('after');
    expect(edgeZone(139, rect)).toBe('after');
  });

  it('pointer levels follow the 16 px indent', () => {
    expect(indentX(1)).toBe(24);
    expect(indentX(12)).toBe(indentX(10));
    expect(pointerLevel(0, 0)).toBe(0);
    expect(pointerLevel(23, 0)).toBe(0);
    expect(pointerLevel(24, 0)).toBe(1);
    expect(pointerLevel(40, 0)).toBe(2);
  });

  describe('outdent on the last child by pointer x', () => {
    // c1: A [A1, A2 [A2a]], B — with A and A2 expanded.
    const list = items('c1', 'A', 'A>A1', 'A>A2', 'A2>A2a', 'B');
    const { forest, forests, rows } = treeCase(list, ['A', 'A2']);
    const dragged = { id: 'B', collectionId: 'c1' };
    const after = (key: string, clientX: number) => resolveItemDrop({
      rows, index: rowIndex(rows, key), dragged, forests, clientX, clientY: rect.top + 35, rect,
    })!.instruction;

    it('right of the row indent: after the row itself', () => {
      expect(after('A2a', indentX(3) + 4)).toEqual({ type: 'after', targetId: 'A2a' });
    });

    it('one step left: after the parent; two steps: after the grandparent', () => {
      expect(after('A2a', indentX(2) + 4)).toEqual({ type: 'after', targetId: 'A2' });
      expect(after('A2a', indentX(1) + 4)).toEqual({ type: 'after', targetId: 'A' });
      expect(after('A2a', 0)).toEqual({ type: 'after', targetId: 'A' });
    });

    it('the indicator line is drawn at the depth the drop goes to', () => {
      const r = resolveItemDrop({ rows, index: rowIndex(rows, 'A2a'), dragged, forests, clientX: indentX(1) + 4, clientY: rect.top + 35, rect })!;
      expect(r.indicator).toEqual({ rowKey: 'A2a', kind: 'line-after', depth: 1 });
    });

    it('not on a child that has a next sibling', () => {
      expect(after('A1', 0)).toEqual({ type: 'after', targetId: 'A1' });
    });

    it('below an expanded parent the drop goes before its first child', () => {
      expect(after('A2', 0)).toEqual({ type: 'before', targetId: 'A2a' });
    });

    it('outdentTarget stops at a node that is not the last child', () => {
      expect(outdentTarget(forest, forest.byId.get('A2a')!, 0).item.id).toBe('A');
      expect(outdentTarget(forest, forest.byId.get('A1')!, 0).item.id).toBe('A1');
    });
  });
});
