import type { DefinitionProblem, Validation } from '../../api/types';

export interface ProblemView {
  severity: 'error' | 'warning';
  code: string;
  message: string;
  /** The node it is about: named by the platform, or found from the position of the problem in the document. */
  nodeId: string | undefined;
  path: string | undefined;
}

export interface ValidationView {
  valid: boolean;
  problems: ProblemView[];
  warnings: ProblemView[];
}

const NODE_POINTER = /^\/nodes\/(\d{1,6})(?:\/|$)/;

/** The node a problem is about: the one the platform names, else the one at `/nodes/N` of the document that was checked. */
export function resolveNodeId(problem: Pick<DefinitionProblem, 'nodeId' | 'path'>, nodeIds: readonly (string | undefined)[]): string | undefined {
  if (typeof problem.nodeId === 'string' && problem.nodeId !== '') return problem.nodeId;
  const match = typeof problem.path === 'string' ? NODE_POINTER.exec(problem.path) : null;
  return match?.[1] === undefined ? undefined : nodeIds[Number(match[1])];
}

/** The ids of the nodes of a document in the order it holds them: what a `/nodes/N` pointer counts. */
export function nodeIdsOf(wire: Record<string, unknown>): (string | undefined)[] {
  const nodes = Array.isArray(wire['nodes']) ? (wire['nodes'] as unknown[]) : [];
  return nodes.map((n) =>
    typeof n === 'object' && n !== null && typeof (n as Record<string, unknown>)['id'] === 'string' ? ((n as Record<string, unknown>)['id'] as string) : undefined,
  );
}

export function viewOf(result: Validation, nodeIds: readonly (string | undefined)[]): ValidationView {
  const view = (p: DefinitionProblem, severity: ProblemView['severity']): ProblemView => ({
    severity,
    code: p.code,
    message: p.message,
    nodeId: resolveNodeId(p, nodeIds),
    path: typeof p.path === 'string' && p.path !== '' ? p.path : undefined,
  });
  return {
    valid: result.valid,
    problems: result.problems.map((p) => view(p, 'error')),
    warnings: result.warnings.map((p) => view(p, 'warning')),
  };
}

export function countsByNode(items: readonly { nodeId: string | undefined }[]): Map<string, number> {
  const out = new Map<string, number>();
  for (const item of items) if (item.nodeId !== undefined && item.nodeId !== '') out.set(item.nodeId, (out.get(item.nodeId) ?? 0) + 1);
  return out;
}

const plural = (n: number, one: string, many: string): string => `${n} ${n === 1 ? one : many}`;

/** "2 problems · 1 warning · 3 pieces of advice" */
export function summaryLine(problems: number, warnings: number, advice: number): string {
  return [plural(problems, 'problem', 'problems'), plural(warnings, 'warning', 'warnings'), plural(advice, 'piece of advice', 'pieces of advice')].join(' · ');
}
