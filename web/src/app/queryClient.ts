// The app's TanStack Query client (docs/ARCHITECTURE.md §9.2).
import { QueryClient } from '@tanstack/react-query';
import { isApiError } from '../api/client';

/** Client errors (4xx) are answers, not glitches: retry only network failures and 5xx, at most twice. */
export function shouldRetry(failureCount: number, error: unknown): boolean {
  if (failureCount >= 2) return false;
  if (isApiError(error)) return error.status === 0 || error.status >= 500;
  return false;
}

export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: shouldRetry, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
}
