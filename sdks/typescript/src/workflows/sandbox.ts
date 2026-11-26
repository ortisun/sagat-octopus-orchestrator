/*
 * The `node:vm` context of one run and the string bridge into it (design 044, Technical Plan 1.2).
 *
 * Each run gets its own context, created with `vm.constants.DONT_CONTEXTIFY` (an ordinary global object made inside the
 * context, not a host object), `microtaskMode: 'afterEvaluate'` (the context's own microtask queue, drained completely
 * after every evaluation and inside its timeout) and no code generation from strings or WebAssembly. No host function or
 * object is placed in it: the host sets one primitive string and evaluates a precompiled call, and reads back the
 * primitive string the call returns. The context is for determinism, not a security boundary (Gap 044-008); what
 * crosses back is still checked, so that the tenant's code cannot reach the host's objects by accident or on purpose.
 *
 * - A dynamic `import()` would otherwise be refused by Node with an error made in the host's realm, from which the
 *   host's `Function` and `process` are reachable. Every script is compiled with an import callback that throws a
 *   primitive string, which needs `--experimental-vm-modules`: the workflow thread is started with it, and a context
 *   refuses to exist in a thread without it.
 * - The two global properties the host writes are made non-configurable data properties before the bundle runs, so
 *   workflow code cannot turn them into setters that would run outside the deadline.
 * - Errors of the context are read only through own data properties, and no text of theirs is copied into a host error.
 */
import { readFileSync } from 'node:fs';
import { types } from 'node:util';
import vm from 'node:vm';
import { BRIDGE_VERSION } from './context.js';
import { checkNodeVersion, WorkflowRuntimeUnavailable } from './preconditions.js';

/** The default activation deadline: an activation run that has not yielded within it fails `ACTIVATION_DEADLINE`. */
export const DEFAULT_ACTIVATION_DEADLINE_MS = 2000;
export const MINIMUM_ACTIVATION_DEADLINE_MS = 100;
/** The file name of every bundle in stack traces: never a path of the host. */
export const BUNDLE_FILENAME = 'workflow-bundle.js';
/** The longest answer the host parses: a completion is at most 320 KiB, the logs of an activation at most 256 KiB. */
export const MAX_REPLY_CHARACTERS = 8 * 1024 * 1024;

const EXCEPTION_TYPE = /^[A-Za-z_][A-Za-z0-9_.$]{0,127}$/;

// Thrown into the context when its code calls import(): a primitive, so it carries no prototype of the host.
function refuseImport(): never {
  // eslint-disable-next-line @typescript-eslint/only-throw-error
  throw 'IMPORT_REFUSED: workflow code imports nothing at run time; the bundler bundles its modules';
}

function script(code: string, filename: string): vm.Script {
  return new vm.Script(code, { filename, importModuleDynamically: refuseImport as unknown as vm.ScriptOptions['importModuleDynamically'] });
}

/** Throws unless this thread can give a dynamic import callback (the workflow thread is started so that it can). */
export function checkImportRefusal(): void {
  if (typeof (vm as { SourceTextModule?: unknown }).SourceTextModule !== 'function') {
    throw new WorkflowRuntimeUnavailable('VM_MODULES',
      'a workflow context runs only where node:vm can refuse a dynamic import: start Node with --experimental-vm-modules, as the workflow thread is');
  }
}

let bridgeCall: vm.Script | undefined;
const PROBE = JSON.stringify({ op: 'probe' });

/** The activation run did not yield in time; the context may be in any state and is dropped. */
export class ActivationDeadlineExceeded extends Error {
  constructor() { super('the activation run did not yield before its deadline'); this.name = 'ActivationDeadlineExceeded'; }
}

/** The bundle is not a workflow bundle of this SDK's bridge, or the context answered something the host refuses. */
export class InvalidWorkflowBundle extends Error {
  constructor(message: string) { super(message); this.name = 'InvalidWorkflowBundle'; }
}

/** A workflow bundle compiled once per digest and evaluated in each run's context. */
export class WorkflowBundle {
  readonly code: string;
  readonly script: vm.Script;
  readonly workflowTypes: readonly string[];

  constructor(code: string) {
    checkNodeVersion();
    checkImportRefusal();
    this.code = code;
    this.script = script(code, BUNDLE_FILENAME);
    // A probe context checks the bridge version and lists the bundle's workflow types; it runs no workflow function.
    const probe = new WorkflowSandbox(this, DEFAULT_ACTIVATION_DEADLINE_MS);
    const answer = probe.call(PROBE, DEFAULT_ACTIVATION_DEADLINE_MS) as { version?: unknown; workflowTypes?: unknown };
    if (typeof answer.version !== 'number' || answer.version !== BRIDGE_VERSION) {
      throw new InvalidWorkflowBundle(`the bundle's bridge is not version ${BRIDGE_VERSION}, this SDK's`);
    }
    if (!Array.isArray(answer.workflowTypes) || answer.workflowTypes.length > 1024 || !answer.workflowTypes.every((type) => typeof type === 'string')) {
      throw new InvalidWorkflowBundle('the bundle did not list its workflow types');
    }
    this.workflowTypes = Object.freeze([...answer.workflowTypes as string[]]);
  }

  static fromFile(path: string): WorkflowBundle {
    return new WorkflowBundle(readFileSync(path, 'utf8'));
  }
}

/** The context of one run. */
export class WorkflowSandbox {
  readonly #context: Record<string, unknown>;

  constructor(bundle: WorkflowBundle, deadlineMs: number) {
    checkImportRefusal();
    this.#context = vm.createContext(vm.constants.DONT_CONTEXTIFY, {
      name: 'sagat-workflow',
      microtaskMode: 'afterEvaluate',
      codeGeneration: { strings: false, wasm: false },
    }) as Record<string, unknown>;
    // A number, not an object: it tells the bundle's runtime to replace the globals before any workflow module runs.
    // Both properties the host writes are data properties workflow code cannot reconfigure into accessors.
    Object.defineProperty(this.#context, '__sagat_context', { value: BRIDGE_VERSION, writable: false, enumerable: false, configurable: false });
    Object.defineProperty(this.#context, '__sagat_in', { value: undefined, writable: true, enumerable: false, configurable: false });
    const begun = performance.now();
    try {
      bundle.script.runInContext(this.#context, { timeout: deadlineMs, breakOnSigint: false });
    } catch (error) {
      if (isTimeout(error, begun, deadlineMs)) throw new ActivationDeadlineExceeded();
      throw new InvalidWorkflowBundle(`the bundle failed to load${nameOf(error)}`);
    }
  }

  /** Calls the bridge with a JSON request; answers the parsed JSON reply. */
  call(request: string, timeoutMs: number): Record<string, unknown> {
    bridgeCall ??= script('globalThis.__sagat_bridge.call(globalThis.__sagat_in)', 'sagat-bridge.js');
    let reply: unknown;
    const allowance = Math.max(1, Math.floor(timeoutMs));
    const begun = performance.now();
    try {
      // A plain write of a data property: no code of the context runs (the bundle cannot have made it an accessor).
      this.#context['__sagat_in'] = request;
      reply = bridgeCall.runInContext(this.#context, { timeout: allowance, breakOnSigint: false });
    } catch (error) {
      if (isTimeout(error, begun, allowance)) throw new ActivationDeadlineExceeded();
      throw new InvalidWorkflowBundle(`the bridge failed${nameOf(error)}`);
    } finally {
      try { this.#context['__sagat_in'] = undefined; } catch { /* a frozen global: the call already failed */ }
    }
    // Only a primitive string crosses back; anything else means the context's bridge was tampered with.
    if (typeof reply !== 'string') throw new InvalidWorkflowBundle('the bridge answered something other than a string');
    if (reply.length > MAX_REPLY_CHARACTERS) throw new InvalidWorkflowBundle('the bridge answered more than the host reads');
    let parsed: unknown;
    try { parsed = JSON.parse(reply); } catch { throw new InvalidWorkflowBundle('the bridge answered no JSON'); }
    if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) throw new InvalidWorkflowBundle('the bridge answered no object');
    const answer = parsed as Record<string, unknown>;
    if (answer['error'] !== undefined) throw new InvalidWorkflowBundle('the bridge refused the request');
    return answer;
  }

  /** Evaluates source text in the context (tests use it to look at the context from inside). */
  evaluate(source: string): unknown {
    return script(source, 'sagat-evaluate.js').runInContext(this.#context, { timeout: DEFAULT_ACTIVATION_DEADLINE_MS });
  }
}

// An error thrown by workflow code is an object of the context: reading it must run none of its code (a getter or a
// proxy trap would run outside the activation's deadline), so only own data properties are read.
function ownString(error: unknown, key: string): string | undefined {
  if (error === null || typeof error !== 'object' || types.isProxy(error)) return undefined;
  const descriptor = Object.getOwnPropertyDescriptor(error, key);
  return descriptor && 'value' in descriptor && typeof descriptor.value === 'string' ? descriptor.value : undefined;
}

// Node makes its timeout error in the context's realm, so workflow code can throw one of the same shape: a deadline is
// judged by the clock as well, and an evaluation that ends early with that code is not one.
function isTimeout(error: unknown, begun: number, allowance: number): boolean {
  return ownString(error, 'code') === 'ERR_SCRIPT_EXECUTION_TIMEOUT' && performance.now() - begun >= allowance - 1;
}

// The name of an error of the context, never its message: a name is an identifier, checked here (the context's own
// RegExp may have been replaced), a message is free text that may hold a payload.
function nameOf(error: unknown): string {
  let name: string | undefined;
  if (error !== null && typeof error === 'object' && !types.isProxy(error)) {
    name = ownString(error, 'name');
    if (name === undefined) {
      const prototype = Object.getPrototypeOf(error) as object | null;
      if (prototype !== null && !types.isProxy(prototype)) {
        const descriptor = Object.getOwnPropertyDescriptor(prototype, 'name');
        if (descriptor && 'value' in descriptor && typeof descriptor.value === 'string') name = descriptor.value;
      }
    }
  }
  return name !== undefined && EXCEPTION_TYPE.test(name) ? ` (${name})` : '';
}
