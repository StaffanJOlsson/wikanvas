import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HOVER_WARM_MS, hoverWarmup, resetWarmup, warmEngine } from './warmup';

const BASE = '/inkboard/abc123def456/';

beforeEach(() => {
  vi.useFakeTimers();
  resetWarmup();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('engine warm-up', () => {
  it('hover warms with fetch(…, {cache: "force-cache"}) after 300 ms; no <link rel=prefetch>', async () => {
    const fetch = vi.fn(async () => new Response(new ArrayBuffer(4)));
    vi.stubGlobal('fetch', fetch);
    const row = hoverWarmup(() => BASE);
    row.onPointerEnter();
    await vi.advanceTimersByTimeAsync(HOVER_WARM_MS - 1);
    expect(fetch).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(fetch).toHaveBeenCalledTimes(2);
    const urls = fetch.mock.calls.map((c) => (c as unknown[])[0]);
    expect(urls).toEqual([`${BASE}inkboard_bg.wasm`, `${BASE}inkboard.js`]);
    for (const c of fetch.mock.calls) {
      expect((c as unknown[])[1]).toMatchObject({ cache: 'force-cache', priority: 'low' });
    }
    expect(document.querySelector('link[rel="prefetch"], link[rel="preload"]')).toBeNull();
    // Once per build.
    row.onPointerLeave();
    row.onPointerEnter();
    await vi.advanceTimersByTimeAsync(HOVER_WARM_MS);
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it('leaving before 300 ms warms nothing, and no engine means no fetch', async () => {
    const fetch = vi.fn(async () => new Response(''));
    vi.stubGlobal('fetch', fetch);
    const row = hoverWarmup(() => BASE);
    row.onPointerEnter();
    await vi.advanceTimersByTimeAsync(HOVER_WARM_MS - 50);
    row.onPointerLeave();
    await vi.advanceTimersByTimeAsync(HOVER_WARM_MS);
    expect(fetch).not.toHaveBeenCalled();
    const none = hoverWarmup(() => null);
    none.onPointerEnter();
    await vi.advanceTimersByTimeAsync(HOVER_WARM_MS);
    expect(fetch).not.toHaveBeenCalled();
  });

  it('a failed warm-up may be retried', async () => {
    const fetch = vi.fn(async () => {
      throw new TypeError('offline');
    });
    vi.stubGlobal('fetch', fetch);
    warmEngine(BASE);
    await vi.advanceTimersByTimeAsync(0);
    warmEngine(BASE);
    await vi.advanceTimersByTimeAsync(0);
    expect(fetch).toHaveBeenCalledTimes(4);
  });
});
