/** The error envelope of the API (`Error` in the OpenAPI contract), or what the browser made of a failure. */
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
  /** The request never produced an answer (offline, refused, a redirect, a timeout). */
  | 'TRANSPORT';

/** Everything a person needs to report a failure, and nothing the server did not call safe to show. */
export class ApiError extends Error {
  readonly code: ErrorCode;
  readonly status: number;
  readonly retryable: boolean;
  readonly traceId: string | undefined;
  /** A stable refinement of the code (`STATE_CONFLICT`, `STREAMS_PER_SUBJECT`, ...). */
  readonly reason: string | undefined;
  readonly details: Record<string, unknown> | undefined;
  readonly retryAfterMs: number | undefined;

  constructor(init: {
    code: ErrorCode;
    status: number;
    message: string;
    retryable?: boolean;
    traceId?: string | undefined;
    details?: Record<string, unknown> | undefined;
    retryAfterMs?: number | undefined;
  }) {
    super(init.message);
    this.name = 'ApiError';
    this.code = init.code;
    this.status = init.status;
    this.retryable = init.retryable ?? false;
    this.traceId = init.traceId;
    this.details = init.details;
    // A stable name from a short list, shown to people: never more than a name's worth of what the server sent.
    const reason = init.details?.['reason'];
    this.reason = typeof reason === 'string' && /^[A-Z][A-Z0-9_]{0,63}$/.test(reason) ? reason : undefined;
    this.retryAfterMs = init.retryAfterMs;
  }
}

export function isApiError(error: unknown): error is ApiError {
  return error instanceof ApiError;
}

const KNOWN: ReadonlySet<string> = new Set<ErrorCode>([
  'BAD_REQUEST',
  'UNAUTHENTICATED',
  'FORBIDDEN',
  'NOT_FOUND',
  'CONFLICT',
  'VALIDATION_FAILED',
  'RATE_LIMITED',
  'QUOTA_EXCEEDED',
  'UNAVAILABLE',
  'INTERNAL',
]);

const MAX_MESSAGE = 300;

function byStatus(status: number): ErrorCode {
  if (status === 400) return 'BAD_REQUEST';
  if (status === 401) return 'UNAUTHENTICATED';
  if (status === 403) return 'FORBIDDEN';
  if (status === 404) return 'NOT_FOUND';
  if (status === 409) return 'CONFLICT';
  if (status === 422) return 'VALIDATION_FAILED';
  if (status === 429) return 'RATE_LIMITED';
  if (status === 408 || status === 502 || status === 503 || status === 504) return 'UNAVAILABLE';
  return 'INTERNAL';
}

/** Seconds (the only form the platform sends), capped at a minute. */
export function retryAfterMs(header: string | null): number | undefined {
  if (header === null || !/^\d{1,6}$/.test(header.trim())) return undefined;
  return Math.min(Number(header.trim()) * 1000, 60_000);
}

/**
 * Reads the answer of a failed request. The envelope is trusted for its shape only: the message is
 * clipped and shown as text, the code must be one the contract names, the trace id must look like
 * one. Anything else (a proxy's HTML, an empty body) becomes an error of the status.
 */
export function errorFromResponse(status: number, body: unknown, headers: Pick<Headers, 'get'>): ApiError {
  const envelope = body !== null && typeof body === 'object' ? (body as Record<string, unknown>) : {};
  const code = typeof envelope['code'] === 'string' && KNOWN.has(envelope['code']) ? (envelope['code'] as ErrorCode) : byStatus(status);
  const message = typeof envelope['message'] === 'string' && envelope['message'] !== '' ? envelope['message'].slice(0, MAX_MESSAGE) : defaultMessage(code);
  const traceId = typeof envelope['traceId'] === 'string' && /^[0-9a-f]{32}$/.test(envelope['traceId']) ? envelope['traceId'] : undefined;
  const details = envelope['details'] !== null && typeof envelope['details'] === 'object' && !Array.isArray(envelope['details']) ? (envelope['details'] as Record<string, unknown>) : undefined;
  return new ApiError({
    code,
    status,
    message,
    retryable: envelope['retryable'] === true || code === 'RATE_LIMITED' || code === 'UNAVAILABLE',
    traceId,
    details,
    retryAfterMs: retryAfterMs(headers.get('retry-after')),
  });
}

export function transportError(cause: unknown): ApiError {
  const aborted = cause instanceof DOMException && cause.name === 'AbortError';
  return new ApiError({
    code: 'TRANSPORT',
    status: 0,
    message: aborted ? 'The request was cancelled or took too long.' : 'The platform could not be reached.',
    retryable: true,
  });
}

function defaultMessage(code: ErrorCode): string {
  switch (code) {
    case 'UNAUTHENTICATED':
      return 'Your session is not valid. Sign in again.';
    case 'FORBIDDEN':
      return 'You are not allowed to do this.';
    case 'NOT_FOUND':
      return 'It does not exist, or it is not yours to see.';
    case 'CONFLICT':
      return 'Someone changed this first.';
    case 'RATE_LIMITED':
      return 'Too many requests. Wait a moment.';
    case 'UNAVAILABLE':
      return 'The platform is not available right now.';
    case 'TRANSPORT':
      return 'The platform could not be reached.';
    default:
      return 'The request failed.';
  }
}

/**
 * What a person can do about a failure, for the codes whose message does not already say (a validation message or a
 * conflict names its own way out). It is advice, not a promise: the platform's message and code stay the authority.
 */
export function remedy(code: ErrorCode): string | undefined {
  switch (code) {
    case 'UNAUTHENTICATED':
      return 'Sign in again.';
    case 'FORBIDDEN':
      return 'Ask an administrator for the permission it needs.';
    case 'NOT_FOUND':
      return 'Check the address and the namespace you are working in.';
    case 'RATE_LIMITED':
      return 'Wait a moment and try again.';
    case 'QUOTA_EXCEEDED':
      return 'Free some of it, or ask an administrator to raise the quota.';
    case 'UNAVAILABLE':
    case 'TRANSPORT':
      return 'Try again in a moment; if it keeps failing, tell whoever runs the platform.';
    case 'INTERNAL':
      return 'Tell whoever runs the platform, and quote the trace id.';
    default:
      return undefined;
  }
}

