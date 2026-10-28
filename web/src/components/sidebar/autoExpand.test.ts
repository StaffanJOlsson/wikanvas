// Auto-expand after 700 ms of hovering "inside" a collapsed row while dragging (§9.6.3), driven through the same
// resolution the drag monitor uses (resolveItemDrop → autoExpandKey → AutoExpandTimer).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { makeCollection } from '../../test/mockApi';
import { items, rowIndex, treeCase } from '../../test/treeFixtures';
import { buildTree, type Forest } from '../tree/buildTree';
import { resolveItemDrop } from '../tree/drop';
import { visibleRows } from '../tree/visibleRows';
import { AUTO_EXPAND_MS, AutoExpandTimer, autoExpandKey } from './autoExpand';

const rect = { top: 100, height: 40, left: 0 };
const INSIDE = rect.top + 20;
const BEFORE = rect.top + 2;

// c1: A [A1], B [B1], C, D — only c1 expanded, so A and B are collapsed rows with children; D is dragged.
const list = items('c1', 'A', 'A>A1', 'B', 'B>B1', 'C', 'D');
const { forests, rows } = treeCase(list, []);
const dragged = { id: 'D', collectionId: 'c1' };
const over = (key: string, clientY: number) => {
  const index = rowIndex(rows, key);
  return autoExpandKey(rows[index]!, resolveItemDrop({ rows, index, dragged, forests, clientX: 40, clientY, rect }));
};

afterEach(() => {
  vi.useRealTimers();
});

describe('auto-expand while dragging', () => {
  it('only the "inside" zone of a collapsed row with children counts', () => {
    expect(over('A', INSIDE)).toBe('A');
    expect(over('A', BEFORE)).toBeNull();
    expect(over('C', INSIDE)).toBeNull(); // no children
    const open = treeCase(list, ['A']);
    const a = rowIndex(open.rows, 'A');
    const resolved = resolveItemDrop({ rows: open.rows, index: a, dragged, forests: open.forests, clientX: 40, clientY: INSIDE, rect });
    expect(autoExpandKey(open.rows[a]!, resolved)).toBeNull(); // already expanded
  });

  it('a collapsed collection row expands too', () => {
    const c2 = makeCollection({ id: 'c2', item_count: 1 });
    const c1 = makeCollection({ id: 'c1', item_count: list.length });
    const fs = new Map<string, Forest | undefined>([['c1', buildTree('c1', list, 'manual')], ['c2', undefined]]);
    const rs = visibleRows({ collections: [c1, c2], forests: fs, expanded: new Set(['c1']) });
    const i = rowIndex(rs, 'c2');
    const resolved = resolveItemDrop({ rows: rs, index: i, dragged, forests: fs, clientX: 40, clientY: INSIDE, rect });
    expect(autoExpandKey(rs[i]!, resolved)).toBe('c2');
  });

  it('expands after 700 ms on the same row, not before', () => {
    vi.useFakeTimers();
    const expand = vi.fn();
    const timer = new AutoExpandTimer(expand);
    timer.hover(over('A', INSIDE));
    vi.advanceTimersByTime(AUTO_EXPAND_MS - 100);
    // Drag updates on the same row do not restart the wait.
    timer.hover(over('A', INSIDE));
    vi.advanceTimersByTime(99);
    expect(expand).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(expand).toHaveBeenCalledExactlyOnceWith('A');
    vi.advanceTimersByTime(AUTO_EXPAND_MS * 3);
    timer.hover(over('A', INSIDE));
    vi.advanceTimersByTime(AUTO_EXPAND_MS * 3);
    expect(expand).toHaveBeenCalledOnce();
  });

  it('moving to another zone or row, or dropping, cancels the wait', () => {
    vi.useFakeTimers();
    const expand = vi.fn();
    const timer = new AutoExpandTimer(expand);
    timer.hover(over('A', INSIDE));
    vi.advanceTimersByTime(AUTO_EXPAND_MS - 1);
    timer.hover(over('A', BEFORE));
    vi.advanceTimersByTime(AUTO_EXPAND_MS);
    expect(expand).not.toHaveBeenCalled();

    timer.hover(over('A', INSIDE));
    vi.advanceTimersByTime(AUTO_EXPAND_MS - 1);
    timer.hover(over('B', INSIDE)); // another row starts its own 700 ms
    vi.advanceTimersByTime(AUTO_EXPAND_MS - 1);
    expect(expand).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(expand).toHaveBeenCalledExactlyOnceWith('B');

    timer.hover(over('A', INSIDE));
    timer.cancel(); // the drop
    vi.advanceTimersByTime(AUTO_EXPAND_MS * 2);
    expect(expand).toHaveBeenCalledOnce();
  });
});
