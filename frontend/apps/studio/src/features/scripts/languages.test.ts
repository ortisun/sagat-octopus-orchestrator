import { describe, expect, it } from 'vitest';
import { LANGUAGES, LIMITS, SAMPLE_INPUT, SCRIPT_NAME, languageInfo, monacoLanguage, sampleSource } from './languages';
import { OCTOPUS_DTS } from './octopusTypes';
import { JSON_SCHEMA_META, META_SCHEMA_URI } from './jsonSchemaMeta';
import { checkInputText } from './jsonText';

describe('languages', () => {
  it('builds TypeScript and Python and does not pretend about the others (G96)', () => {
    expect(LANGUAGES.filter((language) => language.built).map((language) => language.id)).toEqual(['typescript', 'python']);
    expect(LANGUAGES.filter((language) => !language.built).map((language) => language.id)).toEqual(['csharp', 'go', 'java']);
  });

  it('has a language service in the browser for TypeScript only, and says what the others get', () => {
    expect(LANGUAGES.filter((language) => language.lint).map((language) => language.id)).toEqual(['typescript']);
    expect(languageInfo('python').checks).toBe('Syntax highlighting only; the platform checks the source when it builds.');
    expect(languageInfo('typescript').checks).toMatch(/advice only/);
  });

  it('shows a language it does not know as plain text, not as TypeScript', () => {
    expect(monacoLanguage('cobol')).toBe('plaintext');
    expect(languageInfo('cobol').built).toBe(false);
    expect(monacoLanguage('python')).toBe('python');
  });

  it('starts a script from the shape the runner expects', () => {
    expect(sampleSource('typescript')).toMatch(/export default/);
    expect(sampleSource('python')).toMatch(/def run\(input, octopus\)/);
    expect(checkInputText(SAMPLE_INPUT).kind).toBe('ok');
  });

  it('knows the limits and the name rule of the platform', () => {
    expect(LIMITS).toEqual({ sourceBytes: 262_144, schemaBytes: 65_536, inputBytes: 245_760 });
    expect(SCRIPT_NAME.test('double-it')).toBe(true);
    for (const bad of ['', 'a', 'ab', 'Double', '1abc', 'abc-', 'a_b_c', 'a'.repeat(64)]) expect(SCRIPT_NAME.test(bad)).toBe(false);
    expect(SCRIPT_NAME.test('a'.repeat(63))).toBe(true);
  });
});

describe('the declarations for autocomplete', () => {
  it('describe what the runner gives a script and nothing it does not', () => {
    for (const member of ['fetch(', 'secret(', 'log(', 'idempotencyKey', 'text()', 'json()']) expect(OCTOPUS_DTS).toContain(member);
    // The input is the first argument of the default function; there is no octopus.input.
    expect(OCTOPUS_DTS).not.toMatch(/\binput\b/);
    expect(OCTOPUS_DTS).toContain('interface Octopus');
  });
});

describe('the bundled meta-schema', () => {
  it('is the draft-07 one under its own address, so nothing is fetched', () => {
    expect(META_SCHEMA_URI).toBe('http://json-schema.org/draft-07/schema');
    expect(JSON_SCHEMA_META['$id']).toBe('http://json-schema.org/draft-07/schema#');
    const properties = JSON_SCHEMA_META['properties'] as Record<string, unknown>;
    for (const keyword of ['type', 'properties', 'required', 'items', 'enum', 'pattern', '$defs', 'allOf']) expect(properties).toHaveProperty(keyword);
  });
});
