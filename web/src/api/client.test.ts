import { describe, expect, it, vi } from 'vitest';
import { api, API_BASE, ApiError, apiResponse, AUTH_LOST_EVENT } from './client';

function stubFetch(res: () => Response) {
  const fetch = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => res());
  vi.stubGlobal('fetch', fetch);
  return fetch;
}

const json = (status: number, body: unknown, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json', ...headers } });

describe('api client', () => {
  it('prefixes /api/v1, sends the cookie and never Authorization', async () => {
    const fetch = stubFetch(() => json(200, { ok: true }));
    await expect(
      api('POST', '/items', { json: { kind: 'doc' }, headers: { Authorization: 'Bearer qpat_x' } }),
    ).resolves.toEqual({ ok: true });
    const [url, init] = fetch.mock.calls[0]!;
    expect(API_BASE).toBe('/api/v1');
    expect(url).toBe('/api/v1/items');
    expect(init?.credentials).toBe('same-origin');
    const headers = new Headers(init?.headers);
    expect(headers.has('Authorization')).toBe(false);
    expect(headers.get('Content-Type')).toBe('application/json');
    expect(init?.body).toBe('{"kind":"doc"}');
  });

  it('uploads raw bodies as application/octet-stream', async () => {
    const fetch = stubFetch(() => json(201, {}));
    await api('POST', '/attachments?filename=a.txt', { body: new Blob(['hi'], { type: 'text/plain' }) });
    expect(new Headers(fetch.mock.calls[0]![1]?.headers).get('Content-Type')).toBe('application/octet-stream');
  });

  it('throws ApiError with the envelope code and request id', async () => {
    stubFetch(() => json(409, { error: { code: 'cycle', message: 'Would create a cycle' }, request_id: 'req-1' }));
    const err = await api('POST', '/items/x/move', { json: {} }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ status: 409, code: 'cycle', requestId: 'req-1', message: 'Would create a cycle' });
  });

  it('reports network failures as status 0, code network', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => Promise.reject(new TypeError('Failed to fetch'))));
    await expect(api('GET', '/config')).rejects.toMatchObject({ status: 0, code: 'network' });
  });

  it('sends If-None-Match and resolves a 304 with no data', async () => {
    const fetch = stubFetch(() => new Response(null, { status: 304, headers: { ETag: '"t3.1"' } }));
    const res = await apiResponse('GET', '/collections/c/tree', { etag: '"t3.1"' });
    expect(new Headers(fetch.mock.calls[0]![1]?.headers).get('If-None-Match')).toBe('"t3.1"');
    expect(res).toMatchObject({ status: 304, notModified: true, etag: '"t3.1"', data: undefined });
  });

  it('a 401 sends auth-lost, except for failed sign-ins and opted-out calls', async () => {
    const lost = vi.fn();
    window.addEventListener(AUTH_LOST_EVENT, lost);
    stubFetch(() => json(401, { error: { code: 'unauthenticated', message: 'Sign in' }, request_id: 'r' }));
    await expect(api('GET', '/sidebar')).rejects.toMatchObject({ status: 401 });
    expect(lost).toHaveBeenCalledOnce();
    await expect(api('GET', '/auth/me', { authLost: false })).rejects.toBeInstanceOf(ApiError);
    stubFetch(() => json(401, { error: { code: 'invalid_credentials', message: 'No' }, request_id: 'r' }));
    await expect(api('POST', '/auth/login', { json: {} })).rejects.toMatchObject({ code: 'invalid_credentials' });
    expect(lost).toHaveBeenCalledOnce();
    window.removeEventListener(AUTH_LOST_EVENT, lost);
  });

  it('maps a non-envelope error to bad_response', async () => {
    stubFetch(() => new Response('<html>502</html>', { status: 502, headers: { 'Content-Type': 'text/html' } }));
    await expect(api('GET', '/config')).rejects.toMatchObject({ status: 502, code: 'bad_response' });
  });
});
