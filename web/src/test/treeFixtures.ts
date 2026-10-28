// Small trees for the tree tests: `items('c1', 'A', 'A>A1', 'A1>A1a', 'B')` builds a flat list where `X>Y` makes Y a
// child of X; sort keys follow the argument order.
import type { ChildOrder } from '../api/gen/ChildOrder';
import type { ItemSummary } from '../api/gen/ItemSummary';
import { buildTree, type Forest } from '../components/tree/buildTree';
import { visibleRows, type Row } from '../components/tree/visibleRows';
import { makeCollection, makeItem } from './mockApi';

export function items(collectionId: string, ...specs: string[]): ItemSummary[] {
  return specs.map((spec, i) => {
    const [parent, id] = spec.includes('>') ? spec.split('>') as [string, string] : [null, spec];
    return makeItem({ id, collection_id: collectionId, parent_id: parent, title: id, sort_key: `a${String(i).padStart(3, '0')}` });
  });
}

export interface TreeCase {
  forest: Forest;
  forests: Map<string, Forest | undefined>;
  rows: Row[];
}

/** One collection `c1` with the items, every listed id expanded (and the collection). */
export function treeCase(list: ItemSummary[], expandedIds: string[], order: ChildOrder = 'manual'): TreeCase {
  const collection = makeCollection({ id: 'c1', child_order: order, item_count: list.length });
  const forest = buildTree('c1', list, order);
  const forests = new Map<string, Forest | undefined>([['c1', forest]]);
  const rows = visibleRows({ collections: [collection], forests, expanded: new Set(['c1', ...expandedIds]) });
  return { forest, forests, rows };
}

export function rowIndex(rows: Row[], key: string): number {
  const i = rows.findIndex((r) => r.key === key);
  if (i < 0) throw new Error(`no row ${key}`);
  return i;
}
