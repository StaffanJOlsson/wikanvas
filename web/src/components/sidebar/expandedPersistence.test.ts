import { describe, expect, it } from 'vitest';
import {
  EXPANDED_KEY,
  EXPANDED_MAX,
  expandAll,
  isExpanded,
  reloadExpanded,
  setExpanded,
  toggleExpanded,
  useExpandedStore,
} from '../../state/expanded';

const stored = () => JSON.parse(localStorage.getItem(EXPANDED_KEY) ?? 'null') as unknown;

describe('expanded state persistence', () => {
  it('starts empty and persists every change to quire.sidebar.expanded', () => {
    expect(useExpandedStore.getState().ids.size).toBe(0);
    setExpanded('c1', true);
    toggleExpanded('item-1');
    expect(stored()).toEqual(['c1', 'item-1']);
    toggleExpanded('c1');
    expect(stored()).toEqual(['item-1']);
    expandAll(['c2', 'item-1', 'item-2']);
    expect(stored()).toEqual(['item-1', 'c2', 'item-2']);
  });

  it('survives a reload', () => {
    expandAll(['c1', 'a', 'b']);
    setExpanded('a', false);
    // A new page load reads the stored array.
    useExpandedStore.setState({ ids: new Set() });
    reloadExpanded();
    expect(isExpanded('c1')).toBe(true);
    expect(isExpanded('a')).toBe(false);
    expect(isExpanded('b')).toBe(true);
  });

  it('ignores malformed storage', () => {
    localStorage.setItem(EXPANDED_KEY, '{not json');
    reloadExpanded();
    expect(useExpandedStore.getState().ids.size).toBe(0);
    localStorage.setItem(EXPANDED_KEY, JSON.stringify({ c1: true }));
    reloadExpanded();
    expect(useExpandedStore.getState().ids.size).toBe(0);
    localStorage.setItem(EXPANDED_KEY, JSON.stringify(['c1', 7, null, 'c2']));
    reloadExpanded();
    expect([...useExpandedStore.getState().ids]).toEqual(['c1', 'c2']);
  });

  it('does not rewrite storage when nothing changes, and keeps at most EXPANDED_MAX ids', () => {
    setExpanded('x', true);
    localStorage.setItem(EXPANDED_KEY, 'sentinel');
    setExpanded('x', true);
    expandAll(['x']);
    expect(localStorage.getItem(EXPANDED_KEY)).toBe('sentinel');
    expandAll(Array.from({ length: EXPANDED_MAX + 10 }, (_, i) => `id${i}`));
    const list = stored() as string[];
    expect(list).toHaveLength(EXPANDED_MAX);
    expect(list[list.length - 1]).toBe(`id${EXPANDED_MAX + 9}`);
  });
});
