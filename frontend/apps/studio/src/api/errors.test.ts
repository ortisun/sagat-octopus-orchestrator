import { describe, expect, it } from 'vitest';
import { ApiError, errorFromResponse, retryAfterMs, transportError } from './errors';

const headers = (values: Record<string, string> = {}) => new Headers(values);

describe('errorFromResponse', () => {
  it('keeps the envelope of the API: code, safe message, trace id, reason and details', () => {
    const error = errorFromResponse(
      409,
      { code: 'CONFLICT', message: 'the draft moved on', retryable: false, traceId: 'a'.repeat(32), details: { reason: 'STATE_CONFLICT', current: 3 } },
      headers(),
    );
    expect(error).toBeInstanceOf(ApiError);
    expect([error.code, error.status, error.message, error.retryable]).toEqual(['CONFLICT', 409, 'the draft moved on', false]);
    expect(error.traceId).toBe('a'.repeat(32));
    expect(error.reason).toBe('STATE_CONFLICT');
    expect(error.details).toEqual({ reason: 'STATE_CONFLICT', current: 3 });
  });

  it('keeps what the platform says to do about it, clipped and as text', () => {
    const error = errorFromResponse(409, { code: 'CONFLICT', message: 'x', remediation: 'Read the run again and decide on what it shows now.' }, headers());
    expect(error.remediation).toBe('Read the run again and decide on what it shows now.');
    expect(errorFromResponse(409, { code: 'CONFLICT', message: 'x', remediation: 'y'.repeat(5000) }, headers()).remediation).toHaveLength(300);
    for (const odd of [undefined, '', 7, ['a'], { text: 'no' }]) {
      expect(errorFromResponse(409, { code: 'CONFLICT', message: 'x', remediation: odd }, headers()).remediation).toBeUndefined();
    }
  });

  it('does not believe a trace id that is not one, a code the contract does not name, or an endless message', () => {
    const error = errorFromResponse(
      500,
      { code: 'EXPLODED', message: 'x'.repeat(5000), traceId: '<script>alert(1)</script>', details: ['no'] },
      headers(),
    );
    expect(error.code).toBe('INTERNAL');
    expect(error.traceId).toBeUndefined();
    expect(error.message.length).toBe(300);
    expect(error.details).toBeUndefined();
  });

  it('turns an answer that is not the envelope (a proxy page, nothing) into an error of the status', () => {
    for (const [status, code] of [
      [401, 'UNAUTHENTICATED'],
      [403, 'FORBIDDEN'],
      [404, 'NOT_FOUND'],
      [422, 'VALIDATION_FAILED'],
      [429, 'RATE_LIMITED'],
      [502, 'UNAVAILABLE'],
      [418, 'INTERNAL'],
    ] as const) {
      expect(errorFromResponse(status, '<html>bad gateway</html>', headers()).code).toBe(code);
      expect(errorFromResponse(status, null, headers()).code).toBe(code);
    }
    expect(errorFromResponse(503, null, headers()).retryable).toBe(true);
  });

  it('reads Retry-After as seconds only and never waits more than a minute', () => {
    expect(retryAfterMs('5')).toBe(5000);
    expect(retryAfterMs('999999')).toBe(60_000);
    for (const bad of [null, '', 'Wed, 21 Oct 2026 07:28:00 GMT', '-1', '1.5', '1e3', '0x10']) {
      expect(retryAfterMs(bad)).toBeUndefined();
    }
    expect(errorFromResponse(429, null, headers({ 'retry-after': '7' })).retryAfterMs).toBe(7000);
  });

  it('describes a request that never got an answer without leaking what failed', () => {
    const offline = transportError(new TypeError('Failed to fetch https://internal.example/secret'));
    expect([offline.code, offline.status, offline.retryable]).toEqual(['TRANSPORT', 0, true]);
    expect(offline.message).not.toContain('internal.example');
    expect(transportError(new DOMException('x', 'AbortError')).message).toMatch(/cancelled or took too long/);
  });
});
