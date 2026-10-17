// Serves the order-processing activities, starts one canonical workflow, and waits for its result.
// The activities simulate effects locally; they do not contact an external provider.
import {
  ActivityFailure,
  SagatClient,
  SagatError,
  WorkerHost,
} from '@sagat-octopus-orchestrator/sdk';

type Scenario = 'routing' | 'fulfillment' | 'reminder';

interface OrderInput extends Record<string, unknown> {
  orderId: string;
  amount: number;
  failNotify: boolean;
}

const WORKFLOWS: Readonly<Record<Scenario, string>> = {
  routing: 'order-routing',
  fulfillment: 'order-fulfillment',
  reminder: 'order-reminder',
};

function scenarioSetting(): Scenario {
  const value = process.env.SAGAT_SCENARIO || 'routing';
  if (value !== 'routing' && value !== 'fulfillment' && value !== 'reminder') {
    console.error('SAGAT_SCENARIO must be routing, fulfillment or reminder');
    process.exit(2);
  }
  return value;
}

function setting(name: string): string {
  const value = process.env[name];
  if (value === undefined || value === '') {
    console.error(`${name} is not set (see the README)`);
    process.exit(2);
  }
  return value;
}

async function run(): Promise<number> {
  const apiUrl = setting('SAGAT_SERVER');
  const grpcUrl = setting('SAGAT_GRPC');
  const namespace = setting('SAGAT_NAMESPACE');
  const token = setting('SAGAT_TOKEN');
  const scenario = scenarioSetting();
  const input: OrderInput = {
    orderId: 'demo-order-001',
    amount: process.env.SAGAT_REJECT_ORDER === '1' ? -1 : 100,
    failNotify: process.env.SAGAT_FAIL_NOTIFY === '1',
  };

  const interrupted = new AbortController();
  process.once('SIGINT', () => interrupted.abort());

  const host = new WorkerHost({
    grpcUrl,
    namespace,
    token,
    workerName: 'order-processing-typescript',
  });
  host.registerActivity<OrderInput, { accepted: boolean }>('order.validate', (order) => {
    console.log('activity order.validate');
    return { accepted: order.amount > 0 };
  });
  host.registerActivity<OrderInput, { reserved: boolean; orderId: string }>('order.reserve', (order) => {
    console.log('activity order.reserve');
    return { reserved: true, orderId: order.orderId };
  });
  host.registerActivity<OrderInput, { shippingCents: number }>('order.quote', () => {
    console.log('activity order.quote');
    return { shippingCents: 500 };
  });
  host.registerActivity<OrderInput, { notified: boolean; orderId: string }>('order.notify', (order, _ctx) => {
    console.log('activity order.notify');
    // Routing and fulfillment expose earlier outputs through optional ctx.steps keys; reminder-delay
    // is a timer and need not produce an output. Real code should check a key before consuming it.
    // A real notification would send ctx.idempotencyKey to the provider. This example only
    // simulates the effect and deliberately keeps payloads and credentials out of its output.
    if (order.failNotify) {
      throw ActivityFailure.permanent('NOTIFICATION_REJECTED', 'the simulated notification was rejected');
    }
    return { notified: true, orderId: order.orderId };
  });

  try {
    await host.start();
    const api = new SagatClient({ apiUrl, namespace, token });
    const started = await api.start(WORKFLOWS[scenario], input);
    const done = await api.waitForCompletion(started.runId, {
      timeoutMs: 60_000,
      signal: interrupted.signal,
    });
    console.log(`scenario ${scenario} run ${done.runId} ${done.status}`);
    return done.status === 'SUCCEEDED' ? 0 : 1;
  } finally {
    await host.stop(10_000).catch(() => undefined);
  }
}

try {
  process.exitCode = await run();
} catch (error) {
  if (error instanceof SagatError) {
    console.error(`${error.code} (trace ${error.traceId}): ${error.remediation}`);
  } else {
    console.error(error instanceof Error ? error.message : String(error));
  }
  process.exitCode = 1;
}
