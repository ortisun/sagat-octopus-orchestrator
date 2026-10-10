import { SagatError } from './errors.js';

/** Canonical JSON text: UTF-8, keys sorted at every depth, no insignificant whitespace (SPEC §7). */
export function canonicalJson(value: unknown): string {
  return JSON.stringify(sortKeys(value));
}

function sortKeys(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortKeys);
  if (value !== null && typeof value === 'object') {
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(value as Record<string, unknown>).sort()) {
      const item = (value as Record<string, unknown>)[key];
      if (item !== undefined) out[key] = sortKeys(item);
    }
    return out;
  }
  return value;
}

/**
 * The JSON text of a value to send. An integer beyond 2^53 - 1 cannot be held by a JavaScript number
 * without being rounded, and a rounded identifier or amount is worse than an error (SPEC §8);
 * `NaN` and the infinities are not JSON.
 */
export function encodeJson(value: unknown): string {
  assertPlain(value, '$');
  return JSON.stringify(value);
}

function assertPlain(value: unknown, path: string): void {
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) {
      throw new SagatError({ code: 'VALIDATION_FAILED', message: `${path} is not a finite number` });
    }
    if (Number.isInteger(value) && !Number.isSafeInteger(value)) {
      throw new SagatError({ code: 'VALIDATION_FAILED', message: `${path} is an integer beyond 2^53 - 1: send it as a string` });
    }
  } else if (typeof value === 'bigint') {
    throw new SagatError({ code: 'VALIDATION_FAILED', message: `${path} is a bigint: send it as a string` });
  } else if (Array.isArray(value)) {
    value.forEach((item, i) => assertPlain(item, `${path}[${i}]`));
  } else if (value !== null && typeof value === 'object') {
    for (const [key, item] of Object.entries(value as Record<string, unknown>)) {
      if (item !== undefined) assertPlain(item, `${path}.${key}`);
    }
  }
}

/** An RFC 3339 instant in UTC with a `Z`, as the SDK writes every instant. */
export function instant(value: Date): string {
  return value.toISOString();
}
