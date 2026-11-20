import { validConnectionRef, type ConnectionRef } from './database.js';
import { invalid } from './errors.js';
import { canonicalJson } from './json.js';

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

/** A table of an Iceberg catalog: a namespace of 1 to 8 levels and a name. */
export interface IcebergTable { namespace: string[]; name: string }
/**
 * Where the records of the write come from: exactly one of an expression that evaluates to a list of objects (`inline`), one
 * that evaluates to 1 to 32 blob descriptors (`blobs`), or the id of an earlier root-level database step (`databaseResult`).
 */
export type IcebergRecords = { inline: string } | { blobs: string } | { databaseResult: string };
/** A field of a record (a JSON pointer, `/customer/name`) and the column it is written to. */
export interface IcebergMapping { field: string; column: string }
export interface IcebergLimits { maxRecordBytes?: number; maxRecords?: number; timeoutMs?: number }
/**
 * An append to an Iceberg table. Only a scoped logical connection name/revision: no catalog, bucket, credential or
 * secret ever appears in a workflow. The mode is `append`; overwrite and upsert are not built yet.
 */
export interface IcebergWriteOptions {
  connectionRef: ConnectionRef;
  table: IcebergTable;
  /** The branch or tag to commit to; `main` when absent. */
  ref?: string;
  records: IcebergRecords;
  mapping?: IcebergMapping[];
  ignoreUnknownFields?: boolean;
  limits?: IcebergLimits;
  /** `revert`: the step is undone by removing its commit. */
  compensation?: 'revert';
  onError?: string;
}

const LIMITS: Record<string, [number, number]> = { maxRecordBytes: [1024, 2147483648], maxRecords: [1, 100000000], timeoutMs: [1000, 3600000] };
const object = (v: unknown): v is Record<string, unknown> => v !== null && typeof v === 'object' && !Array.isArray(v);
const keys = (value: Record<string, unknown>, allowed: string[]): boolean => Object.keys(value).every(k => allowed.includes(k));
// 1 to `limit` bytes of UTF-8 without a control character: the platform's `plain` text.
const plain = (v: unknown, limit: number): v is string =>
  typeof v === 'string' && v.length > 0 && Buffer.byteLength(v) <= limit && !/[\u0000-\u001f\u007f-\u009f]/.test(v);
// A level or a name of a table: plain, and not a path component of its own.
const part = (v: unknown): boolean => plain(v, 255) && v !== '.' && v !== '..';
// A JSON pointer into a record whose escapes are `~0` and `~1`, of at most 16 segments.
function pointer(v: unknown): boolean {
  if (!plain(v, 512) || !v.startsWith('/')) return false;
  const segments = v.slice(1).split('/');
  return segments.length <= 16 && segments.every(s => s.length > 0) && !/~(?![01])/.test(v);
}

/** @internal. Shape validation only: the catalog, the access, the modes and the classification belong to the server. */
export function icebergWriteNode(id: string, options: IcebergWriteOptions): Record<string, Json> {
  const fail = () => invalid('invalid iceberg write');
  if (!object(options) || !keys(options, ['connectionRef', 'table', 'ref', 'records', 'mapping', 'ignoreUnknownFields', 'limits', 'compensation', 'onError'])) throw fail();
  if (!validConnectionRef(options.connectionRef)) throw fail();
  const table = options.table;
  if (!object(table) || !keys(table, ['namespace', 'name']) || !Array.isArray(table.namespace)
    || table.namespace.length < 1 || table.namespace.length > 8 || !table.namespace.every(part) || !part(table.name)) throw fail();
  if (options.ref !== undefined && !plain(options.ref, 255)) throw fail();
  const records = options.records;
  if (!object(records) || Object.keys(records).length !== 1) throw fail();
  const source = Object.keys(records)[0]!;
  const value = (records as Record<string, unknown>)[source];
  if (source === 'inline' || source === 'blobs') {
    // an expression of 1 to 1024 UTF-8 bytes, as everywhere else: the schema says 4096, the expression engine takes 1024
    if (typeof value !== 'string' || value.length < 1 || Buffer.byteLength(value) > 1024) throw fail();
  } else if (source === 'databaseResult') {
    if (!plain(value, 64)) throw fail();
  } else throw fail();
  if (options.mapping !== undefined) {
    const mapping = options.mapping;
    if (!Array.isArray(mapping) || mapping.length > 1024) throw fail();
    const fields = new Set<string>();
    const columns = new Set<string>();
    for (const pair of mapping) {
      if (!object(pair) || !keys(pair, ['field', 'column']) || !pointer(pair['field']) || !plain(pair['column'], 255)) throw fail();
      // a field or a column twice is two rows of the mapping that disagree about one thing
      if (fields.has(pair['field'] as string) || columns.has(pair['column'] as string)) throw fail();
      fields.add(pair['field'] as string);
      columns.add(pair['column'] as string);
    }
  }
  if (options.ignoreUnknownFields !== undefined && typeof options.ignoreUnknownFields !== 'boolean') throw fail();
  if (options.limits !== undefined) {
    const limits = options.limits;
    if (!object(limits)) throw fail();
    for (const [name, limit] of Object.entries(limits)) {
      const range = LIMITS[name];
      if (range === undefined) throw fail();
      if (limit !== undefined && (typeof limit !== 'number' || !Number.isInteger(limit) || limit < range[0] || limit > range[1])) throw fail();
    }
  }
  if (options.compensation !== undefined && options.compensation !== 'revert') throw fail();
  if (options.onError !== undefined && (typeof options.onError !== 'string' || !options.onError)) throw fail();
  const present = Object.fromEntries(Object.entries(options).filter(([, v]) => v !== undefined));
  // Own a bounded snapshot, so caller mutations cannot silently retarget the retained builder.
  try {
    return JSON.parse(canonicalJson({ id, type: 'icebergWrite', mode: 'append', ...present })) as Record<string, Json>;
  } catch { throw fail(); }
}
