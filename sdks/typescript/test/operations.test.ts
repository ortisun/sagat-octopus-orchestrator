// The client surface of feature 018 against a recording server: what each call sends, what it keeps across a retry,
// what it refuses before sending, and how it reads an answer that is newer than the SDK.
import { strict as assert } from 'node:assert';
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import { after, before, test } from 'node:test';
import { inspect } from 'node:util';
import { actionJson, descriptionJson, scheduleJson } from '../conformance/observe.js';
import { SagatClient, SagatError, type Logger, type Schedule, type ScheduleWrite } from '../src/index.js';

interface Seen {
  method: string;
  url: string;
  headers: IncomingMessage['headers'];
  body: unknown;
}

const NS = '0190aaaa-0000-7000-8000-00000000aaaa';
const WF = '0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee';
const SCHEDULE = '0190aaaa-0000-7000-8000-0000000000a1';
const RUN = '0190aaaa-0000-7000-8000-000000000001';
const BATCH = '0190aaaa-0000-7000-8000-0000000000b1';

const run = (overrides: Record<string, unknown> = {}): Record<string, unknown> => ({
  runId: RUN, workflowId: WF, versionId: '0190aaaa-0000-7000-8000-000000000002', status: 'RUNNING', version: 3, paused: false,
  startedBy: 'alice', createdAt: '2026-10-10T10:00:00Z', updatedAt: '2026-10-10T10:00:01Z', finishedAt: null, deadline: null,
  traceId: 'abcdefabcdefabcdefabcdefabcdefab', correlationId: null, depth: 0, parent: null, ...overrides,
});

const schedule = (overrides: Record<string, unknown> = {}): Record<string, unknown> => ({
  scheduleId: SCHEDULE, name: 'nightly', origin: 'API', state: 'ACTIVE', revision: 2, createdAt: '2026-10-10T10:00:00Z',
  action: { workflowId: WF, versionPolicy: 'ACTIVE', inputDigest: 'a'.repeat(64) }, note: null, remainingActions: null,
  nextActionAt: '2099-01-01T09:00:00Z', updatedAt: '2026-10-10T10:00:00Z', ...overrides,
});

const action = (overrides: Record<string, unknown> = {}): Record<string, unknown> => ({
  nominalTime: '2026-10-10T10:00:00Z', actualTime: '2026-10-10T10:00:01Z', kind: 'TRIGGERED', origin: 'MANUAL', revision: 2,
  outcome: 'STARTED', reason: null, runId: RUN, backfillId: null, decidedAt: '2026-10-10T10:00:01Z', ...overrides,
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

const reply = (res: ServerResponse, status: number, body: unknown): void => {
  res.writeHead(status, { 'content-type': 'application/json', 'x-trace-id': 'f'.repeat(32) });
  res.end(body === undefined ? '' : JSON.stringify(body));
};

const client = (): SagatClient =>
  new SagatClient({ apiUrl: base, namespace: 'payments', token: 'tok-1', retry: { initialDelayMs: 5, maxDelayMs: 20 } });

function prepare(next: (req: Seen, res: ServerResponse) => void): void {
  seen = [];
  handler = next;
}

const only = (): Seen => {
  assert.equal(seen.length, 1, 'exactly one request');
  return seen[0] as Seen;
};

// ---------------------------------------------------------------------------------------------- start options

test('a start sends the business identifier, its policies and the delay once, and says whether it created the run', async () => {
  prepare((_req, res) => reply(res, 202, run({ businessId: 'order-1', startDelayedUntil: '2026-10-10T10:00:05Z' })));
  const started = await client().start(WF, { a: 1 }, {
    idempotencyKey: 'key-12345678', businessId: 'Order/1 ', businessIdConflictPolicy: 'USE_EXISTING',
    businessIdReusePolicy: 'REJECT_DUPLICATE', startDelayMs: 5000.9,
  });
  assert.deepEqual(only().body, {
    workflowId: WF, input: { a: 1 }, businessId: 'Order/1 ', businessIdConflictPolicy: 'USE_EXISTING',
    businessIdReusePolicy: 'REJECT_DUPLICATE', startDelayMs: 5000,
  });
  assert.equal(started.started, true);
  assert.equal(started.businessId, 'order-1');
  assert.equal(started.startDelayedUntil?.toISOString(), '2026-10-10T10:00:05.000Z');
  prepare((_req, res) => reply(res, 202, run()));
  const plain = await client().start(WF, {}, { idempotencyKey: 'key-12345678' });
  assert.equal(plain.businessId, null);
  assert.equal(plain.startDelayedUntil, null);
});

// ---------------------------------------------------------------------------------------------- where a run came from

test('a run says which schedule started it, by which kind of action, for which nominal time and under which revision, and a run no schedule started has none', async () => {
  const body = run({
    startedBy: `system:schedule:${SCHEDULE}`,
    schedule: { scheduleId: SCHEDULE, nominalTime: '2026-11-04T02:00:00.123456Z', kind: 'BACKFILL', revision: 3 },
  });
  prepare((_req, res) => reply(res, 200, body));
  const scheduled = await client().get(RUN);
  assert.deepEqual(scheduled.schedule, {
    scheduleId: SCHEDULE, nominalTime: new Date('2026-11-04T02:00:00.123Z'), kind: 'BACKFILL', revision: 3,
  });
  assert.equal(scheduled.schedule?.nominalTime.toISOString(), '2026-11-04T02:00:00.123Z', 'an instant is a Date, to the millisecond');
  assert.deepEqual((scheduled.raw['schedule'] as Record<string, unknown>)['nominalTime'], '2026-11-04T02:00:00.123456Z', 'the exact text stays in `raw`');
  assert.equal(scheduled.resetFrom, null);

  // a kind this SDK does not know is kept as the server sent it, and a plain run has neither field
  prepare((_req, res) => reply(res, 200, run({ schedule: { scheduleId: SCHEDULE, nominalTime: '2026-11-04T02:00:00Z', kind: 'SOMETHING_NEW', revision: 1 } })));
  assert.equal((await client().get(RUN)).schedule?.kind, 'SOMETHING_NEW');
  prepare((_req, res) => reply(res, 200, run()));
  const plain = await client().get(RUN);
  assert.equal(plain.schedule, null);
  assert.equal(plain.resetFrom, null);
});

test('a run a reset made says the run it came from and the journal sequence it keeps', async () => {
  const source = '0190aaaa-0000-7000-8000-0000000000aa';
  prepare((_req, res) => reply(res, 200, run({ rerunOf: source, resetFrom: { runId: source, sequence: 4 } })));
  const reset = await client().get(RUN);
  assert.deepEqual(reset.resetFrom, { runId: source, sequence: 4 });
  assert.equal(reset.rerunOf, source);
  assert.equal(reset.schedule, null);
  // started again, not reset: `rerunOf` and nothing else
  prepare((_req, res) => reply(res, 200, run({ rerunOf: source })));
  const again = await client().get(RUN);
  assert.equal(again.rerunOf, source);
  assert.equal(again.resetFrom, null);
});

test('an answer whose schedule or reset is not what the contract says is a PROTOCOL error, not a run with half of it', async () => {
  for (const broken of [
    { schedule: { scheduleId: SCHEDULE, nominalTime: 'yesterday', kind: 'SCHEDULED', revision: 1 } },
    { schedule: { scheduleId: SCHEDULE, nominalTime: '2026-11-04T02:00:00Z', kind: 'SCHEDULED', revision: 0 } },
    { schedule: { nominalTime: '2026-11-04T02:00:00Z', kind: 'SCHEDULED', revision: 1 } },
    { schedule: 'nightly' },
    { resetFrom: { runId: RUN } },
    { resetFrom: { runId: RUN, sequence: 0 } },
  ]) {
    prepare((_req, res) => reply(res, 200, run(broken)));
    await assert.rejects(client().get(RUN), (error: unknown) => error instanceof SagatError && error.code === 'PROTOCOL', JSON.stringify(broken));
  }
});

test('the 200 of USE_EXISTING is a run that was not started here, and a start without the new options sends none of them', async () => {
  prepare((_req, res) => reply(res, 200, run()));
  const existing = await client().start(WF, {}, { idempotencyKey: 'key-12345678', businessId: 'o-1', businessIdConflictPolicy: 'USE_EXISTING' });
  assert.equal(existing.started, false);
  prepare((_req, res) => reply(res, 202, run()));
  await client().start(WF, {}, { idempotencyKey: 'key-12345678' });
  assert.deepEqual(only().body, { workflowId: WF, input: {} }, 'an old call is byte for byte what it was');
});

test('a start delay that is not a number is refused before anything is sent, and a retry keeps the one delay it computed', async () => {
  prepare((_req, res) => reply(res, 202, run()));
  await assert.rejects(client().start(WF, {}, { startDelayMs: Number.NaN }), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED');
  assert.equal(seen.length, 0);
  let attempts = 0;
  prepare((_req, res) => (++attempts < 2 ? reply(res, 503, { code: 'UNAVAILABLE', message: 'busy', retryable: true, traceId: 'e'.repeat(32) }) : reply(res, 202, run())));
  await client().start(WF, {}, { businessId: 'o-2', startDelayMs: 1500 });
  assert.equal(seen.length, 2);
  assert.deepEqual((seen[0] as Seen).body, (seen[1] as Seen).body, 'the retry is the same body');
  assert.equal((seen[0] as Seen).headers['idempotency-key'], (seen[1] as Seen).headers['idempotency-key']);
});

// ---------------------------------------------------------------------------------------------- schedules

test('create sends the schedule with its instants in UTC and one key, and reads the schedule it gets back', async () => {
  prepare((_req, res) => reply(res, 201, schedule({ futureField: 1 })));
  const created = await client().schedules.create(NS, {
    name: 'nightly',
    spec: { calendars: [{ hour: [{ start: 9 }], year: [{ start: 2099 }] }], startAt: '2099-01-01T01:00:00+01:00', timezone: 'UTC' },
    policies: { overlap: 'SKIP' },
    action: { workflowId: WF, input: { n: 1 } },
  }, { idempotencyKey: 'sched-key-1' });
  const request = only();
  assert.equal(request.method, 'POST');
  assert.equal(request.url, `/v1/namespaces/${NS}/schedules`);
  assert.equal(request.headers['idempotency-key'], 'sched-key-1');
  assert.deepEqual(request.body, {
    name: 'nightly',
    spec: { calendars: [{ hour: [{ start: 9 }], year: [{ start: 2099 }] }], startAt: '2099-01-01T00:00:00.000Z', timezone: 'UTC' },
    policies: { overlap: 'SKIP' },
    action: { workflowId: WF, input: { n: 1 } },
  });
  assert.equal(created.revision, 2);
  assert.equal(created.nextActionAt?.toISOString(), '2099-01-01T09:00:00.000Z');
  assert.equal(created.action.inputDigest, 'a'.repeat(64), 'the input is never returned, only its digest');
  assert.equal(created.raw['futureField'], 1);
});

test('describe asks for the counts it was given, and keeps an outcome it does not know as the string it came as', async () => {
  prepare((_req, res) => reply(res, 200, {
    ...schedule(),
    nextActions: [{ nominalTime: '2099-01-01T09:00:00Z', actualTime: '2099-01-01T09:00:00Z' }],
    recentActions: [action(), action({ outcome: 'SOMETHING_NEWER' })],
    openRuns: [RUN], bufferedCount: 1, counters: { STARTED: 1, SOMETHING_NEWER: 1 },
  }));
  const described = await client().schedules.get(NS, SCHEDULE, { nextCount: 3, recentCount: 5 });
  const request = only();
  assert.equal(request.method, 'GET');
  assert.equal(request.url, `/v1/namespaces/${NS}/schedules/${SCHEDULE}?nextCount=3&recentCount=5`);
  assert.equal(described.nextActions[0]?.nominalTime.getTime(), Date.parse('2099-01-01T09:00:00Z'));
  assert.equal(described.recentActions[1]?.outcome, 'SOMETHING_NEWER');
  assert.deepEqual(described.openRuns, [RUN]);
  assert.deepEqual(described.counters, { STARTED: 1, SOMETHING_NEWER: 1 });
  assert.equal(described.bufferedCount, 1);
});

test('list sends the filters it was given and listAll follows the opaque cursors to the end', async () => {
  let page = 0;
  prepare((_req, res) => reply(res, 200, ++page === 1
    ? { items: [schedule({ name: 'a-1' })], nextCursor: 'c1' }
    : { items: [schedule({ name: 'a-2' })], nextCursor: null }));
  const names: string[] = [];
  for await (const item of client().schedules.listAll(NS, { state: 'PAUSED', origin: 'API', name: 'a-', limit: 1 })) names.push(item.name);
  assert.deepEqual(names, ['a-1', 'a-2']);
  assert.equal((seen[0] as Seen).url, `/v1/namespaces/${NS}/schedules?limit=1&state=PAUSED&origin=API&name=a-`);
  assert.equal((seen[1] as Seen).url, `/v1/namespaces/${NS}/schedules?cursor=c1&limit=1&state=PAUSED&origin=API&name=a-`);
});

test('update carries the revision it expects in the body, delete in the query, and both carry a key', async () => {
  prepare((_req, res) => reply(res, 200, schedule({ revision: 3 })));
  const write = { name: 'nightly', spec: { cron: ['0 9 * * *'] }, action: { workflowId: WF, input: {} } };
  const updated = await client().schedules.update(NS, SCHEDULE, write, 2, { idempotencyKey: 'upd-key-0001' });
  assert.equal(updated.revision, 3);
  assert.equal(only().method, 'PUT');
  assert.equal(only().headers['idempotency-key'], 'upd-key-0001');
  assert.deepEqual(only().body, { ...write, expectedRevision: 2 });
  prepare((_req, res) => reply(res, 204, undefined));
  await client().schedules.delete(NS, SCHEDULE, 3, { idempotencyKey: 'del-key-0001' });
  assert.equal(only().method, 'DELETE');
  assert.equal(only().url, `/v1/namespaces/${NS}/schedules/${SCHEDULE}?expectedRevision=3`);
  assert.equal(only().headers['idempotency-key'], 'del-key-0001');
});

test('a stale revision is a conflict with its reason, and a revision that cannot be one is refused before it is sent', async () => {
  prepare((_req, res) => reply(res, 409, { code: 'CONFLICT', message: 'stale', retryable: false, traceId: 'e'.repeat(32), details: { reason: 'REVISION_CONFLICT', currentRevision: 4 } }));
  await assert.rejects(
    client().schedules.update(NS, SCHEDULE, { name: 'n', spec: { cron: ['0 9 * * *'] }, action: { workflowId: WF, input: {} } }, 2),
    (error: unknown) => error instanceof SagatError && error.code === 'CONFLICT' && error.reason === 'REVISION_CONFLICT',
  );
  assert.equal(seen.length, 1, 'a conflict is never retried');
  seen = [];
  await assert.rejects(client().schedules.delete(NS, SCHEDULE, 0), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED');
  await assert.rejects(client().schedules.get(NS, '../x'), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED');
  await assert.rejects(client().schedules.get('payments', SCHEDULE), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED');
  assert.equal(seen.length, 0);
});

test('pause and resume send the note, trigger sends the overlap for this action only, and a trigger retry keeps its key', async () => {
  prepare((_req, res) => reply(res, 200, schedule({ state: 'PAUSED', note: 'maintenance' })));
  const paused = await client().schedules.pause(NS, SCHEDULE, 'maintenance', { idempotencyKey: 'pause-key-01' });
  assert.equal(paused.state, 'PAUSED');
  assert.equal(only().url, `/v1/namespaces/${NS}/schedules/${SCHEDULE}/pause`);
  assert.deepEqual(only().body, { note: 'maintenance' });
  prepare((_req, res) => reply(res, 200, schedule()));
  await client().schedules.resume(NS, SCHEDULE, 'done', { idempotencyKey: 'resume-key-1' });
  assert.equal(only().url, `/v1/namespaces/${NS}/schedules/${SCHEDULE}/resume`);
  await assert.rejects(client().schedules.pause(NS, SCHEDULE, ''), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED');

  let attempts = 0;
  prepare((_req, res) => (++attempts < 2 ? reply(res, 503, { code: 'UNAVAILABLE', message: 'busy', retryable: true, traceId: 'e'.repeat(32) }) : reply(res, 202, action({ outcome: 'SKIPPED_OVERLAP' }))));
  const triggered = await client().schedules.trigger(NS, SCHEDULE, { overlap: 'SKIP' });
  assert.equal(triggered.outcome, 'SKIPPED_OVERLAP');
  assert.equal(seen.length, 2);
  assert.equal((seen[0] as Seen).headers['idempotency-key'], (seen[1] as Seen).headers['idempotency-key']);
  assert.deepEqual((seen[1] as Seen).body, { overlap: 'SKIP' });
});

test('a backfill sends its range as UTC instants, a cancel names the backfill, and the history can be filtered by outcome', async () => {
  prepare((_req, res) => reply(res, 202, {
    backfillId: BATCH, start: '2026-10-10T08:00:00Z', end: '2026-10-10T10:00:00Z', cursor: null, state: 'PENDING', overlap: 'ALLOW_ALL', counts: { STARTED: 2 },
  }));
  const backfill = await client().schedules.backfill(NS, SCHEDULE, new Date('2026-10-10T08:00:00Z'), '2026-10-10T10:00:00+00:00', { overlap: 'ALLOW_ALL', idempotencyKey: 'back-key-001' });
  assert.deepEqual(only().body, { start: '2026-10-10T08:00:00.000Z', end: '2026-10-10T10:00:00.000Z', overlap: 'ALLOW_ALL' });
  assert.equal(backfill.cursor, null);
  assert.deepEqual(backfill.counts, { STARTED: 2 });
  await assert.rejects(client().schedules.backfill(NS, SCHEDULE, 'not a time', new Date()), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED');
  prepare((_req, res) => reply(res, 200, { backfillId: BATCH, start: '2026-10-10T08:00:00Z', end: '2026-10-10T10:00:00Z', state: 'CANCELLED', overlap: 'ALLOW_ALL', counts: {} }));
  const cancelled = await client().schedules.cancelBackfill(NS, SCHEDULE, BATCH);
  assert.equal(cancelled.state, 'CANCELLED');
  assert.equal(only().url, `/v1/namespaces/${NS}/schedules/${SCHEDULE}/backfills/${BATCH}/cancel`);
  prepare((_req, res) => reply(res, 200, { items: [action({ outcome: 'ALREADY_ACTED' })], nextCursor: null }));
  const actions = await client().schedules.listActions(NS, SCHEDULE, { outcome: 'ALREADY_ACTED', limit: 10 });
  assert.equal(only().url, `/v1/namespaces/${NS}/schedules/${SCHEDULE}/actions?limit=10&outcome=ALREADY_ACTED`);
  assert.equal(actions.items[0]?.outcome, 'ALREADY_ACTED');
});

// ----------------------------------------------------------------------- the business-id prefix of a schedule's action (FR-018-016)

// Customer text: spaces and upper case are sent as they are, and the SDK has no opinion on the pattern or the length.
const PREFIX = ' Order/Mixed-Case ';
const SERVED = { workflowId: WF, versionPolicy: 'ACTIVE', inputDigest: 'a'.repeat(64) };
const actionOf = (request: Seen): Record<string, unknown> => (request.body as { action: Record<string, unknown> }).action;
const writeWith = (extra: Partial<ScheduleWrite['action']> = {}): ScheduleWrite =>
  ({ name: 'nightly', spec: { cron: ['0 9 * * *'] }, action: { workflowId: WF, input: { n: 1 }, ...extra } });

test('a schedule action sends its business-id prefix and both policies as given, and reads back what the server kept', async () => {
  const kept = { ...SERVED, businessIdPrefix: PREFIX, businessIdConflictPolicy: 'USE_EXISTING', businessIdReusePolicy: 'REJECT_DUPLICATE' };
  prepare((_req, res) => reply(res, 201, schedule({ action: kept })));
  const created = await client().schedules.create(NS, writeWith({
    businessIdPrefix: PREFIX, businessIdConflictPolicy: 'USE_EXISTING', businessIdReusePolicy: 'REJECT_DUPLICATE',
  }), { idempotencyKey: 'sched-key-bid' });
  assert.deepEqual(actionOf(only()), {
    workflowId: WF, input: { n: 1 }, businessIdPrefix: PREFIX, businessIdConflictPolicy: 'USE_EXISTING', businessIdReusePolicy: 'REJECT_DUPLICATE',
  });
  assert.equal(created.action.businessIdPrefix, PREFIX);
  assert.equal(created.action.businessIdConflictPolicy, 'USE_EXISTING');
  assert.equal(created.action.businessIdReusePolicy, 'REJECT_DUPLICATE');
  assert.equal((created.raw['action'] as Record<string, unknown>)['businessIdPrefix'], PREFIX, 'the answer as the server sent it stays in `raw`');
});

test('a prefix alone is sent alone, the empty prefix is sent as the empty string, and the server writes the defaults back', async () => {
  prepare((_req, res) => reply(res, 201, schedule({ action: { ...SERVED, businessIdPrefix: 'order', businessIdConflictPolicy: 'FAIL', businessIdReusePolicy: 'ALLOW_DUPLICATE' } })));
  const created = await client().schedules.create(NS, writeWith({ businessIdPrefix: 'order' }));
  assert.deepEqual(actionOf(only()), { workflowId: WF, input: { n: 1 }, businessIdPrefix: 'order' }, 'no default is invented locally');
  assert.deepEqual([created.action.businessIdPrefix, created.action.businessIdConflictPolicy, created.action.businessIdReusePolicy], ['order', 'FAIL', 'ALLOW_DUPLICATE']);

  // The server refuses it (VALIDATION_FAILED): the SDK does not judge it.
  prepare((_req, res) => reply(res, 400, { code: 'VALIDATION_FAILED', message: 'the schedule is not valid', retryable: false, traceId: 'e'.repeat(32), details: { reason: 'SCHEDULE_SPEC_INVALID', field: '/action/businessIdPrefix' } }));
  await assert.rejects(
    client().schedules.create(NS, writeWith({ businessIdPrefix: '' })),
    (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED' && error.reason === 'SCHEDULE_SPEC_INVALID',
  );
  assert.equal(actionOf(only())['businessIdPrefix'], '', 'an empty prefix is sent as the empty string, not left out');
});

test('a schedule action without a prefix sends none of the three keys, and a server action without one reads as none', async () => {
  prepare((_req, res) => reply(res, 201, schedule()));
  const created = await client().schedules.create(NS, writeWith());
  assert.deepEqual(Object.keys(actionOf(only())).sort(), ['input', 'workflowId']);
  assert.deepEqual([created.action.businessIdPrefix, created.action.businessIdConflictPolicy, created.action.businessIdReusePolicy], [null, null, null]);

  // Keys that are set to nothing are not keys: not null, not undefined, not empty.
  prepare((_req, res) => reply(res, 201, schedule()));
  await client().schedules.create(NS, writeWith({
    businessIdPrefix: undefined, businessIdConflictPolicy: null as unknown as undefined, businessIdReusePolicy: undefined,
  }));
  assert.deepEqual(Object.keys(actionOf(only())).sort(), ['input', 'workflowId']);
  assert.ok(!JSON.stringify(only().body).includes('businessId'));
});

test('an update takes the same action document: a schedule updated without a prefix has none', async () => {
  prepare((_req, res) => reply(res, 200, schedule({ revision: 3, action: { ...SERVED, businessIdPrefix: 'nightly', businessIdConflictPolicy: 'FAIL', businessIdReusePolicy: 'ALLOW_DUPLICATE_FAILED_ONLY' } })));
  const withPrefix = await client().schedules.update(NS, SCHEDULE, writeWith({ businessIdPrefix: 'nightly', businessIdReusePolicy: 'ALLOW_DUPLICATE_FAILED_ONLY' }), 2, { idempotencyKey: 'upd-key-bid1' });
  assert.equal(only().method, 'PUT');
  assert.deepEqual(actionOf(only()), { workflowId: WF, input: { n: 1 }, businessIdPrefix: 'nightly', businessIdReusePolicy: 'ALLOW_DUPLICATE_FAILED_ONLY' });
  assert.equal((only().body as Record<string, unknown>)['expectedRevision'], 2);
  assert.deepEqual([withPrefix.action.businessIdPrefix, withPrefix.action.businessIdConflictPolicy, withPrefix.action.businessIdReusePolicy], ['nightly', 'FAIL', 'ALLOW_DUPLICATE_FAILED_ONLY']);

  prepare((_req, res) => reply(res, 200, schedule({ revision: 4 })));
  const without = await client().schedules.update(NS, SCHEDULE, writeWith(), 3, { idempotencyKey: 'upd-key-bid2' });
  assert.deepEqual(Object.keys(actionOf(only())).sort(), ['input', 'workflowId']);
  assert.equal(without.action.businessIdPrefix, null);
});

test('a policy this SDK does not know is kept as the string the server sent, in a list and in a description', async () => {
  const newer = { ...SERVED, businessIdPrefix: 'p', businessIdConflictPolicy: 'SOMETHING_NEWER', businessIdReusePolicy: 'SOMETHING_ELSE' };
  prepare((_req, res) => reply(res, 200, { items: [schedule({ action: newer })], nextCursor: null }));
  const listed = await client().schedules.list(NS);
  assert.deepEqual([listed.items[0]?.action.businessIdConflictPolicy, listed.items[0]?.action.businessIdReusePolicy], ['SOMETHING_NEWER', 'SOMETHING_ELSE']);
  prepare((_req, res) => reply(res, 200, { ...schedule({ action: newer }), nextActions: [], recentActions: [], openRuns: [], bufferedCount: 0, counters: {} }));
  const described = await client().schedules.get(NS, SCHEDULE);
  assert.equal(described.action.businessIdPrefix, 'p');
  assert.equal(described.action.businessIdConflictPolicy, 'SOMETHING_NEWER');
});

test('the outcomes and reasons of a business-id action are kept as the strings the server sent', async () => {
  prepare((_req, res) => reply(res, 200, {
    ...schedule(), nextActions: [], openRuns: [], bufferedCount: 0, counters: { BUSINESS_ID_CONFLICT: 2, ALREADY_ACTED: 1 },
    recentActions: [
      action({ outcome: 'BUSINESS_ID_CONFLICT', reason: 'BUSINESS_ID_IN_USE', runId: null }),
      action({ outcome: 'BUSINESS_ID_CONFLICT', reason: 'BUSINESS_ID_REUSE_REJECTED', runId: null }),
      action({ outcome: 'ALREADY_ACTED', reason: 'BUSINESS_ID_IN_USE', runId: RUN }),
    ],
  }));
  const described = await client().schedules.get(NS, SCHEDULE);
  assert.deepEqual(described.recentActions.map((a) => [a.outcome, a.reason, a.runId]), [
    ['BUSINESS_ID_CONFLICT', 'BUSINESS_ID_IN_USE', null],
    ['BUSINESS_ID_CONFLICT', 'BUSINESS_ID_REUSE_REJECTED', null],
    ['ALREADY_ACTED', 'BUSINESS_ID_IN_USE', RUN],
  ]);
  assert.deepEqual(described.counters, { BUSINESS_ID_CONFLICT: 2, ALREADY_ACTED: 1 });
  prepare((_req, res) => reply(res, 200, { items: [action({ outcome: 'BUSINESS_ID_CONFLICT', reason: 'BUSINESS_ID_REUSE_REJECTED', runId: null })], nextCursor: null }));
  const listed = await client().schedules.listActions(NS, SCHEDULE, { outcome: 'BUSINESS_ID_CONFLICT' });
  assert.equal(only().url, `/v1/namespaces/${NS}/schedules/${SCHEDULE}/actions?outcome=BUSINESS_ID_CONFLICT`);
  assert.deepEqual([listed.items[0]?.outcome, listed.items[0]?.reason], ['BUSINESS_ID_CONFLICT', 'BUSINESS_ID_REUSE_REJECTED']);
});

test('the conformance runner prints the business-id keys of a schedule action only when the schedule has them', async () => {
  prepare((_req, res) => reply(res, 200, schedule({ action: { ...SERVED, businessIdPrefix: PREFIX, businessIdConflictPolicy: 'FAIL', businessIdReusePolicy: 'ALLOW_DUPLICATE' } })));
  const withPrefix: Schedule = await client().schedules.create(NS, writeWith({ businessIdPrefix: PREFIX }));
  assert.deepEqual(scheduleJson(withPrefix)['action'], {
    workflowId: WF, versionPolicy: 'ACTIVE', versionId: null, inputDigest: 'a'.repeat(64), nominalTimeField: null, runTimeoutMs: null,
    businessIdPrefix: PREFIX, businessIdConflictPolicy: 'FAIL', businessIdReusePolicy: 'ALLOW_DUPLICATE',
  });

  prepare((_req, res) => reply(res, 200, schedule()));
  const plain = await client().schedules.create(NS, writeWith());
  // What every golden has had: these six keys and nothing else, so no existing golden changes.
  assert.deepEqual(Object.keys(scheduleJson(plain)['action']).sort(), ['inputDigest', 'nominalTimeField', 'runTimeoutMs', 'versionId', 'versionPolicy', 'workflowId']);
  assert.deepEqual(scheduleJson(plain)['action'], { workflowId: WF, versionPolicy: 'ACTIVE', versionId: null, inputDigest: 'a'.repeat(64), nominalTimeField: null, runTimeoutMs: null });

  // A description prints the same action, and its recent actions print the outcome and reason strings as they came.
  prepare((_req, res) => reply(res, 200, {
    ...schedule({ action: { ...SERVED, businessIdPrefix: 'p', businessIdConflictPolicy: 'USE_EXISTING', businessIdReusePolicy: 'ALLOW_DUPLICATE' } }),
    nextActions: [], openRuns: [], bufferedCount: 0, counters: {},
    recentActions: [action({ outcome: 'ALREADY_ACTED', reason: 'BUSINESS_ID_IN_USE', runId: RUN })],
  }));
  const described = await client().schedules.get(NS, SCHEDULE);
  const printed = descriptionJson(described);
  assert.equal(printed['action']['businessIdPrefix'], 'p');
  assert.equal(printed['action']['businessIdConflictPolicy'], 'USE_EXISTING');
  const record = actionJson(described.recentActions[0] as never);
  assert.deepEqual([record['outcome'], record['reason'], record['runId']], ['ALREADY_ACTED', 'BUSINESS_ID_IN_USE', RUN]);
  assert.deepEqual(printed['recentActions'][0], record);
});

test('the prefix of a schedule action reaches no log line and no error text the SDK writes, on a retry and on a refusal', async () => {
  const canary = 'CANARY-prefix-8f3a';
  const lines: string[] = [];
  const record = (message: string, fields?: Record<string, unknown>): void => void lines.push(`${message} ${JSON.stringify(fields ?? {})}`);
  const logger: Logger = { debug: record, info: record, warn: record, error: record };
  const logged = new SagatClient({ apiUrl: base, namespace: 'payments', token: 'tok-1', logger, retry: { initialDelayMs: 5, maxDelayMs: 20 } });

  let attempts = 0;
  prepare((_req, res) => (++attempts < 2 ? reply(res, 503, { code: 'UNAVAILABLE', message: 'busy', retryable: true, traceId: 'e'.repeat(32) }) : reply(res, 201, schedule({ action: { ...SERVED, businessIdPrefix: canary } }))));
  await logged.schedules.create(NS, writeWith({ businessIdPrefix: canary }));
  assert.equal(seen.length, 2, 'the create was retried once, with the prefix in its body');
  assert.equal(actionOf(seen[1] as Seen)['businessIdPrefix'], canary);
  assert.ok(lines.length >= 1, 'the retry was logged');

  prepare((_req, res) => reply(res, 400, { code: 'VALIDATION_FAILED', message: 'the schedule is not valid', retryable: false, traceId: 'e'.repeat(32), details: { reason: 'SCHEDULE_SPEC_INVALID', field: '/action/businessIdPrefix' } }));
  const refused = await logged.schedules.create(NS, writeWith({ businessIdPrefix: canary })).then(() => null, (error: unknown) => error);
  assert.ok(refused instanceof SagatError);
  assert.equal(refused.code, 'VALIDATION_FAILED');
  assert.equal(refused.reason, 'SCHEDULE_SPEC_INVALID');
  for (const text of [...lines, refused.message, JSON.stringify(refused), inspect(refused, { depth: 6 })]) {
    assert.ok(!text.includes(canary), 'the prefix is in nothing the SDK writes');
  }
});

// ---------------------------------------------------------------------------------------------- business ids

test('a business identifier is percent-encoded as given, and one that cannot be a path segment is refused locally', async () => {
  prepare((_req, res) => reply(res, 200, {
    businessId: 'Ord/1#x?y', executionId: RUN, currentRunId: RUN, open: false, lastStatus: 'SUCCEEDED', workflowId: WF,
    boundAt: '2026-10-10T10:00:00Z', reusableAfter: '2026-11-09T10:00:00Z',
  }));
  const binding = await client().businessIds.get(NS, 'Ord/1#x?y');
  assert.equal(only().url, `/v1/namespaces/${NS}/business-ids/Ord%2F1%23x%3Fy`, 'no trimming and no case folding');
  prepare((_req, res) => reply(res, 404, { code: 'NOT_FOUND', message: 'none', retryable: false, traceId: 'e'.repeat(32) }));
  await assert.rejects(client().businessIds.get(NS, '...'), (error: unknown) => error instanceof SagatError && error.code === 'NOT_FOUND');
  assert.equal(only().url, `/v1/namespaces/${NS}/business-ids/...`, 'dots that are not a whole segment are an identifier like any other');
  assert.equal(binding.open, false);
  assert.equal(binding.reusableAfter?.toISOString(), '2026-11-09T10:00:00.000Z');
  seen = [];
  for (const bad of ['', ' padded', 'with space', 'x'.repeat(201), 'ünï', '.', '..']) {
    await assert.rejects(client().businessIds.get(NS, bad), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED');
  }
  assert.equal(seen.length, 0);
});

// ---------------------------------------------------------------------------------------------- batch

test('a batch is previewed, created with one key, read, cancelled and its items filtered by state', async () => {
  const job = { batchId: BATCH, action: 'CANCEL', state: 'RUNNING', createdBy: 'alice', createdAt: '2026-10-10T10:00:00Z', counts: { selected: 2, succeeded: 1, skipped: 0, failed: 0, cancelled: 0, deferred: 1 } };
  prepare((_req, res) => reply(res, 200, { matched: 2, capped: false, sample: [RUN] }));
  const preview = await client().batch.preview(NS, { runIds: [RUN] }, 50);
  assert.equal(only().url, `/v1/namespaces/${NS}/batch-operations/preview`);
  assert.deepEqual(only().body, { selector: { runIds: [RUN] }, maxItems: 50 });
  assert.deepEqual([preview.matched, preview.capped, preview.sample], [2, false, [RUN]]);

  prepare((_req, res) => reply(res, 202, job));
  const created = await client().batch.create(NS, {
    selector: { filter: { status: ['RUNNING'], createdAfter: '2026-10-10T02:00:00+02:00' } }, action: 'CANCEL', reason: 'clean up the day', maxItems: 10,
  }, { idempotencyKey: 'batch-key-01' });
  assert.deepEqual(only().body, {
    selector: { filter: { status: ['RUNNING'], createdAfter: '2026-10-10T00:00:00.000Z' } }, action: 'CANCEL', reason: 'clean up the day', maxItems: 10,
  });
  assert.equal(only().headers['idempotency-key'], 'batch-key-01');
  assert.deepEqual(created.counts, { selected: 2, succeeded: 1, skipped: 0, failed: 0, cancelled: 0, deferred: 1 });
  assert.equal(created.hasPayload, false);

  prepare((_req, res) => reply(res, 202, { ...job, state: 'CANCELLING' }));
  const cancelling = await client().batch.cancel(NS, BATCH, 'stop the clean up', { idempotencyKey: 'batch-cancel-1' });
  assert.equal(only().url, `/v1/namespaces/${NS}/batch-operations/${BATCH}/cancel`);
  assert.deepEqual(only().body, { reason: 'stop the clean up' });
  assert.equal(cancelling.state, 'CANCELLING');

  prepare((_req, res) => reply(res, 200, { items: [{ ordinal: 0, runId: RUN, state: 'SKIPPED', reason: 'ALREADY_TERMINAL' }, { ordinal: 1, runId: RUN, state: 'NEWER_STATE' }], nextCursor: 'n' }));
  const items = await client().batch.listItems(NS, BATCH, { state: 'SKIPPED', limit: 2 });
  assert.equal(only().url, `/v1/namespaces/${NS}/batch-operations/${BATCH}/items?limit=2&state=SKIPPED`);
  assert.equal(items.items[0]?.reason, 'ALREADY_TERMINAL');
  assert.equal(items.items[1]?.state, 'NEWER_STATE', 'a state of a newer server is kept as a string');
  assert.equal(items.nextCursor, 'n');
});

// ---------------------------------------------------------------------------------------------- with start

test('signal-with-start names the workflow by id, carries the signal and one key, and says what the signal woke', async () => {
  prepare((_req, res) => reply(res, 202, { runId: RUN, started: true, deliveredTo: ['wait'] }));
  const result = await client().signalWithStart(WF, 'order-9', { a: 1 }, 'go', { b: 2 }, { idempotencyKey: 'sws-key-0001', businessIdReusePolicy: 'ALLOW_DUPLICATE_FAILED_ONLY' });
  assert.equal(only().url, '/v1/runs/signal-with-start');
  assert.deepEqual(only().body, {
    workflowId: WF, businessId: 'order-9', input: { a: 1 }, signal: { name: 'go', payload: { b: 2 } },
    businessIdReusePolicy: 'ALLOW_DUPLICATE_FAILED_ONLY',
  });
  assert.equal(only().headers['idempotency-key'], 'sws-key-0001');
  assert.deepEqual([result.runId, result.started, result.deliveredTo], [RUN, true, ['wait']]);
});

test('update-with-start requires the caller\'s key, keeps it across a retry, and reads a rejection as an answer', async () => {
  const receipt = { runId: RUN, operation: 'set', updateKey: 'u-1', status: 'APPLIED', result: 7, beforeStateRevision: 0, afterStateRevision: 1, runVersion: 4, replayed: true };
  prepare((_req, res) => reply(res, 200, { runId: RUN, started: true, update: { outcome: 'APPLIED', receipt } }));
  const applied = await client().updateWithStart(WF, 'order-9', {}, 'set', 'u-1', { value: 7 }, { idempotencyKey: 'uws-key-0001', businessIdConflictPolicy: 'FAIL', startDelayMs: 2000, expectedStateRevision: 0 });
  assert.equal(only().url, '/v1/runs/update-with-start');
  assert.deepEqual(only().body, {
    workflowId: WF, businessId: 'order-9', input: {}, update: { name: 'set', updateKey: 'u-1', args: { value: 7 }, expectedStateRevision: 0 },
    businessIdConflictPolicy: 'FAIL', startDelayMs: 2000,
  });
  assert.equal(applied.update.outcome, 'APPLIED');
  assert.equal(applied.update.receipt?.afterStateRevision, 1);
  assert.equal(applied.update.receipt?.result, 7);
  assert.equal(applied.update.receipt?.replayed, true, 'a replay says so');

  prepare((_req, res) => reply(res, 200, { runId: RUN, started: false, update: { outcome: 'REJECTED', reason: 'REVISION_CONFLICT' } }));
  const rejected = await client().updateWithStart(WF, 'order-9', {}, 'set', 'u-2', {}, { idempotencyKey: 'uws-key-0002' });
  assert.deepEqual([rejected.started, rejected.update.outcome, rejected.update.reason, rejected.update.receipt], [false, 'REJECTED', 'REVISION_CONFLICT', null]);

  seen = [];
  await assert.rejects(client().updateWithStart(WF, 'order-9', {}, 'set', '', {}), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED');
  await assert.rejects(client().updateWithStart(WF, 'order-9', {}, 'set', 'u-3', {}, { expectedStateRevision: -1 }), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED');
  assert.equal(seen.length, 0);

  let attempts = 0;
  prepare((_req, res) => (++attempts < 2 ? reply(res, 503, { code: 'UNAVAILABLE', message: 'busy', retryable: true, traceId: 'e'.repeat(32) }) : reply(res, 200, { runId: RUN, started: true, update: { outcome: 'APPLIED', receipt } })));
  await client().updateWithStart(WF, 'order-9', {}, 'set', 'u-4', {});
  assert.equal(seen.length, 2);
  assert.deepEqual((seen[0] as Seen).body, (seen[1] as Seen).body);
  assert.equal((seen[0] as Seen).headers['idempotency-key'], (seen[1] as Seen).headers['idempotency-key']);
});

// ---------------------------------------------------------------------------------------------- reset

test('the reset calls list the points, preview the plan, reset with a key and read the register', async () => {
  prepare((_req, res) => reply(res, 200, { items: [{ sequence: 1, kind: 'RUN_STARTED', node: null, occurredAt: '2026-10-10T10:00:00Z' }, { sequence: 4, kind: 'SOMETHING_NEWER', node: 'a', occurredAt: '2026-10-10T10:00:01Z' }], nextCursor: null }));
  const points = await client().resetPoints(RUN, { limit: 5 });
  assert.equal(only().url, `/v1/runs/${RUN}/reset-points?limit=5`);
  assert.deepEqual(points.items.map((p) => [p.sequence, p.kind, p.node]), [[1, 'RUN_STARTED', null], [4, 'SOMETHING_NEWER', 'a']]);

  prepare((_req, res) => reply(res, 200, {
    sequence: 4, restored: [{ node: 'a', kind: 'STEP' }], reexecuted: [{ node: 'b', completedInSource: true, markers: ['irreversible'] }],
    reapply: [], requiresAcknowledgement: true, prefixEvents: 6,
  }));
  const plan = await client().previewReset(RUN, 4, 'NONE');
  assert.deepEqual(only().body, { sequence: 4, reapply: 'NONE' });
  assert.equal(plan.requiresAcknowledgement, true);
  assert.deepEqual(plan.reexecuted, [{ node: 'b', completedInSource: true, markers: ['irreversible'] }]);
  await assert.rejects(client().previewReset(RUN, 0), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED');

  prepare((_req, res) => reply(res, 202, { resetId: BATCH, sourceRunId: RUN, sequence: 4, state: 'CREATED', newRunId: SCHEDULE, reapplyReport: [], createdAt: '2026-10-10T10:00:02Z' }));
  const made = await client().reset(RUN, { sequence: 4, reason: 'start over from a', acknowledgeReexecution: true, businessId: 'DROP' }, { idempotencyKey: 'reset-key-001' });
  assert.equal(only().url, `/v1/runs/${RUN}/resets`);
  assert.equal(only().headers['idempotency-key'], 'reset-key-001');
  assert.deepEqual(only().body, { sequence: 4, reason: 'start over from a', acknowledgeReexecution: true, businessId: 'DROP' });
  assert.deepEqual([made.state, made.newRunId, made.failureReason], ['CREATED', SCHEDULE, null]);
  prepare((_req, res) => reply(res, 202, { resetId: BATCH, sourceRunId: RUN, sequence: 1, state: 'PENDING_SOURCE' }));
  await client().reset(RUN, { sequence: 1, reason: 'start over from the top' });
  assert.deepEqual(only().body, { sequence: 1, reason: 'start over from the top' }, 'what was not asked is not sent');

  prepare((_req, res) => reply(res, 200, { items: [{ resetId: BATCH, sourceRunId: RUN, sequence: 4, state: 'CREATED' }] }));
  assert.equal((await client().listResets(RUN)).length, 1);
  prepare((_req, res) => reply(res, 200, { resetId: BATCH, sourceRunId: RUN, sequence: 4, state: 'CREATED' }));
  assert.equal((await client().getReset(RUN, BATCH)).resetId, BATCH);
  assert.equal(only().url, `/v1/runs/${RUN}/resets/${BATCH}`);
});

test('the token does not appear in anything a group of the client exposes', () => {
  const c = client();
  for (const group of [c.schedules, c.batch, c.businessIds]) {
    assert.ok(!JSON.stringify(group).includes('tok-1'));
    assert.ok(!Object.values(group as unknown as Record<string, unknown>).some((v) => JSON.stringify(v)?.includes('tok-1')));
  }
});
