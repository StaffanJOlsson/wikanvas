// Vitest setup for every web test (vite.config.ts `test.setupFiles`): happy-dom is the environment; this adds an
// in-memory IndexedDB (the doc outbox and the board's unsaved snapshots, §9.4, §9.5) and resets the DOM, storage and
// the global UI stores after each test.
import 'fake-indexeddb/auto';
import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';
import { resetConfigBaseline } from '../api/queries/config';
import { resetUpdate } from '../app/updateCheck';
import { resetDialogs } from '../state/dialogs';
import { reloadExpanded } from '../state/expanded';
import { resetFullWidth } from '../state/fullWidth';
import { clearToasts } from '../state/toasts';
import { reloadUi } from '../state/ui';

afterEach(() => {
  cleanup();
  try {
    localStorage.clear();
    sessionStorage.clear();
  } catch {
    // No storage in this environment.
  }
  resetDialogs();
  clearToasts();
  reloadExpanded();
  resetFullWidth();
  reloadUi();
  resetUpdate();
  resetConfigBaseline();
});
