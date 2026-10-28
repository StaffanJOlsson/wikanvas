import { describe, expect, it } from 'vitest';
import { buildTree, type Forest } from './buildTree';
import { currentPlace, dropToMove, keyboardInstruction, listDropPosition } from './positions';
import { items } from '../../test/treeFixtures';

// c1: A [A1, A2, A3], B, C — c2: X
const c1 = items('c1', 'A', 'A>A1', 'A>A2', 'A>A3', 'B', 'C');
const c2 = items('c2', 'X');
const forests = new Map<string, Forest | undefined>([
  ['c1', buildTree('c1', c1, 'manual')],
  ['c2', buildTree('c2', c2, 'manual')],
]);
const item = (id: string) => [...c1, ...c2].find((i) => i.id === id)!;

describe('drop → {after_id, before_id}', () => {
  it('before a sibling names the row above it and the target', () => {
    expect(dropToMove({ type: 'before', targetId: 'A2' }, item('C'), forests)).toEqual({
      collection_id: 'c1', parent_id: 'A', position: { after_id: 'A1', before_id: 'A2' },
    });
    expect(dropToMove({ type: 'before', targetId: 'A1' }, item('C'), forests)).toEqual({
      collection_id: 'c1', parent_id: 'A', position: { after_id: null, before_id: 'A1' },
    });
  });

  it('after a sibling names the target and the row below it', () => {
    expect(dropToMove({ type: 'after', targetId: 'A2' }, item('B'), forests)).toEqual({
      collection_id: 'c1', parent_id: 'A', position: { after_id: 'A2', before_id: 'A3' },
    });
    expect(dropToMove({ type: 'after', targetId: 'C' }, item('A1'), forests)).toEqual({
      collection_id: 'c1', parent_id: null, position: { after_id: 'C', before_id: null },
    });
  });

  it('never names the moved item as a neighbour', () => {
    // A2 moves below A3: its old place between A1 and A3 is skipped.
    expect(dropToMove({ type: 'after', targetId: 'A3' }, item('A2'), forests)).toEqual({
      collection_id: 'c1', parent_id: 'A', position: { after_id: 'A3', before_id: null },
    });
    // A1 moves above A3: after A2 (not after itself).
    expect(dropToMove({ type: 'before', targetId: 'A3' }, item('A1'), forests)).toEqual({
      collection_id: 'c1', parent_id: 'A', position: { after_id: 'A2', before_id: 'A3' },
    });
  });

  it('inside an item or on a collection row: the end, without a position', () => {
    expect(dropToMove({ type: 'child', targetId: 'B' }, item('C'), forests)).toEqual({ collection_id: 'c1', parent_id: 'B' });
    expect(dropToMove({ type: 'collection', collectionId: 'c2' }, item('A'), forests)).toEqual({ collection_id: 'c2', parent_id: null });
  });

  it('across collections names the target collection', () => {
    expect(dropToMove({ type: 'before', targetId: 'X' }, item('B'), forests)).toEqual({
      collection_id: 'c2', parent_id: null, position: { after_id: null, before_id: 'X' },
    });
  });

  it('a drop that changes nothing is null', () => {
    expect(dropToMove({ type: 'before', targetId: 'A3' }, item('A2'), forests)).toBeNull();
    expect(dropToMove({ type: 'after', targetId: 'A1' }, item('A2'), forests)).toBeNull();
    expect(dropToMove({ type: 'child', targetId: 'A' }, item('A3'), forests)).toBeNull();
    expect(dropToMove({ type: 'collection', collectionId: 'c1' }, item('C'), forests)).toBeNull();
  });

  it('before or after the moved item itself is null (never a self neighbour)', () => {
    expect(dropToMove({ type: 'before', targetId: 'A1' }, item('A1'), forests)).toBeNull();
    expect(dropToMove({ type: 'after', targetId: 'A1' }, item('A1'), forests)).toBeNull();
    expect(dropToMove({ type: 'before', targetId: 'B' }, item('B'), forests)).toBeNull();
  });

  it('currentPlace is the Undo target', () => {
    expect(currentPlace(forests.get('c1'), item('A2'))).toEqual({
      collection_id: 'c1', parent_id: 'A', position: { after_id: 'A1', before_id: 'A3' },
    });
    expect(currentPlace(forests.get('c1'), item('A'))).toEqual({
      collection_id: 'c1', parent_id: null, position: { after_id: null, before_id: 'B' },
    });
  });

  it('flat lists (collections, stars) reorder before/after', () => {
    const ids = ['p', 'q', 'r', 's'];
    expect(listDropPosition(ids, 's', 'q', 'before')).toEqual({ after_id: 'p', before_id: 'q' });
    expect(listDropPosition(ids, 'p', 'r', 'after')).toEqual({ after_id: 'r', before_id: 's' });
    expect(listDropPosition(ids, 'p', 's', 'after')).toEqual({ after_id: 's', before_id: null });
    expect(listDropPosition(ids, 'q', 'p', 'before')).toEqual({ after_id: null, before_id: 'p' });
    expect(listDropPosition(ids, 'q', 'r', 'before')).toBeNull();
    expect(listDropPosition(ids, 'q', 'p', 'after')).toBeNull();
    expect(listDropPosition(ids, 'q', 'q', 'after')).toBeNull();
  });

  it('keyboard moves map to drop instructions', () => {
    const f = forests.get('c1')!;
    expect(keyboardInstruction(f, 'A2', 'indent')).toEqual({ type: 'child', targetId: 'A1' });
    expect(keyboardInstruction(f, 'A1', 'indent')).toBeNull();
    expect(keyboardInstruction(f, 'A2', 'outdent')).toEqual({ type: 'after', targetId: 'A' });
    expect(keyboardInstruction(f, 'B', 'outdent')).toBeNull();
    expect(keyboardInstruction(f, 'A2', 'up')).toEqual({ type: 'before', targetId: 'A1' });
    expect(keyboardInstruction(f, 'A2', 'down')).toEqual({ type: 'after', targetId: 'A3' });
    expect(keyboardInstruction(f, 'A3', 'down')).toBeNull();
    expect(keyboardInstruction(buildTree('c1', c1, 'title'), 'A2', 'up')).toBeNull();
  });
});
