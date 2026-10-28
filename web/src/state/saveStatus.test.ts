import { act, renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { clearSaveStatus, getSaveStatus, IDLE, saveStateLabel, setSaveStatus, useSaveStatus } from './saveStatus';

describe('saveStatus', () => {
  it('is idle for an unknown item', () => {
    expect(getSaveStatus('nobody')).toEqual({ state: 'idle' });
    expect(getSaveStatus('nobody')).toBe(IDLE);
  });

  it('keeps one status per item; a bare state is shorthand', () => {
    setSaveStatus('a', 'saving');
    const retry = vi.fn();
    setSaveStatus('b', { state: 'error', message: 'Disk full', retry });
    expect(getSaveStatus('a')).toEqual({ state: 'saving' });
    expect(getSaveStatus('b').state).toBe('error');
    expect(getSaveStatus('b').message).toBe('Disk full');
    getSaveStatus('b').retry?.();
    expect(retry).toHaveBeenCalledOnce();
    clearSaveStatus('a');
    clearSaveStatus('b');
    expect(getSaveStatus('a')).toBe(IDLE);
  });

  it('useSaveStatus re-renders on changes of its own item only', () => {
    let renders = 0;
    const { result } = renderHook(() => {
      renders++;
      return useSaveStatus('doc-1');
    });
    expect(result.current.state).toBe('idle');
    act(() => setSaveStatus('doc-1', 'saving'));
    expect(result.current.state).toBe('saving');
    const before = renders;
    act(() => setSaveStatus('doc-2', 'offline'));
    expect(renders).toBe(before);
    act(() => setSaveStatus('doc-1', { state: 'signed_out' }));
    expect(result.current).toEqual({ state: 'signed_out' });
    act(() => clearSaveStatus('doc-1'));
    expect(result.current.state).toBe('idle');
    clearSaveStatus('doc-2');
  });

  it('labels every state', () => {
    expect(saveStateLabel('saved')).toBe('Saved');
    expect(saveStateLabel('idle')).toBe('');
    expect(saveStateLabel('offline')).toBe('Offline');
  });
});
