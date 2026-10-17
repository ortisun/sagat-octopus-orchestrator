import { invalid } from './errors.js';
import { canonicalJson } from './json.js';

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export type DatabaseType = 'null' | 'boolean' | 'integer' | 'decimal' | 'float' | 'text' | 'timestamp' | 'date' | 'binary' | 'json';
export type DatabaseValue =
  | { type: 'null'; value: null }
  | { type: 'boolean'; value: boolean }
  | { type: 'float'; value: number }
  /** Exact integer/decimal text, RFC3339/date text, and base64 binary. */
  | { type: 'integer' | 'decimal' | 'text' | 'timestamp' | 'date' | 'binary'; value: string }
  | { type: 'json'; value: Json };
export type DatabaseParameter = { literal: DatabaseValue } | { type: DatabaseType; expression: string };
export interface DatabaseLimits {
  maxStatementBytes?: number; maxParameters?: number; maxParameterBytes?: number;
  maxRows?: number; maxResultBytes?: number; timeoutMs?: number;
  connectTimeoutMs?: number; poolSize?: number; idleTimeoutMs?: number;
}
/** Only a scoped logical name/revision; no host, credentials or provider routing. */
export interface ConnectionRef { name: string; revision?: number }
export interface DatabaseOptions {
  connectionRef: ConnectionRef;
  operation: 'query' | 'execute';
  statement: string;
  parameters?: DatabaseParameter[];
  limits?: DatabaseLimits;
  timeoutMs?: number;
  onError?: string;
}
const TYPES = new Set(['null', 'boolean', 'integer', 'decimal', 'float', 'text', 'timestamp', 'date', 'binary', 'json']);
const CEILINGS: Record<string, number> = { maxStatementBytes: 65536, maxParameters: 256, maxParameterBytes: 262144, maxRows: 10000, maxResultBytes: 4194304, timeoutMs: 30000, connectTimeoutMs: 10000, poolSize: 16, idleTimeoutMs: 60000 };
const object = (v: unknown): v is Record<string, unknown> => v !== null && typeof v === 'object' && !Array.isArray(v);
function keys(value: Record<string, unknown>, allowed: string[]): boolean { return Object.keys(value).every(k => allowed.includes(k)); }
const full = (pattern: RegExp, value: string): boolean => pattern.exec(value)?.[0] === value;
function parameter(p: unknown): boolean {
  if (!object(p)) return false;
  if ('literal' in p) {
    const v = p['literal'];
    if (!keys(p, ['literal']) || !object(v) || !keys(v, ['type', 'value']) || !('value' in v)) return false;
    const value = v['value'];
    switch (v['type']) {
      case 'null': return value === null;
      case 'boolean': return typeof value === 'boolean';
      case 'float': return typeof value === 'number' && Number.isFinite(value);
      case 'integer': return typeof value === 'string' && value.length <= 40 && full(/^-?[0-9]+$/, value);
      case 'decimal': return typeof value === 'string' && value.length <= 16384 && full(/^-?[0-9]+(?:\.[0-9]+)?$/, value);
      case 'text': return typeof value === 'string' && !value.includes('\0');
      case 'timestamp': case 'date': case 'binary': return typeof value === 'string';
      case 'json': return true; // Canonical JSON validation below; PostgreSQL conversion stays server-side.
      default: return false;
    }
  }
  return keys(p, ['type', 'expression']) && typeof p['type'] === 'string' && TYPES.has(p['type'])
    && typeof p['expression'] === 'string' && p['expression'].length > 0;
}
/** @internal. Shape validation only: SQL parsing, policy, access and retries belong to the server. */
export function databaseNode(id: string, options: DatabaseOptions): Record<string, Json> {
  const fail = () => invalid('invalid managed database operation');
  if (!object(options) || !keys(options, ['connectionRef', 'operation', 'statement', 'parameters', 'limits', 'timeoutMs', 'onError'])) throw fail();
  const ref = options.connectionRef;
  if (!object(ref) || !keys(ref, ['name', 'revision']) || typeof ref.name !== 'string'
    || !full(/^[a-z][a-z0-9-]{1,61}[a-z0-9]$/, ref.name) || ref.name.includes('--')
    || (ref.revision !== undefined && (!Number.isInteger(ref.revision) || ref.revision < 1 || ref.revision > 2147483647))) throw fail();
  if (!['query', 'execute'].includes(options.operation) || typeof options.statement !== 'string'
    || !options.statement.trim() || options.statement.includes('\0') || Buffer.byteLength(options.statement) > 65536) throw fail();
  if (options.parameters !== undefined && (!Array.isArray(options.parameters) || options.parameters.length > 256 || !options.parameters.every(parameter))) throw fail();
  if (options.limits !== undefined && (!object(options.limits) || Object.entries(options.limits).some(([k, v]) => !Object.hasOwn(CEILINGS, k) || (v !== undefined && (typeof v !== 'number' || !Number.isInteger(v) || v < 1 || v > CEILINGS[k]!))))) throw fail();
  if (options.timeoutMs !== undefined && (!Number.isInteger(options.timeoutMs) || options.timeoutMs < 1 || options.timeoutMs > 60000)) throw fail();
  if (options.onError !== undefined && (typeof options.onError !== 'string' || !options.onError)) throw fail();
  const present = Object.fromEntries(Object.entries(options).filter(([, v]) => v !== undefined));
  // Own a bounded snapshot, so caller mutations cannot silently retarget the retained builder.
  try {
    if (Buffer.byteLength(canonicalJson(options.parameters ?? [])) > 262144) throw fail();
    return JSON.parse(canonicalJson({ id, type: 'database', ...present })) as Record<string, Json>;
  } catch { throw fail(); }
}
