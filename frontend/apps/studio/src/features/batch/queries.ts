import { useQuery } from '@tanstack/react-query';
import { isApiError } from '../../api/errors';
import { useApp, useNamespace } from '../../app/context';

/**
 * The id of the namespace the person works in. The Studio holds the slug (it travels as a header);
 * the routes of batch operations carry the id, which the list of namespaces gives.
 */
export function useNamespaceId(): { id: string | undefined; pending: boolean; error: unknown } {
  const { ep } = useApp();
  const slug = useNamespace();
  const list = useQuery({ queryKey: ['namespaces'], queryFn: ({ signal }) => ep.namespaces(signal) });
  return { id: list.data?.items.find((namespace) => namespace.slug === slug)?.id, pending: list.isPending, error: list.error };
}

/** Whether the platform refused the selector as one it does not support: a closed reason, kept for a selector a later version may not take. */
export function selectorNotSupported(error: unknown): boolean {
  return isApiError(error) && error.reason === 'BATCH_SELECTOR_UNSUPPORTED';
}

/** Whether the platform answered that it does not serve batch operations at all (the deployment has them off). */
export function notServed(error: unknown): boolean {
  return isApiError(error) && error.reason === 'ROUTE_NOT_FOUND';
}
