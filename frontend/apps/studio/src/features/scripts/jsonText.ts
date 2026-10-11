import { LIMITS } from './languages';
import { clipLabel, utf8Length } from './text';

/** What a text box that should hold a JSON Schema holds. */
export type SchemaCheck = { kind: 'empty' } | { kind: 'ok'; value: Record<string, unknown> } | { kind: 'invalid'; message: string };

function isObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function parseProblem(error: unknown): string {
  const message = error instanceof Error ? error.message : 'it does not parse';
  return `Not valid JSON: ${clipLabel(message, 160)}`;
}

/**
 * The platform stores a schema as a JSON object of at most 64 KiB, and removes it when it is
 * `null`. Empty text means "no schema". The platform compiles the schema when the draft is saved
 * and its answer is the one that counts; this only keeps the Studio from sending what it already
 * knows will be refused.
 */
export function checkSchemaText(text: string): SchemaCheck {
  if (text.trim() === '') return { kind: 'empty' };
  if (utf8Length(text) > LIMITS.schemaBytes) return { kind: 'invalid', message: 'A schema is at most 64 KiB.' };
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    return { kind: 'invalid', message: parseProblem(error) };
  }
  if (!isObject(value)) return { kind: 'invalid', message: 'A schema is a JSON object.' };
  return { kind: 'ok', value };
}

/** The input of a test run. Empty means the script receives `null` (the platform's default). */
export type InputCheck = { kind: 'empty' } | { kind: 'ok'; value: unknown } | { kind: 'invalid'; message: string };

export function checkInputText(text: string): InputCheck {
  if (text.trim() === '') return { kind: 'empty' };
  if (utf8Length(text) > LIMITS.inputBytes) return { kind: 'invalid', message: 'The input is at most 240 KiB.' };
  try {
    return { kind: 'ok', value: JSON.parse(text) as unknown };
  } catch (error) {
    return { kind: 'invalid', message: parseProblem(error) };
  }
}

/** A schema from the platform as the text of an editor. */
export function schemaToText(schema: unknown): string {
  return isObject(schema) ? `${JSON.stringify(schema, null, 2)}\n` : '';
}

/** Any value from the platform as the text of an editor or a `<pre>`. */
export function valueToText(value: unknown): string {
  if (value === undefined) return '';
  try {
    return JSON.stringify(value, null, 2) ?? '';
  } catch {
    return '';
  }
}

/** JSON with the keys of every object in order, so two texts that mean the same thing are the same text. */
function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  if (isObject(value)) {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`)
      .join(',')}}`;
  }
  return JSON.stringify(value) ?? 'null';
}

/**
 * Whether two texts of a schema are the same schema, whatever their layout: the platform hands a
 * schema back as an object, so the author's own spacing and key order must not look like a change.
 * Two texts that are not both JSON objects are the same only if they are the same text.
 */
export function sameSchema(a: string, b: string): boolean {
  if (a === b) return true;
  const left = checkSchemaText(a);
  const right = checkSchemaText(b);
  if (left.kind === 'empty' && right.kind === 'empty') return true;
  if (left.kind !== 'ok' || right.kind !== 'ok') return false;
  return canonical(left.value) === canonical(right.value);
}
