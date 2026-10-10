import { describe, expect, it } from 'vitest';
import { checkInputText, checkSchemaText, sameSchema, schemaToText, valueToText } from './jsonText';

describe('checkSchemaText', () => {
  it('reads empty text as "no schema"', () => {
    expect(checkSchemaText('')).toEqual({ kind: 'empty' });
    expect(checkSchemaText('  \n ')).toEqual({ kind: 'empty' });
  });

  it('accepts a JSON object', () => {
    expect(checkSchemaText('{"type":"object","required":["n"]}')).toEqual({ kind: 'ok', value: { type: 'object', required: ['n'] } });
  });

  it('refuses what is not JSON, and says so', () => {
    const out = checkSchemaText('{"type": ');
    expect(out.kind).toBe('invalid');
    expect(out.kind === 'invalid' && out.message).toMatch(/^Not valid JSON/);
  });

  it('refuses JSON that is not an object (the platform stores an object or nothing)', () => {
    for (const text of ['[]', '"x"', '12', 'null', 'true']) {
      expect(checkSchemaText(text)).toEqual({ kind: 'invalid', message: 'A schema is a JSON object.' });
    }
  });

  it('refuses a schema over 64 KiB before the platform has to', () => {
    const big = JSON.stringify({ description: 'x'.repeat(70 * 1024) });
    expect(checkSchemaText(big)).toEqual({ kind: 'invalid', message: 'A schema is at most 64 KiB.' });
  });
});

describe('checkInputText', () => {
  it('lets the input be empty (the script then receives null), any JSON value, or refuses', () => {
    expect(checkInputText('')).toEqual({ kind: 'empty' });
    expect(checkInputText('null')).toEqual({ kind: 'ok', value: null });
    expect(checkInputText('[1,2]')).toEqual({ kind: 'ok', value: [1, 2] });
    expect(checkInputText('{"n": 21}')).toEqual({ kind: 'ok', value: { n: 21 } });
    expect(checkInputText('{n: 21}').kind).toBe('invalid');
  });

  it('refuses an input over 240 KiB', () => {
    expect(checkInputText(JSON.stringify('x'.repeat(250 * 1024)))).toEqual({ kind: 'invalid', message: 'The input is at most 240 KiB.' });
  });
});

describe('text of values', () => {
  it('writes a schema as an editor shows it, and nothing for no schema', () => {
    expect(schemaToText({ type: 'object' })).toBe('{\n  "type": "object"\n}\n');
    expect(schemaToText(undefined)).toBe('');
    expect(schemaToText(['not', 'a', 'schema'])).toBe('');
    expect(valueToText(null)).toBe('null');
    expect(valueToText(undefined)).toBe('');
    expect(valueToText({ a: [1] })).toBe('{\n  "a": [\n    1\n  ]\n}');
  });
});

describe('sameSchema', () => {
  it('does not take the author\'s spacing and key order for a change', () => {
    expect(sameSchema('{"type":"object","required":["n"]}', '{\n  "required": ["n"],\n  "type": "object"\n}\n')).toBe(true);
    expect(sameSchema('{"a":{"y":1,"x":[1,2]}}', '{"a":{"x":[1,2],"y":1}}')).toBe(true);
  });

  it('knows a real change, including the order of an array, which matters', () => {
    expect(sameSchema('{"type":"object"}', '{"type":"string"}')).toBe(false);
    expect(sameSchema('{"enum":[1,2]}', '{"enum":[2,1]}')).toBe(false);
    expect(sameSchema('{"a":1}', '{"a":1,"b":2}')).toBe(false);
  });

  it('treats no schema as no schema, and text that is not a schema as different unless it is identical', () => {
    expect(sameSchema('', '  \n')).toBe(true);
    expect(sameSchema('', '{}')).toBe(false);
    expect(sameSchema('{"a":', '{"a":')).toBe(true);
    expect(sameSchema('{"a":', '{"a":1}')).toBe(false);
  });
});
