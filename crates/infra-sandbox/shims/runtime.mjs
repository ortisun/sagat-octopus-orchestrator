// The supervisor of a TypeScript script inside its sandbox. It reads the input from the platform,
// runs the default export of the script and writes the result back. Whatever the script prints goes to
// standard error (the log); standard output is the platform's channel. The script has no network and
// no secrets: `octopus.fetch` and `octopus.secret` ask the platform, which may refuse.
import { mkdirSync, writeFileSync, writeSync } from 'node:fs';
import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';
import { inspect } from 'node:util';

const write = (message) =>
  new Promise((resolve) => process.stdout.write(JSON.stringify(message) + '\n', resolve));

// The log is written to the descriptor itself and not through `process.stderr`: that stream queues
// what the pipe will not take and drops what is still queued when the process exits, so a script
// that wrote faster than it was read lost its log and nothing slowed it down. A write to the pipe
// waits until the supervisor has read, which is the backpressure that lets the supervisor stop a
// script that floods its log, and every line written before the script ends is delivered.
const log = (...args) => {
  const data = Buffer.from(
    args.map((a) => (typeof a === 'string' ? a : inspect(a, { depth: 4 }))).join(' ') + '\n',
  );
  let at = 0;
  while (at < data.length) {
    try {
      at += writeSync(2, data, at);
    } catch (error) {
      // Not ready: wait a moment without spending CPU. Anything else (the reader is gone): drop it.
      if (error?.code !== 'EAGAIN') return;
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 1);
    }
  }
};
console.log = console.info = console.debug = console.warn = console.error = log;

let nextId = 1;
const waiting = new Map();
const call = (message) =>
  new Promise((resolve, reject) => {
    const id = nextId++;
    waiting.set(id, { resolve, reject });
    write({ ...message, id });
  });

const octopus = Object.freeze({
  fetch: async (url, init = {}) => {
    const reply = await call({
      op: 'fetch',
      url: String(url),
      method: init.method ?? 'GET',
      headers: init.headers ?? {},
      body: init.body == null ? null : String(init.body),
    });
    const text = reply.body ?? '';
    return Object.freeze({
      status: reply.status,
      headers: reply.headers ?? {},
      text: async () => text,
      json: async () => JSON.parse(text),
    });
  },
  secret: async (name) => (await call({ op: 'secret', name: String(name) })).value,
  log,
});

const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
lines.on('line', (line) => {
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    return;
  }
  if (message.op === 'run') {
    start(message.input, message.files ?? {}, message.idempotencyKey ?? null);
  } else if (message.id !== undefined) {
    const pending = waiting.get(message.id);
    if (pending) {
      waiting.delete(message.id);
      if (message.ok) pending.resolve(message);
      else pending.reject(new Error(message.error || 'refused'));
    }
  }
});

async function start(input, files, idempotencyKey) {
  let code = 0;
  try {
    // The code arrives on the channel and is written where the script can read it but not run it
    // as a program: the scratch area is mounted without the execute bit.
    mkdirSync('/tmp/script', { recursive: true });
    for (const [name, text] of Object.entries(files)) {
      if (/^[A-Za-z0-9_-][A-Za-z0-9._-]{0,63}$/.test(name)) {
        writeFileSync(`/tmp/script/${name}`, text);
      }
    }
    const module = await import(pathToFileURL('/tmp/script/main.mjs').href);
    const entry = module.default ?? module.run;
    if (typeof entry !== 'function') {
      throw new Error('the script must export a default function');
    }
    // The same on every attempt of the same step: put it in the request of an external effect so
    // the receiver can tell a repeat from a new request.
    const value = await entry(input, Object.freeze({ ...octopus, idempotencyKey }));
    await write({ op: 'result', value: value === undefined ? null : value });
  } catch (error) {
    code = 1;
    await write({
      op: 'error',
      message: String(error?.message ?? error),
      stack: String(error?.stack ?? ''),
    });
  }
  process.exit(code);
}

write({ op: 'ready' });
