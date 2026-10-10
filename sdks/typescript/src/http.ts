import type { ClientConfig, TokenSource } from './config.js';
import { SDK_VERSION, safeLogger, silentLogger } from './config.js';
import { SagatError, codeOfHttpStatus, isRetriable, providerFailure, type ErrorCode } from './errors.js';
import { encodeJson, parseJson } from './json.js';
import { tlsFetch } from './tlsfetch.js';
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
  if (typeof token === 'string') return token;
  try {
    return await token();
  } catch {
    // The provider's own error may carry what it was talking to (a client secret, a URL): only its class of failure is ours.
    throw providerFailure();
  }
}

/** The REST transport: one place for headers, error mapping and the retry rules of SPEC §5. */
export class HttpTransport {
  private readonly fetchImpl: typeof fetch;
  private readonly retry: RetryPolicy;
  private readonly timeoutMs: number;
  private readonly logger;

  readonly #config: ClientConfig;

  constructor(config: ClientConfig) {
    this.#config = config;
    this.fetchImpl = config.fetch ?? (config.tls ? tlsFetch(config.tls) : fetch);
    this.retry = { ...DEFAULT_RETRY, ...config.retry };
    this.timeoutMs = config.timeoutMs ?? 30_000;
    this.logger = safeLogger(config.logger ?? silentLogger);
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
        if (!isRetriable(error) || n + 1 >= attempts) throw error;
        const wait = delayAfter(this.retry, n, error);
        this.logger.warn('retrying a request', { method: req.method, path: req.path, code: error.code, attempt: n + 1, waitMs: wait });
        await sleep(wait);
      }
    }
    throw last ?? new SagatError({ code: 'INTERNAL', message: 'no attempt was made' });
  }

  private async once(req: HttpRequest): Promise<HttpResponse> {
    const url = new URL(this.#config.apiUrl.replace(/\/+$/, '') + req.path);
    for (const [key, value] of Object.entries(req.query ?? {})) {
      if (value !== undefined) url.searchParams.set(key, String(value));
    }
    const headers: Record<string, string> = {
      authorization: `Bearer ${await resolveToken(this.#config.token)}`,
      accept: 'application/json',
      'x-octopus-namespace': this.#config.namespace,
      'user-agent': `sagat-sdk-typescript/${SDK_VERSION}${this.#config.userAgent ? ` ${this.#config.userAgent}` : ''}`,
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
        // A redirect would carry the credential to a place the caller did not name: it is never followed.
        redirect: 'manual',
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
      text = await readCapped(response, MAX_BODY_BYTES);
    } catch (error) {
      if (error instanceof SagatError) throw error;
      throw new SagatError({ code: 'TRANSPORT', message: 'the answer was cut off', status: response.status, traceId });
    }
    if (response.status >= 300 && response.status < 400) {
      throw new SagatError({ code: 'PROTOCOL', message: 'the server redirected the call; the SDK does not follow redirects', status: response.status, traceId });
    }
    if (response.status >= 200 && response.status < 300) {
      if (text === '') return { status: response.status, traceId, body: null };
      try {
        return { status: response.status, traceId, body: parseJson(text) };
      } catch (error) {
        if (error instanceof SagatError) throw new SagatError({ code: 'PROTOCOL', message: error.message, status: response.status, traceId });
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
    // A call the server does not have (SPEC §9): 501; a 404 or a 405 whose envelope says so (the platform tells an
    // unknown route and a wrong method apart from a run that does not exist); or a 404 that is not the platform's
    // own envelope at all.
    const said = typeof (envelope?.['details'] as Record<string, unknown> | undefined)?.['reason'] === 'string' ? ((envelope?.['details'] as Record<string, unknown>)['reason'] as string) : undefined;
    if (
      response.status === 501 ||
      (response.status === 404 && (given === undefined || said === 'ROUTE_NOT_FOUND')) ||
      (response.status === 405 && said === 'METHOD_NOT_ALLOWED')
    ) {
      return new SagatError({
        code: 'NOT_FOUND',
        message: 'the server does not have this call',
        status: response.status,
        retryable: false,
        traceId,
        reason: 'UNSUPPORTED_BY_SERVER',
        retryAfterMs: null,
      });
    }
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

/** The most of an answer the SDK reads: a server that streams on is told "no" instead of filling memory (SPEC §10). */
const MAX_BODY_BYTES = 32 * 1024 * 1024;

async function readCapped(response: Response, limit: number): Promise<string> {
  const declared = Number(response.headers.get('content-length'));
  if (Number.isFinite(declared) && declared > limit) {
    await response.body?.cancel().catch(() => undefined);
    throw new SagatError({ code: 'PROTOCOL', message: 'the answer is larger than the SDK reads', status: response.status });
  }
  if (!response.body) return '';
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let total = 0;
  let text = '';
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > limit) {
      await reader.cancel().catch(() => undefined);
      throw new SagatError({ code: 'PROTOCOL', message: 'the answer is larger than the SDK reads', status: response.status });
    }
    text += decoder.decode(value, { stream: true });
  }
  return text + decoder.decode();
}
