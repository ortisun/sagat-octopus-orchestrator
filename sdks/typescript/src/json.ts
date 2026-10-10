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
      // defineProperty, not assignment: `out['__proto__'] = x` would set the prototype and lose the key.
      if (item !== undefined) Object.defineProperty(out, key, { value: sortKeys(item), enumerable: true, writable: true, configurable: true });
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

const UNSAFE = /-?\d{16,}/;

/**
 * Parses JSON that came from the platform. A JavaScript number cannot hold an integer beyond 2^53 - 1: it
 * would be rounded without a word, and a rounded identifier is another identifier (SPEC §8). So the text is
 * looked at first and such an integer is an error; numbers with a fraction or an exponent are floats by
 * nature and are parsed as they are.
 */
export function parseJson(text: string): unknown {
  if (UNSAFE.test(text)) assertSafeIntegers(text);
  return JSON.parse(text);
}

/** Walks the text outside its strings and refuses an integer literal beyond 2^53 - 1. */
function assertSafeIntegers(text: string): void {
  const limit = BigInt(Number.MAX_SAFE_INTEGER);
  let i = 0;
  while (i < text.length) {
    const c = text.charCodeAt(i);
    if (c === 0x22 /* " */) {
      i++;
      while (i < text.length && text.charCodeAt(i) !== 0x22) i += text.charCodeAt(i) === 0x5c /* \ */ ? 2 : 1;
      i++;
    } else if (c === 0x2d || (c >= 0x30 && c <= 0x39)) {
      let j = i + 1;
      while (j < text.length && /[0-9.eE+-]/.test(text[j]!)) j++;
      const token = text.slice(i, j);
      if (/^-?\d+$/.test(token)) {
        const magnitude = BigInt(token.replace('-', ''));
        if (magnitude > limit) {
          throw new SagatError({ code: 'PROTOCOL', message: 'the document holds an integer beyond 2^53 - 1, which cannot be read without rounding it' });
        }
      }
      i = j;
    } else {
      i++;
    }
  }
}

/** An RFC 3339 instant in UTC with a `Z`, as the SDK writes every instant. */
export function instant(value: Date): string {
  return value.toISOString();
}
