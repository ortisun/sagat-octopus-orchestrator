import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';
import test from 'node:test';

const require = createRequire(import.meta.url);
const consumers = ['@eslint/config-array', '@eslint/eslintrc', 'eslint', 'eslint-plugin-jsx-a11y', '@redocly/openapi-core', '@typescript-eslint/typescript-estree'];

for (const consumer of consumers) {
  test(`${consumer} resolves the reviewed bridge and patched matcher`, () => {
    const fromConsumer = createRequire(require.resolve(consumer));
    const entry = fromConsumer.resolve('minimatch');
    const metadata = fromConsumer('minimatch/package.json');
    assert.equal(metadata.name, 'minimatch');
    assert.equal(metadata.version, '11.0.1+sagat.1');
    const fromBridge = createRequire(entry);
    assert.equal(fromBridge('minimatch-upstream/package.json').version, '11.0.1');
    const match = fromConsumer('minimatch');
    assert.equal(typeof match, 'function');
    assert.equal(match.minimatch, match);
    assert.equal(match('src/a.tsx', '**/*.{ts,tsx}'), true);
    assert.equal(match('dist/a.js', '**/*.tsx'), false);
    assert.equal(match('.config.js', '*.js'), false);
    assert.equal(match('.config.js', '*.js', { dot: true }), true);
    assert.equal(match('INPUT', 'input', { nocase: true }), true);
    assert.equal(new match.Minimatch('**/node_modules/**', { dot: true }).match('a/node_modules/.cache/file'), true);
    assert.deepEqual(match.match(['a.ts', 'b.js'], '*.ts'), ['a.ts']);
    assert.deepEqual(['a.ts', 'b.js'].filter(match.filter('*.js')), ['b.js']);
    assert.equal(match.defaults({ dot: true })('.config', '*'), true);
  });
}

test('native ESM has both default and named imports required by tooling', async () => {
  const fromConsumer = createRequire(require.resolve('@eslint/config-array'));
  const imported = await import(pathToFileURL(fromConsumer.resolve('minimatch')).href);
  assert.equal(typeof imported.default, 'function');
  assert.equal(imported.default, imported.minimatch);
  assert.equal(new imported.Minimatch('src/**').match('src/a.ts'), true);
});

test('optimization level 2 bounds globparts amplification in the actual installed matcher', () => {
  const fromConsumer = createRequire(require.resolve('@typescript-eslint/typescript-estree'));
  const entry = fromConsumer.resolve('minimatch');
  const result = spawnSync(process.execPath, ['--max-old-space-size=96', '-e', `
    const assert = require('node:assert/strict');
    const { Minimatch } = require(${JSON.stringify(entry)});
    const pattern = Array(8).fill('**/../a/b').join('/');
    const parsed = new Minimatch(pattern, { optimizationLevel: 2, braceExpandMax: 64 });
    assert.equal(parsed.set.length, 64);
    const simple = new Minimatch('**/../a/b/**/../c/d', { optimizationLevel: 2 });
    assert.equal(simple.match('../a/b/../c/d'), true);
    assert.equal(simple.match('x/a/b/y/c/d'), true);
  `], { encoding: 'utf8', timeout: 3000, maxBuffer: 100_000 });
  assert.equal(result.error, undefined, `bounded parser: ${result.error}`);
  assert.equal(result.status, 0, result.stderr);
});

test('ESLint matching retains directory traversal, basename and negated-pattern options', () => {
  const fromEslint = createRequire(require.resolve('eslint'));
  const { Minimatch } = fromEslint('minimatch');
  assert.equal(new Minimatch('src/**/*.ts').match('src/nested', true), true);
  assert.equal(new Minimatch('src/**/*.ts').match('vendor', true), false);
  assert.equal(new Minimatch('*.ts', { matchBase: true }).match('src/a.ts'), true);
  assert.equal(new Minimatch('!src/**', { flipNegate: true }).match('src/a.ts'), true);
  assert.equal(new Minimatch('!src/**', { flipNegate: true }).match('vendor/a.ts'), false);
});

test('jsx-a11y wildcard custom controls exercise the legacy callable matcher', () => {
  const hasLabel = require('eslint-plugin-jsx-a11y/lib/util/mayHaveAccessibleLabel');
  const containsControl = require('eslint-plugin-jsx-a11y/lib/util/mayContainChildComponent');
  const child = { type: 'JSXElement', openingElement: { name: { type: 'JSXIdentifier', name: 'FormInput' }, attributes: [] }, children: [] };
  assert.equal(containsControl({ children: [child] }, 'Form*'), true);
  assert.equal(containsControl({ children: [child] }, 'Other*'), false);
  assert.equal(hasLabel(child, 1, [], undefined, ['Form*']), false);
  assert.equal(hasLabel(child, 1, [], undefined, ['Other*']), true);
});

test('Redocly URL-header rules include only matching hosts and paths', async () => {
  const { readFileFromUrl } = require('@redocly/openapi-core/lib/utils');
  for (const [url, expected] of [
    ['https://specs.example.test/api/openapi.yaml', { 'x-specific': 'fixture', 'x-host': 'fixture' }],
    ['https://specs.example.test/other.yaml', { 'x-host': 'fixture' }],
    ['https://other.example.test/api/openapi.yaml', {}],
  ]) {
    const result = await readFileFromUrl(url, {
      headers: [
        { matches: 'https://specs.example.test/api/**', name: 'x-specific', value: 'fixture' },
        { matches: 'specs.example.test/**', name: 'x-host', value: 'fixture' },
      ],
      customFetch: async (request, options) => {
        assert.equal(request, url);
        assert.deepEqual(options.headers, expected);
        return new Response('openapi: 3.1.0', { headers: { 'content-type': 'application/yaml' } });
      },
    });
    assert.equal(result.body, 'openapi: 3.1.0');
  }
});
