import { CATALOG } from './catalog.js';

/** The canonical error codes of the SDKs (sdks/SPEC.md §2). */
export type ErrorCode =
  | 'BAD_REQUEST'
  | 'UNAUTHENTICATED'
  | 'FORBIDDEN'
  | 'NOT_FOUND'
  | 'CONFLICT'
  | 'VALIDATION_FAILED'
  | 'RATE_LIMITED'
  | 'QUOTA_EXCEEDED'
  | 'UNAVAILABLE'
  | 'INTERNAL'
  | 'STALE_ATTEMPT'
  | 'IDEMPOTENCY_KEY_REUSED'
  | 'NOT_BOUND'
  | 'SECRET_NOT_USABLE'
  | 'UNSUPPORTED_PROTOCOL'
  | 'TRANSPORT'
  | 'CANCELLED'
  | 'PROTOCOL'
  | 'PAYLOAD_TOO_LARGE';

export interface SagatErrorInit {
  code: ErrorCode;
  message: string;
  /** The HTTP status, or the gRPC status name; null before any answer. */
  status?: number | string | null;
  /** Defaults to the catalog's value for the code. */
  retryable?: boolean;
  traceId?: string;
  /** The server's stable sub-reason (`details.reason`, or the gRPC `x-sagat-error` value). */
  reason?: string | null;
  retryAfterMs?: number | null;
}

const entries = new Map(CATALOG.map((entry) => [entry.code, entry]));

/** What to do about a code: the fixed sentence of the catalog. */
export function remediationOf(code: ErrorCode): string {
  return entries.get(code)?.remediation ?? 'Report the trace id to the operator.';
}

/** The default `retryable` of a code, for an error the server did not classify itself. */
export function retryableByDefault(code: ErrorCode): boolean {
  return entries.get(code)?.retryable ?? false;
}

/** The errors a call may be repeated after (SPEC §5): never a 4xx other than 429, whatever its body claims. */
const RETRIABLE: ReadonlySet<ErrorCode> = new Set<ErrorCode>(['TRANSPORT', 'UNAVAILABLE', 'RATE_LIMITED', 'INTERNAL']);

/** Whether repeating the call that raised `error` is allowed: its code is retriable and it says it may be. */
export function isRetriable(error: { code: ErrorCode; retryable: boolean }): boolean {
  return error.retryable && RETRIABLE.has(error.code);
}

/** The code the catalog gives an HTTP status when the body carried no envelope. */
export function codeOfHttpStatus(status: number): ErrorCode {
  for (const entry of CATALOG) {
    if (entry.origin === 'rest' && entry.http.includes(status)) {
      return entry.code as ErrorCode;
    }
  }
  return status >= 500 ? 'INTERNAL' : 'BAD_REQUEST';
}

const MAX_MESSAGE = 512;

/** The longest wait the SDK takes from what a server says (`Retry-After`, `retry_after_ms`). */
export const MAX_WAIT_MS = 60_000;

/**
 * The one error an SDK raises. `message` is safe to log: it never holds a payload, a token or a
 * secret; `remediation` says what to do about `code`.
 */
export class SagatError extends Error {
  readonly code: ErrorCode;
  readonly status: number | string | null;
  readonly retryable: boolean;
  readonly traceId: string;
  readonly reason: string | null;
  readonly retryAfterMs: number | null;
  readonly remediation: string;

  constructor(init: SagatErrorInit) {
    // What a server says is bounded (SPEC §10): its words are cut, a trace id that is not 32 hexadecimal
    // characters is not repeated, and no wait it asks for is longer than a minute.
    super(init.message.slice(0, MAX_MESSAGE));
    this.name = 'SagatError';
    this.code = init.code;
    this.status = init.status ?? null;
    this.retryable = init.retryable ?? retryableByDefault(init.code);
    this.traceId = init.traceId !== undefined && /^[0-9a-f]{32}$/.test(init.traceId) ? init.traceId : '';
    this.reason = init.reason === undefined || init.reason === null ? null : init.reason.slice(0, 128);
    this.retryAfterMs = init.retryAfterMs === undefined || init.retryAfterMs === null ? null : Math.min(init.retryAfterMs, MAX_WAIT_MS);
    this.remediation = remediationOf(init.code);
  }

  /** The fields that are compared across SDKs; the message is the SDK's own words. */
  toJSON(): Record<string, unknown> {
    return {
      code: this.code,
      status: this.status,
      retryable: this.retryable,
      traceId: this.traceId,
      reason: this.reason,
      retryAfterMs: this.retryAfterMs,
      remediation: this.remediation,
      message: this.message,
    };
  }
}

/** A failure the SDK raises about its own input before anything is sent. */
export function invalid(message: string): SagatError {
  return new SagatError({ code: 'VALIDATION_FAILED', message });
}
