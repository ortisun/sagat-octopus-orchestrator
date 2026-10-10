import { SagatError } from './errors.js';

/**
 * Canonical JSON text (RFC 8785, SPEC §7): UTF-8, no insignificant whitespace, the keys of every object sorted by
 * UTF-16 code unit, numbers as ECMAScript prints them. The text is built here and not through a JavaScript
 * object: an object lists integer-like keys first and in numeric order whatever order they were put in, which
 * would make `"10"` come after `"9"` and the text differ from the other SDKs'.
 */
export function canonicalJson(value: unknown): string {
  return canonical(value, '$');
}

function canonical(value: unknown, path: string): string {
  if (Array.isArray(value)) return `[${value.map((item, i) => (item === undefined ? 'null' : canonical(item, `${path}[${i}]`))).join(',')}]`;
  if (value !== null && typeof value === 'object') {
    const entries = value as Record<string, unknown>;
    const keys = Object.keys(entries).filter((key) => entries[key] !== undefined).sort();
    return `{${keys.map((key) => `${JSON.stringify(key)}:${canonical(entries[key], `${path}.${key}`)}`).join(',')}}`;
  }
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) throw new SagatError({ code: 'VALIDATION_FAILED', message: `${path} is not a finite number` });
    const text = JSON.stringify(value);
    // An integer beyond 2^53 - 1 that ECMAScript prints in digits was rounded on the way in: it is refused, so that the
    // text is the same bytes in every language. One it prints with an exponent (1e21) is a number, not a rounded integer.
    if (Number.isInteger(value) && !Number.isSafeInteger(value) && !text.includes('e')) {
      throw new SagatError({ code: 'VALIDATION_FAILED', message: `${path} is an integer beyond 2^53 - 1: write it as a string` });
    }
    return text;
  }
  if (typeof value === 'bigint') throw new SagatError({ code: 'VALIDATION_FAILED', message: `${path} is a bigint: write it as a string` });
  return JSON.stringify(value) ?? 'null';
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
