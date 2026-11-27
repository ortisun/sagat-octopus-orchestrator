// The fake workflow server (design 044, Task 044-30) driven by a scripted client: no workflow runtime exists yet, so the
// client sends exactly what the goldens say a correct SDK sends, and the fake must find no difference; a changed value
// must be found.
import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import * as grpc from '@grpc/grpc-js';
import {
  goldenCompletion,
  listScenarioRuns,
  startFakeWorkflowServer,
  workerServiceClient,
  type FakeWorkflowServer,
  type GoldenActivation,
  type HistoryEvent,
} from './fake-workflow-server.js';

type Client = grpc.Client & Record<string, any>;

function unary<T = Record<string, any>>(client: Client, method: string, request: Record<string, unknown>): Promise<T> {
  return new Promise((resolve, reject) => {
    client[method](request, (error: grpc.ServiceError | null, response: T) => (error ? reject(error) : resolve(response)));
  });
}

function serverStream<T = Record<string, any>>(client: Client, method: string, request: Record<string, unknown>): Promise<T[]> {
  return new Promise((resolve, reject) => {
    const frames: T[] = [];
    const call = client[method](request) as grpc.ClientReadableStream<T>;
    call.on('data', (frame: T) => frames.push(frame));
    call.on('error', reject);
    call.on('end', () => resolve(frames));
  });
}

interface Session { worker_id: string; session_token: string; accepted_protocol_version: string }

/** Registers a session that offers 1.10 and every (workflow type, code digest) pair of the golden run. */
async function register(fake: FakeWorkflowServer, client: Client): Promise<Session> {
  const { workflowType, codeVersions } = fake.golden.commands;
  return unary<Session>(client, 'RegisterWorker', {
    worker_name: 'scripted',
    namespace: 'default',
    capabilities: {
      languages: ['typescript'],
      protocol_version: '1.10',
      offers_workflow_as_code: true,
      workflow_types: Object.values(codeVersions).map((digest) => ({ workflow_type: workflowType, code_digest: digest })),
    },
  });
}

/** One lease by PollTasks or StreamTasks, or undefined when the fake has none. */
async function take(client: Client, session: Session, viaStream: boolean): Promise<Record<string, any> | undefined> {
  const request = { worker_id: session.worker_id, session_token: session.session_token, max_tasks: 1 };
  const frames = viaStream
    ? await serverStream(client, 'StreamTasks', request)
    : [await unary(client, 'PollTasks', request)];
  const tasks = frames.flatMap((frame) => frame['tasks'] as Array<Record<string, any>>);
  assert.ok(tasks.length <= 1, 'one lease at a time');
  return tasks[0];
}

const fence = (lease: Record<string, any>): Record<string, unknown> => ({
  task_id: lease['task_id'],
  run_id: lease['run_id'],
  attempt: lease['attempt'],
  generation: lease['generation'],
  lease_token: lease['lease_token'],
});

async function readHistory(client: Client, lease: Record<string, any>, fromSeq: number): Promise<Array<Record<string, any>>> {
  return serverStream(client, 'ReadHistory', { ...fence(lease), from_seq: fromSeq });
}

/** The activation and attempt a lease serves, from the fake's task identifier `wt-<k>-<attempt>`. */
function planOf(fake: FakeWorkflowServer, lease: Record<string, any>): { activation: GoldenActivation; attempt: number } {
  const match = /^wt-(\d+)-(\d+)$/.exec(String(lease['task_id']));
  assert.ok(match, `a task identifier of the fake: ${String(lease['task_id'])}`);
  const activation = fake.golden.commands.activations.find((a) => a.k === Number(match[1]));
  assert.ok(activation, `activation ${match[1]} is in the golden`);
  return { activation, attempt: Number(match[2]) };
}

/** What the golden says for this lease: the FailWorkflowTask of a failed attempt, or the completion. */
async function answer(
  fake: FakeWorkflowServer,
  client: Client,
  lease: Record<string, any>,
  change?: (completion: Record<string, any>) => void,
): Promise<Record<string, any>> {
  const { activation, attempt } = planOf(fake, lease);
  const failed = activation.failedAttempts?.[attempt - 1];
  if (failed) {
    return unary(client, 'FailWorkflowTask', {
      ...fence(lease),
      idempotency_key: `fail-${lease['task_id']}`,
      cause: failed.cause,
      command_number: failed.report?.commandNumber ?? 0,
      actual_kind: failed.report?.actualKind ?? 'COMMAND_KIND_UNSPECIFIED',
      actual_target: failed.report?.actualTarget ?? '',
      exception_type: failed.exceptionType ?? '',
      code_digest: failed.report?.codeDigest ?? lease['workflow_task'].code_digest,
    });
  }
  const completion = { ...fence(lease), idempotency_key: `complete-${lease['task_id']}`, ...goldenCompletion(activation) };
  change?.(completion);
  return unary(client, 'CompleteWorkflowTask', completion);
}

const historyThrough = (fake: FakeWorkflowServer, seq: number): HistoryEvent[] => fake.golden.history.filter((e) => e.seq <= seq);
const seqs = (events: Array<Record<string, any>>): number[] => events.map((e) => Number(e['seq']));

test('code-run-lifecycle: every lease, history page and golden completion is served with no mismatch', async () => {
  const fake = await startFakeWorkflowServer({ scenario: 'code-run-lifecycle' });
  const client = workerServiceClient(fake.address);
  try {
    const session = await register(fake, client);
    assert.equal(session.accepted_protocol_version, '1.10');
    const { commands } = fake.golden;
    const outcomes: string[] = [];
    for (const activation of commands.activations) {
      const lease = await take(client, session, activation.k % 2 === 0);
      assert.ok(lease, `the lease of activation ${activation.k}`);
      const S = activation.window.throughSeq as number;
      assert.equal(lease['task_kind'], 'WORKFLOW');
      assert.equal(lease['step_id'], '~workflow');
      assert.equal(lease['run_id'], commands.runId);
      assert.equal(lease['workflow_work'], 'workflow_task');
      const task = lease['workflow_task'] as Record<string, any>;
      assert.equal(task.lease_token, lease['lease_token']);
      assert.equal(Buffer.from(task.seed).toString('hex'), commands.seedHex);
      assert.equal(task.window_start_seq, activation.window.afterSeq);
      assert.equal(task.started_at_us, activation.nowUs);
      assert.equal(task.workflow_type, 'order');
      assert.equal(task.code_digest, commands.codeVersions['v1']);
      assert.equal(task.visible_count, historyThrough(fake, S).length);
      assert.deepEqual(seqs(task.first_page), seqs(historyThrough(fake, S)), 'the first page is the history up to S(k)');
      assert.equal(task.first_page[task.first_page.length - 1].kind, 'HISTORY_WORKFLOW_TASK_STARTED');

      const pages = await readHistory(client, lease, 1);
      assert.equal(pages.length, 1);
      assert.equal(pages[0]?.['last_page'], true);
      assert.deepEqual(pages[0]?.['events'], historyThrough(fake, S), 'ReadHistory serves the decoded golden events up to S(k)');

      const response = await answer(fake, client, lease);
      assert.equal(response['outcome'], activation.outcome);
      assert.equal(response['already_applied'], false);
      outcomes.push(response['outcome'] as string);
    }
    assert.deepEqual(outcomes, Array(4).fill('WORKFLOW_TASK_ACCEPTED'));
    assert.equal(await take(client, session, false), undefined, 'no lease after the last activation');
    assert.deepEqual(fake.mismatches, []);
    assert.equal(fake.finished, true);
    const first = fake.received.find((c) => c.method === 'CompleteWorkflowTask');
    assert.deepEqual(first?.request['commands'].map((c: Record<string, any>) => c['kind']), ['COMMAND_ACTIVITY', 'COMMAND_ACTIVITY']);
    // The run's input decodes from the history as the golden input.
    const input = fake.golden.history[0]?.['run_started'].input.json as Buffer;
    assert.deepEqual(JSON.parse(input.toString('utf8')), commands.input);
  } finally {
    client.close();
    await fake.close();
  }
});

test('code-run-lifecycle: a changed activity input is recorded as a mismatch and answered with a review', async () => {
  const fake = await startFakeWorkflowServer({ scenario: 'code-run-lifecycle' });
  const client = workerServiceClient(fake.address);
  try {
    const session = await register(fake, client);
    const lease = await take(client, session, false);
    assert.ok(lease);
    const response = await answer(fake, client, lease, (completion) => {
      const command = completion['commands'][0] as Record<string, any>;
      command['value'] = { json: Buffer.from('{"quantity":3,"sku":"SKU-7"}') };
    });
    assert.equal(response['outcome'], 'WORKFLOW_TASK_REVIEW');
    assert.deepEqual(fake.mismatches, [
      {
        method: 'CompleteWorkflowTask',
        k: 1,
        attempt: 1,
        message:
          'activation 1 (attempt 1): command 1 (activity inventory.reserve): value.json: ' +
          'expected {"quantity":2,"sku":"SKU-7"}, received {"quantity":3,"sku":"SKU-7"}',
      },
    ]);
    assert.equal(await take(client, session, false), undefined, 'a run under review is not leased again');
  } finally {
    client.close();
    await fake.close();
  }
});

test('code-history-paging: a cold read of activation 3 gets pages of 1,024 events, then the rest', async () => {
  const fake = await startFakeWorkflowServer({ scenario: 'code-history-paging', firstPageMaxEvents: 0 });
  const client = workerServiceClient(fake.address);
  try {
    const session = await register(fake, client);
    for (const activation of fake.golden.commands.activations) {
      const lease = await take(client, session, true);
      assert.ok(lease);
      assert.deepEqual(lease['workflow_task'].first_page, [], 'firstPageMaxEvents 0 sends an empty first page');
      const pages = await readHistory(client, lease, 1);
      const S = activation.window.throughSeq as number;
      assert.ok(pages.every((page) => page['events'].length <= 1024));
      assert.deepEqual(pages.flatMap((page) => seqs(page['events'])), seqs(historyThrough(fake, S)));
      if (activation.historyPages) {
        assert.deepEqual(
          pages.map((page) => ({
            fromSeq: Number(page['events'][0].seq),
            throughSeq: Number(page['events'][page['events'].length - 1].seq),
            events: page['events'].length,
            lastPage: page['last_page'],
          })),
          activation.historyPages,
        );
      }
      const response = await answer(fake, client, lease);
      assert.equal(response['outcome'], 'WORKFLOW_TASK_ACCEPTED');
    }
    const third = fake.historyPages.filter((page) => page.k === 3).map(({ k, attempt, ...page }) => page);
    assert.deepEqual(third, [
      { fromSeq: 1, throughSeq: 1024, events: 1024, lastPage: false },
      { fromSeq: 1025, throughSeq: 1048, events: 24, lastPage: true },
    ]);
    assert.deepEqual(fake.mismatches, []);
  } finally {
    client.close();
    await fake.close();
  }
});

test('code-nondeterminism: the FailWorkflowTask of the golden is recorded with its cause and fields', async () => {
  const fake = await startFakeWorkflowServer({ scenario: 'code-nondeterminism' });
  const client = workerServiceClient(fake.address);
  try {
    const session = await register(fake, client);
    for (;;) {
      const lease = await take(client, session, false);
      if (!lease) break;
      await answer(fake, client, lease);
    }
    assert.deepEqual(fake.failures, [
      {
        k: 3,
        attempt: 1,
        cause: 'CAUSE_NONDETERMINISM',
        commandNumber: 2,
        actualKind: 'COMMAND_ACTIVITY',
        actualTarget: 'sms.send',
        exceptionType: '',
        codeDigest: 'sha256:8bda44227549b57a1fe57d76a6c51d275ff8000efc070132fb35de5cdfcb3971',
      },
    ]);
    assert.deepEqual(fake.mismatches, []);
    assert.equal(fake.finished, true);
  } finally {
    client.close();
    await fake.close();
  }
});

test('a completion for a lease that is not current is refused as a stale attempt', async () => {
  const fake = await startFakeWorkflowServer({ scenario: 'code-run-lifecycle' });
  const client = workerServiceClient(fake.address);
  try {
    const session = await register(fake, client);
    const lease = await take(client, session, false);
    assert.ok(lease);
    const activation = fake.golden.commands.activations[0] as GoldenActivation;
    await assert.rejects(
      unary(client, 'CompleteWorkflowTask', { ...fence(lease), lease_token: 'another', ...goldenCompletion(activation) }),
      (error: grpc.ServiceError) => error.code === grpc.status.ABORTED && error.metadata.get('x-sagat-error')[0] === 'STALE_ATTEMPT',
    );
    assert.equal(fake.mismatches.length, 1);
    const response = await answer(fake, client, lease);
    assert.equal(response['outcome'], 'WORKFLOW_TASK_ACCEPTED');
    const again = await answer(fake, client, lease);
    assert.equal(again['already_applied'], true, 'the same idempotency key is answered with the recorded outcome');
    await assert.rejects(unary(client, 'Heartbeat', {}), (error: grpc.ServiceError) => error.code === grpc.status.UNIMPLEMENTED);
  } finally {
    client.close();
    await fake.close();
  }
});

test('every scenario run of the corpus is served from its goldens with no mismatch', async () => {
  const runs = listScenarioRuns();
  assert.ok(runs.length >= 21, `the corpus has its scenario runs (${runs.length})`);
  for (const { scenario, run } of runs) {
    const fake = await startFakeWorkflowServer({ scenario, run });
    const client = workerServiceClient(fake.address);
    try {
      const session = await register(fake, client);
      const outcomes: string[] = [];
      let leases = 0;
      for (;;) {
        const lease = await take(client, session, leases % 2 === 1);
        if (!lease) break;
        leases += 1;
        const S = Number(lease['workflow_task'].first_page.at(-1)?.seq ?? 0);
        const pages = await readHistory(client, lease, S + 1);
        assert.equal(pages.at(-1)?.['last_page'], true, `${scenario}/${run}: the history ends at the lease's S`);
        const response = await answer(fake, client, lease);
        if ('outcome' in response) outcomes.push(response['outcome']);
      }
      const expected = fake.golden.commands.activations.flatMap((a) => (a.outcome ? [a.outcome] : []));
      assert.deepEqual(fake.mismatches, [], `${scenario}/${run}`);
      assert.deepEqual(outcomes, expected, `${scenario}/${run}: the golden outcomes`);
      assert.equal(fake.finished, true, `${scenario}/${run}: every lease of the golden was served`);
    } finally {
      client.close();
      await fake.close();
    }
  }
});
