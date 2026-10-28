import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createFakeFrame } from './fakeFrame';
import { currentItem, frameEvents, openBoard, resetHost, useHostStore, type BoardSession } from './host';

const theme = 'light' as const;

function session(itemId: string) {
  return {
    itemId,
    flush: vi.fn(async () => {}),
    onChange: vi.fn(),
    onSave: vi.fn(),
    onReady: vi.fn(),
    onLoadError: vi.fn(),
  } satisfies BoardSession;
}

beforeEach(() => resetHost());
afterEach(() => resetHost());

describe('frame events', () => {
  it('events for a non-current itemId are ignored', async () => {
    const fake = createFakeFrame();
    useHostStore.setState({ frame: fake.api });
    const a = session('a');
    const b = session('b');
    await openBoard(a, { itemId: 'a', board: '{}', name: 'A', readOnly: false, theme });
    await openBoard(b, { itemId: 'b', board: '{}', name: 'B', readOnly: false, theme });
    expect(a.flush).toHaveBeenCalledTimes(1);
    expect(currentItem()).toBe('b');

    // Late events of board a (e.g. queued in the frame before setBoard) reach nobody.
    frameEvents.onChange?.({ itemId: 'a', revision: 9 });
    frameEvents.onSave?.('a');
    frameEvents.onReady?.('a');
    frameEvents.onError?.({ itemId: 'a', kind: 'load', message: 'old board' });
    frameEvents.onError?.({ itemId: 'a', kind: 'panic', message: 'old board' });
    expect(a.onChange).not.toHaveBeenCalled();
    expect(b.onChange).not.toHaveBeenCalled();
    expect(a.onSave).not.toHaveBeenCalled();
    expect(b.onSave).not.toHaveBeenCalled();
    expect(b.onLoadError).not.toHaveBeenCalled();
    expect(useHostStore.getState().failure).toBeNull();

    // The current board's events arrive.
    frameEvents.onChange?.({ itemId: 'b', revision: 3 });
    frameEvents.onSave?.('b');
    frameEvents.onError?.({ itemId: 'b', kind: 'load', message: 'bad board' });
    expect(b.onChange).toHaveBeenCalledWith(3);
    expect(b.onSave).toHaveBeenCalledTimes(1);
    expect(b.onLoadError).toHaveBeenCalledWith('bad board');
    // An error without an item (the engine itself) always counts.
    frameEvents.onError?.({ itemId: null, kind: 'webgl', message: 'no WebGL2' });
    expect(useHostStore.getState().failure).toEqual({ kind: 'webgl', message: 'no WebGL2' });
  });

  it('the fake frame reports the item it shows', async () => {
    const fake = createFakeFrame();
    useHostStore.setState({ frame: fake.api });
    const a = session('a');
    await openBoard(a, { itemId: 'a', board: '{}', name: 'A', readOnly: false, theme });
    await fake.driver.connect(document.createElement('iframe'), frameEvents);
    fake.edit();
    expect(a.onChange).toHaveBeenCalledTimes(1);
    // A session that was replaced on the same item id gets nothing.
    const again = session('a');
    await openBoard(again, { itemId: 'a', board: '{}', name: 'A', readOnly: false, theme });
    fake.edit();
    expect(a.onChange).toHaveBeenCalledTimes(1);
    expect(again.onChange).toHaveBeenCalledTimes(1);
  });
});
