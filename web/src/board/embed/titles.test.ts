// Titles (docs/ARCHITECTURE.md §18.7): the engine's title map follows the cached details of the cards' targets,
// debounced 250 ms, sent only when it changed, and again after every open.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import { replaceCachedSummary } from '../../api/queries/tree';
import { createTestQueryClient } from '../../test/render';
import type { BoardFrameApi } from '../bridge';
import { createFakeFrame } from '../fakeFrame';
import { DOC_1, DOC_2, DOC_3, docItem } from '../testkit';
import { cachedItem, TITLE_DEBOUNCE_MS, TitleSync } from './titles';

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

function setup() {
  const qc = createTestQueryClient();
  const fake = createFakeFrame();
  const sync = new TitleSync({ qc, frame: () => fake.api as BoardFrameApi });
  sync.start();
  const sent = () => fake.calls.filter((c) => c.method === 'setEmbedTitles').map((c) => c.args[0]);
  return { qc, fake, sync, sent };
}

describe('title sync', () => {
  it('sends trimmed titles ("Untitled" when blank) of the cached targets in items, 250 ms later', () => {
    const { qc, sync, sent } = setup();
    qc.setQueryData(['item', DOC_1], docItem(DOC_1, { title: '  Meeting notes ' }));
    qc.setQueryData(['item', DOC_2], docItem(DOC_2, { title: '   ' }));
    // DOC_3 is on the board but not cached: left out.
    sync.setItems([DOC_1, DOC_2, DOC_3]);
    vi.advanceTimersByTime(TITLE_DEBOUNCE_MS - 1);
    expect(sent()).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(sent()).toEqual([{ [DOC_1]: 'Meeting notes', [DOC_2]: 'Untitled' }]);
    sync.dispose();
  });

  it('is debounced, sent only when the map changed, and follows a rename in the cache', () => {
    const { qc, sync, sent } = setup();
    sync.setItems([DOC_1]);
    qc.setQueryData(['item', DOC_1], docItem(DOC_1, { title: 'A' }));
    vi.advanceTimersByTime(100);
    qc.setQueryData(['item', DOC_1], docItem(DOC_1, { title: 'Ab' }));
    vi.advanceTimersByTime(TITLE_DEBOUNCE_MS);
    expect(sent()).toEqual([{ [DOC_1]: 'Ab' }]);
    // The same detail again (a refetch): no new call.
    qc.setQueryData(['item', DOC_1], docItem(DOC_1, { title: 'Ab' }));
    vi.advanceTimersByTime(TITLE_DEBOUNCE_MS);
    expect(sent()).toHaveLength(1);
    // A rename anywhere updates every cached detail (the sidebar's rename, the title input).
    replaceCachedSummary(qc, { ...docItem(DOC_1).item, title: 'Renamed' });
    vi.advanceTimersByTime(TITLE_DEBOUNCE_MS);
    expect(sent()).toEqual([{ [DOC_1]: 'Ab' }, { [DOC_1]: 'Renamed' }]);
    sync.dispose();
  });

  it('a detail cached under its url_id counts too; nothing is sent for no targets', () => {
    const { qc, sync, sent } = setup();
    const d: ItemDetail = docItem(DOC_2, { title: 'By url id', url_id: 'UrLiD00001' });
    qc.setQueryData(['item', 'UrLiD00001'], d);
    expect(cachedItem(qc, DOC_2)?.item.title).toBe('By url id');
    sync.setItems([]);
    vi.advanceTimersByTime(TITLE_DEBOUNCE_MS);
    expect(sent()).toEqual([]);
    sync.setItems([DOC_2]);
    vi.advanceTimersByTime(TITLE_DEBOUNCE_MS);
    expect(sent()).toEqual([{ [DOC_2]: 'By url id' }]);
    sync.dispose();
  });

  it('after an open (the engine’s map is empty) the map is sent again', () => {
    const { qc, sync, sent } = setup();
    qc.setQueryData(['item', DOC_1], docItem(DOC_1, { title: 'Kept' }));
    sync.setItems([DOC_1]);
    vi.advanceTimersByTime(TITLE_DEBOUNCE_MS);
    sync.opened();
    vi.advanceTimersByTime(TITLE_DEBOUNCE_MS);
    expect(sent()).toEqual([{ [DOC_1]: 'Kept' }, { [DOC_1]: 'Kept' }]);
    sync.dispose();
  });
});
