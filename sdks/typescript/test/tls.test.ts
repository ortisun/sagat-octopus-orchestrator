// TLS and mTLS to the REST API and to the worker service, with throw-away certificates made by `openssl`;
// and the promise that a credential is never sent anywhere the caller did not point the SDK at.
import { strict as assert } from 'node:assert';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { createServer as createHttp } from 'node:http';
import { createServer as createHttps, type Server } from 'node:https';
import type { AddressInfo } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import * as grpc from '@grpc/grpc-js';
import * as protoLoader from '@grpc/proto-loader';
import { SagatClient, SagatError, WorkerHost, type Logger } from '../src/index.js';

const here = dirname(fileURLToPath(import.meta.url));
const PROTO = join(here, '..', '..', 'proto', 'worker.proto');

let openssl = true;
try {
  execFileSync('openssl', ['version'], { stdio: 'ignore' });
} catch {
  openssl = false;
}
const needsOpenssl = { skip: openssl ? false : 'needs the openssl command' };

interface Pair {
  cert: Buffer;
  key: Buffer;
}
let directory = '';
let serverPair: Pair;
let otherPair: Pair;
let clientPair: Pair;

/** A self-signed certificate for localhost and 127.0.0.1 that is its own trust root. */
function makePair(name: string, commonName: string): Pair {
  const key = join(directory, `${name}.key`);
  const cert = join(directory, `${name}.pem`);
  const config = join(directory, `${name}.cnf`);
  writeFileSync(config, `[req]\ndistinguished_name=dn\nx509_extensions=ext\nprompt=no\n[dn]\nCN=${commonName}\n[ext]\nsubjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=CA:TRUE\n`);
  execFileSync('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '2', '-config', config, '-keyout', key, '-out', cert], { stdio: 'ignore' });
  return { cert: readFileSync(cert), key: readFileSync(key) };
}

before(() => {
  if (!openssl) return;
  directory = mkdtempSync(join(tmpdir(), 'sagat-tls-'));
  serverPair = makePair('server', 'localhost');
  otherPair = makePair('other', 'localhost');
  clientPair = makePair('client', 'the-client');
});
after(() => {
  if (directory) rmSync(directory, { recursive: true, force: true });
});

const RUN = {
  runId: '0190aaaa-0000-7000-8000-000000000001', workflowId: '0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee', versionId: '0190aaaa-0000-7000-8000-000000000002',
  status: 'RUNNING', version: 1, paused: false, startedBy: 'alice', createdAt: '2026-10-10T10:00:00Z', updatedAt: '2026-10-10T10:00:01Z',
  finishedAt: null, deadline: null, traceId: 'abcdefabcdefabcdefabcdefabcdefab', correlationId: null, depth: 0, parent: null,
};

async function httpsServer(options: { requireClientCert?: boolean } = {}): Promise<{ server: Server; url: string; peers: string[] }> {
  const peers: string[] = [];
  const server = createHttps(
    {
      cert: serverPair.cert,
      key: serverPair.key,
      ...(options.requireClientCert ? { requestCert: true, rejectUnauthorized: true, ca: [clientPair.cert] } : {}),
    },
    (req, res) => {
      const peer = (req.socket as unknown as { getPeerCertificate(): { subject?: { CN?: string } } }).getPeerCertificate();
      peers.push(peer.subject?.CN ?? '');
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify(RUN));
    },
  );
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  return { server, url: `https://127.0.0.1:${(server.address() as AddressInfo).port}/v1`, peers };
}

const client = (apiUrl: string, extra: Record<string, unknown> = {}): SagatClient =>
  new SagatClient({ apiUrl, namespace: 'payments', token: 'tok-1', retry: { maxAttempts: 1 }, ...extra });

test('https with a server certificate nobody trusts is a transport failure, never silently accepted', needsOpenssl, async () => {
  const { server, url, peers } = await httpsServer();
  try {
    await assert.rejects(client(url).get(RUN.runId), (error: unknown) => error instanceof SagatError && error.code === 'TRANSPORT');
    await assert.rejects(client(url, { tls: { ca: otherPair.cert } }).get(RUN.runId), (error: unknown) => error instanceof SagatError && error.code === 'TRANSPORT');
    assert.equal(peers.length, 0, 'no request reached the server');
  } finally {
    server.close();
  }
});

test('the configured trust root makes the same server work, and a client certificate is presented when configured', needsOpenssl, async () => {
  const open = await httpsServer();
  const mutual = await httpsServer({ requireClientCert: true });
  try {
    const run = await client(open.url, { tls: { ca: serverPair.cert } }).get(RUN.runId);
    assert.equal(run.runId, RUN.runId);
    // A server that requires a client certificate refuses a client without one, and sees the one given.
    await assert.rejects(client(mutual.url, { tls: { ca: serverPair.cert } }).get(RUN.runId), (error: unknown) => error instanceof SagatError && error.code === 'TRANSPORT');
    const run2 = await client(mutual.url, { tls: { ca: serverPair.cert, cert: clientPair.cert, key: clientPair.key } }).get(RUN.runId);
    assert.equal(run2.runId, RUN.runId);
    assert.deepEqual(mutual.peers, ['the-client']);
  } finally {
    open.server.close();
    mutual.server.close();
  }
});

test('a redirect is never followed: the credential would go to a place the caller did not name', async () => {
  const elsewhere: string[] = [];
  const target = createHttp((req, res) => {
    elsewhere.push(String(req.headers['authorization']));
    res.writeHead(200).end('{}');
  });
  await new Promise<void>((resolve) => target.listen(0, '127.0.0.1', resolve));
  const targetUrl = `http://127.0.0.1:${(target.address() as AddressInfo).port}/stolen`;
  const origin = createHttp((_req, res) => res.writeHead(302, { location: targetUrl }).end());
  await new Promise<void>((resolve) => origin.listen(0, '127.0.0.1', resolve));
  try {
    const url = `http://127.0.0.1:${(origin.address() as AddressInfo).port}/v1`;
    await assert.rejects(client(url).get(RUN.runId), (error: unknown) => error instanceof SagatError && error.code === 'PROTOCOL');
    assert.deepEqual(elsewhere, [], 'the redirect target was never called');
  } finally {
    origin.close();
    target.close();
  }
});

/** The smallest worker service: it registers workers and has nothing to give them. */
async function grpcServer(credentials: grpc.ServerCredentials): Promise<{ server: grpc.Server; url: string; registered: number }> {
  const definition = protoLoader.loadSync(PROTO, { keepCase: true, longs: Number, enums: String, defaults: true, oneofs: true });
  const loaded = grpc.loadPackageDefinition(definition) as unknown as { sagat_octopus_orchestrator: { v1: { WorkerService: grpc.ServiceClientConstructor } } };
  const state = { registered: 0 };
  const server = new grpc.Server();
  server.addService(loaded.sagat_octopus_orchestrator.v1.WorkerService.service, {
    registerWorker: (_call: unknown, callback: grpc.sendUnaryData<unknown>) => {
      state.registered++;
      callback(null, { worker_id: 'w-1', session_token: 's-1', accepted_protocol_version: '1.2' });
    },
    pollTasks: (_call: unknown, callback: grpc.sendUnaryData<unknown>) => callback(null, { tasks: [], retry_after_ms: 5000 }),
  } as unknown as grpc.UntypedServiceImplementation);
  const port = await new Promise<number>((resolve, reject) => server.bindAsync('127.0.0.1:0', credentials, (error, p) => (error ? reject(error) : resolve(p))));
  return Object.assign(state, { server, url: `https://127.0.0.1:${port}` });
}

function host(url: string, extra: Record<string, unknown> = {}): WorkerHost {
  const worker = new WorkerHost({ grpcUrl: url, namespace: 'payments', token: 'tok-1', workerName: 'tls-worker', retry: { maxAttempts: 1 }, ...extra });
  worker.registerActivity('charge', () => null);
  return worker;
}

test('the worker host verifies the server certificate too, takes a trust root, and presents a client certificate', needsOpenssl, async () => {
  const open = await grpcServer(grpc.ServerCredentials.createSsl(null, [{ private_key: serverPair.key, cert_chain: serverPair.cert }], false));
  const mutual = await grpcServer(grpc.ServerCredentials.createSsl(clientPair.cert, [{ private_key: serverPair.key, cert_chain: serverPair.cert }], true));
  try {
    // Nobody trusts it: a transport failure, and the server never saw a registration.
    const refused = host(open.url, { tls: { serverName: 'localhost' } });
    await assert.rejects(refused.start(), (error: unknown) => error instanceof SagatError && error.code === 'TRANSPORT');
    assert.equal(open.registered, 0);
    // The name to verify is checked, not skipped: a name the certificate was not issued for fails.
    await assert.rejects(host(open.url, { tls: { ca: serverPair.cert, serverName: 'wrong.example' } }).start(), (error: unknown) => error instanceof SagatError && error.code === 'TRANSPORT');
    assert.equal(open.registered, 0);

    const trusted = host(open.url, { tls: { ca: serverPair.cert, serverName: 'localhost' } });
    await trusted.start();
    assert.equal(trusted.protocolVersion, '1.2');
    await trusted.stop(200);

    // mTLS: without a client certificate the server refuses, with one it serves.
    await assert.rejects(host(mutual.url, { tls: { ca: serverPair.cert, serverName: 'localhost' } }).start(), (error: unknown) => error instanceof SagatError && error.code === 'TRANSPORT');
    assert.equal(mutual.registered, 0);
    const withCert = host(mutual.url, { tls: { ca: serverPair.cert, cert: clientPair.cert, key: clientPair.key, serverName: 'localhost' } });
    await withCert.start();
    assert.equal(mutual.registered, 1);
    await withCert.stop(200);
  } finally {
    open.server.forceShutdown();
    mutual.server.forceShutdown();
  }
});

test('gRPC over TLS to a bare IP address says what it needs instead of failing in the dark', () => {
  assert.throws(
    () => host('https://127.0.0.1:50051'),
    (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED' && /serverName/.test(error.message),
  );
  assert.doesNotThrow(() => host('https://127.0.0.1:50051', { tls: { serverName: 'octopus.internal' } }));
  assert.doesNotThrow(() => host('https://octopus.example.com:443'));
});

test('https draws no warning and a plain remote URL draws exactly one that names only the host', () => {
  const warnings: Array<{ message: string; fields?: Record<string, unknown> }> = [];
  const logger: Logger = { debug() {}, info() {}, warn: (message, fields) => void warnings.push({ message, fields }), error() {} };
  new SagatClient({ apiUrl: 'https://octopus.example.com/v1', namespace: 'payments', token: 'tok-1', logger });
  assert.equal(warnings.length, 0);
  new SagatClient({ apiUrl: 'http://user:pa55word@octopus.example.com/v1?token=abc', namespace: 'payments', token: 'tok-1', logger });
  assert.equal(warnings.length, 1);
  assert.deepEqual(warnings[0]?.fields, { host: 'octopus.example.com' });
  assert.ok(!JSON.stringify(warnings).includes('pa55word') && !JSON.stringify(warnings).includes('abc'));
});

test('nothing in the library turns certificate verification off', () => {
  const files: string[] = [];
  const walk = (directoryPath: string): void => {
    for (const name of readdirSync(directoryPath)) {
      const path = join(directoryPath, name);
      if (statSync(path).isDirectory()) walk(path);
      else if (path.endsWith('.ts')) files.push(path);
    }
  };
  walk(join(here, '..', '..', 'src'));
  assert.ok(files.length > 5);
  for (const file of files) {
    const text = readFileSync(file, 'utf8');
    for (const forbidden of ['rejectUnauthorized: false', 'NODE_TLS_REJECT_UNAUTHORIZED', 'checkServerIdentity', 'check_server_identity']) {
      assert.ok(!text.includes(forbidden), `${file} contains ${forbidden}`);
    }
  }
});
