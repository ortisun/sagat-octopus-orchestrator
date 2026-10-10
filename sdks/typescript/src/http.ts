import type { ClientConfig, TokenSource } from './config.js';
import { SDK_VERSION, silentLogger } from './config.js';
import { SagatError, codeOfHttpStatus, type ErrorCode } from './errors.js';
import { encodeJson } from './json.js';
import { DEFAULT_RETRY, delayAfter, sleep, type RetryPolicy } from './retry.js';

const CODES = new Set<string>([
  'BAD_REQUEST', 'UNAUTHENTICATED', 'FORBIDDEN', 'NOT_FOUND', 'CONFLICT', 'VALIDATION_FAILED',
  'RATE_LIMITED', 'QUOTA_EXCEEDED', 'UNAVAILABLE', 'INTERNAL',
]);

export interface HttpRequest {
  method: 'GET' | 'POST' | 'PUT' | 'DELETE';
  /** Path under the base URL, starting with `/`. */
  path: string;
  query?: Record<string, string | number | undefined>;
  body?: unknown;
  /** Present on a POST that may be repeated: the call is retried because the server deduplicates it. */
  idempotencyKey?: string;
  traceparent?: string;
  /** True for a call whose repetition cannot do harm (GET, or a POST that carries a key). */
  safeToRepeat: boolean;
}

export interface HttpResponse {
  status: number;
  traceId: string;
  body: unknown;
}

export async function resolveToken(token: TokenSource): Promise<string> {
  return typeof token === 'string' ? token : await token();
}

/** The REST transport: one place for headers, error mapping and the retry rules of SPEC §5. */
export class HttpTransport {
  private readonly fetchImpl: typeof fetch;
  private readonly retry: RetryPolicy;
  private readonly timeoutMs: number;
  private readonly logger;

  constructor(private readonly config: ClientConfig) {
    this.fetchImpl = config.fetch ?? fetch;
    this.retry = { ...DEFAULT_RETRY, ...config.retry };
    this.timeoutMs = config.timeoutMs ?? 30_000;
    this.logger = config.logger ?? silentLogger;
  }

  async request(req: HttpRequest): Promise<HttpResponse> {
    const attempts = req.safeToRepeat ? Math.max(1, this.retry.maxAttempts) : 1;
    let last: SagatError | undefined;
    for (let n = 0; n < attempts; n++) {
      try {
        return await this.once(req);
      } catch (error) {
        if (!(error instanceof SagatError)) throw error;
        last = error;
        if (!error.retryable || n + 1 >= attempts) throw error;
        const wait = delayAfter(this.retry, n, error);
        this.logger.warn('retrying a request', { method: req.method, path: req.path, code: error.code, attempt: n + 1, waitMs: wait });
        await sleep(wait);
      }
    }
    throw last ?? new SagatError({ code: 'INTERNAL', message: 'no attempt was made' });
  }

  private async once(req: HttpRequest): Promise<HttpResponse> {
    const url = new URL(this.config.apiUrl.replace(/\/+$/, '') + req.path);
    for (const [key, value] of Object.entries(req.query ?? {})) {
      if (value !== undefined) url.searchParams.set(key, String(value));
    }
    const headers: Record<string, string> = {
      authorization: `Bearer ${await resolveToken(this.config.token)}`,
      accept: 'application/json',
      'x-octopus-namespace': this.config.namespace,
      'user-agent': `sagat-sdk-typescript/${SDK_VERSION}${this.config.userAgent ? ` ${this.config.userAgent}` : ''}`,
    };
    if (req.traceparent) headers['traceparent'] = req.traceparent;
    if (req.idempotencyKey) headers['idempotency-key'] = req.idempotencyKey;
    let body: string | undefined;
    if (req.body !== undefined) {
      body = encodeJson(req.body);
      headers['content-type'] = 'application/json';
    }

    let response: Response;
    try {
      response = await this.fetchImpl(url, {
        method: req.method,
        headers,
        body,
        signal: AbortSignal.timeout(this.timeoutMs),
      });
    } catch (error) {
      if (error instanceof SagatError) throw error;
      const timedOut = error instanceof Error && (error.name === 'TimeoutError' || error.name === 'AbortError');
      throw new SagatError({
        code: 'TRANSPORT',
        message: timedOut ? 'the request timed out' : 'no answer from the server',
        status: null,
      });
    }

    const traceId = response.headers.get('x-trace-id') ?? '';
    let text: string;
    try {
      text = await response.text();
    } catch {
      throw new SagatError({ code: 'TRANSPORT', message: 'the answer was cut off', status: response.status, traceId });
    }
    if (response.status >= 200 && response.status < 300) {
      if (text === '') return { status: response.status, traceId, body: null };
      try {
        return { status: response.status, traceId, body: JSON.parse(text) };
      } catch {
        throw new SagatError({ code: 'PROTOCOL', message: 'the answer is not JSON', status: response.status, traceId });
      }
    }
    throw this.errorOf(response, text, traceId);
  }

  private errorOf(response: Response, text: string, traceId: string): SagatError {
    const retryAfter = response.headers.get('retry-after');
    const retryAfterMs = retryAfter !== null && /^\d+$/.test(retryAfter) ? Number(retryAfter) * 1000 : null;
    let envelope: Record<string, unknown> | undefined;
    try {
      const parsed: unknown = JSON.parse(text);
      if (parsed !== null && typeof parsed === 'object') envelope = parsed as Record<string, unknown>;
    } catch {
      /* an intermediary's page: classify by status */
    }
    const given = typeof envelope?.['code'] === 'string' && CODES.has(envelope['code']) ? (envelope['code'] as ErrorCode) : undefined;
    const code = given ?? codeOfHttpStatus(response.status);
    const details = envelope?.['details'] as Record<string, unknown> | undefined;
    const reason = typeof details?.['reason'] === 'string' ? details['reason'] : null;
    const message = typeof envelope?.['message'] === 'string' && given ? envelope['message'] : `the server answered ${response.status}`;
    return new SagatError({
      code,
      message,
      status: response.status,
      retryable: typeof envelope?.['retryable'] === 'boolean' && given ? envelope['retryable'] : undefined,
      traceId: typeof envelope?.['traceId'] === 'string' ? envelope['traceId'] : traceId,
      reason,
      retryAfterMs,
    });
  }
}
