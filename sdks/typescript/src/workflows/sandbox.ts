/*
 * The `node:vm` context of one run and the string bridge into it (design 044, Technical Plan 1.2).
 *
 * Each run gets its own context, created with `vm.constants.DONT_CONTEXTIFY` (an ordinary global object made inside the
 * context, not a host object), `microtaskMode: 'afterEvaluate'` (the context's own microtask queue, drained completely
 * after every evaluation and inside its timeout) and no code generation from strings or WebAssembly. No host function or
 * object is placed in it: the host sets one primitive string and evaluates a precompiled call, and reads back the
 * primitive string the call returns. The context is for determinism, not a security boundary (Gap 044-008).
 */
import { readFileSync } from 'node:fs';
import { types } from 'node:util';
import vm from 'node:vm';
import { BRIDGE_VERSION } from './context.js';
import { checkNodeVersion } from './preconditions.js';

/** The default activation deadline: an activation run that has not yielded within it fails `ACTIVATION_DEADLINE`. */
export const DEFAULT_ACTIVATION_DEADLINE_MS = 2000;
export const MINIMUM_ACTIVATION_DEADLINE_MS = 100;

const BRIDGE_CALL = new vm.Script('globalThis.__sagat_bridge.call(globalThis.__sagat_in)', { filename: 'sagat-bridge.js' });
const PROBE = JSON.stringify({ op: 'probe' });

/** The activation run did not yield in time; the context may be in any state and is dropped. */
export class ActivationDeadlineExceeded extends Error {
  constructor() { super('the activation run did not yield before its deadline'); this.name = 'ActivationDeadlineExceeded'; }
}

/** The bundle is not a workflow bundle of this SDK's bridge. */
export class InvalidWorkflowBundle extends Error {
  constructor(message: string) { super(message); this.name = 'InvalidWorkflowBundle'; }
}

/** A workflow bundle compiled once per digest and evaluated in each run's context. */
export class WorkflowBundle {
  readonly code: string;
  readonly script: vm.Script;
  readonly workflowTypes: readonly string[];

  constructor(code: string, filename = 'workflow-bundle.js') {
    checkNodeVersion();
    this.code = code;
    this.script = new vm.Script(code, { filename });
    // A probe context checks the bridge version and lists the bundle's workflow types; it runs no workflow function.
    const probe = new WorkflowSandbox(this, DEFAULT_ACTIVATION_DEADLINE_MS);
    const answer = probe.call(PROBE, DEFAULT_ACTIVATION_DEADLINE_MS) as { version?: unknown; workflowTypes?: unknown };
    if (answer.version !== BRIDGE_VERSION) {
      throw new InvalidWorkflowBundle(`the bundle's bridge is version ${String(answer.version)}, this SDK's is ${BRIDGE_VERSION}`);
    }
    if (!Array.isArray(answer.workflowTypes) || !answer.workflowTypes.every((type) => typeof type === 'string')) {
      throw new InvalidWorkflowBundle('the bundle did not list its workflow types');
    }
    this.workflowTypes = Object.freeze([...answer.workflowTypes as string[]]);
  }

  static fromFile(path: string): WorkflowBundle {
    return new WorkflowBundle(readFileSync(path, 'utf8'), path);
  }
}

/** The context of one run. */
export class WorkflowSandbox {
  readonly #context: Record<string, unknown>;

  constructor(bundle: WorkflowBundle, deadlineMs: number) {
    this.#context = vm.createContext(vm.constants.DONT_CONTEXTIFY, {
      name: 'sagat-workflow',
      microtaskMode: 'afterEvaluate',
      codeGeneration: { strings: false, wasm: false },
    }) as Record<string, unknown>;
    // A number, not an object: it tells the bundle's runtime to replace the globals before any workflow module runs.
    this.#context['__sagat_context'] = BRIDGE_VERSION;
    try {
      bundle.script.runInContext(this.#context, { timeout: deadlineMs, breakOnSigint: false });
    } catch (error) {
      if (isTimeout(error)) throw new ActivationDeadlineExceeded();
      throw new InvalidWorkflowBundle(`the bundle failed to load: ${describe(error)}`);
    }
  }

  /** Calls the bridge with a JSON request; answers the parsed JSON reply. */
  call(request: string, timeoutMs: number): Record<string, unknown> {
    this.#context['__sagat_in'] = request;
    let reply: unknown;
    try {
      reply = BRIDGE_CALL.runInContext(this.#context, { timeout: Math.max(1, Math.floor(timeoutMs)), breakOnSigint: false });
    } catch (error) {
      if (isTimeout(error)) throw new ActivationDeadlineExceeded();
      throw new InvalidWorkflowBundle(`the bridge failed: ${describe(error)}`);
    } finally {
      this.#context['__sagat_in'] = undefined;
    }
    // Only a primitive string crosses back; anything else means the context's bridge was tampered with.
    if (typeof reply !== 'string') throw new InvalidWorkflowBundle('the bridge answered something other than a string');
    const parsed = JSON.parse(reply) as unknown;
    if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) throw new InvalidWorkflowBundle('the bridge answered no object');
    const answer = parsed as Record<string, unknown>;
    if (typeof answer['error'] === 'string') throw new InvalidWorkflowBundle(answer['error']);
    return answer;
  }

  /** Evaluates source text in the context (tests use it to look at the context from inside). */
  evaluate(source: string): unknown {
    return vm.runInContext(source, this.#context, { timeout: DEFAULT_ACTIVATION_DEADLINE_MS });
  }
}

// An error thrown by workflow code is an object of the context: reading it must run none of its code (a getter or a
// proxy trap would run outside the activation's deadline), so only own data properties are read.
function ownString(error: unknown, key: string): string | undefined {
  if (error === null || typeof error !== 'object' || types.isProxy(error)) return undefined;
  const descriptor = Object.getOwnPropertyDescriptor(error, key);
  return descriptor && 'value' in descriptor && typeof descriptor.value === 'string' ? descriptor.value : undefined;
}

function isTimeout(error: unknown): boolean {
  return ownString(error, 'code') === 'ERR_SCRIPT_EXECUTION_TIMEOUT';
}

function describe(error: unknown): string {
  return (ownString(error, 'message') ?? 'an error').slice(0, 500);
}
