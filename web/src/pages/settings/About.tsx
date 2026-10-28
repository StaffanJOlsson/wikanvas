// Settings → About (docs/ARCHITECTURE.md §9.6.6): versions and licences. The licence list is `/licenses.txt`, which
// the image build writes (docker/Dockerfile, stage `licenses`, §13.2); the whiteboard engine's own lists are served
// with the engine under `config.inkboard.base`.
import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { useConfig } from '../../api/queries/config';

/** Where the image build puts the generated licence list (a root file of `web/dist`, §11). */
export const LICENSES_URL = '/licenses.txt';

/** The licence list, or null when this build has none (a dev server or a `pnpm build` outside the image). */
export async function fetchLicenses(signal?: AbortSignal): Promise<string | null> {
  const res = await fetch(LICENSES_URL, { signal, credentials: 'same-origin', cache: 'no-cache' });
  if (res.status === 404) return null;
  if (!res.ok) throw new Error(`The licence list could not be loaded (HTTP ${res.status})`);
  return res.text();
}

function LicenceList() {
  const list = useQuery<string | null, Error>({
    queryKey: ['licenses'],
    queryFn: ({ signal }) => fetchLicenses(signal),
    staleTime: Infinity,
  });
  if (list.isPending) return <p>Loading the licence list…</p>;
  if (list.isError) return <p className="form-error" role="alert">{list.error.message}</p>;
  if (list.data === null) {
    return (
      <p>
        This build has no licence list: the image build generates it. THIRD_PARTY.md in the source lists the same
        packages.
      </p>
    );
  }
  return (
    <pre
      aria-label="Third-party licences"
      tabIndex={0}
      style={{ maxHeight: '60vh', overflow: 'auto', whiteSpace: 'pre', fontSize: 12, lineHeight: '16px' }}
    >
      {list.data}
    </pre>
  );
}

export function About() {
  const config = useConfig();
  const c = config.data;
  const [showList, setShowList] = useState(false);
  const base = c?.inkboard?.base;
  return (
    <>
      <section className="settings-section" aria-labelledby="about-h">
        <h2 id="about-h">Quire</h2>
        <dl className="facts">
          <div><dt>Version</dt><dd>{c?.version ?? '…'}</dd></div>
          <div><dt>Commit</dt><dd><code>{c?.commit ?? '…'}</code></dd></div>
          <div><dt>API</dt><dd>v{c?.api_version ?? '…'} · <a href={c?.openapi_url ?? '/api/v1/openapi.json'}>OpenAPI spec</a></dd></div>
          <div><dt>Whiteboard engine</dt><dd>{c ? (c.inkboard ? c.inkboard.engine_version : 'Not installed') : '…'}</dd></div>
        </dl>
      </section>
      <section className="settings-section" aria-labelledby="licence-h">
        <h2 id="licence-h">Licences</h2>
        <p>
          Quire is free software under the MIT licence. It is built from open-source
          libraries, each under its own licence: the list below names every Rust crate and npm package in this build,
          with the licence files they ship.
        </p>
        {base && (
          <p>
            The whiteboard engine, Inkboard (MIT or Apache-2.0), lists its own libraries and fonts in{' '}
            <a href={`${base}THIRD_PARTY_LICENSES.md`} target="_blank" rel="noopener">THIRD_PARTY_LICENSES.md</a> and its
            icons in <a href={`${base}ICONS-LICENSES.txt`} target="_blank" rel="noopener">ICONS-LICENSES.txt</a>.
          </p>
        )}
        <p>
          <button type="button" className="button" aria-expanded={showList} onClick={() => setShowList((v) => !v)}>
            {showList ? 'Hide the licence list' : 'Show the licence list'}
          </button>{' '}
          <a href={LICENSES_URL} target="_blank" rel="noopener">Open licenses.txt</a>
        </p>
        {showList && <LicenceList />}
      </section>
    </>
  );
}
