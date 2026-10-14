import { SagatError } from '../errors.js';

export type TaskKind = 'FORWARD' | 'COMPENSATION' | 'RECONCILIATION';

export interface BlobDescriptor {
  readonly id: string;
  readonly size: number;
  readonly sha256: string;
  readonly contentType: string;
}

export interface WriteBlobOptions {
  contentType?: string;
  expectedSize?: number;
  expectedSha256?: string;
}

/** Return this explicit wrapper from an activity to complete with a blob reference. */
export class BlobOutput {
  constructor(readonly descriptor: BlobDescriptor) {}
}

/**
 * How an activity fails on purpose. `code` is a stable machine code (`[A-Z0-9_]{1,64}`); `message` is
 * recorded with the step, so it must not carry a payload or a secret.
 */
export class ActivityFailure extends Error {
  readonly code: string;
  readonly retryable: boolean;

  constructor(code: string, message: string, retryable = true) {
    super(message);
    this.name = 'ActivityFailure';
    this.code = code;
    this.retryable = retryable;
  }

  /** The step may be tried again as its node's retry policy says. */
  static retryable(code: string, message: string): ActivityFailure {
    return new ActivityFailure(code, message, true);
  }

  /** The step is failed for good: the policy goes to its error branch or its saga compensates. */
  static permanent(code: string, message: string): ActivityFailure {
    return new ActivityFailure(code, message, false);
  }

  /**
   * The activity sent a request and cannot tell whether it was applied (a timeout after connecting,
   * an answer that never arrived). The platform then reconciles the step, when it has a
   * `reconcileWith`, instead of retrying it blindly.
   */
  static outcomeUnknown(message: string): ActivityFailure {
    return new ActivityFailure('OUTCOME_UNKNOWN', message, false);
  }
}

/** What a reconciler says about the step it looks up (SPEC §6.6). */
export type Verdict =
  | { effect: 'applied'; output?: unknown }
  | { effect: 'not_applied' }
  | { effect: 'unknown' };

export const reconcile = {
  /** The effect happened; `output` is what the step would have returned. */
  applied(output?: unknown): Verdict {
    return output === undefined ? { effect: 'applied' } : { effect: 'applied', output };
  },
  /** The effect did not happen: the platform may try it again, with the same key. */
  notApplied(): Verdict {
    return { effect: 'not_applied' };
  },
  /** Cannot tell: the decision goes to a person. */
  unknown(): Verdict {
    return { effect: 'unknown' };
  },
};

export interface Secrets {
  /** The bytes of a secret the step declares. */
  get(name: string): Buffer;
  /** The secret as UTF-8 text. */
  text(name: string): string;
  names(): string[];
}

export interface ActivityLogger {
  debug(message: string, fields?: Record<string, unknown>): void;
  info(message: string, fields?: Record<string, unknown>): void;
  warn(message: string, fields?: Record<string, unknown>): void;
  error(message: string, fields?: Record<string, unknown>): void;
}

/** What an activity knows about the attempt it is running (SPEC §6.3). */
export interface ActivityContext {
  readonly activityName: string;
  readonly taskId: string;
  readonly runId: string;
  /** The node id of the step in the workflow definition (`charge`), from the step document. */
  readonly node: string;
  /** The platform's id of the step (`step_id` of the lease), which the reports carry. */
  readonly stepId: string;
  readonly attempt: number;
  readonly generation: number;
  /** Stable across the attempts of the step: send it to the provider of an external effect. */
  readonly idempotencyKey: string;
  readonly traceparent: string;
  readonly taskKind: TaskKind;
  /** For a compensation or a reconciliation: the forward step it is about. */
  readonly targetNode: string;
  /** The key that identifies the effect of that step at the provider. */
  readonly targetIdempotencyKey: string;
  /** The outputs of the steps that completed before this one, by node. */
  readonly steps: Readonly<Record<string, unknown>>;
  /** The end of the lease; moves forward with every heartbeat. */
  readonly deadline: Date;
  /** Aborted when the run is cancelled (`run-cancelled`), the lease is lost (`lease-lost`) or the host stops (`shutdown`). */
  readonly signal: AbortSignal;
  readonly log: ActivityLogger;
  /** The secrets the step declares; resolved once per attempt, on first use. Never log them. */
  secrets(): Promise<Secrets>;
  /** Streams bytes once, hashes them locally, and returns only a server descriptor that matches. */
  writeBlob(source: AsyncIterable<Uint8Array> | Iterable<Uint8Array>, options?: WriteBlobOptions): Promise<BlobDescriptor>;
  /** A stream is verified only if it reaches clean EOF; early cancellation makes no integrity claim. */
  readBlob(descriptor: BlobDescriptor): AsyncIterable<Buffer>;
}

export type ActivityHandler<I, O> = (input: I, context: ActivityContext) => O | Promise<O>;

export interface RegisterOptions<I> {
  /** Turns the run input into the handler's type; raise to refuse (reported as `INPUT_INVALID`, not retried). */
  parse?: (raw: unknown) => I;
}

/** True for the error an aborted activity raises. */
export function isCancellation(error: unknown): boolean {
  return (
    (error instanceof SagatError && error.code === 'CANCELLED') ||
    (error instanceof Error && error.name === 'AbortError')
  );
}
