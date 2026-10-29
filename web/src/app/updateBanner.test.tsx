import { screen, waitFor } from '@testing-library/react';
import { Outlet } from 'react-router';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useConfig } from '../api/queries/config';
import { makeConfig, mockApi } from '../test/mockApi';
import { renderRoutes } from '../test/render';
import { lazyPage } from './router';
import { checkForUpdate, isChunkLoadError, isNewBuild } from './updateCheck';
import { UpdateBanner } from './UpdateBanner';

function Shell() {
  // As in AppShell: the first config is the baseline.
  const config = useConfig();
  return (
    <>
      <UpdateBanner />
      {config.data && <Outlet />}
    </>
  );
}

const failing = () => Promise.reject(new TypeError('Failed to fetch dynamically imported module: /assets/DocPage-abc.js'));
const routes = [{ path: '/', Component: Shell, children: [lazyPage('doc/:slugId', failing), { path: 'home', element: <h1>Home</h1> }] }];

describe('the update banner', () => {
  beforeEach(() => {
    // React and React Router log the caught chunk error.
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    vi.spyOn(console, 'warn').mockImplementation(() => undefined);
  });

  it('a failed chunk load plus a changed /api/v1/config version shows the banner', async () => {
    let version = '0.1.0';
    const api = mockApi().on('GET', '/config', () => [200, makeConfig({ version })]);
    const { router } = renderRoutes(routes, '/home');
    await screen.findByRole('heading', { name: 'Home' });
    version = '0.2.0';
    await router.navigate('/doc/plan-AbCdEfGh12');
    const banner = await screen.findByRole('alert');
    expect(banner.textContent).toContain('Wikanvas was updated.');
    expect(screen.getAllByRole('button', { name: 'Reload' }).length).toBeGreaterThan(0);
    expect(screen.getByRole('heading', { name: 'Wikanvas was updated' })).toBeTruthy();
    expect(screen.queryByText(/could not be shown/)).toBeNull();
    expect(api.calls('GET', '/config')).toHaveLength(2);
  });

  it('a failed chunk with the same server build shows the error, not the banner', async () => {
    mockApi().on('GET', '/config', [200, makeConfig()]);
    const { router } = renderRoutes(routes, '/home');
    await screen.findByRole('heading', { name: 'Home' });
    await router.navigate('/doc/plan-AbCdEfGh12');
    expect(await screen.findByRole('heading', { name: 'This page could not be shown' })).toBeTruthy();
    expect(screen.queryByText('Wikanvas was updated.')).toBeNull();
  });

  it('compares version, commit and the engine base', async () => {
    const a = makeConfig();
    expect(isNewBuild(a, makeConfig())).toBe(false);
    expect(isNewBuild(a, makeConfig({ version: '0.1.1' }))).toBe(true);
    expect(isNewBuild(a, makeConfig({ commit: 'def456' }))).toBe(true);
    const engine = { base: '/inkboard/aaa/', engine_version: 'x', format_version: 2, wasm_bytes: { raw: 1, br: 1, gz: 1 } };
    expect(isNewBuild({ ...a, inkboard: engine }, { ...a, inkboard: { ...engine, base: '/inkboard/bbb/' } })).toBe(true);
    expect(isChunkLoadError(new TypeError('Importing a module script failed.'))).toBe(true);
    expect(isChunkLoadError(new Error('boom'))).toBe(false);
    // Without a baseline (config never loaded) nothing is claimed.
    mockApi().on('GET', '/config', [200, makeConfig({ version: '9' })]);
    await expect(checkForUpdate()).resolves.toBe(false);
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
  });
});
