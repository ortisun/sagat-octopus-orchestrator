import type { IrNode } from './ir';

export function object(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

/** Basic authoring feedback. The platform validates SQL, bindings, policy and authorization. */
export function databaseIssue(node: IrNode): string | undefined {
  const ref = object(node['connectionRef']);
  if (typeof ref['name'] !== 'string' || ref['name'] === '') return 'Choose a connection.';
  if (ref['revision'] !== undefined && (!Number.isInteger(ref['revision']) || Number(ref['revision']) < 1)) return 'Connection revision must be a positive whole number.';
  if (!['query', 'execute'].includes(String(node['operation']))) return 'Choose an operation.';
  if (typeof node['statement'] !== 'string' || node['statement'].trim() === '') return 'Enter one SQL statement.';
  if (new TextEncoder().encode(node['statement']).length > 65_536) return 'SQL exceeds 64 KiB.';
  if (node['parameters'] !== undefined && !Array.isArray(node['parameters'])) return 'Parameters must be a JSON array of typed bindings.';
  if (Array.isArray(node['parameters']) && node['parameters'].length > 256) return 'At most 256 parameters.';
  const limits = object(node['limits']);
  for (const [field, max] of [['maxRows', 10_000], ['maxResultBytes', 4_194_304], ['timeoutMs', 30_000], ['connectTimeoutMs', 10_000]] as const) {
    const value = limits[field];
    if (value !== undefined && (!Number.isInteger(value) || Number(value) < 1 || Number(value) > max)) return `${field} must be a whole number from 1 to ${max}.`;
  }
  return undefined;
}
