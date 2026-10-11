import { request as httpRequest, type IncomingMessage } from 'node:http';
import { Readable } from 'node:stream';
import { Agent, request as httpsRequest } from 'node:https';
import type { TlsOptions } from './config.js';

const NO_BODY = new Set([101, 204, 205, 304]);

/**
 * A `fetch` over `node:https` for a client that is given trust roots or a client certificate: the global
 * `fetch` has no per-client TLS settings. It verifies the server certificate (nothing here turns that
 * off), sends the certificate for mTLS, and never follows a redirect: a 3xx is returned as it is and the
 * transport refuses it, so a credential cannot go to a place the caller did not name.
 */
export function tlsFetch(tls: TlsOptions): typeof fetch {
  const agent = new Agent({
    ca: tls.ca,
    cert: tls.cert,
    key: tls.key,
    servername: tls.serverName,
    keepAlive: true,
  });
  return (input, init) =>
    new Promise<Response>((resolve, reject) => {
      const url = new URL(input instanceof Request ? input.url : String(input));
      const secure = url.protocol === 'https:';
      const headers: Record<string, string> = {};
      new Headers(init?.headers).forEach((value, key) => {
        headers[key] = value;
      });
      const options = { method: init?.method ?? 'GET', headers, signal: init?.signal ?? undefined };
      const handle = (response: IncomingMessage): void => {
        const answer = new Headers();
        for (const [name, value] of Object.entries(response.headers)) {
          if (Array.isArray(value)) for (const one of value) answer.append(name, one);
          else if (value !== undefined) answer.set(name, value);
        }
        const status = response.statusCode ?? 0;
        // The body is handed over as a stream, not collected first: the caller stops reading at its cap, and
        // cancelling the stream ends the request.
        const body = NO_BODY.has(status) ? null : (Readable.toWeb(response) as unknown as ReadableStream<Uint8Array>);
        if (body === null) response.resume();
        resolve(new Response(body, { status, headers: answer }));
      };
      const outgoing = secure
        ? httpsRequest(url, { ...options, agent }, handle)
        : httpRequest(url, options, handle);
      outgoing.on('error', reject);
      if (typeof init?.body === 'string') outgoing.write(init.body);
      outgoing.end();
    });
}
