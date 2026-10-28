import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { resetWarmup, WARM_FILES, warmEngine } from './warmup';

const BASE = '/inkboard/abc123def456/';

beforeEach(() => {
  vi.useFakeTimers();
  resetWarmup();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('engine warm-up', () => {
  it('fetches the engine with {cache: "force-cache"} once per build; no <link rel=prefetch>', async () => {
    const fetch = vi.fn(async () => new Response(new ArrayBuffer(4)));
    vi.stubGlobal('fetch', fetch);
    warmEngine(BASE);
    expect(fetch).toHaveBeenCalledTimes(2);
    const urls = fetch.mock.calls.map((c) => (c as unknown[])[0]);
    expect(urls).toEqual(WARM_FILES.map((f) => `${BASE}${f}`));
    for (const c of fetch.mock.calls) {
      expect((c as unknown[])[1]).toMatchObject({ cache: 'force-cache', priority: 'low' });
    }
    expect(document.querySelector('link[rel="prefetch"], link[rel="preload"]')).toBeNull();
    await vi.advanceTimersByTimeAsync(0);
    warmEngine(BASE);
    expect(fetch).toHaveBeenCalledTimes(2);
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
