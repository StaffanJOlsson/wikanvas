// `GET /api/v1/config` (public): version, engine build and limits (docs/ARCHITECTURE.md §7.2, §9.2).
import { useQuery, type UseQueryResult } from '@tanstack/react-query';
import { api, type ApiError } from '../client';
import type { Config } from '../gen/Config';

export const configKey = ['config'] as const;

/** The config this page loaded first: the build the running SPA belongs to (the update check compares with it). */
let baseline: Config | null = null;

export function configBaseline(): Config | null {
  return baseline;
}

/** Forgets the baseline (tests). */
export function resetConfigBaseline(): void {
  baseline = null;
}

export async function fetchConfig(signal?: AbortSignal): Promise<Config> {
  const config = await api<Config>('GET', '/config', { signal });
  baseline ??= config;
  return config;
}

/**
 * The server's config. Fetched once per page load: it changes only with a server update, which the update check
 * (§9.1, a failed chunk or a frame.html 404) detects by refetching `['config']`.
 */
export function useConfig(): UseQueryResult<Config, ApiError> {
  return useQuery<Config, ApiError>({
    queryKey: configKey,
    queryFn: ({ signal }) => fetchConfig(signal),
    staleTime: Infinity,
  });
}
