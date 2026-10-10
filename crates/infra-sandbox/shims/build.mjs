// The builder of a TypeScript script: erases the types, checks that what is left parses as a module,
// and prints the result as a single line. It runs in a sandbox with no network.
import { writeFileSync } from 'node:fs';
import { createInterface } from 'node:readline';
import { stripTypeScriptTypes } from 'node:module';
import { spawnSync } from 'node:child_process';

const say = (message) => process.stdout.write(JSON.stringify(message) + '\n');
const reject = (message) => {
  say({ op: 'rejected', message: String(message).split('\n').slice(0, 6).join('\n') });
  process.exit(0);
};

const first = await new Promise((resolve) => {
  const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
  lines.once('line', (line) => {
    lines.close();
    resolve(line);
  });
});
const source = String(JSON.parse(first).source ?? '');
let code;
try {
  code = stripTypeScriptTypes(source, { mode: 'transform' });
} catch (error) {
  reject(error?.message ?? error);
}
writeFileSync('/tmp/main.mjs', code);
const check = spawnSync(process.execPath, ['--check', '/tmp/main.mjs'], { encoding: 'utf8' });
if (check.status !== 0) reject(check.stderr);
// What the runner loads is `module.default ?? module.run`: any way of exporting one of those counts,
// and a mention in a comment does not.
const bare = code.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|[^:\\])\/\/.*$/gm, '$1');
const exported =
  /\bexport\s+default\b/.test(bare) ||
  /\bexport\s+(async\s+)?function\s+run\b/.test(bare) ||
  /\bexport\s+(const|let|var)\s+run\b/.test(bare) ||
  /\bexport\s*\{[^}]*\b(run|default)\b[^}]*\}/.test(bare);
if (!exported) {
  reject('the script must export a default function (export default async function (input, octopus) { ... })');
}
say({ op: 'built', entry: 'main.mjs', files: { 'main.mjs': code } });
