// Small readers and checks shared by the client modules: ids that become path segments, and the typed reads of an
// answer. An answer that does not have the shape the contract promises is a `PROTOCOL` error, never a guess.
import { SagatError, invalid } from './errors.js';

export const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** A run id is a UUID: anything else would be a path of its own (`..`, `x/operations`) and is refused before a request (SPEC §10). */
export function runIdOf(value: string): string {
  if (typeof value !== 'string' || !UUID.test(value)) throw invalid('a run id is a UUID');
  return value.toLowerCase();
}

export function namespaceIdOf(value: string): string {
  if (typeof value !== 'string' || !UUID.test(value)) throw invalid('a namespace id is a UUID');
  return value.toLowerCase();
}

/** An id of a schedule, a backfill, a batch job or a reset: a UUID, since it becomes a path segment. */
export function idOf(value: string, what: string): string {
  if (typeof value !== 'string' || !UUID.test(value)) throw invalid(`${what} is a UUID`);
  return value.toLowerCase();
}

/** A name in a URL path segment: never empty and never `.` or `..`; the rest is percent-encoded where it is used. */
export function segmentOf(value: string, what: string): string {
  if (typeof value !== 'string' || value === '' || value === '.' || value === '..') throw invalid(`${what} is not a name: it is empty, \`.\` or \`..\``);
  return value;
}

export function object(value: unknown, what: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new SagatError({ code: 'PROTOCOL', message: `${what} is not an object` });
  }
  return value as Record<string, unknown>;
}

export function str(raw: Record<string, unknown>, key: string): string {
  const value = raw[key];
  if (typeof value !== 'string') throw new SagatError({ code: 'PROTOCOL', message: `${key} is missing from the answer` });
  return value;
}

export function optStr(raw: Record<string, unknown>, key: string): string | null {
  const value = raw[key];
  return typeof value === 'string' ? value : null;
}

export function optDate(raw: Record<string, unknown>, key: string): Date | null {
  const value = raw[key];
  if (typeof value !== 'string') return null;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date;
}

export function num(raw: Record<string, unknown>, key: string, fallback: number): number {
  const value = raw[key];
  return typeof value === 'number' ? value : fallback;
}

export function revision(raw: Record<string, unknown>, key: string, minimum = 0): number {
  const value = raw[key];
  if (!Number.isSafeInteger(value) || (value as number) < minimum) {
    throw new SagatError({ code: 'PROTOCOL', message: `${key} is invalid in the answer` });
  }
  return value as number;
}
