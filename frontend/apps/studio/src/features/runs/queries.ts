import { useQuery } from '@tanstack/react-query';
import type { UseQueryResult } from '@tanstack/react-query';
import type { Page, Run, VersionSummary, Workflow } from '../../api/types';
import { useApp, useCan, useNamespace } from '../../app/context';

/** The most pages of workflows read to name the runs; a namespace with more shows ids for the rest. */
const WORKFLOW_PAGES = 5;
const PAGE = 200;

export interface WorkflowIndex {
  byId: ReadonlyMap<string, Workflow>;
  /** Sorted by name, for a filter. */
  list: readonly Workflow[];
  /** There were more workflows than were read. */
  truncated: boolean;
}

/** The workflows of the namespace, to name runs and to filter them. Needs a namespace. */
export function useWorkflowIndex(): UseQueryResult<WorkflowIndex> {
  const { ep } = useApp();
  const namespace = useNamespace();
  const allowed = useCan('definition:read');
  return useQuery({
    queryKey: ['workflows', 'index', namespace],
    enabled: namespace !== undefined && allowed,
    staleTime: 60_000,
    queryFn: async ({ signal }) => {
      const items: Workflow[] = [];
      let cursor: string | undefined;
      let truncated = false;
      for (let page = 0; page < WORKFLOW_PAGES; page++) {
        const result: Page<Workflow> = await ep.listWorkflows({ limit: PAGE, ...(cursor ? { cursor } : {}) }, signal);
        items.push(...result.items);
        cursor = result.nextCursor ?? undefined;
        if (cursor === undefined) break;
        truncated = page === WORKFLOW_PAGES - 1;
      }
      return { byId: new Map(items.map((w) => [w.id, w])), list: [...items].sort((a, b) => a.name.localeCompare(b.name)), truncated };
    },
  });
}

/** The published versions of a workflow (immutable, so they are kept for a while). */
export function useVersions(workflowId: string | undefined): UseQueryResult<readonly VersionSummary[]> {
  const { ep } = useApp();
  const allowed = useCan('definition:read');
  return useQuery({
    queryKey: ['versions', workflowId],
    enabled: workflowId !== undefined && allowed,
    staleTime: 5 * 60_000,
    queryFn: async ({ signal }) => (await ep.versions(workflowId as string, { limit: PAGE }, signal)).items,
  });
}

/** `v3` for a version the platform lists, or a short form of its id when it cannot say. */
export function versionLabel(versions: readonly VersionSummary[] | undefined, versionId: string, active?: VersionSummary | null): string {
  const found = active?.versionId === versionId ? active : versions?.find((v) => v.versionId === versionId);
  return found ? `v${found.versionNumber}` : `version ${versionId.slice(0, 8)}`;
}

/** Private runs deliberately have no public workflow to link to. */
export function originLabel(origin: Run['origin']): string | undefined {
  return origin === 'AD_HOC_STEP' ? 'Ad hoc database step' : origin === 'AD_HOC_WORKFLOW' ? 'Ad hoc database workflow' : origin === 'CONNECTION_PROBE' ? 'Connection probe' : undefined;
}
