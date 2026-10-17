import { SagatError, invalid } from './errors.js';
import { canonicalJson } from './json.js';

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
type Node = Record<string, Json>;

export type DisclosurePolicy = 'declared-business-result-v1';

export interface QueryDefinition {
  readonly argsSchema: Record<string, Json>;
  readonly resultSchema: Record<string, Json>;
  readonly expression: string;
  readonly disclosure: DisclosurePolicy;
}

export interface UpdateDefinition {
  readonly argsSchema: Record<string, Json>;
  readonly validate?: string;
  readonly state: string;
  readonly result: string;
  readonly resultSchema: Record<string, Json>;
  readonly disclosure: DisclosurePolicy;
}

export interface StateDefinition {
  readonly schema: Record<string, Json>;
  readonly initial: string;
  readonly queries?: Readonly<Record<string, QueryDefinition>>;
  readonly updates?: Readonly<Record<string, UpdateDefinition>>;
}

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

/** One ordered branch of a switch decision. */
export interface SwitchCase {
  readonly id: string;
  readonly when: string;
  readonly target: string;
}

export interface CollectionOptions {
  readonly maxItems: number;
  readonly maxConcurrency: number;
}

export interface LoopOptions {
  readonly maxIterations: number;
  readonly timeoutMs: number;
  readonly delayMs?: number;
  readonly onError?: string;
}

export type ApprovalRole = 'admin' | 'developer' | 'operator';

export interface ApprovalPolicy {
  readonly subjects?: readonly string[];
  readonly roles?: readonly ApprovalRole[];
  readonly excludeInitiator?: boolean;
}

export interface ApprovalOptions {
  readonly policy: ApprovalPolicy;
  readonly timeoutMs: number;
  readonly onApprove: string;
  readonly onReject: string;
  readonly onExpire: string;
}

export interface ChainPolicy {
  readonly maxSegments: number;
  readonly timeoutMs: number;
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
const OPERATION_NAME = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/;
const SECRET = /^[a-z][a-z0-9._-]{0,63}$/;
const DISCLOSURE: DisclosurePolicy = 'declared-business-result-v1';

function clean(options: object | undefined): Record<string, Json> {
  const out: Record<string, Json> = {};
  for (const [key, value] of Object.entries((options ?? {}) as Record<string, unknown>)) {
    if (value !== undefined) out[key] = value as Json;
  }
  return out;
}

function copyJson<T extends Json>(value: T): T {
  if (Array.isArray(value)) return value.map((item) => copyJson(item)) as T;
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, copyJson(item)])) as T;
  }
  return value;
}

function expression(value: unknown, subject: string): string {
  if (typeof value !== 'string' || value.length === 0) throw invalid(`${subject} needs an expression`);
  if (new TextEncoder().encode(value).length > 1024) throw invalid(`${subject} expression exceeds 1024 UTF-8 bytes`);
  return value;
}

function integer(value: Json | undefined, minimum: number, maximum: number, subject: string): number {
  if (!Number.isInteger(value) || (value as number) < minimum || (value as number) > maximum) {
    throw invalid(`${subject} must be between ${minimum} and ${maximum}`);
  }
  return value as number;
}

function resultSchema(value: unknown, subject: string): Record<string, Json> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw invalid(`${subject} must be a closed bounded schema`);
  }
  const schema = value as Record<string, Json>;
  const description = schema['description'];
  if (description !== undefined && (typeof description !== 'string' || [...description].length > 1024)) {
    throw invalid(`${subject}.description must be a string of at most 1024 characters`);
  }
  const kind = schema['type'];
  const allowed = new Set(['type', 'description']);
  switch (kind) {
    case 'object': {
      allowed.add('properties'); allowed.add('required'); allowed.add('additionalProperties');
      const properties = schema['properties'];
      if (properties === null || typeof properties !== 'object' || Array.isArray(properties)) {
        throw invalid(`${subject}.properties must be an object`);
      }
      if (schema['additionalProperties'] !== false) {
        throw invalid(`${subject}.additionalProperties must be false`);
      }
      for (const [name, nested] of Object.entries(properties as Record<string, Json>)) {
        resultSchema(nested, `${subject}.properties.${name}`);
      }
      if (schema['required'] !== undefined) {
        const required = schema['required'];
        if (!Array.isArray(required) || required.length > 10_000 || required.some((name) => typeof name !== 'string')
            || new Set(required).size !== required.length
            || required.some((name) => !Object.prototype.hasOwnProperty.call(properties, name as string))) {
          throw invalid(`${subject}.required must name unique declared properties`);
        }
      }
      break;
    }
    case 'array': {
      allowed.add('items'); allowed.add('minItems'); allowed.add('maxItems');
      resultSchema(schema['items'], `${subject}.items`);
      const maximum = integer(schema['maxItems'], 0, 10_000, `${subject}.maxItems`);
      if (schema['minItems'] !== undefined && integer(schema['minItems'], 0, 10_000, `${subject}.minItems`) > maximum) {
        throw invalid(`${subject}.minItems must not exceed maxItems`);
      }
      break;
    }
    case 'string': {
      allowed.add('minLength'); allowed.add('maxLength');
      const maximum = integer(schema['maxLength'], 0, 262_144, `${subject}.maxLength`);
      if (schema['minLength'] !== undefined && integer(schema['minLength'], 0, 262_144, `${subject}.minLength`) > maximum) {
        throw invalid(`${subject}.minLength must not exceed maxLength`);
      }
      break;
    }
    case 'integer':
    case 'number': {
      allowed.add('minimum'); allowed.add('maximum');
      for (const field of ['minimum', 'maximum'] as const) {
        const bound = schema[field];
        if (bound !== undefined && (typeof bound !== 'number' || !Number.isFinite(bound))) {
          throw invalid(`${subject}.${field} must be a finite number`);
        }
      }
      if (typeof schema['minimum'] === 'number' && typeof schema['maximum'] === 'number'
          && schema['minimum'] > schema['maximum']) {
        throw invalid(`${subject}.minimum must not exceed maximum`);
      }
      break;
    }
    case 'boolean':
    case 'null':
      break;
    default:
      throw invalid(`${subject}.type is not a supported closed result type`);
  }
  for (const key of Object.keys(schema)) {
    if (!allowed.has(key)) throw invalid(`${subject}.${key} is not supported in a closed result schema`);
  }
  if (new TextEncoder().encode(canonicalJson(schema)).length > 65_536) {
    throw invalid(`${subject} exceeds 65536 UTF-8 bytes`);
  }
  return copyJson(schema);
}

function argsSchema(value: unknown, subject: string): Record<string, Json> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw invalid(`${subject}.argsSchema must be an object`);
  }
  return copyJson(value as Record<string, Json>);
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

  /** Tries cases in order and follows the first matching target, or the default. */
  switch(id: string, cases: readonly SwitchCase[], defaultTarget: string): this {
    if (cases.length < 1 || cases.length > 32) throw invalid(`switch ${id} needs between 1 and 32 cases`);
    if (!defaultTarget) throw invalid(`switch ${id} needs a default target`);
    const caseIds = new Set<string>();
    const copied = cases.map((item) => {
      if (!item.id) throw invalid(`switch ${id} has a case without an id`);
      if (caseIds.has(item.id)) throw invalid(`switch ${id} uses case id ${JSON.stringify(item.id)} twice`);
      if (!item.when) throw invalid(`switch ${id} case ${JSON.stringify(item.id)} needs a when expression`);
      if (!item.target) throw invalid(`switch ${id} case ${JSON.stringify(item.id)} needs a target`);
      caseIds.add(item.id);
      return { id: item.id, when: item.when, target: item.target };
    });
    return this.push({ id, type: 'switch', cases: copied, default: defaultTarget });
  }

  /** Invokes a named body once for every item captured from `items`. */
  forEach(id: string, items: string, body: string, result: string, options: CollectionOptions): this {
    if (!items || !body || !result) throw invalid(`forEach ${id} needs items, body and result expressions`);
    if (!Number.isInteger(options.maxItems) || options.maxItems < 1 || options.maxItems > 10_000) {
      throw invalid(`forEach ${id} maxItems must be between 1 and 10000`);
    }
    if (!Number.isInteger(options.maxConcurrency) || options.maxConcurrency < 1 || options.maxConcurrency > 256) {
      throw invalid(`forEach ${id} maxConcurrency must be between 1 and 256`);
    }
    return this.push({
      id, type: 'forEach', items, body, result,
      maxItems: options.maxItems, maxConcurrency: options.maxConcurrency,
    });
  }

  /** Value-returning collection authoring; it emits the same durable `forEach` node. */
  map(id: string, items: string, body: string, result: string, options: CollectionOptions): this {
    return this.forEach(id, items, body, result, options);
  }

  /** Runs a named body sequentially while a pre-test condition is true. */
  while(id: string, condition: string, body: string, initial: string, carry: string, options: LoopOptions): this {
    return this.loop('while', id, condition, body, initial, carry, options);
  }

  /** Runs a named body once, then repeats it until a post-test condition is true. */
  repeatUntil(id: string, condition: string, body: string, initial: string, carry: string, options: LoopOptions): this {
    return this.loop('repeatUntil', id, condition, body, initial, carry, options);
  }

  private loop(type: 'while' | 'repeatUntil', id: string, condition: string, body: string, initial: string, carry: string, options: LoopOptions): this {
    if (!condition || !body || !initial || !carry) {
      throw invalid(`${type} ${id} needs condition, body, initial and carry expressions`);
    }
    if (!Number.isInteger(options.maxIterations) || options.maxIterations < 1 || options.maxIterations > 10_000) {
      throw invalid(`${type} ${id} maxIterations must be between 1 and 10000`);
    }
    if (!Number.isInteger(options.timeoutMs) || options.timeoutMs < 1 || options.timeoutMs > 31_622_400_000) {
      throw invalid(`${type} ${id} timeoutMs must be between 1 and 31622400000`);
    }
    if (options.delayMs !== undefined && (!Number.isInteger(options.delayMs) || options.delayMs < 1 || options.delayMs > 31_622_400_000)) {
      throw invalid(`${type} ${id} delayMs must be between 1 and 31622400000`);
    }
    return this.push({
      id, type, condition, body, initial, carry,
      maxIterations: options.maxIterations,
      timeoutMs: options.timeoutMs,
      ...clean({ delayMs: options.delayMs, onError: options.onError }),
    });
  }

  /** Waits for one authorized approval decision or for its deadline. */
  approval(id: string, options: ApprovalOptions): this {
    const subjects = [...(options.policy.subjects ?? [])];
    const roles = [...(options.policy.roles ?? [])];
    if (subjects.length + roles.length < 1 || subjects.length + roles.length > 32) {
      throw invalid(`approval ${id} policy needs between 1 and 32 subjects and roles`);
    }
    if (new Set(subjects).size !== subjects.length) throw invalid(`approval ${id} policy has a duplicate subject`);
    if (new Set(roles).size !== roles.length) throw invalid(`approval ${id} policy has a duplicate role`);
    for (const subject of subjects) {
      if (!subject.trim() || Buffer.byteLength(subject, 'utf8') > 255 || /[\u0000-\u001f\u007f-\u009f]/u.test(subject)
          || subject.slice(0, 7).toLowerCase() === 'system:') {
        throw invalid(`approval ${id} policy has an invalid subject`);
      }
    }
    for (const role of roles) {
      if (role !== 'admin' && role !== 'developer' && role !== 'operator') {
        throw invalid(`approval ${id} policy has an unsupported role`);
      }
    }
    if (!Number.isInteger(options.timeoutMs) || options.timeoutMs < 1 || options.timeoutMs > 31_622_400_000) {
      throw invalid(`approval ${id} timeoutMs must be between 1 and 31622400000`);
    }
    if (!options.onApprove) throw invalid(`approval ${id} needs onApprove`);
    if (!options.onReject) throw invalid(`approval ${id} needs onReject`);
    if (!options.onExpire) throw invalid(`approval ${id} needs onExpire`);
    const policy: Record<string, Json> = {};
    if (options.policy.subjects !== undefined) policy['subjects'] = subjects;
    if (options.policy.roles !== undefined) policy['roles'] = roles;
    if (options.policy.excludeInitiator !== undefined) policy['excludeInitiator'] = options.policy.excludeInitiator;
    return this.push({
      id,
      type: 'approval',
      policy,
      timeoutMs: options.timeoutMs,
      onApprove: options.onApprove,
      onReject: options.onReject,
      onExpire: options.onExpire,
    });
  }

  /** Ends this run and transfers explicitly selected values to the next segment. */
  continueAsNew(id: string, input: string, state?: string): this {
    if (!input) throw invalid(`continueAsNew ${id} needs an input expression`);
    if (state !== undefined && !state) throw invalid(`continueAsNew ${id} state must be a non-empty expression`);
    return this.push({ id, type: 'continueAsNew', input, ...clean({ state }) });
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
      // A branching node names its routes and an end ends: neither has a next node.
      const branches = linked['type'] === 'condition' || linked['type'] === 'switch' || linked['type'] === 'approval'
        || linked['type'] === 'continueAsNew'
        || linked['type'] === 'end' || linked['type'] === 'return';
      if (next && !branches && linked['next'] === undefined) linked['next'] = next['id'] as string;
      out.push(linked);
    });
    for (const inner of this.embedded) out.push(...inner.flatten());
    return out;
  }
}

/** A closed, named collection or loop body with one explicit return boundary. */
export class Body extends Flow {
  returnValue(id: string, result: string): this {
    if (!result) throw invalid(`return ${id} needs a result expression`);
    return this.push({ id, type: 'return', result });
  }
}

/** Builds the workflow IR (`workflow.schema.json`) without any side effect (SPEC §7). */
export class WorkflowBuilder extends Flow {
  private descriptionText: string | undefined;
  private readonly labels: Record<string, string> = {};
  private readonly triggerList: Trigger[] = [];
  private input: Record<string, Json> | undefined;
  private output: Record<string, Json> | undefined;
  private stateDeclaration: Record<string, Json> | undefined;
  private chain: ChainPolicy | undefined;
  private readonly namedBodies = new Map<string, Body>();

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

  /** Declares bounded business state and its side-effect-free named operations. */
  state(definition: StateDefinition): this {
    const state: Record<string, Json> = {
      schema: resultSchema(definition.schema, 'state.schema'),
      initial: expression(definition.initial, 'state.initial'),
    };
    const queryEntries = Object.entries(definition.queries ?? {});
    if (queryEntries.length > 32) throw invalid('state supports at most 32 queries');
    if (queryEntries.length > 0) {
      state['queries'] = Object.fromEntries(queryEntries.map(([name, query]) => {
        if (!OPERATION_NAME.test(name)) throw invalid(`query name ${JSON.stringify(name)} is invalid`);
        if (query === null || typeof query !== 'object') throw invalid(`query ${name} is required`);
        if (query.disclosure !== DISCLOSURE) throw invalid(`query ${name} needs disclosure ${DISCLOSURE}`);
        return [name, {
          argsSchema: argsSchema(query.argsSchema, `query ${name}`),
          resultSchema: resultSchema(query.resultSchema, `query ${name}.resultSchema`),
          expression: expression(query.expression, `query ${name}`),
          disclosure: query.disclosure,
        }];
      }));
    }
    const updateEntries = Object.entries(definition.updates ?? {});
    if (updateEntries.length > 32) throw invalid('state supports at most 32 updates');
    if (updateEntries.length > 0) {
      state['updates'] = Object.fromEntries(updateEntries.map(([name, update]) => {
        if (!OPERATION_NAME.test(name)) throw invalid(`update name ${JSON.stringify(name)} is invalid`);
        if (update === null || typeof update !== 'object') throw invalid(`update ${name} is required`);
        if (update.disclosure !== DISCLOSURE) throw invalid(`update ${name} needs disclosure ${DISCLOSURE}`);
        return [name, clean({
          argsSchema: argsSchema(update.argsSchema, `update ${name}`),
          validate: update.validate === undefined ? undefined : expression(update.validate, `update ${name}.validate`),
          state: expression(update.state, `update ${name}.state`),
          result: expression(update.result, `update ${name}.result`),
          resultSchema: resultSchema(update.resultSchema, `update ${name}.resultSchema`),
          disclosure: update.disclosure,
        })];
      }));
    }
    this.stateDeclaration = state;
    return this;
  }

  /** Sets the finite lifetime shared by every run in a continuation chain. */
  chainPolicy(policy: ChainPolicy): this {
    if (!Number.isInteger(policy.maxSegments) || policy.maxSegments < 1 || policy.maxSegments > 10_000) {
      throw invalid('chainPolicy maxSegments must be between 1 and 10000');
    }
    if (!Number.isInteger(policy.timeoutMs) || policy.timeoutMs < 1 || policy.timeoutMs > 31_622_400_000) {
      throw invalid('chainPolicy timeoutMs must be between 1 and 31622400000');
    }
    this.chain = { ...policy };
    return this;
  }

  /** Registers a closed body by name. The body is read when `build()` runs. */
  body(name: string, definition: Body): this {
    if (!name) throw invalid('a body needs a name');
    if (this.namedBodies.has(name)) throw invalid(`the body ${JSON.stringify(name)} is defined twice`);
    this.namedBodies.set(name, definition);
    return this;
  }

  /** The document. Raises `VALIDATION_FAILED` for what the server would refuse and the builder can see. */
  build(): Record<string, Json> {
    if (this.nodes.length === 0) throw invalid('a workflow needs at least one node');
    const nodes = this.flatten();
    if (this.embedded.some((nested) => nested.flatten().some((node) => node['type'] === 'continueAsNew'))) {
      throw invalid('continueAsNew is valid only in the top-level workflow');
    }
    if (this.nodes.some((node) => node['type'] === 'continueAsNew') && this.chain === undefined) {
      throw invalid('a workflow with continueAsNew needs chainPolicy');
    }
    const bodyNodes = new Map<string, Node[]>();
    for (const [name, definition] of this.namedBodies) {
      const nested = definition.flatten();
      if (nested.length === 0) throw invalid(`body ${JSON.stringify(name)} is empty`);
      if (nested.some((node) => node['type'] === 'end')) throw invalid(`body ${JSON.stringify(name)} cannot hold an end node`);
      if (nested.some((node) => node['type'] === 'continueAsNew')) {
        throw invalid('continueAsNew is valid only in the top-level workflow');
      }
      if (nested.filter((node) => node['type'] === 'return').length !== 1) {
        throw invalid(`body ${JSON.stringify(name)} needs exactly one return node`);
      }
      bodyNodes.set(name, nested);
    }
    const visiting = new Set<string>();
    const visited = new Set<string>();
    const visitBody = (name: string): void => {
      if (visiting.has(name)) throw invalid('body reference cycle is not allowed');
      if (visited.has(name)) return;
      visiting.add(name);
      for (const node of bodyNodes.get(name) ?? []) {
        if (node['type'] === 'forEach' || node['type'] === 'while' || node['type'] === 'repeatUntil') {
          const target = node['body'];
          if (typeof target === 'string' && bodyNodes.has(target)) visitBody(target);
        }
      }
      visiting.delete(name);
      visited.add(name);
    };
    for (const name of bodyNodes.keys()) visitBody(name);
    const validateScope = (scope: Node[]): void => {
      const localIds = new Set<string>();
      for (const node of scope) {
        const id = node['id'] as string;
        if (localIds.has(id)) throw invalid(`the node id ${JSON.stringify(id)} is used twice`);
        localIds.add(id);
      }
      const reference = (from: string, field: string, target: Json | undefined): void => {
        if (typeof target === 'string' && !localIds.has(target)) {
          throw invalid(`${from}.${field} points at ${JSON.stringify(target)}, which is not a node`);
        }
      };
      for (const node of scope) {
        const id = node['id'] as string;
        for (const field of ['next', 'onError', 'onTimeout', 'ifTrue', 'ifFalse', 'begin', 'onApprove', 'onReject', 'onExpire']) {
          reference(id, field, node[field]);
        }
        if (Array.isArray(node['branches'])) node['branches'].forEach((branch) => reference(id, 'branches', branch));
        if (Array.isArray(node['cases'])) {
          node['cases'].forEach((item) => {
            if (item !== null && typeof item === 'object' && !Array.isArray(item)) reference(id, 'cases', item['target']);
          });
        }
        reference(id, 'default', node['default']);
        if (node['type'] === 'forEach' || node['type'] === 'while' || node['type'] === 'repeatUntil') {
          const type = node['type'] as string;
          const bodyName = node['body'];
          if (typeof bodyName !== 'string' || !this.namedBodies.has(bodyName)) {
            throw invalid(`${type} ${id} points at ${JSON.stringify(bodyName)}, which is not a body`);
          }
          if (typeof node['next'] !== 'string') throw invalid(`${type} ${id} needs a next node`);
        }
      }
    };
    validateScope(nodes);
    for (const scope of bodyNodes.values()) {
      validateScope(scope);
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
    if (this.stateDeclaration) document['state'] = copyJson(this.stateDeclaration);
    if (this.chain) document['chainPolicy'] = { ...this.chain };
    if (bodyNodes.size > 0) {
      document['bodies'] = Object.fromEntries([...bodyNodes].map(([name, body]) => [name, {
        entrypoint: body[0]!['id'],
        nodes: body as Json[],
      }])) as Json;
    }
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

/** A closed body for a runtime-sized collection or loop. */
export function body(): Body {
  return new Body();
}

export { SagatError };
