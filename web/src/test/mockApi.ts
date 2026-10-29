// An in-memory stand-in for the HTTP API in web tests (PLAN W3 "Consumes": the W1 and W2 routes are mocked here).
//   const api = mockApi()                       // stubs global fetch
//     .on('GET', '/sidebar', [200, sidebar])
//     .on('POST', '/items/:id/move', (req) => [200, { ...item, parent_id: (req.body as MoveItem).parent_id }]);
//   …
//   expect(api.calls('POST', '/items/:id/move')[0]!.body).toEqual({ … });
// Paths are relative to /api/v1. Later registrations win. Unmatched requests answer the 404 error envelope.
import { vi } from 'vitest';
import type { ChildOrder } from '../api/gen/ChildOrder';
import type { Collection } from '../api/gen/Collection';
import type { CollectionMember } from '../api/gen/CollectionMember';
import type { Config } from '../api/gen/Config';
import type { DirectoryUser } from '../api/gen/DirectoryUser';
import type { ItemDetail } from '../api/gen/ItemDetail';
import type { ItemSummary } from '../api/gen/ItemSummary';
import type { Me } from '../api/gen/Me';
import type { Sidebar } from '../api/gen/Sidebar';

export interface MockRequest {
  method: string;
  /** The path after /api/v1, without the query. */
  path: string;
  params: Record<string, string>;
  query: URLSearchParams;
  /** Parsed JSON, the text of other bodies, or undefined. */
  body: unknown;
  headers: Headers;
}

export type MockReply = [status: number, body?: unknown, headers?: Record<string, string>];
export type MockHandler = (req: MockRequest) => MockReply | Promise<MockReply>;

interface Route {
  method: string;
  pattern: string;
  re: RegExp;
  keys: string[];
  handler: MockHandler;
}

function compile(pattern: string): { re: RegExp; keys: string[] } {
  const keys: string[] = [];
  const src = pattern
    .split('/')
    .map((seg) => {
      if (seg.startsWith(':')) {
        keys.push(seg.slice(1));
        return '([^/]+)';
      }
      return seg.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    })
    .join('/');
  return { re: new RegExp(`^${src}$`), keys };
}

export function errorReply(status: number, code: string, message = code, details?: unknown): MockReply {
  return [status, { error: { code, message, ...(details === undefined ? {} : { details }) }, request_id: 'req-test' }];
}

export class MockApi {
  private routes: Route[] = [];
  private log: (MockRequest & { pattern: string | null })[] = [];

  /** Registers a reply (or a handler) for `METHOD /path/:param`. */
  on(method: string, pattern: string, reply: MockReply | MockHandler): this {
    const { re, keys } = compile(pattern);
    const handler: MockHandler = typeof reply === 'function' ? reply : () => reply;
    this.routes.push({ method: method.toUpperCase(), pattern, re, keys, handler });
    return this;
  }

  /** Every request so far, or those that matched `METHOD pattern`. */
  calls(method?: string, pattern?: string): MockRequest[] {
    return this.log.filter((c) => (!method || c.method === method.toUpperCase()) && (!pattern || c.pattern === pattern));
  }

  readonly fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(raw, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    const path = url.pathname.startsWith('/api/v1') ? url.pathname.slice('/api/v1'.length) : url.pathname;
    let body: unknown;
    if (typeof init?.body === 'string') {
      try {
        body = JSON.parse(init.body);
      } catch {
        body = init.body;
      }
    } else if (init?.body != null) {
      body = init.body;
    }
    const headers = new Headers(init?.headers);
    for (let i = this.routes.length - 1; i >= 0; i--) {
      const r = this.routes[i]!;
      if (r.method !== method) continue;
      const m = r.re.exec(path);
      if (!m) continue;
      const params: Record<string, string> = {};
      r.keys.forEach((k, j) => {
        params[k] = decodeURIComponent(m[j + 1]!);
      });
      const req: MockRequest = { method, path, params, query: url.searchParams, body, headers };
      this.log.push({ ...req, pattern: r.pattern });
      const [status, resBody, resHeaders] = await r.handler(req);
      const h = new Headers({ 'X-Request-Id': 'req-test', ...resHeaders });
      if (resBody === undefined || status === 204 || status === 304) return new Response(null, { status, headers: h });
      if (!h.has('Content-Type')) h.set('Content-Type', 'application/json');
      return new Response(JSON.stringify(resBody), { status, headers: h });
    }
    this.log.push({ method, path, params: {}, query: url.searchParams, body, headers, pattern: null });
    const [status, resBody] = errorReply(404, 'not_found', `no mock for ${method} ${path}`);
    return new Response(JSON.stringify(resBody), { status, headers: { 'Content-Type': 'application/json' } });
  };
}

/** A MockApi installed as the global `fetch` (restored after each test by `unstubGlobals`). */
export function mockApi(): MockApi {
  const api = new MockApi();
  vi.stubGlobal('fetch', vi.fn(api.fetch));
  return api;
}

// ---- fixtures ----

const T0 = '2026-10-28T09:00:00.000Z';
let seq = 0;

/** A 10-character url_id derived from a readable id. */
export function urlIdOf(id: string): string {
  return `${id.replace(/[^A-Za-z0-9]/g, '')}0000000000`.slice(0, 10);
}

export function makeCollection(over: Partial<Collection> & { id: string }): Collection {
  return {
    url_id: urlIdOf(over.id),
    name: over.id,
    description: '',
    icon: null,
    color: '#3E63DD',
    sort_key: `a${seq++}`,
    child_order: 'manual' as ChildOrder,
    item_count: 0,
    updated_at: T0,
    default_access: 'write',
    // The fixture user (`makeMe`) is the owner, who manages every collection (D152).
    my_access: 'manage',
    ...over,
  };
}

export function makeItem(over: Partial<ItemSummary> & { id: string; collection_id: string }): ItemSummary {
  return {
    url_id: urlIdOf(over.id),
    parent_id: null,
    kind: 'doc',
    title: over.id,
    sort_key: 'a0',
    child_count: 0,
    published: true,
    archived: false,
    updated_at: T0,
    content_at: null,
    created_at: T0,
    ...over,
  };
}

export function makeMe(over: Partial<Me> = {}): Me {
  return {
    user: { id: 'u1', email: 'owner@example.com', name: 'Owner', role: 'owner', theme: 'system' },
    workspace: { name: 'Acme Wiki', logo_url: null },
    must_change_password: false,
    auth: { method: 'session', token: null },
    ...over,
  };
}

export function makeDirectoryUser(over: Partial<DirectoryUser> & { id: string }): DirectoryUser {
  return { name: over.id, role: 'member', ...over };
}

/** A membership; `effective_access` is the membership's own unless given. */
export function makeMember(user: DirectoryUser, over: Partial<CollectionMember> = {}): CollectionMember {
  const access = over.access ?? 'write';
  return { user, access, effective_access: access, disabled: false, updated_at: T0, ...over };
}

export function makeSidebar(collections: Collection[], stars: ItemSummary[] = [], drafts_count = 0): Sidebar {
  return { collections, stars, drafts_count };
}

export function makeDetail(item: ItemSummary, collection: Collection, over: Partial<ItemDetail> = {}): ItemDetail {
  return {
    item,
    collection,
    breadcrumbs: [],
    starred: false,
    effectively_published: true,
    trashed_at: null,
    purge_at: null,
    archived_at: null,
    full_width: false,
    board: null,
    meta_version: 1,
    ...over,
  };
}

export function makeConfig(over: Partial<Config> = {}): Config {
  return {
    version: '0.1.0',
    commit: 'abc123',
    api_version: '1.0',
    openapi_url: '/api/v1/openapi.json',
    setup_required: false,
    multi_user: false,
    inkboard: null,
    limits: {
      max_upload_mb: 25,
      max_board_mb: 50,
      max_doc_mb: 16,
      max_update_bytes: 16777216,
      max_markdown_bytes: 1048576,
      max_json_bytes: 4194304,
      max_body_bytes: 1048576,
      json_depth: 100,
      q_max_chars: 500,
      page_max: 100,
      token_rate_per_min: 600,
      token_burst: 120,
      token_bytes_per_hour: 1073741824,
    },
    ...over,
  };
}

/** The tree response of a collection, with an ETag. */
export function treeReply(collection: Collection, items: ItemSummary[], etag = '"t1.0"'): MockReply {
  return [200, { collection, items }, { ETag: etag }];
}
