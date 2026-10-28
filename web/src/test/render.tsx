// Renders a component the way the app does: inside a fresh QueryClient and a memory data router.
//   const { queryClient, router } = render(<ItemHeader item={detail} … />, { route: '/doc/x-AbCdEfGh12' });
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render as rtlRender, type RenderOptions, type RenderResult } from '@testing-library/react';
import type { ReactElement } from 'react';
import { createMemoryRouter, type RouteObject } from 'react-router';
import { RouterProvider } from 'react-router/dom';

/** A QueryClient for tests: no retries, no garbage-collection timers left running. */
export function createTestQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: Infinity, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
}

export interface RenderWithAppOptions extends Omit<RenderOptions, 'wrapper' | 'queries'> {
  /** The initial URL (default `/`). */
  route?: string;
  /** The route pattern `ui` renders at (default `*`), e.g. `/doc/:slugId` so that `useParams` works. */
  path?: string;
  /** More routes next to `ui`'s, e.g. the targets of links the test follows. */
  routes?: RouteObject[];
  queryClient?: QueryClient;
}

export type RenderWithAppResult = RenderResult & {
  queryClient: QueryClient;
  router: ReturnType<typeof createMemoryRouter>;
};

export function render(ui: ReactElement, opts: RenderWithAppOptions = {}): RenderWithAppResult {
  const { route = '/', path = '*', routes = [], queryClient = createTestQueryClient(), ...rest } = opts;
  const router = createMemoryRouter([{ path, element: ui }, ...routes], { initialEntries: [route] });
  const result = rtlRender(
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
    rest,
  );
  return { ...result, queryClient, router };
}

/** Renders a whole route table (e.g. `routes` from app/router.tsx) at `route`. */
export function renderRoutes(routes: RouteObject[], route: string, queryClient = createTestQueryClient()) {
  const router = createMemoryRouter(routes, { initialEntries: [route] });
  const result = rtlRender(
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  return { ...result, queryClient, router };
}

/** A `fetch` stand-in answering JSON: `vi.stubGlobal('fetch', jsonFetch({'GET /api/v1/auth/me': [200, me]}))`. */
export function jsonFetch(responses: Record<string, [status: number, body?: unknown]>) {
  return async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const method = (init?.method ?? 'GET').toUpperCase();
    const pathname = new URL(url, 'http://localhost').pathname;
    const hit = responses[`${method} ${pathname}`];
    if (!hit) {
      return new Response(JSON.stringify({ error: { code: 'not_found', message: `no mock for ${method} ${pathname}` },
        request_id: 'test' }), { status: 404, headers: { 'Content-Type': 'application/json' } });
    }
    const [status, body] = hit;
    if (body === undefined || status === 204 || status === 304) return new Response(null, { status });
    return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
  };
}
