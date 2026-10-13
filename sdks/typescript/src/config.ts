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

/**
 * The caller's logger, made harmless: a logger that throws (a full disk, a closed stream) must not end a heartbeat,
 * the poll loop, a report or a retry (SPEC §10), so every call is guarded.
 */
export function safeLogger(logger: Logger): Logger {
  const guard = (method: Logger['debug']): Logger['debug'] => (message, fields) => {
    try {
      method.call(logger, message, fields);
    } catch {
      /* a broken logger costs nothing */
    }
  };
  return { debug: guard(logger.debug), info: guard(logger.info), warn: guard(logger.warn), error: guard(logger.error) };
}

export interface TlsOptions {
  /** PEM trust roots; the system roots when absent. */
  ca?: string | Buffer;
  /** PEM client certificate and key, for mTLS. */
  cert?: string | Buffer;
  key?: string | Buffer;
  /**
   * The name the server certificate was issued for, when it is not the host of the URL. Required over TLS to a
   * bare IP address (a TLS server name cannot be an IP address), where it is the DNS name in the certificate.
   */
  serverName?: string;
}

/** SDK version, sent in `User-Agent` and as the worker's SDK identity. */
export const SDK_VERSION = '0.1.0';

/** The newest minor version of worker protocol 1 this SDK speaks (SPEC §6.1). */
export const PROTOCOL_VERSION = '1.4';

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
  /** Trust roots and a client certificate (mTLS) for the REST API; the system roots by default. */
  tls?: TlsOptions;
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
