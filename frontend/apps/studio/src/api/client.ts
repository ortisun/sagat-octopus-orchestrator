import { parseAuthoringJson, UnsafeJsonNumber } from '../model/safeJson';
import { ApiError, errorFromResponse, transportError } from './errors';

export interface ClientConfig {
  /** Where the API is, as a path on the page's own origin (`/v1`). */
  baseUrl: string;
  /** The bearer token of the session, or nothing before sign-in. Read on every request. */
  token: () => string | undefined;
  /** The slug of the namespace the user works in (`X-Octopus-Namespace`). */
  namespace: () => string | undefined;
  /** Called when the platform says the token is not valid, so the session can end. */
  onUnauthenticated: () => void;
  fetch?: typeof globalThis.fetch;
  timeoutMs?: number;
  /** An idempotency key for a change; a new one per call unless the caller keeps one across retries. */
  newKey?: () => string;
}

export interface RequestOptions {
  /** Refuse numeric JSON tokens that this authoring UI cannot preserve exactly. */
  safeNumbers?: boolean;

  query?: Record<string, string | number | boolean | undefined>;
  body?: unknown;
  signal?: AbortSignal | undefined;
  /** Use this key instead of a new one (a retry of the same intent keeps its key). */
  key?: string;
  headers?: Record<string, string>;
}

type Method = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';

const DEFAULT_TIMEOUT_MS = 30_000;

/** Joins path segments, each percent-encoded, so an id from the URL can never change the path. */
export function path(strings: TemplateStringsArray, ...values: (string | number)[]): string {
  return strings.reduce((out, text, i) => out + text + (i < values.length ? encodeURIComponent(String(values[i])) : ''), '');
}

/**
 * The only way the Studio talks to the platform. It sends the token to the page's own origin and
 * nowhere else (a path, never a URL), follows no redirect, sends no cookie, remembers nothing of
 * what it reads, and gives every change an idempotency key.
 */
export class ApiClient {
  readonly #config: ClientConfig;

  constructor(config: ClientConfig) {
    this.#config = config;
  }

  get<T>(url: string, options: RequestOptions = {}): Promise<T> {
    return this.request<T>('GET', url, options);
  }

  post<T>(url: string, options: RequestOptions = {}): Promise<T> {
    return this.request<T>('POST', url, options);
  }

  put<T>(url: string, options: RequestOptions = {}): Promise<T> {
    return this.request<T>('PUT', url, options);
  }

  patch<T>(url: string, options: RequestOptions = {}): Promise<T> {
    return this.request<T>('PATCH', url, options);
  }

  delete<T>(url: string, options: RequestOptions = {}): Promise<T> {
    return this.request<T>('DELETE', url, options);
  }

  /** The same client for another token, for finding out whose a token is before it becomes the session's. */
  withToken(token: string): ApiClient {
    return new ApiClient({ ...this.#config, token: () => token, onUnauthenticated: () => undefined });
  }

  /** The headers of a request to the API, for the stream, which reads the body itself. */
  headers(extra: Record<string, string> = {}): Record<string, string> {
    const headers: Record<string, string> = { Accept: 'application/json', ...extra };
    const token = this.#config.token();
    if (token !== undefined && token !== '') headers['Authorization'] = `Bearer ${token}`;
    const namespace = this.#config.namespace();
    if (namespace !== undefined && namespace !== '') headers['X-Octopus-Namespace'] = namespace;
    return headers;
  }

  url(url: string, query?: RequestOptions['query']): string {
    if (!url.startsWith('/') || url.startsWith('//') || url.includes('\\') || url.includes('..')) {
      throw new Error('API paths are absolute paths of the page origin');
    }
    const params = new URLSearchParams();
    for (const [key, value] of Object.entries(query ?? {})) {
      if (value !== undefined) params.set(key, String(value));
    }
    const search = params.toString();
    return `${this.#config.baseUrl}${url}${search === '' ? '' : `?${search}`}`;
  }

  /**
   * Opens a response whose body the caller reads itself (the stream). The same protections as
   * [`request`]; no timeout (a stream is meant to stay open) and no parsing: the caller checks the
   * status, and turns a failed one into an error with [`failure`].
   */
  async open(url: string, options: { query?: RequestOptions['query']; headers?: Record<string, string>; signal?: AbortSignal | undefined } = {}): Promise<Response> {
    const target = this.url(url, options.query);
    try {
      return await (this.#config.fetch ?? globalThis.fetch)(target, {
        method: 'GET',
        headers: this.headers(options.headers),
        ...(options.signal ? { signal: options.signal } : {}),
        credentials: 'omit',
        redirect: 'error',
        cache: 'no-store',
        referrerPolicy: 'no-referrer',
      });
    } catch (cause) {
      if (options.signal?.aborted === true) throw cause;
      throw transportError(cause);
    }
  }

  /** The error a failed response is, and the end of the session if it says the token is not valid. */
  async failure(response: Response): Promise<ApiError> {
    const error = errorFromResponse(response.status, await readBody(response).catch(() => undefined), response.headers);
    if (error.code === 'UNAUTHENTICATED') this.#config.onUnauthenticated();
    return error;
  }

  /** The user's authentication ended: tell the session. */
  unauthenticated(): void {
    this.#config.onUnauthenticated();
  }

  async request<T>(method: Method, url: string, options: RequestOptions = {}): Promise<T> {
    const headers = this.headers(options.headers);
    let body: string | undefined;
    if (options.body !== undefined) {
      body = JSON.stringify(options.body);
      headers['Content-Type'] = 'application/json';
    }
    if (method !== 'GET') {
      headers['Idempotency-Key'] = options.key ?? (this.#config.newKey ?? crypto.randomUUID.bind(crypto))();
    }
    // A path that is not a path of this origin is a programming error, not a failure of the platform.
    const target = this.url(url, options.query);
    const timeout = new AbortController();
    const timer = setTimeout(() => timeout.abort(), this.#config.timeoutMs ?? DEFAULT_TIMEOUT_MS);
    const signal = options.signal ? AbortSignal.any([options.signal, timeout.signal]) : timeout.signal;
    let response: Response;
    try {
      response = await (this.#config.fetch ?? globalThis.fetch)(target, {
        method,
        headers,
        ...(body === undefined ? {} : { body }),
        signal,
        credentials: 'omit',
        redirect: 'error',
        cache: 'no-store',
        referrerPolicy: 'no-referrer',
      });
    } catch (cause) {
      if (options.signal?.aborted === true) throw cause;
      throw transportError(cause);
    } finally {
      clearTimeout(timer);
    }
    const parsed = await readBody(response, options.safeNumbers);
    if (!response.ok) {
      const error = errorFromResponse(response.status, parsed, response.headers);
      if (error.code === 'UNAUTHENTICATED') this.#config.onUnauthenticated();
      throw error;
    }
    return parsed as T;
  }
}

async function readBody(response: Response, safeNumbers = false): Promise<unknown> {
  if (response.status === 204) return undefined;
  const text = await response.text().catch(() => '');
  if (text === '') return undefined;
  try {
    return safeNumbers ? parseAuthoringJson(text) : JSON.parse(text) as unknown;
  } catch (cause) {
    if (cause instanceof UnsafeJsonNumber) throw new ApiError({ code: 'VALIDATION_FAILED', status: response.status, message: 'This definition contains JSON numbers the Studio cannot preserve exactly. Use the SDK or API to represent exact integers and decimals as strings before editing it here.' });
    // A proxy's page, not the platform's answer.
    if (response.ok) {
      throw new ApiError({ code: 'INTERNAL', status: response.status, message: 'The platform sent an answer that is not JSON.' });
    }
    return undefined;
  }
}
