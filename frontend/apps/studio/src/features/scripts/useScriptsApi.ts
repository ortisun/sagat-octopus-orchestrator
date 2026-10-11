import { useQuery } from '@tanstack/react-query';
import { useMemo } from 'react';
import { useApp, useNamespace } from '../../app/context';
import { scriptsApi } from './api';
import type { ScriptsApi } from './api';

export type ScriptsContext =
  /** The namespaces have not been read yet. */
  | { state: 'loading' }
  | { state: 'failed'; error: unknown; retry: () => void }
  /** The user belongs to no namespace, or the one chosen is not among them. */
  | { state: 'none' }
  | { state: 'ready'; api: ScriptsApi; namespace: string };

/**
 * The scripts API for the namespace the user works in. The Studio's namespace is a slug (sent as a
 * header on the rest of the API); the scripts API is addressed by the namespace's *id*, so the id is
 * looked up in the list of namespaces, which the shell already holds (the same query key).
 */
export function useScriptsApi(): ScriptsContext {
  const { api, ep } = useApp();
  const slug = useNamespace();
  const namespaces = useQuery({ queryKey: ['namespaces'], queryFn: ({ signal }) => ep.namespaces(signal) });
  const found = namespaces.data?.items.find((item) => item.slug === slug);
  const id = found?.id;
  const scripts = useMemo(() => (id === undefined ? undefined : scriptsApi(api, id)), [api, id]);

  if (namespaces.isError) return { state: 'failed', error: namespaces.error, retry: () => void namespaces.refetch() };
  if (namespaces.isPending) return { state: 'loading' };
  if (scripts === undefined || slug === undefined) {
    // The shell picks the first namespace when none (or an unknown one) is chosen; until it has, there is nothing to ask.
    return namespaces.data.items.length > 0 ? { state: 'loading' } : { state: 'none' };
  }
  return { state: 'ready', api: scripts, namespace: slug };
}
