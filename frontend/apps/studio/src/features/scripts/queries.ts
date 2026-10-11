import { useInfiniteQuery } from '@tanstack/react-query';
import { useEffect } from 'react';
import type { ScriptVersionSummary, ScriptsApi } from './api';
import { startPolling } from './polling';

/** Query keys of the scripts feature. All start with the namespace id, so a namespace's data never answers for another's. */
export const keys = {
  all: (namespaceId: string) => ['scripts', namespaceId] as const,
  list: (namespaceId: string) => ['scripts', namespaceId, 'list'] as const,
  script: (namespaceId: string, id: string) => ['scripts', namespaceId, 'script', id] as const,
  versions: (namespaceId: string, id: string) => ['scripts', namespaceId, 'versions', id] as const,
  version: (namespaceId: string, id: string, number: number) => ['scripts', namespaceId, 'version', id, number] as const,
  testRuns: (namespaceId: string, id: string) => ['scripts', namespaceId, 'test-runs', id] as const,
};

const PAGE = 20;
/** A build normally takes seconds; a runner that is not there makes it wait, so the wait is bounded. */
export const BUILD_WATCH_MS = 10 * 60_000;

/**
 * The versions of a script, newest first, `PAGE` at a time. While one is building it is looked at
 * again (with the backoff of `startPolling`) until none is, the time is up, or the page is left.
 */
export function useVersionList(api: ScriptsApi, scriptId: string, enabled: boolean) {
  const query = useInfiniteQuery({
    queryKey: keys.versions(api.namespaceId, scriptId),
    initialPageParam: undefined as number | undefined,
    queryFn: ({ pageParam, signal }) => api.versions(scriptId, { ...(pageParam === undefined ? {} : { before: pageParam }), limit: PAGE }, signal),
    getNextPageParam: (last) => (last.items.length >= PAGE ? last.items[last.items.length - 1]?.number : undefined),
    enabled,
  });
  const items: ScriptVersionSummary[] = query.data?.pages.flatMap((page) => page.items) ?? [];
  const building = items.some((version) => version.status === 'BUILDING');
  const { refetch } = query;
  useEffect(() => {
    if (!building) return undefined;
    const poller = startPolling({
      fetch: async () => {
        const result = await refetch();
        if (result.error) throw result.error;
        return result.data?.pages.flatMap((page) => page.items) ?? [];
      },
      isDone: (versions) => !versions.some((version) => version.status === 'BUILDING'),
      onSettled: () => undefined,
      maxDurationMs: BUILD_WATCH_MS,
    });
    return poller.stop;
  }, [building, refetch]);
  return { query, items, building };
}
