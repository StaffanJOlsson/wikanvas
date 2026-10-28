// Settings → Data (docs/ARCHITECTURE.md §9.6.6, D34, §12): the export download, an on-demand backup, and where the
// scheduled backups live.
import { useState } from 'react';
import { api, apiUrl } from '../../api/client';
import type { BackupResult } from '../../api/gen/BackupResult';
import { errorMessage } from '../../state/toasts';

function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export function DataSettings() {
  const [backup, setBackup] = useState<BackupResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      setBackup(await api<BackupResult>('POST', '/admin/backup'));
    } catch (e) {
      setError(errorMessage(e, 'The backup failed'));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <section className="settings-section" aria-labelledby="export-h">
        <h2 id="export-h">Export</h2>
        <p>
          Download everything as one tar file: the database (without sessions, API tokens or password hashes) and every
          uploaded file.
        </p>
        <a className="button" href={apiUrl('/export')} download>Export the workspace</a>
      </section>
      <section className="settings-section" aria-labelledby="backup-h">
        <h2 id="backup-h">Backups</h2>
        <p>
          The server backs up its database to <code>/data/backups</code> every <code>QUIRE_BACKUP_INTERVAL</code> (default
          24 hours) and keeps the newest <code>QUIRE_BACKUP_KEEP</code> copies (default 7). Uploaded files stay in
          <code> /data/files</code>; back up the whole <code>/data</code> volume to keep both.
        </p>
        <button type="button" className="button" disabled={busy} onClick={() => void run()}>
          {busy ? 'Backing up…' : 'Back up now'}
        </button>
        {backup && <p className="notice" role="status">Saved {backup.path} ({size(backup.size)}).</p>}
        {error && <p className="form-error" role="alert">{error}</p>}
      </section>
    </>
  );
}
