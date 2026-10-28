// Title autosave (docs/ARCHITECTURE.md §9.2): 500 ms after the last keystroke, `PATCH /api/v1/items/{id}` with the
// title (no If-Match: the last write wins). Registered in `pending`, so route changes, Ctrl/Cmd+S and beforeunload
// flush or warn. Used by ItemHeader's title input and TopBar's editable title crumb.
import { useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useRef, useState } from 'react';
import { api } from '../../api/client';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import type { ItemSummary } from '../../api/gen/ItemSummary';
import { replaceCachedSummary } from '../../api/queries/tree';
import { registerPending } from '../../state/pending';
import { getSaveStatus, setSaveStatus } from '../../state/saveStatus';

export const TITLE_DEBOUNCE_MS = 500;

export interface TitleAutosave {
  /** The title as typed (the server's until the user types). */
  title: string;
  setTitle(title: string): void;
  /** Sends an unsaved title now (no request when there is none). */
  flush(): Promise<void>;
}

interface State {
  id: string;
  value: string;
  /** `value` is not saved yet (typed, or its last PATCH failed). */
  dirty: boolean;
  timer: ReturnType<typeof setTimeout> | null;
  inFlight: Promise<void> | null;
  failed: boolean;
}

export function useTitleAutosave(item: ItemDetail): TitleAutosave {
  const qc = useQueryClient();
  const id = item.item.id;
  const serverTitle = item.item.title;
  const [title, setLocal] = useState(serverTitle);
  const ref = useRef<State>({ id, value: serverTitle, dirty: false, timer: null, inFlight: null, failed: false });

  // Another item, or a newer server title while nothing is unsaved: adopt it.
  useEffect(() => {
    const s = ref.current;
    if (s.id !== id || (!s.dirty && s.inFlight === null)) {
      s.id = id;
      s.value = serverTitle;
      s.dirty = false;
      setLocal(serverTitle);
    }
  }, [id, serverTitle]);

  const send = useCallback(async (): Promise<void> => {
    const s = ref.current;
    if (s.timer !== null) {
      clearTimeout(s.timer);
      s.timer = null;
    }
    while (s.inFlight) await s.inFlight;
    if (!s.dirty) return;
    s.dirty = false;
    const itemId = s.id;
    const value = s.value;
    const run = (async () => {
      try {
        const saved = await api<ItemSummary>('PATCH', `/items/${encodeURIComponent(itemId)}`, { json: { title: value } });
        // The sidebar, the stars and every cached detail show the new title at once.
        replaceCachedSummary(qc, saved);
        void qc.invalidateQueries({ queryKey: ['tree', saved.collection_id] });
        if (s.failed && getSaveStatus(itemId).state === 'error') setSaveStatus(itemId, 'saved');
        s.failed = false;
      } catch (e) {
        if (s.id === itemId && !s.dirty) s.dirty = true;
        s.failed = true;
        setSaveStatus(itemId, {
          state: 'error',
          message: e instanceof Error ? e.message : 'The title was not saved',
          retry: () => void send(),
        });
      }
    })();
    s.inFlight = run;
    try {
      await run;
    } finally {
      if (s.inFlight === run) s.inFlight = null;
    }
  }, [qc]);

  const setTitle = useCallback((next: string) => {
    const s = ref.current;
    s.value = next;
    s.dirty = true;
    setLocal(next);
    if (s.timer !== null) clearTimeout(s.timer);
    s.timer = setTimeout(() => void send(), TITLE_DEBOUNCE_MS);
  }, [send]);

  // Registered per item; leaving the item (or unmounting) sends what is still unsaved, and the entry stays registered
  // until that PATCH ends, so a page that flushes the item after it (the discard of a new empty item) waits for it.
  useEffect(() => {
    const s = ref.current;
    const unregister = registerPending(id, {
      hasPending: () => s.dirty || s.inFlight !== null,
      flush: send,
    });
    return () => {
      if (s.dirty || s.inFlight !== null) void send().finally(unregister);
      else unregister();
    };
  }, [id, send]);

  return { title, setTitle, flush: send };
}
