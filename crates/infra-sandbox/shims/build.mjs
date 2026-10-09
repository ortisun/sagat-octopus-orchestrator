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
if (!/export\s+default|export\s+(async\s+)?function\s+run/.test(code)) {
  reject('the script must export a default function (export default async function (input, octopus) { ... })');
}
say({ op: 'built', entry: 'main.mjs', files: { 'main.mjs': code } });
