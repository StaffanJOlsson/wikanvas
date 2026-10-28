// Shared setup of the board tests (not part of the app): fixtures, a fetch mock that speaks the board routes (ETag
// headers, raw envelope text), and a render of the frame host next to the board page in a memory router, with the
// fake frame (fakeFrame.ts) as the host's driver.
import type { QueryClient } from '@tanstack/react-query';
import { QueryClientProvider } from '@tanstack/react-query';
import { render } from '@testing-library/react';
import { Outlet, createMemoryRouter, type RouteObject } from 'react-router';
import { RouterProvider } from 'react-router/dom';
import type { BoardMeta } from '../api/gen/BoardMeta';
import type { BoardPutResult } from '../api/gen/BoardPutResult';
import type { Config } from '../api/gen/Config';
import type { ItemDetail } from '../api/gen/ItemDetail';
import { itemPath } from '../lib/urls';
import BoardPage from '../pages/BoardPage';
import { createTestQueryClient } from '../test/render';
import BoardFrameHost from './BoardFrameHost';
import { createFakeFrame, type FakeFrame, type FakeFrameOptions } from './fakeFrame';
import { setFrameDriver } from './frameDriver';
import { resetHost } from './host';

export const BASE = '/inkboard/abc123def456/';

export function config(inkboard = true): Config {
  return {
    version: '0.1.0',
    commit: 'test',
    api_version: '1.0',
    openapi_url: '/api/v1/openapi.json',
    setup_required: false,
    multi_user: false,
    inkboard: inkboard
      ? { base: BASE, engine_version: '0.2.0-beta.3+embed.quire.1', format_version: 2, wasm_bytes: { raw: 10, br: 4, gz: 5 } }
      : null,
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
  };
}

export function meta(rev: number): BoardMeta {
  return {
    rev,
    pages: [{ id: 'p1', name: 'Page 1', svg_len: null }],
    objects: 1,
    background: '#ffffff',
    bytes: 100,
    preview_rev: null,
    text_excerpt: 'Page 1\nHello',
  };
}

/** A board item; `n` makes ids and url_ids unique (`board-<n>`, `BoArDiD00<n>`). */
export function boardItem(n = 1, over: Partial<ItemDetail['item']> = {}): ItemDetail {
  const t = '2026-10-28T09:00:00.000Z';
  return {
    item: {
      id: `board-${n}`,
      url_id: `BoArDiD00${n}`,
      collection_id: 'col-1',
      parent_id: null,
      kind: 'board',
      title: `Board ${n}`,
      sort_key: 'a0',
      child_count: 0,
      published: true,
      archived: false,
      updated_at: t,
      content_at: t,
      created_at: t,
      ...over,
    },
    collection: {
      id: 'col-1',
      url_id: 'CoLlEcT001',
      name: 'Boards',
      description: '',
      icon: null,
      color: '#8D8D8D',
      sort_key: 'a0',
      child_order: 'manual',
      item_count: 2,
      updated_at: t,
    },
    breadcrumbs: [],
    starred: false,
    effectively_published: true,
    trashed_at: null,
    purge_at: null,
    archived_at: null,
    full_width: false,
    board: meta(1),
    meta_version: 1,
  };
}

/** An envelope with `label` in it. */
export function envelope(label: string): string {
  return JSON.stringify({ format: 'inkboard', version: 2, document: { name: '', pages: [{ label }] } });
}

export function putResult(rev: number): BoardPutResult {
  return { rev, etag: `"r${rev}"`, updated_at: '2026-10-28T10:00:00.000Z', meta: meta(rev) };
}

export interface Recorded {
  method: string;
  path: string;
  headers: Headers;
  body: string | null;
}

export type Reply = Response | [status: number, body?: unknown, headers?: Record<string, string>];
export type Handler = (req: Recorded) => Reply | Promise<Reply>;

function toResponse(r: Reply): Response {
  if (r instanceof Response) return r;
  const [status, body, headers = {}] = r;
  if (body === undefined || status === 204 || status === 304) return new Response(null, { status, headers });
  const text = typeof body === 'string' ? body : JSON.stringify(body);
  return new Response(text, { status, headers: { 'Content-Type': 'application/json', ...headers } });
}

export function errorBody(code: string, details?: unknown) {
  return { error: { code, message: code, details }, request_id: 'test' };
}

/** A fetch mock: `routes['PUT /api/v1/items/x/board']` answers; every request is recorded in `requests`. */
export function mockFetch(routes: Record<string, Handler | Reply>) {
  const requests: Recorded[] = [];
  const fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const method = (init?.method ?? 'GET').toUpperCase();
    const path = new URL(url, 'http://localhost').pathname;
    const req: Recorded = {
      method,
      path,
      headers: new Headers(init?.headers),
      body: typeof init?.body === 'string' ? init.body : null,
    };
    requests.push(req);
    const hit = routes[`${method} ${path}`];
    if (!hit) return toResponse([404, errorBody('not_found')]);
    return toResponse(typeof hit === 'function' ? await hit(req) : hit);
  };
  return { fetch, requests };
}

/** `GET …/board` answering `board` at `rev`. */
export function boardReply(board: string, rev: number): Reply {
  return new Response(board, { status: 200, headers: { 'Content-Type': 'application/json', ETag: `"r${rev}"` } });
}

/** The standard routes of one board item: config, the item, its board. */
export function boardRoutes(item: ItemDetail, board = envelope('Hello'), rev = 1, inkboard = true) {
  return {
    'GET /api/v1/config': [200, config(inkboard)] as Reply,
    [`GET /api/v1/items/${item.item.url_id}`]: [200, item] as Reply,
    [`GET /api/v1/items/${item.item.id}`]: [200, item] as Reply,
    [`GET /api/v1/items/${item.item.id}/board`]: () => boardReply(board, rev),
  };
}

export function boardPath(item: ItemDetail): string {
  return itemPath(item.item);
}

export interface Rendered {
  fake: FakeFrame;
  router: ReturnType<typeof createMemoryRouter>;
  queryClient: QueryClient;
  unmount(): void;
}

/** Renders `<BoardFrameHost/>` beside the routes (as AppShell does): `/board/:slugId` → BoardPage, `/home`. */
export function renderBoardApp(path: string, opts: FakeFrameOptions & { fake?: FakeFrame } = {}): Rendered {
  resetHost();
  const fake = opts.fake ?? createFakeFrame(opts);
  setFrameDriver(fake.driver);
  const queryClient = createTestQueryClient();
  const routes: RouteObject[] = [
    {
      element: (
        <>
          <main id="main">
            <Outlet />
          </main>
          <BoardFrameHost />
        </>
      ),
      children: [
        { path: '/board/:slugId', element: <BoardPage /> },
        { path: '/home', element: <h1>Home</h1> },
      ],
    },
  ];
  const router = createMemoryRouter(routes, { initialEntries: [path] });
  const r = render(
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  return {
    fake,
    router,
    queryClient,
    unmount: () => {
      r.unmount();
      setFrameDriver(null);
      resetHost();
    },
  };
}
