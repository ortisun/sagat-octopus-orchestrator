import { strict as assert } from 'node:assert';
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import { after, before, test } from 'node:test';
import { SagatClient, SagatError, warnIfNotSecure, type Logger } from '../src/index.js';

interface Seen {
  method: string;
  url: string;
  headers: IncomingMessage['headers'];
  body: unknown;
}

const WORKFLOW = '0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee';
const run = (overrides: Record<string, unknown> = {}): Record<string, unknown> => ({
  runId: '0190aaaa-0000-7000-8000-000000000001', workflowId: WORKFLOW, versionId: '0190aaaa-0000-7000-8000-000000000002',
  status: 'RUNNING', version: 3, paused: false, startedBy: 'alice', createdAt: '2026-10-10T10:00:00Z', updatedAt: '2026-10-10T10:00:01Z',
  finishedAt: null, deadline: null, traceId: 'abcdefabcdefabcdefabcdefabcdefab', correlationId: null, depth: 0, parent: null, futureField: 7,
  ...overrides,
});

let server: Server;
let base = '';
let seen: Seen[] = [];
let handler: (req: Seen, res: ServerResponse) => void = () => undefined;

before(async () => {
  server = createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on('data', (c: Buffer) => chunks.push(c));
    req.on('end', () => {
      const text = Buffer.concat(chunks).toString('utf8');
      const entry: Seen = { method: req.method ?? '', url: req.url ?? '', headers: req.headers, body: text ? JSON.parse(text) : undefined };
      seen.push(entry);
      handler(entry, res);
    });
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  base = `http://127.0.0.1:${(server.address() as AddressInfo).port}/v1`;
});
after(() => server.close());

const reply = (res: ServerResponse, status: number, body: unknown, headers: Record<string, string> = {}): void => {
  res.writeHead(status, { 'content-type': 'application/json', 'x-trace-id': 'f'.repeat(32), ...headers });
  res.end(body === undefined ? '' : typeof body === 'string' ? body : JSON.stringify(body));
};

const client = (extra: Record<string, unknown> = {}): SagatClient =>
  new SagatClient({ apiUrl: base, namespace: 'payments', token: 'tok-1', retry: { initialDelayMs: 5, maxDelayMs: 20 }, ...extra });

function prepare(next: (req: Seen, res: ServerResponse) => void): void {
  seen = [];
  handler = next;
}

test('a start sends the headers and the body the API expects and returns a typed run that keeps the raw answer', async () => {
  prepare((_req, res) => reply(res, 202, run()));
  const result = await client().start(WORKFLOW, { orderId: 'o-1' }, {
    idempotencyKey: 'key-12345678', correlationId: 'corr-1', traceparent: '00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01',
    deadline: new Date('2030-01-01T00:00:00.000Z'),
  });
  const [request] = seen;
  assert.ok(request);
  assert.equal(request.method, 'POST');
  assert.equal(request.url, '/v1/runs');
  assert.equal(request.headers['authorization'], 'Bearer tok-1');
  assert.equal(request.headers['x-octopus-namespace'], 'payments');
  assert.equal(request.headers['idempotency-key'], 'key-12345678');
  assert.equal(request.headers['traceparent'], '00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01');
  assert.match(String(request.headers['user-agent']), /^sagat-sdk-typescript\/\d+\.\d+\.\d+/);
  assert.deepEqual(request.body, { workflowId: WORKFLOW, input: { orderId: 'o-1' }, correlationId: 'corr-1', deadline: '2030-01-01T00:00:00.000Z' });
  assert.equal(result.status, 'RUNNING');
  assert.equal(result.createdAt.toISOString(), '2026-10-10T10:00:00.000Z');
  assert.equal(result.raw['futureField'], 7, 'a field of a newer server is reachable');
});

test('a call without a key gets a fresh one per call and keeps it across its retries', async () => {
  let attempts = 0;
  prepare((_req, res) => (++attempts < 3 ? reply(res, 503, { code: 'UNAVAILABLE', message: 'busy', retryable: true, traceId: 'e'.repeat(32) }) : reply(res, 202, run())));
  await client().start(WORKFLOW, {});
  assert.equal(seen.length, 3);
  const keys = new Set(seen.map((s) => s.headers['idempotency-key']));
  assert.equal(keys.size, 1, 'the same key on every attempt');
  const [key] = keys;
  assert.match(String(key), /^[0-9a-f-]{36}$/);
  attempts = 0;
  seen = [];
  await client().start(WORKFLOW, {});
  assert.notEqual(seen[0]?.headers['idempotency-key'], key, 'another call, another key');
});

test('an approval decision uses its body key, returns a typed receipt and preserves server fields', async () => {
  const approvalId = '0190aaaa-0000-7000-8000-000000000099';
  prepare((_req, res) => reply(res, 200, {
    receiptId: '0190aaaa-0000-7000-8000-000000000100', approvalId, decisionKey: 'review-1',
    decision: 'APPROVE', reasonCode: 'POLICY_OK', actor: 'alice', decidedAt: '2026-10-15T10:00:00Z',
    outcomeSequence: 7, replayed: false, futureField: 'kept',
  }));
  const receipt = await client().decideApproval(approvalId, 'APPROVE', {
    decisionKey: 'review-1', reasonCode: 'POLICY_OK',
    traceparent: '00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01',
  });
  assert.equal(seen[0]?.url, `/v1/approvals/${approvalId}/decisions`);
  assert.deepEqual(seen[0]?.body, { decisionKey: 'review-1', decision: 'APPROVE', reasonCode: 'POLICY_OK' });
  assert.equal(seen[0]?.headers['idempotency-key'], undefined);
  assert.equal(receipt.decidedAt.toISOString(), '2026-10-15T10:00:00.000Z');
  assert.equal(receipt.outcomeSequence, 7);
  assert.equal(receipt.raw['futureField'], 'kept');
});

test('an approval decision surfaces authorization and idempotency failures and rejects malformed input locally', async () => {
  const approvalId = '0190aaaa-0000-7000-8000-000000000099';
  prepare((_req, res) => reply(res, 403, { code: 'FORBIDDEN', message: 'not eligible', retryable: false, details: { reason: 'NOT_ELIGIBLE' } }));
  await assert.rejects(client().decideApproval(approvalId, 'REJECT', { decisionKey: 'review-1' }), (error: unknown) =>
    error instanceof SagatError && error.status === 403 && error.reason === 'NOT_ELIGIBLE');
  prepare((_req, res) => reply(res, 409, { code: 'CONFLICT', message: 'key reused', retryable: false, details: { reason: 'IDEMPOTENCY_KEY_REUSED' } }));
  await assert.rejects(client().decideApproval(approvalId, 'APPROVE', { decisionKey: 'review-1' }), (error: unknown) =>
    error instanceof SagatError && error.status === 409 && error.reason === 'IDEMPOTENCY_KEY_REUSED');
  const sent = seen.length;
  await assert.rejects(client().decideApproval('../other' as string, 'APPROVE', { decisionKey: 'review-1' }), /approval id/);
  await assert.rejects(client().decideApproval(approvalId, 'APPROVE', { decisionKey: 'bad key' }), /decision key/);
  await assert.rejects(client().decideApproval(approvalId, 'APPROVE', { decisionKey: undefined as unknown as string }), /decision key/);
  assert.equal(seen.length, sent);
});

test('approval pages are typed and listAllApprovals follows the opaque cursor', async () => {
  const runId = '0190aaaa-0000-7000-8000-000000000020';
  const approvalId = '0190aaaa-0000-7000-8000-000000000021';
  prepare((request, res) => reply(res, 200, request.url.includes('cursor=next')
    ? { items: [{ approvalId, runId, nodeId: 'review', instancePath: [], status: 'APPROVED', deadline: '2026-10-16T12:00:00Z', settledAt: '2026-10-15T12:00:00Z' }], nextCursor: null }
    : { items: [{ approvalId, runId, nodeId: 'review', instancePath: [{ scopeNodeId: 'orders', iterationIndex: 2 }], status: 'PENDING', deadline: '2026-10-16T12:00:00Z', settledAt: null }], nextCursor: 'next' }));
  const sdk = client();
  const page = await sdk.listApprovals(runId, { limit: 1, cursor: 'from' });
  const all = [];
  for await (const item of sdk.listAllApprovals(runId, { limit: 1 })) all.push(item);
  assert.equal(page.items[0]?.instancePath[0]?.iterationIndex, 2);
  assert.equal(page.items[0]?.deadline.toISOString(), '2026-10-16T12:00:00.000Z');
  assert.deepEqual(all.map((item) => item.status), ['PENDING', 'APPROVED']);
  assert.ok(seen[0]?.url.includes('limit=1') && seen[0]?.url.includes('cursor=from'));
  assert.ok(seen[2]?.url.includes('cursor=next'));
});

test('declared query and update calls preserve typed revisions, result and update key', async () => {
  const runId = '0190aaaa-0000-7000-8000-000000000001';
  prepare((req, res) => {
    if (req.url.includes('/queries/')) {
      reply(res, 200, { runId, runVersion: 4, stateRevision: 1, result: { balance: 12 }, future: true });
    } else {
      reply(res, 200, {
        runId, operation: 'account.credit', updateKey: 'credit-1', status: 'APPLIED', result: { balance: 15 },
        beforeStateRevision: 1, afterStateRevision: 2, runVersion: 5, replayed: false, future: true,
      });
    }
  });
  const queried = await client().query<{ balance: number }>(runId, 'account.balance', { currency: 'BRL' });
  const updated = await client().update<{ balance: number }>(runId, 'account.credit', { amount: 3 }, {
    updateKey: 'credit-1', expectedStateRevision: 1,
  });
  assert.deepEqual(seen[0]?.body, { args: { currency: 'BRL' } });
  assert.deepEqual(seen[1]?.body, { updateKey: 'credit-1', args: { amount: 3 }, expectedStateRevision: 1 });
  assert.equal(seen[1]?.headers['idempotency-key'], undefined);
  assert.equal(queried.result.balance, 12);
  assert.equal(queried.stateRevision, 1);
  assert.equal(updated.result.balance, 15);
  assert.equal(updated.afterStateRevision, 2);
  assert.equal(updated.raw['future'], true);
});

test('declared updates keep their body key across retries and reject invalid input locally', async () => {
  const runId = '0190aaaa-0000-7000-8000-000000000001';
  let attempts = 0;
  prepare((_req, res) => {
    attempts++;
    if (attempts === 1) reply(res, 503, { code: 'UNAVAILABLE', message: 'busy', retryable: true });
    else reply(res, 200, {
      runId, operation: 'account.credit', updateKey: 'credit-1', status: 'APPLIED', result: 15,
      beforeStateRevision: 1, afterStateRevision: 2, runVersion: 5, replayed: false,
    });
  });
  await client().update(runId, 'account.credit', { amount: 3 }, { updateKey: 'credit-1' });
  assert.equal(seen.length, 2);
  assert.deepEqual(seen[0]?.body, seen[1]?.body);
  const sent = seen.length;
  await assert.rejects(client().query(runId, 'bad operation', {}), /operation name/);
  await assert.rejects(client().update(runId, 'account.credit', {}, { updateKey: 'bad key' }), /update key/);
  await assert.rejects(client().update(runId, 'account.credit', {}, { updateKey: 'credit-2', expectedStateRevision: -1 }), /expectedStateRevision/);
  assert.equal(seen.length, sent);
});

test('chain inspection and commands use namespace identity, expected head and durable keys', async () => {
  const namespaceId = '0190aaaa-0000-7000-8000-000000000010';
  const chainId = '0190aaaa-0000-7000-8000-000000000011';
  const head = '0190aaaa-0000-7000-8000-000000000012';
  prepare((request, res) => request.url.includes('/signals/')
    ? reply(res, 202, { deliveredTo: ['wait'] })
    : request.url.endsWith('/cancel') ? reply(res, 202, undefined) : reply(res, 200, {
      chainId, currentRunId: head, revision: 2, cancelRequested: false, finalOutcome: null,
      segments: [{ index: 0, runId: chainId, predecessorRunId: null, successorRunId: head }, { index: 1, runId: head, predecessorRunId: chainId, successorRunId: null }],
    }));
  const sdk = client();
  const inspection = await sdk.inspectChain(namespaceId, chainId);
  const signal = await sdk.signalChain(namespaceId, chainId, 'chain.release', { ok: true }, { expectedRunId: head, idempotencyKey: 'chain-signal-1' });
  await sdk.cancelChain(namespaceId, chainId, 'operator requested cancellation', { expectedRunId: head, idempotencyKey: 'chain-cancel-1' });
  assert.equal(inspection.segments[1]?.predecessorRunId, chainId);
  assert.deepEqual(signal.deliveredTo, ['wait']);
  assert.deepEqual(seen[1]?.body, { expectedRunId: head, payload: { ok: true } });
  assert.equal(seen[1]?.headers['idempotency-key'], 'chain-signal-1');
  assert.deepEqual(seen[2]?.body, { expectedRunId: head, reason: 'operator requested cancellation' });
});

test('chain query carries the optional expected head and returns the resolved run revisions', async () => {
  const namespaceId = '0190aaaa-0000-7000-8000-000000000010';
  const chainId = '0190aaaa-0000-7000-8000-000000000011';
  const head = '0190aaaa-0000-7000-8000-000000000012';
  prepare((_request, res) => reply(res, 200, {
    runId: head, runVersion: 7, stateRevision: 3, result: { amount: '19.99', occurredAt: '2026-10-15T15:34:56.789Z' },
  }));
  const result = await client().queryChain<{ amount: string; occurredAt: string }>(
    namespaceId, chainId, 'account.snapshot', {}, { expectedRunId: head },
  );
  assert.equal(seen[0]?.url, `/v1/namespaces/${namespaceId}/chains/${chainId}/queries/account.snapshot`);
  assert.deepEqual(seen[0]?.body, { args: {}, expectedRunId: head });
  assert.deepEqual(result.result, { amount: '19.99', occurredAt: '2026-10-15T15:34:56.789Z' });
  assert.equal(result.runId, head);
});

test('cancelling a declared update stops the call without retrying it', async () => {
  const runId = '0190aaaa-0000-7000-8000-000000000001';
  prepare((_request, response) => {
    response.writeHead(200, { 'content-type': 'application/json' });
    response.write('{"runId":');
  });
  const abort = new AbortController();
  const call = client({ timeoutMs: 5_000 }).update(runId, 'account.credit', { amount: '19.99' }, {
    updateKey: 'credit-1', signal: abort.signal,
  });
  while (seen.length === 0) await new Promise((resolve) => setTimeout(resolve, 1));
  await new Promise((resolve) => setTimeout(resolve, 5));
  abort.abort();
  await assert.rejects(call, (error: unknown) => error instanceof SagatError && error.code === 'CANCELLED');
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(seen.length, 1);
});

test('an event without an id gets one per call, kept across retries, so a repetition is a duplicate and not a second event', async () => {
  let attempts = 0;
  prepare((_req, res) => (++attempts < 2 ? reply(res, 503, 'upstream down') : reply(res, 202, { eventId: 'e', source: 'api', status: 'ACCEPTED' })));
  const accepted = await client().publishEvent({ type: 'order.created', data: { id: 1 } });
  assert.equal(accepted.status, 'ACCEPTED');
  assert.equal(seen.length, 2);
  const ids = seen.map((s) => (s.body as { eventId: string }).eventId);
  assert.equal(ids[0], ids[1]);
  assert.match(ids[0] ?? '', /^[0-9a-f-]{36}$/);
});

test('GET is retried on unavailability and on rate limiting, honouring Retry-After, and gives up after the attempts', async () => {
  let attempts = 0;
  prepare((_req, res) => {
    attempts++;
    if (attempts === 1) reply(res, 429, { code: 'RATE_LIMITED', message: 'slow', retryable: true, traceId: 'a'.repeat(32) }, { 'retry-after': '0' });
    else reply(res, 200, run());
  });
  const got = await client().get('0190aaaa-0000-7000-8000-000000000001');
  assert.equal(got.runId, '0190aaaa-0000-7000-8000-000000000001');
  assert.equal(seen.length, 2);

  prepare((_req, res) => reply(res, 503, '<html>bad gateway</html>'));
  await assert.rejects(client().get('0190aaaa-0000-7000-8000-000000000001'), (error: unknown) => {
    assert.ok(error instanceof SagatError);
    assert.equal(error.code, 'UNAVAILABLE');
    assert.equal(error.status, 503);
    return true;
  });
  assert.equal(seen.length, 4, 'four attempts in all');
});

test('a 4xx other than 429 is never retried and keeps what the server said', async () => {
  prepare((_req, res) => reply(res, 409, { code: 'CONFLICT', message: 'the idempotency key was used for another request', retryable: false, traceId: 'c'.repeat(32), details: { reason: 'IDEMPOTENCY_KEY_REUSED' } }));
  await assert.rejects(client().start(WORKFLOW, {}, { idempotencyKey: 'key-12345678' }), (error: unknown) => {
    assert.ok(error instanceof SagatError);
    assert.equal(error.code, 'CONFLICT');
    assert.equal(error.reason, 'IDEMPOTENCY_KEY_REUSED');
    assert.equal(error.traceId, 'c'.repeat(32));
    assert.equal(error.retryable, false);
    assert.ok(error.remediation.length > 0);
    return true;
  });
  assert.equal(seen.length, 1);
});

test('a refused token, a missing resource and a validation error are told apart', async () => {
  for (const [status, code] of [[401, 'UNAUTHENTICATED'], [403, 'FORBIDDEN'], [404, 'NOT_FOUND'], [422, 'VALIDATION_FAILED']] as const) {
    prepare((_req, res) => reply(res, status, { code, message: 'no', retryable: false, traceId: 'b'.repeat(32) }));
    await assert.rejects(client().get('0190aaaa-0000-7000-8000-000000000001'), (error: unknown) => error instanceof SagatError && error.code === code && error.status === status);
    assert.equal(seen.length, 1, String(status));
  }
});

test('the token is asked for before every request, so a rotated one is used at once', async () => {
  let n = 0;
  prepare((_req, res) => reply(res, 200, run()));
  const c = client({ token: () => `tok-${++n}` });
  await c.get('0190aaaa-0000-7000-8000-000000000001');
  await c.get('0190aaaa-0000-7000-8000-000000000001');
  assert.deepEqual(seen.map((s) => s.headers['authorization']), ['Bearer tok-1', 'Bearer tok-2']);
});

test('a name is resolved to the id of its workflow, through every page, and remembered', async () => {
  prepare((req, res) => {
    if (req.url.startsWith('/v1/workflows')) {
      const second = req.url.includes('cursor=c2');
      reply(res, 200, second ? { items: [{ id: WORKFLOW, name: 'order-flow' }], nextCursor: null } : { items: [{ id: '0190aaaa-0000-7000-8000-00000000ffff', name: 'other' }], nextCursor: 'c2' });
    } else reply(res, 202, run());
  });
  const c = client();
  await c.start('order-flow', {});
  await c.start('order-flow', {});
  const lookups = seen.filter((s) => s.url.startsWith('/v1/workflows'));
  assert.equal(lookups.length, 2, 'two pages, once');
  assert.equal((seen.find((s) => s.method === 'POST')?.body as { workflowId: string }).workflowId, WORKFLOW);
  prepare((_req, res) => reply(res, 200, { items: [], nextCursor: null }));
  await assert.rejects(client().start('nothing-here', {}), (error: unknown) => error instanceof SagatError && error.code === 'NOT_FOUND' && error.reason === 'WORKFLOW_NOT_FOUND');
});

test('a malformed traceparent or key is refused before anything is sent', async () => {
  prepare((_req, res) => reply(res, 202, run()));
  await assert.rejects(client().start(WORKFLOW, {}, { traceparent: 'not-a-trace' }), (e: unknown) => e instanceof SagatError && e.code === 'BAD_REQUEST');
  await assert.rejects(client().start(WORKFLOW, {}, { idempotencyKey: 'short' }), (e: unknown) => e instanceof SagatError && e.code === 'VALIDATION_FAILED');
  assert.equal(seen.length, 0);
});

test('the ambient trace context is used when the call has none', async () => {
  prepare((_req, res) => reply(res, 200, run()));
  await client({ traceContext: () => '00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01' }).get('0190aaaa-0000-7000-8000-000000000001');
  assert.equal(seen[0]?.headers['traceparent'], '00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01');
});

test('a list follows its cursor, a deadline given as a duration is computed once, and an unsafe integer is refused', async () => {
  prepare((req, res) => reply(res, 200, req.url.includes('cursor=n2') ? { items: [run({ runId: 'r-2' })], nextCursor: null } : { items: [run({ runId: 'r-1' })], nextCursor: 'n2' }));
  const ids: string[] = [];
  for await (const item of client().listAll({ status: 'RUNNING', limit: 1 })) ids.push(item.runId);
  assert.deepEqual(ids, ['r-1', 'r-2']);
  assert.ok(seen[0]?.url.includes('status=RUNNING') && seen[0]?.url.includes('limit=1'));

  prepare((_req, res) => reply(res, 202, run()));
  const before = Date.now();
  await client().start(WORKFLOW, {}, { deadlineInMs: 60_000 });
  const sent = Date.parse((seen[0]?.body as { deadline: string }).deadline);
  assert.ok(sent >= before + 60_000 && sent < before + 61_000);

  await assert.rejects(client().start(WORKFLOW, { big: 9007199254740993 }), (e: unknown) => e instanceof SagatError && e.code === 'VALIDATION_FAILED');
  await assert.rejects(client().start(WORKFLOW, { nan: Number.NaN }), (e: unknown) => e instanceof SagatError && e.code === 'VALIDATION_FAILED');
});

test('no answer at all is a retryable transport error, and a lost answer is retried only because the call carries a key', async () => {
  let calls = 0;
  const lossy: typeof fetch = async (input, init) => {
    calls++;
    if (calls === 1) {
      // The request reached the server (it is recorded) and the answer was lost.
      await fetch(input, init);
      throw new TypeError('fetch failed');
    }
    return fetch(input, init);
  };
  prepare((_req, res) => reply(res, 202, run()));
  const result = await client({ fetch: lossy }).start(WORKFLOW, {}, { idempotencyKey: 'key-12345678' });
  assert.equal(result.status, 'RUNNING');
  assert.equal(seen.length, 2);
  assert.equal(seen[0]?.headers['idempotency-key'], seen[1]?.headers['idempotency-key']);

  const dead = new SagatClient({ apiUrl: 'http://127.0.0.1:1/v1', namespace: 'p', token: 't', retry: { maxAttempts: 2, initialDelayMs: 1, maxDelayMs: 2 } });
  await assert.rejects(dead.get('0190aaaa-0000-7000-8000-000000000001'), (e: unknown) => e instanceof SagatError && e.code === 'TRANSPORT' && e.retryable && e.status === null);
});

test('cancel and signal are operations with a reason and a payload and the result is typed', async () => {
  prepare((req, res) => (req.url.endsWith('/operations') ? reply(res, 202, { operationId: 'op-1', accepted: true, runStatus: 'CANCELLING', runVersion: 9 }) : reply(res, 202, { deliveredTo: ['wait'] })));
  const cancelled = await client().cancel('0190aaaa-0000-7000-8000-000000000001', 'no longer needed', { expectedRunVersion: 3 });
  assert.deepEqual({ ...cancelled, raw: undefined }, { operationId: 'op-1', accepted: true, runStatus: 'CANCELLING', runVersion: 9, raw: undefined });
  assert.deepEqual(seen[0]?.body, { action: 'CANCEL', reason: 'no longer needed', expectedRunVersion: 3 });
  const signalled = await client().signal('0190aaaa-0000-7000-8000-000000000001', 'payment-confirmed', { paymentId: 'p-1' });
  assert.deepEqual(signalled.deliveredTo, ['wait']);
  assert.equal(seen[1]?.url, '/v1/runs/0190aaaa-0000-7000-8000-000000000001/signals/payment-confirmed');
  assert.deepEqual(seen[1]?.body, { paymentId: 'p-1' });
});

test('waiting for completion polls until the run is terminal', async () => {
  let polls = 0;
  prepare((_req, res) => reply(res, 200, run({ status: ++polls < 3 ? 'RUNNING' : 'SUCCEEDED' })));
  const done = await client().waitForCompletion('0190aaaa-0000-7000-8000-000000000001', { pollIntervalMs: 5, timeoutMs: 5000 });
  assert.equal(done.status, 'SUCCEEDED');
  assert.equal(polls, 3);
  prepare((_req, res) => reply(res, 200, run()));
  await assert.rejects(client().waitForCompletion('0190aaaa-0000-7000-8000-000000000001', { pollIntervalMs: 5, timeoutMs: 30 }), (e: unknown) => e instanceof SagatError && e.reason === 'WAIT_TIMEOUT');
});

test('a 4xx is not repeated whatever the body says about retrying, and a 5xx is only when the server says it may be', async () => {
  let attempts = 0;
  prepare((_req, res) => {
    attempts++;
    reply(res, 422, { code: 'VALIDATION_FAILED', message: 'bad', retryable: true, traceId: 'e'.repeat(32) });
  });
  await assert.rejects(client().get(run()['runId'] as string), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED');
  assert.equal(attempts, 1, 'a validation error that claims to be retryable is still not repeated');
});

test('a call the older server does not have is NOT_FOUND with the reason UNSUPPORTED_BY_SERVER, but a run that does not exist is not', async () => {
  const id = run()['runId'] as string;
  for (const answer of [
    (res: ServerResponse) => reply(res, 501, ''),
    (res: ServerResponse) => reply(res, 404, '<html>no route</html>', { 'content-type': 'text/html' }),
  ]) {
    prepare((_req, res) => answer(res));
    await assert.rejects(client().get(id), (error: unknown) => {
      assert.ok(error instanceof SagatError);
      assert.equal(error.code, 'NOT_FOUND');
      assert.equal(error.reason, 'UNSUPPORTED_BY_SERVER');
      return true;
    });
  }
  // The platform's own 404 carries its envelope: the run is not there, the call is known.
  prepare((_req, res) => reply(res, 404, { code: 'NOT_FOUND', message: 'no such run', retryable: false, traceId: 'e'.repeat(32) }));
  await assert.rejects(client().get(id), (error: unknown) => error instanceof SagatError && error.code === 'NOT_FOUND' && error.reason === null);
});

test('plain http outside loopback is accepted but warned about once; loopback and https are not', () => {
  const warnings: string[] = [];
  const logger: Logger = { debug() {}, info() {}, warn: (message) => void warnings.push(message), error() {} };
  warnIfNotSecure('http://octopus.example.com/v1', logger);
  warnIfNotSecure('https://octopus.example.com/v1', logger);
  for (const local of ['http://127.0.0.1:8080/v1', 'http://localhost:8080/v1', 'http://[::1]:8080/v1', 'http://127.1.2.3/v1']) warnIfNotSecure(local, logger);
  assert.equal(warnings.length, 1);
  assert.ok(!warnings[0]?.includes('octopus.example.com/v1?') && !warnings[0]?.includes('tok'), 'the warning names no credential');
  // The client says it when it is made.
  const seenWarnings: string[] = [];
  new SagatClient({ apiUrl: 'http://octopus.example.com/v1', namespace: 'payments', token: 'tok-1', logger: { ...logger, warn: (message) => void seenWarnings.push(message) } });
  assert.equal(seenWarnings.length, 1);
});

test('the retry rule is by code: a 408 is repeated, a 429 for a quota is not, and a 4xx that claims to be retryable is not believed', async () => {
  const id = '0190aaaa-0000-7000-8000-000000000001';
  let attempts = 0;
  prepare((_req, res) => (++attempts < 3 ? reply(res, 408, { code: 'UNAVAILABLE', message: 'timeout', retryable: true, traceId: 'a'.repeat(32) }) : reply(res, 200, run())));
  assert.equal((await client().get(id)).runId, id);
  assert.equal(attempts, 3);

  attempts = 0;
  prepare((_req, res) => {
    attempts++;
    reply(res, 429, { code: 'QUOTA_EXCEEDED', message: 'no room', retryable: true, traceId: 'a'.repeat(32) });
  });
  await assert.rejects(client().get(id), (error: unknown) => error instanceof SagatError && error.code === 'QUOTA_EXCEEDED');
  assert.equal(attempts, 1, 'a quota is not repeated, whatever the body says');
});

test('a Retry-After is honoured, and one that asks for more than a minute is cut to a minute', async () => {
  const id = '0190aaaa-0000-7000-8000-000000000001';
  let attempts = 0;
  prepare((_req, res) => (++attempts === 1 ? reply(res, 429, { code: 'RATE_LIMITED', message: 'slow', retryable: true, traceId: 'a'.repeat(32) }, { 'retry-after': '1' }) : reply(res, 200, run())));
  const began = Date.now();
  await client({ retry: { initialDelayMs: 1, maxDelayMs: 2 } }).get(id);
  assert.ok(Date.now() - began >= 900, `waited ${Date.now() - began} ms`);
  prepare((_req, res) => reply(res, 429, { code: 'RATE_LIMITED', message: 'slow', retryable: true, traceId: 'a'.repeat(32) }, { 'retry-after': '86400' }));
  await assert.rejects(client({ retry: { maxAttempts: 1 } }).get(id), (error: unknown) => error instanceof SagatError && error.retryAfterMs === 60_000);
});

test('start validates the key and the trace before it looks the workflow up by name', async () => {
  prepare((_req, res) => reply(res, 200, { items: [], nextCursor: null }));
  await assert.rejects(client().start('charge-card', {}, { idempotencyKey: 'short' }), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED');
  await assert.rejects(client().start('charge-card', {}, { traceparent: 'nonsense' }), (error: unknown) => error instanceof SagatError);
  assert.equal(seen.length, 0, 'nothing was asked of the server');
});

test('a token provider that throws is UNAUTHENTICATED with a reason, not the provider\'s own error', async () => {
  prepare((_req, res) => reply(res, 200, run()));
  const failing = client({ token: () => { throw new Error('the identity provider said: client_secret=hunter2'); } });
  await assert.rejects(failing.get('0190aaaa-0000-7000-8000-000000000001'), (error: unknown) => {
    assert.ok(error instanceof SagatError);
    assert.equal(error.code, 'UNAUTHENTICATED');
    assert.equal(error.reason, 'TOKEN_PROVIDER_FAILED');
    assert.ok(!error.message.includes('hunter2'));
    return true;
  });
  assert.equal(seen.length, 0);
});

test('a gRPC target keeps the port of the scheme when the URL has none', async () => {
  const { grpcTarget } = await import('../src/worker/grpc.js');
  assert.equal(grpcTarget(new URL('http://octopus.internal')), 'octopus.internal:80');
  assert.equal(grpcTarget(new URL('https://octopus.internal')), 'octopus.internal:443');
  assert.equal(grpcTarget(new URL('http://octopus.internal:50051')), 'octopus.internal:50051');
  assert.equal(grpcTarget(new URL('https://[::1]:50051')), '[::1]:50051');
});

test('the canonical text sorts integer-like keys as text, as the other SDKs do, and refuses an integer it cannot print exactly', async () => {
  const { workflow } = await import('../src/index.js');
  const text = workflow('canon-check').step('a', 'x.y').inputSchema({ type: 'object', properties: { '10': {}, '9': {}, a: {}, '\u{1F600}': {}, '\uFF5E': {} } }).toJson();
  const keys = [...text.matchAll(/"(10|9|a|\u{1F600}|\uFF5E)":\{\}/gu)].map((m) => m[1]);
  assert.deepEqual(keys, ['10', '9', 'a', '\u{1F600}', '\uFF5E'], 'UTF-16 code unit order: "10" < "9" < "a" < U+1F600 (a surrogate pair) < U+FF5E');
  const big = () => workflow('canon-big').step('a', 'x.y').inputSchema({ type: 'integer', maximum: 2 ** 60 }).toJson();
  assert.throws(big, (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED');
  // 1e21 is written with an exponent by ECMAScript: it is a number, not an integer that was rounded.
  assert.match(workflow('canon-exp').step('a', 'x.y').inputSchema({ maximum: 1e21, minimum: -1e-7 }).toJson(), /"maximum":1e\+21/);
});

test('a call the server does not have is UNSUPPORTED_BY_SERVER by the reasons the server gives, and a missing run is not', async () => {
  const id = '0190aaaa-0000-7000-8000-000000000001';
  prepare((_req, res) => reply(res, 404, { code: 'NOT_FOUND', message: 'resource not found', retryable: false, traceId: 'a'.repeat(32), details: { reason: 'ROUTE_NOT_FOUND' } }));
  await assert.rejects(client().get(id), (error: unknown) => error instanceof SagatError && error.code === 'NOT_FOUND' && error.reason === 'UNSUPPORTED_BY_SERVER');
  prepare((_req, res) => reply(res, 405, { code: 'BAD_REQUEST', message: 'method not allowed', retryable: false, traceId: 'a'.repeat(32), details: { reason: 'METHOD_NOT_ALLOWED' } }));
  await assert.rejects(client().get(id), (error: unknown) => error instanceof SagatError && error.code === 'NOT_FOUND' && error.reason === 'UNSUPPORTED_BY_SERVER');
  prepare((_req, res) => reply(res, 405, { code: 'BAD_REQUEST', message: 'no', retryable: false, traceId: 'a'.repeat(32) }));
  await assert.rejects(client().get(id), (error: unknown) => error instanceof SagatError && error.code === 'BAD_REQUEST' && error.reason === null);
});

test('a 408 with no body at all (a proxy) is UNAVAILABLE by the shared catalog and is repeated', async () => {
  const id = '0190aaaa-0000-7000-8000-000000000001';
  let attempts = 0;
  prepare((_req, res) => (++attempts < 3 ? res.writeHead(408).end() : reply(res, 200, run())));
  assert.equal((await client().get(id)).runId, id);
  assert.equal(attempts, 3);
});

test('a number that is a whole number beyond 2^53 - 1 and is printed in digits is refused in the canonical text, one printed with an exponent is not', async () => {
  const { workflow } = await import('../src/index.js');
  const text = (value: number): string => workflow('canon-num').step('a', 'x.y').inputSchema({ maximum: value }).toJson();
  for (const refused of [2 ** 53, 2 ** 60, 1e20, 2.5e19]) {
    assert.throws(() => text(refused), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED', String(refused));
  }
  assert.match(text(1e21), /"maximum":1e\+21/);
  assert.match(text(9007199254740991), /"maximum":9007199254740991/);
  assert.match(text(1.5), /"maximum":1.5/);
});
