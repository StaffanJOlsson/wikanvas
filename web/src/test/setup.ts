// Vitest setup for every web test (vite.config.ts `test.setupFiles`): happy-dom is the environment; this adds an
// in-memory IndexedDB (the doc outbox and the board's unsaved snapshots, §9.4, §9.5) and resets the DOM and storage
// after each test.
import 'fake-indexeddb/auto';
import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

afterEach(() => {
  cleanup();
  try {
    localStorage.clear();
    sessionStorage.clear();
  } catch {
    // No storage in this environment.
  }
});
