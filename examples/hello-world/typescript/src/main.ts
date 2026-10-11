// The hello-world workflow served and started with the TypeScript SDK: a worker that answers the two
// activities of the workflow (templates/hello-world), a run of it, and the end of the run.
//
//   SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 \
//   SAGAT_NAMESPACE=payments SAGAT_TOKEN=... npm start -- [NAME]
import { SagatClient, SagatError, WorkerHost } from '@sagat-octopus-orchestrator/sdk';

// What a step receives: the input of the run (the first argument) and the outputs of the steps
// that ran before it (`ctx.steps`, by step id).
interface Input {
  name: string;
}

interface Greeting {
  greeting: string;
}

interface Delivery {
  delivered: boolean;
  greeting: string;
}

async function run(): Promise<void> {
  const apiUrl = setting('SAGAT_SERVER');
  const grpcUrl = setting('SAGAT_GRPC');
  const namespace = setting('SAGAT_NAMESPACE');
  const token = setting('SAGAT_TOKEN');
  const name = process.argv[2] ?? 'Ada';

  const interrupted = new AbortController();
  process.once('SIGINT', () => interrupted.abort());

  // The worker: it takes the tasks of the two activities and answers them.
  const host = new WorkerHost({ grpcUrl, namespace, token, workerName: 'hello-world-typescript' });
  host.registerActivity<Input, Greeting>('greeting.compose', (input) => ({ greeting: `Hello, ${input.name}!` }));
  host.registerActivity<Input, Delivery>('greeting.deliver', (_input, ctx) => {
    // A real delivery is an external effect: send ctx.idempotencyKey to the provider, which is
    // the same for every attempt of this step.
    const composed = ctx.steps['compose'] as Greeting;
    console.log(`delivering ${composed.greeting}`);
    return { delivered: true, greeting: composed.greeting };
  });
  await host.start();
  try {
    console.log(`serving greeting.compose and greeting.deliver at protocol ${host.protocolVersion}`);

    // The client: start the run and wait for it.
    const api = new SagatClient({ apiUrl, namespace, token });
    const started = await api.start('hello-world', { name });
    console.log(`run ${started.runId} started`);
    const done = await api.waitForCompletion(started.runId, { timeoutMs: 60_000, signal: interrupted.signal });
    console.log(`run ${done.runId} ${done.status}`);
    if (done.status !== 'SUCCEEDED') throw new Error(`the run ended ${done.status}`);
  } finally {
    // Stop polling and let the tasks in flight finish (up to 10 s); a failure to stop hides nothing.
    await host.stop(10_000).catch(() => undefined);
  }
}

function setting(name: string): string {
  const value = process.env[name];
  if (value === undefined || value === '') {
    console.error(`${name} is not set (see the README)`);
    process.exit(2);
  }
  return value;
}

try {
  await run();
} catch (error) {
  if (error instanceof SagatError) {
    // The code, the trace and what to do about it; never the payload or the token.
    console.error(`${error.code} (trace ${error.traceId}): ${error.remediation}`);
  } else {
    console.error(error instanceof Error ? error.message : String(error));
  }
  process.exitCode = 1;
}
