// Settings → About (docs/ARCHITECTURE.md §9.6.6): the generated licence list and the engine's own lists.
import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { makeConfig, mockApi } from '../../test/mockApi';
import { render } from '../../test/render';
import { About, LICENSES_URL } from './About';

const ENGINE = {
  base: '/inkboard/abc123def456/',
  engine_version: '0.2.0-beta.3+embed.wikanvas.2',
  format_version: 2,
  wasm_bytes: { raw: 10, br: 5, gz: 6 },
};

/** The API mock, plus `/licenses.txt` answered with `licenses` (a status for an error). */
function setup(licenses: string | number, inkboard: typeof ENGINE | null = ENGINE) {
  const api = mockApi().on('GET', '/config', [200, makeConfig({ inkboard })]);
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    if (new URL(url, 'http://localhost').pathname === LICENSES_URL) {
      return typeof licenses === 'number'
        ? new Response('not found\n', { status: licenses })
        : new Response(licenses, { status: 200, headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
    }
    return api.fetch(input, init);
  });
  vi.stubGlobal('fetch', fetchMock);
  render(<About />, { route: '/settings/about' });
  const licenceFetches = () =>
    fetchMock.mock.calls.filter(([input]) => String(input).endsWith(LICENSES_URL)).length;
  return { licenceFetches };
}

describe('About', () => {
  it('loads the licence list only when asked, and shows it as text', async () => {
    const { licenceFetches } = setup('Third-party software in Wikanvas\ncomrak 0.59.0  BSD-2-Clause\n');
    expect(await screen.findByText('0.2.0-beta.3+embed.wikanvas.2')).toBeTruthy();
    expect(licenceFetches()).toBe(0);
    fireEvent.click(screen.getByRole('button', { name: 'Show the licence list' }));
    const list = await screen.findByLabelText('Third-party licences');
    expect(list.textContent).toContain('comrak 0.59.0  BSD-2-Clause');
    expect(licenceFetches()).toBe(1);
    expect(screen.getByRole('button', { name: 'Hide the licence list' }).getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByRole('link', { name: 'Open licenses.txt' }).getAttribute('href')).toBe(LICENSES_URL);
  });

  it('links the engine lists under the engine base', async () => {
    setup('x');
    const thirdParty = await screen.findByRole('link', { name: 'THIRD_PARTY_LICENSES.md' });
    expect(thirdParty.getAttribute('href')).toBe('/inkboard/abc123def456/THIRD_PARTY_LICENSES.md');
    expect(screen.getByRole('link', { name: 'ICONS-LICENSES.txt' }).getAttribute('href')).toBe(
      '/inkboard/abc123def456/ICONS-LICENSES.txt',
    );
  });

  it('says so when the build has no list, and names no engine files without an engine', async () => {
    setup(404, null);
    expect(await screen.findByText('Not installed')).toBeTruthy();
    expect(screen.queryByRole('link', { name: 'THIRD_PARTY_LICENSES.md' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Show the licence list' }));
    expect(await screen.findByText(/This build has no licence list/)).toBeTruthy();
  });

  it('reports a failed load', async () => {
    setup(500);
    fireEvent.click(await screen.findByRole('button', { name: 'Show the licence list' }));
    expect((await screen.findByRole('alert')).textContent).toContain('HTTP 500');
  });
});
