import type { RetryPolicy } from './retry.js';

/** A bearer token, or a function called before every request that returns a fresh one. */
export type TokenSource = string | (() => string | Promise<string>);

/** The SDK never logs a payload, a token or a secret through this. */
export interface Logger {
  debug(message: string, fields?: Record<string, unknown>): void;
  info(message: string, fields?: Record<string, unknown>): void;
  warn(message: string, fields?: Record<string, unknown>): void;
  error(message: string, fields?: Record<string, unknown>): void;
}

export const silentLogger: Logger = { debug() {}, info() {}, warn() {}, error() {} };

export interface TlsOptions {
  /** PEM trust roots; the system roots when absent. */
  ca?: string | Buffer;
  /** PEM client certificate and key, for mTLS. */
  cert?: string | Buffer;
  key?: string | Buffer;
}

/** SDK version, sent in `User-Agent` and as the worker's SDK identity. */
export const SDK_VERSION = '0.1.0';

/** The newest minor version of worker protocol 1 this SDK speaks (SPEC §6.1). */
export const PROTOCOL_VERSION = '1.2';

export interface ClientConfig {
  /** Base URL of the REST API including `/v1`. */
  apiUrl: string;
  /** Slug of the namespace every call is made in. */
  namespace: string;
  token: TokenSource;
  /** Deadline of one call; 30 000 ms by default. */
  timeoutMs?: number;
  retry?: Partial<RetryPolicy>;
  /** Appended to `sagat-sdk-typescript/<version>`. */
  userAgent?: string;
  /** The HTTP transport; the global `fetch` by default. The hook of the conformance suite and of tests. */
  fetch?: typeof fetch;
  /** The ambient trace context, called when a call has no explicit `traceparent`. */
  traceContext?: () => string | undefined;
  logger?: Logger;
}

/**
 * Accepts a plain `http://` URL anywhere but says once that it is not encrypted when the host is not
 * loopback (SPEC §1). Only the host is named: a URL can carry a credential in its userinfo or query.
 */
export function warnIfNotSecure(url: string, logger: Logger): void {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return;
  }
  if (parsed.protocol !== 'http:') return;
  const host = parsed.hostname.replace(/^\[|\]$/g, '');
  const loopback = host === 'localhost' || host === '::1' || /^127(\.\d{1,3}){3}$/.test(host);
  if (!loopback) logger.warn('the connection is not encrypted: use https outside loopback', { host });
}
