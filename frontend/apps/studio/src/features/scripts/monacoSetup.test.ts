import { describe, expect, it, vi } from 'vitest';
import type { WorkerFactories } from './monacoSetup';
import {
  installMonacoEnvironment,
  modelAddress,
  monacoEnvironment,
  resolveTheme,
  severityOf,
  studioThemeData,
  studioThemeName,
  tidyProblems,
  workerKind,
} from './monacoSetup';

function factories() {
  const made: string[] = [];
  const make = (name: string) =>
    class FakeWorker {
      constructor() {
        made.push(name);
      }
    } as unknown as new () => Worker;
  const set: WorkerFactories = { editor: make('editor'), json: make('json'), typescript: make('typescript') };
  return { made, set };
}

describe('workerKind', () => {
  it('gives JSON and TypeScript their own language services, and everything else the editor worker', () => {
    expect(workerKind('json')).toBe('json');
    expect(workerKind('typescript')).toBe('typescript');
    expect(workerKind('javascript')).toBe('typescript');
    expect(workerKind('editorWorkerService')).toBe('editor');
    expect(workerKind('css')).toBe('editor');
    expect(workerKind('')).toBe('editor');
    expect(workerKind('__proto__')).toBe('editor');
  });
});

describe('the Monaco environment', () => {
  it('makes the worker of the bundle for each label, and never asks for a URL', () => {
    const { made, set } = factories();
    const environment = monacoEnvironment(set);
    environment.getWorker('workerMain.js', 'json');
    environment.getWorker('workerMain.js', 'typescript');
    environment.getWorker('workerMain.js', 'editorWorkerService');
    environment.getWorker('workerMain.js', 'something-unknown');
    expect(made).toEqual(['json', 'typescript', 'editor', 'editor']);
    expect('getWorkerUrl' in environment).toBe(false);
  });

  it('is installed once; a second install leaves the first in place', () => {
    const target: { MonacoEnvironment?: unknown } = {};
    const first = installMonacoEnvironment(target, factories().set);
    const second = installMonacoEnvironment(target, factories().set);
    expect(target.MonacoEnvironment).toBe(first);
    expect(second).toBe(first);
  });

  it('does not replace an environment somebody else already put there', () => {
    const theirs = { getWorker: vi.fn() };
    const target: { MonacoEnvironment?: unknown } = { MonacoEnvironment: theirs };
    expect(installMonacoEnvironment(target, factories().set)).toBe(theirs);
  });
});

describe('resolveTheme', () => {
  it('follows the page\'s own choice first, then the system', () => {
    expect(resolveTheme('dark', false)).toBe('vs-dark');
    expect(resolveTheme('light', true)).toBe('vs');
    expect(resolveTheme(null, true)).toBe('vs-dark');
    expect(resolveTheme(undefined, false)).toBe('vs');
    expect(resolveTheme('sepia', true)).toBe('vs-dark');
  });

  it('uses Monaco high-contrast themes while forced colors are active', () => {
    expect(resolveTheme('dark', false, true)).toBe('hc-black');
    expect(resolveTheme('light', true, true)).toBe('hc-light');
    expect(resolveTheme(null, true, true)).toBe('hc-black');
    expect(resolveTheme(null, false, true)).toBe('hc-light');
  });
});

describe('the Studio Monaco theme', () => {
  it('keeps the inherited syntax palette and replaces comments with the semantic muted text color', () => {
    expect(studioThemeName('vs')).toBe('sagat-light');
    expect(studioThemeName('vs-dark')).toBe('sagat-dark');
    expect(studioThemeName('hc-black')).toBe('hc-black');
    expect(studioThemeName('hc-light')).toBe('hc-light');

    expect(studioThemeData('vs-dark', '#8f9298')).toEqual({
      base: 'vs-dark',
      inherit: true,
      rules: [{ token: 'comment', foreground: '8F9298' }],
      colors: {},
    });
  });

  it('rejects a non-hex semantic token instead of passing invalid theme data to Monaco', () => {
    expect(() => studioThemeData('vs-dark', 'CanvasText')).toThrow(/six-digit hex/);
  });
});

describe('modelAddress', () => {
  it('is in memory, fixed per role, and gives TypeScript a module name', () => {
    expect(modelAddress('source', 'typescript')).toBe('file:///main.ts');
    expect(modelAddress('source', 'python')).toBe('inmemory://sagat/source.py');
    expect(modelAddress('inputSchema', 'json')).toBe('inmemory://sagat/inputSchema.json');
    expect(modelAddress('testInput', 'json')).toBe('inmemory://sagat/testInput.json');
    expect(modelAddress('version', 'plaintext')).toBe('inmemory://sagat/version.txt');
  });

  it('never makes an address that fetches anything', () => {
    for (const role of ['source', 'inputSchema', 'outputSchema', 'testInput', 'version'] as const) {
      for (const language of ['typescript', 'python', 'json', 'plaintext']) {
        expect(modelAddress(role, language)).toMatch(/^(file:\/\/\/|inmemory:\/\/sagat\/)[A-Za-z]+\.[a-z]+$/);
      }
    }
  });
});

describe('problems', () => {
  it('maps the severities of the editor', () => {
    expect(severityOf(8)).toBe('error');
    expect(severityOf(4)).toBe('warning');
    expect(severityOf(2)).toBe('info');
    expect(severityOf(1)).toBe('info');
  });

  it('lists the worst first, then by place, bounded, with long messages clipped', () => {
    const many = Array.from({ length: 80 }, (_, i) => ({ severity: 'warning' as const, line: 100 - i, column: 1, message: 'm' }));
    const out = tidyProblems([{ severity: 'info', line: 1, column: 1, message: 'note' }, { severity: 'error', line: 9, column: 4, message: 'x'.repeat(500) }, { severity: 'error', line: 2, column: 7, message: 'first' }, ...many]);
    expect(out).toHaveLength(50);
    expect(out[0]).toMatchObject({ severity: 'error', line: 2 });
    expect(out[1]?.message.length).toBe(301);
    expect(out[2]?.severity).toBe('warning');
    expect(out[2]?.line).toBeLessThan(out[3]?.line ?? 0);
  });
});
