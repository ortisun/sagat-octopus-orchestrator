import { SagatError, invalid } from './errors.js';
import { canonicalJson } from './json.js';

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
type Node = Record<string, Json>;

export interface RetryOptions {
  maxAttempts?: number;
  initialDelayMs?: number;
  maxDelayMs?: number;
  strategy?: 'fixed' | 'exponential';
  jitter?: boolean;
}

export interface StepOptions {
  retry?: RetryOptions;
  timeoutMs?: number;
  /** The activity that undoes this step; it must be idempotent. */
  compensateWith?: string;
  /** The activity that finds out whether this step happened: Reconcile. */
  reconcileWith?: string;
  idempotencyTemplate?: string;
  secrets?: string[];
  compensationRetry?: RetryOptions;
  compensationTimeoutMs?: number;
  irreversible?: boolean;
  highRisk?: boolean;
  onError?: string;
}

export interface WaitOptions {
  eventType: string;
  /** An expression over the run's data that yields the correlation key. */
  correlationExpr: string;
  timeoutMs: number;
  onTimeout?: string;
  lookbackMs?: number;
}

export interface ChildOptions {
  version?: number;
  input?: string;
  cancelPolicy?: 'propagate' | 'detach' | 'request_cancel';
}

export interface SagaOptions {
  onCompensationFailure?: 'manual_review' | 'fail';
  cancelPolicy?: 'compensate_completed' | 'abort_no_compensation' | 'wait_in_flight_then_compensate';
}

export interface Trigger {
  kind: 'manual' | 'event' | 'webhook' | 'message' | 'cron';
  eventType?: string;
  source?: string;
  endpoint?: string;
  connector?: string;
  filter?: string;
  cron?: string;
  [key: string]: Json | undefined;
}

const NAME = /^[a-z][a-z0-9-]{2,63}$/;
const SECRET = /^[a-z][a-z0-9._-]{0,63}$/;

function clean(options: object | undefined): Record<string, Json> {
  const out: Record<string, Json> = {};
  for (const [key, value] of Object.entries((options ?? {}) as Record<string, unknown>)) {
    if (value !== undefined) out[key] = value as Json;
  }
  return out;
}

/** A chain of nodes: the body of a workflow, of a parallel branch or of a saga. */
export class Flow {
  /** @internal */
  readonly nodes: Node[] = [];

  protected push(node: Node): this {
    const id = node['id'];
    if (typeof id !== 'string' || id === '') throw invalid('a node needs an id');
    this.nodes.push(node);
    return this;
  }

  /** Step.Run: an activity executed by a worker. */
  step(id: string, activity: string, options: StepOptions = {}): this {
    if (!activity) throw invalid(`step ${id} needs an activity name`);
    if (options.irreversible && options.compensateWith) {
      throw invalid(`step ${id} cannot be irreversible and have a compensation`);
    }
    for (const secret of options.secrets ?? []) {
      if (!SECRET.test(secret)) throw invalid(`step ${id} names a secret that is not a name: ${JSON.stringify(secret)}`);
    }
    const { onError, ...rest } = options;
    const node: Node = { id, type: 'activity', activity, ...clean(rest) };
    if (onError !== undefined) node['onError'] = onError;
    return this.push(node);
  }

  /** Sleep: waits for a duration. A timer, not a process: nothing is kept alive. */
  sleep(id: string, durationMs: number): this {
    if (!Number.isInteger(durationMs) || durationMs < 1) throw invalid(`sleep ${id} needs a duration of at least 1 ms`);
    return this.push({ id, type: 'timer', durationMs });
  }

  /** WaitForEvent: waits for an event of a type with a correlation key, or for the timeout. */
  waitForEvent(id: string, options: WaitOptions): this {
    if (!options.eventType || !options.correlationExpr) throw invalid(`waitForEvent ${id} needs an eventType and a correlationExpr`);
    if (!Number.isInteger(options.timeoutMs) || options.timeoutMs < 1) throw invalid(`waitForEvent ${id} needs a timeout of at least 1 ms`);
    return this.push({ id, type: 'waitEvent', ...clean(options) });
  }

  /** InvokeChild: starts another workflow of the namespace and waits for it. */
  invokeChild(id: string, workflow: string, options: ChildOptions = {}): this {
    if (!workflow) throw invalid(`invokeChild ${id} needs the name of a workflow`);
    return this.push({ id, type: 'child', workflowId: workflow, ...clean(options) });
  }

  /** A decision over the run's data. */
  condition(id: string, expression: string, ifTrue: string, ifFalse: string): this {
    if (!expression) throw invalid(`condition ${id} needs an expression`);
    return this.push({ id, type: 'condition', expression, ifTrue, ifFalse });
  }

  /** Parallel: runs branches at once; `all` waits for every one, `any` for the first. */
  parallel(id: string, join: 'all' | 'any', branches: Flow[]): this {
    if (branches.length < 2) throw invalid(`parallel ${id} needs at least two branches`);
    if (branches.some((branch) => branch.nodes.length === 0)) throw invalid(`parallel ${id} has an empty branch`);
    const node: Node = {
      id,
      type: 'parallel',
      branches: branches.map((branch) => branch.nodes[0]!['id'] as string),
      join,
    };
    this.push(node);
    this.embedded.push(...branches);
    return this;
  }

  /**
   * Saga: the body is compensated, in reverse order of completion, when it fails or the run is
   * cancelled; a step in it names its undo with `compensateWith`.
   */
  saga(id: string, options: SagaOptions, body: Flow): this {
    if (body.nodes.length === 0) throw invalid(`saga ${id} has an empty body`);
    if (body.nodes.some((node) => node['type'] === 'end')) throw invalid(`saga ${id} cannot hold an end node`);
    this.push({ id, type: 'saga', begin: body.nodes[0]!['id'] as string, ...clean(options) });
    this.embedded.push(body);
    return this;
  }

  /** An explicit end of the flow, optionally with the run's output expression. */
  end(id: string, options: { outputExpr?: string } = {}): this {
    return this.push({ id, type: 'end', ...clean(options) });
  }

  /** @internal */
  readonly embedded: Flow[] = [];

  /** @internal: every node of the flow and of the flows inside it, in order, with `next` set. */
  flatten(): Node[] {
    const out: Node[] = [];
    this.nodes.forEach((node, i) => {
      const linked: Node = { ...node };
      const next = this.nodes[i + 1];
      // A decision names both of its branches and an end ends: neither has a next node.
      const branches = linked['type'] === 'condition' || linked['type'] === 'end';
      if (next && !branches && linked['next'] === undefined) linked['next'] = next['id'] as string;
      out.push(linked);
    });
    for (const inner of this.embedded) out.push(...inner.flatten());
    return out;
  }
}

/** Builds the workflow IR (`workflow.schema.json`) without any side effect (SPEC §7). */
export class WorkflowBuilder extends Flow {
  private descriptionText: string | undefined;
  private readonly labels: Record<string, string> = {};
  private readonly triggerList: Trigger[] = [];
  private input: Record<string, Json> | undefined;
  private output: Record<string, Json> | undefined;

  constructor(private readonly name: string) {
    super();
    if (!NAME.test(name)) throw invalid(`a workflow name is lower case letters, digits and dashes, 3 to 64: ${JSON.stringify(name)}`);
  }

  description(text: string): this {
    this.descriptionText = text;
    return this;
  }

  label(key: string, value: string): this {
    this.labels[key] = value;
    return this;
  }

  trigger(trigger: Trigger): this {
    this.triggerList.push(trigger);
    return this;
  }

  inputSchema(schema: Record<string, Json>): this {
    this.input = schema;
    return this;
  }

  outputSchema(schema: Record<string, Json>): this {
    this.output = schema;
    return this;
  }

  /** The document. Raises `VALIDATION_FAILED` for what the server would refuse and the builder can see. */
  build(): Record<string, Json> {
    if (this.nodes.length === 0) throw invalid('a workflow needs at least one node');
    const nodes = this.flatten();
    const ids = new Set<string>();
    for (const node of nodes) {
      const id = node['id'] as string;
      if (ids.has(id)) throw invalid(`the node id ${JSON.stringify(id)} is used twice`);
      ids.add(id);
    }
    const reference = (from: string, field: string, target: Json | undefined): void => {
      if (typeof target === 'string' && !ids.has(target)) {
        throw invalid(`${from}.${field} points at ${JSON.stringify(target)}, which is not a node`);
      }
    };
    for (const node of nodes) {
      const id = node['id'] as string;
      for (const field of ['next', 'onError', 'onTimeout', 'ifTrue', 'ifFalse', 'begin']) reference(id, field, node[field]);
      if (Array.isArray(node['branches'])) node['branches'].forEach((branch) => reference(id, 'branches', branch));
    }
    const metadata: Record<string, Json> = { name: this.name };
    if (this.descriptionText !== undefined) metadata['description'] = this.descriptionText;
    if (Object.keys(this.labels).length > 0) metadata['labels'] = { ...this.labels };
    const document: Record<string, Json> = {
      apiVersion: 'sagat-octopus-orchestrator.io/v1',
      kind: 'Workflow',
      metadata,
      entrypoint: this.nodes[0]!['id'] as string,
      nodes: nodes as Json[],
    };
    if (this.triggerList.length > 0) document['triggers'] = this.triggerList.map((t) => clean(t));
    if (this.input) document['inputSchema'] = this.input;
    if (this.output) document['outputSchema'] = this.output;
    return document;
  }

  /** The canonical text of `build()`: keys sorted, no whitespace. */
  toJson(): string {
    return canonicalJson(this.build());
  }
}

/** Starts a workflow definition. */
export function workflow(name: string): WorkflowBuilder {
  return new WorkflowBuilder(name);
}

/** A flow for a parallel branch or a saga body. */
export function flow(): Flow {
  return new Flow();
}

export { SagatError };
