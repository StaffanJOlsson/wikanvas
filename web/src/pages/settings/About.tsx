// Settings → About (docs/ARCHITECTURE.md §9.6.6): versions and licences.
import { useConfig } from '../../api/queries/config';

export function About() {
  const config = useConfig();
  const c = config.data;
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
          Quire is free software under the MIT licence. It includes Inkboard (MIT or
          Apache-2.0) for whiteboards, and open-source libraries listed with their licences in THIRD_PARTY.md, which ships
          with every release.
        </p>
      </section>
    </>
  );
}
