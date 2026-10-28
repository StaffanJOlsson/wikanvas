import { describe, expect, it, vi } from 'vitest';
import { flushPending, hasPending, installBeforeUnload, registerPending } from './pending';

function entry(pending: boolean) {
  const e = {
    pending,
    hasPending: () => e.pending,
    flush: vi.fn(async () => {
      e.pending = false;
    }),
  };
  return e;
}

describe('pending', () => {
  it('tracks entries per item and unregisters them', () => {
    const doc = entry(true);
    const title = entry(false);
    const offA = registerPending('a', doc);
    const offB = registerPending('a', title);
    expect(hasPending('a')).toBe(true);
    expect(hasPending('b')).toBe(false);
    expect(hasPending()).toBe(true);
    offA();
    offA(); // idempotent
    expect(hasPending('a')).toBe(false);
    title.pending = true;
    expect(hasPending()).toBe(true);
    offB();
    expect(hasPending()).toBe(false);
  });

  it('flushes only entries with pending work, of one item or of all', async () => {
    const a = entry(true);
    const idle = entry(false);
    const b = entry(true);
    const offs = [registerPending('a', a), registerPending('a', idle), registerPending('b', b)];
    await flushPending('a');
    expect(a.flush).toHaveBeenCalledOnce();
    expect(idle.flush).not.toHaveBeenCalled();
    expect(b.flush).not.toHaveBeenCalled();
    await flushPending();
    expect(b.flush).toHaveBeenCalledOnce();
    expect(hasPending()).toBe(false);
    offs.forEach((off) => off());
  });

  it('a failing flush does not stop the others', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const bad = { hasPending: () => true, flush: vi.fn(async () => Promise.reject(new Error('offline'))) };
    const good = entry(true);
    const offs = [registerPending('x', bad), registerPending('y', good)];
    await expect(flushPending()).resolves.toBeUndefined();
    expect(good.flush).toHaveBeenCalledOnce();
    expect(warn).toHaveBeenCalled();
    offs.forEach((off) => off());
  });

  it('beforeunload prompts only while something is pending', () => {
    const remove = installBeforeUnload();
    const fire = () => {
      const e = new Event('beforeunload', { cancelable: true });
      window.dispatchEvent(e);
      return e.defaultPrevented;
    };
    expect(fire()).toBe(false);
    const off = registerPending('a', entry(true));
    expect(fire()).toBe(true);
    off();
    remove();
  });
});
